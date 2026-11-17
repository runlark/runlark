import assert from "node:assert/strict";
import { test } from "node:test";
import { DaemonClient } from "../src/daemon_client.ts";
import { StreamReconnector } from "../src/stream_reconnector.ts";
import type { DaemonEventItem } from "../src/types.ts";

class MockEventDaemonClient extends DaemonClient {
  public eventLog: DaemonEventItem[] = [];

  constructor() {
    super("http://127.0.0.1:9999");
  }

  override async getEvents(_runId: string, afterSeq = 0): Promise<DaemonEventItem[]> {
    return this.eventLog.filter((e) => e.seq > afterSeq);
  }
}

test("StreamReconnector catches up on missed events after browser reconnects", async () => {
  const client = new MockEventDaemonClient();
  client.eventLog = [
    { seq: 1, event_type: "PromptEnqueued", payload: { prompt: "P1" }, created_at: 100 },
    { seq: 2, event_type: "RunCreated", payload: { worktree: "/w" }, created_at: 110 },
  ];

  const reconnector = new StreamReconnector(client, "run_123", 0);
  const received: DaemonEventItem[] = [];

  // Initial catch up: receives seq 1 and 2
  const count1 = await reconnector.reconnectAndCatchUp((event) => {
    received.push(event);
  });
  assert.equal(count1, 2);
  assert.equal(received.length, 2);
  assert.equal(reconnector.getLastSeq(), 2);

  // Simulate browser disconnect: 3 more events occur in daemon while client is offline
  client.eventLog.push(
    { seq: 3, event_type: "StepStarted", payload: { step: 1 }, created_at: 120 },
    { seq: 4, event_type: "StepFinished", payload: { step: 1 }, created_at: 130 },
    { seq: 5, event_type: "RunCompleted", payload: {}, created_at: 140 },
  );

  // Client reconnects: requests events after seq 2
  const count2 = await reconnector.reconnectAndCatchUp((event) => {
    received.push(event);
  });

  // Verifies only the 3 new events were delivered, no duplicates
  assert.equal(count2, 3);
  assert.equal(received.length, 5);
  assert.deepEqual(
    received.map((e) => e.seq),
    [1, 2, 3, 4, 5],
  );
  assert.equal(reconnector.getLastSeq(), 5);

  // Re-checking when no new events occur yields 0 delivered
  const count3 = await reconnector.reconnectAndCatchUp((event) => {
    received.push(event);
  });
  assert.equal(count3, 0);
  assert.equal(received.length, 5);
});

test("StreamReconnector does not advance lastSeq if onEvent throws", async () => {
  const client = new MockEventDaemonClient();
  client.eventLog = [
    { seq: 1, event_type: "PromptEnqueued", payload: { prompt: "P1" }, created_at: 100 },
    { seq: 2, event_type: "RunCreated", payload: { worktree: "/w" }, created_at: 110 },
  ];

  const reconnector = new StreamReconnector(client, "run_err", 0);
  const delivered: number[] = [];

  await assert.rejects(async () => {
    await reconnector.reconnectAndCatchUp((event) => {
      if (event.seq === 2) {
        throw new Error("transient UI error");
      }
      delivered.push(event.seq);
    });
  }, /transient UI error/);

  assert.equal(reconnector.getLastSeq(), 1);
  assert.deepEqual(delivered, [1]);

  const retried = await reconnector.reconnectAndCatchUp((event) => {
    delivered.push(event.seq);
  });
  assert.equal(retried, 1);
  assert.equal(reconnector.getLastSeq(), 2);
  assert.deepEqual(delivered, [1, 2]);
});
