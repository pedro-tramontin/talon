//! WebSocket hub — a `tokio::sync::broadcast::Sender<WireEvent>`
//! wrapper that fans events out to every connected WS client.

use std::sync::Arc;

use bk_events::WireEvent;
use tokio::sync::broadcast;

/// Capacity of the broadcast channel. 1024 events is plenty
/// for the v1 use case (a few exchanges/second, the user is
/// the only one consuming). Lagged subscribers are dropped;
/// the seq-counter gap in the wire format signals "missed
/// events" to the React side.
const BROADCAST_CAPACITY: usize = 1024;

/// The WebSocket hub. Cheap to clone (the inner `Sender` is
/// already an `Arc`-wrapped broadcast sender).
#[derive(Clone)]
pub struct WsHub {
    tx: Arc<broadcast::Sender<WireEvent>>,
}

impl Default for WsHub {
    fn default() -> Self {
        Self::new()
    }
}

impl WsHub {
    /// Build a new hub with the default capacity.
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
        Self { tx: Arc::new(tx) }
    }

    /// Subscribe a new WS client. The returned `Receiver` is
    /// what the WS handler's send task reads from.
    pub fn subscribe(&self) -> broadcast::Receiver<WireEvent> {
        self.tx.subscribe()
    }

    /// Broadcast a wire event to every subscriber. Returns
    /// `Err` if there are no subscribers (normal at startup
    /// — the WS clients connect after the server starts).
    pub fn broadcast(&self, event: WireEvent) {
        let _ = self.tx.send(event);
    }

    /// The axum WS upgrade callback. Called by
    /// `axum::extract::ws::WebSocketUpgrade::on_upgrade`.
    ///
    /// `WebSocket` in axum 0.7 doesn't implement `Clone`,
    /// so we use the socket's `Stream` + `Sink` impls
    /// sequentially in a single task: poll the broadcast
    /// hub, write to the socket when there's an event,
    /// poll the socket for inbound messages (mostly
    /// no-ops today), loop until either side closes.
    pub async fn on_upgrade(self, socket: axum::extract::ws::WebSocket) {
        use futures::StreamExt;
        let mut socket = socket;
        let mut rx = self.subscribe();

        loop {
            tokio::select! {
                // Inbound: a WS message from the client.
                // Most messages are ignored (the client
                // doesn't send anything meaningful today),
                // but the read keeps the connection alive
                // and detects close frames.
                msg = socket.next() => {
                    match msg {
                        Some(Ok(axum::extract::ws::Message::Close(_))) | None => break,
                        Some(Err(_)) => break,
                        _ => {}
                    }
                }
                // Outbound: a wire event from the broadcast
                // hub. The `select!` biases the inbound
                // branch, but `Rx::recv` is cancel-safe so
                // the events are not lost.
                event = rx.recv() => {
                    match event {
                        Ok(ev) => {
                            let bytes = match serde_json::to_vec(&ev) {
                                Ok(b) => b,
                                Err(e) => {
                                    tracing::warn!("ws serialize failed: {e}");
                                    continue;
                                }
                            };
                            if socket.send(axum::extract::ws::Message::Binary(bytes)).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            tracing::warn!("ws client lagged, dropping events");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    }
}
