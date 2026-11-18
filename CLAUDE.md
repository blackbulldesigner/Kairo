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

## Versiones y novedades

- Cada cambio va en su propio commit con un título claro en español, pensado para quien usa la app: ese
  título aparece en **Acerca de → Novedades** (lo arma `build.rs` con `git log`).
- `build.rs` guarda en el exe el commit, la rama, la fecha y si había cambios sin commit (`src/version.rs`);
  el menú muestra `v0.2.0 · <commit>` y un "+" en amarillo si el exe tiene cambios sin commit.
- Subir `version` en `Cargo.toml` al terminar cada fase.
- Al contarle un cambio a la persona, decirle el commit para que lo compare con Acerca de.

## Interfaz

- Textos en español neutro, tratando de "tú" ("Aprieta", "Elige").
- Estilo oscuro tomado de Transcribichad: colores y widgets en `src/ui/tema.rs` y `src/ui/widgets.rs`.
- Para revisar una página sin abrirla a mano: `cargo run --features dev -- --captura-ui <pagina> <archivo.png>`.

## Código

- Comentarios y nombres en español.
- `src/main.rs`: modos del exe (sin argumentos = bandeja; `--fondo`; `--ajustes`).
- `src/bandeja.rs`: grabadora en la bandeja (ícono, menú, atajos globales, mensajes `WM_APP+n` de los ajustes).
- `src/grabadora/`: `captura.rs` (Desktop Duplication + NV12 en GPU), `encoder.rs` (H.264 por hardware,
  asíncrono), `audio.rs` (WASAPI loopback), `buffer.rs` (RAM), `mp4.rs` (sink writer), `mod.rs` (hilos).
  Video y audio usan el mismo reloj (QPC en 100 ns, `reloj.rs`).
- `src/equipo.rs`: detección del equipo (Windows, RAM, batería, tarjetas, monitores, encoders).
- `src/config.rs`: `%APPDATA%\Kairo\config.json`. `src/estado.rs`: `estado.json` (grabadora → ajustes).
- `src/ui/`: ventana de ajustes (`app.rs` arma la ventana, `paginas.rs` cada página).

## Probar

- No hay ffprobe: `--prueba-grabar` y `--prueba-leer` (feature `dev`) graban y verifican un MP4 de verdad.
- Para hablarle a la grabadora desde PowerShell con `FindWindow`, pasar `[NullString]::Value` como título:
  `$null` se convierte en texto vacío y no la encuentra.
- Mientras alguien prueba el exe de `target\release`, compilar solo en debug (o `cargo check --release`).
