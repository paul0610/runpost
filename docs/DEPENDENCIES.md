# Dependencias verificadas — Sprint 0

Fecha de consulta: **2026-10-16 (America/Lima)**. Las versiones se consultaron
directamente en `https://crates.io/api/v1/crates/<nombre>` y
`https://registry.npmjs.org/<nombre>`. Cargo usa `=versión`; npm usa `versión`
sin prefijos. Los dos lockfiles npm y `Cargo.lock` forman parte de la entrega.
No se ha hecho commit, por instrucción de Paul.

| Cargo (dependencias directas) | Versión |
|---|---|
| anyhow | 1.0.104 |
| axum | 0.8.9 |
| clap | 4.6.7 |
| getrandom | 0.4.3 |
| open | 5.4.4 |
| rmcp | 3.5.1 |
| rust-embed | 8.13.0 |
| serde | 1.0.229 |
| serde_json | 1.0.154 |
| tokio | 1.53.3 |
| tower-http | 0.7.1 |
| tracing | 0.1.44 |
| tracing-subscriber | 0.3.23 |
| tauri | 2.12.4 |
| tauri-build | 2.7.3 |
| tauri-plugin-single-instance | 2.5.4 |
| tauri-plugin-window-state | 2.5.0 |
| tauri-plugin-dialog | 2.8.3 |
| tauri-plugin-clipboard-manager | 2.5.0 |
| tauri-plugin-opener | 2.7.2 |
| toml | 0.9.8 |
| tempfile (tests) | 3.27.0 |

| npm | Versión | Publicación (UTC) |
|---|---|---|
| svelte | 5.57.2 | 2026-10-06 |
| vite | 8.3.4 | 2026-10-08 |
| typescript | 6.0.3 | 2026-04-16 |
| svelte-check | 4.7.6 | 2026-08-13 |
| @sveltejs/vite-plugin-svelte | 7.3.1 | 2026-09-23 |
| @tsconfig/svelte | 5.0.8 | 2026-02-17 |
| @tauri-apps/api | 2.12.2 | 2026-10-09 02:31 |
| @tauri-apps/cli | 2.12.1 | 2026-09-30 |

TypeScript 7 no se eligió porque el peer de svelte-check 4.7.6 requiere TS 5/6.
Se generaron lockfiles con `--before=2026-10-09T05:00:00Z` para aplicar siete días
también a transitivas. `.npmrc` fija `ignore-scripts=true`, `save-exact=true` y
`min-release-age=7`. npm 10 ignora esta última opción: el filtro `--before` al
resolver y la auditoría de edades son los que la aplican en este entorno.
Las instalaciones reproducibles usan `npm ci --ignore-scripts`.

## API oficial leída antes de implementar

- [rmcp 3.5.1 en crates.io](https://crates.io/crates/rmcp/3.5.1): su archivo
  `.cargo_vcs_info.json` fija el commit `79437f291b2c44053d00dcd5db969fd0cca7c887`.
  Se leyeron el [README del crate](https://github.com/modelcontextprotocol/rust-sdk/blob/79437f291b2c44053d00dcd5db969fd0cca7c887/crates/rmcp/README.md),
  el README raíz y el ejemplo oficial
  [counter_stdio.rs](https://github.com/modelcontextprotocol/rust-sdk/blob/79437f291b2c44053d00dcd5db969fd0cca7c887/examples/servers/src/counter_stdio.rs).
  Se comprobaron las firmas de `ServerHandler`, su `list_tools` vacío por defecto,
  `ServerConfig`, `ServiceExt::serve(stdio())` y `waiting()` en el código publicado.
  Esta versión usa `ServerConfig` (no el antiguo `ServerInfo`). No se usó el
  snippet de Gemini ni se añadieron macros de herramientas sin herramientas.
- [Tauri 2.12.4 en crates.io](https://crates.io/crates/tauri/2.12.4): commit
  `59b63ab500da9859f12d8888c3298bc29bba0b92`. Se leyeron el README publicado y el
  ejemplo oficial
  [helloworld](https://github.com/tauri-apps/tauri/blob/59b63ab500da9859f12d8888c3298bc29bba0b92/bench/tests/helloworld/src-tauri/src/main.rs),
  además de la plantilla de app y la ayuda de `@tauri-apps/cli` 2.12.1.
- `rquickjs` no se usa ni instala en S0: corresponde a Sprint 5.

## Auditoría aplicada

Se aplicó la skill local `npm-install-seguro`; no hay una skill `guard` instalada.
OSV individual antes de instalar las ocho dependencias npm: sin avisos.
`npm audit` en ambos proyectos: **0 vulnerabilidades**. `npm audit signatures`:
48 firmas verificadas y 24 attestations en UI; 2 firmas y 2 attestations en desktop.
Los lifecycle scripts permanecieron bloqueados; el build funciona sin habilitarlos.

El script `scripts/audit-dependencies.py` usa solo Python 3.11+ estándar, consulta
OSV por lotes para las versiones exactas de ambos ecosistemas y verifica edad,
integridad/registry y ausencia de paquetes huérfanos. El informe local queda en
`artifacts/dependency-audit.json` (ignorado por Git). No interpreta un resultado
limpio como garantía absoluta contra malware.

Resultado inicial: **635 versiones exactas consultadas**, 0 errores de edad,
integridad o huérfanos, y dos paquetes Rust con avisos. Ambos están ausentes del
árbol Windows según `cargo tree --locked --offline --target x86_64-pc-windows-msvc -i <paquete>`:

| Paquete transitivo | Aviso | Alcance y tratamiento |
|---|---|---|
| glib 0.18.5 | [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html) (alias GHSA-wrw7-89jp-8q8g) | Unsoundness en iteración de Variant; la corrección upstream empieza en glib 0.20. No se compila en Windows. Tauri/GTK de Linux exige la serie 0.18: no se fuerza una versión incompatible. Resolver o parchear antes de distribuir Linux en Sprint 7. |
| proc-macro-error 1.0.4 | [RUSTSEC-2024-0370](https://rustsec.org/advisories/RUSTSEC-2024-0370.html) | Aviso de falta de mantenimiento, sin vulnerabilidad ni parche declarado. Transitiva de macros GTK, ausente en Windows; reevaluar con la cadena GTK de Linux antes del release. |

No se encontraron identificadores `MAL-`. El auditor retorna 1 mientras existan
estos avisos, deliberadamente: no hay una allowlist que los oculte. Reproducir:

```powershell
python scripts/audit-dependencies.py
Push-Location ui
npm audit
npm audit signatures
Pop-Location
Push-Location apps/desktop
npm audit
npm audit signatures
Pop-Location
```

La primera ejecución del script `deep-audit.ps1` de la skill con Windows
PowerShell 5.1 no pudo interpretar la clave vacía estándar del package-lock v3,
por lo que reportó falsos huérfanos. Se verificó el lockfile con JSON estándar en
el auditor reproducible anterior: 73 entradas de UI (48 instaladas en Windows),
12 de desktop (2 instaladas), **0 huérfanos**. Las demás entradas son opcionales
de otras plataformas. No se borraron dependencias para ocultar hallazgos.
