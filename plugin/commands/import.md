---
description: Import a Postman v2.1 collection or environment into RunPost
argument-hint: <path to .postman_collection.json or .postman_environment.json>
---

Import `$ARGUMENTS` into RunPost with `import_postman` (`path`). Then call `ws_overview` and report what
was imported: name, number of requests or variables, and any warnings. Never print secret values.
