//! Qué es "el juego": el programa de la ventana que está adelante ocupando todo su monitor (pantalla
//! completa o ventana sin bordes), salvo los que claramente no son juegos (navegadores, reproductores,
//! el escritorio). Una vez detectado, sigue siendo "el juego" mientras esté abierto, aunque hagas Alt+Tab.
//! Lo usan: elegir el monitor, el modo "Solo el juego", el audio del juego y la carpeta de los clips.

use std::collections::HashSet;

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetForegroundWindow, GetWindow, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, GWL_EXSTYLE, GW_OWNER, WS_EX_TOOLWINDOW,
};

use super::reloj::{self, SEGUNDO};
use crate::registro::log;

/// Programas de Windows que nunca tiene sentido elegir como juego.
const DE_WINDOWS: &[&str] =
    &["explorer", "applicationframehost", "searchhost", "searchapp", "startmenuexperiencehost", "shellexperiencehost", "textinputhost", "systemsettings", "kairo"];

/// Programas que pueden estar en pantalla completa sin ser juegos (nombre del exe en minúsculas).
const NO_SON_JUEGOS: &[&str] = &[
    // Navegadores
    "chrome", "msedge", "firefox", "opera", "brave", "vivaldi", "arc", "iexplore",
    // Reproductores de video y música
    "vlc", "mpv", "mpc-hc", "mpc-hc64", "mpc-be", "mpc-be64", "potplayer", "potplayer64", "potplayermini", "potplayermini64",
    "wmplayer", "video.ui", "microsoft.media.player", "kmplayer", "spotify",
    // Windows
    "explorer", "applicationframehost", "searchhost", "searchapp", "startmenuexperiencehost", "shellexperiencehost",
    "lockapp", "textinputhost", "snippingtool", "screenclippinghost",
    // Presentaciones, streaming y chat
    "powerpnt", "obs64", "obs32", "discord", "steamwebhelper",
    // Kairo mismo
    "kairo",
];

/// Ventanas del escritorio y la barra de tareas (también "ocupan todo el monitor").
const CLASES_DEL_ESCRITORIO: &[&str] = &["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

const SIGUE_ACTIVO: u32 = 259;

pub struct Juego {
    pub pid: u32,
    /// Para mostrar y para la carpeta: la descripción del exe ("VALORANT"), o el título, o el nombre del exe.
    pub nombre: String,
    /// Monitor donde estuvo la última vez en pantalla completa (0 = no se sabe).
    pub monitor: isize,
    proceso: HANDLE,
}

impl Drop for Juego {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.proceso);
        }
    }
}

#[derive(Default)]
pub struct Detector {
    juego: Option<Juego>,
    al_frente: bool,
    /// Último programa en pantalla completa que resultó no ser un juego (para no revisarlo de nuevo).
    descartado: u32,
    /// Para probar: `KAIRO_JUEGO_PID` fuerza qué proceso es "el juego".
    forzado: Option<u32>,
    /// Juego elegido a mano (nombre del exe en minúsculas, "valorant-win64-shipping.exe"). Con esto no
    /// hace falta que esté en pantalla completa: sirve también para juegos en ventana.
    elegido: Option<String>,
    /// Cuándo volver a buscar el juego elegido si no está abierto (100 ns).
    proxima_busqueda: i64,
}

/// Un programa abierto con ventana (para elegir el juego a mano).
#[derive(Clone, Debug)]
pub struct Programa {
    /// Nombre del exe en minúsculas ("valorant-win64-shipping.exe").
    pub exe: String,
    /// Nombre para mostrar ("VALORANT").
    pub nombre: String,
}

/// Qué cambió en la última revisión.
#[derive(Default, PartialEq, Eq)]
pub struct Cambios {
    pub juego: bool,
    pub monitor: bool,
}

fn texto(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n]).trim().to_string()
}

unsafe fn sigue_activo(proceso: HANDLE) -> bool {
    let mut codigo = 0u32;
    GetExitCodeProcess(proceso, &mut codigo).is_ok() && codigo == SIGUE_ACTIVO
}

/// El monitor que la ventana ocupa por completo, si lo ocupa (pantalla completa o sin bordes).
unsafe fn monitor_en_pantalla_completa(hwnd: HWND) -> Option<isize> {
    if hwnd.is_invalid() || IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() {
        return None;
    }
    let mut clase = [0u16; 64];
    let n = GetClassNameW(hwnd, &mut clase).max(0) as usize;
    if CLASES_DEL_ESCRITORIO.contains(&String::from_utf16_lossy(&clase[..n]).as_str()) {
        return None;
    }
    let mut r = RECT::default();
    GetWindowRect(hwnd, &mut r).ok()?;
    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
    if monitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if !GetMonitorInfoW(monitor, &mut info).as_bool() {
        return None;
    }
    let m = info.rcMonitor;
    (r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom).then_some(monitor.0 as isize)
}

/// Ruta completa del exe de un proceso.
unsafe fn ruta_del_exe(proceso: HANDLE) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut largo = buf.len() as u32;
    QueryFullProcessImageNameW(proceso, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut largo).ok()?;
    Some(String::from_utf16_lossy(&buf[..largo as usize]))
}

/// "FileDescription" del exe (lo que Windows muestra como nombre del programa).
unsafe fn descripcion_del_exe(ruta: &str) -> Option<String> {
    let ruta = HSTRING::from(ruta);
    let tam = GetFileVersionInfoSizeW(&ruta, None);
    if tam == 0 {
        return None;
    }
    let mut datos = vec![0u8; tam as usize];
    GetFileVersionInfoW(&ruta, 0, tam, datos.as_mut_ptr() as *mut _).ok()?;
    let mut p = std::ptr::null_mut();
    let mut largo = 0u32;
    // Idioma y página de códigos de la primera traducción.
    if !VerQueryValueW(datos.as_ptr() as *const _, &HSTRING::from("\\VarFileInfo\\Translation"), &mut p, &mut largo).as_bool() || largo < 4 {
        return None;
    }
    let idioma = *(p as *const u16);
    let pagina = *(p as *const u16).add(1);
    let clave = format!("\\StringFileInfo\\{idioma:04x}{pagina:04x}\\FileDescription");
    if !VerQueryValueW(datos.as_ptr() as *const _, &HSTRING::from(clave), &mut p, &mut largo).as_bool() || largo == 0 {
        return None;
    }
    let s = String::from_utf16_lossy(std::slice::from_raw_parts(p as *const u16, largo as usize));
    Some(s.trim_end_matches('\0').trim().to_string()).filter(|s| !s.is_empty())
}

/// Si el proceso es un juego, sus datos (y el handle para saber cuándo se cierra). `forzado`: viene de
/// `KAIRO_JUEGO_PID` y se acepta aunque esté en la lista de los que no son juegos.
unsafe fn identificar(pid: u32, ventana: HWND, monitor: isize, forzado: bool) -> Option<Juego> {
    let proceso = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let Some(ruta) = ruta_del_exe(proceso) else {
        let _ = CloseHandle(proceso);
        return None;
    };
    let exe = std::path::Path::new(&ruta).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !forzado && NO_SON_JUEGOS.contains(&exe.as_str()) {
        let _ = CloseHandle(proceso);
        return None;
    }
    let titulo = if ventana.is_invalid() {
        String::new()
    } else {
        let mut buf = [0u16; 256];
        let n = GetWindowTextW(ventana, &mut buf).max(0) as usize;
        texto(&buf[..n])
    };
    let nombre = descripcion_del_exe(&ruta)
        .or_else(|| Some(titulo).filter(|t| !t.is_empty()))
        .unwrap_or_else(|| std::path::Path::new(&ruta).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    log(&format!("juego detectado: {nombre} ({exe}.exe, pid {pid})"));
    Some(Juego { pid, nombre, monitor, proceso })
}

impl Detector {
    /// `elegido`: el exe del juego elegido a mano (None = automático).
    pub fn new(elegido: Option<String>) -> Self {
        let forzado = std::env::var("KAIRO_JUEGO_PID").ok().and_then(|v| v.parse().ok());
        Self { forzado, elegido: elegido.map(|e| e.to_lowercase()), ..Default::default() }
    }

    pub fn juego(&self) -> Option<&Juego> {
        self.juego.as_ref()
    }

    /// El juego está adelante en pantalla completa ahora mismo.
    pub fn al_frente(&self) -> bool {
        self.al_frente
    }

    /// Mira qué hay adelante. Conviene llamarlo unas 4 veces por segundo.
    pub fn revisar(&mut self) -> Cambios {
        let mut cambios = Cambios::default();
        unsafe {
            if self.juego.as_ref().is_some_and(|j| !sigue_activo(j.proceso)) {
                log(&format!("el juego se cerró: {}", self.juego.as_ref().map(|j| j.nombre.as_str()).unwrap_or("")));
                self.juego = None;
                self.al_frente = false;
                cambios.juego = true;
            }
            if let Some(pid) = self.forzado {
                if self.juego.is_none() {
                    self.juego = identificar(pid, HWND::default(), 0, true);
                    cambios.juego = self.juego.is_some();
                }
                self.al_frente = self.juego.is_some();
                return cambios;
            }
            if self.elegido.is_some() {
                return self.revisar_elegido(cambios);
            }
            let ventana = GetForegroundWindow();
            let mut pid = 0u32;
            GetWindowThreadProcessId(ventana, Some(&mut pid));
            let Some(monitor) = monitor_en_pantalla_completa(ventana) else {
                self.al_frente = false;
                return cambios;
            };
            if let Some(j) = self.juego.as_mut().filter(|j| j.pid == pid) {
                self.al_frente = true;
                if j.monitor != monitor {
                    j.monitor = monitor;
                    cambios.monitor = true;
                }
                return cambios;
            }
            self.al_frente = false;
            if pid == std::process::id() || pid == self.descartado || pid == 0 {
                return cambios;
            }
            match identificar(pid, ventana, monitor, false) {
                Some(j) => {
                    self.juego = Some(j);
                    self.al_frente = true;
                    cambios.juego = true;
                    cambios.monitor = true;
                }
                None => self.descartado = pid,
            }
        }
        cambios
    }

    /// Con el juego elegido a mano: se busca su ventana (cada 2 s mientras no esté abierto) y está
    /// "adelante" cuando su ventana es la del primer plano, aunque no ocupe toda la pantalla.
    unsafe fn revisar_elegido(&mut self, mut cambios: Cambios) -> Cambios {
        let Some(exe) = self.elegido.clone() else { return cambios };
        if self.juego.is_none() {
            let ahora = reloj::ahora();
            if ahora >= self.proxima_busqueda {
                self.proxima_busqueda = ahora + 2 * SEGUNDO;
                let encontrado =
                    ventanas_de_programas().into_iter().find(|&(_, pid)| exe_de(pid).is_some_and(|(e, _)| e == exe));
                if let Some((ventana, pid)) = encontrado {
                    let monitor = MonitorFromWindow(ventana, MONITOR_DEFAULTTONEAREST).0 as isize;
                    self.juego = identificar(pid, ventana, monitor, true);
                    cambios.juego = self.juego.is_some();
                    cambios.monitor = cambios.juego;
                }
            }
        }
        let ventana = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(ventana, Some(&mut pid));
        self.al_frente = false;
        if let Some(j) = self.juego.as_mut().filter(|j| j.pid == pid) {
            if !IsIconic(ventana).as_bool() {
                self.al_frente = true;
                let monitor = MonitorFromWindow(ventana, MONITOR_DEFAULTTONEAREST).0 as isize;
                if j.monitor != monitor {
                    j.monitor = monitor;
                    cambios.monitor = true;
                }
            }
        }
        cambios
    }
}

/// Exe de un proceso: (nombre en minúsculas, ruta completa).
fn exe_de(pid: u32) -> Option<(String, String)> {
    unsafe {
        let proceso = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let ruta = ruta_del_exe(proceso);
        let _ = CloseHandle(proceso);
        let ruta = ruta?;
        let nombre = std::path::Path::new(&ruta).file_name()?.to_string_lossy().to_lowercase();
        Some((nombre, ruta))
    }
}

unsafe extern "system" fn juntar(ventana: HWND, lp: LPARAM) -> BOOL {
    (*(lp.0 as *mut Vec<HWND>)).push(ventana);
    TRUE
}

/// Ventanas "de programa": visibles, con título, sin dueño, que no son herramientas ni están ocultas
/// (las apps de la tienda dejan ventanas invisibles "encapotadas").
fn ventanas_de_programas() -> Vec<(HWND, u32)> {
    let mut todas: Vec<HWND> = Vec::new();
    let mut lista = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(juntar), LPARAM(&mut todas as *mut Vec<HWND> as isize));
        for v in todas {
            if !IsWindowVisible(v).as_bool() || GetWindowTextLengthW(v) == 0 || GetWindow(v, GW_OWNER).is_ok_and(|d| !d.is_invalid()) {
                continue;
            }
            if GetWindowLongW(v, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
                continue;
            }
            let mut oculta = 0u32;
            let _ = DwmGetWindowAttribute(v, DWMWA_CLOAKED, &mut oculta as *mut u32 as *mut _, 4);
            if oculta != 0 {
                continue;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(v, Some(&mut pid));
            if pid != 0 && pid != std::process::id() {
                lista.push((v, pid));
            }
        }
    }
    lista
}

/// Para cada exe (nombre en minúsculas), los procesos "raíz" que están abiertos: los que no fueron
/// abiertos por otro proceso del mismo programa. Capturar el árbol de una raíz incluye a sus hijos (en
/// Discord y en los navegadores el sonido sale de un proceso hijo).
pub fn procesos_raiz(exes: &[String]) -> Vec<Vec<u32>> {
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let mut procesos: Vec<(u32, u32, String)> = Vec::new();
    unsafe {
        if let Ok(foto) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut hay = Process32FirstW(foto, &mut e).is_ok();
            while hay {
                procesos.push((e.th32ProcessID, e.th32ParentProcessID, texto(&e.szExeFile).to_lowercase()));
                hay = Process32NextW(foto, &mut e).is_ok();
            }
            let _ = CloseHandle(foto);
        }
    }
    exes.iter()
        .map(|exe| {
            let pids: HashSet<u32> = procesos.iter().filter(|(_, _, e)| e == exe).map(|(p, _, _)| *p).collect();
            let mut raices: Vec<u32> =
                procesos.iter().filter(|(_, padre, e)| e == exe && !pids.contains(padre)).map(|(p, _, _)| *p).collect();
            raices.sort_unstable();
            raices
        })
        .collect()
}

/// Los programas abiertos con ventana, sin repetir y ordenados por nombre (para elegir el juego).
pub fn programas_abiertos() -> Vec<Programa> {
    let mut vistos = HashSet::new();
    let mut lista = Vec::new();
    for (ventana, pid) in ventanas_de_programas() {
        let Some((exe, ruta)) = exe_de(pid) else { continue };
        let base = exe.trim_end_matches(".exe").to_string();
        if DE_WINDOWS.contains(&base.as_str()) || !vistos.insert(exe.clone()) {
            continue;
        }
        let nombre = unsafe { descripcion_del_exe(&ruta) }.unwrap_or_else(|| {
            let mut buf = [0u16; 256];
            let n = unsafe { GetWindowTextW(ventana, &mut buf) }.max(0) as usize;
            texto(&buf[..n])
        });
        lista.push(Programa { exe, nombre });
    }
    lista.sort_by_key(|p| p.nombre.to_lowercase());
    lista
}

/// Un nombre que sirve como carpeta en Windows ("VALORANT", "Rocket League").
pub fn nombre_para_carpeta(nombre: &str) -> String {
    let limpio: String = nombre.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { ' ' } else { c }).collect();
    let limpio = limpio.split_whitespace().collect::<Vec<_>>().join(" ");
    let limpio = limpio.trim_end_matches('.').chars().take(60).collect::<String>();
    if limpio.is_empty() {
        "Juego".into()
    } else {
        limpio
    }
}
