# Contributing to Talon

Thanks for your interest in contributing. Talon is a single-maintainer solo project right now, so the contribution process is intentionally lightweight — but the technical bar is the same as a larger project.

## Before you start

- **Open an issue first** for non-trivial changes. A change is non-trivial if it's > 100 LOC altered (excluding tests), or changes user-facing behavior, or adds a new dependency. Use the issue to confirm the design fits the project direction.
- **For bug fixes and small improvements** (typos, missing test cases, refactors with no behavior change), skip the issue and open a PR directly.
- **For new Tauri commands or new domain types**, the issue is mandatory — these touch the public IPC surface and the type system. The ADR-style writeup helps avoid wasted work.

## Development setup

See [`docs/developer.md`](docs/developer.md) for the full setup: toolchain, building, running tests, and the inner dev loop. The TL;DR is `make ci` runs the same pipeline CI runs.

## Coding style

The style is enforced by `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `pnpm lint`. There is no separate style guide. Run them before opening a PR:

```bash
make fmt      # cargo fmt --all -- --check
make clippy   # cargo clippy --workspace --all-targets -- -D warnings
pnpm lint     # pnpm lint (UI)
```

When you add a new lint suppression (a `#[allow(...)]` or an `eslint-disable`), leave a one-line `// reason:` comment. Reviewers push back on blanket suppressions.

## Tests

- All new code should have tests. The CI `Rust (ubuntu-latest)` and `UI` jobs run the test suite; if either is red, the PR is blocked.
- For Rust: put unit tests in the same file as the code (the `mod tests { ... }` pattern), integration tests in a `tests/` sibling. The workspace already has 254 passing Rust tests; the bar is that count only goes up.
- For UI: put vitest cases in a `*.test.tsx` sibling to the component. The bar is the same — the vitest count only goes up.
- For the supply-chain audit (`Supply-chain audit` CI job): if you add a new dep, `make audit` should pass. If it doesn't, follow the [supply-chain ADR](docs/adr/0001-supply-chain-monitoring.md) before adding a new `ignore` entry.

## Pull request process

1. Branch off `main` with a descriptive name: `fix/<thing>`, `feat/<thing>`, `docs/<thing>`, `chore/<thing>`. The `infinite-loop-dev` skill (when used by the maintainer's agent) uses `feat/<n>-<slug>`.
2. Commit messages follow the [Conventional Commits](https://www.conventionalcommits.org/) spec — `feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`, `build:`, `ci:`. The `feat:` and `fix:` prefixes trigger a release-please PR on the next push to `main`.
3. The PR description should explain **what** changed and **why** (not just the diff). If the change is a deviation from a documented plan, call that out in the PR body.
4. All 5 CI jobs must be green: `UI`, `Rust (ubuntu-latest)`, `Rust (macos-latest)`, `Rust (windows-latest)`, `Supply-chain audit`. If a CI job is failing for an unrelated reason, fix the unrelated thing or wait for it to recover — don't merge with a red check.
5. **Do not push directly to `main`.** Always go through a PR. Direct pushes bypass the 5-gate check.
6. The maintainer merges PRs. For trusted contributors' PRs, the maintainer may enable `auto-merge` on the PR after review; for everyone else, the maintainer does the squash-merge manually.

## Design documents

Design decisions live in [`docs/adr/`](docs/adr/) as ADRs (Architecture Decision Records). If your change is non-trivial, consider whether the ADR needs a new entry or an update to an existing one. The bar for adding an ADR is "will future-me wonder why we did it this way?" If yes, write the ADR.

The current ADRs:

- `0001-supply-chain-monitoring.md` — the `cargo-deny` + `pnpm audit` policy and the build-time-only crate isolation

## Release process

Talon uses [release-please](https://github.com/googleapis/release-please) for releases. You do not need to cut a release manually — every `feat:` or `fix:` PR that lands on `main` triggers a release-please PR, which the maintainer merges when ready. The release is cut by the tag push, and the `release.yml` workflow builds the binaries. See [`docs/release-process.md`](docs/release-process.md) for the full lifecycle (including the manual-cut recovery recipe for when the `linked-versions` plugin drifts).

## Security

If you find a security issue, **do not open a public issue**. Email the maintainer directly (see the GitHub profile). For non-security bugs, open a public issue.

## AI policy

Talon is a small project maintained by one person, with help from an AI coding agent (Hermes). AI assistance is welcome for:

- **Writing tests.** Vitest / `cargo test` are deterministic; AI-generated tests are fine as long as you can explain what each one is checking.
- **Boilerplate.** Type definitions, Tauri command skeletons, ADR templates.
- **Review.** AI code review (e.g. GitHub Copilot) is encouraged — Copilot catches real bugs (see the §3.5b PR review on `bk-mcp` for a load-bearing example).

AI assistance is **not** welcome for:

- **PRs you don't understand.** If you can't explain what the change does, don't submit it.
- **Comments that only help the AI interact with the code.** Comments that explain what straightforward code does are not useful and should be removed.
- **Drive-by AI reviews on other people's PRs** without the intention to follow up. If you invoke an AI review, be ready to address its findings.

If you used AI to write part or all of a PR, say so in the PR description ("This PR was written with help from [tool]. I reviewed every line and the [X] test that I can't explain is the part I'm least sure about."). Transparency helps the reviewer calibrate.

## License

By contributing, you agree that your contributions will be licensed under the [Apache-2.0](LICENSE) license, the same as the rest of the project.

## Communication

- **GitHub issues** for bugs, feature requests, and design discussion.
- **GitHub PRs** for code review.
- The maintainer is one person. Expect a few days of latency on issue triage.
