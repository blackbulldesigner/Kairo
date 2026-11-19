//! Limpieza automática de la carpeta de clips (Guardado → Limpieza automática). Solo toca clips hechos por
//! Kairo (los que tienen su archivo de datos) que no son favoritos:
//! - los de más de N días;
//! - si todos los clips de Kairo juntos pasan de X GB, los más viejos hasta bajar del límite.
//!
//! Van a la Papelera de reciclaje (o se borran del todo, si se eligió). Corre después de guardar un clip, en
//! el hilo que lo guardó, y con "Limpiar ahora".

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::config::Limpieza;
use crate::datos_clip;
use crate::registro::log;

pub const GB: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ClipEnDisco {
    pub ruta: PathBuf,
    pub tamano: u64,
    pub modificado: SystemTime,
    pub favorito: bool,
}

/// Los clips de Kairo en la carpeta y en sus subcarpetas (las de cada juego), hasta 2 niveles.
pub fn clips_de_kairo(carpeta: &Path) -> Vec<ClipEnDisco> {
    fn buscar(dir: &Path, nivel: u32, v: &mut Vec<ClipEnDisco>) {
        let Ok(entradas) = std::fs::read_dir(dir) else { return };
        for e in entradas.flatten() {
            let ruta = e.path();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                if nivel < 2 {
                    buscar(&ruta, nivel + 1, v);
                }
                continue;
            }
            if !ruta.extension().is_some_and(|x| x.eq_ignore_ascii_case("mp4")) {
                continue;
            }
            // Sin datos de Kairo (o con datos que no hizo la grabadora ni el editor) no es un clip de Kairo.
            let Some(d) = datos_clip::leer(&ruta).filter(|d| d.version >= 1) else { continue };
            v.push(ClipEnDisco { tamano: meta.len(), modificado: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), favorito: d.favorito, ruta });
        }
    }
    let mut v = Vec::new();
    buscar(carpeta, 0, &mut v);
    v
}

/// Qué clips borraría la limpieza ahora (los más viejos primero).
pub fn plan(carpeta: &Path, l: &Limpieza) -> Vec<ClipEnDisco> {
    plan_con(carpeta, l.dias, l.gb as u64 * GB)
}

/// Como `plan`, con el límite de tamaño en bytes (0 = sin límite).
fn plan_con(carpeta: &Path, dias: u32, limite_bytes: u64) -> Vec<ClipEnDisco> {
    let mut clips = clips_de_kairo(carpeta);
    clips.sort_by_key(|c| c.modificado);
    let mut borrar = vec![false; clips.len()];
    if dias > 0 {
        let limite = SystemTime::now() - Duration::from_secs(dias as u64 * 86_400);
        for (i, c) in clips.iter().enumerate() {
            if !c.favorito && c.modificado < limite {
                borrar[i] = true;
            }
        }
    }
    if limite_bytes > 0 {
        let limite = limite_bytes;
        let mut total: u64 = clips.iter().zip(&borrar).filter(|(_, b)| !**b).map(|(c, _)| c.tamano).sum();
        for (i, c) in clips.iter().enumerate() {
            if total <= limite {
                break;
            }
            if !c.favorito && !borrar[i] {
                borrar[i] = true;
                total -= c.tamano;
            }
        }
    }
    clips.into_iter().zip(borrar).filter(|(_, b)| *b).map(|(c, _)| c).collect()
}

/// Borra lo que diga el plan. Devuelve cuántos clips y cuántos bytes.
pub fn limpiar(carpeta: &Path, l: &Limpieza) -> Result<(usize, u64), String> {
    let p = plan(carpeta, l);
    if p.is_empty() {
        return Ok((0, 0));
    }
    let bytes: u64 = p.iter().map(|c| c.tamano).sum();
    let rutas: Vec<PathBuf> = p.iter().flat_map(|c| [c.ruta.clone(), datos_clip::ruta_datos(&c.ruta)]).collect();
    if l.sin_papelera {
        for r in &rutas {
            let _ = std::fs::remove_file(r);
        }
    } else {
        crate::abrir::papelera(&rutas)?;
    }
    log(&format!(
        "limpieza: {} clips ({} MB) {}",
        p.len(),
        bytes / (1024 * 1024),
        if l.sin_papelera { "borrados" } else { "a la Papelera" }
    ));
    Ok((p.len(), bytes))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn clip(dir: &Path, nombre: &str, mb: usize, dias: u64, favorito: bool) {
        let ruta = dir.join(nombre);
        std::fs::write(&ruta, vec![0u8; mb * 1024 * 1024]).unwrap();
        let cuando = SystemTime::now() - Duration::from_secs(dias * 86_400);
        std::fs::File::options().write(true).open(&ruta).unwrap().set_modified(cuando).unwrap();
        datos_clip::escribir(&ruta, &datos_clip::DatosClip { version: 1, favorito, ..Default::default() });
    }

    #[test]
    fn plan_de_limpieza() {
        let dir = std::env::temp_dir().join(format!("kairo-prueba-limpieza-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Juego")).unwrap();
        clip(&dir, "viejo.mp4", 1, 40, false);
        clip(&dir, "viejo favorito.mp4", 1, 40, true);
        clip(&dir.join("Juego"), "medio.mp4", 2, 10, false);
        clip(&dir, "nuevo.mp4", 2, 1, false);
        // Un video que no hizo Kairo: nunca se toca.
        std::fs::write(dir.join("ajeno.mp4"), vec![0u8; 3 * 1024 * 1024]).unwrap();

        let nombres = |dias: u32, limite: u64| -> Vec<String> {
            let mut v: Vec<String> = plan_con(&dir, dias, limite).iter().map(|c| c.ruta.file_name().unwrap().to_string_lossy().into_owned()).collect();
            v.sort();
            v
        };
        // Más de 30 días: solo el viejo que no es favorito.
        assert_eq!(nombres(30, 0), ["viejo.mp4"]);
        // Con límite de tamaño: los más viejos hasta bajar (el favorito no se borra y el ajeno no cuenta).
        // Hay 6 MB de Kairo; con el límite en 4 MB se van viejo (1) y medio (2): quedan 3.
        assert_eq!(nombres(0, 4 * 1024 * 1024), ["medio.mp4", "viejo.mp4"]);
        // Los dos límites juntos y sin nada que borrar.
        assert_eq!(nombres(30, 4 * 1024 * 1024), ["medio.mp4", "viejo.mp4"]);
        assert!(nombres(0, 0).is_empty());
        assert!(nombres(0, 100 * 1024 * 1024).is_empty());
        let mut clips = clips_de_kairo(&dir);
        clips.sort_by_key(|c| c.modificado);
        assert_eq!(clips.len(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
