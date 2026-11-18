//! Las páginas de los ajustes.

use eframe::egui::{self, Ui};

use super::app::App;
use super::tema::*;
use super::widgets::*;
use crate::config::{self, Calidad, CualAtajo, FuenteAudio, ModoCaptura, Resolucion, TarjetaGrabacion};
use crate::equipo::Equipo;

const RAM_SI_NO_SE_SABE: u64 = 8 * 1024 * 1024 * 1024;

impl App {
    pub(super) fn pagina_grabacion(&mut self, ui: &mut Ui, eq: Option<&Equipo>) {
        encabezado(ui, "Grabación", "Qué se graba, por cuánto tiempo y con qué calidad.");

        tarjeta(ui, "Qué se graba", |ui| {
            ui.add_space(6.0);
            let ancho = ui.available_width();
            segmentos(
                ui,
                "modo",
                &mut self.cfg.modo,
                &[
                    (ModoCaptura::Escritorio, Some(icono::MONITOR), "Todo el escritorio"),
                    (ModoCaptura::Juego, Some(icono::JUEGO), "Solo el juego"),
                ],
                Some(ancho),
                40.0,
            );
            ui.add_space(4.0);
            let desc = match self.cfg.modo {
                ModoCaptura::Escritorio => "Graba la pantalla completa: el juego y todo lo que aparezca encima.",
                ModoCaptura::Juego => "Graba solo la ventana del juego: si haces Alt+Tab, en el clip no aparece tu escritorio.",
            };
            ui.label(egui::RichText::new(desc).size(12.5).color(SECUNDARIO));
            ui.add_space(6.0);

            let monitores = eq.map(|e| e.monitores.len()).unwrap_or(0);
            if self.cfg.modo == ModoCaptura::Escritorio && monitores >= 2 {
                let e = eq.expect("hay monitores");
                ui.add_space(4.0);
                separador(ui);
                fila(ui, "Monitor", Some("En automático se graba el monitor donde está el juego."), |ui| {
                    let actual = match &self.cfg.monitor {
                        None => "Automático".to_string(),
                        Some(id) => e
                            .monitores
                            .iter()
                            .position(|m| &m.id == id)
                            .map(|i| format!("Monitor {} · {}×{}", i + 1, e.monitores[i].ancho, e.monitores[i].alto))
                            .unwrap_or_else(|| "Automático".to_string()),
                    };
                    egui::ComboBox::new("monitor", "").selected_text(actual).width(220.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.cfg.monitor, None, "Automático");
                        for (i, m) in e.monitores.iter().enumerate() {
                            let texto = format!("Monitor {} · {}", i + 1, e.texto_monitor(i));
                            ui.selectable_value(&mut self.cfg.monitor, Some(m.id.clone()), texto);
                        }
                    });
                });
            }
        });

        let nativa = eq.map(|e| e.resolucion_principal()).unwrap_or((1920, 1080));
        let ram = eq.map(|e| e.ram).unwrap_or(RAM_SI_NO_SE_SABE);
        tarjeta(ui, "Clip", |ui| {
            let por_segundo = self.cfg.bytes_por_segundo(nativa);
            let maximo = config::Config::segundos_maximos(ram, por_segundo);
            // Solo se muestra recortado: el ajuste cambia cuando lo tocas (abrir la ventana no deja cambios).
            let segundos = self.cfg.segundos.min(maximo);
            let usa = config::texto_bytes(por_segundo * segundos as u64);
            let desc = format!("Lo que se guarda al apretar el atajo. Usa unos {usa} de RAM.");
            fila(ui, "Duración del clip", Some(&desc), |ui| {
                valor(ui, &config::texto_duracion(segundos), 72.0, true);
                let mut s = segundos as f32;
                if deslizador(ui, &mut s, config::SEGUNDOS_MIN as f32..=maximo as f32, 5.0, 190.0, true) {
                    self.cfg.segundos = s as u32;
                }
            });
            separador(ui);

            let corto = nativa.0.min(nativa.1);
            let opciones: Vec<(Resolucion, Option<&str>, &str)> = Resolucion::TODAS
                .iter()
                .filter(|r| r.lado_corto().map_or(true, |l| l < corto))
                .map(|r| (*r, None, r.nombre()))
                .collect();
            // Si el monitor no es más grande que lo elegido, el clip sale como el monitor: se marca "Nativa".
            let mut elegida =
                if opciones.iter().any(|(r, _, _)| *r == self.cfg.resolucion) { self.cfg.resolucion } else { Resolucion::Nativa };
            let (w, h) = self.cfg.tamano_clip(nativa);
            let desc = format!("Tus clips salen a {w}×{h}. Más baja = menos trabajo para la tarjeta de video.");
            fila(ui, "Resolución", Some(&desc), |ui| {
                if segmentos(ui, "resolucion", &mut elegida, &opciones, None, 32.0) {
                    self.cfg.resolucion = elegida;
                }
            });
            separador(ui);

            fila(ui, "Cuadros por segundo", Some("60 se ve más fluido; 30 es más liviano."), |ui| {
                segmentos(ui, "fps", &mut self.cfg.fps, &[(30, None, "30"), (60, None, "60")], None, 32.0);
            });
            separador(ui);

            let desc = format!("Unos {} Mbps. «Ligera» es ideal para laptops.", self.cfg.mbps(nativa).to_string().replace('.', ","));
            fila(ui, "Calidad", Some(&desc), |ui| {
                let opciones: Vec<(Calidad, Option<&str>, &str)> = Calidad::TODAS.iter().map(|c| (*c, None, c.nombre())).collect();
                segmentos(ui, "calidad", &mut self.cfg.calidad, &opciones, None, 32.0);
            });
        });

        tarjeta(ui, "Rendimiento", |ui| {
            match eq {
                None => fila(ui, "Tarjeta de video para grabar", Some("Detectando tu equipo…"), |_| {}),
                Some(e) if e.tiene_dos_tarjetas() => fila(
                    ui,
                    "Tarjeta de video para grabar",
                    Some("En automático se usa la que maneja tu monitor. En laptops, la integrada suele quitarle menos FPS al juego."),
                    |ui| {
                        segmentos(
                            ui,
                            "tarjeta",
                            &mut self.cfg.tarjeta,
                            &[
                                (TarjetaGrabacion::Automatica, None, "Automática"),
                                (TarjetaGrabacion::Integrada, None, "Integrada"),
                                (TarjetaGrabacion::Dedicada, None, "Dedicada"),
                            ],
                            None,
                            32.0,
                        );
                    },
                ),
                Some(e) => {
                    let nombre = e.gpus.first().map(|g| g.nombre.clone()).unwrap_or_else(|| "No se detectó ninguna".into());
                    fila(ui, "Tarjeta de video para grabar", Some(&nombre), |_| {});
                }
            }
            if eq.is_some_and(|e| e.es_laptop()) {
                separador(ui);
                fila(ui, "Ahorro con batería", Some("Desenchufada, la laptop graba a 720p y en calidad Ligera."), |ui| {
                    interruptor(ui, &mut self.cfg.ahorro_bateria, true);
                });
            }
        });
    }

    pub(super) fn pagina_audio(&mut self, ui: &mut Ui, eq: Option<&Equipo>) {
        encabezado(ui, "Audio", "Qué se escucha en tus clips.");
        let por_juego = eq.map_or(true, |e| e.audio_por_juego());
        tarjeta(ui, "Fuentes", |ui| {
            let desc_juego = if por_juego {
                "Solo el juego que estás jugando."
            } else {
                "Necesita Windows 10 versión 2004 o más nuevo."
            };
            fuente(ui, "Sonido del juego", desc_juego, &mut self.cfg.audio_juego, por_juego);
            separador(ui);
            fuente(
                ui,
                "Resto del sistema",
                "Discord, música, navegador… Apágalo para que no salga en tus clips.",
                &mut self.cfg.audio_sistema,
                true,
            );
            separador(ui);
            fuente(ui, "Micrófono", "El micrófono predeterminado de Windows.", &mut self.cfg.audio_mic, true);
        });
        nota(ui, "Por ahora las fuentes se mezclan en una sola pista. Las pistas separadas llegan más adelante.");
    }

    pub(super) fn pagina_clips(&mut self, ui: &mut Ui) {
        encabezado(ui, "Clips", "Dónde se guardan tus clips.");
        tarjeta(ui, "Carpeta", |ui| {
            let ruta = self.cfg.carpeta.display().to_string();
            let libre = config::espacio_libre(&self.cfg.carpeta)
                .map(|b| format!("Libre en ese disco: {}.", config::texto_bytes(b)))
                .unwrap_or_default();
            let mut cambiar = false;
            let mut abrir = false;
            fila(ui, &ruta, Some(&libre), |ui| {
                abrir = boton(ui, Some(icono::CARPETA_ABIERTA), "Abrir", Estilo::Secundario, true).clicked();
                ui.add_space(2.0);
                cambiar = boton(ui, Some(icono::EDITAR), "Cambiar…", Estilo::Secundario, true).clicked();
            });
            if abrir {
                let _ = std::fs::create_dir_all(&self.cfg.carpeta);
                let _ = std::process::Command::new("explorer").arg(&self.cfg.carpeta).spawn();
            }
            if cambiar {
                let inicio = if self.cfg.carpeta.exists() { self.cfg.carpeta.clone() } else { config::carpeta_videos() };
                if let Some(nueva) = rfd::FileDialog::new().set_title("Elige la carpeta de tus clips").set_directory(inicio).pick_folder() {
                    self.cfg.carpeta = nueva;
                }
            }
            separador(ui);
            fila(ui, "Sonido al guardar", Some("Un aviso corto para saber que el clip se guardó."), |ui| {
                interruptor(ui, &mut self.cfg.sonido_al_guardar, true);
            });
        });
    }

    pub(super) fn pagina_atajos(&mut self, ui: &mut Ui) {
        encabezado(ui, "Atajos", "Teclas que funcionan aunque estés dentro del juego.");
        tarjeta(ui, "Atajos globales", |ui| {
            for (i, cual) in CualAtajo::TODOS.into_iter().enumerate() {
                if i > 0 {
                    separador(ui);
                }
                let capturando = self.capturando == Some(cual);
                let mut clic = false;
                fila(ui, cual.titulo(), cual.descripcion(), |ui| {
                    clic = atajo(ui, self.cfg.atajo(cual), capturando).clicked();
                });
                if clic {
                    self.capturando = if capturando { None } else { Some(cual) };
                }
            }
        });
        nota(ui, "Haz clic en un atajo y aprieta la tecla nueva. Esc cancela; Retroceso lo deja sin asignar.");
    }

    pub(super) fn pagina_sistema(&mut self, ui: &mut Ui, eq: Option<&Equipo>) {
        encabezado(ui, "Sistema", "Lo que Kairo detectó de tu equipo. Si algo falla, copia el diagnóstico y compártelo.");
        tarjeta(ui, "Tu equipo", |ui| {
            let Some(e) = eq else {
                fila_info(ui, "Detectando…", "");
                return;
            };
            fila_info(ui, "Equipo", e.texto_energia());
            separador(ui);
            fila_info(ui, "Windows", &e.windows);
            separador(ui);
            fila_info(ui, "Procesador", &e.procesador);
            separador(ui);
            fila_info(ui, "Memoria RAM", &e.texto_ram());
            for (i, g) in e.gpus.iter().enumerate() {
                separador(ui);
                fila_info(ui, Equipo::etiqueta_gpu(g), &e.texto_gpu(i));
            }
            for i in 0..e.monitores.len() {
                separador(ui);
                fila_info(ui, &format!("Monitor {}", i + 1), &e.texto_monitor(i));
            }
            separador(ui);
            fila_info(ui, "Encoders por hardware", &e.texto_encoders());
            separador(ui);
            fila_info(ui, "Audio solo del juego", e.texto_audio_por_juego());
            ui.add_space(8.0);
            let mut copiar = false;
            let mut abrir = false;
            ui.horizontal(|ui| {
                copiar = boton(ui, Some(icono::COPIAR), "Copiar diagnóstico", Estilo::Secundario, true).clicked();
                abrir = boton(ui, Some(icono::CARPETA_ABIERTA), "Abrir carpeta de datos", Estilo::Secundario, true).clicked();
            });
            ui.add_space(8.0);
            if copiar {
                ui.output_mut(|o| o.copied_text = e.como_texto());
                self.avisar("Diagnóstico copiado: pégalo donde quieras compartirlo.", false);
            }
            if abrir {
                let dir = config::datos_dir();
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer").arg(dir).spawn();
            }
        });

        tarjeta(ui, "Acerca de", |ui| {
            fila_info(ui, "Versión", &format!("Kairo {} (vista previa)", env!("CARGO_PKG_VERSION")));
            separador(ui);
            fila_info(ui, "Licencia", "GPL-3.0: código abierto. Puedes usarlo, estudiarlo y mejorarlo.");
            separador(ui);
            fila_info(ui, "Estado", "Los ajustes y el diagnóstico ya funcionan. La grabadora llega en la fase 1.");
        });
    }
}

/// Fila de una fuente de audio: interruptor y volumen.
fn fuente(ui: &mut Ui, titulo: &str, desc: &str, f: &mut FuenteAudio, disponible: bool) {
    fila(ui, titulo, Some(desc), |ui| {
        interruptor(ui, &mut f.activa, disponible);
        ui.add_space(12.0);
        let activo = disponible && f.activa;
        valor(ui, &format!("{} %", f.volumen), 46.0, activo);
        let mut v = f.volumen as f32;
        if deslizador(ui, &mut v, 0.0..=100.0, 5.0, 150.0, activo) {
            f.volumen = v as u8;
        }
    });
}
