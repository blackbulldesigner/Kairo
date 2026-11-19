//! Clips para Discord, que deja subir hasta 10 MB sin Nitro. Si el clip pesa más, se comprime una copia en la
//! carpeta temporal (con el mismo nombre, para que en el chat se vea igual) y esa es la que se arrastra o se
//! copia. El clip de la biblioteca no se toca.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context, Result};

use super::comprimir::{self, Opcion};

/// Lo más que deja subir Discord sin Nitro.
pub const MAXIMO: u64 = 10_000_000;

/// Carpeta de las copias: `%TEMP%\Kairo Discord`.
fn carpeta() -> PathBuf {
    std::env::temp_dir().join("Kairo Discord")
}

/// Dónde va la copia de este clip: una subcarpeta por clip, así el archivo conserva su nombre.
fn ruta_copia(clip: &Path) -> PathBuf {
    let mut h = DefaultHasher::new();
    clip.hash(&mut h);
    carpeta().join(format!("{:016x}", h.finish())).join(clip.file_name().unwrap_or_default())
}

/// El archivo para mandar a Discord, si ya está: el clip mismo si pesa hasta 10 MB, o su copia comprimida
/// (si es más nueva que el clip).
pub fn listo(clip: &Path) -> Option<PathBuf> {
    let original = std::fs::metadata(clip).ok()?;
    if original.len() <= MAXIMO {
        return Some(clip.to_path_buf());
    }
    let copia = ruta_copia(clip);
    let datos = std::fs::metadata(&copia).ok()?;
    let al_dia = matches!((datos.modified(), original.modified()), (Ok(c), Ok(o)) if c >= o);
    (datos.len() > 0 && datos.len() <= MAXIMO && al_dia).then_some(copia)
}

/// Comprime la copia para Discord (con prioridad baja) y devuelve dónde quedó.
pub fn preparar(clip: &Path, progreso: &dyn Fn(f32), cancelar: &AtomicBool) -> Result<PathBuf> {
    let info = super::info(clip)?;
    let (alto, bitrate) = comprimir::plan(Opcion::Discord, &info)?;
    let destino = ruta_copia(clip);
    if let Some(dir) = destino.parent() {
        std::fs::create_dir_all(dir).context("no se pudo crear la carpeta temporal")?;
    }
    let pedido = comprimir::Pedido { origen: clip.to_path_buf(), destino: destino.clone(), alto, bitrate };
    comprimir::comprimir(&pedido, progreso, cancelar)?;
    let peso = std::fs::metadata(&destino)?.len();
    if peso > MAXIMO {
        let _ = std::fs::remove_file(&destino);
        bail!("quedó en {:.1} MB, más de lo que deja Discord: recórtalo (en Editar) y prueba de nuevo", peso as f64 / 1e6);
    }
    Ok(destino)
}

/// Borra las copias de hace más de un día (Discord ya las subió).
pub fn limpiar_viejas() {
    let Ok(dirs) = std::fs::read_dir(carpeta()) else { return };
    let limite = SystemTime::now() - Duration::from_secs(24 * 3600);
    for d in dirs.flatten() {
        let vieja = d.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < limite);
        if vieja {
            let _ = std::fs::remove_dir_all(d.path());
        }
    }
}
