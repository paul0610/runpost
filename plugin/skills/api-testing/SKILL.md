---
name: api-testing
description: Use when testing, exploring or debugging HTTP APIs with the RunPost MCP tools (ws_overview, run_request, run_collection, get_response, env_get, upsert_request...). How to spend few tokens and how to react to `error: policy`.
---

# API testing with RunPost (few tokens)

RunPost keeps the requests, environments and secrets; you refer to them by name. Its answers are
compact on purpose — ask only for what the task needs.

1. **Start with `ws_overview`** (collections, environments, active env). Then `list_requests` with
   `collection`, `folder` or `q`; never list everything when you only need one folder.
2. **Run saved requests by name or id**: `run_request {"request": "00.2 Gateway - ping"}`. Do not
   rebuild URLs, headers or tokens: `{{variables}}` and secrets resolve inside RunPost. Secrets always
   show as `***` — never ask the user for them and never paste them.
3. **Ask for little**: `select` (JSONPath, e.g. `$.session.scope`) for one field; `body_max` to cap
   text; `headers: true` only when the task is about headers.
4. **Do not re-run to read more**: `get_response {"response_id", "range": [from, to]}` or a new
   `select` on the cached response.
5. **Compare instead of rereading**: `get_response {"response_id": B, "diff_against": A}` returns only
   what changed (`{"diff": []}` = same). `save_ref: true` stores a reference; `diff_against: "ref"`
   compares with it. Volatile fields (`timestamp`, `requestId`, `traceId`) are ignored; add more with
   `ignore`.
6. **Suites**: `run_collection {"collection", "folder", "output": "summary"}`; call again with
   `"output": "failures"` only if something failed.
7. **Build in one call**: `upsert_request {"collection", "requests": [...]}` creates or updates many
   requests at once; `import_postman {"path" | "text"}` instead of writing Postman JSON by hand.
8. **Variables**: `env_get {"keys": [...]}`; `env_set {"env", "vars", "secret": ["token"]}` stores
   secrets in the OS keyring.
9. **`{"error": "policy"}`** means the collection's agent policy stopped the call:
   - `rule: "allow_methods"`: agents may not use that method there. Tell the user; do not retry and do
     not work around it (for example with `send` to the same URL).
   - `rule: "require_approval"`: the user must approve. With the RunPost app open (MCP over HTTP) the
     call already waited up to 60 s for Approve/Deny. `"the user denied it in the app"`: stop. `"no
     answer in 60 s"` or `"ask the user to approve in the app"`: ask the user to open RunPost and
     approve, then retry once.
   - `rule: "max_calls_per_minute"`: wait the seconds in `hint`, then retry.
10. **Network errors** (`connect`, `timeout`): report the masked URL and message; check the base URL
    with `env_get` before retrying.
