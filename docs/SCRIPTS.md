# Scripts de RunPost (`pm.*`)

Los scripts **Pre-request** y **Tests** de colecciones, carpetas y peticiones se ejecutan como en
Postman: al enviar una petición desde la app, con `run_request` por MCP, en el Runner y con
`runpost run`. Están implementados con QuickJS.

## Cómo se ejecutan

- Orden: Pre-request de la **colección → carpetas → petición**; después la petición; después Tests
  en el mismo orden.
- Los Pre-request corren **antes** de resolver `{{variables}}`: lo que cambien se usa en la petición.
- Cada script corre en un contexto nuevo y limpio, con **5 s** y **32 MB** como máximo (incluidos sus
  `setTimeout` y `pm.sendRequest`). No hay `require`, ni sistema de archivos, ni `process`.
- Sin modo estricto (como el sandbox de Postman) y con `await` en el nivel superior.
- Un error del script (sintaxis, excepción, tiempo, memoria) **nunca** aborta la ejecución: aparece
  como un test fallido llamado `<script error>` con el mensaje, p. ej.
  `pre-request script of request "ping": ReferenceError: foo is not defined`.
- La consola (`console.log/info/warn/error`) va a la **Consola** de la app, al informe del Runner y,
  resumida, a la salida MCP. Los valores secretos salen siempre como `***`.

## API soportada

| Objeto | Qué hay |
|---|---|
| `pm.environment`, `pm.globals`, `pm.collectionVariables` | `get`, `set`, `unset`, `has`, `clear`, `toObject`, `replaceIn`; `pm.environment.name`. Los cambios se guardan en disco al terminar la ejecución (el flag `secret` de una variable existente se conserva). |
| `pm.variables` | `get` / `has` con el orden de Postman (local → datos → entorno → colección → globales), `set` / `unset` (locales: duran la ejecución; en el Runner, toda la ejecución), `toObject`, `replaceIn` (también `{{$guid}}`, `{{$timestamp}}`, `{{$isoTimestamp}}`, `{{$randomInt}}`). |
| `pm.iterationData` | `get`, `has`, `toObject` (fila del archivo de datos del Runner). |
| `pm.info` | `requestName`, `requestId`, `iteration` (desde 0), `iterationCount`, `eventName`. |
| `pm.execution.skipRequest()` | En Pre-request: no se envía la petición ni corren sus Tests (el Runner la marca "omitida"). |
| `pm.request` | `url` (`toString`, `getHost`, `getPath`, `getQueryString`, `query.get/has/all/toObject`), `method`, `headers` (`get`, `has`, `all`, `toObject`, `each`…; en Pre-request también `add`, `upsert`, `remove`, `clear`), `body` (`mode`, `raw`, `urlencoded`, `formdata`…). En Pre-request es la petición sin resolver; en Tests, la enviada. |
| `pm.response` | `code`, `status`, `responseTime`, `responseSize`, `headers.get(…)`, `text()`, `json()`, `reason()`, `size()` y las aserciones `to.have.status(n \| 'OK')`, `to.have.header(k[, v])`, `to.have.body(…)`, `to.have.jsonBody([ruta[, valor]])`, `to.be.ok / success / error / clientError / serverError / json / notFound / unauthorized / forbidden / badRequest…`, `to.not…`. |
| `pm.test(nombre, fn)` | Síncrona o `async`. `pm.test.skip(nombre)` la marca como omitida. |
| `pm.expect(valor[, mensaje])` | Subconjunto de chai: `to/be/been/is/that/which/and/has/have/with/at/of/same/but/does/still/also`; `not`, `deep`, `own`, `nested`, `any`, `all`; `equal/eql`, `above/below/least/most/within/closeTo`, `true/false/null/undefined/NaN/ok/exist/empty/finite`, `a/an(tipo)`, `include/contain` (texto, arrays, objetos), `match`, `string`, `property(nombre[, valor])`, `length/lengthOf` (también `.lengthOf.above(n)`), `keys`, `oneOf`, `members`, `instanceOf`, `satisfy`, `throw`; `pm.expect.fail`. Los mensajes siguen el formato de chai 4. |
| `pm.sendRequest(url \| {url, method, header, body}, cb?)` | Hasta 10 por script. Con callback `(err, res)` o como promesa (`await`). Cuerpo `raw` o `urlencoded`. Como en Postman, no resuelve `{{variables}}`: usa `pm.variables.replaceIn(…)`. Respeta TLS, proxy y redirecciones de Ajustes. |
| Globales | `console.*`, `setTimeout/clearTimeout/setInterval/clearInterval`, `JSON`, `btoa/atob` (Latin-1, como el navegador). |
| `runpost.crypto` | `sha256(texto[, 'hex'\|'base64'])`, `sha1`, `sha512`, `hmac(clave, texto[, algoritmo = 'sha256'[, 'hex'\|'base64']])`, `base64(texto)` (UTF-8) y `base64.decode(b64)`. |

No disponible (dan un error claro): `require`, `CryptoJS`, `postman.*` (API antigua),
`pm.cookies`, `pm.visualizer`.

## Migrar desde CryptoJS

| Postman (CryptoJS) | RunPost |
|---|---|
| `CryptoJS.SHA256(x).toString()` | `runpost.crypto.sha256(x)` |
| `CryptoJS.SHA256(x).toString(CryptoJS.enc.Base64)` | `runpost.crypto.sha256(x, 'base64')` |
| `CryptoJS.HmacSHA256(mensaje, clave).toString()` | `runpost.crypto.hmac(clave, mensaje)` (ojo: la clave va primero) |
| `CryptoJS.HmacSHA512(mensaje, clave).toString(CryptoJS.enc.Base64)` | `runpost.crypto.hmac(clave, mensaje, 'sha512', 'base64')` |
| `CryptoJS.enc.Base64.stringify(CryptoJS.enc.Utf8.parse(x))` | `runpost.crypto.base64(x)` |
| `CryptoJS.enc.Base64.parse(b).toString(CryptoJS.enc.Utf8)` | `runpost.crypto.base64.decode(b)` |
| `postman.setEnvironmentVariable(k, v)` | `pm.environment.set(k, v)` |
| `postman.getEnvironmentVariable(k)` | `pm.environment.get(k)` |
| `tests["nombre"] = condición;` | `pm.test('nombre', () => pm.expect(condición).to.be.true);` |

## Diferencias con Postman a tener en cuenta

- Un valor no textual guardado con `pm.environment.set` (número, objeto) se guarda como texto (JSON
  en los objetos); dentro del mismo script, `get` devuelve el valor original.
- Los valores del archivo de datos llegan como texto (`"1"`, no `1`).
- Si el archivo de datos trae una variable que es secreta en el entorno (p. ej. `accessToken`), su
  valor también se trata como secreto (sale `***` en historial, consola y MCP).
- `pm.setNextRequest` / `pm.execution.setNextRequest` no están: el Runner sigue el orden del árbol.
