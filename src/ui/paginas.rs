//! Las páginas de los ajustes.

use eframe::egui::{self, Ui};

use super::app::App;
use super::tema::*;
use super::widgets::*;
use crate::config::{self, Calidad, CualAtajo, FuenteAudio, ModoCaptura, OtrosSonidos, Resolucion, TarjetaGrabacion};
use crate::equipo::Equipo;
use crate::version;

const RAM_SI_NO_SE_SABE: u64 = 8 * 1024 * 1024 * 1024;

impl App {
    fn ahora_ui(&self, ui: &Ui) -> f64 {
        ui.input(|i| i.time)
    }

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
                ModoCaptura::Juego => {
                    "Graba solo el juego: si haces Alt+Tab o lo minimizas, esa parte del clip queda en negro y tu \
                     escritorio no aparece."
                }
            };
            ui.label(egui::RichText::new(desc).size(12.5).color(SECUNDARIO));
            ui.add_space(6.0);

            if self.cfg.modo == ModoCaptura::Juego {
                // La lista de programas abiertos se relee cada 3 s mientras esta página está a la vista.
                if self.ahora_ui(ui) >= self.programas_hasta {
                    self.programas = crate::grabadora::juego::programas_abiertos();
                    self.programas_hasta = self.ahora_ui(ui) + 3.0;
                }
                ui.add_space(4.0);
                separador(ui);
                let desc = match &self.cfg.juego_elegido {
                    None => "Automático: el que esté adelante en pantalla completa.".to_string(),
                    Some(j) if self.programas.iter().any(|p| p.exe == j.exe) => {
                        "Elegido a mano: sirve también si juegas en ventana.".to_string()
                    }
                    Some(j) => format!("{} no está abierto: el clip queda en negro hasta que lo abras.", j.nombre),
                };
                fila(ui, "Juego", Some(&desc), |ui| {
                    let actual = self.cfg.juego_elegido.as_ref().map(|j| j.nombre.clone()).unwrap_or_else(|| "Automático".into());
                    egui::ComboBox::new("juego", "").selected_text(actual).width(260.0).height(360.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.cfg.juego_elegido, None, "Automático (pantalla completa)");
                        if let Some(j) = self.cfg.juego_elegido.clone() {
                            if !self.programas.iter().any(|p| p.exe == j.exe) {
                                ui.selectable_value(&mut self.cfg.juego_elegido, Some(j.clone()), format!("{} (cerrado)", j.nombre));
                            }
                        }
                        ui.separator();
                        for p in &self.programas {
                            let elegido = crate::config::JuegoElegido { exe: p.exe.clone(), nombre: p.nombre.clone() };
                            ui.selectable_value(&mut self.cfg.juego_elegido, Some(elegido), &p.nombre).on_hover_text(&p.exe);
                        }
                    });
                });
            }

            let monitores = eq.map(|e| e.monitores.len()).unwrap_or(0);
            if self.cfg.modo == ModoCaptura::Escritorio && monitores >= 2 {
                let e = eq.expect("hay monitores");
                ui.add_space(4.0);
                separador(ui);
                fila(ui, "Monitor", Some("En automático se graba el monitor donde está el juego (si no hay juego, el principal)."), |ui| {
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
                    Some(
                        "En automático se usa la que maneja tu monitor. En laptops, la integrada suele quitarle menos FPS \
                         al juego. Si Windows no deja capturar desde la elegida, Kairo usa la captura de respaldo.",
                    ),
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
            if eq.is_some_and(|e| e.encoders.is_empty()) {
                separador(ui);
                fila(
                    ui,
                    "Sin encoder por hardware",
                    Some("Tu tarjeta no comprime video: Kairo graba con el procesador, a 1080p y 30 fps como máximo, para no quitarle al juego."),
                    |_| {},
                );
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
        encabezado(ui, "Audio", "Qué se escucha en tus clips, cada fuente con su volumen.");
        let por_juego = eq.map_or(true, |e| e.audio_por_juego());
        tarjeta(ui, "Fuentes", |ui| {
            let desc_juego = if por_juego {
                "Solo el juego detectado (o el que elegiste en Grabación)."
            } else {
                "Necesita Windows 10 versión 2004 o más nuevo."
            };
            fuente(ui, "Sonido del juego", desc_juego, &mut self.cfg.audio_juego, por_juego);
            separador(ui);
            fuente(ui, "Micrófono", "El micrófono predeterminado de Windows.", &mut self.cfg.audio_mic, true);
        });

        // La lista de programas abiertos se relee cada 3 s mientras la página está a la vista.
        let ahora = self.ahora_ui(ui);
        if self.cfg.otros_sonidos == OtrosSonidos::Elegidos && ahora >= self.programas_hasta {
            self.programas = crate::grabadora::juego::programas_abiertos();
            self.programas_hasta = ahora + 3.0;
        }
        tarjeta(ui, "Otros sonidos", |ui| {
            if !por_juego {
                fuente(ui, "Resto del sistema", "Todo lo que suena en la PC, el juego incluido.", &mut self.cfg.audio_sistema, true);
                return;
            }
            ui.add_space(6.0);
            let ancho = ui.available_width();
            segmentos(
                ui,
                "otros",
                &mut self.cfg.otros_sonidos,
                &[(OtrosSonidos::Todo, None, "Todo lo demás"), (OtrosSonidos::Elegidos, None, "Solo los programas que elijas")],
                Some(ancho),
                36.0,
            );
            ui.add_space(4.0);
            let desc = match self.cfg.otros_sonidos {
                OtrosSonidos::Todo => "Todo lo que suena en la PC menos el juego, en una sola pista.",
                OtrosSonidos::Elegidos => {
                    "Solo los programas de la lista, cada uno con su volumen y su pista. Si uno no está abierto, se suma \
                     solo cuando lo abras."
                }
            };
            ui.label(egui::RichText::new(desc).size(12.5).color(SECUNDARIO));
            ui.add_space(6.0);
            match self.cfg.otros_sonidos {
                OtrosSonidos::Todo => {
                    separador(ui);
                    fuente(ui, "Resto del sistema", "Discord, música, navegador… Apágalo para que no salga en tus clips.", &mut self.cfg.audio_sistema, true);
                }
                OtrosSonidos::Elegidos => {
                    let mut quitar = None;
                    for (i, p) in self.cfg.programas_audio.iter_mut().enumerate() {
                        separador(ui);
                        let abierto = self.programas.iter().any(|x| x.exe == p.exe);
                        let desc = if abierto { p.exe.clone() } else { format!("{} · no está abierto", p.exe) };
                        fila(ui, &p.nombre, Some(&desc), |ui| {
                            if boton_icono(ui, icono::QUITAR, "Quitar de la lista").clicked() {
                                quitar = Some(i);
                            }
                            ui.add_space(8.0);
                            interruptor(ui, &mut p.activa, true);
                            ui.add_space(12.0);
                            valor(ui, &format!("{} %", p.volumen), 46.0, p.activa);
                            let mut v = p.volumen as f32;
                            if deslizador(ui, &mut v, 0.0..=100.0, 5.0, 150.0, p.activa) {
                                p.volumen = v as u8;
                            }
                        });
                    }
                    if let Some(i) = quitar {
                        self.cfg.programas_audio.remove(i);
                    }
                    separador(ui);
                    let juego = self.cfg.juego_elegido.as_ref().map(|j| j.exe.clone());
                    let disponibles: Vec<_> = self
                        .programas
                        .iter()
                        .filter(|x| !self.cfg.programas_audio.iter().any(|p| p.exe == x.exe) && Some(&x.exe) != juego.as_ref())
                        .cloned()
                        .collect();
                    let lleno = self.cfg.programas_audio.len() >= crate::config::PROGRAMAS_MAX;
                    let desc = if lleno {
                        "Ya hay 8 programas: quita uno para sumar otro."
                    } else if disponibles.is_empty() {
                        "No hay otros programas abiertos con ventana."
                    } else {
                        "Elige entre los programas abiertos (por ejemplo, Discord o tu música)."
                    };
                    let mut agregar = None;
                    fila(ui, "Agregar programa", Some(desc), |ui| {
                        ui.add_enabled_ui(!lleno && !disponibles.is_empty(), |ui| {
                            egui::ComboBox::new("agregar_programa", "").selected_text("+ Agregar programa").width(240.0).height(320.0).show_ui(
                                ui,
                                |ui| {
                                    for x in &disponibles {
                                        if ui.selectable_label(false, &x.nombre).on_hover_text(&x.exe).clicked() {
                                            agregar = Some(x.clone());
                                        }
                                    }
                                },
                            );
                        });
                    });
                    if let Some(x) = agregar {
                        self.cfg.programas_audio.push(crate::config::ProgramaAudio { exe: x.exe, nombre: x.nombre, activa: true, volumen: 100 });
                    }
                }
            }
        });

        tarjeta(ui, "Pistas", |ui| {
            fila(
                ui,
                "Pistas separadas",
                Some(
                    "Además de la mezcla, cada fuente (y cada programa) va en su propia pista del MP4: sirve para editar, por \
                     ejemplo para quitar Discord y dejar solo el juego.",
                ),
                |ui| {
                    interruptor(ui, &mut self.cfg.pistas_separadas, true);
                },
            );
        });
        if !por_juego {
            nota(ui, "En esta versión de Windows no se puede separar el sonido del juego ni de cada programa: va todo junto.");
        }
    }

    pub(super) fn pagina_clips(&mut self, ui: &mut Ui) {
        encabezado(ui, "Guardado", "Dónde y cómo se guardan tus clips.");
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
            fila(
                ui,
                "Una carpeta por juego",
                Some("Los clips de cada juego van en su carpeta (por ejemplo, Clips\\VALORANT) y llevan su nombre."),
                |ui| {
                    interruptor(ui, &mut self.cfg.carpeta_por_juego, true);
                },
            );
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
        let estado = self.estado.clone();
        tarjeta(ui, "Grabadora", |ui| match &estado {
            None => fila_info(ui, "Estado", "Cerrada"),
            Some(e) => {
                let texto = if e.pausado {
                    "En pausa"
                } else if e.grabando {
                    "Grabando"
                } else if e.error.is_some() {
                    "No puede grabar"
                } else {
                    "Preparando"
                };
                fila_info(ui, "Estado", texto);
                separador(ui);
                fila_info(ui, "Juego", e.juego.as_deref().unwrap_or("Ninguno detectado"));
                if let Some(err) = &e.error {
                    separador(ui);
                    fila_info_color(ui, "Problema", err, ROJO);
                }
                if !e.detalle.is_empty() && e.grabando {
                    separador(ui);
                    fila_info(ui, "Grabando con", &e.detalle.replace(" · ", "\n"));
                }
                if e.ahorro {
                    separador(ui);
                    fila_info(ui, "Ahorro con batería", "Activo: 720p y calidad Ligera mientras esté desenchufada.");
                }
                if let Some(c) = &e.ultimo_clip {
                    separador(ui);
                    fila_info(ui, "Último clip", c);
                }
            }
        });
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

        tarjeta(ui, "Inicio", |ui| {
            // Se aplica al momento (no espera a «Guardar cambios»): es un ajuste de Windows, no de config.json.
            fila(ui, "Iniciar con Windows", Some("Kairo arranca en la bandeja, sin abrir ventanas."), |ui| {
                if interruptor(ui, &mut self.autoinicio, true) {
                    crate::autoinicio::poner(self.autoinicio);
                    self.autoinicio = crate::autoinicio::activo();
                }
            });
        });
    }

    pub(super) fn pagina_acerca(&mut self, ui: &mut Ui) {
        encabezado(ui, "Acerca de", "Qué versión de Kairo estás usando y qué cambió en cada una.");
        let novedades = version::novedades();
        tarjeta(ui, "Esta versión", |ui| {
            fila_info(ui, "Versión", &format!("Kairo {} (vista previa)", version::VERSION));
            separador(ui);
            let compilacion = if version::COMMIT.is_empty() {
                "Sin datos de git".to_string()
            } else {
                format!("Commit {} · rama {}", version::COMMIT, version::RAMA)
            };
            fila_info(ui, "Compilación", &compilacion);
            if let Some(n) = novedades.first() {
                separador(ui);
                fila_info(ui, "Último cambio", &format!("{} ({})", n.texto, version::FECHA_COMMIT));
            }
            separador(ui);
            fila_info(ui, "Compilado el", &version::compilado());
            if version::con_cambios_sin_commit() {
                separador(ui);
                fila_info_color(
                    ui,
                    "Atención",
                    "Este exe se compiló con cambios que todavía no tienen commit: puede traer más de lo que dice la lista.",
                    AMARILLO,
                );
            }
            ui.add_space(8.0);
            let mut copiar = false;
            let mut github = false;
            ui.horizontal(|ui| {
                copiar = boton(ui, Some(icono::COPIAR), "Copiar versión", Estilo::Secundario, true).clicked();
                github = boton(ui, Some(icono::ENLACE), "Ver en GitHub", Estilo::Secundario, true).clicked();
            });
            ui.add_space(8.0);
            if copiar {
                let texto = format!("Kairo {} ({}, compilado el {})", version::corta(), version::FECHA_COMMIT, version::compilado());
                ui.output_mut(|o| o.copied_text = texto);
                self.avisar("Versión copiada.", false);
            }
            if github {
                let _ = std::process::Command::new("explorer").arg("https://github.com/blackbulldesigner/Kairo").spawn();
            }
        });

        tarjeta(ui, "Novedades", |ui| {
            if novedades.is_empty() {
                fila_info(ui, "Sin datos", "Este exe se compiló sin git: no trae la lista de cambios.");
            }
            for (i, n) in novedades.iter().enumerate() {
                if i > 0 {
                    separador(ui);
                }
                fila_novedad(ui, n.fecha, n.texto, n.commit, i == 0);
            }
        });

        tarjeta(ui, "Licencia", |ui| {
            fila_info(ui, "GPL-3.0", "Código abierto: puedes usarlo, estudiarlo, mejorarlo y compartirlo con su código.");
        });
    }
}

/// Fila de una fuente de audio: interruptor y volumen.
fn fuente(ui: &mut Ui, titulo: &str, desc: &str, f: &mut FuenteAudio, disponible: bool) {
    fila(ui, titulo, Some(desc), |ui| {
        // Si la fuente no está disponible se muestra apagada, aunque el ajuste guardado diga otra cosa.
        let mut apagada = false;
        interruptor(ui, if disponible { &mut f.activa } else { &mut apagada }, disponible);
        ui.add_space(12.0);
        let activo = disponible && f.activa;
        valor(ui, &format!("{} %", f.volumen), 46.0, activo);
        let mut v = f.volumen as f32;
        if deslizador(ui, &mut v, 0.0..=100.0, 5.0, 150.0, activo) {
            f.volumen = v as u8;
        }
    });
}
