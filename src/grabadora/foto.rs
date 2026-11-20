//! Fotos (captura de pantalla con un atajo): el último cuadro que vio la grabadora, guardado en PNG junto a
//! los clips. Leer el cuadro de la tarjeta de video cuesta un poco, pero solo pasa cuando se pide una foto.

use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::Config;
use crate::registro::log;

/// Un cuadro en la memoria (BGRA, de arriba hacia abajo).
pub struct Imagen {
    pub ancho: u32,
    pub alto: u32,
    pub bgra: Vec<u8>,
}

/// Guarda la foto en la carpeta de clips (en la del juego, con su nombre y la fecha, como los clips) con sus
/// datos al lado. Devuelve la ruta del PNG.
pub fn guardar(cfg: &Config, juego: Option<String>, img: &Imagen) -> Result<PathBuf> {
    let ruta = super::ruta_nueva(cfg, juego.as_deref(), "png");
    guardar_en(&ruta, juego, img)
}

pub fn guardar_en(ruta: &Path, juego: Option<String>, img: &Imagen) -> Result<PathBuf> {
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut rgb = Vec::with_capacity(img.bgra.len() / 4 * 3);
    for p in img.bgra.chunks_exact(4) {
        rgb.extend_from_slice(&[p[2], p[1], p[0]]);
    }
    let tmp = ruta.with_extension("png.tmp");
    let r = (|| -> Result<()> {
        let mut enc = png::Encoder::new(BufWriter::new(std::fs::File::create(&tmp)?), img.ancho, img.alto);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        // Rápido antes que chico: pesa un poco más, pero se guarda enseguida y sin cargar al procesador.
        enc.set_compression(png::Compression::Fast);
        enc.write_header()?.write_image_data(&rgb)?;
        Ok(())
    })();
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, ruta)?;
    crate::datos_clip::escribir(
        ruta,
        &crate::datos_clip::DatosClip {
            version: 1,
            juego,
            pistas: Vec::new(),
            duracion: 0.0,
            ancho: img.ancho,
            alto: img.alto,
            fps: 0,
            creado: crate::registro::fecha_para_archivo(),
            mezcla_marcada: true,
            favorito: false,
        },
    );
    log(&format!("foto guardada: {} ({}×{})", ruta.display(), img.ancho, img.alto));
    Ok(ruta.to_path_buf())
}
