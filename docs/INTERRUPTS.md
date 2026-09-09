<!-- SPDX-License-Identifier: Apache-2.0 -->
# Excepciones, interrupciones y tiempo — #33

## Corte R0 y responsabilidades

El kernel instala GDT, TSS e IDT propias, atiende el temporizador PIT mediante el PIC y puede esperar plazos sin espera activa. Este corte usa una CPU x86_64, QEMU pc-q35-8.2/qemu64/TCG, UEFI/OVMF y la toolchain ya fijada. No introduce dependencias Cargo ni cambia a nightly.

| Módulo | Responsabilidad |
| --- | --- |
| `arch/x86_64/interrupts/segments.rs` | GDT de ring 0, TSS y pila de emergencia de 16 KiB |
| `table.rs` / `entry.S` | IDT y puente de registros entre CPU y ABI de Rust |
| `dispatch.rs` | Clasificación de vectores, tick, reconocimiento y diagnóstico terminal |
| `pic.rs` / `pit.rs` | Puertos del PIC y del canal 0 del PIT, respectivamente |
| `mask.rs` | Sección crítica local que conserva/restaura IF y espera con STI/HLT |
| `clock.rs` | Contador de ticks atómico y conversión a nanosegundos |
| `time/deadline.rs` / `time/waits.rs` | Contratos puros de plazos y registros simultáneos; sin arquitectura ni unsafe |
| `interrupts/tests.rs` | Escenarios del invitado, separados de los mecanismos |
| `tools/boot_support/scenarios.py` | Evidencia exigida por el ejecutor directo y el aislado |

El punto de entrada continúa componiendo módulos. Un token de controlador exclusivo, no Send/Sync, posee la inicialización y las esperas; no hay un gestor de todos los subsistemas. Los registros pendientes pertenecen a su `WaitSet`, con capacidad fija y sin allocator.

## Contrato de CPU y seguridad de memoria

La GDT contiene descriptores de código/datos de kernel y una TSS de 104 bytes. La IDT tiene 256 puertas de interrupción de ring 0. Los vectores 0–47 tienen entradas identificadas; los demás terminan como vector inesperado 255. El vector 8 usa IST1 y una pila estática independiente. La reserva explícita de GDT/TSS/IDT/pila es 20.624 bytes, sin contar alineación, código ni contadores. Aún no hay páginas de guarda ni protección de tablas mediante mapeos propios; corresponde a #9.

La entrada ensamblador normaliza el código de error, conserva los 15 registros generales, limpia DF antes de llamar a Rust, alinea la pila para la llamada y retorna mediante IRETQ. El marco tiene 176 bytes; tamaño y offsets críticos se comprueban al compilar. Los registros y flags del contexto interrumpido se restauran al retornar. Las puertas deshabilitan interrupciones enmascarables durante el handler.

Se usa el ABI soft-float y sin red zone de [x86_64-unknown-none](https://doc.rust-lang.org/rustc/platform-support/x86_64-unknown-none.html). Los stubs no guardan SIMD/FPU; se rechaza la compilación con SSE/SSE2/AVX habilitados. Introducir SIMD, ring 3, SMP, cambio de contexto, FS/GS por proceso o recuperación de fallos exige revisar este puente y sus pruebas antes de usarlo. Los detalles de IDT, marcos de excepción, TSS/IST e IRETQ se basan en el volumen 3 del [manual Intel SDM](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html).

Las tablas se escriben mediante punteros crudos durante una única inicialización con IF=0. Permanecen mapeadas durante toda la vida del kernel; no se entregan referencias mutables persistentes. El único cambio posterior es el fixture terminal que invalida deliberadamente la puerta de #GP para provocar #DF. No se reclama memoria del cargador.

INT3 es recuperable y cuenta el evento. El resto de excepciones termina la VM de prueba con vector, error, RIP, RSP, CR2 e indicador de pila de emergencia. La UART conserva su propiedad exclusiva: si una excepción interrumpe a su propietario, no se espera un lock ni se crea un alias; puede faltar diagnóstico serie y el ejecutor rechazará el falso éxito. No se promete recuperación de doble fallo ni de una pila de emergencia corrupta.

## PIC, reloj y esperas

PIC remapeado a 32–47, modo 8086 y EOI explícito. Solo IRQ0 está habilitada. IRQ7/15 sin bit ISR activo se consideran espurias: no se envía EOI al controlador que no atendió la interrupción; una espuria del esclavo reconoce únicamente la cascada del maestro. Las pruebas entran por INT 0x27/0x2f con ISR vacío: verifican esa ruta sin afirmar haber producido una carrera eléctrica real. Los registros y el protocolo PIC/PIT se contrastaron con las secciones 11 y 21 del [datasheet Intel de plataforma](https://cdrdv2-public.intel.com/332995/332995-skl-io-platform-datasheet-vol1_rev004.pdf).

PIT canal 0 en modo 2, entrada nominal de 1.193.182 Hz y divisor 11.932: aproximadamente 100 Hz, periodo nominal de 10.000.150,857 ns. El contador de ticks usa saturación y nunca envuelve; los plazos rechazan overflow. La conversión calcula la razón completa con enteros de 128 bits y satura la salida, sin acumular redondeo por tick.

Es tiempo monotónico derivado de interrupciones atendidas, no UTC ni tiempo real calibrado. Mantener IF deshabilitado durante varios periodos puede perder ticks. Las pausas de la VM y la carga del anfitrión afectan el tiempo observado. No usar este reloj para validar certificados TLS ni prometer límites de tiempo civil. APIC/HPET, SMP, suspensión y reloj civil quedan fuera de #33.

`wait_until` exige IF habilitado y un contexto normal del kernel. Comprueba el plazo con IRQ deshabilitada y duerme con la secuencia indivisible respecto a IRQ `sti; hlt`, seguida de `cli` para recuperar la sección crítica. Así no queda una ventana entre comprobar y dormir que pierda una interrupción pendiente. El guard restaura el IF previo y permite anidamiento. No debe moverse de CPU, guardarse indefinidamente ni usarse para protección frente a NMI o frente a otra CPU. Los handlers no asignan memoria, esperan ni adquieren locks bloqueantes; el tick atómico no publica otros datos.

`WaitSet<N>` admite plazos simultáneos, rechaza un slot ocupado o fuera de rango, permite cancelación y entrega cada finalización una sola vez. Son registros atendidos por un propietario; todavía no son hilos suspendidos. El planificador de #10 podrá consumir este contrato.

## Pruebas y aceptación

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py test --timeout 15
python3 tools/sandbox.py prepare
python3 tools/sandbox.py test --revision "$(git rev-parse HEAD)"
```

La suite directa incluye ocho escenarios. `ok` exige completar las pruebas del kernel antes de SUCCESS: INT3 con registros/DF conservados, dos espurias, guards anidados, rechazo de espera con IF=0, tres plazos pendientes (dos iguales), uno cancelado y cien esperas cortas. Se exige ninguna finalización anticipada y retraso máximo de dos ticks en R0; una carga que exceda esa tolerancia falla la prueba, no relaja automáticamente el criterio.

Los negativos adicionales son #UD (vector 6, sin error hardware), #GP (13, error 0xfff8), #DF real durante entrega de #GP (8, error 0 y `emergency=1`) y `timer-stall`: tras recibir un tick se enmascara IRQ0 y la espera debe agotar el timeout externo. Un triple fallo, un reinicio, otra excepción o un timeout anterior al marcador del fixture no acreditan el resultado esperado.

Los tres escenarios de excepciones salen con QEMU 39 y estado `exception`; la suite espera ese fallo deliberado. El comando individual devuelve 1. `timer-stall` devuelve 124 al agotar su plazo. Se mantienen panic, hang e invalid de #8. La suite aislada suma doce casos, conservando también fallos de compilación, cancelación y repetición limpia de #21.

Los directorios de artefactos contienen configuración, hashes, imagen/ELF, logs serie/QEMU y resultados. El marcador IRQ añade ticks y duración nominal del ejercicio; una muestra inicial local obtuvo 109 ticks y 1.090.016.443 ns, con arranque total cercano a cinco segundos incluyendo firmware y aproximadamente un segundo de autoprueba. Es una muestra de R0, no un benchmark universal. La issue conserva la revisión y CI usados para el cierre. Revisión por el agente implementador, sin revisión independiente.
