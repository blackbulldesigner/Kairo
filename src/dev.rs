//! Pruebas sin ventana (solo con `--features dev`):
//! - `--prueba-grabar <segundos> <archivo.mp4> [tono] [juego] [solo-juego]`: graba y guarda el clip.
//!   `tono` suena un tono a la mitad; `juego` hace que este mismo proceso sea "el juego" (el tono tiene
//!   que quedar en la pista del juego y no en la del resto del sistema; activa las pistas separadas);
//!   `solo-juego` graba en modo "Solo el juego"; `programa` suma este exe como programa elegido (el tono tiene
//!   que quedar en su pista); `avanzado` graba con opciones avanzadas fuera de lo normal (6 Mbps constantes,
//!   cuadro clave cada 2 s, perfil Básico, velocidad Rápida, audio a 128 kbps).
//! - `--prueba-leer <archivo.mp4> [cuadro.png]`: abre el MP4 como lo haría un reproductor, cuenta cuadros,
//!   mide cada pista de audio y guarda una imagen del cuadro del medio.
//! - `--prueba-juego <segundos>`: muestra qué detecta como juego, dos veces por segundo.
//! - `--prueba-instalar` / `--prueba-desinstalar` (con `KAIRO_PRUEBA_INSTALACION=<carpeta>`): instalan y
//!   desinstalan dentro de esa carpeta y revisan que cada cosa quede (o se vaya) donde tiene que estar.

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
        let extra = |x: &str| args.iter().skip(k + 3).any(|a| a == x);
        let opciones = Opciones {
            tono: extra("tono"),
            juego: extra("juego"),
            solo_juego: extra("solo-juego"),
            programa: extra("programa"),
            avanzado: extra("avanzado"),
        };
        return Some(informar(prueba_grabar(segundos, args.get(k + 2)?, opciones)));
    }
    if let Some(k) = pos("--prueba-juego") {
        let segundos: u32 = args.get(k + 1).and_then(|s| s.parse().ok()).unwrap_or(5);
        let mut d = crate::grabadora::juego::Detector::new(None);
        println!("programas abiertos: {:?}", crate::grabadora::juego::programas_abiertos().iter().map(|p| &p.nombre).collect::<Vec<_>>());
        for _ in 0..segundos * 2 {
            d.revisar();
            match d.juego() {
                Some(j) => println!("juego: {} (pid {}) · adelante: {}", j.nombre, j.pid, d.al_frente()),
                None => println!("sin juego"),
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        return Some(0);
    }
    if let Some(k) = pos("--prueba-leer") {
        return Some(informar(prueba_leer(args.get(k + 1)?, args.get(k + 2).map(|s| s.as_str()))));
    }
    if pos("--prueba-instalar").is_some() {
        return Some(informar(prueba_instalar()));
    }
    if pos("--prueba-desinstalar").is_some() {
        return Some(informar(prueba_desinstalar()));
    }
    if let Some(k) = pos("--prueba-actualizacion") {
        return Some(informar(prueba_actualizacion(args.get(k + 1).map(|s| s.as_str()))));
    }
    None
}

/// Busca el último release de Kairo y, con `[api]` (la API de otro repositorio con releases), prueba bajar su
/// archivo más chico con SHA-256: tiene que pasar con el SHA bueno y rechazarse con uno cambiado.
fn prueba_actualizacion(api: Option<&str>) -> Result<()> {
    use crate::actualizacion::{self, Archivo};
    match actualizacion::buscar()? {
        None => println!("Kairo: todavía no hay releases publicados"),
        Some(d) => println!(
            "Kairo: último release {} (¿más nuevo que este exe? {}) · instalador: {:?}",
            d.version,
            actualizacion::es_mas_nueva(&d.version),
            d.instalador
        ),
    }
    let Some(api) = api else { return Ok(()) };
    let (codigo, cuerpo) = crate::red::texto(api, Some("application/vnd.github+json"))?;
    if codigo != 200 {
        return Err(anyhow!("{api} respondió {codigo}"));
    }
    let j: serde_json::Value = serde_json::from_str(&cuerpo)?;
    let d = actualizacion::desde_json(&j);
    println!("release de prueba: {:?}", d.as_ref().map(|d| (&d.version, &d.pagina)));
    let asset = j["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["digest"].as_str().is_some_and(|d| d.starts_with("sha256:")))
        .min_by_key(|a| a["size"].as_u64().unwrap_or(u64::MAX))
        .ok_or_else(|| anyhow!("ese release no tiene archivos con SHA-256"))?;
    let bueno = Archivo {
        nombre: asset["name"].as_str().unwrap_or_default().to_string(),
        url: asset["browser_download_url"].as_str().unwrap_or_default().to_string(),
        tamano: asset["size"].as_u64().unwrap_or(0),
        sha256: asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(|s| s.to_string()),
    };
    println!("bajando {} ({} bytes)…", bueno.nombre, bueno.tamano);
    let ruta = actualizacion::descargar(&bueno, &|_| {})?;
    println!("ok   bajado y verificado: {} ({} bytes)", ruta.display(), std::fs::metadata(&ruta)?.len());
    let malo = Archivo { sha256: Some("0".repeat(64)), ..bueno };
    match actualizacion::descargar(&malo, &|_| {}) {
        Ok(_) => return Err(anyhow!("se aceptó un archivo con el SHA-256 cambiado")),
        Err(e) => println!("ok   con el SHA-256 cambiado se rechaza: {e}"),
    }
    actualizacion::limpiar_descargas();
    Ok(())
}

/// Las pruebas del instalador solo corren con `KAIRO_PRUEBA_INSTALACION` (todo queda en esa carpeta).
fn destinos_de_prueba() -> Result<crate::instalacion::Destinos> {
    let d = crate::instalacion::destinos();
    if !d.prueba {
        return Err(anyhow!("falta KAIRO_PRUEBA_INSTALACION=<carpeta>: sin eso instalaría en la PC de verdad"));
    }
    Ok(d)
}

fn prueba_instalar() -> Result<()> {
    let d = destinos_de_prueba()?;
    let op = crate::instalacion::Opciones { escritorio: true, inicio_windows: true };
    let exe = crate::instalacion::instalar(&op, &|p| println!("paso: {p}"))?;
    let revisar = |que: &str, ok: bool| -> Result<()> {
        println!("{} {que}", if ok { "ok  " } else { "MAL " });
        if ok { Ok(()) } else { Err(anyhow!("falta: {que}")) }
    };
    let propio = std::fs::metadata(std::env::current_exe()?)?.len();
    revisar(&format!("exe copiado en {}", exe.display()), std::fs::metadata(&exe).is_ok_and(|m| m.len() == propio))?;
    revisar("acceso del menú", d.acceso_menu.exists())?;
    revisar("acceso del escritorio", d.acceso_escritorio.exists())?;
    let version = crate::instalacion::version_instalada();
    revisar(&format!("versión registrada ({version:?})"), version.as_deref() == Some(crate::version::VERSION))?;
    Ok(())
}

fn prueba_desinstalar() -> Result<()> {
    let d = destinos_de_prueba()?;
    crate::instalacion::desinstalar(false)?;
    crate::instalacion::borrar_carpeta_al_salir()?;
    // Si este exe es el instalado, la carpeta recién se puede borrar cuando se cierre: eso se revisa afuera.
    // (Con canonicalize, para comparar rutas aunque una venga con nombres cortos, como "BLACKB~1".)
    let adentro = match (std::env::current_exe().and_then(std::fs::canonicalize), std::fs::canonicalize(&d.carpeta)) {
        (Ok(e), Ok(c)) => e.starts_with(c),
        _ => false,
    };
    if adentro {
        println!("este exe está en la carpeta: se borra cuando se cierre");
    }
    // La carpeta la borra cmd un momento después.
    for _ in 0..if adentro { 0 } else { 40 } {
        if !d.carpeta.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let mut mal = Vec::new();
    for (que, queda) in [
        ("carpeta del programa", !adentro && d.carpeta.exists()),
        ("acceso del menú", d.acceso_menu.exists()),
        ("acceso del escritorio", d.acceso_escritorio.exists()),
        ("entrada en el registro", crate::instalacion::version_instalada().is_some()),
    ] {
        println!("{} {que} {}", if queda { "MAL " } else { "ok  " }, if queda { "sigue ahí" } else { "borrado" });
        if queda {
            mal.push(que);
        }
    }
    if mal.is_empty() { Ok(()) } else { Err(anyhow!("quedaron cosas: {mal:?}")) }
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

struct Opciones {
    tono: bool,
    juego: bool,
    solo_juego: bool,
    programa: bool,
    avanzado: bool,
}

fn prueba_grabar(segundos: u32, ruta: &str, o: Opciones) -> Result<()> {
    let Opciones { tono, juego, solo_juego, programa, avanzado } = o;
    iniciar_mf()?;
    let mut cfg = Config::cargar();
    cfg.segundos = segundos;
    if juego {
        // Este proceso es "el juego": el tono sale de acá.
        std::env::set_var("KAIRO_JUEGO_PID", std::process::id().to_string());
        cfg.audio_juego.activa = true;
        cfg.audio_sistema.activa = true;
        cfg.pistas_separadas = true;
    }
    if solo_juego {
        cfg.modo = crate::config::ModoCaptura::Juego;
    }
    if programa {
        // Este exe es un "programa elegido": el tono tiene que quedar en su pista.
        cfg.otros_sonidos = crate::config::OtrosSonidos::Elegidos;
        cfg.programas_audio = vec![crate::config::ProgramaAudio { exe: "kairo.exe".into(), nombre: "Prueba".into(), activa: true, volumen: 100 }];
        cfg.pistas_separadas = true;
    }
    if avanzado {
        // Opciones avanzadas fuera de lo normal: el encoder tiene que aceptarlas.
        use crate::config::{ControlBitrate, PerfilH264, VelocidadEncoder, VideoAvanzado};
        cfg.avanzado = VideoAvanzado {
            activo: true,
            mbps: 6.0,
            control: ControlBitrate::Constante,
            pico: 1.5,
            clave_cada: 2.0,
            perfil: PerfilH264::Basico,
            velocidad: VelocidadEncoder::Rapida,
            audio_kbps: 128,
        };
    }
    let g = Grabadora::iniciar(&cfg);
    std::thread::sleep(Duration::from_millis(segundos as u64 * 500));
    if tono {
        sonido::guardado(); // para comprobar que el audio queda en el clip
    }
    std::thread::sleep(Duration::from_millis(segundos as u64 * 500 + 1500));
    let info = g.info.lock().map(|i| i.clone()).unwrap_or_default();
    println!("grabando: {} · {} · juego: {:?} · error: {:?}", info.grabando, info.detalle, info.juego, info.error);
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
        let mut pistas_audio = Vec::new();
        while let Ok(t) = lector.GetNativeMediaType(i, 0) {
            let mayor = guid_nombre(t.GetGUID(&MF_MT_MAJOR_TYPE)?);
            let sub = guid_nombre(t.GetGUID(&MF_MT_SUBTYPE)?);
            let extra = if mayor == "video" {
                let s = t.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
                let r = t.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(0);
                let perfil = match t.GetUINT32(&MF_MT_MPEG2_PROFILE).unwrap_or(0) {
                    66 => "perfil Básico",
                    77 => "perfil Principal",
                    100 => "perfil Alto",
                    _ => "perfil ?",
                };
                let kbps = t.GetUINT32(&MF_MT_AVG_BITRATE).unwrap_or(0) / 1000;
                format!("{}×{} a {} fps, {perfil}, {kbps} kbps", s >> 32, s & 0xFFFF_FFFF, (r >> 32) / (r & 0xFFFF_FFFF).max(1))
            } else {
                format!(
                    "{} Hz, {} canales, {} kbps",
                    t.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(0),
                    t.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(0),
                    t.GetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND).unwrap_or(0) * 8 / 1000
                )
            };
            println!("pista {i}: {mayor} {sub} {extra}");
            if mayor == "audio" {
                pistas_audio.push(i);
            }
            i += 1;
        }

        // Video: decodificar todo, contar cuadros y guardar el del medio.
        let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let rgb = MFCreateMediaType()?;
        rgb.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        rgb.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        lector.SetCurrentMediaType(video, None, &rgb)?;
        let actual = lector.GetCurrentMediaType(video)?;
        let s = actual.GetUINT64(&MF_MT_FRAME_SIZE)?;
        let (w, h) = ((s >> 32) as usize, (s & 0xFFFF_FFFF) as usize);
        for &k in &pistas_audio {
            let _ = lector.SetStreamSelection(k, false);
        }
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

        // Audio: cada pista por separado, decodificada a PCM. Por ventanas de 100 ms: cuánto tiempo
        // tiene sonido y dónde está lo más fuerte (para ver en qué pista quedó el tono de prueba).
        let pcm = MFCreateMediaType()?;
        pcm.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        pcm.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        pcm.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        if pistas_audio.is_empty() {
            println!("sin pistas de audio");
        }
        for (n, &indice) in pistas_audio.iter().enumerate() {
            for k in 0..i {
                let _ = lector.SetStreamSelection(k, k == indice);
            }
            lector.SetCurrentMediaType(indice, None, &pcm)?;
            lector.SetCurrentPosition(&windows::core::GUID::zeroed(), &PROPVARIANT::from(0i64))?;
            let mut muestras: Vec<i16> = Vec::new();
            loop {
                let (mut flags, mut muestra) = (0u32, None);
                lector.ReadSample(indice, 0, None, Some(&mut flags), None, Some(&mut muestra))?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    break;
                }
                let Some(m) = muestra else { continue };
                let b = m.ConvertToContiguousBuffer()?;
                let mut p = std::ptr::null_mut();
                let mut largo = 0u32;
                b.Lock(&mut p, None, Some(&mut largo))?;
                muestras.extend_from_slice(std::slice::from_raw_parts(p as *const i16, largo as usize / 2));
                b.Unlock()?;
            }
            let ventana = 4_800 * 2; // 100 ms estéreo
            let rms: Vec<f64> = muestras
                .chunks(ventana)
                .map(|c| (c.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / c.len().max(1) as f64).sqrt())
                .collect();
            let con_sonido = rms.iter().filter(|&&r| r > 300.0).count() as f64 / 10.0;
            let (mayor_i, mayor) = rms.iter().enumerate().fold((0, 0.0f64), |a, (k, &r)| if r > a.1 { (k, r) } else { a });
            let nombre = if n == 0 { "mezcla".to_string() } else { format!("pista {n}") };
            println!(
                "audio {nombre}: {:.2} s, con sonido {con_sonido:.1} s, lo más fuerte RMS {mayor:.0} a los {:.1} s",
                muestras.len() as f64 / 2.0 / 48_000.0,
                mayor_i as f64 / 10.0
            );
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
