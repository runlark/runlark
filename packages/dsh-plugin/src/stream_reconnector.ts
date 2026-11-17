import type { DaemonClient } from "./daemon_client.ts";
import type { DaemonEventItem } from "./types.ts";

export class StreamReconnector {
  private lastSeq = 0;
  private readonly client: DaemonClient;
  private readonly runId: string;

  constructor(client: DaemonClient, runId: string, initialSeq = 0) {
    this.client = client;
    this.runId = runId;
    this.lastSeq = initialSeq;
  }

  getLastSeq(): number {
    return this.lastSeq;
  }

  async reconnectAndCatchUp(onEvent: (event: DaemonEventItem) => void): Promise<number> {
    const events = await this.client.getEvents(this.runId, this.lastSeq);
    let caughtUp = 0;

    for (const event of events) {
      if (event.seq > this.lastSeq) {
        onEvent(event);
        this.lastSeq = event.seq;
        caughtUp += 1;
      }
    }

    return caughtUp;
  }
}
