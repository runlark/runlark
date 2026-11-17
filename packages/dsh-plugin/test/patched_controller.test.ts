import assert from "node:assert/strict";
import http from "node:http";
import { test } from "node:test";
import { Context } from "@deepseek-ai/cordis";
import { brandString } from "@deepseek-ai/dsh-brand";
import { SessionController } from "@deepseek-ai/dsh-api-session-controller";
import { SessionId, SessionLogOffset } from "@deepseek-ai/dsh-session";
import { DaemonClient } from "../src/daemon_client.ts";
import { RunlarkSessionInterceptor } from "../src/interceptor.ts";
import type { SessionRequestId } from "../src/types.ts";

const reqId = (id: string): SessionRequestId => brandString<SessionRequestId>(id);

function createFakeAgent(sessionId: string, cwd = "/tmp/wt") {
  const followups: unknown[] = [];
  const steers: unknown[] = [];
  const cancels: unknown[] = [];
  const session = {
    id: SessionId(sessionId),
    seq: SessionLogOffset(0),
    inheritedEventCount: SessionLogOffset(0),
    header: {
      version: 4,
      id: SessionId(sessionId),
      createdAt: 1000,
      cwd,
      isSeeded: false,
    },
    snapshotEvents: () => [],
  };
  const agent = {
    id: SessionId(sessionId),
    session,
    inbox: { nextTurn: [], nextStep: [] },
    model: { current: () => undefined },
    followup(msg: unknown) {
      followups.push(msg);
    },
    steer(msg: unknown) {
      steers.push(msg);
    },
    cancel(cause: unknown, opts: unknown) {
      cancels.push({ cause, opts });
    },
    runMaintenance: async <T>(fn: () => Promise<T>) => await fn(),
  };
  return { agent, followups, steers, cancels };
}

async function setupControllerHarness() {
  const ctx = new Context();
  const agentsMap = new Map<string, ReturnType<typeof createFakeAgent>>();
  const sessionsMap = new Map<string, ReturnType<typeof createFakeAgent>["agent"]["session"]>();

  ctx.provide("agentDefaultModel", {});
  ctx.provide("agents", {
    get: (id: string) => agentsMap.get(id)?.agent,
    resume: async ({ resumeSessionId }: { resumeSessionId: string }) => {
      const entry = agentsMap.get(resumeSessionId);
      if (!entry) throw new Error(`missing agent ${resumeSessionId}`);
      return { agent: entry.agent };
    },
  });
  ctx.provide("attachments", {
    admitPromptContent: async (content: unknown) => content,
  });
  ctx.provide("fileUploads", {
    registerAgentResolver: () => () => {},
    resolve: () => {
      throw new Error("unexpected file upload");
    },
    bindPrompt: () => ({ commit() {}, [Symbol.dispose]() {} }),
  });
  ctx.provide("fs", {});
  ctx.provide("llm", {});
  ctx.provide("remoteAccess", { allows: () => true });
  ctx.provide("sessions", {
    get: (id: string) => sessionsMap.get(id),
    list: () => [...sessionsMap.values()],
  });
  ctx.provide("sessionProjections", {
    register: async () => ({
      stateOf: () => ({ ending: null }),
      [Symbol.dispose]() {},
    }),
    onEventChanged: () => () => {},
  });
  ctx.provide("sessionQuery", {
    listHeaders: async () => [],
    observeSession: async (id: string) => {
      const sess = sessionsMap.get(id);
      if (!sess) throw new Error(`missing session ${id}`);
      return { header: sess.header, [Symbol.dispose]() {} };
    },
  });
  ctx.provide("typert", {
    resources: { register() {} },
    lookups: { configure() {} },
    contexts: { configureHost() {} },
  });
  ctx.provide("typertGateway", {});
  ctx.provide("workspaceRegistry", {
    archivedSessionIds: [],
    list: () => [],
  });

  await ctx.plugin(SessionController, { nativeOpen: false });

  return {
    ctx,
    registerAgent(sessionId: string, cwd = "/tmp/wt") {
      const entry = createFakeAgent(sessionId, cwd);
      agentsMap.set(sessionId, entry);
      sessionsMap.set(sessionId, entry.agent.session);
      return entry;
    },
  };
}

test("patched SessionController intercepts managed sessions and passes unmanaged through", async () => {
  const receivedCommands: { type: string; payload: Record<string, unknown> }[] = [];
  let currentRevision = 1;
  let rejectNextCommand = false;

  const server = http.createServer(async (req, res) => {
    const url = req.url ?? "/";
    if (req.method === "GET" && url === "/api/v1/sessions/sess_managed") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(
        JSON.stringify({
          session_id: "sess_managed",
          task_id: "task_1",
          cwd: "/tmp/wt",
          status: currentRevision > 1 ? "active" : "created",
          current_revision: currentRevision,
          active_run_id: null,
          created_at: 1000,
          updated_at: 1000,
        }),
      );
      return;
    }
    if (req.method === "POST" && url === "/api/v1/commands") {
      const chunks: Buffer[] = [];
      for await (const c of req) chunks.push(c as Buffer);
      const body = JSON.parse(Buffer.concat(chunks).toString("utf8")) as {
        type: string;
        payload: Record<string, unknown>;
      };
      if (rejectNextCommand) {
        rejectNextCommand = false;
        res.writeHead(500, { "Content-Type": "text/plain" });
        res.end("simulated persistence failure");
        return;
      }
      receivedCommands.push(body);
      currentRevision += 1;
      res.writeHead(201, { "Content-Type": "application/json" });
      res.end(
        JSON.stringify({
          command_id: body.payload.command_id,
          status: "pending",
          current_revision: currentRevision,
          run_id: body.payload.run_id,
          run_status: body.type === "StopRun" ? "cancel_requested" : "running",
        }),
      );
      return;
    }
    res.writeHead(404);
    res.end("not found");
  });

  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const addr = server.address();
  assert.ok(addr && typeof addr === "object");
  const daemonUrl = `http://127.0.0.1:${addr.port}`;

  try {
    const { ctx, registerAgent } = await setupControllerHarness();
    const managed = registerAgent("sess_managed");
    const unmanaged = registerAgent("sess_unmanaged");

    const interceptor = new RunlarkSessionInterceptor(
      new DaemonClient(daemonUrl),
      new Set(["sess_managed"]),
    );
    ctx.provide("sessionCommandInterceptor", interceptor);

    // 1. Unmanaged session bypasses interceptor completely
    const unmanagedRes = await ctx.sessionController.prompt(
      {
        requestId: reqId("unmanaged_req_1"),
        sessionId: SessionId("sess_unmanaged"),
        mode: "queue",
        content: [{ type: "text", text: "Hello unmanaged" }],
      },
      new AbortController().signal,
    );
    assert.deepEqual(unmanagedRes, { accepted: true });
    assert.equal(unmanaged.followups.length, 1);
    assert.equal(receivedCommands.length, 0);

    const unmanagedCancel = await ctx.sessionController.cancel({
      sessionId: SessionId("sess_unmanaged"),
    });
    assert.deepEqual(unmanagedCancel, { accepted: true });
    assert.equal(unmanaged.cancels.length, 1);
    assert.equal(receivedCommands.length, 0);

    // 2. Managed session persists SendPrompt BEFORE agent.followup()
    const managedRes = await ctx.sessionController.prompt(
      {
        requestId: reqId("managed_req_1"),
        sessionId: SessionId("sess_managed"),
        mode: "queue",
        content: [{ type: "text", text: "Hello managed" }],
      },
      new AbortController().signal,
    );
    assert.deepEqual(managedRes, { accepted: true });
    assert.equal(managed.followups.length, 1);
    assert.equal(receivedCommands.length, 1);
    assert.equal(receivedCommands[0].type, "SendPrompt");
    assert.equal(receivedCommands[0].payload.command_id, "managed_req_1");
    assert.equal(receivedCommands[0].payload.expected_revision, 1);

    // 3. Persistence failure aborts BEFORE agent.followup() is ever called
    rejectNextCommand = true;
    await assert.rejects(
      ctx.sessionController.prompt(
        {
          requestId: reqId("managed_req_fail"),
          sessionId: SessionId("sess_managed"),
          mode: "queue",
          content: [{ type: "text", text: "Should not run" }],
        },
        new AbortController().signal,
      ),
      /Persist-before-send failed/,
    );
    assert.equal(managed.followups.length, 1, "agent.followup must NOT run if persist fails");

    // 4. Managed cancel persists StopRun BEFORE agent.cancel() and awaits Promise<SessionCancelValue>
    const cancelRes = await ctx.sessionController.cancel({
      sessionId: SessionId("sess_managed"),
    });
    assert.deepEqual(cancelRes, { accepted: true });
    assert.equal(managed.cancels.length, 1);
    assert.equal(receivedCommands.length, 2);
    assert.equal(receivedCommands[1].type, "StopRun");
    assert.equal(receivedCommands[1].payload.expected_revision, 2);

    await ctx.fiber.dispose();
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});
