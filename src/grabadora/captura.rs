//! Captura del monitor y conversión a NV12 en la tarjeta de video (procesador de video de D3D11):
//! la imagen nunca pasa por el procesador.
//!
//! Hay dos formas de capturar:
//! - **Desktop Duplication** (DXGI): la más liviana, pero solo funciona desde la tarjeta que maneja el
//!   monitor. En laptops con dos tarjetas, si Kairo está forzado a la dedicada, Windows la rechaza.
//! - **Windows Graphics Capture** (respaldo): funciona desde cualquier tarjeta (Windows hace la copia
//!   entre tarjetas). Necesita Windows 10 1903 o más nuevo; en Windows 10 dibuja un borde amarillo
//!   alrededor del monitor mientras graba (en la pantalla, no en el video).

use std::mem::ManuallyDrop;

use anyhow::{anyhow, Context, Result};
use windows::core::Interface;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, LUID};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use crate::config::TarjetaGrabacion;
use crate::registro::log;

/// Cuántas texturas NV12 rotan: la cola de la grabadora y el encoder pueden tener varias en uso.
const TEXTURAS: usize = 10;

/// El monitor a grabar y la tarjeta que lo maneja.
pub struct Monitor {
    pub adaptador: IDXGIAdapter1,
    pub salida: IDXGIOutput1,
    pub luid: LUID,
    pub id: String,
    pub hmonitor: HMONITOR,
}

fn texto_utf16(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

fn mismo_luid(a: LUID, b: LUID) -> bool {
    a.LowPart == b.LowPart && a.HighPart == b.HighPart
}

/// Busca el monitor por su nombre de Windows (`\\.\DISPLAY1`). Sin nombre, o si ya no existe, el principal.
pub fn buscar_monitor(id: Option<&str>) -> Result<Monitor> {
    buscar(|m| id == Some(m.id.as_str()))
}

/// El monitor con ese HMONITOR (el del juego). Si ya no existe, el principal.
pub fn buscar_monitor_por_handle(h: isize) -> Result<Monitor> {
    buscar(|m| m.hmonitor.0 as isize == h)
}

fn buscar(es: impl Fn(&Monitor) -> bool) -> Result<Monitor> {
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
                let m = Monitor {
                    adaptador: adaptador.clone(),
                    salida: salida.cast()?,
                    luid: d.AdapterLuid,
                    id: texto_utf16(&od.DeviceName),
                    hmonitor: od.Monitor,
                };
                if es(&m) {
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

/// La tarjeta con la que se graba. En automático, la que maneja el monitor (así sirve Desktop
/// Duplication y no hay copias entre tarjetas). Integrada/dedicada se piden a Windows por preferencia
/// de consumo; si no hay dos tarjetas, queda la del monitor.
pub fn tarjeta_para(eleccion: TarjetaGrabacion, m: &Monitor) -> IDXGIAdapter1 {
    let preferencia = match eleccion {
        TarjetaGrabacion::Automatica => return m.adaptador.clone(),
        TarjetaGrabacion::Integrada => DXGI_GPU_PREFERENCE_MINIMUM_POWER,
        TarjetaGrabacion::Dedicada => DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
    };
    unsafe {
        CreateDXGIFactory1::<IDXGIFactory6>()
            .ok()
            .and_then(|f| f.EnumAdapterByGpuPreference::<IDXGIAdapter1>(0, preferencia).ok())
            .filter(|a| a.GetDesc1().is_ok_and(|d| d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0 && d.VendorId != 0x1414))
            .unwrap_or_else(|| m.adaptador.clone())
    }
}

/// La captura se perdió (cambio de resolución, pantalla de UAC, un juego que entra o sale de pantalla
/// completa exclusiva). Se arregla volviendo a empezar, sin avisarle a nadie.
#[derive(Debug)]
pub struct CapturaPerdida;

impl std::fmt::Display for CapturaPerdida {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("se perdió la captura")
    }
}

impl std::error::Error for CapturaPerdida {}

pub fn es_captura_perdida(e: &anyhow::Error) -> bool {
    e.downcast_ref::<CapturaPerdida>().is_some() || e.downcast_ref::<windows::core::Error>().is_some_and(|w| w.code() == DXGI_ERROR_ACCESS_LOST)
}

/// Windows Graphics Capture del monitor.
struct Respaldo {
    _item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    sesion: GraphicsCaptureSession,
    tamano: SizeInt32,
}

impl Respaldo {
    fn abrir(dispositivo: &ID3D11Device, monitor: HMONITOR, mouse: bool) -> Result<Self> {
        if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
            return Err(anyhow!("esta versión de Windows no tiene Windows Graphics Capture"));
        }
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForMonitor(monitor) }.context("Windows Graphics Capture no acepta el monitor")?;
        let dxgi: IDXGIDevice = dispositivo.cast()?;
        let d3d: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
        let tamano = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&d3d, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, tamano)?;
        let sesion = pool.CreateCaptureSession(&item)?;
        // El mouse, solo si se pidió. Y sin el borde amarillo (solo se puede en Windows 11).
        let _ = sesion.SetIsCursorCaptureEnabled(mouse);
        let _ = sesion.SetIsBorderRequired(false);
        sesion.StartCapture()?;
        Ok(Self { _item: item, pool, sesion, tamano })
    }
}

impl Drop for Respaldo {
    fn drop(&mut self) {
        let _ = self.sesion.Close();
        let _ = self.pool.Close();
    }
}

enum Origen {
    Duplicacion { salida: IDXGIOutput1, dup: Option<IDXGIOutputDuplication> },
    Respaldo(Respaldo),
}

pub struct Captura {
    pub dispositivo: ID3D11Device,
    pub luid: LUID,
    pub nombre_tarjeta: String,
    /// "Desktop Duplication" o "captura de respaldo".
    pub metodo: &'static str,
    pub ancho_monitor: u32,
    pub alto_monitor: u32,
    pub ancho: u32,
    pub alto: u32,
    contexto: ID3D11DeviceContext,
    origen: Origen,
    /// Copia del último cuadro del escritorio (BGRA).
    ultimo: ID3D11Texture2D,
    hay_cuadro: bool,
    video_contexto: ID3D11VideoContext,
    procesador: ID3D11VideoProcessor,
    entrada: ID3D11VideoProcessorInputView,
    /// Cuadro negro del tamaño del monitor ("Solo el juego" cuando el juego no está adelante).
    negro: ID3D11VideoProcessorInputView,
    texturas: Vec<(ID3D11Texture2D, ID3D11VideoProcessorOutputView)>,
    siguiente: usize,
    /// "Mostrar el mouse" con Desktop Duplication: el puntero y la copia del cuadro donde se dibuja.
    mouse: Option<super::mouse::Mouse>,
    con_mouse: Option<(ID3D11Texture2D, ID3D11RenderTargetView, ID3D11VideoProcessorInputView)>,
}

impl Captura {
    /// Prepara la captura del monitor `m` desde la tarjeta `tarjeta`, y la conversión a NV12 del
    /// tamaño que diga `tamano` (a partir del tamaño del monitor). Usa Desktop Duplication si se puede
    /// y, si no, la captura de respaldo.
    pub fn abrir(
        m: &Monitor,
        tarjeta: &IDXGIAdapter1,
        forzar_respaldo: bool,
        tamano: impl Fn(u32, u32) -> (u32, u32),
        fps: u32,
        mouse: bool,
    ) -> Result<Self> {
        unsafe {
            let desc_tarjeta = tarjeta.GetDesc1()?;
            let nombre_tarjeta = texto_utf16(&desc_tarjeta.Description);
            let mut dispositivo = None;
            let mut contexto = None;
            D3D11CreateDevice(
                tarjeta,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0]),
                D3D11_SDK_VERSION,
                Some(&mut dispositivo),
                None,
                Some(&mut contexto),
            )
            .with_context(|| format!("no se pudo usar la tarjeta {nombre_tarjeta}"))?;
            let dispositivo: ID3D11Device = dispositivo.ok_or_else(|| anyhow!("sin dispositivo D3D11"))?;
            let contexto: ID3D11DeviceContext = contexto.ok_or_else(|| anyhow!("sin contexto D3D11"))?;
            // El encoder usa el mismo dispositivo desde sus propios hilos.
            if let Ok(mt) = dispositivo.cast::<ID3D11Multithread>() {
                let _ = mt.SetMultithreadProtected(true);
            }

            // Desktop Duplication solo sirve desde la tarjeta que maneja el monitor.
            let mismo = mismo_luid(desc_tarjeta.AdapterLuid, m.luid);
            let dup = if mismo && !forzar_respaldo {
                match m.salida.DuplicateOutput(&dispositivo) {
                    Ok(d) => Some(d),
                    Err(e) if e.code() == DXGI_ERROR_UNSUPPORTED => {
                        log("Desktop Duplication no funciona desde esta tarjeta (laptop con dos tarjetas): se usa la captura de respaldo");
                        None
                    }
                    Err(e) if e.code() == DXGI_ERROR_NOT_CURRENTLY_AVAILABLE => {
                        return Err(anyhow!("Demasiados programas están capturando la pantalla a la vez. Cierra alguno y vuelve a intentar."));
                    }
                    Err(e) => return Err(anyhow::Error::from(e).context("no se pudo empezar a capturar la pantalla")),
                }
            } else {
                None
            };
            let (origen, metodo, ancho_monitor, alto_monitor) = match dup {
                Some(d) => {
                    let md = d.GetDesc().ModeDesc;
                    (Origen::Duplicacion { salida: m.salida.clone(), dup: Some(d) }, "Desktop Duplication", md.Width, md.Height)
                }
                None => {
                    let r = Respaldo::abrir(&dispositivo, m.hmonitor, mouse).map_err(|e| {
                        anyhow!(
                            "Windows no deja capturar este monitor desde esta tarjeta y la captura de respaldo no funcionó ({e:#}). \
                             Si es una laptop con dos tarjetas, abre Configuración → Sistema → Pantalla → Gráficos y pon Kairo en «Ahorro de energía»."
                        )
                    })?;
                    let (w, h) = (r.tamano.Width as u32, r.tamano.Height as u32);
                    (Origen::Respaldo(r), "captura de respaldo", w, h)
                }
            };
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

            // Negro opaco (BGRA 0,0,0,255), una sola vez.
            let pixeles: Vec<u8> = (0..ancho_monitor as usize * alto_monitor as usize).flat_map(|_| [0u8, 0, 0, 255]).collect();
            let datos = D3D11_SUBRESOURCE_DATA { pSysMem: pixeles.as_ptr() as *const _, SysMemPitch: ancho_monitor * 4, SysMemSlicePitch: 0 };
            let desc_negro = D3D11_TEXTURE2D_DESC {
                Width: ancho_monitor,
                Height: alto_monitor,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                // Igual que la del último cuadro: el procesador de video no acepta texturas inmutables.
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32 | D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut tex_negra = None;
            dispositivo.CreateTexture2D(&desc_negro, Some(&datos), Some(&mut tex_negra)).context("no se pudo crear el cuadro negro")?;
            let tex_negra = tex_negra.ok_or_else(|| anyhow!("sin textura negra"))?;
            let mut negro = None;
            video_dispositivo
                .CreateVideoProcessorInputView(&tex_negra, &enumerador, &vista_entrada, Some(&mut negro))
                .context("el procesador de video no acepta el cuadro negro")?;
            let negro = negro.ok_or_else(|| anyhow!("sin vista del cuadro negro"))?;

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

            // El mouse (con Desktop Duplication; la captura de respaldo lo dibuja sola). Si algo falla, se graba sin él.
            let (mouse, con_mouse) = if mouse && matches!(origen, Origen::Duplicacion { .. }) {
                let preparar = || -> Result<_> {
                    let m = super::mouse::Mouse::nuevo(&dispositivo)?;
                    let t = textura(
                        &dispositivo,
                        ancho_monitor,
                        alto_monitor,
                        DXGI_FORMAT_B8G8R8A8_UNORM,
                        D3D11_BIND_RENDER_TARGET.0 as u32 | D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    )?;
                    let mut rtv = None;
                    dispositivo.CreateRenderTargetView(&t, None, Some(&mut rtv))?;
                    let mut vista = None;
                    video_dispositivo.CreateVideoProcessorInputView(&t, &enumerador, &vista_entrada, Some(&mut vista))?;
                    let (rtv, vista) = (rtv.ok_or_else(|| anyhow!("sin vista"))?, vista.ok_or_else(|| anyhow!("sin vista"))?);
                    Ok((m, (t, rtv, vista)))
                };
                match preparar() {
                    Ok((m, c)) => (Some(m), Some(c)),
                    Err(e) => {
                        log(&format!("no se puede dibujar el mouse: {e:#}"));
                        (None, None)
                    }
                }
            } else {
                (None, None)
            };

            Ok(Self {
                dispositivo,
                luid: desc_tarjeta.AdapterLuid,
                nombre_tarjeta,
                metodo,
                ancho_monitor,
                alto_monitor,
                ancho,
                alto,
                contexto,
                origen,
                ultimo,
                hay_cuadro: false,
                video_contexto,
                procesador,
                entrada,
                negro,
                texturas,
                siguiente: 0,
                mouse,
                con_mouse,
            })
        }
    }

    pub fn hay_cuadro(&self) -> bool {
        self.hay_cuadro
    }

    /// Toma el cuadro nuevo del escritorio, si hay. Si no cambió nada, se sigue usando el anterior.
    pub fn capturar(&mut self) -> Result<()> {
        match &mut self.origen {
            Origen::Duplicacion { salida, dup } => unsafe {
                let d = match dup {
                    Some(d) => d.clone(),
                    None => {
                        let d = salida.DuplicateOutput(&self.dispositivo)?;
                        *dup = Some(d.clone());
                        d
                    }
                };
                let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
                let mut recurso = None;
                match d.AcquireNextFrame(0, &mut info, &mut recurso) {
                    Ok(()) => {
                        if let Some(m) = self.mouse.as_mut() {
                            m.actualizar(&d, &info);
                        }
                    }
                    Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(()),
                    Err(e) => {
                        *dup = None;
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
                let _ = d.ReleaseFrame();
                r
            },
            Origen::Respaldo(r) => {
                // Puede haber más de un cuadro esperando: se usa el más nuevo.
                let mut nuevo = None;
                while let Ok(c) = r.pool.TryGetNextFrame() {
                    nuevo = Some(c);
                }
                let Some(cuadro) = nuevo else { return Ok(()) };
                let tam = cuadro.ContentSize()?;
                if tam.Width != r.tamano.Width || tam.Height != r.tamano.Height {
                    return Err(CapturaPerdida.into()); // cambió la resolución
                }
                let tex: ID3D11Texture2D = unsafe { cuadro.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()? };
                let caja = D3D11_BOX { left: 0, top: 0, front: 0, right: self.ancho_monitor, bottom: self.alto_monitor, back: 1 };
                unsafe { self.contexto.CopySubresourceRegion(&self.ultimo, 0, 0, 0, 0, &tex, 0, Some(&caja)) };
                self.hay_cuadro = true;
                let _ = cuadro.Close();
                Ok(())
            }
        }
    }

    /// Convierte el último cuadro a NV12 (y al tamaño del clip) en una de las texturas que rotan.
    pub fn convertir(&mut self) -> Result<ID3D11Texture2D> {
        // Con el mouse a la vista: el cuadro se copia y el puntero se dibuja encima de la copia.
        if let (Some(m), Some((tex, rtv, vista))) = (&self.mouse, &self.con_mouse) {
            if m.se_ve() {
                unsafe {
                    self.contexto.CopyResource(tex, &self.ultimo);
                    m.dibujar(&self.contexto, rtv);
                }
                let vista = vista.clone();
                return self.procesar(vista);
            }
        }
        let entrada = self.entrada.clone();
        self.procesar(entrada)
    }

    /// Un cuadro negro en NV12 ("Solo el juego" con el juego minimizado o detrás de otra ventana).
    pub fn negro(&mut self) -> Result<ID3D11Texture2D> {
        let entrada = self.negro.clone();
        self.procesar(entrada)
    }

    fn procesar(&mut self, entrada: ID3D11VideoProcessorInputView) -> Result<ID3D11Texture2D> {
        let (tex, vista) = &self.texturas[self.siguiente];
        self.siguiente = (self.siguiente + 1) % self.texturas.len();
        unsafe {
            let mut flujo = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: ManuallyDrop::new(Some(entrada)),
                ..Default::default()
            };
            let r = self.video_contexto.VideoProcessorBlt(&self.procesador, vista, 0, std::slice::from_ref(&flujo));
            ManuallyDrop::drop(&mut flujo.pInputSurface);
            r?;
        }
        Ok(tex.clone())
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
