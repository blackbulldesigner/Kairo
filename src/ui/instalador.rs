//! La ventana del instalador (KairoSetup.exe): opciones, botón "Instalar" y el progreso. La instalación
//! corre en otro hilo para que la ventana no se trabe.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Frame, Margin, RichText};

use super::captura::Pedido;
use super::tema::{self, *};
use super::widgets::*;
use crate::instalacion;

pub const TITULO_INSTALADOR: &str = "Instalar Kairo";

#[derive(Clone)]
enum Etapa {
    Eligiendo,
    Instalando(String),
    Listo(PathBuf),
    Error(String),
}

struct Instalador {
    escritorio: bool,
    inicio_windows: bool,
    abrir_al_terminar: bool,
    etapa: Arc<Mutex<Etapa>>,
    instalada: Option<String>,
    carpeta: String,
    captura: Option<Pedido>,
}

pub fn abrir(captura: Option<Pedido>) -> eframe::Result<()> {
    let opciones = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITULO_INSTALADOR)
            .with_inner_size([560.0, 580.0])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_icon(super::icono_app()),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        TITULO_INSTALADOR,
        opciones,
        Box::new(move |cc| {
            tema::aplicar(&cc.egui_ctx);
            super::ventana::barra_de_titulo_oscura(TITULO_INSTALADOR);
            Ok(Box::new(Instalador {
                escritorio: true,
                inicio_windows: true,
                abrir_al_terminar: true,
                etapa: Arc::new(Mutex::new(Etapa::Eligiendo)),
                instalada: instalacion::version_instalada(),
                carpeta: instalacion::destinos().carpeta.display().to_string(),
                captura,
            }))
        }),
    )
}

impl Instalador {
    fn empezar(&mut self, ctx: &egui::Context) {
        let etapa = self.etapa.clone();
        let ctx = ctx.clone();
        let op = instalacion::Opciones { escritorio: self.escritorio, inicio_windows: self.inicio_windows };
        let abrir = self.abrir_al_terminar;
        *etapa.lock().unwrap() = Etapa::Instalando("Preparando…".into());
        std::thread::spawn(move || {
            let avisar = |t: &str| {
                if let Ok(mut e) = etapa.lock() {
                    *e = Etapa::Instalando(t.to_string());
                }
                ctx.request_repaint();
            };
            let fin = match instalacion::instalar(&op, &avisar) {
                Ok(exe) => {
                    if abrir {
                        instalacion::abrir_instalado(&exe);
                    }
                    Etapa::Listo(exe)
                }
                Err(e) => Etapa::Error(format!("{e:#}")),
            };
            if let Ok(mut e) = etapa.lock() {
                *e = fin;
            }
            ctx.request_repaint();
        });
    }
}

impl eframe::App for Instalador {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let etapa = self.etapa.lock().map(|e| e.clone()).unwrap_or(Etapa::Eligiendo);
        egui::CentralPanel::default().frame(Frame::none().fill(FONDO).inner_margin(Margin::same(28.0))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                logo(ui, 52.0);
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.add_space(4.0);
                    ui.label(RichText::new("Kairo").family(semibold()).size(24.0).color(TEXTO));
                    ui.label(RichText::new(format!("Versión {}", crate::version::VERSION)).size(13.0).color(SECUNDARIO));
                });
            });
            ui.add_space(10.0);
            ui.label(RichText::new("Guarda tus mejores jugadas con una tecla, sin bajar los FPS.").size(13.5).color(SECUNDARIO));
            ui.add_space(16.0);

            match &etapa {
                Etapa::Eligiendo | Etapa::Error(_) => {
                    let ancho = ui.available_width();
                    tarjeta(ui, "Instalación", |ui| {
                        ui.set_width(ancho - 38.0);
                        fila(ui, "Acceso directo en el escritorio", None, |ui| {
                            interruptor(ui, &mut self.escritorio, true);
                        });
                        separador(ui);
                        fila(ui, "Iniciar con Windows", Some("Kairo queda listo para grabar al prender la PC."), |ui| {
                            interruptor(ui, &mut self.inicio_windows, true);
                        });
                        separador(ui);
                        fila(ui, "Abrir Kairo al terminar", None, |ui| {
                            interruptor(ui, &mut self.abrir_al_terminar, true);
                        });
                    });
                    nota(ui, &format!("Se instala solo para tu usuario, sin permisos de administrador, en {}", self.carpeta));
                    if let Some(v) = &self.instalada {
                        ui.add_space(4.0);
                        nota(ui, &format!("Ya tienes Kairo {v}: se va a actualizar (tus ajustes y tus clips no se tocan)."));
                    }
                    if let Etapa::Error(e) = &etapa {
                        ui.add_space(6.0);
                        ui.label(RichText::new(e).size(12.5).color(ROJO));
                    }
                    ui.add_space(12.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let texto = if self.instalada.is_some() { "Actualizar" } else { "Instalar" };
                        if boton(ui, None, texto, Estilo::Primario, true).clicked() {
                            self.empezar(ctx);
                        }
                    });
                }
                Etapa::Instalando(paso) => {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.spinner();
                        ui.add_space(10.0);
                        ui.label(RichText::new(paso).size(14.0).color(TEXTO));
                    });
                }
                Etapa::Listo(exe) => {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Listo: Kairo quedó instalado.").family(semibold()).size(17.0).color(VERDE));
                        ui.add_space(6.0);
                        let atajo = crate::config::Config::cargar().atajo(crate::config::CualAtajo::Guardar).map(|a| a.texto());
                        let ayuda = match atajo {
                            Some(a) => format!("Búscalo junto al reloj. Mientras juegas, aprieta {a} para guardar un clip."),
                            None => "Búscalo junto al reloj, en la bandeja del sistema.".to_string(),
                        };
                        ui.label(RichText::new(ayuda).size(13.0).color(SECUNDARIO));
                        ui.add_space(18.0);
                        ui.horizontal(|ui| {
                            ui.add_space((ui.available_width() - 220.0).max(0.0) / 2.0);
                            if !self.abrir_al_terminar && boton(ui, None, "Abrir Kairo", Estilo::Secundario, true).clicked() {
                                instalacion::abrir_instalado(exe);
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            if boton(ui, None, "Cerrar", Estilo::Primario, true).clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                    });
                }
            }
        });
        if let Some(c) = &mut self.captura {
            c.paso(ctx, true);
        }
    }
}
