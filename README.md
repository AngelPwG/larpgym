# LarpGym

Aplicación personal de gimnasio en español para terminal Linux, escrita en Rust. TUI Ratatui/Crossterm y backend Axum/Tokio independiente; cada instalación tiene su SQLite. Puedes registrar datos con la laptop apagada y sincronizarlos después. No necesita navegador, Docker, VPS ni servicios de pago.

## Primera ejecución

Desde este directorio:

```bash
nix develop -f shell.nix
cargo build --workspace --locked
./target/debug/gym-tui --offline
```

`shell.nix` es el entorno utilizado para desarrollar y validar. Usa los paquetes de tu `<nixpkgs>`; para fijar también Nixpkgs, `nix develop` usa `flake.lock`. `Cargo.lock` fija las dependencias Rust. No se requiere nightly; se validó con Rust estable 1.98.1. El workspace declara Rust 1.88 como mínimo del código, pero no se ha probado ese mínimo con todas las dependencias del lockfile.

En Linux convencional instala Rust estable y certificados CA del sistema y herramientas de compilación C/C++ (GCC, make, pkg-config y CMake), y ejecuta los mismos comandos Cargo. SQLite se compila integrado; Reqwest usa Rustls con certificados CA. El paquete Nix incluye una ruta CA por defecto; en un Linux mínimo puedes configurar `SSL_CERT_FILE` con un bundle válido. OpenSSL solo se usa en los comandos de generación de secretos de esta guía.

La primera apertura añade seis ejercicios editables y el hábito Creatina. **No crea pesos, asistencias, sesiones ni consumos ficticios.** Para experimentar con otra base:

```bash
./target/debug/gym-tui --offline --db /tmp/larpgym-prueba/client.sqlite
```

## Backend y cliente en el mismo equipo para probar

Dentro de `nix develop -f shell.nix`, prepara una vez el secreto:

```bash
umask 077
mkdir -p .dev
# No repitas este comando sobre un token en uso: sería una rotación.
openssl rand -hex 32 > .dev/token
chmod 600 .dev/token
```

Terminal A:

```bash
GYM_BIND=127.0.0.1:8743 \
GYM_SERVER_DB="$PWD/.dev/server.sqlite" \
GYM_TOKEN_FILE="$PWD/.dev/token" \
./target/debug/gym-server
```

Terminal B, en el mismo directorio:

```bash
GYM_SERVER_URL=http://127.0.0.1:8743 \
GYM_TOKEN_FILE="$PWD/.dev/token" \
./target/debug/gym-tui --db "$PWD/.dev/client.sqlite"
```

`8743` es un valor predeterminado configurable, no una afirmación de que esté libre. Si está ocupado elige otro en ambos comandos. `/healthz` no exige token y solo responde `ok`; las rutas de datos siempre lo exigen. `/api/v1/changes` sin autenticación responde 401.

## Conectar con tu laptop

Compila o instala **el servidor en la laptop**, con el toolchain y la arquitectura de esa máquina. La base del servidor debe estar en su disco local. Compila el cliente en el PC. Un binario producido por Nix puede depender de rutas del Nix store; no presupongas que copiar `target/debug` funcionará en otro equipo. Con Nix puedes compilar `nix build` en cada equipo y usar `result/bin/gym-server` o `result/bin/gym-tui`.

1. Genera el token en la laptop. Transfiere una copia al PC por un canal privado, por ejemplo SSH/SCP ya configurado. Usa archivos con permisos `0600`; no lo incluyas en Git ni en la URL.
2. Arranca el backend en la laptop con su `GYM_SERVER_DB` y `GYM_TOKEN_FILE`.
3. Elige acceso privado por Tailscale o HTTPS público por Funnel según [operación y red](docs/operacion.md). El bind es localhost por defecto. Una IP de Tailscale necesita un bind apropiado; Serve/Funnel pueden hacer proxy al localhost.
4. Crea `$XDG_CONFIG_HOME/larpgym/client.toml` (por defecto `~/.config/larpgym/client.toml`) a partir de [examples/client.toml](examples/client.toml):

```toml
timezone = "America/Mazatlan"
sync_interval_secs = 60
server_url = "https://REEMPLAZAR-POR-LA-URL-REAL"
token_file = "/ruta/absoluta/al/token-en-el-PC"
```

Usa la **URL raíz** del backend, sin `/api/v1`, usuario, contraseña, query ni fragmento. Se rechazan redirecciones HTTP para no enviar el secreto a otra ubicación. La URL de Funnel es pública; incluso allí la API comprueba el token. No hay ninguna modificación automática de Tailscale, Funnel, firewall ni servicios existentes.

El cliente usa `~/.local/share/larpgym/client.sqlite` o `$XDG_DATA_HOME/larpgym/client.sqlite`. Permite `database` en TOML, `GYM_CLIENT_DB` y `--db` (en ese orden de prioridad). `--config ARCHIVO` cambia el TOML; `--offline` desactiva la red incluso si está configurada. `GYM_SERVER_URL` y `GYM_TOKEN_FILE` prevalecen sobre el TOML. Las rutas en TOML deben ser absolutas: `~` no se expande.

Sin URL la TUI funciona localmente. Una configuración de conexión o token inválida se muestra en el estado y permite seguir registrando; corrígela y reinicia. Una zona horaria o TOML inválidos producen un error de arranque. `sync_interval_secs` admite 15–3600 segundos.

## Uso con teclado

| Tecla | Acción |
|---|---|
| `1`–`8`, `gt` / `gT`, `Tab` / `Shift+Tab` | Hoy, Peso, Sesiones, Ejercicios, Rutinas, Hábitos, Progreso, Conflictos |
| `j/k` o `↓/↑` | Bajar/subir en listas y selectores |
| `gg` / `G`, `Ctrl+d/u`, `Ctrl+f/b` | Inicio/final, media página, página completa |
| `l` / `Enter` / `→`, `h` / `Esc` / `←` | Abrir / volver o cancelar (fuera de campos de texto) |
| `[` / `]`, `D` | Cambiar fecha / escribir una fecha anterior o futura |
| `s`, `?` o `F1`, `q` | Sincronizar, ayuda, salir |

Cada panel muestra sus atajos. En Hoy: `w` registra peso, `a` asistencia, `n` sesión vacía, `r` desde rutina, `c` creatina y `m` resumen mensual. `Enter` sobre un hábito ofrece completado, pendiente u omitido. Abrir una sesión no la completa. Puedes marcar descanso programado, falta o dejar el día sin registrar.

En una sesión: `a` añade ejercicio, `n` serie, `Enter/e` edita, `d` elimina con confirmación, `r` cambia nombre/notas, `c` completa y marca Entrené, `o` reabre para correcciones. La carga admite cero y hasta tres decimales. Los objetivos no son series realizadas. La referencia anterior se muestra bajo la tabla al seleccionar un ejercicio. Para eliminar la última serie de una sesión completada, reábrela antes.

En catálogos: `n` crea, `e` edita y `a` archiva/restaura. Dentro de una rutina: `a` añade ejercicio, `e` configura objetivos, `J/K` cambia el orden y `r` inicia sesión. Las sesiones guardan sus propios nombres y objetivos; cambiar el catálogo o rutina no reescribe el historial. En hábitos: `t` cambia el estado de la fecha seleccionada y `H` consulta su historial, incluidos hábitos archivados.

Los movimientos de Vim funcionan en listas, selectores, ayuda y comparaciones. `g` espera una segunda tecla (`gg`, `gt`, `gT`); una secuencia inválida se cancela sin ejecutar acciones. `h` vuelve desde un detalle; `q` sale de la aplicación y cierra selectores, ayuda y comparaciones. Los números `1`–`8` son accesos a paneles, no contadores de movimientos.

Formularios: las letras `h/j/k/l`, `g` y `G` escriben texto. `Tab/↑↓` cambia campo, flechas izquierda/derecha y `Home/End` mueven el cursor, `Backspace/Delete` borran, `Ctrl+U` vacía, `Enter` avanza o guarda al final y `Ctrl+Enter` guarda directamente. Los errores conservan la propuesta. Se espera la confirmación SQLite antes de indicar guardado o permitir salir con una escritura en curso.

Los recordatorios usan los días ISO (1=lunes, 7=domingo), hora local opcional `HH:MM` y zona IANA. Solo aparecen en la TUI abierta; no hay un daemon de notificaciones. No se sugieren dosis. Las faltas nunca se infieren de la ausencia de datos.

Progreso muestra cargas máximas **registradas** y repeticiones realizadas con esa carga, además del detalle de series por ejercicio. Incluye series guardadas de sesiones en curso; no calcula 1RM. El gráfico de peso usa muestras ordenadas por fecha, sin inventar mediciones intermedias; la variación es último menos primer peso, no una predicción.

## Persistencia y sincronización

Cada confirmación guarda el registro y una operación pendiente en la misma transacción SQLite, con WAL y `synchronous=FULL`. Un trabajador maneja el almacenamiento y otro la sincronización: ninguna consulta SQLite ni HTTP se ejecuta en el bucle de teclado. Timeouts: 2 segundos de conexión y 8 por petición; reintentos automáticos con esperas de 5, 10, 20… hasta 300 segundos. Se intenta al abrir, periódicamente y con `s`.

Se muestran pendientes, conflictos, última sincronización UTC y errores recuperables. «Guardado localmente» no significa enviado al servidor. «Conectado» describe la última comunicación, no una conexión permanente. Al suspenderse o desconectarse la laptop, el cliente conserva los datos y la cola; vuelve a intentarlo mientras esté abierto. Los demás dispositivos ven lo último sincronizado. Al cerrar la TUI no se realizan sincronizaciones ni recordatorios.

El protocolo usa UUID, identidad persistente, operaciones idempotentes, versiones del servidor, cursor paginado y tombstones. Los días tienen identidad determinista por fecha y cada hábito aparece una vez en el mapa de ese día: no pueden aparecer dos pesos o estados del mismo hábito para esa fecha.

**La unidad de conflicto es el día completo**, o un ejercicio, rutina o hábito. Dos dispositivos editando la misma fecha pueden requerir resolución aunque hayan cambiado campos diferentes. El panel 8 compara ambas propuestas; `L` elige local y `S` servidor, con confirmación. Las propuestas se conservan en `resolutions` dentro de la SQLite local. Ambas elecciones se vuelven a comprobar contra la versión vigente del servidor; si ha cambiado se presenta otro conflicto. No hay fusión por reloj del dispositivo.

Consulta [docs/protocolo.md](docs/protocolo.md) para transacciones, interrupciones, dependencias, límites y recuperación. Nunca copies una SQLite abierta para sincronizarla ni la pongas en una carpeta de red.

## Proyecto y reutilización

```text
crates/gym-core/       Modelos, precisión, validación de relaciones y contratos v1
crates/gym-storage/    Migración SQL, SQLite, outbox, journal y auditoría
crates/gym-sync/       Cliente HTTP y coordinación de push/pull
crates/gym-tui/        Ratatui/Crossterm, formularios, configuración y trabajadores
crates/gym-server/     Axum, autenticación y proceso servidor
examples/             Configuraciones y servicios Linux/NixOS
scripts/tui-smoke.py   Verificación real mediante pseudoterminal
```

Android podrá reutilizar modelos, reglas, representación de pesos, snapshots de sesiones y contratos Serde de `gym-core`, sin Ratatui, rutas Linux ni transporte. `gym-storage` y `gym-sync` separan persistencia y HTTP para adaptar su integración más adelante. No se ha implementado ni probado Android, JNI, Slint ni UniFFI. Las configuraciones de hábitos conservan días y hora para futuras notificaciones locales.

## Validación

En el shell de desarrollo:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --locked
cargo test --workspace --locked
python3 scripts/tui-smoke.py
python3 scripts/server-smoke.py
python3 scripts/sync-smoke.py
python3 scripts/vim-smoke.py
```

Las pruebas Rust abren bases temporales y servidores HTTP en puertos efímeros: verifican persistencia sin servidor, reinicios, segundo cliente, respuestas perdidas, modificaciones/tombstones, conflictos y resoluciones concurrentes, unicidad diaria, autenticación, relaciones, límites, exclusión de sincronizaciones simultáneas y más de dos páginas de cambios. La prueba PTY maneja el binario real por teclado y consulta su base temporal; también verifica recuperación, confirmaciones, 40×12 y restauración del terminal.

`sync-smoke.py` ejecuta dos TUIs y el servidor como procesos independientes. Verifica la subida automática de un peso creado offline, su descarga desde el segundo cliente, el registro durante una interrupción del backend, la reconexión automática y la sincronización manual. Todas sus bases y secretos son temporales.

Los resultados concretos de esta entrega están en [docs/validacion.md](docs/validacion.md). Los ejemplos de despliegue no se han instalado en tu laptop. Copias, restauración, token y servicios están explicados en [docs/operacion.md](docs/operacion.md).

## Límites actuales

- Un usuario y un token compartido, sin roles ni revocación por dispositivo. HTTP no incorpora TLS propio: usa Tailscale privado o HTTPS de Funnel.
- Resolución de conflictos por registro completo; no hay edición combinada campo a campo. La auditoría se conserva en SQLite y sus copias, sin explorador propio en la TUI.
- No hay notificaciones con la TUI cerrada, sincronización en segundo plano tras salir, 1RM estimado ni app móvil.
- Las bases, tombstones, operaciones idempotentes y journal no se compactan. El cliente carga el historial personal en memoria; no está diseñado para múltiples usuarios ni millones de registros.
- Máximo 128 KiB por registro; por día, 30 sesiones y 200 hábitos; por sesión, 100 ejercicios; por ejercicio realizado, 200 series. Hay errores de validación antes de confirmar límites locales.
- Mínimo utilizable: 30×10; se probó interacción en 40×12 y 110×30. No se garantiza compatibilidad con todos los emuladores de terminal.
- Un proceso TUI por base. Cada dispositivo debe empezar con una base nueva; no clones una base cliente para crear otro dispositivo, pues copiarías su identidad y cola.
