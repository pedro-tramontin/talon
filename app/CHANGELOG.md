# Changelog

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
