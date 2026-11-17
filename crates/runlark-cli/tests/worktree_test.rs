use runlark_cli::WorktreeError;
use runlark_cli::WorktreeManager;
use runlark_domain::RunId;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn init_git_repo(dir: &Path) {
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    };

    run(&["init"]);
    run(&["config", "user.email", "test@runlark.dev"]);
    run(&["config", "user.name", "Runlark Test"]);
    fs::write(dir.join("README.md"), "# Test Repo\n").expect("write readme");
    run(&["add", "README.md"]);
    run(&["commit", "-m", "initial commit"]);
}

#[test]
fn test_worktree_create_reuse_confinement_and_cleanup() {
    let temp = TempDir::new().expect("create temp dir");
    let repo_dir = temp.path();
    init_git_repo(repo_dir);

    let run_id = RunId::from("run_wt_1");

    // 1. Create isolated worktree at <repo>/.runlark/worktrees/<run_id>
    let res1 = WorktreeManager::ensure_worktree(repo_dir, &run_id).expect("create worktree");
    assert!(res1.is_git_worktree);
    assert_eq!(res1.branch_name.as_deref(), Some("runlark/run_wt_1"));
    assert!(res1.worktree_path.ends_with(".runlark/worktrees/run_wt_1"));
    assert!(res1.worktree_path.join("README.md").is_file());

    // 2. Calling ensure_worktree again (e.g. on resume/restart) succeeds idempotently
    let res2 = WorktreeManager::ensure_worktree(repo_dir, &run_id).expect("reuse worktree");
    assert_eq!(res2.worktree_path, res1.worktree_path);
    assert!(res2.is_git_worktree);

    // 3. CWD confinement allows inside paths and rejects traversal escapes
    let inside = WorktreeManager::validate_cwd_confined(
        &res1.worktree_path,
        Path::new("subdir/new_file.txt"),
    )
    .expect("inside path allowed");
    assert!(inside.starts_with(&res1.worktree_path));

    let escape =
        WorktreeManager::validate_cwd_confined(&res1.worktree_path, Path::new("../../outside.txt"));
    assert!(matches!(escape, Err(WorktreeError::PathEscape { .. })));

    // 4. Cleanup removes worktree and optional branch without disturbing main repo
    let removed =
        WorktreeManager::remove_worktree(repo_dir, &run_id, true).expect("remove worktree");
    assert!(removed);
    assert!(!res1.worktree_path.exists());
    assert!(repo_dir.join("README.md").is_file());

    // Idempotent second cleanup returns false
    let removed_again =
        WorktreeManager::remove_worktree(repo_dir, &run_id, true).expect("remove again");
    assert!(!removed_again);
}

#[test]
fn test_worktree_rejects_unsafe_run_id_and_unmanaged_conflicts() {
    let temp = TempDir::new().expect("create temp dir");
    let repo_dir = temp.path();
    init_git_repo(repo_dir);

    assert!(matches!(
        WorktreeManager::ensure_worktree(repo_dir, &RunId::from("../escape")),
        Err(WorktreeError::InvalidRunId(_))
    ));
    assert!(matches!(
        WorktreeManager::ensure_worktree(repo_dir, &RunId::from("a/b")),
        Err(WorktreeError::InvalidRunId(_))
    ));

    // Pre-existing non-worktree directory at target must raise PathConflict
    let conflict_run = RunId::from("run_conflict");
    let target = repo_dir.join(".runlark/worktrees/run_conflict");
    fs::create_dir_all(&target).expect("create conflict dir");
    fs::write(target.join("precious.txt"), "keep").expect("write file");

    assert!(matches!(
        WorktreeManager::ensure_worktree(repo_dir, &conflict_run),
        Err(WorktreeError::PathConflict(_))
    ));
}

#[test]
fn test_worktree_plain_directory_falls_back_cleanly() {
    let temp = TempDir::new().expect("create temp dir");
    let res = WorktreeManager::ensure_worktree(temp.path(), &RunId::from("run_plain"))
        .expect("plain dir should succeed");
    assert!(!res.is_git_worktree);
    assert_eq!(res.repo_root, None);
    assert_eq!(
        res.worktree_path,
        fs::canonicalize(temp.path()).expect("canonicalize")
    );
}
