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
    // Una sola ventana de ajustes: si ya hay una, se trae al frente.
    let _candado = if captura.is_none() {
        match ventana::candado_unico() {
            Some(c) => Some(c),
            None => {
                ventana::traer_al_frente(TITULO);
                return Ok(());
            }
        }
    } else {
        None
    };
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

/// Ícono de la ventana (el mismo dibujo que el de la bandeja).
fn icono_app() -> egui::IconData {
    let c = tema::ACENTO;
    egui::IconData { rgba: crate::icono::rgba(64, [c.r(), c.g(), c.b()]), width: 64, height: 64 }
}
