# Operación en la laptop

Los comandos son ejemplos para adaptar. No se ha instalado ni cambiado ningún servicio, regla de firewall o configuración de Tailscale existente. Separa el PC cliente de la laptop servidor; no necesitan compartir configuración ni arquitectura.

## Configuración del backend

| Variable | Valor predeterminado |
|---|---|
| `GYM_BIND` | `127.0.0.1:8743` |
| `GYM_SERVER_DB` | `$XDG_DATA_HOME/larpgym-server/server.sqlite`, o `~/.local/share/larpgym-server/server.sqlite` |
| `GYM_TOKEN_FILE` | Obligatoria, ruta a un archivo secreto |

Elige un puerto disponible. La base no contiene el token y se crea con permisos `0600`; el directorio de datos nuevo tiene `0700`. No reutilices la SQLite del cliente en el servidor. Las migraciones se aplican al abrir; una versión de esquema futura se rechaza.

Genera al menos 32 bytes aleatorios: `umask 077; openssl rand -hex 32 > token`. Los archivos de secretos deben tener permisos `0600` (las credenciales systemd de solo lectura también se aceptan). No incluyas el valor en argumentos, TOML, unidades o el Nix store. Los ejemplos usan `LoadCredential`, que entrega al usuario dedicado una copia restringida, y `%d/gym-token` como ruta. Ese mecanismo está documentado por [systemd](https://github.com/systemd/systemd/blob/main/docs/CREDENTIALS.md?plain=1).

Para rotar: crea otro archivo de token, reemplaza de forma controlada el del servidor, reinicia el servicio y actualiza las copias de los clientes. Reinicia las TUIs para que lo lean. Los clientes con el anterior reciben 401 y mantienen sus cambios pendientes. No existe recuperación ni token alternativo de emergencia.

`GET /healthz` permite comprobar que responde el proceso sin mostrar datos personales. No demuestra que el token, la persistencia ni toda la sincronización funcionen. El proceso registra arranque, dirección de escucha, cierre y fallos internos de almacenamiento sin cuerpos ni cabeceras de autorización. Las validaciones recuperables se muestran en el cliente como conflictos.

## Acceso privado por Tailscale

Puedes escuchar en la **IP real de Tailscale de la laptop** y en el puerto que elijas, y configurar en el PC `server_url = "http://IP_TAILSCALE_REAL:PUERTO"`. La conexión queda dentro de tu tailnet; exige igualmente el token. Verifica tus ACL y firewall existentes; este proyecto no los modifica.

Otra opción mantiene el backend en localhost y usa Serve como proxy HTTPS privado. Consulta primero `tailscale serve status` y `tailscale funnel status`, y elige un puerto HTTPS que no esté ocupado por otro proyecto. Plantilla para adaptar, no para ejecutar literalmente:

```text
tailscale serve --bg --https=PUERTO_HTTPS_ELEGIDO http://127.0.0.1:PUERTO_BACKEND
```

Copia en el cliente la URL real anunciada. Serve limita el acceso a la tailnet; no uses un puerto configurado como Funnel si buscas privacidad. Consulta la [referencia oficial de Serve](https://tailscale.com/docs/reference/tailscale-cli/serve) para las opciones de tu versión.

## HTTPS público por Funnel

Mantén `GYM_BIND=127.0.0.1:PUERTO_BACKEND`. Revisa antes `tailscale funnel status` y la configuración de otros proyectos. Plantilla adaptable:

```text
tailscale funnel --bg --https=PUERTO_HTTPS_COMPATIBLE_Y_LIBRE http://127.0.0.1:PUERTO_BACKEND
```

Usa únicamente puertos admitidos por tu versión y política de Funnel y evita sustituir una ruta o puerto de otro servicio. Configura `server_url` con la URL HTTPS real que anuncia Funnel. La URL es **pública en internet**, no privada de Tailscale; la API sigue exigiendo autenticación. El backend no implementa TLS propio. No utilices `reset` para instalar esta aplicación junto a tus otros servicios. Opciones y requisitos: [referencia oficial de Funnel](https://tailscale.com/docs/reference/tailscale-cli/funnel).

Este cliente espera que el backend esté en la raíz de esa URL. No admite alojarlo bajo un prefijo como `/gym`; usa un puerto/endpoint separado o un proxy propio que presente la API desde la raíz. No se inventa aquí un dominio, IP ni puerto libre de tu infraestructura.

## Servicio en NixOS

Compila para la arquitectura de la laptop. El flake exporta el paquete y `nixosModules.default`; se puede importar como input desde tu propia configuración. Ejemplo conceptual dentro de tu módulo existente:

```nix
{ inputs, pkgs, ... }: {
  imports = [ inputs.larpgym.nixosModules.default ];
  services.larpgym = {
    enable = true;
    package = inputs.larpgym.packages.${pkgs.stdenv.hostPlatform.system}.default;
    bind = "127.0.0.1:8743"; # cambia el puerto si está ocupado
    tokenFile = "/etc/larpgym/token"; # string, nunca ./token ni builtins.readFile
  };
}
```

En tu flake añade un input `larpgym` que apunte a tu copia del proyecto o a su repositorio cuando lo publiques; este proyecto no asume que ya tengas uno remoto. Antes de activar, crea `/etc/larpgym/token` fuera del store, propiedad `root:root`, modo `0600`. La integración con tu configuración y `nixos-rebuild` debes ejecutarla en la laptop después de revisarla.

El módulo crea usuario/grupo `larpgym`, `/var/lib/larpgym` persistente de modo `0700`, reinicio ante fallo y endurecimiento systemd. No abre puertos ni habilita Tailscale. Consulta [examples/nixos-module.nix](../examples/nixos-module.nix). Para evaluar sin activar nada:

```bash
nix develop -f shell.nix -c nix flake check --no-build
nix develop -f shell.nix -c nix eval --impure --json --file scripts/check-nixos.nix
```

## Servicio systemd en Linux convencional

Compila `cargo build --release --locked --workspace` en esa laptop o mediante su Nix. Para un binario Cargo convencional, adapta estas rutas y comandos administrativos a la distribución; `nologin` puede estar en otra ubicación:

```bash
sudo useradd --system --home-dir /var/lib/larpgym --shell /usr/sbin/nologin larpgym
sudo install -m 0755 target/release/gym-server /usr/local/bin/gym-server
sudo install -d -m 0700 /etc/larpgym
# Copia el secreto ya generado; no imprimas su contenido.
sudo install -o root -g root -m 0600 /ruta/privada/token /etc/larpgym/token
sudo install -m 0644 examples/server.env /etc/larpgym/server.env
sudo install -m 0644 examples/larpgym.service /etc/systemd/system/larpgym.service
# Edita server.env para elegir dirección/puerto y revisa ExecStart antes de activar.
sudo systemctl daemon-reload
sudo systemctl enable --now larpgym
sudo systemctl status larpgym
journalctl -u larpgym
```

Si el usuario ya existe, utiliza el existente en lugar de repetir `useradd`. Con un paquete Nix usa la ruta del paquete en `ExecStart`; no copies un binario Nix aislado fuera de su cierre. Las unidades necesitan una versión de systemd compatible con `LoadCredential`. `StateDirectory` crea el directorio persistente y permite su escritura con `ProtectSystem=strict`.

SIGINT/SIGTERM cierran el servidor ordenadamente. No hay servicio global para el cliente: el almacenamiento y la red se ejecutan solo dentro del proceso TUI.

## Copias de seguridad consistentes

La sincronización no sustituye a un backup. Tanto servidor como clientes contienen datos personales; conserva las copias con permisos restringidos. El token se respalda por separado, si deseas conservarlo. La SQLite incluye versiones, journal, identidad, operaciones pendientes y auditoría.

Los comandos incorporados usan la API de backup de SQLite y obtienen una copia consistente incluso con WAL activo. No sobrescriben un destino existente. Para un cliente, cierra la TUI o usa el mismo origen si está abierta:

```bash
umask 077
./target/debug/gym-tui --db /ruta/client.sqlite --backup /ruta/copias/client-2026-10-14.sqlite
./target/debug/gym-server backup /ruta/server.sqlite /ruta/copias/server-2026-10-14.sqlite
sqlite3 /ruta/copias/server-2026-10-14.sqlite 'PRAGMA integrity_check;'
```

Con systemd y sus directorios restringidos, ejecuta el comando del servidor como el usuario del servicio, usando una carpeta de copias donde pueda escribir, o administra la copia como root. Sustituye el ejecutable por el instalado. El origen debe existir; un destino existente o un backup fallido genera un error. Verifica `integrity_check` y guarda copias fechadas fuera del directorio de trabajo. La operación no respalda automáticamente TOML ni secretos.

No hagas `cp server.sqlite` mientras el proceso escribe: puede faltar información del WAL. Como alternativa, detén el servicio y respalda todo el conjunto de archivos, o utiliza `sqlite3 ORIGEN '.backup DESTINO'` y verifica el resultado.

## Restauración coordinada

1. Detén el backend y todas las TUIs. Respáldalos primero mediante los comandos anteriores. Conserva las bases cliente con pendientes y auditorías; no descartes cambios offline sin recuperarlos.
2. Verifica la copia elegida con `PRAGMA integrity_check`. Mueve el directorio de datos actual a una carpeta de resguardo **con todos sus archivos SQLite, `-wal` y `-shm`**, con los procesos detenidos. No pongas un backup sobre un WAL viejo.
3. Crea un directorio limpio, instala allí la SQLite de backup con el nombre configurado y permisos `0600`, y restaura la propiedad del usuario dedicado. Arranca el backend y comprueba salud y sincronización en una base cliente de prueba nueva.
4. Si restauraste una copia antigua del servidor, crea bases cliente nuevas para todos los dispositivos (por ejemplo cambia `--db` o `database`). **No reutilices sus cursores viejos contra el journal restaurado**: podría haber ramas divergentes incluso si el cursor no supera el máximo actual. Conserva las bases anteriores offline para recuperar manualmente datos posteriores al backup y propuestas no enviadas, usando la TUI contra la copia antigua con `--offline` y registrándolos explícitamente en la nueva. No hay importador automático de ramas restauradas en esta versión.
5. Para restaurar únicamente un cliente del **mismo dispositivo** contra un servidor que no se ha rebobinado, puedes recuperar su backup con identidad y cola; el protocolo reenviará operaciones idempotentemente. No lo uses como clon de otro dispositivo ni ejecutes la copia y el original a la vez.

Restaurar un backup anterior no recupera por sí mismo cambios posteriores a su fecha. Mantén el resguardo hasta reconciliar esos datos. Una recuperación simple sin cambios posteriores puede arrancar directamente con el backup verificado y clientes nuevos.

## Laptop suspendida o desconectada

Mientras duerme, está apagada o pierde conexión, el servidor no responde. La TUI arranca sin esperarlo, guarda cada confirmación localmente y muestra el error y la cola. Al despertar la laptop, los intentos periódicos o `s` reanudan la sincronización. Reiniciar el backend no necesita reconstruir datos del cliente y no borra la cola. Ninguna opción de este proyecto cambia la política de suspensión de tu laptop.
