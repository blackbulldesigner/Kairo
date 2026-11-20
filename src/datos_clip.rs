//! Datos de cada clip, guardados al lado del MP4 en un archivo oculto ("clip.mp4.kairo", JSON): el
//! juego, los nombres de las pistas de audio y el formato. Los usan la biblioteca (filtrar por juego,
//! mostrar la duración sin abrir el video) y, más adelante, el editor (quitar una pista, como Discord).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::{SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DatosClip {
    pub version: u32,
    pub juego: Option<String>,
    /// Pistas de audio en el orden del archivo: "Mezcla" primero y, con pistas separadas, cada fuente.
    pub pistas: Vec<String>,
    /// Segundos.
    pub duracion: f64,
    pub ancho: u32,
    pub alto: u32,
    pub fps: u32,
    /// "2026-11-18 21-30-05".
    pub creado: String,
    /// La mezcla ya está marcada como la pista que tocan los reproductores (ver `pistas_mp4`). Los clips de
    /// antes de la 0.7.2 no la tienen: la biblioteca los repara.
    pub mezcla_marcada: bool,
    /// Marcado con la estrella en la biblioteca: la limpieza automática nunca lo borra.
    pub favorito: bool,
}

pub fn ruta_datos(clip: &Path) -> PathBuf {
    let mut nombre = clip.as_os_str().to_owned();
    nombre.push(".kairo");
    PathBuf::from(nombre)
}

pub fn escribir(clip: &Path, datos: &DatosClip) {
    let ruta = ruta_datos(clip);
    if let Ok(json) = serde_json::to_string_pretty(datos) {
        // Si ya existe está oculto: Windows no deja reemplazar un archivo oculto "creándolo" de nuevo, pero sí
        // vaciarlo y escribirlo.
        let escrito = if ruta.exists() {
            std::fs::OpenOptions::new().write(true).truncate(true).open(&ruta).and_then(|mut f| std::io::Write::write_all(&mut f, json.as_bytes()))
        } else {
            std::fs::write(&ruta, json)
        };
        if escrito.is_ok() {
            // Oculto: en la carpeta de clips solo se ven los videos.
            unsafe {
                let _ = SetFileAttributesW(&HSTRING::from(ruta.as_os_str()), FILE_ATTRIBUTE_HIDDEN);
            }
        }
    }
}

#[allow(dead_code)]
pub fn leer(clip: &Path) -> Option<DatosClip> {
    serde_json::from_str(&std::fs::read_to_string(ruta_datos(clip)).ok()?).ok()
}
