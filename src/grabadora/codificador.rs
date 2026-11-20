//! El codificador que usa la grabadora: el encoder por hardware de la tarjeta de video o, si la
//! tarjeta no tiene (o no funciona), el H.264 de Windows por procesador.

use anyhow::{anyhow, Context, Result};
use windows::core::{Interface, VARIANT};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

use super::buffer::PaqueteVideo;
use super::encoder::{color_bt709, sacar_paquete, tamano, Encoder, Salida};
use super::reloj::SEGUNDO;
use crate::config::{AjustesEncoder, PerfilH264};

pub enum Codificador {
    Hardware(Encoder),
    Procesador(EncoderProcesador),
}

impl Codificador {
    pub fn nombre(&self) -> &str {
        match self {
            Codificador::Hardware(e) => &e.nombre,
            Codificador::Procesador(_) => "Procesador (H.264 de Windows)",
        }
    }

    /// El formato en que comprime (el de procesador, siempre H.264).
    pub fn codec(&self) -> crate::config::Codec {
        match self {
            Codificador::Hardware(e) => e.codec,
            Codificador::Procesador(_) => crate::config::Codec::H264,
        }
    }

    pub fn tamano(&self) -> (u32, u32) {
        match self {
            Codificador::Hardware(e) => (e.ancho, e.alto),
            Codificador::Procesador(e) => (e.ancho, e.alto),
        }
    }

    pub fn atender(&mut self, salida: &mut Vec<PaqueteVideo>) -> Result<()> {
        match self {
            Codificador::Hardware(e) => e.atender(salida),
            Codificador::Procesador(e) => {
                salida.append(&mut e.listos);
                Ok(())
            }
        }
    }

    pub fn puede_recibir(&self) -> bool {
        match self {
            Codificador::Hardware(e) => e.puede_recibir(),
            Codificador::Procesador(_) => true,
        }
    }

    pub fn enviar(&mut self, textura: &ID3D11Texture2D, pts: i64) -> Result<()> {
        match self {
            Codificador::Hardware(e) => e.enviar(textura, pts),
            Codificador::Procesador(e) => e.enviar(textura, pts),
        }
    }
}

/// H.264 de Microsoft por procesador. Cada cuadro NV12 se trae de la tarjeta a la memoria (textura
/// de lectura) y se comprime. Es mucho más pesado que el de hardware: por eso se usa con 1080p y
/// 30 fps como máximo.
pub struct EncoderProcesador {
    mft: IMFTransform,
    contexto: ID3D11DeviceContext,
    lectura: ID3D11Texture2D,
    pub ancho: u32,
    pub alto: u32,
    dur: i64,
    listos: Vec<PaqueteVideo>,
}

impl EncoderProcesador {
    /// Siempre con bitrate constante y lo más rápido posible; del resto se respeta lo elegido (cuadros clave
    /// y perfil, que en este encoder es como mucho Main).
    pub fn abrir(dispositivo: &ID3D11Device, ancho: u32, alto: u32, ajustes: &AjustesEncoder) -> Result<Self> {
        let (fps, bitrate) = (ajustes.fps, ajustes.bitrate);
        unsafe {
            let mft: IMFTransform =
                CoCreateInstance(&CLSID_MSH264EncoderMFT, None, CLSCTX_INPROC_SERVER).context("Windows no tiene el encoder H.264")?;
            if let Ok(api) = mft.cast::<ICodecAPI>() {
                let poner = |clave: &windows::core::GUID, v: VARIANT| {
                    let _ = api.SetValue(clave, &v);
                };
                poner(&CODECAPI_AVEncCommonRateControlMode, VARIANT::from(eAVEncCommonRateControlMode_CBR.0 as u32));
                poner(&CODECAPI_AVEncCommonMeanBitRate, VARIANT::from(bitrate));
                poner(&CODECAPI_AVEncMPVGOPSize, VARIANT::from(ajustes.gop));
                poner(&CODECAPI_AVEncMPVDefaultBPictureCount, VARIANT::from(0u32));
                poner(&CODECAPI_AVLowLatencyMode, VARIANT::from(true));
                // Prioriza la velocidad: lo importante es no quitarle procesador al juego.
                poner(&CODECAPI_AVEncCommonQualityVsSpeed, VARIANT::from(0u32));
            }
            let salida = MFCreateMediaType()?;
            salida.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            salida.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            salida.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
            salida.SetUINT64(&MF_MT_FRAME_SIZE, tamano(ancho, alto))?;
            salida.SetUINT64(&MF_MT_FRAME_RATE, tamano(fps, 1))?;
            salida.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, tamano(1, 1))?;
            salida.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            let perfil = if ajustes.perfil == PerfilH264::Basico { PerfilH264::Basico } else { PerfilH264::Principal };
            salida.SetUINT32(&MF_MT_MPEG2_PROFILE, super::encoder::perfil(perfil))?;
            color_bt709(&salida)?;
            mft.SetOutputType(0, &salida, 0).with_context(|| format!("el encoder por procesador no acepta {ancho}×{alto}"))?;

            let entrada = MFCreateMediaType()?;
            entrada.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            entrada.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            entrada.SetUINT64(&MF_MT_FRAME_SIZE, tamano(ancho, alto))?;
            entrada.SetUINT64(&MF_MT_FRAME_RATE, tamano(fps, 1))?;
            entrada.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, tamano(1, 1))?;
            entrada.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            color_bt709(&entrada)?;
            mft.SetInputType(0, &entrada, 0)?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;

            let desc = D3D11_TEXTURE2D_DESC {
                Width: ancho,
                Height: alto,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_NV12,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut lectura = None;
            dispositivo.CreateTexture2D(&desc, None, Some(&mut lectura))?;
            let lectura = lectura.ok_or_else(|| anyhow!("sin textura de lectura"))?;
            let contexto = dispositivo.GetImmediateContext()?;
            Ok(Self { mft, contexto, lectura, ancho, alto, dur: SEGUNDO / fps as i64, listos: Vec::new() })
        }
    }

    fn enviar(&mut self, textura: &ID3D11Texture2D, pts: i64) -> Result<()> {
        let (w, h) = (self.ancho as usize, self.alto as usize);
        unsafe {
            self.contexto.CopyResource(&self.lectura, textura);
            let mut mapa = D3D11_MAPPED_SUBRESOURCE::default();
            self.contexto.Map(&self.lectura, 0, D3D11_MAP_READ, 0, Some(&mut mapa))?;
            let buffer = MFCreateMemoryBuffer((w * h * 3 / 2) as u32);
            let r = (|| -> Result<IMFMediaBuffer> {
                let buffer = buffer?;
                let mut destino = std::ptr::null_mut();
                buffer.Lock(&mut destino, None, None)?;
                let origen = mapa.pData as *const u8;
                let paso = mapa.RowPitch as usize;
                // NV12: el plano Y (h filas) y debajo el plano UV intercalado (h/2 filas).
                for fila in 0..h + h / 2 {
                    std::ptr::copy_nonoverlapping(origen.add(fila * paso), destino.add(fila * w), w);
                }
                buffer.Unlock()?;
                buffer.SetCurrentLength((w * h * 3 / 2) as u32)?;
                Ok(buffer)
            })();
            self.contexto.Unmap(&self.lectura, 0);
            let buffer = r?;
            let muestra = MFCreateSample()?;
            muestra.AddBuffer(&buffer)?;
            muestra.SetSampleTime(pts)?;
            muestra.SetSampleDuration(self.dur)?;
            self.mft.ProcessInput(0, &muestra, 0)?;
            loop {
                match sacar_paquete(&self.mft, self.dur, crate::config::Codec::H264)? {
                    Salida::Paquete(p) => self.listos.push(p),
                    Salida::Reintentar => continue,
                    Salida::NadaMas => break,
                }
            }
        }
        Ok(())
    }
}
