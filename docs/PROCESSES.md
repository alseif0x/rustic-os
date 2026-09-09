<!-- SPDX-License-Identifier: Apache-2.0 -->

# Procesos nativos y aislamiento — #10

## Resultado y alcance

RusticOS carga ELF64 estáticos en raíces propias y ejecuta sus instrucciones en
ring 3. Un temporizador devuelve el control al kernel aunque el programa no
solicite servicios. El planificador selecciona por turnos entre procesos listos;
una excepción de aplicación conserva un resultado de fallo y permite continuar
a sus compañeros. Crear, consultar, avanzar, terminar y esperar/recoger son
operaciones internas tipadas, sin depender de shell, filesystem, red ni modelo.

Es un corte monoprocesador con cuatro procesos residentes como máximo. Cada
programa tiene hasta 256 páginas de datos/código/pila, incluidas cuatro páginas
de pila. Las tablas de paginación y los metadatos de proceso se contabilizan
aparte. Los procesos terminados ocupan su ranura y memoria hasta que el propietario
recoge el resultado; la tabla acotada evita zombis ilimitados. Las identidades
son monotónicas durante la vida del administrador; una creación fallida puede
consumir una identidad y nunca la reutiliza. Agotar el contador es un error.

No hay hilos, fork, enlace dinámico, TLS, señales, prioridad, SMP, demanda de
páginas ni estado de punto flotante/SIMD por proceso. Las instrucciones de esa
última categoría provocan un fallo del proceso: no se dejan compartir registros
extendidos accidentalmente. El protocolo inicial de llamadas enteras está en
[PROCESS-ABI.md](PROCESS-ABI.md); IPC, handles, copia de buffers y autoridad de
servicios pertenecen a #34/#13. La GUI y los agentes consumirán servicios posteriores.

## Separación de responsabilidades

| Módulo | Contrato |
| --- | --- |
| `process/elf.rs` en la biblioteca | Decodificación y validación completa sin unsafe, CPU ni asignación |
| `process/lifecycle.rs` | Identidad, estados, capacidad, turnos y consumo del resultado |
| `process/abi.rs` | Constantes del primer ABI, sin dependencias de implementación |
| `process/runtime/error.rs` | Errores de la frontera de carga/proceso, sin dependencia del administrador |
| `process/runtime/loader.rs` | Copiar segmentos, completar con ceros y revertir una carga incompleta |
| `process/runtime/manager.rs` | Poseer espacios/contextos, ejecutar un turno y aplicar el resultado del evento |
| `arch/x86_64/memory/user.rs` | Mapeos, copia inicial, ejecución acotada con cambio/restauración de raíz y destrucción |
| `interrupts/frame.rs` | Layout exacto de registros y validación del retorno de usuario |
| `interrupts/user_cpu.rs` | Deshabilitar entradas de privilegio ajenas al ABI y fijar opciones de CPU |
| `interrupts/user.rs` | Transición acotada ring 0/3, captura del evento y restauración del llamante |
| `interrupts/segments.rs` / `table.rs` | Segmentos, TSS, pila de entrada y puerta de llamada con DPL 3 |
| `process/runtime/tests/` / `fixture.S` | Aplicaciones originales y escenarios del invitado, separados de los mecanismos |
| `tools/boot_support/process_evidence.py` | Rechazar éxitos incompletos, fallos incorrectos y pérdida de recursos |

El administrador recibe el propietario de memoria al operar; no lo publica como
global ni lo presta a la IRQ. Debe terminar/recoger todos sus procesos antes de
abandonarse. El propietario de memoria y sus mapeos superiores permanecen vivos
durante toda la ejecución. La API todavía es interna al binario de pruebas.

## Formato cargado y procedencia

El formato adoptado aquí es ELF64 little-endian, ET_EXEC, EM_X86_64, identificación
System V versión cero y ELF versión uno. Solo se admiten PT_NULL y PT_LOAD. Se
rechazan intérprete, segmento dinámico, TLS y cabeceras no implementadas, en vez
de ignorar necesidades del programa. Los tamaños fijos de cabecera son 64 y
56 bytes. Hasta 16 cabeceras, ocho segmentos cargables y un archivo de 1 MiB.

Las sumas se comprueban antes de indexar o reservar. Los segmentos cargables
están ordenados y no comparten páginas; el corte admite inicios no alineados
si offset y dirección son congruentes. Se verifican potencias de dos de
alineación, límites de archivo, filesz <= memsz, presupuesto y exclusión de
página nula, mitad superior y reserva de pila. Solo se aceptan R, RX y RW;
W+X no es válido. La entrada pertenece a bytes presentes de un segmento
ejecutable. Se ignoran secciones de depuración: el cargador usa cabeceras de
programa. Es un subconjunto explícito del [formato ELF](https://gabi.xinuos.com/elf/02-eheader.html)
y su [contrato de carga](https://gabi.xinuos.com/elf/07-pheader.html).

Las dos aplicaciones iniciales se ensamblan desde `fixture.S` como archivos ELF
inmutables embebidos en la imagen de arranque. El kernel valida sus bytes,
reserva páginas independientes y copia el código: no llama sus etiquetas como
funciones de ring 0. Contienen instrucciones nativas originales; no incorporan
un runtime de terceros. No se compila todavía una aplicación Rust mediante SDK.
El mismo cargador recibe un slice de bytes y no conoce la ubicación embebida;
conectar módulos de arranque externos o archivos será una fuente posterior.

Cada página se limpia antes de cargar contenido. Se copia solo el intervalo
del archivo perteneciente a esa página, conservando ceros en BSS y bordes.
Un error libera todos los segmentos/tablas/raíz adquiridos por la carga y la
ranura reservada. El kernel no ejecuta una imagen parcialmente construida.

## Privilegios, interrupción y propiedad unsafe

La GDT mantiene código/datos de kernel y añade código/datos DPL 3. RSP0 de la
TSS apunta a una pila de entrada de 16 KiB, común al único CPU, con una página
de guarda. La pila IST de doble fallo conserva su propia guarda y reserva.
Las dos guardas están desmapeadas también en HHDM. TSS, stacks y tablas son
supervisor; el mapa de permisos de E/S queda fuera del límite TSS y deniega
puertos a usuario. IOPL permanece cero. Los mecanismos de TSS, puertas e IRET
siguen el [Intel SDM](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html).

La puerta 0x80 es la única invocable desde usuario; 0x81 es una entrada interna
DPL 0 que guarda la continuación del llamante. Las IRQ/excepciones guardan todos
los registros generales. La IRQ reconoce primero el PIC y actualiza el reloj.
Después copia el marco del usuario al intercambio acotado y retorna al llamante
de kernel. El administrador procesa ese evento fuera del handler y elige otro
turno. Cada tick termina el turno; una llamada también es un punto de planificación.
No hay desalojo de código kernel ni pilas kernel suspendidas por cada proceso.

Se deshabilita SYSCALL mediante EFER.SCE y se vacían los MSR de SYSENTER cuando
CPUID anuncia SEP. No se heredan entradas privilegiadas del cargador. También
se retiran CR4.FSGSBASE/PCE; el primer ABI no ofrece TLS ni contadores de rendimiento.

La dependencia de arquitectura va de memoria hacia el puente de CPU; el módulo de interrupciones no importa memoria ni procesos. La capa de carga y el administrador comparten tipos de error sin importarse mutuamente.

El único puntero temporal del puente se instala con IF=0 sobre un objeto vivo
en la pila común. El llamante queda suspendido durante su préstamo al handler;
se elimina el puntero antes de recuperar acceso al objeto. No hay asignaciones
ni referencias a memoria inferior que sobrevivan a un cambio de CR3. NMI,
doble fallo y machine check no toman prestado ese intercambio: siguen siendo
terminales del kernel. Un fallo originado en ring 0 tampoco se atribuye a una
aplicación para ocultarlo.

Antes de retornar se validan selectores y direcciones de instrucción/pila y se
filtran flags. Un RSP no canónico termina el proceso antes de intentar IRET.
CR0.TS impide usar estado FP/SIMD sin guardar; se restaura el valor del llamante
al recuperar el kernel. Los resultados de excepción guardan vector/error y CR2
solo para un fallo de página. El kernel recupera su raíz antes de liberar recursos.

Este aislamiento cubre los accesos e instrucciones comprobados en R0. No acredita
resistencia a canales laterales, fallos de drivers/kernel, hardware diferente o
un cargador malicioso. No hay auditoría independiente; revisión del implementador.

## Pruebas reproducibles

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py test --timeout 15
python3 tools/sandbox.py prepare
python3 tools/sandbox.py test --revision "$(git rev-parse HEAD)"
```

El escenario `ok` ahora exige la evidencia de procesos además de IRQ y memoria:

- Un programa no cooperante avanza en un bucle sin llamadas, recibe al menos
  dos desalojos de temporizador y permite que un compañero complete. Ambos usan
  la misma dirección virtual de datos con valores distintos.
- Lectura/escritura de una página exclusiva del compañero y del kernel, escritura
  de código, ejecución de pila y acceso a la guarda producen el fallo de página
  concreto. CLI, opcode inválido, FP sin soporte, entrada kernel y retorno inválido
  se contienen igualmente. Tras cada caso el compañero recibe otro turno de CPU.
- SYSCALL deshabilitado, E/S de puertos y HLT desde usuario se rechazan sin
  permitir que la aplicación desactive o detenga la planificación.
- Los programas comprueban CS/SS, alineación de pila, BSS, datos inicializados,
  identidad, versión, llamada desconocida, valores preservados y cuota de diagnóstico.
- Se comprueban capacidad llena, espera pendiente, terminación explícita, resultado
  recogido una sola vez, identidades nuevas y 16 ciclos completos sin pérdida de marcos.
- El invitado rechaza ejecutables inválidos antes de reservar. Reservando temporalmente
  el inventario real hasta dejar 0, 5 o 9 páginas libres, se fuerzan fallos al crear
  raíz o cargar parcialmente segmentos/pila. Se devuelve el contador libre exacto.

Las suites siguen teniendo 13 escenarios de VM y 17 del ejecutor: la aceptación
de procesos y sus negativos contenidos se integra en `ok`. Los fallos de kernel
de los otros escenarios siguen siendo terminales esperados. Los contratos de
validación/política tienen además tests de host; esos tests no sustituyen ring 3.

La evidencia de cierre enlaza commit, CI, versiones y artefactos en #10. Configuración:
R0 de una CPU y 256 MiB, QEMU 8.2.2 q35/qemu64/TCG, OVMF 2024.02 y Rust 1.98.1.
El diagnóstico registra desalojos, fallos contenidos, repeticiones y memoria libre
antes/después. Los tiempos incluyen autopruebas y no son latencia de un arranque
de producto. El presupuesto de 2 GiB del futuro escenario integrado todavía
requiere ampliar el límite físico de #9; no queda acreditado por esta prueba.

`PROCESS_MEMORY` registra el máximo de marcos usados al llenar las cuatro ranuras,
incluyendo tablas y pilas de usuario, el tamaño real de `Manager` y la reserva
adicional de pila de entrada de 20 KiB (incluida guarda). Esta reserva estática
no se vuelve a cobrar por proceso. El ELF de cada fixture ocupa 8.200 bytes en la
imagen; dos variantes quedan embebidas como datos de prueba, fuera del consumo
dinámico de las raíces. Los tres puntos de agotamiento se registran por separado.
