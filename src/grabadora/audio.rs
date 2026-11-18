//! Audio del sistema: todo lo que suena por la salida predeterminada (captura "loopback" de WASAPI).
//! Se convierte a 48 kHz estéreo de 16 bits y se marca con el mismo reloj que el video.

use std::sync::Arc;

use anyhow::{bail, Result};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};

use super::buffer::{TrozoAudio, FRECUENCIA_AUDIO};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Formato {
    F32,
    I16,
    I32,
}

pub struct CapturaAudio {
    cliente: IAudioClient,
    captura: IAudioCaptureClient,
    id: String,
    canales: usize,
    formato: Formato,
    remuestreo: Remuestreo,
    /// Posición (muestras de 48 kHz) donde tiene que empezar el próximo trozo para quedar pegado al anterior.
    siguiente: Option<i64>,
}

/// Si el reloj y la cuenta de muestras difieren en más que esto (20 ms), hubo un hueco de verdad
/// (Windows deja de mandar audio cuando no suena nada) o el reloj del dispositivo se corrió: se
/// vuelve a ubicar por el reloj. Por debajo, la diferencia es solo el temblor de las marcas de tiempo.
const TOLERANCIA: i64 = FRECUENCIA_AUDIO as i64 / 50;

fn muestras_desde_reloj(t: i64) -> i64 {
    (t as i128 * FRECUENCIA_AUDIO as i128 / 10_000_000) as i64
}

unsafe fn id_predeterminado(en: &IMMDeviceEnumerator) -> Result<(IMMDevice, String)> {
    let disp = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
    let p = disp.GetId()?;
    let id = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    Ok((disp, id))
}

/// El id de la salida de audio predeterminada ahora (para notar si la persona cambió de auriculares).
pub fn salida_predeterminada() -> Option<String> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        id_predeterminado(&en).ok().map(|(_, id)| id)
    }
}

impl CapturaAudio {
    pub fn abrir() -> Result<Self> {
        unsafe {
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let (disp, id) = id_predeterminado(&en)?;
            let cliente: IAudioClient = disp.Activate(CLSCTX_ALL, None)?;
            let fmt = cliente.GetMixFormat()?;
            let w = std::ptr::read_unaligned(fmt);
            let (etiqueta, canales, frecuencia, bits) = (w.wFormatTag, w.nChannels as usize, w.nSamplesPerSec, w.wBitsPerSample);
            // 0xFFFE = WAVEFORMATEXTENSIBLE: el tipo real está en SubFormat (1 = PCM, 3 = float).
            let tipo = if etiqueta == 0xFFFE {
                std::ptr::read_unaligned(fmt as *const WAVEFORMATEXTENSIBLE).SubFormat.data1 as u16
            } else {
                etiqueta
            };
            let formato = match (tipo, bits) {
                (3, 32) => Formato::F32,
                (1, 16) => Formato::I16,
                (1, 32) => Formato::I32,
                _ => {
                    CoTaskMemFree(Some(fmt as *const _));
                    bail!("formato de audio no soportado ({tipo}, {bits} bits)");
                }
            };
            let r = cliente.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 2_000_000, 0, fmt, None);
            CoTaskMemFree(Some(fmt as *const _));
            r?;
            let captura: IAudioCaptureClient = cliente.GetService()?;
            cliente.Start()?;
            if canales == 0 {
                bail!("la salida de audio no tiene canales");
            }
            Ok(Self { cliente, captura, id, canales, formato, remuestreo: Remuestreo::new(frecuencia), siguiente: None })
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Lee todo lo que haya llegado desde la última vez.
    pub fn leer(&mut self, salida: &mut Vec<TrozoAudio>) -> Result<()> {
        unsafe {
            loop {
                if self.captura.GetNextPacketSize()? == 0 {
                    return Ok(());
                }
                let mut datos = std::ptr::null_mut();
                let mut cuadros = 0u32;
                let mut flags = 0u32;
                let mut qpc = 0u64;
                self.captura.GetBuffer(&mut datos, &mut cuadros, &mut flags, None, Some(&mut qpc))?;
                let n = cuadros as usize;
                let silencio = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
                let mut estereo = Vec::with_capacity(n * 2);
                if silencio || datos.is_null() {
                    estereo.resize(n * 2, 0.0f32);
                } else {
                    let total = n * self.canales;
                    match self.formato {
                        Formato::F32 => mezclar_a_estereo(std::slice::from_raw_parts(datos as *const f32, total), self.canales, &mut estereo, |x| x),
                        Formato::I16 => mezclar_a_estereo(std::slice::from_raw_parts(datos as *const i16, total), self.canales, &mut estereo, |x| {
                            x as f32 / 32768.0
                        }),
                        Formato::I32 => mezclar_a_estereo(std::slice::from_raw_parts(datos as *const i32, total), self.canales, &mut estereo, |x| {
                            x as f32 / 2_147_483_648.0
                        }),
                    }
                }
                self.captura.ReleaseBuffer(cuadros)?;

                let muestras: Vec<i16> =
                    self.remuestreo.procesar(&estereo).into_iter().map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16).collect();
                if muestras.is_empty() {
                    continue;
                }
                // Dónde va este trozo: pegado al anterior, salvo que el reloj diga que hubo un hueco real.
                let error_de_tiempo = qpc == 0 || flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0;
                let por_reloj = (!error_de_tiempo).then(|| muestras_desde_reloj(qpc as i64));
                let posicion = match (self.siguiente, por_reloj) {
                    (Some(s), Some(r)) if (r - s).abs() <= TOLERANCIA => s,
                    (Some(s), None) => s,
                    (_, Some(r)) => r,
                    (None, None) => muestras_desde_reloj(super::reloj::ahora()),
                };
                self.siguiente = Some(posicion + (muestras.len() / 2) as i64);
                let pts = (posicion as i128 * 10_000_000 / FRECUENCIA_AUDIO as i128) as i64;
                salida.push(TrozoAudio { pts, posicion, muestras: Arc::new(muestras) });
            }
        }
    }
}

impl Drop for CapturaAudio {
    fn drop(&mut self) {
        unsafe {
            let _ = self.cliente.Stop();
        }
    }
}

/// Pasa de N canales a estéreo. Con más de dos (5.1, 7.1) suma el centro y los traseros para no
/// perder diálogos; el subwoofer (canal 4) se deja afuera.
fn mezclar_a_estereo<T: Copy>(datos: &[T], canales: usize, salida: &mut Vec<f32>, a_f32: impl Fn(T) -> f32) {
    for cuadro in datos.chunks_exact(canales) {
        let (l, r) = match canales {
            1 => {
                let m = a_f32(cuadro[0]);
                (m, m)
            }
            2 => (a_f32(cuadro[0]), a_f32(cuadro[1])),
            _ => {
                let (mut l, mut r) = (a_f32(cuadro[0]), a_f32(cuadro[1]));
                let c = a_f32(cuadro[2]) * 0.707;
                l += c;
                r += c;
                for (i, &s) in cuadro.iter().enumerate().skip(4) {
                    if i % 2 == 0 {
                        l += a_f32(s) * 0.707;
                    } else {
                        r += a_f32(s) * 0.707;
                    }
                }
                (l * 0.6, r * 0.6)
            }
        };
        salida.push(l);
        salida.push(r);
    }
}

/// Remuestreo lineal a 48 kHz que recuerda dónde quedó entre un trozo y el siguiente.
struct Remuestreo {
    de: u32,
    /// Posición del próximo cuadro de salida, en cuadros de entrada desde el inicio del trozo actual
    /// (puede ser negativa: entre el último cuadro del trozo anterior y el primero de este).
    pos: f64,
    anterior: (f32, f32),
}

impl Remuestreo {
    fn new(de: u32) -> Self {
        Self { de: de.max(1), pos: 0.0, anterior: (0.0, 0.0) }
    }

    fn procesar(&mut self, entrada: &[f32]) -> Vec<f32> {
        if self.de == FRECUENCIA_AUDIO {
            return entrada.to_vec();
        }
        let n = entrada.len() / 2;
        if n == 0 {
            return Vec::new();
        }
        let paso = self.de as f64 / FRECUENCIA_AUDIO as f64;
        let cuadro = |i: isize| -> (f32, f32) {
            if i < 0 {
                self.anterior
            } else {
                (entrada[i as usize * 2], entrada[i as usize * 2 + 1])
            }
        };
        let mut salida = Vec::with_capacity((n as f64 / paso) as usize * 2 + 4);
        while self.pos < (n - 1) as f64 {
            let i = self.pos.floor();
            let t = (self.pos - i) as f32;
            let (a, b) = (cuadro(i as isize), cuadro(i as isize + 1));
            salida.push(a.0 + (b.0 - a.0) * t);
            salida.push(a.1 + (b.1 - a.1) * t);
            self.pos += paso;
        }
        self.pos -= n as f64;
        self.anterior = cuadro(n as isize - 1);
        salida
    }
}
