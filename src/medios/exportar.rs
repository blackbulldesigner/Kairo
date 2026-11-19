//! Guarda un clip editado como clip nuevo: recortado y con el volumen de cada pista. El video se copia
//! tal cual (sin recodificar: es instantáneo y no pierde calidad), así que el recorte empieza en el cuadro
//! clave anterior al inicio elegido. El audio se vuelve a armar desde cada pista.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, bail, Context, Result};
use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::*;

use super::{bytes_de, ir_a, FRECUENCIA, SEGUNDO};

/// 100 ms de audio por muestra escrita.
const TROZO: usize = FRECUENCIA as usize / 10;

pub struct Pedido {
    pub origen: PathBuf,
    pub destino: PathBuf,
    /// Segundos.
    pub inicio: f64,
    pub fin: f64,
    /// Volumen de cada pista de audio del archivo, en su orden (0 = fuera). Si el clip tiene pistas
    /// separadas, la mezcla nueva se arma con ellas (la mezcla original no se usa).
    pub volumenes: Vec<f32>,
    /// Nombre de cada pista de audio del archivo ("Mezcla", "Juego", "Discord"…).
    pub nombres: Vec<String>,
}

pub struct Resultado {
    /// Dónde empieza de verdad el clip nuevo en el original (el cuadro clave), en segundos.
    pub inicio_real: f64,
    pub duracion: f64,
}

/// Una pista del original, decodificada a PCM desde el inicio del recorte.
struct Fuente {
    l: IMFSourceReader,
    flujo: u32,
    /// Estéreo intercalado.
    cola: std::collections::VecDeque<f32>,
    fin: bool,
    desde: Option<i64>,
    vol: f32,
}

impl Fuente {
    unsafe fn abrir(pedido: &Pedido, indice: usize, desde: i64, vol: f32) -> Result<Self> {
        let l = super::lector(&pedido.origen, false)?;
        let (_, audio) = super::flujos(&l);
        let flujo = *audio.get(indice).ok_or_else(|| anyhow!("falta la pista {indice}"))?;
        super::pedir_pcm(&l, flujo)?;
        ir_a(&l, desde)?;
        Ok(Self { l, flujo, cola: Default::default(), fin: false, desde: Some(desde), vol })
    }

    unsafe fn llenar(&mut self, cuadros: usize) -> Result<()> {
        while self.cola.len() < cuadros * 2 && !self.fin {
            let (mut flags, mut ts, mut m) = (0u32, 0i64, None);
            self.l.ReadSample(self.flujo, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut m))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                self.fin = true;
                break;
            }
            let Some(m) = m else { continue };
            let mut v: Vec<f32> = bytes_de(&m)?.chunks_exact(2).map(|x| i16::from_le_bytes([x[0], x[1]]) as f32).collect();
            if let Some(d) = self.desde.take() {
                let sobra = ((d - ts).max(0) as i128 * FRECUENCIA as i128 / SEGUNDO as i128) as usize * 2;
                v.drain(..sobra.min(v.len()));
            }
            self.cola.extend(v.into_iter().map(|x| x * self.vol));
        }
        Ok(())
    }

    /// Saca `cuadros` cuadros (con silencio si se terminó).
    unsafe fn sacar(&mut self, cuadros: usize) -> Result<Vec<f32>> {
        self.llenar(cuadros)?;
        let n = (cuadros * 2).min(self.cola.len());
        let mut v: Vec<f32> = self.cola.drain(..n).collect();
        v.resize(cuadros * 2, 0.0);
        Ok(v)
    }
}

/// Una pista del clip nuevo: la suma de algunas fuentes.
struct PistaNueva {
    flujo: u32,
    fuentes: Vec<usize>,
    escrito: usize,
}

unsafe fn muestra_pcm(pcm: &[f32], tiempo: i64) -> Result<IMFSample> {
    let datos: Vec<i16> = pcm.iter().map(|x| x.clamp(-32768.0, 32767.0) as i16).collect();
    let bytes = std::slice::from_raw_parts(datos.as_ptr() as *const u8, datos.len() * 2);
    let buffer = MFCreateMemoryBuffer(bytes.len() as u32)?;
    let mut p = std::ptr::null_mut();
    buffer.Lock(&mut p, None, None)?;
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    buffer.Unlock()?;
    buffer.SetCurrentLength(bytes.len() as u32)?;
    let m = MFCreateSample()?;
    m.AddBuffer(&buffer)?;
    m.SetSampleTime(tiempo)?;
    m.SetSampleDuration((pcm.len() / 2) as i64 * SEGUNDO / FRECUENCIA as i64)?;
    Ok(m)
}

/// Pista AAC de 192 kbps que recibe PCM de 16 bits.
pub(super) unsafe fn agregar_aac(escritor: &IMFSinkWriter, kbps: u32) -> Result<u32> {
    let aac = MFCreateMediaType()?;
    aac.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
    aac.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
    aac.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, FRECUENCIA)?;
    aac.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
    aac.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
    aac.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, kbps * 1000 / 8)?;
    let flujo = escritor.AddStream(&aac)?;
    let pcm = MFCreateMediaType()?;
    pcm.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
    pcm.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
    pcm.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, FRECUENCIA)?;
    pcm.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
    pcm.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
    pcm.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
    pcm.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, FRECUENCIA * 4)?;
    escritor.SetInputMediaType(flujo, &pcm, None).context("Windows no tiene el encoder AAC")?;
    Ok(flujo)
}

/// Sink writer de MP4 hacia `ruta` (el contenedor no depende de la extensión).
pub(super) unsafe fn escritor_mp4(ruta: &std::path::Path, hardware: bool) -> Result<IMFSinkWriter> {
    let mut atributos = None;
    MFCreateAttributes(&mut atributos, 3)?;
    let atributos = atributos.ok_or_else(|| anyhow!("sin atributos"))?;
    atributos.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
    if hardware {
        atributos.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
    }
    MFCreateSinkWriterFromURL(&HSTRING::from(ruta.as_os_str()), None, &atributos).context("no se pudo crear el archivo")
}

unsafe fn escribir_audio_hasta(
    escritor: &IMFSinkWriter,
    pistas: &mut [PistaNueva],
    fuentes: &mut [Fuente],
    hecho: &mut usize,
    hasta: usize,
) -> Result<()> {
    // Todas las pistas avanzan juntas, de a 100 ms: cada fuente se decodifica una sola vez.
    while *hecho < hasta {
        let n = TROZO.min(hasta - *hecho);
        let trozos: Vec<Vec<f32>> = fuentes.iter_mut().map(|f| f.sacar(n)).collect::<Result<_>>()?;
        let tiempo = *hecho as i64 * SEGUNDO / FRECUENCIA as i64;
        for p in pistas.iter_mut() {
            let mut suma = vec![0f32; n * 2];
            for &f in &p.fuentes {
                for (s, x) in suma.iter_mut().zip(&trozos[f]) {
                    *s += x;
                }
            }
            escritor.WriteSample(p.flujo, &muestra_pcm(&suma, tiempo)?)?;
            p.escrito += n;
        }
        *hecho += n;
    }
    Ok(())
}

pub fn exportar(p: &Pedido, progreso: &dyn Fn(f32), cancelar: &AtomicBool) -> Result<Resultado> {
    super::iniciar();
    if p.fin <= p.inicio + 0.1 {
        bail!("el recorte es demasiado corto");
    }
    let tmp = p.destino.with_extension("mp4.tmp");
    let r = unsafe { exportar_en(p, &tmp, progreso, cancelar) };
    match r {
        Ok(res) => {
            std::fs::rename(&tmp, &p.destino).context("no se pudo guardar el clip")?;
            Ok(res)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

unsafe fn exportar_en(p: &Pedido, tmp: &std::path::Path, progreso: &dyn Fn(f32), cancelar: &AtomicBool) -> Result<Resultado> {
    // Video comprimido, tal cual.
    let lv = super::lector(&p.origen, false)?;
    let (video, audio) = super::flujos(&lv);
    let video = video.ok_or_else(|| anyhow!("el clip no tiene video"))?;
    lv.SetStreamSelection(video, true)?;
    let tipo_video = lv.GetNativeMediaType(video, 0)?;
    ir_a(&lv, (p.inicio * SEGUNDO as f64) as i64)?;
    let fin = (p.fin * SEGUNDO as f64) as i64;

    // El primer cuadro clave: ahí empieza el clip nuevo (video y audio juntos).
    let mut primero = None;
    for _ in 0..1000 {
        let (mut flags, mut ts, mut m) = (0u32, 0i64, None);
        lv.ReadSample(video, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut m))?;
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            break;
        }
        if let Some(m) = m {
            if m.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0 {
                primero = Some((ts, m));
                break;
            }
        }
    }
    let (t0, primera) = primero.ok_or_else(|| anyhow!("no se encontró un cuadro clave en el recorte"))?;
    if tipo_video.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER).is_err() {
        // Si el tipo no trae SPS/PPS, se sacan del cuadro clave.
        if let Some(enc) = crate::grabadora::h264::encabezado(&bytes_de(&primera)?) {
            tipo_video.SetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &enc)?;
        }
    }
    let duracion = (fin - t0) as f64 / SEGUNDO as f64;

    // Audio: con pistas separadas, la mezcla nueva es la suma de las separadas; si no, la única pista.
    let separadas = audio.len() > 1;
    let usar: Vec<usize> = (0..audio.len())
        .filter(|&i| (!separadas || i > 0) && p.volumenes.get(i).copied().unwrap_or(0.0) > 0.0)
        .collect();
    let mut fuentes = Vec::new();
    for &i in &usar {
        fuentes.push(Fuente::abrir(p, i, t0, p.volumenes[i])?);
    }

    let escritor = escritor_mp4(tmp, false)?;
    let flujo_video = escritor.AddStream(&tipo_video)?;
    escritor.SetInputMediaType(flujo_video, &tipo_video, None)?;
    // Las pistas: la mezcla primero y, si había pistas separadas, cada una de las que quedan. El MP4 las deja
    // en orden inverso al que se agregan, así que se agregan al revés.
    let mut planes: Vec<(String, Vec<usize>)> = Vec::new();
    if !fuentes.is_empty() {
        planes.push(("Mezcla".into(), (0..fuentes.len()).collect()));
        if separadas && fuentes.len() > 1 {
            for (k, &i) in usar.iter().enumerate() {
                planes.push((p.nombres.get(i).cloned().unwrap_or_else(|| format!("Pista {i}")), vec![k]));
            }
        }
    }
    let mut pistas = Vec::new();
    for (_, f) in planes.iter().rev() {
        pistas.push(PistaNueva { flujo: agregar_aac(&escritor, 192)?, fuentes: f.clone(), escrito: 0 });
    }
    escritor.BeginWriting()?;

    let total_audio = ((fin - t0) as i128 * FRECUENCIA as i128 / SEGUNDO as i128) as usize;
    let mut audio_hecho = 0usize;
    let mut siguiente = Some((t0, primera));
    while let Some((ts, m)) = siguiente.take() {
        if ts >= fin {
            break;
        }
        if cancelar.load(Ordering::SeqCst) {
            bail!("cancelado");
        }
        let t = ts - t0;
        let hasta = ((t as i128 * FRECUENCIA as i128 / SEGUNDO as i128) as usize).min(total_audio);
        escribir_audio_hasta(&escritor, &mut pistas, &mut fuentes, &mut audio_hecho, hasta)?;
        m.SetSampleTime(t)?;
        escritor.WriteSample(flujo_video, &m)?;
        progreso((t as f64 / (fin - t0).max(1) as f64) as f32);
        let (mut flags, mut ts2, mut m2) = (0u32, 0i64, None);
        lv.ReadSample(video, 0, None, Some(&mut flags), Some(&mut ts2), Some(&mut m2))?;
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 == 0 {
            siguiente = m2.map(|m| (ts2, m));
        }
    }
    escribir_audio_hasta(&escritor, &mut pistas, &mut fuentes, &mut audio_hecho, total_audio)?;
    escritor.Finalize().context("no se pudo terminar el archivo")?;
    if let Err(e) = crate::pistas_mp4::marcar_mezcla(tmp) {
        crate::registro::log(&format!("no se pudo marcar la mezcla en el MP4: {e:#}"));
    }
    progreso(1.0);

    // Los datos del clip nuevo (juego y nombres de las pistas), como los de la grabadora.
    let original = crate::datos_clip::leer(&p.origen).unwrap_or_default();
    let info = super::info(tmp).unwrap_or_default();
    let datos = crate::datos_clip::DatosClip {
        version: original.version,
        juego: original.juego.clone(),
        pistas: planes.iter().map(|(n, _)| n.clone()).collect(),
        duracion,
        ancho: info.ancho,
        alto: info.alto,
        fps: info.fps.round() as u32,
        creado: crate::registro::fecha_para_archivo(),
        mezcla_marcada: true,
    };
    crate::datos_clip::escribir(&p.destino, &datos);
    Ok(Resultado { inicio_real: t0 as f64 / SEGUNDO as f64, duracion })
}
