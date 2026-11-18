//! Guarda dentro del exe de qué versión del código sale: commit, rama, fecha, si había cambios sin
//! commit y los últimos cambios (sus títulos). La página "Acerca de" los muestra para saber si el exe
//! que estás usando tiene lo último.

use std::path::Path;
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

    // Últimos cambios: hash, fecha y título, separados por el carácter 0x1F (no aparece en los títulos).
    let novedades = git(&["log", "-20", "--no-merges", "--format=%h%x1f%ad%x1f%s", "--date=format:%d/%m/%Y"]).unwrap_or_default();
    let destino = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("novedades.txt");
    std::fs::write(destino, novedades).expect("no se pudo escribir novedades.txt");
}
