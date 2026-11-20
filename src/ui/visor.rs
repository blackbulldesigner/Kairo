//! El visor de la Biblioteca: el reproductor de Kairo, el editor (recortar y el volumen de cada pista) y el
//! compresor. Vive en la ventana: con la ventana cerrada no existe.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, pos2, vec2, Align2, Color32, CursorIcon, FontId, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, Ui};

use super::app::App;
use super::tema::*;
use super::widgets::*;
use crate::config::texto_bytes;
use crate::medios::comprimir::{self, Opcion};
use crate::medios::exportar;
use crate::medios::reproductor::Reproductor;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    Ver,
    Editar,
    Comprimir,
}

/// Qué se está arrastrando.
#[derive(Clone, Copy, PartialEq)]
enum Arrastre {
    /// La bolita de la barra de reproducción (o un lugar vacío de la pista del recorte).
    Cabezal,
    /// La manija izquierda del recorte.
    Inicio,
    /// La manija derecha.
    Fin,
    /// La parte elegida entera: se mueve sin cambiar de largo. `agarre`: dónde se agarró (segundos).
    Rango { agarre: f64, inicio: f64, fin: f64 },
}

/// Guardar o comprimir, corriendo en otro hilo.
struct Tarea {
    que: &'static str,
    progreso: Arc<Mutex<f32>>,
    cancelar: Arc<AtomicBool>,
    resultado: Arc<Mutex<Option<Result<PathBuf, String>>>>,
}

pub struct Visor {
    pub ruta: PathBuf,
    nombre: String,
    juego: Option<String>,
    tamano: u64,
    reproductor: Option<Reproductor>,
    error: Option<String>,
    textura: Option<TextureHandle>,
    pub modo: Modo,
    /// Nombre de cada pista de audio del archivo ("Mezcla", "Juego", "Discord"…).
    pistas: Vec<String>,
    volumenes: Vec<f32>,
    activas: Vec<bool>,
    inicio: f64,
    fin: f64,
    arrastre: Option<Arrastre>,
    /// Última vez que el mouse se movió sobre el video (los controles se esconden a los 2 s).
    ultimo_movimiento: f64,
    /// Destello del ícono al reproducir: (cuándo, si empezó a sonar).
    destello: Option<(f64, bool)>,
    /// Estaba sonando cuando se empezó a arrastrar: al soltar, sigue.
    seguir_al_soltar: bool,
    /// La última búsqueda hecha mientras se arrastra (para no repetir la misma).
    ultima_busqueda: f64,
    tarea: Option<Tarea>,
    /// El último clip que se guardó o comprimió desde acá.
    hecho: Option<PathBuf>,
    opcion: Opcion,
}

fn ruta_volumen() -> std::path::PathBuf {
    crate::config::datos_dir().join("reproductor.json")
}

/// El volumen del reproductor que quedó la última vez: (volumen, silencio).
pub(super) fn leer_volumen() -> (f32, bool) {
    let v: Option<serde_json::Value> = std::fs::read_to_string(ruta_volumen()).ok().and_then(|s| serde_json::from_str(&s).ok());
    let volumen = v.as_ref().and_then(|v| v["volumen"].as_f64()).unwrap_or(0.8) as f32;
    let silencio = v.as_ref().and_then(|v| v["silencio"].as_bool()).unwrap_or(false);
    (volumen.clamp(0.0, 1.0), silencio)
}

fn guardar_volumen(volumen: f32, silencio: bool) {
    let _ = std::fs::write(ruta_volumen(), format!("{{\"volumen\": {volumen:.2}, \"silencio\": {silencio}}}"));
}

/// "0:12"
fn tiempo(s: f64) -> String {
    let s = s.max(0.0).floor() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// "0:12,5"
fn tiempo_fino(s: f64) -> String {
    let d = (s.max(0.0) * 10.0).round() as u64;
    format!("{}:{:02},{}", d / 600, (d / 10) % 60, d % 10)
}

impl Visor {
    pub fn abrir(ctx: &egui::Context, ruta: &Path, modo: Modo) -> Self {
        let datos = crate::datos_clip::leer(ruta);
        let aviso = ctx.clone();
        let (reproductor, error) = match Reproductor::abrir(ruta, Vec::new(), move || aviso.request_repaint()) {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(format!("No se pudo abrir el clip: {e:#}"))),
        };
        let n = reproductor.as_ref().map(|r| r.info.pistas_audio).unwrap_or(0);
        let mut pistas: Vec<String> = datos.as_ref().map(|d| d.pistas.clone()).unwrap_or_default();
        pistas.resize_with(n, Default::default);
        for (i, p) in pistas.iter_mut().enumerate() {
            if p.is_empty() {
                *p = if i == 0 { "Mezcla".into() } else { format!("Pista {}", i + 1) };
            }
        }
        let duracion = reproductor.as_ref().map(|r| r.info.duracion).unwrap_or(0.0);
        let mut v = Self {
            ruta: ruta.to_path_buf(),
            nombre: ruta.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            juego: datos.and_then(|d| d.juego),
            tamano: std::fs::metadata(ruta).map(|m| m.len()).unwrap_or(0),
            reproductor,
            error,
            textura: None,
            modo,
            pistas,
            volumenes: vec![1.0; n],
            activas: vec![true; n],
            inicio: 0.0,
            fin: duracion,
            arrastre: None,
            ultimo_movimiento: 0.0,
            destello: None,
            seguir_al_soltar: false,
            ultima_busqueda: -1.0,
            tarea: None,
            hecho: None,
            opcion: Opcion::Liviano,
        };
        // Para las capturas de prueba: `KAIRO_CAPTURA_RECORTE=2,6` pone el recorte de 2 a 6 s.
        if let Some((a, b)) = std::env::var("KAIRO_CAPTURA_RECORTE").ok().filter(|_| cfg!(feature = "dev")).and_then(|s| {
            let (a, b) = s.split_once(',')?;
            Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?))
        }) {
            v.inicio = a.clamp(0.0, duracion);
            v.fin = b.clamp(v.inicio, duracion);
            if let Some(r) = &v.reproductor {
                r.buscar(v.inicio + 1.0);
            }
        }
        v.aplicar_volumenes();
        v
    }

    /// Para las capturas de prueba: ya se ve el primer cuadro.
    pub fn listo(&self) -> bool {
        self.textura.is_some() || self.reproductor.is_none()
    }

    pub fn pausar(&self) {
        if let Some(r) = &self.reproductor {
            if r.reproduciendo() {
                r.pausar();
            }
        }
    }

    fn duracion(&self) -> f64 {
        self.reproductor.as_ref().map(|r| r.info.duracion).unwrap_or(0.0)
    }

    fn posicion(&self) -> f64 {
        self.reproductor.as_ref().map(|r| r.posicion()).unwrap_or(0.0)
    }

    /// Reproducir o pausar. En el editor, si el video está fuera del recorte, arranca desde el inicio del recorte.
    fn alternar(&self) {
        let Some(r) = &self.reproductor else { return };
        if !r.reproduciendo() && self.modo == Modo::Editar {
            let p = r.posicion();
            if p < self.inicio - 0.05 || p >= self.fin - 0.05 {
                r.buscar(self.inicio);
            }
        }
        r.alternar();
    }

    /// Mientras se arrastra, el video queda en pausa (al soltar, sigue si estaba sonando).
    fn empezar_arrastre(&mut self, a: Arrastre) {
        if let Some(r) = &self.reproductor {
            self.seguir_al_soltar = r.reproduciendo();
            if self.seguir_al_soltar {
                r.pausar();
            }
        }
        self.arrastre = Some(a);
        self.ultima_busqueda = -1.0;
    }

    /// Busca mientras se arrastra, sin repetir la misma posición.
    fn buscar_arrastrando(&mut self, t: f64) {
        if (t - self.ultima_busqueda).abs() >= 0.02 {
            if let Some(r) = &self.reproductor {
                r.buscar(t);
            }
            self.ultima_busqueda = t;
        }
    }

    fn terminar_arrastre(&mut self) {
        // Después de mover el recorte, el video queda en su inicio: "reproducir" muestra cómo va a quedar.
        if matches!(self.arrastre, Some(Arrastre::Inicio | Arrastre::Rango { .. })) {
            if let Some(r) = &self.reproductor {
                r.buscar(self.inicio);
            }
        }
        self.arrastre = None;
        if std::mem::take(&mut self.seguir_al_soltar) {
            if let Some(r) = &self.reproductor {
                r.reproducir();
            }
        }
    }

    /// La pista del recorte: todo el clip de punta a punta, la parte elegida resaltada con una manija en cada
    /// punta (se arrastran; arrastrar el medio mueve la parte entera), y una línea donde está el video.
    /// Debajo, el inicio y el fin.
    fn barra_recorte(&mut self, ui: &mut Ui) {
        const MANIJA: f32 = 14.0;
        let (dur, pos) = (self.duracion().max(0.001), self.posicion());
        let ancho = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(ancho, 44.0), Sense::click_and_drag());
        // Los tiempos van entre las manijas, para que la de cada punta entre entera.
        let pista = Rect::from_min_max(pos2(rect.left() + MANIJA, rect.top()), pos2(rect.right() - MANIJA, rect.bottom()));
        let x_de = |t: f64| pista.left() + (t / dur).clamp(0.0, 1.0) as f32 * pista.width();
        let t_de = |x: f32| ((x - pista.left()) / pista.width()).clamp(0.0, 1.0) as f64 * dur;
        let (xa, xb) = (x_de(self.inicio), x_de(self.fin));
        let manija_a = Rect::from_min_max(pos2(xa - MANIJA, rect.top()), pos2(xa, rect.bottom()));
        let manija_b = Rect::from_min_max(pos2(xb, rect.top()), pos2(xb + MANIJA, rect.bottom()));
        let elegido = Rect::from_min_max(pos2(xa, rect.top()), pos2(xb, rect.bottom()));

        // Qué hay bajo el mouse (para el cursor y para saber qué se arrastra).
        let bajo = |p: egui::Pos2| -> Arrastre {
            if manija_a.expand(4.0).contains(p) {
                Arrastre::Inicio
            } else if manija_b.expand(4.0).contains(p) {
                Arrastre::Fin
            } else if elegido.contains(p) {
                Arrastre::Rango { agarre: t_de(p.x), inicio: self.inicio, fin: self.fin }
            } else {
                Arrastre::Cabezal
            }
        };
        if let Some(h) = resp.hover_pos() {
            let cursor = match self.arrastre.unwrap_or_else(|| bajo(h)) {
                Arrastre::Inicio | Arrastre::Fin => CursorIcon::ResizeHorizontal,
                Arrastre::Rango { .. } if self.arrastre.is_some() => CursorIcon::Grabbing,
                Arrastre::Rango { .. } => CursorIcon::Grab,
                Arrastre::Cabezal => CursorIcon::PointingHand,
            };
            ui.ctx().set_cursor_icon(cursor);
        }

        let p = ui.painter();
        // Fondo con marcas cada 5, 10 o 30 s.
        p.rect(pista, 6.0, FONDO, Stroke::new(1.0_f32, BORDE));
        let paso = if dur <= 30.0 { 5.0 } else if dur <= 120.0 { 10.0 } else { 30.0 };
        let mut t = paso;
        while t < dur {
            let x = x_de(t);
            p.line_segment([pos2(x, pista.bottom() - 7.0), pos2(x, pista.bottom() - 2.0)], Stroke::new(1.0_f32, APAGADO));
            t += paso;
        }
        // Lo elegido.
        p.rect_filled(elegido, 0.0, ACENTO.gamma_multiply(0.22));
        p.line_segment([elegido.left_top(), elegido.right_top()], Stroke::new(2.0_f32, ACENTO));
        p.line_segment([elegido.left_bottom(), elegido.right_bottom()], Stroke::new(2.0_f32, ACENTO));
        // Las manijas, con dos rayitas para agarrar.
        let activa = |a: Arrastre| self.arrastre == Some(a) || (self.arrastre.is_none() && resp.hover_pos().is_some_and(|h| bajo(h) == a));
        for (m, cual, redondeo) in [
            (manija_a, Arrastre::Inicio, egui::Rounding { nw: 6.0, sw: 6.0, ne: 0.0, se: 0.0 }),
            (manija_b, Arrastre::Fin, egui::Rounding { nw: 0.0, sw: 0.0, ne: 6.0, se: 6.0 }),
        ] {
            p.rect_filled(m, redondeo, if activa(cual) { ACENTO_HOVER } else { ACENTO });
            for dx in [-2.0, 2.0] {
                let x = m.center().x + dx;
                p.line_segment([pos2(x, m.center().y - 7.0), pos2(x, m.center().y + 7.0)], Stroke::new(1.5_f32, Color32::WHITE));
            }
        }
        // Dónde está el video.
        let xp = x_de(pos);
        p.line_segment([pos2(xp, rect.top() + 3.0), pos2(xp, rect.bottom() - 3.0)], Stroke::new(2.0_f32, Color32::WHITE));

        // Interacción.
        if let Some(puntero) = resp.interact_pointer_pos() {
            if resp.drag_started() {
                self.empezar_arrastre(bajo(puntero));
            }
            if resp.dragged() {
                let t = t_de(puntero.x);
                let dur = self.duracion();
                let ir = match self.arrastre {
                    Some(Arrastre::Inicio) => {
                        self.inicio = t.clamp(0.0, (self.fin - 0.5).max(0.0));
                        Some(self.inicio)
                    }
                    Some(Arrastre::Fin) => {
                        self.fin = t.clamp((self.inicio + 0.5).min(dur), dur);
                        Some(self.fin)
                    }
                    Some(Arrastre::Rango { agarre, inicio, fin }) => {
                        let largo = fin - inicio;
                        self.inicio = (inicio + t - agarre).clamp(0.0, (dur - largo).max(0.0));
                        self.fin = self.inicio + largo;
                        Some(self.inicio)
                    }
                    Some(Arrastre::Cabezal) => Some(t),
                    None => None,
                };
                if let Some(t) = ir {
                    self.buscar_arrastrando(t);
                }
            } else if resp.clicked() {
                if let Some(r) = &self.reproductor {
                    r.buscar(t_de(puntero.x));
                }
            }
        }
        if resp.drag_stopped() {
            self.terminar_arrastre();
        }

        // Debajo: el inicio y el fin (juntos en una sola etiqueta si quedan muy cerca).
        let (fila, _) = ui.allocate_exact_size(vec2(ancho, 18.0), Sense::hover());
        let fuente = FontId::proportional(12.0);
        let etiqueta = |x: f32, texto: String, color: Color32| {
            let g = ui.fonts(|f| f.layout_no_wrap(texto, fuente.clone(), color));
            let medio = g.size().x / 2.0;
            let x = x.clamp(fila.left() + medio, fila.right() - medio);
            ui.painter().galley(pos2(x - medio, fila.top() + 3.0), g, color);
        };
        if xb - xa < 120.0 {
            etiqueta((xa + xb) / 2.0, format!("{} – {}", tiempo_fino(self.inicio), tiempo_fino(self.fin)), TEXTO);
        } else {
            etiqueta(xa, tiempo_fino(self.inicio), TEXTO);
            etiqueta(xb, tiempo_fino(self.fin), TEXTO);
        }
    }

    /// ¿El clip tiene pistas separadas (la mezcla y cada fuente)?
    fn separadas(&self) -> bool {
        self.pistas.len() > 1
    }

    /// Lo que suena: en el editor, cada pista con su volumen (sin la mezcla original, si hay separadas);
    /// si no, la mezcla.
    fn volumenes_efectivos(&self) -> Vec<f32> {
        let n = self.pistas.len();
        (0..n)
            .map(|i| match self.modo {
                Modo::Editar if self.separadas() && i == 0 => 0.0,
                Modo::Editar => self.volumenes[i] * self.activas[i] as u8 as f32,
                _ => (i == 0) as u8 as f32,
            })
            .collect()
    }

    fn aplicar_volumenes(&self) {
        if let Some(r) = &self.reproductor {
            r.poner_volumenes(self.volumenes_efectivos());
        }
    }

    fn ocupado(&self) -> bool {
        self.tarea.is_some()
    }

    fn empezar_tarea(&mut self, que: &'static str, trabajo: impl FnOnce(&dyn Fn(f32), &AtomicBool) -> Result<PathBuf, String> + Send + 'static, ctx: &egui::Context) {
        let t = Tarea {
            que,
            progreso: Arc::new(Mutex::new(0.0)),
            cancelar: Arc::new(AtomicBool::new(false)),
            resultado: Arc::new(Mutex::new(None)),
        };
        let (progreso, cancelar, resultado, ctx) = (t.progreso.clone(), t.cancelar.clone(), t.resultado.clone(), ctx.clone());
        std::thread::spawn(move || {
            let avance = |f: f32| {
                if let Ok(mut p) = progreso.lock() {
                    *p = f;
                }
                ctx.request_repaint();
            };
            let r = trabajo(&avance, &cancelar);
            if let Ok(mut x) = resultado.lock() {
                *x = Some(r);
            }
            ctx.request_repaint();
        });
        self.tarea = Some(t);
        self.hecho = None;
    }

    fn guardar_editado(&mut self, ctx: &egui::Context) {
        let pedido = exportar::Pedido {
            origen: self.ruta.clone(),
            destino: crate::medios::ruta_libre(&self.ruta, "editado"),
            inicio: self.inicio,
            fin: self.fin,
            volumenes: (0..self.pistas.len()).map(|i| self.volumenes[i] * self.activas[i] as u8 as f32).collect(),
            nombres: self.pistas.clone(),
        };
        self.empezar_tarea(
            "Guardando el clip editado",
            move |avance, cancelar| exportar::exportar(&pedido, avance, cancelar).map(|_| pedido.destino.clone()).map_err(|e| format!("{e:#}")),
            ctx,
        );
    }

    fn comprimir(&mut self, ctx: &egui::Context) {
        let Some(info) = self.reproductor.as_ref().map(|r| r.info.clone()) else { return };
        let (alto, bitrate) = match comprimir::plan(self.opcion, &info) {
            Ok(p) => p,
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                return;
            }
        };
        let pedido = comprimir::Pedido { origen: self.ruta.clone(), destino: crate::medios::ruta_libre(&self.ruta, "comprimido"), alto, bitrate };
        self.empezar_tarea(
            "Comprimiendo",
            move |avance, cancelar| comprimir::comprimir(&pedido, avance, cancelar).map(|_| pedido.destino.clone()).map_err(|e| format!("{e:#}")),
            ctx,
        );
    }
}

impl App {
    /// Volumen del reproductor (0 a 1): se aplica en el próximo cuadro y se recuerda.
    fn cambiar_volumen(&mut self, v: f32) {
        self.volumen = v.clamp(0.0, 1.0);
        guardar_volumen(self.volumen, self.silencio);
    }

    fn alternar_silencio(&mut self) {
        self.silencio = !self.silencio;
        if !self.silencio && self.volumen == 0.0 {
            self.volumen = 0.5;
        }
        guardar_volumen(self.volumen, self.silencio);
    }

    pub(super) fn alternar_pantalla_completa(&mut self, ctx: &egui::Context) {
        self.pantalla_completa = !self.pantalla_completa;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.pantalla_completa));
    }

    pub(super) fn abrir_visor(&mut self, ctx: &egui::Context, ruta: &Path, modo: Modo) {
        self.visor = Some(Visor::abrir(ctx, ruta, modo));
    }

    /// La Biblioteca con un clip abierto.
    pub(super) fn pagina_visor(&mut self, ui: &mut Ui) {
        let Some(mut v) = self.visor.take() else { return };
        let ctx = ui.ctx().clone();
        let mut cerrar = false;

        // Teclado: Espacio, ← → y Esc (si no se está escribiendo en otro lado). Se toman antes de dibujar los
        // botones: si no, Espacio también "apretaría" el botón que tenga el foco.
        if !ctx.wants_keyboard_input() {
            use egui::{Key, Modifiers};
            let (espacio, izq, der, esc) = ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::Space),
                    i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                    i.consume_key(Modifiers::NONE, Key::ArrowRight),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                )
            });
            let (arriba, abajo, m, f) = ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::M),
                    i.consume_key(Modifiers::NONE, Key::F),
                )
            });
            if arriba || abajo {
                self.silencio = false;
                self.cambiar_volumen(self.volumen + if arriba { 0.1 } else { -0.1 });
            }
            if m {
                self.alternar_silencio();
            }
            if f || (esc && self.pantalla_completa) {
                self.alternar_pantalla_completa(&ctx);
            }
            let esc = esc && !self.pantalla_completa && !f;
            if espacio {
                v.alternar();
            }
            if let Some(r) = &v.reproductor {
                if izq {
                    r.buscar(r.posicion() - 5.0);
                }
                if der {
                    r.buscar(r.posicion() + 5.0);
                }
            }
            if esc && !v.ocupado() {
                cerrar = true;
            }
        }

        // ¿Terminó de guardar o comprimir?
        if let Some(t) = &v.tarea {
            let r = t.resultado.lock().ok().and_then(|mut x| x.take());
            if let Some(r) = r {
                let que = t.que;
                v.tarea = None;
                match r {
                    Ok(ruta) => {
                        let nombre = ruta.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                        self.avisar(format!("Listo: «{nombre}» está en tu biblioteca."), false);
                        self.biblioteca.escanear_ya();
                        v.hecho = Some(ruta);
                    }
                    Err(e) if e == "cancelado" => self.avisar(format!("{que}: cancelado."), false),
                    Err(e) => v.error = Some(e),
                }
            }
        }

        // Barra de arriba: volver, nombre y modo (en pantalla completa, solo el video).
        if !self.pantalla_completa {
        ui.horizontal(|ui| {
            if boton(ui, Some(icono::ATRAS), "Biblioteca", Estilo::Texto, true).clicked() {
                cerrar = true;
            }
            ui.add_space(6.0);
            ui.label(RichText::new(&v.nombre).family(semibold()).size(19.0).color(TEXTO));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let antes = v.modo;
                ui.add_enabled_ui(!v.ocupado(), |ui| {
                    segmentos(
                        ui,
                        "modo_visor",
                        &mut v.modo,
                        &[(Modo::Ver, Some(icono::REPRODUCIR), "Ver"), (Modo::Editar, Some(icono::TIJERA), "Editar"), (Modo::Comprimir, None, "Comprimir")],
                        None,
                        32.0,
                    );
                });
                if v.modo != antes {
                    v.aplicar_volumenes();
                    v.error = None;
                }
            });
        });
        let mut meta = Vec::new();
        if let Some(j) = &v.juego {
            meta.push(j.clone());
        }
        if let Some(r) = &v.reproductor {
            meta.push(format!("{}×{} · {:.0} fps", r.info.ancho, r.info.alto, r.info.fps));
            meta.push(tiempo(r.info.duracion));
        }
        meta.push(texto_bytes(v.tamano));
        ui.label(RichText::new(meta.join(" · ")).size(12.5).color(SECUNDARIO));
        ui.add_space(10.0);
        }

        if let Some(r) = &v.reproductor {
            // El cuadro nuevo (si toca) a la textura.
            if let Some(c) = r.tomar_cuadro() {
                let img = c.imagen;
                match &mut v.textura {
                    Some(t) => t.set(img, TextureOptions::LINEAR),
                    None => v.textura = Some(ctx.load_texture("visor", img, TextureOptions::LINEAR)),
                }
            }
            if r.reproduciendo() {
                ctx.request_repaint();
            }
        }

        // El video, lo más grande que entre (en pantalla completa, todo). Los controles van encima, abajo.
        let completa = self.pantalla_completa;
        let ancho = ui.available_width();
        let (w, h) = v.reproductor.as_ref().map(|r| (r.info.ancho as f32, r.info.alto as f32)).unwrap_or((16.0, 9.0));
        let alto_max = if completa {
            ui.available_height()
        } else {
            // En Editar y Comprimir, más chico: abajo van las opciones y el botón.
            (ctx.screen_rect().height() - if v.modo == Modo::Ver { 230.0 } else { 520.0 }).max(200.0)
        };
        let alto = if completa { alto_max } else { (ancho * h / w.max(1.0)).min(alto_max) };
        let (caja, resp) = ui.allocate_exact_size(vec2(ancho, alto), Sense::click());
        let p = ui.painter().clone();
        p.rect_filled(caja, if completa { 0.0 } else { 12.0 }, Color32::BLACK);
        let escala = (caja.width() / w.max(1.0)).min(caja.height() / h.max(1.0));
        let video = Rect::from_center_size(caja.center(), vec2(w * escala, h * escala));
        if let Some(t) = &v.textura {
            p.image(t.id(), video, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else if v.reproductor.is_some() {
            // Cargando: un arco que gira.
            let t = ui.input(|i| i.time) as f32 * 5.0;
            let puntos: Vec<egui::Pos2> = (0..=24).map(|k| {
                let a = t + k as f32 / 24.0 * 4.2;
                caja.center() + vec2(a.cos(), a.sin()) * 18.0
            }).collect();
            p.add(egui::Shape::line(puntos, Stroke::new(3.0_f32, ACENTO)));
            ctx.request_repaint();
        }
        let sonando = v.reproductor.as_ref().is_some_and(|r| r.reproduciendo());

        // ¿Se ven los controles? Siempre en pausa; sonando, mientras el mouse se mueve encima (y 2 s más).
        let ahora = ui.input(|i| i.time);
        let encima = ui.input(|i| i.pointer.hover_pos()).is_some_and(|q| caja.contains(q));
        if encima && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
            v.ultimo_movimiento = ahora;
        }
        let mostrar = !sonando || v.arrastre.is_some() || (encima && ahora - v.ultimo_movimiento < 2.0);
        if sonando && encima && ahora - v.ultimo_movimiento < 2.0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        let vis = super::animar::suave(&ctx, egui::Id::new("visor_controles"), mostrar as u8 as f32, 0.2);
        if !mostrar && encima && v.reproductor.is_some() {
            ctx.set_cursor_icon(CursorIcon::None);
        }

        // En pausa, el botón grande de reproducir; al apretar, un destello del ícono que crece y se va.
        let pausa = 1.0 - super::animar::hacia(&ctx, egui::Id::new("visor_sonando"), sonando);
        if pausa > 0.0 && v.reproductor.is_some() && v.textura.is_some() {
            p.circle_filled(caja.center(), 32.0, Color32::from_black_alpha((150.0 * pausa) as u8));
            p.text(caja.center() + vec2(2.0, 0.0), Align2::CENTER_CENTER, icono::REPRODUCIR, FontId::new(26.0, iconos()), Color32::WHITE.gamma_multiply(pausa));
        }
        if let Some((desde, a_sonar)) = v.destello {
            let t = ((ahora - desde) / 0.45) as f32;
            if t >= 1.0 {
                v.destello = None;
            } else if a_sonar {
                let r = 28.0 + 18.0 * t;
                p.circle_filled(caja.center(), r, Color32::from_black_alpha((140.0 * (1.0 - t)) as u8));
                p.text(caja.center() + vec2(2.0, 0.0), Align2::CENTER_CENTER, icono::REPRODUCIR, FontId::new(22.0 + 8.0 * t, iconos()), Color32::WHITE.gamma_multiply(1.0 - t));
                ctx.request_repaint();
            }
        }
        if let Some(e) = &v.error {
            if v.reproductor.is_none() {
                // En varias líneas, centrado (el mensaje de HEVC es largo).
                let g = p.layout(e.clone(), FontId::proportional(13.5), ROJO, (caja.width() - 60.0).max(100.0));
                let pos = caja.center() - g.size() / 2.0 - vec2(0.0, 18.0);
                p.galley(pos, g, ROJO);
            }
        }

        // Clic: reproducir o pausar. Doble clic: pantalla completa. Rueda: volumen.
        let resp = resp.on_hover_cursor(if mostrar { CursorIcon::PointingHand } else { CursorIcon::None });
        if resp.double_clicked() {
            self.alternar_pantalla_completa(&ctx);
        } else if resp.clicked() {
            v.alternar();
            v.destello = Some((ahora, !sonando));
        }
        if encima {
            let rueda = ui.input(|i| i.raw_scroll_delta.y);
            if rueda != 0.0 {
                self.cambiar_volumen(self.volumen + rueda.signum() * 0.05);
            }
        }
        if let Some(r) = &v.reproductor {
            r.poner_volumen_general(if self.silencio { 0.0 } else { self.volumen });
        }

        // Los controles: la barra de tiempo arriba y, abajo, reproducir, tiempo, volumen y pantalla completa.
        if v.reproductor.is_some() && vis > 0.01 {
            let (dur, pos) = (v.duracion(), v.posicion());
            // En el editor, el video se para al llegar al fin del recorte.
            if v.modo == Modo::Editar && sonando && v.arrastre.is_none() && pos >= v.fin {
                if let Some(r) = &v.reproductor {
                    r.pausar();
                    r.buscar(v.fin);
                }
            }
            // Fondo que se oscurece hacia abajo (franjas de opacidad creciente).
            let banda = Rect::from_min_max(pos2(caja.left(), caja.bottom() - 84.0), caja.max);
            for k in 0..8 {
                let f = Rect::from_min_max(pos2(banda.left(), banda.top() + banda.height() * k as f32 / 8.0), pos2(banda.right(), banda.top() + banda.height() * (k + 1) as f32 / 8.0));
                p.rect_filled(f, 0.0, Color32::from_black_alpha(((20.0 + 22.0 * k as f32) * vis) as u8));
            }
            let zona = banda.shrink2(vec2(14.0, 8.0));
            let mut hijo = ui.new_child(egui::UiBuilder::new().max_rect(zona).layout(egui::Layout::top_down(egui::Align::Min)));
            hijo.set_opacity(vis);
            let (mut alternar, mut silenciar, mut completa_clic, mut vol) = (false, false, false, self.volumen);
            let barra = Some(barra_reproduccion(&mut hijo, dur, pos, v.arrastre == Some(Arrastre::Cabezal)));
            hijo.horizontal(|ui| {
                let (icono_rp, ayuda) = if sonando { (icono::PAUSA, "Pausa (Espacio)") } else { (icono::REPRODUCIR, "Reproducir (Espacio)") };
                alternar = boton_icono(ui, icono_rp, ayuda).clicked();
                let (icono_vol, ayuda_vol) = if self.silencio || self.volumen == 0.0 { (icono::SILENCIO, "Activar sonido (M)") } else { (icono::VOLUMEN, "Silenciar (M)") };
                silenciar = boton_icono(ui, icono_vol, ayuda_vol).clicked();
                let mut x = if self.silencio { 0.0 } else { self.volumen * 100.0 };
                if deslizador(ui, &mut x, 0.0..=100.0, 1.0, 90.0, true) {
                    vol = x / 100.0;
                }
                ui.add_space(8.0);
                ui.label(RichText::new(format!("{} / {}", tiempo(pos), tiempo(dur))).size(12.5).color(Color32::from_gray(225)).monospace());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ayuda = if completa { "Salir de pantalla completa (F o Esc)" } else { "Pantalla completa (F o doble clic)" };
                    completa_clic = boton_icono(ui, if completa { icono::ACHICAR } else { icono::AGRANDAR }, ayuda).clicked();
                });
            });
            if alternar {
                v.alternar();
                v.destello = Some((ahora, !sonando));
            }
            if silenciar {
                self.alternar_silencio();
            }
            if vol != self.volumen {
                self.silencio = false;
                self.cambiar_volumen(vol);
            }
            if completa_clic {
                self.alternar_pantalla_completa(&ctx);
            }
            if let Some((resp, t)) = barra {
                if resp.drag_started() {
                    v.empezar_arrastre(Arrastre::Cabezal);
                }
                if let Some(t) = t.filter(|_| resp.dragged() || resp.clicked()) {
                    v.buscar_arrastrando(t);
                }
                if resp.drag_stopped() {
                    v.terminar_arrastre();
                }
            }
        }
        if completa {
            if cerrar {
                self.alternar_pantalla_completa(&ctx);
            }
            self.visor = Some(v);
            return;
        }
        ui.add_space(14.0);

        match v.modo {
            Modo::Ver => panel_ver(ui, &v),
            Modo::Editar => self.panel_editar(ui, &mut v),
            Modo::Comprimir => self.panel_comprimir(ui, &mut v),
        }
        if let Some(e) = v.error.clone().filter(|_| v.reproductor.is_some()) {
            ui.add_space(6.0);
            ui.label(RichText::new(e).size(12.5).color(ROJO));
        }
        let mut abrir_hecho = None;
        if let Some(h) = &v.hecho {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let nombre = h.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(RichText::new(format!("Guardado como «{nombre}».")).size(12.5).color(VERDE));
                if boton(ui, Some(icono::REPRODUCIR), "Ver el clip nuevo", Estilo::Secundario, true).clicked() {
                    abrir_hecho = Some(h.clone());
                }
                if boton(ui, Some(icono::CARPETA_ABIERTA), "Mostrar en la carpeta", Estilo::Texto, true).clicked() {
                    super::pagina_biblioteca::mostrar_en_carpeta(h);
                }
            });
        }

        if cerrar {
            if self.pantalla_completa {
                self.alternar_pantalla_completa(&ctx);
            }
            if let Some(t) = &v.tarea {
                t.cancelar.store(true, Ordering::SeqCst);
            }
            return; // el visor se suelta: se cierran el reproductor y sus hilos
        }
        if let Some(r) = abrir_hecho {
            self.abrir_visor(&ctx, &r, Modo::Ver);
            return;
        }
        self.visor = Some(v);
    }

    fn panel_editar(&mut self, ui: &mut Ui, v: &mut Visor) {
        let dur = v.duracion();
        let pos = v.reproductor.as_ref().map(|r| r.posicion()).unwrap_or(0.0);
        let ocupado = v.ocupado();
        tarjeta(ui, "Recorte", |ui| {
            ui.add_space(10.0);
            ui.add_enabled_ui(!ocupado, |ui| v.barra_recorte(ui));
            ui.add_space(4.0);
            separador(ui);
            let titulo = format!("El clip nuevo dura {}", tiempo_fino(v.fin - v.inicio));
            let desc = "Arrastra las manijas azules (o la parte elegida entera), o lleva el video a donde quieras cortar y usa \
                        los botones. El video no se vuelve a comprimir (se guarda al instante, sin perder calidad), así que \
                        empieza en el cuadro clave anterior al inicio: como mucho 1 s antes.";
            fila(ui, &titulo, Some(desc), |ui| {
                if boton(ui, None, "Todo", Estilo::Texto, !ocupado).clicked() {
                    v.inicio = 0.0;
                    v.fin = dur;
                }
                if boton(ui, None, "Fin aquí", Estilo::Secundario, !ocupado && pos > v.inicio + 0.5).clicked() {
                    v.fin = pos;
                }
                if boton(ui, None, "Inicio aquí", Estilo::Secundario, !ocupado && pos < v.fin - 0.5).clicked() {
                    v.inicio = pos;
                }
            });
        });

        let mut cambio = false;
        tarjeta(ui, "Pistas de audio", |ui| {
            if v.pistas.is_empty() {
                fila_info(ui, "Sin audio", "Este clip no tiene sonido.");
                return;
            }
            let desde = if v.separadas() { 1 } else { 0 };
            for i in desde..v.pistas.len() {
                if i > desde {
                    separador(ui);
                }
                let desc = if v.separadas() { "Se escucha así mientras editas." } else { "Activa «Pistas separadas» en Audio para editar cada fuente por separado." };
                let nombre = if !v.separadas() { "Audio".to_string() } else { v.pistas[i].clone() };
                fila(ui, &nombre, Some(desc), |ui| {
                    cambio |= interruptor(ui, &mut v.activas[i], !ocupado);
                    ui.add_space(12.0);
                    let activa = v.activas[i];
                    valor(ui, &format!("{:.0} %", v.volumenes[i] * 100.0), 50.0, activa);
                    let mut x = v.volumenes[i] * 100.0;
                    if deslizador(ui, &mut x, 0.0..=150.0, 5.0, 150.0, activa && !ocupado) {
                        v.volumenes[i] = x / 100.0;
                        cambio = true;
                    }
                });
            }
        });
        if cambio {
            v.aplicar_volumenes();
        }

        self.fila_tarea(ui, v, |app, ui, v| {
            let _ = app;
            if boton(ui, Some(icono::GUARDAR), "Guardar como clip nuevo", Estilo::Primario, true).clicked() {
                v.pausar();
                v.guardar_editado(ui.ctx());
            }
            ui.label(RichText::new("El original queda como está.").size(12.5).color(TENUE));
        });
    }

    fn panel_comprimir(&mut self, ui: &mut Ui, v: &mut Visor) {
        let Some(info) = v.reproductor.as_ref().map(|r| r.info.clone()) else { return };
        let ocupado = v.ocupado();
        tarjeta(ui, "Comprimir para compartir", |ui| {
            for (k, o) in Opcion::TODAS.into_iter().enumerate() {
                if k > 0 {
                    separador(ui);
                }
                let estimado = comprimir::plan(o, &info).map(|(alto, b)| {
                    let (wf, hf) = (info.ancho as f64, info.alto as f64);
                    let ancho = (wf * alto as f64 / hf.max(1.0) / 2.0).round() as u32 * 2;
                    format!("{}×{alto} · unos {}", ancho.min(info.ancho), texto_bytes(comprimir::tamano_estimado(&info, b)))
                });
                let desc = match &estimado {
                    Ok(e) if comprimir::ya_es_liviano(o, &info) => format!("{} {e} (tu clip ya es liviano: baja poco).", o.descripcion()),
                    Ok(e) => format!("{} {e}.", o.descripcion()),
                    Err(e) => format!("{e:#}."),
                };
                let mut elegida = v.opcion == o;
                fila(ui, o.nombre(), Some(&desc), |ui| {
                    if interruptor(ui, &mut elegida, !ocupado && estimado.is_ok()) && elegida {
                        v.opcion = o;
                    }
                });
            }
        });
        let juego = self.estado.as_ref().and_then(|e| e.juego.clone());
        if let Some(j) = juego {
            nota(ui, &format!("{j} está abierto: comprimir usa la tarjeta de video y puede bajarle algunos FPS mientras dura."));
        } else {
            nota(ui, "Usa la tarjeta de video con prioridad baja, y solo mientras comprime. El clip comprimido lleva solo la mezcla de audio.");
        }
        self.fila_tarea(ui, v, |_, ui, v| {
            if boton(ui, Some(icono::GUARDAR), "Comprimir", Estilo::Primario, true).clicked() {
                v.pausar();
                v.error = None;
                v.comprimir(ui.ctx());
            }
            ui.label(RichText::new("Se guarda como clip nuevo; el original queda como está.").size(12.5).color(TENUE));
        });
    }

    /// El botón de la tarea o, mientras corre, su progreso con "Cancelar".
    fn fila_tarea(&mut self, ui: &mut Ui, v: &mut Visor, boton_inicio: impl FnOnce(&mut App, &mut Ui, &mut Visor)) {
        ui.add_space(8.0);
        match &v.tarea {
            Some(t) => {
                let f = t.progreso.lock().map(|p| *p).unwrap_or(0.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}… {:.0} %", t.que, f * 100.0)).size(13.0).color(TEXTO));
                    ui.add_space(8.0);
                    ui.add(egui::ProgressBar::new(f).desired_width(260.0).desired_height(6.0).fill(ACENTO));
                    ui.add_space(8.0);
                    if boton(ui, None, "Cancelar", Estilo::Texto, true).clicked() {
                        t.cancelar.store(true, Ordering::SeqCst);
                    }
                });
            }
            None => {
                ui.horizontal(|ui| boton_inicio(self, ui, v));
            }
        }
    }
}

fn panel_ver(ui: &mut Ui, v: &Visor) {
    if v.reproductor.is_none() {
        // Si Kairo no lo puede mostrar (por ejemplo, HEVC sin la extensión), con el programa predeterminado.
        if boton(ui, Some(icono::ENLACE), "Abrir con otro programa", Estilo::Secundario, true).clicked() {
            crate::abrir::archivo(&v.ruta);
        }
        return;
    }
    nota(ui, "Espacio: reproducir o pausar · ← →: 5 s · ↑ ↓ o la rueda: volumen · M: silencio · F o doble clic: pantalla completa · Esc: volver. En «Editar» puedes recortarlo y cambiar el volumen de cada pista.");
}

/// La barra de reproducción: clic o arrastre para ir a otro momento (mientras se arrastra, el video queda en
/// pausa). Devuelve la respuesta y el instante bajo el puntero, si se está usando.
fn barra_reproduccion(ui: &mut Ui, dur: f64, pos: f64, arrastrando: bool) -> (egui::Response, Option<f64>) {
    let ancho = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ancho, 24.0), Sense::click_and_drag());
    let activa = arrastrando || resp.hovered();
    let barra = Rect::from_center_size(rect.center(), vec2(rect.width() - 16.0, if activa { 6.0 } else { 4.0 }));
    let x_de = |t: f64| barra.left() + (t / dur.max(0.001)).clamp(0.0, 1.0) as f32 * barra.width();
    let t_de = |x: f32| (((x - barra.left()) / barra.width()).clamp(0.0, 1.0) as f64) * dur;
    let p = ui.painter();
    p.rect_filled(barra, 3.0, PISTA);
    let mut jugado = barra;
    jugado.set_right(x_de(pos));
    p.rect_filled(jugado, 3.0, ACENTO);
    p.circle_filled(pos2(x_de(pos), rect.center().y), if activa { 8.0 } else { 6.0 }, Color32::WHITE);
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if let Some(h) = resp.hover_pos() {
        resp.clone().on_hover_text_at_pointer(tiempo_fino(t_de(h.x)));
    }
    let t = resp.interact_pointer_pos().map(|p| t_de(p.x));
    (resp, t)
}
