//! Process-wide event bus for live updates to web clients.
//!
//! Multi-user scenario: analyst A adds an entity; analyst B (already
//! looking at the same case in their browser) should see it appear
//! without F5. Tauri (single-user desktop) doesn't subscribe — the
//! bus's only consumer is the axum SSE handler.
//!
//! Wire format is `Event::GraphChanged { case_id, kind }` — the
//! frontend treats both as "re-fetch this case's entities + edges".
//! Sending the kind anyway lets future clients react more precisely
//! (e.g. flash the affected node) without protocol churn.
//!
//! Backpressure: `tokio::sync::broadcast` drops the oldest event for
//! lagging subscribers. The frontend's reaction is a full re-fetch
//! anyway, so missing an event is harmless — the next event triggers
//! a fetch that picks up everything missed.

use serde::Serialize;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// One or more entities/edges/tags/notes changed in the named case.
    /// `change` is a short verb for diagnostics + future per-kind
    /// reactions; today the frontend just refetches.
    GraphChanged { case_id: i64, change: &'static str },
    /// A new row landed in `notifications` — bell-badge should refresh.
    NotificationCreated { case_id: i64 },
    /// One-line "heartbeat" so clients can tell the stream is alive
    /// during long idle periods (browsers will close a silent EventSource
    /// eventually). Emitted by the SSE handler itself, not by mutators.
    Heartbeat,
}

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Event>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for EventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBus")
            .field("subscribers", &self.tx.receiver_count())
            .finish()
    }
}

impl EventBus {
    pub fn new() -> Self {
        // Capacity 128 is plenty: events are tiny and consumers drain
        // them immediately; lagging consumers just lose the oldest.
        let (tx, _rx) = broadcast::channel(128);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    pub fn publish(&self, event: Event) {
        // `send` returns `Err(SendError)` only when there are zero
        // subscribers. Either way we ignore — events are fire-and-
        // forget.
        let _ = self.tx.send(event);
    }
}

/// Convenience: every mutating core fn ends with a call to this so
/// every web client re-fetches. Skipped for Tauri / read-only flows
/// — the cost is one Arc clone + a no-op broadcast send when no one
/// is listening.
pub fn publish_graph_changed(bus: &EventBus, case_id: i64, change: &'static str) {
    bus.publish(Event::GraphChanged { case_id, change });
}
