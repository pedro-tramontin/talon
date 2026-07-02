# Talon Makefile
#
# Build orchestration for the Talon workspace. Cargo handles the Rust
# side; this Makefile handles the cross-language bits (building the UI
# before cargo) and gives a single entry point for the common commands
# developers and CI both invoke.
#
# Usage:
#   make build-ui   # build the React UI (ui/dist/) -- required before cargo check
#   make ci        # the full local pipeline: build-ui + fmt + clippy + test + audit
#   make fmt       # cargo fmt --all -- --check
#   make clippy    # cargo clippy --workspace --all-targets -- -D warnings
#   make test      # cargo test --workspace
#   make audit     # supply-chain audit (pnpm audit + cargo deny)
#   make audit-prod# same as make audit, but pnpm audit --prod only
#
# CI uses the same `make fmt`, `make clippy`, `make test`, `make audit`
# targets from .github/workflows/ci.yml, but the rust job downloads the
# UI dist as a workflow artifact instead of running `make build-ui`
# (pnpm is not installed in the rust job's runner; the UI job does the
# pnpm build and uploads the result). The supply-chain job runs the
# audit checks directly.

.PHONY: help build-ui fmt clippy test audit audit-prod audit-ci ci clean

# Default target: print help.
help:
	@echo "Talon build targets:"
	@echo "  make build-ui   Build the React UI (ui/dist/)"
	@echo "  make ci         Full local pipeline: build-ui + fmt + clippy + test + audit"
	@echo "  make fmt        cargo fmt --all -- --check"
	@echo "  make clippy     cargo clippy --workspace --all-targets -- -D warnings"
	@echo "  make test       cargo test --workspace"
	@echo "  make audit      pnpm audit + cargo deny check advisories (full tree)"
	@echo "  make audit-prod pnpm audit --prod + cargo deny check advisories"
	@echo "  make clean      cargo clean + rm -rf ui/dist ui/node_modules"

# Build the React UI. Required before any cargo command that triggers
# the app crate's tauri::generate_context!() macro (i.e. cargo check /
# clippy / test / build). The macro reads ui/dist/index.html at
# expansion time and panics with a cryptic error if it's missing.
build-ui:
	cd ui && pnpm install --frozen-lockfile && pnpm build

# Rust-side checks. Each is a thin wrapper around cargo so that the
# local Makefile and the CI workflow call identical commands.
fmt:
	cargo fmt --all -- --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

# Supply-chain audit. See docs/adr/0001-supply-chain-monitoring.md
# for the policy. Two flavors:
#   - audit: full tree (dev + prod deps) -- what the supply-chain CI
#     job runs. Catches dev-toolchain vulns (e.g. vite dev server RCE).
#   - audit-prod: prod-only deps. Catches what we actually ship. Less
#     noisy but misses dev-only vulns.
#
# The supply-chain CI job runs `audit` (full tree), as required by
# ADR-0001.
audit:
	cd ui && pnpm audit --audit-level=moderate
	cargo deny check advisories

audit-prod:
	cd ui && pnpm audit --prod --audit-level=moderate
	cargo deny check advisories

# The full local pipeline. `make ci` is the command new contributors
# should run before opening a PR; it's also the contract that the
# CI workflow validates.
ci: build-ui fmt clippy test audit

# Convenience: nuke the build artifacts. Useful when switching branches
# or debugging stale build script state.
clean:
	cargo clean
	rm -rf ui/dist ui/node_modules
