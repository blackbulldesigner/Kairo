# Kairo

Grabador de clips instantáneos para Windows 10 y 11. Graba en segundo plano y, con una tecla, guarda
los últimos segundos como MP4. Está pensado para que el juego no pierda FPS, tanto en laptops como en
PCs de escritorio.

> **Estado: versión 0.1 (vista previa).** Ya están la ventana de ajustes y el diagnóstico del equipo.
> La grabadora todavía no existe: llega en la fase 1.

## Principios

- **Nada dentro del juego:** sin overlay y sin inyectar nada. Así tampoco hay problemas con los anti-trampas.
- **Sin navegador escondido** (nada de Electron ni WebView). El proceso que graba solo tiene el ícono de la
  bandeja; la ventana de ajustes es un proceso aparte que solo existe mientras la tienes abierta.
- **El video nunca sale de la tarjeta de video:** captura, conversión y compresión por hardware.
- **Nada se escribe al disco** hasta que aprietas la tecla.
- **Sin telemetría ni cuentas.**

## Plan

- [ ] **Fase 0: datos de las laptops.** Cada amigo abre Kairo → Sistema → *Copiar diagnóstico* y lo
  comparte, junto con sus FPS sin grabar y con Medal (medidos con PresentMon o CapFrameX).
- [ ] **Fase 1: grabadora mínima.** Ícono en la bandeja, tecla global (F9) para guardar, captura del monitor,
  encoder H.264 por hardware, buffer en RAM y MP4 con el audio del sistema.
- [ ] **Fase 2: laptops primero.** Captura de respaldo para laptops con dos tarjetas de video, elegir con qué
  tarjeta comprimir, modo laptop y prioridad baja en la GPU.
- [ ] **Fase 3: audio y juegos.** Detectar el juego, modo "Solo el juego", audio solo del juego, micrófono,
  pistas separadas y carpetas por juego.
- [ ] **Fase 4: extras.** Recorte rápido de clips, actualizaciones verificadas y firma digital.

**Cómo medimos "no afecta los FPS":** mismo juego, misma escena, laptop conectada al cargador y 3 corridas por
caso: sin grabar, con Medal, con Kairo y con la grabadora de NVIDIA o AMD. Se anotan FPS promedio, "1 % low" y
temperatura. La meta es perder menos que Medal y apuntar a 3 % o menos.

## Compilar

Necesitas [Rust](https://rustup.rs) en Windows.

```powershell
cargo build --release
```

El exe queda en `target\release\kairo.exe`.

Para desarrollo hay una opción que guarda una imagen de una página de los ajustes (sirve para revisar la
interfaz sin abrirla a mano):

```powershell
cargo run --features dev -- --captura-ui sistema captura.png
```

Las páginas son `grabacion`, `audio`, `clips`, `atajos` y `sistema`.

## Archivos del usuario

- Ajustes: `%APPDATA%\Kairo\config.json`
- Clips: `Videos\Kairo` (se puede cambiar en la página Clips)

## Licencia

[GPL-3.0-or-later](LICENSE). Puedes usar, estudiar, cambiar y compartir Kairo; si compartes una versión
modificada, tiene que ser con su código y bajo la misma licencia.

La idea se inspira en otros grabadores de repeticiones instantáneas (como
[Clippy](https://github.com/santiremedi/clippy)), pero el código de Kairo está escrito desde cero.
