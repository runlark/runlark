import type {
  AnswerQuestionResult,
  CommandActionResult,
  DaemonEventItem,
  DaemonQuestion,
  DaemonSession,
  RunlarkApiClient,
} from "./runlark-types";

export interface HttpClientOptions {
  readonly baseUrl?: string;
  readonly mode?: "dsh-proxy" | "daemon";
  readonly fetchImpl?: typeof fetch;
}

function createId(prefix: string): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return `${prefix}_${crypto.randomUUID()}`;
  }
  return `${prefix}_${Date.now()}_${Math.random().toString(36).slice(2, 10)}`;
}

async function readErrorMessage(res: Response): Promise<string> {
  const text = await res.text();
  if (!text) {
    return `HTTP ${res.status}`;
  }
  try {
    const parsed = JSON.parse(text) as { message?: string; error?: string };
    return parsed.message ?? parsed.error ?? text;
  } catch {
    return text;
  }
}

export function createRunlarkApiClient(options: HttpClientOptions = {}): RunlarkApiClient {
  const mode = options.mode ?? "dsh-proxy";
  const baseUrl = (options.baseUrl ?? "/api/runlark").replace(/\/+$/u, "");
  const fetchFn = options.fetchImpl ?? globalThis.fetch.bind(globalThis);
  const apiPrefix = mode === "daemon" ? `${baseUrl}/api/v1` : baseUrl;

  return {
    async getSession(sessionId, signal) {
      const res = await fetchFn(`${apiPrefix}/sessions/${encodeURIComponent(sessionId)}`, {
        ...(signal ? { signal } : {}),
      });
      if (res.status === 404) {
        return null;
      }
      if (!res.ok) {
        throw new Error(await readErrorMessage(res));
      }
      return (await res.json()) as DaemonSession;
    },

    async ensureSession({ sessionId, taskId, cwd }) {
      const res = await fetchFn(`${apiPrefix}/sessions`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          session_id: sessionId,
          task_id: taskId,
          cwd,
        }),
      });
      if (!res.ok) {
        throw new Error(await readErrorMessage(res));
      }
      return (await res.json()) as DaemonSession;
    },

    async listSessionQuestions(sessionId, signal) {
      const res = await fetchFn(
        `${apiPrefix}/sessions/${encodeURIComponent(sessionId)}/questions`,
        {
          ...(signal ? { signal } : {}),
        },
      );
      if (res.status === 404) {
        return [];
      }
      if (!res.ok) {
        throw new Error(await readErrorMessage(res));
      }
      return (await res.json()) as DaemonQuestion[];
    },

    async getSessionEvents(sessionId, opts = {}) {
      const params = new URLSearchParams();
      if (opts.afterSeq !== undefined) {
        params.set("after_seq", String(opts.afterSeq));
      }
      if (opts.waitMs !== undefined && opts.waitMs > 0) {
        params.set("wait_ms", String(opts.waitMs));
      }
      const query = params.toString();
      const targetPath = opts.runId
        ? `${apiPrefix}/runs/${encodeURIComponent(opts.runId)}/events`
        : `${apiPrefix}/sessions/${encodeURIComponent(sessionId)}/events`;
      const url = query ? `${targetPath}?${query}` : targetPath;
      const res = await fetchFn(url, {
        ...(opts.signal ? { signal: opts.signal } : {}),
      });
      if (res.status === 404) {
        return [];
      }
      if (!res.ok) {
        throw new Error(await readErrorMessage(res));
      }
      return (await res.json()) as DaemonEventItem[];
    },

    async sendPrompt({ sessionId, prompt, expectedRevision, activeRunId }) {
      if (mode === "dsh-proxy") {
        const res = await fetchFn(`${apiPrefix}/sessions/${encodeURIComponent(sessionId)}/send`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ prompt, commandId: createId("ui_send") }),
        });
        if (!res.ok) {
          throw new Error(await readErrorMessage(res));
        }
        return (await res.json()) as CommandActionResult;
      }

      const commandId = createId("ui_send");
      const runId = activeRunId ?? `run_${sessionId}_${commandId}`;
      const res = await fetchFn(`${apiPrefix}/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          type: "SendPrompt",
          payload: {
            command_id: commandId,
            session_id: sessionId,
            run_id: runId,
            prompt,
            expected_revision: expectedRevision,
            created_at: Date.now(),
          },
        }),
      });
      if (!res.ok) {
        throw new Error(await readErrorMessage(res));
      }
      return (await res.json()) as CommandActionResult;
    },

    async stopRun({ sessionId, runId, expectedRevision }) {
      if (mode === "dsh-proxy") {
        const res = await fetchFn(`${apiPrefix}/sessions/${encodeURIComponent(sessionId)}/stop`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({}),
        });
        if (!res.ok) {
          throw new Error(await readErrorMessage(res));
        }
        return (await res.json()) as CommandActionResult;
      }

      const res = await fetchFn(`${apiPrefix}/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          type: "StopRun",
          payload: {
            command_id: createId("ui_stop"),
            session_id: sessionId,
            run_id: runId,
            reason: "Requested from Runlark UI",
            expected_revision: expectedRevision,
            created_at: Date.now(),
          },
        }),
      });
      if (!res.ok) {
        throw new Error(await readErrorMessage(res));
      }
      return (await res.json()) as CommandActionResult;
    },

    async resumeSession({ sessionId, runId, expectedRevision, afterSeq }) {
      if (mode === "dsh-proxy") {
        const res = await fetchFn(`${apiPrefix}/sessions/${encodeURIComponent(sessionId)}/resume`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            ...(afterSeq !== undefined ? { afterSeq } : {}),
          }),
        });
        if (!res.ok) {
          throw new Error(await readErrorMessage(res));
        }
        return (await res.json()) as CommandActionResult;
      }

      const res = await fetchFn(`${apiPrefix}/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          type: "ResumeSession",
          payload: {
            command_id: createId("ui_resume"),
            session_id: sessionId,
            run_id: runId,
            ...(afterSeq !== undefined ? { after_seq: afterSeq } : {}),
            expected_revision: expectedRevision,
            created_at: Date.now(),
          },
        }),
      });
      if (!res.ok) {
        throw new Error(await readErrorMessage(res));
      }
      return (await res.json()) as CommandActionResult;
    },

    async archiveSession({ sessionId, runId, expectedRevision }) {
      if (mode === "dsh-proxy") {
        const res = await fetchFn(
          `${apiPrefix}/sessions/${encodeURIComponent(sessionId)}/archive`,
          {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({}),
          },
        );
        if (!res.ok) {
          throw new Error(await readErrorMessage(res));
        }
        return (await res.json()) as CommandActionResult;
      }

      const res = await fetchFn(`${apiPrefix}/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          type: "ArchiveSession",
          payload: {
            command_id: createId("ui_archive"),
            session_id: sessionId,
            run_id: runId ?? `run_${sessionId}`,
            expected_revision: expectedRevision,
            created_at: Date.now(),
          },
        }),
      });
      if (!res.ok) {
        throw new Error(await readErrorMessage(res));
      }
      return (await res.json()) as CommandActionResult;
    },

    async answerQuestion({ sessionId, questionId, answer, expectedRevision, commandId }) {
      const res = await fetchFn(`${apiPrefix}/questions/${encodeURIComponent(questionId)}/answer`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          command_id: commandId ?? `ui_ans_${questionId}`,
          answer,
          expected_revision: expectedRevision,
          session_id: sessionId,
        }),
      });
      if (!res.ok) {
        throw new Error(await readErrorMessage(res));
      }
      return (await res.json()) as AnswerQuestionResult;
    },
  };
}
