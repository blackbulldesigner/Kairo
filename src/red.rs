//! Pedidos HTTPS con WinHTTP (viene con Windows: no suma dependencias). Solo GET, para buscar
//! actualizaciones y bajar el instalador.

use std::ffi::c_void;

use anyhow::{anyhow, Result};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Networking::WinHttp::*;

struct Manija(*mut c_void);

impl Drop for Manija {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

fn manija(p: *mut c_void, que: &str) -> Result<Manija> {
    if p.is_null() {
        Err(anyhow!("{que}: {}", windows::core::Error::from_win32()))
    } else {
        Ok(Manija(p))
    }
}

unsafe fn numero(pedido: *mut c_void, que: u32) -> Option<u64> {
    let mut v = 0u32;
    let mut largo = 4u32;
    WinHttpQueryHeaders(pedido, que | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut v as *mut u32 as *mut _), &mut largo, std::ptr::null_mut())
        .ok()?;
    Some(v as u64)
}

/// GET a `url` (solo https; las redirecciones se siguen solas). Con respuesta 200, llama a `trozo` con cada
/// parte que llega y el tamaño total (si el servidor lo dice). Devuelve el código HTTP.
pub fn obtener(url: &str, aceptar: Option<&str>, mut trozo: impl FnMut(&[u8], Option<u64>) -> Result<()>) -> Result<u32> {
    let resto = url.strip_prefix("https://").ok_or_else(|| anyhow!("solo se aceptan direcciones https: {url}"))?;
    let (host, ruta) = match resto.find('/') {
        Some(i) => (&resto[..i], &resto[i..]),
        None => (resto, "/"),
    };
    unsafe {
        let agente = HSTRING::from(format!("Kairo/{}", crate::version::VERSION));
        let sesion = manija(WinHttpOpen(&agente, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0), "WinHttpOpen")?;
        // 15 s para encontrar y conectar el servidor; 60 s como mucho sin que llegue nada.
        let _ = WinHttpSetTimeouts(sesion.0, 15_000, 15_000, 30_000, 60_000);
        let conexion = manija(WinHttpConnect(sesion.0, &HSTRING::from(host), INTERNET_DEFAULT_HTTPS_PORT, 0), "WinHttpConnect")?;
        let pedido = manija(
            WinHttpOpenRequest(conexion.0, w!("GET"), &HSTRING::from(ruta), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE),
            "WinHttpOpenRequest",
        )?;
        if let Some(a) = aceptar {
            let h: Vec<u16> = format!("Accept: {a}").encode_utf16().collect();
            WinHttpAddRequestHeaders(pedido.0, &h, WINHTTP_ADDREQ_FLAG_ADD)?;
        }
        WinHttpSendRequest(pedido.0, None, None, 0, 0, 0).map_err(|e| anyhow!("sin conexión con {host}: {e}"))?;
        WinHttpReceiveResponse(pedido.0, std::ptr::null_mut()).map_err(|e| anyhow!("{host} no respondió: {e}"))?;
        let codigo = numero(pedido.0, WINHTTP_QUERY_STATUS_CODE).unwrap_or(0) as u32;
        if codigo != 200 {
            return Ok(codigo);
        }
        let total = numero(pedido.0, WINHTTP_QUERY_CONTENT_LENGTH);
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut leidos = 0u32;
            WinHttpReadData(pedido.0, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut leidos)?;
            if leidos == 0 {
                break;
            }
            trozo(&buf[..leidos as usize], total)?;
        }
        Ok(codigo)
    }
}

/// GET que devuelve el texto completo (como mucho 4 MB).
pub fn texto(url: &str, aceptar: Option<&str>) -> Result<(u32, String)> {
    let mut datos = Vec::new();
    let codigo = obtener(url, aceptar, |t, _| {
        datos.extend_from_slice(t);
        if datos.len() > 4 * 1024 * 1024 {
            return Err(anyhow!("la respuesta es demasiado grande"));
        }
        Ok(())
    })?;
    Ok((codigo, String::from_utf8_lossy(&datos).into_owned()))
}

/// SHA-256 en hexadecimal (minúsculas), con la criptografía de Windows.
pub fn sha256(datos: &[u8]) -> Result<String> {
    use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
    let mut salida = [0u8; 32];
    unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, datos, &mut salida) }.ok()?;
    Ok(salida.iter().map(|b| format!("{b:02x}")).collect())
}
