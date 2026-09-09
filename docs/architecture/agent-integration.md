<!-- SPDX-License-Identifier: Apache-2.0 -->

# Integración de agentes: contratos nativos y adaptadores

Fecha: 2026-09-09. Estado: propuesta técnica investigada, pendiente de contratos ejecutables y mediciones en #6/#43. No es una ABI aprobada ni funcionalidad implementada.

## Conclusión y alcance

RusticOS debe ofrecer servicios estructurados a personas, aplicaciones y agentes. MCP será un adaptador de interoperabilidad, no el protocolo obligatorio del kernel, de las aplicaciones o del piloto integrado. La ventaja de producto es poder descubrir capacidades reales, operar recursos y verificar efectos sin interpretar píxeles. Una API HTTP por cada syscall no consigue ese objetivo.

La separación por capas estaba en #1/#6/#22/#23/#39, pero faltaban métodos concretos y una comparación de las rutas. Además, #39 imponía HTTPS aun cuando un cliente local podría usar stdio. Esta revisión concreta el trabajo y corrige esa dependencia.

## Capas propuestas

| Capa | Responsabilidad | Elección propuesta |
| --- | --- | --- |
| Kernel/IPC (#3/#34) | Aislamiento, handles con derechos, canales, espera, límites | ABI explícita y mensajes acotados; decidir codificación y transferencia de handles con pruebas de portabilidad. |
| Servicios/SDK (#6/#11/#13) | Recursos, métodos tipados, estados, eventos y autoridad efectiva | Contratos versionados; una implementación de cada servicio compartida por CLI, GUI y tools. |
| Herramientas (#22) | Acciones comprensibles para agentes, selección contextual y resultados comprobables | Catálogo nativo con entradas/salidas estructuradas y adaptaciones revisadas de los contratos. |
| Piloto integrado (#23) | Bucle de observación, llamada al modelo, ejecución autorizada y verificación | Ejecutar localmente las llamadas estructuradas del modelo mediante el catálogo nativo. |
| Clientes externos (#39) | Interoperabilidad con hosts de agentes | MCP con versión y transporte probados contra un cliente independiente. |
| Integraciones adicionales | Clientes convencionales o delegación a agentes independientes | Evaluar HTTP/OpenAPI, gRPC o A2A solo cuando exista un consumidor y una necesidad medible. |

El registro permite descubrir servicios, pero no debe convertirse necesariamente en un proxy por el que pase todo byte del SO. Separar el control (comandos, permisos, estados) de los datos voluminosos (archivos, imágenes, vídeo): streams o handles autorizados y acotados, sin copiar todo al contexto del modelo.

Fuchsia muestra una separación entre definiciones tipadas, bindings y canales IPC; inspira el diseño, sin implicar portar FIDL completo. D-Bus aporta patrones de introspección, propiedades, eventos y versionado. [FIDL](https://fuchsia.dev/fuchsia-src/concepts/fidl/overview), [diseño de APIs D-Bus](https://dbus.freedesktop.org/doc/dbus-api-design.html).

gRPC ofrece contratos de servicio y llamadas simples o streaming. Es una alternativa a estudiar para clientes que lo requieran; su utilidad no demuestra que su runtime sea la mejor base para un SO nuevo. [Conceptos de gRPC](https://grpc.io/docs/what-is-grpc/core-concepts/).

## Cómo se conecta el agente

1. El piloto obtiene las herramientas disponibles y permitidas para su sesión.
2. Envía al modelo solo descriptores relevantes y el contexto autorizado.
3. El modelo devuelve nombre y argumentos estructurados.
4. El ejecutor local valida esquema, límites y delegación; el servicio vuelve a comprobar la autoridad al actuar.
5. El servicio devuelve resultado o identificador de operación.
6. El piloto consulta estado/evidencia y decide el siguiente paso dentro del presupuesto.

Este flujo permite que la inferencia sea remota mientras el ejecutor vive dentro de RusticOS; no requiere publicar un servidor entrante del SO. Es una elección de arquitectura basada en el flujo documentado de function calling, donde la aplicación ejecuta el código. [OpenAI function calling](https://developers.openai.com/api/docs/guides/function-calling).

Un cliente externo puede usar MCP para llegar al mismo catálogo. La especificación consultada distingue stdio y Streamable HTTP. Elegir stdio no resuelve por sí solo cruzar anfitrión/invitado: si el cliente corre fuera de la VM, hace falta un puente definido y autorizado. Para HTTP remoto se añaden #16/#17, autenticación y el alcance de exposición. No crear transportes personalizados salvo necesidad demostrada. [Transportes MCP 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports).

Hay que fijar versión de protocolo y comprobar compatibilidad real de SDK/cliente, no seguir automáticamente latest. Las conexiones MCP administradas por proveedores pueden admitir transportes distintos de los de un host local. [OpenAI MCP](https://developers.openai.com/api/docs/guides/tools-connectors-mcp).

A2A trata la colaboración entre agentes independientes y sus tareas; no sustituye las APIs de archivos, procesos o ventanas. Queda como opción futura, sin nueva dependencia de v0.1. [A2A y MCP](https://a2a-protocol.org/dev/topics/a2a-and-mcp/).

## Primer contrato vertical propuesto

Nombres lógicos provisionales, no URLs ni syscalls finales. Las ocho operaciones deben existir primero en un backend determinista (#43), después contra servicios reales (#22). Los fixtures pueden precrear el workspace y el archivo: esto no pretende ser todavía una API de archivos completa.

| Método | Entrada principal | Salida observable |
| --- | --- | --- |
| capabilities.list | filtro, cursor, límite | capacidades visibles, versión, disponibilidad, cursor siguiente |
| capabilities.describe | capability_id, versión soportada | esquema, efectos, permisos requeridos, límites |
| files.read | workspace_id, resource_id, rango acotado | contenido, versión, hash, truncamiento explícito |
| files.replace | workspace_id, resource_id, expected_version, contenido acotado, idempotency_key | operación o recibo con nueva versión y hash |
| operations.get | operation_id | estado, progreso, efectos conocidos, evidencia |
| operations.cancel | operation_id | solicitud de cancelación aceptada/rechazada; estado actual |
| events.read | ámbito, cursor, límite, espera máxima | cambios autorizados, cursor o indicación de resincronización |
| system.status | selección de campos | salud y perfil/capacidades activos, instante de observación |

Ejemplo de argumentos de files.replace:

```json
{
  "workspace_id": "ws-demo",
  "resource_id": "file-demo",
  "expected_version": "v7",
  "content_utf8": "Hola RusticOS\n",
  "idempotency_key": "mission-demo-step-2"
}
```

La identidad y los derechos no son argumentos que el modelo pueda inventar: vienen de la sesión/handles autenticados. Los identificadores son referencias, no permisos. Resolver y autorizar el recurso dentro de su workspace; comprobar precondición y cambio de forma atómica en el servicio.

Una aceptación asíncrona devuelve operation_id; una operación finalizada devuelve estado, resource_id, versión/hash y referencia de auditoría. La misión verifica el resultado mediante files.read y la versión/hash esperadas. Nunca deduce éxito únicamente del texto producido por el modelo.

## Semántica que hay que cerrar antes de ampliar el catálogo

- Esquemas de entrada/salida con límites de tamaño, profundidad, rangos y campos permitidos; documentación de efectos y errores.
- Estado de ejecución: queued, running, succeeded, failed o cancelled. Distinguir cancel_requested del resultado final. Una desconexión puede dejar al cliente sin conocer el desenlace: reconciliar mediante operation_id/clave antes de repetir.
- Idempotencia por identidad, ámbito y clave; definir periodo de retención. Misma clave con argumentos distintos produce conflicto. No prometer exactamente una vez para efectos externos.
- Cancelar detiene trabajo cuando sea posible; no implica deshacer efectos ya confirmados. Reportar efectos parciales y posibilidades reales de compensación.
- Errores distinguibles: invalid_argument, permission_denied, not_found, version_conflict, unavailable, resource_exhausted; indicar posibilidad de reintento sin filtrar recursos ajenos.
- Eventos paginados, cursor, retención acotada, backpressure y resincronización tras pérdida. No prometer registro eterno ni entrega exactamente una vez.
- Presupuestos de tiempo, resultados y recursos. Los timeouts de transporte no equivalen a fallo de la operación.
- Fuente contractual canónica con generación/validación de descriptores. JSON Schema es candidato práctico para la frontera de tools; no representa por sí solo transferencia de handles del kernel. Documentar explícitamente la correspondencia con tipos del IPC, bytes, enteros y versiones.
- Descubrimiento por tarea y sesión: no enviar cientos de tools a cada petición del modelo ni confundir una capacidad disponible con una autorizada.
- Autoridad en servicios/kernel, políticas explícitas y revocación. MCP describe intercambios; no puede hacer cumplir por sí mismo la autoridad del SO. [Especificación MCP](https://modelcontextprotocol.io/specification/2026-07-28).

## Expansión por producto

| Familia | Operaciones a diseñar | Responsable |
| --- | --- | --- |
| Archivos/workspaces | listar, crear, mover, eliminar, patch y versiones | #6/#12/#25 |
| Procesos y servicios | listar, iniciar, consultar, detener; estado y reinicio | #6/#10/#13 |
| Configuración y adaptación | consultar, validar, previsualizar y aplicar; explicar degradación | #6/#38 |
| Escritorio/aplicaciones | listar ventanas, activar, acciones semánticas, accesibilidad y captura seleccionada | #40 |
| Navegador | navegar, consultar contenido seleccionado, identificar elementos y actuar con estado verificable | #41 |
| Construcción/candidatas | enviar trabajo acotado, seguir pruebas, verificar y activar artefacto | #42/#26/#27 |

La cobertura de apps propias incluye acciones de producto y estados semánticos. Las apps ajenas solo tendrán la cobertura que permita su API/adaptador/accesibilidad; declarar dónde se necesita visión. No es realista prometer control semántico completo de cualquier binario externo.

## Experimento de decisión (#43)

Comparar el mismo contrato y los mismos fixtures por:
- cliente nativo de referencia;
- adaptador de function calling con respuestas de modelo simuladas;
- adaptador MCP mínimo en el anfitrión con cliente independiente.

El prototipo de adaptadores es desechable y acotado; no requiere todos los servicios de RusticOS ni completa #23/#39. Mantener fijo payload, backend, hardware, versiones y carga. Medir latencia p50/p95, RAM, bytes por misión, número de llamadas y coste de integración. Separar tiempo de transporte del tiempo de inferencia; este último no se mide con simulación.

Pruebas obligatorias: misión correcta, denegación, revocación, argumentos inválidos, conflicto concurrente, reintento tras perder respuesta, cancelación con efectos parciales, paginación y cursor expirado. Una ruta que cambie permisos o declare éxito falso se descarta, aunque sea más rápida.

Registrar candidatos IPC/codificación y requisitos de runtime; no extrapolar cifras del host a la VM. Fijar presupuestos de aceptación a partir de la línea base antes de seleccionar implementación. Repetir los casos en el invitado al cerrar #22/#39. La calidad de selección de herramientas por un modelo real se evalúa en #23, no con respuestas simuladas.

## Mejoras realistas

- Especificación ejecutable antes de multiplicar endpoints: descubrir → leer → modificar con precondición → verificar, también bajo fallos.
- Mapa de capacidades consultable que explique por qué una función está ausente, degradada o no autorizada cuando la política permita revelarlo.
- Acciones de producto y árbol semántico de aplicaciones como parte del SDK; la GUI y el agente actúan sobre el mismo estado.
- Previsualización de cambios para configuración y candidatas cuando sea realizable; no inventar dry-run perfecto para operaciones irreversibles.
- Recibos estructurados y registro reproducible de misiones con datos sensibles omitidos; ampliar a replay en entornos desechables después.
- Posponer A2A y adaptadores HTTP/gRPC adicionales hasta contar con un consumidor real. Reducir protocolos iniciales conserva recursos para kernel, drivers, SDK y navegador.

## Decisiones pendientes

#3/#34: IPC, ABI y codificación con medidas y portabilidad. #6: esquemas ejecutables y semántica definitiva. #39: pareja cliente/SDK, revisión MCP y transporte. #43: evidencia comparativa. No se afirma todavía que una biblioteca o transporte sea el mejor: esta propuesta fija las fronteras y cómo decidir.
