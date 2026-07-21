# ADR-0001: Supply-chain monitoring policy

- **Status:** Accepted
- **Date:** 2026-07-02
- **Deciders:** Pedro Tramontin, Hermes

## Threat model

Before listing the policy, here is the explicit threat model that justifies the 18 `ignore` entries below. This section exists so a future security review can challenge the model — if it ever changes, the ignores may need to change with it.

**The Tauri webview in Talon only ever renders one source of HTML/JS/CSS: the bundled `ui/dist/` directory that ships inside the Tauri binary.** Specifically:

- The webview is created with `WebviewUrl::App(...)` (the Tauri 2 default for `tauri.conf.json`'s `build.frontendDist`), which means the webview's only valid URL is the local file path to `index.html` in the bundled dist. There is no path from Talon's UI to load a remote URL — doing so requires code change to use `WebviewUrl::External(...)`, which we never do.
- The CSP in `app/tauri.conf.json` (`app.security.csp`) is enforced by the webview engine at runtime. The current policy: `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self' ipc: http://ipc.localhost ws://localhost:* http://localhost:*; base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'`. The `connect-src` allows `localhost:*` for Vite HMR during development; in a release build the webview is bundled and the localhost sources are unused but still allow-listed for the dev case.
- The 4 Tauri IPC channels are typed commands (`#[tauri::command]` Rust functions), and the only one that exists in phase 1 is `greet`, which returns a static string. None of the IPC commands parse untrusted HTML, XML, or URL patterns.
- Talon does not have a feature that opens arbitrary URLs in the webview (e.g. a "preview this request" pane showing raw HTML from an HTTP response). When/if such a feature is added, this threat model changes and the `quick-xml`/`html5ever` `ignore` entries below need to be re-evaluated.

**What the 18 advisories are actually about:**

| Group | Crates | Where it runs | Reachable from user input in Talon? |
|---|---|---|---|
| gtk-rs GTK3 (×11) | atk, atk-sys, gdk, gdk-sys, gdkx11, gdkx11-sys, gdkwayland-sys, gtk, gtk-sys, gtk3-macros + proc-macro-error | `tao`, `wry`, `webkit2gtk-sys` (Tauri Linux windowing layer) | **No.** These run regardless of UI input. Unmaintained, not vulnerable. |
| unic-* (×5) | unic-char-range, unic-char-property, unic-common, unic-ucd-ident, unic-ucd-version | `tauri-utils → urlpattern` (used internally by Tauri to parse URL patterns in our config at app load time) | **No.** Only at app startup, parsing our own `tauri.conf.json`. We don't accept user-supplied URL patterns. |
| `quick-xml` (×2) | quick-xml 0.39.4 | `tauri → plist` (macOS bundle manifest writer, used at `tauri build --bundles app` time) | **No.** Build-time only, never reached at runtime. |
| `html5ever` (×2) | (via `tauri-utils → dom_query`, used for icon extraction at `tauri build` time) | **No.** Build-time only. |

**Conclusion:** the practical exploitability of any of the 18 advisories in Talon's current scope is **zero**. The `ignore` list is a forward-looking hygiene measure: we want the audit to fail the build the moment a fix exists upstream, but in the meantime, none of these are reachable from the application we ship. The CSP and the `WebviewUrl::App` default are the load-bearing controls that keep the threat model true — if either changes, this ADR must be re-evaluated.

## Upstream tracking

These are the upstream issues/PRs that would unblock each group of `ignore` entries. When any of them ships, remove the corresponding entries from `deny.toml` in a follow-up PR.

| Group | Upstream | Status | Tracking |
|---|---|---|---|
| gtk-rs GTK3 (×11) | Tauri 2 GTK4 migration | In progress | https://github.com/tauri-apps/tauri/issues?q=is%3Aissue+gtk4 (search for the latest GTK4 tracking issue) |
| unic-* (×5) | `urlpattern` dropping unmaintained unic-* (or inlining the tables) | Not started | (No public tracking issue as of 2026-07-02) |
| `quick-xml` (×2) | `dtolnay/plist` bumping `quick-xml` to 0.41+ | Not started | (No public tracking issue; we are not opening the PR for now — see ADR-0001 re-evaluation trigger 6) |
| `html5ever` (×2) | `tauri-utils` bumping `dom_query` past 0.27 | Not started | (Bundled into the same upstream conversation as unic-*) |

A quarterly cron (`talon-gtk4-watchdog`, scheduled in the user's Hermes cron registry) polls the Tauri release notes and the `denoland/urlpattern` release page for any of the above. When one ships, the cron delivers a one-line report to the user's Matrix home channel; the user opens a follow-up PR to remove the relevant `ignore` entries.

## Context

JavaScript and Rust ecosystems both have a history of supply-chain incidents:

- **npm:** typosquatting (e.g. `cross-env.js` 2017, `event-stream` 2018, `ua-parser-js` 2021, `colors.js` / `faker.js` 2022, `node-ipc` 2022), compromised maintainer accounts.
- **crates.io:** typosquatting (`rustc-serialize` 2024), compromised maintainer accounts (`uutils` coreutils 2024), unmaintained-but-still-in-tree crates (gtk-rs GTK3 family, unic-* family).

Talon is a Tauri 2 desktop app. The React UI is bundled into the binary at build time, so transitive dev-dependency vulnerabilities do not affect shipped users directly, but **build-time** vulnerabilities in our dev toolchain are a real concern (e.g. an attacker on the developer's LAN can exploit the Vite dev server to read local files).

For the Rust side, transitive runtime dependencies of Tauri itself carry both **unmaintained** warnings (zero CVEs but no upstream maintainer) and real **vulnerabilities** (DoS in `quick-xml` and `html5ever`).

The project's dependency tree will only grow over time. We need an automated, blocking check now — at the moment we have 5 JS advisories and 18 Rust advisories — so that adding new dependencies cannot silently introduce new ones.

## Decision

We adopt the following supply-chain policy, enforced as a **blocking** CI step (`.github/workflows/ci.yml` → `supply-chain` job):

### JavaScript (UI)

- **Tool:** `pnpm audit` (built into pnpm 9+).
- **Scope:** `pnpm audit --prod` is the primary signal. Dev-dependency audits run on PRs as well (full tree).
- **Severity threshold:** `moderate` and above fails the build. Low/informational are warned.
- **Renewal:** When a fixed version of an affected package exists, the dep tree is updated as part of the same PR that surfaces the advisory (no "we'll fix it next sprint" drift).

### Rust (workspace)

- **Tool:** `cargo deny` (the standard Rust supply-chain tool; covers advisories + licenses + bans + sources).
- **Scope:** `cargo deny check advisories` is the primary signal. License and bans sections are intentionally disabled until a separate ADR defines the policy.
- **Severity threshold:** Any advisory of any kind fails the build by default.
- **Upstream-blocked advisories** are tolerated via `deny.toml`'s `ignore` list, with a **mandatory comment** per entry naming:
  1. The upstream crate responsible for the transitive dependency.
  2. The release or migration that should make the ignore unnecessary.
  3. A reference to this ADR.

### Automated renewals

- **Renovate** runs weekly on both `npm` and `cargo` ecosystems (plus GitHub Actions versions), opens PRs for new versions, and groups related minor/patch updates to limit noise. The previous Dependabot config was retired in 2026-07 in favour of Renovate for richer grouping rules.

### Local enforcement

- `make audit` runs both `pnpm audit --prod` and `cargo deny check advisories` locally.
- `make ci` includes `make audit` so local runs match CI.

## Consequences

### Positive

- New vulnerabilities in transitive dependencies fail CI before they can be merged.
- Upstream-blocked advisories are documented with a known reason and a known fix-path. The moment a fix ships upstream, CI re-surfaces the advisory and we know to update.
- Weekly Renovate PRs keep the dep tree current without manual review overhead.

### Negative

- 18 Rust advisories are currently in the `ignore` list. If we forget to revisit them, they can stay forever. Mitigation: ADR-0001 + the comment-per-entry rule + a quarterly manual review (not automated; ADR-0002 if/when we add it).
- Renovate will open PRs on a weekly cadence. We must not let them pile up; the user merges per the project's PR discipline.
- The supply-chain CI job adds ~1-2 minutes to PR feedback.

## The 18 currently-ignored advisories

These are split into three groups by upstream root cause:

### 1. gtk-rs GTK3 bindings — 11 advisories (RUSTSEC-2024-0411..0420 + RUSTSEC-2024-0370)

```
RUSTSEC-2024-0411  gdkwayland-sys
RUSTSEC-2024-0412  gdk
RUSTSEC-2024-0413  atk
RUSTSEC-2024-0414  gdkx11-sys
RUSTSEC-2024-0415  gtk
RUSTSEC-2024-0416  atk-sys
RUSTSEC-2024-0417  gdkx11
RUSTSEC-2024-0418  gdk-sys
RUSTSEC-2024-0419  gtk3-macros
RUSTSEC-2024-0420  gtk-sys
RUSTSEC-2024-0370  proc-macro-error (via glib-macros)
```

**Root cause:** Tauri's Linux backend (`tao`, `wry`, `webkit2gtk-sys`) depends on `gtk 0.18.x`, which transitively pulls the entire gtk-rs GTK3 family. The gtk-rs team archived the GTK3 repo in 2024 in favor of `gtk4-rs`.

**Fix path:** Tauri 2 migrating to GTK4. Track [tauri-apps/tauri#gtk4](https://github.com/tauri-apps/tauri/labels/gtk4) (or equivalent issue tracker). When Tauri ships a release that drops the gtk 0.18 dep, all 11 advisories should disappear in one go.

### 2. unic-* family — 5 advisories (RUSTSEC-2025-0075, 0080, 0081, 0098, 0100)

```
RUSTSEC-2025-0075  unic-char-range
RUSTSEC-2025-0080  unic-common
RUSTSEC-2025-0081  unic-char-property
RUSTSEC-2025-0098  unic-ucd-version
RUSTSEC-2025-0100  unic-ucd-ident
```

**Root cause:** `tauri-utils → urlpattern 0.3.0` depends on the unic-* family, which the open-i18n/rust-unic project abandoned in 2025.

**Fix path:** `tauri-utils` bumping to a `urlpattern` version that either inlines the Unicode tables or depends on the actively-maintained `unicode-*` crates. Track `tauri-utils` releases.

### 3. quick-xml DoS — 2 advisories (RUSTSEC-2026-0194, 0195)

```
RUSTSEC-2026-0194  quick-xml 0.39.4 (DoS via duplicate attribute names)
RUSTSEC-2026-0195  quick-xml 0.39.4 (DoS via unbounded namespace declarations)
```

**Root cause:** `tauri → plist 1.9.0` pins `quick-xml = "^0.39.2"`. `quick-xml 0.41.0` fixes both, but the version range constraint blocks the bump.

**Fix path:** `plist 1.10+` dropping the version pin. Track [dtolnay/plist](https://github.com/dtolnay/plist) releases. These are DoS-class and require a malicious XML payload; Talon does not parse untrusted XML in normal operation, so the practical risk is low.

## Re-evaluation triggers

This ADR should be re-evaluated when any of the following becomes true:

1. Tauri ships a release that drops `gtk 0.18` (→ 11 ignores become unnecessary).
2. `tauri-utils` ships a release that bumps `urlpattern` past unmaintained unic-* (→ 5 ignores become unnecessary).
3. `plist` ships a release that uses `quick-xml 0.41+` (→ 2 ignores become unnecessary).
4. A new advisory appears that we **can** fix ourselves (e.g. a vuln in one of our direct deps).
5. Talon's threat model changes to include parsing untrusted HTML/XML (the two `quick-xml` DoS advisories become first-class concerns).
6. A future PR to `dtolnay/plist` lands (we are not opening it ourselves at this time per the user's preference; the `talon-gtk4-watchdog` cron only watches `tauri-apps/tauri` and `denoland/urlpattern` releases).

When trigger 1, 2, or 3 fires, remove the corresponding `ignore` entries from `deny.toml` in a follow-up PR. When trigger 4 fires, update the affected direct dep. When trigger 5 fires, revisit the doS-class ignores with a fresh risk analysis.

## Alternatives considered

- **Warn-only mode for the Rust side:** Easier, but means the 2 real DoS CVEs get lost in the 16 unmaintained warnings. Rejected.
- **Fail-permanently for all 18:** Forces a multi-day yak-shave (migrate to GTK4, fork Tauri, or migrate to a non-Tauri framework) that is not justified for phase 1. Rejected.
- **Drop Tauri entirely (egui, Slint, Flutter):** Removes GTK3-rs warnings but throws away the React app and the Tauri ecosystem. Multi-month UI rewrite. Rejected for phase 1; revisit if our threat model changes.
- **Pin to Tauri beta/next with GTK4 work:** Bleeding edge, may have other issues. Rejected.
