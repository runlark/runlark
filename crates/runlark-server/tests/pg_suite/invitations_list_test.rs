use crate::pg_suite::harness::{
    accept_invite, bootstrap_user, create_invite, create_ws, spawn_test_server,
};
use reqwest::StatusCode;
use serde_json::Value;

#[tokio::test]
async fn list_invitations_enforces_rbac_and_hides_token_hash() {
    let srv = spawn_test_server().await;
    let owner_tok = bootstrap_user(&srv, "usr_owner_inv", "Alice Owner").await;
    create_ws(&srv, &owner_tok, "ws_inv_list", "inv-list-ws").await;

    let (inv1_id, inv1_tok) =
        create_invite(&srv, &owner_tok, "ws_inv_list", "bob@test", "member").await;
    let (inv2_id, _inv2_tok) =
        create_invite(&srv, &owner_tok, "ws_inv_list", "carol@test", "viewer").await;

    let bob_tok = accept_invite(&srv, &inv1_tok, "usr_bob_inv", "Bob Member").await;

    // Owner lists both invitations; secret token/token_hash must never be returned.
    let res = srv
        .http
        .get(format!(
            "{}/api/v1/workspaces/ws_inv_list/invitations",
            srv.base_url
        ))
        .bearer_auth(&owner_tok)
        .send()
        .await
        .expect("owner list invitations");
    assert_eq!(res.status(), StatusCode::OK);
    let body: Vec<Value> = res.json().await.expect("invitations json");
    assert_eq!(body.len(), 2);
    for item in &body {
        assert!(item.get("token_hash").is_none());
        assert!(item.get("invite_token").is_none());
    }
    let inv1 = body
        .iter()
        .find(|i| i["id"] == inv1_id)
        .expect("inv1 present");
    assert_eq!(inv1["status"], "accepted");
    let inv2 = body
        .iter()
        .find(|i| i["id"] == inv2_id)
        .expect("inv2 present");
    assert_eq!(inv2["status"], "pending");

    // Plain member cannot enumerate invitations (requires InviteMembers permission).
    let member_res = srv
        .http
        .get(format!(
            "{}/api/v1/workspaces/ws_inv_list/invitations",
            srv.base_url
        ))
        .bearer_auth(&bob_tok)
        .send()
        .await
        .expect("member list invitations");
    assert_eq!(member_res.status(), StatusCode::FORBIDDEN);
}
