//! Las actualizaciones en la ventana: el aviso del menú y la tarjeta de Acerca de. Buscar, bajar e instalar
//! corren en otro hilo (ver `crate::actualizacion`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;

use crate::actualizacion::{self, Disponible};

#[derive(Clone)]
pub enum Paso {
    /// Todavía no se sabe (o no se busca).
    Nada,
    Buscando,
    AlDia,
    Hay(Disponible),
    Bajando(Disponible, f32),
    /// El instalador ya corre: cierra Kairo y lo vuelve a abrir.
    Instalando,
    Error(String, Option<Disponible>),
}

pub struct Actualizador {
    paso: Arc<Mutex<Paso>>,
}

impl Actualizador {
    /// Al abrir la ventana: si se buscan solas y pasaron más de 6 horas, se pregunta a GitHub; si no, se
    /// muestra lo último que se supo.
    pub fn new(ctx: &egui::Context, buscar: bool) -> Self {
        // Para las capturas de prueba: `KAIRO_CAPTURA_VERSION=0.9.0` simula que hay una versión nueva.
        if let Some(v) = std::env::var("KAIRO_CAPTURA_VERSION").ok().filter(|_| cfg!(feature = "dev")) {
            let d = Disponible {
                version: v,
                pagina: "https://github.com/blackbulldesigner/Kairo/releases".into(),
                notas: "- Editor: recorta tus clips\n- Reproductor propio\n- Compresor de clips".into(),
                instalador: None,
            };
            return Self { paso: Arc::new(Mutex::new(Paso::Hay(d))) };
        }
        let a = Self { paso: Arc::new(Mutex::new(Paso::Nada)) };
        let (paso, ctx) = (a.paso.clone(), ctx.clone());
        std::thread::spawn(move || {
            let d = if buscar { actualizacion::revisar(Duration::from_secs(6 * 3600)) } else { actualizacion::conocida() };
            if let Ok(mut p) = paso.lock() {
                if matches!(*p, Paso::Nada) {
                    *p = match d {
                        Some(d) => Paso::Hay(d),
                        None if actualizacion::revisado().is_some() => Paso::AlDia,
                        None => Paso::Nada,
                    };
                }
            }
            ctx.request_repaint();
        });
        a
    }

    pub fn paso(&self) -> Paso {
        self.paso.lock().map(|p| p.clone()).unwrap_or(Paso::Nada)
    }

    fn poner(&self, p: Paso) {
        if let Ok(mut x) = self.paso.lock() {
            *x = p;
        }
    }

    /// La versión nueva, si se sabe de una.
    pub fn disponible(&self) -> Option<Disponible> {
        match self.paso() {
            Paso::Hay(d) | Paso::Bajando(d, _) | Paso::Error(_, Some(d)) => Some(d),
            _ => None,
        }
    }

    pub fn ocupado(&self) -> bool {
        matches!(self.paso(), Paso::Buscando | Paso::Bajando(..) | Paso::Instalando)
    }

    pub fn buscar_ahora(&self, ctx: &egui::Context) {
        if self.ocupado() {
            return;
        }
        self.poner(Paso::Buscando);
        let (paso, ctx) = (self.paso.clone(), ctx.clone());
        std::thread::spawn(move || {
            let nuevo = match actualizacion::revisar_ahora() {
                Ok(Some(d)) => Paso::Hay(d),
                Ok(None) => Paso::AlDia,
                Err(e) => Paso::Error(format!("No se pudo buscar: {e:#}"), None),
            };
            if let Ok(mut p) = paso.lock() {
                *p = nuevo;
            }
            ctx.request_repaint();
        });
    }

    /// Baja el instalador, lo verifica y lo corre. Cuando corre, esta ventana se cierra (ver `revisar_cierre`).
    pub fn actualizar(&self, ctx: &egui::Context) {
        let Some(d) = self.disponible() else { return };
        if self.ocupado() {
            return;
        }
        let Some(archivo) = d.instalador.clone() else {
            self.poner(Paso::Error("Este release no trae el instalador: bájalo desde la página.".into(), Some(d)));
            return;
        };
        self.poner(Paso::Bajando(d.clone(), 0.0));
        let (paso, ctx) = (self.paso.clone(), ctx.clone());
        std::thread::spawn(move || {
            let avance = |f: f32| {
                if let Ok(mut p) = paso.lock() {
                    *p = Paso::Bajando(d.clone(), f);
                }
                ctx.request_repaint();
            };
            let r = actualizacion::descargar(&archivo, &avance).and_then(|ruta| actualizacion::instalar(&ruta));
            let nuevo = match r {
                Ok(()) => Paso::Instalando,
                Err(e) => Paso::Error(format!("No se pudo actualizar: {e:#}"), Some(d.clone())),
            };
            if let Ok(mut p) = paso.lock() {
                *p = nuevo;
            }
            ctx.request_repaint();
        });
    }

    /// Con el instalador corriendo, la ventana se cierra sola (el instalador abre la versión nueva).
    pub fn revisar_cierre(&self, ctx: &egui::Context) {
        if matches!(self.paso(), Paso::Instalando) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// "hace 3 horas", "hace 2 días"…
pub fn hace(segundos_1970: u64) -> String {
    let ahora = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = ahora.saturating_sub(segundos_1970);
    match s {
        0..=89 => "hace un momento".into(),
        90..=3599 => format!("hace {} minutos", s / 60),
        3600..=7199 => "hace una hora".into(),
        7200..=86_399 => format!("hace {} horas", s / 3600),
        86_400..=172_799 => "ayer".into(),
        _ => format!("hace {} días", s / 86_400),
    }
}
