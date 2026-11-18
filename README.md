# Kairo

Grabador de clips instantáneos para Windows 10 y 11. Graba en segundo plano y, con una tecla, guarda
los últimos segundos como MP4. Está pensado para que el juego no pierda FPS, tanto en laptops como en
PCs de escritorio.

> **Estado: vista previa.** La grabadora ya funciona: graba el monitor en segundo plano y guarda los
> últimos segundos con F9, con el sonido del sistema. Funciona también en laptops con dos tarjetas
> (captura de respaldo), en tarjetas sin encoder (por procesador) y baja la calidad con batería.
> Detecta el juego, puede grabar solo el juego y separa el audio del juego, del resto del sistema y
> del micrófono, y graba aparte los programas que elijas (Discord, música…). Tiene biblioteca de clips.
> Próximo: editor y recorte de clips.

## Cómo se usa

- Abre `kairo.exe`: aparece el ícono de Kairo junto al reloj y se abre la ventana de Kairo en la Biblioteca.
- **F9** guarda los últimos segundos como MP4 (por defecto en `Videos\Kairo`). Suena un aviso corto.
- Clic derecho en el ícono: guardar, pausar, abrir la carpeta de clips, ajustes y salir.
- En Sistema puedes activar **Iniciar con Windows**.
- **Acerca de** muestra la versión exacta del exe (commit y fecha) y la lista de novedades. Debajo del nombre,
  en el menú, siempre se ve la versión corta (por ejemplo `v0.2.0 · b820923`).

## Cómo funciona

```
Monitor ─Desktop Duplication (o Windows Graphics Capture)─▶ BGRA (GPU) ─procesador de video─▶ NV12 (GPU)
                                                                   │
                       encoder H.264 por hardware (o el de Windows por procesador, 1080p30 como máximo)
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
- [x] **Fase 2: laptops primero.** Captura de respaldo (Windows Graphics Capture) para laptops con dos tarjetas,
  elegir con qué tarjeta grabar, grabación por procesador si la tarjeta no tiene encoder, ahorro con batería y
  cola de cuadros (no se saltan cuadros si el encoder se demora). Queda por medir en laptops reales; la
  prioridad baja en la GPU queda como experimento para cuando haya mediciones.
- [x] **Fase 3: audio y juegos.** Detecta el juego (pantalla completa) o se elige a mano; sigue al juego de
  monitor; "Solo el juego" deja el clip en negro con Alt+Tab; audio solo del juego, resto del sistema sin el
  juego y micrófono; pistas separadas opcionales (la mezcla siempre primero); carpeta y nombre por juego; la
  ventana de ajustes crece con la ventana.
- [x] **Fase 3b: audio por programa.** "Otros sonidos": todo lo demás, o solo los programas que elijas con "+"
  (Discord, música…), cada uno con su volumen y su pista. Cada clip guarda al lado un archivo oculto
  (`clip.mp4.kairo`) con el juego y los nombres de las pistas, para la biblioteca y el editor.
- [x] **Fase 4: biblioteca de clips.** Cuadrícula con miniatura, duración, juego, tamaño y fecha; filtro por juego,
  búsqueda y orden; abrir, mostrar en la carpeta, renombrar y mandar a la Papelera. Las miniaturas se generan en
  segundo plano y quedan en caché (`%APPDATA%\Kairo\miniaturas`). Vive en la ventana de Kairo: no pesa nada
  mientras juegas.
- [ ] **Fase 5: editor.** Reproductor dentro de Kairo, recorte y exportar sin recodificar, y quitar pistas
  (por ejemplo, Discord) usando los datos de cada clip.
- [ ] **Fase 6: extras.** Actualizaciones verificadas, instalador y firma digital.

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
- Miniaturas de la biblioteca: `%APPDATA%\Kairo\miniaturas`
- Datos de cada clip (juego y pistas): `clip.mp4.kairo`, oculto al lado del video
- Clips: `Videos\Kairo` (se puede cambiar en la página Clips)

## Licencia

[GPL-3.0-or-later](LICENSE). Puedes usar, estudiar, cambiar y compartir Kairo; si compartes una versión
modificada, tiene que ser con su código y bajo la misma licencia.

La idea se inspira en otros grabadores de repeticiones instantáneas (como
[Clippy](https://github.com/santiremedi/clippy)), pero el código de Kairo está escrito desde cero.
