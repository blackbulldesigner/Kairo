//! Ajustes de Kairo. Se guardan en %APPDATA%\Kairo\config.json; la grabadora los lee al arrancar.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::core::HSTRING;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Videos, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

/// Duración mínima y máxima de un clip (segundos). El máximo real de cada PC depende de su RAM.
pub const SEGUNDOS_MIN: u32 = 10;
pub const SEGUNDOS_MAX: u32 = 600;
/// Audio sin comprimir en el buffer: 48 kHz, estéreo, 16 bits (bytes por segundo y por fuente).
const AUDIO_BYTES_POR_SEG: u64 = 48_000 * 2 * 2;
/// Parte de la RAM que puede ocupar el buffer en el peor caso.
const PARTE_DE_LA_RAM: u64 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ModoCaptura {
    /// La pantalla completa: el juego y todo lo que aparezca encima.
    Escritorio,
    /// Solo la ventana del juego.
    Juego,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Resolucion {
    Nativa,
    P1440,
    P1080,
    P720,
}

impl Resolucion {
    pub const TODAS: [Resolucion; 4] = [Resolucion::Nativa, Resolucion::P1440, Resolucion::P1080, Resolucion::P720];

    /// Lado corto del clip (None = como el monitor).
    pub fn lado_corto(self) -> Option<u32> {
        match self {
            Resolucion::Nativa => None,
            Resolucion::P1440 => Some(1440),
            Resolucion::P1080 => Some(1080),
            Resolucion::P720 => Some(720),
        }
    }

    pub fn nombre(self) -> &'static str {
        match self {
            Resolucion::Nativa => "Nativa",
            Resolucion::P1440 => "1440p",
            Resolucion::P1080 => "1080p",
            Resolucion::P720 => "720p",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Calidad {
    Ligera,
    Equilibrada,
    Alta,
}

impl Calidad {
    pub const TODAS: [Calidad; 3] = [Calidad::Ligera, Calidad::Equilibrada, Calidad::Alta];

    /// Bitrate de referencia para 1080p a 60 fps (Mbps).
    fn mbps_1080p60(self) -> f32 {
        match self {
            Calidad::Ligera => 8.0,
            Calidad::Equilibrada => 16.0,
            Calidad::Alta => 30.0,
        }
    }

    pub fn nombre(self) -> &'static str {
        match self {
            Calidad::Ligera => "Ligera",
            Calidad::Equilibrada => "Equilibrada",
            Calidad::Alta => "Alta",
        }
    }
}

/// Con qué tarjeta de video se comprime el video (importa en laptops con dos tarjetas).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum TarjetaGrabacion {
    /// La que maneja el monitor que se graba.
    Automatica,
    Integrada,
    Dedicada,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct FuenteAudio {
    pub activa: bool,
    /// 0 a 100.
    pub volumen: u8,
}

impl Default for FuenteAudio {
    fn default() -> Self {
        Self { activa: true, volumen: 100 }
    }
}

/// Un atajo de teclado global. `tecla` es el nombre de la tecla (por ejemplo "F9" o "K").
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Atajo {
    pub tecla: String,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
}

impl Atajo {
    pub fn tecla_sola(tecla: &str) -> Self {
        Self { tecla: tecla.to_string(), ctrl: false, shift: false, alt: false }
    }

    /// Las teclas en orden, para mostrarlas como chips: ["Ctrl", "Shift", "F9"].
    pub fn partes(&self) -> Vec<String> {
        let mut v = Vec::new();
        if self.ctrl {
            v.push("Ctrl".to_string());
        }
        if self.shift {
            v.push("Shift".to_string());
        }
        if self.alt {
            v.push("Alt".to_string());
        }
        v.push(nombre_tecla(&self.tecla));
        v
    }

    pub fn texto(&self) -> String {
        self.partes().join(" + ")
    }

    /// Las letras y los números sin Ctrl ni Alt dejarían de escribirse en todo Windows.
    pub fn es_valido(&self) -> bool {
        let simple = self.tecla.chars().count() == 1 && self.tecla.chars().all(|c| c.is_ascii_alphanumeric());
        !simple || self.ctrl || self.alt
    }
}

fn nombre_tecla(t: &str) -> String {
    match t {
        "Space" => "Espacio",
        "Home" => "Inicio",
        "End" => "Fin",
        "PageUp" => "Re Pág",
        "PageDown" => "Av Pág",
        "Insert" => "Insert",
        "Delete" => "Supr",
        "ArrowUp" => "↑",
        "ArrowDown" => "↓",
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        otro => otro,
    }
    .to_string()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CualAtajo {
    Guardar,
    Pausa,
    Carpeta,
}

impl CualAtajo {
    pub const TODOS: [CualAtajo; 3] = [CualAtajo::Guardar, CualAtajo::Pausa, CualAtajo::Carpeta];

    pub fn titulo(self) -> &'static str {
        match self {
            CualAtajo::Guardar => "Guardar clip",
            CualAtajo::Pausa => "Pausar o reanudar la grabación",
            CualAtajo::Carpeta => "Abrir la carpeta de clips",
        }
    }

    pub fn descripcion(self) -> Option<&'static str> {
        match self {
            CualAtajo::Guardar => Some("Guarda los últimos segundos como MP4."),
            CualAtajo::Pausa => Some("Mientras está en pausa no se usa nada de la tarjeta de video."),
            CualAtajo::Carpeta => None,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub modo: ModoCaptura,
    /// Monitor a grabar en modo escritorio (nombre de Windows, como `\\.\DISPLAY1`). None = automático.
    pub monitor: Option<String>,
    pub segundos: u32,
    pub resolucion: Resolucion,
    pub fps: u32,
    pub calidad: Calidad,
    pub tarjeta: TarjetaGrabacion,
    /// Laptops: con la batería, graba a 720p y en calidad Ligera.
    pub ahorro_bateria: bool,
    pub audio_juego: FuenteAudio,
    pub audio_sistema: FuenteAudio,
    pub audio_mic: FuenteAudio,
    pub carpeta: PathBuf,
    pub sonido_al_guardar: bool,
    pub atajo_guardar: Option<Atajo>,
    pub atajo_pausa: Option<Atajo>,
    pub atajo_carpeta: Option<Atajo>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            modo: ModoCaptura::Escritorio,
            monitor: None,
            segundos: 30,
            resolucion: Resolucion::P1080,
            fps: 60,
            calidad: Calidad::Equilibrada,
            tarjeta: TarjetaGrabacion::Automatica,
            ahorro_bateria: true,
            audio_juego: FuenteAudio::default(),
            audio_sistema: FuenteAudio::default(),
            audio_mic: FuenteAudio { activa: false, volumen: 100 },
            carpeta: carpeta_videos(),
            sonido_al_guardar: true,
            atajo_guardar: Some(Atajo::tecla_sola("F9")),
            atajo_pausa: None,
            atajo_carpeta: None,
        }
    }
}

impl Config {
    pub fn cargar() -> Config {
        let mut c: Config = std::fs::read_to_string(archivo())
            .ok()
            .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default();
        c.normalizar();
        c
    }

    /// Escribe primero a un archivo temporal y después lo renombra: si se corta la luz, no queda a medias.
    pub fn guardar(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(datos_dir())?;
        let tmp = archivo().with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, archivo())
    }

    fn normalizar(&mut self) {
        self.segundos = (self.segundos.clamp(SEGUNDOS_MIN, SEGUNDOS_MAX) / 5) * 5;
        if self.fps != 30 {
            self.fps = 60;
        }
        for f in [&mut self.audio_juego, &mut self.audio_sistema, &mut self.audio_mic] {
            f.volumen = f.volumen.min(100);
        }
        if self.carpeta.as_os_str().is_empty() {
            self.carpeta = carpeta_videos();
        }
    }

    pub fn atajo(&self, c: CualAtajo) -> Option<&Atajo> {
        match c {
            CualAtajo::Guardar => self.atajo_guardar.as_ref(),
            CualAtajo::Pausa => self.atajo_pausa.as_ref(),
            CualAtajo::Carpeta => self.atajo_carpeta.as_ref(),
        }
    }

    pub fn atajo_mut(&mut self, c: CualAtajo) -> &mut Option<Atajo> {
        match c {
            CualAtajo::Guardar => &mut self.atajo_guardar,
            CualAtajo::Pausa => &mut self.atajo_pausa,
            CualAtajo::Carpeta => &mut self.atajo_carpeta,
        }
    }

    /// Tamaño del clip para un monitor de `nativa` píxeles (nunca más grande que el monitor, en pares).
    pub fn tamano_clip(&self, nativa: (u32, u32)) -> (u32, u32) {
        let (w, h) = nativa;
        let corto = w.min(h).max(1);
        let par = |x: f32| ((x / 2.0).round() as u32 * 2).max(2);
        match self.resolucion.lado_corto() {
            Some(objetivo) if objetivo < corto => {
                let escala = objetivo as f32 / corto as f32;
                (par(w as f32 * escala), par(h as f32 * escala))
            }
            _ => (par(w as f32), par(h as f32)),
        }
    }

    /// Bitrate estimado del video (Mbps): crece con los píxeles y los cuadros, pero menos que lineal.
    pub fn mbps(&self, nativa: (u32, u32)) -> f32 {
        let (w, h) = self.tamano_clip(nativa);
        let pixeles = (w * h) as f32 / (1920.0 * 1080.0);
        let cuadros = self.fps as f32 / 60.0;
        let m = self.calidad.mbps_1080p60() * pixeles.powf(0.75) * cuadros.powf(0.7);
        (m * 2.0).round() / 2.0
    }

    /// Bytes por segundo que ocupa el buffer en RAM (video comprimido + audio sin comprimir).
    pub fn bytes_por_segundo(&self, nativa: (u32, u32)) -> u64 {
        let fuentes = [&self.audio_juego, &self.audio_sistema, &self.audio_mic].iter().filter(|f| f.activa).count() as u64;
        (self.mbps(nativa) as f64 * 125_000.0) as u64 + fuentes * AUDIO_BYTES_POR_SEG
    }

    /// Duración máxima para que el buffer no pase de una parte de la RAM (múltiplo de 5 s).
    pub fn segundos_maximos(ram: u64, bytes_por_segundo: u64) -> u32 {
        let s = (ram / PARTE_DE_LA_RAM) / bytes_por_segundo.max(1);
        ((s.min(SEGUNDOS_MAX as u64) as u32) / 5 * 5).max(SEGUNDOS_MIN)
    }
}

/// "30 s", "1 min", "1 min 30 s".
pub fn texto_duracion(s: u32) -> String {
    match (s / 60, s % 60) {
        (0, seg) => format!("{seg} s"),
        (min, 0) => format!("{min} min"),
        (min, seg) => format!("{min} min {seg} s"),
    }
}

/// "75 MB", "1,2 GB".
pub fn texto_bytes(b: u64) -> String {
    let mb = b as f64 / (1024.0 * 1024.0);
    if mb < 1000.0 {
        format!("{:.0} MB", mb)
    } else {
        format!("{:.1} GB", mb / 1024.0).replace('.', ",")
    }
}

/// %APPDATA%\Kairo
pub fn datos_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Kairo")
}

fn archivo() -> PathBuf {
    datos_dir().join("config.json")
}

/// La carpeta Videos del usuario (aunque la haya movido a otro disco) + "Kairo".
pub fn carpeta_videos() -> PathBuf {
    let videos = unsafe {
        SHGetKnownFolderPath(&FOLDERID_Videos, KF_FLAG_DEFAULT, HANDLE::default()).ok().and_then(|p| {
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const _));
            s
        })
    };
    videos
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|u| PathBuf::from(u).join("Videos")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Kairo")
}

/// Espacio libre en el disco de `p` (si la carpeta todavía no existe, se mira la primera que sí).
pub fn espacio_libre(p: &Path) -> Option<u64> {
    let mut dir = p.to_path_buf();
    while !dir.exists() {
        if !dir.pop() {
            return None;
        }
    }
    let mut libre = 0u64;
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(dir.as_os_str()), Some(&mut libre), None, None).ok()? };
    Some(libre)
}
