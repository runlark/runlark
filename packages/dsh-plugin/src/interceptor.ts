import type { DaemonClient } from "./daemon_client.ts";
import type {
  MessageSource,
  SessionCancelRequest,
  SessionCancelValue,
  SessionCommandInterceptor,
  SessionPromptRequest,
  SessionPromptValue,
} from "./types.ts";

export class RunlarkSessionInterceptor implements SessionCommandInterceptor {
  private readonly client: DaemonClient;
  private readonly managedSessions: Set<string>;
  private readonly activeRunIds = new Map<string, string>();

  constructor(client: DaemonClient, managedSessions: Set<string>) {
    this.client = client;
    this.managedSessions = managedSessions;
  }

  handles(sessionId: string): boolean {
    return this.managedSessions.has(sessionId);
  }

  manageSession(sessionId: string): void {
    this.managedSessions.add(sessionId);
  }

  unmanageSession(sessionId: string): void {
    this.managedSessions.delete(sessionId);
    this.activeRunIds.delete(sessionId);
  }

  async prompt(
    request: SessionPromptRequest,
    _source: MessageSource,
    next: () => Promise<SessionPromptValue>,
  ): Promise<SessionPromptValue> {
    const commandId = crypto.randomUUID();
    const runId = request.runId ?? `run_${request.sessionId}`;

    // 1. PERSIST-BEFORE-SEND: Must be stored in SQLite before external effect
    const persistResult = await this.client.submitCommand({
      type: "SendPrompt",
      payload: {
        command_id: commandId,
        session_id: request.sessionId,
        run_id: runId,
        prompt: request.prompt,
        expected_revision: 1,
        created_at: Date.now(),
      },
    });

    if (!persistResult.ok) {
      throw new Error(`Persist-before-send failed: ${persistResult.error}`);
    }

    this.activeRunIds.set(request.sessionId, runId);

    // 2. Dispatch effect to DSH agent loop
    return await next();
  }

  async cancel(
    request: SessionCancelRequest,
    next: () => SessionCancelValue,
  ): Promise<SessionCancelValue> {
    const commandId = crypto.randomUUID();
    const runId = this.activeRunIds.get(request.sessionId) ?? `run_${request.sessionId}`;

    // 1. Record intent to cancel before stopping loop
    const persistResult = await this.client.submitCommand({
      type: "StopRun",
      payload: {
        command_id: commandId,
        session_id: request.sessionId,
        run_id: runId,
        reason: request.reason ?? "user_cancel",
        expected_revision: 1,
        created_at: Date.now(),
      },
    });

    if (!persistResult.ok) {
      throw new Error(`Cancel persist failed: ${persistResult.error}`);
    }

    // 2. Signal stop to DSH agent loop
    return next();
  }
}
