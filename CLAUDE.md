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
- Estilo "Estudio": pestañas arriba (con el estado y la actualización como píldoras a la derecha), el contenido a
  todo el ancho, y los colores oscuros de siempre (`src/ui/tema.rs`; widgets en `src/ui/widgets.rs`).
- Animaciones suaves con `src/ui/animar.rs` (`suave` y `hacia`: ease-out, piden cuadros solo mientras algo se
  mueve). Hover y selección se animan en cada widget; la página nueva entra con fundido; la barra de guardar sube.
- `KAIRO_CAPTURA_HOVER=<nombre del clip>` muestra esa tarjeta de la biblioteca con el mouse encima en la captura.
- `KAIRO_CAPTURA_SELECCION=<n>` selecciona los primeros `n` clips de la biblioteca en la captura (para ver la barra de selección).
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
- `src/grabadora/`: `captura.rs` (Desktop Duplication + NV12 en GPU; `mouse.rs` dibuja el puntero encima con dos
  shaders mínimos si se pide "Mostrar el mouse"), `encoder.rs` (H.264 por hardware,
  asíncrono), `audio.rs` (WASAPI loopback), `buffer.rs` (RAM), `mp4.rs` (sink writer), `mod.rs` (hilos).
  Video y audio usan el mismo reloj (QPC en 100 ns, `reloj.rs`).
- `src/equipo.rs`: detección del equipo (Windows, RAM, batería, tarjetas, monitores, encoders).
- `src/config.rs`: `%APPDATA%\Kairo\config.json`. `src/estado.rs`: `estado.json` (grabadora → ajustes).
- `src/ui/`: ventana de Kairo (`app.rs` arma la ventana, `paginas.rs` las páginas de ajustes,
  `biblioteca.rs` + `pagina_biblioteca.rs` la biblioteca de clips y sus miniaturas).
- `src/datos_clip.rs`: datos de cada clip (`clip.mp4.kairo`, oculto): juego, nombres de las pistas, formato.
- `src/pistas_mp4.rs`: marca la mezcla como la pista de audio que tocan todos los reproductores.
- `src/limpieza.rs`: limpieza automática (después de guardar, en la grabadora, con prioridad baja). Solo clips con
  datos de Kairo (`version >= 1`) y sin estrella; `cargo test` prueba el plan. `--captura-ui guardado-limpieza`.
- `src/medios/` (solo la ventana, nunca la grabadora): `reproductor.rs` (hilo de video que achica a 1280×720 y
  hilo de audio con WASAPI que mezcla las pistas con su volumen y lleva el reloj), `exportar.rs` (recorte sin
  recodificar el video, audio rearmado por pista) y `comprimir.rs` (recodificar con el sink writer). La interfaz
  es `src/ui/visor.rs` (Ver, Editar, Comprimir), dentro de la Biblioteca. `discord.rs`: la copia de menos de 20 MB,
  en `<carpeta de clips>\Para Discord` (categoría propia en la Biblioteca, fuera de "Todos"). Se borra sola después de
  `discord_minutos` (15, 60 o 1440): `limpieza::borrar_copias_discord`, en la grabadora cada 5 min y en la ventana
  al revisar la Biblioteca. La limpieza automática no cuenta esa carpeta.
- `src/aviso.rs` (grabadora): el cartelito al guardar un clip o una foto. Ventana `WS_EX_LAYERED` con
  `SetLayeredWindowAttributes` (transparencia pareja) y `WDA_EXCLUDEFROMCAPTURE` para que no salga en los clips: con
  `UpdateLayeredWindow` (transparencia por píxel, para sombra y esquinas redondeadas) Windows rechaza la exclusión
  ("no hay memoria"), probado. Por eso es rectangular. Trabaja en píxeles reales (DPI por monitor) solo en ese rato.
- `src/grabadora/foto.rs`: la foto del atajo "Sacar foto". El hilo de video copia `ultimo` (el cuadro BGRA) a una
  textura que lee el procesador solo cuando se pide; la bandeja la guarda en PNG en otro hilo, con prioridad baja.
  La Biblioteca muestra los PNG como fotos (`Clip::foto`).
- `src/compartir.rs` (solo la ventana): copiar archivos al portapapeles (CF_HDROP), como "Copiar" en el Explorador.
  (Arrastrar clips afuera de la ventana se probó y se sacó: la persona prefiere clic derecho → Copiar para Discord.)
- `src/ui/logos.rs`: logos que no trae Segoe MDL2 (el de Discord), dibujados una vez como textura chica.

## Probar

- No hay ffprobe: `--prueba-grabar` y `--prueba-leer` (feature `dev`) graban y verifican un MP4 de verdad.
- Caminos que esta PC no usa sola (escritorio con una NVIDIA): `KAIRO_CAPTURA=respaldo` fuerza Windows Graphics
  Capture y `KAIRO_PROCESADOR=1` el encoder por procesador. Sirven también para diagnosticar en otra PC.
- Las pruebas graban la pantalla real: borrar los clips y las imágenes de prueba al terminar.
- `--prueba-grabar <s> <mp4> tono juego` hace que el propio proceso sea "el juego" (`KAIRO_JUEGO_PID`) con
  pistas separadas: el tono tiene que quedar en la pista del juego y no en la del sistema. `solo-juego` sin
  juego adelante tiene que dar un clip negro. `--prueba-juego <s>` muestra la detección y los programas abiertos.
  `programa` suma `kairo.exe` como programa elegido: el tono tiene que quedar en su pista y en la mezcla.
  `hevc` graba en HEVC (`av1` cae a H.264: el MP4 de Windows 10 acepta la pista AV1 pero no termina el archivo, ni
  con el sequence header ni con av1C). `--prueba-codecs` lista encoders y decodificadores; `--prueba-recorte <mp4>`
  recorta sin decodificar (sirve con HEVC sin la extensión). Para ver un HEVC sin la extensión: VLC.
  `largo` deja el clip normal en 5 s y el largo en lo pedido (el buffer tiene que guardar lo del largo).
  `mouse` graba con "Mostrar el mouse" (con `--prueba-tira` a la altura del puntero se ve si quedó dibujado).
  Ojo: `con.mp4` no sirve de nombre de prueba (CON es un nombre reservado de Windows).
  `avanzado` graba con opciones avanzadas raras (CBR, cuadro clave cada 2 s, perfil Básico, audio a 128 kbps):
  `--prueba-leer` muestra el perfil y el bitrate de cada pista para comprobarlo.
- `--captura-ui <pagina> <png> [1700x950]` saca la página con ese tamaño de ventana; `grabacion-juego` la
  muestra con "Solo el juego" puesto y `audio-programas` con "Solo los programas que elijas" y
  `grabacion-avanzado` con las opciones avanzadas abiertas (sin guardarlo).
  `biblioteca` espera a que estén las miniaturas (como mucho ~10 s).
- Pistas de audio del MP4: Media Foundation (Windows y Kairo) las enumera al revés del archivo y toca la que
  enumera primera, así que la mezcla se agrega última. VLC, navegadores y celulares tocan la primera del archivo
  o la habilitada: por eso `pistas_mp4::marcar_mezcla` deja todas en el grupo alternativo 1 y solo la mezcla
  habilitada (la biblioteca repara los clips viejos). `--marcar-mezcla <mp4>` lo hace a mano. Para ver qué toca
  VLC: `vlc -I dummy --sout=#transcode{vcodec=none,acodec=s16l}:std{access=file,mux=wav,dst=x.wav} <mp4>
  vlc://quit` y medir el WAV.
- Para hablarle a la grabadora desde PowerShell con `FindWindow`, pasar `[NullString]::Value` como título:
  `$null` se convierte en texto vacío y no la encuentra.
- Mientras alguien prueba el exe de `target\release`, compilar solo en debug (o `cargo check --release`).
- El instalador se prueba con `KAIRO_PRUEBA_INSTALACION=<carpeta>` (todo queda ahí y usa otra clave del
  registro): `--prueba-instalar`, `--prueba-desinstalar` y `--captura-instalador <png>`. Sin esa variable
  instala en la PC de verdad: nunca probar así. Al terminar, borrar la carpeta de prueba.
- `--prueba-actualizacion [api]` busca el release de Kairo y, con la API de otro repositorio (por ejemplo
  `https://api.github.com/repos/cli/cli/releases/latest`), baja su archivo más chico y comprueba el SHA-256.
  `KAIRO_CAPTURA_VERSION=0.9.0` hace que `--captura-ui` muestre el aviso de una versión nueva.
- `--prueba-medios <clip>` reproduce, busca, recorta y comprime ese clip y revisa cada resultado (borrar después
  el clip, el "(editado)" y el "(comprimido)"). `KAIRO_CAPTURA_CLIP=<mp4>` + `KAIRO_CAPTURA_MODO=ver|editar|comprimir`
  con `--captura-ui biblioteca` saca el visor con ese clip (`KAIRO_CAPTURA_RECORTE=2,6` pone un recorte).
  `--prueba-medios` también manda 100 búsquedas seguidas sonando: tiene que quedar en la última.
- `--prueba-discord <clip>` deja el clip listo para Discord (achica la copia si pesa más de 20 MB), revisa el peso
  y lo copia al portapapeles (`Get-Clipboard -Format FileDropList` lo muestra). Cambia el portapapeles de la persona.
  `KAIRO_CAPTURA_DISCORD=<n>` con `--captura-ui biblioteca` muestra la tarjeta n achicándose.
- `--prueba-discord <clip> sin-copiar` no toca el portapapeles (úsalo si la persona está usando la PC). La copia queda
  en `<carpeta de clips>\Para Discord`: borrarla después.
- `--prueba-aviso <png> [mostrar]` dibuja los avisos (clip, foto y error, a 100 % y 150 %) en un PNG; con `mostrar`
  saca uno de verdad 3 s y muestra la afinidad (17 = fuera de las capturas).
- `--prueba-foto <png>` arranca la grabadora, pide una foto y la guarda (muestra tamaño, tiempos y brillo; 0 = negra).
  Es la pantalla real: no mirarla, borrarla (y su `.kairo`).
- No mover el mouse de Windows con scripts para probar (la persona puede estar usando la PC). Para lo que se hace
  con el mouse, un mouse de mentira solo adentro de egui: `KAIRO_PRUEBA_MARCO=x0,y0,x1,y1[,soltar|menu|clic]` con
  `--captura-ui biblioteca` arrastra un recuadro de selección (en puntos de la ventana; con `soltar` lo suelta) o,
  con `menu`, hace clic derecho en (x0, y0) y con `clic`, un clic izquierdo (por ejemplo, en un chip). Con 1280x780, la primera fila de tarjetas va de y≈253 a 483.
- Prueba de FPS sin jugar, Kairo contra Medal: `scripts\comparar-fps.ps1 [-Segundos 60] [-SinMedal]` (compila en
  `target\medir` con `--release --features dev`). Corre `--prueba-fps calibrar` (carga para ~200 FPS) y
  `--prueba-fps medir <s> <carga> <json>` (`src/prueba_fps.rs`: escena D3D11 pesada, igual en cada ronda, sin límite
  de FPS, y el uso de procesador, tarjeta, encoder y RAM de `kairo.exe` y de lo que corre desde `%LOCALAPPDATA%\Medal`,
  con los contadores de la tarjeta en inglés porque Windows los traduce). Kairo graba con `--grabar-fondo <s>` (tus
  ajustes, todo el escritorio, sin guardar). Tapa la pantalla: solo con la persona de acuerdo. Medal tiene que
  grabar la pantalla completa y abrirlo/cerrarlo lo hace la persona (el script pregunta).
- Imágenes para compartir: `scripts\promo.ps1` arma `promo\kairo-1.0-*.png` desde `promo\fuente\*.html` con Edge sin
  ventana (Edge sin ventana no aplica `backdrop-filter` ni desenfoca bien elementos que se salen: los nombres se
  desenfocan en los recursos y los resplandores van en el fondo). Los recursos (`promo\recursos`, fuera de git) salen de
  `--exportar-icono <png> <n>`, `--exportar-aviso <png> <escala> clip|foto|error <título> <detalle>`,
  `--exportar-cuadro <mp4> <s> <png>` y `--captura-ui` con `KAIRO_CAPTURA_ATAJO=F9` (para no mostrar el atajo de la
  persona). Desenfocar los nombres de otros jugadores antes de publicar.
- `--prueba-tira <mp4> <png> <y> <alto> <n>` apila una franja de `n` cuadros del clip: sirve para ver un video que
  manda la persona (por ejemplo, cómo se mueve una barra) sin abrirlo entero. Borrar la imagen después.
