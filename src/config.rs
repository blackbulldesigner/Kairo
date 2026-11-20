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

/// Qué más se graba aparte del juego y el micrófono.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum OtrosSonidos {
    /// Todo lo demás que suena en la PC (sin el juego), en una pista.
    Todo,
    /// Solo los programas de la lista, cada uno en su pista.
    Elegidos,
}

/// Un programa cuyo sonido se graba en su propia pista (Discord, música…).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ProgramaAudio {
    /// Nombre del exe en minúsculas ("discord.exe").
    pub exe: String,
    pub nombre: String,
    pub activa: bool,
    /// 0 a 100.
    pub volumen: u8,
}

/// Cuántos programas se pueden sumar como máximo.
pub const PROGRAMAS_MAX: usize = 8;

/// El juego elegido a mano para "Solo el juego".
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct JuegoElegido {
    /// Nombre del exe en minúsculas ("valorant-win64-shipping.exe").
    pub exe: String,
    /// Para mostrar ("VALORANT").
    pub nombre: String,
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

/// Cómo reparte el encoder los bits.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ControlBitrate {
    /// Variable con tope: más bits en las escenas movidas, sin pasarse del pico.
    Variable,
    /// Constante: el mismo tamaño cada segundo (más predecible, a veces menos calidad).
    Constante,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum PerfilH264 {
    /// High: la mejor compresión.
    Alto,
    /// Main.
    Principal,
    /// Baseline: para celulares o reproductores muy viejos.
    Basico,
}

/// El formato del video. H.264 se ve en todos lados; HEVC y AV1 pesan menos con la misma calidad, pero no
/// todos los programas los reproducen y necesitan una tarjeta que los comprima.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

impl Codec {
    pub fn nombre(self) -> &'static str {
        match self {
            Codec::H264 => "H.264",
            Codec::Hevc => "HEVC",
            Codec::Av1 => "AV1",
        }
    }
}

/// Cuánto trabaja el encoder por cuadro (CODECAPI_AVEncCommonQualityVsSpeed).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum VelocidadEncoder {
    /// Lo que decida el driver de la tarjeta.
    Automatica,
    /// Menos trabajo para la tarjeta (ideal en laptops).
    Rapida,
    Equilibrada,
    /// Mejor imagen con el mismo bitrate, un poco más de trabajo.
    Calidad,
}

impl VelocidadEncoder {
    /// Valor para el encoder (0 = lo más rápido, 100 = la mejor calidad).
    pub fn valor(self) -> Option<u32> {
        match self {
            VelocidadEncoder::Automatica => None,
            VelocidadEncoder::Rapida => Some(0),
            VelocidadEncoder::Equilibrada => Some(50),
            VelocidadEncoder::Calidad => Some(100),
        }
    }
}

/// Grabación → Opciones avanzadas. Si `activo` es false se usa lo de siempre (lo que dice "Calidad").
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoAvanzado {
    pub activo: bool,
    /// Bitrate promedio del video, en Mbps.
    pub mbps: f32,
    pub control: ControlBitrate,
    /// Con bitrate variable: el pico, como múltiplo del promedio (1,5 = hasta 50 % más).
    pub pico: f32,
    /// Un cuadro clave cada tantos segundos (0,5, 1 o 2). El clip y los recortes empiezan en uno.
    pub clave_cada: f32,
    pub perfil: PerfilH264,
    pub velocidad: VelocidadEncoder,
    /// Audio AAC en kbps (96, 128, 160 o 192).
    pub audio_kbps: u32,
    pub codec: Codec,
}

impl Default for VideoAvanzado {
    fn default() -> Self {
        Self {
            activo: false,
            mbps: 16.0,
            control: ControlBitrate::Variable,
            pico: 1.5,
            clave_cada: 1.0,
            perfil: PerfilH264::Alto,
            velocidad: VelocidadEncoder::Automatica,
            audio_kbps: 192,
            codec: Codec::H264,
        }
    }
}

pub const MBPS_MIN: f32 = 2.0;
pub const MBPS_MAX: f32 = 80.0;

/// Guardado → Limpieza automática: qué clips viejos se borran solos. Nunca los favoritos ni los videos que
/// no hizo Kairo (ver `limpieza.rs`).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Limpieza {
    pub activa: bool,
    /// Borrar los de más de tantos días (0 = sin límite).
    pub dias: u32,
    /// Si todos juntos pasan de tantos GB, borrar los más viejos hasta bajar (0 = sin límite).
    pub gb: u32,
    /// Borrar del todo en vez de mandarlos a la Papelera.
    pub sin_papelera: bool,
}

impl Default for Limpieza {
    fn default() -> Self {
        Self { activa: false, dias: 30, gb: 50, sin_papelera: false }
    }
}

/// Lo que necesita el encoder de video (sale de la configuración: ver `Config::ajustes_encoder`).
#[derive(Clone, Copy, Debug)]
pub struct AjustesEncoder {
    pub fps: u32,
    /// Bits por segundo.
    pub bitrate: u32,
    pub control: ControlBitrate,
    pub pico: f32,
    /// Cuadros entre cuadros clave.
    pub gop: u32,
    pub perfil: PerfilH264,
    pub velocidad: VelocidadEncoder,
    pub codec: Codec,
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

    /// Código de tecla virtual de Windows (para `RegisterHotKey`). None si la tecla no se puede usar.
    pub fn tecla_virtual(&self) -> Option<u32> {
        let t = self.tecla.as_str();
        let mut letras = t.chars();
        if let (Some(c), None) = (letras.next(), letras.next()) {
            if c.is_ascii_digit() || c.is_ascii_uppercase() {
                return Some(c as u32);
            }
        }
        if let Some(n) = t.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
            return (1..=24).contains(&n).then_some(0x70 + n - 1);
        }
        Some(match t {
            "Backspace" => 0x08,
            "Tab" => 0x09,
            "Enter" => 0x0D,
            "Space" => 0x20,
            "PageUp" => 0x21,
            "PageDown" => 0x22,
            "End" => 0x23,
            "Home" => 0x24,
            "ArrowLeft" => 0x25,
            "ArrowUp" => 0x26,
            "ArrowRight" => 0x27,
            "ArrowDown" => 0x28,
            "Insert" => 0x2D,
            "Delete" => 0x2E,
            "Semicolon" => 0xBA,
            "Plus" | "Equals" => 0xBB,
            "Comma" => 0xBC,
            "Minus" => 0xBD,
            "Period" => 0xBE,
            "Slash" => 0xBF,
            "Backtick" => 0xC0,
            "OpenBracket" => 0xDB,
            "Backslash" => 0xDC,
            "CloseBracket" => 0xDD,
            "Quote" => 0xDE,
            _ => return None,
        })
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
    /// Un clip más largo (la duración de "Clip largo" en Grabación).
    GuardarLargo,
    /// Una foto de lo que se está grabando.
    Foto,
    Pausa,
    Carpeta,
}

impl CualAtajo {
    pub const TODOS: [CualAtajo; 5] = [CualAtajo::Guardar, CualAtajo::GuardarLargo, CualAtajo::Foto, CualAtajo::Pausa, CualAtajo::Carpeta];

    pub fn titulo(self) -> &'static str {
        match self {
            CualAtajo::Guardar => "Guardar clip",
            CualAtajo::GuardarLargo => "Guardar clip largo",
            CualAtajo::Foto => "Sacar foto",
            CualAtajo::Pausa => "Pausar o reanudar la grabación",
            CualAtajo::Carpeta => "Abrir la carpeta de clips",
        }
    }

    pub fn descripcion(self) -> Option<&'static str> {
        match self {
            CualAtajo::Guardar => Some("Guarda los últimos segundos como MP4."),
            CualAtajo::GuardarLargo => Some("Guarda un clip más largo (la duración se elige en Grabación → Clip largo)."),
            CualAtajo::Foto => Some("Guarda una foto de lo que se está grabando (PNG), que aparece en la Biblioteca."),
            CualAtajo::Pausa => Some("Mientras está en pausa no se usa nada de la tarjeta de video."),
            CualAtajo::Carpeta => None,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub modo: ModoCaptura,
    /// En "Solo el juego": el juego elegido a mano (None = automático, el que esté en pantalla completa).
    pub juego_elegido: Option<JuegoElegido>,
    /// Monitor a grabar en modo escritorio (nombre de Windows, como `\\.\DISPLAY1`). None = automático.
    pub monitor: Option<String>,
    pub segundos: u32,
    pub resolucion: Resolucion,
    pub fps: u32,
    pub calidad: Calidad,
    pub avanzado: VideoAvanzado,
    pub tarjeta: TarjetaGrabacion,
    /// Laptops: con la batería, graba a 720p, 30 FPS y en calidad Ligera.
    pub ahorro_bateria: bool,
    pub audio_juego: FuenteAudio,
    pub audio_sistema: FuenteAudio,
    pub audio_mic: FuenteAudio,
    pub otros_sonidos: OtrosSonidos,
    pub programas_audio: Vec<ProgramaAudio>,
    pub carpeta: PathBuf,
    pub sonido_al_guardar: bool,
    /// Un cartelito en pantalla al guardar un clip o una foto (apagado, queda solo el sonido).
    pub aviso_al_guardar: bool,
    /// Las copias para Discord se borran solas después de estos minutos (15, 60 o 1440).
    pub discord_minutos: u32,
    /// Guardar cada clip en una carpeta con el nombre del juego.
    pub carpeta_por_juego: bool,
    /// Además de la mezcla, cada fuente de audio en su propia pista (para editar).
    pub pistas_separadas: bool,
    pub atajo_guardar: Option<Atajo>,
    pub atajo_guardar_largo: Option<Atajo>,
    /// El clip largo prendido. Apagado no usa nada de más (ni RAM ni el atajo). None = config de antes de la
    /// 0.9.1: se prende si ya tenía atajo.
    pub clip_largo_activo: Option<bool>,
    /// Duración del clip largo (segundo atajo).
    pub segundos_largo: u32,
    pub atajo_pausa: Option<Atajo>,
    pub atajo_carpeta: Option<Atajo>,
    pub atajo_foto: Option<Atajo>,
    /// Buscar versiones nuevas en GitHub (una consulta chica por día).
    pub buscar_actualizaciones: bool,
    /// Dibujar el puntero del mouse en los clips.
    pub mostrar_mouse: bool,
    pub limpieza: Limpieza,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            modo: ModoCaptura::Escritorio,
            juego_elegido: None,
            monitor: None,
            segundos: 30,
            resolucion: Resolucion::P1080,
            fps: 60,
            calidad: Calidad::Equilibrada,
            avanzado: VideoAvanzado::default(),
            tarjeta: TarjetaGrabacion::Automatica,
            ahorro_bateria: true,
            audio_juego: FuenteAudio::default(),
            audio_sistema: FuenteAudio::default(),
            audio_mic: FuenteAudio { activa: false, volumen: 100 },
            otros_sonidos: OtrosSonidos::Todo,
            programas_audio: Vec::new(),
            carpeta: carpeta_videos(),
            sonido_al_guardar: true,
            aviso_al_guardar: true,
            discord_minutos: 60,
            carpeta_por_juego: true,
            pistas_separadas: false,
            atajo_guardar: Some(Atajo::tecla_sola("F9")),
            atajo_guardar_largo: None,
            clip_largo_activo: Some(false),
            segundos_largo: 120,
            atajo_pausa: None,
            atajo_carpeta: None,
            atajo_foto: None,
            buscar_actualizaciones: true,
            mostrar_mouse: false,
            limpieza: Limpieza::default(),
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
        self.segundos_largo = (self.segundos_largo.clamp(SEGUNDOS_MIN, SEGUNDOS_MAX) / 5) * 5;
        if self.clip_largo_activo.is_none() {
            self.clip_largo_activo = Some(self.atajo_guardar_largo.is_some());
        }
        if self.fps != 30 {
            self.fps = 60;
        }
        if ![15, 60, 1440].contains(&self.discord_minutos) {
            self.discord_minutos = 60;
        }
        for f in [&mut self.audio_juego, &mut self.audio_sistema, &mut self.audio_mic] {
            f.volumen = f.volumen.min(100);
        }
        self.programas_audio.truncate(PROGRAMAS_MAX);
        for p in &mut self.programas_audio {
            p.volumen = p.volumen.min(100);
            p.exe = p.exe.to_lowercase();
        }
        if self.carpeta.as_os_str().is_empty() {
            self.carpeta = carpeta_videos();
        }
        let a = &mut self.avanzado;
        a.mbps = if a.mbps.is_finite() { (a.mbps.clamp(MBPS_MIN, MBPS_MAX) * 2.0).round() / 2.0 } else { 16.0 };
        a.pico = if a.pico.is_finite() { (a.pico.clamp(1.1, 3.0) * 10.0).round() / 10.0 } else { 1.5 };
        a.clave_cada = [0.5, 1.0, 2.0].into_iter().find(|v| (v - a.clave_cada).abs() < 0.01).unwrap_or(1.0);
        if ![96, 128, 160, 192].contains(&a.audio_kbps) {
            a.audio_kbps = 192;
        }
    }

    /// Los ajustes del encoder de video para un monitor de `nativa` píxeles.
    pub fn ajustes_encoder(&self, nativa: (u32, u32)) -> AjustesEncoder {
        let a = if self.avanzado.activo { self.avanzado.clone() } else { VideoAvanzado::default() };
        AjustesEncoder {
            fps: self.fps,
            bitrate: (self.mbps(nativa) * 1_000_000.0) as u32,
            control: a.control,
            pico: a.pico,
            gop: ((self.fps as f32 * a.clave_cada).round() as u32).max(1),
            perfil: a.perfil,
            velocidad: a.velocidad,
            codec: a.codec,
        }
    }

    /// Bitrate del audio AAC en bytes por segundo (lo que pide Media Foundation).
    pub fn audio_bytes_por_segundo(&self) -> u32 {
        let kbps = if self.avanzado.activo { self.avanzado.audio_kbps } else { 192 };
        kbps * 1000 / 8
    }

    /// ¿Está prendido el clip largo?
    pub fn clip_largo(&self) -> bool {
        self.clip_largo_activo == Some(true)
    }

    /// Cuántos segundos guarda el buffer en RAM: los del clip, o los del clip largo si tiene atajo y es más largo.
    pub fn segundos_en_ram(&self) -> u32 {
        if self.clip_largo() {
            self.segundos.max(self.segundos_largo)
        } else {
            self.segundos
        }
    }

    pub fn atajo(&self, c: CualAtajo) -> Option<&Atajo> {
        match c {
            CualAtajo::Guardar => self.atajo_guardar.as_ref(),
            CualAtajo::GuardarLargo => self.atajo_guardar_largo.as_ref(),
            CualAtajo::Pausa => self.atajo_pausa.as_ref(),
            CualAtajo::Carpeta => self.atajo_carpeta.as_ref(),
            CualAtajo::Foto => self.atajo_foto.as_ref(),
        }
    }

    pub fn atajo_mut(&mut self, c: CualAtajo) -> &mut Option<Atajo> {
        match c {
            CualAtajo::Guardar => &mut self.atajo_guardar,
            CualAtajo::GuardarLargo => &mut self.atajo_guardar_largo,
            CualAtajo::Pausa => &mut self.atajo_pausa,
            CualAtajo::Carpeta => &mut self.atajo_carpeta,
            CualAtajo::Foto => &mut self.atajo_foto,
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

    /// Bitrate del video (Mbps). Con las opciones avanzadas, el elegido; si no, uno que crece con los píxeles
    /// y los cuadros, pero menos que lineal.
    pub fn mbps(&self, nativa: (u32, u32)) -> f32 {
        if self.avanzado.activo {
            return self.avanzado.mbps;
        }
        self.mbps_de_la_calidad(nativa)
    }

    /// El bitrate que daría "Calidad" (también es el punto de partida al activar las opciones avanzadas).
    pub fn mbps_de_la_calidad(&self, nativa: (u32, u32)) -> f32 {
        let (w, h) = self.tamano_clip(nativa);
        let pixeles = (w * h) as f32 / (1920.0 * 1080.0);
        let cuadros = self.fps as f32 / 60.0;
        let m = self.calidad.mbps_1080p60() * pixeles.powf(0.75) * cuadros.powf(0.7);
        (m * 2.0).round() / 2.0
    }

    /// Bytes por segundo que ocupa el buffer en RAM (video comprimido + audio sin comprimir).
    pub fn bytes_por_segundo(&self, nativa: (u32, u32)) -> u64 {
        let sistema = self.otros_sonidos == OtrosSonidos::Todo && self.audio_sistema.activa;
        let programas = if self.otros_sonidos == OtrosSonidos::Elegidos {
            self.programas_audio.iter().filter(|p| p.activa).count()
        } else {
            0
        };
        let fuentes = (self.audio_juego.activa as usize + sistema as usize + self.audio_mic.activa as usize + programas) as u64;
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
    if mb < 1.0 {
        // (Fotos chicas: "350 KB" en vez de "0 MB".)
        format!("{:.0} KB", (b as f64 / 1024.0).max(1.0))
    } else if mb < 10.0 {
        format!("{:.1} MB", mb).replace('.', ",")
    } else if mb < 1000.0 {
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
