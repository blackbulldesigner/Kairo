//! Ventana de ajustes. Es un proceso aparte de la grabadora: solo existe mientras la tienes abierta,
//! así que no pesa nada mientras juegas.

mod app;
pub mod captura;
mod paginas;
pub mod tema;
mod ventana;
mod widgets;

use eframe::egui;

pub use app::Pagina;

pub const TITULO: &str = "Kairo · Ajustes";

pub fn abrir_ajustes(captura: Option<captura::Pedido>) -> eframe::Result<()> {
    let opciones = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITULO)
            .with_inner_size([980.0, 680.0])
            .with_min_inner_size([820.0, 560.0])
            .with_icon(icono_app()),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(TITULO, opciones, Box::new(move |cc| Ok(Box::new(app::App::new(cc, captura)))))
}

/// Ícono de la ventana: cuadrado redondeado azul con un anillo abierto (rebobinar) y un punto (grabar).
fn icono_app() -> egui::IconData {
    const N: usize = 64;
    const SUB: usize = 4;
    let azul = tema::ACENTO;
    let mut rgba = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let (mut fondo, mut blanco) = (0.0f32, 0.0f32);
            for sy in 0..SUB {
                for sx in 0..SUB {
                    let px = x as f32 + (sx as f32 + 0.5) / SUB as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SUB as f32;
                    // Cuadrado redondeado de 2 a 62 con radio 14.
                    let (cx, cy) = (px.clamp(16.0, 48.0), py.clamp(16.0, 48.0));
                    if (px - cx).powi(2) + (py - cy).powi(2) <= 14.0f32.powi(2) {
                        fondo += 1.0;
                    }
                    let (dx, dy) = (px - 32.0, py - 32.0);
                    let d = (dx * dx + dy * dy).sqrt();
                    let angulo = dy.atan2(dx).to_degrees();
                    let anillo = (14.0..=19.0).contains(&d) && !(-75.0..=-25.0).contains(&angulo);
                    if anillo || d <= 6.5 {
                        blanco += 1.0;
                    }
                }
            }
            let total = (SUB * SUB) as f32;
            let (f, b) = (fondo / total, (blanco / total).min(fondo / total));
            let mezcla = |c: u8| (c as f32 * (1.0 - b / f.max(1e-6)) + 255.0 * (b / f.max(1e-6))).round() as u8;
            let i = (y * N + x) * 4;
            rgba[i] = mezcla(azul.r());
            rgba[i + 1] = mezcla(azul.g());
            rgba[i + 2] = mezcla(azul.b());
            rgba[i + 3] = (f * 255.0).round() as u8;
        }
    }
    egui::IconData { rgba, width: N as u32, height: N as u32 }
}
