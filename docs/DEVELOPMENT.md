<!-- SPDX-License-Identifier: Apache-2.0 -->
# Desarrollo y comprobaciones

## Entorno

Base verificada: Ubuntu 24.04 amd64 (también bajo WSL2), Rust 1.98.1, rustup 1.29.1. En Windows ejecutar los comandos siguientes dentro de Ubuntu. El checkout puede estar en /mnt/c; para mejores tiempos de E/S puede usarse un checkout en el filesystem de Linux.

Instalar las herramientas básicas con apt (requiere administrador):
```sh
sudo apt-get update
sudo apt-get install -y build-essential curl python3 git
```

Si falta rustup, obtenerlo de la [instalación oficial](https://rust-lang.github.io/rustup/installation/index.html). En esta base se usó el instalador oficial con --profile minimal --no-modify-path; activar el entorno mediante:
```sh
. "$HOME/.cargo/env"
rustup toolchain install 1.98.1 --profile minimal --component rustfmt --component clippy --target x86_64-unknown-none
```
rust-toolchain.toml selecciona esa versión exacta. No se requiere nightly ni crates de terceros. Cargo.lock está versionado; las comprobaciones usan --locked. El enlazador del target es rust-lld incluido en esa toolchain; todavía solo se genera una biblioteca, no un ELF arrancable.

## Comandos ejecutables

Desde la raíz:
```sh
cargo xtask check
```

Ejecuta secuencialmente formato, Clippy con warnings como errores, tests de host, compilación no_std y Clippy para el target del kernel. Para aplicar formato:
```sh
cargo fmt --all
```

La biblioteca kernel valida aritmética de rangos de memoria suministrados durante arranque. Sus tests se ejecutan en el host; no demuestran inicialización de memoria, aislamiento ni arranque del invitado.

## Organización

- kernel/src/lib.rs: entrada y composición de la biblioteca no_std.
- kernel/src/boot/mod.rs: fachada del módulo de datos de arranque.
- kernel/src/boot/region.rs: validación de rangos; no depende de Limine ni asigna memoria.
- kernel/tests/boot_regions.rs: pruebas del contrato público y sus casos límite.
- tools/xtask/src/main.rs: entrada de la utilidad de host.
- tools/xtask/src/checks.rs: secuencia de comprobaciones.
- tools/xtask/src/command.rs: ejecución de procesos y propagación de errores.

No hay directorios de futuros subsistemas vacíos. Las reglas de modularidad están en [AGENTS.md](../AGENTS.md). El forbid(unsafe_code) actual corresponde a esta biblioteca pura; cualquier futura frontera privilegiada debe introducirse en un módulo adecuado con revisión explícita.

## Herramientas para el primer arranque

No son necesarias para cargo xtask check. Se fijan ahora para #8:
```sh
python3 tools/environment.py install
python3 tools/environment.py verify
python3 tools/environment.py fetch-bootloader
```

install instala las versiones exactas de tools/environment.toml; verify rechaza paquetes o hashes de firmware diferentes y comprueba la máquina pc-q35-8.2. fetch-bootloader descarga Limine 12.8.0 y comprueba SHA-256 sin extraer ni ejecutar el archivo. .cache queda fuera del repositorio. Si una versión deja de estar disponible en apt, la instalación falla: actualizar la base con revisión/evidencia, no sustituir silenciosamente por latest. Las dependencias transitivas de Ubuntu y la imagen del runner no están fijadas por digest; esta base no promete reconstrucción hermética ni identidad binaria.

La imagen FAT/EFI, bindings y revisión concreta del protocolo Limine usados por el kernel se incorporarán y verificarán en #8. No hay todavía comando run ni imagen arrancable.

## CI y prueba negativa

.github/workflows/check.yml ejecuta el mismo cargo xtask check en ubuntu-24.04, para push y pull_request. Acciones fijadas por SHA, token contents:read, checkout sin credenciales persistentes y sin secretos de proyecto. Registra versión de toolchain, logs y biblioteca no_std como artefactos con retención de 14 días. QEMU no se instala en esta CI porque aún no arranca una VM.

Después, tools/check-failure.sh introduce un test que falla deliberadamente y exige que cargo xtask check lo rechace. La prueba identifica el marcador esperado para no aceptar como evidencia un error de compilación o de herramientas. Ejecutarla solo en checkout desechable: formatea y añade temporalmente el fixture, retirado al salir.

```sh
bash tools/check-failure.sh
```

El mismo usuario de CI podría modificar código de una PR; el flujo limita permisos y no suministra credenciales de publicación. La CI del arranque llegará en #8/#21.
