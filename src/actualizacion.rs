//! Versiones nuevas. Kairo mira el último Release publicado en GitHub (blackbulldesigner/Kairo) y, si es más
//! nuevo que este exe, avisa. "Actualizar" baja el KairoSetup de ese release, comprueba su SHA-256 (el que
//! informa GitHub, o el archivo `.sha256` del release) y lo corre sin ventana: cierra Kairo, lo reemplaza y
//! lo vuelve a abrir. Tus ajustes y tus clips no se tocan.
//!
//! La grabadora revisa un minuto después de arrancar y después una vez por día (una consulta chica a GitHub);
//! la ventana, al abrirse, si pasaron más de 6 horas. Lo último que se supo queda en
//! `%APPDATA%\Kairo\actualizacion.json`. Se puede apagar en Acerca de.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};

use crate::red;
use crate::registro::log;

const API: &str = "https://api.github.com/repos/blackbulldesigner/Kairo/releases/latest";
const TAMANO_MAXIMO: usize = 200 * 1024 * 1024;

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Disponible {
    pub version: String,
    /// La página del release (para leer qué trae).
    pub pagina: String,
    pub notas: String,
    pub instalador: Option<Archivo>,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Archivo {
    pub nombre: String,
    pub url: String,
    pub tamano: u64,
    pub sha256: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Guardado {
    /// Cuándo se le preguntó a GitHub por última vez (segundos desde 1970).
    revisado: u64,
    disponible: Option<Disponible>,
    /// La última versión de la que ya se avisó con un globo (para no repetir el aviso).
    avisada: Option<String>,
}

fn ruta() -> PathBuf {
    crate::config::datos_dir().join("actualizacion.json")
}

fn leer() -> Guardado {
    std::fs::read_to_string(ruta()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn escribir(g: &Guardado) {
    let _ = std::fs::create_dir_all(crate::config::datos_dir());
    if let Ok(s) = serde_json::to_string_pretty(g) {
        let _ = std::fs::write(ruta(), s);
    }
}

fn ahora() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// "v0.7.1" → (0, 7, 1). Lo que venga después de un guion ("-beta") no cuenta.
pub fn numeros(v: &str) -> Option<(u32, u32, u32)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let v = v.split('-').next()?;
    let mut partes = v.split('.').map(|p| p.parse::<u32>());
    let a = partes.next()?.ok()?;
    let b = partes.next().unwrap_or(Ok(0)).ok()?;
    let c = partes.next().unwrap_or(Ok(0)).ok()?;
    Some((a, b, c))
}

/// ¿`v` es más nueva que este exe?
pub fn es_mas_nueva(v: &str) -> bool {
    match (numeros(v), numeros(crate::version::VERSION)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Lee un release de la API de GitHub.
pub fn desde_json(j: &serde_json::Value) -> Result<Disponible> {
    let tag = j["tag_name"].as_str().unwrap_or_default();
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    if numeros(&version).is_none() {
        bail!("el release no tiene un número de versión: «{tag}»");
    }
    let assets = j["assets"].as_array().cloned().unwrap_or_default();
    let nombre = |a: &serde_json::Value| a["name"].as_str().unwrap_or_default().to_string();
    let setup = assets.iter().find(|a| {
        let n = nombre(a).to_lowercase();
        n.starts_with("kairosetup") && n.ends_with(".exe")
    });
    let mut instalador = setup.map(|a| Archivo {
        nombre: nombre(a),
        url: a["browser_download_url"].as_str().unwrap_or_default().to_string(),
        tamano: a["size"].as_u64().unwrap_or(0),
        // GitHub informa el SHA-256 de cada archivo subido ("digest": "sha256:…").
        sha256: a["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(|s| s.to_lowercase()),
    });
    // Si no, se usa el archivo "<instalador>.sha256" del mismo release.
    if let Some(i) = instalador.as_mut().filter(|i| i.sha256.is_none()) {
        let buscado = format!("{}.sha256", i.nombre).to_lowercase();
        if let Some(url) = assets.iter().find(|a| nombre(a).to_lowercase() == buscado).and_then(|a| a["browser_download_url"].as_str()) {
            if let Ok((200, t)) = red::texto(url, None) {
                let hex: String = t.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
                if hex.len() == 64 {
                    i.sha256 = Some(hex.to_lowercase());
                }
            }
        }
    }
    let notas: String = j["body"].as_str().unwrap_or_default().chars().take(3000).collect();
    Ok(Disponible { version, pagina: j["html_url"].as_str().unwrap_or_default().to_string(), notas, instalador })
}

/// Le pregunta a GitHub cuál es el último release. None = todavía no hay ninguno publicado.
pub fn buscar() -> Result<Option<Disponible>> {
    let api = std::env::var("KAIRO_ACTUALIZACION_API").unwrap_or_else(|_| API.to_string());
    let (codigo, cuerpo) = red::texto(&api, Some("application/vnd.github+json"))?;
    match codigo {
        200 => {}
        404 => return Ok(None),
        c => bail!("GitHub respondió con el código {c}"),
    }
    let j: serde_json::Value = serde_json::from_str(&cuerpo)?;
    desde_json(&j).map(Some)
}

/// Si pasó `cada` desde la última consulta, vuelve a preguntar. Devuelve la versión nueva, si hay una más
/// nueva que este exe. Si no hay internet, se queda con lo último que se supo.
pub fn revisar(cada: Duration) -> Option<Disponible> {
    let mut g = leer();
    if ahora() >= g.revisado.saturating_add(cada.as_secs()) {
        match buscar() {
            Ok(d) => {
                g.revisado = ahora();
                g.disponible = d;
                escribir(&g);
            }
            Err(e) => log(&format!("no se pudo buscar actualizaciones: {e:#}")),
        }
    }
    g.disponible.filter(|d| es_mas_nueva(&d.version))
}

/// Pregunta ya mismo (botón "Buscar ahora"). Ok(None) = estás al día.
pub fn revisar_ahora() -> Result<Option<Disponible>> {
    let d = buscar()?;
    let mut g = leer();
    g.revisado = ahora();
    g.disponible = d;
    escribir(&g);
    Ok(g.disponible.filter(|d| es_mas_nueva(&d.version)))
}

/// Lo último que se supo, sin ir a internet.
pub fn conocida() -> Option<Disponible> {
    leer().disponible.filter(|d| es_mas_nueva(&d.version))
}

/// Cuándo se revisó por última vez (segundos desde 1970), si alguna vez se revisó.
pub fn revisado() -> Option<u64> {
    Some(leer().revisado).filter(|&r| r > 0)
}

/// Marca que ya se avisó de esta versión. Devuelve true si es la primera vez.
pub fn marcar_avisada(version: &str) -> bool {
    let mut g = leer();
    if g.avisada.as_deref() == Some(version) {
        return false;
    }
    g.avisada = Some(version.to_string());
    escribir(&g);
    true
}

fn carpeta_descargas() -> PathBuf {
    std::env::temp_dir().join("Kairo-actualizacion")
}

/// Borra los instaladores que quedaron de una actualización anterior.
pub fn limpiar_descargas() {
    let _ = std::fs::remove_dir_all(carpeta_descargas());
}

/// Baja el instalador y comprueba que sea idéntico al publicado (SHA-256). `progreso` recibe de 0 a 1.
pub fn descargar(a: &Archivo, progreso: &dyn Fn(f32)) -> Result<PathBuf> {
    let Some(esperado) = a.sha256.clone() else {
        bail!("el release no dice el SHA-256 del instalador: por seguridad no se instala solo (bájalo desde la página)");
    };
    if !a.url.starts_with("https://github.com/") {
        bail!("el instalador no está en GitHub: {}", a.url);
    }
    let mut datos = Vec::with_capacity(a.tamano.min(TAMANO_MAXIMO as u64) as usize);
    let codigo = red::obtener(&a.url, None, |t, total| {
        datos.extend_from_slice(t);
        if datos.len() > TAMANO_MAXIMO {
            return Err(anyhow!("el instalador es demasiado grande"));
        }
        let total = total.unwrap_or(a.tamano).max(1);
        progreso((datos.len() as f32 / total as f32).min(1.0));
        Ok(())
    })?;
    if codigo != 200 {
        bail!("no se pudo bajar el instalador (código {codigo})");
    }
    let calculado = red::sha256(&datos)?;
    if calculado != esperado {
        bail!("el instalador bajado no es idéntico al publicado (SHA-256 distinto): no se instala");
    }
    // El nombre se arma acá (no se usa el del release tal cual).
    let version: String = a.nombre.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-').collect();
    let dir = carpeta_descargas();
    std::fs::create_dir_all(&dir)?;
    let destino = dir.join(if version.to_lowercase().starts_with("kairosetup") { version } else { "KairoSetup.exe".into() });
    std::fs::write(&destino, &datos)?;
    log(&format!("instalador bajado y verificado: {} (sha256 {calculado})", destino.display()));
    Ok(destino)
}

/// Corre el instalador bajado sin ventana: cierra Kairo (esta ventana incluida), lo reemplaza y lo vuelve
/// a abrir.
pub fn instalar(instalador: &Path) -> Result<()> {
    std::process::Command::new(instalador).args(["--instalar", "--sin-ventana", "--abrir"]).spawn()?;
    Ok(())
}
