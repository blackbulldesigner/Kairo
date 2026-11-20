//! Prueba de FPS sin jugar (solo en desarrollo): un "juego de prueba" que dibuja siempre la misma escena pesada y
//! en movimiento (un túnel de luces), sin límite de FPS y a pantalla completa, y mide cada cuadro. Mientras tanto
//! anota cuánto procesador, tarjeta de video, encoder y RAM usa cada grabador (Kairo y Medal). La misma escena en
//! cada ronda hace la comparación pareja: `scripts\comparar-fps.ps1` la corre sin nada, con Kairo y con Medal.
//!
//! - `--prueba-fps calibrar`: busca la carga para que la escena ande a ~200 FPS sin grabar (imprime `carga N`).
//! - `--prueba-fps medir <segundos> <carga> <archivo.json>`: 8 s para calentar, mide y escribe el resultado.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::Serialize;
use windows::core::{s, w, Interface, PCSTR, PWSTR};
use windows::Win32::Foundation::{BOOL, CloseHandle, FILETIME, HMODULE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_DRIVER_TYPE_HARDWARE};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_ALPHA_MODE_UNSPECIFIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W,
    PDH_FMT_DOUBLE, PDH_MORE_DATA,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::*;

/// FPS que busca la calibración (sin grabar): como un juego exigente en un monitor de alta frecuencia.
const FPS_OBJETIVO: f64 = 200.0;
/// Segundos para calentar (la tarjeta sube de frecuencia y el medidor arranca) antes de medir.
const CALENTAR: f64 = 8.0;

const SHADERS: &str = r#"
cbuffer Datos : register(b0) { float tiempo; float carga; float2 tam; float progreso; float3 relleno; };

float4 vs(uint id : SV_VertexID) : SV_Position {
    float2 p = float2((id << 1) & 2, id & 2);
    return float4(p * float2(2, -2) + float2(-1, 1), 0, 1);
}

float mapa(float3 p) {
    float3 q = p;
    q.xy = abs(fmod(q.xy + 64.0, 4.0) - 2.0);
    q.z = fmod(q.z + 64.0, 3.0) - 1.5;
    float esfera = length(q) - (0.45 + 0.15 * sin(p.z * 0.7 + tiempo * 2.0));
    float tunel = 3.2 - length(p.xy);
    return min(esfera, tunel);
}

float4 ps(float4 pos : SV_Position) : SV_Target {
    float2 uv = (pos.xy - 0.5 * tam) / tam.y;
    float a = tiempo * 0.35;
    float2x2 giro = float2x2(cos(a), -sin(a), sin(a), cos(a));
    float3 origen = float3(0.0, 0.0, tiempo * 3.0);
    float3 dir = normalize(float3(mul(giro, uv), 1.1));
    float t = 0.0;
    float3 color = 0.0;
    int pasos = (int)carga;
    [loop] for (int i = 0; i < pasos; i++) {
        float3 p = origen + dir * t;
        float d = mapa(p);
        color += 0.9 / (1.0 + d * d * 60.0) * (0.5 + 0.5 * cos(float3(0.0, 2.1, 4.2) + p.z * 0.25 + tiempo)) / carga * 24.0;
        t += max(abs(d) * 0.5, 0.03);
        t = fmod(t, 40.0);
    }
    color = color / (1.0 + color);
    if (pos.y > tam.y - 6.0 && pos.x < tam.x * progreso) color = float3(0.23, 0.5, 0.91);
    return float4(color, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Datos {
    tiempo: f32,
    carga: f32,
    tam: [f32; 2],
    progreso: f32,
    relleno: [f32; 3],
}

/// La ventana a pantalla completa con su escena.
struct Escena {
    hwnd: HWND,
    contexto: ID3D11DeviceContext,
    cadena: IDXGISwapChain1,
    rtv: ID3D11RenderTargetView,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    datos: ID3D11Buffer,
    ancho: u32,
    alto: u32,
    /// Se puede presentar sin esperar al monitor (sin límite de FPS).
    desgarro: bool,
    cerrar: bool,
}

static CERRAR: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn procedimiento(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
            CERRAR.store(true, Ordering::SeqCst);
            LRESULT(0)
        }
        WM_CLOSE | WM_DESTROY => {
            CERRAR.store(true, Ordering::SeqCst);
            LRESULT(0)
        }
        WM_SETCURSOR => {
            SetCursor(None);
            LRESULT(1)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn compilar(entrada: PCSTR, perfil: PCSTR) -> Result<Vec<u8>> {
    let mut codigo: Option<ID3DBlob> = None;
    let mut errores: Option<ID3DBlob> = None;
    let r = D3DCompile(SHADERS.as_ptr() as *const _, SHADERS.len(), PCSTR::null(), None, None, entrada, perfil, 0, 0, &mut codigo, Some(&mut errores));
    if let Err(e) = r {
        let detalle = errores
            .map(|b| String::from_utf8_lossy(std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize())).into_owned())
            .unwrap_or_default();
        return Err(anyhow!("no se pudo compilar la escena: {e} {detalle}"));
    }
    let b = codigo.ok_or_else(|| anyhow!("shader vacío"))?;
    Ok(std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()).to_vec())
}

impl Escena {
    unsafe fn abrir() -> Result<Escena> {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CERRAR.store(false, Ordering::SeqCst);
        let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let r = info.rcMonitor;
        let (ancho, alto) = ((r.right - r.left) as u32, (r.bottom - r.top) as u32);

        let instancia = GetModuleHandleW(None)?;
        let clase = WNDCLASSW { lpfnWndProc: Some(procedimiento), hInstance: instancia.into(), lpszClassName: w!("KairoPruebaFps"), ..Default::default() };
        RegisterClassW(&clase);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("KairoPruebaFps"),
            w!("Kairo · prueba de FPS"),
            WS_POPUP | WS_VISIBLE,
            r.left,
            r.top,
            ancho as i32,
            alto as i32,
            None,
            None,
            instancia,
            None,
        )?;
        let _ = SetForegroundWindow(hwnd);

        let (mut dispositivo, mut contexto) = (None, None);
        D3D11CreateDevice(
            None::<&IDXGIAdapter>,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut dispositivo),
            None,
            Some(&mut contexto),
        )?;
        let dispositivo: ID3D11Device = dispositivo.ok_or_else(|| anyhow!("sin dispositivo"))?;
        let contexto = contexto.ok_or_else(|| anyhow!("sin contexto"))?;
        let fabrica: IDXGIFactory2 = dispositivo.cast::<IDXGIDevice>()?.GetAdapter()?.GetParent()?;
        let mut permite = BOOL(0);
        let desgarro = fabrica
            .cast::<IDXGIFactory5>()
            .and_then(|f| f.CheckFeatureSupport(DXGI_FEATURE_PRESENT_ALLOW_TEARING, &mut permite as *mut _ as *mut _, std::mem::size_of::<BOOL>() as u32))
            .is_ok()
            && permite.as_bool();
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: ancho,
            Height: alto,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
            Flags: if desgarro { DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING.0 as u32 } else { 0 },
            ..Default::default()
        };
        let cadena = fabrica.CreateSwapChainForHwnd(&dispositivo, hwnd, &desc, None, None)?;
        let fondo: ID3D11Texture2D = cadena.GetBuffer(0)?;
        let mut rtv = None;
        dispositivo.CreateRenderTargetView(&fondo, None, Some(&mut rtv))?;
        let (codigo_vs, codigo_ps) = (compilar(s!("vs"), s!("vs_5_0"))?, compilar(s!("ps"), s!("ps_5_0"))?);
        let (mut vs, mut ps) = (None, None);
        dispositivo.CreateVertexShader(&codigo_vs, None, Some(&mut vs))?;
        dispositivo.CreatePixelShader(&codigo_ps, None, Some(&mut ps))?;
        let bdesc = D3D11_BUFFER_DESC {
            ByteWidth: std::mem::size_of::<Datos>() as u32,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            ..Default::default()
        };
        let mut datos = None;
        dispositivo.CreateBuffer(&bdesc, None, Some(&mut datos))?;
        Ok(Escena {
            hwnd,
            contexto,
            cadena,
            rtv: rtv.ok_or_else(|| anyhow!("sin vista"))?,
            vs: vs.ok_or_else(|| anyhow!("sin vertex shader"))?,
            ps: ps.ok_or_else(|| anyhow!("sin pixel shader"))?,
            datos: datos.ok_or_else(|| anyhow!("sin datos"))?,
            ancho,
            alto,
            desgarro,
            cerrar: false,
        })
    }

    /// Dibuja y presenta un cuadro. Devuelve falso si se cerró (Esc).
    unsafe fn cuadro(&mut self, tiempo: f32, carga: u32, progreso: f32) -> bool {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if CERRAR.load(Ordering::SeqCst) || self.cerrar {
            self.cerrar = true;
            return false;
        }
        let d = Datos { tiempo, carga: carga as f32, tam: [self.ancho as f32, self.alto as f32], progreso, relleno: [0.0; 3] };
        let c = &self.contexto;
        c.UpdateSubresource(&self.datos, 0, None, &d as *const _ as *const _, 0, 0);
        c.OMSetRenderTargets(Some(&[Some(self.rtv.clone())]), None);
        c.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: self.ancho as f32, Height: self.alto as f32, MinDepth: 0.0, MaxDepth: 1.0 }]));
        c.IASetInputLayout(None);
        c.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        c.VSSetShader(&self.vs, None);
        c.PSSetShader(&self.ps, None);
        c.PSSetConstantBuffers(0, Some(&[Some(self.datos.clone())]));
        c.Draw(3, 0);
        let banderas = if self.desgarro { DXGI_PRESENT_ALLOW_TEARING } else { DXGI_PRESENT(0) };
        let _ = self.cadena.Present(0, banderas);
        true
    }
}

impl Drop for Escena {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// Busca la carga (pasos por píxel) para que la escena ande a ~200 FPS sin grabar.
pub fn calibrar() -> Result<u32> {
    unsafe {
        let mut e = Escena::abrir()?;
        let mut carga = 200u32;
        let inicio = Instant::now();
        for _ in 0..5 {
            // Medio segundo para que la tarjeta suba de frecuencia y 1,2 s para medir.
            let t0 = Instant::now();
            let mut n = 0u32;
            let mut medir = None;
            while t0.elapsed().as_secs_f64() < 1.7 {
                if !e.cuadro(inicio.elapsed().as_secs_f32(), carga, 0.0) {
                    return Err(anyhow!("cancelada"));
                }
                if t0.elapsed().as_secs_f64() >= 0.5 {
                    medir.get_or_insert_with(Instant::now);
                    n += 1;
                }
            }
            let fps = n as f64 / medir.map(|m| m.elapsed().as_secs_f64()).unwrap_or(1.0).max(0.1);
            let nueva = ((carga as f64 * fps / FPS_OBJETIVO).round() as u32).clamp(8, 20_000);
            println!("carga {carga}: {fps:.0} fps");
            if (nueva as i64 - carga as i64).abs() <= (carga as i64 / 50).max(1) {
                break;
            }
            carga = nueva;
        }
        Ok(carga)
    }
}

/// Lo que usó un grabador (promedios por segundo).
#[derive(Serialize, Default, Clone, Debug)]
pub struct Uso {
    /// Procesador, en % de toda la PC.
    pub cpu: f64,
    /// Tarjeta de video (3D, copias y procesador de video), en % de un motor.
    pub gpu: f64,
    /// El encoder de video de la tarjeta (NVENC, etc.).
    pub encoder: f64,
    /// RAM privada (MB).
    pub ram_mb: f64,
    pub procesos: usize,
}

#[derive(Serialize, Debug)]
pub struct Resultado {
    pub segundos: f64,
    pub carga: u32,
    pub ancho: u32,
    pub alto: u32,
    pub sin_limite: bool,
    pub cuadros: usize,
    pub fps_promedio: f64,
    pub bajo_1: f64,
    pub bajo_01: f64,
    pub peor_ms: f64,
    /// Lo que usó la tarjeta la escena misma (3D).
    pub gpu_juego: f64,
    pub grabadores: HashMap<String, Uso>,
}

/// Corre la escena `segundos` (después de 5 s para calentar) y mide.
pub fn medir(segundos: f64, carga: u32) -> Result<Resultado> {
    unsafe {
        let mut e = Escena::abrir()?;
        let inicio = Instant::now();
        let total = CALENTAR + segundos;
        let detener = Arc::new(AtomicBool::new(false));
        let mut muestreo = None;
        let mut tiempos = Vec::with_capacity((segundos * 400.0) as usize);
        let mut anterior: Option<Instant> = None;
        loop {
            let t = inicio.elapsed().as_secs_f64();
            if t >= total {
                break;
            }
            if !e.cuadro(t as f32, carga, (t / total) as f32) {
                detener.store(true, Ordering::SeqCst);
                return Err(anyhow!("cancelada con Esc"));
            }
            let ahora = Instant::now();
            // El medidor arranca durante el calentamiento: su primera vuelta (la más pesada) no cae en la medición.
            if t >= 1.0 && muestreo.is_none() {
                let d = detener.clone();
                muestreo = Some(std::thread::spawn(move || muestrear(&d)));
            }
            if t >= CALENTAR {
                if let Some(a) = anterior {
                    tiempos.push((ahora - a).as_secs_f64() * 1000.0);
                }
            }
            anterior = Some(ahora);
        }
        detener.store(true, Ordering::SeqCst);
        let (grabadores, gpu_juego) = muestreo.and_then(|h| h.join().ok()).unwrap_or_default();
        let (ancho, alto, sin_limite) = (e.ancho, e.alto, e.desgarro);
        drop(e);

        let n = tiempos.len().max(1);
        let suma: f64 = tiempos.iter().sum();
        let mut ordenados = tiempos.clone();
        ordenados.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let peores = |parte: usize| {
            let k = (n / parte).max(1);
            1000.0 / (ordenados.iter().take(k).sum::<f64>() / k as f64).max(0.001)
        };
        Ok(Resultado {
            segundos,
            carga,
            ancho,
            alto,
            sin_limite,
            cuadros: tiempos.len(),
            fps_promedio: 1000.0 * n as f64 / suma.max(0.001),
            bajo_1: peores(100),
            bajo_01: peores(1000),
            peor_ms: ordenados.first().copied().unwrap_or(0.0),
            gpu_juego,
            grabadores,
        })
    }
}

/// Qué grabador es cada proceso: `kairo.exe` (menos esta prueba) y todo lo que corre desde la carpeta de Medal.
fn grabadores() -> HashMap<u32, &'static str> {
    let mut v = HashMap::new();
    let propio = std::process::id();
    let medal = std::env::var("LOCALAPPDATA").map(|d| format!("{d}\\Medal\\").to_lowercase()).unwrap_or_default();
    unsafe {
        let Ok(foto) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return v };
        let mut p = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut hay = Process32FirstW(foto, &mut p).is_ok();
        while hay {
            let largo = p.szExeFile.iter().position(|&c| c == 0).unwrap_or(p.szExeFile.len());
            let nombre = String::from_utf16_lossy(&p.szExeFile[..largo]).to_lowercase();
            if p.th32ProcessID != propio {
                if nombre == "kairo.exe" {
                    v.insert(p.th32ProcessID, "Kairo");
                } else if !medal.is_empty() && ruta_de(p.th32ProcessID).is_some_and(|r| r.to_lowercase().starts_with(&medal)) {
                    v.insert(p.th32ProcessID, "Medal");
                }
            }
            hay = Process32NextW(foto, &mut p).is_ok();
        }
        let _ = CloseHandle(foto);
    }
    v
}

fn ruta_de(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut largo = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut largo);
        let _ = CloseHandle(h);
        r.ok()?;
        Some(String::from_utf16_lossy(&buf[..largo as usize]))
    }
}

/// Procesador (en unidades de 100 ns) y RAM privada (bytes) de un proceso.
fn tiempos_y_ram(pid: u32) -> Option<(u64, u64)> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut c, mut s, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let t = GetProcessTimes(h, &mut c, &mut s, &mut k, &mut u).ok().map(|_| {
            let v = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
            v(k) + v(u)
        });
        let mut m = PROCESS_MEMORY_COUNTERS_EX { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
        let ram = GetProcessMemoryInfo(h, &mut m as *mut _ as *mut PROCESS_MEMORY_COUNTERS, m.cb).ok().map(|_| m.PrivateUsage as u64);
        let _ = CloseHandle(h);
        Some((t?, ram.unwrap_or(0)))
    }
}

/// Cada segundo, hasta que lo frenen: cuánto usa cada grabador (procesador, tarjeta, encoder y RAM) y cuánto la
/// escena. Los contadores de la tarjeta se piden por su nombre en inglés (Windows en español los traduce).
fn muestrear(detener: &AtomicBool) -> (HashMap<String, Uso>, f64) {
    let propio = std::process::id();
    let nucleos = unsafe {
        let mut s = SYSTEM_INFO::default();
        GetSystemInfo(&mut s);
        s.dwNumberOfProcessors.max(1) as f64
    };
    let mut consulta = 0isize;
    let mut contador = 0isize;
    let gpu_ok = unsafe {
        PdhOpenQueryW(None, 0, &mut consulta) == 0
            && PdhAddEnglishCounterW(consulta, w!("\\GPU Engine(*)\\Utilization Percentage"), 0, &mut contador) == 0
            && PdhCollectQueryData(consulta) == 0
    };
    let mut sumas: HashMap<String, Uso> = HashMap::new();
    let mut muestras = 0u32;
    let mut gpu_juego = 0.0;
    let mut previo: HashMap<u32, u64> = HashMap::new();
    let mut ultimo = Instant::now();
    let mut primera = true;
    while !detener.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(1000));
        let pasaron = ultimo.elapsed().as_secs_f64();
        ultimo = Instant::now();
        let grupos = grabadores();
        let mut esta: HashMap<String, Uso> = HashMap::new();
        for (pid, grupo) in &grupos {
            let u = esta.entry(grupo.to_string()).or_default();
            u.procesos += 1;
            if let Some((t, ram)) = tiempos_y_ram(*pid) {
                if let Some(antes) = previo.get(pid) {
                    u.cpu += (t.saturating_sub(*antes)) as f64 / (pasaron * 1e7 * nucleos) * 100.0;
                }
                previo.insert(*pid, t);
                u.ram_mb += ram as f64 / 1_048_576.0;
            }
        }
        let mut juego = 0.0;
        if gpu_ok {
            unsafe {
                let _ = PdhCollectQueryData(consulta);
                let (mut tam, mut n) = (0u32, 0u32);
                if PdhGetFormattedCounterArrayW(contador, PDH_FMT_DOUBLE, &mut tam, &mut n, None) == PDH_MORE_DATA {
                    let mut buf = vec![0u64; tam as usize / 8 + 1];
                    let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
                    if PdhGetFormattedCounterArrayW(contador, PDH_FMT_DOUBLE, &mut tam, &mut n, Some(items)) == 0 {
                        for item in std::slice::from_raw_parts(items, n as usize) {
                            let nombre = item.szName.to_string().unwrap_or_default();
                            let valor = item.FmtValue.Anonymous.doubleValue;
                            if item.FmtValue.CStatus > 1 || !valor.is_finite() {
                                continue;
                            }
                            // "pid_1234_luid_..._engtype_VideoEncode"
                            let Some(pid) = nombre.strip_prefix("pid_").and_then(|r| r.split('_').next()).and_then(|p| p.parse::<u32>().ok()) else { continue };
                            let encoder = nombre.ends_with("engtype_VideoEncode");
                            if pid == propio {
                                if nombre.ends_with("engtype_3D") {
                                    juego += valor;
                                }
                            } else if let Some(grupo) = grupos.get(&pid) {
                                let u = esta.entry(grupo.to_string()).or_default();
                                if encoder {
                                    u.encoder += valor;
                                } else {
                                    u.gpu += valor;
                                }
                            }
                        }
                    }
                }
            }
        }
        // La primera vuelta no tiene con qué comparar el procesador: no cuenta.
        if primera {
            primera = false;
            continue;
        }
        muestras += 1;
        gpu_juego += juego;
        for (g, u) in esta {
            let s = sumas.entry(g).or_default();
            s.cpu += u.cpu;
            s.gpu += u.gpu;
            s.encoder += u.encoder;
            s.ram_mb += u.ram_mb;
            s.procesos = s.procesos.max(u.procesos);
        }
    }
    if gpu_ok {
        unsafe {
            let _ = PdhCloseQuery(consulta);
        }
    }
    let m = muestras.max(1) as f64;
    for u in sumas.values_mut() {
        u.cpu /= m;
        u.gpu /= m;
        u.encoder /= m;
        u.ram_mb /= m;
    }
    (sumas, gpu_juego / m)
}
