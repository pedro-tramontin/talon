# Changelog

## [0.2.4](https://github.com/pedro-tramontin/talon/compare/v0.2.3...v0.2.4) (2026-08-03)


### Features

* **bk-engine:** add deferred Phase 8 methods (delete, update_notes, set_starred, tag_*) ([#15](https://github.com/pedro-tramontin/talon/issues/15)) ([6dfcb9e](https://github.com/pedro-tramontin/talon/commit/6dfcb9e6fbbef9ed61f644f035970fae5aeff93a))
* **scope:** scope + match & replace engines, Tauri CRUD commands, pipeline shim ([#65](https://github.com/pedro-tramontin/talon/issues/65)) ([b99d1cf](https://github.com/pedro-tramontin/talon/commit/b99d1cf3e794a554eebeffc124efd14c73b499ba))
* **test:** mock feature PR for release-pipeline smoke test ([#90](https://github.com/pedro-tramontin/talon/issues/90)) ([5b5d7d2](https://github.com/pedro-tramontin/talon/commit/5b5d7d2ce7830445b42f6f208ce78ed59482aa45))
* **ui:** 3 ExchangeList filter dropdowns (v0.6 P2 [#6](https://github.com/pedro-tramontin/talon/issues/6)) ([#85](https://github.com/pedro-tramontin/talon/issues/85)) ([71275da](https://github.com/pedro-tramontin/talon/commit/71275da7cdcde48bafde5fa95b094dda65a9fbf2))
* **v0.5+:** wire active-project rules into start_proxy, persist ProjectSettings + replay history ([#73](https://github.com/pedro-tramontin/talon/issues/73)) ([9dfa6f8](https://github.com/pedro-tramontin/talon/commit/9dfa6f81808460c7c61cbcaaf74ce3929e719000))
* **v0.5:** hex viewer, LCS diff, markdown notes, base64 wire format, type-DELETE confirm, and 3 polish items ([#48](https://github.com/pedro-tramontin/talon/issues/48)) ([0df1793](https://github.com/pedro-tramontin/talon/commit/0df1793bbf12a8d43f5a9440d1ce548e6251afec))


### Bug Fixes

* **ci:** inline Cargo version in each crate (release-please compat) ([#53](https://github.com/pedro-tramontin/talon/issues/53)) ([b87389e](https://github.com/pedro-tramontin/talon/commit/b87389e842da4f13857d72e0538a1f92f7c1827b))
* **test:** mock fix PR for release-pipeline smoke test ([#91](https://github.com/pedro-tramontin/talon/issues/91)) ([6f56a73](https://github.com/pedro-tramontin/talon/commit/6f56a730c755af2adbe2f00a292b8ba5b30266c0))
* **ui:** v0.5+ post-batch gap-fix (P2 + P3) — SettingsModal rename, back-to-source, list_projects, search.test rename ([#84](https://github.com/pedro-tramontin/talon/issues/84)) ([e133ab4](https://github.com/pedro-tramontin/talon/commit/e133ab4688d93c6b9f4ade8c0492c8bd7338128b))

## [0.2.3](https://github.com/pedro-tramontin/talon/compare/v0.2.2...v0.2.3) (2026-07-24)


### Features

* **test:** mock feature PR for release-pipeline smoke test ([#90](https://github.com/pedro-tramontin/talon/issues/90)) ([99bb577](https://github.com/pedro-tramontin/talon/commit/99bb577a10147e7f3633687a94cff22f7b30520e))


### Bug Fixes

* **test:** mock fix PR for release-pipeline smoke test ([#91](https://github.com/pedro-tramontin/talon/issues/91)) ([8315be2](https://github.com/pedro-tramontin/talon/commit/8315be208251fe901beb7ad038688fa56fcd3400))

## [0.2.1](https://github.com/pedro-tramontin/talon/compare/v0.2.0...v0.2.1) (2026-07-23)


### Features

* **bk-engine:** §7b v0.5+ — `Engine::save_settings` + `Engine::append_replay_history` / `Engine::list_replay_history` + settings/replay_history rehydration on project open ([#73](https://github.com/pedro-tramontin/talon/pull/73)) ([8f32e25](https://github.com/pedro-tramontin/talon/commit/8f32e25))


## [0.2.0](https://github.com/pedro-tramontin/talon/compare/v0.1.1...v0.2.0) (2026-07-22)


### Features

* **bk-engine:** wire scope + match &amp; replace pipeline into projects store via `Engine::get_project` accessor ([#65](https://github.com/pedro-tramontin/talon/issues/65)) ([98137cd](https://github.com/pedro-tramontin/talon/commit/98137cd2a4daa43cfa77b6e29523017b82c41f65))

## [0.1.1](https://github.com/pedro-tramontin/talon/compare/v0.1.0...v0.1.1) (2026-07-21)


### Features

* **bk-engine:** add deferred Phase 8 methods (delete, update_notes, set_starred, tag_*) ([#15](https://github.com/pedro-tramontin/talon/issues/15)) ([f1a2539](https://github.com/pedro-tramontin/talon/commit/f1a253940763cd9a4685df0976325834f4a5bd32))
* **v0.5:** hex viewer, LCS diff, markdown notes, base64 wire format, type-DELETE confirm, and 3 polish items ([#48](https://github.com/pedro-tramontin/talon/issues/48)) ([9a6d902](https://github.com/pedro-tramontin/talon/commit/9a6d9029185f0bd61495f3adbe5237291e18fc31))


### Bug Fixes

* **ci:** inline Cargo version in each crate (release-please compat) ([#53](https://github.com/pedro-tramontin/talon/issues/53)) ([7aeee95](https://github.com/pedro-tramontin/talon/commit/7aeee955460279183d2082cdd373a5588815c639))
