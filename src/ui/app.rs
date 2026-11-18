//! La ventana de ajustes: menú a la izquierda, la página elegida al centro y abajo Descartar / Guardar.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, Align, CentralPanel, Frame, Layout, Margin, RichText, ScrollArea, SidePanel, TopBottomPanel};

use super::captura::Pedido;
use super::tema::{self, *};
use super::widgets::*;
use crate::bandeja;
use crate::config::{Atajo, CualAtajo, Config};
use crate::equipo::Equipo;
use crate::estado::{self, Estado};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pagina {
    Grabacion,
    Audio,
    Clips,
    Atajos,
    Sistema,
    AcercaDe,
}

impl Pagina {
    pub const TODAS: [Pagina; 6] = [Pagina::Grabacion, Pagina::Audio, Pagina::Clips, Pagina::Atajos, Pagina::Sistema, Pagina::AcercaDe];

    pub fn nombre(self) -> &'static str {
        match self {
            Pagina::Grabacion => "Grabación",
            Pagina::Audio => "Audio",
            Pagina::Clips => "Clips",
            Pagina::Atajos => "Atajos",
            Pagina::Sistema => "Sistema",
            Pagina::AcercaDe => "Acerca de",
        }
    }

    fn icono(self) -> &'static str {
        match self {
            Pagina::Grabacion => icono::VIDEO,
            Pagina::Audio => icono::MICROFONO,
            Pagina::Clips => icono::CLIPS,
            Pagina::Atajos => icono::TECLADO,
            Pagina::Sistema => icono::AJUSTES,
            Pagina::AcercaDe => icono::INFO,
        }
    }

    /// Para la línea de comandos: "grabacion", "audio", "clips", "atajos", "sistema", "acerca".
    pub fn desde(s: &str) -> Option<Pagina> {
        match s.to_lowercase().as_str() {
            "grabacion" | "grabación" => Some(Pagina::Grabacion),
            "audio" => Some(Pagina::Audio),
            "clips" => Some(Pagina::Clips),
            "atajos" => Some(Pagina::Atajos),
            "sistema" => Some(Pagina::Sistema),
            "acerca" => Some(Pagina::AcercaDe),
            _ => None,
        }
    }
}

pub struct App {
    pub(super) pagina: Pagina,
    /// Lo que se está editando.
    pub(super) cfg: Config,
    /// Lo último guardado en disco (para saber si hay cambios).
    pub(super) guardada: Config,
    equipo: Arc<Mutex<Option<Equipo>>>,
    /// Atajo que espera una tecla nueva.
    pub(super) capturando: Option<CualAtajo>,
    /// Mensaje del pie: (texto, es error, hasta cuándo se muestra).
    aviso: Option<(String, bool, f64)>,
    ahora: f64,
    captura: Option<Pedido>,
    /// Lo que informa la grabadora (se relee cada segundo). None = la grabadora está cerrada.
    pub(super) estado: Option<Estado>,
    proxima_lectura: f64,
    /// Se le pidió a la grabadora que suelte los atajos (mientras se elige uno nuevo).
    atajos_suspendidos: bool,
    pub(super) autoinicio: bool,
    alto_estado: f32,
    /// Programas abiertos (para elegir el juego a mano) y cuándo volver a leerlos.
    pub(super) programas: Vec<crate::grabadora::juego::Programa>,
    pub(super) programas_hasta: f64,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, captura: Option<Pedido>) -> Self {
        tema::aplicar(&cc.egui_ctx);
        // El zoom lo maneja Kairo según el tamaño de la ventana (ver `ajustar_zoom`).
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        super::ventana::barra_de_titulo_oscura(super::TITULO);

        let equipo = Arc::new(Mutex::new(None));
        {
            let destino = equipo.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                let e = Equipo::detectar();
                if let Ok(mut d) = destino.lock() {
                    *d = Some(e);
                }
                ctx.request_repaint();
            });
        }

        let mut cfg = Config::cargar();
        if captura.as_ref().is_some_and(|c| c.modo_juego) {
            cfg.modo = crate::config::ModoCaptura::Juego;
        }
        if captura.as_ref().is_some_and(|c| c.programas) {
            cfg.otros_sonidos = crate::config::OtrosSonidos::Elegidos;
            if let Some(p) = crate::grabadora::juego::programas_abiertos().into_iter().next() {
                cfg.programas_audio = vec![crate::config::ProgramaAudio { exe: p.exe, nombre: p.nombre, activa: true, volumen: 80 }];
            }
        }
        let pagina = captura.as_ref().map(|c| c.pagina).unwrap_or(Pagina::Grabacion);
        Self {
            pagina,
            guardada: cfg.clone(),
            cfg,
            equipo,
            capturando: None,
            aviso: None,
            ahora: 0.0,
            captura,
            estado: None,
            proxima_lectura: 0.0,
            atajos_suspendidos: false,
            autoinicio: crate::autoinicio::activo(),
            alto_estado: 64.0,
            programas: Vec::new(),
            programas_hasta: 0.0,
        }
    }

    /// Con la ventana grande (por ejemplo, maximizada), todo crece: letras, controles y espacios.
    /// Se calcula con el ancho sin zoom, así cambiar el zoom no vuelve a cambiar la cuenta.
    fn ajustar_zoom(&self, ctx: &egui::Context) {
        let nativo = ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(1.0).max(0.1);
        let ancho = ctx.screen_rect().width() * ctx.pixels_per_point() / nativo;
        // En pasos de 5 %, para no cambiar a cada pixel mientras se arrastra el borde.
        let zoom = ((ancho / 1600.0).clamp(1.0, 1.1) * 20.0).round() / 20.0;
        if (zoom - ctx.zoom_factor()).abs() > 0.01 {
            ctx.set_zoom_factor(zoom);
        }
    }

    /// Relee el estado de la grabadora una vez por segundo.
    fn leer_estado(&mut self, ctx: &egui::Context) {
        if self.ahora >= self.proxima_lectura {
            self.estado = if bandeja::ventana().is_some() { Some(estado::leer().unwrap_or_default()) } else { None };
            self.proxima_lectura = self.ahora + 1.0;
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    /// Mientras se elige un atajo, la grabadora tiene que soltar los suyos (si no, F9 guardaría un clip
    /// en vez de llegar a esta ventana).
    fn sincronizar_atajos(&mut self) {
        let quiere = self.capturando.is_some();
        if quiere != self.atajos_suspendidos {
            bandeja::avisar(bandeja::WM_SUSPENDER_ATAJOS, quiere as usize);
            self.atajos_suspendidos = quiere;
        }
    }

    fn tarjeta_estado(&mut self, ui: &mut egui::Ui) {
        let atajo = self.guardada.atajo(CualAtajo::Guardar).map(|a| a.texto());
        let (color, titulo, detalle, iniciar) = match &self.estado {
            None => (TENUE, "Grabadora cerrada".to_string(), "Kairo no está grabando.".to_string(), true),
            Some(e) if e.pausado => (AMARILLO, "En pausa".into(), "Reanúdala desde el ícono de la bandeja.".into(), false),
            Some(e) if e.grabando => {
                let seg = crate::config::texto_duracion(self.guardada.segundos);
                let tecla = atajo.map(|a| format!(" · {a} guarda")).unwrap_or_default();
                let ahorro = if e.ahorro { "\nAhorro con batería: 720p, Ligera" } else { "" };
                let juego = e.juego.as_ref().map(|j| format!("\nJuego: {j}")).unwrap_or_default();
                (VERDE, "Grabando".into(), format!("Últimos {seg}{tecla}{juego}{ahorro}"), false)
            }
            Some(e) => match &e.error {
                Some(err) => (ROJO, "No puede grabar".into(), err.clone(), false),
                None => (TENUE, "Preparando…".into(), "Abriendo la captura.".into(), false),
            },
        };
        let r = Frame::none().fill(FONDO).stroke(egui::Stroke::new(1.0_f32, BORDE)).rounding(8.0).inner_margin(Margin::same(11.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, color);
                ui.label(RichText::new(titulo).family(semibold()).size(13.0).color(TEXTO));
            });
            ui.label(RichText::new(detalle).size(12.0).color(SECUNDARIO));
            if iniciar {
                ui.add_space(4.0);
                if boton(ui, None, "Iniciar grabadora", Estilo::Secundario, true).clicked() {
                    bandeja::iniciar_en_fondo();
                    self.proxima_lectura = self.ahora + 0.5;
                }
            }
        });
        let alto = r.response.rect.height();
        if (alto - self.alto_estado).abs() > 0.5 {
            self.alto_estado = alto;
            ui.ctx().request_repaint();
        }
    }

    pub(super) fn avisar(&mut self, texto: impl Into<String>, error: bool) {
        self.aviso = Some((texto.into(), error, self.ahora + 4.0));
    }

    fn guardar(&mut self) {
        match self.cfg.guardar() {
            Ok(()) => {
                self.guardada = self.cfg.clone();
                bandeja::avisar(bandeja::WM_RECARGAR, 0);
                self.avisar("Cambios guardados", false);
                self.proxima_lectura = self.ahora + 0.5;
            }
            Err(e) => self.avisar(format!("No se pudieron guardar los cambios: {e}"), true),
        }
    }

    /// Mientras un atajo espera una tecla: Esc cancela, Retroceso o Supr lo dejan sin asignar.
    fn leer_tecla_de_atajo(&mut self, ctx: &egui::Context) {
        let Some(cual) = self.capturando else { return };
        let eventos = ctx.input(|i| i.events.clone());
        for ev in eventos {
            let egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } = ev else { continue };
            match key {
                egui::Key::Escape => self.capturando = None,
                egui::Key::Backspace | egui::Key::Delete => {
                    *self.cfg.atajo_mut(cual) = None;
                    self.capturando = None;
                }
                _ => {
                    let nuevo = Atajo { tecla: key.name().to_string(), ctrl: modifiers.ctrl, shift: modifiers.shift, alt: modifiers.alt };
                    if !nuevo.es_valido() {
                        self.avisar("Las letras y los números necesitan Ctrl o Alt: si no, dejarías de poder escribirlos.", true);
                        continue;
                    }
                    if nuevo.tecla_virtual().is_none() {
                        self.avisar("Esa tecla no se puede usar como atajo. Prueba con otra (por ejemplo, F1 a F12).", true);
                        continue;
                    }
                    // Si otro atajo usaba la misma combinación, queda libre.
                    for otro in CualAtajo::TODOS {
                        if otro != cual && self.cfg.atajo(otro) == Some(&nuevo) {
                            *self.cfg.atajo_mut(otro) = None;
                            self.avisar(format!("«{}» quedó sin asignar porque usaba {}.", otro.titulo(), nuevo.texto()), false);
                        }
                    }
                    *self.cfg.atajo_mut(cual) = Some(nuevo);
                    self.capturando = None;
                }
            }
            break;
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            logo(ui, 26.0);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("Kairo").family(semibold()).size(16.0).color(TEXTO));
                // Siempre a la vista: para saber de un vistazo qué versión es este exe.
                let color = if crate::version::con_cambios_sin_commit() { AMARILLO } else { TENUE };
                ui.label(RichText::new(crate::version::corta()).size(11.5).color(color));
            });
        });
        ui.add_space(18.0);
        for p in Pagina::TODAS {
            if item_menu(ui, p.icono(), p.nombre(), self.pagina == p).clicked() {
                self.pagina = p;
                self.capturando = None;
            }
            ui.add_space(2.0);
        }
        // Estado de la grabadora, pegado abajo (su alto se mide en el cuadro anterior).
        ui.add_space((ui.available_height() - self.alto_estado).max(0.0));
        self.tarjeta_estado(ui);
    }

    fn pie(&mut self, ui: &mut egui::Ui) {
        let hay_cambios = self.cfg != self.guardada;
        ui.horizontal_centered(|ui| {
            if let Some((texto, error, hasta)) = self.aviso.clone() {
                if self.ahora < hasta {
                    ui.label(RichText::new(texto).size(12.5).color(if error { ROJO } else { VERDE }));
                    ui.ctx().request_repaint_after(Duration::from_secs_f64((hasta - self.ahora).max(0.05)));
                } else {
                    self.aviso = None;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if boton(ui, None, "Guardar cambios", Estilo::Primario, hay_cambios).clicked() {
                    self.guardar();
                }
                ui.add_space(4.0);
                if boton(ui, None, "Descartar", Estilo::Texto, hay_cambios).clicked() {
                    self.cfg = self.guardada.clone();
                    self.capturando = None;
                }
            });
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ahora = ctx.input(|i| i.time);
        self.ajustar_zoom(ctx);
        self.leer_tecla_de_atajo(ctx);
        self.sincronizar_atajos();
        self.leer_estado(ctx);
        let equipo = self.equipo.lock().ok().and_then(|e| e.clone());

        TopBottomPanel::bottom("pie")
            .exact_height(58.0)
            .frame(Frame::none().fill(PANEL).inner_margin(Margin::symmetric(20.0, 0.0)))
            .show(ctx, |ui| self.pie(ui));

        SidePanel::left("menu")
            .exact_width(214.0)
            .resizable(false)
            .frame(Frame::none().fill(PANEL).inner_margin(Margin { left: 12.0, right: 12.0, top: 16.0, bottom: 14.0 }))
            .show(ctx, |ui| self.menu(ui));

        CentralPanel::default().frame(Frame::none().fill(FONDO).inner_margin(Margin { left: 30.0, right: 18.0, top: 24.0, bottom: 0.0 })).show(
            ctx,
            |ui| {
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    // Columna de hasta 960 puntos, centrada si sobra lugar (ventana grande).
                    let ancho = ui.available_width();
                    let contenido = ancho.min(960.0);
                    let margen = ((ancho - contenido) / 2.0).floor().max(0.0);
                    ui.horizontal_top(|ui| {
                        ui.add_space(margen);
                        ui.vertical(|ui| {
                            ui.set_width(contenido);
                            match self.pagina {
                                Pagina::Grabacion => self.pagina_grabacion(ui, equipo.as_ref()),
                                Pagina::Audio => self.pagina_audio(ui, equipo.as_ref()),
                                Pagina::Clips => self.pagina_clips(ui),
                                Pagina::Atajos => self.pagina_atajos(ui),
                                Pagina::Sistema => self.pagina_sistema(ui, equipo.as_ref()),
                                Pagina::AcercaDe => self.pagina_acerca(ui),
                            }
                            ui.add_space(12.0);
                        });
                    });
                });
            },
        );

        if let Some(c) = &mut self.captura {
            c.paso(ctx, equipo.is_some());
        }
    }

    /// Si se cierra la ventana mientras se elegía un atajo, la grabadora vuelve a tomar los suyos.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.atajos_suspendidos {
            bandeja::avisar(bandeja::WM_SUSPENDER_ATAJOS, 0);
        }
    }
}
