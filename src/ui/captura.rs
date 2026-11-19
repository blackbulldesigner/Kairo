//! `--captura-ui <página> <archivo.png>` (solo con `--features dev`): abre la ventana en esa página,
//! guarda una imagen y se cierra. Sirve para revisar la interfaz sin abrirla a mano.
//! `--captura-instalador <archivo.png>` hace lo mismo con la ventana del instalador.

use std::path::PathBuf;

use eframe::egui;

use super::Pagina;

pub struct Pedido {
    pub pagina: Pagina,
    /// Tamaño de la ventana (por ejemplo "1700x950"), para ver la interfaz grande.
    pub tamano: Option<[f32; 2]>,
    /// "grabacion-juego": la página Grabación con "Solo el juego" puesto (sin guardarlo).
    pub modo_juego: bool,
    /// "audio-programas": la página Audio con "Solo los programas que elijas" (sin guardarlo).
    pub programas: bool,
    ruta: PathBuf,
    cuadros: u32,
    pedida: bool,
}

pub fn desde_args(args: &[String]) -> Option<Pedido> {
    if !cfg!(feature = "dev") {
        return None;
    }
    let i = args.iter().position(|a| a == "--captura-ui")?;
    let nombre = args.get(i + 1)?;
    let modo_juego = nombre == "grabacion-juego";
    let programas = nombre == "audio-programas";
    let pagina = Pagina::desde(nombre.trim_end_matches("-juego").trim_end_matches("-programas"))?;
    let ruta = PathBuf::from(args.get(i + 2)?);
    let tamano = args.get(i + 3).and_then(|t| {
        let (w, h) = t.split_once('x')?;
        Some([w.parse().ok()?, h.parse().ok()?])
    });
    Some(Pedido { pagina, tamano, modo_juego, programas, ruta, cuadros: 0, pedida: false })
}

/// `--captura-instalador <archivo.png>`: la ventana del instalador (no instala nada).
pub fn del_instalador(args: &[String]) -> Option<Pedido> {
    if !cfg!(feature = "dev") {
        return None;
    }
    let i = args.iter().position(|a| a == "--captura-instalador")?;
    let ruta = PathBuf::from(args.get(i + 1)?);
    Some(Pedido { pagina: Pagina::AcercaDe, tamano: None, modo_juego: false, programas: false, ruta, cuadros: 0, pedida: false })
}

impl Pedido {
    /// Se llama en cada cuadro. Espera a que el equipo esté detectado y a unos cuadros de dibujo.
    pub fn paso(&mut self, ctx: &egui::Context, listo: bool) {
        ctx.request_repaint();
        self.cuadros += 1;
        if !self.pedida {
            // Como mucho unos 10 s esperando (por ejemplo, miniaturas que tardan).
            if (listo && self.cuadros >= 8) || self.cuadros >= 600 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                self.pedida = true;
            }
            return;
        }
        let imagen = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = imagen {
            if let Err(e) = guardar_png(&self.ruta, &img) {
                eprintln!("no se pudo guardar la captura: {e}");
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

#[cfg(feature = "dev")]
fn guardar_png(ruta: &std::path::Path, img: &egui::ColorImage) -> std::io::Result<()> {
    let archivo = std::io::BufWriter::new(std::fs::File::create(ruta)?);
    let mut enc = png::Encoder::new(archivo, img.size[0] as u32, img.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(std::io::Error::other)?;
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    w.write_image_data(&bytes).map_err(std::io::Error::other)
}

#[cfg(not(feature = "dev"))]
fn guardar_png(_: &std::path::Path, _: &egui::ColorImage) -> std::io::Result<()> {
    Err(std::io::Error::other("compilado sin --features dev"))
}
