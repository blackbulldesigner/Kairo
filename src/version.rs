//! De qué versión del código sale este exe (lo guarda `build.rs` al compilar).

use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Commit corto (vacío si se compiló sin git).
pub const COMMIT: &str = env!("KAIRO_COMMIT");
pub const RAMA: &str = env!("KAIRO_RAMA");
/// Fecha del commit, "18/11/2026 14:05".
pub const FECHA_COMMIT: &str = env!("KAIRO_FECHA_COMMIT");

/// ¿Se compiló con cambios que todavía no tienen commit? (Entonces el commit no lo identifica del todo.)
pub fn con_cambios_sin_commit() -> bool {
    env!("KAIRO_SUCIO") == "1"
}

/// "v0.2.0 · b820923" (con "+" si tenía cambios sin commit).
pub fn corta() -> String {
    let mut s = format!("v{VERSION}");
    if !COMMIT.is_empty() {
        s += &format!(" · {COMMIT}");
        if con_cambios_sin_commit() {
            s.push('+');
        }
    }
    s
}

/// Cuándo se compiló, en hora local: "18/11/2026 14:32".
pub fn compilado() -> String {
    let segundos: u64 = env!("KAIRO_COMPILADO").parse().unwrap_or(0);
    // FILETIME cuenta intervalos de 100 ns desde 1601.
    let t = (segundos + 11_644_473_600) * 10_000_000;
    let ft = FILETIME { dwLowDateTime: t as u32, dwHighDateTime: (t >> 32) as u32 };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err() {
            return String::new();
        }
        if SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            local = utc;
        }
    }
    format!("{:02}/{:02}/{} {:02}:{:02}", local.wDay, local.wMonth, local.wYear, local.wHour, local.wMinute)
}

pub struct Novedad {
    pub commit: &'static str,
    pub fecha: &'static str,
    pub texto: &'static str,
}

/// Los últimos cambios (el primero es el de este exe).
pub fn novedades() -> Vec<Novedad> {
    include_str!(concat!(env!("OUT_DIR"), "/novedades.txt"))
        .lines()
        .filter_map(|l| {
            let mut p = l.splitn(3, '\u{1f}');
            Some(Novedad { commit: p.next()?, fecha: p.next()?, texto: p.next()? })
        })
        .collect()
}
