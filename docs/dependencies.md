<!-- SPDX-License-Identifier: Apache-2.0 -->
# Inventario inicial de dependencias y herramientas

Fecha: 2026-09-09. Revisión realizada por el agente implementador; sin revisión independiente. Complementa [LICENSING.md](LICENSING.md).

Tras #8, Cargo.lock contiene rustic-kernel, xtask, limine 0.5.0 y bitflags 2.13.1. El código original es Apache-2.0; las crates conservan sus licencias. La imagen CI incorpora BOOTX64.EFI de Limine y el ELF propio; el firmware OVMF sigue siendo externo.

| Componente | Versión / fuente | Uso y distribución |
| --- | --- | --- |
| Rust/Cargo/rust-lld | Rust 1.98.1, commit 48a229ceaefd4985c50990b14116b6d856af0985; LLVM 22.1.8; static.rust-lang.org | Toolchain externa; no se redistribuye en el repo. Rust usa MIT/Apache-2.0; LLVM conserva sus términos y excepciones. |
| rustup | 1.29.1, instalación oficial | Gestor externo, no distribuido. |
| QEMU | 1:8.2.2+ds-0ubuntu1.18, Ubuntu noble | Ejecutor externo GPL-2.0 y componentes con avisos propios; no distribuido ni enlazado en kernel. |
| OVMF | 2024.02-2ubuntu0.9, Ubuntu noble | Firmware externo; avisos por componente en paquete Ubuntu/EDK II. No distribuido. |
| mtools / dosfstools / xorriso | Versiones exactas en tools/environment.toml, Ubuntu noble | Utilidades externas; conservar sus términos si se distribuyen en el futuro. |
| Limine | 12.8.0, [release oficial](https://github.com/Limine-Bootloader/Limine/releases/tag/v12.8.0) | BOOTX64.EFI extraído del archivo verificado e incorporado a la imagen. LICENSE BSD-2-Clause de Mintsuki y colaboradores se copia íntegra a /licenses/LIMINE.txt. |
| checkout / upload-artifact | SHA en workflow; repositorios oficiales actions | Acciones remotas de CI, no vendorizadas. No proporcionan una licencia al código del proyecto. |
| Docker Engine / Ubuntu container | Engine 29.7.2 validado localmente; Ubuntu 24.04 amd64 con digest en tools/sandbox_support/prepare.py | Herramientas externas de #21. Imagen construida localmente, sin publicación en registro; cada paquete conserva sus avisos. La identidad final se guarda por trabajo. |

tools/environment.toml conserva URL/hash del archivo Limine, hashes de OVMF y revisión del protocolo. OVMF no se empaqueta en la imagen ni en los artefactos de CI. Los avisos originales de dependencias se conservan en licenses/ y dentro de la imagen; esos textos no se relicencian bajo Apache-2.0.

## Código incorporado al ELF de #8

| Componente | Versión/checksum del lockfile | Aviso distribuido |
| --- | --- | --- |
| limine, bindings Rust | 0.5.0 / af6d2ee42712e7bd2c787365cd1dab06ef59a61becbf87bec7b32b970bd2594b | MIT OR Apache-2.0; se conserva LICENSE-MIT en licenses/limine-rust-MIT.txt. |
| bitflags | 2.13.1 / b588b76d00fde79687d7646a9b5bdf3cc0f655e0bbd080335a95d7e96f3587da | Dependencia transitiva no_std; MIT conservada en licenses/bitflags-MIT.txt. |
| Rust core/runtime | Toolchain 1.98.1 | LICENSE-MIT oficial del tag conservada en licenses/rust-MIT.txt. |

Estas licencias se copian a /licenses junto a RUSTIC.txt (Apache-2.0 del proyecto). La biblioteca pura sigue sin depender de Limine; los bindings solo se activan para el binario boot-image. No se copia código de un template de kernel. Limine 0.6.5 se inspeccionó y descartó por necesitar ptr_metadata experimental; no está en el artefacto. Se prueba limine 0.5.0 con base revision 3 y cargador 12.8.0.

Fuentes de revisión para componentes externos: LICENSE del archivo de Limine verificado por hash; metadatos y copyright de los paquetes instalados bajo /usr/share/doc; [Rust copyright](https://github.com/rust-lang/rust/blob/master/COPYRIGHT), [QEMU licencia](https://www.qemu.org/docs/master/about/license.html). Las condiciones concretas de un futuro paquete distribuido se revisan antes de publicarlo.
