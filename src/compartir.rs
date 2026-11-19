//! Sacar clips de la ventana de Kairo hacia otros programas: arrastrarlos (a Discord, al Explorador, al
//! navegador) y copiarlos al portapapeles como archivo, para pegarlos con Ctrl+V. La grabadora no lo usa.

use std::cell::{Cell, RefCell};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{GlobalFree, BOOL, HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT};
use windows::Win32::System::Ole::{IDropSource, OleInitialize, CF_HDROP, DROPEFFECT_COPY};
use windows::Win32::UI::Input::KeyboardAndMouse::GetCapture;
use windows::Win32::UI::Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName, SHDoDragDrop, DROPFILES};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, PostMessageW, RegisterClassW, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
    WM_LBUTTONUP, WNDCLASSW,
};

/// Mensajes de la ventanita invisible del hilo.
const ARRASTRAR: u32 = WM_APP + 1;
const PROBAR: u32 = WM_APP + 2;

thread_local! {
    /// Archivo por arrastrar apenas vuelva el bucle de mensajes de la ventana.
    static POR_ARRASTRAR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    /// Hay un arrastre en curso (mientras dura, la ventana se sigue dibujando y podría pedir otro).
    static ARRASTRANDO: Cell<bool> = const { Cell::new(false) };
    /// La prueba del bucle anidado pendiente (ventana y segundos).
    static PRUEBA: RefCell<Option<(eframe::egui::Context, f32)>> = const { RefCell::new(None) };
    /// Ventanita invisible (solo de mensajes) que recibe el aviso para arrancar el arrastre.
    static BUZON: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
}

/// Empieza a arrastrar el archivo fuera de la ventana, como si se arrastrara desde el Explorador. Hay que
/// llamarla desde el hilo de la ventana con el botón del mouse todavía apretado. El arrastre de Windows tiene
/// su propio bucle de mensajes y no puede correr en medio de un cuadro de egui: arranca apenas termina el
/// cuadro, con un mensaje a la ventanita del hilo.
pub fn arrastrar(ruta: &Path) {
    if ARRASTRANDO.get() {
        return;
    }
    POR_ARRASTRAR.with(|p| *p.borrow_mut() = Some(ruta.to_path_buf()));
    avisar(ARRASTRAR);
}

/// Manda el mensaje a la ventanita del hilo (la crea la primera vez). Es un mensaje común: Windows lo entrega
/// apenas termina el cuadro, antes que los repintados (un temporizador, en cambio, espera a que no quede nada
/// por dibujar, y con una animación andando puede no llegar nunca).
fn avisar(mensaje: u32) {
    unsafe {
        let mut h = BUZON.get();
        if h.is_invalid() {
            let instancia: HINSTANCE = GetModuleHandleW(None).map(Into::into).unwrap_or_default();
            let clase = WNDCLASSW { lpfnWndProc: Some(buzon), hInstance: instancia, lpszClassName: w!("KairoCompartir"), ..Default::default() };
            // Si ya estaba registrada, esto falla y sirve igual.
            RegisterClassW(&clase);
            h = CreateWindowExW(WINDOW_EX_STYLE(0), w!("KairoCompartir"), None, WINDOW_STYLE(0), 0, 0, 0, 0, HWND_MESSAGE, None, instancia, None)
                .unwrap_or_default();
            BUZON.set(h);
        }
        if PostMessageW(h, mensaje, WPARAM(0), LPARAM(0)).is_err() {
            crate::registro::log("no se pudo avisar para arrastrar");
        }
    }
}

unsafe extern "system" fn buzon(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        ARRASTRAR => al_arrastrar(),
        PROBAR => al_probar(),
        _ => return DefWindowProcW(h, msg, w, l),
    }
    LRESULT(0)
}

unsafe fn al_arrastrar() {
    let Some(ruta) = POR_ARRASTRAR.with(|p| p.borrow_mut().take()) else { return };
    // Mientras el botón sigue apretado, la ventana tiene el mouse capturado; si no, ya se soltó.
    let ventana = GetCapture();
    if ventana.is_invalid() || ARRASTRANDO.get() {
        return;
    }
    ARRASTRANDO.set(true);
    if let Err(e) = arrastrar_ahora(ventana, &ruta) {
        crate::registro::log(&format!("no se pudo arrastrar «{}»: {e}", ruta.display()));
    }
    ARRASTRANDO.set(false);
    // El arrastre se quedó con el "soltar" del botón: se lo pasamos a la ventana para que no siga creyendo
    // que está apretado.
    let mut p = POINT::default();
    let _ = GetCursorPos(&mut p);
    let _ = ScreenToClient(ventana, &mut p);
    let lparam = (((p.y as u16 as u32) << 16) | p.x as u16 as u32) as isize;
    let _ = PostMessageW(ventana, WM_LBUTTONUP, WPARAM(0), LPARAM(lparam));
}

/// El paquete de datos de un archivo: el mismo que arma el Explorador al arrastrarlo (con su miniatura).
unsafe fn paquete(ruta: &Path) -> windows::core::Result<IDataObject> {
    let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(ruta.as_os_str()), None)?;
    item.BindToHandler(None, &BHID_DataObject)
}

unsafe fn arrastrar_ahora(ventana: HWND, ruta: &Path) -> windows::core::Result<()> {
    // La ventana ya lo hizo (winit); si no, esto lo deja listo en este hilo.
    let _ = OleInitialize(None);
    let datos = paquete(ruta)?;
    // Solo copiar: soltarlo en una carpeta nunca mueve el clip.
    SHDoDragDrop(ventana, &datos, None::<&IDropSource>, DROPEFFECT_COPY)?;
    Ok(())
}

/// Para probar sin mouse (feature `dev`): por el mismo camino que el arrastre, arma el paquete de un archivo y
/// corre un bucle de mensajes anidado como el del arrastre durante `segundos`; anota cuántos cuadros dibujó la
/// ventana mientras tanto.
pub fn probar_bucle_anidado(ctx: &eframe::egui::Context, segundos: f32) {
    PRUEBA.with(|p| *p.borrow_mut() = Some((ctx.clone(), segundos)));
    avisar(PROBAR);
}

unsafe fn al_probar() {
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, PostQuitMessage, TranslateMessage, MSG, PM_REMOVE, WM_QUIT};
    let Some((ctx, segundos)) = PRUEBA.with(|p| p.borrow_mut().take()) else { return };
    let _ = OleInitialize(None);
    if let Ok(exe) = std::env::current_exe() {
        let r = paquete(&exe);
        crate::registro::log(&format!("paquete para arrastrar: {}", if r.is_ok() { "listo".to_string() } else { format!("{:?}", r.err()) }));
    }
    ARRASTRANDO.set(true);
    let cuadros = ctx.cumulative_pass_nr();
    let fin = std::time::Instant::now() + std::time::Duration::from_secs_f32(segundos);
    let mut mensajes = 0;
    'bucle: while std::time::Instant::now() < fin {
        ctx.request_repaint();
        let mut msg = MSG::default();
        // (Mirando el reloj también acá: si la ventana pide cuadros sin parar, la cola nunca se vacía.)
        while std::time::Instant::now() < fin && PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                PostQuitMessage(msg.wParam.0 as i32);
                break 'bucle;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
            mensajes += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    ARRASTRANDO.set(false);
    crate::registro::log(&format!("bucle anidado: {mensajes} mensajes, {} cuadros dibujados adentro", ctx.cumulative_pass_nr() - cuadros));
}

/// Copia los archivos al portapapeles como "Copiar" en el Explorador: se pegan con Ctrl+V en Discord, en una
/// carpeta o en el navegador.
pub fn copiar(rutas: &[PathBuf]) -> Result<(), String> {
    unsafe { copiar_en(rutas) }.map_err(|e| format!("No se pudo copiar al portapapeles ({}).", e.message()))
}

unsafe fn copiar_en(rutas: &[PathBuf]) -> windows::core::Result<()> {
    // CF_HDROP: un DROPFILES y después las rutas en UTF-16, cada una terminada en \0 y la lista en \0\0.
    let mut lista: Vec<u16> = Vec::new();
    for r in rutas {
        lista.extend(r.as_os_str().encode_wide());
        lista.push(0);
    }
    lista.push(0);
    let cabecera = std::mem::size_of::<DROPFILES>();
    let archivos = memoria(cabecera + lista.len() * 2, |p| {
        *(p as *mut DROPFILES) = DROPFILES { pFiles: cabecera as u32, fWide: BOOL(1), ..Default::default() };
        std::ptr::copy_nonoverlapping(lista.as_ptr(), p.add(cabecera) as *mut u16, lista.len());
    })?;
    // "Copiar" y no "cortar": si se pega en una carpeta, el clip queda donde estaba.
    let efecto = memoria(4, |p| *(p as *mut u32) = DROPEFFECT_COPY.0)?;

    // El portapapeles necesita una ventana dueña: una invisible, solo para esto.
    let dueña = CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), None, WINDOW_STYLE(0), 0, 0, 0, 0, HWND_MESSAGE, None, None, None)?;
    // Otro programa puede estar usando el portapapeles: se reintenta un ratito.
    let mut abierto = OpenClipboard(dueña);
    for _ in 0..10 {
        if abierto.is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        abierto = OpenClipboard(dueña);
    }
    // Lo que Windows acepta pasa a ser suyo; lo que no, se libera acá.
    let (mut suyo_archivos, mut suyo_efecto) = (false, false);
    let r = abierto.and_then(|_| {
        let r = EmptyClipboard().and_then(|_| SetClipboardData(CF_HDROP.0 as u32, HANDLE(archivos.0)));
        if r.is_ok() {
            suyo_archivos = true;
            // Opcional: sin esto, Ctrl+V igual pega el archivo.
            suyo_efecto = SetClipboardData(RegisterClipboardFormatW(w!("Preferred DropEffect")), HANDLE(efecto.0)).is_ok();
        }
        let _ = CloseClipboard();
        r.map(|_| ())
    });
    let _ = DestroyWindow(dueña);
    if !suyo_archivos {
        let _ = GlobalFree(archivos);
    }
    if !suyo_efecto {
        let _ = GlobalFree(efecto);
    }
    r
}

/// Un bloque de memoria global (lo que pide el portapapeles), llenado por `llenar`.
unsafe fn memoria(bytes: usize, llenar: impl FnOnce(*mut u8)) -> windows::core::Result<HGLOBAL> {
    let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes)?;
    let p = GlobalLock(h) as *mut u8;
    if p.is_null() {
        let _ = GlobalFree(h);
        return Err(windows::core::Error::from_win32());
    }
    llenar(p);
    let _ = GlobalUnlock(h);
    Ok(h)
}
