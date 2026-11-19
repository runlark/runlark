import assert from "node:assert/strict";
import http from "node:http";
import { test } from "node:test";
import { DaemonClient } from "../src/daemon_client.ts";
import { handleRunlarkWebRequest, type WebBridgeTarget } from "../src/web_routes.ts";

test("web_board_routes proxies team workspaces/invites/promote/sync/drafts/comments/runners", async () => {
  const recorded: { method: string; path: string; query: string }[] = [];
  const daemonServer = http.createServer(async (req, res) => {
    const url = new URL(req.url ?? "/", "http://127.0.0.1");
    const chunks: Buffer[] = [];
    for await (const c of req) chunks.push(c as Buffer);
    recorded.push({ method: req.method ?? "GET", path: url.pathname, query: url.search });

    if (req.method === "POST" && url.pathname === "/api/v1/team/login") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(
        JSON.stringify({ user_id: "usr_a", display_name: "Alice", server_url: "http://srv" }),
      );
      return;
    }
    if (req.method === "GET" && url.pathname === "/api/v1/team/status") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ sessions: [], workspaces: [], unresolved_drafts_count: 1 }));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/team/workspaces") {
      res.writeHead(201, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ workspace: { id: "ws_1", name: "Core" }, members: [] }));
      return;
    }
    if (req.method === "GET" && url.pathname === "/api/v1/team/workspaces/ws_1/members") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify([{ user_id: "usr_a", role: "owner" }]));
      return;
    }
    if (req.method === "PATCH" && url.pathname === "/api/v1/team/workspaces/ws_1/members/usr_b") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ user_id: "usr_b", role: "maintainer" }));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/team/workspaces/ws_1/invitations") {
      res.writeHead(201, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ invitation: { id: "inv_1" }, invite_token: "tok_1" }));
      return;
    }
    if (
      req.method === "POST" &&
      url.pathname === "/api/v1/team/workspaces/ws_1/invitations/inv_1/revoke"
    ) {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ id: "inv_1", status: "revoked" }));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/team/invitations/accept") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ member: { workspace_id: "ws_1" }, token_saved: true }));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/projects/proj_1/promote-team") {
      res.writeHead(409, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: "solo_to_team_blocked", message: "active runs exist" }));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/team/sync") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ workspaces_synced: 1, events_applied: 3 }));
      return;
    }
    if (req.method === "GET" && url.pathname === "/api/v1/team/drafts") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ drafts: [{ id: "draft_1" }], outbox: [] }));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/team/drafts") {
      res.writeHead(201, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ id: "draft_2" }));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/team/drafts/draft_1/retry") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ id: "draft_1", sync_state: { state: "synced" } }));
      return;
    }
    if (req.method === "DELETE" && url.pathname === "/api/v1/team/drafts/draft_1") {
      res.writeHead(204);
      res.end();
      return;
    }
    if (req.method === "GET" && url.pathname === "/api/v1/tasks/task_1/comments") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify([{ id: "cmt_1", body: "hello" }]));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/tasks/task_1/comments") {
      res.writeHead(202, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ delivery: "queued_offline", draft: { id: "draft_cmt_1" } }));
      return;
    }
    if (req.method === "GET" && url.pathname === "/api/v1/team/runners") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify([{ id: "runner_a", status: "online" }]));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/team/runners/register") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ id: "runner_a", name: "Laptop" }));
      return;
    }
    if (req.method === "GET" && url.pathname === "/api/v1/team/runners/status") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ local_runner_id: "runner_a", buffered_events_count: 0 }));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/team/runs/run_1/assign") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ run: { id: "run_1" }, ownership: { lease_epoch: 1 } }));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/team/runs/run_1/cancel-unreachable") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ aggregate_id: "run_1", cursor_seq: 9 }));
      return;
    }
    res.writeHead(404, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ error: "not_found" }));
  });

  await new Promise<void>((r) => daemonServer.listen(0, "127.0.0.1", r));
  const dAddr = daemonServer.address();
  assert.ok(dAddr && typeof dAddr === "object");
  const daemonClient = new DaemonClient(`http://127.0.0.1:${dAddr.port}`);

  const bridgeTarget: WebBridgeTarget = {
    ensureManagedSession: async () => ({}),
    sendManagedPrompt: async () => ({}),
    stopManagedRun: async () => ({}),
    resumeManagedSession: async () => ({}),
    archiveManagedSession: async () => {},
    applyQuestionAnswer: () => {},
    handleOutboxDispatch: async () => {},
  };

  const proxyServer = http.createServer((req, res) => {
    void handleRunlarkWebRequest(req, res, bridgeTarget, daemonClient);
  });
  await new Promise<void>((r) => proxyServer.listen(0, "127.0.0.1", r));
  const pAddr = proxyServer.address();
  assert.ok(pAddr && typeof pAddr === "object");
  const baseUrl = `http://127.0.0.1:${pAddr.port}`;

  try {
    const login = await (
      await fetch(`${baseUrl}/api/runlark/team/login`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ server_url: "http://srv", bearer_token: "rlk_tok" }),
      })
    ).json();
    assert.equal(login.user_id, "usr_a");

    const status = await (await fetch(`${baseUrl}/api/runlark/team/status`)).json();
    assert.equal(status.unresolved_drafts_count, 1);

    const wsRes = await fetch(`${baseUrl}/api/runlark/team/workspaces`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "Core", slug: "core" }),
    });
    assert.equal(wsRes.status, 201);

    const members = await (
      await fetch(`${baseUrl}/api/runlark/team/workspaces/ws_1/members`)
    ).json();
    assert.equal(members[0].role, "owner");

    const patched = await (
      await fetch(`${baseUrl}/api/runlark/team/workspaces/ws_1/members/usr_b`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ role: "maintainer" }),
      })
    ).json();
    assert.equal(patched.role, "maintainer");

    const invRes = await fetch(`${baseUrl}/api/runlark/team/workspaces/ws_1/invitations`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ invitee_hint: "bob", role: "member" }),
    });
    assert.equal(invRes.status, 201);

    const revRes = await (
      await fetch(`${baseUrl}/api/runlark/team/workspaces/ws_1/invitations/inv_1/revoke`, {
        method: "POST",
      })
    ).json();
    assert.equal(revRes.status, "revoked");

    const acc = await (
      await fetch(`${baseUrl}/api/runlark/team/invitations/accept`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          server_url: "http://srv",
          invite_token: "tok",
          display_name: "Bob",
        }),
      })
    ).json();
    assert.equal(acc.token_saved, true);

    const promote = await fetch(`${baseUrl}/api/runlark/projects/proj_1/promote-team`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ workspace_id: "ws_1" }),
    });
    assert.equal(promote.status, 409);

    const sync = await (
      await fetch(`${baseUrl}/api/runlark/team/sync`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({}),
      })
    ).json();
    assert.equal(sync.events_applied, 3);

    const drafts = await (
      await fetch(`${baseUrl}/api/runlark/team/drafts?project_id=proj_1`)
    ).json();
    assert.equal(drafts.drafts.length, 1);

    const retry = await (
      await fetch(`${baseUrl}/api/runlark/team/drafts/draft_1/retry`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ expected_revision: 2 }),
      })
    ).json();
    assert.equal(retry.sync_state.state, "synced");

    const delRes = await fetch(`${baseUrl}/api/runlark/team/drafts/draft_1`, { method: "DELETE" });
    assert.equal(delRes.status, 204);

    const cmts = await (await fetch(`${baseUrl}/api/runlark/tasks/task_1/comments`)).json();
    assert.equal(cmts[0].body, "hello");

    const cmtPost = await fetch(`${baseUrl}/api/runlark/tasks/task_1/comments`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ body: "offline note" }),
    });
    assert.equal(cmtPost.status, 202);

    const runners = await (
      await fetch(`${baseUrl}/api/runlark/team/runners?workspace_id=ws_1`)
    ).json();
    assert.equal(runners[0].id, "runner_a");

    const reg = await (
      await fetch(`${baseUrl}/api/runlark/team/runners/register`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "Laptop" }),
      })
    ).json();
    assert.equal(reg.id, "runner_a");

    const rStatus = await (
      await fetch(`${baseUrl}/api/runlark/team/runners/status?workspace_id=ws_1`)
    ).json();
    assert.equal(rStatus.local_runner_id, "runner_a");

    const assign = await (
      await fetch(`${baseUrl}/api/runlark/team/runs/run_1/assign`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ runner_id: "runner_a" }),
      })
    ).json();
    assert.equal(assign.ownership.lease_epoch, 1);

    const cancel = await (
      await fetch(`${baseUrl}/api/runlark/team/runs/run_1/cancel-unreachable`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ reason: "host offline" }),
      })
    ).json();
    assert.equal(cancel.cursor_seq, 9);
    assert.ok(
      recorded.some((r) => r.path === "/api/v1/team/drafts" && r.query === "?project_id=proj_1"),
    );

    const badUri = await fetch(`${baseUrl}/api/runlark/team/workspaces/%ZZ/members`);
    assert.equal(badUri.status, 400);
    const badUriBody = (await badUri.json()) as { error: string };
    assert.equal(badUriBody.error, "invalid_uri");
  } finally {
    proxyServer.closeAllConnections();
    daemonServer.closeAllConnections();
    await new Promise<void>((r) => proxyServer.close(() => r()));
    await new Promise<void>((r) => daemonServer.close(() => r()));
  }
});
