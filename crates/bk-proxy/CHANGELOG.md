# Changelog

## [0.2.4](https://github.com/pedro-tramontin/talon/compare/v0.2.3...v0.2.4) (2026-08-03)


### Features

* **bk-proxy:** §3.1+§3.2 — TCP listener, dynamic root CA, per-host leaf signing ([#16](https://github.com/pedro-tramontin/talon/issues/16)) ([370f3fe](https://github.com/pedro-tramontin/talon/commit/370f3feec4f88dffc9bfe60abc0442df009ea306))
* **bk-proxy:** §3.3.5 — body streaming + upstream connection pool ([#18](https://github.com/pedro-tramontin/talon/issues/18)) ([f08f35a](https://github.com/pedro-tramontin/talon/commit/f08f35adbb4991484b2ad089b78a6584f80b85c5))
* **bk-proxy:** §3.5 — HTTP/2 upstream forwarder (ALPN h2 + H1/H2 enum) ([#23](https://github.com/pedro-tramontin/talon/issues/23)) ([ccd1a3f](https://github.com/pedro-tramontin/talon/commit/ccd1a3fc30f193a6f06114a10beb718a64fdfb73))
* **bk-proxy:** CONNECT intercept, MITM TLS termination, upstream forward ([#17](https://github.com/pedro-tramontin/talon/issues/17)) ([44cce64](https://github.com/pedro-tramontin/talon/commit/44cce6455f075f09312010fb92be79b2d31c3fb2))
* **replay:** §5.4+§5.5+§5.6 view, request editor, history panel, wire event arm + smoke test ([#64](https://github.com/pedro-tramontin/talon/issues/64)) ([6499afd](https://github.com/pedro-tramontin/talon/commit/6499afd45e8ad4cdeecd0edc58b0f31bffd31d11))
* **scope:** scope + match & replace engines, Tauri CRUD commands, pipeline shim ([#65](https://github.com/pedro-tramontin/talon/issues/65)) ([b99d1cf](https://github.com/pedro-tramontin/talon/commit/b99d1cf3e794a554eebeffc124efd14c73b499ba))
* **test:** mock feature PR for release-pipeline smoke test ([#90](https://github.com/pedro-tramontin/talon/issues/90)) ([5b5d7d2](https://github.com/pedro-tramontin/talon/commit/5b5d7d2ce7830445b42f6f208ce78ed59482aa45))
* **ui:** 3 ExchangeList filter dropdowns (v0.6 P2 [#6](https://github.com/pedro-tramontin/talon/issues/6)) ([#85](https://github.com/pedro-tramontin/talon/issues/85)) ([71275da](https://github.com/pedro-tramontin/talon/commit/71275da7cdcde48bafde5fa95b094dda65a9fbf2))


### Bug Fixes

* **bk-proxy:** §3.3.6 — keep PooledConn alive until response body drained ([#20](https://github.com/pedro-tramontin/talon/issues/20)) ([0e07bc6](https://github.com/pedro-tramontin/talon/commit/0e07bc60cc236a23f63007cfb7e88c1b674e1ecd))
* **ci:** inline Cargo version in each crate (release-please compat) ([#53](https://github.com/pedro-tramontin/talon/issues/53)) ([b87389e](https://github.com/pedro-tramontin/talon/commit/b87389e842da4f13857d72e0538a1f92f7c1827b))
* **test:** mock fix PR for release-pipeline smoke test ([#91](https://github.com/pedro-tramontin/talon/issues/91)) ([6f56a73](https://github.com/pedro-tramontin/talon/commit/6f56a730c755af2adbe2f00a292b8ba5b30266c0))

## [0.2.3](https://github.com/pedro-tramontin/talon/compare/v0.2.2...v0.2.3) (2026-07-24)


### Features

* **test:** mock feature PR for release-pipeline smoke test ([#90](https://github.com/pedro-tramontin/talon/issues/90)) ([99bb577](https://github.com/pedro-tramontin/talon/commit/99bb577a10147e7f3633687a94cff22f7b30520e))


### Bug Fixes

* **test:** mock fix PR for release-pipeline smoke test ([#91](https://github.com/pedro-tramontin/talon/issues/91)) ([8315be2](https://github.com/pedro-tramontin/talon/commit/8315be208251fe901beb7ad038688fa56fcd3400))

## [0.2.1](https://github.com/pedro-tramontin/talon/compare/v0.2.0...v0.2.1) (2026-07-23)


### Features

* **bk-proxy:** §7b v0.5+ — wire active-project rules (scope + M&R) into `start_proxy` (no longer empty `Vec`) + `take_pending_rules` for the future v0.5+ capture loop ([#73](https://github.com/pedro-tramontin/talon/pull/73)) ([8f32e25](https://github.com/pedro-tramontin/talon/commit/8f32e25))


## [0.2.0](https://github.com/pedro-tramontin/talon/compare/v0.1.1...v0.2.0) (2026-07-22)


### Features

* **bk-proxy:** §6 — scope + match &amp; replace engines, pipeline shim (classify + rewrite) + replay-pipeline E2E test ([#64](https://github.com/pedro-tramontin/talon/issues/64), [#65](https://github.com/pedro-tramontin/talon/issues/65)) ([78fe263](https://github.com/pedro-tramontin/talon/commit/78fe263ee286e97fa3c2f10178f694848b1f41af))

## [0.1.1](https://github.com/pedro-tramontin/talon/compare/v0.1.0...v0.1.1) (2026-07-21)


### Features

* **bk-proxy:** §3.1+§3.2 — TCP listener, dynamic root CA, per-host leaf signing ([#16](https://github.com/pedro-tramontin/talon/issues/16)) ([b97a73e](https://github.com/pedro-tramontin/talon/commit/b97a73ecda8ce32f3759445a71809022fdd4db1f))
* **bk-proxy:** §3.3.5 — body streaming + upstream connection pool ([#18](https://github.com/pedro-tramontin/talon/issues/18)) ([17c20b2](https://github.com/pedro-tramontin/talon/commit/17c20b2fed0ca381f3d7a05ef2c01450fb44a2f7))
* **bk-proxy:** §3.5 — HTTP/2 upstream forwarder (ALPN h2 + H1/H2 enum) ([#23](https://github.com/pedro-tramontin/talon/issues/23)) ([8f60965](https://github.com/pedro-tramontin/talon/commit/8f609653d3a9194b175626c13666c00e3759a754))
* **bk-proxy:** CONNECT intercept, MITM TLS termination, upstream forward ([#17](https://github.com/pedro-tramontin/talon/issues/17)) ([82ca0b3](https://github.com/pedro-tramontin/talon/commit/82ca0b3d4612440c4684657ba9d67d47c97caddf))


### Bug Fixes

* **bk-proxy:** §3.3.6 — keep PooledConn alive until response body drained ([#20](https://github.com/pedro-tramontin/talon/issues/20)) ([b8407f1](https://github.com/pedro-tramontin/talon/commit/b8407f17fef409f15bd582066db0130e3c721171))
* **ci:** inline Cargo version in each crate (release-please compat) ([#53](https://github.com/pedro-tramontin/talon/issues/53)) ([7aeee95](https://github.com/pedro-tramontin/talon/commit/7aeee955460279183d2082cdd373a5588815c639))
