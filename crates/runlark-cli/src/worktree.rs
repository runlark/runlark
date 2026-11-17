use runlark_domain::RunId;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use thiserror::Error;

/// Errors raised during git worktree creation, validation, or cleanup.
#[derive(Debug, Error)]
pub enum WorktreeError {
    #[error("invalid run_id for worktree path: {0}")]
    InvalidRunId(String),

    #[error("workspace directory does not exist: {0}")]
    WorkspaceNotFound(PathBuf),

    #[error("worktree target path exists and is not a registered worktree: {0}")]
    PathConflict(PathBuf),

    #[error("path escapes confined worktree {worktree}: {target}")]
    PathEscape { worktree: PathBuf, target: PathBuf },

    #[error("git command `{command}` failed: {stderr}")]
    GitFailed { command: String, stderr: String },

    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result of resolving or creating a run's execution directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeResolution {
    pub worktree_path: PathBuf,
    pub repo_root: Option<PathBuf>,
    pub is_git_worktree: bool,
    pub branch_name: Option<String>,
}

/// Manages isolated per-run git worktrees under `<workspace>/.runlark/worktrees/<run_id>`.
#[derive(Debug, Default, Clone, Copy)]
pub struct WorktreeManager;

impl WorktreeManager {
    /// Validates that `run_id` is a single safe path component with no traversal segments.
    ///
    /// # Errors
    /// Returns `WorktreeError::InvalidRunId` if `run_id` is empty or contains path separators.
    pub fn validate_run_id(run_id: &RunId) -> Result<(), WorktreeError> {
        let raw = run_id.as_ref();
        if raw.is_empty() || raw.contains(['/', '\\', '\0']) {
            return Err(WorktreeError::InvalidRunId(raw.to_string()));
        }
        let mut components = Path::new(raw).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(_)), None) => Ok(()),
            _ => Err(WorktreeError::InvalidRunId(raw.to_string())),
        }
    }

    /// Returns the deterministic branch name for a run's git worktree.
    #[must_use]
    pub fn branch_name(run_id: &RunId) -> String {
        format!("runlark/{}", run_id.as_ref())
    }

    /// Computes `<repo_root>/.runlark/worktrees/<run_id>`.
    ///
    /// # Errors
    /// Returns `WorktreeError::InvalidRunId` if `run_id` is unsafe.
    pub fn worktree_dir(repo_root: &Path, run_id: &RunId) -> Result<PathBuf, WorktreeError> {
        Self::validate_run_id(run_id)?;
        Ok(repo_root
            .join(".runlark")
            .join("worktrees")
            .join(run_id.as_ref()))
    }

    /// Ensures an isolated worktree exists for `run_id`.
    ///
    /// If `workspace` is inside a git repository with a `HEAD` commit, creates (or reuses
    /// idempotently on resume/restart) a worktree at `<repo_root>/.runlark/worktrees/<run_id>`.
    /// If `workspace` is a plain non-git directory, returns its canonical path without failing.
    ///
    /// # Errors
    /// Returns `WorktreeError` if `run_id` is invalid, `workspace` does not exist, an unmanaged
    /// directory conflicts at the worktree target, or git worktree creation fails.
    pub fn ensure_worktree(
        workspace: &Path,
        run_id: &RunId,
    ) -> Result<WorktreeResolution, WorktreeError> {
        Self::validate_run_id(run_id)?;
        if !workspace.is_dir() {
            return Err(WorktreeError::WorkspaceNotFound(workspace.to_path_buf()));
        }
        let canonical_workspace = fs::canonicalize(workspace)?;

        let Some(repo_root) = Self::detect_repo_root(&canonical_workspace)? else {
            return Ok(WorktreeResolution {
                worktree_path: canonical_workspace,
                repo_root: None,
                is_git_worktree: false,
                branch_name: None,
            });
        };

        let target = Self::worktree_dir(&repo_root, run_id)?;
        let branch = Self::branch_name(run_id);

        if target.exists() {
            if Self::is_registered_worktree(&repo_root, &target)? {
                let canonical_target = fs::canonicalize(&target)?;
                return Ok(WorktreeResolution {
                    worktree_path: canonical_target,
                    repo_root: Some(repo_root),
                    is_git_worktree: true,
                    branch_name: Some(branch),
                });
            }
            return Err(WorktreeError::PathConflict(target));
        }

        // Verify the repository has at least one commit before adding a worktree.
        Self::run_git(&repo_root, &["rev-parse", "--verify", "HEAD"])?;

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        Self::run_git(&repo_root, &["worktree", "prune"])?;

        let branch_ref = format!("refs/heads/{branch}");
        let target_str = target.to_string_lossy();
        if Self::git_succeeds(
            &repo_root,
            &["show-ref", "--verify", "--quiet", &branch_ref],
        ) {
            Self::run_git(&repo_root, &["worktree", "add", &target_str, &branch])?;
        } else {
            Self::run_git(
                &repo_root,
                &["worktree", "add", "-b", &branch, &target_str, "HEAD"],
            )?;
        }

        let canonical_target = fs::canonicalize(&target)?;
        Ok(WorktreeResolution {
            worktree_path: canonical_target,
            repo_root: Some(repo_root),
            is_git_worktree: true,
            branch_name: Some(branch),
        })
    }

    /// Removes a run's isolated git worktree (and optionally its branch) during cleanup/archive.
    ///
    /// Returns `Ok(true)` if a worktree directory was removed, or `Ok(false)` if none existed or
    /// `workspace` is not a git repository.
    ///
    /// # Errors
    /// Returns `WorktreeError` if `run_id` is invalid, `workspace` is missing, or git removal fails.
    pub fn remove_worktree(
        workspace: &Path,
        run_id: &RunId,
        delete_branch: bool,
    ) -> Result<bool, WorktreeError> {
        Self::validate_run_id(run_id)?;
        if !workspace.is_dir() {
            return Err(WorktreeError::WorkspaceNotFound(workspace.to_path_buf()));
        }
        let canonical_workspace = fs::canonicalize(workspace)?;
        let Some(repo_root) = Self::detect_repo_root(&canonical_workspace)? else {
            return Ok(false);
        };

        let target = Self::worktree_dir(&repo_root, run_id)?;
        let branch = Self::branch_name(run_id);
        let mut removed = false;

        if target.exists() {
            let target_str = target.to_string_lossy();
            Self::run_git(&repo_root, &["worktree", "remove", "--force", &target_str])?;
            removed = true;
        }
        Self::run_git(&repo_root, &["worktree", "prune"])?;

        if delete_branch {
            let branch_ref = format!("refs/heads/{branch}");
            if Self::git_succeeds(
                &repo_root,
                &["show-ref", "--verify", "--quiet", &branch_ref],
            ) {
                Self::run_git(&repo_root, &["branch", "-D", &branch])?;
            }
        }

        Ok(removed)
    }

    /// Validates that `candidate` resolves inside `worktree_root` without escaping via `..` or symlinks.
    ///
    /// # Errors
    /// Returns `WorktreeError::PathEscape` if `candidate` escapes `worktree_root`,
    /// or `WorktreeError::Io` if canonicalization fails.
    pub fn validate_cwd_confined(
        worktree_root: &Path,
        candidate: &Path,
    ) -> Result<PathBuf, WorktreeError> {
        let canonical_root = fs::canonicalize(worktree_root)?;
        let joined = if candidate.is_relative() {
            canonical_root.join(candidate)
        } else {
            candidate.to_path_buf()
        };

        let resolved = Self::canonicalize_with_nonexistent_tail(&joined)?;
        if !resolved.starts_with(&canonical_root) {
            return Err(WorktreeError::PathEscape {
                worktree: canonical_root,
                target: resolved,
            });
        }
        Ok(resolved)
    }

    fn detect_repo_root(dir: &Path) -> Result<Option<PathBuf>, WorktreeError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "--show-toplevel"])
            .output()?;
        if !output.status.success() {
            return Ok(None);
        }
        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if raw.is_empty() {
            return Ok(None);
        }
        Ok(Some(fs::canonicalize(PathBuf::from(raw))?))
    }

    fn is_registered_worktree(repo_root: &Path, target: &Path) -> Result<bool, WorktreeError> {
        if !target.is_dir() {
            return Ok(false);
        }
        let canonical_target = fs::canonicalize(target)?;
        let Some(wt_top) = Self::detect_repo_root(&canonical_target)? else {
            return Ok(false);
        };
        if wt_top != canonical_target {
            return Ok(false);
        }

        let list_out = Self::run_git(repo_root, &["worktree", "list", "--porcelain"])?;
        for line in list_out.lines() {
            if let Some(path_str) = line.strip_prefix("worktree ") {
                let candidate = PathBuf::from(path_str.trim());
                if candidate.exists()
                    && let Ok(canon) = fs::canonicalize(&candidate)
                    && canon == canonical_target
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn canonicalize_with_nonexistent_tail(path: &Path) -> Result<PathBuf, WorktreeError> {
        if path.exists() {
            return Ok(fs::canonicalize(path)?);
        }
        let mut ancestor = path;
        let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
        while !ancestor.exists() {
            let Some(name) = ancestor.file_name() else {
                break;
            };
            tail.push(name);
            let Some(parent) = ancestor.parent() else {
                break;
            };
            ancestor = parent;
        }
        let mut base = if ancestor.exists() {
            fs::canonicalize(ancestor)?
        } else {
            ancestor.to_path_buf()
        };
        for seg in tail.into_iter().rev() {
            if seg == ".." {
                base.pop();
            } else if seg != "." {
                base.push(seg);
            }
        }
        Ok(base)
    }

    fn git_succeeds(cwd: &Path, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .is_ok_and(|out| out.status.success())
    }

    fn run_git(cwd: &Path, args: &[&str]) -> Result<String, WorktreeError> {
        let output = Command::new("git").arg("-C").arg(cwd).args(args).output()?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(WorktreeError::GitFailed {
            command: format!("git {}", args.join(" ")),
            stderr,
        })
    }
}
