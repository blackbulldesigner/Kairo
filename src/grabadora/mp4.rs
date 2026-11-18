//! Escribe un clip como MP4 con el "sink writer" de Media Foundation. El video ya viene comprimido
//! (se copia tal cual, sin recodificar); el audio se comprime a AAC al escribir.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::*;

use super::buffer::{Clip, FRECUENCIA_AUDIO};
use super::reloj::SEGUNDO;

/// Cuadros de audio por muestra escrita (100 ms).
const TROZO_AUDIO: usize = FRECUENCIA_AUDIO as usize / 10;

fn tamano(a: u32, b: u32) -> u64 {
    ((a as u64) << 32) | b as u64
}

/// Unidades NAL de un flujo Annex B (sin los códigos de inicio).
fn unidades_nal(datos: &[u8]) -> Vec<&[u8]> {
    let mut inicios = Vec::new();
    let mut i = 0;
    while i + 3 <= datos.len() {
        if datos[i] == 0 && datos[i + 1] == 0 && datos[i + 2] == 1 {
            inicios.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut nal = Vec::new();
    for (k, &ini) in inicios.iter().enumerate() {
        let mut fin = inicios.get(k + 1).map(|&s| s - 3).unwrap_or(datos.len());
        // El código de inicio de 4 bytes deja un cero de más al final de la unidad anterior.
        while fin > ini && datos[fin - 1] == 0 && k + 1 < inicios.len() {
            fin -= 1;
        }
        if fin > ini {
            nal.push(&datos[ini..fin]);
        }
    }
    nal
}

/// SPS y PPS del primer cuadro clave, en Annex B: el encabezado que necesita el MP4. Se toman del
/// video mismo para que siempre coincidan con lo que el encoder escribió.
fn encabezado_h264(datos: &[u8]) -> Option<Vec<u8>> {
    let nal = unidades_nal(datos);
    let sps = nal.iter().find(|n| n[0] & 0x1F == 7)?;
    let pps = nal.iter().find(|n| n[0] & 0x1F == 8)?;
    let mut h = vec![0, 0, 0, 1];
    h.extend_from_slice(sps);
    h.extend_from_slice(&[0, 0, 0, 1]);
    h.extend_from_slice(pps);
    Some(h)
}

/// Audio continuo de todo el clip: cada trozo va donde dice su tiempo y los huecos quedan en silencio.
fn audio_continuo(clip: &Clip, volumen: f32) -> Vec<i16> {
    let cuadros = ((clip.fin - clip.inicio) * FRECUENCIA_AUDIO as i64 / SEGUNDO).max(0) as usize;
    let mut pcm = vec![0i16; cuadros * 2];
    for t in &clip.audio {
        let desde = ((t.pts - clip.inicio) as i128 * FRECUENCIA_AUDIO as i128 / SEGUNDO as i128) as i64;
        for (k, par) in t.muestras.chunks_exact(2).enumerate() {
            let i = desde + k as i64;
            if i < 0 {
                continue;
            }
            let i = i as usize;
            if i >= cuadros {
                break;
            }
            pcm[i * 2] = (par[0] as f32 * volumen).clamp(-32768.0, 32767.0) as i16;
            pcm[i * 2 + 1] = (par[1] as f32 * volumen).clamp(-32768.0, 32767.0) as i16;
        }
    }
    pcm
}

unsafe fn muestra_de(bytes: &[u8], tiempo: i64, duracion: i64) -> Result<IMFSample> {
    let buffer = MFCreateMemoryBuffer(bytes.len() as u32)?;
    let mut p = std::ptr::null_mut();
    buffer.Lock(&mut p, None, None)?;
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    buffer.Unlock()?;
    buffer.SetCurrentLength(bytes.len() as u32)?;
    let m = MFCreateSample()?;
    m.AddBuffer(&buffer)?;
    m.SetSampleTime(tiempo)?;
    m.SetSampleDuration(duracion)?;
    Ok(m)
}

/// Escribe el clip en `ruta`. `volumen` multiplica el audio (None = sin pista de audio).
pub fn escribir(ruta: &Path, clip: &Clip, volumen: Option<f32>) -> Result<()> {
    let primero = clip.video.first().ok_or_else(|| anyhow!("el clip está vacío"))?;
    let encabezado = encabezado_h264(&primero.datos).ok_or_else(|| anyhow!("el primer cuadro no trae SPS/PPS"))?;
    let f = &clip.formato;
    unsafe {
        let mut atributos = None;
        MFCreateAttributes(&mut atributos, 4)?;
        let atributos = atributos.ok_or_else(|| anyhow!("sin atributos"))?;
        atributos.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
        // Se escribe a un archivo temporal: el contenedor se elige acá y no por la extensión.
        atributos.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
        let escritor = MFCreateSinkWriterFromURL(&HSTRING::from(ruta.as_os_str()), None, &atributos).context("no se pudo crear el archivo")?;

        let tipo_video = MFCreateMediaType()?;
        tipo_video.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        tipo_video.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        tipo_video.SetUINT64(&MF_MT_FRAME_SIZE, tamano(f.ancho, f.alto))?;
        tipo_video.SetUINT64(&MF_MT_FRAME_RATE, tamano(f.fps, 1))?;
        tipo_video.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, tamano(1, 1))?;
        tipo_video.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        tipo_video.SetUINT32(&MF_MT_AVG_BITRATE, f.bitrate)?;
        tipo_video.SetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &encabezado)?;
        let flujo_video = escritor.AddStream(&tipo_video)?;
        escritor.SetInputMediaType(flujo_video, &tipo_video, None)?;

        let flujo_audio = match volumen {
            None => None,
            Some(_) => {
                let aac = MFCreateMediaType()?;
                aac.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                aac.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
                aac.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, FRECUENCIA_AUDIO)?;
                aac.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
                aac.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                // 24000 bytes/s = 192 kbps.
                aac.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 24_000)?;
                let flujo = escritor.AddStream(&aac)?;
                let pcm = MFCreateMediaType()?;
                pcm.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                pcm.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
                pcm.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, FRECUENCIA_AUDIO)?;
                pcm.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
                pcm.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                pcm.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
                pcm.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, FRECUENCIA_AUDIO * 4)?;
                escritor.SetInputMediaType(flujo, &pcm, None).context("Windows no tiene el encoder AAC")?;
                Some(flujo)
            }
        };

        escritor.BeginWriting()?;
        let pcm = volumen.map(|v| audio_continuo(clip, v)).unwrap_or_default();
        let mut audio_escrito = 0usize; // en cuadros
        let total_audio = pcm.len() / 2;
        let mut escribir_audio_hasta = |escritor: &IMFSinkWriter, t: i64| -> Result<()> {
            let Some(flujo) = flujo_audio else { return Ok(()) };
            let hasta = ((t.max(0) as i128 * FRECUENCIA_AUDIO as i128 / SEGUNDO as i128) as usize).min(total_audio);
            while audio_escrito < hasta {
                let n = TROZO_AUDIO.min(total_audio - audio_escrito);
                let trozo = &pcm[audio_escrito * 2..(audio_escrito + n) * 2];
                let bytes = std::slice::from_raw_parts(trozo.as_ptr() as *const u8, trozo.len() * 2);
                let tiempo = audio_escrito as i64 * SEGUNDO / FRECUENCIA_AUDIO as i64;
                let m = muestra_de(bytes, tiempo, n as i64 * SEGUNDO / FRECUENCIA_AUDIO as i64)?;
                escritor.WriteSample(flujo, &m)?;
                audio_escrito += n;
            }
            Ok(())
        };

        for p in &clip.video {
            let t = p.pts - clip.inicio;
            escribir_audio_hasta(&escritor, t)?;
            let m = muestra_de(&p.datos, t, p.dur)?;
            if p.clave {
                m.SetUINT32(&MFSampleExtension_CleanPoint, 1)?;
            }
            escritor.WriteSample(flujo_video, &m)?;
        }
        escribir_audio_hasta(&escritor, clip.fin - clip.inicio)?;
        escritor.Finalize().context("no se pudo terminar el archivo")?;
    }
    Ok(())
}
