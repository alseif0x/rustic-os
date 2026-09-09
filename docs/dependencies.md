<!-- SPDX-License-Identifier: Apache-2.0 -->
# Inventario inicial de dependencias y herramientas

Fecha: 2026-09-09. Revisión realizada por el agente implementador; sin revisión independiente. Complementa [LICENSING.md](LICENSING.md).

Cargo.lock contiene únicamente rustic-kernel y xtask, ambos código original Apache-2.0. No hay crates de terceros ni código vendorizado. El artefacto de CI es una biblioteca del proyecto; no contiene el cargador o firmware.

| Componente | Versión / fuente | Uso y distribución |
| --- | --- | --- |
| Rust/Cargo/rust-lld | Rust 1.98.1, commit 48a229ceaefd4985c50990b14116b6d856af0985; LLVM 22.1.8; static.rust-lang.org | Toolchain externa; no se redistribuye en el repo. Rust usa MIT/Apache-2.0; LLVM conserva sus términos y excepciones. |
| rustup | 1.29.1, instalación oficial | Gestor externo, no distribuido. |
| QEMU | 1:8.2.2+ds-0ubuntu1.18, Ubuntu noble | Ejecutor externo GPL-2.0 y componentes con avisos propios; no distribuido ni enlazado en kernel. |
| OVMF | 2024.02-2ubuntu0.9, Ubuntu noble | Firmware externo; avisos por componente en paquete Ubuntu/EDK II. No distribuido. |
| mtools / dosfstools / xorriso | Versiones exactas en tools/environment.toml, Ubuntu noble | Utilidades externas; conservar sus términos si se distribuyen en el futuro. |
| Limine | 12.8.0, [release oficial](https://github.com/Limine-Bootloader/Limine/releases/tag/v12.8.0) | Descarga local opcional de archivo binario; no extraído ni incorporado a imagen todavía. LICENSE del archivo comprobada: BSD-2-Clause, Mintsuki y colaboradores. |
| checkout / upload-artifact | SHA en workflow; repositorios oficiales actions | Acciones remotas de CI, no vendorizadas. No proporcionan una licencia al código del proyecto. |

tools/environment.toml conserva URL/hash del archivo Limine y hashes de OVMF. Antes de distribuir la imagen de #8, incluir los avisos requeridos de Limine junto al binario, revisar el inventario del firmware si se empaqueta y registrar los bindings/crates seleccionados. Esta base no autoriza redistribuir OVMF sin revisar todos sus avisos. No hay NOTICE nuevo porque esta entrega no incorpora material de terceros al repositorio.

Fuentes de revisión para componentes externos: LICENSE del archivo de Limine verificado por hash; metadatos y copyright de los paquetes instalados bajo /usr/share/doc; [Rust copyright](https://github.com/rust-lang/rust/blob/master/COPYRIGHT), [QEMU licencia](https://www.qemu.org/docs/master/about/license.html). Las condiciones concretas de un futuro paquete distribuido se revisan antes de publicarlo.
