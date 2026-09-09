<!-- SPDX-License-Identifier: Apache-2.0 -->

# RusticOS

Sistema operativo independiente, desarrollado principalmente en Rust, diseñado para que personas y agentes puedan utilizar sus capacidades mediante servicios comunes.

Cada capacidad propia debe ofrecer una forma estructurada de descubrirla, consultar su estado, operarla y verificar el resultado. Las APIs, las herramientas semánticas y la interoperabilidad MCP facilitan el control por agentes. La interfaz gráfica y el acceso visual son complementarios; la IA será opcional.

Universalidad y adaptación son objetivos progresivos, demostrados por arquitectura, dispositivo, aplicación y escenario. Las versiones prometerán únicamente capacidades verificadas.

## Estado

Fase inicial: planificación y base documental. El kernel, las aplicaciones y los agentes descritos aún están pendientes de implementación. No hay una imagen arrancable ni instrucciones de compilación verificadas; se incorporarán al resolver las tareas del entorno y arranque.

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
