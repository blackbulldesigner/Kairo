# Kairo

Grabador de clips instantáneos para Windows 10 y 11. Graba en segundo plano y, con una tecla, guarda
los últimos segundos como MP4. Está pensado para que el juego no pierda FPS, tanto en laptops como en
PCs de escritorio.

> **Estado: vista previa.** La grabadora ya funciona: graba el monitor en segundo plano y guarda los
> últimos segundos con F9, con el sonido del sistema. Faltan la captura de respaldo para laptops con dos
> tarjetas (fase 2) y el modo "Solo el juego", el audio separado y el micrófono (fase 3).

## Cómo se usa

- Abre `kairo.exe`: aparece el ícono de Kairo junto al reloj y se abren los ajustes.
- **F9** guarda los últimos segundos como MP4 (por defecto en `Videos\Kairo`). Suena un aviso corto.
- Clic derecho en el ícono: guardar, pausar, abrir la carpeta de clips, ajustes y salir.
- En Sistema puedes activar **Iniciar con Windows**.

## Cómo funciona

```
Monitor ─Desktop Duplication─▶ textura BGRA (GPU) ─procesador de video D3D11─▶ NV12 (GPU)
                                                                   │
                                                 encoder H.264 por hardware (Media Foundation)
                                                                   ▼
Sonido del sistema (WASAPI loopback) ─▶ PCM 48 kHz ─▶  buffer en RAM (últimos N s)
                                                                   │ F9
                                                                   ▼
                                                  MP4 (video copiado tal cual + audio AAC)
```

Un mismo exe tiene tres modos: sin argumentos es la grabadora en la bandeja (y abre los ajustes),
`--fondo` es la grabadora sin ventanas (inicio con Windows) y `--ajustes` es la ventana de ajustes, un
proceso aparte que solo existe mientras está abierta. Se comunican con mensajes de Windows y con
`%APPDATA%\Kairo\estado.json`.

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
- [x] **Fase 1: grabadora mínima.** Ícono en la bandeja, tecla global (F9) para guardar, captura del monitor,
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

Con `--features dev` también hay pruebas de la grabadora sin ventanas:

```powershell
cargo run --features dev -- --prueba-grabar 6 prueba.mp4
cargo run --features dev -- --prueba-leer prueba.mp4 cuadro.png
```

La primera graba 6 segundos (con un tono a la mitad) y guarda el clip; la segunda lo abre como un reproductor,
cuenta cuadros, mide el audio y guarda la imagen del cuadro del medio.

## Archivos del usuario

- Ajustes: `%APPDATA%\Kairo\config.json`
- Estado de la grabadora: `%APPDATA%\Kairo\estado.json`
- Registro (para entender fallas): `%APPDATA%\Kairo\kairo.log`
- Clips: `Videos\Kairo` (se puede cambiar en la página Clips)

## Licencia

[GPL-3.0-or-later](LICENSE). Puedes usar, estudiar, cambiar y compartir Kairo; si compartes una versión
modificada, tiene que ser con su código y bajo la misma licencia.

La idea se inspira en otros grabadores de repeticiones instantáneas (como
[Clippy](https://github.com/santiremedi/clippy)), pero el código de Kairo está escrito desde cero.
