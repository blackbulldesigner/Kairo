//! Captura del monitor con Desktop Duplication (DXGI) y conversión a NV12 en la tarjeta de video
//! (procesador de video de D3D11). La imagen nunca pasa por el procesador.

use std::mem::ManuallyDrop;

use anyhow::{anyhow, bail, Context, Result};
use windows::core::Interface;
use windows::Win32::Foundation::{HMODULE, LUID};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

/// Cuántas texturas NV12 rotan: el encoder trabaja en paralelo y puede tener varias en uso.
const TEXTURAS: usize = 8;

/// El monitor a grabar y la tarjeta que lo maneja.
pub struct Monitor {
    pub adaptador: IDXGIAdapter1,
    pub salida: IDXGIOutput1,
    pub luid: LUID,
    pub id: String,
}

fn texto_utf16(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// Busca el monitor por su nombre de Windows (`\\.\DISPLAY1`). Sin nombre, o si ya no existe, el principal.
pub fn buscar_monitor(id: Option<&str>) -> Result<Monitor> {
    unsafe {
        let fabrica: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut principal = None;
        let mut i = 0;
        while let Ok(adaptador) = fabrica.EnumAdapters1(i) {
            i += 1;
            let d = adaptador.GetDesc1()?;
            let mut j = 0;
            while let Ok(salida) = adaptador.EnumOutputs(j) {
                j += 1;
                let od = salida.GetDesc()?;
                if !od.AttachedToDesktop.as_bool() {
                    continue;
                }
                let r = od.DesktopCoordinates;
                let m = Monitor { adaptador: adaptador.clone(), salida: salida.cast()?, luid: d.AdapterLuid, id: texto_utf16(&od.DeviceName) };
                if id == Some(m.id.as_str()) {
                    return Ok(m);
                }
                if r.left == 0 && r.top == 0 && principal.is_none() {
                    principal = Some(m);
                }
            }
        }
        principal.ok_or_else(|| anyhow!("no se encontró ningún monitor"))
    }
}

pub struct Captura {
    pub dispositivo: ID3D11Device,
    pub luid: LUID,
    pub ancho_monitor: u32,
    pub alto_monitor: u32,
    pub ancho: u32,
    pub alto: u32,
    contexto: ID3D11DeviceContext,
    salida: IDXGIOutput1,
    duplicacion: Option<IDXGIOutputDuplication>,
    /// Copia del último cuadro del escritorio (BGRA).
    ultimo: ID3D11Texture2D,
    hay_cuadro: bool,
    video_contexto: ID3D11VideoContext,
    procesador: ID3D11VideoProcessor,
    entrada: ID3D11VideoProcessorInputView,
    texturas: Vec<(ID3D11Texture2D, ID3D11VideoProcessorOutputView)>,
    siguiente: usize,
}

/// El error que devuelve DXGI cuando se pierde la captura (cambio de resolución, pantalla de UAC,
/// un juego que entra o sale de pantalla completa exclusiva). Se arregla volviendo a empezar.
pub fn es_captura_perdida(e: &anyhow::Error) -> bool {
    e.downcast_ref::<windows::core::Error>().is_some_and(|w| w.code() == DXGI_ERROR_ACCESS_LOST)
}

impl Captura {
    /// Prepara la captura del monitor y la conversión a NV12 de `ancho`×`alto` (sin deformar).
    pub fn abrir(m: &Monitor, tamano: impl Fn(u32, u32) -> (u32, u32), fps: u32) -> Result<Self> {
        unsafe {
            let mut dispositivo = None;
            let mut contexto = None;
            D3D11CreateDevice(
                &m.adaptador,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0]),
                D3D11_SDK_VERSION,
                Some(&mut dispositivo),
                None,
                Some(&mut contexto),
            )
            .context("no se pudo usar la tarjeta de video")?;
            let dispositivo: ID3D11Device = dispositivo.ok_or_else(|| anyhow!("sin dispositivo D3D11"))?;
            let contexto: ID3D11DeviceContext = contexto.ok_or_else(|| anyhow!("sin contexto D3D11"))?;
            // El encoder usa el mismo dispositivo desde sus propios hilos.
            if let Ok(mt) = dispositivo.cast::<ID3D11Multithread>() {
                let _ = mt.SetMultithreadProtected(true);
            }

            let duplicacion = duplicar(&m.salida, &dispositivo)?;
            let d = duplicacion.GetDesc();
            let (ancho_monitor, alto_monitor) = (d.ModeDesc.Width, d.ModeDesc.Height);
            let (ancho, alto) = tamano(ancho_monitor, alto_monitor);

            let ultimo = textura(
                &dispositivo,
                ancho_monitor,
                alto_monitor,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                D3D11_BIND_RENDER_TARGET.0 as u32 | D3D11_BIND_SHADER_RESOURCE.0 as u32,
            )?;

            let video_dispositivo: ID3D11VideoDevice = dispositivo.cast()?;
            let video_contexto: ID3D11VideoContext = contexto.cast()?;
            let fr = DXGI_RATIONAL { Numerator: fps, Denominator: 1 };
            let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: fr,
                InputWidth: ancho_monitor,
                InputHeight: alto_monitor,
                OutputFrameRate: fr,
                OutputWidth: ancho,
                OutputHeight: alto,
                Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
            };
            let enumerador = video_dispositivo.CreateVideoProcessorEnumerator(&desc)?;
            let procesador = video_dispositivo.CreateVideoProcessor(&enumerador, 0)?;

            let mut entrada = None;
            let vista_entrada = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: 0 } },
            };
            video_dispositivo.CreateVideoProcessorInputView(&ultimo, &enumerador, &vista_entrada, Some(&mut entrada))?;
            let entrada = entrada.ok_or_else(|| anyhow!("sin vista de entrada"))?;

            let mut texturas = Vec::with_capacity(TEXTURAS);
            let vista_salida = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } },
            };
            for _ in 0..TEXTURAS {
                let t = textura(&dispositivo, ancho, alto, DXGI_FORMAT_NV12, D3D11_BIND_RENDER_TARGET.0 as u32)?;
                let mut v = None;
                video_dispositivo.CreateVideoProcessorOutputView(&t, &enumerador, &vista_salida, Some(&mut v))?;
                texturas.push((t, v.ok_or_else(|| anyhow!("sin vista de salida"))?));
            }

            // Entrada: RGB de rango completo. Salida: BT.709 de rango limitado (lo que esperan los reproductores).
            video_contexto.VideoProcessorSetStreamColorSpace(&procesador, 0, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 });
            video_contexto.VideoProcessorSetOutputColorSpace(&procesador, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: (1 << 2) | (1 << 4) });
            video_contexto.VideoProcessorSetStreamFrameFormat(&procesador, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
            video_contexto.VideoProcessorSetStreamAutoProcessingMode(&procesador, 0, false);

            Ok(Self {
                dispositivo,
                luid: m.luid,
                ancho_monitor,
                alto_monitor,
                ancho,
                alto,
                contexto,
                salida: m.salida.clone(),
                duplicacion: Some(duplicacion),
                ultimo,
                hay_cuadro: false,
                video_contexto,
                procesador,
                entrada,
                texturas,
                siguiente: 0,
            })
        }
    }

    pub fn hay_cuadro(&self) -> bool {
        self.hay_cuadro
    }

    /// Toma el cuadro nuevo del escritorio, si hay. Si no cambió nada, se sigue usando el anterior.
    pub fn capturar(&mut self) -> Result<()> {
        unsafe {
            let dup = match &self.duplicacion {
                Some(d) => d.clone(),
                None => {
                    let d = duplicar(&self.salida, &self.dispositivo)?;
                    self.duplicacion = Some(d.clone());
                    d
                }
            };
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut recurso = None;
            match dup.AcquireNextFrame(0, &mut info, &mut recurso) {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(()),
                Err(e) => {
                    self.duplicacion = None;
                    return Err(e.into());
                }
            }
            // LastPresentTime en 0 = solo se movió el mouse: la imagen es la misma.
            let r = (|| -> Result<()> {
                if info.LastPresentTime != 0 || !self.hay_cuadro {
                    let tex: ID3D11Texture2D = recurso.ok_or_else(|| anyhow!("cuadro vacío"))?.cast()?;
                    self.contexto.CopyResource(&self.ultimo, &tex);
                    self.hay_cuadro = true;
                }
                Ok(())
            })();
            let _ = dup.ReleaseFrame();
            r
        }
    }

    /// Convierte el último cuadro a NV12 (y al tamaño del clip) en una de las texturas que rotan.
    pub fn convertir(&mut self) -> Result<ID3D11Texture2D> {
        let (tex, vista) = &self.texturas[self.siguiente];
        self.siguiente = (self.siguiente + 1) % self.texturas.len();
        unsafe {
            let mut flujo = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: ManuallyDrop::new(Some(self.entrada.clone())),
                ..Default::default()
            };
            let r = self.video_contexto.VideoProcessorBlt(&self.procesador, vista, 0, std::slice::from_ref(&flujo));
            ManuallyDrop::drop(&mut flujo.pInputSurface);
            r?;
        }
        Ok(tex.clone())
    }
}

unsafe fn duplicar(salida: &IDXGIOutput1, dispositivo: &ID3D11Device) -> Result<IDXGIOutputDuplication> {
    match salida.DuplicateOutput(dispositivo) {
        Ok(d) => Ok(d),
        Err(e) if e.code() == DXGI_ERROR_UNSUPPORTED => bail!(
            "Windows no deja capturar este monitor desde esta tarjeta de video. Si es una laptop con dos tarjetas, \
             abre Configuración → Sistema → Pantalla → Gráficos y pon Kairo en «Ahorro de energía»."
        ),
        Err(e) if e.code() == DXGI_ERROR_NOT_CURRENTLY_AVAILABLE => {
            bail!("Demasiados programas están capturando la pantalla a la vez. Cierra alguno y vuelve a intentar.")
        }
        Err(e) => Err(anyhow::Error::from(e).context("no se pudo empezar a capturar la pantalla")),
    }
}

unsafe fn textura(d: &ID3D11Device, ancho: u32, alto: u32, formato: DXGI_FORMAT, bind: u32) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: ancho,
        Height: alto,
        MipLevels: 1,
        ArraySize: 1,
        Format: formato,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut t = None;
    d.CreateTexture2D(&desc, None, Some(&mut t))?;
    t.ok_or_else(|| anyhow!("no se pudo crear una textura de {ancho}×{alto}"))
}
