import { randomUUID } from "node:crypto";
import type { Context } from "@deepseek-ai/cordis";
import { brandString } from "@deepseek-ai/dsh-brand";
import { SessionId as brandSessionId } from "@deepseek-ai/dsh-session";
import type { SessionCommandInterceptor } from "@deepseek-ai/dsh-api-session-controller";
import type {} from "@deepseek-ai/dsh-host-webserver";
import type {} from "@deepseek-ai/dsh-workspace";
import { DaemonBoardClient } from "./daemon_board_client.ts";
import type { DaemonClient } from "./daemon_client.ts";
import type { RunlarkSessionInterceptor } from "./interceptor.ts";
import { QuestionCoordinator } from "./questions.ts";
import { StreamReconnector } from "./stream_reconnector.ts";
import { registerRunlarkChatTool } from "./tools.ts";
import { UsageBridge } from "./usage_bridge.ts";
import { handleRunlarkWebRequest } from "./web_routes.ts";
import type {
  DaemonEventItem,
  DaemonSession,
  OutboxDispatchRequest,
  SessionCreateValue,
  SessionEvent,
  SessionPromptRequest,
  SessionPromptValue,
  SessionRequestId,
} from "./types.ts";

export interface ManagedSessionCreateOptions {
  sessionId: string;
  taskId: string;
  cwd: string;
  agentPreset?: string;
}

export interface ManagedResumeOptions {
  afterSeq?: number;
  activate?: boolean;
  onEvent?: (event: DaemonEventItem) => void;
}

export class RunlarkHostBridge {
  private readonly ctx: Context;
  private readonly client: DaemonClient;
  private readonly interceptor: RunlarkSessionInterceptor;
  private readonly questions: QuestionCoordinator;
  private readonly boardClient: DaemonBoardClient;
  private readonly usageBridge: UsageBridge;
  private readonly eventQueues = new Map<string, Promise<void>>();
  private readonly dshArchivedInternally = new Set<string>();

  constructor(ctx: Context, client: DaemonClient, interceptor: RunlarkSessionInterceptor) {
    this.ctx = ctx;
    this.client = client;
    this.interceptor = interceptor;
    this.questions = new QuestionCoordinator(ctx, client, interceptor);
    this.boardClient = new DaemonBoardClient(client.getBaseUrl());
    this.usageBridge = new UsageBridge(this.boardClient);
  }

  getInterceptor(): SessionCommandInterceptor {
    return this.interceptor;
  }

  async syncManagedSessionsFromDaemon(): Promise<DaemonSession[]> {
    const sessions = await this.client.listSessions();
    for (const session of sessions) {
      if (session.status === "archived") {
        this.interceptor.unmanageSession(session.session_id);
        continue;
      }
      this.interceptor.manageSession(
        session.session_id,
        session.current_revision,
        session.active_run_id,
      );
    }
    return sessions;
  }

  registerCordisListeners(): void {
    this.ctx.on("session/event", (session, event) => {
      if (!this.interceptor.handles(session.id)) return;
      if (event.type === "tool/call" && event.data.name === "ask_user_question") {
        this.questions.trackToolCall(session.id, event.data.callId, event.data.arguments);
      }
      this.enqueueSessionEvent(session.id, event);
    });

    this.ctx.on("workspace/session-activity", async (request, next) => {
      const upstream = await next();
      if (!this.interceptor.handles(request.sessionId)) {
        return upstream;
      }
      if (upstream.some((entry) => entry.kind === "turn")) {
        return upstream;
      }
      try {
        const daemonSession = await this.client.getSession(request.sessionId);
        if (
          daemonSession?.active_run_status &&
          ["queued", "running", "waiting_for_approval", "cancel_requested"].includes(
            daemonSession.active_run_status,
          )
        ) {
          return [{ kind: "turn" }, ...upstream];
        }
      } catch {
        if (this.interceptor.getActiveRunId(request.sessionId)) {
          return [{ kind: "turn" }, ...upstream];
        }
      }
      return upstream;
    });

    this.ctx.on("workspace/session-stop", async (request) => {
      if (!this.interceptor.handles(request.sessionId)) return;
      if (this.dshArchivedInternally.has(request.sessionId)) return;
      try {
        await this.ctx.sessionController.cancel({
          sessionId: brandSessionId(request.sessionId),
        });
      } catch (error) {
        this.ctx.logger?.warn?.("runlark: workspace/session-stop cancel failed: %s", error);
      }
    });

    this.ctx.on("workspace/session-archived", async (request) => {
      if (!this.interceptor.handles(request.sessionId)) return;
      if (this.dshArchivedInternally.delete(request.sessionId)) return;
      try {
        await this.persistArchiveToDaemon(request.sessionId);
        this.interceptor.unmanageSession(request.sessionId);
      } catch (error) {
        this.ctx.logger?.warn?.("runlark: workspace/session-archived sync failed: %s", error);
      }
    });

    this.questions.registerWaterfallListener();
    registerRunlarkChatTool(this.ctx, this.boardClient);

    this.ctx.inject(["webServer"], (webCtx) => {
      webCtx.effect(
        () =>
          webCtx.webServer.register({
            kind: "prefix",
            path: "/api/runlark",
            handler: (req, res) => handleRunlarkWebRequest(req, res, this, this.client),
          }),
        "runlark: /api/runlark/*",
      );
    });
  }

  private enqueueSessionEvent(sessionId: string, event: SessionEvent): void {
    const prev = this.eventQueues.get(sessionId) ?? Promise.resolve();
    const next = prev
      .then(() => this.forwardSessionEvent(sessionId, event))
      .catch((error: unknown) => {
        this.ctx.logger?.warn?.("runlark: failed to forward session/event: %s", error);
      });
    this.eventQueues.set(sessionId, next);
  }

  private async forwardSessionEvent(sessionId: string, event: SessionEvent): Promise<void> {
    this.usageBridge.observeEvent(sessionId, event);
    if (event.type === "tool/call" && event.data.name === "ask_user_question") {
      this.questions.trackToolCall(sessionId, event.data.callId, event.data.arguments);
    }

    const runId = this.interceptor.getActiveRunId(sessionId);
    if (event.type === "turn/end") {
      if (runId) {
        await this.usageBridge
          .recordTurnEndUsage(sessionId, runId, event)
          .catch((error: unknown) => {
            this.ctx.logger?.warn?.("runlark: failed to record turn/end usage: %s", error);
          });
      }
      this.interceptor.setActiveRunId(sessionId, null);
      this.questions.clearSession(sessionId);
    }

    await this.client.postDshEvent({
      session_id: sessionId,
      ...(runId ? { run_id: runId } : {}),
      event: {
        type: event.type,
        seq: event.seq,
        time: event.time,
        data: event.data,
      },
    });

    if (event.type === "turn/start" || event.type === "turn/end") {
      await this.interceptor.syncSession(sessionId).catch(() => {});
    }
  }

  async ensureManagedSession(opts: ManagedSessionCreateOptions): Promise<DaemonSession> {
    let existing = await this.client.getSession(opts.sessionId);
    if (!existing) {
      existing = await this.client.createSession({
        session_id: opts.sessionId,
        task_id: opts.taskId,
        cwd: opts.cwd,
      });
    }
    this.interceptor.manageSession(
      existing.session_id,
      existing.current_revision,
      existing.active_run_id,
    );
    return existing;
  }

  async applyQuestionAnswer(sessionId: string, questionId: string, answer: string): Promise<void> {
    await this.questions.applyExternalAnswer(sessionId, questionId, answer);
  }

  async createManagedSession(opts: ManagedSessionCreateOptions): Promise<SessionCreateValue> {
    const existing = await this.client.getSession(opts.sessionId);
    if (!existing) {
      await this.client.createSession({
        session_id: opts.sessionId,
        task_id: opts.taskId,
        cwd: opts.cwd,
      });
    }
    const created = await this.ctx.sessionController.create({
      sessionId: brandSessionId(opts.sessionId),
      cwd: opts.cwd,
      ...(opts.agentPreset !== undefined ? { agentPreset: opts.agentPreset } : {}),
    });
    const synced = await this.client.getSession(created.sessionId);
    this.interceptor.manageSession(
      created.sessionId,
      synced?.current_revision ?? 1,
      synced?.active_run_id ?? null,
    );
    return created;
  }

  async sendManagedPrompt(
    request: SessionPromptRequest,
    signal: AbortSignal = new AbortController().signal,
  ): Promise<SessionPromptValue> {
    return await this.ctx.sessionController.prompt(request, signal);
  }

  async stopManagedRun(sessionId: string): Promise<{ accepted: true }> {
    return await this.ctx.sessionController.cancel({
      sessionId: brandSessionId(sessionId),
    });
  }

  async resumeManagedSession(
    sessionId: string,
    opts: ManagedResumeOptions = {},
  ): Promise<{ session: DaemonSession; caughtUp: number }> {
    const session = await this.interceptor.syncSession(sessionId);
    this.interceptor.manageSession(sessionId, session.current_revision, session.active_run_id);

    if (session.active_run_id) {
      const commandId = `resume_${sessionId}_${session.current_revision}_${randomUUID()}`;
      const runId = session.active_run_id;
      const result = await this.interceptor.persistWithRevision(sessionId, (expectedRevision) => ({
        type: "ResumeSession",
        payload: {
          command_id: commandId,
          session_id: sessionId,
          run_id: runId,
          ...(opts.afterSeq !== undefined ? { after_seq: opts.afterSeq } : {}),
          expected_revision: expectedRevision,
          created_at: Date.now(),
        },
      }));
      if (!result.ok) {
        throw new Error(`ResumeSession persist failed: ${result.error}`);
      }
    }

    const resolved = await this.ctx.sessionController.resolveAgent(brandSessionId(sessionId));
    if ("error" in resolved) {
      throw resolved.error;
    }

    let caughtUp = 0;
    if (session.active_run_id && opts.onEvent) {
      const reconnector = new StreamReconnector(
        this.client,
        session.active_run_id,
        opts.afterSeq ?? 0,
      );
      caughtUp = await reconnector.reconnectAndCatchUp(opts.onEvent);
    }

    const updated = await this.interceptor.syncSession(sessionId);
    return { session: updated, caughtUp };
  }

  async changeManagedCwd(sessionId: string, newCwd: string): Promise<DaemonSession> {
    const session = await this.interceptor.syncSession(sessionId);
    const commandId = `cwd_${sessionId}_${randomUUID()}`;
    const runId = session.active_run_id ?? `run_${sessionId}`;
    const result = await this.interceptor.persistWithRevision(sessionId, (expectedRevision) => ({
      type: "ChangeCwd",
      payload: {
        command_id: commandId,
        session_id: sessionId,
        run_id: runId,
        new_cwd: newCwd,
        expected_revision: expectedRevision,
        created_at: Date.now(),
      },
    }));
    if (!result.ok) {
      throw new Error(`ChangeCwd persist failed: ${result.error}`);
    }
    return await this.interceptor.syncSession(sessionId);
  }

  private async persistArchiveToDaemon(sessionId: string): Promise<void> {
    const session = await this.interceptor.syncSession(sessionId);
    const commandId = `archive_${sessionId}_${session.current_revision}`;
    const runId = session.active_run_id ?? `run_${sessionId}`;
    const result = await this.interceptor.persistWithRevision(sessionId, (expectedRevision) => ({
      type: "ArchiveSession",
      payload: {
        command_id: commandId,
        session_id: sessionId,
        run_id: runId,
        expected_revision: expectedRevision,
        created_at: Date.now(),
      },
    }));
    if (!result.ok) {
      throw new Error(`ArchiveSession persist failed: ${result.error}`);
    }
  }

  async archiveManagedSession(
    sessionId: string,
    opts: { stopActivity?: boolean } = {},
  ): Promise<void> {
    await this.persistArchiveToDaemon(sessionId);
    this.dshArchivedInternally.add(sessionId);
    try {
      await this.ctx.workspaceRegistry.archiveSession(brandSessionId(sessionId), {
        stopActivity: opts.stopActivity ?? true,
      });
    } finally {
      this.dshArchivedInternally.delete(sessionId);
    }
    this.interceptor.unmanageSession(sessionId);
  }

  async handleOutboxDispatch(req: OutboxDispatchRequest): Promise<void> {
    const {
      command_id: commandId,
      session_id: sessionId,
      command,
      worktree_path: worktreePath,
    } = req;
    this.interceptor.manageSession(sessionId);

    switch (command.type) {
      case "SendPrompt": {
        if (this.interceptor.consumeLocallyHandledCommand(commandId)) {
          return;
        }
        const sid = brandSessionId(sessionId);
        const existingAgent = this.ctx.agents.get(sid);
        if (!existingAgent) {
          const resolved = await this.ctx.sessionController.resolveAgent(sid);
          if ("error" in resolved) {
            const targetCwd = worktreePath ?? process.cwd();
            await this.ctx.sessionController.create({
              sessionId: sid,
              cwd: targetCwd,
            });
          }
        }
        this.interceptor.setActiveRunId(sessionId, command.payload.run_id);
        this.interceptor.markDispatchedCommand(commandId);
        await this.ctx.sessionController.prompt(
          {
            requestId: brandString<SessionRequestId>(commandId),
            sessionId: sid,
            mode: "queue",
            content: [{ type: "text", text: command.payload.prompt }],
          },
          new AbortController().signal,
        );
        return;
      }
      case "StopRun": {
        if (this.interceptor.consumeLocallyHandledCommand(commandId)) {
          return;
        }
        const sid = brandSessionId(sessionId);
        const agent = this.ctx.agents.get(sid);
        if (agent) {
          agent.cancel({ kind: "user" }, { keepInbox: true });
        }
        return;
      }
      case "ResumeSession": {
        const sid = brandSessionId(sessionId);
        this.interceptor.setActiveRunId(sessionId, command.payload.run_id);
        const resolved = await this.ctx.sessionController.resolveAgent(sid);
        if ("error" in resolved) {
          throw resolved.error;
        }
        return;
      }
      case "ChangeCwd": {
        return;
      }
      case "ArchiveSession": {
        const sid = brandSessionId(sessionId);
        this.dshArchivedInternally.add(sessionId);
        try {
          await this.ctx.workspaceRegistry.archiveSession(sid, { stopActivity: true });
        } finally {
          this.dshArchivedInternally.delete(sessionId);
        }
        this.interceptor.unmanageSession(sessionId);
        return;
      }
      case "AnswerQuestion": {
        await this.questions.applyExternalAnswer(
          sessionId,
          command.payload.question_id,
          command.payload.answer,
        );
        return;
      }
    }
  }
}
