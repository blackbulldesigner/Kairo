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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Asa {
    Inicio,
    Fin,
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
    arrastrando: Option<Asa>,
    tarea: Option<Tarea>,
    /// El último clip que se guardó o comprimió desde acá.
    hecho: Option<PathBuf>,
    opcion: Opcion,
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
        let v = Self {
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
            arrastrando: None,
            tarea: None,
            hecho: None,
            opcion: Opcion::Liviano,
        };
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
            if let Some(r) = &v.reproductor {
                if espacio {
                    r.alternar();
                }
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

        // Barra de arriba: volver, nombre y modo.
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

        if let Some(r) = &v.reproductor {
            // El cuadro nuevo (si toca) a la textura.
            if let Some(c) = r.tomar_cuadro() {
                let img = egui::ColorImage::from_rgba_unmultiplied([c.ancho, c.alto], &c.rgba);
                match &mut v.textura {
                    Some(t) => t.set(img, TextureOptions::LINEAR),
                    None => v.textura = Some(ctx.load_texture("visor", img, TextureOptions::LINEAR)),
                }
            }
            if r.reproduciendo() {
                ctx.request_repaint();
            }
        }

        // El video, lo más grande que entre dejando lugar para los controles.
        let ancho = ui.available_width();
        let (w, h) = v.reproductor.as_ref().map(|r| (r.info.ancho as f32, r.info.alto as f32)).unwrap_or((16.0, 9.0));
        // En Editar y Comprimir, más chico: abajo van las opciones y el botón.
        let alto_max = (ctx.screen_rect().height() - if v.modo == Modo::Ver { 300.0 } else { 540.0 }).max(200.0);
        let alto = (ancho * h / w.max(1.0)).min(alto_max);
        let (caja, resp) = ui.allocate_exact_size(vec2(ancho, alto), Sense::click());
        let p = ui.painter();
        p.rect_filled(caja, 10.0, Color32::BLACK);
        let ancho_video = (alto * w / h.max(1.0)).min(ancho);
        let video = Rect::from_center_size(caja.center(), vec2(ancho_video, ancho_video * h / w.max(1.0)));
        if let Some(t) = &v.textura {
            p.image(t.id(), video, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        let sonando = v.reproductor.as_ref().is_some_and(|r| r.reproduciendo());
        if !sonando && v.reproductor.is_some() {
            // Botón grande de reproducir en el medio.
            p.circle_filled(caja.center(), 30.0, Color32::from_black_alpha(150));
            p.text(caja.center() + vec2(2.0, 0.0), Align2::CENTER_CENTER, icono::REPRODUCIR, FontId::new(24.0, iconos()), Color32::WHITE);
        }
        if let Some(e) = &v.error {
            if v.reproductor.is_none() {
                p.text(caja.center(), Align2::CENTER_CENTER, e, FontId::proportional(13.5), ROJO);
            }
        }
        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            if let Some(r) = &v.reproductor {
                r.alternar();
            }
        }
        ui.add_space(8.0);

        // Controles: reproducir/pausa, tiempo y la línea de tiempo.
        if let Some(r) = &v.reproductor {
            let dur = r.info.duracion;
            let pos = r.posicion();
            let mut buscar = None;
            ui.horizontal(|ui| {
                let (icono_rp, ayuda) = if sonando { (icono::PAUSA, "Pausa (Espacio)") } else { (icono::REPRODUCIR, "Reproducir (Espacio)") };
                if boton_icono(ui, icono_rp, ayuda).clicked() {
                    r.alternar();
                }
                ui.add_space(4.0);
                ui.label(RichText::new(format!("{} / {}", tiempo(pos), tiempo(dur))).size(12.5).color(SECUNDARIO).monospace());
                ui.add_space(8.0);
                let recorte = (v.modo == Modo::Editar).then_some((v.inicio, v.fin));
                let (b, asa) = linea_de_tiempo(ui, dur, pos, recorte, &mut v.arrastrando);
                buscar = b;
                if let Some((a, t)) = asa {
                    match a {
                        Asa::Inicio => v.inicio = t.clamp(0.0, (v.fin - 0.5).max(0.0)),
                        Asa::Fin => v.fin = t.clamp((v.inicio + 0.5).min(dur), dur),
                    }
                    buscar = Some(if a == Asa::Inicio { v.inicio } else { v.fin });
                }
            });
            if let Some(t) = buscar {
                r.buscar(t);
            }
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
            let titulo = format!("{} → {}  ·  dura {}", tiempo_fino(v.inicio), tiempo_fino(v.fin), tiempo_fino(v.fin - v.inicio));
            let desc = "Arrastra las marcas blancas de la barra, o lleva el video a donde quieras cortar y usa los botones. \
                        El video no se vuelve a comprimir (se guarda al instante, sin perder calidad), así que empieza en el \
                        cuadro clave anterior al inicio: como mucho 1 s antes.";
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
        return;
    }
    nota(ui, "Espacio: reproducir o pausar · ← →: 5 segundos · Esc: volver a la biblioteca. En «Editar» puedes recortarlo y cambiar el volumen de cada pista.");
}

/// La barra de tiempo. Clic o arrastre: buscar. En el editor, las dos marcas del recorte se arrastran.
/// Devuelve (a dónde buscar, qué marca se movió y a qué instante).
fn linea_de_tiempo(ui: &mut Ui, dur: f64, pos: f64, recorte: Option<(f64, f64)>, arrastrando: &mut Option<Asa>) -> (Option<f64>, Option<(Asa, f64)>) {
    let ancho = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ancho, 26.0), Sense::click_and_drag());
    let barra = Rect::from_center_size(rect.center(), vec2(rect.width() - 12.0, 6.0));
    let x_de = |t: f64| barra.left() + (t / dur.max(0.001)).clamp(0.0, 1.0) as f32 * barra.width();
    let t_de = |x: f32| (((x - barra.left()) / barra.width()).clamp(0.0, 1.0) as f64) * dur;
    let p = ui.painter();
    p.rect_filled(barra, 3.0, PISTA);
    let mut jugado = barra;
    jugado.set_right(x_de(pos));
    p.rect_filled(jugado, 3.0, ACENTO);
    if let Some((a, b)) = recorte {
        // Fuera del recorte, más oscuro; las marcas, blancas.
        let sombra = Color32::from_black_alpha(150);
        p.rect_filled(Rect::from_min_max(pos2(barra.left(), rect.top() + 4.0), pos2(x_de(a), rect.bottom() - 4.0)), 2.0, sombra);
        p.rect_filled(Rect::from_min_max(pos2(x_de(b), rect.top() + 4.0), pos2(barra.right(), rect.bottom() - 4.0)), 2.0, sombra);
        for x in [x_de(a), x_de(b)] {
            p.rect_filled(Rect::from_center_size(pos2(x, rect.center().y), vec2(6.0, 24.0)), 2.0, Color32::WHITE);
        }
        p.line_segment([pos2(x_de(a), rect.center().y), pos2(x_de(b), rect.center().y)], Stroke::new(2.0_f32, AMARILLO));
    }
    p.circle_filled(pos2(x_de(pos), rect.center().y), 7.0, Color32::WHITE);

    let mut buscar = None;
    let mut asa = None;
    if let Some(puntero) = resp.interact_pointer_pos() {
        if resp.drag_started() || resp.clicked() {
            *arrastrando = recorte.and_then(|(a, b)| {
                let (da, db) = ((puntero.x - x_de(a)).abs(), (puntero.x - x_de(b)).abs());
                if da.min(db) > 10.0 {
                    None
                } else if da <= db {
                    Some(Asa::Inicio)
                } else {
                    Some(Asa::Fin)
                }
            });
        }
        match *arrastrando {
            Some(a) => asa = Some((a, t_de(puntero.x))),
            None if resp.dragged() || resp.clicked() => buscar = Some(t_de(puntero.x)),
            None => {}
        }
    }
    if resp.drag_stopped() || resp.clicked() {
        *arrastrando = None;
    }
    if let Some(h) = resp.hover_pos() {
        resp.clone().on_hover_text_at_pointer(tiempo_fino(t_de(h.x)));
    }
    (buscar, asa)
}
