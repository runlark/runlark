export type RunStatus =
  | "queued"
  | "running"
  | "waiting_for_approval"
  | "cancel_requested"
  | "stopped"
  | "completed"
  | "failed"
  | "interrupted"
  | "unknown";

export type SessionStatus = "created" | "active" | "archived" | "unknown";

export interface DaemonQuestion {
  readonly question_id: string;
  readonly session_id: string;
  readonly run_id: string;
  readonly prompt: string;
  readonly options: readonly string[];
  readonly status: string;
  readonly answer: string | null;
  readonly created_at: number;
  readonly updated_at: number;
}

export interface DaemonSession {
  readonly session_id: string;
  readonly task_id: string;
  readonly cwd: string;
  readonly status: string;
  readonly current_revision: number;
  readonly active_run_id: string | null;
  readonly active_run_status?: string | null;
  readonly worktree_path?: string | null;
  readonly latest_run_id?: string | null;
  readonly latest_run_status?: string | null;
  readonly latest_worktree_path?: string | null;
  readonly dispatched_commands_count?: number;
  readonly pending_question?: DaemonQuestion | null;
  readonly created_at: number;
  readonly updated_at: number;
}

export interface DaemonEventItem {
  readonly run_id?: string;
  readonly seq: number;
  readonly event_type: string;
  readonly payload: unknown;
  readonly created_at: number;
}

export interface AnswerQuestionResult {
  readonly command_id: string;
  readonly status: string;
  readonly question_id: string;
  readonly current_revision: number;
  readonly run_status?: string;
}

export interface CommandActionResult {
  readonly command_id?: string;
  readonly status?: string;
  readonly current_revision?: number;
  readonly run_id?: string | null;
  readonly run_status?: string | null;
}

export interface RunlarkApiClient {
  getSession(sessionId: string, signal?: AbortSignal): Promise<DaemonSession | null>;
  ensureSession(input: {
    readonly sessionId: string;
    readonly taskId: string;
    readonly cwd: string;
  }): Promise<DaemonSession>;
  listSessionQuestions(sessionId: string, signal?: AbortSignal): Promise<readonly DaemonQuestion[]>;
  getSessionEvents(
    sessionId: string,
    opts?: {
      readonly runId?: string;
      readonly afterSeq?: number;
      readonly waitMs?: number;
      readonly signal?: AbortSignal;
    },
  ): Promise<readonly DaemonEventItem[]>;
  sendPrompt(input: {
    readonly sessionId: string;
    readonly prompt: string;
    readonly expectedRevision: number;
    readonly activeRunId?: string | null;
  }): Promise<CommandActionResult>;
  stopRun(input: {
    readonly sessionId: string;
    readonly runId: string;
    readonly expectedRevision: number;
  }): Promise<CommandActionResult>;
  resumeSession(input: {
    readonly sessionId: string;
    readonly runId: string;
    readonly expectedRevision: number;
    readonly afterSeq?: number;
  }): Promise<CommandActionResult>;
  archiveSession(input: {
    readonly sessionId: string;
    readonly runId?: string | null;
    readonly expectedRevision: number;
  }): Promise<CommandActionResult>;
  answerQuestion(input: {
    readonly sessionId: string;
    readonly questionId: string;
    readonly answer: string;
    readonly expectedRevision: number;
    readonly commandId?: string;
  }): Promise<AnswerQuestionResult>;
}

const KNOWN_RUN_STATUSES: ReadonlySet<string> = new Set([
  "queued",
  "running",
  "waiting_for_approval",
  "cancel_requested",
  "stopped",
  "completed",
  "failed",
  "interrupted",
]);

export function normalizeRunStatus(raw: string | null | undefined): RunStatus {
  if (!raw) {
    return "unknown";
  }
  return KNOWN_RUN_STATUSES.has(raw) ? (raw as RunStatus) : "unknown";
}

export function normalizeSessionStatus(raw: string | null | undefined): SessionStatus {
  if (raw === "created" || raw === "active" || raw === "archived") {
    return raw;
  }
  return "unknown";
}

export function resolveDisplayRunId(session: DaemonSession): string | null {
  return session.active_run_id ?? session.latest_run_id ?? null;
}

export function resolveDisplayWorktree(session: DaemonSession): string {
  return session.worktree_path ?? session.latest_worktree_path ?? session.cwd;
}

export function resolveDisplayRunStatus(
  session: DaemonSession,
  connectionError: boolean,
): RunStatus {
  if (connectionError) {
    return "unknown";
  }
  const raw = session.active_run_status ?? session.latest_run_status;
  if (!raw) {
    if (session.status === "created") {
      return "unknown";
    }
    return "unknown";
  }
  return normalizeRunStatus(raw);
}

export function formatEventTime(timestampMs: number): string {
  if (!Number.isFinite(timestampMs) || timestampMs <= 0) {
    return "--:--:--";
  }
  const date = new Date(timestampMs);
  const hh = String(date.getHours()).padStart(2, "0");
  const mm = String(date.getMinutes()).padStart(2, "0");
  const ss = String(date.getSeconds()).padStart(2, "0");
  return `${hh}:${mm}:${ss}`;
}

export function summarizeEventPayload(event: DaemonEventItem): string {
  const payload = event.payload;
  if (payload === null || payload === undefined) {
    return "";
  }
  if (typeof payload === "string") {
    return payload;
  }
  if (typeof payload !== "object") {
    return String(payload);
  }
  const record = payload as Record<string, unknown>;
  if (typeof record.type === "string" && "data" in record) {
    const data = record.data;
    if (data && typeof data === "object") {
      const dataRec = data as Record<string, unknown>;
      if (typeof dataRec.name === "string") {
        return `${record.type} · ${dataRec.name}`;
      }
      if (typeof dataRec.turn === "number") {
        return `${record.type} · turn ${String(dataRec.turn)}`;
      }
    }
    return record.type;
  }
  const keys = Object.keys(record);
  if (keys.length === 1 && keys[0]) {
    const inner = record[keys[0]];
    if (inner && typeof inner === "object") {
      const innerRec = inner as Record<string, unknown>;
      if (typeof innerRec.prompt === "string") {
        return innerRec.prompt;
      }
      if (typeof innerRec.answer === "string") {
        return `Answer: ${innerRec.answer}`;
      }
      if (typeof innerRec.reason === "string") {
        return innerRec.reason;
      }
      if (typeof innerRec.error === "string") {
        return innerRec.error;
      }
    }
  }
  return "";
}
