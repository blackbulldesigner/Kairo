//! La página Biblioteca: cuadrícula de clips con miniatura, filtros por juego, búsqueda y orden.

use eframe::egui::{self, pos2, text::LayoutJob, vec2, Align2, Color32, CursorIcon, FontId, Rect, RichText, Rounding, Sense, Stroke, TextFormat, Ui};

use super::app::App;
use super::biblioteca::{hace_cuanto, texto_duracion, Clip, Orden};
use super::tema::*;
use super::widgets::*;
use crate::config::{texto_bytes, CualAtajo};

const SEPARACION: f32 = 14.0;
const ANCHO_MINIMO: f32 = 250.0;

/// Texto en una sola línea, cortado con "…" si no entra.
fn una_linea(ui: &Ui, texto: &str, fuente: FontId, color: Color32, ancho: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(texto.to_owned(), TextFormat { font_id: fuente, color, ..Default::default() });
    job.wrap.max_width = ancho;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.fonts(|f| f.layout_job(job))
}

fn abrir(ruta: &std::path::Path) {
    let _ = std::process::Command::new("explorer").arg(ruta).spawn();
}

fn mostrar_en_carpeta(ruta: &std::path::Path) {
    let mut arg = std::ffi::OsString::from("/select,");
    arg.push(ruta.as_os_str());
    let _ = std::process::Command::new("explorer").arg(arg).spawn();
}

impl App {
    pub(super) fn pagina_biblioteca(&mut self, ui: &mut Ui) {
        let total = self.biblioteca.clips.len();
        let peso: u64 = self.biblioteca.clips.iter().map(|c| c.tamano).sum();
        let sub = if total == 0 {
            "Tus clips aparecen acá.".to_string()
        } else {
            format!("{total} clip{} · {}", if total == 1 { "" } else { "s" }, texto_bytes(peso))
        };
        encabezado(ui, "Biblioteca", &sub);

        // Barra: búsqueda, orden y carpeta.
        ui.horizontal(|ui| {
            let busqueda = egui::TextEdit::singleline(&mut self.biblioteca.busqueda)
                .hint_text("Buscar por nombre o juego")
                .desired_width(260.0)
                .margin(egui::Margin::symmetric(10.0, 7.0));
            ui.scope(|ui| {
                // Como el resto de los controles: fondo hundido, borde suave y esquinas redondeadas.
                let v = ui.visuals_mut();
                v.extreme_bg_color = FONDO;
                v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BOTON_BORDE);
                v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, SEGMENTO_BORDE);
                v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACENTO);
                v.selection.stroke = Stroke::new(1.0_f32, ACENTO);
                ui.add(busqueda);
            });
            ui.add_space(6.0);
            let orden = match self.biblioteca.orden {
                Orden::Nuevos => "Más nuevos",
                Orden::Viejos => "Más viejos",
                Orden::Grandes => "Más pesados",
            };
            egui::ComboBox::new("orden", "").selected_text(orden).width(140.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.biblioteca.orden, Orden::Nuevos, "Más nuevos");
                ui.selectable_value(&mut self.biblioteca.orden, Orden::Viejos, "Más viejos");
                ui.selectable_value(&mut self.biblioteca.orden, Orden::Grandes, "Más pesados");
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if boton(ui, Some(icono::CARPETA_ABIERTA), "Abrir carpeta", Estilo::Secundario, true).clicked() {
                    let _ = std::fs::create_dir_all(&self.guardada.carpeta);
                    abrir(&self.guardada.carpeta);
                }
            });
        });
        ui.add_space(10.0);

        // Juegos (como chips).
        let juegos = self.biblioteca.juegos();
        if !juegos.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                if chip(ui, &format!("Todos  {total}"), self.biblioteca.filtro_juego.is_none()).clicked() {
                    self.biblioteca.filtro_juego = None;
                }
                for (j, n) in &juegos {
                    let sel = self.biblioteca.filtro_juego.as_deref() == Some(j.as_str());
                    if chip(ui, &format!("{j}  {n}"), sel).clicked() {
                        self.biblioteca.filtro_juego = if sel { None } else { Some(j.clone()) };
                    }
                }
            });
            ui.add_space(12.0);
        }

        let clips = self.biblioteca.visibles();
        if clips.is_empty() {
            let atajo = self.guardada.atajo(CualAtajo::Guardar).map(|a| a.texto()).unwrap_or_else(|| "el atajo".into());
            let texto = if total == 0 {
                format!("Todavía no hay clips. Aprieta {atajo} mientras juegas para guardar el primero.")
            } else {
                "Ningún clip coincide con la búsqueda o el filtro.".to_string()
            };
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(icono::CLIPS).family(iconos()).size(36.0).color(APAGADO));
                ui.add_space(8.0);
                ui.label(RichText::new(texto).size(13.5).color(SECUNDARIO));
            });
            return;
        }

        // Cuadrícula: tantas columnas como entren, todas del mismo ancho.
        let ancho = ui.available_width();
        let columnas = (((ancho + SEPARACION) / (ANCHO_MINIMO + SEPARACION)).floor() as usize).max(1);
        let ancho_tarjeta = (ancho - SEPARACION * (columnas - 1) as f32) / columnas as f32;
        for fila_clips in clips.chunks(columnas) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SEPARACION;
                for clip in fila_clips {
                    self.tarjeta_clip(ui, clip, ancho_tarjeta);
                }
            });
            ui.add_space(SEPARACION);
        }

        self.dialogos_biblioteca(ui.ctx());
    }

    fn tarjeta_clip(&mut self, ui: &mut Ui, clip: &Clip, ancho: f32) {
        let alto_imagen = (ancho * 9.0 / 16.0).round();
        let (rect, resp) = ui.allocate_exact_size(vec2(ancho, alto_imagen + 62.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return; // fuera de la vista: ni se dibuja ni se pide la miniatura
        }
        let hover = resp.hovered();
        let p = ui.painter().clone();
        p.rect(rect, 10.0, PANEL, Stroke::new(1.0_f32, if hover { SEGMENTO_BORDE } else { BORDE }));

        // Miniatura (o un fondo con el ícono mientras se genera).
        let imagen = Rect::from_min_size(rect.min, vec2(ancho, alto_imagen)).shrink(1.0);
        let redondeo = Rounding { nw: 9.0, ne: 9.0, sw: 0.0, se: 0.0 };
        let fallo = self.biblioteca.fallo_miniatura(&clip.ruta);
        match self.biblioteca.miniatura(&clip.ruta) {
            Some(tex) => {
                egui::Image::new((tex.id(), tex.size_vec2())).rounding(redondeo).paint_at(ui, imagen);
            }
            None => {
                p.rect_filled(imagen, redondeo, FONDO);
                let (icono, color) = if fallo { (icono::QUITAR, APAGADO) } else { (icono::CLIPS, APAGADO) };
                p.text(imagen.center(), Align2::CENTER_CENTER, icono, FontId::new(26.0, iconos()), color);
            }
        }
        if hover {
            p.rect_filled(imagen, redondeo, Color32::from_white_alpha(10));
        }
        // Duración, abajo a la derecha de la miniatura.
        if let Some(d) = clip.duracion {
            let g = una_linea(ui, &texto_duracion(d), FontId::new(12.0, semibold()), Color32::WHITE, 100.0);
            let caja = Rect::from_min_size(
                pos2(imagen.right() - g.size().x - 18.0, imagen.bottom() - g.size().y - 14.0),
                vec2(g.size().x + 12.0, g.size().y + 6.0),
            );
            p.rect_filled(caja, 5.0, Color32::from_black_alpha(190));
            p.galley(pos2(caja.left() + 6.0, caja.top() + 3.0), g, Color32::WHITE);
        }
        // Nombre y datos.
        let x = rect.left() + 12.0;
        let ancho_texto = ancho - 24.0;
        let titulo = una_linea(ui, &clip.nombre, FontId::new(13.5, semibold()), TEXTO, ancho_texto);
        p.galley(pos2(x, imagen.bottom() + 10.0), titulo, TEXTO);
        let mut datos = Vec::new();
        if let Some(j) = &clip.juego {
            datos.push(j.clone());
        }
        datos.push(texto_bytes(clip.tamano));
        datos.push(hace_cuanto(clip.modificado));
        let meta = una_linea(ui, &datos.join(" · "), FontId::proportional(12.0), SECUNDARIO, ancho_texto);
        p.galley(pos2(x, imagen.bottom() + 32.0), meta, SECUNDARIO);

        let resp = resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Clic para abrir · clic derecho para más opciones");
        if resp.clicked() {
            abrir(&clip.ruta);
        }
        resp.context_menu(|ui| {
            if ui.button("Abrir").clicked() {
                abrir(&clip.ruta);
                ui.close_menu();
            }
            if ui.button("Mostrar en la carpeta").clicked() {
                mostrar_en_carpeta(&clip.ruta);
                ui.close_menu();
            }
            if ui.button("Renombrar…").clicked() {
                self.biblioteca.renombrando = Some((clip.ruta.clone(), clip.nombre.clone()));
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Mandar a la Papelera…").clicked() {
                self.biblioteca.borrando = Some(clip.ruta.clone());
                ui.close_menu();
            }
        });
    }

    /// Ventanitas para renombrar y para confirmar el borrado.
    fn dialogos_biblioteca(&mut self, ctx: &egui::Context) {
        if let Some((ruta, mut nombre)) = self.biblioteca.renombrando.take() {
            let mut abierto = true;
            let mut aceptar = false;
            egui::Window::new("Renombrar clip")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut abierto)
                .show(ctx, |ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut nombre).desired_width(320.0));
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        aceptar = true;
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if boton(ui, None, "Renombrar", Estilo::Primario, !nombre.trim().is_empty()).clicked() {
                            aceptar = true;
                        }
                    });
                });
            if aceptar {
                if let Err(e) = self.biblioteca.renombrar(&ruta, &nombre) {
                    self.avisar(e, true);
                }
            } else if abierto && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.biblioteca.renombrando = Some((ruta, nombre));
            }
        }

        if let Some(ruta) = self.biblioteca.borrando.take() {
            let mut abierto = true;
            let mut borrar = false;
            let mut cancelar = false;
            let nombre = ruta.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            egui::Window::new("Mandar a la Papelera")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut abierto)
                .show(ctx, |ui| {
                    ui.label(RichText::new(format!("«{nombre}» va a la Papelera de reciclaje. Lo puedes recuperar desde ahí.")).color(SECUNDARIO));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        borrar = boton(ui, Some(icono::QUITAR), "Mandar a la Papelera", Estilo::Primario, true).clicked();
                        cancelar = boton(ui, None, "Cancelar", Estilo::Texto, true).clicked();
                    });
                });
            if borrar {
                match self.biblioteca.a_la_papelera(&ruta) {
                    Ok(()) => self.avisar(format!("«{nombre}» se mandó a la Papelera."), false),
                    Err(e) => self.avisar(e, true),
                }
            } else if abierto && !cancelar && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.biblioteca.borrando = Some(ruta);
            }
        }
    }
}
