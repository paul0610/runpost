<p align="center"><img src="apps/desktop/src-tauri/icons/128x128@2x.png" width="96" alt="Logotipo de RunPost"></p>

<h1 align="center">RunPost</h1>

<p align="center"><b>Un binario. Sin cuenta. Sin telemetría.</b><br>
El cliente HTTP que usan tú y tu agente de IA: una app de escritorio tipo Postman con servidor MCP integrado.</p>

<p align="center">
  <a href="https://github.com/paul0610/runpost/actions/workflows/ci.yml"><img src="https://github.com/paul0610/runpost/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/paul0610/runpost/releases"><img src="https://img.shields.io/github/v/release/paul0610/runpost?include_prereleases&amp;sort=semver" alt="Última versión"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="Licencia MIT"></a>
  <a href="docs/MCP.md"><img src="https://img.shields.io/badge/MCP-13_tools-ff6c37.svg" alt="Servidor MCP con 13 tools"></a>
  <img src="https://img.shields.io/badge/Windows%20%C2%B7%20macOS%20%C2%B7%20Linux-lightgrey.svg" alt="Windows, macOS y Linux">
</p>

<p align="center">
  <a href="https://paul0610.github.io/runpost/es.html">Web</a> ·
  <a href="README.md">English</a> ·
  <a href="https://github.com/paul0610/runpost/releases/latest">Descargas</a> ·
  <a href="docs/MCP.md">Tools MCP</a> ·
  <a href="CONTRIBUTING.md">Contribuir</a>
</p>

![RunPost: enviar una petición guardada, pasar al tema oscuro y el panel Agente mientras Claude Code trabaja, con una aprobación](docs/media/runpost-demo.gif)

RunPost es un cliente HTTP local para quien trabaja con colecciones al estilo Postman **y** con
agentes de IA. Construyes y envías peticiones en una interfaz conocida; tu agente (Claude Code,
Codex…) ejecuta esas mismas peticiones guardadas por el servidor MCP de RunPost, recibe respuestas
compactas con los secretos tapados, y tú ves cada llamada en vivo —y apruebas las delicadas— en el
panel **Agente**.

## Por qué RunPost

A octubre de 2026 ([detalle y fuentes](docs/COMPARISON.md); se aceptan correcciones):

| | RunPost | Postman | Bruno | Insomnia | Yaak |
|---|---|---|---|---|---|
| Cuenta para guardar colecciones | No | Sí | No | Sí para proyectos | No |
| Archivos de texto para Git | JSON por colección | YAML (v12), ligado a la nube | YAML / `.bru` | Git Sync, con cuenta | YAML opcional |
| Servidor MCP para agentes | Integrado (stdio + HTTP), sin cuenta | Aparte (remoto o npm), con cuenta | Aparte, temprano (`bruno-mcp`) | No (solo cliente MCP) | Plugin en la app |
| Llamadas del agente en vivo y aprobación en la app | Sí | — | — | — | — |
| Secretos en el llavero del sistema | Por variable | Clave del vault | Cifrado del sistema | Clave del vault | Clave maestra |
| Runner por línea de comandos con JUnit | Sí | Sí (Postman CLI, Newman) | Sí (`bru`) | No | No |
| Licencia | MIT, gratis | Propietario, plan gratis | MIT, planes de pago | Apache-2.0, plan gratis | Código MIT, de pago para trabajo |
| Interfaz en español | Sí | No | No | No | No |

"—": no encontrado en su documentación.

## Cifras

Medidas con los binarios release reales; las de tokens, contra respuestas simuladas de Acme
Gateway (detalle en [docs/BENCH.md](docs/BENCH.md)):

| Qué | RunPost |
|---|---|
| `tools/list` completo (13 tools, una vez por sesión) | ≈ 1 675 tokens |
| `run_request` "00.2 Gateway - ping" | ≈ 200 caracteres ≈ 50 tokens |
| `run_request` con `select` a un campo | < 60 tokens |
| Una respuesta JSON típica de 5 KB | < 700 tokens (minificada, truncada, el resto a demanda) |
| Coste fijo del plugin de Claude Code | ≈ 112 tokens (+ ≈ 630 cuando se usa la skill) |
| App de escritorio en Windows 11: tamaño, arranque hasta la ventana usable, memoria en reposo | 20,5 MB, 1,25–1,45 s, 35–37 MB (más el WebView del sistema) |
| CLI / servidor MCP: tamaño, arranque hasta `initialize` | 17,9 MB, ≈ 75 ms |
| Sesión completa de Claude Code, curl frente a RunPost | en medición: el arnés está en `scripts/bench/` (ver [docs/BENCH.md](docs/BENCH.md)) |

## Instalar

Descarga desde [Releases](https://github.com/paul0610/runpost/releases/latest). Los binarios aún no
están firmados, así que el primer arranque pide confirmación.

**Windows**: descarga `RunPost_0.2.0_x64-setup.exe` y ábrelo con doble clic. Se instala para tu
usuario en `%LOCALAPPDATA%\RunPost` (sin permisos de administrador), con `runpost_mcp.exe`, y queda en
el menú Inicio. `RunPost_0.2.0_x64-portable.exe` funciona sin instalar. O, en PowerShell:

```powershell
Invoke-WebRequest https://github.com/paul0610/runpost/releases/download/v0.2.0/RunPost_0.2.0_x64-setup.exe -OutFile RunPost-setup.exe
.\RunPost-setup.exe
[Environment]::SetEnvironmentVariable('Path', [Environment]::GetEnvironmentVariable('Path','User') + ";$env:LOCALAPPDATA\RunPost", 'User')
```

**macOS** (Apple silicon):

```bash
curl -LO https://github.com/paul0610/runpost/releases/download/v0.2.0/RunPost_0.2.0_aarch64.dmg
hdiutil attach RunPost_0.2.0_aarch64.dmg && cp -R /Volumes/RunPost/RunPost.app /Applications/ && hdiutil detach /Volumes/RunPost
xattr -dr com.apple.quarantine /Applications/RunPost.app && sudo ln -sf /Applications/RunPost.app/Contents/MacOS/runpost_mcp /usr/local/bin/runpost_mcp
```

**Linux** (Debian/Ubuntu; también hay `.AppImage`):

```bash
curl -LO https://github.com/paul0610/runpost/releases/download/v0.2.0/RunPost_0.2.0_amd64.deb
sudo apt install ./RunPost_0.2.0_amd64.deb
runpost_app
```

La última línea de cada bloque deja `runpost_mcp` (el CLI y servidor MCP) en el `PATH`, que es lo que
necesitan las integraciones con agentes. El release trae también el CLI suelto.

## Conectar tu agente

**Claude Code**: una línea en la terminal (Claude Code 2.1.275 o posterior):

```bash
claude plugin install runpost --marketplace paul0610/runpost
```

(o dentro de Claude Code: `/plugin marketplace add paul0610/runpost` y después `/plugin install runpost@runpost`).
El plugin añade el servidor MCP de RunPost, la skill `api-testing` (cómo gastar pocos tokens) y los
comandos `/runpost:run`, `/runpost:suite` y `/runpost:import`. Con la app de RunPost abierta sobre el
mismo workspace, sus llamadas pasan por la app: las ves en el panel Agente y las peticiones protegidas
esperan tu aprobación.

**Codex, Claude Desktop y otros**: en la app, **Agente → Conectar agente ▾** copia la configuración
con tus rutas. Ejemplos en [docs/mcp-client-configs/](docs/mcp-client-configs/), con un
[`AGENTS.md`](docs/mcp-client-configs/AGENTS.md) para Codex.

## Funciones

- Interfaz tipo Postman: colecciones, carpetas, pestañas, editores de params/cabeceras/cuerpo/auth,
  entornos y globales, historial, consola, documentación markdown, tema claro y oscuro, atajos de
  Postman. En español e inglés (Ajustes → Idioma).
- Importar desde Postman v2.1, OpenAPI 3, HAR, Insomnia (v4 y v5), Bruno (`.bru`, YAML o JSON) y
  cURL ([qué se traduce](docs/IMPORT.md)); exportar a Postman v2.1. Snippets de código (cURL, fetch,
  PowerShell, Python) que dejan los secretos como `{{variables}}`, y cuerpos GraphQL
  ([detalle](docs/REQUESTS.md)).
- Auth: bearer, basic, API key, digest y OAuth 2.0 (client credentials, y authorization code con
  PKCE y callback local). Los tokens quedan en memoria o en el llavero del sistema, nunca en archivos
  ([detalle](docs/AUTH.md)).
- Gestor de cookies y certificados de cliente por host para TLS mutuo ([detalle](docs/NETWORK.md)).
- Ejemplos guardados por petición, buscar y reemplazar en una colección (con expresión regular si
  quieres) y papelera con deshacer durante 30 días ([detalle](docs/COLLECTIONS.md)).
- Scripts `pm.*` Pre-request y Tests (sandbox QuickJS), Collection Runner con datos CSV/JSON e
  informes JUnit/HTML, también desde la terminal (`runpost_mcp run`).
- Servidor MCP (stdio, o Streamable HTTP con la app abierta) con 13 tools compactas: peticiones
  guardadas por nombre, `select` (JSONPath), `get_response` por rangos, `diff_against` para ver solo lo
  que cambió, resúmenes de `run_collection`.
- Guardarraíles por colección para el agente: métodos permitidos, aprobación en la app (60 s), límite
  por minuto.
- Secretos tapados en todas partes (`***` para el agente, el historial y los logs) y, si quieres, en el
  llavero del sistema (Credential Manager, Keychain, Secret Service) en vez del archivo del workspace.
- Workspace en texto plano y amigable con Git: un JSON por colección y por entorno.

## Compilar y desarrollar

Cómo contribuir: [CONTRIBUTING.md](CONTRIBUTING.md). Los ejemplos usan la colección de
`tests/fixtures/` (Acme, una API ficticia) y su mock, `scripts/dev/mock_acme.py`.

### Compilar en Windows 11

Requisitos: Rust mediante rustup (el archivo `rust-toolchain.toml` instala
**1.97.0**, clippy y rustfmt), Node.js LTS compatible con Vite (Node 22.15.1
funciona; mínimo 20.19 o 22.12), npm, Microsoft C++ Build Tools con herramientas
MSVC x64 y Windows SDK, y Microsoft Edge WebView2 Runtime. Windows 11 normalmente
ya incluye WebView2. [Requisitos oficiales de Tauri](https://v2.tauri.app/start/prerequisites/).

Desde CMD o PowerShell, en la raíz:

```bat
scripts\build.bat
scripts\start-ui.bat
```

Salidas del **mismo workspace Cargo**:

- `target\release\runpost_mcp.exe`: CLI con `mcp`, `serve`, `run`, `import`, `export`.
- `target\release\runpost_app.exe`: aplicación nativa, título RunPost.

Los nombres evitan que `runpost.exe` y `RunPost.exe` choquen en Windows, que no distingue
mayúsculas. Los scripts usan el CLI de Tauri instalado localmente en `apps/desktop`, con versión
exacta y lockfile, sin requerir `cargo tauri` global.

`build.bat` hace `npm ci --ignore-scripts`, compila `ui/dist`, compila el CLI y
ejecuta Tauri con `--no-bundle`. No genera instalador (lo hace el release). La compilación Cargo
directa requiere preparar primero la UI:

```powershell
Push-Location ui
npm ci --ignore-scripts
npm run check
npm run build
Pop-Location
cargo build --release
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

### Usar RunPost

```bat
scripts\start-ui.bat
scripts\serve-browser.bat
scripts\mcp.bat
scripts\dev.bat
```

`start-ui` abre la ventana nativa (1280×800, mínimo 900×600): árbol de colecciones, pestañas,
constructor de
petición (Params, Auth, Headers, Body, Scripts, Ajustes, Docs y Ejemplos), visor de respuesta, tema
claro/oscuro/sistema y atajos de Postman (`Ctrl+Enter`, `Ctrl+S`, `Ctrl+T`, `Ctrl+W`,
`Ctrl+K`, `Ctrl+B`, `Ctrl+,`, `Ctrl+E`, `Ctrl+Alt+C`, `Ctrl+Shift+L`). En el primer arranque, el
estado vacío ofrece **Importar de Postman**, **Nueva petición** y **Conectar tu agente**.
Además: gestor de entornos y globales (`Variable | Valor | Secreto | 🔒`, los secretos
se ven `••••••••` hasta pulsar el ojo), historial (abrir, guardar en una colección, vaciar),
**Consola** en el panel inferior con la petición resuelta y los secretos como `***`, pestaña
**Docs** en markdown, y en el menú contextual **Exportar** (con "Quitar secretos" marcado) y
**Copiar como curl** (los secretos quedan como `{{variable}}`). El botón `</>` muestra la petición
como cURL, fetch, PowerShell o Python; **Importar** acepta un cURL pegado o archivos; clic derecho en
una colección → **Buscar y reemplazar…**; y lo que borras va a la **Papelera** (al pie del panel), con
**Deshacer**. Todo se maneja también con teclado.
Instancia única, tamaño y posición recordados; cerrar la app detiene su servidor.
El modo navegador (`serve-browser`) sirve la misma UI en `http://127.0.0.1:7878` hasta
Ctrl+C. `dev` inicia Vite en `http://localhost:5173` y la ventana nativa con recarga.
Los cambios que haga un agente por MCP aparecen en la UI al instante.

El workspace se elige con prioridad `--workspace DIR` > `RUNPOST_WORKSPACE` >
`./workspace`. Los scripts resuelven su ubicación desde la carpeta del script,
conservan un `RUNPOST_WORKSPACE` existente y pasan sus argumentos al ejecutable:

```powershell
.\scripts\start-ui.bat --workspace D:\datos\runpost
.\target\release\runpost_mcp.exe serve --browser --port 7878 --workspace D:\datos\runpost
```

#### Importar y exportar

```powershell
# Colección: el archivo nativo toma el nombre del archivo importado.
.\target\release\runpost_mcp.exe import .\tests\fixtures\Acme-Gateway-Scopes.postman_collection.json
# -> workspace\collections\acme-gateway-scopes.collection.json (3 carpetas, 10 peticiones)
# Entorno (se detecta solo; --kind collection|environment lo fuerza).
.\target\release\runpost_mcp.exe import .\tests\fixtures\Acme-DESA-fake.postman_environment.json
# OpenAPI 3, HAR, Insomnia o una carpeta de Bruno: el formato se detecta por el contenido.
.\target\release\runpost_mcp.exe import D:\datos\mi-coleccion-bruno
# Exportar por id, nombre o slug; --strip-secrets vacía las variables secretas.
.\target\release\runpost_mcp.exe export acme-gateway-scopes --strip-secrets -o acme.postman_collection.json
```

Reimportar la misma colección la sustituye y conserva los ids. Qué se traduce de cada formato y
qué no: [docs/IMPORT.md](docs/IMPORT.md).

#### Scripts y Collection Runner

Los scripts Pre-request y Tests de colecciones, carpetas y peticiones corren como en Postman
(subconjunto `pm.*`, sin `require` ni CryptoJS: ver [docs/SCRIPTS.md](docs/SCRIPTS.md)). En la app,
clic derecho en una colección o carpeta → **Ejecutar** abre el Runner en el panel inferior. En la
terminal:

```powershell
# Toda la colección con el entorno falso (también vale el slug o un trozo único del nombre).
.\target\release\runpost_mcp.exe run "Acme Gateway Scopes" --env fake
# Una iteración por fila del archivo de datos e informe JUnit para CI.
.\target\release\runpost_mcp.exe run "Acme Gateway Scopes" --env fake --data .\tests\fixtures\tokens-matriz-fake.csv --out junit -o informe.xml
```

Opciones: `--folder`, `--iterations`, `--delay` (ms), `--out pretty|json|junit`, `--fail-fast`,
`-o archivo`. Sale con **0** si todo pasa, **1** si falla un test o una petición no responde y **2**
si no puede empezar. `scripts\dev\mock_acme.py` simula el gateway (3080) y la API (3081) de Acme
para probarlo sin red.

#### Agente por HTTP, aprobaciones y llavero

Con la app abierta, el MCP también escucha en `http://127.0.0.1:7879/mcp` (mismas tools, mismo token
local). En el panel **Agente** (cuarto icono): **Conectar agente ▾** copia la configuración de
Claude Code o Codex, y verás cada llamada del modelo (tool, petición, estado, ms, bytes crudos →
enviados), con filtro por tool y sesión, el medidor de tokens ahorrados y **Exportar** (tabla
Markdown para `docs/BENCH.md`). Configurar Claude Code: pega
[docs/mcp-client-configs/claude-code-http.mcp.json](docs/mcp-client-configs/claude-code-http.mcp.json)
como `.mcp.json` de tu proyecto; el cliente pide el token del arranque en curso con
`runpost_mcp.exe mcp-headers` (detalles en [docs/mcp-client-configs/](docs/mcp-client-configs/)).

Clic derecho en una colección → **Agente…**: métodos permitidos, qué requiere aprobación y límite
por minuto. Si el modelo pide algo protegido (p. ej. un `DELETE`), arriba a la derecha aparece
"Claude Code quiere ejecutar…" con **Aprobar** / **Denegar** y 60 s de cuenta atrás; al denegar, el
modelo recibe `error: policy`. En **Entornos**, **Mover al llavero** saca un secreto del archivo JSON
y lo guarda en el Credential Manager (Keychain en macOS, Secret Service en Linux); el archivo solo
conserva `"store": "keyring"`. `get_response` con `diff_against` dice qué cambió entre dos
respuestas ignorando campos volátiles como `timestamp`.

### MCP stdio

Configura el cliente MCP para lanzar `scripts\mcp.bat` (en clientes Windows que
necesiten un ejecutable real, usa `cmd.exe /d /c <ruta>\scripts\mcp.bat`), o
directamente `target\release\runpost_mcp.exe` con argumento `mcp`.
Ejemplos listos para Claude Code, Claude Desktop y Codex en
[docs/mcp-client-configs/](docs/mcp-client-configs/). Expone 13 tools (`ws_overview`,
`run_request`, `run_collection`, `get_response`, `env_get`, `upsert_request`…) con respuestas compactas y
secretos siempre enmascarados; el contrato está en [docs/MCP.md](docs/MCP.md) y las
medidas de tokens en [docs/BENCH.md](docs/BENCH.md). Logs exclusivamente en stderr y
`workspace/logs/mcp.log` (una línea por llamada, sin contenido).

Prueba de una sola línea desde CMD:

```bat
echo {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}} | scripts\mcp.bat
```

Para `tools/list`, mantén stdin abierto y envía después de la respuesta a
`initialize` la notificación `notifications/initialized` y la petición:

```json
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
```

### Comprobar la autenticación HTTP

Con `scripts\serve-browser.bat` activo, ejecuta en **otra PowerShell**:

```powershell
curl.exe -s -o NUL -w '%{http_code}' http://127.0.0.1:7878/api/health
# 401
$page = (Invoke-WebRequest http://127.0.0.1:7878).Content
$match = [regex]::Match($page, 'window\.__RUNPOST_BACKEND__=(.*?);</script>')
$backend = $match.Groups[1].Value | ConvertFrom-Json
curl.exe -s -o NUL -w '%{http_code}' -H "X-RunPost-Token: $($backend.token)" http://127.0.0.1:7878/api/health
# 200; omite -o NUL -w ... para ver {"ok":true}
Remove-Variable page,match,backend
```

El token es efímero (uno por arranque), nunca en URL, archivos ni logs; mientras la app
está abierta también queda en el llavero del sistema para que el agente lo pida por HTTP
(`runpost_mcp.exe mcp-headers`), y se borra al cerrarla. El HTML del modo navegador lo
incluye en su bootstrap (sin caché); los assets empaquetados de Tauri no lo contienen. Todos los binds son `127.0.0.1`, se valida `Origin` y `Host`
y CORS permite solo los orígenes de la especificación (`localhost:5173` solo con
`--dev`). Los datos locales y logs se crean al iniciar; los entornos y colecciones locales, la
papelera, los logs y SQLite están excluidos de Git.

### macOS y Linux

```bash
chmod +x scripts/*.sh
./scripts/build.sh
./scripts/start-ui.sh
./scripts/serve-browser.sh
./scripts/mcp.sh
./scripts/dev.sh
```

Los `.sh` hacen las mismas operaciones en el mismo orden que los `.bat`. macOS
requiere Xcode Command Line Tools; Linux requiere las dependencias oficiales de
Tauri v2, incluido `webkit2gtk-4.1` y `libayatana-appindicator3`. GitHub Actions compila y
prueba en Windows, macOS y Linux. Los avisos transitivos de Linux y la auditoría aplicada están en
[docs/DEPENDENCIES.md](docs/DEPENDENCIES.md).

MIT © 2026 Paul Rojas — creado por [Paul Rojas](https://github.com/paul0610).
