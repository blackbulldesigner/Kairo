//! Animaciones suaves para la interfaz. egui interpola en línea recta; acá cada valor recuerda de dónde a dónde
//! va y desde cuándo, y avanza con una curva que frena al final (ease-out). Mientras algo se mueve se pide
//! otro cuadro; quieto, no se dibuja nada de más.

use eframe::egui::{Context, Id};

#[derive(Clone, Copy)]
struct Tramo {
    desde: f32,
    hasta: f32,
    inicio: f64,
}

fn curva(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// El valor animado hacia `objetivo` en `duracion` segundos. La primera vez arranca ya en el objetivo.
pub fn suave(ctx: &Context, id: Id, objetivo: f32, duracion: f32) -> f32 {
    let ahora = ctx.input(|i| i.time);
    let previo: Option<Tramo> = ctx.data(|d| d.get_temp(id));
    let Some(t) = previo else {
        ctx.data_mut(|d| d.insert_temp(id, Tramo { desde: objetivo, hasta: objetivo, inicio: ahora }));
        return objetivo;
    };
    let avance = (((ahora - t.inicio) as f32) / duracion.max(0.001)).clamp(0.0, 1.0);
    let actual = t.desde + (t.hasta - t.desde) * curva(avance);
    if (t.hasta - objetivo).abs() > 0.0001 {
        // Cambió el destino: sale desde donde está ahora, sin saltos.
        ctx.data_mut(|d| d.insert_temp(id, Tramo { desde: actual, hasta: objetivo, inicio: ahora }));
        ctx.request_repaint();
        return actual;
    }
    if avance < 1.0 {
        ctx.request_repaint();
    }
    actual
}

/// 0 → 1 cuando `encendido` (hover, seleccionado…), en 0,15 s.
pub fn hacia(ctx: &Context, id: Id, encendido: bool) -> f32 {
    suave(ctx, id, encendido as u8 as f32, 0.15)
}
