# Configurar un cliente MCP con RunPost

Los ejemplos usan la ruta del instalador de Windows (`C:\Users\you\AppData\Local\RunPost\runpost_mcp.exe`)
y un workspace en `C:\Users\you\RunPost`: cámbialas por las tuyas (compilado desde el código, el
binario está en `target\release\runpost_mcp.exe`). Lo más cómodo: en la app, panel **Agente** →
**Conectar agente ▾** → el cliente; copia esta misma configuración con tus rutas.

Hay dos formas de conectar, con las mismas 13 tools:

| | Por HTTP (app abierta) | Por stdio (sin app) |
|---|---|---|
| Quién arranca el servidor | la app (`runpost_app.exe`) o `runpost_mcp.exe serve --browser` | el propio cliente (`runpost_mcp.exe mcp`) |
| Panel **Agente** con las llamadas en vivo y el medidor | sí | no |
| Peticiones con `require_approval` | aviso en la app, Aprobar / Denegar (60 s) | se rechazan con `error: policy` |
| Clientes | Claude Code, Codex | Claude Code, Codex, Claude Desktop |

## Por HTTP: `http://127.0.0.1:7879/mcp`

El puerto es fijo (`settings.json` → `mcp.http_port`, `null` lo apaga) para pegar la configuración una
sola vez. Lo protege el mismo token local que la API de la app, que cambia en cada arranque: el
cliente lo pide en cada conexión ejecutando `runpost_mcp.exe mcp-headers` (sin `--port` usa el de la URL
que le pasa Claude Code, o 7879), que lo lee del llavero del
sistema (la app lo deja ahí al abrirse y lo borra al cerrarse). Con la app cerrada imprime `{}` y el
cliente no conecta.

| Cliente | Archivo de ejemplo | Dónde va |
|---|---|---|
| Claude Code | `claude-code-http.mcp.json` | `.mcp.json` en la raíz del proyecto donde usarás RunPost |
| Codex CLI | `codex-http-config.toml` | `~/.codex/config.toml` |

**Claude Code** (2.1 o superior; `headersHelper` existe desde la 1.0.119):

1. Abre la app (o `scripts\serve-browser.bat`).
2. Pega `claude-code-http.mcp.json` como `.mcp.json` en la raíz de tu proyecto (o añade la entrada
   `runpost` a la clave `mcpServers` si ya tienes uno).
3. Abre Claude Code en esa carpeta y acepta la confianza en la carpeta: Claude Code solo ejecuta el
   `headersHelper` de un `.mcp.json` de proyecto después de eso.
4. `/mcp` debe mostrar `runpost` conectado y el panel Agente de la app, `Claude Code conectado (HTTP)`.

Si reinicias la app no hay que tocar nada: el cliente vuelve a pedir el token al reconectar (y lo
repite solo si recibe un 401).

Alternativa sin `headersHelper`, válida **hasta que cierres la app** (el token cambia al reabrirla):

```powershell
$token = (& .\target\release\runpost_mcp.exe mcp-headers --port 7879 | ConvertFrom-Json).'X-RunPost-Token'
claude mcp add --transport http runpost http://127.0.0.1:7879/mcp --header "X-RunPost-Token: $token"
```

**Codex**: `codex-http-config.toml` usa `http_headers_helper` (solo para servidores locales) y sube
`tool_timeout_sec` a 90, porque Codex corta una tool a los 60 s y la espera de una aprobación puede
durar justo eso.

El endpoint solo acepta `Host` `127.0.0.1:7879` / `localhost:7879`, rechaza cualquier petición de un
navegador (cabecera `Origin`) y admite el token como `X-RunPost-Token` o `Authorization: Bearer`.

## Por stdio (sin la app)

| Cliente | Archivo de ejemplo | Dónde va |
|---|---|---|
| Claude Code | `claude-code.mcp.json` | `.mcp.json` en la raíz del proyecto donde usarás RunPost, o el comando de abajo |
| Claude Desktop | `claude_desktop_config.json` | `%APPDATA%\Claude\claude_desktop_config.json` (fusiona la clave `mcpServers`) |
| Codex CLI | `codex-config.toml` | `~/.codex/config.toml` |

Claude Code en una línea (PowerShell, desde la raíz de este repo):

```powershell
claude mcp add runpost --env RUNPOST_WORKSPACE="$PWD\workspace" -- "$PWD\target\release\runpost_mcp.exe" mcp
```

Se apunta al `.exe` directamente: no necesita `cmd /c`. `scripts\mcp.bat` también
sirve (compila si falta el binario), pero algunos clientes de Windows exigen un
ejecutable real; en ese caso `"command": "cmd.exe", "args": ["/d", "/c", "<ruta>\scripts\mcp.bat"]`.

Claude Desktop solo admite servidores locales por stdio en su archivo de configuración. Si la app
está abierta sobre el mismo workspace, `runpost_mcp mcp` le pasa cada llamada: salen en el panel
Agente y pasan por las aprobaciones. Con la app cerrada, responde él solo.

Para comprobarlo en una sesión nueva: pide al modelo `ws_overview` y luego
`run_request` sobre `"00.2 Gateway - ping"`; `env_get` debe mostrar `accessToken: "***"`.
