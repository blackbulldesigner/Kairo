//! El mouse en los clips (opcional, Grabación → "Mostrar el mouse"). Desktop Duplication no lo dibuja en
//! la imagen: da la forma y la posición del puntero aparte. Acá se dibuja encima del cuadro, en la tarjeta
//! de video (un cuadradito con transparencia y dos shaders mínimos), sin traer nada al procesador.

use anyhow::{anyhow, Result};
use windows::core::{s, PCSTR};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{IDXGIOutputDuplication, DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTDUPL_POINTER_SHAPE_INFO};

/// Un cuadrado del tamaño del viewport con la textura del puntero.
const SHADERS: &str = r#"
struct S { float4 pos : SV_POSITION; float2 uv : TEXCOORD0; };
S vs(uint id : SV_VertexID) {
    S s;
    float2 uv = float2(id & 1, id >> 1);
    s.pos = float4(uv.x * 2 - 1, 1 - uv.y * 2, 0, 1);
    s.uv = uv;
    return s;
}
Texture2D t : register(t0);
SamplerState m : register(s0);
float4 ps(S s) : SV_TARGET { return t.Sample(m, s.uv); }
"#;

// Tipos de puntero de Desktop Duplication.
const MONOCROMO: u32 = 1;
const COLOR: u32 = 2;
const COLOR_CON_MASCARA: u32 = 4;

pub struct Mouse {
    dispositivo: ID3D11Device,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    mezcla: ID3D11BlendState,
    muestreo: ID3D11SamplerState,
    /// La forma actual del puntero y su tamaño.
    forma: Option<(ID3D11ShaderResourceView, u32, u32)>,
    visible: bool,
    x: i32,
    y: i32,
    buffer: Vec<u8>,
}

unsafe fn compilar(entrada: PCSTR, perfil: PCSTR) -> Result<Vec<u8>> {
    let mut codigo: Option<ID3DBlob> = None;
    let mut errores: Option<ID3DBlob> = None;
    let r = D3DCompile(SHADERS.as_ptr() as *const _, SHADERS.len(), PCSTR::null(), None, None, entrada, perfil, 0, 0, &mut codigo, Some(&mut errores));
    if let Err(e) = r {
        let detalle = errores
            .map(|b| String::from_utf8_lossy(std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize())).into_owned())
            .unwrap_or_default();
        return Err(anyhow!("no se pudo compilar el shader del mouse: {e} {detalle}"));
    }
    let b = codigo.ok_or_else(|| anyhow!("shader vacío"))?;
    Ok(std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()).to_vec())
}

/// La forma del puntero a BGRA con transparencia. Los píxeles que "invierten" la pantalla (el cursor de
/// texto) no se pueden hacer sin leer la pantalla: se dibujan en negro.
fn a_bgra(info: &DXGI_OUTDUPL_POINTER_SHAPE_INFO, datos: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let (w, pitch) = (info.Width as usize, info.Pitch as usize);
    match info.Type {
        MONOCROMO => {
            // Dos máscaras de 1 bit por píxel, una debajo de la otra: AND y XOR.
            let h = info.Height as usize / 2;
            let mut px = vec![0u8; w * h * 4];
            for y in 0..h {
                for x in 0..w {
                    let bit = 0x80u8 >> (x % 8);
                    let y_and = *datos.get(y * pitch + x / 8)? & bit != 0;
                    let y_xor = *datos.get((y + h) * pitch + x / 8)? & bit != 0;
                    let c: [u8; 4] = match (y_and, y_xor) {
                        (false, false) => [0, 0, 0, 255],
                        (false, true) => [255, 255, 255, 255],
                        (true, false) => [0, 0, 0, 0],
                        (true, true) => [0, 0, 0, 255],
                    };
                    px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&c);
                }
            }
            Some((w as u32, h as u32, px))
        }
        COLOR | COLOR_CON_MASCARA => {
            let h = info.Height as usize;
            let mut px = vec![0u8; w * h * 4];
            for y in 0..h {
                let fila = datos.get(y * pitch..y * pitch + w * 4)?;
                for x in 0..w {
                    let mut c = [fila[x * 4], fila[x * 4 + 1], fila[x * 4 + 2], fila[x * 4 + 3]];
                    if info.Type == COLOR_CON_MASCARA {
                        // Alfa 0: el color va tal cual. Alfa 255: XOR con la pantalla (0 = no cambia nada).
                        c = if c[3] == 0 {
                            [c[0], c[1], c[2], 255]
                        } else if c[0] | c[1] | c[2] == 0 {
                            [0, 0, 0, 0]
                        } else {
                            [c[0], c[1], c[2], 255]
                        };
                    }
                    px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&c);
                }
            }
            Some((w as u32, h as u32, px))
        }
        _ => None,
    }
}

impl Mouse {
    pub unsafe fn nuevo(d: &ID3D11Device) -> Result<Self> {
        let codigo_vs = compilar(s!("vs"), s!("vs_4_0"))?;
        let codigo_ps = compilar(s!("ps"), s!("ps_4_0"))?;
        let mut vs = None;
        d.CreateVertexShader(&codigo_vs, None, Some(&mut vs))?;
        let mut ps = None;
        d.CreatePixelShader(&codigo_ps, None, Some(&mut ps))?;
        let mut desc = D3D11_BLEND_DESC::default();
        desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            SrcBlend: D3D11_BLEND_SRC_ALPHA,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let mut mezcla = None;
        d.CreateBlendState(&desc, Some(&mut mezcla))?;
        let muestreo_desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            ComparisonFunc: D3D11_COMPARISON_NEVER,
            MaxLOD: f32::MAX,
            ..Default::default()
        };
        let mut muestreo = None;
        d.CreateSamplerState(&muestreo_desc, Some(&mut muestreo))?;
        Ok(Self {
            dispositivo: d.clone(),
            vs: vs.ok_or_else(|| anyhow!("sin vertex shader"))?,
            ps: ps.ok_or_else(|| anyhow!("sin pixel shader"))?,
            mezcla: mezcla.ok_or_else(|| anyhow!("sin blend state"))?,
            muestreo: muestreo.ok_or_else(|| anyhow!("sin sampler"))?,
            forma: None,
            visible: false,
            x: 0,
            y: 0,
            buffer: Vec::new(),
        })
    }

    /// ¿Hay que dibujarlo en este cuadro?
    pub fn se_ve(&self) -> bool {
        self.visible && self.forma.is_some()
    }

    /// Lee lo nuevo del puntero (posición y, si cambió, la forma). Va antes de soltar el cuadro.
    pub unsafe fn actualizar(&mut self, dup: &IDXGIOutputDuplication, info: &DXGI_OUTDUPL_FRAME_INFO) {
        if info.LastMouseUpdateTime != 0 {
            self.visible = info.PointerPosition.Visible.as_bool();
            self.x = info.PointerPosition.Position.x;
            self.y = info.PointerPosition.Position.y;
        }
        if info.PointerShapeBufferSize == 0 {
            return;
        }
        self.buffer.resize(info.PointerShapeBufferSize as usize, 0);
        let (mut necesario, mut forma) = (0u32, DXGI_OUTDUPL_POINTER_SHAPE_INFO::default());
        if dup.GetFramePointerShape(self.buffer.len() as u32, self.buffer.as_mut_ptr() as *mut _, &mut necesario, &mut forma).is_err() {
            return;
        }
        let Some((w, h, px)) = a_bgra(&forma, &self.buffer) else { return };
        if w == 0 || h == 0 {
            return;
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let datos = D3D11_SUBRESOURCE_DATA { pSysMem: px.as_ptr() as *const _, SysMemPitch: w * 4, SysMemSlicePitch: 0 };
        let mut tex = None;
        if self.dispositivo.CreateTexture2D(&desc, Some(&datos), Some(&mut tex)).is_err() {
            return;
        }
        let Some(tex) = tex else { return };
        let mut vista = None;
        if self.dispositivo.CreateShaderResourceView(&tex, None, Some(&mut vista)).is_ok() {
            self.forma = vista.map(|v| (v, w, h));
        }
    }

    /// Dibuja el puntero sobre `destino` (el cuadro del escritorio, en BGRA).
    pub unsafe fn dibujar(&self, ctx: &ID3D11DeviceContext, destino: &ID3D11RenderTargetView) {
        let Some((vista, w, h)) = &self.forma else { return };
        let viewport = D3D11_VIEWPORT { TopLeftX: self.x as f32, TopLeftY: self.y as f32, Width: *w as f32, Height: *h as f32, MinDepth: 0.0, MaxDepth: 1.0 };
        ctx.OMSetRenderTargets(Some(&[Some(destino.clone())]), None);
        ctx.RSSetViewports(Some(&[viewport]));
        ctx.IASetInputLayout(None);
        ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        ctx.VSSetShader(&self.vs, None);
        ctx.PSSetShader(&self.ps, None);
        ctx.PSSetShaderResources(0, Some(&[Some(vista.clone())]));
        ctx.PSSetSamplers(0, Some(&[Some(self.muestreo.clone())]));
        ctx.OMSetBlendState(&self.mezcla, None, 0xFFFF_FFFF);
        ctx.Draw(4, 0);
        // Nada queda enganchado al contexto (lo usa también el procesador de video).
        ctx.OMSetRenderTargets(None, None);
        ctx.PSSetShaderResources(0, Some(&[None]));
    }
}
