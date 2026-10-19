# Colecciones: ejemplos, buscar y reemplazar y papelera

## Ejemplos guardados

Como en Postman, una petición puede guardar respuestas reales como **ejemplos**, para documentarla
o tener a mano cómo responde en cada caso (un 200, un 401 sin token, un 500…).

- **Guardar como ejemplo**: después de enviar una petición guardada, el botón **Guardar como
  ejemplo** del visor de respuesta pide un nombre (por defecto, el estado: `200 OK`) y la guarda.
  Una pestaña sin guardar no puede tener ejemplos: guarda antes la petición.
- **Pestaña Ejemplos** de la petición: la lista con el estado de cada uno y, del elegido, la
  petición que se envió (método y URL), el cuerpo formateado y las cabeceras. Desde ahí se
  **renombra** o se **elimina**.

El ejemplo sale del **historial**, que ya está enmascarado: los secretos que se conocen (variables
secretas, tokens) aparecen como `***`, en la petición y en la respuesta. Se guarda dentro del archivo
de la colección, así que viaja con ella y entra en Git si la colección está versionada. El cuerpo
tiene un límite de 1 MB.

Importar y exportar a **Postman v2.1** conserva los ejemplos: son los `response` de cada petición.

API local:

- `POST /api/collections/{id}/items/{petición}/examples` con `{"history_id", "name"?}`.
- `PATCH …/examples/{ejemplo}` con `{"name"}` para renombrar, y `DELETE …/examples/{ejemplo}`.

## Buscar y reemplazar

Menú contextual de una colección → **Buscar y reemplazar…**. Busca mientras escribes en toda la
colección: nombres, URL, params, headers, cuerpo, auth, scripts (Pre-request y Tests),
documentación y variables de la colección, de las carpetas y de las peticiones.

- **Opciones**: mayúsculas y minúsculas, palabra completa y **expresión regular**, con la sintaxis de
  la biblioteca `regex` de Rust. En el reemplazo, `$1` o `${nombre}` insertan grupos capturados.
  Sin expresión regular, el texto se reemplaza tal cual, `$` incluido.
- **Resultados**: uno por campo, con la petición o carpeta, el campo, cuántas veces aparece y el
  texto de alrededor. Las credenciales (contraseña, token, `client_secret`, valor de API key y
  variables secretas) también se buscan, pero su texto se muestra como `***`.
- **Reemplazar seleccionados** cambia solo los campos marcados; **Reemplazar todo**, todos. Las dos
  piden confirmación y guardan la colección.

Los secretos que están en el llavero del sistema no se buscan: no están en el archivo de la
colección.

API local: `POST /api/collections/{id}/find` con `{"query", "case_sensitive"?, "whole_word"?,
"regex"?}` y `POST /api/collections/{id}/replace` con lo mismo más `"replacement"` y, opcional,
`"targets": [{"item_id", "field"}]`.

## Papelera

Borrar una **colección, carpeta, petición o entorno** no lo borra para siempre: va a la papelera.

- Al borrar sale un aviso con **Deshacer**, que lo devuelve a su sitio.
- El botón **Papelera**, al pie del panel de colecciones y del de entornos, lista lo borrado con su
  fecha. **Restaurar** lo devuelve; **Borrar para siempre** y **Vaciar papelera** no se pueden
  deshacer.
- Lo que lleva más de **30 días** en la papelera se borra solo.

Al restaurar:

- Una carpeta o petición vuelve a su carpeta y a su posición. Si la carpeta ya no existe, vuelve a la
  raíz de la colección. Si la colección tampoco está, primero hay que restaurar la colección.
- Si ya hay una colección o un entorno con el mismo id (por ejemplo, porque se volvió a importar),
  vuelve como una copia con ids nuevos.

Los secretos de un entorno que están en el **llavero** se quedan en él mientras el entorno está en la
papelera, así que restaurarlo lo deja como estaba. Se borran del llavero al borrarlo para siempre,
salvo que otro entorno use ya su id (por ejemplo, el mismo entorno importado otra vez).

La papelera es la carpeta `.trash/` del workspace. Lleva su propio `.gitignore`, así que no entra en
Git. Cada entrada es un JSON con lo borrado y de dónde salió.

API local:

- `DELETE` de una colección, carpeta, petición o entorno responde
  `{"trash": {"id", "kind", "name", "location", "deleted_at", "requests"}}`.
- `GET /api/trash` lista; `POST /api/trash/{id}/restore` restaura.
- `DELETE /api/trash/{id}` borra una entrada para siempre; `DELETE /api/trash` vacía la papelera.
