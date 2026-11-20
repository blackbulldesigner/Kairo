//! Logos que no vienen en los íconos de Windows (Segoe MDL2), dibujados una vez como imagen chica.

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};

/// El de Discord (para "Copiar para Discord"): su silueta en su violeta, con los ojos transparentes.
pub fn discord(ctx: &egui::Context) -> TextureHandle {
    let id = egui::Id::new("logo_discord");
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return t;
    }
    let imagen = dibujar(64, adentro_discord, Color32::from_rgb(0x58, 0x65, 0xF2));
    let t = ctx.load_texture("logo_discord", imagen, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    t
}

/// La silueta en coordenadas de -1 a 1 (y hacia abajo): una cabeza redondeada con dos "patas" abajo, un hueco
/// entre ellas y dos ojos.
fn adentro_discord(x: f32, y: f32) -> bool {
    let elipse = |cx: f32, cy: f32, rx: f32, ry: f32| ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) <= 1.0;
    let cuerpo = elipse(0.0, -0.02, 0.86, 0.66) || rect_redondeado(x, y, -0.94, -0.10, 0.94, 0.70, 0.30);
    let hueco = elipse(0.0, 1.0, 0.56, 0.42);
    let ojos = elipse(-0.34, 0.14, 0.155, 0.175) || elipse(0.34, 0.14, 0.155, 0.175);
    cuerpo && !hueco && !ojos
}

fn rect_redondeado(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> bool {
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    let (cx, cy) = (x.clamp(x0 + r, x1 - r), y.clamp(y0 + r, y1 - r));
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

/// Pinta la forma en una imagen cuadrada de `lado` píxeles, con bordes suaves (4×4 muestras por píxel).
fn dibujar(lado: usize, adentro: fn(f32, f32) -> bool, color: Color32) -> ColorImage {
    const MUESTRAS: usize = 4;
    let mut rgba = Vec::with_capacity(lado * lado * 4);
    for py in 0..lado {
        for px in 0..lado {
            let mut dentro = 0;
            for sy in 0..MUESTRAS {
                for sx in 0..MUESTRAS {
                    let x = (px as f32 + (sx as f32 + 0.5) / MUESTRAS as f32) / lado as f32 * 2.0 - 1.0;
                    let y = (py as f32 + (sy as f32 + 0.5) / MUESTRAS as f32) / lado as f32 * 2.0 - 1.0;
                    dentro += adentro(x, y) as usize;
                }
            }
            let alfa = (dentro * 255 / (MUESTRAS * MUESTRAS)) as u8;
            rgba.extend_from_slice(&[color.r(), color.g(), color.b(), alfa]);
        }
    }
    ColorImage::from_rgba_unmultiplied([lado, lado], &rgba)
}
