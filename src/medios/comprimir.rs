//! Comprimir un clip para compartirlo: se vuelve a codificar más chico (resolución y bitrate) con el
//! encoder de la tarjeta de video, o con el de Windows si la tarjeta no puede. Corre con prioridad baja,
//! solo cuando lo pides, y deja el original como estaba. El clip comprimido lleva solo la mezcla de audio.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, bail, Context, Result};
use windows::core::{Interface, VARIANT};
use windows::Win32::Media::MediaFoundation::*;

use super::exportar::{agregar_aac, escritor_mp4};
use super::{InfoClip, SEGUNDO};

const AUDIO_KBPS: u32 = 128;
/// Discord deja subir hasta 10 MB sin Nitro: se apunta un poco más abajo.
const LIMITE_DISCORD: f64 = 9_500_000.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Opcion {
    /// 1080p, 10 Mbps.
    Compartir,
    /// 720p, 5 Mbps.
    Liviano,
    /// Menos de 10 MB.
    Discord,
}

impl Opcion {
    pub const TODAS: [Opcion; 3] = [Opcion::Compartir, Opcion::Liviano, Opcion::Discord];

    pub fn nombre(self) -> &'static str {
        match self {
            Opcion::Compartir => "Buena calidad",
            Opcion::Liviano => "Liviano",
            Opcion::Discord => "Para Discord",
        }
    }

    pub fn descripcion(self) -> &'static str {
        match self {
            Opcion::Compartir => "1080p a 10 Mbps: se ve casi igual y pesa bastante menos.",
            Opcion::Liviano => "720p a 5 Mbps: para mandar por WhatsApp o subir rápido.",
            Opcion::Discord => "Menos de 10 MB, lo que deja subir Discord sin Nitro.",
        }
    }
}

/// Qué va a hacer cada opción con este clip: (alto, bitrate del video en bps).
pub fn plan(opcion: Opcion, info: &InfoClip) -> Result<(u32, u32)> {
    let (alto, bitrate) = match opcion {
        Opcion::Compartir => (1080, 10_000_000),
        Opcion::Liviano => (720, 5_000_000),
        Opcion::Discord => {
            let total = LIMITE_DISCORD * 8.0 / info.duracion.max(0.1);
            let video = total - (AUDIO_KBPS * 1000) as f64 * 1.05;
            if video < 300_000.0 {
                bail!("el clip es muy largo para pesar menos de 10 MB: recórtalo primero (en Editar)");
            }
            let alto = if video >= 2_500_000.0 { 720 } else { 480 };
            (alto, video.min(6_000_000.0) as u32)
        }
    };
    // Nunca más grande que el original, ni con más bitrate (si ya es liviano, igual baja un poco).
    let original = original_bps(info);
    let bitrate = if original > 0 { bitrate.min(original / 4 * 3) } else { bitrate };
    Ok((alto.min(info.alto.max(2)), bitrate.max(250_000)))
}

/// El bitrate del video original (bps).
fn original_bps(info: &InfoClip) -> u32 {
    info.bitrate_video
}

/// ¿Esta opción casi no le cambiaría el peso al clip? (Para avisar.)
pub fn ya_es_liviano(opcion: Opcion, info: &InfoClip) -> bool {
    let original = original_bps(info);
    match plan(opcion, info) {
        Ok((alto, b)) => original > 0 && alto >= info.alto && b >= original / 4 * 3,
        Err(_) => false,
    }
}

/// Cuánto va a pesar, más o menos (bytes).
pub fn tamano_estimado(info: &InfoClip, bitrate: u32) -> u64 {
    ((bitrate + AUDIO_KBPS * 1000) as f64 / 8.0 * info.duracion) as u64
}

pub struct Pedido {
    pub origen: PathBuf,
    pub destino: PathBuf,
    pub alto: u32,
    pub bitrate: u32,
}

pub fn comprimir(p: &Pedido, progreso: &dyn Fn(f32), cancelar: &AtomicBool) -> Result<()> {
    super::iniciar();
    // Prioridad baja (procesador y disco): si estás jugando, el juego va primero.
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
    let tmp = p.destino.with_extension("mp4.tmp");
    // Primero con la tarjeta de video; si no se puede, con el encoder de Windows.
    let mut r = unsafe { comprimir_en(p, &tmp, true, progreso, cancelar) };
    if r.is_err() && !cancelar.load(Ordering::SeqCst) {
        crate::registro::log(&format!("comprimir con la tarjeta falló ({:#}): se usa el procesador", r.as_ref().err().unwrap()));
        let _ = std::fs::remove_file(&tmp);
        r = unsafe { comprimir_en(p, &tmp, false, progreso, cancelar) };
    }
    match r {
        Ok(()) => {
            std::fs::rename(&tmp, &p.destino).context("no se pudo guardar el clip")?;
            let original = crate::datos_clip::leer(&p.origen).unwrap_or_default();
            let info = super::info(&p.destino).unwrap_or_default();
            let datos = crate::datos_clip::DatosClip {
                version: original.version,
                juego: original.juego,
                pistas: vec!["Mezcla".into()],
                duracion: info.duracion,
                ancho: info.ancho,
                alto: info.alto,
                fps: info.fps.round() as u32,
                creado: crate::registro::fecha_para_archivo(),
                mezcla_marcada: true,
                favorito: false,
            };
            crate::datos_clip::escribir(&p.destino, &datos);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

unsafe fn comprimir_en(p: &Pedido, tmp: &std::path::Path, hardware: bool, progreso: &dyn Fn(f32), cancelar: &AtomicBool) -> Result<()> {
    let l = super::lector(&p.origen, true)?;
    let duracion = super::duracion(&l).max(0.1);
    let (video, audio) = super::flujos(&l);
    let video = video.ok_or_else(|| anyhow!("el clip no tiene video"))?;
    l.SetStreamSelection(video, true)?;

    // El lector decodifica y achica a NV12 (lo que come el encoder).
    let nativo = l.GetNativeMediaType(video, 0)?;
    let tam = nativo.GetUINT64(&MF_MT_FRAME_SIZE)?;
    let (w, h) = ((tam >> 32) as f64, (tam & 0xFFFF_FFFF) as f64);
    let escala = (p.alto as f64 / h).min(1.0);
    let par = |x: f64| ((x / 2.0).round() as u64 * 2).max(2);
    let (w2, h2) = (par(w * escala), par(h * escala));
    let nv12 = MFCreateMediaType()?;
    nv12.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
    nv12.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
    nv12.SetUINT64(&MF_MT_FRAME_SIZE, (w2 << 32) | h2)?;
    l.SetCurrentMediaType(video, None, &nv12).context("no se pudo achicar el video")?;
    let entrada = l.GetCurrentMediaType(video)?;
    let cuadros = nativo.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or((60u64 << 32) | 1);

    let con_audio = !audio.is_empty();
    if con_audio {
        super::pedir_pcm(&l, audio[0])?;
    }

    let escritor = escritor_mp4(tmp, hardware)?;
    let salida = MFCreateMediaType()?;
    salida.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
    salida.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
    salida.SetUINT32(&MF_MT_AVG_BITRATE, p.bitrate)?;
    salida.SetUINT64(&MF_MT_FRAME_SIZE, (w2 << 32) | h2)?;
    salida.SetUINT64(&MF_MT_FRAME_RATE, cuadros)?;
    salida.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
    salida.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
    salida.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
    let flujo_video = escritor.AddStream(&salida)?;
    escritor.SetInputMediaType(flujo_video, &entrada, None).context("el encoder no acepta el video")?;
    // Bitrate variable con tope: el tamaño final queda cerca del estimado.
    let mut api: Option<ICodecAPI> = None;
    if escritor.GetServiceForStream(flujo_video, &windows::core::GUID::zeroed(), &ICodecAPI::IID, &mut api as *mut _ as *mut _).is_ok() {
        if let Some(api) = &api {
            if api.SetValue(&CODECAPI_AVEncCommonRateControlMode, &VARIANT::from(eAVEncCommonRateControlMode_PeakConstrainedVBR.0 as u32)).is_ok() {
                let _ = api.SetValue(&CODECAPI_AVEncCommonMaxBitRate, &VARIANT::from(p.bitrate / 5 * 6));
            }
            let _ = api.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &VARIANT::from(p.bitrate));
        }
    }
    let flujo_audio = if con_audio { Some(agregar_aac(&escritor, AUDIO_KBPS)?) } else { None };
    escritor.BeginWriting()?;

    let mut terminados = 0;
    let esperados = 1 + con_audio as usize;
    while terminados < esperados {
        if cancelar.load(Ordering::SeqCst) {
            bail!("cancelado");
        }
        let (mut indice, mut flags, mut ts, mut m) = (0u32, 0u32, 0i64, None);
        l.ReadSample(MF_SOURCE_READER_ANY_STREAM.0 as u32, 0, Some(&mut indice), Some(&mut flags), Some(&mut ts), Some(&mut m))?;
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            terminados += 1;
            continue;
        }
        let Some(m) = m else { continue };
        if indice == video {
            escritor.WriteSample(flujo_video, &m)?;
            progreso((ts as f64 / SEGUNDO as f64 / duracion).clamp(0.0, 1.0) as f32);
        } else if let Some(fa) = flujo_audio {
            escritor.WriteSample(fa, &m)?;
        }
    }
    escritor.Finalize().context("no se pudo terminar el archivo")?;
    progreso(1.0);
    Ok(())
}
