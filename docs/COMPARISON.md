# RunPost frente a Postman, Bruno, Insomnia y Yaak

Comprobado el 2026-10-18 en la documentación, los cambios y el código de cada proyecto (Postman 12,
Bruno 4.2.1, Insomnia 13.3.1, Yaak 2026.9.0). Si algo cambió, abre un issue con la fuente. La tabla
resumida está en el [README](../README.md#why-runpost).

| | RunPost | Postman | Bruno | Insomnia | Yaak |
|---|---|---|---|---|---|
| Cuenta para guardar colecciones | No | Sí (enviar peticiones sin cuenta sí) | No | Sí para proyectos (Scratch Pad sin cuenta) | No |
| Archivos de texto para Git | Sí: un JSON por colección y entorno | Sí desde v12 (YAML "Collection v3"), ligado a un workspace en la nube | Sí: YAML (OpenCollection) o `.bru` | Git Sync (YAML), con cuenta | Sí, opcional: un YAML por recurso |
| Servidor MCP para agentes | Integrado (stdio y HTTP), sin cuenta | Aparte: remoto `mcp.postman.com` o npm `@postman/postman-mcp-server`; con cuenta | Aparte y temprano: `usebruno/bruno-mcp` (stdio, se compila desde el código) | No (solo cliente MCP; el servidor de prueba se cerró sin fusionar) | Plugin oficial `@yaak/mcp-server` dentro de la app; el servidor integrado está fusionado pero sin publicar |
| Secretos en el llavero del sistema | Sí, por variable | Parcial: la clave del Vault | Parcial: cifrado del sistema (safeStorage) | Parcial: la clave del vault (safeStorage) | Sí: la clave maestra |
| Runner por línea de comandos con JUnit | Sí (`runpost_mcp run --out junit`) | Sí (Postman CLI y Newman) | Sí (`bru --reporter-junit`) | No (Inso CLI sin JUnit) | No (CLI sin JUnit) |
| Importar Postman v2.1 | Sí | Sí | Sí | Sí | Sí |
| Licencia y precio | MIT, gratis | Propietario, plan gratis de 1 usuario | MIT, con planes de pago | Apache-2.0, plan gratis | Código MIT; binarios gratis para uso personal, de pago para trabajo |
| Interfaz en español | Sí | No encontrado (inglés y japonés) | No | No | No |

Además, en lo comprobado, solo RunPost muestra en vivo las llamadas del agente (panel Agente con
medidor de tokens) y le pide aprobación en la app para lo que la colección marque como protegido.

## Fuentes

**Postman**
- Cuenta: https://learning.postman.com/latest-v-12/docs/getting-started/basics/using-api-client ·
  https://learning.postman.com/latest-v-12/docs/use/use-collections/create-collections
- Git: https://learning.postman.com/latest-v-12/docs/use/native-git/sync ·
  https://learning.postman.com/latest-v-12/docs/use/use-collections/collections-schemas
- MCP: https://github.com/postmanlabs/postman-mcp-server
- Llavero: https://learning.postman.com/latest-v-12/docs/use/postman-vault/postman-vault-key
- CLI y JUnit: https://learning.postman.com/latest-v-12/docs/postman-cli/postman-cli-reporters ·
  https://github.com/postmanlabs/newman
- Importar: https://learning.postman.com/latest-v-12/docs/getting-started/importing-and-exporting/importing-data
- Precio: https://www.postman.com/pricing/
- Idiomas: https://learning.postman.com/latest-v-12/docs/getting-started/installation/settings/general-settings ·
  https://www.businesswire.com/news/home/20231211943255/en

**Bruno**
- Cuenta: https://github.com/usebruno/bruno
- Git: https://blog.usebruno.com/making-yaml-the-default-in-bruno-v3.1
- MCP: https://github.com/usebruno/bruno-mcp
- Secretos: https://docs.usebruno.com/secrets-management/secret-variables ·
  https://github.com/usebruno/bruno/blob/main/packages/bruno-electron/src/utils/encryption.js
- CLI: https://github.com/usebruno/bruno/blob/main/packages/bruno-cli/readme.md
- Importar: https://docs.usebruno.com/get-started/import-export-data/postman-migration
- Precio: https://www.usebruno.com/pricing
- Idiomas: https://github.com/usebruno/bruno/tree/main/packages/bruno-app/src/i18n/translation

**Insomnia**
- Cuenta: https://github.com/Kong/insomnia · https://developer.konghq.com/insomnia/storage/
- Git: https://developer.konghq.com/insomnia/git-sync/
- MCP: https://developer.konghq.com/insomnia/mcp-clients-in-insomnia/ ·
  https://github.com/Kong/insomnia/pull/9524 · https://developer.konghq.com/koh/
- Secretos: https://developer.konghq.com/insomnia/environments/ ·
  https://github.com/Kong/insomnia/blob/develop/packages/insomnia/src/main/ipc/secret-storage.ts
- CLI: https://developer.konghq.com/inso-cli/reference/run_collection/
- Importar: https://developer.konghq.com/insomnia/import-export/
- Precio: https://developer.konghq.com/insomnia/accounts/ · https://insomnia.rest/pricing
- Idiomas: https://github.com/Kong/insomnia/discussions/3497

**Yaak**
- Cuenta: https://yaak.app/docs/getting-started/security-and-privacy
- Git: https://yaak.app/docs/collaboration/local-directory-git-sync
- MCP: https://yaak.app/docs/getting-started/mcp-server · https://github.com/mountain-loop/yaak/pull/752
- Secretos: https://yaak.app/docs/collaboration/secrets-encryption · https://yaak.app/changelog/2025.6.0
- CLI: https://yaak.app/docs/getting-started/cli-usage
- Importar: https://github.com/mountain-loop/yaak/blob/main/plugins/importer-postman/src/index.ts
- Precio: https://yaak.app/pricing
- Idiomas: https://github.com/mountain-loop/yaak
