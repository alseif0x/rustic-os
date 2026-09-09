<!-- SPDX-License-Identifier: Apache-2.0 -->
# IPC y handles R0 — #34

## Contrato binario

La extensión usa INT 0x80; los números 0–3 y QUERY=0x00010000 de #10 se conservan.
`crates/abi` es la fuente de constantes independiente del kernel y del futuro SDK.
Se descubre esta extensión mediante INFO; no se habilitan SYSCALL/SYSENTER.

| RAX | Operación | RDI | RSI | RDX | Resultado |
| --- | --- | --- | --- | --- | --- |
| 4 | SEND | Handle | Puntero de origen | Longitud | 0 o error |
| 5 | RECEIVE | Handle | Puntero de destino | Capacidad | Bytes copiados o error |
| 6 | WAIT | Handle | Ignorado | Ignorado | 0 cuando hay mensaje; error al cerrar/cancelar |
| 7 | CLOSE | Handle | Ignorado | Ignorado | 0 o error |
| 8 | INFO | Ignorado | Ignorado | Ignorado | Versión de IPC: 1 |

RAX contiene el resultado; los otros registros generales se conservan. SEND y
RECEIVE son no bloqueantes. WAIT espera disponibilidad de recepción, sin consumir
ni copiar; después se llama RECEIVE. No se guarda un puntero de usuario durante
una espera. Si todas las tareas están bloqueadas, el administrador devuelve ausencia
de trabajo listo; no hace polling ni promete detectar/resolver un interbloqueo.

Los errores de IPC son `u64::MAX - n`: n=2 handle ajeno/inválido/obsoleto, 3 permiso
denegado, 4 dirección inválida, 5 tamaño inválido, 6 versión incompatible,
7 mensaje inválido, 8 cola vacía/llena, 9 extremo cerrado, 10 espera cancelada,
11 cuota agotada. Los valores n=0 y n=1 siguen reservados por el ABI anterior.

El mensaje tiene 24 bytes de cabecera y hasta 64 de payload. Enteros little-endian:
versión u16 en 0, opcode u16 en 2, longitud de payload u32 en 4, correlación u64
en 8, identidad de emisor u64 en 16. La longitud suministrada debe ser exactamente
24+payload; versión 1 y opcode DATA=1 son los únicos admitidos. El campo de emisor
debe ser cero al enviar: el kernel lo sustituye por el PID del proceso ejecutado.
La recepción devuelve esa identidad, incluyendo mensajes encolados antes de que
muriera el emisor. No se transmiten structs Rust, padding ni direcciones del kernel.

La correlación y el payload son datos de aplicación; no conceden autoridad.
La negociación inicial consulta INFO y usa versión 1; una versión/opcode distinto
se rechaza sin encolar. Los contratos de servicios de #6 serán otra capa.

## Propiedad, derechos y límites

El broker posee cuatro canales dúplex como máximo, con dos mensajes por dirección.
La tabla tiene 16 entradas como límite defensivo y ocho por propietario; cuatro
canales sin duplicación producen como máximo ocho handles activos. Los tokens
monotónicos no se reutilizan durante la vida del broker y se comprueban junto con
el propietario. Conocer o adivinar el número de otro proceso no permite utilizarlo.
No se afirma que sean secretos criptográficos.

READ=1 permite recibir/esperar, WRITE=2 enviar y TRANSFER=4 mover el extremo.
Cerrar requiere propiedad, sin derecho adicional. El lanzador de confianza crea
el par y concede cada extremo a procesos vivos. La transferencia es una operación
interna del lanzador: exige TRANSFER, mueve el extremo y sus mensajes pendientes,
solo reduce derechos, invalida el token anterior y emite uno nuevo para el receptor.
No duplica referencias. Se valida destino/capacidad antes de modificar el dueño.
El lanzador entrega los tokens mediante argumentos antes de la primera ejecución.
Crear/transferir/cancelar por PID no se exponen como syscalls sin una autoridad
de supervisor; esa política corresponde a #13. No hay transferencia implícita
al incluir un entero en un mensaje.

Cerrar o terminar un proceso retira sus handles inmediatamente, aunque su memoria
se recoja después. Se descarta la cola del extremo cerrado; el compañero puede
consumir mensajes que ya tenía y luego recibe Closed. No puede enviar nuevos.
Al cerrar ambos extremos se recupera el canal. La cancelación interna despierta
únicamente al proceso bloqueado seleccionado y completa WAIT con Cancelled;
el handle sigue disponible para una operación posterior. No hay alias o préstamo
pendiente a memoria del usuario.

## Validación y ejecución

El kernel verifica propietario/derecho antes de acceder al buffer. Las longitudes
de SEND/RECEIVE se limitan a 24–88 bytes. RECEIVE no consume si la capacidad no
alcanza, el destino es inválido o no tiene permiso de escritura. Se valida el
intervalo que realmente se copiará, no bytes adicionales de capacidad sin utilizar.

Memoria comprueba overflow, mitad canónica inferior, página nula, presencia y
permisos efectivos USER/WRITE en todas las páginas, antes de copiar ningún byte.
La transferencia usa el HHDM de marcos propios: nunca crea una referencia Rust a
un puntero de usuario. El origen/destino pertenece a una raíz inactiva y ningún
usuario se ejecuta entre validación y copia/consumo. El corte es una CPU, sin DMA
a esas páginas ni mutaciones de tablas desde IRQ. SMP o memoria compartida exigirán
pinning/sincronización adicionales; validar y luego copiar no bastaría por sí solo.

Broker, colas, mensajes y handles son mecanismos puros en `kernel/src/ipc/`.
`memory/copy.rs` de arquitectura posee los accesos físicos. El módulo de syscalls
traduce ABI y errores; `ipc_control.rs` conecta concesiones, cierre y despertar
con el ciclo de vida. El planificador solo añade el estado Blocked y su transición
a Ready. La IRQ continúa sin conocer procesos, IPC ni asignación.

La documentación de cierre y pruebas se completa con evidencia del invitado;
las constantes o tests de host por sí solos no acreditan IPC entre procesos reales.

## Pruebas y evidencia

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py test --timeout 20
python3 tools/sandbox.py prepare
python3 tools/sandbox.py test --revision "$(git rev-parse HEAD)"
```

`ok` exige la aceptación de IPC, además de memoria/procesos/IRQ. Dos aplicaciones
ELF originales en ring 3 intercambian petición/respuesta con payload, correlación
e identidad comprobados durante 16 ciclos. Se verifican espera vacía, ausencia de
trabajo cuando todos están bloqueados, cancelación por el lanzador, CLOSE desde
usuario, muerte del compañero, vaciado de mensajes ya encolados y recuperación.

Veintidós negativos invocan syscalls desde usuario: punteros nulos, overflow,
no canónicos, kernel, no mapeados y cruce hacia hueco; tamaños cortos/grandes;
versión/opcode, identidad falsificada; destino de solo lectura; handle ajeno,
obsoleto y derecho denegado. Se comprueba el código esperado, conservación de la
cola y, en el cruce inválido de recepción, que ni el prefijo válido se modifica.
También se copian mensajes correctamente entre dos páginas distintas, se llena
la cola y se comprueba FIFO sin pérdida, se mueve un extremo con READ al nuevo
propietario y se rechaza recuperar derechos eliminados. Cerrar el antiguo dueño
no retira el extremo movido.

Los tests de host cubren formato, autenticación del emisor, FIFO/backpressure,
movimiento/atenuación, capacidad, recuperación y no reutilización de tokens,
además de transiciones Blocked/Ready. El verificador del anfitrión exige todos
los marcadores, códigos, recursos a cero y contadores libres iguales; un log
parcial o un error mal clasificado no se convierte en éxito.

R0 conserva una CPU, 256 MiB, QEMU 8.2.2 q35/qemu64/TCG, OVMF 2024.02 y Rust
1.98.1. La nueva aceptación se integra en los 13 escenarios de VM y 17 aislados.
Una muestra local mide 3.648 bytes para el administrador con broker y estados,
frente a 1.040 del corte #10; los cuatro procesos siguen usando 52 marcos para
sus páginas/tablas. El primer arranque con IPC tardó unos 8,9 segundos incluyendo
autopruebas. Son muestras identificadas por el ejecutor, no límites universales.
La issue de cierre enlaza commit/CI, hashes y repetición aislada.

Revisión disponible: agente implementador, sin auditoría independiente. No se
acredita SMP, DMA a buffers, memoria compartida, servicio de nombres, transferencia
por mensajes o autoridad de producto. Las concesiones y cancelaciones internas del
lanzador no son una API abierta a aplicaciones; #13 deberá darles un contexto de
autoridad antes de exponerlas. IPC fiable no acredita todavía archivos, shell,
SDK o piloto. El siguiente corte #11 consumirá el ABI para aplicaciones Rust.
