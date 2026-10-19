# Autenticación en RunPost

La pestaña **Auth** de una petición, carpeta o colección tiene los mismos tipos que Postman:
**Inherit**, **No auth**, **Bearer Token**, **Basic Auth**, **API Key**, **Digest Auth** y
**OAuth 2.0**. "Inherit" usa la auth de la carpeta más cercana que defina una, o la de la
colección. Los campos aceptan `{{variables}}`; para credenciales, usa variables secretas.

Cuando se envía, en la pestaña Headers aparece qué cabecera añade la auth. En el historial, en la
consola y en lo que ve un agente por MCP, los valores secretos salen como `***`.

## Digest Auth (RFC 7616)

RunPost envía la petición sin credenciales. Si el servidor contesta `401` con
`WWW-Authenticate: Digest …`, repite la petición con la cabecera `Authorization: Digest …` firmada:

- Algoritmos: MD5, SHA-256 y SHA-512-256, con y sin `-sess`. Si el servidor ofrece varios, se elige
  el más fuerte.
- `qop=auth` (o sin `qop`, como en RFC 2069), `opaque` y `userhash`.
- `auth-int` (que firma el cuerpo) no se soporta: ese reto se ignora.

El historial guarda la cabecera del segundo intento con `response="***"`. La contraseña no se guarda
nunca.

## OAuth 2.0

Un solo tipo, con su **tipo de concesión**:

| Concesión | Campos |
|---|---|
| **Client Credentials** | Access Token URL, Client ID, Client Secret, Scope |
| **Authorization Code (PKCE)** | Auth URL, Access Token URL, Client ID, Client Secret (opcional), Scope, Callback URL |

En los dos casos, **Client Authentication** elige cómo se presenta el cliente al pedir el token:
cabecera Basic (por defecto, la que todo servidor debe aceptar) o `client_id` y `client_secret` en
el cuerpo. Un cliente sin secreto (público) siempre manda `client_id` en el cuerpo.

### Client Credentials

No hace falta hacer nada más: al enviar, RunPost pide el token y lo usa como
`Authorization: Bearer …`. Lo reutiliza mientras no caduque y pide otro cuando caduca (30 segundos
antes). **Obtener token nuevo** lo pide en el momento.

### Authorization Code con PKCE

1. Registra la **Callback URL** en el proveedor. Por defecto es `http://127.0.0.1:7880/callback`, y
   se puede cambiar por otro puerto u otra ruta de tu equipo.
2. Pulsa **Obtener token nuevo**: se abre el navegador en la página de inicio de sesión del
   proveedor, y RunPost escucha la vuelta en esa dirección durante 5 minutos.
3. Al iniciar sesión, el proveedor vuelve al callback. RunPost comprueba el `state` (protección
   CSRF), canjea el código con el `code_verifier` de PKCE (S256) y guarda el token.

Si el navegador no se abrió, **Copiar enlace de inicio de sesión** copia la URL para pegarla a mano.
Lo que llegue al callback sin el `state` de ese inicio de sesión se rechaza. El callback solo acepta
direcciones de este equipo: `127.0.0.1`, `localhost` o `[::1]`, por `http://` y con puerto.

Cuando el token caduca, RunPost lo renueva con el **refresh token**, si el proveedor lo dio. Si la
renovación falla, hay que volver a iniciar sesión.

### Dónde viven los tokens

- En memoria, mientras la app está abierta.
- Los de authorization code, además, en el **llavero del sistema** (Windows Credential Manager,
  Keychain de macOS, Secret Service en Linux), para no repetir el inicio de sesión al reiniciar.
- **Nunca** en el workspace, en el historial ni en la salida MCP. La UI tampoco los recibe: el panel
  **Token actual** solo muestra el tipo, la caducidad, el scope y si se renueva solo.

**Borrar token** lo olvida, del llavero también. Si cambias el Client Secret, la URL o el scope,
cuenta como otra configuración y se pide un token nuevo.

### Con un agente (MCP)

`run_request` y `send` usan el token igual que la app. Con client credentials lo piden solos. Con
authorization code, el usuario tiene que iniciar sesión antes en la app: si no hay token, el agente
recibe el error `oauth2` con esa indicación. `get_request` muestra `***` en lugar de las
credenciales escritas tal cual en la auth (las `{{variables}}` se ven como tales).

### Importar y exportar

- **Postman v2.1**: `digest` y `oauth2` (client credentials y authorization code, con o sin PKCE) se
  importan y se exportan.
- **Insomnia** y **Bruno**: digest y OAuth 2.0 con esas dos concesiones.
- **OpenAPI 3**: los flujos `clientCredentials` y `authorizationCode`, con `{{clientId}}` y
  `{{clientSecret}}` como variables de la colección.

Si el callback del origen no es de este equipo (por ejemplo, el de Postman en la nube), se usa el de
RunPost y el aviso de la importación pide registrarlo en el proveedor. Las demás concesiones
(implicit, password) llegan como "No auth", con un aviso.

### Snippets de código

Con OAuth 2.0, el snippet lleva `Authorization: Bearer {{access_token}}` y un comentario: el token
real no sale nunca del motor. Con digest, curl usa `--digest`, Python `HTTPDigestAuth` y
PowerShell `-Credential`. fetch no puede contestar el reto, y lo indica en un comentario.
