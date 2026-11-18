//! `--captura-ui <página> <archivo.png>` (solo con `--features dev`): abre la ventana en esa página,
//! guarda una imagen y se cierra. Sirve para revisar la interfaz sin abrirla a mano.

use std::path::PathBuf;

use eframe::egui;

use super::Pagina;

pub struct Pedido {
    pub pagina: Pagina,
    ruta: PathBuf,
    cuadros: u32,
    pedida: bool,
}

pub fn desde_args(args: &[String]) -> Option<Pedido> {
    if !cfg!(feature = "dev") {
        return None;
    }
    let i = args.iter().position(|a| a == "--captura-ui")?;
    let pagina = Pagina::desde(args.get(i + 1)?)?;
    let ruta = PathBuf::from(args.get(i + 2)?);
    Some(Pedido { pagina, ruta, cuadros: 0, pedida: false })
}

impl Pedido {
    /// Se llama en cada cuadro. Espera a que el equipo esté detectado y a unos cuadros de dibujo.
    pub fn paso(&mut self, ctx: &egui::Context, listo: bool) {
        ctx.request_repaint();
        self.cuadros += 1;
        if !self.pedida {
            if listo && self.cuadros >= 8 {
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
