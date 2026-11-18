//! Piezas de la interfaz con el estilo de Kairo: tarjetas, filas, interruptores, deslizadores,
//! selectores por segmentos, chips de teclas y botones.

use std::sync::Arc;

use eframe::egui::{
    self, pos2, text::LayoutJob, vec2, Align2, Color32, CursorIcon, FontId, Frame, Galley, Margin, Rect, Response, RichText, Sense,
    Stroke, TextFormat, Ui, UiBuilder,
};

use super::tema::*;
use crate::config::Atajo;

fn medir(ui: &Ui, texto: &str, fuente: FontId) -> Arc<Galley> {
    ui.fonts(|f| f.layout_no_wrap(texto.to_owned(), fuente, Color32::PLACEHOLDER))
}

fn medir_ajustado(ui: &Ui, texto: &str, fuente: FontId, ancho: f32) -> Arc<Galley> {
    ui.fonts(|f| f.layout(texto.to_owned(), fuente, Color32::PLACEHOLDER, ancho))
}

pub fn mezclar(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Título y subtítulo de una página.
pub fn encabezado(ui: &mut Ui, titulo: &str, subtitulo: &str) {
    ui.label(RichText::new(titulo).family(semibold()).size(21.0).color(TEXTO));
    ui.add_space(1.0);
    ui.label(RichText::new(subtitulo).size(13.0).color(SECUNDARIO));
    ui.add_space(16.0);
}

/// Tarjeta con un título en mayúsculas pequeñas.
pub fn tarjeta<R>(ui: &mut Ui, titulo: &str, contenido: impl FnOnce(&mut Ui) -> R) -> R {
    let r = Frame::none()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, BORDE))
        .rounding(10.0)
        .inner_margin(Margin { left: 18.0, right: 18.0, top: 14.0, bottom: 8.0 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut job = LayoutJob::default();
            job.append(
                &titulo.to_uppercase(),
                0.0,
                TextFormat { font_id: FontId::new(11.0, semibold()), color: TENUE, extra_letter_spacing: 0.8, ..Default::default() },
            );
            ui.label(job);
            ui.add_space(4.0);
            contenido(ui)
        })
        .inner;
    ui.add_space(14.0);
    r
}

/// Línea fina entre filas de una tarjeta.
pub fn separador(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0_f32, LINEA));
}

/// Fila de ajuste: título y descripción a la izquierda, el control a la derecha (centrado en alto).
/// El control se agrega de derecha a izquierda.
pub fn fila(ui: &mut Ui, titulo: &str, desc: Option<&str>, control: impl FnOnce(&mut Ui)) {
    let ancho = ui.available_width();
    let ancho_texto = (ancho * 0.48).max(200.0).min((ancho - 120.0).max(160.0));
    let g_tit = medir_ajustado(ui, titulo, FontId::proportional(13.5), ancho_texto);
    let g_desc = desc.map(|d| medir_ajustado(ui, d, FontId::proportional(12.0), ancho_texto));
    let alto_texto = g_tit.size().y + g_desc.as_ref().map_or(0.0, |g| 2.0 + g.size().y);
    let alto = (alto_texto + 18.0).max(48.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ancho, alto), Sense::hover());
    let y = rect.center().y - alto_texto / 2.0;
    let alto_tit = g_tit.size().y;
    ui.painter().galley(pos2(rect.left(), y), g_tit, TEXTO);
    if let Some(g) = g_desc {
        ui.painter().galley(pos2(rect.left(), y + alto_tit + 2.0), g, SECUNDARIO);
    }
    let zona = Rect::from_min_max(pos2(rect.left() + ancho_texto + 16.0, rect.top()), rect.max);
    let mut hijo = ui.new_child(UiBuilder::new().max_rect(zona).layout(egui::Layout::right_to_left(egui::Align::Center)));
    control(&mut hijo);
}

/// Fila de solo lectura: etiqueta a la izquierda (ancho fijo) y el valor a la derecha.
pub fn fila_info(ui: &mut Ui, etiqueta: &str, valor: &str) {
    fila_info_color(ui, etiqueta, valor, TEXTO);
}

pub fn fila_info_color(ui: &mut Ui, etiqueta: &str, valor: &str, color: Color32) {
    const ANCHO_ETIQUETA: f32 = 170.0;
    let ancho = ui.available_width();
    let g_et = medir_ajustado(ui, etiqueta, FontId::proportional(13.0), ANCHO_ETIQUETA - 12.0);
    let g_val = medir_ajustado(ui, valor, FontId::proportional(13.0), (ancho - ANCHO_ETIQUETA).max(100.0));
    let alto = g_et.size().y.max(g_val.size().y) + 18.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ancho, alto), Sense::hover());
    ui.painter().galley(pos2(rect.left(), rect.top() + 9.0), g_et, SECUNDARIO);
    ui.painter().galley(pos2(rect.left() + ANCHO_ETIQUETA, rect.top() + 9.0), g_val, color);
}

/// Un cambio de la lista de novedades: fecha, título y commit. `actual` marca el de este exe.
pub fn fila_novedad(ui: &mut Ui, fecha: &str, texto: &str, commit: &str, actual: bool) {
    const ANCHO_FECHA: f32 = 96.0;
    let ancho = ui.available_width();
    let g_commit = medir(ui, commit, FontId::monospace(11.5));
    let g_chip = actual.then(|| medir(ui, "este exe", FontId::proportional(11.5)));
    let derecha = g_commit.size().x + g_chip.as_ref().map_or(0.0, |g| g.size().x + 22.0);
    let ancho_texto = (ancho - ANCHO_FECHA - derecha - 16.0).max(120.0);
    let g_fecha = medir(ui, fecha, FontId::proportional(12.5));
    let g_texto = medir_ajustado(ui, texto, FontId::proportional(13.0), ancho_texto);
    let alto = g_texto.size().y.max(g_fecha.size().y) + 16.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ancho, alto), Sense::hover());
    let p = ui.painter();
    let y = rect.top() + 8.0;
    p.galley(pos2(rect.left(), y + 1.0), g_fecha, TENUE);
    p.galley(pos2(rect.left() + ANCHO_FECHA, y), g_texto, if actual { TEXTO } else { SECUNDARIO });
    let x_commit = rect.right() - g_commit.size().x;
    p.galley(pos2(x_commit, y + 1.0), g_commit, TENUE);
    if let Some(g) = g_chip {
        let r = Rect::from_min_size(pos2(x_commit - 10.0 - g.size().x - 12.0, y - 1.0), vec2(g.size().x + 12.0, g.size().y + 4.0));
        p.rect_filled(r, 5.0, SELECCION);
        p.galley(pos2(r.left() + 6.0, r.top() + 2.0), g, TEXTO_SEL);
    }
}

/// Texto chico y tenue, para notas debajo de una tarjeta.
pub fn nota(ui: &mut Ui, texto: &str) {
    ui.label(RichText::new(texto).size(12.0).color(TENUE));
}

/// Valor alineado a la derecha en un ancho fijo ("30 s", "100 %").
pub fn valor(ui: &mut Ui, texto: &str, ancho: f32, activo: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ancho, 20.0), Sense::hover());
    let color = if activo { SECUNDARIO } else { APAGADO };
    ui.painter().text(pos2(rect.right(), rect.center().y), Align2::RIGHT_CENTER, texto, FontId::proportional(13.0), color);
}

pub fn interruptor(ui: &mut Ui, encendido: &mut bool, activo: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(36.0, 20.0), if activo { Sense::click() } else { Sense::hover() });
    let mut cambiado = false;
    if activo && resp.clicked() {
        *encendido = !*encendido;
        cambiado = true;
    }
    let t = ui.ctx().animate_bool(resp.id, *encendido);
    let fondo = mezclar(PISTA, ACENTO, t);
    let (fondo, bolita) = if activo { (fondo, Color32::WHITE) } else { (mezclar(PANEL, fondo, 0.45), mezclar(PANEL, Color32::WHITE, 0.4)) };
    let p = ui.painter();
    p.rect_filled(rect, 10.0, fondo);
    let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), t);
    p.circle_filled(pos2(x, rect.center().y), 7.0, bolita);
    if activo {
        resp.on_hover_cursor(CursorIcon::PointingHand);
    }
    cambiado
}

/// Deslizador fino con el tramo recorrido en azul. Devuelve true si cambió el valor.
pub fn deslizador(ui: &mut Ui, valor: &mut f32, rango: std::ops::RangeInclusive<f32>, paso: f32, ancho: f32, activo: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ancho, 22.0), if activo { Sense::click_and_drag() } else { Sense::hover() });
    let (lo, hi) = (*rango.start(), *rango.end());
    let pista = rect.shrink2(vec2(8.0, 0.0));
    let mut cambiado = false;
    if activo && (resp.dragged() || resp.clicked()) {
        if let Some(p) = resp.interact_pointer_pos() {
            let t = ((p.x - pista.left()) / pista.width()).clamp(0.0, 1.0);
            let mut v = lo + t * (hi - lo);
            if paso > 0.0 {
                v = (v / paso).round() * paso;
            }
            let v = v.clamp(lo, hi);
            if (v - *valor).abs() > f32::EPSILON {
                *valor = v;
                cambiado = true;
            }
        }
    }
    let t = if hi > lo { ((*valor - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
    let (y, x) = (rect.center().y, pista.left() + t * pista.width());
    let (lleno, bolita) = if activo { (ACENTO, Color32::WHITE) } else { (mezclar(PANEL, ACENTO, 0.35), mezclar(PANEL, Color32::WHITE, 0.35)) };
    let p = ui.painter();
    p.rect_filled(Rect::from_min_max(pos2(pista.left(), y - 2.0), pos2(pista.right(), y + 2.0)), 2.0, PISTA);
    p.rect_filled(Rect::from_min_max(pos2(pista.left(), y - 2.0), pos2(x, y + 2.0)), 2.0, lleno);
    let r = if activo && (resp.hovered() || resp.dragged()) { 8.0 } else { 7.0 };
    p.circle_filled(pos2(x, y), r, bolita);
    if activo {
        resp.on_hover_cursor(CursorIcon::PointingHand);
    }
    cambiado
}

/// Selector de una opción entre varias, como botones pegados. Con `ancho` las opciones se reparten
/// ese ancho; si no, cada una mide lo que su texto. Devuelve true si cambió.
pub fn segmentos<T: PartialEq + Copy>(
    ui: &mut Ui,
    id: &str,
    valor: &mut T,
    opciones: &[(T, Option<&str>, &str)],
    ancho: Option<f32>,
    alto: f32,
) -> bool {
    let tam_texto = if alto >= 36.0 { 13.5 } else { 13.0 };
    let medidas: Vec<(Arc<Galley>, Option<Arc<Galley>>)> = opciones
        .iter()
        .map(|(_, ic, t)| (medir(ui, t, FontId::proportional(tam_texto)), ic.map(|i| medir(ui, i, FontId::new(14.0, iconos())))))
        .collect();
    let naturales: Vec<f32> =
        medidas.iter().map(|(t, i)| t.size().x + i.as_ref().map_or(0.0, |g| g.size().x + 8.0) + 26.0).collect();
    let total = ancho.unwrap_or(naturales.iter().sum::<f32>() + 6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(total, alto), Sense::hover());
    let interior = rect.shrink(3.0);
    let base = ui.id().with(id);
    let mut cambiado = false;
    let mut x = interior.left();
    let p = ui.painter().clone();
    p.rect(rect, 8.0, FONDO, Stroke::new(1.0_f32, BORDE));
    for (i, ((v, _, _), (gt, gi))) in opciones.iter().zip(medidas).enumerate() {
        let w = match ancho {
            Some(_) => interior.width() / opciones.len() as f32,
            None => naturales[i],
        };
        let r = Rect::from_min_size(pos2(x, interior.top()), vec2(w, interior.height()));
        x += w;
        let resp = ui.interact(r, base.with(i), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        if resp.clicked() && *valor != *v {
            *valor = *v;
            cambiado = true;
        }
        let sel = *valor == *v;
        if sel {
            p.rect(r, 6.0, SEGMENTO, Stroke::new(1.0_f32, SEGMENTO_BORDE));
        } else if resp.hovered() {
            p.rect_filled(r, 6.0, HOVER);
        }
        let color = if sel { TEXTO } else { SECUNDARIO };
        let ancho_contenido = gt.size().x + gi.as_ref().map_or(0.0, |g| g.size().x + 8.0);
        let mut cx = r.center().x - ancho_contenido / 2.0;
        if let Some(g) = gi {
            let w = g.size().x;
            p.galley(pos2(cx, r.center().y - g.size().y / 2.0), g, color);
            cx += w + 8.0;
        }
        p.galley(pos2(cx, r.center().y - gt.size().y / 2.0), gt.clone(), color);
    }
    cambiado
}

/// Chips de teclas de un atajo ("Ctrl" "Shift" "F9"). Clic para cambiarlo.
pub fn atajo(ui: &mut Ui, atajo: Option<&Atajo>, capturando: bool) -> Response {
    let partes: Vec<String> = if capturando {
        vec!["Aprieta una tecla…".into()]
    } else if let Some(a) = atajo {
        a.partes()
    } else {
        vec!["Sin asignar".into()]
    };
    let (alto, pad, sep) = (26.0, 9.0, 5.0);
    let medidas: Vec<Arc<Galley>> = partes.iter().map(|t| medir(ui, t, FontId::proportional(12.5))).collect();
    let ancho: f32 = medidas.iter().map(|g| g.size().x + pad * 2.0).sum::<f32>() + sep * medidas.len().saturating_sub(1) as f32;
    let (rect, resp) = ui.allocate_exact_size(vec2(ancho, alto), Sense::click());
    let hover = resp.hovered();
    let (borde, fondo, color) = if capturando {
        (ACENTO, SELECCION, TEXTO_SEL)
    } else if atajo.is_none() {
        (if hover { BOTON_BORDE } else { BORDE }, FONDO, TENUE)
    } else {
        (if hover { SEGMENTO_BORDE } else { BOTON_BORDE }, CHIP, TEXTO)
    };
    let p = ui.painter();
    let mut x = rect.left();
    for g in medidas {
        let w = g.size().x + pad * 2.0;
        let r = Rect::from_min_size(pos2(x, rect.top()), vec2(w, alto));
        p.rect(r, 6.0, fondo, Stroke::new(1.0_f32, borde));
        p.galley(pos2(r.left() + pad, r.center().y - g.size().y / 2.0), g, color);
        x += w + sep;
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Estilo {
    Primario,
    Secundario,
    Texto,
}

pub fn boton(ui: &mut Ui, icono: Option<&str>, texto: &str, estilo: Estilo, activo: bool) -> Response {
    let gt = medir(ui, texto, FontId::proportional(13.0));
    let gi = icono.map(|i| medir(ui, i, FontId::new(13.0, iconos())));
    let contenido = gt.size().x + gi.as_ref().map_or(0.0, |g| g.size().x + 8.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(contenido + 28.0, 32.0), if activo { Sense::click() } else { Sense::hover() });
    let hover = activo && resp.hovered();
    let (fondo, borde, color) = match (estilo, activo) {
        (Estilo::Primario, true) => (if hover { ACENTO_HOVER } else { ACENTO }, Color32::TRANSPARENT, Color32::WHITE),
        (Estilo::Primario, false) => (ACENTO_APAGADO, Color32::TRANSPARENT, TEXTO_ACENTO_APAGADO),
        (Estilo::Secundario, _) => (if hover { BOTON_HOVER } else { BOTON }, BOTON_BORDE, if activo { TEXTO } else { TENUE }),
        (Estilo::Texto, _) => (if hover { BOTON } else { Color32::TRANSPARENT }, Color32::TRANSPARENT, if activo { NAV } else { APAGADO }),
    };
    let p = ui.painter();
    p.rect(rect, 7.0, fondo, Stroke::new(1.0_f32, borde));
    let mut x = rect.center().x - contenido / 2.0;
    if let Some(g) = gi {
        let w = g.size().x;
        p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, color);
        x += w + 8.0;
    }
    p.galley(pos2(x, rect.center().y - gt.size().y / 2.0), gt, color);
    if activo {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Opción del menú de la izquierda.
pub fn item_menu(ui: &mut Ui, icono: &str, texto: &str, seleccionado: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    let p = ui.painter();
    if seleccionado {
        p.rect_filled(rect, 7.0, SELECCION);
    } else if resp.hovered() {
        p.rect_filled(rect, 7.0, HOVER);
    }
    let color = if seleccionado {
        TEXTO_SEL
    } else if resp.hovered() {
        TEXTO
    } else {
        NAV
    };
    p.text(pos2(rect.left() + 14.0, rect.center().y), Align2::LEFT_CENTER, icono, FontId::new(15.0, iconos()), color);
    p.text(pos2(rect.left() + 40.0, rect.center().y), Align2::LEFT_CENTER, texto, FontId::proportional(13.5), color);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Logo chico (el mismo dibujo que el ícono de la ventana).
pub fn logo(ui: &mut Ui, lado: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(lado, lado), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, lado * 0.24, ACENTO);
    let c = rect.center();
    let radio = lado * 0.26;
    let trazo = Stroke::new(lado * 0.08, Color32::WHITE);
    // Anillo abierto arriba a la derecha.
    let puntos: Vec<egui::Pos2> = (0..=40)
        .map(|i| {
            let a = (-25.0f32 + (360.0 - 50.0) * i as f32 / 40.0).to_radians();
            pos2(c.x + radio * a.cos(), c.y + radio * a.sin())
        })
        .collect();
    p.add(egui::Shape::line(puntos, trazo));
    p.circle_filled(c, lado * 0.1, Color32::WHITE);
}
