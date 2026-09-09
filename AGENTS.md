<!-- SPDX-License-Identifier: Apache-2.0 -->

# Instrucciones para trabajar en RusticOS

Estas reglas aplican a todo el repositorio. El propietario exige Rust modular y submodular con separación de responsabilidades.

## Diseño e implementación

- Organizar por responsabilidad y límites de confianza: boot, arquitectura, memoria, procesos, IPC, drivers y servicios. Crear submódulos cuando haya conceptos independientes; no llenar un archivo con subsistemas distintos.
- main.rs y lib.rs son puntos de entrada/composición. mod.rs o el archivo raíz del módulo declara estructura, API pública y coordinación mínima; no acumula implementación de subsistemas.
- Mantener detalles privados por defecto. Usar pub(super) o pub(crate) cuando baste; exponer una API pública pequeña con tipos y errores definidos.
- Dependencias acíclicas y dirigidas. El kernel no depende del SDK de usuario, modelo, MCP, GUI o herramientas del anfitrión. Los contratos compartidos no importan implementaciones.
- Separar mecanismo de política: el kernel aplica memoria/handles/aislamiento; servicios deciden políticas de producto sobre esa autoridad.
- Encapsular instrucciones de CPU, MMIO/port I/O y unsafe en módulos estrechos; documentar invariantes de validez, propiedad, duración y concurrencia.
- Evitar módulos utils/common que mezclen responsabilidades, gestores que conozcan todo y estado global mutable sin propietario. El estado de cada subsistema tiene un dueño y reglas de acceso explícitos.
- Extraer una crate cuando exista una frontera útil de reutilización, plataforma, confianza o compilación. No crear una crate por archivo ni abstracciones/traits sin una necesidad real.
- Separar código del anfitrión (construcción/pruebas) del código del invitado. No introducir std accidentalmente en el kernel no_std.
- Dividir por cohesión y motivos de cambio, no por un límite arbitrario de líneas. No crear árboles vacíos para funcionalidades futuras.
- Un núcleo monolítico puede compartir espacio privilegiado y seguir siendo modular en código. La modularidad no acredita aislamiento de sus drivers.

## Revisión y validación

En cada cambio Rust comprobar responsabilidad de módulos, dirección de dependencias, visibilidad, propiedad de estado y nuevas fronteras unsafe. Añadir pruebas de comportamiento/fallos en la capa que posee el contrato; evitar pruebas que solo repitan la implementación.

Ejecutar formato, lints y pruebas que estén configurados y sean adecuados al cambio. Hasta que #4 los establezca, no inventar comandos exitosos. El arranque, los contratos y la autoridad tienen validaciones distintas; un mock del host no demuestra ejecución en RusticOS.

## Plan y alcance

Consultar docs/requirements-v0.1.md, docs/architecture/ADR-0001-kernel-and-boot.md y la issue activa. Conservar trazabilidad al revisar decisiones. No declarar capacidades implementadas ni cerrar pruebas por existir documentación. Seguir CONTRIBUTING.md y docs/LICENSING.md para procedencia y avisos.
