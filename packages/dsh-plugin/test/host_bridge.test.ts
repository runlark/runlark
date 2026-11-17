import assert from "node:assert/strict";
import http from "node:http";
import { test } from "node:test";
import { Context } from "@deepseek-ai/cordis";
import { brandString } from "@deepseek-ai/dsh-brand";
import { ToolCallId } from "@deepseek-ai/dsh-llm";
import { SessionController } from "@deepseek-ai/dsh-api-session-controller";
import { SessionId, SessionLogOffset, SessionSeq } from "@deepseek-ai/dsh-session";
import { SessionQueryError } from "@deepseek-ai/dsh-session-query";
import { apply as applyRunlarkPlugin } from "../src/plugin.ts";
import type { SessionRequestId } from "../src/types.ts";

const reqId = (id: string): SessionRequestId => brandString<SessionRequestId>(id);

test("host bridge plugin covers all 5 ops, event forwarding, and ask_user_question sync", async () => {
  const daemonState = {
    sessions: new Map<string, Record<string, unknown>>(),
    commands: [] as { type: string; payload: Record<string, unknown> }[],
    dshEvents: [] as Record<string, unknown>[],
    answers: [] as { questionId: string; body: Record<string, unknown> }[],
  };

  const server = http.createServer(async (req, res) => {
    const url = new URL(req.url ?? "/", "http://127.0.0.1");
    const chunks: Buffer[] = [];
    for await (const c of req) chunks.push(c as Buffer);
    const raw = Buffer.concat(chunks).toString("utf8");
    const json = raw ? (JSON.parse(raw) as Record<string, unknown>) : undefined;

    if (req.method === "GET" && url.pathname === "/api/v1/sessions") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify([...daemonState.sessions.values()]));
      return;
    }

    if (req.method === "POST" && url.pathname === "/api/v1/sessions" && json) {
      const sess = {
        session_id: json.session_id,
        task_id: json.task_id,
        cwd: json.cwd,
        status: "created",
        current_revision: 1,
        active_run_id: null,
        active_run_status: null,
        created_at: 1000,
        updated_at: 1000,
      };
      daemonState.sessions.set(String(json.session_id), sess);
      res.writeHead(201, { "Content-Type": "application/json" });
      res.end(JSON.stringify(sess));
      return;
    }

    if (req.method === "GET" && url.pathname.startsWith("/api/v1/sessions/")) {
      const sid = decodeURIComponent(url.pathname.slice("/api/v1/sessions/".length));
      const sess = daemonState.sessions.get(sid);
      if (!sess) {
        res.writeHead(404);
        res.end("not found");
        return;
      }
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify(sess));
      return;
    }

    if (req.method === "POST" && url.pathname === "/api/v1/commands" && json) {
      const cmdType = String(json.type);
      const payload = json.payload as Record<string, unknown>;
      const sid = String(payload.session_id);
      const sess = daemonState.sessions.get(sid)!;
      if (payload.expected_revision !== sess.current_revision) {
        res.writeHead(409, { "Content-Type": "application/json" });
        res.end(
          JSON.stringify({
            error: "revision_mismatch",
            actual_revision: sess.current_revision,
          }),
        );
        return;
      }
      sess.current_revision = Number(sess.current_revision) + 1;
      if (cmdType === "SendPrompt" || cmdType === "ResumeSession") {
        sess.status = "active";
        sess.active_run_id = payload.run_id;
        sess.active_run_status = "running";
      } else if (cmdType === "StopRun") {
        sess.active_run_status = "cancel_requested";
      } else if (cmdType === "ChangeCwd") {
        sess.cwd = payload.new_cwd;
      } else if (cmdType === "ArchiveSession") {
        sess.status = "archived";
        sess.active_run_id = null;
        sess.active_run_status = null;
      }
      daemonState.commands.push({ type: cmdType, payload });
      res.writeHead(201, { "Content-Type": "application/json" });
      res.end(
        JSON.stringify({
          command_id: payload.command_id,
          status: "pending",
          current_revision: sess.current_revision,
          run_id: sess.active_run_id,
          run_status: sess.active_run_status,
        }),
      );
      return;
    }

    if (req.method === "POST" && url.pathname === "/api/v1/dsh/events" && json) {
      daemonState.dshEvents.push(json);
      if ("event" in json && isWireTurnEnd(json.event)) {
        const sess = daemonState.sessions.get(String(json.session_id));
        if (sess) {
          sess.current_revision = Number(sess.current_revision) + 1;
          sess.active_run_id = null;
          sess.active_run_status = "stopped";
        }
      }
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ ok: true }));
      return;
    }

    if (req.method === "POST" && url.pathname.startsWith("/api/v1/questions/") && json) {
      const questionId = decodeURIComponent(
        url.pathname.slice("/api/v1/questions/".length).replace(/\/answer$/, ""),
      );
      daemonState.answers.push({ questionId, body: json });
      res.writeHead(201, { "Content-Type": "application/json" });
      res.end(
        JSON.stringify({
          command_id: json.command_id ?? "cmd_q",
          status: "pending",
          question_id: questionId,
          current_revision: 99,
          run_status: "running",
        }),
      );
      return;
    }

    if (req.method === "GET" && url.pathname === "/api/v1/runs/run_1/events") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(
        JSON.stringify([
          { seq: 1, event_type: "PromptEnqueued", payload: {}, created_at: 1000 },
          { seq: 2, event_type: "RunCompleted", payload: {}, created_at: 1100 },
        ]),
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
    const ctx = new Context();
    const agentsMap = new Map<string, Record<string, unknown>>();
    const sessionsMap = new Map<string, Record<string, unknown>>();
    const archivedIds: string[] = [];
    const userQuestionAnswers: unknown[] = [];

    const makeAgent = (sid: string, cwd: string) => {
      const session = {
        id: SessionId(sid),
        seq: SessionLogOffset(0),
        inheritedEventCount: SessionLogOffset(0),
        header: {
          version: 4,
          id: SessionId(sid),
          createdAt: 1000,
          cwd,
          isSeeded: false,
        },
        snapshotEvents: () => [],
      };
      const agent = {
        id: SessionId(sid),
        session,
        inbox: { nextTurn: [], nextStep: [] },
        model: { current: () => undefined },
        followups: [] as unknown[],
        cancels: [] as unknown[],
        followup(m: unknown) {
          this.followups.push(m);
        },
        steer() {},
        cancel(cause: unknown, opts: unknown) {
          this.cancels.push({ cause, opts });
        },
      };
      agentsMap.set(sid, agent);
      sessionsMap.set(sid, session);
      return agent;
    };

    ctx.provide("agentDefaultModel", {});
    ctx.provide("agents", {
      get: (id: string) => agentsMap.get(id),
      create: async (opts: { sessionId?: string; cwd?: string }) => {
        const sid = opts.sessionId ?? "sess_minted";
        return { agent: makeAgent(sid, opts.cwd ?? "/tmp/wt") };
      },
      resume: async ({ resumeSessionId }: { resumeSessionId: string }) => {
        const agent = agentsMap.get(resumeSessionId) ?? makeAgent(resumeSessionId, "/tmp/wt");
        return { agent };
      },
    });
    ctx.provide("attachments", { admitPromptContent: async (c: unknown) => c });
    ctx.provide("fileUploads", {
      registerAgentResolver: () => () => {},
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
      register: async () => ({ stateOf: () => ({ ending: null }), [Symbol.dispose]() {} }),
      onEventChanged: () => () => {},
    });
    ctx.provide("sessionQuery", {
      listHeaders: async () => [],
      observeSession: async (id: string) => {
        const sess = sessionsMap.get(id);
        if (!sess) {
          throw new SessionQueryError(`not found: ${id}`, "SESSION_QUERY_SESSION_NOT_FOUND");
        }
        return { header: (sess as { header: unknown }).header, [Symbol.dispose]() {} };
      },
    });
    ctx.provide("typert", {
      resources: { register() {} },
      lookups: { configure() {} },
      contexts: { configureHost() {} },
    });
    ctx.provide("typertGateway", {});
    ctx.provide("workspaceRegistry", {
      archivedSessionIds: archivedIds,
      list: () => [],
      archiveSession: async (sid: string) => {
        if (!archivedIds.includes(sid)) archivedIds.push(sid);
      },
    });
    ctx.provide("userQuestions", {
      answer: (agent: unknown, callId: unknown, ans: unknown) => {
        userQuestionAnswers.push({ agent, callId, ans });
        return true;
      },
    });

    await ctx.plugin(SessionController, { nativeOpen: false });
    const bridge = applyRunlarkPlugin(ctx, { daemonUrl, syncOnStart: false });

    // 1. CWD + Create
    const created = await bridge.createManagedSession({
      sessionId: "sess_hb",
      taskId: "task_hb",
      cwd: "/tmp/wt_hb",
    });
    assert.equal(created.sessionId, "sess_hb");

    // Seed agent directly for SessionController.prompt/cancel
    const agent = makeAgent("sess_hb", "/tmp/wt_hb");

    // 2. Send (through patched SessionController -> RunlarkSessionInterceptor)
    await bridge.sendManagedPrompt({
      requestId: reqId("cmd_hb_send"),
      sessionId: SessionId("sess_hb"),
      mode: "queue",
      content: [{ type: "text", text: "Build feature" }],
    });
    assert.equal(agent.followups.length, 1);
    assert.equal(daemonState.commands.at(-1)?.type, "SendPrompt");

    // 3. Forward SessionWireEvent (turn/start, tool/call ask_user_question)
    const fakeSess = agent.session as never;
    ctx.emit("session/event", fakeSess, {
      type: "turn/start",
      seq: SessionSeq(1),
      time: 1010,
      data: { turn: 1 },
    });
    ctx.emit("session/event", fakeSess, {
      type: "tool/call",
      seq: SessionSeq(2),
      time: 1020,
      data: {
        turn: 1,
        step: 1,
        callId: brandString("call_q1"),
        name: "ask_user_question",
        arguments: JSON.stringify({
          questions: [
            {
              id: "q1",
              question: "Proceed?",
              options: [{ label: "Yes" }, { label: "No" }],
            },
          ],
        }),
      },
    });
    await new Promise((r) => setTimeout(r, 30));
    assert.equal(daemonState.dshEvents.length, 2);

    // 4. Rust-originated AnswerQuestion resolves pending waterfall waiter!
    const waterfallPromise = ctx.waterfall(
      "user-questions/request",
      {
        agent: agent as never,
        questions: [
          { id: "q1", question: "Proceed?", options: [{ label: "Yes" }, { label: "No" }] },
        ],
        wait: { callId: ToolCallId("call_q1"), timed: true },
      },
      async () => {
        const err = new Error("no browser client") as Error & { code?: string };
        err.code = "NO_PROVIDER";
        throw err;
      },
    );
    await new Promise((r) => setTimeout(r, 10));
    await bridge.handleOutboxDispatch({
      command_id: "cmd_ans_rust",
      session_id: "sess_hb",
      run_id: "run_sess_hb_cmd_hb_send",
      worktree_path: "/tmp/wt_hb",
      command: {
        type: "AnswerQuestion",
        payload: {
          command_id: "cmd_ans_rust",
          session_id: "sess_hb",
          run_id: "run_sess_hb_cmd_hb_send",
          question_id: "call_q1",
          answer: "Yes",
          expected_revision: 3,
          created_at: 1030,
        },
      },
    });
    const resolvedAns = await waterfallPromise;
    assert.deepEqual(resolvedAns.answers, [{ id: "q1", selected: ["Yes"] }]);

    // 5. Stop -> persists StopRun BEFORE agent.cancel(), stays CancelRequested until turn/end
    await bridge.stopManagedRun("sess_hb");
    assert.equal(agent.cancels.length, 1);
    assert.equal(daemonState.commands.at(-1)?.type, "StopRun");

    // Emit turn/end -> forwarded to daemon, clears active run
    ctx.emit("session/event", fakeSess, {
      type: "turn/end",
      seq: SessionSeq(3),
      time: 1040,
      data: { turn: 1, reason: { kind: "aborted", reason: { kind: "user" } } },
    });
    await new Promise((r) => setTimeout(r, 30));
    assert.equal(daemonState.dshEvents.length, 3);

    // 6. Resume + catch up events
    const resumed = await bridge.resumeManagedSession("sess_hb", { afterSeq: 0 });
    assert.equal(resumed.session.session_id, "sess_hb");

    // 7. ChangeCwd (while idle)
    const updatedCwd = await bridge.changeManagedCwd("sess_hb", "/tmp/wt_hb_v2");
    assert.equal(updatedCwd.cwd, "/tmp/wt_hb_v2");
    assert.equal(daemonState.commands.at(-1)?.type, "ChangeCwd");

    // 8. Archive -> persists ArchiveSession BEFORE calling workspaceRegistry.archiveSession
    await bridge.archiveManagedSession("sess_hb");
    assert.equal(daemonState.commands.at(-1)?.type, "ArchiveSession");
    assert.ok(archivedIds.includes("sess_hb"));

    await ctx.fiber.dispose();
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});

function isWireTurnEnd(event: unknown): boolean {
  return (
    typeof event === "object" &&
    event !== null &&
    "type" in event &&
    (event as { type: unknown }).type === "turn/end"
  );
}
