# Changelog

## [0.2.3](https://github.com/pedro-tramontin/talon/compare/v0.2.2...v0.2.3) (2026-07-24)


### Features

* **test:** mock feature PR for release-pipeline smoke test ([#90](https://github.com/pedro-tramontin/talon/issues/90)) ([99bb577](https://github.com/pedro-tramontin/talon/commit/99bb577a10147e7f3633687a94cff22f7b30520e))


### Bug Fixes

* **test:** mock fix PR for release-pipeline smoke test ([#91](https://github.com/pedro-tramontin/talon/issues/91)) ([8315be2](https://github.com/pedro-tramontin/talon/commit/8315be208251fe901beb7ad038688fa56fcd3400))

## [0.2.1](https://github.com/pedro-tramontin/talon/compare/v0.2.0...v0.2.1) (2026-07-23)


### Features

* **bk-events:** §7b v0.5+ — `tauri::ipc::Json<Request>` direct-arg refactor for `send_replay` (skip JSON-string round-trip) ([#73](https://github.com/pedro-tramontin/talon/pull/73)) ([8f32e25](https://github.com/pedro-tramontin/talon/commit/8f32e25))


## [0.2.0](https://github.com/pedro-tramontin/talon/compare/v0.1.2...v0.2.0) (2026-07-22)


### Features

* **bk-events:** add `WireEvent::Replay` variant + `fan_in` support for replay events ([#63](https://github.com/pedro-tramontin/talon/issues/63)) ([00262df](https://github.com/pedro-tramontin/talon/commit/00262df00687878391ffda143c4bd053bc1868c6))

## [0.1.2](https://github.com/pedro-tramontin/talon/compare/v0.1.1...v0.1.2) (2026-07-21)


### Bug Fixes

* **bk-events:** pass tokio runtime handle to fan_in from sync callers ([#57](https://github.com/pedro-tramontin/talon/issues/57)) ([10158d0](https://github.com/pedro-tramontin/talon/commit/10158d0134bb5f7197aba03f6f6d2c2efedf09bd))

## [0.1.1](https://github.com/pedro-tramontin/talon/compare/v0.1.0...v0.1.1) (2026-07-21)


### Features

* **bk-events:** WireEvent envelope + fan_in helper + WireClient ([#39](https://github.com/pedro-tramontin/talon/issues/39)) ([cf0f26b](https://github.com/pedro-tramontin/talon/commit/cf0f26bb7f22d15d4dbd3e29ca2afa5de4f39bc3))


### Bug Fixes

* **ci:** inline Cargo version in each crate (release-please compat) ([#53](https://github.com/pedro-tramontin/talon/issues/53)) ([7aeee95](https://github.com/pedro-tramontin/talon/commit/7aeee955460279183d2082cdd373a5588815c639))
