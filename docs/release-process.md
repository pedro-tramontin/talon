# Release process

This document describes how a Talon release is built, signed, and published. It is the human-readable companion to the two GitHub Actions workflows that do the actual work:

- [`.github/workflows/release-please.yml`](../.github/workflows/release-please.yml) — opens and updates the release PR; creates the git tag and the GitHub Release.
- [`.github/workflows/release.yml`](../.github/workflows/release.yml) — builds the Tauri bundles for all three target OSes and attaches them to the release.

If you are about to cut a release for the first time, read **§1 (prerequisites)** and **§2 (cutting a release)** end-to-end before pushing the green button.

---

## 1. Prerequisites

### 1.1 Repository secrets

Configure these in **Settings → Secrets and variables → Actions**. The list below maps each secret to the workflow that uses it.

| Secret | Used by | Required? | Notes |
|---|---|---|---|
| `RELEASE_PLEASE_TOKEN` | `release-please.yml` | **Yes** | A fine-grained PAT scoped to the talon repo with `Contents: Read and write` and `Pull requests: Read and write`. Do NOT use the default `GITHUB_TOKEN` — GitHub intentionally blocks the default token from triggering downstream workflows, which means a tag push won't fire `release.yml`. This is the bug that shipped v0.1.0 of vaultenv with zero assets. |
| `GITHUB_TOKEN` | `release.yml` (each OS job) | Auto | The default per-run token. tauri-action uses it with `contents: write` to attach bundles to the GitHub Release. No setup needed. |
| `APPLE_CERTIFICATE` | `release.yml` (macOS job) | No (optional) | Base64-encoded `.p12` Developer ID Application certificate. Without this, the macOS `.dmg` is unsigned and Apple will warn on first launch. Recommended for public releases. |
| `APPLE_CERTIFICATE_PASSWORD` | `release.yml` (macOS job) | Only if `APPLE_CERTIFICATE` set | Password for the `.p12`. |
| `APPLE_SIGNING_IDENTITY` | `release.yml` (macOS job) | Only if `APPLE_CERTIFICATE` set | `Developer ID Application: <Your Name> (<TEAM_ID>)`. |
| `APPLE_ID` | `release.yml` (macOS job) | Only if `APPLE_CERTIFICATE` set | Apple ID email for notarisation. |
| `APPLE_PASSWORD` | `release.yml` (macOS job) | Only if `APPLE_CERTIFICATE` set | App-specific password (not your Apple ID password). |
| `APPLE_TEAM_ID` | `release.yml` (macOS job) | Only if `APPLE_CERTIFICATE` set | 10-character Apple Developer Team ID. |

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
- A draft `.deb` and `.AppImage` build for **both** `x86_64` and `aarch64` Linux is **not** included in the first-cut pipeline. The current Linux job builds on `ubuntu-22.04` (x86_64 only); aarch64 Linux is a follow-up. Track this separately.

---

## 3. What lives where

| File | Purpose |
|---|---|
| `release-please-config.json` | Declares the rust package as the canonical version source + the two `extra-files` (tauri.conf.json, ui/package.json) that get the same version mechanically stamped. |
| `.release-please-manifest.json` | The version release-please is "tracking" right now. Updated on every release PR. |
| `Cargo.toml` `[workspace.package] version` | Single source of truth. release-please computes the next version from conventional commits and writes it here first; the extra-files entries mirror. |
| `app/tauri.conf.json` `"version"` | Mirrored by release-please's `extra-files`. The bundle metadata at build time is stamped from the tag (not from this field), so a one-commit drift here is harmless but ugly. |
| `ui/package.json` `"version"` | Same as `app/tauri.conf.json`. Mirrored for in-UI display and to keep `pnpm version` honest. |
| `CHANGELOG.md` | Regenerated by release-please on every release. Do NOT edit by hand — release-please will overwrite it on the next release PR. |
| `.github/workflows/release-please.yml` | Runs release-please on every push to `main`. Uses `RELEASE_PLEASE_TOKEN` (not the default token). |
| `.github/workflows/release.yml` | Tag-triggered. 3 parallel OS builds + 1 finalize job. Uses `tauri-apps/tauri-action@v0`. |
| `renovate.json5` | Weekly version-PRs for deps + GitHub Actions. Unrelated to release-please. |

---

## 4. Troubleshooting

### 4.1 release-please didn't open a release PR

Causes (most → least likely):

1. **No `feat:` / `fix:` / breaking-change commits since the last release.** This is correct behavior — release-please only opens a release PR when there's a bump to do. Wait for a release-worthy commit.
2. **Wrong branch.** release-please only watches `main`. If you merged a release-worthy commit to a feature branch, it won't trigger.
3. **The `RELEASE_PLEASE_TOKEN` secret is missing or expired.** The release-please workflow will fail with `403 Forbidden` on the API call. Rotate the PAT in Settings → Secrets.

### 4.2 release.yml didn't fire after a tag push

Almost always: the `RELEASE_PLEASE_TOKEN` PAT is the same as a normal user token without `workflow` write permission. The tag is created (release-please used the PAT's `Contents: write` scope), but the workflow doesn't fire because GitHub treats the push as coming from an "untrusted actor" with no workflow trigger rights.

Fix: regenerate the PAT with the additional `workflow` scope (fine-grained: `Actions: Read and write`), or use a GitHub App instead. After rotation, delete and recreate the tag — release-please won't recreate an existing tag.

```bash
# To retry without bumping versions: delete the tag, push, re-run the
# failed release-please job.
git push origin :refs/tags/v0.2.0
gh workflow run release-please.yml
```

### 4.3 One OS build fails but the others succeed

`release.yml` is `fail-fast: false` in spirit — each OS job is independent and uploads its assets to the same draft Release as they finish. So if, say, the macOS build fails (e.g. signing cert expired), you get a Release with the Linux .deb + Windows .msi but no .dmg.

Fix: address the underlying issue (renew the cert, fix the build error), delete the tag, re-run. The new release.yml run will upload the missing bundle. The existing bundles will be **re-uploaded** with the same filenames — GitHub deduplicates by filename in the UI.

```bash
git push origin :refs/tags/v0.2.0
git push origin v0.2.0   # release-please won't recreate; you need to
                          # re-run release-please explicitly, OR fix
                          # the underlying commit and let release-please
                          # open a new release PR (then re-merge + retag).
```

### 4.4 The Release body is wrong or stale

The body is set by the first build job to finish (via tauri-action's `releaseBody:` input). Subsequent jobs don't overwrite it. If the body is wrong (e.g. a `feat:` was misclassified and the changelog is misleading), delete the release, fix the commit message, and cut a new release.

### 4.5 Renovate opened a PR for a dep that should be ignored

Add a carve-out to `renovate.json5` under `packageRules` with `matchUpdateTypes: ["major"]` or `matchPackageNames` as appropriate. The next Renovate scan (within `prHourlyLimit: 2`) will pick up the new rule.

---

## 5. Future work (not yet implemented)

- **Windows code signing.** Add a `WINDOWS_CERTIFICATE` secret (base64 .pfx) and the matching `tauri-action` env vars. Track separately.
- **Linux aarch64 builds.** Add `aarch64-unknown-linux-gnu` to a second Linux job that runs on a cross-build runner. Track separately.
- **Auto-publish draft release.** Currently the `finalize` job flips draft → published. tauri-action's newer versions do this automatically when all jobs finish; we'll switch when we adopt v1.
- **SBOM + Sigstore for Tauri bundles.** The Tauri v2 ecosystem is converging on a standard SBOM format; when the tooling stabilises, add a `actions/attest-build-provenance` step after tauri-action (similar to comfyui-anime-bootstrap's Docker-image provenance).
