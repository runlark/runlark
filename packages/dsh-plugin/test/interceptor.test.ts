import assert from "node:assert/strict";
import { test } from "node:test";
import { brandString } from "@deepseek-ai/dsh-brand";
import { SessionId } from "@deepseek-ai/dsh-session";
import { DaemonClient } from "../src/daemon_client.ts";
import { RunlarkSessionInterceptor } from "../src/interceptor.ts";
import type {
  DaemonSession,
  MessageSource,
  SessionCommand,
  SessionRequestId,
  SubmitCommandResult,
} from "../src/types.ts";

class MockDaemonClient extends DaemonClient {
  public sessions = new Map<string, DaemonSession>();
  public submittedCommands: SessionCommand[] = [];
  public failNext = false;

  constructor() {
    super("http://127.0.0.1:9999");
  }

  seedSession(sessionId: string, revision = 1, activeRunId: string | null = null): void {
    this.sessions.set(sessionId, {
      session_id: sessionId,
      task_id: `task_${sessionId}`,
      cwd: "/tmp/workspace",
      status: revision > 1 ? "active" : "created",
      current_revision: revision,
      active_run_id: activeRunId,
      created_at: 1000,
      updated_at: 1000,
    });
  }

  override async getSession(sessionId: string): Promise<DaemonSession | null> {
    const sess = this.sessions.get(sessionId);
    return sess ? { ...sess } : null;
  }

  override async submitCommand(command: SessionCommand): Promise<SubmitCommandResult> {
    if (this.failNext) {
      this.failNext = false;
      return { ok: false, error: "Database locked" };
    }

    const sess = this.sessions.get(command.payload.session_id);
    if (!sess) {
      return { ok: false, error: "Daemon error (404): Session not found" };
    }

    if (command.payload.expected_revision !== sess.current_revision) {
      return {
        ok: false,
        conflict: true,
        error_code: "revision_mismatch",
        actual_revision: sess.current_revision,
        error: `Revision mismatch: expected ${command.payload.expected_revision}, actual ${sess.current_revision}`,
      };
    }

    sess.current_revision += 1;
    if (command.type === "SendPrompt") {
      sess.active_run_id = command.payload.run_id;
    }
    this.submittedCommands.push(command);
    return {
      ok: true,
      command_id: command.payload.command_id,
      status: "pending",
      current_revision: sess.current_revision,
      run_id: command.payload.run_id,
      run_status: command.type === "StopRun" ? "cancel_requested" : "running",
    };
  }
}

const reqId = (id: string): SessionRequestId => brandString<SessionRequestId>(id);
const userSource = (id: string): MessageSource => ({ kind: "user", rpcId: reqId(id) });

test("handles returns true only for managed sessions", () => {
  const client = new MockDaemonClient();
  const interceptor = new RunlarkSessionInterceptor(client, new Set(["sess_managed"]));

  assert.equal(interceptor.handles("sess_managed"), true);
  assert.equal(interceptor.handles("sess_other"), false);

  interceptor.manageSession("sess_other");
  assert.equal(interceptor.handles("sess_other"), true);

  interceptor.unmanageSession("sess_other");
  assert.equal(interceptor.handles("sess_other"), false);
});

test("prompt enforces persist-before-send: next() is called only after persistence succeeds", async () => {
  const client = new MockDaemonClient();
  client.seedSession("sess_1", 1);
  const interceptor = new RunlarkSessionInterceptor(client, new Set(["sess_1"]));

  let nextCalled = false;
  const result = await interceptor.prompt(
    {
      requestId: reqId("rpc_1"),
      sessionId: SessionId("sess_1"),
      mode: "queue",
      content: [{ type: "text", text: "Write unit tests" }],
    },
    userSource("rpc_1"),
    async () => {
      assert.equal(client.submittedCommands.length, 1);
      assert.equal(client.submittedCommands[0].type, "SendPrompt");
      assert.equal(client.submittedCommands[0].payload.expected_revision, 1);
      assert.equal(client.submittedCommands[0].payload.command_id, "rpc_1");
      nextCalled = true;
      return { accepted: true };
    },
  );

  assert.equal(nextCalled, true);
  assert.equal(result.accepted, true);
  assert.equal(interceptor.getKnownRevision("sess_1"), 2);
});

test("prompt aborts when persist-before-send fails or session missing: next() is NEVER called", async () => {
  const client = new MockDaemonClient();
  client.seedSession("sess_1", 1);
  client.failNext = true;
  const interceptor = new RunlarkSessionInterceptor(client, new Set(["sess_1", "sess_missing"]));

  let nextCalled = false;
  await assert.rejects(async () => {
    await interceptor.prompt(
      {
        requestId: reqId("rpc_fail"),
        sessionId: SessionId("sess_1"),
        mode: "queue",
        content: [{ type: "text", text: "Failing prompt" }],
      },
      userSource("rpc_fail"),
      async () => {
        nextCalled = true;
        return { accepted: true };
      },
    );
  }, /Persist-before-send failed/);

  assert.equal(nextCalled, false, "next() must NOT be called if persistence failed");

  await assert.rejects(async () => {
    await interceptor.prompt(
      {
        requestId: reqId("rpc_missing"),
        sessionId: SessionId("sess_missing"),
        mode: "queue",
        content: [{ type: "text", text: "Should fail" }],
      },
      userSource("rpc_missing"),
      async () => {
        nextCalled = true;
        return { accepted: true };
      },
    );
  }, /Managed session not found in daemon/);
  assert.equal(nextCalled, false);
});

test("tracks dynamic revisions across multiple commands (rev 1 -> 2 -> 3 -> 4)", async () => {
  const client = new MockDaemonClient();
  client.seedSession("sess_seq", 1);
  const interceptor = new RunlarkSessionInterceptor(client, new Set(["sess_seq"]));

  await interceptor.prompt(
    {
      requestId: reqId("rpc_step1"),
      sessionId: SessionId("sess_seq"),
      mode: "queue",
      content: [{ type: "text", text: "Step 1" }],
    },
    userSource("rpc_step1"),
    async () => ({ accepted: true }),
  );
  await interceptor.prompt(
    {
      requestId: reqId("rpc_step2"),
      sessionId: SessionId("sess_seq"),
      mode: "steer",
      content: [{ type: "text", text: "Step 2" }],
    },
    userSource("rpc_step2"),
    async () => ({ accepted: true }),
  );
  await interceptor.cancel({ sessionId: SessionId("sess_seq") }, () => ({
    accepted: true,
  }));

  assert.equal(client.submittedCommands.length, 3);
  assert.equal(client.submittedCommands[0].payload.expected_revision, 1);
  assert.equal(client.submittedCommands[1].payload.expected_revision, 2);
  assert.equal(client.submittedCommands[2].type, "StopRun");
  assert.equal(client.submittedCommands[2].payload.expected_revision, 3);
  assert.equal(
    client.submittedCommands[2].payload.run_id,
    client.submittedCommands[0].payload.run_id,
  );
  assert.equal(interceptor.getKnownRevision("sess_seq"), 4);
});

test("fetches initial non-1 revision from daemon and refreshes cache on 409 conflict", async () => {
  const client = new MockDaemonClient();
  client.seedSession("sess_sync", 5, "run_existing");
  const interceptor = new RunlarkSessionInterceptor(client, new Set(["sess_sync"]));

  await interceptor.prompt(
    {
      requestId: reqId("rpc_sync_1"),
      sessionId: SessionId("sess_sync"),
      mode: "queue",
      content: [{ type: "text", text: "Continue" }],
    },
    userSource("rpc_sync_1"),
    async () => ({ accepted: true }),
  );
  assert.equal(client.submittedCommands[0].payload.expected_revision, 5);
  assert.equal(client.submittedCommands[0].payload.run_id, "run_existing");
  assert.equal(interceptor.getKnownRevision("sess_sync"), 6);

  const remoteSession = client.sessions.get("sess_sync")!;
  remoteSession.current_revision = 8;

  let nextCalledOnConflict = false;
  await assert.rejects(async () => {
    await interceptor.prompt(
      {
        requestId: reqId("rpc_sync_conflict"),
        sessionId: SessionId("sess_sync"),
        mode: "queue",
        content: [{ type: "text", text: "Stale rev prompt" }],
      },
      userSource("rpc_sync_conflict"),
      async () => {
        nextCalledOnConflict = true;
        return { accepted: true };
      },
    );
  }, /Persist-before-send failed: Revision mismatch/);

  assert.equal(nextCalledOnConflict, false);
  assert.equal(interceptor.getKnownRevision("sess_sync"), 8);

  await interceptor.prompt(
    {
      requestId: reqId("rpc_sync_2"),
      sessionId: SessionId("sess_sync"),
      mode: "queue",
      content: [{ type: "text", text: "Fresh rev prompt" }],
    },
    userSource("rpc_sync_2"),
    async () => ({ accepted: true }),
  );
  assert.equal(client.submittedCommands.length, 2);
  assert.equal(client.submittedCommands[1].payload.expected_revision, 8);
  assert.equal(interceptor.getKnownRevision("sess_sync"), 9);
});
