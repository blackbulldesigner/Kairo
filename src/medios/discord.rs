//! Clips para Discord, que deja subir hasta 20 MB sin Nitro. Si el clip pesa más, se comprime una copia en la
//! carpeta "Para Discord" (dentro de la de clips, con el mismo nombre, para que en el chat se vea igual) y esa
//! es la que se copia al portapapeles. El clip no se toca, y la copia se borra sola después del tiempo elegido
//! (`limpieza::borrar_copias_discord`).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{bail, Context, Result};

use super::comprimir::{self, Opcion};

/// Lo más que deja subir Discord sin Nitro.
pub const MAXIMO: u64 = 20_000_000;

/// Dónde va la copia de este clip: en "Para Discord", con el mismo nombre.
fn ruta_copia(clip: &Path, carpeta: &Path) -> PathBuf {
    carpeta.join(crate::limpieza::CARPETA_DISCORD).join(clip.file_name().unwrap_or_default())
}

/// El archivo para mandar a Discord, si ya está: el clip mismo si pesa hasta 20 MB, o su copia comprimida
/// (si es más nueva que el clip).
pub fn listo(clip: &Path, carpeta: &Path) -> Option<PathBuf> {
    let original = std::fs::metadata(clip).ok()?;
    if original.len() <= MAXIMO {
        return Some(clip.to_path_buf());
    }
    let copia = ruta_copia(clip, carpeta);
    let datos = std::fs::metadata(&copia).ok()?;
    let al_dia = matches!((datos.modified(), original.modified()), (Ok(c), Ok(o)) if c >= o);
    (datos.len() > 0 && datos.len() <= MAXIMO && al_dia).then_some(copia)
}

/// Comprime la copia para Discord (con prioridad baja) y devuelve dónde quedó.
pub fn preparar(clip: &Path, carpeta: &Path, progreso: &dyn Fn(f32), cancelar: &AtomicBool) -> Result<PathBuf> {
    let info = super::info(clip)?;
    let (alto, bitrate) = comprimir::plan(Opcion::Discord, &info)?;
    let destino = ruta_copia(clip, carpeta);
    if let Some(dir) = destino.parent() {
        std::fs::create_dir_all(dir).context("no se pudo crear la carpeta «Para Discord»")?;
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
