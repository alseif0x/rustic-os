<!-- SPDX-License-Identifier: Apache-2.0 -->

# RusticOS

Sistema operativo independiente, desarrollado principalmente en Rust, diseñado para que personas y agentes puedan utilizar sus capacidades mediante servicios comunes.

Cada capacidad propia debe ofrecer una forma estructurada de descubrirla, consultar su estado, operarla y verificar el resultado. Las APIs, las herramientas semánticas y la interoperabilidad MCP facilitan el control por agentes. La interfaz gráfica y el acceso visual son complementarios; la IA será opcional.

Universalidad y adaptación son objetivos progresivos, demostrados por arquitectura, dispositivo, aplicación y escenario. Las versiones prometerán únicamente capacidades verificadas.

## Estado

Fase inicial: kernel Rust modular que arranca mediante Limine/UEFI en QEMU, valida el mapa de memoria y emite diagnóstico serie. Las pruebas distinguen éxito, panic, bloqueo y argumentos inválidos. Todavía no hay shell, procesos aislados, aplicaciones ni piloto. Consulta [desarrollo](docs/DEVELOPMENT.md) y [arranque y pruebas](docs/BOOT.md) para ejecutar la base.

El [ejecutor aislado](docs/EXECUTOR.md) construye revisiones Git y comprueba el arranque en contenedores separados, sin red durante los trabajos y con límites de recursos, cancelación y resultados JSON.

El kernel incorpora [excepciones, interrupciones y reloj](docs/INTERRUPTS.md): tablas de CPU propias, pila de emergencia para doble fallo, temporizador y esperas acotadas por plazos. Las pruebas verifican estas funciones dentro de QEMU antes de informar éxito.

## Plan y participación

- [Plan vigente: requisitos, hitos, riesgos y reglas de ejecución](https://github.com/alseif0x/rustic-os/issues/1).
- [Milestones de GitHub](https://github.com/alseif0x/rustic-os/milestones).
- [Horizontes y propuestas para experimentar](https://github.com/alseif0x/rustic-os/issues/31).
- [Primer trabajo de definición: requisitos y plataforma](https://github.com/alseif0x/rustic-os/issues/2).
- [Guía de contribución](CONTRIBUTING.md).

La v0.1 experimental tiene como objetivo consola nativa, piloto opcional, navegador con motor y renderizado dentro de RusticOS y un ciclo verificable de cambio y recuperación. El modelo, compilador y entorno de pruebas pueden ser externos, declarando esa dependencia.

## Licencia

El código y la documentación originales de RusticOS se publican bajo **Apache License 2.0** (`Apache-2.0`). El texto completo está en [LICENSE](LICENSE).

Los componentes de terceros conservan sus licencias y avisos. Consulta el [inventario y las reglas de licencias](docs/LICENSING.md). La publicación de la licencia implementa la [decisión del propietario en #32](https://github.com/alseif0x/rustic-os/issues/32).
