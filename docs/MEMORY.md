<!-- SPDX-License-Identifier: Apache-2.0 -->
# Memoria física, virtual y protecciones — #9

## Alcance R0

El kernel asigna y libera marcos físicos de 4 KiB, construye tablas de páginas propias y cambia entre espacios de direcciones con datos independientes. La base sigue siendo x86_64, cuatro niveles de paginación, una CPU y QEMU q35/TCG con 256 MiB. Se rechazan LA57, PCID y CPU sin NX. No hay nuevas dependencias Cargo ni cambio de toolchain.

El asignador mínimo tiene granularidad de página. No hay aún `GlobalAlloc`, `Box`, heap de tamaños variables, swapping o NUMA. Las pruebas propias de este subsistema cambian CR3 en ring 0; #10 añade por separado [procesos y planificación con pruebas en ring 3](PROCESSES.md).

## Responsabilidades y propiedad

| Módulo | Responsabilidad |
| --- | --- |
| `memory/frames.rs` de la biblioteca | Inventario, reservas, asignación y liberación por bitmaps, sin punteros ni CPU |
| `memory/page.rs` de la biblioteca | Direcciones virtuales válidas y política de permisos |
| `arch/x86_64/memory/physical.rs` | Propietario único de bitmaps, marcos y acceso crudo por HHDM |
| `cpu.rs` | CR0.WP, EFER.NXE, CR3 e invalidación de traducciones |
| `tables.rs` | Recorrido de tablas, permisos efectivos y división de hojas grandes |
| `bootstrap.rs` | Copia privada de tablas del cargador y protección del kernel/alias |
| `space.rs` | Ciclo de vida de raíces, mapeo, desmapeo y devolución de recursos |
| `memory/tests/` de arquitectura | Asignación, agotamiento, espacios, hojas grandes y fallos deliberados |
| `boot/limine.rs` | Traduce respuestas del cargador a valores propios; no filtra tipos Limine al asignador |

`Memory` posee el asignador físico y la raíz de kernel; los espacios secundarios dependen de que esa raíz y sus tablas superiores sigan vivas. El token es local a una CPU. No se asigna ni libera memoria en handlers de IRQ/NMI, y los métodos requieren acceso exclusivo al propietario. Este modelo no sustituye locks/SMP ni autoriza asignación concurrente desde interrupciones. No se exponen referencias Rust a páginas que puedan invalidarse al cambiar de espacio.

## Inventario y reservas

Dos bitmaps estáticos de 32 KiB registran marcos administrables y marcos asignados: 64 KiB de metadatos para direcciones físicas inferiores a 1 GiB. Ese es un límite explícito del corte, no una promesa de soporte probado para una máquina de 1 GiB. Una región usable que lo sobrepase se rechaza antes de importarla; no se ignora silenciosamente. Ampliarlo requiere revisar metadatos, direcciones, presupuestos y pruebas.

Se valida completamente el mapa antes de importar páginas. Solo entran páginas completas de regiones `USABLE`; cualquier tipo desconocido, firmware, ACPI, ejecutable/módulos o memoria del cargador queda excluido. Se reserva además el primer MiB y la extensión física completa del ELF, obtenida de la respuesta de dirección del ejecutable y de símbolos del linker. Las reservas redondean hacia fuera; las regiones usables se redondean hacia dentro. Se rechazan liberación doble, direcciones desalineadas, marcos no administrados y reservas de marcos en uso.

El cargador sigue siendo de confianza. Sus tablas, respuestas y pila de arranque permanecen reservadas aunque ya existan tablas propias. La semántica de `USABLE`, HHDM y la dirección física del ejecutable se toma de la [revisión fijada del protocolo Limine](https://github.com/limine-bootloader/limine-protocol/blob/da65184e91f80fcb397270121b1e2515a11e01ee/PROTOCOL.md). El validador R0 exige mapa ordenado y sin solapamientos, incluso para regiones reservadas: puede rechazar mapas que otra plataforma pudiera admitir; no se acredita compatibilidad universal.

Agotar el inventario devuelve un error tipado y conserva el estado. Se limpia una página completa antes de exponerla en otro mapeo o espacio. No se ejecuta una búsqueda interminable, no se roba memoria reservada y no se sustituye el fallo por una dirección nula.

## Tablas y permisos

Antes de activar una raíz propia se copian recursivamente las tablas superiores del cargador en marcos nuevos. Se vacía la mitad inferior, se retira el acceso de usuario de todos los mapeos heredados y se deshabilita la ejecución por defecto. Se mantienen las direcciones superiores necesarias para código, pila, IRQ, TSS y HHDM. La raíz y todos sus descendientes son propios; no se modifican las tablas del cargador.

Los símbolos del linker delimitan código, datos de solo lectura, datos modificables y final del ELF. El código queda RO/X; datos constantes y peticiones quedan RO/NX; datos modificables, bitmaps y tablas quedan RW/NX. Los alias HHDM del código y los datos constantes también son de solo lectura y no ejecutables. Sin esa protección de alias, otra dirección permitiría modificar el mismo código físico. La pila de emergencia de #33 conserva 16 KiB utilizables y añade una página de guarda, desmapeada tanto en la dirección del kernel como en HHDM. La pila de arranque heredada no gana una guarda en este corte.

Cuando hace falta granularidad de 4 KiB, se dividen las hojas heredadas grandes conservando direcciones y atributos de caché, incluida la distinta posición de PAT. El caso de 1 GiB se comprueba estructuralmente mediante una raíz sintética que nunca se activa; no se acredita ejecución de esa hoja en el CPU de referencia. El arranque y los accesos reales verifican los mapeos usados por R0.

Se habilitan CR0.WP y EFER.NXE y se comprueba NX en CPUID. PGE queda deshabilitado y las tablas nuevas no crean entradas globales. Cambiar CR3 e invalidar traducciones respeta las reglas de paginación del volumen 3 del [Intel SDM](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html). Los accesos crudos a tablas están acotados a páginas residentes y entradas alineadas; las operaciones no crean referencias Rust que puedan aliasar las modificaciones A/D del hardware.

## Asignación virtual y espacios

Los mapeos nuevos se restringen a páginas alineadas de la mitad canónica inferior, excluyendo la página nula. La API asigna sus propios marcos; no permite que un solicitante elija un marco del kernel. Rechaza mapeos duplicados y permisos simultáneamente escribibles/ejecutables. El bit de usuario solo se concede en esas páginas nuevas, con permisos efectivos calculados a través de todos los niveles.

Una asignación que agota memoria durante la creación de tablas elimina las entradas añadidas y devuelve todos los marcos de esa operación. Desmapear invalida la traducción activa antes de reutilizar el marco, retira tablas vacías y recarga la raíz para invalidar también las cachés de recorrido. No existen otros CPUs que necesiten un TLB shootdown; introducirlos exige otra política de sincronización.

Los espacios secundarios comparten las tablas superiores del kernel y poseen su raíz, tablas inferiores y páginas de datos. Las tablas superiores quedan fijas tras el bootstrap; no se proporciona una API para mutarlas después. Destruir un espacio inactivo devuelve sus recursos inferiores y raíz, nunca las tablas superiores compartidas. Se rechaza destruir el espacio activo o la raíz del kernel. La destrucción es explícita; #10 la vincula a la recogida del proceso terminado. No se implementa todavía conteo de referencias, páginas compartidas, copy-on-write ni un recolector de espacios abandonados.

Si falla la construcción inicial de tablas, el arranque termina con diagnóstico antes de activar la nueva raíz. Esa ruta terminal no intenta continuar como un kernel parcialmente inicializado. La reversión y recuperación de recursos se comprueban para las operaciones ordinarias posteriores.

## Pruebas y evidencia

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py test --timeout 15
python3 tools/sandbox.py prepare
python3 tools/sandbox.py test --revision "$(git rev-parse HEAD)"
```

`ok` exige que las pruebas de IRQ y memoria terminen antes de SUCCESS. Las pruebas de memoria realizan 16 ciclos de mapeo/escritura/desmapeo, rechazan duplicados y W+X, comprueban código/rodata/alias/guarda, crean dos espacios con la misma dirección virtual y marcos distintos, alternan CR3 y verifican sus datos, rechazan destruir el activo y devuelven sus recursos. Una prueba estructural comprueba la división 1 GiB → 2 MiB → 4 KiB y PAT.

Después se agota el inventario físico real del invitado. La lista de marcos temporales se guarda dentro de ellos para evitar una gran reserva adicional de heap o pila. Se libera un único marco, se vuelve a obtener ese mismo marco y se comprueban sus 4 KiB limpios. Se dejan solo dos marcos libres y se intenta una asignación que necesita cuatro: debe fallar y devolver lo adquirido parcialmente. Finalmente se recupera exactamente el contador libre inicial.

Cinco fixtures deben producir #PF (vector 14), CR2 igual a la dirección anunciada y el código concreto:

| Modo | Acceso deliberado | Error |
| --- | --- | --- |
| `memory-ro` | Escribir página RO desde ring 0 | 0x3 |
| `memory-nx` | Ejecutar una página NX | 0x11 |
| `memory-unmapped` | Leer tras desmapear y liberar | 0x0 |
| `memory-text-alias` | Escribir código mediante su alias HHDM | 0x3 |
| `memory-guard` | Leer la guarda de la pila de emergencia | 0x0 |

Se usan instrucciones de prueba explícitas, sin formar referencias Rust inválidas. Un acceso permitido por error acaba en UD2 y no satisface el resultado esperado. Un #DF, otra dirección, otro código o un timeout tampoco pasan como prueba de protección. Son fallos terminales de la VM de prueba; la supervivencia de un segundo proceso tras un fallo de aplicación se comprueba por separado en [#10](PROCESSES.md).

La suite directa tiene 13 escenarios y la aislada 17, conservando los anteriores de #8/#21/#33. Los resultados y logs quedan en los mismos directorios de evidencia; cada imagen conserva revisión, configuración y hashes. Una muestra inicial con 256 MiB registró 52.795 marcos administrados, 15 marcos de tablas propias y 52.780 libres antes y después del agotamiento; metadatos de 65.536 bytes. El arranque completo, con autopruebas, tardó unos 6,3 segundos. Son medidas de esa revisión/R0, no umbrales universales. La issue enlaza el commit y CI de cierre; revisión por el agente implementador, sin revisión independiente.
