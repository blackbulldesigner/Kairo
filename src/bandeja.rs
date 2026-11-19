//! La grabadora vive en la bandeja del sistema (junto al reloj): ícono, menú con clic derecho,
//! atajos globales y los mensajes que le manda la ventana de ajustes. No tiene interfaz gráfica
//! propia, así que mientras juegas solo usa lo que necesita para grabar.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Media::MediaFoundation::{MFShutdown, MFStartup, MFSTARTUP_FULL, MF_VERSION};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::{Calidad, Config, CualAtajo, Resolucion};
use crate::estado::{self, Estado};
use crate::grabadora::{sonido, Grabadora};
use crate::registro::log;

pub const CLASE: PCWSTR = w!("KairoGrabadora");
const MUTEX: PCWSTR = w!("Local\\Kairo.Grabadora");

const WM_BANDEJA: u32 = WM_APP + 1;
/// La ventana de ajustes guardó cambios: hay que volver a leer config.json.
pub const WM_RECARGAR: u32 = WM_APP + 2;
/// wparam 1 = soltar los atajos globales (mientras se elige uno nuevo); 0 = volver a tomarlos.
pub const WM_SUSPENDER_ATAJOS: u32 = WM_APP + 3;
const WM_CLIP_LISTO: u32 = WM_APP + 4;
/// El hilo de actualizaciones encontró una versión nueva (de la que todavía no se avisó).
const WM_ACTUALIZACION: u32 = WM_APP + 5;
/// Clic en el globo de aviso (llega en el lparam de WM_BANDEJA).
const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;

const ID_GUARDAR: usize = 1;
const ID_PAUSA: usize = 2;
const ID_CARPETA: usize = 3;
const ID_AJUSTES: usize = 4;
const ID_SALIR: usize = 5;
const ID_ACTUALIZAR: usize = 6;
const TEMPORIZADOR: usize = 1;

static MSG_BARRA_DE_TAREAS: AtomicU32 = AtomicU32::new(0);

struct Bandeja {
    hwnd: HWND,
    cfg: Config,
    grabadora: Option<Grabadora>,
    pausado: bool,
    atajos_suspendidos: bool,
    error_atajos: Option<String>,
    guardando: Arc<AtomicBool>,
    resultado_clip: Arc<Mutex<Option<Result<String, String>>>>,
    ultimo_clip: Option<String>,
    icono: HICON,
    icono_gris: HICON,
    estado_escrito: Option<Estado>,
    error_avisado: Option<String>,
    /// La laptop está desenchufada (en escritorio, siempre false).
    en_bateria: bool,
    /// El último globo fue el de una versión nueva (un clic abre Acerca de).
    globo_de_actualizacion: bool,
}

/// ¿Es una laptop funcionando con la batería?
fn en_bateria_ahora() -> bool {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut s = SYSTEM_POWER_STATUS::default();
    // BatteryFlag 128 = no hay batería; 255 = no se sabe. ACLineStatus 0 = desenchufada.
    unsafe { GetSystemPowerStatus(&mut s) }.is_ok() && !matches!(s.BatteryFlag, 128 | 255) && s.ACLineStatus == 0
}

thread_local! {
    static APP: RefCell<Option<Bandeja>> = const { RefCell::new(None) };
}

/// Usa el estado de la bandeja. Ojo: nada de lo que se haga adentro puede mandar mensajes a la
/// ventana de forma sincrónica (volvería a entrar acá).
fn con_app<R>(f: impl FnOnce(&mut Bandeja) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut b| b.as_mut().map(f)))
}

fn copiar(destino: &mut [u16], texto: &str) {
    let v: Vec<u16> = texto.encode_utf16().take(destino.len() - 1).collect();
    destino[..v.len()].copy_from_slice(&v);
    destino[v.len()] = 0;
}

/// Ícono de la bandeja a partir del dibujo de Kairo (formato de recurso: BITMAPINFOHEADER + BGRA + máscara).
unsafe fn crear_icono(n: usize, color: [u8; 3]) -> HICON {
    let rgba = crate::icono::rgba(n, color);
    let fila_mascara = n.div_ceil(32) * 4;
    let mut d = Vec::with_capacity(40 + n * n * 4 + fila_mascara * n);
    d.extend_from_slice(&40u32.to_le_bytes());
    d.extend_from_slice(&(n as i32).to_le_bytes());
    d.extend_from_slice(&(n as i32 * 2).to_le_bytes());
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&32u16.to_le_bytes());
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend_from_slice(&((n * n * 4 + fila_mascara * n) as u32).to_le_bytes());
    d.extend_from_slice(&[0u8; 16]);
    for y in (0..n).rev() {
        for x in 0..n {
            let i = (y * n + x) * 4;
            d.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    d.resize(d.len() + fila_mascara * n, 0);
    CreateIconFromResourceEx(&d, TRUE, 0x0003_0000, n as i32, n as i32, LR_DEFAULTCOLOR).unwrap_or_default()
}

/// ¿Cambió algo que obligue a reiniciar la grabación? (Volumen, carpeta o atajos no.)
fn cambia_la_grabacion(a: &Config, b: &Config) -> bool {
    let mut x = a.clone();
    x.carpeta = b.carpeta.clone();
    x.sonido_al_guardar = b.sonido_al_guardar;
    x.carpeta_por_juego = b.carpeta_por_juego;
    x.pistas_separadas = b.pistas_separadas;
    x.atajo_guardar = b.atajo_guardar.clone();
    x.atajo_pausa = b.atajo_pausa.clone();
    x.atajo_carpeta = b.atajo_carpeta.clone();
    x.buscar_actualizaciones = b.buscar_actualizaciones;
    x.audio_juego.volumen = b.audio_juego.volumen;
    x.audio_sistema.volumen = b.audio_sistema.volumen;
    x.audio_mic.volumen = b.audio_mic.volumen;
    x != *b
}

impl Bandeja {
    fn datos_icono(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: self.hwnd, uID: 1, ..Default::default() }
    }

    /// ¿Está activo el ahorro con batería ahora mismo?
    fn ahorrando(&self) -> bool {
        self.cfg.ahorro_bateria && self.en_bateria
    }

    /// Lo que se graba de verdad: con el ahorro activo, 720p y calidad Ligera.
    fn cfg_grabacion(&self) -> Config {
        let mut c = self.cfg.clone();
        if self.ahorrando() {
            c.resolucion = Resolucion::P720;
            c.calidad = Calidad::Ligera;
        }
        c
    }

    fn grabando(&self) -> bool {
        self.grabadora.as_ref().and_then(|g| g.info.lock().ok().map(|i| i.grabando)).unwrap_or(false)
    }

    fn texto_atajo_guardar(&self) -> String {
        self.cfg.atajo(CualAtajo::Guardar).map(|a| a.texto()).unwrap_or_else(|| "sin atajo".into())
    }

    fn tooltip(&self) -> String {
        if self.pausado {
            "Kairo · en pausa".into()
        } else if self.grabando() {
            let ahorro = if self.ahorrando() { " · ahorro con batería" } else { "" };
            format!("Kairo · guarda los últimos {} ({}){ahorro}", crate::config::texto_duracion(self.cfg.segundos), self.texto_atajo_guardar())
        } else {
            "Kairo · no está grabando".into()
        }
    }

    unsafe fn agregar_icono(&self) {
        let mut d = self.datos_icono();
        d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        d.uCallbackMessage = WM_BANDEJA;
        d.hIcon = self.icono;
        copiar(&mut d.szTip, &self.tooltip());
        let _ = Shell_NotifyIconW(NIM_ADD, &d);
    }

    unsafe fn actualizar_icono(&self) {
        let mut d = self.datos_icono();
        d.uFlags = NIF_ICON | NIF_TIP;
        d.hIcon = if self.grabando() && !self.pausado { self.icono } else { self.icono_gris };
        copiar(&mut d.szTip, &self.tooltip());
        let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
    }

    unsafe fn globo(&mut self, titulo: &str, texto: &str, advertencia: bool) {
        self.globo_de_actualizacion = false;
        let mut d = self.datos_icono();
        d.uFlags = NIF_INFO;
        copiar(&mut d.szInfoTitle, titulo);
        copiar(&mut d.szInfo, texto);
        d.dwInfoFlags = if advertencia { NIIF_WARNING } else { NIIF_INFO };
        let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
    }

    unsafe fn registrar_atajos(&mut self) {
        for c in CualAtajo::TODOS {
            let _ = UnregisterHotKey(self.hwnd, id_atajo(c));
        }
        self.error_atajos = None;
        if self.atajos_suspendidos {
            return;
        }
        let mut ocupados = Vec::new();
        for c in CualAtajo::TODOS {
            let Some(a) = self.cfg.atajo(c) else { continue };
            let Some(vk) = a.tecla_virtual() else {
                ocupados.push(a.texto());
                continue;
            };
            let mut m = MOD_NOREPEAT.0;
            if a.ctrl {
                m |= MOD_CONTROL.0;
            }
            if a.shift {
                m |= MOD_SHIFT.0;
            }
            if a.alt {
                m |= MOD_ALT.0;
            }
            if RegisterHotKey(self.hwnd, id_atajo(c), HOT_KEY_MODIFIERS(m), vk).is_err() {
                ocupados.push(a.texto());
            }
        }
        if !ocupados.is_empty() {
            let texto = format!("Otro programa ya usa {}. Elige otro atajo en Kairo → Atajos.", ocupados.join(" y "));
            log(&format!("atajos: {texto}"));
            self.error_atajos = Some(texto);
        }
    }

    fn iniciar_grabadora(&mut self) {
        if !self.pausado && self.grabadora.is_none() {
            self.grabadora = Some(Grabadora::iniciar(&self.cfg_grabacion()));
        }
    }

    fn detener_grabadora(&mut self) {
        if let Some(g) = self.grabadora.take() {
            g.detener();
        }
    }

    unsafe fn alternar_pausa(&mut self) {
        self.pausado = !self.pausado;
        if self.pausado {
            self.detener_grabadora();
            log("en pausa");
        } else {
            self.iniciar_grabadora();
            log("grabación reanudada");
        }
        self.actualizar_estado();
    }

    unsafe fn recargar(&mut self) {
        let antes = self.cfg_grabacion();
        self.cfg = Config::cargar();
        let reiniciar = cambia_la_grabacion(&antes, &self.cfg_grabacion());
        self.registrar_atajos();
        if reiniciar {
            log("ajustes nuevos: se reinicia la grabación");
            self.detener_grabadora();
            self.iniciar_grabadora();
        }
        self.actualizar_estado();
    }

    /// Se enchufó o desenchufó la laptop: si eso cambia lo que se graba, se reinicia la grabación.
    unsafe fn cambio_de_energia(&mut self) {
        let ahora = en_bateria_ahora();
        if ahora == self.en_bateria {
            return;
        }
        let antes = self.cfg_grabacion();
        self.en_bateria = ahora;
        log(if ahora { "laptop desenchufada" } else { "laptop enchufada" });
        if cambia_la_grabacion(&antes, &self.cfg_grabacion()) {
            log(if self.ahorrando() { "ahorro con batería: 720p, calidad Ligera" } else { "fin del ahorro con batería" });
            self.detener_grabadora();
            self.iniciar_grabadora();
        }
        self.actualizar_estado();
    }

    unsafe fn guardar_clip(&mut self) {
        let Some(guardador) = self.grabadora.as_ref().map(|g| g.guardador()) else {
            sonido::error();
            return;
        };
        if self.guardando.swap(true, Ordering::SeqCst) {
            return; // ya se está guardando uno
        }
        let (cfg, guardando, resultado) = (self.cfg.clone(), self.guardando.clone(), self.resultado_clip.clone());
        let hwnd = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
            }
            let r = guardador.guardar(&cfg);
            match &r {
                Ok(_) if cfg.sonido_al_guardar => sonido::guardado(),
                Ok(_) => {}
                Err(e) => {
                    log(&format!("no se pudo guardar el clip: {e:#}"));
                    sonido::error();
                }
            }
            if let Ok(mut x) = resultado.lock() {
                *x = Some(r.map(|p| p.display().to_string()).map_err(|e| e.to_string()));
            }
            guardando.store(false, Ordering::SeqCst);
            unsafe {
                let _ = PostMessageW(HWND(hwnd as *mut _), WM_CLIP_LISTO, WPARAM(0), LPARAM(0));
            }
        });
    }

    unsafe fn clip_listo(&mut self) {
        let r = self.resultado_clip.lock().ok().and_then(|mut x| x.take());
        match r {
            Some(Ok(ruta)) => self.ultimo_clip = Some(ruta),
            Some(Err(e)) => self.globo("No se pudo guardar el clip", &e, true),
            None => {}
        }
        self.actualizar_estado();
    }

    /// Cada segundo (y después de cada cambio): escribe estado.json, actualiza el ícono y avisa
    /// una sola vez de cada error nuevo.
    unsafe fn actualizar_estado(&mut self) {
        let info = self.grabadora.as_ref().and_then(|g| g.info.lock().ok().map(|i| i.clone())).unwrap_or_default();
        let error = if self.pausado { None } else { info.error.clone() }.or_else(|| self.error_atajos.clone());
        let e = Estado {
            grabando: info.grabando && !self.pausado,
            pausado: self.pausado,
            detalle: info.detalle.clone(),
            juego: info.juego.clone(),
            error: error.clone(),
            ultimo_clip: self.ultimo_clip.clone(),
            ahorro: self.ahorrando() && !self.pausado,
        };
        if self.estado_escrito.as_ref() != Some(&e) {
            estado::escribir(&e);
            self.actualizar_icono();
            if let Some(err) = &error {
                if self.error_avisado.as_ref() != Some(err) {
                    self.globo("Kairo no puede grabar", err, true);
                }
            }
            self.error_avisado = error;
            self.estado_escrito = Some(e);
        }
    }

    unsafe fn avisar_actualizacion(&mut self) {
        let Some(d) = crate::actualizacion::conocida() else { return };
        self.globo(
            "Hay una versión nueva de Kairo",
            &format!("Kairo {} ya está disponible. Haz clic para actualizar (tus ajustes y tus clips no se tocan).", d.version),
            false,
        );
        self.globo_de_actualizacion = true;
    }

    unsafe fn cerrar(mut self) {
        self.detener_grabadora();
        let _ = Shell_NotifyIconW(NIM_DELETE, &self.datos_icono());
        estado::escribir(&Estado::default());
        let _ = DestroyIcon(self.icono);
        let _ = DestroyIcon(self.icono_gris);
        log("Kairo se cerró");
    }
}

fn id_atajo(c: CualAtajo) -> i32 {
    match c {
        CualAtajo::Guardar => ID_GUARDAR as i32,
        CualAtajo::Pausa => ID_PAUSA as i32,
        CualAtajo::Carpeta => ID_CARPETA as i32,
    }
}

/// Abre la ventana de ajustes (un proceso aparte del mismo exe).
pub fn abrir_ajustes() {
    abrir_ajustes_en(None);
}

/// Abre la ventana de ajustes en una página ("acerca", "biblioteca"…).
fn abrir_ajustes_en(pagina: Option<&str>) {
    if let Ok(exe) = std::env::current_exe() {
        let mut c = std::process::Command::new(exe);
        c.arg("--ajustes");
        if let Some(p) = pagina {
            c.args(["--pagina", p]);
        }
        let _ = c.spawn();
    }
}

/// Busca versiones nuevas en un hilo que casi siempre duerme: un minuto después de arrancar y después cada
/// hora mira si pasó un día desde la última consulta. Si hay una nueva, avisa una sola vez.
fn vigilar_actualizaciones(hwnd: HWND) {
    let hwnd = hwnd.0 as isize;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(60));
        crate::actualizacion::limpiar_descargas();
        loop {
            if Config::cargar().buscar_actualizaciones {
                if let Some(d) = crate::actualizacion::revisar(std::time::Duration::from_secs(24 * 3600)) {
                    if crate::actualizacion::marcar_avisada(&d.version) {
                        log(&format!("hay una versión nueva: {}", d.version));
                        unsafe {
                            let _ = PostMessageW(HWND(hwnd as *mut _), WM_ACTUALIZACION, WPARAM(0), LPARAM(0));
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    });
}

/// Abre la grabadora en la bandeja sin ventanas.
pub fn iniciar_en_fondo() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--fondo").spawn();
    }
}

fn abrir_carpeta(cfg: &Config) {
    let _ = std::fs::create_dir_all(&cfg.carpeta);
    let _ = std::process::Command::new("explorer").arg(&cfg.carpeta).spawn();
}

/// La ventana oculta de la grabadora, si está abierta.
pub fn ventana() -> Option<HWND> {
    unsafe { FindWindowW(CLASE, PCWSTR::null()).ok().filter(|h| !h.is_invalid()) }
}

/// Le manda un aviso a la grabadora (si está abierta).
pub fn avisar(mensaje: u32, wparam: usize) {
    if let Some(h) = ventana() {
        unsafe {
            let _ = PostMessageW(h, mensaje, WPARAM(wparam), LPARAM(0));
        }
    }
}

unsafe fn accion(hwnd: HWND, id: usize) {
    match id {
        ID_GUARDAR => {
            con_app(|b| b.guardar_clip());
        }
        ID_PAUSA => {
            con_app(|b| b.alternar_pausa());
        }
        ID_CARPETA => {
            con_app(|b| abrir_carpeta(&b.cfg));
        }
        ID_AJUSTES => abrir_ajustes(),
        ID_ACTUALIZAR => abrir_ajustes_en(Some("acerca")),
        ID_SALIR => {
            let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        _ => {}
    }
}

unsafe fn mostrar_menu(hwnd: HWND) {
    let Some((pausado, atajo)) = con_app(|b| (b.pausado, b.texto_atajo_guardar())) else { return };
    let Ok(menu) = CreatePopupMenu() else { return };
    let item = |id: usize, texto: &str| {
        let _ = AppendMenuW(menu, MF_STRING, id, &HSTRING::from(texto));
    };
    item(ID_GUARDAR, &format!("Guardar clip\t{atajo}"));
    item(ID_PAUSA, if pausado { "Reanudar la grabación" } else { "Pausar la grabación" });
    item(ID_CARPETA, "Abrir la carpeta de clips");
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    item(ID_AJUSTES, "Ajustes…");
    if let Some(d) = crate::actualizacion::conocida() {
        item(ID_ACTUALIZAR, &format!("Actualizar a Kairo {}…", d.version));
    }
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    item(ID_SALIR, "Salir de Kairo");
    let _ = SetMenuDefaultItem(menu, ID_AJUSTES as u32, 0);
    let mut p = POINT::default();
    let _ = GetCursorPos(&mut p);
    // Sin esto el menú no se cierra al hacer clic afuera.
    let _ = SetForegroundWindow(hwnd);
    let elegido = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, p.x, p.y, 0, hwnd, None);
    let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);
    if elegido.0 != 0 {
        accion(hwnd, elegido.0 as usize);
    }
}

unsafe extern "system" fn procedimiento(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_BANDEJA => {
            match lp.0 as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => abrir_ajustes(),
                WM_RBUTTONUP | WM_CONTEXTMENU => mostrar_menu(hwnd),
                NIN_BALLOONUSERCLICK => {
                    if con_app(|b| b.globo_de_actualizacion).unwrap_or(false) {
                        abrir_ajustes_en(Some("acerca"));
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_HOTKEY | WM_COMMAND => {
            accion(hwnd, wp.0 & 0xFFFF);
            LRESULT(0)
        }
        WM_RECARGAR => {
            con_app(|b| b.recargar());
            LRESULT(0)
        }
        WM_SUSPENDER_ATAJOS => {
            con_app(|b| {
                b.atajos_suspendidos = wp.0 != 0;
                b.registrar_atajos();
            });
            LRESULT(0)
        }
        WM_CLIP_LISTO => {
            con_app(|b| b.clip_listo());
            LRESULT(0)
        }
        WM_ACTUALIZACION => {
            con_app(|b| b.avisar_actualizacion());
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            con_app(|b| b.cambio_de_energia());
            LRESULT(1)
        }
        WM_TIMER => {
            con_app(|b| b.actualizar_estado());
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(b) = APP.with(|a| a.borrow_mut().take()) {
                b.cerrar();
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        m if m != 0 && m == MSG_BARRA_DE_TAREAS.load(Ordering::Relaxed) => {
            // El explorador se reinició: hay que volver a poner el ícono.
            con_app(|b| b.agregar_icono());
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Arranca la grabadora en la bandeja. `fondo` = sin abrir la ventana de ajustes (inicio con Windows).
pub fn ejecutar(fondo: bool) {
    unsafe {
        // Una sola grabadora: si ya hay una, abrir el exe de nuevo solo abre los ajustes.
        let mutex = CreateMutexW(None, TRUE, MUTEX);
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if !fondo {
                abrir_ajustes();
            }
            return;
        }
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();
        if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
            log(&format!("Media Foundation no está disponible: {e}"));
        }
        windows::Win32::Media::timeBeginPeriod(1);
        crate::autoinicio::reparar();
        log(&format!("Kairo {} arrancó{}", crate::version::corta(), if fondo { " (con Windows)" } else { "" }));

        let Ok(modulo) = GetModuleHandleW(None) else { return };
        let clase = WNDCLASSW { lpfnWndProc: Some(procedimiento), hInstance: modulo.into(), lpszClassName: CLASE, ..Default::default() };
        RegisterClassW(&clase);
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), CLASE, w!("Kairo"), WINDOW_STYLE(0), 0, 0, 0, 0, HWND::default(), HMENU::default(), modulo, None)
        else {
            log("no se pudo crear la ventana de la bandeja");
            return;
        };
        MSG_BARRA_DE_TAREAS.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        let lado = GetSystemMetrics(SM_CXSMICON).max(16) as usize;
        let a = crate::ui::tema::ACENTO;
        let g = crate::ui::tema::TENUE;
        let bandeja = Bandeja {
            hwnd,
            cfg: Config::cargar(),
            grabadora: None,
            pausado: false,
            atajos_suspendidos: false,
            error_atajos: None,
            guardando: Arc::new(AtomicBool::new(false)),
            resultado_clip: Arc::new(Mutex::new(None)),
            ultimo_clip: None,
            icono: crear_icono(lado, [a.r(), a.g(), a.b()]),
            icono_gris: crear_icono(lado, [g.r(), g.g(), g.b()]),
            estado_escrito: None,
            error_avisado: None,
            en_bateria: en_bateria_ahora(),
            globo_de_actualizacion: false,
        };
        APP.with(|x| *x.borrow_mut() = Some(bandeja));
        con_app(|b| {
            b.agregar_icono();
            b.registrar_atajos();
            b.iniciar_grabadora();
            b.actualizar_estado();
        });
        SetTimer(hwnd, TEMPORIZADOR, 1000, None);
        vigilar_actualizaciones(hwnd);
        if !fondo {
            abrir_ajustes();
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = MFShutdown();
        if let Ok(m) = mutex {
            let _ = CloseHandle(m);
        }
    }
}
