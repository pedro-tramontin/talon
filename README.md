# Talon

A fast, modern web-security toolkit for solo bug bounty hunters. Proxy, replay, sitemap, fuzzer.

**Status:** Phase 1 — project skeleton. The Rust workspace, Tauri 2 shell, React UI, and CI pipeline are wired up. Subsequent phases add the actual security tooling (proxy, replay, etc.) on top of this foundation.

## Requirements

- **Rust** stable (≥ 1.78). Install via [rustup](https://rustup.rs/).
- **Node.js** ≥ 22.13 and **pnpm** ≥ 11. Install via [nvm](https://github.com/nvm-sh/nvm) + `npm install -g pnpm` or via [Corepack](https://nodejs.org/api/corepack.html) (the `package.json` `packageManager` field pins the exact pnpm version used in CI).
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
make audit-binary  # release-build + scan binary for build-time-only crate symbols (slow; CI-only)
make clean      # cargo clean + rm -rf ui/dist ui/node_modules
```

## Supply-chain security

The project enforces a blocking supply-chain audit on every PR and push to main. See [`docs/adr/0001-supply-chain-monitoring.md`](docs/adr/0001-supply-chain-monitoring.md) for the full policy.

For dep-version bumps (Tauri, React, Vite, etc.) and which ones go in which PR, see [`docs/dependency-upgrade-plan.md`](docs/dependency-upgrade-plan.md).

**What's enforced:**

- `pnpm audit --audit-level=moderate` — fails on any JS advisory at moderate severity or above.
- `cargo deny check advisories` — fails on any Rust advisory (with 18 known upstream-blocked ones in `deny.toml`'s `ignore` list).
- `make audit-binary` (CI only) — builds the release binary unstripped and scans the symbol table to confirm `dom_query` and `quick-xml` (the two build-time-only DoS-reachable crates) have not leaked into the runtime binary. This is the threat-model guarantee from ADR-0001.
- Dependabot opens weekly PRs for new npm and cargo versions (see `.github/dependabot.yml`).
- A quarterly cron (`talon-gtk4-watchdog`) checks the Tauri and `urlpattern` release pages; when a release mentions GTK4 or `unic-*` migration, the user is alerted that the relevant `deny.toml` ignores can be removed.

**Threat-model controls** (in `app/tauri.conf.json`):

- A strict CSP that locks the webview to bundled assets only (`default-src 'self'`, plus explicit `base-uri 'none'`, `form-action 'none'`, `object-src 'none'`, `frame-ancestors 'none'`).
- The webview is created with `WebviewUrl::App(...)` (the Tauri 2 default for `tauri.conf.json`'s `build.frontendDist`), which means the only valid URL is the local file path to `index.html` in the bundled dist. No remote URL loading is possible without a code change.

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
│   ├── dependency-upgrade-plan.md
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
