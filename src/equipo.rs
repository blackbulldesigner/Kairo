//! Lo que Kairo sabe de tu PC: Windows, procesador, RAM, batería, tarjetas de video, monitores y
//! encoders por hardware. Se detecta en un hilo aparte al abrir la ventana (tarda unas décimas).
//! Sirve para elegir bien cómo grabar y para el diagnóstico que se comparte cuando algo falla.

use windows::core::{Interface, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_SUCCESS, LUID};
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::{EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

/// Desde esta compilación (Windows 10 2004) se puede capturar el audio de un solo proceso.
const COMPILACION_AUDIO_POR_PROCESO: u32 = 19041;
const NT: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
const CPU: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TipoGpu {
    Integrada,
    Dedicada,
    /// La única tarjeta del equipo.
    Unica,
}

#[derive(Clone, Debug)]
pub struct Gpu {
    pub nombre: String,
    pub vram_mb: u64,
    pub tipo: TipoGpu,
}

#[derive(Clone, Debug)]
pub struct Monitor {
    /// Nombre de Windows, como `\\.\DISPLAY1`.
    pub id: String,
    pub ancho: u32,
    pub alto: u32,
    pub hz: u32,
    pub principal: bool,
    /// Índice en `Equipo::gpus` de la tarjeta que lo maneja.
    pub gpu: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Energia {
    Escritorio,
    LaptopEnchufada,
    LaptopBateria,
}

#[derive(Clone, Debug)]
pub struct Equipo {
    pub windows: String,
    pub compilacion: u32,
    pub procesador: String,
    pub ram: u64,
    pub energia: Energia,
    pub gpus: Vec<Gpu>,
    pub monitores: Vec<Monitor>,
    /// Encoders H.264 por hardware que ofrece Windows (Media Foundation).
    pub encoders: Vec<String>,
}

impl Equipo {
    pub fn detectar() -> Equipo {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
        }
        let compilacion = reg_texto(NT, "CurrentBuildNumber").and_then(|s| s.parse().ok()).unwrap_or(0);
        let (gpus, monitores) = tarjetas_y_monitores();
        Equipo {
            windows: version_windows(compilacion),
            compilacion,
            procesador: reg_texto(CPU, "ProcessorNameString").unwrap_or_else(|| "Desconocido".into()),
            ram: ram_total(),
            energia: energia(),
            gpus,
            monitores,
            encoders: encoders_hardware(),
        }
    }

    pub fn es_laptop(&self) -> bool {
        self.energia != Energia::Escritorio
    }

    pub fn audio_por_juego(&self) -> bool {
        self.compilacion >= COMPILACION_AUDIO_POR_PROCESO
    }

    pub fn tiene_dos_tarjetas(&self) -> bool {
        self.gpus.iter().any(|g| g.tipo == TipoGpu::Integrada) && self.gpus.iter().any(|g| g.tipo == TipoGpu::Dedicada)
    }

    /// Resolución del monitor principal (o del primero que haya).
    pub fn resolucion_principal(&self) -> (u32, u32) {
        self.monitores
            .iter()
            .find(|m| m.principal)
            .or(self.monitores.first())
            .map(|m| (m.ancho, m.alto))
            .unwrap_or((1920, 1080))
    }

    pub fn texto_energia(&self) -> &'static str {
        match self.energia {
            Energia::Escritorio => "PC de escritorio",
            Energia::LaptopEnchufada => "Laptop (enchufada)",
            Energia::LaptopBateria => "Laptop (con batería)",
        }
    }

    pub fn texto_ram(&self) -> String {
        format!("{} GB", (self.ram as f64 / (1024.0 * 1024.0 * 1024.0)).round())
    }

    pub fn etiqueta_gpu(g: &Gpu) -> &'static str {
        match g.tipo {
            TipoGpu::Integrada => "Tarjeta integrada",
            TipoGpu::Dedicada => "Tarjeta dedicada",
            TipoGpu::Unica => "Tarjeta de video",
        }
    }

    pub fn texto_gpu(&self, i: usize) -> String {
        let g = &self.gpus[i];
        let mut s = g.nombre.clone();
        if g.vram_mb >= 1024 {
            s += &format!(" · {} GB", (g.vram_mb as f64 / 1024.0).round());
        }
        let monitores: Vec<String> =
            self.monitores.iter().enumerate().filter(|(_, m)| m.gpu == i).map(|(n, _)| (n + 1).to_string()).collect();
        match monitores.len() {
            0 => {}
            1 => s += &format!(" · maneja el monitor {}", monitores[0]),
            _ => s += &format!(" · maneja los monitores {}", monitores.join(" y ")),
        }
        s
    }

    pub fn texto_monitor(&self, i: usize) -> String {
        let m = &self.monitores[i];
        let mut s = format!("{}×{} a {} Hz", m.ancho, m.alto, m.hz);
        if m.principal {
            s += " · principal";
        }
        s
    }

    pub fn texto_encoders(&self) -> String {
        if self.encoders.is_empty() {
            "No se encontró ninguno: Kairo comprimirá con el procesador".into()
        } else {
            self.encoders.join("\n")
        }
    }

    pub fn texto_audio_por_juego(&self) -> &'static str {
        if self.audio_por_juego() {
            "Disponible"
        } else {
            "No disponible (necesita Windows 10 versión 2004 o más nuevo)"
        }
    }

    /// Resumen para copiar y compartir.
    pub fn como_texto(&self) -> String {
        let mut s = format!("Kairo {} · diagnóstico\n", env!("CARGO_PKG_VERSION"));
        s += &format!("Equipo: {}\n", self.texto_energia());
        s += &format!("Windows: {}\n", self.windows);
        s += &format!("Procesador: {}\n", self.procesador);
        s += &format!("Memoria RAM: {}\n", self.texto_ram());
        for (i, g) in self.gpus.iter().enumerate() {
            s += &format!("{}: {}\n", Self::etiqueta_gpu(g), self.texto_gpu(i));
        }
        for i in 0..self.monitores.len() {
            s += &format!("Monitor {}: {}\n", i + 1, self.texto_monitor(i));
        }
        s += &format!("Encoders por hardware: {}\n", self.texto_encoders().replace('\n', "; "));
        s += &format!("Audio solo del juego: {}\n", self.texto_audio_por_juego());
        s
    }
}

fn texto_utf16(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n]).trim().to_string()
}

fn reg_texto(clave: &str, valor: &str) -> Option<String> {
    let (k, v) = (HSTRING::from(clave), HSTRING::from(valor));
    let mut largo = 0u32;
    unsafe {
        if RegGetValueW(HKEY_LOCAL_MACHINE, &k, &v, RRF_RT_REG_SZ, None, None, Some(&mut largo)) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; (largo as usize).div_ceil(2) + 1];
        if RegGetValueW(HKEY_LOCAL_MACHINE, &k, &v, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut largo))
            != ERROR_SUCCESS
        {
            return None;
        }
        Some(texto_utf16(&buf)).filter(|s| !s.is_empty())
    }
}

fn reg_numero(clave: &str, valor: &str) -> Option<u32> {
    let (k, v) = (HSTRING::from(clave), HSTRING::from(valor));
    let mut dato = 0u32;
    let mut largo = 4u32;
    unsafe {
        (RegGetValueW(HKEY_LOCAL_MACHINE, &k, &v, RRF_RT_REG_DWORD, None, Some(&mut dato as *mut u32 as *mut _), Some(&mut largo))
            == ERROR_SUCCESS)
            .then_some(dato)
    }
}

/// "Windows 11 Pro 24H2 (compilación 26100.2314)". El registro dice "Windows 10" también en Windows 11,
/// así que se decide por la compilación.
fn version_windows(compilacion: u32) -> String {
    let nombre = if compilacion >= 22000 { "Windows 11" } else { "Windows 10" };
    let edicion = match reg_texto(NT, "EditionID").as_deref() {
        Some("Core") | Some("CoreSingleLanguage") => " Home".to_string(),
        Some("Professional") => " Pro".to_string(),
        Some(otra) => format!(" {otra}"),
        None => String::new(),
    };
    let version = reg_texto(NT, "DisplayVersion").or_else(|| reg_texto(NT, "ReleaseId")).map(|v| format!(" {v}")).unwrap_or_default();
    let ubr = reg_numero(NT, "UBR").map(|u| format!(".{u}")).unwrap_or_default();
    format!("{nombre}{edicion}{version} (compilación {compilacion}{ubr})")
}

fn ram_total() -> u64 {
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut m) }.map(|_| m.ullTotalPhys).unwrap_or(8 * 1024 * 1024 * 1024)
}

fn energia() -> Energia {
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err() {
        return Energia::Escritorio;
    }
    // 128 = no hay batería; 255 = no se sabe.
    match s.BatteryFlag {
        128 | 255 => Energia::Escritorio,
        _ if s.ACLineStatus == 1 => Energia::LaptopEnchufada,
        _ => Energia::LaptopBateria,
    }
}

fn mismo_luid(a: LUID, b: LUID) -> bool {
    a.LowPart == b.LowPart && a.HighPart == b.HighPart
}

/// Tarjetas de video (sin las de software) y los monitores que maneja cada una.
fn tarjetas_y_monitores() -> (Vec<Gpu>, Vec<Monitor>) {
    let mut gpus = Vec::new();
    let mut luids = Vec::new();
    let mut monitores = Vec::new();
    unsafe {
        let Ok(fabrica) = CreateDXGIFactory1::<IDXGIFactory1>() else {
            return (gpus, monitores);
        };
        let mut i = 0;
        while let Ok(adaptador) = fabrica.EnumAdapters1(i) {
            i += 1;
            let Ok(d) = adaptador.GetDesc1() else { continue };
            // 0x1414 = Microsoft (Basic Render Driver y otros adaptadores de software).
            if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 || d.VendorId == 0x1414 {
                continue;
            }
            let indice = gpus.len();
            gpus.push(Gpu { nombre: texto_utf16(&d.Description), vram_mb: d.DedicatedVideoMemory as u64 / (1024 * 1024), tipo: TipoGpu::Unica });
            luids.push(d.AdapterLuid);
            let mut j = 0;
            while let Ok(salida) = adaptador.EnumOutputs(j) {
                j += 1;
                let Ok(od) = salida.GetDesc() else { continue };
                if !od.AttachedToDesktop.as_bool() {
                    continue;
                }
                let r = od.DesktopCoordinates;
                let mut m = Monitor {
                    id: texto_utf16(&od.DeviceName),
                    ancho: (r.right - r.left).max(0) as u32,
                    alto: (r.bottom - r.top).max(0) as u32,
                    hz: 60,
                    principal: r.left == 0 && r.top == 0,
                    gpu: indice,
                };
                let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
                if EnumDisplaySettingsW(PCWSTR(od.DeviceName.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm).as_bool() {
                    m.ancho = dm.dmPelsWidth;
                    m.alto = dm.dmPelsHeight;
                    m.hz = dm.dmDisplayFrequency.max(1);
                }
                monitores.push(m);
            }
        }
        // Con dos o más tarjetas, Windows sabe cuál gasta menos (la integrada) y cuál rinde más (la dedicada).
        if gpus.len() >= 2 {
            if let Ok(f6) = fabrica.cast::<IDXGIFactory6>() {
                let luid = |pref| -> Option<LUID> {
                    let a: IDXGIAdapter1 = f6.EnumAdapterByGpuPreference(0, pref).ok()?;
                    a.GetDesc1().ok().map(|d| d.AdapterLuid)
                };
                let baja = luid(DXGI_GPU_PREFERENCE_MINIMUM_POWER);
                let alta = luid(DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE);
                if let (Some(baja), Some(alta)) = (baja, alta) {
                    if !mismo_luid(baja, alta) {
                        for (g, l) in gpus.iter_mut().zip(&luids) {
                            g.tipo = if mismo_luid(*l, baja) { TipoGpu::Integrada } else { TipoGpu::Dedicada };
                        }
                    }
                }
            }
        }
    }
    // El principal primero: es el que la gente llama "monitor 1".
    monitores.sort_by_key(|m| !m.principal);
    (gpus, monitores)
}

/// Nombres de los encoders H.264 por hardware (NVIDIA, AMD, Intel Quick Sync…).
fn encoders_hardware() -> Vec<String> {
    let mut nombres = Vec::new();
    unsafe {
        if MFStartup(MF_VERSION, MFSTARTUP_LITE).is_err() {
            return nombres;
        }
        let salida = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_H264 };
        let mut lista: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut cantidad = 0u32;
        let flags = MFT_ENUM_FLAG(MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SORTANDFILTER.0);
        if MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, flags, None, Some(&salida), &mut lista, &mut cantidad).is_ok() && !lista.is_null() {
            for a in std::slice::from_raw_parts_mut(lista, cantidad as usize) {
                if let Some(act) = a.take() {
                    let mut texto = PWSTR::null();
                    let mut largo = 0u32;
                    if act.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut texto, &mut largo).is_ok() {
                        if let Ok(n) = texto.to_string() {
                            nombres.push(n.trim().to_string());
                        }
                        CoTaskMemFree(Some(texto.0 as *const _));
                    }
                }
            }
            CoTaskMemFree(Some(lista as *const _));
        }
        let _ = MFShutdown();
    }
    nombres
}
