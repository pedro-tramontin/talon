# Changelog

## [0.2.3](https://github.com/pedro-tramontin/talon/compare/v0.2.2...v0.2.3) (2026-07-24)


### Features

* **test:** mock feature PR for release-pipeline smoke test ([#90](https://github.com/pedro-tramontin/talon/issues/90)) ([99bb577](https://github.com/pedro-tramontin/talon/commit/99bb577a10147e7f3633687a94cff22f7b30520e))


### Bug Fixes

* **test:** mock fix PR for release-pipeline smoke test ([#91](https://github.com/pedro-tramontin/talon/issues/91)) ([8315be2](https://github.com/pedro-tramontin/talon/commit/8315be208251fe901beb7ad038688fa56fcd3400))

## [0.2.2](https://github.com/pedro-tramontin/talon/compare/v0.2.1...v0.2.2) (2026-07-24)


### Features

* **ui:** × delete button + confirm dialog for hard-deleting exchanges from the exchange list ([#86](https://github.com/pedro-tramontin/talon/pull/86)) ([9c4f079](https://github.com/pedro-tramontin/talon/commit/9c4f0799ddb56e7e6a3b8ba6e2bb1d59c8d39e96))
* **ui:** 3 ExchangeList filter dropdowns (Status / Method / Tag) + extended `matchesExchangeFilter` ([#85](https://github.com/pedro-tramontin/talon/pull/85)) ([e2c6211](https://github.com/pedro-tramontin/talon/commit/e2c6211239b67b9d23c19ed7a5a7aaf8cbe3c8e3))
* **ui:** listExchanges on project switch + close-project button + openReplayTab `body_truncated` notice (P1 trio) ([#83](https://github.com/pedro-tramontin/talon/pull/83)) ([fe0c6f6](https://github.com/pedro-tramontin/talon/commit/fe0c6f63450d49b1b7e7a8e6e4a3e6f4b2c2b1b0))
* **ui:** add proxy control + `proxy_event` wire subscription (P0 — was the missing top-bar control for the MITM proxy) ([#82](https://github.com/pedro-tramontin/talon/pull/82)) ([df52dc6](https://github.com/pedro-tramontin/talon/commit/df52dc6f8c2c5d3e2b6a3c1d4e5f6a7b8c9d0e1f))
* **server:** browser-access mode (Phase 8) — new `bk-server` axum crate serves the same React UI + Rust backend over HTTP/WS, with loopback / remote-with-auth / remote-with-mDNS threat-model modes, `Sec-WebSocket-Protocol: talon-auth.<token>` WS auth, and a per-install auth token ([#81](https://github.com/pedro-tramontin/talon/pull/81)) ([7f2bdea](https://github.com/pedro-tramontin/talon/commit/7f2bdeaa52aca843243e9ed75416199e891acc31))
* **ui:** add New Project modal next to the project dropdown — wired the existing `open_project` Tauri command to a UI affordance (the v0.5+ post-batch gap fix) ([#79](https://github.com/pedro-tramontin/talon/pull/79)) ([7063c23](https://github.com/pedro-tramontin/talon/commit/7063c23080e3f6b3a3b8b3a3b3a3b3a3b3a3b3a3))
* **release:** add top-level `/CHANGELOG.md` managed by release-please (the aggregator per `changelog-path` in `release-please-config.json`) ([#78](https://github.com/pedro-tramontin/talon/pull/78)) ([112cf17](https://github.com/pedro-tramontin/talon/commit/112cf17c8b61fdd682b90eab4004f7551c9ff5e9))


### Bug Fixes

* **ci:** use `RELEASE_PLEASE_TOKEN` (not `GITHUB_TOKEN`) in `release.yml` — fixes the "Resource not accessible by integration" error on tauri-action's upload step (gotcha #13 in the tauri-release-pipeline skill) ([#87](https://github.com/pedro-tramontin/talon/pull/87)) ([3ede3fb](https://github.com/pedro-tramontin/talon/commit/3ede3fb))
* **ui:** v0.5+ post-batch gap-fix bundle (P2 + P3) — SettingsModal title "Settings" → "Match & Replace", back-to-source button on the replay tab, `Engine::list_open_projects` + new `list_projects` Tauri command, `ui/src/state/search.test.tsx` → `ui/src/state/ui.search.test.tsx` rename ([#84](https://github.com/pedro-tramontin/talon/pull/84)) ([c35946a](https://github.com/pedro-tramontin/talon/commit/c35946a))
* **docs:** correct broken cross-doc links in developer.md ([#77](https://github.com/pedro-tramontin/talon/pull/77)) ([5030558](https://github.com/pedro-tramontin/talon/commit/5030558))


### Notes

This release was cut manually via the gotcha #15a bypass (release-please's
internal PR-tracker was stuck on PR #60 — the v0.1.3 release PR — and
bailed on every subsequent run). The version bumps + the CHANGELOG entry
were done by hand following the bypass recipe. The next release-please
run (after PR #87's workflow fix lands) will resume normal operation
and the v0.2.2 → v0.2.3 cycle will work via the standard path.

## [0.2.1](https://github.com/pedro-tramontin/talon/compare/v0.2.0...v0.2.1) (2026-07-23)


### Features

* **ui:** M&R row-edit + SecLists bulk-import + JSON/form-data views + fork from history ([#75](https://github.com/pedro-tramontin/talon/pull/75)) ([e90de12](https://github.com/pedro-tramontin/talon/commit/e90de12))
* **ui:** persist replay history (UI side) + add M&R "Test" button ([#74](https://github.com/pedro-tramontin/talon/pull/74)) ([5a81922](https://github.com/pedro-tramontin/talon/commit/5a81922))
* **ui:** §7b v0.5+ — wire active-project rules into start_proxy, persist ProjectSettings + replay history (Rust side + replay history UI) ([#73](https://github.com/pedro-tramontin/talon/pull/73)) ([8f32e25](https://github.com/pedro-tramontin/talon/commit/8f32e25))


## [0.2.0](https://github.com/pedro-tramontin/talon/compare/v0.1.3...v0.2.0) (2026-07-22)


### Features

* **app,ui:** scope rule editor + Settings modal + Match &amp; Replace editor ([#66](https://github.com/pedro-tramontin/talon/issues/66)) ([bf29044](https://github.com/pedro-tramontin/talon/commit/bf29044ddca622004e31542578925e56f51b0a8b))
* **app,ui:** scope + match &amp; replace engines, Tauri CRUD commands, pipeline shim ([#65](https://github.com/pedro-tramontin/talon/issues/65)) ([98137cd](https://github.com/pedro-tramontin/talon/commit/98137cd2a4daa43cfa77b6e29523017b82c41f65))
* **app,ui:** §5.4+§5.5+§5.6 replay view, request editor, history panel + WireEvent arm + smoke test ([#64](https://github.com/pedro-tramontin/talon/issues/64)) ([78fe263](https://github.com/pedro-tramontin/talon/commit/78fe263ee286e97fa3c2f10178f694848b1f41af))
* **app:** §5.1+§5.2+§5.3 replay Tauri commands (open_replay_tab, send_replay) + ReplayStore + WireEvent::Replay variant ([#63](https://github.com/pedro-tramontin/talon/issues/63)) ([00262df](https://github.com/pedro-tramontin/talon/commit/00262df00687878391ffda143c4bd053bc1868c6))


### Bug Fixes

* **ci:** ship unsigned macOS bundle without breaking release.yml (no `if:`) ([#62](https://github.com/pedro-tramontin/talon/issues/62)) ([02dda93](https://github.com/pedro-tramontin/talon/commit/02dda933ce823b8905e034de4bfb95139e13c62e))
* **ci:** ship unsigned macOS `.dmg` when `APPLE_CERTIFICATE` is absent ([#61](https://github.com/pedro-tramontin/talon/issues/61)) ([e6bf006](https://github.com/pedro-tramontin/talon/commit/e6bf00674b165f6e60cbf931f492ef1c2a570c39))

## [0.1.3](https://github.com/pedro-tramontin/talon/compare/v0.1.2...v0.1.3) (2026-07-21)


### Bug Fixes

* **windows:** set PE subsystem to WINDOWS in release builds ([#59](https://github.com/pedro-tramontin/talon/issues/59)) ([8725cd3](https://github.com/pedro-tramontin/talon/commit/8725cd340dcf5c2943ac5b3e23f16fe0b586db0e))

## [0.1.2](https://github.com/pedro-tramontin/talon/compare/v0.1.1...v0.1.2) (2026-07-21)


### Bug Fixes

* **bk-events:** pass tokio runtime handle to fan_in from sync callers ([#57](https://github.com/pedro-tramontin/talon/issues/57)) ([10158d0](https://github.com/pedro-tramontin/talon/commit/10158d0134bb5f7197aba03f6f6d2c2efedf09bd))

## [0.1.1](https://github.com/pedro-tramontin/talon/compare/v0.1.0...v0.1.1) (2026-07-21)


### Features

* **app,ui:** wire bk-agent into Tauri shell with confirmation flow ([#29](https://github.com/pedro-tramontin/talon/issues/29)) ([07e5d34](https://github.com/pedro-tramontin/talon/commit/07e5d34ca5169ff3e8baf91e4e0fb4421e1dc1b3))
* **app:** engine + proxy wiring in Tauri shell with 6 commands ([#40](https://github.com/pedro-tramontin/talon/issues/40)) ([2d894b4](https://github.com/pedro-tramontin/talon/commit/2d894b4a02c0af0ab54c09244c5359eca59ff112))
* **app:** wire engine + proxy event buses through WireEvent fan-in ([#41](https://github.com/pedro-tramontin/talon/issues/41)) ([23f1373](https://github.com/pedro-tramontin/talon/commit/23f1373ce712638dd74915126ce27ab6c7a51716))
* **bk-events:** WireEvent envelope + fan_in helper + WireClient ([#39](https://github.com/pedro-tramontin/talon/issues/39)) ([cf0f26b](https://github.com/pedro-tramontin/talon/commit/cf0f26bb7f22d15d4dbd3e29ca2afa5de4f39bc3))
* **ui,app:** right-rail inspector tabs (4 panels + update_notes Tauri command) ([#45](https://github.com/pedro-tramontin/talon/issues/45)) ([5ffb399](https://github.com/pedro-tramontin/talon/commit/5ffb3996ecfee20a8287a0a1ab15c2170b7666fc))
* **ui,app:** wire FTS5 search to Tauri command + UI ([#46](https://github.com/pedro-tramontin/talon/issues/46)) ([b7cada0](https://github.com/pedro-tramontin/talon/commit/b7cada02668541ad73ea29ee2aad9e31a57d8291))
* **v0.5:** hex viewer, LCS diff, markdown notes, base64 wire format, type-DELETE confirm, and 3 polish items ([#48](https://github.com/pedro-tramontin/talon/issues/48)) ([9a6d902](https://github.com/pedro-tramontin/talon/commit/9a6d9029185f0bd61495f3adbe5237291e18fc31))


### Bug Fixes

* **ci:** inline Cargo version in each crate (release-please compat) ([#53](https://github.com/pedro-tramontin/talon/issues/53)) ([7aeee95](https://github.com/pedro-tramontin/talon/commit/7aeee955460279183d2082cdd373a5588815c639))
