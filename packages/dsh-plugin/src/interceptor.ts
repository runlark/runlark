import { randomUUID } from "node:crypto";
import type { DaemonClient } from "./daemon_client.ts";
import type {
  DaemonSession,
  MessageSource,
  PromptContentPart,
  SessionCancelRequest,
  SessionCancelValue,
  SessionCommand,
  SessionCommandInterceptor,
  SessionId,
  SessionPromptRequest,
  SessionPromptValue,
  SubmitCommandResult,
} from "./types.ts";

const TERMINAL_RUN_STATUSES = new Set(["stopped", "completed", "failed"]);

export function extractPromptText(content: readonly PromptContentPart[]): string {
  const parts: string[] = [];
  for (const part of content) {
    if (part.type === "text") {
      parts.push(part.text);
    } else if (part.type === "image") {
      parts.push(`[image:${part.mediaType}]`);
    } else if (part.type === "file") {
      parts.push(`[file:${part.receiptId}]`);
    }
  }
  return parts.join("\n").trim();
}

export class RunlarkSessionInterceptor implements SessionCommandInterceptor {
  private readonly client: DaemonClient;
  private readonly managedSessions: Set<string>;
  private readonly activeRunIds = new Map<string, string>();
  private readonly sessionRevisions = new Map<string, number>();
  private readonly dispatchedCommandIds = new Set<string>();
  private readonly locallyHandledCommandIds = new Set<string>();

  constructor(client: DaemonClient, managedSessions: Set<string> = new Set()) {
    this.client = client;
    this.managedSessions = managedSessions;
  }

  handles(sessionId: SessionId | string): boolean {
    return this.managedSessions.has(sessionId);
  }

  manageSession(sessionId: string, initialRevision?: number, activeRunId?: string | null): void {
    this.managedSessions.add(sessionId);
    if (initialRevision !== undefined) {
      this.sessionRevisions.set(sessionId, initialRevision);
    }
    if (activeRunId) {
      this.activeRunIds.set(sessionId, activeRunId);
    }
  }

  unmanageSession(sessionId: string): void {
    this.managedSessions.delete(sessionId);
    this.activeRunIds.delete(sessionId);
    this.sessionRevisions.delete(sessionId);
  }

  getKnownRevision(sessionId: string): number | undefined {
    return this.sessionRevisions.get(sessionId);
  }

  getActiveRunId(sessionId: string): string | undefined {
    return this.activeRunIds.get(sessionId);
  }

  setActiveRunId(sessionId: string, runId: string | null): void {
    if (runId) {
      this.activeRunIds.set(sessionId, runId);
    } else {
      this.activeRunIds.delete(sessionId);
    }
  }

  markDispatchedCommand(commandId: string): void {
    this.dispatchedCommandIds.add(commandId);
  }

  consumeDispatchedCommand(commandId: string): boolean {
    return this.dispatchedCommandIds.delete(commandId);
  }

  markLocallyHandledCommand(commandId: string): void {
    if (this.locallyHandledCommandIds.size >= 1000) {
      const oldest = this.locallyHandledCommandIds.values().next().value;
      if (oldest !== undefined) {
        this.locallyHandledCommandIds.delete(oldest);
      }
    }
    this.locallyHandledCommandIds.add(commandId);
  }

  consumeLocallyHandledCommand(commandId: string): boolean {
    return this.locallyHandledCommandIds.delete(commandId);
  }

  async syncSession(sessionId: string): Promise<DaemonSession> {
    const session = await this.client.getSession(sessionId);
    if (!session) {
      this.sessionRevisions.delete(sessionId);
      throw new Error(`Managed session not found in daemon: ${sessionId}`);
    }
    this.sessionRevisions.set(sessionId, session.current_revision);
    if (
      session.active_run_id &&
      (!session.active_run_status || !TERMINAL_RUN_STATUSES.has(session.active_run_status))
    ) {
      this.activeRunIds.set(sessionId, session.active_run_id);
    } else {
      this.activeRunIds.delete(sessionId);
    }
    return session;
  }

  private async resolveSessionSnapshot(sessionId: string): Promise<number> {
    const cached = this.sessionRevisions.get(sessionId);
    if (cached !== undefined) {
      return cached;
    }
    const session = await this.syncSession(sessionId);
    return session.current_revision;
  }

  private async refreshAfterConflict(
    sessionId: string,
    result: SubmitCommandResult,
  ): Promise<void> {
    if (result.actual_revision !== undefined) {
      this.sessionRevisions.set(sessionId, result.actual_revision);
    }
    try {
      await this.syncSession(sessionId);
    } catch {
      if (result.actual_revision === undefined) {
        this.sessionRevisions.delete(sessionId);
      }
    }
  }

  async persistWithRevision(
    sessionId: string,
    buildCommand: (expectedRevision: number) => SessionCommand,
  ): Promise<SubmitCommandResult> {
    const expectedRevision = await this.resolveSessionSnapshot(sessionId);
    const command = buildCommand(expectedRevision);
    const persistResult = await this.client.submitCommand(command);

    if (persistResult.ok) {
      const nextRevision =
        typeof persistResult.current_revision === "number"
          ? persistResult.current_revision
          : expectedRevision + 1;
      this.sessionRevisions.set(sessionId, nextRevision);
      if (persistResult.run_id && persistResult.run_status) {
        if (TERMINAL_RUN_STATUSES.has(persistResult.run_status)) {
          this.activeRunIds.delete(sessionId);
        } else {
          this.activeRunIds.set(sessionId, persistResult.run_id);
        }
      }
      return persistResult;
    }

    if (persistResult.conflict) {
      await this.refreshAfterConflict(sessionId, persistResult);
    }

    return persistResult;
  }

  async prompt(
    request: SessionPromptRequest,
    _source: MessageSource,
    next: () => Promise<SessionPromptValue>,
  ): Promise<SessionPromptValue> {
    if (this.consumeDispatchedCommand(request.requestId)) {
      return await next();
    }

    const commandId = request.requestId || randomUUID();
    await this.resolveSessionSnapshot(request.sessionId);
    const existingRunId = this.activeRunIds.get(request.sessionId);
    const runId = existingRunId ?? `run_${request.sessionId}_${commandId}`;
    const promptText = extractPromptText(request.content);

    // 1. PERSIST-BEFORE-SEND: Must be stored in SQLite before external effect
    this.markLocallyHandledCommand(commandId);
    const persistResult = await this.persistWithRevision(request.sessionId, (expectedRevision) => ({
      type: "SendPrompt",
      payload: {
        command_id: commandId,
        session_id: request.sessionId,
        run_id: runId,
        prompt: promptText,
        expected_revision: expectedRevision,
        created_at: Date.now(),
      },
    }));

    if (!persistResult.ok) {
      this.locallyHandledCommandIds.delete(commandId);
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
    const commandId = randomUUID();
    await this.resolveSessionSnapshot(request.sessionId);
    const runId = this.activeRunIds.get(request.sessionId) ?? `run_${request.sessionId}`;

    // 1. Record intent to cancel before stopping loop (remains CancelRequested until turn/end)
    this.markLocallyHandledCommand(commandId);
    const persistResult = await this.persistWithRevision(request.sessionId, (expectedRevision) => ({
      type: "StopRun",
      payload: {
        command_id: commandId,
        session_id: request.sessionId,
        run_id: runId,
        reason: "user_cancel",
        expected_revision: expectedRevision,
        created_at: Date.now(),
      },
    }));

    if (!persistResult.ok) {
      this.locallyHandledCommandIds.delete(commandId);
      throw new Error(`Cancel persist failed: ${persistResult.error}`);
    }

    // 2. Signal stop to DSH agent loop
    return next();
  }
}
