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

# Force bash for every recipe. Default /bin/sh on Debian/Ubuntu is dash,
# which lacks `[[` and other bashisms. Several targets below use
# `[[ ... ]]` and bash conditionals.
SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c

.PHONY: help build-ui fmt clippy test audit audit-prod audit-binary ci clean

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
	@echo "  make audit-binary  release-build + scan binary for build-time-only crate symbols"
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

# Runtime-binary check: assert that the 18 currently-ignored
# advisories stay out of the release binary's symbol table. This is
# defense-in-depth: even if a new Tauri release accidentally moves
# one of these crates from build-time to runtime, this check will
# fail.
#
# We check the actual release binary (after strip), not the cargo
# tree, because:
#   - cargo tree includes proc-macro and build-time deps in the
#     "normal" tree, which would always trigger false positives.
#   - We care about what's actually linkable to user input, which
#     is what ends up in the binary.
#
# The release profile in Cargo.toml has `strip = true`, so symbols
# are gone from the final binary. The CI check builds an UNSTRIPPED
# release binary and greps for crate names. See ci.yml.
#
# Notes on the blocklist:
#   - dom_query: build-time only (icon extraction at `tauri build`).
#     MUST NOT be in the binary.
#   - quick-xml: build-time only (macOS plist writer). MUST NOT be
#     in the binary.
#   - urlpattern: IS in the runtime binary on Tauri 2.x today, but
#     only at app startup to parse URL patterns from our own
#     tauri.conf.json -- not user-input-reachable. We do NOT
#     blocklist it here; it's covered by the threat model in
#     ADR-0001.
#   - unic-*: not in the binary (inlined or dead-code-eliminated).
#     We do NOT blocklist it for the same reason.
#   - The GTK3-rs chain (atk, gdk, gtk, etc.) IS in the binary on
#     Linux because webkit2gtk-sys loads it. We do NOT blocklist
#     it because the 11 ignores are unmaintained warnings, not
#     runtime-reachable vulnerabilities (see ADR-0001).
#
# If a deny.toml `ignore` entry is removed (e.g. the upstream fix
# landed), drop the corresponding line from the blocklist below.
# If a new ignore is added for a build-time-only crate, add a line.
audit-binary:
	@echo "audit-binary: building unstripped release binary and scanning symbol table"
	@cd app && \
	  cargo build --release --bin talon --config 'profile.release.strip=false' 2>&1 | tail -3
	@BINARY=$$(find target/release -maxdepth 1 -name 'talon' -type f -executable | head -1) && \
	  if [[ -z "$$BINARY" ]]; then \
	    echo "ERROR: release binary not found at $$(pwd)/target/release/talon"; \
	    echo "Hint: the binary may be at a different path. Check target/ manually."; \
	    exit 1; \
	  fi && \
	  echo "audit-binary: scanning $$BINARY ($$(stat -c%s $$BINARY) bytes)" && \
	  BLOCKLIST='dom_query quick_xml quick-xml' && \
	  HITS=$$(nm "$$BINARY" 2>/dev/null | grep -E "$$(echo $$BLOCKLIST | tr ' ' '|')" | head -20 || true) && \
	  if [[ -n "$$HITS" ]]; then \
	    echo "FAIL: the following deny.toml-ignored crates have symbols in the release binary:"; \
	    echo "$$HITS"; \
	    echo ""; \
	    echo "This means a Tauri change has moved a build-time-only crate to runtime."; \
	    echo "Either (a) the threat model has changed and we need to revisit the ignores,"; \
	    echo "or (b) the new Tauri version pulls the crate as a runtime dep, which"; \
	    echo "warrants a follow-up PR. See docs/adr/0001-supply-chain-monitoring.md."; \
	    exit 1; \
	  fi && \
	  echo "audit-binary: ok (no denied crates have symbols in the release binary)"

# The full local pipeline. `make ci` is the command new contributors
# should run before opening a PR; it's also the contract that the
# CI workflow validates.
ci: build-ui fmt clippy test audit

# Convenience: nuke the build artifacts. Useful when switching branches
# or debugging stale build script state.
clean:
	cargo clean
	rm -rf ui/dist ui/node_modules
