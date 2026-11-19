//! El reproductor de Kairo. Dos hilos:
//! - audio (y control): decodifica cada pista de audio, las mezcla con su volumen y las manda a los
//!   parlantes con WASAPI. Maneja reproducir, pausar y buscar, y lleva el reloj.
//! - video: decodifica los cuadros ya achicados (como mucho 1280×720) y los deja en una cola corta; la
//!   ventana muestra el que corresponde según el reloj.
//!
//! El volumen de cada pista se puede cambiar mientras suena: así el editor deja escuchar el clip sin
//! Discord, por ejemplo, antes de exportarlo.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

use super::{bytes_de, ir_a, FRECUENCIA, SEGUNDO};

/// Lado más grande del video que se muestra (la ventana no necesita más).
const ANCHO_MAXIMO: u32 = 1280;
const ALTO_MAXIMO: u32 = 720;
/// Cuadros decodificados por adelantado.
const COLA_VIDEO: usize = 4;

pub struct Cuadro {
    /// Segundos desde el inicio del clip.
    pub t: f64,
    pub ancho: usize,
    pub alto: usize,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Reloj {
    base: f64,
    desde: Option<Instant>,
}

impl Reloj {
    fn ahora(&self) -> f64 {
        self.base + self.desde.map(|d| d.elapsed().as_secs_f64()).unwrap_or(0.0)
    }
}

struct Compartido {
    salir: AtomicBool,
    reproduciendo: AtomicBool,
    reloj: Mutex<Reloj>,
    cuadros: Mutex<VecDeque<Cuadro>>,
    /// Búsqueda pendiente para el hilo de video (100 ns; -1 = ninguna).
    buscar_video: AtomicI64,
    /// Después de una búsqueda: mostrar el primer cuadro que llegue, aunque esté en pausa.
    mostrar_siguiente: AtomicBool,
    /// Volumen de cada pista de audio del archivo (mismo orden).
    volumenes: Mutex<Vec<f32>>,
    duracion: f64,
    /// Para pedirle a la ventana que se vuelva a dibujar.
    avisar: Box<dyn Fn() + Send + Sync>,
}

impl Compartido {
    fn posicion(&self) -> f64 {
        self.reloj.lock().map(|r| r.ahora()).unwrap_or(0.0).clamp(0.0, self.duracion)
    }

    fn poner_reloj(&self, base: f64, corriendo: bool) {
        if let Ok(mut r) = self.reloj.lock() {
            *r = Reloj { base, desde: corriendo.then(Instant::now) };
        }
    }
}

enum Orden {
    Reproducir,
    Pausar,
    Buscar(f64),
}

pub struct Reproductor {
    c: Arc<Compartido>,
    ordenes: Sender<Orden>,
    hilos: Vec<JoinHandle<()>>,
    pub info: super::InfoClip,
}

impl Reproductor {
    /// Abre el clip (en pausa, mostrando el primer cuadro). `volumenes`: el de cada pista de audio.
    pub fn abrir(ruta: &Path, volumenes: Vec<f32>, avisar: impl Fn() + Send + Sync + 'static) -> Result<Self> {
        let info = super::info(ruta)?;
        if info.ancho == 0 {
            return Err(anyhow!("el archivo no tiene video"));
        }
        let c = Arc::new(Compartido {
            salir: AtomicBool::new(false),
            reproduciendo: AtomicBool::new(false),
            reloj: Mutex::new(Reloj { base: 0.0, desde: None }),
            cuadros: Mutex::new(VecDeque::new()),
            buscar_video: AtomicI64::new(-1),
            mostrar_siguiente: AtomicBool::new(true),
            volumenes: Mutex::new(volumenes),
            duracion: info.duracion,
            avisar: Box::new(avisar),
        });
        let (tx, rx) = channel();
        let mut hilos = Vec::new();
        {
            let (c, ruta) = (c.clone(), ruta.to_path_buf());
            hilos.push(std::thread::spawn(move || hilo_video(&ruta, &c)));
        }
        {
            let (c, ruta, pistas) = (c.clone(), ruta.to_path_buf(), info.pistas_audio);
            hilos.push(std::thread::spawn(move || hilo_audio(&ruta, &c, rx, pistas)));
        }
        Ok(Self { c, ordenes: tx, hilos, info })
    }

    pub fn reproducir(&self) {
        let _ = self.ordenes.send(Orden::Reproducir);
    }

    pub fn pausar(&self) {
        let _ = self.ordenes.send(Orden::Pausar);
    }

    pub fn reproduciendo(&self) -> bool {
        self.c.reproduciendo.load(Ordering::SeqCst)
    }

    pub fn alternar(&self) {
        if self.reproduciendo() {
            self.pausar();
        } else {
            self.reproducir();
        }
    }

    pub fn buscar(&self, t: f64) {
        // El reloj se mueve ya, para que la barra responda al instante.
        let t = t.clamp(0.0, self.info.duracion);
        self.c.poner_reloj(t, false);
        let _ = self.ordenes.send(Orden::Buscar(t));
    }

    /// Segundos desde el inicio.
    pub fn posicion(&self) -> f64 {
        self.c.posicion()
    }

    pub fn poner_volumenes(&self, v: Vec<f32>) {
        if let Ok(mut x) = self.c.volumenes.lock() {
            *x = v;
        }
    }

    /// El cuadro que toca mostrar ahora, si cambió.
    pub fn tomar_cuadro(&self) -> Option<Cuadro> {
        let mut cola = self.c.cuadros.lock().ok()?;
        if self.c.mostrar_siguiente.load(Ordering::SeqCst) {
            let f = cola.pop_front()?;
            self.c.mostrar_siguiente.store(false, Ordering::SeqCst);
            return Some(f);
        }
        let p = self.posicion();
        let mut elegido = None;
        while cola.front().is_some_and(|f| f.t <= p + 0.004) {
            elegido = cola.pop_front();
        }
        elegido
    }
}

impl Drop for Reproductor {
    fn drop(&mut self) {
        self.c.salir.store(true, Ordering::SeqCst);
        for h in self.hilos.drain(..) {
            let _ = h.join();
        }
    }
}

// --- Video ------------------------------------------------------------------------------------

/// Abre el video ya convertido a RGB32 y achicado. Devuelve (lector, flujo, ancho, alto, stride).
unsafe fn abrir_video(ruta: &Path) -> Result<(IMFSourceReader, u32, usize, usize, isize)> {
    let l = super::lector(ruta, true)?;
    let (video, _) = super::flujos(&l);
    let flujo = video.ok_or_else(|| anyhow!("sin video"))?;
    l.SetStreamSelection(flujo, true)?;
    let nativo = l.GetNativeMediaType(flujo, 0)?.GetUINT64(&MF_MT_FRAME_SIZE)?;
    let (w, h) = ((nativo >> 32) as f64, (nativo & 0xFFFF_FFFF) as f64);
    let escala = (ANCHO_MAXIMO as f64 / w).min(ALTO_MAXIMO as f64 / h).min(1.0);
    let par = |x: f64| ((x / 2.0).round() as u64 * 2).max(2);
    let tipo = MFCreateMediaType()?;
    tipo.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
    tipo.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
    tipo.SetUINT64(&MF_MT_FRAME_SIZE, (par(w * escala) << 32) | par(h * escala))?;
    if l.SetCurrentMediaType(flujo, None, &tipo).is_err() {
        // Si no se puede achicar, al tamaño original.
        let _ = tipo.DeleteItem(&MF_MT_FRAME_SIZE);
        l.SetCurrentMediaType(flujo, None, &tipo)?;
    }
    let actual = l.GetCurrentMediaType(flujo)?;
    let s = actual.GetUINT64(&MF_MT_FRAME_SIZE)?;
    let (aw, ah) = ((s >> 32) as usize, (s & 0xFFFF_FFFF) as usize);
    let stride = actual.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|v| v as i32 as isize).unwrap_or(aw as isize * 4);
    Ok((l, flujo, aw, ah, stride))
}

/// BGRX (RGB32 de Windows) a RGBA, respetando el stride (negativo = la imagen viene de abajo hacia arriba).
fn a_rgba(datos: &[u8], ancho: usize, alto: usize, stride: isize) -> Option<Vec<u8>> {
    let paso = stride.unsigned_abs();
    if paso < ancho * 4 || datos.len() < paso * (alto - 1) + ancho * 4 {
        return None;
    }
    let mut rgba = vec![0u8; ancho * alto * 4];
    for y in 0..alto {
        let fila = if stride < 0 { alto - 1 - y } else { y };
        let origen = &datos[fila * paso..fila * paso + ancho * 4];
        let destino = &mut rgba[y * ancho * 4..(y + 1) * ancho * 4];
        for (d, o) in destino.chunks_exact_mut(4).zip(origen.chunks_exact(4)) {
            d[0] = o[2];
            d[1] = o[1];
            d[2] = o[0];
            d[3] = 255;
        }
    }
    Some(rgba)
}

fn hilo_video(ruta: &Path, c: &Compartido) {
    super::iniciar();
    let Ok((l, flujo, ancho, alto, stride)) = (unsafe { abrir_video(ruta) }) else {
        crate::registro::log(&format!("reproductor: no se pudo abrir el video de {}", ruta.display()));
        return;
    };
    let mut descartar_hasta: Option<f64> = None;
    let mut fin = false;
    while !c.salir.load(Ordering::SeqCst) {
        let pedido = c.buscar_video.swap(-1, Ordering::SeqCst);
        if pedido >= 0 {
            unsafe {
                let _ = ir_a(&l, pedido);
            }
            if let Ok(mut q) = c.cuadros.lock() {
                q.clear();
            }
            descartar_hasta = Some(pedido as f64 / SEGUNDO as f64);
            c.mostrar_siguiente.store(true, Ordering::SeqCst);
            fin = false;
        }
        let lleno = c.cuadros.lock().map(|q| q.len() >= COLA_VIDEO).unwrap_or(true);
        if lleno || fin {
            std::thread::sleep(Duration::from_millis(4));
            continue;
        }
        let (mut flags, mut ts, mut muestra) = (0u32, 0i64, None);
        if unsafe { l.ReadSample(flujo, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut muestra)) }.is_err() {
            fin = true;
            continue;
        }
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            fin = true;
            continue;
        }
        let Some(m) = muestra else { continue };
        let t = ts as f64 / SEGUNDO as f64;
        // Después de buscar se decodifica desde el cuadro clave anterior: esos no se muestran.
        if let Some(d) = descartar_hasta {
            if t < d - 0.008 {
                continue;
            }
            descartar_hasta = None;
        }
        let Some(rgba) = unsafe { bytes_de(&m) }.ok().and_then(|b| a_rgba(&b, ancho, alto, stride)) else { continue };
        if let Ok(mut q) = c.cuadros.lock() {
            q.push_back(Cuadro { t, ancho, alto, rgba });
        }
        if c.mostrar_siguiente.load(Ordering::SeqCst) {
            (c.avisar)();
        }
    }
}

// --- Audio --------------------------------------------------------------------------------------

/// Una pista de audio del archivo, decodificada de a poco.
struct PistaAudio {
    l: IMFSourceReader,
    flujo: u32,
    /// Muestras estéreo intercaladas, de -1 a 1.
    cola: VecDeque<f32>,
    fin: bool,
    /// Después de buscar: lo que llegue antes de este instante (100 ns) se descarta.
    desde: Option<i64>,
}

impl PistaAudio {
    unsafe fn abrir(ruta: &Path, indice: usize) -> Result<Self> {
        let l = super::lector(ruta, false)?;
        let (_, audio) = super::flujos(&l);
        let flujo = *audio.get(indice).ok_or_else(|| anyhow!("no hay pista {indice}"))?;
        super::pedir_pcm(&l, flujo)?;
        Ok(Self { l, flujo, cola: VecDeque::new(), fin: false, desde: None })
    }

    fn buscar(&mut self, t: f64) {
        let t = (t * SEGUNDO as f64) as i64;
        unsafe {
            let _ = ir_a(&self.l, t);
        }
        self.cola.clear();
        self.fin = false;
        self.desde = Some(t);
    }

    /// Decodifica hasta tener `cuadros` cuadros (o hasta el final).
    fn llenar(&mut self, cuadros: usize) {
        while self.cola.len() < cuadros * 2 && !self.fin {
            let (mut flags, mut ts, mut muestra) = (0u32, 0i64, None);
            if unsafe { self.l.ReadSample(self.flujo, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut muestra)) }.is_err()
                || flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0
            {
                self.fin = true;
                break;
            }
            let Some(m) = muestra else { continue };
            let Ok(b) = (unsafe { bytes_de(&m) }) else { continue };
            let mut muestras: Vec<f32> = b.chunks_exact(2).map(|x| i16::from_le_bytes([x[0], x[1]]) as f32 / 32768.0).collect();
            if let Some(desde) = self.desde.take() {
                // El primer trozo puede empezar antes de lo pedido: se corta justo en el instante pedido.
                let sobra = ((desde - ts).max(0) as i128 * FRECUENCIA as i128 / SEGUNDO as i128) as usize * 2;
                muestras.drain(..sobra.min(muestras.len()));
            }
            self.cola.extend(muestras);
        }
    }
}

/// La salida de audio (los parlantes predeterminados), en float estéreo a 48 kHz.
struct Salida {
    cliente: IAudioClient,
    render: IAudioRenderClient,
    total: u32,
}

impl Salida {
    unsafe fn abrir() -> Result<Self> {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let disp = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let cliente: IAudioClient = disp.Activate(CLSCTX_ALL, None)?;
        let fmt = WAVEFORMATEX {
            wFormatTag: 3, // float
            nChannels: 2,
            nSamplesPerSec: FRECUENCIA,
            nAvgBytesPerSec: FRECUENCIA * 8,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        let flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        // 150 ms de margen.
        cliente.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 1_500_000, 0, &fmt, None)?;
        let total = cliente.GetBufferSize()?;
        let render: IAudioRenderClient = cliente.GetService()?;
        Ok(Self { cliente, render, total })
    }

    unsafe fn libre(&self) -> usize {
        self.total.saturating_sub(self.cliente.GetCurrentPadding().unwrap_or(self.total)) as usize
    }

    unsafe fn escribir(&self, muestras: &[f32]) {
        let cuadros = (muestras.len() / 2) as u32;
        if cuadros == 0 {
            return;
        }
        if let Ok(p) = self.render.GetBuffer(cuadros) {
            std::ptr::copy_nonoverlapping(muestras.as_ptr(), p as *mut f32, muestras.len());
            let _ = self.render.ReleaseBuffer(cuadros, 0);
        }
    }

    unsafe fn arrancar(&self) {
        let _ = self.cliente.Start();
    }

    /// Para y tira lo que quedaba en el buffer.
    unsafe fn parar(&self) {
        let _ = self.cliente.Stop();
        let _ = self.cliente.Reset();
    }
}

/// Mezcla `cuadros` cuadros de todas las pistas con su volumen.
fn mezclar(pistas: &mut [PistaAudio], volumenes: &[f32], cuadros: usize) -> Vec<f32> {
    let mut salida = vec![0f32; cuadros * 2];
    for (i, p) in pistas.iter_mut().enumerate() {
        let vol = volumenes.get(i).copied().unwrap_or(0.0);
        p.llenar(cuadros);
        let n = (cuadros * 2).min(p.cola.len());
        if vol > 0.0 {
            for (s, x) in salida.iter_mut().zip(p.cola.iter()) {
                *s += x * vol;
            }
        }
        p.cola.drain(..n);
    }
    for s in &mut salida {
        *s = s.clamp(-1.0, 1.0);
    }
    salida
}

fn hilo_audio(ruta: &Path, c: &Compartido, ordenes: Receiver<Orden>, cantidad: usize) {
    super::iniciar();
    let mut pistas: Vec<PistaAudio> = (0..cantidad).filter_map(|i| unsafe { PistaAudio::abrir(ruta, i) }.ok()).collect();
    // Sin parlantes (o si falla), el reloj sigue igual: el video se ve sin sonido.
    let salida = unsafe { Salida::abrir() }.map_err(|e| crate::registro::log(&format!("reproductor: sin salida de audio ({e})"))).ok();
    let mut reproduciendo = false;
    // Después de llegar al final, "reproducir" vuelve a empezar.
    let mut al_final = false;

    let prellenar = |pistas: &mut [PistaAudio]| {
        if let Some(s) = &salida {
            let vol = c.volumenes.lock().map(|v| v.clone()).unwrap_or_default();
            unsafe {
                let libre = s.libre();
                s.escribir(&mezclar(pistas, &vol, libre));
            }
        }
    };
    let buscar_todo = |pistas: &mut [PistaAudio], t: f64| {
        for p in pistas.iter_mut() {
            p.buscar(t);
        }
        c.buscar_video.store((t * SEGUNDO as f64) as i64, Ordering::SeqCst);
    };

    while !c.salir.load(Ordering::SeqCst) {
        let espera = if reproduciendo { Duration::from_millis(5) } else { Duration::from_millis(50) };
        match ordenes.recv_timeout(espera) {
            Ok(Orden::Reproducir) if !reproduciendo => {
                let mut desde = c.posicion();
                if al_final || desde >= c.duracion - 0.05 {
                    desde = 0.0;
                    buscar_todo(&mut pistas, 0.0);
                }
                al_final = false;
                prellenar(&mut pistas);
                if let Some(s) = &salida {
                    unsafe { s.arrancar() };
                }
                c.poner_reloj(desde, true);
                reproduciendo = true;
            }
            Ok(Orden::Pausar) if reproduciendo => {
                let t = c.posicion();
                if let Some(s) = &salida {
                    unsafe { s.parar() };
                }
                c.poner_reloj(t, false);
                reproduciendo = false;
                // Lo decodificado de más ya no sirve: el audio vuelve justo a donde quedó.
                for p in pistas.iter_mut() {
                    p.buscar(t);
                }
            }
            Ok(Orden::Buscar(t)) => {
                if let Some(s) = &salida {
                    if reproduciendo {
                        unsafe { s.parar() };
                    }
                }
                al_final = false;
                buscar_todo(&mut pistas, t);
                if reproduciendo {
                    prellenar(&mut pistas);
                    if let Some(s) = &salida {
                        unsafe { s.arrancar() };
                    }
                }
                c.poner_reloj(t, reproduciendo);
            }
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        c.reproduciendo.store(reproduciendo, Ordering::SeqCst);
        if !reproduciendo {
            continue;
        }
        if let Some(s) = &salida {
            let vol = c.volumenes.lock().map(|v| v.clone()).unwrap_or_default();
            unsafe {
                let libre = s.libre();
                if libre > 0 {
                    s.escribir(&mezclar(&mut pistas, &vol, libre));
                }
            }
        }
        if c.posicion() >= c.duracion {
            if let Some(s) = &salida {
                unsafe { s.parar() };
            }
            c.poner_reloj(c.duracion, false);
            reproduciendo = false;
            al_final = true;
            c.reproduciendo.store(false, Ordering::SeqCst);
            (c.avisar)();
        }
    }
    if let Some(s) = &salida {
        unsafe { s.parar() };
    }
}
