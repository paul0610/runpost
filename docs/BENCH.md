# RunPost — medidas

Tokens ≈ caracteres / 4. Medido el 2026-10-18 en Windows 11 con el binario real
(`runpost_mcp.exe mcp`, stdio) y la colección de ejemplo importada (Acme, una API ficticia), contra
`wiremock` con respuestas simuladas. Ninguna llamada sale a la red.

Reproducir:

```powershell
cargo test -p runpost-cli --test mcp_bench -- --ignored --nocapture
```

## Objetivos

| Métrica | Objetivo | Medido | Dónde se comprueba |
|---|---|---|---|
| `tools/list` completo | < 2 500 tokens | **≈ 1 608** (6 432 caracteres, 13 tools) | `mcp_tools.rs`, test unitario de `runpost-mcp` |
| `run_request` con `select` a un campo | < 60 tokens | **< 60** para la respuesta (`$.session.scope`, 4 valores); los `tests` y la `console` de los scripts de la petición van aparte | `mcp_tools.rs`, `budget.rs` |
| `run_request` típico (JSON de 5 KB, sin `select`) | < 700 tokens | **< 700** (cuerpo truncado a 2 000 caracteres; el test falla si se supera) | `budget.rs` `typical_5kb_json_stays_under_700_tokens` |
| "00.2 Gateway - ping" | < 300 bytes | **201 caracteres**, con el resultado de sus tests | `mcp_tools.rs` |
| Secreto en la salida MCP | nunca | 0 apariciones en la transcripción completa | `mcp_tools.rs` |
| Arranque de la app hasta la ventana usable | < 1,5 s | **1,13 s** de mediana (1,19 s contando desde que se lanza el ejecutable) | `scripts/dev/startup_time.py` |
| `runpost_mcp mcp` hasta responder `initialize` | < 100 ms | **53 ms** | `startup_time.py` |
| Memoria en reposo de la app (proceso Rust, sin el webview) | < 60 MB | **37 MB** | `startup_time.py` |
| Ejecutable de la app | < 25 MB | **19,1 MB** | `startup_time.py` |
| Binario `runpost_mcp` | < 15 MB | **16,4 MB**: todavía no | `startup_time.py` |

## Arranque, memoria y tamaño

Medido el 2026-10-18 en Windows 11 (AMD Ryzen 9 6900HX, 32 GB, SSD NVMe) con los binarios release,
un workspace temporal vacío y el llavero en memoria:

```powershell
python scripts/dev/startup_time.py --runs 5
```

| Medida (mín / mediana / máx) | Objetivo | Antes: `lto = "thin"` | Ahora: LTO completo y `opt-level = "s"` |
|---|---|---|---|
| App: del arranque del proceso a la ventana usable | < 1 500 ms | ≈ 1 400 ms de mediana | 1 111 / 1 127 / 1 279 ms |
| App: lo mismo desde que se lanza el ejecutable | < 1 500 ms | ≈ 2 000 ms de mediana | 1 168 / 1 191 / 1 870 ms |
| `runpost_mcp mcp` hasta responder `initialize` | < 100 ms | — | 53 / 53 / 78 ms |
| App en reposo, memoria del proceso (sin el webview) | < 60 MB | — | 37 MB |
| `runpost_app.exe` | < 25 MB | 33,3 MB | 19,1 MB (19,9 MB compilado junto al CLI) |
| `runpost_mcp.exe` | < 15 MB | 28,4 MB | 16,4 MB (15,3 MB con `opt-level = "z"`) |

- "Ventana usable" es cuando la SPA ya cargada pide `backend_info`; la app lo anota si
  `RUNPOST_STARTUP_LOG` está definida. El máximo desde que se lanza el ejecutable es el primer
  arranque de la serie, con WebView2 en frío.
- La memoria es el *working set* del proceso `runpost_app.exe` 3 s después de la ventana usable; los
  procesos de WebView2 van aparte.
- Tamaños compilando cada binario por separado (`cargo build --release -p …`), como hace el release.
  Compilados juntos con `cargo build --release`, la app también lleva la SPA embebida del modo
  navegador (unos 0,8 MB más), porque cargo unifica las características.
- La columna "antes" del arranque es de una medida anterior del mismo día con el perfil `thin` (solo
  la mediana); los tamaños, de `cargo build --release` con ese perfil.

## Sesión guiada de 15 llamadas (colección de ejemplo: ¿qué scope necesita cada endpoint?)

| # | tool | argumentos | respuesta (chars) | ≈ tokens | crudo HTTP (bytes) |
|---|------|------------|------------------:|---------:|-------------------:|
| 1 | `ws_overview` | `{}` | 278 | 69 | — |
| 2 | `list_requests` | `{"collection":"acme-gateway-scopes"}` | 2206 | 551 | — |
| 3 | `env_get` | `{"keys":["gatewayUrl","accessToken","currency","accountId"]}` | 145 | 36 | — |
| 4 | `env_set` | `{"env":"Acme DESA (fake)","vars":{"gatewayUrl":"<mock>","api…` | 45 | 11 | — |
| 5 | `run_request` | `{"request":"00.2 Gateway - ping"}` | 201 | 50 | 116 |
| 6 | `run_request` | `{"request":"00.1 API","select":"$.session.scope"}` | 396 | 99 | 601 |
| 7 | `run_request` | `{"request":"#5 POST keys/local"}` | 377 | 94 | 292 |
| 8 | `run_request` | `{"request":"#6 POST encrypt"}` | 377 | 94 | 292 |
| 9 | `run_request` | `{"request":"#7 POST sign (sonda"}` | 576 | 144 | 292 |
| 10 | `run_request` | `{"request":"#5 keys/local SIN token"}` | 409 | 102 | 168 |
| 11 | `run_request` | `{"request":"#7 sign token INVÁLIDO"}` | 253 | 63 | 168 |
| 12 | `get_response` | `{"response_id":"h_01a150d151e772b7b5d2e2414adf3cb7","range":…` | 274 | 68 | — |
| 13 | `history` | `{"limit":5}` | 765 | 191 | — |
| 14 | `get_request` | `{"request":"#7b POST sign","fields":["url","headers"]}` | 231 | 57 | — |
| 15 | `search` | `{"q":"sign"}` | 740 | 185 | — |

Totales: `tools/list` ≈ 1 608 tokens (una vez por sesión) + 15 respuestas ≈ 1 818 tokens (las
respuestas de esta colección llevan el resultado de sus tests y su consola). Lo que el modelo escribe
en cada llamada son 30–90 caracteres de argumentos: referencia la petición por nombre y no reescribe
URL, cabeceras, token ni JSON.

## Cómo leer estas cifras frente a una sesión con curl

La referencia es una sesión real de Claude Code haciendo una tarea equivalente con curl y Postman
contra una API real: ≈ 98k tokens para probar los 3 endpoints, ≈ 158k para construir la colección y
≈ 45k para validarla, y el 95 % fue releer el contexto en cada paso. Estas tablas miden otra cosa:
cuánto **añade al contexto** cada llamada. No son comparables uno a uno y todavía no se afirma un %
de ahorro; la comparación de sesión completa (curl frente a RunPost, con el medidor de tokens) está
pendiente, con el arnés en `scripts/bench/`.

Lo que sí muestran:

- Con las respuestas pequeñas del mock, lo enviado es **mayor** que el crudo HTTP (por ejemplo 377
  caracteres frente a 292 bytes en `#5 POST keys/local`, con sus tests): el sobre
  `status/ms/bytes/ct/response_id/t` cuesta ≈ 40 tokens. RunPost ahorra cuando la respuesta es grande
  (truncado a 2 000 caracteres, `select`, cabeceras omitidas) y, sobre todo, en lo que el modelo
  **no tiene que escribir**: con curl cada prueba repite URL, `Authorization: Bearer …` (el token
  entra en el contexto) y el JSON escapado para PowerShell/cmd.
- `list_requests` es la llamada más cara (551 tokens): repite el nombre de la colección en cada
  ruta. Se puede filtrar por `folder` o `q`.
- `select` a un campo de una respuesta de sesión (601 bytes crudos) deja la respuesta en 396
  caracteres, contando los tests y la consola de la petición.

## Medidor del panel Agente

Con la app abierta, cada llamada MCP suma al medidor del panel Agente: bytes de la respuesta completa
frente a bytes entregados al modelo, y tokens estimados ahorrados. El botón **Exportar** del panel
copia esta tabla; la de abajo es la de la demo del GIF del README (`scripts/dev/ui-screens.mjs` en
modo `gif`, un cliente MCP que se presenta como Claude Code contra el mock de Acme, así que las
respuestas son diminutas).

#### Medidor de tokens del panel Agente (2026-10-18 21:05 UTC)

| Hora (UTC) | Cliente | Tool | Petición | Estado | ms | Crudo (B) | Enviado (B) | Tokens ahorrados (≈) |
|---|---|---|---|---|---:|---:|---:|---:|
| 21:05:47 | Claude Code | run_request | 00.2 Gateway - ping | 200 | 32 | 168 | 202 | 0 |
| 21:05:48 | Claude Code | run_request | #7 POST sign (sonda de autorización) · select $.error | 500 | 23 | 191 | 274 | 0 |
| 21:05:49 | Claude Code | run_collection | 01 - Matriz de scopes (token bajo prueba) · summary | 3/3 | 69 | 2154 | 93 | 515 |
| 21:05:53 | Claude Code | run_request | #8 DELETE account (sonda) | 200 | 3216 | 206 | 204 | 1 |
| **Total** | | **4 llamadas** | | | | **2719** | **773** | **487** |

Tokens estimados como bytes / 4. "Crudo" es la respuesta completa; "enviado", lo que recibió el modelo.
