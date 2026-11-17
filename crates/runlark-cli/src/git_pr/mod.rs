mod pr_service;

pub use pr_service::PrCreationService;

use runlark_domain::{ArtifactKind, ArtifactRef, CommandId, RunId, Timestamp};
use runlark_local::{ArtifactCasStore, StorageError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::process::Command;
use std::time::Duration;
use thiserror::Error;

/// Real git diff and SHA snapshot computed from a run's isolated worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitDiffSnapshot {
    pub base_sha: String,
    pub head_sha: String,
    pub branch_name: String,
    pub changed_files: Vec<String>,
    pub diff_text: String,
    pub diff_bytes: u64,
}

/// Errors raised during git diffing or PR creation.
#[derive(Debug, Error)]
pub enum GitPrError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("git/pr command failed: {0}")]
    CommandFailed(String),

    #[error("ambiguous external PR outcome: {0}")]
    UnknownOutcome(String),
}

/// Output captured from executing `git` or `gh` during PR creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrCommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Failure modes from a [`PrCommandRunner`] invocation.
#[derive(Debug, Error)]
pub enum PrCommandError {
    #[error("command failed cleanly: {0}")]
    Failed(String),

    #[error("ambiguous/unknown external execution outcome: {0}")]
    Unknown(String),
}

/// Injectable runner abstraction for `git push` and `gh pr create` operations.
pub trait PrCommandRunner: Send + Sync {
    fn run_git<'a>(
        &'a self,
        cwd: &'a Path,
        args: &'a [&'a str],
    ) -> Pin<Box<dyn Future<Output = Result<PrCommandOutput, PrCommandError>> + Send + 'a>>;

    fn run_gh<'a>(
        &'a self,
        cwd: &'a Path,
        args: &'a [&'a str],
    ) -> Pin<Box<dyn Future<Output = Result<PrCommandOutput, PrCommandError>> + Send + 'a>>;
}

/// Production [`PrCommandRunner`] invoking real `git` and `gh` binaries with bounded timeout.
#[derive(Debug, Clone)]
pub struct SystemPrRunner {
    pub timeout: Duration,
}

impl Default for SystemPrRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
        }
    }
}

impl SystemPrRunner {
    async fn exec_binary(
        &self,
        bin: &str,
        cwd: &Path,
        args: &[&str],
    ) -> Result<PrCommandOutput, PrCommandError> {
        let mut cmd = tokio::process::Command::new(bin);
        if bin == "git" {
            cmd.arg("-C").arg(cwd);
        } else {
            cmd.current_dir(cwd);
        }
        cmd.args(args);
        cmd.kill_on_drop(true);

        let output = match tokio::time::timeout(self.timeout, cmd.output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(err)) => {
                return Err(PrCommandError::Failed(format!(
                    "failed to spawn `{bin}`: {err}"
                )));
            }
            Err(_) => {
                return Err(PrCommandError::Unknown(format!(
                    "`{bin} {}` timed out after {:?}",
                    args.join(" "),
                    self.timeout
                )));
            }
        };

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if output.status.success() {
            Ok(PrCommandOutput {
                exit_code,
                stdout,
                stderr,
            })
        } else if stderr.contains("502")
            || stderr.contains("504")
            || stderr.contains("timed out")
            || stderr.contains("connection reset")
        {
            Err(PrCommandError::Unknown(format!(
                "`{bin} {}` failed ambiguously (exit {exit_code}): {}",
                args.join(" "),
                stderr.trim()
            )))
        } else {
            Err(PrCommandError::Failed(format!(
                "`{bin} {}` failed (exit {exit_code}): {}",
                args.join(" "),
                stderr.trim()
            )))
        }
    }
}

impl PrCommandRunner for SystemPrRunner {
    fn run_git<'a>(
        &'a self,
        cwd: &'a Path,
        args: &'a [&'a str],
    ) -> Pin<Box<dyn Future<Output = Result<PrCommandOutput, PrCommandError>> + Send + 'a>> {
        Box::pin(async move { self.exec_binary("git", cwd, args).await })
    }

    fn run_gh<'a>(
        &'a self,
        cwd: &'a Path,
        args: &'a [&'a str],
    ) -> Pin<Box<dyn Future<Output = Result<PrCommandOutput, PrCommandError>> + Send + 'a>> {
        Box::pin(async move { self.exec_binary("gh", cwd, args).await })
    }
}

/// Request parameters for creating a PR from an accepted run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePrRequest {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub title: Option<String>,
    pub body: Option<String>,
    pub base_branch: Option<String>,
    pub draft: bool,
    pub expected_revision: Option<u64>,
}

/// Result of attempting PR creation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CreatePrOutcome {
    Created {
        command_id: String,
        run_id: String,
        branch: String,
        pr_url: String,
        receipt_digest: String,
        replayed: bool,
    },
    Unknown {
        command_id: String,
        run_id: String,
        reason: String,
    },
}

/// Computes real git diff, `base_sha`, `head_sha`, branch name, and changed files inside `worktree`.
///
/// # Errors
/// Returns [`GitPrError::CommandFailed`] if `worktree` is not a valid git repository.
pub fn compute_worktree_diff(
    worktree: &Path,
    base_sha_hint: Option<&str>,
) -> Result<GitDiffSnapshot, GitPrError> {
    if !worktree.is_dir() {
        return Err(GitPrError::CommandFailed(format!(
            "worktree directory does not exist: {}",
            worktree.display()
        )));
    }
    let head_sha = git_stdout(worktree, &["rev-parse", "HEAD"])?;
    let branch_name = git_stdout(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_else(|_| "HEAD".to_string());

    let base_sha = match base_sha_hint.map(str::trim).filter(|s| !s.is_empty()) {
        Some(hint) => git_stdout(worktree, &["rev-parse", "--verify", hint])?,
        None => head_sha.clone(),
    };

    let diff_out = git_stdout(worktree, &["diff", &base_sha])?;
    let tracked_changed = git_stdout(worktree, &["diff", "--name-only", &base_sha])?;
    let untracked = git_stdout(
        worktree,
        &[
            "-c",
            "core.quotePath=false",
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )
    .unwrap_or_default();

    let mut files = BTreeSet::new();
    for line in tracked_changed.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            files.insert(trimmed.to_string());
        }
    }

    let mut diff_text = diff_out;
    for path in untracked
        .split('\0')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Ok(content) = std::fs::read_to_string(worktree.join(path)) {
            files.insert(path.to_string());
            if !diff_text.is_empty() && !diff_text.ends_with('\n') {
                diff_text.push('\n');
            }
            let line_count = content.lines().count();
            let hunk_header = if line_count == 0 {
                "@@ -0,0 +0,0 @@".to_string()
            } else {
                format!("@@ -0,0 +1,{line_count} @@")
            };
            let _ = writeln!(
                diff_text,
                "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n{hunk_header}"
            );
            for line in content.lines() {
                diff_text.push('+');
                diff_text.push_str(line);
                diff_text.push('\n');
            }
        }
    }

    let diff_bytes = u64::try_from(diff_text.len()).unwrap_or(u64::MAX);
    Ok(GitDiffSnapshot {
        base_sha,
        head_sha,
        branch_name,
        changed_files: files.into_iter().collect(),
        diff_text,
        diff_bytes,
    })
}

/// Stores a [`GitDiffSnapshot`] diff payload into [`ArtifactCasStore`] and links it to `run_id`.
///
/// # Errors
/// Returns [`StorageError`] if CAS persistence or `SQLite` linking fails.
pub fn store_diff_in_cas(
    cas: &ArtifactCasStore,
    conn: &runlark_local::Connection,
    run_id: &RunId,
    snapshot: &GitDiffSnapshot,
    now: Timestamp,
) -> Result<ArtifactRef, StorageError> {
    let artifact = cas.put_bytes(
        conn,
        ArtifactKind::Diff,
        "text/x-diff",
        snapshot.diff_text.as_bytes(),
        now,
    )?;
    ArtifactCasStore::link_to_run(conn, run_id, &artifact.digest_sha256, "diff", now)?;
    Ok(artifact)
}

pub(super) fn git_stdout(cwd: &Path, args: &[&str]) -> Result<String, GitPrError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(|e| GitPrError::CommandFailed(format!("failed to spawn git: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GitPrError::CommandFailed(format!(
            "git {} failed: {stderr}",
            args.join(" ")
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .into_owned()
        .trim()
        .to_string())
}
