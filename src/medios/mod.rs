//! Reproducir, editar y comprimir clips. Todo esto corre solo en la ventana de Kairo: la grabadora (el
//! proceso de la bandeja) no lo usa, así que mientras juegas con la ventana cerrada no pesa nada.

pub mod comprimir;
pub mod exportar;
pub mod reproductor;

use std::path::Path;

use anyhow::{anyhow, Result};
use windows::core::{GUID, HSTRING, PROPVARIANT};
use windows::Win32::Media::MediaFoundation::*;

/// Todo el audio de Kairo es de 48 kHz, estéreo.
pub const FRECUENCIA: u32 = 48_000;
/// 100 ns por segundo (la unidad de tiempo de Media Foundation).
pub const SEGUNDO: i64 = 10_000_000;

/// Prepara COM y Media Foundation en el hilo que llama (se puede llamar más de una vez).
pub fn iniciar() {
    unsafe {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
        let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    }
}

/// Abre un lector del archivo. `procesar_video`: deja que el lector convierta y achique el video.
pub unsafe fn lector(ruta: &Path, procesar_video: bool) -> Result<IMFSourceReader> {
    let mut atributos = None;
    MFCreateAttributes(&mut atributos, 2)?;
    let atributos = atributos.ok_or_else(|| anyhow!("sin atributos"))?;
    if procesar_video {
        atributos.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
    }
    let l = MFCreateSourceReaderFromURL(&HSTRING::from(ruta.as_os_str()), &atributos)?;
    let _ = l.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
    Ok(l)
}

/// Duración en segundos.
pub unsafe fn duracion(l: &IMFSourceReader) -> f64 {
    l.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
        .ok()
        .and_then(|p| u64::try_from(&p).ok())
        .unwrap_or(0) as f64
        / 1e7
}

/// El flujo de video (si hay) y los de audio, en el orden del archivo (el primero es la mezcla).
pub unsafe fn flujos(l: &IMFSourceReader) -> (Option<u32>, Vec<u32>) {
    let (mut video, mut audio) = (None, Vec::new());
    let mut i = 0;
    while let Ok(t) = l.GetNativeMediaType(i, 0) {
        match t.GetGUID(&MF_MT_MAJOR_TYPE) {
            Ok(g) if g == MFMediaType_Video && video.is_none() => video = Some(i),
            Ok(g) if g == MFMediaType_Audio => audio.push(i),
            _ => {}
        }
        i += 1;
    }
    (video, audio)
}

/// Va a `t` (100 ns). Con video comprimido, el lector cae en el cuadro clave anterior.
pub unsafe fn ir_a(l: &IMFSourceReader, t: i64) -> Result<()> {
    l.SetCurrentPosition(&GUID::zeroed(), &PROPVARIANT::from(t.max(0)))?;
    Ok(())
}

/// Pide el audio de `flujo` decodificado: PCM de 16 bits, 48 kHz, estéreo.
pub unsafe fn pedir_pcm(l: &IMFSourceReader, flujo: u32) -> Result<()> {
    let t = MFCreateMediaType()?;
    t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
    t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
    t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, FRECUENCIA)?;
    t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
    t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
    t.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
    t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, FRECUENCIA * 4)?;
    l.SetStreamSelection(flujo, true)?;
    l.SetCurrentMediaType(flujo, None, &t)?;
    Ok(())
}

/// Los bytes de una muestra (copiados).
pub unsafe fn bytes_de(m: &IMFSample) -> Result<Vec<u8>> {
    let b = m.ConvertToContiguousBuffer()?;
    let mut p = std::ptr::null_mut();
    let mut largo = 0u32;
    b.Lock(&mut p, None, Some(&mut largo))?;
    let v = std::slice::from_raw_parts(p, largo as usize).to_vec();
    let _ = b.Unlock();
    Ok(v)
}

/// Lo que hace falta saber de un clip para mostrarlo.
#[derive(Clone, Debug, Default)]
pub struct InfoClip {
    pub duracion: f64,
    pub ancho: u32,
    pub alto: u32,
    pub fps: f64,
    pub pistas_audio: usize,
    pub bitrate_video: u32,
}

pub fn info(ruta: &Path) -> Result<InfoClip> {
    iniciar();
    unsafe {
        let l = lector(ruta, false)?;
        let (video, audio) = flujos(&l);
        let mut i = InfoClip { duracion: duracion(&l), pistas_audio: audio.len(), ..Default::default() };
        if let Some(v) = video {
            let t = l.GetNativeMediaType(v, 0)?;
            let s = t.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
            let r = t.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(0);
            i.ancho = (s >> 32) as u32;
            i.alto = s as u32;
            i.fps = (r >> 32) as f64 / ((r & 0xFFFF_FFFF) as f64).max(1.0);
            i.bitrate_video = t.GetUINT32(&MF_MT_AVG_BITRATE).unwrap_or(0);
        }
        Ok(i)
    }
}

/// Un nombre que no pisa a ningún archivo: "clip (editado).mp4", "clip (editado 2).mp4"…
pub fn ruta_libre(original: &Path, sufijo: &str) -> std::path::PathBuf {
    let dir = original.parent().unwrap_or(Path::new("."));
    let base = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "clip".into());
    for n in 1.. {
        let nombre = if n == 1 { format!("{base} ({sufijo}).mp4") } else { format!("{base} ({sufijo} {n}).mp4") };
        let r = dir.join(nombre);
        if !r.exists() {
            return r;
        }
    }
    unreachable!()
}
