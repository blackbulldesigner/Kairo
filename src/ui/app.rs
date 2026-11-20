//! La ventana de Kairo: arriba las páginas como pestañas (y el estado y la actualización como píldoras), la
//! página elegida a todo el ancho, y abajo Descartar / Guardar, que sube solo cuando hay cambios.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, Align, CentralPanel, Frame, Layout, Margin, RichText, ScrollArea, TopBottomPanel};

use super::animar;
use super::captura::Pedido;
use super::tema::{self, *};
use super::widgets::*;
use crate::bandeja;
use crate::config::{Atajo, CualAtajo, Config};
use crate::equipo::Equipo;
use crate::estado::{self, Estado};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pagina {
    Biblioteca,
    Grabacion,
    Audio,
    Guardado,
    Atajos,
    Sistema,
    AcercaDe,
}

impl Pagina {
    pub const TODAS: [Pagina; 7] =
        [Pagina::Biblioteca, Pagina::Grabacion, Pagina::Audio, Pagina::Guardado, Pagina::Atajos, Pagina::Sistema, Pagina::AcercaDe];

    pub fn nombre(self) -> &'static str {
        match self {
            Pagina::Biblioteca => "Biblioteca",
            Pagina::Grabacion => "Grabación",
            Pagina::Audio => "Audio",
            Pagina::Guardado => "Guardado",
            Pagina::Atajos => "Atajos",
            Pagina::Sistema => "Sistema",
            Pagina::AcercaDe => "Acerca de",
        }
    }

    /// Para la línea de comandos: "grabacion", "audio", "clips", "atajos", "sistema", "acerca".
    pub fn desde(s: &str) -> Option<Pagina> {
        match s.to_lowercase().as_str() {
            "grabacion" | "grabación" => Some(Pagina::Grabacion),
            "biblioteca" => Some(Pagina::Biblioteca),
            "audio" => Some(Pagina::Audio),
            "guardado" | "clips" => Some(Pagina::Guardado),
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
    pub(super) ahora: f64,
    captura: Option<Pedido>,
    /// Lo que informa la grabadora (se relee cada segundo). None = la grabadora está cerrada.
    pub(super) estado: Option<Estado>,
    proxima_lectura: f64,
    /// Se le pidió a la grabadora que suelte los atajos (mientras se elige uno nuevo).
    atajos_suspendidos: bool,
    pub(super) autoinicio: bool,
    /// Página (y si hay un clip abierto) del cuadro anterior, y cuándo cambió: para la animación de entrada.
    vista_anterior: (Pagina, bool),
    pub(super) cambio_de_vista: f64,
    /// La bienvenida (la primera vez que se abre Kairo): en qué paso va, y cuándo se llegó a él (para animar).
    pub(super) bienvenida: Option<usize>,
    pub(super) bienvenida_desde: f64,
    /// Programas abiertos (para elegir el juego a mano) y cuándo volver a leerlos.
    pub(super) programas: Vec<crate::grabadora::juego::Programa>,
    pub(super) programas_hasta: f64,
    pub(super) biblioteca: super::biblioteca::Biblioteca,
    /// Último clip que avisó la grabadora (si cambia, la biblioteca se vuelve a leer enseguida).
    ultimo_clip_visto: Option<String>,
    pub(super) actualizador: super::actualizar::Actualizador,
    /// El clip abierto en la Biblioteca (reproductor, editor y compresor).
    pub(super) visor: Option<super::visor::Visor>,
    /// Volumen del reproductor (0 a 1) y si está en silencio (se recuerdan en reproductor.json).
    pub(super) volumen: f32,
    pub(super) silencio: bool,
    /// El video del visor ocupa toda la pantalla (sin barra de arriba ni de abajo).
    pub(super) pantalla_completa: bool,
    /// Guardado → Limpieza: (carpeta y límites con que se calculó, hasta cuándo vale, resumen).
    /// Para las pruebas: el mouse de mentira del recuadro de selección (`KAIRO_PRUEBA_MARCO`).
    prueba_marco: Option<super::captura::PruebaMarco>,
    pub(super) limpieza_resumen: Option<((std::path::PathBuf, crate::config::Limpieza), f64, super::paginas::ResumenLimpieza)>,
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

        // La primera vez (sin ajustes guardados), la bienvenida. En las capturas, `KAIRO_CAPTURA_BIENVENIDA=<paso>`.
        let bienvenida = if captura.is_some() {
            std::env::var("KAIRO_CAPTURA_BIENVENIDA").ok().and_then(|p| p.parse().ok())
        } else {
            (!crate::config::existe()).then_some(0)
        };
        let mut cfg = Config::cargar();
        if captura.as_ref().is_some_and(|c| c.modo_juego) {
            cfg.modo = crate::config::ModoCaptura::Juego;
        }
        if captura.as_ref().is_some_and(|c| c.avanzado) {
            cfg.avanzado.activo = true;
        }
        if captura.as_ref().is_some_and(|c| c.limpieza) {
            cfg.limpieza.activa = true;
        }
        // Para las capturas (por ejemplo, los diseños): `KAIRO_CAPTURA_ATAJO=F9` muestra ese atajo de guardar (no se guarda).
        if captura.is_some() {
            if let Ok(tecla) = std::env::var("KAIRO_CAPTURA_ATAJO") {
                cfg.atajo_guardar = Some(crate::config::Atajo::tecla_sola(&tecla));
            }
        }
        if captura.as_ref().is_some_and(|c| c.programas) {
            cfg.otros_sonidos = crate::config::OtrosSonidos::Elegidos;
            if let Some(p) = crate::grabadora::juego::programas_abiertos().into_iter().next() {
                cfg.programas_audio = vec![crate::config::ProgramaAudio { exe: p.exe, nombre: p.nombre, activa: true, volumen: 80 }];
            }
        }
        // `--pagina acerca` (por ejemplo, desde el aviso de una versión nueva).
        let pedida = std::env::args().skip_while(|a| a != "--pagina").nth(1).and_then(|p| Pagina::desde(&p));
        let pagina = captura.as_ref().map(|c| c.pagina).or(pedida).unwrap_or(Pagina::Biblioteca);
        // En las capturas de prueba no se va a internet.
        let actualizador = super::actualizar::Actualizador::new(&cc.egui_ctx, cfg.buscar_actualizaciones && captura.is_none());
        let visor = captura
            .as_ref()
            .and_then(|_| super::captura::clip_pedido())
            .map(|(clip, modo)| super::visor::Visor::abrir(&cc.egui_ctx, &clip, modo));
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
            vista_anterior: (pagina, false),
            cambio_de_vista: -1.0,
            bienvenida,
            bienvenida_desde: -1.0,
            programas: Vec::new(),
            programas_hasta: 0.0,
            biblioteca: super::biblioteca::Biblioteca::new(&cc.egui_ctx),
            ultimo_clip_visto: None,
            actualizador,
            visor,
            volumen: super::visor::leer_volumen().0,
            silencio: super::visor::leer_volumen().1,
            pantalla_completa: false,
            limpieza_resumen: None,
            prueba_marco: super::captura::PruebaMarco::desde_env(),
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

    /// Píldora del estado de la grabadora (arriba a la derecha). Con la grabadora cerrada, es el botón para
    /// iniciarla; al pasar el mouse, el detalle.
    fn pildora_estado(&mut self, ui: &mut egui::Ui, compacta: bool) {
        let atajo = self.guardada.atajo(CualAtajo::Guardar).map(|a| a.texto());
        let (color, texto, detalle) = match &self.estado {
            None => (TENUE, "Iniciar grabadora".to_string(), "Kairo no está grabando: clic para empezar.".to_string()),
            Some(e) if e.pausado => (AMARILLO, "En pausa".into(), "Reanúdala desde el ícono de la bandeja.".into()),
            Some(e) if e.grabando => {
                let seg = crate::config::texto_duracion(self.guardada.segundos);
                let ahorro = if e.ahorro { "\nAhorro con batería: 720p, 30 FPS, Ligera" } else { "" };
                let juego = e.juego.as_ref().map(|j| format!("\nJuego: {j}")).unwrap_or_default();
                let texto = match &atajo {
                    Some(a) => format!("Grabando · {a}"),
                    None => "Grabando".into(),
                };
                (VERDE, texto, format!("Guarda los últimos {seg}{juego}{ahorro}"))
            }
            Some(e) => match &e.error {
                Some(err) => (ROJO, "No puede grabar".into(), err.clone()),
                None => (TENUE, "Preparando…".into(), "Abriendo la captura.".into()),
            },
        };
        // Compacta (ventana angosta): solo el punto; el texto pasa a la ayuda.
        let detalle = if compacta { format!("{texto}\n{detalle}") } else { detalle };
        let g = ui.fonts(|f| f.layout_no_wrap(if compacta { String::new() } else { texto }, egui::FontId::proportional(12.5), TEXTO));
        let ancho = if compacta { 30.0 } else { g.size().x + 34.0 };
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ancho, 30.0), egui::Sense::click());
        let h = animar::hacia(ui.ctx(), resp.id.with("h"), resp.hovered());
        let fondo = mezclar(mezclar(FONDO, color, 0.10), mezclar(FONDO, color, 0.18), h);
        let borde = mezclar(mezclar(BORDE, color, 0.30), mezclar(BORDE, color, 0.55), h);
        let p = ui.painter();
        p.rect(rect, 15.0, fondo, egui::Stroke::new(1.0_f32, borde));
        let x_punto = if compacta { rect.center().x } else { rect.left() + 14.0 };
        p.circle_filled(egui::pos2(x_punto, rect.center().y), 4.0, color);
        p.galley(egui::pos2(rect.left() + 24.0, rect.center().y - g.size().y / 2.0), g, TEXTO);
        let resp = resp.on_hover_text(detalle);
        if self.estado.is_none() && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            bandeja::iniciar_en_fondo();
            self.proxima_lectura = self.ahora + 0.5;
        }
    }

    /// Píldora de la versión nueva (si hay): clic para actualizar, con el progreso adentro.
    fn pildora_actualizacion(&mut self, ui: &mut egui::Ui, compacta: bool) {
        use super::actualizar::Paso;
        let paso = self.actualizador.paso();
        let (texto, ayuda, progreso) = match &paso {
            Paso::Hay(d) => (format!("{}  Kairo {}", icono::DESCARGAR, d.version), "Actualizar ahora (tus ajustes y tus clips no se tocan)".to_string(), None),
            Paso::Bajando(_, f) => (format!("Bajando {:.0} %", f * 100.0), "Bajando la versión nueva".to_string(), Some(*f)),
            Paso::Instalando => ("Instalando…".to_string(), "Kairo se cierra y se vuelve a abrir".to_string(), Some(1.0)),
            Paso::Error(_, Some(_)) => ("No se pudo actualizar".to_string(), "Ver el detalle en Acerca de".to_string(), None),
            _ => return,
        };
        let error = matches!(paso, Paso::Error(..));
        let fuente = egui::FontId::proportional(12.5);
        // El ícono va con su fuente; el resto con la normal.
        let mut job = egui::text::LayoutJob::default();
        let ayuda = if compacta { format!("{}\n{ayuda}", texto.trim_start_matches(icono::DESCARGAR).trim()) } else { ayuda };
        if compacta {
            // Solo el ícono (y el texto en la ayuda).
            let ic = if error { icono::INFO } else { icono::DESCARGAR };
            job.append(ic, 0.0, egui::TextFormat { font_id: egui::FontId::new(13.0, iconos()), color: if error { AMARILLO } else { TEXTO_SEL }, ..Default::default() });
        } else if let Some(resto) = texto.strip_prefix(icono::DESCARGAR) {
            job.append(icono::DESCARGAR, 0.0, egui::TextFormat { font_id: egui::FontId::new(12.5, iconos()), color: TEXTO_SEL, ..Default::default() });
            job.append(resto, 0.0, egui::TextFormat { font_id: fuente.clone(), color: TEXTO_SEL, ..Default::default() });
        } else {
            job.append(&texto, 0.0, egui::TextFormat { font_id: fuente, color: if error { AMARILLO } else { TEXTO_SEL }, ..Default::default() });
        }
        let g = ui.fonts(|f| f.layout_job(job));
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(g.size().x + 24.0, 30.0), egui::Sense::click());
        let h = animar::hacia(ui.ctx(), resp.id.with("h"), resp.hovered());
        let p = ui.painter();
        p.rect(rect, 15.0, mezclar(SELECCION, ACENTO_APAGADO, h), egui::Stroke::new(1.0_f32, mezclar(ACENTO_APAGADO, ACENTO, h)));
        if let Some(f) = progreso {
            let mut lleno = rect.shrink(2.0);
            lleno.set_right(lleno.left() + lleno.width() * f);
            p.rect_filled(lleno, 13.0, ACENTO.gamma_multiply(0.45));
        }
        p.galley(egui::pos2(rect.left() + 12.0, rect.center().y - g.size().y / 2.0), g, TEXTO_SEL);
        if resp.on_hover_text(ayuda).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            match paso {
                Paso::Hay(_) => self.actualizador.actualizar(ui.ctx()),
                Paso::Error(..) => self.pagina = Pagina::AcercaDe,
                _ => {}
            }
        }
    }

    pub(super) fn avisar(&mut self, texto: impl Into<String>, error: bool) {
        self.avisar_por(texto, error, 4.0);
    }

    /// Un aviso que queda más tiempo (para los que hay que leer con calma).
    pub(super) fn avisar_por(&mut self, texto: impl Into<String>, error: bool, segundos: f64) {
        self.aviso = Some((texto.into(), error, self.ahora + segundos));
    }

    pub(super) fn guardar(&mut self) {
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

    /// Barra de arriba: logo y versión, las páginas como pestañas, y a la derecha el estado y la actualización.
    fn barra_superior(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            logo(ui, 24.0);
            ui.add_space(4.0);
            // "Kairo" y la versión en dos renglones, como un solo bloque: así queda centrado con el logo y las
            // pestañas (un `ui.vertical` arrancaba arriba de la barra y se veía corrido hacia arriba).
            // La versión siempre a la vista: para saber de un vistazo qué versión es este exe.
            let color = if crate::version::con_cambios_sin_commit() { AMARILLO } else { TENUE };
            let nombre = ui.fonts(|f| f.layout_no_wrap("Kairo".to_string(), egui::FontId::new(14.5, semibold()), TEXTO));
            let version = ui.fonts(|f| f.layout_no_wrap(crate::version::corta(), egui::FontId::proportional(10.5), color));
            let alto_nombre = nombre.size().y;
            let tam = egui::vec2(nombre.size().x.max(version.size().x), alto_nombre + version.size().y);
            let (zona, _) = ui.allocate_exact_size(tam, egui::Sense::hover());
            ui.painter().galley(zona.min, nombre, TEXTO);
            ui.painter().galley(zona.min + egui::vec2(0.0, alto_nombre), version, color);
            ui.add_space(18.0);
            let nombres: Vec<&str> = Pagina::TODAS.iter().map(|p| p.nombre()).collect();
            let actual = Pagina::TODAS.iter().position(|p| *p == self.pagina).unwrap_or(0);
            // ¿Entran las pestañas y las píldoras completas? Si no, las píldoras se achican (solo el punto o el ícono).
            let ancho_pestanas: f32 = nombres
                .iter()
                .map(|n| ui.fonts(|f| f.layout_no_wrap(n.to_string(), egui::FontId::proportional(13.5), TEXTO).size().x) + 24.0)
                .sum();
            let hay_version = self.actualizador.disponible().is_some() || matches!(self.actualizador.paso(), super::actualizar::Paso::Instalando);
            let pildoras = 150.0 + if hay_version { 130.0 } else { 0.0 };
            let compactas = ui.available_width() < ancho_pestanas + pildoras + 24.0;
            if let Some(i) = pestanas(ui, "paginas", actual, &nombres, 56.0) {
                let nueva = Pagina::TODAS[i];
                // Otro clic en Biblioteca (con un clip abierto) vuelve a la cuadrícula.
                if nueva == Pagina::Biblioteca && self.pagina == Pagina::Biblioteca {
                    self.visor = None;
                }
                self.pagina = nueva;
                self.capturando = None;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.pildora_estado(ui, compactas);
                ui.add_space(6.0);
                self.pildora_actualizacion(ui, compactas);
            });
        });
    }

    /// "Guardar cambios" y "Descartar": sube desde abajo cuando hay cambios sin guardar.
    fn barra_guardar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new(icono::INFO).family(iconos()).size(13.0).color(TEXTO_SEL));
            ui.label(RichText::new("Tienes cambios sin guardar").size(13.0).color(TEXTO));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if boton(ui, None, "Guardar cambios", Estilo::Primario, true).clicked() {
                    self.guardar();
                }
                ui.add_space(4.0);
                if boton(ui, None, "Descartar", Estilo::Texto, true).clicked() {
                    self.cfg = self.guardada.clone();
                    self.capturando = None;
                }
            });
        });
    }

    /// El último aviso ("Cambios guardados", errores…) flota abajo al centro: entra subiendo y se va solo.
    fn aviso_flotante(&mut self, ctx: &egui::Context, sobre: f32) {
        let vigente = self.aviso.as_ref().is_some_and(|(_, _, hasta)| self.ahora < *hasta);
        let t = animar::suave(ctx, egui::Id::new("aviso"), vigente as u8 as f32, 0.25);
        if let Some((_, _, hasta)) = &self.aviso {
            if vigente {
                ctx.request_repaint_after(Duration::from_secs_f64((hasta - self.ahora).max(0.05)));
            } else if t <= 0.001 {
                self.aviso = None;
            }
        }
        let Some((texto, error, _)) = self.aviso.clone() else { return };
        if t <= 0.001 {
            return;
        }
        let color = if error { ROJO } else { VERDE };
        egui::Area::new(egui::Id::new("aviso_area"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -(sobre + 18.0) + (1.0 - t) * 16.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                Frame::none()
                    .fill(PANEL)
                    .stroke(egui::Stroke::new(1.0_f32, mezclar(BORDE, color, 0.45)))
                    .rounding(10.0)
                    .inner_margin(Margin::symmetric(14.0, 9.0))
                    .shadow(egui::epaint::Shadow { offset: egui::vec2(0.0, 6.0), blur: 18.0, spread: 0.0, color: egui::Color32::from_black_alpha(110) })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.0, color);
                            ui.label(RichText::new(texto).size(12.5).color(TEXTO));
                        });
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
        self.actualizador.revisar_cierre(ctx);
        self.revisar_discord();
        let equipo = self.equipo.lock().ok().and_then(|e| e.clone());
        let nuevo_clip = self.estado.as_ref().and_then(|e| e.ultimo_clip.clone());
        if nuevo_clip.is_some() && nuevo_clip != self.ultimo_clip_visto {
            self.ultimo_clip_visto = nuevo_clip;
            self.biblioteca.escanear_ya();
        }
        // Fuera de la Biblioteca, el clip abierto se pausa.
        if self.pagina != Pagina::Biblioteca {
            if let Some(v) = &self.visor {
                v.pausar();
            }
        }
        if self.pagina == Pagina::Biblioteca {
            let carpeta = self.guardada.carpeta.clone();
            self.biblioteca.actualizar(&carpeta, self.ahora, ctx);
        }

        // Sin clip abierto no hay pantalla completa.
        if self.pantalla_completa && (self.visor.is_none() || self.pagina != Pagina::Biblioteca) {
            self.pantalla_completa = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        // La primera vez: la bienvenida, en lugar de todo lo demás.
        if self.bienvenida.is_some() {
            self.pantalla_bienvenida(ctx);
            self.aviso_flotante(ctx, 0.0);
            if let Some(c) = &mut self.captura {
                c.paso(ctx, equipo.is_some());
            }
            return;
        }

        let completa = self.pantalla_completa;
        if !completa {
        TopBottomPanel::top("barra")
            .exact_height(56.0)
            .frame(Frame::none().fill(PANEL).inner_margin(Margin::symmetric(18.0, 0.0)).stroke(egui::Stroke::new(1.0_f32, BORDE)))
            .show(ctx, |ui| self.barra_superior(ui));

        // La barra de guardar sube desde abajo cuando hay cambios (y baja al guardar o descartar).
        }
        let t_guardar = animar::suave(ctx, egui::Id::new("barra_guardar"), (self.cfg != self.guardada) as u8 as f32, 0.22);
        let alto_guardar = 58.0 * t_guardar;
        if alto_guardar > 0.5 && !completa {
            TopBottomPanel::bottom("pie")
                .exact_height(alto_guardar)
                .frame(Frame::none().fill(PANEL).inner_margin(Margin::symmetric(20.0, 0.0)).stroke(egui::Stroke::new(1.0_f32, BORDE)))
                .show(ctx, |ui| self.barra_guardar(ui));
        }
        self.aviso_flotante(ctx, alto_guardar);

        // Al cambiar de página (o abrir un clip), lo nuevo entra con un fundido y sube un poquito.
        let vista = (self.pagina, self.visor.is_some());
        if vista != self.vista_anterior {
            self.vista_anterior = vista;
            self.cambio_de_vista = self.ahora;
        }
        let entrada = (((self.ahora - self.cambio_de_vista) / 0.28) as f32).clamp(0.0, 1.0);
        let entrada = 1.0 - (1.0 - entrada).powi(3);
        if entrada < 1.0 {
            ctx.request_repaint();
        }

        let margen = if completa { Margin::same(0.0) } else { Margin { left: 28.0, right: 20.0, top: 0.0, bottom: 0.0 } };
        CentralPanel::default().frame(Frame::none().fill(if completa { egui::Color32::BLACK } else { FONDO }).inner_margin(margen)).show(
            ctx,
            |ui| {
                ui.set_opacity(entrada);
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if !completa {
                        ui.add_space(22.0 + (1.0 - entrada) * 14.0);
                    }
                    // Columna de hasta 960 puntos, centrada si sobra lugar (ventana grande).
                    let ancho = ui.available_width();
                    // La biblioteca aprovecha más ancho (más columnas de clips).
                    let maximo = match self.pagina {
                        Pagina::Biblioteca if self.visor.is_some() => 1200.0,
                        Pagina::Biblioteca => 1700.0,
                        _ => 900.0,
                    };
                    let contenido = if completa { ancho } else { ancho.min(maximo) };
                    let margen = ((ancho - contenido) / 2.0).floor().max(0.0);
                    ui.horizontal_top(|ui| {
                        ui.add_space(margen);
                        ui.vertical(|ui| {
                            ui.set_width(contenido);
                            match self.pagina {
                                Pagina::Biblioteca if self.visor.is_some() => self.pagina_visor(ui),
                                Pagina::Biblioteca => self.pagina_biblioteca(ui),
                                Pagina::Grabacion => self.pagina_grabacion(ui, equipo.as_ref()),
                                Pagina::Audio => self.pagina_audio(ui, equipo.as_ref()),
                                Pagina::Guardado => self.pagina_clips(ui),
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
            let listo = equipo.is_some()
                && match &self.visor {
                    Some(v) if self.pagina == Pagina::Biblioteca => v.listo(),
                    _ => self.pagina != Pagina::Biblioteca || self.biblioteca.pendientes() == 0,
                }
                && self.prueba_marco.as_ref().map_or(true, |p| p.terminada());
            c.paso(ctx, listo);
        }
    }

    /// Para las pruebas: el mouse de mentira entra como si fuera el de verdad.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(p) = &mut self.prueba_marco {
            p.paso(raw);
        }
    }

    /// Si se cierra la ventana mientras se elegía un atajo, la grabadora vuelve a tomar los suyos.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.atajos_suspendidos {
            bandeja::avisar(bandeja::WM_SUSPENDER_ATAJOS, 0);
        }
    }
}
