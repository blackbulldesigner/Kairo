//! Iniciar con Windows: el valor "Kairo" en HKCU\Software\Microsoft\Windows\CurrentVersion\Run,
//! que abre la grabadora en la bandeja sin ventanas (`--fondo`).

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};

const CLAVE: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NOMBRE: PCWSTR = w!("Kairo");

fn linea_de_comandos(exe: &std::path::Path) -> String {
    format!("\"{}\" --fondo", exe.display())
}

fn registrado() -> Option<String> {
    unsafe {
        let mut largo = 0u32;
        if RegGetValueW(HKEY_CURRENT_USER, CLAVE, NOMBRE, RRF_RT_REG_SZ, None, None, Some(&mut largo)) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; (largo as usize).div_ceil(2) + 1];
        if RegGetValueW(HKEY_CURRENT_USER, CLAVE, NOMBRE, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut largo)) != ERROR_SUCCESS {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }
}

pub fn activo() -> bool {
    registrado().is_some()
}

pub fn poner(si: bool) {
    if let Ok(exe) = std::env::current_exe() {
        poner_para(&exe, si);
    }
}

/// Como `poner`, pero con el exe que se indica (el instalador registra el exe instalado, no el suyo).
pub fn poner_para(exe: &std::path::Path, si: bool) {
    unsafe {
        if si {
            let linea = linea_de_comandos(exe);
            let v: Vec<u16> = linea.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = RegSetKeyValueW(HKEY_CURRENT_USER, CLAVE, NOMBRE, REG_SZ.0, Some(v.as_ptr() as *const _), (v.len() * 2) as u32);
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, CLAVE, NOMBRE);
        }
    }
}

/// Si Kairo se movió de carpeta, el inicio con Windows apunta a un exe que ya no existe: se corrige.
pub fn reparar() {
    let Some(actual) = registrado() else { return };
    let ruta = actual.trim_start().trim_start_matches('"').split('"').next().unwrap_or("").to_string();
    if !std::path::Path::new(&ruta).exists() {
        poner(true);
    }
}
