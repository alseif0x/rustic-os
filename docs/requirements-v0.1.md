<!-- SPDX-License-Identifier: Apache-2.0 -->

# Requisitos y plataforma de referencia — v0.1 experimental

Estado: base de trabajo adoptada para #2 el 2026-09-09. El propietario autorizó continuar con la propuesta de primera entrega mediante «adelante pues». Se conserva el alcance previamente acordado; las elecciones técnicas se realizan dentro de esa delegación. La aceptación del documento no acredita implementación ni aprobación individual previa de cada parámetro.

## Producto y límites

RusticOS es un SO propio principalmente en Rust. Debe funcionar manualmente sin IA y permitir a agentes operar capacidades propias por APIs/tools, con visión complementaria. El piloto vive en RusticOS; inferencia, compilación y VM de pruebas pueden ser externos, declarando su ubicación. Consola, GUI y agente comparten servicios y autoridad.

v0.1 incluye: arranque, procesos aislados, archivos persistentes, shell, herramientas nativas, adaptación determinista, red/HTTPS, piloto opcional, interoperabilidad MCP, escritorio y navegador con renderizado local, candidata verificable y recuperación. No promete producción, toda la web, cualquier dispositivo, compatibilidad binaria universal, inferencia local, compilación interna completa ni sustitución del kernel en caliente.

Universalidad se mide por separado: arquitectura CPU, dispositivos, aplicaciones e interacción. Inicialmente una CPU y un conjunto virtual de dispositivos. Adaptación significa consultar capacidades reales y seleccionar/degradar funciones según políticas y presupuestos; no modificar código automáticamente para improvisar soporte.

## Plataforma R0

Los siguientes son parámetros de la configuración de prueba, no requisitos mínimos de un producto terminado.

| Elemento | Base acordada |
| --- | --- |
| Desarrollo | Ubuntu 24.04 en WSL2 de este equipo (observado: 24.04.4 LTS). CI Linux Ubuntu 24.04; fijar imagen y herramientas en #4. |
| CPU/invitado | x86_64, modelo QEMU qemu64, 1 vCPU. No utilizar cpu=host como base reproducible. |
| Máquina | Familia QEMU q35; #4 fija el nombre pc-q35-X.Y disponible en la versión seleccionada. |
| Ejecución | TCG obligatorio en la prueba base, sin requerir virtualización anidada. Aceleración adicional tiene resultados separados. |
| Firmware | UEFI mediante OVMF, Secure Boot desactivado en R0; copia desechable de variables por ejecución. #4 fija build y hashes. |
| Arranque | Protocolo Limine, kernel ELF x86_64-unknown-none; imagen con partición FAT/EFI y cargador. #4 fija revisión del cargador/protocolo/bindings e imagen; #8 valida el recorrido. |
| Memoria | H0: 256 MiB. Prueba integrada inicial: 2048 MiB. Perfil restringido para #20/#38: 512 MiB. No se exige navegador en todos los perfiles; declarar degradación. |
| Disco | Arranque de solo lectura siempre que la herramienta lo permita; persistencia separada: virtio-blk-pci, disco virtual de 4 GiB con copia por escenario. Nunca discos físicos del anfitrión. |
| NIC | virtio-net-pci, desactivada H0/H1. Desde H3 red de pruebas aislada; egreso habilitado explícitamente por escenario. |
| Gráficos | Framebuffer entregado por el cargador con dispositivo VGA virtual estándar, modo objetivo 1024×768; renderizado software inicial. Sin aceleración 3D obligatoria. |
| Entrada | Teclado y ratón PS/2 emulados; consola serie para el mínimo manual. |
| Reloj/interrupciones | LAPIC/IOAPIC y temporización inicial a concretar en #33, bajo R0 monoprocesador; no requisito SMP inicial. |
| Diagnóstico | UART serie COM1 a log; monitor QEMU separado del canal de usuario. Salida de pruebas distinguible del texto de arranque. |
| Entropía | virtio-rng-pci para #17; fuente externa declarada, no inferir entropía del reloj. |
| Compartición | Sin carpetas personales, portapapeles ni dispositivos físicos compartidos por defecto. Puente de construcción exclusivo de #42. |

#4 debe registrar versiones exactas de Rust/Cargo, QEMU, OVMF, Limine, utilidades de imagen y CI; hashes/fuentes y comandos verificados. No descargar latest en cada construcción. #4 no se cierra con una ficha sin versiones. Esta asignación evita inventar versiones antes de probar compatibilidad.

## Requisitos trazables

Cada escenario se ejecutará contra el invitado salvo indicación expresa. Evidencia común: commit, configuración R0, herramientas, comandos, resultado esperado/obtenido, logs y hashes de artefactos. Conservar negativos, no solo el caso satisfactorio.

| ID | Requisito verificable | Escenario / evidencia | Issues |
| --- | --- | --- | --- |
| R01 | Imagen propia y diagnóstico de éxito, fallo y bloqueo | B0/B1/B2: serie, código/estado y timeout del ejecutor; reconstrucción limpia | #4 #8 #21 |
| R02 | Uso manual sin IA, procesos aislados y persistencia | S1: proceso A falla sin matar B/shell; guardar/reiniciar/leer archivo | #9 #10 #11 #12 #13 #14 #34 |
| R03 | Cobertura estructurada de capacidades propias incluidas | M1 y catálogo de acciones de producto con API/tool y comprobación; cobertura calculada sobre lista versionada | #6 #22 #40 #41 #29 |
| R04 | Autoridad del propietario y autonomía separadas | A1: permiso denegado, revocado y limitado a workspace; toma de control sin modelo | #5 #13 #24 #28 |
| R05 | Piloto real opcional operando sin visión | M2, resultado de servicio, identidad de modelo/configuración y apagado | #23 #29 |
| R06 | Cliente MCP independiente sobre servicios reales | C1: repetir M1 por adaptador; mismo efecto y mismas denegaciones | #39 |
| R07 | Red, DNS y HTTPS dentro del invitado | N1: resolver/consultar fixture, rechazo de certificado inválido y fallo de DNS explícito | #16 #17 #36 |
| R08 | Navegador con motor y renderizado local | W1–W5, capturas/estado/acciones y pruebas del servicio web | #7 #19 |
| R09 | Escritorio operable y visión opcional acotada | D1: abrir/activar/cerrar ventana por API; captura de selección autorizada; volver a consola ante fallo | #18 #37 #40 #41 |
| R10 | Adaptación determinista observable | P1: misma entrada/política produce mismo perfil; memoria limitada revela capacidades ausentes/degradadas | #20 #38 |
| R11 | Cambio, candidata y recuperación verificables | M3, hashes de origen/artefacto, pruebas y arranque de recuperación sin IA | #25 #26 #27 #42 |
| R12 | Publicación reproducible y procedencia | L1: inventario, licencia, release experimental, comandos y límites documentados | #30 #32 |

## Primeros escenarios B0–B2 y S1

B0: desde checkout limpio construir imagen y arrancar sin red/GUI/IA. Emitir identificador de build y marcador final de éxito; el ejecutor comprueba ambos y el estado de salida definido. Un mensaje temprano no basta.

B1: variante de prueba con panic deliberado. Debe conservar diagnóstico y terminar como fallo aunque haya emitido el mensaje inicial. B2: variante bloqueada; el ejecutor termina solo esa VM al agotar el plazo y devuelve timeout, no éxito. #8/#21 fijan los códigos y el timeout tras medir línea base. Repetir construcción y comparar hashes; no prometer identidad binaria antes de resolver diferencias.

S1, H1: arrancar shell nativa sin red/GUI/modelo; lanzar dos procesos, provocar acceso inválido en uno y mantener operativo el otro. Crear/leer archivo, reiniciar y verificar persistencia. Entrada manual por serie permitida.

## Tres misiones de producto

M1, H2: workspace y archivo precreados. Descubrir tools, leer contenido/versión, reemplazar con expected_version y clave de idempotencia, consultar operación y verificar contenido/hash. Repetir con permiso denegado, versión obsoleta y respuesta perdida; no alterar otro workspace ni duplicar efectos. Cliente determinista, capturas/OCR/clicks deshabilitados.

M2, H3: mismo objetivo con piloto ejecutándose dentro de RusticOS y modelo real sustituible, local o remoto. Registrar pasos, errores, presupuesto y verificación del servicio. Inyectar llamada inválida, caída de proveedor y cancelación; no ampliar permisos y conservar uso manual. MCP se valida por C1, sin obligar al piloto a utilizarlo.

M3, H5: modificar una utilidad en workspace, enviar construcción al ejecutor acotado, recibir artefacto identificado y resultados, activar bajo política del propietario. Inducir fallo y recuperar versión anterior sin IA ni proveedor disponible. No permitir que un resultado de texto del agente sustituya pruebas o hash.

## Contrato web mínimo

Fixtures locales versionados, servidos por un servidor de pruebas controlado; renderizado y JavaScript ocurren en RusticOS. #7 evalúa motor y registra brechas; no exige un sitio público cambiante como prueba.

| Caso | Fixture y aceptación |
| --- | --- |
| W1 HTML/CSS | Documento con títulos, párrafos, enlaces, lista, imagen local y cajas con tamaño/color/margen. DOM y geometría esperada; revisión de captura con tolerancias de fuente documentadas. |
| W2 Unicode | UTF-8: español con tildes y ñ, texto griego y CJK con fuentes de prueba licenciadas. Contenido extraído conserva codepoints; sin caracteres de sustitución inesperados. |
| W3 JavaScript | Botón que incrementa contador y actualiza DOM; ejecución por acción del usuario y API produce el mismo valor observable. |
| W4 Formularios | Campos etiquetados, foco por teclado, envío GET/POST al fixture; servidor recibe nombres/valores esperados y el navegador muestra la respuesta. |
| W5 HTTPS | CA de prueba instalada explícitamente, certificado válido aceptado; caducado, nombre incorrecto o CA no confiable rechazados. Sin opción silenciosa de ignorar validación. |

Accesibilidad inicial: navegación por teclado, foco visible, nombres/roles/estado consultables en controles propios y del fixture, salida textual de errores. No se promete conformidad integral de accesibilidad ni toda la plataforma web. Formularios nativos y ventanas tendrán equivalencia entre acción manual y semántica.

## Datos y autoridad

Modo manual sin modelo; modo híbrido y automático según #24, independientemente del perfil de permisos de #5. El propietario delega ámbitos y puede revocarlos. El descubrimiento no concede autoridad. Identidad vinculada a sesión/handles, nunca confiada a argumentos del modelo.

Cada conexión al proveedor declara destino y datos enviados. Fixtures sintéticos en pruebas; secretos fuera del prompt/log; seleccionar contenido antes de egreso y registrar metadatos sin almacenar credenciales. Contenido web/archivos no concede nuevas instrucciones privilegiadas. Capturas limitadas a la selección autorizada. Políticas exactas y almacenamiento de credenciales se implementan en #5/#13/#23.

## Decisiones abiertas y responsables por issue

#3 adopta arquitectura/arranque. #4 fija herramientas y máquina versionada. #5 concreta Low/Medium/Total y reglas de consentimiento. #6 fija contratos. #7 selecciona motor/fuentes y brechas web. #20 fija presupuestos medidos. #33 concreta temporización. #39 fija SDK/cliente/transporte MCP. #42 elige el canal al ejecutor. No bloquean la definición inicial con decisiones de componentes todavía no implementados.

Cambiar hardware objetivo, alcance obligatorio o ubicación de componentes requiere registrar motivo e impacto en #1 y actualizar este documento. Una adaptación de parámetros de prueba por evidencia no se presenta como capacidad universal nueva.
