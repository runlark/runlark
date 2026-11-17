import assert from "node:assert/strict";
import http from "node:http";
import { test } from "node:test";
import { Context } from "@deepseek-ai/cordis";
import { DaemonBoardClient } from "../src/daemon_board_client.ts";
import { DaemonClient } from "../src/daemon_client.ts";
import { registerRunlarkChatTool } from "../src/tools.ts";
import { extractAttemptCostFromTurnEnd, UsageBridge } from "../src/usage_bridge.ts";
import { handleRunlarkWebRequest, type WebBridgeTarget } from "../src/web_routes.ts";

test("web_board_routes proxies board/projects/tasks/chat/runs/plugins and preserves 409 conflicts", async () => {
  const recorded: { method: string; path: string; body?: unknown }[] = [];
  const daemonServer = http.createServer(async (req, res) => {
    const url = new URL(req.url ?? "/", "http://127.0.0.1");
    const chunks: Buffer[] = [];
    for await (const c of req) chunks.push(c as Buffer);
    const text = Buffer.concat(chunks).toString("utf8");
    const body = text ? (JSON.parse(text) as unknown) : undefined;
    recorded.push({ method: req.method ?? "GET", path: url.pathname, body });

    if (req.method === "GET" && url.pathname === "/api/v1/board") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ slots: { max: 2, used: 1, available: 1 }, tasks: [] }));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/projects") {
      res.writeHead(201, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ project_id: "proj_1", name: "Demo" }));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/tasks/task_1/pr") {
      res.writeHead(409, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: "pr_precondition_failed", message: "Acceptance required" }));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/chat/dispatch") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ command: "board", summary: "Board: 0 tasks", data: {} }));
      return;
    }
    if (req.method === "GET" && url.pathname === "/api/v1/runs/run_1/diff") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ run_id: "run_1", diff: "+ok\n" }));
      return;
    }
    if (req.method === "GET" && url.pathname === "/api/v1/runs/run_1/history") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ run: { id: "run_1" }, attempts: [] }));
      return;
    }
    if (req.method === "POST" && url.pathname === "/api/v1/runs/run_1/verify") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ verdict: "pass" }));
      return;
    }
    if (req.method === "GET" && url.pathname === "/api/v1/plugins") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify([{ name: "solo-default" }]));
      return;
    }
    if (req.method === "GET" && url.pathname === "/api/v1/scheduler/status") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ max_slots: 2, active_slots_used: 0, available_slots: 2 }));
      return;
    }
    res.writeHead(404, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ error: "not_found", message: "missing" }));
  });

  await new Promise<void>((r) => daemonServer.listen(0, "127.0.0.1", r));
  const dAddr = daemonServer.address();
  assert.ok(dAddr && typeof dAddr === "object");
  const daemonBase = `http://127.0.0.1:${dAddr.port}`;
  const daemonClient = new DaemonClient(daemonBase);

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
    const board = await (await fetch(`${baseUrl}/api/runlark/board`)).json();
    assert.equal(board.slots.max, 2);

    const projRes = await fetch(`${baseUrl}/api/runlark/projects`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "Demo", repo_root: "/tmp/repo" }),
    });
    assert.equal(projRes.status, 201);

    const prConflict = await fetch(`${baseUrl}/api/runlark/tasks/task_1/pr`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({}),
    });
    assert.equal(prConflict.status, 409);

    const chat = await (
      await fetch(`${baseUrl}/api/runlark/chat`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ input: "/board" }),
      })
    ).json();
    assert.equal(chat.command, "board");

    const diff = await (await fetch(`${baseUrl}/api/runlark/runs/run_1/diff`)).json();
    assert.equal(diff.diff, "+ok\n");
    const history = await (await fetch(`${baseUrl}/api/runlark/runs/run_1/history`)).json();
    assert.equal(history.run.id, "run_1");
    const verify = await (
      await fetch(`${baseUrl}/api/runlark/runs/run_1/verify`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ task_id: "task_1" }),
      })
    ).json();
    assert.equal(verify.verdict, "pass");
    const plugins = await (await fetch(`${baseUrl}/api/runlark/plugins`)).json();
    assert.equal(plugins[0].name, "solo-default");
    const sched = await (await fetch(`${baseUrl}/api/runlark/scheduler/status`)).json();
    assert.equal(sched.max_slots, 2);

    // Exercise tools.ts registration when ctx.tools is present
    const registeredTools: {
      name: string;
      execute: (args: unknown, exec: unknown) => Promise<unknown>;
    }[] = [];
    const ctx = new Context();
    ctx.provide("tools", undefined as never);
    ctx.set("tools", {
      register: (def: {
        name: string;
        execute: (args: unknown, exec: unknown) => Promise<unknown>;
      }) => {
        registeredTools.push(def);
        return () => {};
      },
    } as never);
    registerRunlarkChatTool(ctx, new DaemonBoardClient(daemonBase));
    await new Promise((r) => setTimeout(r, 10));
    assert.equal(registeredTools.length, 1);
    assert.equal(registeredTools[0].name, "runlark_chat_command");
    const toolOut = (await registeredTools[0].execute(
      { input: "/board" },
      { signal: new AbortController().signal },
    )) as { command: string };
    assert.equal(toolOut.command, "board");
  } finally {
    proxyServer.closeAllConnections();
    daemonServer.closeAllConnections();
    await new Promise<void>((r) => proxyServer.close(() => r()));
    await new Promise<void>((r) => daemonServer.close(() => r()));
  }
});

test("UsageBridge preserves missing cost as null UnknownCost and never folds into zero", async () => {
  const unpriced = extractAttemptCostFromTurnEnd(
    { turn: 1, reason: { kind: "completed" } },
    { inputTokens: 120, outputTokens: 45 },
  );
  assert.equal(unpriced.costMicroUsd, null);
  assert.ok(unpriced.unknownCostReason?.includes("without USD pricing"));

  const priced = extractAttemptCostFromTurnEnd({
    turn: 1,
    reason: { kind: "completed" },
    costMicroUsd: 42_500,
  });
  assert.equal(priced.costMicroUsd, 42_500);
  assert.equal(priced.unknownCostReason, null);

  const posted: unknown[] = [];
  const server = http.createServer(async (req, res) => {
    const chunks: Buffer[] = [];
    for await (const c of req) chunks.push(c as Buffer);
    posted.push(JSON.parse(Buffer.concat(chunks).toString("utf8")));
    res.writeHead(201, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ run_id: "run_u", attempts: [], cost_summary: {} }));
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  const addr = server.address();
  assert.ok(addr && typeof addr === "object");

  try {
    const bridge = new UsageBridge(new DaemonBoardClient(`http://127.0.0.1:${addr.port}`));
    bridge.observeEvent("sess_u", {
      type: "assistant/message",
      seq: 2 as never,
      time: 100,
      surfaceOp: "append",
      data: {
        turn: 1,
        step: 1,
        message: { role: "assistant", content: [] } as never,
        stream: [],
        usage: { inputTokens: 50, outputTokens: 20 },
      },
    });
    const payload = await bridge.recordTurnEndUsage("sess_u", "run_u", {
      type: "turn/end",
      seq: 3 as never,
      time: 200,
      data: { turn: 1, reason: { kind: "completed" } },
    });
    assert.equal(payload.cost_micro_usd, null);
    assert.ok(payload.unknown_cost_reason);
    assert.equal(posted.length, 1);
  } finally {
    server.closeAllConnections();
    await new Promise<void>((r) => server.close(() => r()));
  }
});
