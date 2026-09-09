<!-- SPDX-License-Identifier: Apache-2.0 -->
# Primer arranque de RusticOS

Esta imagen experimental está destinada a la máquina virtual R0. Arranca, valida datos y finaliza la prueba; no es todavía una sesión interactiva ni una shell. QEMU y las herramientas se ejecutan en Ubuntu/WSL2 como entorno de referencia; el invitado ejecuta su propio kernel.

## Ejecutar

Desde la raíz, dentro de Ubuntu con las herramientas de [desarrollo](DEVELOPMENT.md):
```sh
source ~/.cargo/env
python3 tools/environment.py install
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py run --mode ok
```

La última orden construye el ELF estático y el volumen FAT32 de 64 MiB, verifica los paquetes/hashes de referencia y ejecuta QEMU sin pantalla, red o monitor interactivo. El invitado imprime START, MAP y SUCCESS, y finaliza mediante el dispositivo de pruebas de QEMU. El comando devuelve 0 si verifica marcador, identidad y estado de salida.

Para construir sin ejecutar:
```sh
python3 tools/boot.py image --mode ok
```

La imagen resultante está en artifacts/boot/ok/rustic-os.img y el ELF en el mismo directorio. Es un volumen FAT32 de arranque extraíble, sin GPT, con EFI/BOOT/BOOTX64.EFI, kernel.elf, limine.conf y avisos en licenses/. Su ruta de prueba es la unidad virtio de QEMU/OVMF; no se ha validado en hardware físico.

## Casos y códigos

```sh
python3 tools/boot.py test --timeout 30
```

| Fixture | Evidencia del invitado | QEMU | Comando run |
| --- | --- | --- | --- |
| ok | SUCCESS con build id esperado | 33 | 0 |
| panic | PANIC tras validar mapa | 35 | 1 |
| hang | HANG antes de detener CPU | Terminado por el ejecutor al agotar plazo | 124 |
| invalid | FATAL por modo desconocido | 37 | 1 |
| exception | #UD, vector 6 y error 0 | 39 | 1 |
| gp | #GP, vector 13 y error 0xfff8 | 39 | 1 |
| doublefault | #DF, vector 8, error 0 y pila de emergencia | 39 | 1 |
| timer-stall | Espera con IRQ0 enmascarada tras verificar un tick | Terminado por timeout | 124 |

El dispositivo isa-debug-exit transforma el valor escrito por el kernel en (valor × 2) + 1; estos códigos son exclusivos de la prueba R0. Cualquier combinación inesperada devuelve 2. La suite completa devuelve 0 solo si los ocho casos coinciden con sus resultados y marcadores esperados. Un timeout de firmware sin alcanzar el fixture no pasa la prueba de bloqueo. `ok` comprueba también [interrupciones y esperas](INTERRUPTS.md) antes de SUCCESS.

30 segundos es el presupuesto local inicial, con arranques positivos observados en torno a 4 segundos; CI usa 45 segundos para absorber variación de runner. No constituye un objetivo de rendimiento universal. El anfitrión mata únicamente el proceso QEMU creado por esa ejecución y espera su salida; también lo retira ante interrupción del ejecutor.

## Evidencia

Cada directorio de fixture conserva image.json (versiones/configuración, commit y estado del checkout, identificador de fuentes y hashes), kernel.elf, rustic-os.img, serial.log, qemu.log y result.json (comando, duración, timeout y estado). suite.json se escribe únicamente al completar los ocho casos. Los logs se reinician por ejecución; no se reutiliza un éxito previo.

El build id directo identifica los archivos Rust y ensamblador del kernel y su configuración de compilación; en el ejecutor aislado identifica el commit candidato. No reemplaza el hash SHA-256 del ELF o de la imagen. La imagen contiene fechas del filesystem FAT: no se promete identidad binaria entre reconstrucciones. El entorno tampoco es hermético por la base Ubuntu y sus dependencias transitivas.

## Responsabilidades y confianza

- main.rs compone entrada y panic.
- boot/entry.rs coordina las fases; boot/limine.rs concentra las peticiones y traducción del protocolo.
- boot/map.rs, mode.rs y region.rs son lógica pura comprobada en el host; no incluyen dependencias de Limine.
- arch/x86_64/io.rs contiene instrucciones de puertos; serial.rs posee la UART con un token exclusivo, espera acotada y sin asignaciones.
- diagnostic.rs emite errores; el panic no espera locks ni crea alias del propietario serie.
- tools/boot_support/image.py crea archivos de imagen en directorios propios; runner.py ejecuta QEMU y clasifica evidencia.

Bindings limine 0.5.0 con base revision 3, cargador Limine 12.8.0, stack solicitado de 64 KiB. La revisión consultada de la [especificación](https://github.com/limine-bootloader/limine-protocol/blob/da65184e91f80fcb397270121b1e2515a11e01ee/PROTOCOL.md) se registra en tools/environment.toml. No se adopta limine 0.6.5 porque requiere ptr_metadata experimental en la versión inspeccionada.

El bootloader y sus punteros son de confianza: los bindings dependen de su validez y duración. Se comprueba disponibilidad de respuestas, revisión, rangos no vacíos/sin overflow, orden/no solapamiento, límite de 4096 entradas y presencia de memoria usable. Esto no protege de un cargador malicioso ni configura tablas de páginas propias.

El kernel entra con interrupciones deshabilitadas, valida el arranque e instala GDT/TSS/IDT y PIC/PIT propios antes de habilitar IRQ0, en una CPU. No reclama memoria del cargador ni crea allocator. Los segmentos del ELF separan escritura y ejecución. La prueba usa disco de solo lectura, variables OVMF desechables y ningún disco o directorio personal.

Las licencias de Limine, los bindings, bitflags y Rust se incluyen en la imagen. OVMF y QEMU permanecen externos. Véase [inventario de componentes](dependencies.md). #33 añade [excepciones y tiempo](INTERRUPTS.md); memoria y procesos continúan en #9/#10.
