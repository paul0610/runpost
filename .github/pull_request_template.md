## What and why / Qué y por qué

<!-- One topic per PR. Link the issue: Closes #… -->

## How it was tested / Cómo se probó

- [ ] `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [ ] `cd ui && npm run check && npm test && npm run build` (if `ui/` changed)
- [ ] Screenshots in light and dark themes (if the UI changed)

## Checklist

- [ ] No real tokens, passwords or environments in the diff (fixtures use fake values)
- [ ] New dependencies: exact version, published ≥ 7 days ago, clean on OSV
