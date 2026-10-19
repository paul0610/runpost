# Importar en RunPost

RunPost importa colecciones y entornos de **Postman v2.1**, **OpenAPI 3**, **HAR**, **Insomnia**
(v4 y v5), **Bruno** (`.bru`, YAML y JSON) y comandos **cURL**. El formato se detecta por el
contenido; no hace falta elegirlo.

## Cómo

- **App**: botón **Importar** (barra lateral o pantalla vacía). Pega el texto (un comando cURL o el
  contenido de una exportación) y pulsa **Importar** (`Ctrl+Enter`), o **Elegir archivos…**. Un
  cURL se abre en una pestaña nueva sin guardar; lo demás se guarda en el workspace.
- **CLI**: `runpost import <archivo o carpeta>` (`--kind collection|environment` solo para Postman).
- **MCP**: herramienta `import_postman` con `path` (archivo o carpeta de Bruno) o `text`.
- **API local**: `POST /api/import` con `{"text": …}`, `{"path": …}`, `{"paths": […]}` o archivos
  `multipart/form-data`.

Una importación puede crear varias cosas a la vez (una colección y sus entornos, o varias
colecciones de una exportación de Insomnia). Lo que no se pudo trasladar sale como **aviso** en el
informe: nunca se descarta en silencio.

**Reimportar sustituye** en vez de duplicar cuando el origen tiene identidad: el `_postman_id` de
Postman, el id del espacio de trabajo de Insomnia o el nombre de la colección de Bruno. Las
carpetas y peticiones que siguen en la misma ruta conservan su id (y su historial). OpenAPI, HAR y
cURL crean siempre una colección nueva.

## Formatos

### Postman v2.1

Colecciones y entornos, con carpetas, auth (`noauth`, `inherit`, `bearer`, `basic`, `apikey`,
`digest` y `oauth2` con client credentials o authorization code), cuerpos, scripts `pm.*`,
variables y ejemplos guardados (`response`, que pasan a los [ejemplos](COLLECTIONS.md) de la
petición). Las variables `secret` de un entorno siguen siendo secretas. Las variables de ruta (`:id`)
se quedan tal cual en la URL.

### OpenAPI 3 (JSON o YAML)

Una petición por operación, en carpetas por etiqueta. `{{baseUrl}}` sale del primer servidor. Los
parámetros de ruta pasan a `{{nombre}}` (variables de la colección); los de query y cabecera llevan
su ejemplo, y el cuerpo es el ejemplo del documento o uno generado desde el esquema. La seguridad
global es la auth de la colección y sus credenciales quedan como variables secretas vacías. Swagger
2 no se importa.

### HAR

Las capturas de las herramientas de desarrollo del navegador. Una petición por entrada, en carpetas
por host si hay más de uno; si la captura indica el tipo de recurso, solo entran las llamadas
XHR/fetch. No se copian las cookies ni las cabeceras que pone el cliente (`Host`,
`Content-Length`, pseudo-cabeceras de HTTP/2). Si alguna petición lleva `Authorization`, el aviso
pide pasarla a una variable secreta antes de compartir la colección.

### cURL

Comillas y continuaciones de bash, cmd (`^`) y PowerShell (`` ` ``), incluido "Copy as cURL" de los
navegadores. Entiende `-X`, `-H`, `-d`/`--data-*`, `--json`, `-F`, `-u`, `-G`, `-I`, `-L`, `-k`,
`-m`/`--max-time`, `-A`, `-e`, `-b` y `--url`; lo que no cambia la petición (`-s`, `-v`, `-o`,
`--compressed`…) se ignora.

### Insomnia

- **v4** (`_type: export`, JSON o YAML) y **v5** (YAML `collection.insomnia.rest/5.0`;
  `spec.insomnia.rest/5.0` cuando el documento tiene una especificación o tests; y el entorno global
  `environment.insomnia.rest/5.0`).
- Cada espacio de trabajo es una colección. El **entorno base** pasa a variables de la colección y
  cada **subentorno** a un entorno de RunPost (`<colección> - <subentorno>`). Los datos anidados se
  aplanan: `{"auth": {"token": …}}` → `auth.token`.
- Plantillas: `{{ _.x }}`, `{{ x }}` y `{{ _['mi-clave'] }}` pasan a `{{x}}`. `{% uuid %}` →
  `{{$guid}}`, `{% now 'iso-8601' %}` → `{{$isoTimestamp}}`, `{% now 'unix' %}` → `{{$timestamp}}`.
  Las demás etiquetas (`{% response %}`, `{% base64 %}`, `{% faker %}`…) y los filtros quedan tal
  cual, con un aviso.
- Carpetas: su auth es la de la carpeta; sus cabeceras se copian a cada petición (Insomnia las envía
  todas) y sus variables van al script previo de la carpeta (`pm.variables.set`).
- Auth: `{}` hereda; `none`, `basic`, `bearer` y `apikey` se traducen. Un bearer con prefijo propio y
  una API key en cookie pasan a su cabecera. Las demás (OAuth, digest, AWS, Hawk…) entran como
  "sin auth" con un aviso.
- Scripts: la API `insomnia.*` copia la de Postman, así que se traducen cambiando el prefijo a `pm.*`
  (lo mismo que hace Insomnia al importar de Postman). El aviso pide revisarlos.
- Secretos: Insomnia no exporta los valores del **vault** (`{{ vault.x }}`); pasan a variables
  secretas vacías. Las variables de una tabla `secret` son secretas. Si una variable normal se llama
  como una credencial (`token`, `password`, `key`…), el aviso propone marcarla como secreta.
- No se importan las peticiones gRPC, WebSocket ni Socket.IO, los mocks, la especificación de un
  documento de diseño (impórtala como OpenAPI) ni las cookies.

### Bruno

- **Carpeta de la colección**: en la app elige su `bruno.json` (formato `.bru`) o su
  `opencollection.yml` (YAML, el formato por defecto desde Bruno 3.1); en el CLI basta con la
  carpeta. También se importa la **exportación en un archivo**: JSON ("Bruno Collection") u
  OpenCollection YAML empaquetado.
- Los entornos de `environments/` pasan a entornos de RunPost (`<colección> - <entorno>`). Bruno no
  guarda el valor de los secretos en los archivos: llegan como variables secretas vacías.
- **El `.env` de la colección no se lee nunca.** Las referencias `{{process.env.X}}` pasan a `{{X}}`
  con una variable secreta vacía en la colección, y el aviso dice cuáles hay que rellenar.
- Las cabeceras de la colección y de las carpetas se copian a cada petición (la de la petición
  manda). Las variables de la colección son variables de la colección; las de carpeta y de petición
  van a su script previo (`pm.variables.set`).
- Parámetros: `params:query` manda sobre la query de la URL. Los parámetros de ruta (`:id`) se
  escriben en la URL; si no tienen valor, pasan a `{{id}}` con una variable de la colección.
- Cuerpos: JSON, texto, XML, SPARQL, formulario, multipart (con archivos), GraphQL y archivo binario.
  Auth: `inherit`, `none`, `basic`, `bearer` y `apikey`; las demás entran como "sin auth" con aviso.
- **Aserciones** (`assert`) y **variables posteriores a la respuesta** se traducen a `pm.*`:
  `res.status: eq 200` → `pm.test("res.status: eq 200", () =>
  pm.expect(pm.response.code).to.eql(200))`, `token: res.body.token` →
  `pm.variables.set("token", pm.response.json().token)`. Solo se traducen las lecturas de
  propiedades (`res.status`, `res.body.a[0].b`, `res.headers['x']`, `res.responseTime`); lo demás
  queda comentado con un aviso.
- Los **scripts** y **tests** usan la API de Bruno (`bru`, `req`, `res`): entran **comentados**,
  para no fallar al ejecutar, con un aviso que pide reescribirlos con `pm.*`.
- No se importan las peticiones gRPC ni WebSocket ni los archivos de script sueltos. Al leer la
  carpeta no se siguen enlaces simbólicos ni se entra en lo oculto (`.git`), `node_modules` ni lo que
  liste `ignore`. Límites: 10 000 archivos, 8 MB por archivo y 16 niveles de carpetas.
