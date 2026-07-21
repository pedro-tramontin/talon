# Dependency upgrade plan

This plan captures the scope and rationale of the current and upcoming dependency upgrades for Talon. It is **not** a roadmap of features — see the README for the phase plan. This file only tracks *which dependency bumps go into which PR* and why.

The principle: **never mix a dep upgrade with a feature change.** Each dep-bump PR is reviewable on its own, can be reverted independently, and stays small enough that if CI catches a regression we know exactly what caused it.

## Status

- **Phase 1 (this PR — `phase-01-skeleton`)**: safe minor + major-bumps-that-don't-need-config-changes. See section "Phase 1" below.
- **Phase 2 (next PR)**: major versions that need config-file or code changes (Tailwind 4 config rewrite, etc.).
- **Phase 3+**: monitored by Renovate + the quarterly GTK4/unic watchdog. No manual bumps unless something goes wrong.

## Why split the work

The 13-day-old PR was authored against Tauri 2.1. The lockfile resolved Tauri 2.11.5. UI deps that *match* Tauri 2.11's official `create-tauri-app` template are safe to bump in this PR. The few that need *config changes* (e.g. Tailwind 4's CSS-first config model) belong in their own PR so a config regression can't hide inside a "dep bump" commit.

## Phase 1 — bumps landed in `phase-01-skeleton`

All of these are confirmed compatible with **Tauri 2.11.5 (Rust) + @tauri-apps/api 2.11.x (JS)**, the version the lockfile resolves to.

### Runtime deps (`ui/package.json` `dependencies`)

- `@tauri-apps/api`: `^2.0.0` → `^2.11.1` — **must match the Rust tauri minor**, otherwise the IPC contract drifts. Safe.
- `react`: `^18.3.1` → `^19.1.0` — Tauri official template pin. React 18→19 is a major, but the PR's UI is a single component (one `<App />`, one test). No legacy patterns to migrate.
- `react-dom`: `^18.3.1` → `^19.1.0` — follows react.
- `zustand`: `^4.5.0` → `^5.0.14` — hard peer is `use-sync-external-store >= 1.2.0` (added below) + `react >= 18`. Safe.
- `use-sync-external-store`: **new dep**, `^1.2.0` — pulled in by zustand 5.

### Dev deps (`ui/package.json` `devDependencies`)

- `@testing-library/jest-dom`: `^6.5.0` → `^6.9.1` — patch range, safe.
- `@testing-library/react`: `^16.0.0` → `^16.3.2` — minor, safe.
- `@types/react`: `^18.3.0` → `^19.1.8` — follows React major.
- `@types/react-dom`: `^18.3.0` → `^19.1.6` — follows React major.
- `@vitejs/plugin-react`: `^4.3.4` → `^6.0.2` — **requires Vite 8** (hard peer), so bumped together.
- `eslint`: `^9.0.0` → `^9.36.0` — stay on 9.x line, defer 10 to a later PR.
- `jsdom`: `^25.0.0` → `^25.0.0` — **unchanged**. Defer 25→29 to a later PR (4 majors is a lot to bundle).
- `postcss`: `^8.4.45` → `^8.4.45` — **unchanged**. Tailwind 4 won't use it, so this becomes irrelevant if/when we move to Tailwind 4.
- `tailwindcss`: `^3.4.10` → `^3.4.10` — **unchanged** (Phase 2).
- `typescript`: `^5.5.0` → `^5.6.0` — stay on 5.x line, defer 5→7 to a later PR.
- `vite`: `^6.4.3` → `^8.0.16` — Tauri official template pin. Bumped from the previous 6.x (which was itself a bump for the 5→6 advisory fix in commit `f4af921`).
- `vitest`: `^3.2.6` → `^4.1.10` — explicitly supports Vite 6/7/8.

### What stays on the previous version (intentional, not forgotten)

- **Node engine `>=20.0.0`** — fine; Node 22 is current LTS.
- **Rust toolchain pinned to stable via `dtolnay/rust-toolchain@stable` in CI** — no action.
- **`cargo-deny` ignores** in `deny.toml` — unchanged. The 18 upstream-blocked advisories are still upstream-blocked.

## Phase 2 — bumps deferred to a follow-up PR

These are major-version bumps that need **config-file or code changes** in the repo, not just `package.json` edits. They get their own PR so a config regression can't get hidden in a "bump deps" diff.

> **Update (2026-07-15):** PR #9 (`phase-02-dep-cleanup`) folds the three
> non-breaking majors below into one PR with three commits
> (`113c30e` jsdom 29 → `4a98285` eslint 10 → `b2b5762` tailwind 4). The fourth
> one, **TypeScript 5 → 7**, is blocked upstream and is **not** in #9:
> `typescript-eslint@8` reads `typescript.module.Cjs` which TS 7 removed, so
> the eslint flat-config parser crashes. There is no `typescript-eslint@9`
> published (canary is `8.64.1-alpha.3`, still 8.x). A `globals.css.d.ts`
> shim fixes `tsc -b` but the crash is in `typescript-estree`, so the shim
> does not help lint. Re-attempt when `typescript-eslint@9` ships.

### Tailwind 3 → 4

- **What changes**: CSS-first config model. `tailwind.config.js` (with `content: [...]`, `theme.extend`, etc.) is replaced by `@theme {}` blocks inside the CSS file. `postcss.config.js` is replaced by a Vite plugin or `@tailwindcss/postcss`.
- **Files affected**:
  - `ui/tailwind.config.js` — rewrite or delete
  - `ui/postcss.config.js` — rewrite
  - `ui/src/styles/globals.css` — change `@tailwind base/components/utilities` to `@import "tailwindcss"`
  - `ui/vite.config.ts` — add `@tailwindcss/vite` plugin
  - Any custom theme tokens currently in `tailwind.config.js` (need to be moved to CSS)
- **Why defer**: easy to introduce silent visual regressions that look fine in tests but ship broken layout. Needs eyes-on-the-screenshot review, not just CI.

### TypeScript 5 → 7

- **What changes**: TS 7 tightened a few defaults; some patterns that were warnings in 5 are errors in 7. May need small tweaks to `tsconfig.json` (e.g. stricter `noUncheckedIndexedAccess`).
- **Files affected**: `ui/tsconfig.json`, possibly a few `.tsx` files.
- **Why defer**: clean to do in a standalone PR with its own `pnpm tsc --noEmit` baseline.

### ESLint 9 → 10

- **What changes**: flat config may need tweaks. Some plugin APIs changed.
- **Files affected**: any `eslint.config.*` if present (none in this PR yet), possibly `.eslintrc*`.
- **Why defer**: project doesn't have a custom eslint config yet (uses defaults), so this is mostly a low-risk bump. Defer because the PR scope should stay focused.

### jsdom 25 → 29

- **What changes**: jsdom 26–29 each had internal API cleanups. Test code that touches DOM internals may need small adjustments.
- **Files affected**: `ui/src/test-setup.ts` (if it does), and possibly test files.
- **Why defer**: the PR's only test renders `<App />` and asserts a string. No DOM internals touched. But still worth its own PR for the diff to stay small.

### pnpm 9 → 11

- **What changes**: pnpm 10 changed `node_modules` layout slightly and deprecated some lifecycle script defaults. pnpm 11 is current.
- **Files affected**: `package.json` (`packageManager` field), possibly `.npmrc`.
- **Why defer**: this is a toolchain change, not a dep change. Should land with a fresh `pnpm install` and a CI re-run to confirm nothing breaks.

> **Update (2026-07-15):** pnpm 9.12.3 → 11.13.0 was pulled into Phase 1 of the
> skeleton PR. npm retired the legacy `pnpm audit` endpoint in 2026, and pnpm's
> replacement is only in 11.x. Without this bump, the supply-chain CI job fails
> with `ERR_PNPM_AUDIT_BAD_RESPONSE`. The bump is safe: `pnpm 11.13.0` reads
> the pnpm-9 lockfile natively (lockfileVersion 9.0 still works), all 5 UI
> jobs (install / test / build / audit) pass on 11.13.0, and the
> `pnpm install --frozen-lockfile` step in CI works without regenerating
> the lockfile. The transitive patch bumps that pnpm 11 picks up
> (`autoprefixer 10.4.20 → 10.5.x`, `postcss 8.4.45 → 8.5.x`,
> `eslint 9.36.0 → 9.39.x`) are within the major ranges already pinned.
> CI workflow `.github/workflows/ci.yml` updated: `version: 9` → `version: 11`
> in both the `ui` and `supply-chain` jobs.
>
> **Cascading bump: Node 20 → 22.** pnpm 11.x's `engines` field requires
> `node >= 22.13.0`. The CI workflow pinned `node-version: 20`, so the
> `pnpm install` step failed with `ERR_UNKNOWN_BUILTIN_MODULE: node:sqlite`
> (pnpm 11 uses sqlite for store metadata, which Node 20 doesn't ship).
> Fix: bump `node-version: 20` → `node-version: 22` (current LTS) in both
> the `ui` and `supply-chain` jobs, and update the `engines.node` /
> `engines.pnpm` fields in `ui/package.json` to match. Node 22 also
> silences the upstream deprecation warning that GitHub's runners have
> been printing ("Node 20 is being deprecated").

## Phase 3+ — Renovate + the watchdog

Once `phase-01-skeleton` lands:

- **Renovate** (configured in `renovate.json5` at the repo root) opens weekly PRs for new npm, cargo, and GitHub Actions versions. PRs are grouped by ecosystem cluster (vite-stack, react-stack, tauri-stack, async-stack, serde-stack) so a one-line `Cargo.toml` change is one reviewable PR, not eight. Major-version bumps for `vite` and `vitest` are suppressed so we drive those migrations explicitly (vite 5→6, vitest 2→3 were manual last time). Reviewers: `pedro-tramontin`.
- **The `talon-gtk4-watchdog` cron** (quarterly) checks the Tauri and `urlpattern` release pages; when a release mentions GTK4 or `unic-*` migration, it alerts. When that fires, we can remove the relevant `ignore` entries from `deny.toml` (per ADR-0001).
- **No scheduled manual dep sweeps**. Renovate + the watchdog cover steady-state.

## What to NOT bundle in a dep PR

- New Tauri plugins — those belong with the feature they enable.
- New Rust deps in `bk-store`, `bk-proxy`, `bk-fuzzer`, `bk-server` — those are Phase 2+ workspace additions.
- Changes to the `gen/schemas/acl-manifests.json` (the auto-generated Tauri ACL file) — that regenerates from `tauri-build` on every build. Don't touch by hand.

## Verification checklist (every dep-bump PR)

Before opening or merging any dep-bump PR, run locally:

```bash
make fmt
make clippy
make test
make audit            # pnpm audit + cargo deny check advisories
make audit-prod       # production-only audit
make build-ui         # make sure the UI builds clean
make ci               # the whole pipeline
```

And on the PR itself:

- All 5 CI jobs green (3× Rust matrix + UI + supply-chain audit).
- No new `cargo-deny` ignores added.
- No new entries in `pnpm audit` (or, if there are, address them in the same PR).

## Tracking

- Phase 1 implementation: PR `phase-01-skeleton`, commit message convention `chore(deps): ...`
- Phase 2: will be opened as a new PR after Phase 1 lands
- Phase 3: Renovate will create PRs automatically (see `renovate.json5`)
