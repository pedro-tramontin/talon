# Changelog

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
