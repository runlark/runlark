import type {
  AnswerQuestionRequest,
  AnswerQuestionResponse,
  CreateSessionRequest,
  DaemonEventItem,
  DaemonQuestion,
  DaemonSession,
  DshWireEventEnvelope,
  RuntimeEvent,
  SessionCommand,
  SubmitCommandResult,
} from "./types.ts";

interface ConflictBody {
  error?: string;
  message?: string;
  actual_revision?: number;
}

export class DaemonClient {
  private readonly baseUrl: string;

  constructor(baseUrl: string) {
    this.baseUrl = baseUrl.replace(/\/+$/u, "");
  }

  getBaseUrl(): string {
    return this.baseUrl;
  }

  async healthCheck(): Promise<boolean> {
    try {
      const res = await fetch(`${this.baseUrl}/health`);
      if (!res.ok) return false;
      const data = (await res.json()) as { status?: string };
      return data.status === "ok" || data.status === "healthy";
    } catch {
      return false;
    }
  }

  async listSessions(): Promise<DaemonSession[]> {
    const res = await fetch(`${this.baseUrl}/api/v1/sessions`);
    if (!res.ok) {
      throw new Error(`Failed to list sessions (${res.status}): ${await res.text()}`);
    }
    return (await res.json()) as DaemonSession[];
  }

  async getSession(sessionId: string): Promise<DaemonSession | null> {
    const res = await fetch(`${this.baseUrl}/api/v1/sessions/${encodeURIComponent(sessionId)}`);
    if (res.status === 404) {
      return null;
    }
    if (!res.ok) {
      throw new Error(`Failed to fetch session (${res.status}): ${await res.text()}`);
    }
    return (await res.json()) as DaemonSession;
  }

  async createSession(request: CreateSessionRequest): Promise<DaemonSession> {
    const res = await fetch(`${this.baseUrl}/api/v1/sessions`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(request),
    });
    if (!res.ok) {
      throw new Error(`Failed to create session (${res.status}): ${await res.text()}`);
    }
    return (await res.json()) as DaemonSession;
  }

  async submitCommand(command: SessionCommand): Promise<SubmitCommandResult> {
    try {
      const res = await fetch(`${this.baseUrl}/api/v1/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(command),
      });

      if (res.status === 409) {
        const text = await res.text();
        let parsed: ConflictBody | undefined;
        try {
          parsed = JSON.parse(text) as ConflictBody;
        } catch {
          parsed = undefined;
        }
        return {
          ok: false,
          conflict: true,
          error_code: parsed?.error,
          actual_revision:
            typeof parsed?.actual_revision === "number" ? parsed.actual_revision : undefined,
          error: parsed?.message ?? (text || "Conflict: command rejected (409)"),
        };
      }

      if (!res.ok) {
        const text = await res.text();
        return { ok: false, error: `Daemon error (${res.status}): ${text}` };
      }

      let body: {
        command_id: string;
        status: string;
        current_revision?: number;
        run_id?: string;
        run_status?: string;
      };
      try {
        body = (await res.json()) as {
          command_id: string;
          status: string;
          current_revision?: number;
          run_id?: string;
          run_status?: string;
        };
      } catch {
        return { ok: false, error: `Invalid response body (${res.status})` };
      }
      return {
        ok: true,
        command_id: body.command_id,
        status: body.status,
        current_revision: body.current_revision,
        run_id: body.run_id,
        run_status: body.run_status,
      };
    } catch (err) {
      return { ok: false, error: `Network error: ${String(err)}` };
    }
  }

  async postDshEvent(event: RuntimeEvent | DshWireEventEnvelope): Promise<void> {
    const res = await fetch(`${this.baseUrl}/api/v1/dsh/events`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(event),
    });
    if (!res.ok) {
      throw new Error(`Failed to forward DSH event (${res.status}): ${await res.text()}`);
    }
  }

  async listSessionQuestions(sessionId: string): Promise<DaemonQuestion[]> {
    const res = await fetch(
      `${this.baseUrl}/api/v1/sessions/${encodeURIComponent(sessionId)}/questions`,
    );
    if (!res.ok) {
      throw new Error(`Failed to list session questions (${res.status}): ${await res.text()}`);
    }
    return (await res.json()) as DaemonQuestion[];
  }

  async answerQuestion(
    questionId: string,
    request: AnswerQuestionRequest,
  ): Promise<AnswerQuestionResponse> {
    const res = await fetch(
      `${this.baseUrl}/api/v1/questions/${encodeURIComponent(questionId)}/answer`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(request),
      },
    );
    if (!res.ok) {
      throw new Error(`Failed to answer question (${res.status}): ${await res.text()}`);
    }
    return (await res.json()) as AnswerQuestionResponse;
  }

  async getEvents(
    runId: string,
    afterSeq?: number,
    waitMs?: number,
    signal?: AbortSignal,
  ): Promise<DaemonEventItem[]> {
    const params = new URLSearchParams();
    if (afterSeq !== undefined) params.set("after_seq", String(afterSeq));
    if (waitMs !== undefined && waitMs > 0) params.set("wait_ms", String(waitMs));
    const query = params.toString() ? `?${params.toString()}` : "";
    const res = await fetch(
      `${this.baseUrl}/api/v1/runs/${encodeURIComponent(runId)}/events${query}`,
      signal ? { signal } : undefined,
    );
    if (!res.ok) {
      throw new Error(`Failed to fetch events (${res.status}): ${await res.text()}`);
    }
    return (await res.json()) as DaemonEventItem[];
  }

  async getSessionEvents(
    sessionId: string,
    afterSeq?: number,
    waitMs?: number,
    signal?: AbortSignal,
  ): Promise<DaemonEventItem[]> {
    const params = new URLSearchParams();
    if (afterSeq !== undefined) params.set("after_seq", String(afterSeq));
    if (waitMs !== undefined && waitMs > 0) params.set("wait_ms", String(waitMs));
    const query = params.toString() ? `?${params.toString()}` : "";
    const res = await fetch(
      `${this.baseUrl}/api/v1/sessions/${encodeURIComponent(sessionId)}/events${query}`,
      signal ? { signal } : undefined,
    );
    if (res.status === 404) {
      return [];
    }
    if (!res.ok) {
      throw new Error(`Failed to fetch session events (${res.status}): ${await res.text()}`);
    }
    return (await res.json()) as DaemonEventItem[];
  }
}
