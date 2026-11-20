//! Copiar clips al portapapeles como archivo (como "Copiar" en el Explorador), para pegarlos con Ctrl+V en
//! Discord, en una carpeta o en el navegador. Solo la ventana de Kairo lo usa, nunca la grabadora.

use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

use windows::core::w;
use windows::Win32::Foundation::{GlobalFree, BOOL, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT};
use windows::Win32::System::Ole::{CF_HDROP, DROPEFFECT_COPY};
use windows::Win32::UI::Shell::DROPFILES;
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE};

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
