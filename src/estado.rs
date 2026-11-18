//! Estado de la grabadora (%APPDATA%\Kairo\estado.json). Lo escribe la grabadora y lo lee la ventana
//! de ajustes para mostrar si está grabando, con qué y si hubo un error.

use serde::{Deserialize, Serialize};

use crate::config::datos_dir;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Estado {
    pub grabando: bool,
    pub pausado: bool,
    /// "1920×1080 a 60 fps · NVIDIA H.264 Encoder MFT".
    pub detalle: String,
    /// Qué impide grabar (o qué atajo no se pudo registrar), dicho para la persona.
    pub error: Option<String>,
    pub ultimo_clip: Option<String>,
    /// El juego detectado ahora.
    pub juego: Option<String>,
    /// La laptop está con batería y se graba en 720p y calidad Ligera.
    pub ahorro: bool,
}

fn archivo() -> std::path::PathBuf {
    datos_dir().join("estado.json")
}

pub fn escribir(e: &Estado) {
    let _ = std::fs::create_dir_all(datos_dir());
    if let Ok(json) = serde_json::to_string_pretty(e) {
        let tmp = archivo().with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, archivo());
        }
    }
}

pub fn leer() -> Option<Estado> {
    serde_json::from_str(&std::fs::read_to_string(archivo()).ok()?).ok()
}
