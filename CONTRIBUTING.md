# Contributing to RunPost

Thanks for helping! Issues and pull requests are welcome in **English or Spanish**
(*se aceptan issues y PR en español*).

## Before you start

- Read the [README](README.md) and the MCP contract in [docs/MCP.md](docs/MCP.md). For a new
  feature or a UI change, open an issue first (a sketch or screenshot helps) so we agree on the
  design before you write code.
- Look for issues labelled **good first issue** or **help wanted**.
- Security problems: do **not** open a public issue; follow [SECURITY.md](SECURITY.md).

## Development setup

Requirements: Rust via rustup (`rust-toolchain.toml` pins the version), Node.js LTS (22.12+), npm, and
the [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
cd ui && npm ci --ignore-scripts && npm run build && cd ..
cargo build
cargo run -p runpost-cli -- serve --browser   # the UI at http://127.0.0.1:7878
```

On Windows, `scripts\build.bat` and `scripts\start-ui.bat` build and open the native app.

## Checks (all must pass)

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd ui && npm run check && npm test && npm run build
```

Tests never touch the real OS keyring (`RUNPOST_KEYRING=memory` / `MemoryKeyring`) nor the network
(`wiremock`). Never commit real tokens or environments: fixtures use fake values only.

## Pull requests

- One topic per PR, with tests. Keep the existing style: small modules, comments that explain *why*.
- Dependencies: exact versions (`=x.y.z` in Cargo, no `^`/`~` in npm), published at least 7 days
  ago and clean on [OSV](https://osv.dev); explain why the dependency is needed.
- UI changes: include screenshots in light and dark themes.

## License

By contributing you agree that your contributions are licensed under the [GNU AGPL-3.0](LICENSE),
and that Paul Rojas may also license them under other terms, including the commercial license
described in [LICENSING.md](LICENSING.md).
