//! Pruebas sin ventana (solo con `--features dev`):
//! - `--prueba-grabar <segundos> <archivo.mp4>`: graba, suena un tono a la mitad y guarda el clip.
//! - `--prueba-leer <archivo.mp4> [cuadro.png]`: abre el MP4 como lo haría un reproductor, cuenta cuadros
//!   y audio, y guarda una imagen del cuadro del medio.

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Result};
use windows::core::{HSTRING, PROPVARIANT};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use crate::config::Config;
use crate::grabadora::{sonido, Grabadora};

pub fn ejecutar(args: &[String]) -> Option<i32> {
    let pos = |f: &str| args.iter().position(|a| a == f);
    if let Some(k) = pos("--prueba-grabar") {
        let segundos: u32 = args.get(k + 1)?.parse().ok()?;
        let tono = args.get(k + 3).is_some_and(|a| a == "tono");
        return Some(informar(prueba_grabar(segundos, args.get(k + 2)?, tono)));
    }
    if let Some(k) = pos("--prueba-leer") {
        return Some(informar(prueba_leer(args.get(k + 1)?, args.get(k + 2).map(|s| s.as_str()))));
    }
    None
}

fn informar(r: Result<()>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ERROR: {e:#}");
            1
        }
    }
}

fn iniciar_mf() -> Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
        MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
        windows::Win32::Media::timeBeginPeriod(1);
    }
    Ok(())
}

fn prueba_grabar(segundos: u32, ruta: &str, tono: bool) -> Result<()> {
    iniciar_mf()?;
    let mut cfg = Config::cargar();
    cfg.segundos = segundos;
    let g = Grabadora::iniciar(&cfg);
    std::thread::sleep(Duration::from_millis(segundos as u64 * 500));
    if tono {
        sonido::guardado(); // para comprobar que el audio queda en el clip
    }
    std::thread::sleep(Duration::from_millis(segundos as u64 * 500 + 1500));
    let info = g.info.lock().map(|i| i.clone()).unwrap_or_default();
    println!("grabando: {} · {} · error: {:?}", info.grabando, info.detalle, info.error);
    println!("en el buffer: {:.1} s de video", g.segundos_en_buffer());
    let t = std::time::Instant::now();
    let r = g.guardador().guardar_en(&cfg, Path::new(ruta), segundos);
    println!("guardar tardó {} ms", t.elapsed().as_millis());
    g.detener();
    let p = r?;
    println!("guardado: {} ({} KB)", p.display(), std::fs::metadata(&p)?.len() / 1024);
    Ok(())
}

unsafe fn guid_nombre(g: windows::core::GUID) -> String {
    for (n, v) in [
        ("H.264", MFVideoFormat_H264),
        ("AAC", MFAudioFormat_AAC),
        ("PCM", MFAudioFormat_PCM),
        ("RGB32", MFVideoFormat_RGB32),
        ("video", MFMediaType_Video),
        ("audio", MFMediaType_Audio),
    ] {
        if g == v {
            return n.into();
        }
    }
    format!("{g:?}")
}

fn prueba_leer(ruta: &str, png: Option<&str>) -> Result<()> {
    iniciar_mf()?;
    unsafe {
        let mut atributos = None;
        MFCreateAttributes(&mut atributos, 2)?;
        let atributos = atributos.ok_or_else(|| anyhow!("sin atributos"))?;
        atributos.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
        let lector = MFCreateSourceReaderFromURL(&HSTRING::from(ruta), &atributos)?;
        let fuente = MF_SOURCE_READER_MEDIASOURCE.0 as u32;
        let duracion = lector.GetPresentationAttribute(fuente, &MF_PD_DURATION).ok().and_then(|p| u64::try_from(&p).ok()).unwrap_or(0);
        println!("duración: {:.2} s", duracion as f64 / 1e7);
        let mut i = 0;
        while let Ok(t) = lector.GetNativeMediaType(i, 0) {
            let mayor = guid_nombre(t.GetGUID(&MF_MT_MAJOR_TYPE)?);
            let sub = guid_nombre(t.GetGUID(&MF_MT_SUBTYPE)?);
            let extra = if mayor == "video" {
                let s = t.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
                let r = t.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(0);
                format!("{}×{} a {} fps", s >> 32, s & 0xFFFF_FFFF, (r >> 32) / (r & 0xFFFF_FFFF).max(1))
            } else {
                format!(
                    "{} Hz, {} canales",
                    t.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(0),
                    t.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(0)
                )
            };
            println!("pista {i}: {mayor} {sub} {extra}");
            i += 1;
        }

        // Video: decodificar todo, contar cuadros y guardar el del medio.
        let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let audio = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
        let rgb = MFCreateMediaType()?;
        rgb.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        rgb.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        lector.SetCurrentMediaType(video, None, &rgb)?;
        let actual = lector.GetCurrentMediaType(video)?;
        let s = actual.GetUINT64(&MF_MT_FRAME_SIZE)?;
        let (w, h) = ((s >> 32) as usize, (s & 0xFFFF_FFFF) as usize);
        let _ = lector.SetStreamSelection(audio, false);
        let mut cuadros = 0;
        let mut guardado = false;
        loop {
            let (mut flags, mut ts, mut muestra) = (0u32, 0i64, None);
            lector.ReadSample(video, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut muestra))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            let Some(m) = muestra else { continue };
            cuadros += 1;
            if let (Some(png), false) = (png, guardado) {
                if ts as u64 >= duracion / 2 {
                    let b = m.ConvertToContiguousBuffer()?;
                    let mut p = std::ptr::null_mut();
                    let mut largo = 0u32;
                    b.Lock(&mut p, None, Some(&mut largo))?;
                    let bgra = std::slice::from_raw_parts(p, largo as usize);
                    let mut rgba = Vec::with_capacity(w * h * 4);
                    for px in bgra.chunks_exact(4).take(w * h) {
                        rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                    }
                    b.Unlock()?;
                    guardar_png(Path::new(png), w as u32, h as u32, &rgba)?;
                    println!("cuadro del medio ({:.2} s) guardado en {png}", ts as f64 / 1e7);
                    guardado = true;
                }
            }
        }
        println!("cuadros de video decodificados: {cuadros}");

        // Audio: decodificar a PCM y medir el volumen (para saber si no quedó en silencio).
        let pcm = MFCreateMediaType()?;
        pcm.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        pcm.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        pcm.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        if lector.SetStreamSelection(audio, true).is_ok() && lector.SetCurrentMediaType(audio, None, &pcm).is_ok() {
            let _ = lector.SetStreamSelection(video, false);
            lector.SetCurrentPosition(&windows::core::GUID::zeroed(), &PROPVARIANT::from(0i64))?;
            let (mut total, mut pico, mut suma) = (0usize, 0i32, 0f64);
            loop {
                let (mut flags, mut muestra) = (0u32, None);
                lector.ReadSample(audio, 0, None, Some(&mut flags), None, Some(&mut muestra))?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    break;
                }
                let Some(m) = muestra else { continue };
                let b = m.ConvertToContiguousBuffer()?;
                let mut p = std::ptr::null_mut();
                let mut largo = 0u32;
                b.Lock(&mut p, None, Some(&mut largo))?;
                for s in std::slice::from_raw_parts(p as *const i16, largo as usize / 2) {
                    pico = pico.max((*s as i32).abs());
                    suma += (*s as f64) * (*s as f64);
                    total += 1;
                }
                b.Unlock()?;
            }
            let rms = (suma / total.max(1) as f64).sqrt();
            println!("audio decodificado: {:.2} s, pico {pico}, RMS {rms:.0}", total as f64 / 2.0 / 48_000.0);
        } else {
            println!("sin pista de audio");
        }
    }
    Ok(())
}

fn guardar_png(ruta: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    let archivo = std::io::BufWriter::new(std::fs::File::create(ruta)?);
    let mut enc = png::Encoder::new(archivo, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}
