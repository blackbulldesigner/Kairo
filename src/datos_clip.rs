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
}

pub fn ruta_datos(clip: &Path) -> PathBuf {
    let mut nombre = clip.as_os_str().to_owned();
    nombre.push(".kairo");
    PathBuf::from(nombre)
}

pub fn escribir(clip: &Path, datos: &DatosClip) {
    let ruta = ruta_datos(clip);
    if let Ok(json) = serde_json::to_string_pretty(datos) {
        if std::fs::write(&ruta, json).is_ok() {
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
