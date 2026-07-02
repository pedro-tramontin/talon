# Talon

A fast, modern web-security toolkit for solo bug bounty hunters. Proxy, replay, sitemap, fuzzer.

**Status:** Phase 1 — project skeleton. The Rust workspace, Tauri 2 shell, React UI, and CI pipeline are wired up. Subsequent phases add the actual security tooling (proxy, replay, etc.) on top of this foundation.

## Requirements

- **Rust** stable (≥ 1.78). Install via [rustup](https://rustup.rs/).
- **Node.js** ≥ 20 and **pnpm** ≥ 9. Install via [nvm](https://github.com/nvm-sh/nvm) + `npm install -g pnpm`. The `package.json` `packageManager` field pins the exact pnpm version used in CI.
- **cargo-deny** for the supply-chain audit. Install with `cargo install cargo-deny --locked`. (Optional locally; CI installs it automatically.)
- **Tauri 2 system dependencies** for your platform — see [Tauri's prerequisites](https://v2.tauri.app/start/prerequisites/). The CI workflow installs Linux deps automatically.

## Build

The project is a Rust workspace with a Tauri 2 desktop app. The UI (React) lives in `ui/` and must be built before any `cargo` command that triggers the `app` crate's `tauri::generate_context!()` proc macro (which reads `ui/dist/index.html` at compile time).

A `Makefile` at the repo root orchestrates the cross-language build:

```bash
make build-ui   # build the React UI (ui/dist/) — required once per fresh checkout
make ci         # the full pipeline: build-ui + fmt + clippy + test + audit
```

The `make ci` target is the single command new contributors should run before opening a PR, and it's also what the CI workflow validates.

Individual targets:

```bash
make fmt        # cargo fmt --all -- --check
make clippy     # cargo clippy --workspace --all-targets -- -D warnings
make test       # cargo test --workspace
make audit      # pnpm audit + cargo deny check advisories (full tree)
make audit-prod # pnpm audit --prod + cargo deny check advisories
make clean      # cargo clean + rm -rf ui/dist ui/node_modules
```

## Supply-chain security

The project enforces a blocking supply-chain audit on every PR and push to main. See [`docs/adr/0001-supply-chain-monitoring.md`](docs/adr/0001-supply-chain-monitoring.md) for the full policy.

**What's enforced:**

- `pnpm audit --audit-level=moderate` — fails on any JS advisory at moderate severity or above.
- `cargo deny check advisories` — fails on any Rust advisory (with 18 known upstream-blocked ones in `deny.toml`'s `ignore` list).
- Dependabot opens weekly PRs for new npm and cargo versions (see `.github/dependabot.yml`).

**Adding a new dependency?**

- For npm: add it to `ui/package.json`, run `pnpm install`, then `make audit`. CI will run the audit on your PR.
- For cargo: add it to the relevant `Cargo.toml`, then `make audit`. CI will run the audit on your PR.

**If CI fails on an advisory you didn't introduce:**

- Check if it's a new one (you can fix it by bumping the affected dep).
- If it's an upstream-blocked one (matches an ID in `deny.toml`), follow the ADR's re-evaluation rules before adding a new `ignore` entry.

## Layout

```
talon/
├── Cargo.toml              # workspace root
├── deny.toml               # cargo-deny config (supply-chain policy)
├── Makefile                # build orchestration
├── docs/
│   ├── requirements.md
│   └── adr/
│       └── 0001-supply-chain-monitoring.md
├── crates/
│   └── bk-core/            # Phase 1.2 — minimal library crate
├── app/                    # Phase 1.3 — Tauri 2 shell + IPC commands
│   ├── Cargo.toml
│   ├── build.rs            # thin wrapper around tauri_build::build()
│   ├── tauri.conf.json
│   ├── gen/schemas/        # auto-generated IPC permission schemas (committed for cold-build speed)
│   └── src/
└── ui/                     # React + Vite + Tailwind UI
    ├── package.json
    ├── vite.config.ts
    └── src/
```

## Development

```bash
# One-time setup
make build-ui

# During development
cd ui && pnpm dev              # Vite dev server (HMR for the UI)
cargo run --bin talon          # in another terminal — starts the Tauri app pointed at the dev server

# Before opening a PR
make ci
```

## License

Apache-2.0.
