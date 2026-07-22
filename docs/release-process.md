# Release process

This document describes how a Talon release is built, signed, and published. It is the human-readable companion to the three GitHub Actions workflows that do the actual work:

- [`.github/workflows/release-please.yml`](../.github/workflows/release-please.yml) — opens and updates the release PR; creates the git tag and the GitHub Release.
- [`.github/workflows/draft-build.yml`](../.github/workflows/draft-build.yml) — builds **debug-mode** Tauri bundles for every `vX.Y.Z-pre.N` pre-release tag and attaches them to the pre-release.
- [`.github/workflows/release.yml`](../.github/workflows/release.yml) — builds **release-mode** Tauri bundles for every stable `vX.Y.Z` tag and attaches them to the published release.

If you are about to cut a release for the first time, read **§1 (prerequisites)** and **§2 (cutting a release)** end-to-end before pushing the green button.

---

## 1. Prerequisites

### 1.1 Repository secrets

Configure these in **Settings → Secrets and variables → Actions**. The list below maps each secret to the workflow that uses it.

| Secret | Used by | Required? | Notes |
|---|---|---|---|
| `RELEASE_PLEASE_TOKEN` | `release-please.yml` | **Yes** | A fine-grained PAT scoped to the talon repo with `Contents: Read and write` and `Pull requests: Read and write`. Do NOT use the default `GITHUB_TOKEN` — GitHub intentionally blocks the default token from triggering downstream workflows, which means a tag push won't fire `release.yml`. This is the bug that shipped v0.1.0 of vaultenv with zero assets. |
| `GITHUB_TOKEN` | `release.yml` (each OS job) | Auto | The default per-run token. tauri-action uses it with `contents: write` to attach bundles to the GitHub Release. No setup needed. |
| `APPLE_CERTIFICATE` | `release.yml` (macOS job) | No (optional) | Base64-encoded `.p12` Developer ID Application certificate. **Without this, the macOS `.dmg` is unsigned** (the unsigned build path runs — see §4.3a for the prior failure mode where an empty value made the whole job fail). Recommended for public releases. |
| `APPLE_CERTIFICATE_PASSWORD` | `release.yml` (macOS job) | Only if `APPLE_CERTIFICATE` set | Password for the `.p12`. |
| `APPLE_SIGNING_IDENTITY` | `release.yml` (macOS job) | Only if `APPLE_CERTIFICATE` set | `Developer ID Application: <Your Name> (<TEAM_ID>)`. |
| `APPLE_ID` | `release.yml` (macOS job) | Only if `APPLE_CERTIFICATE` set | Apple ID email for notarisation. |
| `APPLE_PASSWORD` | `release.yml` (macOS job) | Only if `APPLE_CERTIFICATE` set | App-specific password (not your Apple ID password). |
| `APPLE_TEAM_ID` | `release.yml` (macOS job) | Only if `APPLE_CERTIFICATE` set | 10-character Apple Developer Team ID. |
| `KEYCHAIN_PASSWORD` | `release.yml` (macOS job, signed path) | Only if `APPLE_CERTIFICATE` set | Arbitrary password used to lock the temporary `build.keychain` that holds the imported `.p12` for the duration of the job. Any value works — pick something long but not the same as the `.p12` password. |

The Windows code-signing certificate (`.pfx`) is **not** yet wired in — Talon is unsigned on Windows. The artifact is installable, but SmartScreen will warn on first launch. Adding the cert is a follow-up tracked separately.

### 1.2 Renovate GitHub App

The Renovate GitHub App must be installed on the repo. Without it, `renovate.json5` is just a static file. Install via the [Renovate dashboard](https://app.renovatebot.com/) — the standard config picks up `renovate.json5` automatically.

### 1.3 Conventional commits

Every commit on `main` should follow [Conventional Commits](https://www.conventionalcommits.org/). The prefix determines what release-please bumps:

| Commit prefix | Bump | Example |
|---|---|---|
| `feat:` | **minor** (0.1.0 → 0.2.0) | `feat(bk-proxy): add HTTP/2 support` |
| `fix:` | **patch** (0.1.0 → 0.1.1) | `fix(bk-store): handle FTS5 query timeout` |
| `feat!:` or `BREAKING CHANGE:` footer | **major** (0.x → 1.0) | `feat(bk-agent)!: rename WRITE_TOOLS` |
| `chore:`, `docs:`, `refactor:`, `test:`, `style:`, `ci:` | **no release** | `chore(deps): bump tokio to 1.42` |
| `chore(deps):` (any prefix, any scope) | **never** a release (Renovate handles dep bumps) | |

`feat:` without a scope also triggers a minor bump. If a `feat:` is not user-facing (e.g. internal refactor that you want in the changelog but not as a release), use `chore:` instead.

---

## 2. Cutting a release

The end-to-end flow is push-button from the developer's perspective. release-please runs on every push to `main`, so all you do is merge PRs.

```
1. dev merges PRs to main (each commit follows Conventional Commits)
2. release-please opens or updates a release PR titled
   "chore(main): release <NEW_VERSION>"
   The PR body lists the changelog entries and a release notes preview.
   Files it touches:
     - Cargo.toml ([workspace.package] version)
     - app/tauri.conf.json ("version")   ← synced via extra-files
     - ui/package.json ("version")       ← synced via extra-files
     - CHANGELOG.md
3. Reviewer (you) sanity-checks:
     - Is the version bump correct? (feat → minor, fix → patch, etc.)
     - Is the changelog list complete? (release-please picks up only
       conventional-commits that match the "release-worthy" types)
     - Are the three version files in lockstep? (Cargo.toml = tauri =
       package.json — they should all show the same new version)
4. Reviewer merges the release PR
5. release-please creates:
     - git tag v<X>.<Y>.<Z>
     - GitHub Release (draft) titled "Talon v<X>.<Y>.<Z>"
6. The tag push fires release.yml
7. release.yml builds 3 OS bundles in parallel:
     - Linux:   .deb + .AppImage
     - macOS:   .dmg (.app under the hood) — universal binary (Intel + Apple Silicon)
     - Windows: .msi + .exe
8. tauri-action uploads each bundle to the GitHub Release as it finishes
9. The `finalize` job publishes the draft
10. The release is live at
    https://github.com/pedro-tramontin/talon/releases/tag/v<X>.<Y>.<Z>
```

A first-cut release typically takes **45–60 minutes** end-to-end (release-please PR + review + merge + 3-OS build, dominated by the macOS universal-binary build).

### 2.1 Pre-release checklist

Before merging the release-please PR:

- [ ] CI is green on `main` (all 5 jobs: 3× Rust matrix + UI + supply-chain).
- [ ] No unmerged Renovate PRs that should land in this release.
  - If a Renovate PR is sitting open and you don't want to wait for it, merge it before the release PR (release-please will pick it up on the next push).
  - If a Renovate PR is open and you want to include it, merge it first; the next release-please run will incorporate it.
- [ ] No `feat:` or `fix:` commits in the changelog that don't match the code in the release. (release-please is correct, but a quick visual check catches "feat:" typos that would otherwise land as silent patch bumps.)
- [ ] If macOS is being signed, the signing secrets are still valid (cert hasn't expired). `tauri-action` will fail with a clear error if the cert is bad.

### 2.2 When the release is published

- The release body is auto-generated from the CHANGELOG.md entry for that version.
- Each bundle has a SHA-256 next to it (GitHub's UI computes it for the asset). The README's install instructions point at the latest release.

### 2.3 Daily flow with pre-releases (the typical path)

With `prerelease: true` in `release-please-config.json`, the default flow is the pre-release path described in §6. The day-to-day is:

```
1. Merge PRs to main. release-please accumulates them into a
   pre-release PR titled "chore(main): release X.Y.Z-pre.N".
2. Merge the pre-release PR. The tag vX.Y.Z-pre.N fires
   draft-build.yml. Debug bundles attach to the pre-release page
   (~20-30 min).
3. Smoke-test the pre-release bundles (or share the URL with
   collaborators for QA).
4. When ready to ship, push a `chore: promote vX.Y.Z` empty
   commit. release-please auto-promotes the pre-release to a
   stable tag vX.Y.Z, which fires release.yml for the prod build
   (~45-60 min).
```

§6 has the full detail on the pre-release path, the promotion mechanic, the cleanup story, and the re-run recovery. Read §6 once before you do your first promotion.
- A draft `.deb` and `.AppImage` build for **both** `x86_64` and `aarch64` Linux is **not** included in the first-cut pipeline. The current Linux job builds on `ubuntu-22.04` (x86_64 only); aarch64 Linux is a follow-up. Track this separately.

---

## 3. What lives where

Talon is a multi-crate Cargo workspace (8 crates: `bk-core`, `bk-store`, `bk-engine`, `bk-proxy`, `bk-mcp`, `bk-agent`, `bk-events`, `app`) **plus** a Tauri config file and a Node `package.json`, all of which carry a `version` field. release-please needs to keep all 10 of these in lockstep.

| File | Purpose |
|---|---|---|
| `release-please-config.json` | 8 `packages` entries, one per crate, each with its `component` name and `release-type: "rust"`. The `linked-versions` plugin (groupName `talon`, all 8 component names) keeps all crates on a single version line — when any one is bumped, the highest version is picked and applied to all 8 in the same release PR. The `extra-files` rule mirrors the unified version into `app/tauri.conf.json` and `ui/package.json` after the bump. **`prerelease: true` + `prerelease-type: "pre"`** enable the draft-release flow (see §6): every push to `main` produces a `vX.Y.Z-pre.N` pre-release tag instead of going straight to `vX.Y.Z`. The pre-release is auto-promoted to a stable release on the next no-bump commit. |
| `.release-please-manifest.json` | 8 entries (one per crate) at `0.1.0`. Updated on every release PR. |
| `crates/*/Cargo.toml` and `app/Cargo.toml` | Each has `version = "0.1.0"` as a **literal string** (not `version.workspace = true`). Inlined manually because release-please can't update workspace-inherited versions — see §3 "Why this shape" below. |
| `Cargo.toml` `[workspace.package] version` | Still present, still a literal `0.1.0`. The `edition`, `rust-version`, `license`, `repository`, `authors` fields in this block are still inherited by all 8 crates via `*.workspace = true` (release-please doesn't try to update those, so the inheritance pattern works for them). The `version` field here is the "default" that new crates would inherit if any future crate reintroduces the pattern, but in practice every crate now inlines its own. |
| `app/tauri.conf.json` `"version"` | Mirrored by release-please's `extra-files`. The bundle metadata at build time is stamped from the tag (not from this field), so a one-commit drift here is harmless but ugly. |
| `ui/package.json` `"version"` | Same as `app/tauri.conf.json`. Mirrored for in-UI display and to keep `pnpm version` honest. |
| `CHANGELOG.md` | Regenerated by release-please on every release. Do NOT edit by hand — release-please will overwrite it on the next release PR. |
| `.github/workflows/release-please.yml` | Runs release-please on every push to `main`. Uses `RELEASE_PLEASE_TOKEN` (not the default token). |
| `.github/workflows/draft-build.yml` | **Draft-release workflow.** Tag-triggered on `v*-pre.*`. Builds debug-mode Tauri bundles for all 3 OSes in parallel and attaches them to the pre-release created by release-please. macOS is unsigned (no `APPLE_*` env var forwarding). See §6. |
| `.github/workflows/release.yml` | **Stable-release workflow.** Tag-triggered on `v[0-9]+.[0-9]+.[0-9]+` (regex intentionally excludes `-pre.N` suffixes). Builds release-mode Tauri bundles for all 3 OSes in parallel and attaches them to the published release. macOS is signed if `APPLE_CERTIFICATE` is configured, unsigned otherwise. Has a `finalize` job that publishes the draft. |
| `renovate.json5` | Weekly version-PRs for deps + GitHub Actions. Unrelated to release-please. |

### Why this shape (and the inline-`version` choice)

The **natural** design for a Cargo workspace is to have a single `[workspace.package] version = "0.1.0"` at the root, and every member crate inherit via `version.workspace = true`. This is the canonical Rust 1.64+ workspace pattern.

**release-please cannot handle that pattern.** Both the `cargo-workspace` plugin and the bare `release-type: rust` strategy iterate the workspace members and call `replaceTomlValue` to update `package.version` in each. The `replaceTomlValue` implementation (in `src/util/toml-edit.ts:152`) requires the value to be a "tagged" leaf — i.e., a string. But iarna's TOML parser represents `version.workspace = true` as `{ workspace: true }` — an **object** — not a string. So `replaceTomlValue` throws `value at path package.version is not tagged`.

This is a [known and open issue](https://github.com/googleapis/release-please/issues/2111) (filed Nov 2023, priority p3, labeled "help wanted", still unresolved as of 2026-07). The maintainers aren't actively fixing it. The practical workaround in the issue thread is "inline the version in each crate" — which is what talon does.

**Trade-off:** the canonical workspace inheritance pattern is no longer used for `version`. Every release bumps 8 literal `version = "X.Y.Z"` lines instead of 1 + 8 inheritances. release-please's `linked-versions` plugin does the syncing automatically; no manual effort at release time. The other 5 inherited fields (`edition`, `rust-version`, `license`, `repository`, `authors`) **still use** the inheritance pattern, because release-please doesn't try to update them.

**Future revert path:** if/when release-please fixes issue #2111, the inline `version` lines can be removed and `version.workspace = true` re-introduced in all 8 crates. The release flow needs no other changes.

---

## 4. Troubleshooting

### 4.1 release-please didn't open a release PR

Causes (most → least likely):

1. **No `feat:` / `fix:` / breaking-change commits since the last release.** This is correct behavior — release-please only opens a release PR when there's a bump to do. Wait for a release-worthy commit.
2. **Wrong branch.** release-please only watches `main`. If you merged a release-worthy commit to a feature branch, it won't trigger.
3. **The `RELEASE_PLEASE_TOKEN` secret is missing or expired.** The release-please workflow will fail with `403 Forbidden` on the API call. Rotate the PAT in Settings → Secrets.

### 4.1a `value at path package.version is not tagged`

This was the failure mode on the very first release-please run for talon (2026-07-21). It means release-please scanned the Cargo workspace, found 8 crates with `[package] version` fields, and tried to look up each one in `.release-please-manifest.json` — but the manifest only had a single root key (`".": "0.1.0"`) instead of one key per crate.

**Root cause:** the original `release-please-config.json` declared a single `package` (the root) with `release-type: rust`, which assumes a single-crate repo. The Cargo workspace has 8 member crates, and release-please's rust releaser discovers each one and wants to release it on its own line. The single-package config didn't declare them, so the manifest didn't track them, and the run bailed.

**Intermediate fix (later superseded, see §3 "Why this shape"):** the config was rewritten to list all 8 crates as separate `packages` entries (one per `crates/<name>` path, each with a `component: <name>` and `release-type: "rust"`) plus two plugins (`cargo-workspace` with `merge: false` and `linked-versions` with `groupName: "talon"`). The manifest grew to 8 entries (one per crate) all at `0.1.0`.

**Final fix (current design, see §3):** dropped the per-crate packages and the `cargo-workspace` plugin entirely (see §4.1c for why). The config is now a single root package, and the 8 per-crate `Cargo.toml` files are not managed by release-please at all — they inherit their version from `[workspace.package] version` via `version.workspace = true`.

If you ever see this error again, the symptom is **multiple packages declared in the config** but **the manifest is missing entries** for one of them. Either add the missing key to the manifest, or drop the per-package entries from the config and let the root-only pattern handle it.

### 4.1b `node (pedro-tramontin/talon): Missing required file: crates/<crate>/package.json`

This was the failure mode on the second release-please run for talon (2026-07-21, immediately after #4.1a's fix). It means release-please's per-package releaser defaulted to `node` and tried to read a `package.json` from each crate's path.

**Root cause:** the manifest-driven release-please config has a **default `release-type: node`** when no top-level or per-package `release-type` is set. The first fix (#4.1a) added the multi-package shape but didn't carry the top-level `release-type: rust` forward from the original config, so each of the 8 packages silently inherited the node default. release-please then looked for `crates/bk-core/package.json`, etc. — files that don't exist (those directories contain Rust crates, not Node packages).

**Fix:** added `"release-type": "rust"` at both the top level of `release-please-config.json` (as the default for any new package that doesn't override) and on every per-package entry (defense in depth — a Phase 5+ maintainer who adds `crates/bk-fuzzer` without setting a release-type will inherit the correct default).

If you ever see this error again, the symptom is **a new crate added to the workspace** with a per-package entry that doesn't set `"release-type": "rust"`. Fix: add the field to the new package's config block. (Or, the same fix works at the top level: a single `release-type` change applies to all packages.)

### 4.1c `cargo-workspace (...): package manifest at crates/<crate>/Cargo.toml has an invalid [package.version]`

This was the failure mode on the third release-please run for talon (2026-07-21, after the two previous fixes). The error message is misleading — the `[package.version]` field in each per-crate `Cargo.toml` is `version.workspace = true`, which the cargo-workspace plugin doesn't understand.

**Root cause:** the `cargo-workspace` plugin requires each member crate's `[package] version` to be a **literal string** (e.g. `version = "0.1.0"`), not a workspace inheritance (e.g. `version.workspace = true`). The plugin's source has an explicit type check (`src/plugins/cargo-workspace.ts`):

```ts
} else if (typeof version !== 'string') {
  throw new ConfigurationError(
    `package manifest at ${manifestPath} has an invalid [package.version]`,
    'cargo-workspace', ...
  );
}
```

Talon uses `version.workspace = true` in all 8 crates (the canonical Rust workspace pattern), so the plugin bails.

**Fix:** dropped the `cargo-workspace` plugin. The config now declares the **root** (`.`) as the only package, with `release-type: rust`. release-please's Rust strategy reads the root `Cargo.toml`'s `[workspace.package] version` (a literal string), computes the next version, and writes it back. The per-crate `Cargo.toml` files don't need to be touched because `version.workspace = true` automatically picks up the new value from the workspace root.

The trade-off: we lose the per-crate "patch-bump dependents" behaviour that `cargo-workspace` provides. For talon that's fine — all 8 crates are versioned in lockstep, and the `app` crate's path-based dependencies don't need version-aware bumping. If we ever need per-crate independent versioning (e.g. publishing `bk-store` to crates.io as its own crate), we'd need to switch to inlining the version in each crate's `Cargo.toml` (`version = "0.1.0"`) to make the plugin happy.

If you ever see this error again, the symptom is **a new crate added to the workspace** that uses `version.workspace = true` while the `cargo-workspace` plugin is enabled. Two fixes:
- If you want to keep workspace inheritance, the plugin must be dropped (we're doing this).
- If you need per-crate independent versioning, inline the version in every crate's `Cargo.toml` and keep the plugin.

### 4.2 release.yml didn't fire after a tag push

Almost always: the `RELEASE_PLEASE_TOKEN` PAT is the same as a normal user token without `workflow` write permission. The tag is created (release-please used the PAT's `Contents: write` scope), but the workflow doesn't fire because GitHub treats the push as coming from an "untrusted actor" with no workflow trigger rights.

Fix: regenerate the PAT with the additional `workflow` scope (fine-grained: `Actions: Read and write`), or use a GitHub App instead. After rotation, delete and recreate the tag — release-please won't recreate an existing tag.

```bash
# To retry without bumping versions: delete the tag, push, re-run the
# failed release-please job.
git push origin :refs/tags/v0.2.0
gh workflow run release-please.yml
```

### 4.2a `Error creating Pull Request: Resource not accessible by personal access token`

This was the failure mode on the FIFTH release-please run for talon (2026-07-21 15:00 UTC, run 29841900230, **5m22s** — the long duration is the tell that release-please did real work and then died at the final push). The workflow log shows release-please successfully built the release branch `release-please--branches--main` and tried to push it, but the API call to `POST /repos/pedro-tramontin/talon/git/refs` returned 403.

**Root cause:** the `RELEASE_PLEASE_TOKEN` PAT doesn't have `Contents: write` permission. release-please uses this scope to push the release branch and create the tag. The PAT was likely created with only `Pull requests: write` (or as a fine-grained PAT where the "Contents" permission defaulted to "No access").

**Fix:** update the PAT in https://github.com/settings/tokens (or https://github.com/settings/personal-access-tokens for fine-grained):
- **Fine-grained:** Edit the PAT → Repository access = `pedro-tramontin/talon` only → Permissions → "Contents" = "Read and write" + "Pull requests" = "Read and write" + "Metadata" = "Read-only" (required)
- **Classic:** Can't edit scopes — must regenerate. Ensure `repo` (or `public_repo` + `workflow`) scope is selected, plus any other scopes you originally needed.

After rotation, update the secret value in repo Settings → Secrets and variables → Actions. Then trigger a re-run:

```bash
gh run rerun 29841900230 --failed
```

(Or, equivalently, push a no-op commit to main and let the next release-please run pick it up.)

**Verification:** the next run should successfully open a release PR titled "chore(main): release 0.X.Y" listing the 8 crate version bumps + the tauri.conf.json + ui/package.json bumps + a CHANGELOG.md regeneration. The PR will be open, draft, and ready for review.

### 4.3 One OS build fails but the others succeed

`release.yml` is `fail-fast: false` in spirit — each OS job is independent and uploads its assets to the same draft Release as they finish. So if, say, the macOS build fails, you get a Release with the Linux .deb + Windows .msi but no .dmg.

Fix: address the underlying issue (renew the cert, fix the build error), then **re-run the failed job on the same tag** rather than deleting the tag and re-pushing. The new run will upload the missing bundle. The existing bundles will be **re-uploaded** with the same filenames — GitHub deduplicates by filename in the UI.

```bash
# Find the failed run for the tag
gh run list --workflow="Release" --json databaseId,conclusion,headBranch \
  --jq '.[] | select(.headBranch == "v0.2.0") | "\(.databaseId) \(.conclusion)"'

# Re-run just the macOS job from the failed run
gh run rerun <RUN_ID> --failed
```

The previous version of this section recommended `git push origin :refs/tags/v0.2.0 && git push origin v0.2.0`. **Do not do that** — release-please will not recreate an existing tag, and force-pushing tags can confuse downstream tooling. Re-running the failed job is the only correct recovery.

### 4.3a `SecKeychainItemImport: One or more parameters passed to a function were not valid` (macOS job fails, no .dmg ships)

This was the failure mode for **every release from v0.1.0 through v0.1.2**. The macOS job's final log line was always:

```
security: SecKeychainItemImport: One or more parameters passed to a function were not valid.
failed to bundle project: failed codesign application: failed to run command security import: failed to import keychain certificate
Error: Command "tauri ["build","--bundles","app,dmg","--target","universal-apple-darwin"]" failed with exit code 1
```

**Root cause:** the old `release.yml` unconditionally passed `APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}` to the macOS job. When the secret was **not configured** (which has been the case for every talon release so far), GitHub Actions substitutes an empty string. Tauri 2's macOS bundler reads the env var at bundle time, sees it's set, and calls `security import` on the empty value — which fails with the `SecKeychainItemImport` error above. The whole `tauri build` then aborts, so no `.app` or `.dmg` is produced. Meanwhile, release-please has already published the GitHub Release from the tag push, so the release ships with only the Linux + Windows assets.

**Fix (in `release.yml`):** the macOS job now has three mutually-exclusive steps:

- `Import Apple Developer Certificate` — runs only when `secrets.APPLE_CERTIFICATE != ''`; does the manual `security import` into a temporary `build.keychain`.
- `Build Tauri bundle (macOS, signed) — tauri-action` — runs only when the secret is present; invokes `tauri-action@v0` with all the `APPLE_*` env vars set.
- `Build Tauri bundle (macOS, unsigned)` — runs only when the secret is **absent**; invokes `tauri-action@v0` with no `APPLE_*` env vars at all, so Tauri 2's macOS bundler skips the cert-import path entirely and produces an unsigned `.app` + `.dmg`.

Result: macOS assets ship on **every** release, signed or not. Unsigned .dmg is the right shape for now — Apple Gatekeeper will warn on first launch ("unidentified developer"), but the app runs.

**Why two `if:`-gated steps instead of conditionally clearing env vars:** GitHub Actions evaluates a step's `if:` before its `env:` block takes effect, so `env.APPLE_CERTIFICATE != ''` is not a usable gate. The check has to be on `secrets.APPLE_CERTIFICATE` (the secret), and to keep the env block "all or nothing" you need two separate steps. That's why the unsigned tauri-action call is in its own step rather than conditionally unsetting vars on the signed step.

**Verification:** the next release (or a manual re-run of the v0.1.2 release workflow after this fix lands) should attach the `Talon_<version>_universal.dmg` and `Talon.app` to the release. Run `gh release view v0.X.Y --json assets --jq '.assets | map(.name)'` and check both filenames are present.

**Re-recovering the v0.1.2 release:** after this fix lands on `main`, you can either (a) cut v0.1.3 (which will have the fix automatically) or (b) re-run the failed v0.1.2 macOS job against the existing v0.1.2 tag — `gh run rerun 29858434755 --failed`. The new macOS run will upload the missing `.dmg` and `.app` to the existing v0.1.2 release.

### 4.4 The Release body is wrong or stale

The body is set by the first build job to finish (via tauri-action's `releaseBody:` input). Subsequent jobs don't overwrite it. If the body is wrong (e.g. a `feat:` was misclassified and the changelog is misleading), delete the release, fix the commit message, and cut a new release.

### 4.5 Renovate opened a PR for a dep that should be ignored

Add a carve-out to `renovate.json5` under `packageRules` with `matchUpdateTypes: ["major"]` or `matchPackageNames` as appropriate. The next Renovate scan (within `prHourlyLimit: 2`) will pick up the new rule.

### 4.6 `untagged, merged release PRs outstanding - aborting`

This was the failure mode that blocked all 4 release-please runs between **v0.1.3 (2026-07-21) and v0.2.0 (2026-07-22)**, leaving the Phase 5 (replay) + Phase 6 (scope/M&R) work on `main` without a user-installable build for ~30 hours. The release-please run log shows:

```
✔ Merging 1 pull requests
❯ Found pull request #60: 'chore: release main'
⚠ There are untagged, merged release PRs outstanding - aborting
```

**Root cause (two compounding issues):**

1. **The v0.1.3 tag was created via a workaround, not the standard flow.** The macOS-bundle fix in #61 + #62 took several iterations to land; while debugging, the `release-please--branches--main` branch was manually closed and the v0.1.3 tag was pushed by the `cf1d846 chore: retrigger release-please after cleaning up stale v0.1.3 tag` commit. That left release-please's internal PR-→-tag tracker in a half-state where PR #60 was merged but the bookkeeping thought its tag was missing.

2. **The `linked-versions` plugin in `release-please-config.json` had been silently failing for 2 releases.** The plugin's job is to keep all linked components on the same version line: any feat: commit in any linked crate should bump all 8 crates. It worked correctly for v0.1.1 (all 8 crates bumped in PR #56). For v0.1.2 (PR #58) it only bumped `app` + `bk-events`. For v0.1.3 (PR #60) it only bumped `app`. By v0.1.3, the manifest had 7 distinct versions across 8 entries (`crates/bk-{core,store,engine,proxy,mcp,agent}: 0.1.1`, `crates/bk-events: 0.1.2`, `app: 0.1.3`) — well past the "drift" threshold where the plugin gives up. Every release-please run after #60 saw this diverged manifest and aborted.

**Diagnostic (how to confirm this is the failure mode you're hitting):**

```bash
# Show the manifest (should have consistent versions if release-please is healthy):
cat .release-please-manifest.json | jq .

# Show the tag history (compare to manifest):
git show-ref --tags

# Show the latest release-please run's tail (look for "aborting"):
gh run list --workflow=release-please.yml --limit 1 --json databaseId \
  | jq -r '.[0].databaseId' \
  | xargs -I{} gh run view {} --log \
  | grep -E '(aborting|untagged|Found pull request)'
```

**Recovery (the manual v0.2.0 cut):**

The recovery is to do by hand what release-please would have done if it weren't broken — cut a `chore: release main` PR with all 8 crates + `app` at the same version. This is the "drop the `linked-versions` plugin" flow in practice.

```bash
# 1. Create a release branch off main:
git checkout -b chore/0.2.0-manual-release main

# 2. Bump all 9 version fields to 0.2.0:
#    - .release-please-manifest.json (all 8 entries)
#    - crates/*/Cargo.toml + app/Cargo.toml (9 inline versions)
#    - app/tauri.conf.json + ui/package.json (extra-files mirrors)
for f in crates/*/Cargo.toml app/Cargo.toml; do
  sed -i 's/^version = "0\.1\.[0-9]*"/version = "0.2.0"/' "$f"
done
# (then edit .release-please-manifest.json, app/tauri.conf.json,
#  ui/package.json by hand to the same 0.2.0)

# 3. Regenerate Cargo.lock to pick up the new version pins:
cargo update --workspace

# 4. Add per-crate CHANGELOG entries for crates that had user-facing
#    changes since their last changelog entry (matches what release-please
#    would generate). Crates without changes get a version bump but no
#    changelog entry. See PR #67 for the format.

# 5. Commit with the release-please standard title (so the next
#    release-please run sees it as a release-please PR):
git commit -am "chore: release main"

# 6. Open the PR (the body should match release-please's format):
gh pr create --title "chore: release main" --body-file /tmp/v0.2.0-body.md

# 7. After CI is green and the PR is merged:
git checkout main && git pull --ff-only
git tag -a v0.2.0 -m "Release v0.2.0" HEAD
git push origin v0.2.0
# (release.yml fires on the tag push; 3-OS builds + finalize)
```

**Permanent fix (the v0.2.0 follow-up):**

The `linked-versions` plugin is the root cause of the drift. The follow-up PR drops the plugin from `release-please-config.json`. With no plugin, each package's version bumps independently based on the conventional commits since its own last changelog entry. The per-crate versions may diverge (e.g. `app: 0.3.0` while `crates/*: 0.2.1` after a `app/`-only feature), but the single `v*` tag (per `include-component-in-tag: false` + `include-v-in-tag: true`) still uses the highest version — which will almost always be `app` since it's the most actively changed package. The Tauri app's user-facing version is always correct because `app/tauri.conf.json` and `ui/package.json` are mirrored by the `extra-files` rule from the `app` package's version.

After the fix, a no-op commit to main should produce a clean release-please run that either:
- No-ops (no feat/fix commits since the last release)
- Opens a single `chore: release main` PR with one entry per package that had user-facing changes (typically just `app`)

**Why this section exists:** the v0.1.3 → v0.2.0 gap is the first time a release was silently lost to a config-vs-state drift. Without a §4.6 entry, the next session that hits the same symptom would re-derive the diagnosis from the release-please logs and the manifest's divergent state — which is fine for a one-time recovery but wasteful when the same symptom recurs. This section records both the root cause (linked-versions drift) and the canonical recovery (the manual release PR), so future sessions can fix it in one shot.

### 4.7 draft-build.yml didn't fire after a pre-release tag was created

Symptom: release-please created the `vX.Y.Z-pre.N` tag and the GitHub pre-release page exists, but `gh run list --workflow="Draft build" --limit 5` shows zero rows for the tag.

Causes (most → least likely):

1. **Tag pattern mismatch.** `draft-build.yml` triggers on `'v*-pre.*'`. If the tag was created as `vX.Y.Z-preN` (no dot between `pre` and `N`), the pattern doesn't match. Fix: delete the bad tag (`git push origin :refs/tags/vX.Y.Z-preN && gh release delete vX.Y.Z-preN --yes`) and push the right tag shape (`vX.Y.Z-pre.N`).
2. **The release-please `prerelease-type` config is wrong.** If `prerelease-type` is set to a different value (e.g. `alpha`), the tag shape becomes `vX.Y.Z-alpha.N` and `v*-pre.*` doesn't match. Fix: align `prerelease-type` in `release-please-config.json` with the workflow's tag pattern. The two are coupled by convention, not by config validation.
3. **The tag was force-pushed to a different commit** with `git push --force`. GitHub Actions does NOT re-fire a workflow on force-push of an existing tag. Fix: delete and recreate the tag.
4. **The `RELEASE_PLEASE_TOKEN` PAT is missing `workflow` scope.** Same root cause as §4.2 — the tag is created (release-please used `Contents: write`), but the tag-push event is treated as coming from an untrusted actor with no workflow trigger rights. The release-please job appears to succeed (it created the tag and the GitHub Release), but `draft-build.yml` is never invoked.

Diagnostic for #4:

```bash
# Confirm the tag exists
git ls-remote --tags origin | grep vX.Y.Z-pre

# Check if the tag-push event fired any workflow
gh run list --workflow="Draft build" --limit 10
# (empty list = the tag-push event was never received by Actions,
# or the workflow file was rejected by the parser)
```

If `draft-build.yml` is in the rejected-parser state (the `gh run list` returns zero rows even after a fresh push), the workflow file has a YAML or `if:` issue. Tag-triggered workflows reject any `if:` expression — same root cause as the release.yml's macOS-job history in §4.3a. Fix: move the conditional out of YAML and into the `run:` block (or just delete the `if:`).

### 4.7a Pre-release exists but has no assets (no .deb/.dmg/.msi)

Symptom: the `vX.Y.Z-pre.N` pre-release page exists on GitHub (with the "Pre-release" badge), but the assets list is empty or only has assets from one OS.

Causes:

1. **The build jobs are still running.** First time you cut a pre-release, all 3 OS builds run in parallel. A 5-10 min wait is normal. Check `gh run list --workflow="Draft build"` for in-progress jobs.
2. **One OS build failed and the others didn't upload.** Same as §4.3 for the prod path. Each OS job is independent; a failure in one doesn't stop the others. Re-run the failed job via `gh run rerun <RUN_ID> --failed` — see §6.5 for the exact command.
3. **Tauri-action failed to attach to the pre-release** (e.g. the GitHub Release was deleted or moved by an out-of-band operation). Check the build job logs for `Error: Not Found` or `404` on the release API call. Fix: re-run the build job; tauri-action will re-attach.

---

## 6. Draft releases (pre-release builds)

Every `feat:` or `fix:` commit merged to `main` accumulates into a **pre-release** tag (`vX.Y.Z-pre.N`). The pre-release tag is built in debug mode by `draft-build.yml`, and the bundles are attached to a GitHub Release marked as "Pre-release" (visible with a badge, but not surfaced in the "Latest" feed). When you're ready to ship, you promote the pre-release to a stable release and the production build pipeline takes over (see §6.3 for the promotion step). The day-to-day version of this flow is summarized in §2.3.

### 6.1 How a pre-release gets created

```
1. Dev merges PRs to main (each commit follows Conventional Commits)
2. release-please opens or updates a release PR titled
   "chore(main): release X.Y.Z-pre.N"
   The N counter increments on each merge. If only `chore:` /
   `docs:` commits land, no pre-release is opened (the existing
   pre-release, if any, stays put).
   Files it touches (same as the stable release path):
     - Cargo.toml ([workspace.package] version)
     - app/tauri.conf.json ("version")   ← synced via extra-files
     - ui/package.json ("version")       ← synced via extra-files
     - CHANGELOG.md (pre-release section appended)
3. Reviewer (you) sanity-checks:
     - Is the bump correct? (feat → minor, fix → patch, etc.)
     - Is the changelog list complete?
     - Are the three version files in lockstep?
4. Reviewer merges the release PR
5. release-please creates:
     - git tag v<X>.<Y>.<Z>-pre.<N>
     - GitHub Release titled "Talon v<X>.<Y>.<Z>-pre.<N>"
       marked as a pre-release (NOT as a draft — assets are
       downloadable to anyone with the URL)
6. The tag push fires draft-build.yml
7. draft-build.yml builds 3 OS debug bundles in parallel:
     - Linux:   .deb + .AppImage  (debug profile, no LTO, no strip)
     - macOS:   .dmg (.app under the hood) — universal binary, UNSIGNED
     - Windows: .msi + .exe       (debug profile)
8. tauri-action uploads each bundle to the pre-release as it finishes
9. The pre-release is live at
   https://github.com/pedro-tramontin/talon/releases/tag/v<X>.<Y>.<Z>-pre.<N>
```

A pre-release build typically takes **20-30 min** end-to-end (3-OS parallel debug build, dominated by the macOS universal-binary build). Per-merge CI cost is +$3-8 in Actions minutes (the debug profile compiles faster than release, but the multi-OS matrix still adds up).

### 6.2 Why debug mode for pre-releases

| Reason | Detail |
|---|---|
| **No Apple notarisation quota burn** | Every macOS notarisation is a paid API call. Signing a debug bundle on every merge would exhaust quarterly quota before the next prod release. The macOS build in `draft-build.yml` is **unsigned by design** — `APPLE_*` secrets are NOT forwarded to that job. |
| **Matches `cargo run`** | `tauri build --debug` produces a binary that behaves identically to `cargo run -p app`. QA can validate that the bundle on the pre-release page actually matches the source on main, not a hypothetical "optimized" version. |
| **Faster than release** | The debug cargo profile skips LTO, codegen-units, and strip, so each OS build is ~30-50% faster than its release-mode equivalent. |
| **Larger binaries** | Trade-off: debug binaries are 5-10× larger (Talon debug `.dmg` is ~600 MB vs ~80 MB release) and start slower. Acceptable for internal smoke tests. |

The Linux + Windows builds **are** signed/built with the same toolchain as the prod release path; only the cargo profile is different. macOS is unsigned — see the file header of `draft-build.yml` for the explicit "no APPLE_* forwarding" design.

### 6.3 Promoting a pre-release to stable

When the pre-release is ready to ship as a real release, you promote it. The promotion is a single no-bump commit to `main`:

```bash
# From main, on any branch, after the latest pre-release is merged
git commit --allow-empty -m "chore: promote v<X>.<Y>.<Z>"
git push origin main
```

What happens next:

```
1. release-please sees the empty `chore:` commit. No new conventional
   commits since v<X>.<Y>.<Z>-pre.<N> → no version bump is needed,
   but there IS an open pre-release at that version.
2. release-please auto-promotes: closes the v<X>.<Y>.<Z>-pre.<N>
   pre-release by creating a v<X>.<Y>.<Z> tag on the same commit.
3. The stable v<X>.<Y>.<Z> tag fires release.yml (NOT draft-build.yml
   — release.yml's tag filter is `v[0-9]+.[0-9]+.[0-9]+`, which
   does not match the pre.N suffix).
4. release.yml builds 3 OS RELEASE bundles in parallel (~45-60 min).
5. The stable GitHub Release is published. The pre-release
   (`v<X>.<Y>.<Z>-pre.<N>`) is left in place as a historical
   record (you can delete it manually if you want a clean release
   list — see §6.4).
```

**Why the chore: commit trick works:** release-please treats a `chore:` commit as "no bump needed." When the version it's about to publish is the same as the open pre-release, it promotes instead of creating a new pre-release tag. The mechanism is built into release-please; you do NOT touch version files or the manifest by hand.

### 6.4 Cleaning up after a promotion

After a successful stable release, you'll have:

- A `v<X>.<Y>.<Z>` stable release (published, "Latest" badge)
- A `v<X>.<Y>.<Z>-pre.<N>` pre-release (still in the releases list, with debug bundles attached)
- N-1 older pre-release tags (`v<X>.<Y>.<Z>-pre.1` through `-pre.N-1`)

To clean up:

```bash
# Delete the pre-release records from GitHub (keeps the git tags)
gh release delete v<X>.<Y>.<Z>-pre.<N> --yes
gh release delete v<X>.<Y>.<Z>-pre.1 --yes
# ... etc for each pre.N tag

# Delete the git tags
git push origin --delete v<X>.<Y>.<Z>-pre.<N>
git push origin --delete v<X>.<Y>.<Z>-pre.1
# ... etc
```

The git tag deletion is **not** picked up by release-please as a "fix the manifest" signal — release-please only reads tags when it needs to determine the baseline, and the baseline for the next release is `v<X>.<Y>.<Z>` (the stable tag you just shipped), not any of the pre.N tags. So cleanup is purely cosmetic.

### 6.5 Re-running a failed pre-release build

`draft-build.yml` has a `workflow_dispatch` trigger for manual re-runs (the prod `release.yml` has the same). If a build fails (e.g. macOS runner flake, transient network error), don't push another commit to re-trigger the tag — push the tag again or re-run via the Actions UI:

```bash
# Find the failed run for the tag
gh run list --workflow="Draft build" --json databaseId,conclusion,headBranch \
  --jq '.[] | select(.headBranch == "v0.3.0-pre.3") | "\(.databaseId) \(.conclusion)"'

# Re-run the failed job (re-runs all jobs; failed-only is the same as
# release.yml's pattern)
gh run rerun <RUN_ID> --failed
```

The `concurrency: cancel-in-progress: true` setting means that if release-please re-pushes the same `vX.Y.Z-pre.N` tag (e.g. during a recovery from a config error), the in-flight build is cancelled. This is intentional — a new push of the same tag means the tag now points at a different commit, and the old build's artifacts are no longer valid.

### 6.6 When to use the pre-release path vs. straight-to-stable

The pre-release path is the default. Every merge to main accumulates into the active pre-release; you don't need to opt in. The decision you make is **when to promote** (cut a `chore:` commit + push).

If you want to bypass the pre-release path entirely and cut a stable release straight from main (e.g. an emergency hotfix on a Saturday), you can still do that by:

1. Closing the open pre-release PR (do NOT merge it)
2. Manually tagging `HEAD` of main as `v<X>.<Y>.<Z>`
3. Pushing the tag — this fires `release.yml` directly

But this is the exceptional case. For 99% of releases, the pre-release path produces the right behavior automatically.

---

## 7. Future work (not yet implemented)

- **Windows code signing.** Add a `WINDOWS_CERTIFICATE` secret (base64 .pfx) and the matching `tauri-action` env vars. Track separately.
- **Linux aarch64 builds.** Add `aarch64-unknown-linux-gnu` to a second Linux job that runs on a cross-build runner. Track separately.
- **Auto-publish draft release.** Currently the `finalize` job flips draft → published. tauri-action's newer versions do this automatically when all jobs finish; we'll switch when we adopt v1.
- **SBOM + Sigstore for Tauri bundles.** The Tauri v2 ecosystem is converging on a standard SBOM format; when the tooling stabilises, add a `actions/attest-build-provenance` step after tauri-action (similar to comfyui-anime-bootstrap's Docker-image provenance).
