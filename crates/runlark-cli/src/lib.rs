pub mod chat;
pub mod cli;
pub mod daemon;
pub mod dsh_events;
pub mod dsh_subscriber;
pub mod git_pr;
pub mod outbox;
pub mod plugin;
pub mod repair;
pub mod routes;
pub mod runner_agent;
mod runner_buffer;
pub mod scheduler;
pub mod state;
pub mod team_http;
pub mod team_sync;
pub mod verification;
pub mod worktree;

pub use chat::{ChatDispatchRequest, ChatDispatchResponse, dispatch_chat_command};
pub use daemon::{build_router, run_daemon, run_daemon_with_dsh};
pub use dsh_events::{
    DshWireEvent, DshWireEventEnvelope, apply_runtime_event_and_reconcile, process_dsh_wire_event,
};
pub use dsh_subscriber::{DshEndpoint, DshSubscriber};
pub use git_pr::{
    CreatePrOutcome, CreatePrRequest, GitDiffSnapshot, GitPrError, PrCommandError, PrCommandOutput,
    PrCommandRunner, PrCreationService, SystemPrRunner, compute_worktree_diff, store_diff_in_cas,
};
pub use outbox::{
    DispatchContext, DispatchStats, EffectOutcome, HttpDshEffector, LocalBridgeEffector,
    OutboxConfig, OutboxDispatcher, OutboxEffector,
};
pub use repair::{PipelineError, VerifyPipelineOptions, VerifyPipelineOutcome, WorkflowPipeline};
pub use runner_agent::{
    RunOwnershipResponse, RunnerAgentWorker, RunnerStateInput, RunnerTickSummary,
};
pub use scheduler::{DEFAULT_MAX_CONCURRENT_SLOTS, Scheduler, SchedulerStatus};
pub use state::{AppState, StreamEventNotification};
pub use team_http::TeamServerError;
pub use team_sync::{TeamCommandSubmissionResult, TeamSyncSummary, TeamSyncWorker};
pub use verification::VerificationService;
pub use worktree::{WorktreeError, WorktreeManager, WorktreeResolution};
