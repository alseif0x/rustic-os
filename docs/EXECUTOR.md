<!-- SPDX-License-Identifier: Apache-2.0 -->
# Ejecutor aislado del anfitrión (#21)

El propietario puede construir una revisión Git y probar su kernel en QEMU sin ejecutar los scripts candidatos directamente en su sesión. No requiere un modelo, MCP ni interfaz gráfica. Es infraestructura del anfitrión; no es todavía un servicio de RusticOS ni el puente de #42.

## Uso

Base: Linux amd64, Python 3.12, Git y Docker Engine con cgroup v2. Validado en Ubuntu 24.04 bajo WSL2 y en CI. En Windows ejecutar desde Ubuntu; QEMU usa TCG, sin KVM ni acceso a dispositivos del anfitrión.

Desde un checkout de infraestructura revisado:

```sh
python3 tools/sandbox.py prepare
python3 tools/sandbox.py run --revision "$(git rev-parse HEAD)"
python3 tools/sandbox.py test --revision "$(git rev-parse HEAD)"
python3 tools/sandbox.py cancel ID_DEL_TRABAJO
```

`prepare` descarga herramientas y dependencias y construye la imagen de referencia. Es una operación de confianza del propietario, con red, que puede tardar hasta 15 minutos. Repetirla cuando cambien las herramientas o las dependencias revisadas. No ejecutar este paso desde código candidato no revisado. La configuración queda en `.cache/sandbox-image.json`: digest de Ubuntu, identidad inmutable de la imagen y hash de los archivos de infraestructura. No se ejecuta el Dockerfile de una revisión candidata.

`run` exige el SHA completo de un commit local. Exporta ese commit con `git archive`; no incluye `.git` ni cambios sin commit. No admite comandos, montajes, variables de entorno, dispositivos, URLs ni opciones Docker del solicitante. Los únicos ajustes son `--mode ok|panic|hang|invalid|exception|gp|doublefault|timer-stall|memory-ro|memory-nx|memory-unmapped|memory-text-alias|memory-guard`, `--build-timeout 1..300` (120 por defecto) y `--boot-timeout 1..120` (30 por defecto). Los modos distintos de `ok` son pruebas deliberadas de arranque, excepciones y pérdida del temporizador; véase [INTERRUPTS.md](INTERRUPTS.md).

La salida estándar es un objeto JSON. Los eventos van a stderr; el evento `started` incluye el identificador necesario para cancelar desde otra terminal. `Ctrl+C` y SIGTERM también solicitan cancelación y limpieza. Código de salida 0: éxito, preparación o cancelación atendida; 1: fallo. Los errores de sintaxis de argparse usan 2.

## Fronteras y recursos

1. Un contenedor construye el snapshot con Cargo offline y un registro de dependencias desechable. El target es `x86_64-unknown-none`; se fija `RUSTIC_BUILD_ID` al prefijo de la revisión.
2. Se exporta únicamente un ELF acotado mediante un ejecutable de referencia de solo lectura. El receptor del anfitrión acepta un único miembro regular con nombre exacto; rechaza enlaces, rutas alternativas y tamaños excesivos. No extrae rutas del archivo tar. Los bytes candidatos continúan siendo datos no confiables, incluso si cambian durante la exportación.
3. Se elimina el contenedor de construcción. Otro contenedor empaqueta esos bytes con Limine, configuración y avisos de referencia, y ejecuta QEMU. Los scripts candidatos no controlan el evaluador de arranque. El éxito exige la salida de QEMU y el marcador serie de la revisión esperada.
4. Se recogen los artefactos y se eliminan ambos contenedores, incluidos sus procesos y tmpfs. No hay reutilización de workspaces entre trabajos.

| Límite por contenedor | Política |
| --- | --- |
| CPU / RAM / swap adicional | 2 CPU, 2 GiB, 0 |
| Procesos | 128 |
| Espacio de trabajo / temporal | tmpfs de 1 GiB / 128 MiB, incluidos en la RAM |
| Red / privilegios | ninguna red externa, UID 1000, sin capabilities, no-new-privileges, seccomp predeterminado |
| Sistema de archivos | raíz de solo lectura; sin montajes del anfitrión, socket Docker ni dispositivos añadidos |
| Snapshot / ELF / imagen recuperados | máximo 32 MiB / 16 MiB / 64 MiB |
| Logs de cliente / logs de VM recuperados | 8 MiB por comando / 1 MiB por log |
| Concurrencia e intentos | un trabajo activo por checkout; un intento, sin reintentos automáticos |

Los límites de tiempo cubren cada fase: la construcción usa el presupuesto indicado; la fase de empaquetado y VM permite 30 segundos adicionales al timeout de QEMU. Creación, exportación y limpieza tienen plazos propios de hasta 30 segundos por comando. No se promete que el tiempo total sea exactamente la suma de los dos parámetros.

Docker comparte el kernel Linux del anfitrión. Estas comprobaciones verifican restricciones concretas, no una garantía frente a vulnerabilidades de Docker, del kernel o de QEMU. Para código hostil de terceros que requiera una frontera más fuerte, ejecutar también el servicio Docker y el controlador dentro de una VM desechable. El usuario autorizado a manejar Docker mantiene autoridad sobre el anfitrión: no entregar ese socket ni una shell de ese usuario al invitado o a un agente sin mediación.

La sonda usa un archivo sintético del host; comprueba ausencia de ese archivo y del socket, denegación de escritura en raíz y conexión externa, UID, capabilities, seccomp, cgroups, cuotas tmpfs y ausencia de montajes/dispositivos añadidos. No inspecciona documentos personales. Cualquier secreto previamente incluido en un commit exportado sí forma parte del snapshot; el ejecutor no es un detector de secretos.

## Resultados y recuperación

`artifacts/jobs/ID/job.json` registra `schema_version: 1`, revisión, imagen de herramientas, hashes del controlador, infraestructura de imagen y snapshot, límites, configuración observada de los contenedores, tiempos, resultado del invitado y artefactos con SHA-256 y tamaño. `image.json` añade versiones/hashes de Rust, QEMU, OVMF, Limine y del kernel/imagen. Los logs y los artefactos permanecen en el anfitrión tras la limpieza.

| Estado final | Significado |
| --- | --- |
| `success` | compilación y arranque de la revisión esperada verificados |
| `build_failed` | la compilación devolvió error |
| `build_timeout` | la construcción superó el presupuesto |
| `boot_failed` | fallo de empaquetado/VM o panic/fatal/salida inesperada del invitado; consultar logs y `guest_result` |
| `boot_timeout` | timeout de QEMU o de la fase de arranque |
| `resource_limit` | Docker notificó muerte por falta de memoria; otros límites pueden aparecer como fallo de fase |
| `cancelled` | cancelación atendida y limpieza terminada |
| `executor_error` | error del controlador o transferencia; no se acredita fallo del kernel |
| `cleanup_failed` | no se pudo confirmar la eliminación; `cleanup_errors` requiere atención del propietario |
| `request_error` | entrada/configuración no válida o herramienta local no disponible |

`cancel ID` solicita detener los contenedores del trabajo y comprueba su etiqueta de propiedad antes de eliminarlos. Su respuesta `cancellation_requested` acredita la solicitud atendida; el estado terminal completo debe leerse en `job.json`. Tras SIGKILL, reinicio o caída de Docker, volver a ejecutar `cancel ID` cuando Docker esté disponible; si el controlador ya no está activo, reconcilia también el estado abandonado. No se borran contenedores ajenos, imágenes del propietario ni artefactos para ocultar errores. No hay una instalación del kernel en el anfitrión que restaurar: se vuelve a ejecutar un commit conocido en un workspace nuevo.

La cuota de los trabajos no incluye el aprovisionamiento de Docker ni el historial acumulado de artefactos. El propietario gestiona esa retención; CI conserva evidencia 14 días. No se realiza limpieza global de Docker. La imagen de herramientas se construye localmente y no se publica en un registro; los paquetes conservan sus propias licencias.

## Verificación y límites de la evidencia

`test` ejecuta diecisiete casos: éxito, panic, bloqueo del invitado, argumentos inválidos, #UD, #GP, doble fallo, pérdida del temporizador, cinco fallos de protección de memoria, error real de compilación, bloqueo real de build script, cancelación durante compilación y repetición limpia exitosa. Los fixtures son commits locales sin referencias, creados con un índice Git temporal: no modifican archivos, staging ni ramas. Los casos de bloqueo/error exigen su marcador para evitar aceptar un fallo previo de herramientas. `artifacts/sandbox-suite.json` enlaza los trabajos; `artifacts/isolation-probe/result.json` registra las restricciones comprobadas.

CI aprovisiona desde el código de su revisión, sin credenciales persistentes ni secretos de proyecto. Un autor que cambie también el workflow o el controlador puede cambiar sus propias pruebas: el verde de una PR no es una atestación externa frente a un autor malicioso. El uso con candidatos separados exige una infraestructura revisada y una imagen preparada por el propietario. Las pruebas del arranque tampoco demuestran aislamiento de procesos dentro del futuro RusticOS.

La imagen ejecutada se fija por identidad de contenido, y Ubuntu por digest. La resolución de todos los paquetes transitivos durante `prepare` no es hermética: reconstruir en otra fecha puede producir otra identidad de imagen, que debe conservarse en la evidencia. La política se apoya en los mecanismos documentados de [límites Docker](https://docs.docker.com/engine/containers/resource_constraints/), [ejecución de contenedores](https://docs.docker.com/engine/containers/run/) y [seccomp](https://docs.docker.com/engine/security/seccomp/).
