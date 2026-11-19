//! Instalar y desinstalar Kairo, solo para el usuario actual (no pide permisos de administrador):
//! - el exe va a %LOCALAPPDATA%\Programs\Kairo\Kairo.exe;
//! - accesos directos en el menú Inicio y (si se pide) en el escritorio;
//! - la entrada en "Aplicaciones instaladas" de Windows (HKCU\…\Uninstall\Kairo), con su desinstalador;
//! - "Iniciar con Windows" (si se pide).
//!
//! El instalador es el mismo exe de Kairo con otro nombre ("KairoSetup….exe") o con `--instalar`: se copia
//! a sí mismo. Desinstalar es `Kairo.exe --desinstalar`. Los clips nunca se tocan.
//!
//! Para probar sin tocar la PC: `KAIRO_PRUEBA_INSTALACION=<carpeta>` instala todo dentro de esa carpeta,
//! usa otra clave del registro y no cierra el Kairo abierto ni toca el inicio con Windows.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use windows::core::{w, Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE,
    REG_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Programs, FOLDERID_UserProgramFiles, IShellLinkW, SHGetKnownFolderPath, ShellLink, KF_FLAG_DEFAULT};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_CLOSE};

use crate::registro::log;

const CLAVE_REAL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Kairo";
// (Directo bajo Software: al desinstalar no queda ninguna clave madre vacía.)
const CLAVE_PRUEBA: &str = r"Software\KairoPruebaInstalacion";
const REPOSITORIO: &str = "https://github.com/blackbulldesigner/Kairo";

/// Dónde va cada cosa.
pub struct Destinos {
    pub carpeta: PathBuf,
    pub acceso_menu: PathBuf,
    pub acceso_escritorio: PathBuf,
    clave: &'static str,
    /// En modo prueba: no se cierra el Kairo abierto ni se toca el inicio con Windows.
    pub prueba: bool,
}

impl Destinos {
    pub fn exe(&self) -> PathBuf {
        self.carpeta.join("Kairo.exe")
    }
}

fn carpeta_conocida(id: &windows::core::GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, HANDLE::default()).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

pub fn destinos() -> Destinos {
    if let Some(base) = std::env::var_os("KAIRO_PRUEBA_INSTALACION").map(PathBuf::from) {
        return Destinos {
            carpeta: base.join("Programs").join("Kairo"),
            acceso_menu: base.join("Menu").join("Kairo.lnk"),
            acceso_escritorio: base.join("Escritorio").join("Kairo.lnk"),
            clave: CLAVE_PRUEBA,
            prueba: true,
        };
    }
    let local = std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Programs"));
    Destinos {
        carpeta: carpeta_conocida(&FOLDERID_UserProgramFiles).or(local).unwrap_or_else(|| PathBuf::from(".")).join("Kairo"),
        acceso_menu: carpeta_conocida(&FOLDERID_Programs).unwrap_or_default().join("Kairo.lnk"),
        acceso_escritorio: carpeta_conocida(&FOLDERID_Desktop).unwrap_or_default().join("Kairo.lnk"),
        clave: CLAVE_REAL,
        prueba: false,
    }
}

/// ¿Este exe tiene que hacer de instalador? Si se llama "KairoSetup…" (o tiene "instalador" en el
/// nombre) o si se abrió con `--instalar`.
pub fn es_instalador(args: &[String]) -> bool {
    if args.iter().any(|a| a == "--instalar") {
        return true;
    }
    std::env::current_exe()
        .ok()
        .and_then(|e| e.file_stem().map(|s| s.to_string_lossy().to_lowercase()))
        .is_some_and(|n| n.starts_with("kairosetup") || n.starts_with("kairo-setup") || n.contains("instalador"))
}

/// La versión instalada (la de la entrada de "Aplicaciones instaladas"), si hay.
pub fn version_instalada() -> Option<String> {
    let d = destinos();
    let (clave, valor) = (HSTRING::from(d.clave), HSTRING::from("DisplayVersion"));
    unsafe {
        let mut largo = 0u32;
        RegGetValueW(HKEY_CURRENT_USER, &clave, &valor, RRF_RT_REG_SZ, None, None, Some(&mut largo)).ok().ok()?;
        let mut buf = vec![0u16; (largo as usize).div_ceil(2) + 1];
        RegGetValueW(HKEY_CURRENT_USER, &clave, &valor, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut largo)).ok().ok()?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n])).filter(|v| !v.is_empty())
    }
}

/// Cierra la grabadora y la ventana de Kairo si están abiertas (como "Salir de Kairo") y espera.
fn cerrar_kairo_abierto() {
    let titulo_ajustes = HSTRING::from(crate::ui::TITULO);
    unsafe {
        if let Some(h) = crate::bandeja::ventana() {
            let _ = PostMessageW(h, WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        if let Ok(h) = FindWindowW(PCWSTR::null(), &titulo_ajustes) {
            if !h.is_invalid() {
                let _ = PostMessageW(h, WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
    for _ in 0..50 {
        let ajustes = unsafe { FindWindowW(PCWSTR::null(), &titulo_ajustes).is_ok_and(|h: HWND| !h.is_invalid()) };
        if crate::bandeja::ventana().is_none() && !ajustes {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    // Un momento más para que Windows suelte el exe.
    std::thread::sleep(Duration::from_millis(300));
}

unsafe fn crear_acceso(lnk: &Path, exe: &Path) -> Result<()> {
    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    if let Some(dir) = lnk.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let enlace: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
    enlace.SetPath(&HSTRING::from(exe.as_os_str()))?;
    if let Some(dir) = exe.parent() {
        enlace.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
    }
    enlace.SetDescription(w!("Kairo: clips instantáneos"))?;
    enlace.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)?;
    let archivo: IPersistFile = enlace.cast()?;
    archivo.Save(&HSTRING::from(lnk.as_os_str()), true)?;
    Ok(())
}

unsafe fn poner_texto(clave: HKEY, nombre: &str, valor: &str) {
    let v: Vec<u16> = valor.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2);
    let _ = RegSetValueExW(clave, &HSTRING::from(nombre), 0, REG_SZ, Some(bytes));
}

unsafe fn poner_numero(clave: HKEY, nombre: &str, valor: u32) {
    let _ = RegSetValueExW(clave, &HSTRING::from(nombre), 0, REG_DWORD, Some(&valor.to_le_bytes()));
}

/// La entrada en "Aplicaciones instaladas".
fn registrar(d: &Destinos, exe: &Path) -> Result<()> {
    unsafe {
        let mut clave = HKEY::default();
        RegCreateKeyExW(HKEY_CURRENT_USER, &HSTRING::from(d.clave), 0, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut clave, None)
            .ok()
            .context("no se pudo escribir en el registro")?;
        let exe_txt = exe.display().to_string();
        poner_texto(clave, "DisplayName", "Kairo");
        poner_texto(clave, "DisplayVersion", crate::version::VERSION);
        poner_texto(clave, "Publisher", "Kairo");
        poner_texto(clave, "DisplayIcon", &format!("{exe_txt},0"));
        poner_texto(clave, "InstallLocation", &d.carpeta.display().to_string());
        poner_texto(clave, "UninstallString", &format!("\"{exe_txt}\" --desinstalar"));
        poner_texto(clave, "URLInfoAbout", REPOSITORIO);
        poner_texto(clave, "InstallDate", &crate::registro::fecha_para_archivo().replace('-', "").chars().take(8).collect::<String>());
        poner_numero(clave, "NoModify", 1);
        poner_numero(clave, "NoRepair", 1);
        let kb = std::fs::metadata(exe).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
        poner_numero(clave, "EstimatedSize", kb);
        let _ = RegCloseKey(clave);
    }
    Ok(())
}

pub struct Opciones {
    pub escritorio: bool,
    pub inicio_windows: bool,
}

/// Instala (o actualiza) Kairo copiando este mismo exe. `paso` recibe qué se está haciendo.
/// Devuelve la ruta del Kairo.exe instalado.
pub fn instalar(op: &Opciones, paso: &dyn Fn(&str)) -> Result<PathBuf> {
    let d = destinos();
    let propio = std::env::current_exe().context("no se encontró el instalador")?;
    let exe = d.exe();
    log(&format!("instalando Kairo {} en {}", crate::version::VERSION, d.carpeta.display()));

    if !d.prueba {
        paso("Cerrando Kairo si está abierto…");
        cerrar_kairo_abierto();
    }

    paso("Copiando Kairo…");
    std::fs::create_dir_all(&d.carpeta).with_context(|| format!("no se pudo crear {}", d.carpeta.display()))?;
    if propio != exe {
        // Si Windows todavía no soltó el exe anterior, se reintenta un rato.
        let mut ultimo = None;
        for _ in 0..20 {
            match std::fs::copy(&propio, &exe) {
                Ok(_) => {
                    ultimo = None;
                    break;
                }
                Err(e) => {
                    ultimo = Some(e);
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
        if let Some(e) = ultimo {
            return Err(anyhow!("no se pudo copiar Kairo ({e}). Cierra Kairo y vuelve a intentar."));
        }
    }

    paso("Creando los accesos directos…");
    unsafe {
        crear_acceso(&d.acceso_menu, &exe).context("no se pudo crear el acceso del menú Inicio")?;
        if op.escritorio {
            crear_acceso(&d.acceso_escritorio, &exe).context("no se pudo crear el acceso del escritorio")?;
        } else {
            let _ = std::fs::remove_file(&d.acceso_escritorio);
        }
    }

    paso("Registrando Kairo en Windows…");
    registrar(&d, &exe)?;
    if !d.prueba {
        crate::autoinicio::poner_para(&exe, op.inicio_windows);
    }
    log("instalación terminada");
    Ok(exe)
}

/// `--instalar --sin-ventana`: actualiza el Kairo instalado sin preguntar nada, conservando lo que ya
/// tenía (acceso en el escritorio, inicio con Windows). No lo abre. Devuelve si salió bien.
pub fn actualizar_sin_ventana() -> bool {
    let d = destinos();
    let op = Opciones {
        escritorio: d.acceso_escritorio.exists(),
        inicio_windows: crate::autoinicio::activo(),
    };
    match instalar(&op, &|_| {}) {
        Ok(_) => true,
        Err(e) => {
            log(&format!("no se pudo actualizar: {e:#}"));
            eprintln!("{e:#}");
            false
        }
    }
}

/// Abre el Kairo instalado (la grabadora en la bandeja y la ventana).
pub fn abrir_instalado(exe: &Path) {
    let _ = std::process::Command::new(exe).spawn();
}

/// Desinstala: quita accesos, la entrada de Windows y el inicio con Windows. Con `borrar_ajustes`, también
/// %APPDATA%\Kairo. Los clips no se tocan. La carpeta del programa se borra después, con
/// `borrar_carpeta_al_salir`, porque este exe suele estar ahí adentro.
pub fn desinstalar(borrar_ajustes: bool) -> Result<()> {
    let d = destinos();
    log("desinstalando Kairo");
    if !d.prueba {
        cerrar_kairo_abierto();
        crate::autoinicio::poner(false);
    }
    let _ = std::fs::remove_file(&d.acceso_menu);
    let _ = std::fs::remove_file(&d.acceso_escritorio);
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(d.clave));
    }
    if borrar_ajustes && !d.prueba {
        let _ = std::fs::remove_dir_all(crate::config::datos_dir());
    }
    Ok(())
}

/// Borra la carpeta del programa cuando este exe ya se cerró: se llama justo antes de salir. cmd espera un
/// segundo y reintenta (unos 20 s como mucho) mientras Windows no suelte el exe. Arranca en otra carpeta:
/// si arrancara adentro (el acceso directo abre Kairo ahí), no la podría borrar.
pub fn borrar_carpeta_al_salir() -> Result<()> {
    const SIN_VENTANA: u32 = 0x0800_0000;
    let carpeta = destinos().carpeta.display().to_string();
    // (`ping` para esperar: `timeout` se niega a correr sin una consola de verdad.)
    let orden = format!(
        "/C for /L %i in (1,1,20) do @(if exist \"{carpeta}\" (ping -n 2 127.0.0.1 >NUL & rmdir /S /Q \"{carpeta}\" 2>NUL))"
    );
    std::process::Command::new("cmd.exe")
        .raw_arg(orden)
        .current_dir(std::env::temp_dir())
        .creation_flags(SIN_VENTANA)
        .spawn()
        .context("no se pudo programar el borrado de la carpeta")?;
    Ok(())
}

/// `Kairo.exe --desinstalar`: pregunta, desinstala y avisa (con los cuadros de diálogo de Windows).
pub fn desinstalar_preguntando() {
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_YESNO};
    unsafe {
        let r = MessageBoxW(
            HWND::default(),
            &HSTRING::from("¿Desinstalar Kairo?\n\nTus clips no se borran: quedan en su carpeta."),
            w!("Desinstalar Kairo"),
            MB_YESNO | MB_ICONQUESTION,
        );
        if r != IDYES {
            return;
        }
        let ajustes = MessageBoxW(
            HWND::default(),
            &HSTRING::from("¿Borrar también tus ajustes de Kairo?\n\nSi vas a volver a instalarlo, elige «No» para conservarlos."),
            w!("Desinstalar Kairo"),
            MB_YESNO | MB_ICONQUESTION,
        ) == IDYES;
        let resultado = desinstalar(ajustes);
        let texto = match &resultado {
            Ok(()) => "Kairo se desinstaló. Tus clips siguen en su carpeta.".to_string(),
            Err(e) => format!("No se pudo desinstalar del todo: {e:#}"),
        };
        MessageBoxW(HWND::default(), &HSTRING::from(texto), w!("Desinstalar Kairo"), MB_ICONINFORMATION);
        if resultado.is_ok() {
            let _ = borrar_carpeta_al_salir();
        }
    }
}
