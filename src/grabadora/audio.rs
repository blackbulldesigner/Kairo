//! Captura de audio con WASAPI. Tres orígenes:
//! - **Salida completa**: todo lo que suena por la salida predeterminada (loopback).
//! - **Proceso**: solo lo que suena de un proceso y sus hijos (el juego), o todo menos eso (el resto
//!   del sistema). Necesita Windows 10 2004 o más nuevo.
//! - **Micrófono**: la entrada predeterminada.
//!
//! Todo se convierte a 48 kHz estéreo de 16 bits y se marca con el mismo reloj que el video.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use windows::core::{implement, Interface, IUnknown, HRESULT, PROPVARIANT};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};
use windows::Win32::System::Threading::CreateEventW;

use super::buffer::{TrozoAudio, FRECUENCIA_AUDIO};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origen {
    SalidaCompleta,
    /// `incluir`: solo ese proceso (y sus hijos); si no, todo menos ese proceso.
    Proceso { pid: u32, incluir: bool },
    Microfono,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Formato {
    F32,
    I16,
    I32,
}

pub struct CapturaAudio {
    cliente: IAudioClient,
    captura: IAudioCaptureClient,
    /// Dispositivo usado (para notar si la persona cambió el predeterminado). Vacío en la captura por proceso.
    id: String,
    canales: usize,
    formato: Formato,
    remuestreo: Remuestreo,
    /// Posición (muestras de 48 kHz) donde tiene que empezar el próximo trozo para quedar pegado al anterior.
    siguiente: Option<i64>,
    evento: Option<HANDLE>,
}

/// Si el reloj y la cuenta de muestras difieren en más que esto (20 ms), hubo un hueco de verdad
/// (Windows deja de mandar audio cuando no suena nada) o el reloj del dispositivo se corrió: se
/// vuelve a ubicar por el reloj. Por debajo, la diferencia es solo el temblor de las marcas de tiempo.
const TOLERANCIA: i64 = FRECUENCIA_AUDIO as i64 / 50;

fn muestras_desde_reloj(t: i64) -> i64 {
    (t as i128 * FRECUENCIA_AUDIO as i128 / 10_000_000) as i64
}

unsafe fn predeterminado(en: &IMMDeviceEnumerator, flujo: EDataFlow) -> Result<(IMMDevice, String)> {
    let disp = en.GetDefaultAudioEndpoint(flujo, eConsole)?;
    let p = disp.GetId()?;
    let id = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    Ok((disp, id))
}

/// El id del dispositivo predeterminado ahora (para notar si la persona cambió de auriculares o de micrófono).
pub fn dispositivo_predeterminado(origen: Origen) -> Option<String> {
    let flujo = match origen {
        Origen::SalidaCompleta => eRender,
        Origen::Microfono => eCapture,
        Origen::Proceso { .. } => return None,
    };
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        predeterminado(&en, flujo).ok().map(|(_, id)| id)
    }
}

/// Avisa cuando Windows termina de activar la captura por proceso.
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Activado(mpsc::SyncSender<()>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Activado_Impl {
    fn ActivateCompleted(&self, _operacion: Option<&IActivateAudioInterfaceAsyncOperation>) -> windows::core::Result<()> {
        let _ = self.0.send(());
        Ok(())
    }
}

/// PROPVARIANT de tipo VT_BLOB, armado a mano (es lo que pide la activación por proceso).
#[repr(C)]
struct VarianteBlob {
    tipo: u16,
    reservado: [u16; 3],
    tamano: u32,
    datos: *const u8,
}

const VT_BLOB: u16 = 65;

/// Cliente de audio que captura un solo proceso (o todo menos él).
unsafe fn cliente_de_proceso(pid: u32, incluir: bool) -> Result<IAudioClient> {
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: if incluir {
                    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE
                } else {
                    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE
                },
            },
        },
    };
    let blob = VarianteBlob {
        tipo: VT_BLOB,
        reservado: [0; 3],
        tamano: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
        datos: &params as *const _ as *const u8,
    };
    let (tx, rx) = mpsc::sync_channel(1);
    let aviso: IActivateAudioInterfaceCompletionHandler = Activado(tx).into();
    let operacion = ActivateAudioInterfaceAsync(
        VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
        &IAudioClient::IID,
        Some(&blob as *const VarianteBlob as *const PROPVARIANT),
        &aviso,
    )
    .context("Windows no tiene captura de audio por proceso")?;
    // Se espera poco: si Windows no contesta, se sigue sin este audio y se reintenta más tarde.
    rx.recv_timeout(Duration::from_secs(2)).map_err(|_| anyhow!("la captura de audio por proceso no respondió"))?;
    let mut resultado = HRESULT(0);
    let mut interfaz: Option<IUnknown> = None;
    operacion.GetActivateResult(&mut resultado, &mut interfaz)?;
    resultado.ok()?;
    Ok(interfaz.ok_or_else(|| anyhow!("sin cliente de audio"))?.cast()?)
}

/// Lee el formato de mezcla de un dispositivo: (formato, canales, frecuencia).
unsafe fn leer_formato(fmt: *const WAVEFORMATEX) -> Result<(Formato, usize, u32)> {
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
        _ => bail!("formato de audio no soportado ({tipo}, {bits} bits)"),
    };
    if canales == 0 {
        bail!("el dispositivo de audio no tiene canales");
    }
    Ok((formato, canales, frecuencia))
}

impl CapturaAudio {
    pub fn abrir(origen: Origen) -> Result<Self> {
        unsafe {
            let (cliente, id, formato, canales, frecuencia, evento) = match origen {
                Origen::SalidaCompleta | Origen::Microfono => {
                    let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                    let flujo = if origen == Origen::Microfono { eCapture } else { eRender };
                    let (disp, id) = predeterminado(&en, flujo).with_context(|| {
                        if origen == Origen::Microfono {
                            "no hay micrófono".to_string()
                        } else {
                            "no hay salida de audio".to_string()
                        }
                    })?;
                    let cliente: IAudioClient = disp.Activate(CLSCTX_ALL, None)?;
                    let fmt = cliente.GetMixFormat()?;
                    let leido = leer_formato(fmt);
                    let flags = if origen == Origen::Microfono { 0 } else { AUDCLNT_STREAMFLAGS_LOOPBACK };
                    let iniciado = match &leido {
                        Ok(_) => cliente.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, fmt, None),
                        Err(_) => Ok(()),
                    };
                    CoTaskMemFree(Some(fmt as *const _));
                    let (formato, canales, frecuencia) = leido?;
                    iniciado?;
                    (cliente, id, formato, canales, frecuencia, None)
                }
                Origen::Proceso { pid, incluir } => {
                    let cliente = cliente_de_proceso(pid, incluir)?;
                    // La captura por proceso no tiene formato propio: se le pide justo el que usa Kairo.
                    let fmt = WAVEFORMATEX {
                        wFormatTag: 1,
                        nChannels: 2,
                        nSamplesPerSec: FRECUENCIA_AUDIO,
                        nAvgBytesPerSec: FRECUENCIA_AUDIO * 4,
                        nBlockAlign: 4,
                        wBitsPerSample: 16,
                        cbSize: 0,
                    };
                    let flags = AUDCLNT_STREAMFLAGS_LOOPBACK
                        | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
                    cliente.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, &fmt, None)?;
                    // Pide un evento aunque se lea cada 10 ms igual que los otros orígenes.
                    let evento = CreateEventW(None, false, false, None)?;
                    cliente.SetEventHandle(evento)?;
                    (cliente, String::new(), Formato::I16, 2, FRECUENCIA_AUDIO, Some(evento))
                }
            };
            let captura: IAudioCaptureClient = cliente.GetService()?;
            cliente.Start()?;
            Ok(Self { cliente, captura, id, canales, formato, remuestreo: Remuestreo::new(frecuencia), siguiente: None, evento })
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
                let largo = (muestras.len() / 2) as i64;
                // Dónde va este trozo: pegado al anterior, salvo que el reloj diga que hubo un hueco real.
                // Si Windows no da la marca de tiempo (pasa en la captura por proceso), se estima con el
                // reloj de ahora: el trozo acaba de terminar.
                let error_de_tiempo = qpc == 0 || flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0;
                let por_reloj =
                    if error_de_tiempo { muestras_desde_reloj(super::reloj::ahora()) - largo } else { muestras_desde_reloj(qpc as i64) };
                let posicion = match self.siguiente {
                    Some(s) if (por_reloj - s).abs() <= TOLERANCIA => s,
                    // Sin marca de tiempo, el reloj de ahora llega tarde si los trozos vienen en tanda:
                    // solo se reubica si hubo un hueco claro (adelante), nunca hacia atrás.
                    Some(s) if error_de_tiempo && por_reloj < s => s,
                    _ => por_reloj,
                };
                self.siguiente = Some(posicion + largo);
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
            if let Some(e) = self.evento {
                let _ = CloseHandle(e);
            }
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
