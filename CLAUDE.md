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

## Después de cada cambio

- La persona usa Kairo en esta PC. Después de cada cambio (con commit o sin él) hay que cerrarlo, compilar la
  versión final y volver a abrirlo: `powershell -ExecutionPolicy Bypass -File scripts\actualizar.ps1`.
  Después, decirle qué versión quedó abierta (lo que muestra Acerca de). Si Kairo está instalado
  (`%LOCALAPPDATA%\Programs\Kairo`), el script actualiza esa copia (`--instalar --sin-ventana`) y abre esa.

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
- `src/main.rs`: modos del exe (sin argumentos = bandeja; `--fondo`; `--ajustes`; el instalador si el exe se
  llama `KairoSetup…` o con `--instalar`; `--desinstalar`).
- `src/instalacion.rs` + `src/ui/instalador.rs`: instalar solo para el usuario (sin administrador), accesos,
  entrada en "Aplicaciones instaladas" (HKCU) y desinstalar. `build.rs` pone el ícono y la versión en el exe.
- `src/actualizacion.rs` (+ `src/red.rs`, WinHTTP y SHA-256 de Windows; `src/ui/actualizar.rs`): busca el último
  Release de GitHub, avisa (globo de la bandeja, menú, ventana) y actualiza con el KairoSetup verificado.
  Publicar es `scripts\publicar.ps1`: solo cuando la persona lo pida (es público).
- `src/bandeja.rs`: grabadora en la bandeja (ícono, menú, atajos globales, mensajes `WM_APP+n` de los ajustes).
- `src/grabadora/`: `captura.rs` (Desktop Duplication + NV12 en GPU), `encoder.rs` (H.264 por hardware,
  asíncrono), `audio.rs` (WASAPI loopback), `buffer.rs` (RAM), `mp4.rs` (sink writer), `mod.rs` (hilos).
  Video y audio usan el mismo reloj (QPC en 100 ns, `reloj.rs`).
- `src/equipo.rs`: detección del equipo (Windows, RAM, batería, tarjetas, monitores, encoders).
- `src/config.rs`: `%APPDATA%\Kairo\config.json`. `src/estado.rs`: `estado.json` (grabadora → ajustes).
- `src/ui/`: ventana de Kairo (`app.rs` arma la ventana, `paginas.rs` las páginas de ajustes,
  `biblioteca.rs` + `pagina_biblioteca.rs` la biblioteca de clips y sus miniaturas).
- `src/datos_clip.rs`: datos de cada clip (`clip.mp4.kairo`, oculto): juego, nombres de las pistas, formato.

## Probar

- No hay ffprobe: `--prueba-grabar` y `--prueba-leer` (feature `dev`) graban y verifican un MP4 de verdad.
- Caminos que esta PC no usa sola (escritorio con una NVIDIA): `KAIRO_CAPTURA=respaldo` fuerza Windows Graphics
  Capture y `KAIRO_PROCESADOR=1` el encoder por procesador. Sirven también para diagnosticar en otra PC.
- Las pruebas graban la pantalla real: borrar los clips y las imágenes de prueba al terminar.
- `--prueba-grabar <s> <mp4> tono juego` hace que el propio proceso sea "el juego" (`KAIRO_JUEGO_PID`) con
  pistas separadas: el tono tiene que quedar en la pista del juego y no en la del sistema. `solo-juego` sin
  juego adelante tiene que dar un clip negro. `--prueba-juego <s>` muestra la detección y los programas abiertos.
  `programa` suma `kairo.exe` como programa elegido: el tono tiene que quedar en su pista y en la mezcla.
  `avanzado` graba con opciones avanzadas raras (CBR, cuadro clave cada 2 s, perfil Básico, audio a 128 kbps):
  `--prueba-leer` muestra el perfil y el bitrate de cada pista para comprobarlo.
- `--captura-ui <pagina> <png> [1700x950]` saca la página con ese tamaño de ventana; `grabacion-juego` la
  muestra con "Solo el juego" puesto y `audio-programas` con "Solo los programas que elijas" y
  `grabacion-avanzado` con las opciones avanzadas abiertas (sin guardarlo).
  `biblioteca` espera a que estén las miniaturas (como mucho ~10 s).
- El sink writer de MP4 deja las pistas en orden inverso al que se agregan: la mezcla se agrega última para
  quedar primera (es la que tocan los reproductores).
- Para hablarle a la grabadora desde PowerShell con `FindWindow`, pasar `[NullString]::Value` como título:
  `$null` se convierte en texto vacío y no la encuentra.
- Mientras alguien prueba el exe de `target\release`, compilar solo en debug (o `cargo check --release`).
- El instalador se prueba con `KAIRO_PRUEBA_INSTALACION=<carpeta>` (todo queda ahí y usa otra clave del
  registro): `--prueba-instalar`, `--prueba-desinstalar` y `--captura-instalador <png>`. Sin esa variable
  instala en la PC de verdad: nunca probar así. Al terminar, borrar la carpeta de prueba.
- `--prueba-actualizacion [api]` busca el release de Kairo y, con la API de otro repositorio (por ejemplo
  `https://api.github.com/repos/cli/cli/releases/latest`), baja su archivo más chico y comprueba el SHA-256.
  `KAIRO_CAPTURA_VERSION=0.9.0` hace que `--captura-ui` muestre el aviso de una versión nueva.
