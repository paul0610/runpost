# RunPost plugin for Claude Code

<!-- SPEC-DEVIATION: S7-D1 (commands are /runpost:*, not /api:*) and S7-D2 (stdio only, with the
     bridge to the open app). See docs/DEVIATIONS.md. -->

What it adds:

- **MCP server `runpost`** — `runpost_mcp mcp` over stdio, the same 13 tools as the app
  ([docs/MCP.md](../docs/MCP.md)). If the RunPost app is open on the same workspace, every call goes
  through the app, so you see it in the Agent panel and protected requests wait for your approval.
  If the app is closed, the server answers by itself.
- **Skill `api-testing`** — how to use the tools spending few tokens (`ws_overview` first, `select`,
  `get_response` by ranges) and how to react to `error: policy`.
- **Commands** — `/runpost:run <request>`, `/runpost:suite <collection>`, `/runpost:import <file>`.

Requirements: Claude Code 2.1.275 or later and `runpost_mcp` on your `PATH` (the install steps in the
[README](../README.md#install) do that).

Install:

```bash
claude plugin install runpost --marketplace paul0610/runpost
```

Check it locally without installing: `claude plugin validate plugin --strict` from the repository
root, or `claude --plugin-dir plugin`.
