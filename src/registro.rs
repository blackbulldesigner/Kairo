//! Registro de lo que pasa (%APPDATA%\Kairo\kairo.log). Sirve para entender fallas en PCs ajenas.

use std::io::Write;

use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::config::datos_dir;

/// Tamaño máximo del registro: al pasarlo se empieza de nuevo.
const MAXIMO: u64 = 1024 * 1024;

/// "2026-11-18 21-30-05" (sirve para nombres de archivo).
pub fn fecha_para_archivo() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:04}-{:02}-{:02} {:02}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

fn marca() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds)
}

pub fn log(mensaje: &str) {
    let dir = datos_dir();
    let _ = std::fs::create_dir_all(&dir);
    let ruta = dir.join("kairo.log");
    if std::fs::metadata(&ruta).map(|m| m.len() > MAXIMO).unwrap_or(false) {
        let _ = std::fs::remove_file(&ruta);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&ruta) {
        let _ = writeln!(f, "[{}] [{}] {}", marca(), std::process::id(), mensaje);
    }
    #[cfg(debug_assertions)]
    eprintln!("{mensaje}");
}
