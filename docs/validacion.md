# Validación de la primera versión

Ejecutada en Linux/NixOS, con `nix develop -f shell.nix`, Rust 1.98.1 y Cargo 1.98.0. Todas las bases de comprobación son temporales; no se utilizaron datos reales del usuario ni se instaló ningún servicio.

## Resultados comprobados

- `cargo fmt --all -- --check`: correcto.
- `cargo clippy --workspace --all-targets -- -D warnings`: correcto, sin advertencias.
- `cargo build --workspace --locked`: correcto, ambos ejecutables compilados.
- `cargo test --workspace --locked`: **19 pruebas aprobadas**, cero fallos (2 de dominio, 1 de almacenamiento, 9 de integración HTTP y 7 de navegación de la TUI).
- `python3 scripts/vim-smoke.py`: aprobado. Binario real en pseudoterminal: `hjkl`, `gg/G`, `Ctrl+d/u`, `gt/gT`, fecha con `D`, selección de ejercicios, texto literal en formularios, ayuda y regreso. Verifica validación y corrección de un peso inválido, persistencia e integridad SQLite. Las pruebas Rust también cubren scroll y redimensionado en 40×12, secuencias `g` canceladas y separación entre `l` para abrir y `L` para resolver conflictos.
- `python3 scripts/tui-smoke.py`: aprobado. Pseudoterminal con el binario real, teclas y consultas a su SQLite temporal; sin mocks de persistencia.
- `python3 scripts/server-smoke.py`: aprobado. Proceso backend independiente en puerto efímero, token aleatorio temporal, salud, 401, archivo 0600, cierre SIGTERM, logs sin el secreto y backup consistente.
- `python3 scripts/sync-smoke.py`: aprobado. Dos TUIs reales en pseudoterminal y backend independiente: arranque offline, subida y descarga automáticas, identidades distintas, registro con el servidor detenido, reintento progresivo y sincronización manual. Se eliminan `SSL_CERT_FILE` y `SSL_CERT_DIR` del entorno heredado de esos procesos.
- `nix build --no-link`: aprobado en `x86_64-linux` antes de la actualización de navegación Vim, incluidas las 12 pruebas Rust de esa versión. Se corrigió la ausencia de certificados CA del sandbox proporcionando `cacert` y una ruta CA por defecto en el ejecutable empaquetado de la TUI. El intento anterior se interrumpió además por falta de espacio; la compilación se completó después de disponer de espacio suficiente. Los cambios de navegación se compilaron y probaron con Cargo dentro de `shell.nix`; no se ha repetido el empaquetado Nix para esos cambios.
- Los tres scripts (`tui-smoke.py`, `server-smoke.py` y `sync-smoke.py`) también aprobaron contra los ejecutables de ese paquete Nix anterior a los cambios Vim. La prueba con dos TUIs no hereda las variables de certificados del shell de desarrollo.
- `nix flake check --no-build`: aprobado en `x86_64-linux`. `flake.lock` generado.
- `nix eval --impure --json --file scripts/check-nixos.nix`: aprobado; módulo evaluado, sin activación de servicios.
- `python3 scripts/check-systemd.py` desde el shell Nix: aprobado. Se verificó una copia temporal con `ExecStart` apuntando al binario compilado, sin instalarla. La primera ejecución fuera del shell falló por restricciones de sockets del entorno; al ejecutarla dentro del shell funcionó. Hubo una advertencia sobre `cups.socket` del sistema anfitrión, ajena a la unidad LarpGym.

## Criterios de aceptación y evidencia

| Criterio | Comprobación |
|---|---|
| Registrar sin servidor y reiniciar | Integración con conexión rechazada, reapertura de SQLite, misma identidad y outbox; también PTY con reinicio |
| Sincronizar al aparecer el backend | Servidor HTTP real en loopback y SQLite independiente |
| Segundo cliente independiente | Otra ruta SQLite y otro device ID, descarga de documentos reales |
| Respuesta perdida | Operación congelada reenviada sin ack; mismo resultado persistido y una sola entrada de journal |
| Ediciones y eliminaciones | Propagación, tombstone y resurrección con versión posterior |
| Ediciones concurrentes | Ambas propuestas retenidas y elección local/servidor; nueva carrera antes del envío de la resolución |
| Unicidad por fecha/hábito | Peso y estado de hábito divergentes del mismo día, un solo documento canónico; rechazo de ID diario falso |
| Autenticación | Salud pública, GET/POST sin token o token incorrecto rechazados, lectura autenticada aceptada |
| Sesión y asistencia | Series reales, carga cero y transición completada + Entrené; abrir sesión no la completa |
| Varias páginas | 207 cambios, páginas de 100/100/7, cursor final 207, sin omisiones |
| Cambios mientras una respuesta está en vuelo | Varias ediciones locales posteriores conservadas y enviadas con bases correctas |
| Dependencias y validación | Catálogo antes de sesiones; rutina con ejercicio inexistente rechazada; límites HTTP |
| Bootstrap de otro dispositivo | Adopta catálogo modificado del servidor; conserva una edición offline real como conflicto |
| Backup y reinicio del servidor | Restauración conserva journal y resultados idempotentes de operaciones ya enviadas |
| Formulario obsoleto y backup local | Escritura con original obsoleto rechazada; copia recupera identidad, datos y cola |

La prueba PTY también cubre editar peso sin duplicados, hábito Creatina, asistencia explícita, CRUD de series, cancelar y confirmar eliminación, sesión desde rutina con objetivos vacíos de series realizadas, edición de rutina sin alterar snapshots, renombrar ejercicio sin alterar historial y archivar un hábito conservando su registro. Se comprobaron los modos canónico/eco del terminal al salir y navegación tras reabrir en 40×12; el flujo principal se ejecutó en 110×30.

## Lo que no se ha probado en infraestructura real

No se ha desplegado en la laptop del usuario, ni se han modificado Tailscale, Funnel o firewall. No se ha validado conexión por su URL HTTPS real, suspensión física de la laptop, cortes de alimentación reales, todos los emuladores de terminal, otra arquitectura o Android. Los reintentos tras pérdida de respuesta se simulan omitiendo deliberadamente el ack después de una escritura HTTP real; no se afirma haber cortado una red física.

Los tests de base y PTY verifican integridad SQLite; no constituyen una auditoría de seguridad ni una prueba de durabilidad del hardware. La restauración de un servidor antiguo necesita la coordinación descrita en `operacion.md`.

## Repetir

```bash
nix develop -f shell.nix
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --locked
cargo test --workspace --locked
python3 scripts/vim-smoke.py
python3 scripts/tui-smoke.py
python3 scripts/server-smoke.py
python3 scripts/sync-smoke.py
python3 scripts/check-systemd.py
nix flake check --no-build
nix eval --impure --json --file scripts/check-nixos.nix
```

`check-systemd.py` requiere `systemd-analyze` del sistema Linux; no es necesario para ejecutar la aplicación. Para comprobar también el empaquetado reproducible: `nix build --no-link`. Eso compila en el sandbox de Nix y puede descargar dependencias la primera vez.

Los scripts aceptan rutas de ejecutables para verificar una instalación: `python3 scripts/tui-smoke.py /ruta/paquete/bin/gym-tui`, `python3 scripts/server-smoke.py /ruta/paquete/bin/gym-server` y `python3 scripts/sync-smoke.py /ruta/paquete/bin/gym-tui /ruta/paquete/bin/gym-server`. No requieren instalar servicios para hacerlo.
