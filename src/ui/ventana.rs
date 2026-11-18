//! Barra de título oscura para que la ventana combine con el resto (Windows 10 2004+ y Windows 11).

use windows::Win32::Foundation::{BOOL, COLORREF, FALSE, HWND, LPARAM, TRUE};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWINDOWATTRIBUTE,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{EnumThreadWindows, GetWindowTextW};

use super::tema;

struct Busqueda {
    titulo: Vec<u16>,
    ventana: HWND,
}

unsafe extern "system" fn revisar(hwnd: HWND, lp: LPARAM) -> BOOL {
    let b = &mut *(lp.0 as *mut Busqueda);
    let mut buf = [0u16; 256];
    let n = GetWindowTextW(hwnd, &mut buf);
    if n > 0 && buf[..n as usize] == b.titulo[..] {
        b.ventana = hwnd;
        return FALSE;
    }
    TRUE
}

fn colorref(c: eframe::egui::Color32) -> COLORREF {
    COLORREF(c.r() as u32 | (c.g() as u32) << 8 | (c.b() as u32) << 16)
}

/// Busca la ventana de este hilo con ese título y le pone la barra de título oscura.
/// Si algo no está disponible (Windows viejo), queda la barra normal: no es un error.
pub fn barra_de_titulo_oscura(titulo: &str) {
    let mut b = Busqueda { titulo: titulo.encode_utf16().collect(), ventana: HWND::default() };
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(revisar), LPARAM(&mut b as *mut Busqueda as isize));
        if b.ventana.is_invalid() {
            return;
        }
        let si: BOOL = TRUE;
        let tam = std::mem::size_of::<BOOL>() as u32;
        let p = &si as *const BOOL as *const _;
        // Antes de Windows 10 2004 el atributo era el 19.
        if DwmSetWindowAttribute(b.ventana, DWMWA_USE_IMMERSIVE_DARK_MODE, p, tam).is_err() {
            let _ = DwmSetWindowAttribute(b.ventana, DWMWINDOWATTRIBUTE(19), p, tam);
        }
        // Solo Windows 11: el color exacto del panel.
        for (atributo, color) in [(DWMWA_CAPTION_COLOR, tema::PANEL), (DWMWA_BORDER_COLOR, tema::BORDE), (DWMWA_TEXT_COLOR, tema::NAV)] {
            let c = colorref(color);
            let _ = DwmSetWindowAttribute(b.ventana, atributo, &c as *const COLORREF as *const _, std::mem::size_of::<COLORREF>() as u32);
        }
    }
}
