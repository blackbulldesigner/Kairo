//! Colores, fuentes e íconos. Paleta oscura tomada de Transcribichad.

use eframe::egui::{self, vec2, Color32, FontData, FontDefinitions, FontFamily, FontId, Rounding, Stroke, TextStyle, Visuals};

pub const FONDO: Color32 = Color32::from_rgb(0x0D, 0x10, 0x15);
pub const PANEL: Color32 = Color32::from_rgb(0x14, 0x18, 0x21);
pub const BORDE: Color32 = Color32::from_rgb(0x22, 0x26, 0x2F);
pub const LINEA: Color32 = Color32::from_rgb(0x1D, 0x21, 0x2A);
pub const HOVER: Color32 = Color32::from_rgb(0x1A, 0x1F, 0x2A);
pub const SELECCION: Color32 = Color32::from_rgb(0x19, 0x28, 0x3D);
pub const SEGMENTO: Color32 = Color32::from_rgb(0x21, 0x2B, 0x35);
pub const SEGMENTO_BORDE: Color32 = Color32::from_rgb(0x2C, 0x36, 0x42);
pub const BOTON: Color32 = Color32::from_rgb(0x1C, 0x20, 0x2B);
pub const BOTON_HOVER: Color32 = Color32::from_rgb(0x24, 0x29, 0x35);
pub const BOTON_BORDE: Color32 = Color32::from_rgb(0x30, 0x34, 0x3D);
pub const CHIP: Color32 = Color32::from_rgb(0x16, 0x1A, 0x23);
pub const PISTA: Color32 = Color32::from_rgb(0x26, 0x2A, 0x33);

pub const TEXTO: Color32 = Color32::from_rgb(0xE6, 0xEB, 0xF1);
pub const TEXTO_SEL: Color32 = Color32::from_rgb(0xD3, 0xE2, 0xF8);
pub const NAV: Color32 = Color32::from_rgb(0xA1, 0xAC, 0xB8);
pub const SECUNDARIO: Color32 = Color32::from_rgb(0x95, 0x9C, 0xA7);
pub const TENUE: Color32 = Color32::from_rgb(0x6B, 0x70, 0x79);
pub const APAGADO: Color32 = Color32::from_rgb(0x4A, 0x4E, 0x57);

pub const ACENTO: Color32 = Color32::from_rgb(0x3B, 0x7F, 0xE8);
pub const ACENTO_HOVER: Color32 = Color32::from_rgb(0x4C, 0x8C, 0xF0);
pub const ACENTO_APAGADO: Color32 = Color32::from_rgb(0x25, 0x45, 0x75);
pub const TEXTO_ACENTO_APAGADO: Color32 = Color32::from_rgb(0x7D, 0x93, 0xB8);
pub const VERDE: Color32 = Color32::from_rgb(0x35, 0xC6, 0x83);
pub const ROJO: Color32 = Color32::from_rgb(0xE8, 0x5D, 0x5D);
pub const AMARILLO: Color32 = Color32::from_rgb(0xE8, 0xB0, 0x4B);

/// Íconos de Segoe MDL2 Assets (viene con Windows 10 y 11).
pub mod icono {
    pub const VIDEO: &str = "\u{E714}";
    pub const MONITOR: &str = "\u{E7F4}";
    pub const JUEGO: &str = "\u{E7FC}";
    pub const MICROFONO: &str = "\u{E720}";
    /// Pila de fotogramas.
    pub const CLIPS: &str = "\u{E8B9}";
    /// Carpeta horizontal (la E8B7 de MDL2 sale vertical y parece una tablet).
    pub const CARPETA_ABIERTA: &str = "\u{ED25}";
    pub const TECLADO: &str = "\u{E765}";
    pub const AJUSTES: &str = "\u{E713}";
    pub const COPIAR: &str = "\u{E8C8}";
    pub const EDITAR: &str = "\u{E70F}";
    pub const INFO: &str = "\u{E946}";
    /// Abrir en otra ventana (para enlaces).
    pub const ENLACE: &str = "\u{E8A7}";
    /// Tacho de basura.
    pub const QUITAR: &str = "\u{E74D}";
}

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn iconos() -> FontFamily {
    FontFamily::Name("iconos".into())
}

pub fn aplicar(ctx: &egui::Context) {
    fuentes(ctx);
    estilo(ctx);
}

/// Segoe UI (la de Windows) para el texto y Segoe MDL2 Assets para los íconos. Si alguna falta,
/// se usa la fuente que trae egui.
fn fuentes(ctx: &egui::Context) {
    let carpeta = std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts");
    let mut f = FontDefinitions::default();
    let base = f.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut cargar = |nombre: &str, archivo: &str| -> bool {
        match std::fs::read(carpeta.join(archivo)) {
            Ok(datos) => {
                f.font_data.insert(nombre.to_string(), FontData::from_owned(datos));
                true
            }
            Err(_) => false,
        }
    };
    let regular = cargar("segoe", "segoeui.ttf");
    let semi = cargar("segoe-sb", "seguisb.ttf");
    let mdl2 = cargar("mdl2", "segmdl2.ttf");

    let mut proporcional = base.clone();
    if regular {
        proporcional.insert(0, "segoe".into());
    }
    let mut negrita = proporcional.clone();
    if semi {
        negrita.insert(0, "segoe-sb".into());
    }
    let mut iconos_lista = proporcional.clone();
    if mdl2 {
        iconos_lista.insert(0, "mdl2".into());
    }
    f.families.insert(FontFamily::Proportional, proporcional);
    f.families.insert(semibold(), negrita);
    f.families.insert(iconos(), iconos_lista);
    ctx.set_fonts(f);
}

fn estilo(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = FONDO;
    v.window_fill = PANEL;
    v.extreme_bg_color = FONDO;
    v.faint_bg_color = PANEL;
    v.window_stroke = Stroke::new(1.0_f32, BORDE);
    v.window_rounding = Rounding::same(8.0);
    v.menu_rounding = Rounding::same(8.0);
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.weak_bg_fill = PANEL;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, SECUNDARIO);
    for (w, fondo) in [
        (&mut v.widgets.inactive, BOTON),
        (&mut v.widgets.hovered, BOTON_HOVER),
        (&mut v.widgets.active, BOTON_HOVER),
        (&mut v.widgets.open, BOTON_HOVER),
    ] {
        w.bg_fill = fondo;
        w.weak_bg_fill = fondo;
        w.bg_stroke = Stroke::new(1.0_f32, BOTON_BORDE);
        w.fg_stroke = Stroke::new(1.0_f32, TEXTO);
        w.rounding = Rounding::same(7.0);
        w.expansion = 0.0;
    }
    v.selection.bg_fill = SELECCION;
    v.selection.stroke = Stroke::new(1.0_f32, TEXTO_SEL);
    v.hyperlink_color = ACENTO;
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.text_styles = [
            (TextStyle::Heading, FontId::new(21.0, semibold())),
            (TextStyle::Body, FontId::proportional(13.5)),
            (TextStyle::Button, FontId::proportional(13.5)),
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.interaction.selectable_labels = false;
    });
}
