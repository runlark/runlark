import assert from "node:assert/strict";
import http from "node:http";
import { test } from "node:test";
import { DaemonClient } from "../src/daemon_client.ts";
import { handleRunlarkWebRequest, type WebBridgeTarget } from "../src/web_routes.ts";

test("web_routes exposes Runlark HTTP proxy routes and forwards answers", async () => {
  const calls: string[] = [];
  const daemonServer = http.createServer(async (req, res) => {
    const url = new URL(req.url ?? "/", "http://127.0.0.1");
    if (req.method === "GET" && url.pathname === "/health") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ status: "ok" }));
      return;
    }
    if (req.method === "GET" && url.pathname === "/api/v1/sessions/sess_r1") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(
        JSON.stringify({
          session_id: "sess_r1",
          task_id: "task_r1",
          cwd: "/tmp/wt_r1",
          status: "active",
          current_revision: 3,
          active_run_id: "run_r1",
          active_run_status: "waiting_for_approval",
          worktree_path: "/tmp/wt_r1",
          created_at: 1000,
          updated_at: 1000,
        }),
      );
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/questions/q_r1/answer") {
      const chunks: Buffer[] = [];
      for await (const c of req) chunks.push(c as Buffer);
      calls.push(`daemon:answer:${Buffer.concat(chunks).toString("utf8")}`);
      res.writeHead(201, { "Content-Type": "application/json" });
      res.end(
        JSON.stringify({
          command_id: "cmd_ans",
          status: "pending",
          question_id: "q_r1",
          current_revision: 4,
          run_status: "running",
        }),
      );
      return;
    }
    res.writeHead(404);
    res.end("not found");
  });

  await new Promise<void>((resolveListen) => daemonServer.listen(0, "127.0.0.1", resolveListen));
  const daemonAddr = daemonServer.address();
  assert.ok(daemonAddr && typeof daemonAddr === "object");
  const daemonClient = new DaemonClient(`http://127.0.0.1:${daemonAddr.port}`);

  const bridgeTarget: WebBridgeTarget = {
    ensureManagedSession: async (opts) => ({ session_id: opts.sessionId }),
    sendManagedPrompt: async (req) => {
      calls.push(`bridge:send:${req.sessionId}`);
      return { accepted: true };
    },
    stopManagedRun: async (sid) => {
      calls.push(`bridge:stop:${sid}`);
      return { accepted: true };
    },
    resumeManagedSession: async (sid) => {
      calls.push(`bridge:resume:${sid}`);
      return { caughtUp: 0 };
    },
    archiveManagedSession: async (sid) => {
      calls.push(`bridge:archive:${sid}`);
    },
    applyQuestionAnswer: (sid, qid, ans) => {
      calls.push(`bridge:answer:${sid}:${qid}:${ans}`);
    },
    handleOutboxDispatch: async () => {},
  };

  const proxyServer = http.createServer((req, res) => {
    void handleRunlarkWebRequest(req, res, bridgeTarget, daemonClient);
  });
  await new Promise<void>((resolveListen) => proxyServer.listen(0, "127.0.0.1", resolveListen));
  const proxyAddr = proxyServer.address();
  assert.ok(proxyAddr && typeof proxyAddr === "object");
  const baseUrl = `http://127.0.0.1:${proxyAddr.port}`;

  try {
    const health = await (await fetch(`${baseUrl}/api/runlark/health`)).json();
    assert.deepEqual(health, { status: "ok", daemonHealthy: true });

    const sess = await (await fetch(`${baseUrl}/api/runlark/sessions/sess_r1`)).json();
    assert.equal(sess.active_run_status, "waiting_for_approval");

    await fetch(`${baseUrl}/api/runlark/sessions/sess_r1/send`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ prompt: "Hello" }),
    });
    await fetch(`${baseUrl}/api/runlark/sessions/sess_r1/stop`, { method: "POST" });
    await fetch(`${baseUrl}/api/runlark/questions/q_r1/answer`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ answer: "Approve", session_id: "sess_r1", expected_revision: 3 }),
    });

    assert.ok(calls.includes("bridge:send:sess_r1"));
    assert.ok(calls.includes("bridge:stop:sess_r1"));
    assert.ok(calls.includes("bridge:answer:sess_r1:q_r1:Approve"));
    assert.ok(calls.some((c) => c.startsWith("daemon:answer:")));
  } finally {
    proxyServer.closeAllConnections();
    daemonServer.closeAllConnections();
    await new Promise<void>((r) => proxyServer.close(() => r()));
    await new Promise<void>((r) => daemonServer.close(() => r()));
  }
});
