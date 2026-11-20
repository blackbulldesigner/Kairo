//! Sonidos cortos para avisar que un clip se guardó (o que no se pudo). Se generan por código
//! (dos notas suaves) para no depender de archivos.

use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

const FRECUENCIA: u32 = 44_100;

/// WAV en memoria (16 bits, mono) con las notas dadas en Hz, cada una de `ms` milisegundos.
fn wav(notas: &[f32], ms: u32, volumen: f32) -> Vec<u8> {
    let por_nota = (FRECUENCIA * ms / 1000) as usize;
    let mut muestras = Vec::with_capacity(por_nota * notas.len());
    for &f in notas {
        for i in 0..por_nota {
            let t = i as f32 / FRECUENCIA as f32;
            // Entrada y salida suaves para que no haga "clic".
            let env = (i as f32 / 300.0).min(1.0) * ((por_nota - i) as f32 / 1200.0).min(1.0);
            let s = (t * f * std::f32::consts::TAU).sin() * env * volumen;
            muestras.push((s * 32767.0) as i16);
        }
    }
    let datos = muestras.len() as u32 * 2;
    let mut w = Vec::with_capacity(44 + datos as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + datos).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&FRECUENCIA.to_le_bytes());
    w.extend_from_slice(&(FRECUENCIA * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&datos.to_le_bytes());
    for s in muestras {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

fn reproducir(sonido: &'static [u8]) {
    unsafe {
        let _ = PlaySoundW(PCWSTR(sonido.as_ptr() as *const u16), HMODULE::default(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

/// Dos notas hacia arriba: clip guardado.
pub fn guardado() {
    static S: OnceLock<Vec<u8>> = OnceLock::new();
    reproducir(S.get_or_init(|| wav(&[880.0, 1318.5], 70, 0.22)));
}

/// Dos notas hacia abajo: no se pudo guardar.
pub fn error() {
    static S: OnceLock<Vec<u8>> = OnceLock::new();
    reproducir(S.get_or_init(|| wav(&[523.3, 392.0], 110, 0.22)));
}
