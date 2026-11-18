use super::harness::{
    accept_invite, bootstrap_user, create_invite, create_ws, get_snapshot, post_command,
    promote_project, sample_task_cmd, spawn_test_server,
};
use reqwest::StatusCode;
use runlark_domain::{
    ArtifactDigest, BudgetPolicy, Decision, DecisionId, DecisionKind, PolicyDigest, ProjectId,
    Revision, Run, RunId, RunStatus, SessionId, Task, TaskId, TaskKind, TaskStage, TeamProjectSpec,
    Timestamp, WorkflowStage, WorkspaceEventEnvelope, WorkspaceId, WorkspaceMember, WorkspaceRole,
    WorkspaceSyncEvent,
};
use serde_json::{Value, json};
use std::process::Command;

#[tokio::test]
async fn test_workspace_rbac_and_last_owner_guard() {
    let srv = spawn_test_server().await;
    let owner_tok = bootstrap_user(&srv, "usr_owner", "Owner").await;
    create_ws(&srv, &owner_tok, "ws_rbac", "ws-rbac").await;
    promote_project(&srv, &owner_tok, "ws_rbac", "proj_rbac").await;

    let whoami = srv
        .http
        .post(format!("{}/api/v1/auth/whoami", srv.base_url))
        .bearer_auth(&owner_tok)
        .send()
        .await
        .expect("whoami");
    assert_eq!(whoami.status(), StatusCode::OK);

    let (_, viewer_inv_tok) =
        create_invite(&srv, &owner_tok, "ws_rbac", "viewer@example.com", "viewer").await;
    let viewer_tok = accept_invite(&srv, &viewer_inv_tok, "usr_viewer", "Viewer").await;

    let members: Vec<WorkspaceMember> = srv
        .http
        .get(format!(
            "{}/api/v1/workspaces/ws_rbac/members",
            srv.base_url
        ))
        .bearer_auth(&viewer_tok)
        .send()
        .await
        .expect("list members")
        .json()
        .await
        .expect("members json");
    assert_eq!(members.len(), 2);

    assert_eq!(
        srv.http
            .get(format!(
                "{}/api/v1/workspaces/ws_rbac/sync/snapshot",
                srv.base_url
            ))
            .bearer_auth(&viewer_tok)
            .send()
            .await
            .expect("viewer snapshot")
            .status(),
        StatusCode::OK
    );

    let denied = sample_task_cmd("cmd_viewer_deny", "task_denied", "proj_rbac", "Denied");
    assert_eq!(
        post_command(&srv, &viewer_tok, "ws_rbac", &denied)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );

    let demote = srv
        .http
        .patch(format!(
            "{}/api/v1/workspaces/ws_rbac/members/usr_owner",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .json(&json!({ "role": WorkspaceRole::Member }))
        .send()
        .await
        .expect("demote last owner");
    assert_eq!(demote.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_invitation_path_accept_revocation_and_takeover_prevention() {
    let srv = spawn_test_server().await;
    let owner_tok = bootstrap_user(&srv, "usr_inv_owner", "Owner").await;
    create_ws(&srv, &owner_tok, "ws_inv", "ws-inv").await;

    // Unauthenticated caller with an invite token MUST NOT claim an existing user_id.
    let (_, hijack_tok) =
        create_invite(&srv, &owner_tok, "ws_inv", "attacker@example.com", "viewer").await;
    let hijack_attempt = srv
        .http
        .post(format!("{}/api/v1/invitations/accept", srv.base_url))
        .json(&json!({
            "invite_token": hijack_tok,
            "user_id": "usr_inv_owner",
            "display_name": "Attacker"
        }))
        .send()
        .await
        .expect("hijack attempt");
    assert_eq!(hijack_attempt.status(), StatusCode::UNAUTHORIZED);

    // Same invite remains usable by a legitimate fresh user via path route.
    let path_accept = srv
        .http
        .post(format!(
            "{}/api/v1/invitations/{hijack_tok}/accept",
            srv.base_url
        ))
        .json(&json!({ "user_id": "usr_path_member", "display_name": "Path Viewer" }))
        .send()
        .await
        .expect("accept by path");
    assert_eq!(path_accept.status(), StatusCode::OK);

    // Authenticated user from ws_inv2 can join ws_inv when authenticated as themselves,
    // but cannot spoof another user_id.
    let existing_tok = bootstrap_user(&srv, "usr_second_ws", "Second").await;
    let (_, auth_inv_tok) =
        create_invite(&srv, &owner_tok, "ws_inv", "second@example.com", "member").await;
    let spoof = srv
        .http
        .post(format!(
            "{}/api/v1/invitations/{auth_inv_tok}/accept",
            srv.base_url
        ))
        .bearer_auth(&existing_tok)
        .json(&json!({ "user_id": "usr_inv_owner", "display_name": "Spoof" }))
        .send()
        .await
        .expect("spoof auth accept");
    assert_eq!(spoof.status(), StatusCode::FORBIDDEN);

    let legit_auth = srv
        .http
        .post(format!(
            "{}/api/v1/invitations/{auth_inv_tok}/accept",
            srv.base_url
        ))
        .bearer_auth(&existing_tok)
        .json(&json!({ "display_name": "Second" }))
        .send()
        .await
        .expect("legit auth accept");
    assert_eq!(legit_auth.status(), StatusCode::OK);

    // Revoked invitation returns 409 Conflict.
    let (rev_id, rev_tok) =
        create_invite(&srv, &owner_tok, "ws_inv", "revoked@example.com", "member").await;
    let revoke_res = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/ws_inv/invitations/{rev_id}/revoke",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .send()
        .await
        .expect("revoke invite");
    assert_eq!(revoke_res.status(), StatusCode::OK);

    let accept_revoked = srv
        .http
        .post(format!(
            "{}/api/v1/invitations/{rev_tok}/accept",
            srv.base_url
        ))
        .json(&json!({ "display_name": "Late" }))
        .send()
        .await
        .expect("accept revoked");
    assert_eq!(accept_revoked.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_solo_to_team_promotion_blocks_active_runs_and_strips_local_paths() {
    let srv = spawn_test_server().await;
    let owner_tok = bootstrap_user(&srv, "usr_promo", "Promoter").await;
    create_ws(&srv, &owner_tok, "ws_promo", "ws-promo").await;

    let project_spec = TeamProjectSpec {
        project_id: ProjectId::from("proj_promo"),
        workspace_id: WorkspaceId::from("ws_promo"),
        name: "SoloRepo".to_string(),
        default_branch: "main".to_string(),
        max_concurrent_runs: 2,
        budget_policy: BudgetPolicy::default(),
        policy_digest: PolicyDigest::from("pol_promo"),
        ownership_epoch: 1,
        revision: Revision(1),
        created_at: Timestamp(100),
        updated_at: Timestamp(100),
    };
    let active_run = Run {
        id: RunId::from("run_active_block"),
        session_id: SessionId::from("sess_b"),
        status: RunStatus::Running,
        worktree_path: "/local/secret/worktree".to_string(),
        project_id: Some(ProjectId::from("proj_promo")),
        task_id: Some(TaskId::from("task_p1")),
        ..Default::default()
    };

    let blocked = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/ws_promo/projects/promote",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .json(
            &json!({ "project": project_spec, "tasks": [], "runs": [active_run], "decisions": [] }),
        )
        .send()
        .await
        .expect("blocked promote");
    assert_eq!(blocked.status(), StatusCode::CONFLICT);

    let done_task = Task {
        id: TaskId::from("task_p1"),
        project_id: ProjectId::from("proj_promo"),
        title: "Past solo task".to_string(),
        description: String::new(),
        kind: TaskKind::QuickFix,
        stage: TaskStage::Done,
        workflow_stage: WorkflowStage::HumanAcceptance,
        plan_text: None,
        plan_version: 1,
        base_ref: Some("main".to_string()),
        head_ref: None,
        pr_url: None,
        active_session_id: None,
        active_run_id: None,
        latest_run_id: Some(RunId::from("run_done_p1")),
        budget_override: None,
        revision: Revision(2),
        created_at: Timestamp(100),
        updated_at: Timestamp(200),
    };
    let completed_run = Run {
        id: RunId::from("run_done_p1"),
        session_id: SessionId::from("sess_done"),
        status: RunStatus::Completed,
        worktree_path: "/local/secret/worktree".to_string(),
        project_id: Some(ProjectId::from("proj_promo")),
        task_id: Some(TaskId::from("task_p1")),
        ..Default::default()
    };

    let ok_promo = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/ws_promo/projects/promote",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .json(&json!({ "project": project_spec, "tasks": [done_task], "runs": [completed_run], "decisions": [] }))
        .send()
        .await
        .expect("promote ok");
    assert_eq!(ok_promo.status(), StatusCode::OK);
    let ack: Value = ok_promo.json().await.expect("ack json");
    assert!(ack["initial_cursor"].as_u64().expect("cursor") >= 2);
    assert_eq!(ack["ownership_epoch"].as_u64(), Some(1));

    let snap = get_snapshot(&srv, &owner_tok, "ws_promo").await;
    assert_eq!(snap.projects.len(), 1);
    assert_eq!(snap.runs[0].run.worktree_path, "");
}

fn sample_promoted_bundle() -> (TeamProjectSpec, Task, Run, Decision) {
    let project_spec = TeamProjectSpec {
        project_id: ProjectId::from("proj_sync_promo"),
        workspace_id: WorkspaceId::from("ws_sync_promo"),
        name: "SoloRepo".to_string(),
        default_branch: "main".to_string(),
        max_concurrent_runs: 2,
        budget_policy: BudgetPolicy::default(),
        policy_digest: PolicyDigest::from("pol_promo"),
        ownership_epoch: 1,
        revision: Revision(1),
        created_at: Timestamp(100),
        updated_at: Timestamp(100),
    };
    let done_task = Task {
        id: TaskId::from("task_sp1"),
        project_id: ProjectId::from("proj_sync_promo"),
        title: "Promoted task".to_string(),
        description: String::new(),
        kind: TaskKind::QuickFix,
        stage: TaskStage::Done,
        workflow_stage: WorkflowStage::HumanAcceptance,
        plan_text: None,
        plan_version: 1,
        base_ref: Some("main".to_string()),
        head_ref: None,
        pr_url: None,
        active_session_id: None,
        active_run_id: None,
        latest_run_id: Some(RunId::from("run_sp1")),
        budget_override: None,
        revision: Revision(2),
        created_at: Timestamp(100),
        updated_at: Timestamp(200),
    };
    let completed_run = Run {
        id: RunId::from("run_sp1"),
        session_id: SessionId::from("sess_sp1"),
        status: RunStatus::Completed,
        worktree_path: "/local/secret/worktree".to_string(),
        project_id: Some(ProjectId::from("proj_sync_promo")),
        task_id: Some(TaskId::from("task_sp1")),
        ..Default::default()
    };
    let decision = Decision {
        id: DecisionId::from("dec_sp1"),
        task_id: TaskId::from("task_sp1"),
        run_id: RunId::from("run_sp1"),
        kind: DecisionKind::Accept,
        actor: "Promoter".to_string(),
        plan_version: 1,
        head_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        base_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        policy_digest: PolicyDigest::from("pol_promo"),
        evidence_digest: ArtifactDigest::parse("a".repeat(64)).expect("digest"),
        comment: Some("Approved before promotion".to_string()),
        created_at: Timestamp(250),
    };
    (project_spec, done_task, completed_run, decision)
}

#[tokio::test]
async fn test_solo_to_team_promotion_emits_run_and_decision_events_and_validates_project_id() {
    let srv = spawn_test_server().await;
    let owner_tok = bootstrap_user(&srv, "usr_sp", "Promoter").await;
    create_ws(&srv, &owner_tok, "ws_sync_promo", "ws-sync-promo").await;
    let cursor_before = get_snapshot(&srv, &owner_tok, "ws_sync_promo")
        .await
        .cursor_seq
        .0;

    let (project_spec, done_task, completed_run, decision) = sample_promoted_bundle();
    let mut wrong_project_run = completed_run.clone();
    wrong_project_run.project_id = Some(ProjectId::from("proj_other"));
    let bad_promo = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/ws_sync_promo/projects/promote",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .json(&json!({
            "project": project_spec,
            "tasks": [done_task],
            "runs": [wrong_project_run],
            "decisions": [decision]
        }))
        .send()
        .await
        .expect("bad promote");
    assert_eq!(bad_promo.status(), StatusCode::BAD_REQUEST);

    let ok_promo = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/ws_sync_promo/projects/promote",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .json(&json!({
            "project": project_spec,
            "tasks": [done_task],
            "runs": [completed_run],
            "decisions": [decision]
        }))
        .send()
        .await
        .expect("promote ok");
    assert_eq!(ok_promo.status(), StatusCode::OK);
    let ack: Value = ok_promo.json().await.expect("ack json");
    let initial_cursor = ack["initial_cursor"].as_u64().expect("initial_cursor");

    let snap_after = get_snapshot(&srv, &owner_tok, "ws_sync_promo").await;
    assert_eq!(initial_cursor, snap_after.cursor_seq.0);

    let events: Vec<WorkspaceEventEnvelope> = srv
        .http
        .get(format!(
            "{}/api/v1/workspaces/ws_sync_promo/sync/events?after_cursor={cursor_before}",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .send()
        .await
        .expect("delta events")
        .json()
        .await
        .expect("events json");
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event, WorkspaceSyncEvent::ProjectPromoted { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event, WorkspaceSyncEvent::TaskUpserted { .. }))
    );
    assert!(events.iter().any(|e| matches!(
        &e.event,
        WorkspaceSyncEvent::RunUpserted { run, .. } if run.id == RunId::from("run_sp1") && run.worktree_path.is_empty()
    )));
    assert!(events.iter().any(|e| matches!(
        &e.event,
        WorkspaceSyncEvent::DecisionRecorded { decision } if decision.id == DecisionId::from("dec_sp1")
    )));
}

#[tokio::test]
async fn test_bootstrap_owner_cli_creates_owner_and_workspace() {
    let srv = spawn_test_server().await;
    let exe = env!("CARGO_BIN_EXE_runlark-server");
    let output = Command::new(exe)
        .args([
            "bootstrap-owner",
            "--postgres-url",
            &srv.test_pg_url,
            "--user-id",
            "usr_cli_owner",
            "--display-name",
            "CLI Owner",
            "--workspace-id",
            "ws_cli_boot",
            "--workspace-name",
            "CLI Workspace",
            "--workspace-slug",
            "cli-workspace",
        ])
        .output()
        .expect("run bootstrap-owner cli");
    assert!(
        output.status.success(),
        "bootstrap-owner failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let parsed: Value = serde_json::from_str(&stdout_str)
        .unwrap_or_else(|e| panic!("parse bootstrap-owner json failed ({e}): {stdout_str:?}"));
    assert_eq!(parsed["workspace_id"].as_str(), Some("ws_cli_boot"));
    let token = parsed["bearer_token"].as_str().expect("bearer_token");

    let ws_res = srv
        .http
        .get(format!("{}/api/v1/workspaces/ws_cli_boot", srv.base_url))
        .bearer_auth(token)
        .send()
        .await
        .expect("get bootstrapped workspace");
    assert_eq!(ws_res.status(), StatusCode::OK);
}

async fn patch_member_role(
    srv: &super::harness::TestServer,
    token: &str,
    wid: &str,
    uid: &str,
    role: WorkspaceRole,
) -> StatusCode {
    srv.http
        .patch(format!(
            "{}/api/v1/workspaces/{wid}/members/{uid}",
            srv.base_url
        ))
        .bearer_auth(token)
        .json(&json!({ "role": role }))
        .send()
        .await
        .expect("patch member role")
        .status()
}

#[tokio::test]
async fn test_maintainer_cannot_modify_owner_even_with_multiple_owners() {
    let srv = spawn_test_server().await;
    let owner1_tok = bootstrap_user(&srv, "usr_own1", "Owner 1").await;
    create_ws(&srv, &owner1_tok, "ws_hier", "ws-hier").await;

    let (_, own2_inv) = create_invite(&srv, &owner1_tok, "ws_hier", "own2@ex.com", "member").await;
    let _owner2_tok = accept_invite(&srv, &own2_inv, "usr_own2", "Owner 2").await;
    assert_eq!(
        patch_member_role(
            &srv,
            &owner1_tok,
            "ws_hier",
            "usr_own2",
            WorkspaceRole::Owner
        )
        .await,
        StatusCode::OK
    );

    let (_, maint_inv) =
        create_invite(&srv, &owner1_tok, "ws_hier", "maint@ex.com", "maintainer").await;
    let maint_tok = accept_invite(&srv, &maint_inv, "usr_maint", "Maintainer").await;

    let (_, mem_inv) = create_invite(&srv, &owner1_tok, "ws_hier", "mem@ex.com", "member").await;
    let _mem_tok = accept_invite(&srv, &mem_inv, "usr_mem", "Member").await;

    // Maintainer MUST NOT demote or modify an Owner even when owner_count > 1.
    for role in [WorkspaceRole::Member, WorkspaceRole::Owner] {
        assert_eq!(
            patch_member_role(&srv, &maint_tok, "ws_hier", "usr_own2", role).await,
            StatusCode::FORBIDDEN
        );
    }

    // Maintainer MUST NOT grant Owner role to a non-Owner.
    assert_eq!(
        patch_member_role(&srv, &maint_tok, "ws_hier", "usr_mem", WorkspaceRole::Owner).await,
        StatusCode::FORBIDDEN
    );

    // Maintainer CAN manage lower roles (Member -> Viewer).
    assert_eq!(
        patch_member_role(
            &srv,
            &maint_tok,
            "ws_hier",
            "usr_mem",
            WorkspaceRole::Viewer
        )
        .await,
        StatusCode::OK
    );

    // Owner CAN demote another Owner when owner_count > 1, and revoke Maintainer.
    assert_eq!(
        patch_member_role(
            &srv,
            &owner1_tok,
            "ws_hier",
            "usr_own2",
            WorkspaceRole::Member
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        patch_member_role(
            &srv,
            &owner1_tok,
            "ws_hier",
            "usr_maint",
            WorkspaceRole::Member
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        patch_member_role(
            &srv,
            &maint_tok,
            "ws_hier",
            "usr_mem",
            WorkspaceRole::Member
        )
        .await,
        StatusCode::FORBIDDEN
    );
}
