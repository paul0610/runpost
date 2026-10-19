# Red: cookies, certificados de cliente, TLS y proxy

## Cookies

Como en Postman, RunPost guarda las cookies que fijan las respuestas (`Set-Cookie`) y las envía en
las peticiones siguientes a su dominio y ruta (RFC 6265). Tiene en cuenta `Domain`, `Path`,
`Expires`/`Max-Age`, `Secure` y `HttpOnly`. Todas las peticiones de la app comparten el mismo
tarro: las de la UI, las del Runner y las de un agente por el MCP HTTP de la app.

**Gestionar cookies**: el enlace **Cookies**, a la derecha de las pestañas de la petición, abre la
lista por dominio:

- Ver nombre, valor, ruta, caducidad (o "sesión") y banderas. "subdominios" indica una cookie con
  `Domain`, que vale también para los subdominios.
- **Añadir** una cookie: el dominio y su texto `Set-Cookie`, por ejemplo
  `sid=abc123; Path=/; HttpOnly`. Sin `Path` vale para `/`.
- **Editar**: pulsa el nombre, cambia el texto y guarda. Con otro nombre, ruta o dominio, es otra
  cookie y la anterior se borra. Una fecha pasada la borra.
- **Borrar** una cookie, las de un dominio o, desde la API, todas.

Las cookies viven **en memoria** mientras la app está abierta: no se guardan en el workspace, porque
las de sesión son credenciales. El MCP por stdio es otro proceso y tiene su propio tarro.

API local: `GET /api/cookies`, `PUT /api/cookies` con `{"domain", "cookie"}`, y
`DELETE /api/cookies?domain=…&path=…&name=…` (una cookie), `?domain=…` (un dominio) o sin
parámetros (todas).

## Certificados de cliente (TLS mutuo)

Para las APIs que piden certificado de cliente: **Ajustes → Certificados de cliente → Añadir
certificado**.

| Campo | Qué es |
|---|---|
| Host | `api.example.com`, o `*.example.com` para sus subdominios (no el dominio base) |
| Puerto | Opcional: sin puerto vale para cualquiera |
| Certificado | Archivo PEM, con la cadena intermedia si la hay |
| Clave | Archivo PEM de la clave privada **sin cifrar** (PKCS#8, RSA o EC) |

- El certificado solo se presenta a las peticiones HTTPS cuyo host (y puerto, si se indica)
  coinciden. Si varios coinciden, vale el primero de la lista.
- Los ajustes guardan las **rutas**; la clave nunca se copia al workspace.
- Un archivo PFX/PKCS#12 se convierte con
  `openssl pkcs12 -in client.pfx -out client.pem -nodes`, que deja el certificado y la clave en un
  mismo PEM: usa ese archivo en los dos campos.
- Si un archivo falta o no es válido, la petición no sale y el error lo dice:
  `client certificate for api.example.com: cannot read the key …`.

Los archivos se leen al crear el cliente HTTP de ese certificado. Si los sustituyes con la app
abierta, reiníciala para que use los nuevos.

## TLS y proxy

- **Verificar certificados TLS** (Ajustes): desactivarlo acepta certificados de servidor no válidos
  o autofirmados. Cada petición puede cambiarlo en su pestaña **Ajustes**.
- **Proxy** (Ajustes → Avanzado): `http://host:puerto` para todas las peticiones. Sin él no se usa
  ninguno, tampoco el de las variables de entorno `HTTP_PROXY`/`HTTPS_PROXY`.
