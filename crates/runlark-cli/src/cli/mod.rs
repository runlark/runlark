pub mod client;
pub mod handlers;
pub mod run_chat_handlers;
pub mod team_handlers;

use crate::cli::client::DaemonHttpClient;
use crate::cli::handlers::{
    handle_chat_repl, handle_chat_send, handle_plugin, handle_project, handle_question, handle_run,
    handle_task,
};
use crate::cli::team_handlers::{handle_runner, handle_team};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "runlark")]
#[command(about = "Runlark local daemon and workflow orchestrator", long_about = None)]
pub struct Cli {
    /// Base URL of a running Runlark daemon (falls back to `RUNLARK_DAEMON_URL` or `<http://127.0.0.1:9876>`)
    #[arg(long, global = true)]
    pub daemon_url: Option<String>,

    /// Emit structured JSON output
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the Runlark local daemon process
    Daemon {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(short, long, default_value_t = 9876)]
        port: u16,
        #[arg(long, default_value = ".runlark")]
        data_dir: PathBuf,
        #[arg(long)]
        dsh_url: Option<String>,
        #[arg(long)]
        dsh_token: Option<String>,
    },
    /// Manage Runlark project workspaces
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Manage Kanban tasks and workflow actions
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Inspect workflow runs, events, history, diffs, and verification
    Run {
        #[command(subcommand)]
        action: RunAction,
    },
    /// List and answer pending human approval questions
    Question {
        #[command(subcommand)]
        action: QuestionAction,
    },
    /// Send slash commands or open an interactive chat REPL
    Chat {
        #[command(subcommand)]
        action: ChatAction,
    },
    /// Inspect, validate, install, and list workflow/WASM validator packages
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// Manage A2 team workspaces, invitations, synchronization, and offline drafts
    Team {
        #[command(subcommand)]
        action: TeamAction,
    },
    /// Manage A2 execution runner registration, leases, and unreachable-runner cancellation
    Runner {
        #[command(subcommand)]
        action: RunnerAction,
    },
}

#[derive(Subcommand)]
pub enum ProjectAction {
    /// Initialize/register a project workspace
    #[command(alias = "init")]
    Create {
        #[arg(long)]
        id: Option<String>,
        name: String,
        #[arg(long)]
        repo_root: Option<String>,
        #[arg(long, default_value = "main")]
        default_branch: String,
        #[arg(long, default_value_t = 3)]
        max_attempts: u32,
        #[arg(long, default_value_t = 1_800_000)]
        max_duration_ms: u64,
        #[arg(long, default_value_t = 5_000_000)]
        max_spend_micro_usd: u64,
    },
    /// List registered projects
    List,
    /// Show details of a project
    Show { id: String },
    /// Promote a solo local project into a shared team workspace (requires zero active runs)
    PromoteTeam {
        project_id: String,
        #[arg(long)]
        server: String,
        #[arg(long)]
        workspace: String,
        /// Bearer token (prefer `RUNLARK_TEAM_TOKEN`, `--token-file`, or `--token-stdin` to avoid argv exposure)
        #[arg(long, conflicts_with_all = ["token_file", "token_stdin"])]
        token: Option<String>,
        /// Read bearer token from file path (or `-` for stdin)
        #[arg(long, value_name = "PATH", conflicts_with = "token_stdin")]
        token_file: Option<PathBuf>,
        /// Read bearer token from stdin
        #[arg(long)]
        token_stdin: bool,
    },
}

#[derive(Subcommand)]
pub enum TaskAction {
    /// Create a Standard (or `QuickFix` with --quick) task in Inbox
    Create {
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        project_id: Option<String>,
        title: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long)]
        quick: bool,
    },
    /// Create a `QuickFix` task directly
    Fix {
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        project_id: Option<String>,
        title: String,
        #[arg(long, default_value = "")]
        description: String,
    },
    /// List tasks
    List {
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        stage: Option<String>,
    },
    /// Show detailed task summary
    Show { id: String },
    /// Move task to a Kanban stage (inbox, ready, `in_progress`, acceptance, done)
    Move {
        id: String,
        stage: String,
        #[arg(long, default_value = "human")]
        actor: String,
    },
    /// Record clarification input and advance Clarification -> Plan
    Clarify {
        id: String,
        clarification: String,
        #[arg(long)]
        no_advance: bool,
    },
    /// Update task plan and optionally approve into Implementation
    Plan {
        id: String,
        plan_text: String,
        #[arg(long)]
        approve: bool,
    },
    /// Start or enqueue a run for a task (subject to 2-slot scheduler)
    Start {
        id: String,
        #[arg(long)]
        prompt: Option<String>,
    },
    /// Request cancellation of an active run
    Stop {
        run_id: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Record bound human acceptance (moves task to Done)
    Accept {
        id: String,
        #[arg(long, default_value = "human")]
        actor: String,
        #[arg(long)]
        comment: Option<String>,
    },
    /// Request changes on a task in Acceptance
    Reject {
        id: String,
        reason: String,
        #[arg(long, default_value = "human")]
        actor: String,
    },
    /// Push branch and create a PR via gh for an accepted task
    Pr {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        base_branch: Option<String>,
        #[arg(long)]
        draft: bool,
    },
    /// View full Kanban board snapshot
    Board {
        #[arg(long)]
        project_id: Option<String>,
    },
    /// Append an immutable discussion comment onto a task timeline
    Comment {
        task_id: String,
        message: String,
        #[arg(long)]
        run_id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum TeamAction {
    /// Authenticate against a Runlark collaboration server
    Login {
        #[arg(long)]
        server: String,
        /// Bearer token (prefer `RUNLARK_TEAM_TOKEN`, `--token-file`, or `--token-stdin` to avoid argv exposure)
        #[arg(long, conflicts_with_all = ["token_file", "token_stdin"])]
        token: Option<String>,
        /// Read bearer token from file path (or `-` for stdin)
        #[arg(long, value_name = "PATH", conflicts_with = "token_stdin")]
        token_file: Option<PathBuf>,
        /// Read bearer token from stdin
        #[arg(long)]
        token_stdin: bool,
        #[arg(long)]
        workspace: Option<String>,
    },
    /// List workspace members and roles
    Members {
        #[arg(long)]
        workspace: String,
    },
    /// Create a workspace membership invitation
    Invite {
        #[arg(long)]
        workspace: String,
        #[arg(long)]
        invitee: String,
        #[arg(long, default_value = "member")]
        role: String,
        #[arg(long)]
        ttl_ms: Option<u64>,
    },
    /// Accept a workspace invitation token and pull initial workspace snapshot
    AcceptInvite {
        #[arg(long)]
        server: String,
        /// Invite token (prefer `RUNLARK_TEAM_INVITE_TOKEN`, `RUNLARK_INVITE_TOKEN`, `--token-file`, or `--token-stdin` to avoid argv exposure)
        #[arg(long, conflicts_with_all = ["token_file", "token_stdin"])]
        token: Option<String>,
        /// Read invite token from file path (or `-` for stdin)
        #[arg(long, value_name = "PATH", conflicts_with = "token_stdin")]
        token_file: Option<PathBuf>,
        /// Read invite token from stdin
        #[arg(long)]
        token_stdin: bool,
        #[arg(long)]
        name: String,
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Synchronize outbox commands, events/snapshots, and CAS artifacts with the team server
    Sync {
        #[arg(long)]
        workspace: Option<String>,
    },
    /// List local offline drafts and queued/conflicted sync outbox commands
    Drafts {
        #[arg(long)]
        project_id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum RunnerAction {
    /// Register this local daemon as a team execution runner
    Register {
        #[arg(long)]
        name: String,
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        runner_id: Option<String>,
    },
    /// Show local runner identity, buffered event queue, and workspace runners
    Status {
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Exclusively assign an online runner to a run
    Assign {
        run_id: String,
        #[arg(long)]
        runner_id: String,
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
    /// Explicitly cancel an unreachable runner's active run without silent takeover
    CancelUnreachable {
        run_id: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        expected_revision: Option<u64>,
    },
}

#[derive(Subcommand)]
pub enum RunAction {
    /// List runs
    List {
        #[arg(long)]
        task_id: Option<String>,
        #[arg(long)]
        status: Option<String>,
    },
    /// Show a single run
    Show { id: String },
    /// Show run event stream
    Events {
        id: String,
        #[arg(long)]
        after_seq: Option<u64>,
    },
    /// Show full run history (attempts, cost, decisions, artifacts, events)
    History { id: String },
    /// Show git diff for a run's isolated worktree
    Diff { id: String },
    /// Execute digest-pinned WASM verification on a run
    Verify {
        id: String,
        #[arg(long)]
        task_id: String,
        #[arg(long, default_value = "solo-default")]
        package_name: String,
    },
}

#[derive(Subcommand)]
pub enum QuestionAction {
    /// List pending questions
    List {
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        run_id: Option<String>,
    },
    /// Answer a pending question
    Answer { id: String, answer: String },
}

#[derive(Subcommand)]
pub enum ChatAction {
    /// Send a single slash command or prompt
    Send {
        input: String,
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        run_id: Option<String>,
    },
    /// Start an interactive chat REPL
    Repl {
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        session_id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum PluginAction {
    /// Validate manifest, SHA-256 digests, WIT compatibility, and Wasmtime execution
    Validate {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Inspect package metadata, digests, WIT capabilities, and workflow stages
    Inspect {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Install and pin a workflow/validator package into the running daemon
    Install { path: PathBuf },
    /// List installed workflow/validator packages in the running daemon
    List,
}

/// Dispatches a parsed [`Cli`] command.
///
/// # Errors
/// Returns an error if daemon execution, HTTP request, or plugin validation fails.
pub async fn run_cli(cli: Cli) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let resolved_url = cli
        .daemon_url
        .or_else(|| std::env::var("RUNLARK_DAEMON_URL").ok())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:9876".to_string());
    let client = DaemonHttpClient::new(&resolved_url);
    let json_mode = cli.json;

    match cli.command {
        Commands::Daemon {
            host,
            port,
            data_dir,
            dsh_url,
            dsh_token,
        } => crate::daemon::run_daemon_with_dsh(host, port, data_dir, dsh_url, dsh_token).await,
        Commands::Project { action } => handle_project(&client, action, json_mode).await,
        Commands::Task { action } => handle_task(&client, action, json_mode).await,
        Commands::Run { action } => handle_run(&client, action, json_mode).await,
        Commands::Question { action } => handle_question(&client, action, json_mode).await,
        Commands::Chat { action } => match action {
            ChatAction::Send {
                input,
                project_id,
                session_id,
                run_id,
            } => handle_chat_send(&client, input, project_id, session_id, run_id, json_mode).await,
            ChatAction::Repl {
                project_id,
                session_id,
            } => handle_chat_repl(&client, project_id, session_id).await,
        },
        Commands::Plugin { action } => handle_plugin(&client, action, json_mode).await,
        Commands::Team { action } => handle_team(&client, action, json_mode).await,
        Commands::Runner { action } => handle_runner(&client, action, json_mode).await,
    }
}
