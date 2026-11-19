// Kairo: grabador de clips instantáneos para Windows.
// SPDX-License-Identifier: GPL-3.0-or-later

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Un solo exe con varios modos:
//! - sin argumentos: la grabadora en la bandeja (y abre los ajustes);
//! - `--fondo`: la grabadora sin abrir ventanas (inicio con Windows);
//! - `--ajustes`: la ventana de ajustes (un proceso aparte que solo existe mientras está abierta);
//! - `--instalar` o el exe llamado "KairoSetup…": el instalador (`--instalar --sin-ventana` actualiza el
//!   Kairo instalado sin preguntar); `--desinstalar`: el desinstalador.

mod abrir;
mod actualizacion;
mod autoinicio;
mod bandeja;
mod config;
mod datos_clip;
#[cfg(feature = "dev")]
mod dev;
mod equipo;
mod estado;
mod grabadora;
mod icono;
mod instalacion;
mod medios;
mod red;
mod registro;
mod ui;
mod version;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    #[cfg(feature = "dev")]
    if let Some(codigo) = dev::ejecutar(&args) {
        std::process::exit(codigo);
    }
    if args.iter().any(|a| a == "--desinstalar") {
        instalacion::desinstalar_preguntando();
        return;
    }
    if let Some(captura) = ui::captura::del_instalador(&args) {
        let _ = ui::instalador::abrir(Some(captura));
        return;
    }
    if instalacion::es_instalador(&args) {
        if args.iter().any(|a| a == "--sin-ventana") {
            let abrir = args.iter().any(|a| a == "--abrir");
            std::process::exit(if instalacion::actualizar_sin_ventana(abrir) { 0 } else { 1 });
        }
        let _ = ui::instalador::abrir(None);
        return;
    }
    let captura = ui::captura::desde_args(&args);
    if captura.is_some() || args.iter().any(|a| a == "--ajustes") {
        let _ = ui::abrir_ajustes(captura);
        return;
    }
    bandeja::ejecutar(args.iter().any(|a| a == "--fondo"));
}
