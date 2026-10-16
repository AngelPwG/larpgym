# Protocolo y garantías de datos

## Registros y precisión

`Document` contiene ID, tombstone (`deleted`) y una entidad tipada. `Revision` añade versión entera positiva y `updated_at` UTC asignados por el servidor. Un registro local nuevo usa versión 0 hasta recibir confirmación. Ningún orden ni comparación de versiones depende del reloj del dispositivo.

Las entidades de catálogo (ejercicio, rutina, hábito), sesiones, ejercicios realizados, series y operaciones reciben UUID v4 en el cliente. Los días usan UUID v5 en un namespace fijo y su fecha ISO explícita. Los ejercicios iniciales y Creatina también usan IDs deterministas para converger entre dispositivos de este único usuario.

Un día agrupa peso opcional, asistencia explícita, mapa `habit_id → estado` y sesiones. No hay otra tabla donde pueda crearse un segundo peso para esa fecha. Un ID diario que no corresponde a su fecha se rechaza. JSON duplicado en mapas se normaliza a una entrada al deserializar y nunca puede persistir múltiples estados. Los cambios de fecha se hacen abriendo otro día; no se mueve una sesión entre fechas en esta versión.

Todos los pesos son gramos enteros (`u32`) presentados en kg, con precisión de 0.001 kg. Cargas de 0 kg son válidas; peso corporal debe ser mayor que cero. No se utiliza flotante para identidad, versiones ni pesos. Instantes UTC y fechas locales son tipos distintos. La zona IANA del cliente gobierna «hoy» y la interpretación de recordatorios.

Una rutina guarda orden e IDs de ejercicios y objetivos opcionales. Crear una sesión copia los nombres y objetivos; las series permanecen vacías hasta registrarse. Completar exige al menos una serie real y guarda `Completed` y asistencia `Trained` en el mismo documento y transacción. No puede guardarse un día con sesión completada y otra asistencia. Reabrir o eliminar una sesión no borra automáticamente la asistencia histórica; puedes corregirla explícitamente cuando ya no haya sesiones completadas.

## Transacción local y cola

La migración `migration-001.sql`, aplicada mediante `user_version`, crea vistas materializadas en JSON tipado, sombra remota, outbox ordenada, conflictos, auditoría y journal del servidor. El rol almacenado impide usar accidentalmente una base cliente como servidor. La identidad del dispositivo se genera una vez y persiste en `metadata`.

Cada confirmación:

1. Inicia `BEGIN IMMEDIATE`.
2. Comprueba que el documento original del formulario sigue vigente, valida campos y grafo de relaciones.
3. Escribe el documento local y añade una operación UUID a la outbox.
4. Confirma ambas cosas juntas con SQLite WAL y `synchronous=FULL`.

Si falla, la TUI conserva el formulario y muestra el error. No anuncia guardado antes de la confirmación. Un cierre o reinicio conserva la cola y sesiones en curso. No se promete recuperar teclas no confirmadas con Enter.

Cada operación guarda `device_id`, `id`, `base_version`, `strict_version` y el documento completo. Puede haber varias operaciones de un registro: cada una depende de su predecesora local. Al preparar el envío se congela su contenido. El ack solo ajusta la versión base de su sucesora **no enviada**, usando la versión aceptada de esa predecesora. Descargar una edición de otro dispositivo nunca rebasa una edición real local.

Se envían en orden de confirmación. Así se crean ejercicios antes de rutinas/sesiones que los usan. Un conflicto bloquea su registro, pero permite avanzar otros registros. Si una relación depende de una propuesta rechazada, el servidor también la conserva como conflicto de validación; resuelve primero el catálogo y después reenvía la propuesta dependiente. El grafo candidato completo se valida dentro de cada transacción, incluidos borrados con referencias entrantes. Archivar mantiene las relaciones históricas.

El catálogo inicial sin editar es bootstrap, no historia del usuario. Antes de enviar, el cliente descarga cambios; puede adoptar las definiciones del servidor en lugar de los defaults todavía intactos. Una modificación explícita local quita esa marca y sigue las reglas normales de conflicto.

## HTTP v1

| Endpoint | Contrato | Autenticación |
|---|---|---|
| `GET /healthz` | Texto `ok`, sin datos personales | No |
| `POST /api/v1/push` | `PushRequest { operations }` → `PushResponse { outcomes }` | Bearer obligatorio |
| `GET /api/v1/changes?cursor=0&limit=100` | `Page { changes, next_cursor, has_more }` | Bearer obligatorio |

Máximo 1 MiB por petición, 100 operaciones por lote y 100 cambios por página. El cliente actual envía una operación por petición, como simplificación intencionada. Cada operación y su resultado son atómicos; **el lote completo no es una única transacción**. Si un lote falla parcialmente, se reintenta con los mismos IDs; los resultados ya guardados son idempotentes. Las consultas están parametrizadas; los nombres SQL internos son constantes del código.

Al recibir una operación el servidor, bajo transacción de escritura:

1. Busca su UUID. Si ya existe con idéntico contenido, devuelve el resultado persistido. Si se reutiliza con otro contenido o dispositivo, rechaza la petición.
2. Valida el documento y sus relaciones. Los errores semánticos producen un conflicto recuperable y conservan la propuesta en el cliente.
3. Comprueba `base_version`. Si difiere de la actual, devuelve un conflicto con revisión del servidor. Una propuesta ordinaria exactamente idéntica al estado vigente puede aceptarse sin crear versión ni journal adicionales.
4. Si procede, incrementa la versión de ese registro, escribe la revisión y añade su entrada al journal con cursor autoincremental.
5. Guarda el resultado de la operación y confirma la transacción antes de responder.

La autenticación compara hashes SHA-256 del token en tiempo constante; no registra cabeceras ni cuerpos. El secreto se lee de un archivo restringido y se mantiene en memoria. Cambiarlo requiere reiniciar servidor y clientes.

Una respuesta perdida después de un commit se recupera reenviando exactamente la misma operación. Nunca se cambia su ID ni versión mientras esté congelada. El ack, eliminación de cola y actualización de la sombra remota son otra transacción SQLite. Una respuesta vieja no sustituye una sombra remota más reciente ni una edición local posterior.

## Descarga, exclusión y fallos

Una pasada obtiene un lock de archivo exclusivo por base, descarga, envía hasta 500 operaciones y vuelve a descargar hasta `has_more=false`. Quedan en disco las operaciones restantes para la siguiente pasada. El planificador tiene un solo intento activo y cola manual acotada; otro proceso sobre la misma base tampoco puede sincronizar simultáneamente.

Cada página se aplica junto con su cursor en una sola transacción. El cliente valida avance, tamaño y orden del cursor. Una interrupción antes del commit repite la página; después del commit continúa desde el cursor guardado. Un registro con cambios pendientes conserva su documento local mientras la sombra recibe novedades. Los tombstones se propagan igual que las otras revisiones y conservan el cuerpo para revisión y auditoría.

No hay compacción ni expiración de IDs de operación o cambios; por tanto un cliente que lleve mucho tiempo apagado puede recorrer todo el journal. Un cursor mayor que el máximo del servidor se rechaza y pide revisar una posible restauración antigua. Esto **no detecta todas las historias divergentes tras restaurar un backup**: sigue el procedimiento de restauración coordinada de `operacion.md` y crea bases cliente nuevas. No uses la misma base cliente contra servidores independientes.

## Conflictos y resolución

Las propuestas locales permanecen en `records` y la outbox; las del servidor, en `remote`. Ambas sobreviven al reinicio. La TUI muestra los contenidos completos, incluida la fecha, peso, asistencia, hábitos y series. La unicidad diaria es estructural: dos creaciones offline del mismo día con valores distintos entran en conflicto, sin generar duplicados.

Al resolver, una transacción comprueba que el documento local y la revisión remota siguen siendo los mostrados, archiva ambas propuestas en `resolutions`, retira la cola bloqueada y crea **una operación nueva con `strict_version=true`**. Esto sucede tanto al elegir local como servidor. Si se elige ausencia del servidor, se propone un tombstone. La elección completa se conserva en la auditoría aunque vuelva a haber conflicto.

El servidor verifica estrictamente esa versión antes incluso de aceptar contenido idéntico. Si otro dispositivo cambió el registro desde la comparación, la resolución vuelve a ser un conflicto. El servidor no garantiza que el registro no cambie *después* de aceptar una resolución: las ediciones posteriores se descargan normalmente.

No hay merge automático de campos, CRDT, orden por «último timestamp» ni transacciones entre varios días o catálogos. Es una aplicación de un solo usuario y se prioriza detectar y conservar propuestas sobre fusionarlas automáticamente.
