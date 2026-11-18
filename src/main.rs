// Kairo: grabador de clips instantáneos para Windows.
// SPDX-License-Identifier: GPL-3.0-or-later

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Un solo exe con varios modos:
//! - sin argumentos: la grabadora en la bandeja (y abre los ajustes);
//! - `--fondo`: la grabadora sin abrir ventanas (inicio con Windows);
//! - `--ajustes`: la ventana de ajustes (un proceso aparte que solo existe mientras está abierta).

mod autoinicio;
mod bandeja;
mod config;
#[cfg(feature = "dev")]
mod dev;
mod equipo;
mod estado;
mod grabadora;
mod icono;
mod registro;
mod ui;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    #[cfg(feature = "dev")]
    if let Some(codigo) = dev::ejecutar(&args) {
        std::process::exit(codigo);
    }
    let captura = ui::captura::desde_args(&args);
    if captura.is_some() || args.iter().any(|a| a == "--ajustes") {
        let _ = ui::abrir_ajustes(captura);
        return;
    }
    bandeja::ejecutar(args.iter().any(|a| a == "--fondo"));
}
