<!-- SPDX-License-Identifier: Apache-2.0 -->

# ABI de procesos R0 — versión 1.0

Contrato mínimo de #10, conservado por la extensión de [IPC/handles de #34](IPC.md). El SDK sigue en #11. La fuente compartida de constantes es `crates/abi`, sin dependencia del kernel.
No es el ABI Linux ni una interfaz MCP. La identidad proviene del proceso que
el kernel está ejecutando; ningún argumento puede sustituirla.

Un programa ELF64 estático x86_64 entra por e_entry en ring 3, CS=0x2b,
SS=0x33, RSP alineado a 16 bytes, IF=1 e IOPL=0. No recibe una dirección de
retorno: debe terminar mediante EXIT. El primer corte admite código entero
sin x87/MMX/SIMD, TLS, enlace dinámico ni red zone requerida por el kernel.
RDI, RSI y RDX contienen tres enteros de arranque suministrados por el
lanzador de pruebas. Los demás registros generales comienzan en cero.
La pila de usuario tiene 16 KiB RW/NX y una página inferior sin mapear.
SYSCALL/SYSENTER no son entradas admitidas: el kernel deshabilita esas rutas.

INT 0x80 usa RAX como número y RDI como argumento entero. Devuelve un entero
de 64 bits en RAX y conserva los demás registros generales. RFLAGS conserva
los indicadores aritméticos y DF; el kernel fija IF y elimina flags de
control no admitidos antes de reanudar. No hay punteros ni buffers de usuario
en estas llamadas. No se modifica la memoria del llamante.

| RAX | Nombre | RDI | Resultado |
| --- | --- | --- | --- |
| 0 | QUERY | Ignorado | 0x00010000, versión 1.0 |
| 1 | EXIT | Código de salida u64 | Termina; no retorna |
| 2 | REPORT | Valor diagnóstico u64 | 0; máximo ocho valores por proceso, después QUOTA |
| 3 | GET_PID | Ignorado | Identidad monotónica asignada por kernel |

Número desconocido devuelve NOT_SUPPORTED = u64::MAX. QUOTA = u64::MAX - 1.
REPORT conserva contador y último valor en el registro del proceso: no concede
E/S arbitraria ni acepta texto del usuario para interpretarlo como log del kernel.
La espera, terminación por el supervisor de pruebas y creación se ofrecen por
API interna tipada; aún no son syscalls. Esperar un proceso vivo devuelve
pendiente; esperar uno terminado recupera sus recursos y consume su resultado.
PID desconocido o ya recogido produce error. La extensión de [IPC](IPC.md) añade handles por propietario y derechos atenuables. La concesión/transferencia y cancelación por PID permanecen en la API del lanzador de confianza, sin exponer autoridad arbitraria por syscall.
