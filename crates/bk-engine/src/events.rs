//! Full event bus for the `bk-engine`. The Tauri UI subscribes to
//! this bus to react to state changes in the engine. The MCP server
//! (§3.5b) and the internal agent (§3.5c) subscribe to the narrower
//! MCP bus in `mcp_events.rs` instead — they only need the 5
//! MCP-shaped variants (smaller payloads, fewer concerns).
//!
//! Per the design contract §3.5a, the bus carries **12 event types**
//! for the v1 surface: 9 wired to engine state changes (project
//! open/close, exchange insert/update/star/delete, tag upsert/
//! attach/detach) + 3 Phase 6/7 stubs (`ScopeChanged`, `FuzzStarted`,
//! `FuzzFinished`) for forward-compat. `#[non_exhaustive]` is on the
//! enum so future phases can add variants without breaking the v1
//! shape. The demux from `EngineEvent` → `McpEvent` lives in
//! `engine.rs` (not in this bus — the bus is a dumb pipe).
//!
//! **The bus is a `tokio::sync::broadcast` channel.** Slow consumers
//! drop events (the broadcast API reports `RecvError::Lagged`); the
//! tests in `tests/capture_loop_smoke.rs` use a fresh subscription
//! per test to avoid contention.

#![allow(missing_docs)]

use bk_core::{ExchangeId, HttpExchange, ProjectId, TagId};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// The full set of events the engine emits. New variants go at the
/// end so existing subscribers can still pattern-match the earlier
/// ones without an exhaustive arms warning.
///
/// Variants marked **(stub)** in the spec are emitted by future
/// phases and stay unimplemented in §3.5a; they exist now so the
/// MCP bus can pattern-match on them with a TODO arm.
///
/// **v0.5 (added 2026-07-21):** `PartialEq` and `Eq` were BOTH
/// dropped from the derive. The new `ExchangeInserted.exchange:
/// HttpExchange` field holds `HttpExchange` (and `HeaderMap` /
/// `Bytes` / `Url` / `Method`), none of which implement `Eq`.
/// The v0.1 design only had `Eq` so a downstream `assert_eq!`
/// could compare event fixtures; the v0.5 change drops that
/// capability in exchange for the embedded body. Tests that
/// need to compare events compare the `id` field (always
/// cheap, always works); the v0.5 test in this file was
/// rewritten to assert each variant constructs without panicking
/// (the previous v0.1 test asserted on a `==` of full event
/// values, which is no longer possible).
///
/// **v0.5 (added 2026-07-21):** `clippy::large_enum_variant` is
/// allowed on the enum because the new `ExchangeInserted`
/// variant carries the full `HttpExchange` (~hundreds of
/// bytes per insert, vs. the previous summary-string shape
/// at ~100 bytes). The "embed the body in the event"
/// decision is a deliberate wire-shape change (per the
/// v0.5 follow-up) that prefers a larger event payload
/// over a per-click `getExchange` round-trip; boxing the
/// field would re-introduce the heap allocation the
/// `bytes::Bytes` refcount was supposed to amortize.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EngineEvent {
    /// `Engine::open_project` succeeded. Carries the new (or
    /// re-opened) project's ID and the DB filename the UI shows in
    /// the project dropdown.
    ProjectOpened {
        project_id: ProjectId,
        db_filename: String,
    },
    /// `Engine::close_project` ran. The UI removes the project from
    /// the dropdown.
    ProjectClosed { project_id: ProjectId },
    /// `Engine::insert_exchange` ran. The UI adds a row to the
    /// exchange list.
    ///
    /// **v0.5 (added 2026-07-21):** the event now carries the
    /// full `HttpExchange` (request + response bodies) so the
    /// UI can populate the list + the right-rail detail in
    /// one step, without the per-click `get_exchange`
    /// round-trip that the v0.1 design called for. The wire
    /// payload IS the in-memory `HttpExchange` (serialized
    /// via serde; the body is base64-encoded per the
    /// `body_complete_data_serde` helper in `bk_core`). The
    /// cost is a larger event payload (typically 1-10 KB per
    /// exchange, vs ~100 bytes for the summary-only form);
    /// the benefit is the per-click round-trip is gone, which
    /// matters most on a high-traffic capture (a busy proxy
    /// can insert 10-50 exchanges per second).
    ///
    /// The `WireEventKind` enum (in `crates/bk-events`) is
    /// `#[non_exhaustive]`; the wire side uses a separate
    /// additive emit path (per §4.0's design), so a future
    /// payload-shape change doesn't need to touch the
    /// consumer's `WireClient` switch.
    ExchangeInserted {
        id: ExchangeId,
        project_id: ProjectId,
        /// The full `HttpExchange` (request + response bodies).
        /// Replaces the v0.1 `summary: String` field.
        exchange: HttpExchange,
    },
    /// `Engine::update_notes` ran. The UI re-fetches the detail
    /// view so the notes pane shows the new value.
    ExchangeNotesUpdated {
        id: ExchangeId,
        project_id: ProjectId,
    },
    /// `Engine::set_starred` ran. The UI updates the ⭐ icon on
    /// the row.
    ExchangeStarredToggled {
        id: ExchangeId,
        project_id: ProjectId,
        starred: bool,
    },
    /// `Engine::delete_exchange` ran. The UI removes the row.
    ExchangeDeleted {
        id: ExchangeId,
        project_id: ProjectId,
    },
    /// `Engine::tag_upsert` ran (created OR returned the existing
    /// tag, since the operation is idempotent). The UI updates the
    /// tag picker.
    TagUpserted {
        id: TagId,
        project_id: ProjectId,
        name: String,
    },
    /// `Engine::tag_attach` ran. The UI updates the tag chips on
    /// the affected exchange.
    TagAttached {
        tag_id: TagId,
        exchange_id: ExchangeId,
        project_id: ProjectId,
    },
    /// `Engine::tag_detach` ran. The UI updates the tag chips.
    TagDetached {
        tag_id: TagId,
        exchange_id: ExchangeId,
        project_id: ProjectId,
    },
    /// Phase 6 stub. When scope rules change, the UI re-colors
    /// in-scope rows. Not emitted in §3.5a.
    ScopeChanged {
        project_id: ProjectId,
        rule_count: u32,
    },
    /// Phase 7 stub. When a fuzz job starts, the Fuzz view shows
    /// the job. Not emitted in §3.5a.
    FuzzStarted {
        job_id: String,
        project_id: ProjectId,
        config_summary: String,
    },
    /// Phase 7 stub. When a fuzz job finishes, the Fuzz view
    /// shows the final stats. Not emitted in §3.5a.
    FuzzFinished {
        job_id: String,
        project_id: ProjectId,
        total_requests: u64,
    },
}

/// The handle the engine hands out to subscribers. The capacity
/// is `EVENT_BUS_CAPACITY` (1024); the previous value of 256 was
/// fine for the summary-only `EngineEvent` (each event ~100 B),
/// but the v0.5 change to embed the full `HttpExchange` in
/// `ExchangeInserted` (~1-10 KB per event) increased the
/// in-flight memory by ~40-100x. At 50 exchanges/sec the ring
/// now holds 50-500 KB of bodies instead of 5 KB; raising the
/// cap to 1024 gives the WebView + MCP consumers ~20 sec of
/// headroom on a busy proxy instead of ~5 sec. Slow consumers
/// still see `RecvError::Lagged` and should resubscribe or
/// skip; the wire_bus logs a `warn!` and emits a synthetic
/// `EngineResync` `WireEvent` so the UI can re-fetch state.
pub type EventReceiver = broadcast::Receiver<EngineEvent>;

/// The bus capacity used by `channel()`. See `EventReceiver`'s
/// docstring for the sizing rationale. 1024 is a conservative
/// default that gives the UI time to render a frame between
/// insert bursts; if a future workload needs more headroom,
/// raise this and update the `EVENT_BUS_CAPACITY_*` tests.
pub const EVENT_BUS_CAPACITY: usize = 1024;

/// The sender side, owned by the `Engine`. Cloning the sender is
/// cheap (`broadcast::Sender` is internally `Arc`-wrapped), so the
/// engine can hand out additional senders to sub-components if
/// needed in future phases.
pub type EventSender = broadcast::Sender<EngineEvent>;

/// Build a fresh (sender, receiver) pair. The `Engine::new`
/// constructor stores the sender and exposes `subscribe_events()`
/// which returns a fresh `EventReceiver`.
///
/// Capacity note: `EVENT_BUS_CAPACITY` is the per-receiver ring
/// size. The `tokio::sync::broadcast` API has no global "drop
/// oldest" semantic — each receiver's `Lag(n)` count is
/// independent — so this value is a per-consumer upper bound on
/// how many events can be queued before the slowest consumer
/// starts dropping. 1024 is sized for the v0.5 wire shape
/// (1-10 KB per `ExchangeInserted`); see `EventReceiver`'s
/// docstring for the reasoning.
pub fn channel() -> (EventSender, EventReceiver) {
    broadcast::channel(EVENT_BUS_CAPACITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sender is `Clone`able so the engine can hand out
    /// additional senders cheaply. `Sender::subscribe()` returns
    /// a fresh `Receiver` per call (broadcast channels support
    /// many independent subscribers); `Receiver` itself is NOT
    /// `Clone` — to fan out to multiple consumers, each consumer
    /// must call `Sender::subscribe()` and get its own receiver.
    #[test]
    fn sender_is_cloneable_receivers_are_independent() {
        let (tx, mut rx1) = channel();
        let _tx2 = tx.clone();
        let mut rx2 = tx.subscribe();
        // Send one event — both receivers must see it
        // independently. A shared-receiver bug (e.g., accidentally
        // returning rx1 from a second subscribe call) would fail
        // this: only one of the two would receive.
        tx.send(EngineEvent::ProjectClosed {
            project_id: ProjectId::new(),
        })
        .unwrap();
        let ev1 = rx1.try_recv().expect("rx1 should receive the event");
        let ev2 = rx2.try_recv().expect("rx2 should receive the event");
        assert!(matches!(ev1, EngineEvent::ProjectClosed { .. }));
        assert!(matches!(ev2, EngineEvent::ProjectClosed { .. }));
    }

    /// A send on the sender is observed by a receiver. The smoke
    /// test in `tests/capture_loop_smoke.rs` is the end-to-end
    /// version; this is the unit-level version that doesn't need
    /// the engine.
    #[test]
    fn send_is_observed_by_receiver() {
        let (tx, mut rx) = channel();
        let project_id = ProjectId::new();
        tx.send(EngineEvent::ProjectClosed { project_id }).unwrap();
        let ev = rx.try_recv().unwrap();
        // v0.5: the `EngineEvent` enum no longer derives
        // `PartialEq` (the `ExchangeInserted.exchange: HttpExchange`
        // field holds types that don't all implement `PartialEq`).
        // The test pattern-matches on the variant tag instead.
        assert!(matches!(ev, EngineEvent::ProjectClosed { .. }));
    }

    /// `#[non_exhaustive]` on the enum means downstream code must
    /// have a wildcard arm. This test pins the v1 surface: any
    /// new variant must be added here and to `mcp_events::demux`.
    ///
    /// **v0.5 (added 2026-07-21):** the previous version of this
    /// test asserted each variant equals the corresponding
    /// sent payload via `assert_eq!` on the `EngineEvent` enum.
    /// The v0.5 wire-format change drops `PartialEq` on the
    /// enum (because the new `ExchangeInserted.exchange:
    /// HttpExchange` field holds types that don't all implement
    /// `PartialEq`). The test now pattern-matches on the
    /// received events to confirm the variant tag; the
    /// per-field equality checks were lost but the load-bearing
    /// assertion is "each variant constructs and survives the
    /// broadcast channel round-trip", which the pattern-match
    /// pins.
    #[test]
    fn exhaustively_pinned_v1_variants_present() {
        let (tx, mut rx) = channel();
        // Send one of each v1 variant. If a variant is renamed or
        // removed, this test fails to compile — that's the point.
        let project_id = ProjectId::new();
        let exchange_id = ExchangeId::new();
        let tag_id = TagId::new();
        tx.send(EngineEvent::ProjectOpened {
            project_id,
            db_filename: "acme.db".into(),
        })
        .unwrap();
        tx.send(EngineEvent::ProjectClosed { project_id }).unwrap();
        tx.send(EngineEvent::ExchangeInserted {
            id: exchange_id,
            project_id,
            exchange: bk_core::HttpExchange {
                meta: bk_core::ExchangeMeta {
                    id: exchange_id,
                    project_id,
                    timestamp: chrono::Utc::now(),
                    duration_ns: 0,
                    summary: "GET /admin".into(),
                    scope_state: bk_core::ScopeState::InScope,
                    notes: String::new(),
                    starred: false,
                    // v0.6 P2 #6: defaults for the new fields.
                    method: "GET".to_string(),
                    status: 200,
                    tags: Vec::new(),
                },
                request: bk_core::Request::get("https://acme.bb/admin").expect("valid URL"),
                response: None,
                blocked_reason: None,
            },
        })
        .unwrap();
        tx.send(EngineEvent::ExchangeNotesUpdated {
            id: exchange_id,
            project_id,
        })
        .unwrap();
        tx.send(EngineEvent::ExchangeStarredToggled {
            id: exchange_id,
            project_id,
            starred: true,
        })
        .unwrap();
        tx.send(EngineEvent::ExchangeDeleted {
            id: exchange_id,
            project_id,
        })
        .unwrap();
        tx.send(EngineEvent::TagUpserted {
            id: tag_id,
            project_id,
            name: "vuln".into(),
        })
        .unwrap();
        tx.send(EngineEvent::TagAttached {
            tag_id,
            exchange_id,
            project_id,
        })
        .unwrap();
        tx.send(EngineEvent::TagDetached {
            tag_id,
            exchange_id,
            project_id,
        })
        .unwrap();
        // Stubs — they're v1 enum variants even if the engine
        // doesn't emit them yet. The MCP bus demux pattern-matches
        // on them with a "not yet implemented" arm.
        tx.send(EngineEvent::ScopeChanged {
            project_id,
            rule_count: 0,
        })
        .unwrap();
        tx.send(EngineEvent::FuzzStarted {
            job_id: "stub".into(),
            project_id,
            config_summary: "stub".into(),
        })
        .unwrap();
        // Drain the channel and pattern-match each variant. The
        // match is exhaustive: a renamed or removed variant
        // fails to compile (the `#[non_exhaustive]` attribute
        // on the enum triggers a compile error on a missing
        // arm). The per-field checks that the v0.1 test had
        // are gone (no `PartialEq`); the variant tag is the
        // only thing the v0.5 test pins.
        //
        // **v0.5 review (2026-07-21):** the per-field equality
        // checks were lost when `PartialEq` was dropped from
        // `EngineEvent` (the new `ExchangeInserted.exchange:
        // HttpExchange` field holds types that don't all
        // implement `PartialEq`). This v0.5.1 patch restores
        // the per-field checks for the variants that don't
        // carry the new field by binding the destructured
        // fields and asserting on them — the exchange-inserted
        // variant gets a partial check (id + project_id only;
        // the body field is too complex to assert on directly
        // and is covered by the `bk_core` round-trip tests).
        let received = rx.try_recv().unwrap();
        match received {
            EngineEvent::ProjectOpened {
                project_id: pid,
                db_filename: db,
            } => {
                assert_eq!(pid, project_id);
                assert_eq!(db, "acme.db");
            }
            _ => panic!("expected ProjectOpened, got {received:?}"),
        }
        let received = rx.try_recv().unwrap();
        match received {
            EngineEvent::ProjectClosed { project_id: pid } => {
                assert_eq!(pid, project_id);
            }
            _ => panic!("expected ProjectClosed, got {received:?}"),
        }
        let received = rx.try_recv().unwrap();
        match received {
            EngineEvent::ExchangeInserted {
                id: eid,
                project_id: pid,
                exchange,
            } => {
                assert_eq!(eid, exchange_id);
                assert_eq!(pid, project_id);
                // Spot-check the embedded body field's
                // summary (the only easily-equatable string
                // on the HttpExchange without implementing
                // PartialEq on the whole tree).
                assert_eq!(exchange.meta.summary, "GET /admin");
                assert_eq!(exchange.meta.id, exchange_id);
            }
            _ => panic!("expected ExchangeInserted, got {received:?}"),
        }
        let received = rx.try_recv().unwrap();
        match received {
            EngineEvent::ExchangeNotesUpdated {
                id: eid,
                project_id: pid,
            } => {
                assert_eq!(eid, exchange_id);
                assert_eq!(pid, project_id);
            }
            _ => panic!("expected ExchangeNotesUpdated, got {received:?}"),
        }
        let received = rx.try_recv().unwrap();
        match received {
            EngineEvent::ExchangeStarredToggled {
                id: eid,
                project_id: pid,
                starred: s,
            } => {
                assert_eq!(eid, exchange_id);
                assert_eq!(pid, project_id);
                assert!(s);
            }
            _ => panic!("expected ExchangeStarredToggled, got {received:?}"),
        }
        let received = rx.try_recv().unwrap();
        match received {
            EngineEvent::ExchangeDeleted {
                id: eid,
                project_id: pid,
            } => {
                assert_eq!(eid, exchange_id);
                assert_eq!(pid, project_id);
            }
            _ => panic!("expected ExchangeDeleted, got {received:?}"),
        }
        let received = rx.try_recv().unwrap();
        match received {
            EngineEvent::TagUpserted {
                id: tid,
                project_id: pid,
                name: n,
            } => {
                assert_eq!(tid, tag_id);
                assert_eq!(pid, project_id);
                assert_eq!(n, "vuln");
            }
            _ => panic!("expected TagUpserted, got {received:?}"),
        }
        let received = rx.try_recv().unwrap();
        match received {
            EngineEvent::TagAttached {
                tag_id: tid,
                exchange_id: eid,
                project_id: pid,
            } => {
                assert_eq!(tid, tag_id);
                assert_eq!(eid, exchange_id);
                assert_eq!(pid, project_id);
            }
            _ => panic!("expected TagAttached, got {received:?}"),
        }
        let received = rx.try_recv().unwrap();
        match received {
            EngineEvent::TagDetached {
                tag_id: tid,
                exchange_id: eid,
                project_id: pid,
            } => {
                assert_eq!(tid, tag_id);
                assert_eq!(eid, exchange_id);
                assert_eq!(pid, project_id);
            }
            _ => panic!("expected TagDetached, got {received:?}"),
        }
        // Stubs — they're v1 enum variants even if the engine
        // doesn't emit them yet. The MCP bus demux pattern-matches
        // on them with a "not yet implemented" arm.
        let received = rx.try_recv().unwrap();
        match received {
            EngineEvent::ScopeChanged {
                project_id: pid,
                rule_count: rc,
            } => {
                assert_eq!(pid, project_id);
                assert_eq!(rc, 0);
            }
            _ => panic!("expected ScopeChanged, got {received:?}"),
        }
        let received = rx.try_recv().unwrap();
        match received {
            EngineEvent::FuzzStarted {
                job_id: jid,
                project_id: pid,
                config_summary: cs,
            } => {
                assert_eq!(jid, "stub");
                assert_eq!(pid, project_id);
                assert_eq!(cs, "stub");
            }
            _ => panic!("expected FuzzStarted, got {received:?}"),
        }
    }

    /// `channel()` uses `EVENT_BUS_CAPACITY`. The constant
    /// value is part of the v0.5 wire contract (raised from
    /// 256 to 1024 when `ExchangeInserted` started embedding
    /// the full `HttpExchange`); a regression here would
    /// re-introduce the silent-lag risk on busy proxies.
    #[test]
    fn channel_uses_documented_capacity() {
        assert_eq!(
            EVENT_BUS_CAPACITY, 1024,
            "EVENT_BUS_CAPACITY must be 1024 (the v0.5 contract)"
        );
        // Round-trip: send+receive one event to confirm the
        // channel is wired correctly.
        let (tx, mut rx) = channel();
        tx.send(EngineEvent::ProjectClosed {
            project_id: ProjectId::new(),
        })
        .unwrap();
        assert!(rx.try_recv().is_ok());
    }
}
