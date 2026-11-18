//! Lo que se guarda en RAM: el video ya comprimido (H.264) y el audio sin comprimir, con sus tiempos.
//! Se descarta lo que sea más viejo que la duración del clip (más un margen para empezar en un
//! cuadro clave).

use std::collections::VecDeque;
use std::sync::Arc;

use super::reloj::SEGUNDO;

pub const FRECUENCIA_AUDIO: u32 = 48_000;

#[derive(Clone)]
pub struct PaqueteVideo {
    /// Tiempo de presentación (100 ns, reloj común).
    pub pts: i64,
    pub dur: i64,
    /// Cuadro clave: un clip tiene que empezar en uno.
    pub clave: bool,
    /// H.264 en formato Annex B (con códigos de inicio).
    pub datos: Arc<Vec<u8>>,
}

#[derive(Clone)]
pub struct TrozoAudio {
    pub pts: i64,
    /// 48 kHz, estéreo intercalado, 16 bits.
    pub muestras: Arc<Vec<i16>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormatoVideo {
    pub ancho: u32,
    pub alto: u32,
    pub fps: u32,
    pub bitrate: u32,
    pub encoder: String,
}

pub struct Clip {
    pub formato: FormatoVideo,
    pub video: Vec<PaqueteVideo>,
    pub audio: Vec<TrozoAudio>,
    /// Desde y hasta (100 ns, reloj común).
    pub inicio: i64,
    pub fin: i64,
}

pub struct Buffer {
    pub formato: Option<FormatoVideo>,
    video: VecDeque<PaqueteVideo>,
    audio: VecDeque<TrozoAudio>,
    /// Cuánto se guarda (100 ns).
    retener: i64,
}

impl Buffer {
    pub fn new(segundos: u32) -> Self {
        Self { formato: None, video: VecDeque::new(), audio: VecDeque::new(), retener: (segundos as i64 + 3) * SEGUNDO }
    }

    /// Si cambia el formato (otra resolución, otro encoder), lo anterior no se puede mezclar en un clip.
    pub fn poner_formato(&mut self, f: FormatoVideo) {
        if self.formato.as_ref() != Some(&f) {
            self.video.clear();
            self.formato = Some(f);
        }
    }

    pub fn agregar_video(&mut self, paquetes: impl IntoIterator<Item = PaqueteVideo>) {
        self.video.extend(paquetes);
        let Some(ultimo) = self.video.back().map(|p| p.pts) else { return };
        while self.video.front().is_some_and(|p| ultimo - p.pts > self.retener) {
            self.video.pop_front();
        }
        // Que siempre empiece en un cuadro clave.
        while self.video.front().is_some_and(|p| !p.clave) {
            self.video.pop_front();
        }
    }

    pub fn agregar_audio(&mut self, trozos: impl IntoIterator<Item = TrozoAudio>) {
        self.audio.extend(trozos);
        let Some(ultimo) = self.audio.back().map(|t| t.pts) else { return };
        while self.audio.front().is_some_and(|t| ultimo - t.pts > self.retener + SEGUNDO) {
            self.audio.pop_front();
        }
    }

    pub fn segundos_de_video(&self) -> f64 {
        match (self.video.front(), self.video.back()) {
            (Some(a), Some(b)) => (b.pts + b.dur - a.pts) as f64 / SEGUNDO as f64,
            _ => 0.0,
        }
    }

    /// Copia (barata: los datos se comparten) de los últimos `segundos`, empezando en el cuadro clave
    /// que está justo antes.
    pub fn instantanea(&self, segundos: u32) -> Option<Clip> {
        let formato = self.formato.clone()?;
        let ultimo = self.video.back()?;
        let fin = ultimo.pts + ultimo.dur;
        let desde = fin - segundos as i64 * SEGUNDO;
        let primero = self.video.iter().rposition(|p| p.clave && p.pts <= desde).or_else(|| self.video.iter().position(|p| p.clave))?;
        let video: Vec<PaqueteVideo> = self.video.iter().skip(primero).cloned().collect();
        let inicio = video.first()?.pts;
        let audio = self
            .audio
            .iter()
            .filter(|t| {
                let dur = (t.muestras.len() / 2) as i64 * SEGUNDO / FRECUENCIA_AUDIO as i64;
                t.pts + dur > inicio && t.pts < fin
            })
            .cloned()
            .collect();
        Some(Clip { formato, video, audio, inicio, fin })
    }
}
