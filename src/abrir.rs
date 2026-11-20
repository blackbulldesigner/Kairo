//! Abrir carpetas, archivos y páginas con el Shell de Windows. (Con `explorer /select,…` por línea de
//! comandos, una ruta con espacios o paréntesis terminaba abriendo Documentos.)

use std::path::Path;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

fn ejecutar(que: &HSTRING) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        ShellExecuteW(HWND::default(), w!("open"), que, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

/// Abre la carpeta en el Explorador (la crea si no existe).
pub fn carpeta(ruta: &Path) {
    let _ = std::fs::create_dir_all(ruta);
    ejecutar(&HSTRING::from(ruta.as_os_str()));
}

/// Abre la carpeta del archivo con el archivo seleccionado.
pub fn mostrar(archivo: &Path) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        let pidl = ILCreateFromPathW(&HSTRING::from(archivo.as_os_str()));
        if pidl.is_null() {
            // Si el archivo ya no está, al menos su carpeta.
            if let Some(dir) = archivo.parent() {
                carpeta(dir);
            }
            return;
        }
        if SHOpenFolderAndSelectItems(pidl, None, 0).is_err() {
            if let Some(dir) = archivo.parent() {
                carpeta(dir);
            }
        }
        ILFree(Some(pidl));
    }
}

/// Abre el archivo con el programa predeterminado de Windows.
pub fn archivo(ruta: &Path) {
    ejecutar(&HSTRING::from(ruta.as_os_str()));
}

/// Abre una página en el navegador (solo https).
pub fn pagina(url: &str) {
    if url.starts_with("https://") {
        ejecutar(&HSTRING::from(url));
    }
}

/// Manda los archivos a la Papelera de reciclaje (sin preguntar; los que no existen se saltean).
pub fn papelera(rutas: &[std::path::PathBuf]) -> Result<(), String> {
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
