<!-- SPDX-License-Identifier: Apache-2.0 -->

# Contribuir a RusticOS

El [plan vigente](https://github.com/alseif0x/rustic-os/issues/1) define requisitos y reglas de ejecución. Las issues indican resultado, dependencias, entregables, pruebas y límites. Los milestones agrupan criterios de aceptación, sin impedir investigaciones tempranas.

## Antes de implementar

Elige una tarea con dependencias resueltas, acuerda su responsable y la revisión disponible, y concreta el cambio verificable. Si necesita varios cambios grandes independientes, desglósala conservando enlaces.

El plan puede mejorar: explica problema, propuesta, alternativas, coste de mantenimiento, experimento y criterios de éxito. Actualiza las dependencias y requisitos afectados. No mantengas una decisión solo porque exista en un documento.

Ejecuta `cargo xtask check` según [la guía de desarrollo](docs/DEVELOPMENT.md). Comprueba formato, lints, tests del anfitrión y compilación de la biblioteca `no_std`. El arranque del SO se incorpora en #8; estas comprobaciones no lo sustituyen. Aplica las reglas de módulos/submódulos y separación de responsabilidades de [AGENTS.md](AGENTS.md).

## Criterios de cambio

- Conecta cada capacidad propia de producto con API/tool, autoridad, estado y una forma independiente de verificar el efecto.
- Aplica pruebas positivas y negativas a cada frontera desde su implementación: memoria, IPC, dispositivos, servicios, transporte y herramientas.
- Enlaza código o decisión, revisión/configuración, comandos, resultados y límites. Distingue fixture del anfitrión de ejecución real en RusticOS.
- Documenta invariantes y revisión de cada frontera `unsafe`.
- Conserva la lógica de servicio común a consola, GUI y agente; el modelo y MCP no conceden privilegios.
- Mantén las casillas pendientes hasta tener evidencia. El mismo criterio se aplica a contribuciones humanas y generadas con IA.

## Licencias y procedencia

El material original se publica bajo [Apache-2.0](LICENSE). Usa `SPDX-License-Identifier: Apache-2.0` con la sintaxis de comentario del archivo: por ejemplo, comentario de línea en Rust o script, y comentario HTML en Markdown. No rompas formatos que no admiten comentarios.

Conserva los avisos de copyright que correspondan a su autoría real; no inventes titulares, elimines atribuciones ni sustituyas cabeceras de terceros. Antes de incorporar una dependencia, completa su registro y revisión en [docs/LICENSING.md](docs/LICENSING.md).

No se añade un acuerdo separado de contribución. Aporta únicamente material que tengas derecho a contribuir y declara cualquier contenido con condiciones distintas.
