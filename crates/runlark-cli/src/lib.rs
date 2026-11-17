pub mod daemon;
pub mod dsh_events;
pub mod dsh_subscriber;
pub mod outbox;
pub mod routes;
pub mod state;
pub mod worktree;

pub use daemon::{build_router, run_daemon, run_daemon_with_dsh};
pub use dsh_events::{
    DshWireEvent, DshWireEventEnvelope, apply_runtime_event_and_reconcile, process_dsh_wire_event,
};
pub use dsh_subscriber::{DshEndpoint, DshSubscriber};
pub use outbox::{
    DispatchContext, DispatchStats, EffectOutcome, HttpDshEffector, LocalBridgeEffector,
    OutboxConfig, OutboxDispatcher, OutboxEffector,
};
pub use state::{AppState, StreamEventNotification};
pub use worktree::{WorktreeError, WorktreeManager, WorktreeResolution};
