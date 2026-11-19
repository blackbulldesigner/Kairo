//! Encoder H.264 por hardware de la tarjeta de video (NVIDIA, AMD o Intel), a través de Media
//! Foundation. Recibe texturas NV12 que ya están en la tarjeta y devuelve paquetes comprimidos.
//! Los encoders por hardware son asíncronos: avisan con eventos cuándo quieren un cuadro y cuándo
//! tienen un paquete listo.

use std::mem::ManuallyDrop;

use anyhow::{anyhow, bail, Context, Result};
use windows::core::{Interface, PWSTR, VARIANT};
use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::CoTaskMemFree;

use super::buffer::PaqueteVideo;
use super::h264;
use super::reloj::SEGUNDO;
use crate::config::{AjustesEncoder, ControlBitrate, PerfilH264};

/// El perfil H.264 para Media Foundation.
pub(super) fn perfil(p: PerfilH264) -> u32 {
    match p {
        PerfilH264::Alto => eAVEncH264VProfile_High.0 as u32,
        PerfilH264::Principal => eAVEncH264VProfile_Main.0 as u32,
        PerfilH264::Basico => eAVEncH264VProfile_Base.0 as u32,
    }
}

pub struct Encoder {
    mft: IMFTransform,
    eventos: IMFMediaEventGenerator,
    _administrador: IMFDXGIDeviceManager,
    pub nombre: String,
    pub ancho: u32,
    pub alto: u32,
    dur: i64,
    /// Cuántos cuadros pidió el encoder y todavía no recibió.
    pedidos: u32,
}

pub(super) fn tamano(a: u32, b: u32) -> u64 {
    ((a as u64) << 32) | b as u64
}

unsafe fn nombre_de(act: &IMFActivate) -> String {
    let mut texto = PWSTR::null();
    let mut largo = 0u32;
    if act.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut texto, &mut largo).is_ok() {
        let n = texto.to_string().unwrap_or_default();
        CoTaskMemFree(Some(texto.0 as *const _));
        n.trim().to_string()
    } else {
        "Encoder por hardware".to_string()
    }
}

/// La tarjeta a la que pertenece un encoder por hardware (si lo dice).
unsafe fn luid_de(act: &IMFActivate) -> Option<LUID> {
    let mut luid = LUID::default();
    let bytes = std::slice::from_raw_parts_mut(&mut luid as *mut LUID as *mut u8, std::mem::size_of::<LUID>());
    act.GetBlob(&MFT_ENUM_ADAPTER_LUID, bytes, None).ok().map(|_| luid)
}

impl Encoder {
    /// Abre el encoder por hardware de la tarjeta `luid`. Si el tamaño no le gusta (algunos piden
    /// múltiplos de 16), prueba con el múltiplo de 16 más cercano hacia abajo.
    pub fn abrir(dispositivo: &ID3D11Device, luid: LUID, ancho: u32, alto: u32, ajustes: &AjustesEncoder) -> Result<Self> {
        let candidatos = unsafe { encoders_de(luid)? };
        if candidatos.is_empty() {
            bail!("Tu tarjeta de video no tiene encoder H.264 por hardware (la grabación por procesador llega más adelante).");
        }
        let mut ultimo_error = None;
        for act in &candidatos {
            let tamanos = [(ancho, alto), (ancho / 16 * 16, alto / 16 * 16)];
            for (i, &(w, h)) in tamanos.iter().enumerate() {
                if i > 0 && (w, h) == tamanos[0] {
                    continue;
                }
                match unsafe { Self::probar(act, dispositivo, w, h, ajustes) } {
                    Ok(e) => return Ok(e),
                    Err(e) => ultimo_error = Some(e),
                }
            }
        }
        Err(ultimo_error.unwrap_or_else(|| anyhow!("ningún encoder aceptó la configuración")))
    }

    unsafe fn probar(act: &IMFActivate, dispositivo: &ID3D11Device, ancho: u32, alto: u32, ajustes: &AjustesEncoder) -> Result<Self> {
        let (fps, bitrate) = (ajustes.fps, ajustes.bitrate);
        let nombre = nombre_de(act);
        let mft: IMFTransform = act.ActivateObject().with_context(|| format!("no se pudo abrir {nombre}"))?;
        let atributos = mft.GetAttributes()?;
        if atributos.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) == 0 {
            bail!("{nombre} no es asíncrono");
        }
        atributos.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;

        let mut token = 0u32;
        let mut administrador = None;
        MFCreateDXGIDeviceManager(&mut token, &mut administrador)?;
        let administrador = administrador.ok_or_else(|| anyhow!("sin administrador DXGI"))?;
        administrador.ResetDevice(dispositivo, token)?;
        mft.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, administrador.as_raw() as usize)
            .with_context(|| format!("{nombre} no acepta la tarjeta de video"))?;

        // Ajustes del encoder (si alguno no lo soporta, se sigue con lo que tenga por defecto).
        if let Ok(api) = mft.cast::<ICodecAPI>() {
            let poner = |clave: &windows::core::GUID, v: VARIANT| api.SetValue(clave, &v).is_ok();
            // VBR con tope (por defecto 1,5 × el promedio): calidad pareja sin pasarse de tamaño. Si no lo
            // soporta, o si se eligió constante, CBR. (Con VBR sin tope, NVIDIA llegó al doble del bitrate pedido.)
            let variable = ajustes.control == ControlBitrate::Variable
                && poner(&CODECAPI_AVEncCommonRateControlMode, VARIANT::from(eAVEncCommonRateControlMode_PeakConstrainedVBR.0 as u32));
            if variable {
                poner(&CODECAPI_AVEncCommonMaxBitRate, VARIANT::from((bitrate as f32 * ajustes.pico) as u32));
            } else {
                poner(&CODECAPI_AVEncCommonRateControlMode, VARIANT::from(eAVEncCommonRateControlMode_CBR.0 as u32));
            }
            poner(&CODECAPI_AVEncCommonMeanBitRate, VARIANT::from(bitrate));
            // Un cuadro clave por segundo (o lo elegido): el clip empieza como mucho ese tiempo antes de lo pedido.
            poner(&CODECAPI_AVEncMPVGOPSize, VARIANT::from(ajustes.gop));
            if let Some(v) = ajustes.velocidad.valor() {
                poner(&CODECAPI_AVEncCommonQualityVsSpeed, VARIANT::from(v));
            }
            // Sin cuadros B: los tiempos de presentación y decodificación coinciden.
            poner(&CODECAPI_AVEncMPVDefaultBPictureCount, VARIANT::from(0u32));
            poner(&CODECAPI_AVLowLatencyMode, VARIANT::from(true));
        }

        let salida = MFCreateMediaType()?;
        salida.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        salida.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        salida.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
        salida.SetUINT64(&MF_MT_FRAME_SIZE, tamano(ancho, alto))?;
        salida.SetUINT64(&MF_MT_FRAME_RATE, tamano(fps, 1))?;
        salida.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, tamano(1, 1))?;
        salida.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        salida.SetUINT32(&MF_MT_MPEG2_PROFILE, perfil(ajustes.perfil))?;
        color_bt709(&salida)?;
        if mft.SetOutputType(0, &salida, 0).is_err() {
            salida.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Main.0 as u32)?;
            mft.SetOutputType(0, &salida, 0).with_context(|| format!("{nombre} no acepta {ancho}×{alto} a {fps} fps"))?;
        }

        let mut entrada = None;
        let mut i = 0;
        while let Ok(t) = mft.GetInputAvailableType(0, i) {
            i += 1;
            if t.GetGUID(&MF_MT_SUBTYPE).ok() == Some(MFVideoFormat_NV12) {
                entrada = Some(t);
                break;
            }
        }
        let entrada = match entrada {
            Some(t) => t,
            None => {
                let t = MFCreateMediaType()?;
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
                t
            }
        };
        entrada.SetUINT64(&MF_MT_FRAME_SIZE, tamano(ancho, alto))?;
        entrada.SetUINT64(&MF_MT_FRAME_RATE, tamano(fps, 1))?;
        entrada.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, tamano(1, 1))?;
        entrada.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        color_bt709(&entrada)?;
        mft.SetInputType(0, &entrada, 0).with_context(|| format!("{nombre} no acepta NV12 de {ancho}×{alto}"))?;

        mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
        mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        let eventos: IMFMediaEventGenerator = mft.cast()?;
        Ok(Self { mft, eventos, _administrador: administrador, nombre, ancho, alto, dur: SEGUNDO / fps as i64, pedidos: 0 })
    }

    /// Atiende los eventos pendientes del encoder y deja en `salida` los paquetes que estén listos.
    pub fn atender(&mut self, salida: &mut Vec<PaqueteVideo>) -> Result<()> {
        unsafe {
            loop {
                let ev = match self.eventos.GetEvent(MF_EVENT_FLAG_NO_WAIT) {
                    Ok(ev) => ev,
                    Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => return Ok(()),
                    Err(e) => return Err(e.into()),
                };
                let tipo = ev.GetType()?;
                if tipo == METransformNeedInput.0 as u32 {
                    self.pedidos += 1;
                } else if tipo == METransformHaveOutput.0 as u32 {
                    self.sacar(salida)?;
                }
            }
        }
    }

    /// ¿Está pidiendo un cuadro? Si no, el cuadro espera en la cola de la grabadora.
    pub fn puede_recibir(&self) -> bool {
        self.pedidos > 0
    }

    /// Le da un cuadro al encoder (solo si lo está pidiendo: ver `puede_recibir`).
    pub fn enviar(&mut self, textura: &ID3D11Texture2D, pts: i64) -> Result<()> {
        if self.pedidos == 0 {
            bail!("el encoder no pidió un cuadro");
        }
        unsafe {
            let buffer = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, textura, 0, false)?;
            if let Ok(b2) = buffer.cast::<IMF2DBuffer>() {
                if let Ok(largo) = b2.GetContiguousLength() {
                    buffer.SetCurrentLength(largo)?;
                }
            }
            let muestra = MFCreateSample()?;
            muestra.AddBuffer(&buffer)?;
            muestra.SetSampleTime(pts)?;
            muestra.SetSampleDuration(self.dur)?;
            self.mft.ProcessInput(0, &muestra, 0)?;
        }
        self.pedidos -= 1;
        Ok(())
    }

    unsafe fn sacar(&mut self, salida: &mut Vec<PaqueteVideo>) -> Result<()> {
        if let Salida::Paquete(p) = sacar_paquete(&self.mft, self.dur)? {
            salida.push(p);
        }
        Ok(())
    }
}

/// Lo que devuelve un encoder cuando se le pide salida.
pub(super) enum Salida {
    Paquete(PaqueteVideo),
    /// Cambió su formato de salida (ya se aceptó): hay que volver a pedir.
    Reintentar,
    /// No tiene nada más hasta recibir otro cuadro.
    NadaMas,
}

/// Saca un paquete del encoder (sirve para el de hardware y el de procesador). Los cuadros clave
/// salen siempre con SPS/PPS: si el encoder no los pone, se agregan desde su formato de salida, así
/// un clip puede empezar en cualquier cuadro clave.
pub(super) unsafe fn sacar_paquete(mft: &IMFTransform, dur_por_defecto: i64) -> Result<Salida> {
    let info = mft.GetOutputStreamInfo(0)?;
    let provee = info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32 != 0;
    let muestra = if provee {
        None
    } else {
        let m = MFCreateSample()?;
        m.AddBuffer(&MFCreateMemoryBuffer(info.cbSize.max(1024 * 1024))?)?;
        Some(m)
    };
    let mut datos = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: ManuallyDrop::new(muestra), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
    let mut estado = 0u32;
    let r = mft.ProcessOutput(0, &mut datos, &mut estado);
    let muestra = ManuallyDrop::take(&mut datos[0].pSample);
    ManuallyDrop::drop(&mut datos[0].pEvents);
    match r {
        Ok(()) => {}
        Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
            // El encoder cambió algo de su salida (suele pasar al principio): se acepta lo que propone.
            let t = mft.GetOutputAvailableType(0, 0)?;
            mft.SetOutputType(0, &t, 0)?;
            return Ok(Salida::Reintentar);
        }
        Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(Salida::NadaMas),
        Err(e) => return Err(e.into()),
    }
    let Some(m) = muestra else { return Ok(Salida::NadaMas) };
    let pts = m.GetSampleTime()?;
    let dur = m.GetSampleDuration().unwrap_or(dur_por_defecto);
    let clave = m.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0;
    let buffer = m.ConvertToContiguousBuffer()?;
    let mut p = std::ptr::null_mut();
    let mut largo = 0u32;
    buffer.Lock(&mut p, None, Some(&mut largo))?;
    let mut bytes = std::slice::from_raw_parts(p, largo as usize).to_vec();
    buffer.Unlock()?;
    if clave && !h264::tiene_encabezado(&bytes) {
        if let Some(mut h) = encabezado_del_formato(mft) {
            h.extend_from_slice(&bytes);
            bytes = h;
        }
    }
    Ok(Salida::Paquete(PaqueteVideo { pts, dur, clave, datos: std::sync::Arc::new(bytes) }))
}

/// SPS/PPS que el encoder anuncia en su formato de salida (si están en Annex B).
unsafe fn encabezado_del_formato(mft: &IMFTransform) -> Option<Vec<u8>> {
    let t = mft.GetOutputCurrentType(0).ok()?;
    let n = t.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER).ok()? as usize;
    let mut v = vec![0u8; n];
    t.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut v, None).ok()?;
    h264::es_annex_b(&v).then_some(v)
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            let _ = self.mft.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            if let Ok(s) = self.eventos.cast::<IMFShutdown>() {
                let _ = s.Shutdown();
            }
        }
    }
}

/// Colores BT.709 de rango limitado: lo que produce la conversión y lo que esperan los reproductores.
pub(super) unsafe fn color_bt709(t: &IMFMediaType) -> Result<()> {
    t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
    t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
    t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
    t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
    Ok(())
}

/// Encoders H.264 por hardware, primero los de la tarjeta `luid`.
unsafe fn encoders_de(luid: LUID) -> Result<Vec<IMFActivate>> {
    let tipo = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_H264 };
    let mut lista: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut cantidad = 0u32;
    let flags = MFT_ENUM_FLAG(MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SORTANDFILTER.0);
    MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, flags, None, Some(&tipo), &mut lista, &mut cantidad)?;
    let mut propios = Vec::new();
    let mut sin_dato = Vec::new();
    if !lista.is_null() {
        for a in std::slice::from_raw_parts_mut(lista, cantidad as usize) {
            if let Some(act) = a.take() {
                match luid_de(&act) {
                    Some(l) if l.LowPart == luid.LowPart && l.HighPart == luid.HighPart => propios.push(act),
                    Some(_) => {}
                    None => sin_dato.push(act),
                }
            }
        }
        CoTaskMemFree(Some(lista as *const _));
    }
    propios.extend(sin_dato);
    Ok(propios)
}
