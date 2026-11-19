//! Biblioteca de clips: busca los MP4 en la carpeta de clips (y en las carpetas de cada juego), genera
//! las miniaturas en un hilo aparte (con caché en %APPDATA%\Kairo\miniaturas) y los muestra en una
//! cuadrícula con filtros. Todo vive en la ventana de Kairo: mientras está cerrada no usa nada.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

use crate::config::datos_dir;
use crate::datos_clip;

/// Tamaño de las miniaturas (16:9).
pub const MINI_ANCHO: usize = 384;
pub const MINI_ALTO: usize = 216;

#[derive(Clone)]
pub struct Clip {
    pub ruta: PathBuf,
    pub nombre: String,
    pub juego: Option<String>,
    pub tamano: u64,
    pub modificado: SystemTime,
    pub duracion: Option<f64>,
    /// Clip de Kairo de antes de la 0.7.2 con pistas separadas: hay que marcar la mezcla (ver `pistas_mp4`).
    reparar: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Orden {
    Nuevos,
    Viejos,
    Grandes,
}

enum Miniatura {
    Pedida,
    Lista(TextureHandle),
    Fallo,
}

struct Resultado {
    ruta: PathBuf,
    imagen: Option<ColorImage>,
    duracion: Option<f64>,
}

pub struct Biblioteca {
    pub clips: Vec<Clip>,
    carpeta: PathBuf,
    proximo_escaneo: f64,
    miniaturas: HashMap<PathBuf, Miniatura>,
    pedir: Sender<PathBuf>,
    recibir: Receiver<Resultado>,
    pub filtro_juego: Option<String>,
    pub busqueda: String,
    pub orden: Orden,
    /// Clip que se está renombrando (con el nombre nuevo) o por mandar a la Papelera.
    pub renombrando: Option<(PathBuf, String)>,
    pub borrando: Option<PathBuf>,
    /// Clips que ya se mandaron a reparar en esta sesión.
    reparados: HashSet<PathBuf>,
}

impl Biblioteca {
    pub fn new(ctx: &egui::Context) -> Self {
        let (pedir, pedidos) = mpsc::channel::<PathBuf>();
        let (enviar, recibir) = mpsc::channel::<Resultado>();
        let ctx = ctx.clone();
        std::thread::spawn(move || hilo_miniaturas(pedidos, enviar, ctx));
        Self {
            clips: Vec::new(),
            carpeta: PathBuf::new(),
            proximo_escaneo: 0.0,
            miniaturas: HashMap::new(),
            pedir,
            recibir,
            filtro_juego: None,
            busqueda: String::new(),
            orden: Orden::Nuevos,
            renombrando: None,
            borrando: None,
            reparados: HashSet::new(),
        }
    }

    /// Vuelve a mirar la carpeta cada 4 s (o enseguida si cambió la carpeta o se pidió).
    pub fn actualizar(&mut self, carpeta: &Path, ahora: f64, ctx: &egui::Context) {
        while let Ok(r) = self.recibir.try_recv() {
            let m = match r.imagen {
                Some(img) => Miniatura::Lista(ctx.load_texture(r.ruta.display().to_string(), img, TextureOptions::LINEAR)),
                None => Miniatura::Fallo,
            };
            self.miniaturas.insert(r.ruta.clone(), m);
            if let (Some(d), Some(c)) = (r.duracion, self.clips.iter_mut().find(|c| c.ruta == r.ruta)) {
                c.duracion.get_or_insert(d);
            }
        }
        if carpeta != self.carpeta || ahora >= self.proximo_escaneo {
            if carpeta != self.carpeta {
                self.carpeta = carpeta.to_path_buf();
                self.filtro_juego = None;
            }
            self.escanear();
            self.proximo_escaneo = ahora + 4.0;
        }
    }

    pub fn escanear_ya(&mut self) {
        self.proximo_escaneo = 0.0;
    }

    fn escanear(&mut self) {
        let mut clips = Vec::new();
        buscar_clips(&self.carpeta, &self.carpeta, 0, &mut clips);
        // Lo que ya se sabía (duración por el video) se conserva entre escaneos.
        for c in &mut clips {
            if c.duracion.is_none() {
                c.duracion = self.clips.iter().find(|v| v.ruta == c.ruta).and_then(|v| v.duracion);
            }
        }
        let rutas: HashSet<&PathBuf> = clips.iter().map(|c| &c.ruta).collect();
        self.miniaturas.retain(|r, _| rutas.contains(r));
        // Los clips viejos con pistas separadas: la mezcla se marca para que VLC y los celulares la toquen (en
        // otro hilo; son unos bytes por archivo).
        let reparados = &mut self.reparados;
        let pendientes: Vec<PathBuf> = clips.iter().filter(|c| c.reparar && reparados.insert(c.ruta.clone())).map(|c| c.ruta.clone()).collect();
        if !pendientes.is_empty() {
            std::thread::spawn(move || reparar_mezcla(&pendientes));
        }
        self.clips = clips;
    }

    /// La miniatura del clip, si ya está. La pide (una sola vez) si todavía no se pidió.
    pub fn miniatura(&mut self, ruta: &Path) -> Option<&TextureHandle> {
        if !self.miniaturas.contains_key(ruta) {
            self.miniaturas.insert(ruta.to_path_buf(), Miniatura::Pedida);
            let _ = self.pedir.send(ruta.to_path_buf());
        }
        match self.miniaturas.get(ruta) {
            Some(Miniatura::Lista(t)) => Some(t),
            _ => None,
        }
    }

    /// Miniaturas pedidas que todavía no llegaron.
    pub fn pendientes(&self) -> usize {
        self.miniaturas.values().filter(|m| matches!(m, Miniatura::Pedida)).count()
    }

    pub fn fallo_miniatura(&self, ruta: &Path) -> bool {
        matches!(self.miniaturas.get(ruta), Some(Miniatura::Fallo))
    }

    /// Juegos con clips, de más clips a menos: (juego, cantidad).
    pub fn juegos(&self) -> Vec<(String, usize)> {
        let mut cuenta: HashMap<&str, usize> = HashMap::new();
        for c in &self.clips {
            if let Some(j) = &c.juego {
                *cuenta.entry(j.as_str()).or_default() += 1;
            }
        }
        let mut v: Vec<(String, usize)> = cuenta.into_iter().map(|(j, n)| (j.to_string(), n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        v
    }

    /// Los clips que se ven con el filtro, la búsqueda y el orden elegidos.
    pub fn visibles(&self) -> Vec<Clip> {
        let busqueda = self.busqueda.trim().to_lowercase();
        let mut v: Vec<Clip> = self
            .clips
            .iter()
            .filter(|c| self.filtro_juego.is_none() || c.juego == self.filtro_juego)
            .filter(|c| {
                busqueda.is_empty()
                    || c.nombre.to_lowercase().contains(&busqueda)
                    || c.juego.as_deref().is_some_and(|j| j.to_lowercase().contains(&busqueda))
            })
            .cloned()
            .collect();
        match self.orden {
            Orden::Nuevos => v.sort_by(|a, b| b.modificado.cmp(&a.modificado)),
            Orden::Viejos => v.sort_by(|a, b| a.modificado.cmp(&b.modificado)),
            Orden::Grandes => v.sort_by(|a, b| b.tamano.cmp(&a.tamano)),
        }
        v
    }

    /// Cambia el nombre del clip (y de sus datos). Devuelve un error dicho para la persona.
    pub fn renombrar(&mut self, ruta: &Path, nuevo: &str) -> Result<(), String> {
        let nuevo = crate::grabadora::nombre_para_carpeta(nuevo.trim());
        let destino = ruta.with_file_name(format!("{nuevo}.mp4"));
        if destino == ruta {
            return Ok(());
        }
        if destino.exists() {
            return Err(format!("Ya hay un clip que se llama «{nuevo}»."));
        }
        std::fs::rename(ruta, &destino).map_err(|e| format!("No se pudo renombrar: {e}"))?;
        let _ = std::fs::rename(datos_clip::ruta_datos(ruta), datos_clip::ruta_datos(&destino));
        self.escanear();
        Ok(())
    }

    /// Manda el clip (y sus datos) a la Papelera.
    pub fn a_la_papelera(&mut self, ruta: &Path) -> Result<(), String> {
        papelera(&[ruta.to_path_buf(), datos_clip::ruta_datos(ruta)])?;
        self.escanear();
        Ok(())
    }
}

/// Busca MP4 en la carpeta y en sus subcarpetas (las de cada juego), hasta 2 niveles.
fn buscar_clips(raiz: &Path, dir: &Path, nivel: u32, salida: &mut Vec<Clip>) {
    let Ok(entradas) = std::fs::read_dir(dir) else { return };
    for e in entradas.flatten() {
        let ruta = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            if nivel < 2 {
                buscar_clips(raiz, &ruta, nivel + 1, salida);
            }
            continue;
        }
        if !ruta.extension().is_some_and(|x| x.eq_ignore_ascii_case("mp4")) {
            continue;
        }
        let datos = datos_clip::leer(&ruta);
        // El juego: el de sus datos; si no hay, la carpeta donde está (si no es la carpeta de clips).
        let juego = datos.as_ref().and_then(|d| d.juego.clone()).or_else(|| {
            let padre = ruta.parent()?;
            (padre != raiz).then(|| padre.file_name().map(|n| n.to_string_lossy().into_owned())).flatten()
        });
        salida.push(Clip {
            nombre: ruta.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            juego,
            tamano: meta.len(),
            modificado: meta.modified().unwrap_or(UNIX_EPOCH),
            reparar: datos.as_ref().is_some_and(|d| d.pistas.len() > 1 && !d.mezcla_marcada),
            duracion: datos.map(|d| d.duracion).filter(|d| *d > 0.0),
            ruta,
        });
    }
}

/// Marca la mezcla en clips de antes de la 0.7.2 y lo anota en sus datos. Si el clip está abierto en otro
/// programa no se puede: se vuelve a intentar la próxima vez que se abra Kairo.
fn reparar_mezcla(rutas: &[PathBuf]) {
    for ruta in rutas {
        match crate::pistas_mp4::marcar_mezcla(ruta) {
            Ok(_) => {
                if let Some(mut d) = datos_clip::leer(ruta) {
                    d.mezcla_marcada = true;
                    datos_clip::escribir(ruta, &d);
                }
                crate::registro::log(&format!("mezcla marcada en {}", ruta.display()));
            }
            Err(e) => crate::registro::log(&format!("no se pudo marcar la mezcla en {}: {e:#}", ruta.display())),
        }
    }
}

/// "hace 5 min", "hace 2 h", "ayer", "18/11/2026".
pub fn hace_cuanto(t: SystemTime) -> String {
    let s = SystemTime::now().duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    match s {
        0..=59 => "hace un momento".into(),
        60..=3599 => format!("hace {} min", s / 60),
        3600..=86399 => format!("hace {} h", s / 3600),
        86400..=172799 => "ayer".into(),
        172800..=604799 => format!("hace {} días", s / 86400),
        _ => {
            // Fecha local del archivo.
            let segundos = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            fecha_local(segundos)
        }
    }
}

fn fecha_local(segundos: u64) -> String {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let t = (segundos + 11_644_473_600) * 10_000_000;
    let ft = FILETIME { dwLowDateTime: t as u32, dwHighDateTime: (t >> 32) as u32 };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    unsafe {
        let _ = FileTimeToSystemTime(&ft, &mut utc);
        if SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            local = utc;
        }
    }
    format!("{:02}/{:02}/{}", local.wDay, local.wMonth, local.wYear)
}

/// "0:45", "1:30", "10:05".
pub fn texto_duracion(s: f64) -> String {
    let s = s.round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn papelera(rutas: &[PathBuf]) -> Result<(), String> {
    use windows::Win32::UI::Shell::{SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE, SHFILEOPSTRUCTW};
    // Lista de rutas separadas por \0 y terminada en \0\0 (solo las que existen).
    let mut desde: Vec<u16> = Vec::new();
    for r in rutas.iter().filter(|r| r.exists()) {
        desde.extend(r.as_os_str().to_string_lossy().encode_utf16());
        desde.push(0);
    }
    if desde.is_empty() {
        return Ok(());
    }
    desde.push(0);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: windows::core::PCWSTR(desde.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI).0 as u16,
        ..Default::default()
    };
    let r = unsafe { SHFileOperationW(&mut op) };
    if r == 0 && !op.fAnyOperationsAborted.as_bool() {
        Ok(())
    } else {
        Err(format!("No se pudo mandar a la Papelera (código {r})."))
    }
}

// --- Miniaturas --------------------------------------------------------------------------------

fn ruta_cache(clip: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(clip).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    clip.hash(&mut h);
    meta.len().hash(&mut h);
    meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs().hash(&mut h);
    Some(datos_dir().join("miniaturas").join(format!("{:016x}.png", h.finish())))
}

fn hilo_miniaturas(pedidos: Receiver<PathBuf>, enviar: Sender<Resultado>, ctx: egui::Context) {
    unsafe {
        use windows::Win32::Media::MediaFoundation::{MFStartup, MFSTARTUP_FULL, MF_VERSION};
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
        let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    }
    while let Ok(ruta) = pedidos.recv() {
        let cache = ruta_cache(&ruta);
        // La duración se guarda al lado de la miniatura ("<hash>.dur"): los clips viejos no tienen datos.
        let leer_dur = |c: &PathBuf| std::fs::read_to_string(c.with_extension("dur")).ok().and_then(|s| s.trim().parse::<f64>().ok());
        let desde_cache = cache.as_ref().and_then(|c| leer_png(c).map(|img| (img, leer_dur(c))));
        let (imagen, duracion) = match desde_cache {
            Some((img, dur)) => (Some(img), dur),
            None => match generar(&ruta) {
                Some((img, dur)) => {
                    if let Some(c) = &cache {
                        let _ = std::fs::create_dir_all(c.parent().unwrap_or(Path::new(".")));
                        let _ = guardar_png(c, &img);
                        let _ = std::fs::write(c.with_extension("dur"), format!("{dur}"));
                    }
                    (Some(img), Some(dur))
                }
                None => (None, None),
            },
        };
        if enviar.send(Resultado { ruta, imagen, duracion }).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// Decodifica un cuadro del clip (a un tercio, como mucho a 2 s) y lo achica a la miniatura.
fn generar(ruta: &Path) -> Option<(ColorImage, f64)> {
    use windows::core::{HSTRING, PROPVARIANT};
    use windows::Win32::Media::MediaFoundation::*;
    unsafe {
        let mut atributos = None;
        MFCreateAttributes(&mut atributos, 1).ok()?;
        let atributos = atributos?;
        atributos.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1).ok()?;
        let lector = MFCreateSourceReaderFromURL(&HSTRING::from(ruta.as_os_str()), &atributos).ok()?;
        let duracion = lector
            .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
            .ok()
            .and_then(|p| u64::try_from(&p).ok())
            .unwrap_or(0) as f64
            / 1e7;
        let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let _ = lector.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
        lector.SetStreamSelection(video, true).ok()?;
        let rgb = MFCreateMediaType().ok()?;
        rgb.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).ok()?;
        rgb.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).ok()?;
        lector.SetCurrentMediaType(video, None, &rgb).ok()?;
        let tam = lector.GetCurrentMediaType(video).ok()?.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
        let (w, h) = ((tam >> 32) as usize, (tam & 0xFFFF_FFFF) as usize);
        let objetivo = ((duracion / 3.0).min(2.0) * 1e7) as i64;
        if objetivo > 0 {
            let _ = lector.SetCurrentPosition(&windows::core::GUID::zeroed(), &PROPVARIANT::from(objetivo));
        }
        for _ in 0..240 {
            let (mut flags, mut ts, mut muestra) = (0u32, 0i64, None);
            lector.ReadSample(video, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut muestra)).ok()?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return None;
            }
            let Some(m) = muestra else { continue };
            if ts < objetivo - 200_000 {
                continue; // la búsqueda cae en el cuadro clave anterior: se avanza hasta el pedido
            }
            let b = m.ConvertToContiguousBuffer().ok()?;
            let mut p = std::ptr::null_mut();
            let mut largo = 0u32;
            b.Lock(&mut p, None, Some(&mut largo)).ok()?;
            let bgra = std::slice::from_raw_parts(p, largo as usize);
            let img = achicar(bgra, w, h);
            let _ = b.Unlock();
            return img.map(|i| (i, duracion));
        }
        None
    }
}

/// Achica un cuadro BGRA a la miniatura (promedio por bloques) y lo pasa a RGBA.
fn achicar(bgra: &[u8], w: usize, h: usize) -> Option<ColorImage> {
    if w == 0 || h == 0 || bgra.len() < w * h * 4 {
        return None;
    }
    let mut px = vec![0u8; MINI_ANCHO * MINI_ALTO * 4];
    for y in 0..MINI_ALTO {
        let (y0, y1) = (y * h / MINI_ALTO, ((y + 1) * h / MINI_ALTO).max(y * h / MINI_ALTO + 1));
        for x in 0..MINI_ANCHO {
            let (x0, x1) = (x * w / MINI_ANCHO, ((x + 1) * w / MINI_ANCHO).max(x * w / MINI_ANCHO + 1));
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            // Con saltos de 2 alcanza para una miniatura y es 4 veces más rápido.
            for yy in (y0..y1.min(h)).step_by(2) {
                for xx in (x0..x1.min(w)).step_by(2) {
                    let i = (yy * w + xx) * 4;
                    b += bgra[i] as u32;
                    g += bgra[i + 1] as u32;
                    r += bgra[i + 2] as u32;
                    n += 1;
                }
            }
            let n = n.max(1);
            let o = (y * MINI_ANCHO + x) * 4;
            px[o..o + 4].copy_from_slice(&[(r / n) as u8, (g / n) as u8, (b / n) as u8, 255]);
        }
    }
    Some(ColorImage::from_rgba_unmultiplied([MINI_ANCHO, MINI_ALTO], &px))
}

fn guardar_png(ruta: &Path, img: &ColorImage) -> std::io::Result<()> {
    let archivo = std::io::BufWriter::new(std::fs::File::create(ruta)?);
    let mut enc = png::Encoder::new(archivo, img.size[0] as u32, img.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(std::io::Error::other)?;
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    w.write_image_data(&bytes).map_err(std::io::Error::other)
}

fn leer_png(ruta: &Path) -> Option<ColorImage> {
    let decodificador = png::Decoder::new(std::fs::File::open(ruta).ok()?);
    let mut lector = decodificador.read_info().ok()?;
    let mut buf = vec![0u8; lector.output_buffer_size()];
    let info = lector.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    Some(ColorImage::from_rgba_unmultiplied([info.width as usize, info.height as usize], &buf[..info.buffer_size()]))
}
