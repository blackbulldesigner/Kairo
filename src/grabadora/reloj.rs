//! Reloj común para video y audio: el contador de alta precisión de Windows en unidades de 100 ns,
//! la misma escala que usan Media Foundation y las marcas de tiempo del audio (WASAPI).

use std::sync::OnceLock;

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

pub const SEGUNDO: i64 = 10_000_000;

pub fn ahora() -> i64 {
    static FRECUENCIA: OnceLock<i64> = OnceLock::new();
    let f = *FRECUENCIA.get_or_init(|| {
        let mut f = 0i64;
        unsafe {
            let _ = QueryPerformanceFrequency(&mut f);
        }
        f.max(1)
    });
    let mut c = 0i64;
    unsafe {
        let _ = QueryPerformanceCounter(&mut c);
    }
    ((c as i128 * SEGUNDO as i128) / f as i128) as i64
}

/// Duerme hasta el instante `t` (o vuelve enseguida si ya pasó). Requiere `timeBeginPeriod(1)`
/// para tener precisión de 1 ms; se activa al arrancar la grabadora.
pub fn dormir_hasta(t: i64) {
    let falta = t - ahora();
    if falta > 0 {
        std::thread::sleep(std::time::Duration::from_nanos(falta as u64 * 100));
    }
}
