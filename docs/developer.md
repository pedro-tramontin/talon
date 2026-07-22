# Developer guide

This is the hands-on guide for working on Talon — building, running, testing, and debugging. For the high-level design, see [`docs/architecture.md`](docs/architecture.md). For how to contribute, see [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Prerequisites

The full per-OS list is in [`docs/requirements.md`](docs/requirements.md). The short version:

- **Rust ≥ 1.78** (`rustup update stable`)
- **Node ≥ 22.13** and **pnpm ≥ 11** (the `package.json` `packageManager` field pins the exact pnpm version)
- **Tauri 2 system dependencies** for your OS (Linux: `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, etc. — see requirements.md)
- **`cargo-deny`** for the supply-chain audit (`cargo install cargo-deny --locked`)

## Building

The project is a Rust workspace with a Tauri 2 desktop app. The UI (React) lives in `ui/` and must be built before any `cargo` command that triggers the `app` crate's `tauri::generate_context!()` proc macro (which reads `ui/dist/index.html` at compile time).

```bash
# One-time setup
git clone https://github.com/pedro-tramontin/talon
cd talon
make build-ui       # build the React UI to ui/dist/

# Build the desktop app (debug)
cargo run --bin talon

# Build the desktop app (release, matching the release.yml)
cargo build --release --bin talon
# The binary is at target/release/talon
```

The `Makefile` is the single source of truth for build orchestration. The CI workflow (`ci.yml`) runs the same `make` targets, so what works locally will work in CI.

## Inner dev loop

For UI work, use Vite's HMR. For Rust work, the cycle is `cargo check` → `cargo test` → `cargo run`.

```bash
# Terminal 1: UI dev server (HMR for React + Tailwind)
cd ui && pnpm dev

# Terminal 2: Tauri app pointed at the dev server
cargo run --bin talon
```

Vite's HMR re-renders React components on save without losing state. Zustand stores survive the HMR cycle because `ui/src/state/` uses the `persist` middleware selectively (state that should survive HMR).

For Rust, the cycle is `cargo check` (fast, no codegen) → `cargo test -p <crate>` (targeted) → `cargo run --bin talon` (full app). The Makefile has aliases:

```bash
make fmt        # cargo fmt --all -- --check
make clippy     # cargo clippy --workspace --all-targets -- -D warnings
make test       # cargo test --workspace
make audit      # pnpm audit + cargo deny check advisories
```

## Running tests

```bash
# Rust: full workspace
cargo test --workspace

# Rust: one crate
cargo test -p bk-proxy

# Rust: one test by name
cargo test -p bk-proxy scope::tests::priority_wins_over_in_scope

# UI: vitest
pnpm test

# UI: one file
pnpm test ui/src/components/ExchangeList.test.tsx

# UI: with coverage
pnpm test -- --coverage
```

Some tests are `#[ignore]`-d by design (live httpbin, H2 multiplexing). To run them:

```bash
cargo test --workspace -- --ignored
```

The ignored tests need network access; they're not part of the default CI run.

## Debugging

### Rust logs

The app uses `tracing` for structured logs. The default log level is `info`; bump it with `RUST_LOG`:

```bash
RUST_LOG=debug cargo run --bin talon
RUST_LOG=bk_proxy=trace,info cargo run --bin talon   # one crate at trace, rest at info
```

For the wire events specifically, the events fan-in logs dropped events at `warn` level. Watch for `bk_events::fan_in: missed N events` to spot a saturated broadcast channel.

### Tauri DevTools

The Tauri 2 webview exposes DevTools in debug builds. Right-click in the app window and select "Inspect Element", or use Cmd-Option-I (macOS) / Ctrl-Shift-I (Linux/Windows). The DevTools are NOT available in release builds.

### The proxy in isolation

`bk-proxy` has a CLI binary (`bk-proxy-cli`) that runs the proxy without the Tauri shell. Useful for testing a build's MITM path end-to-end:

```bash
cargo run -p bk-proxy --bin bk-proxy-cli -- --help
cargo run -p bk-proxy --bin bk-proxy-cli -- --listen 127.0.0.1:8080 --upstream example.com:443
```

### The MCP server in isolation

`bk-mcp` is a stdio MCP server. It speaks JSON-RPC over stdin/stdout. To smoke-test it:

```bash
cargo run -p bk-mcp
# In another terminal, send a JSON-RPC request:
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | cargo run -p bk-mcp
```

The server exits cleanly on EOF, no leaked tasks.

### The agent in isolation

`bk-agent` has its own test harness using wiremock to mock an OpenAI-compatible endpoint:

```bash
cargo test -p bk-agent
```

The wiremock fixture at `crates/bk-agent/tests/agent_loop_e2e.rs` is the canonical example for writing new agent tests.

## Common dev tasks

### Add a Tauri command

1. Define the command in the appropriate file under `app/src/commands/` (`core.rs` for general engine commands, `replay.rs` for replay-specific, `scope.rs` for scope/M&R, `search.rs` for search).
2. Register the command in `app/src/lib.rs`'s `invoke_handler!` macro.
3. Add a typed wrapper in `ui/src/api.ts`.
4. Add a vitest case for the wrapper (the wrapper is the public contract; the test pins the shape).
5. If the command emits events, add the event variant to `bk-events`'s `WireEvent` enum. The TS consumer will fail to compile until the new arm is added — that's the safety net.

### Add a new domain type

1. Define the type in `bk-core` with `#[non_exhaustive]`. The marker is what lets Phase 10 add new variants without breaking the IPC surface.
2. Add a `serde` derive. Wire format is JSON.
3. If the type has a body, follow the `bk-core::Body` pattern: `Complete { data: Vec<u8> }` + `Streaming { ... }`, with the IPC DTO converting `Vec<u8>` → `String` (base64) for the JSON wire.

### Add a new MCP tool

1. Add the tool function in `crates/bk-mcp/src/tools.rs`.
2. Register it in the tool registry in the same file.
3. Add a test in the same file (the `bk-mcp` policy: every tool gets one happy-path test + one error-path test, per the §3.5b review).
4. Update the tool list in `CONTRIBUTING.md` if the tool is user-facing (probably yes).

### Add a new UI store

1. Create a new file in `ui/src/state/`.
2. Match the existing pattern: one store per concern, one Zustand action per UI verb, all updates go through the store.
3. If the store is wired to the wire bus, add the event dispatch in the same file.
4. Add a vitest case in a sibling `*.test.tsx` (or `*.test.ts` if no JSX).

## Code review checklist

Before opening a PR, walk through this:

- [ ] `cargo fmt --all -- --check` is clean
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is clean
- [ ] `pnpm lint` is clean
- [ ] `cargo test --workspace` passes (count went up, not down)
- [ ] `pnpm test` passes (count went up, not down)
- [ ] `make audit` passes (no new advisories)
- [ ] New code has tests
- [ ] No `unwrap()` / `expect()` in hot paths (the proxy hot path is in `bk-proxy/src/mitm.rs` and `bk-proxy/src/upstream_pool.rs`)
- [ ] No new `#[allow(...)]` without a `// reason:` comment
- [ ] No commented-out code
- [ ] The PR description explains **what** and **why**, not just the diff
- [ ] If the change is a deviation from a documented plan, the PR body calls that out

## Performance and profiling

The proxy hot path is allocation-sensitive. If you're touching `bk-proxy/src/mitm.rs` or `bk-proxy/src/upstream_pool.rs`:

- Prefer `Bytes` over `String` for byte buffers
- Prefer `&[u8]` over `Vec<u8>` in function signatures (caller-owned)
- Avoid `format!()` in the per-request path
- Use `tokio::task::yield_now()` sparingly; long-running tasks in the proxy loop starve the upstream pool

For profiling:

```bash
# Build with debug symbols, no LTO
cargo build --release --bin talon --config 'profile.release.lto=false' --config 'profile.release.codegen-units=256'

# Profile with perf (Linux)
perf record -F 99 -p $(pgrep talon) -g -- sleep 30
perf report
```

For flamegraphs, the [`cargo-flamegraph`](https://github.com/flamegraph-rs/flamegraph) tool works out of the box.

## Common gotchas

- **The UI must be built before `cargo run`.** The `app` crate's `tauri::generate_context!()` proc macro reads `ui/dist/index.html` at compile time. If you forget `make build-ui`, the error is a `tauri::generate_context!` failure pointing at a missing file.
- **Don't change the events bus protocol without updating every consumer.** The fan-in enforces a monotonic seq; if you add a new variant to `WireEvent`, the UI's `WireEventKind` switch needs a new arm. The TS compiler will tell you.
- **The proxy hot path is not a place to add tracing.** A `tracing::info!` per request is enough to saturate the event bus. Use `tracing::trace!` (disabled by default) for per-request logs.
- **`make audit-binary` is CI-only.** It builds unstripped and scans the binary symbol table. Don't run it locally; it takes 10+ minutes and the result is the same.
