use super::harness::{
    TestServer, accept_invite, bootstrap_user, create_invite, create_ws, get_snapshot,
    post_command, promote_project, sample_run_cmd, sample_task_cmd, spawn_test_server,
};
use reqwest::StatusCode;
use runlark_domain::{
    CancelTeamRunCommand, CommandId, EventSeq, ForceCancelUnreachableRunCommand, LeaseEpoch,
    Revision, RunId, RunStatus, RunnerEventInput, RunnerId, RunnerRunReport, RunnerStatus,
    TeamCommand, TeamRunEvent, Timestamp, WorkspaceEventEnvelope, WorkspaceSyncEvent,
};
use serde_json::json;

async fn setup_owned_run_pair(srv: &TestServer, wid: &str, pid: &str) -> (String, String) {
    let owner_tok = bootstrap_user(srv, "usr_alice", "Alice").await;
    create_ws(srv, &owner_tok, wid, wid).await;
    promote_project(srv, &owner_tok, wid, pid).await;

    let (_, inv_tok) = create_invite(srv, &owner_tok, wid, "bob", "member").await;
    let bob_tok = accept_invite(srv, &inv_tok, "usr_bob", "Bob").await;

    for (tok, rid, name) in [
        (&owner_tok, "runner_a", "Alice Mac"),
        (&bob_tok, "runner_b", "Bob Linux"),
    ] {
        let r = srv
            .http
            .post(format!(
                "{}/api/v1/workspaces/{wid}/runners/register",
                srv.base_url
            ))
            .bearer_auth(tok)
            .json(&json!({ "runner_id": rid, "name": name }))
            .send()
            .await
            .expect("register runner");
        assert_eq!(r.status(), StatusCode::OK);
    }

    assert_eq!(
        post_command(
            srv,
            &owner_tok,
            wid,
            &sample_task_cmd("cmd_own_t1", "task_own_1", pid, "Owned task")
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        post_command(
            srv,
            &owner_tok,
            wid,
            &sample_run_cmd(
                "cmd_own_r1",
                "run_own_1",
                pid,
                "task_own_1",
                Some("runner_a")
            )
        )
        .await
        .status(),
        StatusCode::CREATED
    );

    (owner_tok, bob_tok)
}

async fn post_runner_report(
    srv: &TestServer,
    token: &str,
    wid: &str,
    run_id: &str,
    report: &RunnerRunReport,
) -> StatusCode {
    srv.http
        .post(format!(
            "{}/api/v1/workspaces/{wid}/runs/{run_id}/runner-state",
            srv.base_url
        ))
        .bearer_auth(token)
        .json(report)
        .send()
        .await
        .expect("runner-state")
        .status()
}

async fn post_sweep(srv: &TestServer, token: &str, wid: &str) -> StatusCode {
    srv.http
        .post(format!(
            "{}/api/v1/workspaces/{wid}/runners/sweep",
            srv.base_url
        ))
        .bearer_auth(token)
        .json(&json!({ "timeout_ms": 0 }))
        .send()
        .await
        .expect("sweep")
        .status()
}

#[tokio::test]
async fn test_runner_fencing_and_run_event_dedup() {
    let srv = spawn_test_server().await;
    let (owner_tok, bob_tok) = setup_owned_run_pair(&srv, "ws_fence", "proj_fence").await;

    let takeover = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/ws_fence/runs/run_own_1/assign",
            srv.base_url
        ))
        .bearer_auth(&bob_tok)
        .json(&json!({ "runner_id": "runner_b", "expected_revision": 2 }))
        .send()
        .await
        .expect("takeover attempt");
    assert_eq!(takeover.status(), StatusCode::CONFLICT);

    let bad_report = RunnerRunReport {
        runner_id: RunnerId::from("runner_b"),
        lease_epoch: LeaseEpoch(1),
        status: None,
        workflow_stage: None,
        head_sha: None,
        base_sha: None,
        events: vec![],
    };
    assert_eq!(
        post_runner_report(&srv, &bob_tok, "ws_fence", "run_own_1", &bad_report).await,
        StatusCode::CONFLICT
    );

    let report = RunnerRunReport {
        runner_id: RunnerId::from("runner_a"),
        lease_epoch: LeaseEpoch(1),
        status: None,
        workflow_stage: None,
        head_sha: Some("1111111111111111111111111111111111111111".to_string()),
        base_sha: Some("0000000000000000000000000000000000000000".to_string()),
        events: vec![RunnerEventInput {
            seq: EventSeq(1),
            event_type: "turn_started".to_string(),
            payload_json: "{\"step\":1}".to_string(),
            created_at: Timestamp(200),
        }],
    };
    for _ in 0..2 {
        assert_eq!(
            post_runner_report(&srv, &owner_tok, "ws_fence", "run_own_1", &report).await,
            StatusCode::OK
        );
    }

    let run_events: Vec<TeamRunEvent> = srv
        .http
        .get(format!(
            "{}/api/v1/workspaces/ws_fence/runs/run_own_1/events",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .send()
        .await
        .expect("run events")
        .json()
        .await
        .expect("events json");
    assert_eq!(run_events.len(), 1);
}

#[tokio::test]
async fn test_cancel_requested_transition_and_terminal_run_guards() {
    let srv = spawn_test_server().await;
    let (owner_tok, _) = setup_owned_run_pair(&srv, "ws_cguard", "proj_cguard").await;

    let cancel = TeamCommand::CancelRun(CancelTeamRunCommand {
        command_id: CommandId::from("cmd_cg_cancel"),
        run_id: RunId::from("run_own_1"),
        reason: "stop".to_string(),
        expected_revision: Revision(2),
        created_at: Timestamp(210),
    });
    assert_eq!(
        post_command(&srv, &owner_tok, "ws_cguard", &cancel)
            .await
            .status(),
        StatusCode::CREATED
    );

    // Runner cannot move CancelRequested back to Running.
    let revert = RunnerRunReport {
        runner_id: RunnerId::from("runner_a"),
        lease_epoch: LeaseEpoch(1),
        status: Some(RunStatus::Running),
        workflow_stage: None,
        head_sha: None,
        base_sha: None,
        events: vec![],
    };
    assert_eq!(
        post_runner_report(&srv, &owner_tok, "ws_cguard", "run_own_1", &revert).await,
        StatusCode::CONFLICT
    );

    // Settling CancelRequested -> Stopped clears task.active_run_id and bumps task revision.
    let settle = RunnerRunReport {
        runner_id: RunnerId::from("runner_a"),
        lease_epoch: LeaseEpoch(1),
        status: Some(RunStatus::Stopped),
        workflow_stage: None,
        head_sha: None,
        base_sha: None,
        events: vec![],
    };
    assert_eq!(
        post_runner_report(&srv, &owner_tok, "ws_cguard", "run_own_1", &settle).await,
        StatusCode::OK
    );

    // Terminal run rejects late event appends or SHA mutations.
    let late = RunnerRunReport {
        runner_id: RunnerId::from("runner_a"),
        lease_epoch: LeaseEpoch(1),
        status: None,
        workflow_stage: None,
        head_sha: Some("2222222222222222222222222222222222222222".to_string()),
        base_sha: None,
        events: vec![RunnerEventInput {
            seq: EventSeq(2),
            event_type: "late".to_string(),
            payload_json: "{}".to_string(),
            created_at: Timestamp(300),
        }],
    };
    assert_eq!(
        post_runner_report(&srv, &owner_tok, "ws_cguard", "run_own_1", &late).await,
        StatusCode::CONFLICT
    );

    let snap = get_snapshot(&srv, &owner_tok, "ws_cguard").await;
    assert_eq!(snap.tasks[0].active_run_id, None);
    assert!(snap.tasks[0].revision.0 >= 3);
}

#[tokio::test]
async fn test_unreachable_no_auto_takeover_sweep_rbac_and_task_sync() {
    let srv = spawn_test_server().await;
    let (owner_tok, _) = setup_owned_run_pair(&srv, "ws_unreach", "proj_unreach").await;

    let (_, viewer_inv) =
        create_invite(&srv, &owner_tok, "ws_unreach", "v@example.com", "viewer").await;
    let viewer_tok = accept_invite(&srv, &viewer_inv, "usr_unreach_viewer", "Viewer").await;
    assert_eq!(
        post_sweep(&srv, &viewer_tok, "ws_unreach").await,
        StatusCode::FORBIDDEN
    );

    let cancel_online = TeamCommand::CancelRun(CancelTeamRunCommand {
        command_id: CommandId::from("cmd_cancel_online"),
        run_id: RunId::from("run_own_1"),
        reason: "user cancel".to_string(),
        expected_revision: Revision(2),
        created_at: Timestamp(210),
    });
    assert_eq!(
        post_command(&srv, &owner_tok, "ws_unreach", &cancel_online)
            .await
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        get_snapshot(&srv, &owner_tok, "ws_unreach").await.runs[0]
            .run
            .status,
        RunStatus::CancelRequested
    );

    assert_eq!(
        post_sweep(&srv, &owner_tok, "ws_unreach").await,
        StatusCode::OK
    );
    let snap1 = get_snapshot(&srv, &owner_tok, "ws_unreach").await;
    assert_eq!(
        snap1.runs[0].ownership.assigned_runner_id,
        Some(RunnerId::from("runner_a"))
    );
    assert!(snap1.runs[0].ownership.runner_unreachable);

    let hb = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/ws_unreach/runners/runner_a/heartbeat",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .send()
        .await
        .expect("heartbeat");
    assert_eq!(hb.status(), StatusCode::OK);

    assert_eq!(
        post_sweep(&srv, &owner_tok, "ws_unreach").await,
        StatusCode::OK
    );
    let snap2 = get_snapshot(&srv, &owner_tok, "ws_unreach").await;
    let task_rev_before = snap2.tasks[0].revision.0;
    let cursor_before = snap2.cursor_seq.0;

    let force_cancel = TeamCommand::ForceCancelUnreachableRun(ForceCancelUnreachableRunCommand {
        command_id: CommandId::from("cmd_cancel_unreach"),
        run_id: RunId::from("run_own_1"),
        reason: "runner offline".to_string(),
        expected_revision: snap2.runs[0].run.revision,
        created_at: Timestamp(300),
    });
    assert_eq!(
        post_command(&srv, &owner_tok, "ws_unreach", &force_cancel)
            .await
            .status(),
        StatusCode::CREATED
    );

    let snap3 = get_snapshot(&srv, &owner_tok, "ws_unreach").await;
    assert_eq!(snap3.runs[0].run.status, RunStatus::Stopped);
    assert_eq!(snap3.tasks[0].active_run_id, None);
    assert_eq!(snap3.tasks[0].revision.0, task_rev_before + 1);

    let events: Vec<WorkspaceEventEnvelope> = srv
        .http
        .get(format!(
            "{}/api/v1/workspaces/ws_unreach/sync/events?after_cursor={cursor_before}",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .send()
        .await
        .expect("delta events")
        .json()
        .await
        .expect("events json");
    assert!(events.iter().any(|ev| matches!(
        &ev.event,
        WorkspaceSyncEvent::TaskUpserted { task } if task.active_run_id.is_none()
    )));
}

#[tokio::test]
async fn test_heartbeat_after_force_cancel_does_not_mutate_stopped_run() {
    let srv = spawn_test_server().await;
    let (owner_tok, _) = setup_owned_run_pair(&srv, "ws_fc_hb", "proj_fc_hb").await;

    assert_eq!(
        post_sweep(&srv, &owner_tok, "ws_fc_hb").await,
        StatusCode::OK
    );
    let snap_before = get_snapshot(&srv, &owner_tok, "ws_fc_hb").await;
    let force_cancel = TeamCommand::ForceCancelUnreachableRun(ForceCancelUnreachableRunCommand {
        command_id: CommandId::from("cmd_fc_hb_stop"),
        run_id: RunId::from("run_own_1"),
        reason: "runner offline".to_string(),
        expected_revision: snap_before.runs[0].run.revision,
        created_at: Timestamp(300),
    });
    assert_eq!(
        post_command(&srv, &owner_tok, "ws_fc_hb", &force_cancel)
            .await
            .status(),
        StatusCode::CREATED
    );
    let cursor_after_stop = get_snapshot(&srv, &owner_tok, "ws_fc_hb")
        .await
        .cursor_seq
        .0;

    // Subsequent heartbeat and re-register MUST restore runner Online without mutating the
    // already-terminal Stopped run or emitting a spurious RunUpserted event.
    let hb = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/ws_fc_hb/runners/runner_a/heartbeat",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .send()
        .await
        .expect("heartbeat after stop");
    assert_eq!(hb.status(), StatusCode::OK);

    let rereg = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/ws_fc_hb/runners/register",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .json(&json!({ "runner_id": "runner_a", "name": "Alice Mac" }))
        .send()
        .await
        .expect("reregister after stop");
    assert_eq!(rereg.status(), StatusCode::OK);

    let post_stop_events: Vec<WorkspaceEventEnvelope> = srv
        .http
        .get(format!(
            "{}/api/v1/workspaces/ws_fc_hb/sync/events?after_cursor={cursor_after_stop}",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .send()
        .await
        .expect("post stop events")
        .json()
        .await
        .expect("post stop events json");
    assert!(
        post_stop_events
            .iter()
            .any(|ev| matches!(ev.event, WorkspaceSyncEvent::RunnerUpdated { .. }))
    );
    assert!(
        !post_stop_events
            .iter()
            .any(|ev| matches!(ev.event, WorkspaceSyncEvent::RunUpserted { .. }))
    );
}

#[tokio::test]
async fn test_unreachable_runner_cannot_report_state_until_heartbeat_restores_online() {
    let srv = spawn_test_server().await;
    let (owner_tok, _) = setup_owned_run_pair(&srv, "ws_unr_sync", "proj_unr_sync").await;

    assert_eq!(
        post_sweep(&srv, &owner_tok, "ws_unr_sync").await,
        StatusCode::OK
    );

    let report = RunnerRunReport {
        runner_id: RunnerId::from("runner_a"),
        lease_epoch: LeaseEpoch(1),
        status: None,
        workflow_stage: None,
        head_sha: Some("3333333333333333333333333333333333333333".to_string()),
        base_sha: None,
        events: vec![RunnerEventInput {
            seq: EventSeq(1),
            event_type: "turn_started".to_string(),
            payload_json: "{\"step\":1}".to_string(),
            created_at: Timestamp(250),
        }],
    };
    assert_eq!(
        post_runner_report(&srv, &owner_tok, "ws_unr_sync", "run_own_1", &report).await,
        StatusCode::CONFLICT
    );

    // Neither runner status nor run.runner_unreachable nor run_events may desync on rejection.
    let snap_rejected = get_snapshot(&srv, &owner_tok, "ws_unr_sync").await;
    assert!(snap_rejected.runs[0].ownership.runner_unreachable);
    let runner_a = snap_rejected
        .runners
        .iter()
        .find(|r| r.id == RunnerId::from("runner_a"))
        .expect("runner_a");
    assert_eq!(runner_a.status, RunnerStatus::Unreachable);
    let events_before: Vec<TeamRunEvent> = srv
        .http
        .get(format!(
            "{}/api/v1/workspaces/ws_unr_sync/runs/run_own_1/events",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .send()
        .await
        .expect("run events")
        .json()
        .await
        .expect("events json");
    assert!(events_before.is_empty());

    // Heartbeat restores Online and clears run.runner_unreachable atomically; report now succeeds.
    let hb = srv
        .http
        .post(format!(
            "{}/api/v1/workspaces/ws_unr_sync/runners/runner_a/heartbeat",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .send()
        .await
        .expect("heartbeat");
    assert_eq!(hb.status(), StatusCode::OK);

    let snap_online = get_snapshot(&srv, &owner_tok, "ws_unr_sync").await;
    assert!(!snap_online.runs[0].ownership.runner_unreachable);
    let runner_a_online = snap_online
        .runners
        .iter()
        .find(|r| r.id == RunnerId::from("runner_a"))
        .expect("runner_a");
    assert_eq!(runner_a_online.status, RunnerStatus::Online);

    assert_eq!(
        post_runner_report(&srv, &owner_tok, "ws_unr_sync", "run_own_1", &report).await,
        StatusCode::OK
    );
}
