use super::harness::{
    TestServer, bootstrap_user, create_ws, get_snapshot, post_command, promote_project,
    sample_run_cmd, sample_task_cmd, spawn_test_server,
};
use reqwest::StatusCode;
use runlark_domain::{
    ArtifactDigest, ArtifactKind, AttachArtifactCommand, CommandId, RunId, TeamCommand, Timestamp,
};
use runlark_server::cas::sha256_hex;
use runlark_server::pg::create_pool;
use std::process::Command;

async fn put_cas_artifact(
    srv: &TestServer,
    token: &str,
    wid: &str,
    digest: &str,
    kind: &str,
    media_type: &str,
    payload: &[u8],
) -> StatusCode {
    srv.http
        .put(format!(
            "{}/api/v1/workspaces/{wid}/artifacts/{digest}?kind={kind}&media_type={media_type}",
            srv.base_url
        ))
        .bearer_auth(token)
        .body(payload.to_vec())
        .send()
        .await
        .expect("upload cas")
        .status()
}

#[tokio::test]
async fn test_cas_workspace_scoped_metadata_and_same_workspace_conflict() {
    let srv = spawn_test_server().await;
    let tok_a = bootstrap_user(&srv, "usr_cas_iso_a", "Alice").await;
    let tok_b = bootstrap_user(&srv, "usr_cas_iso_b", "Bob").await;
    create_ws(&srv, &tok_a, "ws_cas_iso_a", "ws-cas-iso-a").await;
    create_ws(&srv, &tok_b, "ws_cas_iso_b", "ws-cas-iso-b").await;
    promote_project(&srv, &tok_b, "ws_cas_iso_b", "proj_cas_iso_b").await;

    let payload = b"shared-identical-cas-payload";
    let digest = sha256_hex(payload);

    // Workspace A uploads as diff; identical re-upload succeeds; conflicting kind returns 409.
    assert_eq!(
        put_cas_artifact(
            &srv,
            &tok_a,
            "ws_cas_iso_a",
            &digest,
            "diff",
            "text/x-patch",
            payload
        )
        .await,
        StatusCode::CREATED
    );
    assert_eq!(
        put_cas_artifact(
            &srv,
            &tok_a,
            "ws_cas_iso_a",
            &digest,
            "diff",
            "text/x-patch",
            payload
        )
        .await,
        StatusCode::CREATED
    );
    assert_eq!(
        put_cas_artifact(
            &srv,
            &tok_a,
            "ws_cas_iso_a",
            &digest,
            "evidence_bundle",
            "application/json",
            payload
        )
        .await,
        StatusCode::CONFLICT
    );

    // Workspace B uploads identical bytes as evidence_bundle and attaches them cleanly.
    assert_eq!(
        put_cas_artifact(
            &srv,
            &tok_b,
            "ws_cas_iso_b",
            &digest,
            "evidence_bundle",
            "application/json",
            payload
        )
        .await,
        StatusCode::CREATED
    );

    verify_ws_b_attach_and_ws_a_content_type(&srv, &tok_a, &tok_b, &digest, payload.len() as u64)
        .await;
}

async fn verify_ws_b_attach_and_ws_a_content_type(
    srv: &TestServer,
    tok_a: &str,
    tok_b: &str,
    digest: &str,
    size_bytes: u64,
) {
    assert_eq!(
        post_command(
            srv,
            tok_b,
            "ws_cas_iso_b",
            &sample_task_cmd("cmd_cb_t1", "task_cb_1", "proj_cas_iso_b", "CAS attach")
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        post_command(
            srv,
            tok_b,
            "ws_cas_iso_b",
            &sample_run_cmd("cmd_cb_r1", "run_cb_1", "proj_cas_iso_b", "task_cb_1", None)
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    let attach = TeamCommand::AttachArtifact(AttachArtifactCommand {
        command_id: CommandId::from("cmd_cb_att"),
        run_id: RunId::from("run_cb_1"),
        digest_sha256: ArtifactDigest::parse(digest).expect("digest"),
        kind: ArtifactKind::EvidenceBundle,
        media_type: "application/json".to_string(),
        size_bytes,
        label: "evidence_bundle".to_string(),
        created_at: Timestamp(200),
    });
    assert_eq!(
        post_command(srv, tok_b, "ws_cas_iso_b", &attach)
            .await
            .status(),
        StatusCode::CREATED
    );

    let snap_b = get_snapshot(srv, tok_b, "ws_cas_iso_b").await;
    assert_eq!(snap_b.artifacts.len(), 1);
    assert_eq!(
        snap_b.artifacts[0].artifact.kind,
        ArtifactKind::EvidenceBundle
    );
    assert_eq!(snap_b.artifacts[0].artifact.media_type, "application/json");

    let dl_a = srv
        .http
        .get(format!(
            "{}/api/v1/workspaces/ws_cas_iso_a/artifacts/{digest}",
            srv.base_url
        ))
        .bearer_auth(tok_a)
        .send()
        .await
        .expect("download a");
    assert_eq!(dl_a.status(), StatusCode::OK);
    assert_eq!(
        dl_a.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/x-patch")
    );
}

#[tokio::test]
async fn test_bootstrap_owner_requires_slug_and_rolls_back_atomically_on_conflict() {
    let srv = spawn_test_server().await;
    let exe = env!("CARGO_BIN_EXE_runlark-server");

    // Passing --workspace-id without --workspace-slug MUST fail instead of ignoring it.
    let missing_slug = Command::new(exe)
        .args([
            "bootstrap-owner",
            "--postgres-url",
            &srv.test_pg_url,
            "--user-id",
            "usr_no_slug",
            "--display-name",
            "No Slug",
            "--workspace-id",
            "ws_ignored",
        ])
        .output()
        .expect("run bootstrap-owner without slug");
    assert!(!missing_slug.status.success());

    // First bootstrap succeeds with slug "dup-slug".
    let first = Command::new(exe)
        .args([
            "bootstrap-owner",
            "--postgres-url",
            &srv.test_pg_url,
            "--user-id",
            "usr_boot_1",
            "--display-name",
            "First",
            "--workspace-slug",
            "dup-slug",
        ])
        .output()
        .expect("run first bootstrap");
    assert!(first.status.success());

    // Duplicate slug MUST fail and roll back user + auth_tokens atomically (no orphaned token).
    let dup = Command::new(exe)
        .args([
            "bootstrap-owner",
            "--postgres-url",
            &format!("  {}  ", srv.test_pg_url),
            "--user-id",
            "usr_boot_orphan",
            "--display-name",
            "Orphan",
            "--workspace-slug",
            "dup-slug",
        ])
        .output()
        .expect("run duplicate slug bootstrap");
    assert!(!dup.status.success());

    let pool = create_pool(&srv.test_pg_url).expect("pool");
    let client = pool.get().await.expect("client");
    let orphan_tokens: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM auth_tokens WHERE user_id = 'usr_boot_orphan'",
            &[],
        )
        .await
        .expect("count orphan tokens")
        .get(0);
    assert_eq!(orphan_tokens, 0);
}
