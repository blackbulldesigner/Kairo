# Kairo: guía para agentes

Grabador de clips instantáneos para Windows 10 y 11, en Rust (eframe/egui 0.29 con glow, windows 0.58).
Ver `README.md` para el plan por fases.

## Decisiones tomadas

- **Licencia GPL-3.0-or-later.** El código se escribe desde cero: no copiar ni traducir archivos de otros
  grabadores (en particular santiremedi/clippy). Las ideas y las APIs de Windows sí se pueden usar.
- **Windows 10 y 11.** Lo que solo existe en uno (por ejemplo, quitar el borde amarillo de Windows Graphics
  Capture, que es solo de Windows 11) necesita un camino alternativo en el otro.
- **Laptops primero, sin olvidar escritorio:** laptops con dos tarjetas de video, varios monitores, monitores de
  alta frecuencia (144 Hz o más) y resoluciones altas.
- **Liviano:** la grabadora no tiene interfaz gráfica (solo la bandeja); la ventana de ajustes es un proceso
  aparte. No sumar dependencias si Windows ya trae la API.
- **Modos de captura:** "Todo el escritorio" y "Solo el juego".

## Interfaz

- Textos en español neutro, tratando de "tú" ("Aprieta", "Elige").
- Estilo oscuro tomado de Transcribichad: colores y widgets en `src/ui/tema.rs` y `src/ui/widgets.rs`.
- Para revisar una página sin abrirla a mano: `cargo run --features dev -- --captura-ui <pagina> <archivo.png>`.

## Código

- Comentarios y nombres en español.
- `src/equipo.rs`: detección del equipo (Windows, RAM, batería, tarjetas, monitores, encoders).
- `src/config.rs`: `%APPDATA%\Kairo\config.json`.
- `src/ui/`: ventana de ajustes (`app.rs` arma la ventana, `paginas.rs` cada página).
