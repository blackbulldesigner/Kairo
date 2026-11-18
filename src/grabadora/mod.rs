//! La grabadora: un hilo de video (captura → NV12 → encoder) y uno de audio, que llenan el buffer
//! en RAM. `guardar` toma los últimos segundos y escribe el MP4.
//!
//! El hilo de video también detecta el juego (`juego.rs`): lo comparte con el de audio (para grabar
//! solo el juego o todo menos el juego) y con el guardado (carpeta y nombre del clip).

mod audio;
mod buffer;
mod captura;
mod codificador;
mod encoder;
mod h264;
pub mod juego;
mod mp4;
pub mod reloj;
pub mod sonido;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{anyhow, Result};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

use crate::config::{Config, ModoCaptura, OtrosSonidos, Resolucion};
use crate::registro::log;
use audio::{CapturaAudio, Origen};
use buffer::{Buffer, FormatoVideo, PISTA_JUEGO, PISTA_MICROFONO, PISTA_PROGRAMAS, PISTA_SISTEMA};
use reloj::SEGUNDO;

pub use juego::nombre_para_carpeta;

/// Qué está pasando, para mostrarlo en la bandeja y en los ajustes.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub grabando: bool,
    /// "1920×1080 a 60 fps · NVIDIA H.264 Encoder MFT · Desktop Duplication · NVIDIA GeForce …".
    pub detalle: String,
    pub error: Option<String>,
    /// El juego detectado (sigue siendo "el juego" aunque hagas Alt+Tab, hasta que se cierre).
    pub juego: Option<String>,
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
    info: Arc<Mutex<Info>>,
}

impl Grabadora {
    pub fn iniciar(cfg: &Config) -> Grabadora {
        let detener = Arc::new(AtomicBool::new(false));
        let buffer = Arc::new(Mutex::new(Buffer::new(cfg.segundos, PISTA_PROGRAMAS + cfg.programas_audio.len())));
        let info = Arc::new(Mutex::new(Info::default()));
        // pid del juego detectado (0 = ninguno): lo escribe el hilo de video y lo lee el de audio.
        let pid_juego = Arc::new(AtomicU32::new(0));
        let mut hilos = Vec::new();
        {
            let (cfg, detener, buffer, info, pid) = (cfg.clone(), detener.clone(), buffer.clone(), info.clone(), pid_juego.clone());
            hilos.push(std::thread::spawn(move || con_com(|| hilo_video(&cfg, &detener, &buffer, &info, &pid))));
        }
        let programas = cfg.otros_sonidos == OtrosSonidos::Elegidos && cfg.programas_audio.iter().any(|p| p.activa);
        if cfg.audio_juego.activa || cfg.audio_sistema.activa || cfg.audio_mic.activa || programas {
            let (cfg, detener, buffer, pid) = (cfg.clone(), detener.clone(), buffer.clone(), pid_juego.clone());
            hilos.push(std::thread::spawn(move || con_com(|| hilo_audio(&cfg, &detener, &buffer, &pid))));
        }
        Grabadora { detener, hilos, buffer, info }
    }

    pub fn guardador(&self) -> Guardador {
        Guardador { buffer: self.buffer.clone(), info: self.info.clone() }
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
    /// Escribe los últimos segundos en la carpeta de clips (en la del juego, si hay uno y está activado).
    /// El nombre lleva el juego y la fecha: "VALORANT 2026-11-18 21-30-05.mp4". Devuelve la ruta del MP4.
    pub fn guardar(&self, cfg: &Config) -> Result<PathBuf> {
        let juego = self.info.lock().ok().and_then(|i| i.juego.clone()).map(|j| nombre_para_carpeta(&j));
        let fecha = crate::registro::fecha_para_archivo();
        let (carpeta, nombre) = match &juego {
            Some(j) if cfg.carpeta_por_juego => (cfg.carpeta.join(j), format!("{j} {fecha}.mp4")),
            Some(j) => (cfg.carpeta.clone(), format!("{j} {fecha}.mp4")),
            None => (cfg.carpeta.clone(), format!("Kairo {fecha}.mp4")),
        };
        self.guardar_en(cfg, &carpeta.join(nombre), cfg.segundos)
    }

    pub fn guardar_en(&self, cfg: &Config, ruta: &std::path::Path, segundos: u32) -> Result<PathBuf> {
        let clip = self
            .buffer
            .lock()
            .map_err(|_| anyhow!("buffer bloqueado"))?
            .instantanea(segundos)
            .ok_or_else(|| anyhow!("todavía no hay nada grabado"))?;
        let (volumenes, nombres) = pistas_de_audio(cfg);
        let audio = mp4::AudioDelClip { volumenes, nombres, separadas: cfg.pistas_separadas };
        if let Some(dir) = ruta.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = ruta.with_extension("mp4.tmp");
        let pistas = match mp4::escribir(&tmp, &clip, &audio) {
            Ok(p) => p,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        };
        std::fs::rename(&tmp, ruta)?;
        let seg = (clip.fin - clip.inicio) as f64 / SEGUNDO as f64;
        crate::datos_clip::escribir(
            ruta,
            &crate::datos_clip::DatosClip {
                version: 1,
                juego: self.info.lock().ok().and_then(|i| i.juego.clone()),
                pistas,
                duracion: seg,
                ancho: clip.formato.ancho,
                alto: clip.formato.alto,
                fps: clip.formato.fps,
                creado: crate::registro::fecha_para_archivo(),
            },
        );
        let trozos: Vec<String> = clip
            .audio
            .iter()
            .enumerate()
            .map(|(i, t)| format!("{} {}", audio.nombres.get(i).map(|n| n.as_str()).unwrap_or("?"), t.len()))
            .collect();
        log(&format!("clip guardado: {} ({seg:.1} s, {} cuadros; audio: {})", ruta.display(), clip.video.len(), trozos.join(", ")));
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

fn hilo_video(cfg: &Config, detener: &AtomicBool, buffer: &Mutex<Buffer>, info: &Mutex<Info>, pid_juego: &AtomicU32) {
    // El detector vive fuera de cada sesión: si la captura se reinicia, el juego sigue detectado.
    // El juego elegido a mano solo cuenta en "Solo el juego"; en el resto, se detecta solo.
    let elegido = (cfg.modo == ModoCaptura::Juego).then(|| cfg.juego_elegido.as_ref().map(|j| j.exe.clone())).flatten();
    let mut detector = juego::Detector::new(elegido);
    let mut error_anterior = String::new();
    while !detener.load(Ordering::SeqCst) {
        match sesion_video(cfg, detener, buffer, info, &mut detector, pid_juego) {
            Ok(()) => break,
            Err(e) => {
                let texto = format!("{e:#}");
                let perdida = captura::es_captura_perdida(&e);
                if texto != error_anterior && !perdida {
                    log(&format!("video: {texto}"));
                    error_anterior = texto.clone();
                }
                // Perder la captura es normal (cambio de resolución, UAC, el juego cambió de monitor):
                // se vuelve enseguida y sin avisar.
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

/// Ayudas para probar caminos que en esta PC no se darían solos (también sirven para diagnosticar):
/// `KAIRO_CAPTURA=respaldo` fuerza Windows Graphics Capture y `KAIRO_PROCESADOR=1` el encoder por procesador.
fn forzado(variable: &str, valor: &str) -> bool {
    std::env::var(variable).is_ok_and(|v| v.eq_ignore_ascii_case(valor))
}

/// Grabar con el procesador cuesta mucho más: como máximo 1080p y 30 fps, para no quitarle al juego.
fn limitar_para_procesador(cfg: &Config) -> Config {
    let mut c = cfg.clone();
    if matches!(c.resolucion, Resolucion::Nativa | Resolucion::P1440) {
        c.resolucion = Resolucion::P1080;
    }
    c.fps = 30;
    c
}

/// Cuántos cuadros pueden esperar a que el encoder los pida (si se llena, se descarta el más viejo).
const COLA_MAXIMA: usize = 4;
/// Cada cuánto se mira qué hay adelante (250 ms).
const CADA_CUANTO_DETECTAR: i64 = SEGUNDO / 4;

/// El monitor que hay que grabar: en "Solo el juego", o con el monitor en automático, el del juego (si
/// hay uno detectado); si no, el elegido o el principal.
fn monitor_deseado(cfg: &Config, detector: &juego::Detector) -> Option<isize> {
    let sigue_al_juego = cfg.modo == ModoCaptura::Juego || cfg.monitor.is_none();
    detector.juego().map(|j| j.monitor).filter(|&m| sigue_al_juego && m != 0)
}

fn sesion_video(
    cfg_pedida: &Config,
    detener: &AtomicBool,
    buffer: &Mutex<Buffer>,
    info: &Mutex<Info>,
    detector: &mut juego::Detector,
    pid_juego: &AtomicU32,
) -> Result<()> {
    detector.revisar();
    let deseado = monitor_deseado(cfg_pedida, detector);
    let monitor = match deseado {
        Some(h) => captura::buscar_monitor_por_handle(h)?,
        None => captura::buscar_monitor(cfg_pedida.monitor.as_deref())?,
    };
    let tarjeta = captura::tarjeta_para(cfg_pedida.tarjeta, &monitor);
    let respaldo = forzado("KAIRO_CAPTURA", "respaldo");
    fn tamano(c: &Config) -> impl Fn(u32, u32) -> (u32, u32) + '_ {
        move |w, h| c.tamano_clip((w, h))
    }
    let bitrate = |c: &Config, cap: &captura::Captura| (c.mbps((cap.ancho_monitor, cap.alto_monitor)) * 1_000_000.0) as u32;

    let mut cfg = cfg_pedida.clone();
    let mut cap = captura::Captura::abrir(&monitor, &tarjeta, respaldo, tamano(&cfg), cfg.fps)?;
    let hardware = if forzado("KAIRO_PROCESADOR", "1") {
        Err(anyhow!("forzado con KAIRO_PROCESADOR"))
    } else {
        encoder::Encoder::abrir(&cap.dispositivo, cap.luid, cap.ancho, cap.alto, cfg.fps, bitrate(&cfg, &cap))
    };
    let mut enc = match hardware {
        Ok(e) => codificador::Codificador::Hardware(e),
        Err(e) => {
            log(&format!("sin encoder por hardware ({e:#}): se graba con el procesador"));
            cfg = limitar_para_procesador(cfg_pedida);
            drop(cap);
            cap = captura::Captura::abrir(&monitor, &tarjeta, respaldo, tamano(&cfg), cfg.fps)?;
            let e = codificador::EncoderProcesador::abrir(&cap.dispositivo, cap.ancho, cap.alto, cfg.fps, bitrate(&cfg, &cap))?;
            // Que el juego tenga prioridad sobre la compresión.
            unsafe {
                use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
            }
            codificador::Codificador::Procesador(e)
        }
    };
    let (ancho, alto) = enc.tamano();
    let bitrate = bitrate(&cfg, &cap);
    let formato = FormatoVideo { ancho, alto, fps: cfg.fps, bitrate, encoder: enc.nombre().to_string() };
    let detalle = format!("{ancho}×{alto} a {} fps · {} · {} · {}", cfg.fps, enc.nombre(), cap.metodo, cap.nombre_tarjeta);
    log(&format!("grabando {} (monitor {} de {}×{})", detalle, monitor.id, cap.ancho_monitor, cap.alto_monitor));
    if let Ok(mut b) = buffer.lock() {
        b.poner_formato(formato);
    }
    poner_info(info, |i| {
        i.grabando = true;
        i.detalle = detalle;
        i.error = None;
        i.juego = detector.juego().map(|j| j.nombre.clone());
    });
    pid_juego.store(detector.juego().map(|j| j.pid).unwrap_or(0), Ordering::SeqCst);

    let solo_juego = cfg.modo == ModoCaptura::Juego;
    let dur = SEGUNDO / cfg.fps as i64;
    let inicio = reloj::ahora();
    let mut proxima_deteccion = inicio + CADA_CUANTO_DETECTAR;
    let mut n: i64 = 0;
    let mut paquetes = Vec::new();
    // Cuadros convertidos que esperan a que el encoder los pida: si está ocupado un momento, el cuadro
    // no se pierde.
    let mut cola = VecDeque::new();
    let mut descartados = 0u64;
    while !detener.load(Ordering::SeqCst) {
        let mut objetivo = inicio + n * dur;
        let ahora = reloj::ahora();
        if ahora - objetivo > 4 * dur {
            // Nos atrasamos (la PC estuvo muy cargada): se salta al presente en vez de acumular.
            n = (ahora - inicio) / dur;
            objetivo = inicio + n * dur;
        }
        reloj::dormir_hasta(objetivo);

        if objetivo >= proxima_deteccion {
            proxima_deteccion = objetivo + CADA_CUANTO_DETECTAR;
            let cambios = detector.revisar();
            if cambios.juego {
                pid_juego.store(detector.juego().map(|j| j.pid).unwrap_or(0), Ordering::SeqCst);
                poner_info(info, |i| i.juego = detector.juego().map(|j| j.nombre.clone()));
            }
            // El juego está en otro monitor: se vuelve a empezar la captura en ese.
            if (cambios.juego || cambios.monitor) && monitor_deseado(cfg_pedida, detector).is_some_and(|m| m != monitor.hmonitor.0 as isize) {
                log("el juego está en otro monitor: se cambia la captura");
                return Err(captura::CapturaPerdida.into());
            }
        }

        // "Solo el juego": si el juego no está adelante (Alt+Tab, minimizado, todavía no hay juego),
        // el cuadro va en negro y no se captura nada.
        if solo_juego && !detector.al_frente() {
            enc.atender(&mut paquetes)?;
            cola.push_back((cap.negro()?, objetivo));
        } else {
            cap.capturar()?;
            enc.atender(&mut paquetes)?;
            if cap.hay_cuadro() {
                cola.push_back((cap.convertir()?, objetivo));
            }
        }
        if cola.len() > COLA_MAXIMA {
            cola.pop_front();
            descartados += 1;
            if descartados.is_power_of_two() {
                log(&format!("video: el encoder no da abasto, {descartados} cuadros descartados"));
            }
        }
        while enc.puede_recibir() {
            let Some((tex, pts)) = cola.pop_front() else { break };
            enc.enviar(&tex, pts)?;
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

/// Volumen (None = no va) y nombre de cada pista del buffer, en su orden.
fn pistas_de_audio(cfg: &Config) -> (Vec<Option<f32>>, Vec<String>) {
    let vol = |activa: bool, v: u8| activa.then_some(v as f32 / 100.0);
    let todo = cfg.otros_sonidos == OtrosSonidos::Todo;
    let mut volumenes = vec![
        vol(cfg.audio_juego.activa, cfg.audio_juego.volumen),
        vol(todo && cfg.audio_sistema.activa, cfg.audio_sistema.volumen),
        vol(cfg.audio_mic.activa, cfg.audio_mic.volumen),
    ];
    let mut nombres = vec!["Juego".to_string(), "Resto del sistema".to_string(), "Micrófono".to_string()];
    for p in &cfg.programas_audio {
        volumenes.push(vol(!todo && p.activa, p.volumen));
        nombres.push(p.nombre.clone());
    }
    (volumenes, nombres)
}

/// Qué capturar para cada pista, según los ajustes, el juego detectado (pid 0 = ninguno) y los procesos
/// abiertos de cada programa elegido.
/// - Juego: solo el proceso del juego (si hay juego).
/// - "Todo lo demás": todo menos el juego (si hay juego) o todo (si no hay).
/// - "Solo los que elijas": cada programa (cada proceso raíz suyo) en su pista. El juego no se repite.
/// - Micrófono: la entrada predeterminada.
fn plan_de_audio(cfg: &Config, pid: u32, programas: &[Vec<u32>]) -> Vec<(usize, Origen)> {
    let mut plan = Vec::new();
    if cfg.audio_juego.activa && pid != 0 {
        plan.push((PISTA_JUEGO, Origen::Proceso { pid, incluir: true }));
    }
    match cfg.otros_sonidos {
        OtrosSonidos::Todo if cfg.audio_sistema.activa => {
            let origen = if pid != 0 { Origen::Proceso { pid, incluir: false } } else { Origen::SalidaCompleta };
            plan.push((PISTA_SISTEMA, origen));
        }
        OtrosSonidos::Todo => {}
        OtrosSonidos::Elegidos => {
            for (i, (p, pids)) in cfg.programas_audio.iter().zip(programas).enumerate() {
                if p.activa {
                    for &raiz in pids.iter().filter(|&&r| r != pid) {
                        plan.push((PISTA_PROGRAMAS + i, Origen::Proceso { pid: raiz, incluir: true }));
                    }
                }
            }
        }
    }
    if cfg.audio_mic.activa {
        plan.push((PISTA_MICROFONO, Origen::Microfono));
    }
    plan
}

struct FuenteActiva {
    pista: usize,
    nombre: String,
    origen: Origen,
    captura: Option<CapturaAudio>,
    /// Último error (para no repetirlo en el registro) y cuándo reintentar.
    error: Option<String>,
    reintentar: i64,
}

fn hilo_audio(cfg: &Config, detener: &AtomicBool, buffer: &Mutex<Buffer>, pid_juego: &AtomicU32) {
    let (_, nombres) = pistas_de_audio(cfg);
    let exes: Vec<String> = cfg.programas_audio.iter().map(|p| p.exe.clone()).collect();
    let buscar_programas = cfg.otros_sonidos == OtrosSonidos::Elegidos && !exes.is_empty();
    let mut pid = u32::MAX; // fuerza el primer armado
    let mut programas: Vec<Vec<u32>> = Vec::new();
    let mut fuentes: Vec<FuenteActiva> = Vec::new();
    let mut trozos = Vec::new();
    let mut vueltas = 0u64;
    // Sin captura por proceso (Windows 10 anterior a 2004) no se puede separar el juego: el resto del
    // sistema es "todo" y el juego no tiene pista propia.
    let sin_proceso = crate::equipo::compilacion_windows() < crate::equipo::COMPILACION_AUDIO_POR_PROCESO;
    if sin_proceso {
        log("audio: esta versión de Windows no separa el audio del juego");
    }
    while !detener.load(Ordering::SeqCst) {
        let actual = pid_juego.load(Ordering::SeqCst);
        // Cada 2 s (y al empezar): qué procesos de los programas elegidos están abiertos.
        let mut programas_cambiaron = false;
        if buscar_programas && !sin_proceso && vueltas % 200 == 0 {
            let abiertos = juego::procesos_raiz(&exes);
            if abiertos != programas {
                programas = abiertos;
                programas_cambiaron = true;
            }
        }
        if actual != pid || programas_cambiaron {
            pid = actual;
            let pid_plan = if sin_proceso { 0 } else { pid };
            // Las capturas que siguen sirviendo se conservan (así no se corta su audio).
            let mut anteriores = std::mem::take(&mut fuentes);
            fuentes = plan_de_audio(cfg, pid_plan, &programas)
                .into_iter()
                .map(|(pista, origen)| match anteriores.iter().position(|f| f.pista == pista && f.origen == origen) {
                    Some(k) => anteriores.swap_remove(k),
                    None => FuenteActiva {
                        pista,
                        nombre: nombres.get(pista).cloned().unwrap_or_default(),
                        origen,
                        captura: None,
                        error: None,
                        reintentar: 0,
                    },
                })
                .collect();
        }
        let ahora = reloj::ahora();
        for f in &mut fuentes {
            if f.captura.is_none() && ahora >= f.reintentar {
                match CapturaAudio::abrir(f.origen) {
                    Ok(c) => {
                        log(&format!("audio ({}): capturando {:?}", f.nombre, f.origen));
                        f.captura = Some(c);
                        f.error = None;
                    }
                    Err(e) => {
                        let texto = format!("{e:#}");
                        if f.error.as_ref() != Some(&texto) {
                            log(&format!("audio ({}): {texto}", f.nombre));
                        }
                        f.error = Some(texto);
                        f.reintentar = ahora + 2 * SEGUNDO;
                    }
                }
            }
            if let Some(c) = &mut f.captura {
                if let Err(e) = c.leer(&mut trozos) {
                    log(&format!("audio ({}): {e:#}", f.nombre));
                    f.captura = None;
                    f.reintentar = ahora + SEGUNDO;
                }
                if !trozos.is_empty() {
                    if let Ok(mut b) = buffer.lock() {
                        b.agregar_audio(f.pista, trozos.drain(..));
                    }
                }
            }
        }
        // Cada 2 s: si cambió la salida o el micrófono predeterminado, se pasa al nuevo.
        vueltas += 1;
        if vueltas % 200 == 0 {
            for f in &mut fuentes {
                let cambio = f
                    .captura
                    .as_ref()
                    .is_some_and(|c| !c.id().is_empty() && audio::dispositivo_predeterminado(f.origen).is_some_and(|id| id != c.id()));
                if cambio {
                    log(&format!("audio ({}): cambió el dispositivo predeterminado", f.nombre));
                    f.captura = None;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
