//! Qué pista de audio tocan los reproductores. El MP4 que escribe Media Foundation deja todas las pistas de
//! audio "habilitadas" y sin grupo. Cada reproductor elige distinto:
//! - Windows (Media Foundation, y con él Kairo) enumera las pistas al revés del archivo y toca la última:
//!   por eso la mezcla se agrega última.
//! - VLC, los navegadores, los celulares y los editores tocan la primera del archivo (o la "habilitada").
//!
//! Para que todos toquen la mezcla, después de escribir el archivo las pistas de audio se marcan como
//! alternativas (grupo 1) y solo la mezcla queda habilitada. Las demás siguen ahí (para editar, o para
//! elegirlas a mano en VLC). Se cambian unos pocos bytes del "moov"; el video y el audio no se tocan.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{bail, Result};

/// Cajas hijas de `datos[inicio..fin]`: (tipo, inicio del contenido, fin).
fn cajas(datos: &[u8], inicio: usize, fin: usize) -> Vec<([u8; 4], usize, usize)> {
    let mut v = Vec::new();
    let mut i = inicio;
    while i + 8 <= fin {
        let tam32 = u32::from_be_bytes([datos[i], datos[i + 1], datos[i + 2], datos[i + 3]]) as usize;
        let tipo = [datos[i + 4], datos[i + 5], datos[i + 6], datos[i + 7]];
        let (tam, cab) = match tam32 {
            0 => (fin - i, 8),
            1 if i + 16 <= fin => (u64::from_be_bytes(datos[i + 8..i + 16].try_into().unwrap_or_default()) as usize, 16),
            _ => (tam32, 8),
        };
        if tam < cab || i + tam > fin {
            break;
        }
        v.push((tipo, i + cab, i + tam));
        i += tam;
    }
    v
}

fn hija(datos: &[u8], inicio: usize, fin: usize, tipo: &[u8; 4]) -> Option<(usize, usize)> {
    cajas(datos, inicio, fin).into_iter().find(|(t, _, _)| t == tipo).map(|(_, a, b)| (a, b))
}

/// Marca la mezcla (la última pista de audio del archivo) como la única habilitada y todas las de audio como
/// alternativas. Devuelve si cambió algo (false si tiene una sola pista de audio o ya estaba marcado).
pub fn marcar_mezcla(ruta: &Path) -> Result<bool> {
    let mut f = OpenOptions::new().read(true).write(true).open(ruta)?;
    let largo = f.metadata()?.len();
    // Buscar el "moov" entre las cajas de arriba (el sink writer lo deja al final).
    let mut pos = 0u64;
    let moov = loop {
        if pos + 8 > largo {
            bail!("el archivo no tiene moov");
        }
        f.seek(SeekFrom::Start(pos))?;
        let mut cab = [0u8; 16];
        f.read_exact(&mut cab[..8])?;
        let tam32 = u32::from_be_bytes([cab[0], cab[1], cab[2], cab[3]]) as u64;
        let (tam, cab_tam) = match tam32 {
            0 => (largo - pos, 8),
            1 => {
                f.read_exact(&mut cab[8..16])?;
                (u64::from_be_bytes(cab[8..16].try_into()?), 16)
            }
            t => (t, 8),
        };
        if tam < cab_tam || pos + tam > largo {
            bail!("caja rota en {pos}");
        }
        if &cab[4..8] == b"moov" {
            break (pos + cab_tam, tam - cab_tam);
        }
        pos += tam;
    };
    if moov.1 > 64 * 1024 * 1024 {
        bail!("moov demasiado grande");
    }
    let mut datos = vec![0u8; moov.1 as usize];
    f.seek(SeekFrom::Start(moov.0))?;
    f.read_exact(&mut datos)?;

    // Las pistas de audio, en el orden del archivo: dónde está su tkhd y de qué versión es.
    let mut audio = Vec::new();
    for (tipo, a, b) in cajas(&datos, 0, datos.len()) {
        if &tipo != b"trak" {
            continue;
        }
        let Some((ma, mb)) = hija(&datos, a, b, b"mdia") else { continue };
        let Some((ha, _)) = hija(&datos, ma, mb, b"hdlr") else { continue };
        if datos.get(ha + 8..ha + 12) != Some(b"soun") {
            continue;
        }
        let Some((ta, tb)) = hija(&datos, a, b, b"tkhd") else { continue };
        let version = datos[ta];
        // Hasta el grupo alternativo: versión 0 = 36 bytes; versión 1 = 48.
        let grupo = ta + if version == 1 { 46 } else { 34 };
        if grupo + 2 > tb {
            continue;
        }
        audio.push((ta, grupo));
    }
    if audio.len() < 2 {
        return Ok(false);
    }

    let mut cambio = false;
    let ultima = audio.len() - 1;
    for (k, &(ta, grupo)) in audio.iter().enumerate() {
        let flags = u32::from_be_bytes([0, datos[ta + 1], datos[ta + 2], datos[ta + 3]]);
        let nuevos = if k == ultima { flags | 1 } else { flags & !1 };
        if nuevos != flags {
            datos[ta + 1..ta + 4].copy_from_slice(&nuevos.to_be_bytes()[1..]);
            f.seek(SeekFrom::Start(moov.0 + ta as u64 + 1))?;
            f.write_all(&datos[ta + 1..ta + 4])?;
            cambio = true;
        }
        if datos[grupo..grupo + 2] != [0, 1] {
            f.seek(SeekFrom::Start(moov.0 + grupo as u64))?;
            f.write_all(&[0, 1])?;
            cambio = true;
        }
    }
    f.flush()?;
    Ok(cambio)
}
