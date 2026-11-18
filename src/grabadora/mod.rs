//! La grabadora: un hilo de video (captura → NV12 → encoder) y uno de audio, que llenan el buffer
//! en RAM. `guardar` toma los últimos segundos y escribe el MP4.

mod audio;
mod buffer;
mod captura;
mod encoder;
mod mp4;
pub mod reloj;
pub mod sonido;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{anyhow, Result};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

use crate::config::Config;
use crate::registro::log;
use buffer::{Buffer, FormatoVideo};
use reloj::SEGUNDO;

/// Qué está pasando, para mostrarlo en la bandeja y en los ajustes.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub grabando: bool,
    /// "1920×1080 a 60 fps · NVIDIA H.264 Encoder MFT".
    pub detalle: String,
    pub error: Option<String>,
}

pub struct Grabadora {
    detener: Arc<AtomicBool>,
    hilos: Vec<JoinHandle<()>>,
    buffer: Arc<Mutex<Buffer>>,
    pub info: Arc<Mutex<Info>>,
}

/// Lo necesario para guardar un clip desde otro hilo.
#[derive(Clone)]
pub struct Guardador {
    buffer: Arc<Mutex<Buffer>>,
}

impl Grabadora {
    pub fn iniciar(cfg: &Config) -> Grabadora {
        let detener = Arc::new(AtomicBool::new(false));
        let buffer = Arc::new(Mutex::new(Buffer::new(cfg.segundos)));
        let info = Arc::new(Mutex::new(Info::default()));
        let mut hilos = Vec::new();
        {
            let (cfg, detener, buffer, info) = (cfg.clone(), detener.clone(), buffer.clone(), info.clone());
            hilos.push(std::thread::spawn(move || con_com(|| hilo_video(&cfg, &detener, &buffer, &info))));
        }
        if cfg.audio_sistema.activa || cfg.audio_juego.activa {
            let (detener, buffer) = (detener.clone(), buffer.clone());
            hilos.push(std::thread::spawn(move || con_com(|| hilo_audio(&detener, &buffer))));
        }
        Grabadora { detener, hilos, buffer, info }
    }

    pub fn guardador(&self) -> Guardador {
        Guardador { buffer: self.buffer.clone() }
    }

    pub fn detener(self) {
        self.detener.store(true, Ordering::SeqCst);
        for h in self.hilos {
            let _ = h.join();
        }
    }

    #[allow(dead_code)]
    pub fn segundos_en_buffer(&self) -> f64 {
        self.buffer.lock().map(|b| b.segundos_de_video()).unwrap_or(0.0)
    }
}

impl Guardador {
    /// Escribe los últimos segundos en la carpeta de clips. Devuelve la ruta del MP4.
    pub fn guardar(&self, cfg: &Config) -> Result<PathBuf> {
        let nombre = format!("Kairo {}.mp4", crate::registro::fecha_para_archivo());
        self.guardar_en(cfg, &cfg.carpeta.join(nombre), cfg.segundos)
    }

    pub fn guardar_en(&self, cfg: &Config, ruta: &std::path::Path, segundos: u32) -> Result<PathBuf> {
        let clip = self
            .buffer
            .lock()
            .map_err(|_| anyhow!("buffer bloqueado"))?
            .instantanea(segundos)
            .ok_or_else(|| anyhow!("todavía no hay nada grabado"))?;
        // Por ahora todo el sonido de la PC va en una sola pista (separar el juego llega en la fase 3).
        let volumen = if cfg.audio_sistema.activa {
            Some(cfg.audio_sistema.volumen as f32 / 100.0)
        } else if cfg.audio_juego.activa {
            Some(cfg.audio_juego.volumen as f32 / 100.0)
        } else {
            None
        };
        if let Some(dir) = ruta.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = ruta.with_extension("mp4.tmp");
        let r = mp4::escribir(&tmp, &clip, volumen);
        if let Err(e) = r {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        std::fs::rename(&tmp, ruta)?;
        let seg = (clip.fin - clip.inicio) as f64 / SEGUNDO as f64;
        log(&format!("clip guardado: {} ({seg:.1} s, {} cuadros, {} trozos de audio)", ruta.display(), clip.video.len(), clip.audio.len()));
        Ok(ruta.to_path_buf())
    }
}

fn con_com(f: impl FnOnce()) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
    }
    f();
    unsafe { CoUninitialize() };
}

fn poner_info(info: &Mutex<Info>, f: impl FnOnce(&mut Info)) {
    if let Ok(mut i) = info.lock() {
        f(&mut i);
    }
}

fn hilo_video(cfg: &Config, detener: &AtomicBool, buffer: &Mutex<Buffer>, info: &Mutex<Info>) {
    let mut error_anterior = String::new();
    while !detener.load(Ordering::SeqCst) {
        match sesion_video(cfg, detener, buffer, info) {
            Ok(()) => break,
            Err(e) => {
                let texto = format!("{e:#}");
                let perdida = captura::es_captura_perdida(&e);
                if texto != error_anterior && !perdida {
                    log(&format!("video: {texto}"));
                    error_anterior = texto.clone();
                }
                // Perder la captura es normal (cambio de resolución, UAC): se vuelve enseguida y sin avisar.
                if !perdida {
                    poner_info(info, |i| {
                        i.grabando = false;
                        i.error = Some(e.root_cause().to_string());
                    });
                }
                let espera = if perdida { 100 } else { 3000 };
                for _ in 0..espera / 50 {
                    if detener.load(Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }
    poner_info(info, |i| i.grabando = false);
}

fn sesion_video(cfg: &Config, detener: &AtomicBool, buffer: &Mutex<Buffer>, info: &Mutex<Info>) -> Result<()> {
    let monitor = captura::buscar_monitor(cfg.monitor.as_deref())?;
    let mut cap = captura::Captura::abrir(&monitor, |w, h| cfg.tamano_clip((w, h)), cfg.fps)?;
    let bitrate = (cfg.mbps((cap.ancho_monitor, cap.alto_monitor)) * 1_000_000.0) as u32;
    let mut enc = encoder::Encoder::abrir(&cap.dispositivo, cap.luid, cap.ancho, cap.alto, cfg.fps, bitrate)?;
    let formato = FormatoVideo { ancho: enc.ancho, alto: enc.alto, fps: cfg.fps, bitrate, encoder: enc.nombre.clone() };
    let detalle = format!("{}×{} a {} fps · {}", enc.ancho, enc.alto, cfg.fps, enc.nombre);
    log(&format!("grabando {} (monitor {} de {}×{})", detalle, monitor.id, cap.ancho_monitor, cap.alto_monitor));
    if let Ok(mut b) = buffer.lock() {
        b.poner_formato(formato);
    }
    poner_info(info, |i| {
        i.grabando = true;
        i.detalle = detalle;
        i.error = None;
    });

    let dur = SEGUNDO / cfg.fps as i64;
    let inicio = reloj::ahora();
    let mut n: i64 = 0;
    let mut paquetes = Vec::new();
    while !detener.load(Ordering::SeqCst) {
        let mut objetivo = inicio + n * dur;
        let ahora = reloj::ahora();
        if ahora - objetivo > 4 * dur {
            // Nos atrasamos (la PC estuvo muy cargada): se salta al presente en vez de acumular.
            n = (ahora - inicio) / dur;
            objetivo = inicio + n * dur;
        }
        reloj::dormir_hasta(objetivo);
        cap.capturar()?;
        enc.atender(&mut paquetes)?;
        if cap.hay_cuadro() {
            let tex = cap.convertir()?;
            enc.enviar(&tex, objetivo)?;
        }
        enc.atender(&mut paquetes)?;
        if !paquetes.is_empty() {
            if let Ok(mut b) = buffer.lock() {
                b.agregar_video(paquetes.drain(..));
            }
        }
        n += 1;
    }
    Ok(())
}

fn hilo_audio(detener: &AtomicBool, buffer: &Mutex<Buffer>) {
    let mut error_anterior = String::new();
    while !detener.load(Ordering::SeqCst) {
        match audio::CapturaAudio::abrir() {
            Ok(mut a) => {
                log("audio: capturando la salida predeterminada");
                let mut trozos = Vec::new();
                let mut vueltas = 0u32;
                loop {
                    if detener.load(Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                    if let Err(e) = a.leer(&mut trozos) {
                        log(&format!("audio: {e:#}"));
                        break;
                    }
                    if !trozos.is_empty() {
                        if let Ok(mut b) = buffer.lock() {
                            b.agregar_audio(trozos.drain(..));
                        }
                    }
                    // Cada 2 s: si cambió la salida predeterminada (otros auriculares), se pasa a esa.
                    vueltas += 1;
                    if vueltas % 200 == 0 && audio::salida_predeterminada().is_some_and(|id| id != a.id()) {
                        log("audio: cambió la salida predeterminada");
                        break;
                    }
                }
            }
            Err(e) => {
                let texto = format!("{e:#}");
                if texto != error_anterior {
                    log(&format!("audio: {texto}"));
                    error_anterior = texto;
                }
                for _ in 0..40 {
                    if detener.load(Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }
}
