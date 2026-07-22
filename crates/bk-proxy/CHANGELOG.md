# Changelog

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
