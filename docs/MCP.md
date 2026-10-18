# RunPost MCP — contrato de tools

Servidor: `runpost_mcp.exe mcp` (stdio) o, con la app abierta, Streamable HTTP en
`http://127.0.0.1:7879/mcp` (mismo token local que la API, en `X-RunPost-Token` o
`Authorization: Bearer`). Las mismas tools por los dos transportes; por HTTP, cada llamada aparece en
el panel Agente de la app y `require_approval` pregunta al usuario. Si la app está abierta sobre el
mismo workspace, `runpost_mcp mcp` le pasa cada `tools/call` por HTTP (el puente de
`crates/runpost-mcp/src/bridge.rs`): los clientes por stdio, como el plugin de Claude Code, también
salen en el panel y pasan por las aprobaciones. Mantener sincronizado con `crates/runpost-mcp`.
Configuración de clientes en [mcp-client-configs/](mcp-client-configs/).

## Reglas comunes

- Toda salida es **JSON minificado** en un único bloque de texto. Nada de cabeceras
  salvo que se pidan, ni cuerpos sin truncar.
- `request` acepta un id (`req_…`) o una ruta. Orden de búsqueda: ruta exacta
  `"Colección/Carpeta/Nombre"` → igual sin mayúsculas → prefijo único de la ruta →
  sufijo único (`"Carpeta/Nombre"` o solo `"Nombre"`) → prefijo único de cualquier
  cola de la ruta (`"00.1 API"`).
- `env` acepta id o nombre; si se omite, se usa `settings.active_environment`.
- Variables y secretos se resuelven en el servidor. Cualquier valor de una variable
  `secret` (y sus formas codificadas) sale como `***` en URL, cabeceras, cuerpo y errores.
- Errores: `isError: true` y `{"error":"código","msg":"…","hint":"…"}`. Códigos:
  `request_not_found`, `request_ambiguous`, `env_not_found`, `collection_not_found`,
  `no_active_env`, `invalid_request`, `invalid_auth`, `response_not_found`, `select`,
  `ref_not_found`, `no_saved_request`, `keyring`, `policy`, y los de red `connect`, `timeout`,
  `redirect`, `body`, `decode`, `request`
  (estos con `url` enmascarada, `msg` ≤ 300 caracteres y `response_id` si se grabó).
  Por stdio con el puente a la app, `app_unavailable` si la app deja de responder con la
  llamada en curso: pudo ejecutarse, así que no se repite; el `hint` pide mirar `history` antes de
  reintentar, y las siguientes llamadas van sin la app.

## Forma de `run_request` / `send` (TokenBudget, 8.3)

```json
{"status":200,"ms":143,"bytes":5120,"ct":"application/json","response_id":"h_…",
 "body":{…},"t":{"raw":5300,"sent":180}}
```

| Campo | Cuándo |
|---|---|
| `status`, `ms`, `bytes` | siempre |
| `ct` | si hay `Content-Type` (sin `; charset=…`) |
| `response_id` | siempre (`h_…` si se grabó en historial, `c_…` si `save_history:false`) |
| `body` | JSON minificado, o el resultado de `select`; HTML → `título \| texto visible`; binario → `"<binary N bytes>"` |
| `truncated`, `total_chars` | solo si el cuerpo superó `body_max` (por defecto `settings.mcp.body_max_chars` = 2000); el JSON truncado va como texto |
| `select_error` | si `select` no es JSONPath válido o el cuerpo no es JSON (se devuelve el cuerpo sin filtrar) |
| `headers` | solo con `headers:true`; `authorization`, `cookie`, `set-cookie`, `proxy-authorization` y `x-*-key`/`x-*-token` siempre `***` |
| `tests` | solo si hubo tests: `{"passed","failed","failures":[{"name","msg"}] (máx. 10)}` |
| `vars_changed`, `console` | solo si los hay; consola máx. 5 líneas de 200 caracteres |
| `skipped` | `true` si un script `pre` llamó a `pm.execution.skipRequest()`: no se envió nada y no hay `status` ni `body` |
| `t` | `{"raw": bytes HTTP recibidos (cuerpo + cabeceras), "sent": bytes de esta respuesta}` |

## Tools

| Tool | Entrada (negrita = obligatoria) | Salida |
|---|---|---|
| `ws_overview` | — | `{"collections":[{"id","name","requests"}],"environments":[{"id","name","active"}],"active_env"}` |
| `list_requests` | `collection`, `folder`, `q` | `[{"id","path","m","url"}]` (url sin resolver) |
| `get_request` | **`request`**, `fields` (`method,url,params,headers,body` por defecto; también `auth,scripts,settings,description,name`) | petición nativa (5.1) con `id` + los campos pedidos |
| `run_request` | **`request`**, `env`, `vars` `{k:v}`, `select`, `body_max`, `headers`, `save_history` (true) | forma de arriba |
| `send` | **`method`**, **`url`**, `headers` `{k:v}`, `body` (texto o JSON), `auth` (nativa, 5.4), `env`, `select`, `body_max`, `save_as` `{collection, folder?, name}` | forma de arriba (+ `saved_id`). Con `save_as`, la petición se guarda antes de enviarla y hereda variables y auth de esa colección |
| `get_response` | **`response_id`**, `select`, `range` `[desde, hasta)` en caracteres, `headers`, `diff_against` (`"ref"` o otro `response_id`), `ignore` (campos volátiles extra), `save_ref` | `{"body"}` (+ `truncated`/`total_chars`), `{"body","range","total_chars"}` o `{"headers":{…}}`; con `diff_against`, `{"diff":[{"path","from"?,"to"?}],"more"?}` (ver abajo); con `save_ref`, `{"ref_saved":true}` |
| `env_get` | `env`, `keys` | `{"name","vars":{k:v}}`, secretos `***` |
| `env_set` | **`env`** (se crea si no existe), **`vars`** `{k:v}`, `secret` (claves a marcar; van al llavero del sistema) | `{"ok":true,"changed":[k]}`; sin llavero en el sistema, además `"store":"file","hint":"OS keyring unavailable…"` |
| `upsert_request` | **`collection`** (se crea si no existe), `folder` (`"A/B"` anida), `name`, `method`, `url`, `headers`, `body`, `auth`, `scripts` `{pre,test}`, o `requests:[…]` con esos campos por elemento | `{"id"}` o, en lote, `{"ids":[…]}`. Actualiza por carpeta + nombre; crear exige `url` |
| `import_postman` | `path` o `text`, `kind` (`collection`/`environment`, se detecta) | `{"id","name","requests"}` o `{"id","name","vars"}` (+ `warnings`, máx. 5) |
| `history` | `limit` (10, máx. 100), `q`, `status` | `[{"id","ts","m","url","status","ms","err"?}]`, `ts` en ms desde epoch |
| `search` | **`q`** | `[{"id","path","where"}]` con `where` = `name`, `url`, `headers` o `body` (máx. 50) |
| `run_collection` | **`collection`** (id, nombre o slug), `folder` (id, ruta o nombre), `env`, `iterations` (por defecto una por fila de datos, o 1), `data` (ruta de un CSV o JSON), `output` | `summary` (por defecto): `{"total","passed","failed","ms","requests","skipped"?,"errors"?,"failures":[{"path","it"?,"test","msg"}]}` (máx. 20 fallos + `more_failures`); `failures`: `{"failed","failures":[…]}`; `full`: el resumen + `items:[{"path","it"?,"status","ms","tests":[{"name","ok","msg"?}]}]`. Una petición sin respuesta cuenta como `"test":"<request error>"` |

`run_collection` aplica la política del agente a cada petición: la que no pasa queda como
`<request error>` con `policy: …` y no se envía. No guarda las peticiones en el historial (como el
Collection Runner de Postman) ni devuelve la consola de los scripts.

## Referencias y diff (`get_response`)

- `save_ref: true` guarda esa respuesta como referencia de su petición guardada (una por petición,
  en `history.db`); `diff_against: "ref"` compara con ella y `diff_against: "<response_id>"`, con
  otra respuesta. Se pueden pedir juntos: compara con la referencia anterior y luego la sustituye.
- El diff es estructural sobre JSON: `{"path":"$.a.b","from":1,"to":2}`; sin `from` = campo nuevo,
  sin `to` = desaparecido; `path: "status"` si cambió el código. Un cuerpo que no es JSON da
  `{"path":"body:<línea>","from","to"}` con la primera línea distinta. Máximo 50 cambios (`more`
  cuenta el resto) y valores de más de 200 caracteres resumidos.
- Campos volátiles: `settings.json` → `diff.ignore` (por defecto `timestamp`, `requestId`,
  `traceId`) más el `ignore` de la llamada. Un nombre vale a cualquier profundidad; una ruta
  (`$.items[*].updatedAt`), solo ahí. `{"diff":[]}` = nada relevante cambió.
- Solo tienen referencia las respuestas de peticiones guardadas (`run_request`):
  `no_saved_request` si no; `ref_not_found` si aún no se guardó ninguna.

## Política (`agent_policy` de la colección)

Se evalúa en `run_request` (y en cada petición de `run_collection`) antes de enviar:

- `allow_methods`: método fuera de la lista → `{"error":"policy","rule":"allow_methods","hint":"ask the user to approve in the app"}`.
- `require_approval` (métodos o ids `req_…`): por HTTP con la app abierta, la tool **espera hasta
  60 s** a que el usuario pulse Aprobar o Denegar en el aviso de la app. Aprobar → se ejecuta
  normalmente; denegar → `{"error":"policy","rule":"require_approval","hint":"the user denied it in
  the app"}`; sin respuesta → `hint: "no answer in 60 s; ask the user before retrying"`. Por stdio
  sin la app abierta sobre el mismo workspace, o sin ninguna ventana de la app, responde al momento
  con `hint: "ask the user to approve in the app"`. `run_collection` por HTTP pregunta una sola vez por todas sus peticiones protegidas.
- `max_calls_per_minute`: ventana deslizante de 60 s por colección →
  `rule: "max_calls_per_minute"`, `hint: "limit N/min; retry in Ss"`.

Las peticiones ad hoc de `send` no pertenecen a una colección y no tienen política.

## Logs y panel Agente

Por stdio, una línea por llamada en stderr y en `workspace/logs/mcp.log`: nombre de la tool,
`ok`, bytes enviados y ms. Nunca argumentos ni respuestas. Por HTTP (y por stdio cuando el puente
pasa la llamada a la app), cada llamada es una fila del panel Agente de la app (hora, cliente,
sesión, tool, petición, estado, ms y bytes crudos → enviados, con los secretos enmascarados), que no
se guarda en disco.
