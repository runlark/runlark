import assert from "node:assert/strict";
import { test } from "node:test";
import { DaemonClient } from "../src/daemon_client.ts";
import { RunlarkSessionInterceptor } from "../src/interceptor.ts";
import type { SessionCommand, SubmitCommandResult } from "../src/types.ts";

class MockDaemonClient extends DaemonClient {
  public submittedCommands: SessionCommand[] = [];
  public failNext = false;

  constructor() {
    super("http://127.0.0.1:9999");
  }

  override async submitCommand(command: SessionCommand): Promise<SubmitCommandResult> {
    if (this.failNext) {
      return { ok: false, error: "Database locked" };
    }
    this.submittedCommands.push(command);
    return { ok: true, command_id: command.payload.command_id, status: "pending" };
  }
}

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
  const interceptor = new RunlarkSessionInterceptor(client, new Set(["sess_1"]));

  let nextCalled = false;
  const result = await interceptor.prompt(
    { sessionId: "sess_1", prompt: "Write unit tests" },
    "user",
    async () => {
      // Must only be called after command is recorded
      assert.equal(client.submittedCommands.length, 1);
      assert.equal(client.submittedCommands[0].type, "SendPrompt");
      nextCalled = true;
      return { turnId: "turn_1" };
    },
  );

  assert.equal(nextCalled, true);
  assert.equal(result.turnId, "turn_1");
});

test("prompt aborts when persist-before-send fails: next() is NEVER called", async () => {
  const client = new MockDaemonClient();
  client.failNext = true;
  const interceptor = new RunlarkSessionInterceptor(client, new Set(["sess_1"]));

  let nextCalled = false;
  await assert.rejects(async () => {
    await interceptor.prompt(
      { sessionId: "sess_1", prompt: "Failing prompt" },
      "user",
      async () => {
        nextCalled = true;
        return { turnId: "turn_fail" };
      },
    );
  }, /Persist-before-send failed/);

  assert.equal(nextCalled, false, "next() must NOT be called if persistence failed");
});

test("cancel records StopRun command prior to signaling stop to agent loop", async () => {
  const client = new MockDaemonClient();
  const interceptor = new RunlarkSessionInterceptor(client, new Set(["sess_1"]));

  let nextCalled = false;
  const cancelResult = await interceptor.cancel(
    { sessionId: "sess_1", reason: "User abort" },
    () => {
      assert.equal(client.submittedCommands.length, 1);
      assert.equal(client.submittedCommands[0].type, "StopRun");
      nextCalled = true;
      return { cancelled: true };
    },
  );

  assert.equal(nextCalled, true);
  assert.equal(cancelResult.cancelled, true);
});

test("cancel addresses StopRun to the active runId recorded by prompt", async () => {
  const client = new MockDaemonClient();
  const interceptor = new RunlarkSessionInterceptor(client, new Set(["sess_custom"]));

  await interceptor.prompt(
    { sessionId: "sess_custom", runId: "custom_run_42", prompt: "Start run" },
    "user",
    async () => ({ turnId: "turn_1" }),
  );

  await interceptor.cancel({ sessionId: "sess_custom", reason: "Stop custom" }, () => ({
    cancelled: true,
  }));

  assert.equal(client.submittedCommands.length, 2);
  assert.equal(client.submittedCommands[0].payload.run_id, "custom_run_42");
  assert.equal(client.submittedCommands[1].type, "StopRun");
  assert.equal(client.submittedCommands[1].payload.run_id, "custom_run_42");
});
