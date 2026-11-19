//! Al compilar:
//! - Guarda dentro del exe de qué versión del código sale: commit, rama, fecha, si había cambios sin
//!   commit y los últimos cambios (sus títulos). La página "Acerca de" los muestra.
//! - Le pone al exe su ícono (el mismo dibujo que el de la ventana y la bandeja) y la información de
//!   versión que muestra Windows (Administrador de tareas, propiedades del archivo, el instalador).

#[path = "src/icono.rs"]
mod icono;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn git(args: &[&str]) -> Option<String> {
    let salida = Command::new("git").args(args).output().ok()?;
    salida.status.success().then(|| String::from_utf8_lossy(&salida.stdout).trim().to_string())
}

fn main() {
    // Se vuelve a correr si cambia el código o si hay un commit nuevo (aunque no cambie ningún archivo).
    for p in ["src", "Cargo.toml", "build.rs", ".git/HEAD", ".git/index", ".git/logs/HEAD"] {
        println!("cargo:rerun-if-changed={p}");
    }
    let commit = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_default();
    let rama = git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let fecha = git(&["log", "-1", "--format=%cd", "--date=format:%d/%m/%Y %H:%M"]).unwrap_or_default();
    let sucio = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    let compilado = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    println!("cargo:rustc-env=KAIRO_COMMIT={commit}");
    println!("cargo:rustc-env=KAIRO_RAMA={rama}");
    println!("cargo:rustc-env=KAIRO_FECHA_COMMIT={fecha}");
    println!("cargo:rustc-env=KAIRO_SUCIO={}", if sucio { "1" } else { "0" });
    println!("cargo:rustc-env=KAIRO_COMPILADO={compilado}");

    let salida = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    // Últimos cambios: hash, fecha y título, separados por el carácter 0x1F (no aparece en los títulos).
    let novedades = git(&["log", "-20", "--no-merges", "--format=%h%x1f%ad%x1f%s", "--date=format:%d/%m/%Y"]).unwrap_or_default();
    std::fs::write(salida.join("novedades.txt"), novedades).expect("no se pudo escribir novedades.txt");

    icono_y_version(&salida);
}

/// PNG de `n`×`n` con el ícono de Kairo.
fn png(n: usize) -> Vec<u8> {
    let mut datos = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut datos, n as u32, n as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("png");
        w.write_image_data(&icono::rgba(n, [0x3B, 0x7F, 0xE8])).expect("png");
    }
    datos
}

/// Archivo .ico con varios tamaños (en PNG, el formato que Windows acepta desde Vista) y un .rc con el
/// ícono y la información de versión, que se compila y se mete en el exe.
fn icono_y_version(salida: &Path) {
    let tamanos = [16usize, 20, 24, 32, 40, 48, 64, 96, 128, 256];
    let imagenes: Vec<Vec<u8>> = tamanos.iter().map(|&n| png(n)).collect();
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(tamanos.len() as u16).to_le_bytes());
    let mut posicion = 6 + 16 * tamanos.len() as u32;
    for (&n, img) in tamanos.iter().zip(&imagenes) {
        let lado = if n >= 256 { 0 } else { n as u8 }; // 0 significa 256
        ico.extend_from_slice(&[lado, lado, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes()); // planos
        ico.extend_from_slice(&32u16.to_le_bytes()); // bits por pixel
        ico.extend_from_slice(&(img.len() as u32).to_le_bytes());
        ico.extend_from_slice(&posicion.to_le_bytes());
        posicion += img.len() as u32;
    }
    for img in &imagenes {
        ico.extend_from_slice(img);
    }
    let ruta_ico = salida.join("kairo.ico");
    std::fs::write(&ruta_ico, ico).expect("no se pudo escribir kairo.ico");

    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let numeros: Vec<u16> = version.split('.').map(|x| x.parse().unwrap_or(0)).chain(std::iter::repeat(0)).take(4).collect();
    let coma = format!("{},{},{},{}", numeros[0], numeros[1], numeros[2], numeros[3]);
    let ruta_rc = salida.join("kairo.rc");
    let mut rc = std::fs::File::create(&ruta_rc).expect("no se pudo escribir kairo.rc");
    // Solo ASCII en el .rc: así no hay problemas de codificación con el compilador de recursos.
    writeln!(rc, "1 ICON \"{}\"", ruta_ico.display().to_string().replace('\\', "\\\\")).unwrap();
    writeln!(
        rc,
        r#"1 VERSIONINFO
FILEVERSION {coma}
PRODUCTVERSION {coma}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "0C0A04B0"
    BEGIN
      VALUE "CompanyName", "Kairo"
      VALUE "FileDescription", "Kairo"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "kairo"
      VALUE "LegalCopyright", "GPL-3.0-or-later"
      VALUE "OriginalFilename", "Kairo.exe"
      VALUE "ProductName", "Kairo"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0C0A, 1200
  END
END"#
    )
    .unwrap();
    drop(rc);
    let _ = embed_resource::compile(&ruta_rc, embed_resource::NONE);
}
