//! Escribe un clip como MP4 con el "sink writer" de Media Foundation. El video ya viene comprimido
//! (se copia tal cual, sin recodificar); el audio se comprime a AAC al escribir.
//!
//! Audio: la primera pista es la mezcla de las fuentes activas (juego, resto del sistema, micrófono)
//! con su volumen: es la que escuchan los reproductores. Con "pistas separadas", además va cada fuente
//! en su propia pista, para editar.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::*;

use super::buffer::{Clip, FRECUENCIA_AUDIO};
use super::reloj::SEGUNDO;

/// Cuadros de audio por muestra escrita (100 ms).
const TROZO_AUDIO: usize = FRECUENCIA_AUDIO as usize / 10;

/// Qué audio lleva el clip.
pub struct AudioDelClip {
    /// Volumen de cada pista del buffer (mismo orden); None = esa pista no va.
    pub volumenes: Vec<Option<f32>>,
    /// Nombre de cada pista del buffer ("Juego", "Discord"…).
    pub nombres: Vec<String>,
    pub separadas: bool,
    /// AAC: 12 000 (96 kbps), 16 000, 20 000 o 24 000 (192 kbps) bytes por segundo.
    pub bytes_por_segundo: u32,
}

fn tamano(a: u32, b: u32) -> u64 {
    ((a as u64) << 32) | b as u64
}

/// Audio continuo de una pista en todo el clip: cada trozo va donde dice su posición y los huecos
/// quedan en silencio. Los trozos se suman: una pista puede juntar varios procesos del mismo programa.
/// None si esa pista no tiene nada.
fn audio_continuo(clip: &Clip, pista: usize, nombre: &str) -> Option<Vec<i16>> {
    let trozos = clip.audio.get(pista)?;
    if trozos.is_empty() {
        return None;
    }
    let cuadros = ((clip.fin - clip.inicio) * FRECUENCIA_AUDIO as i64 / SEGUNDO).max(0) as usize;
    let mut suma = vec![0i32; cuadros * 2];
    // Diagnóstico: cuántos trozos no empiezan justo donde terminó el anterior (huecos o solapes).
    let (mut cortes, mut mayor, mut fin_anterior) = (0usize, 0i64, None::<i64>);
    // Todo en muestras enteras: el inicio del clip se convierte una sola vez, así los trozos contiguos
    // quedan contiguos también en el archivo.
    let base = (clip.inicio as i128 * FRECUENCIA_AUDIO as i128 / SEGUNDO as i128) as i64;
    for t in trozos {
        let desde = t.posicion - base;
        if let Some(f) = fin_anterior {
            if desde != f {
                cortes += 1;
                mayor = mayor.max((desde - f).abs());
            }
        }
        fin_anterior = Some(desde + (t.muestras.len() / 2) as i64);
        let salta = (-desde).max(0) as usize;
        let inicio = desde.max(0) as usize;
        for (k, par) in t.muestras.chunks_exact(2).skip(salta).enumerate() {
            let i = inicio + k;
            if i >= cuadros {
                break;
            }
            suma[i * 2] += par[0] as i32;
            suma[i * 2 + 1] += par[1] as i32;
        }
    }
    crate::registro::log(&format!(
        "audio del clip ({nombre}): {} trozos, {cortes} no son continuos (el mayor, {mayor} muestras)",
        trozos.len()
    ));
    Some(suma.into_iter().map(|x| x.clamp(-32768, 32767) as i16).collect())
}

/// Suma las fuentes con su volumen (recortando para no pasarse).
fn mezclar(fuentes: &[(String, Vec<i16>, f32)]) -> Vec<i16> {
    let largo = fuentes.iter().map(|(_, p, _)| p.len()).max().unwrap_or(0);
    let mut suma = vec![0f32; largo];
    for (_, pcm, vol) in fuentes {
        for (s, &x) in suma.iter_mut().zip(pcm) {
            *s += x as f32 * vol;
        }
    }
    suma.into_iter().map(|s| s.clamp(-32768.0, 32767.0) as i16).collect()
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

/// Una pista de audio del archivo y cuánto se escribió de ella.
struct PistaDeAudio {
    flujo: u32,
    pcm: Vec<i16>,
    escrito: usize,
}

impl PistaDeAudio {
    /// Escribe hasta el instante `t` (100 ns desde el inicio del clip).
    unsafe fn escribir_hasta(&mut self, escritor: &IMFSinkWriter, t: i64) -> Result<()> {
        let total = self.pcm.len() / 2;
        let hasta = ((t.max(0) as i128 * FRECUENCIA_AUDIO as i128 / SEGUNDO as i128) as usize).min(total);
        while self.escrito < hasta {
            let n = TROZO_AUDIO.min(total - self.escrito);
            let trozo = &self.pcm[self.escrito * 2..(self.escrito + n) * 2];
            let bytes = std::slice::from_raw_parts(trozo.as_ptr() as *const u8, trozo.len() * 2);
            let tiempo = self.escrito as i64 * SEGUNDO / FRECUENCIA_AUDIO as i64;
            let m = muestra_de(bytes, tiempo, n as i64 * SEGUNDO / FRECUENCIA_AUDIO as i64)?;
            escritor.WriteSample(self.flujo, &m)?;
            self.escrito += n;
        }
        Ok(())
    }
}

unsafe fn agregar_pista_aac(escritor: &IMFSinkWriter, bytes_por_segundo: u32) -> Result<u32> {
    let aac = MFCreateMediaType()?;
    aac.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
    aac.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
    aac.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, FRECUENCIA_AUDIO)?;
    aac.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
    aac.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
    // 24000 bytes/s = 192 kbps (el encoder AAC de Windows acepta 12000, 16000, 20000 y 24000).
    aac.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, bytes_por_segundo)?;
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
    Ok(flujo)
}

/// Escribe el clip en `ruta` con el audio pedido. Devuelve los nombres de las pistas de audio en el
/// orden del archivo ("Mezcla", "Juego", "Discord"…).
pub fn escribir(ruta: &Path, clip: &Clip, audio: &AudioDelClip) -> Result<Vec<String>> {
    let primero = clip.video.first().ok_or_else(|| anyhow!("el clip está vacío"))?;
    let encabezado = super::h264::encabezado(&primero.datos).ok_or_else(|| anyhow!("el primer cuadro no trae SPS/PPS"))?;
    let f = &clip.formato;

    // Las fuentes que van (activas y con algo grabado), la mezcla y, si se piden, las pistas separadas.
    let fuentes: Vec<(String, Vec<i16>, f32)> = (0..clip.audio.len())
        .filter_map(|p| {
            let vol = audio.volumenes.get(p).copied().flatten()?;
            let nombre = audio.nombres.get(p).cloned().unwrap_or_else(|| format!("Pista {}", p + 1));
            Some((nombre.clone(), audio_continuo(clip, p, &nombre)?, vol))
        })
        .collect();
    let mut pcm_pistas = Vec::new();
    let mut nombres = Vec::new();
    if !fuentes.is_empty() {
        pcm_pistas.push(mezclar(&fuentes));
        nombres.push("Mezcla".to_string());
        if audio.separadas && fuentes.len() > 1 {
            for fuente in &fuentes {
                pcm_pistas.push(mezclar(std::slice::from_ref(fuente)));
                nombres.push(fuente.0.clone());
            }
        }
    }

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

        // El MP4 queda con las pistas en el orden inverso al que se agregan (comprobado). Los reproductores
        // tocan la primera pista de audio, que tiene que ser la mezcla: por eso se agregan al revés.
        let mut pistas = Vec::with_capacity(pcm_pistas.len());
        for pcm in pcm_pistas.into_iter().rev() {
            pistas.push(PistaDeAudio { flujo: agregar_pista_aac(&escritor, audio.bytes_por_segundo)?, pcm, escrito: 0 });
        }

        escritor.BeginWriting()?;
        for p in &clip.video {
            let t = p.pts - clip.inicio;
            for pista in &mut pistas {
                pista.escribir_hasta(&escritor, t)?;
            }
            let m = muestra_de(&p.datos, t, p.dur)?;
            if p.clave {
                m.SetUINT32(&MFSampleExtension_CleanPoint, 1)?;
            }
            escritor.WriteSample(flujo_video, &m)?;
        }
        for pista in &mut pistas {
            pista.escribir_hasta(&escritor, clip.fin - clip.inicio)?;
        }
        escritor.Finalize().context("no se pudo terminar el archivo")?;
    }
    Ok(nombres)
}
