//! Bienvenida: la primera vez que se abre Kairo (todavía no hay ajustes guardados), unos pasos cortos para
//! dejarlo listo: el atajo y cuánto guardar, qué grabar y el micrófono, y la carpeta. Al terminar (o al
//! saltarla) se guardan los ajustes, la grabadora los toma y ya no vuelve a aparecer.

use eframe::egui::{self, pos2, vec2, Align, Align2, Color32, CursorIcon, FontId, Key, Layout, Margin, Rect, RichText, Sense, Stroke, Ui};

use super::app::{App, Pagina};
use super::animar;
use super::tema::*;
use super::widgets::*;
use crate::config::{self, CualAtajo, ModoCaptura};

/// Cuántos pasos tiene (el último es "listo").
const PASOS: usize = 5;
/// Ancho del contenido de la tarjeta y alto fijo del paso (así la tarjeta no salta de tamaño entre pasos).
const ANCHO: f32 = 560.0;
const ALTO_PASO: f32 = 350.0;

impl App {
    /// La bienvenida a pantalla completa, en lugar de las pestañas.
    pub(super) fn pantalla_bienvenida(&mut self, ctx: &egui::Context) {
        let Some(paso) = self.bienvenida else { return };
        // Fondo con un resplandor azul suave detrás de la tarjeta.
        egui::CentralPanel::default().frame(egui::Frame::none().fill(FONDO)).show(ctx, |ui| {
            let zona = ui.max_rect();
            resplandor(ui.painter(), zona.center(), zona.width().max(zona.height()) * 0.5);
        });
        let capturando = self.capturando == Some(CualAtajo::Guardar);
        let mut ir_a: Option<usize> = None;
        let mut terminar = false;
        egui::Area::new(egui::Id::new("bienvenida")).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
            egui::Frame::none()
                .fill(PANEL)
                .stroke(Stroke::new(1.0_f32, BORDE))
                .rounding(18.0)
                .inner_margin(Margin::same(36.0))
                .shadow(egui::epaint::Shadow { offset: vec2(0.0, 18.0), blur: 50.0, spread: 0.0, color: Color32::from_black_alpha(150) })
                .show(ui, |ui| {
                    ui.set_width(ANCHO);
                    // Arriba: el avance y "Saltar".
                    ui.horizontal(|ui| {
                        puntos(ui, paso);
                        if paso + 1 < PASOS {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if boton(ui, None, "Saltar", Estilo::Texto, true).on_hover_text("Deja todo como viene de fábrica (lo cambias cuando quieras)").clicked() {
                                    terminar = true;
                                }
                            });
                        }
                    });
                    ui.add_space(22.0);
                    // El paso nuevo entra con un fundido y sube un poquito.
                    let t = (((self.ahora - self.bienvenida_desde) / 0.3) as f32).clamp(0.0, 1.0);
                    let t = 1.0 - (1.0 - t).powi(3);
                    if t < 1.0 {
                        ui.ctx().request_repaint();
                    }
                    let inicio = ui.cursor().top();
                    ui.scope(|ui| {
                        ui.set_opacity(t);
                        ui.add_space((1.0 - t) * 12.0);
                        match paso {
                            0 => self.paso_hola(ui),
                            1 => self.paso_atajo(ui, capturando),
                            2 => self.paso_que_grabar(ui),
                            3 => self.paso_carpeta(ui),
                            _ => self.paso_listo(ui),
                        }
                    });
                    let usado = ui.cursor().top() - inicio;
                    ui.add_space((ALTO_PASO - usado).max(0.0));
                    // Abajo: atrás y siguiente.
                    ui.add_space(18.0);
                    ui.horizontal(|ui| {
                        if paso > 0 && paso + 1 < PASOS && boton(ui, Some(icono::ATRAS), "Atrás", Estilo::Texto, true).clicked() {
                            ir_a = Some(paso - 1);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let texto = match paso {
                                0 => "Empezar",
                                p if p + 1 == PASOS => "Ir a la Biblioteca",
                                _ => "Siguiente",
                            };
                            if boton(ui, None, texto, Estilo::Primario, true).clicked() {
                                if paso + 1 == PASOS {
                                    terminar = true;
                                } else {
                                    ir_a = Some(paso + 1);
                                }
                            }
                        });
                    });
                });
        });
        // Enter avanza (salvo mientras se elige la tecla).
        if !capturando && ir_a.is_none() && !terminar && ctx.input(|i| i.key_pressed(Key::Enter)) {
            if paso + 1 == PASOS {
                terminar = true;
            } else {
                ir_a = Some(paso + 1);
            }
        }
        if let Some(n) = ir_a {
            self.capturando = None;
            self.bienvenida = Some(n);
            self.bienvenida_desde = self.ahora;
        }
        if terminar {
            self.terminar_bienvenida();
        }
    }

    /// Guarda los ajustes (la grabadora los toma enseguida) y pasa a la Biblioteca.
    fn terminar_bienvenida(&mut self) {
        self.capturando = None;
        self.guardar();
        self.bienvenida = None;
        self.pagina = Pagina::Biblioteca;
        self.cambio_de_vista = self.ahora;
        if self.guardada == self.cfg {
            self.avisar("Todo listo. ¡A jugar!", false);
        }
    }

    fn paso_hola(&mut self, ui: &mut Ui) {
        logo(ui, 56.0);
        ui.add_space(18.0);
        titulo(ui, "Te damos la bienvenida a Kairo");
        texto(ui, "Kairo graba en segundo plano sin que tu juego lo note. Cuando pasa algo bueno, aprietas un atajo y se guardan los últimos segundos.");
        ui.add_space(24.0);
        punto(ui, icono::CLIPS, "Clips al instante", "Con un atajo, aunque estés dentro del juego.");
        ui.add_space(14.0);
        punto(ui, icono::TIJERA, "Recórtalos y compártelos", "Editor, compresor y «Copiar para Discord», sin salir de Kairo.");
        ui.add_space(14.0);
        punto(ui, icono::JUEGO, "Liviano de verdad", "En nuestra prueba le quitó 2,6 % de FPS al juego (Medal, 4,4 %).");
    }

    fn paso_atajo(&mut self, ui: &mut Ui, capturando: bool) {
        titulo(ui, "Tu atajo para guardar");
        texto(ui, "Apriétalo mientras juegas y Kairo guarda lo que acaba de pasar. Funciona aunque estés dentro del juego.");
        ui.add_space(28.0);
        ui.vertical_centered(|ui| {
            if teclas(ui, self.cfg.atajo(CualAtajo::Guardar).map(|a| a.partes()), capturando).clicked() {
                self.capturando = if capturando { None } else { Some(CualAtajo::Guardar) };
            }
            ui.add_space(10.0);
            let ayuda = if capturando { "Aprieta la tecla o la combinación que quieras (Esc cancela)." } else { "Haz clic en la tecla para cambiarla." };
            ui.label(RichText::new(ayuda).size(12.5).color(TENUE));
        });
        ui.add_space(28.0);
        ui.label(RichText::new("¿Cuánto guardar?").family(semibold()).size(14.0).color(TEXTO));
        ui.add_space(8.0);
        segmentos(ui, "bv_segundos", &mut self.cfg.segundos, &[(30, None, "Últimos 30 s"), (60, None, "1 min"), (120, None, "2 min")], Some(ANCHO), 36.0);
        ui.add_space(8.0);
        ui.label(RichText::new("Más tiempo usa más RAM. Lo cambias cuando quieras en Grabación.").size(12.5).color(TENUE));
    }

    fn paso_que_grabar(&mut self, ui: &mut Ui) {
        titulo(ui, "¿Qué grabamos?");
        texto(ui, "Lo puedes cambiar cuando quieras en Grabación.");
        ui.add_space(22.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            let ancho = (ANCHO - 14.0) / 2.0;
            let modo = self.cfg.modo;
            if opcion(ui, ancho, icono::MONITOR, "Todo el escritorio", "Lo que ves en la pantalla: el juego y lo que aparezca encima.", modo == ModoCaptura::Escritorio).clicked() {
                self.cfg.modo = ModoCaptura::Escritorio;
            }
            if opcion(ui, ancho, icono::JUEGO, "Solo el juego", "Kairo lo detecta solo. Si haces Alt+Tab, esa parte queda en negro: tu escritorio no sale.", modo == ModoCaptura::Juego).clicked() {
                self.cfg.modo = ModoCaptura::Juego;
            }
        });
        ui.add_space(20.0);
        fila(ui, "Tu micrófono", Some("Para que en los clips se escuche lo que dices."), |ui| {
            interruptor(ui, &mut self.cfg.audio_mic.activa, true);
        });
    }

    fn paso_carpeta(&mut self, ui: &mut Ui) {
        titulo(ui, "¿Dónde guardamos tus clips?");
        texto(ui, "Cada clip es un MP4 común: lo abres con cualquier reproductor y lo subes a donde quieras.");
        ui.add_space(22.0);
        let mut cambiar = false;
        egui::Frame::none().fill(FONDO).stroke(Stroke::new(1.0_f32, BORDE)).rounding(12.0).inner_margin(Margin::symmetric(16.0, 14.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icono::CARPETA_ABIERTA).family(iconos()).size(20.0).color(ACENTO));
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(RichText::new(self.cfg.carpeta.display().to_string()).size(13.5).color(TEXTO));
                    if let Some(b) = config::espacio_libre(&self.cfg.carpeta) {
                        ui.label(RichText::new(format!("Libre en ese disco: {}", config::texto_bytes(b))).size(12.0).color(SECUNDARIO));
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    cambiar = boton(ui, Some(icono::EDITAR), "Cambiar…", Estilo::Secundario, true).clicked();
                });
            });
        });
        if cambiar {
            let inicio = if self.cfg.carpeta.exists() { self.cfg.carpeta.clone() } else { config::carpeta_videos() };
            if let Some(nueva) = rfd::FileDialog::new().set_title("Elige la carpeta de tus clips").set_directory(inicio).pick_folder() {
                self.cfg.carpeta = nueva;
            }
        }
        ui.add_space(16.0);
        fila(ui, "Una carpeta por juego", Some("Los clips de cada juego van en su carpeta y llevan su nombre."), |ui| {
            interruptor(ui, &mut self.cfg.carpeta_por_juego, true);
        });
    }

    fn paso_listo(&mut self, ui: &mut Ui) {
        let (zona, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
        // El check entra creciendo.
        let t = (((self.ahora - self.bienvenida_desde) / 0.45) as f32).clamp(0.0, 1.0);
        let crece = 1.0 - (1.0 - t).powi(3);
        ui.painter().circle_filled(zona.center(), 28.0 * (0.6 + 0.4 * crece), VERDE);
        ui.painter().text(zona.center(), Align2::CENTER_CENTER, icono::CHECK, FontId::new(24.0 * crece.max(0.3), iconos()), Color32::WHITE);
        ui.add_space(18.0);
        titulo(ui, "¡Listo para grabar!");
        let atajo = self.cfg.atajo(CualAtajo::Guardar).map(|a| a.texto()).unwrap_or_else(|| "tu atajo".into());
        texto(ui, &format!("Aprieta {atajo} mientras juegas: el clip aparece en la Biblioteca y te avisa con un cartelito en pantalla."));
        ui.add_space(18.0);
        // Lo que quedó elegido, de un vistazo.
        let segundos = crate::config::texto_duracion(self.cfg.segundos);
        let modo = if self.cfg.modo == ModoCaptura::Juego { "Solo el juego" } else { "Todo el escritorio" };
        let mic = if self.cfg.audio_mic.activa { "Con micrófono" } else { "Sin micrófono" };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for t in [format!("{atajo} para guardar").as_str(), &format!("Últimos {segundos}"), modo, mic] {
                resumen(ui, t);
            }
        });
        ui.add_space(20.0);
        egui::Frame::none().fill(FONDO).stroke(Stroke::new(1.0_f32, BORDE)).rounding(12.0).inner_margin(Margin::symmetric(16.0, 14.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(icono::INFO).family(iconos()).size(18.0).color(ACENTO));
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "Kairo queda junto al reloj y graba aunque cierres esta ventana. Con clic derecho en su ícono lo pausas o lo cierras.",
                        )
                        .size(13.0)
                        .color(SECUNDARIO),
                    )
                    .wrap(),
                );
            });
        });
    }
}

/// Un resplandor azul suave (degradado de verdad, del centro hacia afuera) detrás de la tarjeta.
fn resplandor(p: &egui::Painter, c: egui::Pos2, radio: f32) {
    use egui::epaint::Mesh;
    // Colores premultiplicados: el azul de Kairo, del 28 % en el centro a nada en el borde.
    let con = |a: f32| Color32::from_rgba_premultiplied((59.0 * a) as u8, (127.0 * a) as u8, (232.0 * a) as u8, (255.0 * a) as u8);
    let anillos = [(0.0, con(0.28)), (0.3, con(0.18)), (0.65, con(0.07)), (1.0, con(0.0))];
    let lados = 72;
    let mut m = Mesh::default();
    for (r, color) in anillos {
        for i in 0..lados {
            let a = i as f32 / lados as f32 * std::f32::consts::TAU;
            m.colored_vertex(c + vec2(a.cos(), a.sin()) * radio * r, color);
        }
    }
    for k in 0..anillos.len() as u32 - 1 {
        for i in 0..lados {
            let (a, b) = (k * lados + i, k * lados + (i + 1) % lados);
            let (c2, d) = (a + lados, b + lados);
            m.add_triangle(a, b, d);
            m.add_triangle(a, d, c2);
        }
    }
    p.add(egui::Shape::mesh(m));
}

/// Una píldora del resumen del final.
fn resumen(ui: &mut Ui, t: &str) {
    let g = ui.fonts(|f| f.layout_no_wrap(t.to_string(), FontId::proportional(13.0), TEXTO_SEL));
    let (r, _) = ui.allocate_exact_size(vec2(g.size().x + 24.0, 30.0), Sense::hover());
    ui.painter().rect(r, 15.0, SELECCION, Stroke::new(1.0_f32, ACENTO_APAGADO));
    ui.painter().galley(pos2(r.left() + 12.0, r.center().y - g.size().y / 2.0), g, TEXTO_SEL);
}

fn titulo(ui: &mut Ui, t: &str) {
    ui.label(RichText::new(t).family(semibold()).size(26.0).color(TEXTO));
    ui.add_space(6.0);
}

fn texto(ui: &mut Ui, t: &str) {
    ui.add(egui::Label::new(RichText::new(t).size(14.5).color(SECUNDARIO)).wrap());
}

/// Una ventaja: ícono en un cuadrito azul, título y una línea.
fn punto(ui: &mut Ui, icono_: &str, titulo: &str, desc: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::hover());
        ui.painter().rect_filled(r, 10.0, SELECCION);
        ui.painter().text(r.center(), Align2::CENTER_CENTER, icono_, FontId::new(17.0, iconos()), TEXTO_SEL);
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(titulo).family(semibold()).size(14.0).color(TEXTO));
            ui.label(RichText::new(desc).size(12.5).color(SECUNDARIO));
        });
    });
}

/// El avance: un punto por paso, el actual alargado (y se estira de a poco al cambiar).
fn puntos(ui: &mut Ui, paso: usize) {
    let base = ui.id().with("bv_puntos");
    let anchos: Vec<f32> = (0..PASOS).map(|i| animar::suave(ui.ctx(), base.with(i), if i == paso { 24.0 } else { 8.0 }, 0.25)).collect();
    let total: f32 = anchos.iter().sum::<f32>() + 6.0 * (PASOS - 1) as f32;
    let (r, _) = ui.allocate_exact_size(vec2(total, 8.0), Sense::hover());
    let mut x = r.left();
    for (i, w) in anchos.into_iter().enumerate() {
        let color = if i <= paso { ACENTO } else { BOTON_BORDE };
        ui.painter().rect_filled(Rect::from_min_size(pos2(x, r.top()), vec2(w, 8.0)), 4.0, color);
        x += w + 6.0;
    }
}

/// Las teclas del atajo, grandes (con un borde de abajo, como una tecla de verdad). Clic para cambiarla.
fn teclas(ui: &mut Ui, partes: Option<Vec<String>>, capturando: bool) -> egui::Response {
    let partes = if capturando { vec!["Aprieta una tecla…".to_string()] } else { partes.unwrap_or_else(|| vec!["Sin asignar".to_string()]) };
    let fuente = FontId::new(if capturando { 18.0 } else { 26.0 }, semibold());
    let galeras: Vec<_> = partes.iter().map(|p| ui.fonts(|f| f.layout_no_wrap(p.clone(), fuente.clone(), TEXTO))).collect();
    let (alto, pad, sep) = (62.0, 22.0, 10.0);
    let anchos: Vec<f32> = galeras.iter().map(|g| (g.size().x + pad * 2.0).max(alto)).collect();
    let total = anchos.iter().sum::<f32>() + sep * anchos.len().saturating_sub(1) as f32;
    let (r, resp) = ui.allocate_exact_size(vec2(total, alto + 5.0), Sense::click());
    let h = animar::hacia(ui.ctx(), resp.id.with("h"), resp.hovered());
    // Mientras espera la tecla, el borde late.
    let latido = if capturando {
        ui.ctx().request_repaint();
        0.5 + 0.5 * (ui.input(|i| i.time) as f32 * 4.0).sin()
    } else {
        0.0
    };
    let borde = if capturando { mezclar(SEGMENTO_BORDE, ACENTO, 0.4 + 0.6 * latido) } else { mezclar(BOTON_BORDE, ACENTO, h * 0.7) };
    let p = ui.painter();
    let mut x = r.left();
    for (g, w) in galeras.into_iter().zip(anchos) {
        let tapa = Rect::from_min_size(pos2(x, r.top()), vec2(w, alto));
        p.rect_filled(tapa.translate(vec2(0.0, 5.0)), 12.0, Color32::from_rgb(0x0B, 0x0E, 0x13));
        p.rect(tapa, 12.0, mezclar(CHIP, BOTON_HOVER, h), Stroke::new(1.5_f32, borde));
        let color = if capturando { TEXTO_SEL } else { TEXTO };
        p.galley(pos2(tapa.center().x - g.size().x / 2.0, tapa.center().y - g.size().y / 2.0), g, color);
        x += w + sep;
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Una opción grande para elegir (como un botón de radio, pero tarjeta).
fn opcion(ui: &mut Ui, ancho: f32, icono_: &str, titulo: &str, desc: &str, elegida: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ancho, 160.0), Sense::click());
    let h = animar::hacia(ui.ctx(), resp.id.with("h"), resp.hovered());
    let s = animar::hacia(ui.ctx(), resp.id.with("s"), elegida);
    let p = ui.painter();
    p.rect(r, 14.0, mezclar(mezclar(BOTON, BOTON_HOVER, h), SELECCION, s), Stroke::new(1.0 + s, mezclar(mezclar(BOTON_BORDE, SEGMENTO_BORDE, h), ACENTO, s)));
    let dentro = r.shrink(18.0);
    p.text(dentro.left_top() + vec2(0.0, 2.0), Align2::LEFT_TOP, icono_, FontId::new(24.0, iconos()), mezclar(SECUNDARIO, ACENTO, s));
    // El círculo de elegida, arriba a la derecha.
    let c = pos2(dentro.right() - 10.0, dentro.top() + 12.0);
    p.circle_stroke(c, 10.0, Stroke::new(1.5_f32, mezclar(BOTON_BORDE, ACENTO, s)));
    if s > 0.01 {
        p.circle_filled(c, 10.0 * s, ACENTO);
        p.text(c, Align2::CENTER_CENTER, icono::CHECK, FontId::new(10.0, iconos()), Color32::WHITE.gamma_multiply(s));
    }
    let g = ui.fonts(|f| f.layout_no_wrap(titulo.to_string(), FontId::new(15.0, semibold()), TEXTO));
    p.galley(pos2(dentro.left(), dentro.top() + 44.0), g, TEXTO);
    let d = ui.fonts(|f| f.layout(desc.to_string(), FontId::proportional(12.5), SECUNDARIO, dentro.width()));
    p.galley(pos2(dentro.left(), dentro.top() + 70.0), d, SECUNDARIO);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}
