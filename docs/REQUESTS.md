# Peticiones: snippets de código y cuerpo GraphQL

## Snippets de código

El botón **`</>`** de la barra de la petición abre, a la derecha como en Postman, el panel
**Fragmento de código**: la petición en **cURL**, **JavaScript (fetch)**, **PowerShell**
(`Invoke-RestMethod`) o **Python** (`requests`), con un botón **Copiar**. El panel recuerda si estaba
abierto y el lenguaje. **Copiar como curl**, en el menú `⋯` de la petición y en el del árbol, usa el
mismo generador.

El snippet sale de la petición tal como está en la pestaña, también sin guardar:

- Las **variables normales** se sustituyen por su valor del entorno activo, la colección o las
  globales.
- Las **variables secretas**, las dinámicas (`{{$guid}}`…) y las que no existen quedan como
  `{{nombre}}`. Un snippet puede acabar en un chat o en un ticket, así que nunca lleva un secreto:
  pon el valor al ejecutarlo.
- Lleva la **auth heredada** de la carpeta o la colección. Basic se codifica al ejecutar el snippet,
  no dentro de él. Digest usa lo que trae cada herramienta (`curl --digest`, `-Credential` en
  PowerShell, `HTTPDigestAuth` en Python; fetch no tiene digest y lo dice en un comentario). OAuth 2.0
  deja `Authorization: Bearer {{access_token}}`.
- Los archivos de form-data y binary se citan por su ruta: no se leen ni se copian.

También desde fuera de la app:

- **MCP**: `get_request` con `snippet` (`curl`, `fetch`, `powershell` o `python`) devuelve
  `{"id", "snippet"}`. Ver [MCP.md](MCP.md).
- **API local**: `POST /api/codegen/{lenguaje}` con una petición guardada o un borrador.

## Cuerpo GraphQL

En la pestaña **Body**, el modo **GraphQL** muestra lado a lado la **consulta** y las **variables de
GraphQL** (JSON). Las dos admiten `{{variables}}`.

Al enviar, el cuerpo es el JSON estándar de GraphQL sobre HTTP: `{"query": …, "variables": …}`, con
`Content-Type: application/json` si la petición no fija otro. Sin variables, va `"variables": null`.
Si las variables no son JSON válido, la petición no sale y el error lo dice
(`graphql variables are not JSON`). El método es el de la petición; lo normal es `POST`.

Importar trae los cuerpos GraphQL de Postman v2.1, Insomnia y Bruno, y exportar a Postman los deja
como `mode: graphql`. Los snippets envían el mismo JSON.
