---
description: Run a saved RunPost request and summarize the result
argument-hint: <request name, path or id> [what to check]
---

Run the saved RunPost request `$ARGUMENTS` with the `run_request` tool (start with `ws_overview` or
`list_requests` only if the name is ambiguous). Report status, time and only the part of the body that
matters; use `select` when you know the field. If it fails, explain why from the `error`/`hint`
(for `error: policy`, follow the api-testing skill).
