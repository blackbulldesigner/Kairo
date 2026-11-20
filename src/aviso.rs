//! Aviso en pantalla al guardar un clip o una foto: un cartelito arriba a la derecha del monitor del juego, por
//! unos 2,5 segundos. Es una ventana aparte de la grabadora que no toma el foco ni los clics, se dibuja una sola
//! vez por aviso (después solo cambia su transparencia) y no sale en los clips: Windows la deja fuera de las
//! capturas. Por eso es rectangular y sin sombra: Windows no deja sacar de las capturas una ventana con
//! transparencia por píxel (`UpdateLayeredWindow` da "no hay memoria"), solo con transparencia pareja.
//! Funciona encima de juegos en ventana sin bordes; en pantalla completa exclusiva no se ve.

use std::cell::{Cell, RefCell};
use std::time::Instant;

use anyhow::{anyhow, Result};
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW, EndPaint, GdiFlush,
    GetMonitorInfoW, GetTextExtentPoint32W, InvalidateRect, MonitorFromWindow, SelectObject, SetBkMode, SetTextColor, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, HBITMAP, HDC, HFONT,
    HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTOPRIMARY, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetForegroundWindow, KillTimer, RegisterClassW, SetLayeredWindowAttributes, SetTimer, SetWindowDisplayAffinity,
    SetWindowPos, ShowWindow, HWND_TOPMOST, LWA_ALPHA, SWP_NOACTIVATE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, WDA_EXCLUDEFROMCAPTURE,
    WM_MOUSEACTIVATE, WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::registro::log;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tipo {
    Clip,
    Foto,
    Error,
}

/// Cuánto dura cada parte (segundos): aparece, queda y se va.
const ENTRA: f32 = 0.16;
const QUEDA: f32 = 2.3;
const SALE: f32 = 0.35;
/// Un poco transparente también cuando está quieto.
const OPACIDAD: f32 = 0.96;

thread_local! {
    static VENTANA: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static ABIERTO: RefCell<Option<Abierto>> = const { RefCell::new(None) };
    static FUERA_DE_CAPTURAS: Cell<bool> = const { Cell::new(false) };
}

/// El aviso que se está viendo.
struct Abierto {
    dibujo: Dibujo,
    /// Dónde va cuando ya entró (arriba a la derecha).
    pos: POINT,
    escala: f32,
    inicio: Instant,
}

/// Muestra el aviso (si había otro, lo reemplaza). Hay que llamarla desde el hilo de la bandeja.
pub fn mostrar(tipo: Tipo, titulo: &str, detalle: &str) {
    unsafe {
        // Nítido en pantallas con escala (125 %, 150 %...): esta parte trabaja en píxeles reales.
        let anterior = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if let Err(e) = mostrar_en(tipo, titulo, detalle) {
            log(&format!("no se pudo mostrar el aviso: {e:#}"));
        }
        if !anterior.0.is_null() {
            SetThreadDpiAwarenessContext(anterior);
        }
    }
}

unsafe fn mostrar_en(tipo: Tipo, titulo: &str, detalle: &str) -> Result<()> {
    // El monitor donde está el juego (o lo que esté adelante).
    let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = GetMonitorInfoW(monitor, &mut info);
    let (mut dpi, mut dpi_y) = (96u32, 96u32);
    let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y);
    let escala = dpi as f32 / 96.0;

    let dibujo = Dibujo::nuevo(tipo, titulo, detalle, escala)?;
    let hwnd = ventana()?;
    // Arriba a la derecha, a 22 puntos de los bordes.
    let margen = (22.0 * escala).round() as i32;
    let pos = POINT { x: info.rcMonitor.right - margen - dibujo.ancho, y: info.rcMonitor.top + margen };
    let (ancho, alto) = (dibujo.ancho, dibujo.alto);
    ABIERTO.with(|a| *a.borrow_mut() = Some(Abierto { dibujo, pos, escala, inicio: Instant::now() }));
    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
    // Encima del juego (los juegos en ventana sin bordes no son "siempre visibles"), sin tomar el foco.
    let _ = SetWindowPos(hwnd, HWND_TOPMOST, pos.x, pos.y, ancho, alto, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    let _ = InvalidateRect(hwnd, None, false);
    // Que no salga en los clips (antes de Windows 10 2004 eso la mostraría en negro: ahí se deja como está).
    if !FUERA_DE_CAPTURAS.get() {
        let compilacion = crate::equipo::compilacion_windows();
        if compilacion >= 19041 {
            match SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) {
                Ok(()) => FUERA_DE_CAPTURAS.set(true),
                Err(e) => log(&format!("el aviso no se pudo dejar fuera de las capturas (Windows {compilacion}): {e}")),
            }
        }
    }
    paso(hwnd);
    SetTimer(hwnd, 1, 15, None);
    Ok(())
}

/// La afinidad de pantalla de la ventana del aviso (17 = fuera de las capturas), para las pruebas.
#[cfg_attr(not(feature = "dev"), allow(dead_code))]
pub fn afinidad() -> Option<u32> {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowDisplayAffinity;
    let h = VENTANA.get();
    let mut a = 0u32;
    (!h.is_invalid() && unsafe { GetWindowDisplayAffinity(h, &mut a) }.is_ok()).then_some(a)
}

/// La ventana del aviso (se crea la primera vez y se reusa).
unsafe fn ventana() -> Result<HWND> {
    let h = VENTANA.get();
    if !h.is_invalid() {
        return Ok(h);
    }
    let instancia = GetModuleHandleW(None)?;
    let clase = WNDCLASSW { lpfnWndProc: Some(procedimiento), hInstance: instancia.into(), lpszClassName: w!("KairoAviso"), ..Default::default() };
    RegisterClassW(&clase);
    let h = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
        w!("KairoAviso"),
        w!("Kairo"),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        instancia,
        None,
    )?;
    VENTANA.set(h);
    Ok(h)
}

unsafe extern "system" fn procedimiento(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            paso(hwnd);
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            ABIERTO.with(|a| {
                if let Some(a) = a.borrow().as_ref() {
                    let _ = BitBlt(dc, 0, 0, a.dibujo.ancho, a.dibujo.alto, a.dibujo.dc, 0, 0, SRCCOPY);
                }
            });
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        // Los clics pasan de largo y nunca toma el foco.
        WM_NCHITTEST => LRESULT(-1),
        WM_MOUSEACTIVATE => LRESULT(3),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Un cuadro de la animación: entra subiendo un poquito, queda y se desvanece.
unsafe fn paso(hwnd: HWND) {
    let terminado = ABIERTO.with(|a| {
        let a = a.borrow();
        let Some(a) = a.as_ref() else { return true };
        let t = a.inicio.elapsed().as_secs_f32();
        if t >= ENTRA + QUEDA + SALE {
            return true;
        }
        let (opacidad, subida) = if t < ENTRA {
            let x = t / ENTRA;
            (x, 1.0 - (1.0 - x).powi(3))
        } else if t < ENTRA + QUEDA {
            (1.0, 1.0)
        } else {
            (1.0 - (t - ENTRA - QUEDA) / SALE, 1.0)
        };
        if t < ENTRA + 0.05 {
            let y = a.pos.y + ((1.0 - subida) * 8.0 * a.escala) as i32;
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, a.pos.x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
        }
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), (opacidad.clamp(0.0, 1.0) * OPACIDAD * 255.0) as u8, LWA_ALPHA);
        false
    });
    if terminado {
        let _ = KillTimer(hwnd, 1);
        let _ = ShowWindow(hwnd, SW_HIDE);
        ABIERTO.with(|a| *a.borrow_mut() = None);
    }
}

/// El aviso dibujado en un bitmap de 32 bits (BGRA, opaco).
pub struct Dibujo {
    pub dc: HDC,
    bitmap: HBITMAP,
    viejo: HGDIOBJ,
    bits: *mut u8,
    pub ancho: i32,
    pub alto: i32,
}

impl Drop for Dibujo {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.viejo);
            let _ = DeleteObject(self.bitmap);
            let _ = DeleteDC(self.dc);
        }
    }
}

impl Dibujo {
    /// Los píxeles (BGRA, de arriba hacia abajo), para las pruebas.
    #[cfg_attr(not(feature = "dev"), allow(dead_code))]
    pub fn pixeles(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.bits, (self.ancho * self.alto * 4) as usize) }
    }

    pub unsafe fn nuevo(tipo: Tipo, titulo: &str, detalle: &str, escala: f32) -> Result<Dibujo> {
        let px = |v: f32| (v * escala).round() as i32;
        let fuente = |alto: f32, peso: i32, nombre: &str| -> HFONT {
            // (Altura negativa = alto de las letras; calidad 5 = ClearType.)
            CreateFontW(-px(alto), 0, 0, 0, peso, 0, 0, 0, 1, 0, 0, 5, 0, &HSTRING::from(nombre))
        };
        let f_titulo = fuente(15.0, 600, "Segoe UI");
        let f_detalle = fuente(12.5, 400, "Segoe UI");
        let f_icono = fuente(14.0, 400, "Segoe MDL2 Assets");

        let dc = CreateCompatibleDC(None);
        let medir = |f: HFONT, texto: &str| -> i32 {
            let viejo = SelectObject(dc, f);
            let t: Vec<u16> = texto.encode_utf16().collect();
            let mut s = SIZE::default();
            let _ = GetTextExtentPoint32W(dc, &t, &mut s);
            SelectObject(dc, viejo);
            s.cx
        };
        let ancho_texto = medir(f_titulo, titulo).max(medir(f_detalle, detalle)).min(px(330.0));
        let (circulo, izquierda, derecha) = (px(30.0), px(14.0), px(18.0));
        let ancho = izquierda + circulo + px(12.0) + ancho_texto + derecha;
        let alto = px(58.0);

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: ancho,
                biHeight: -alto,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) if !bits.is_null() => b,
            _ => {
                let _ = DeleteDC(dc);
                for f in [f_titulo, f_detalle, f_icono] {
                    let _ = DeleteObject(f);
                }
                return Err(anyhow!("no se pudo crear el dibujo del aviso"));
            }
        };
        let viejo = SelectObject(dc, bitmap);
        let d = Dibujo { dc, bitmap, viejo, bits: bits as *mut u8, ancho, alto };

        // 1) Fondo con borde de 1 punto y el círculo del ícono (con bordes suaves: el fondo es conocido).
        let (fondo, borde) = ([0x17u8, 0x1C, 0x26], [0x2E, 0x36, 0x42]);
        let color_icono = match tipo {
            Tipo::Clip => [0x35, 0xC6, 0x83],
            Tipo::Foto => [0x3B, 0x7F, 0xE8],
            Tipo::Error => [0xE8, 0x5D, 0x5D],
        };
        let grosor = escala.max(1.0).round() as i32;
        let centro = (izquierda as f32 + circulo as f32 / 2.0, alto as f32 / 2.0);
        let pix = std::slice::from_raw_parts_mut(d.bits, (ancho * alto * 4) as usize);
        for y in 0..alto {
            for x in 0..ancho {
                let en_borde = x < grosor || y < grosor || x >= ancho - grosor || y >= alto - grosor;
                let mut c = if en_borde { borde } else { fondo };
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let dist = ((fx - centro.0).powi(2) + (fy - centro.1).powi(2)).sqrt() - circulo as f32 / 2.0;
                c = mezclar(c, color_icono, (0.5 - dist).clamp(0.0, 1.0));
                let i = ((y * ancho + x) * 4) as usize;
                pix[i..i + 4].copy_from_slice(&[c[2], c[1], c[0], 255]);
            }
        }

        // 2) El texto y el ícono, con ClearType sobre el fondo.
        let _ = SetBkMode(dc, TRANSPARENT);
        let escribir = |f: HFONT, color: [u8; 3], texto: &str, mut r: RECT, formato| {
            let viejo = SelectObject(dc, f);
            SetTextColor(dc, COLORREF(color[0] as u32 | (color[1] as u32) << 8 | (color[2] as u32) << 16));
            let mut t: Vec<u16> = texto.encode_utf16().collect();
            DrawTextW(dc, &mut t, &mut r, formato);
            SelectObject(dc, viejo);
        };
        let glifo = match tipo {
            Tipo::Clip => "\u{E73E}",
            Tipo::Foto => "\u{E722}",
            Tipo::Error => "\u{E711}",
        };
        let circ = RECT { left: izquierda, top: (alto - circulo) / 2, right: izquierda + circulo, bottom: (alto + circulo) / 2 };
        escribir(f_icono, [255, 255, 255], glifo, circ, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        let tx = izquierda + circulo + px(12.0);
        let renglon = |arriba: f32, alto: f32| RECT { left: tx, top: px(arriba), right: tx + ancho_texto, bottom: px(arriba + alto) };
        let formato = DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX;
        escribir(f_titulo, [0xE6, 0xEB, 0xF1], titulo, renglon(9.0, 22.0), formato);
        escribir(f_detalle, [0x95, 0x9C, 0xA7], detalle, renglon(31.0, 18.0), formato);
        let _ = GdiFlush();
        for f in [f_titulo, f_detalle, f_icono] {
            let _ = DeleteObject(f);
        }
        // El texto de Windows deja la transparencia en 0: se vuelve a poner opaco.
        for p in pix.chunks_exact_mut(4) {
            p[3] = 255;
        }
        Ok(d)
    }
}

fn mezclar(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}
