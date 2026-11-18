// Kairo: grabador de clips instantáneos para Windows.
// SPDX-License-Identifier: GPL-3.0-or-later

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod equipo;
mod ui;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    // Por ahora el único modo es la ventana de ajustes. La grabadora (bandeja) llega en la fase 1.
    ui::abrir_ajustes(ui::captura::desde_args(&args))
}
