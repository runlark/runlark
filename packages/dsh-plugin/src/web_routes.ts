import { randomUUID } from "node:crypto";
import type { IncomingMessage, ServerResponse } from "node:http";
import { brandString } from "@deepseek-ai/dsh-brand";
import { SessionId as brandSessionId } from "@deepseek-ai/dsh-session";
import type { DaemonClient } from "./daemon_client.ts";
import type { OutboxDispatchRequest, SessionRequestId } from "./types.ts";

export interface WebBridgeTarget {
  ensureManagedSession(opts: { sessionId: string; taskId: string; cwd: string }): Promise<unknown>;
  sendManagedPrompt(request: {
    requestId: SessionRequestId;
    sessionId: ReturnType<typeof brandSessionId>;
    mode: "queue" | "steer";
    content: readonly [{ type: "text"; text: string }];
  }): Promise<unknown>;
  stopManagedRun(sessionId: string): Promise<unknown>;
  resumeManagedSession(sessionId: string, opts?: { afterSeq?: number }): Promise<unknown>;
  archiveManagedSession(sessionId: string): Promise<unknown>;
  applyQuestionAnswer(sessionId: string, questionId: string, answer: string): Promise<void> | void;
  handleOutboxDispatch(req: OutboxDispatchRequest): Promise<void>;
}

function sendJson(res: ServerResponse, status: number, body: unknown): void {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
}

async function readJsonBody(req: IncomingMessage): Promise<Record<string, unknown>> {
  const chunks: Buffer[] = [];
  for await (const chunk of req) {
    chunks.push(chunk as Buffer);
  }
  const text = Buffer.concat(chunks).toString("utf8").trim();
  if (!text) return {};
  return JSON.parse(text) as Record<string, unknown>;
}

function parseOptionalInt(value: string | null): number | undefined {
  if (value === null || value === "") return undefined;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? Math.floor(parsed) : undefined;
}

export async function handleRunlarkWebRequest(
  req: IncomingMessage,
  res: ServerResponse,
  bridge: WebBridgeTarget,
  client: DaemonClient,
): Promise<void> {
  const method = req.method ?? "GET";
  const url = new URL(req.url ?? "/api/runlark", "http://127.0.0.1");
  const path = url.pathname;
  const reqAbort = new AbortController();
  res.on("close", () => {
    if (!res.writableEnded) {
      reqAbort.abort();
    }
  });

  try {
    if (method === "GET" && path === "/api/runlark/health") {
      const daemonHealthy = await client.healthCheck();
      sendJson(res, 200, { status: "ok", daemonHealthy });
      return;
    }

    if (method === "POST" && path === "/api/runlark/dispatch") {
      const body = (await readJsonBody(req)) as unknown as OutboxDispatchRequest;
      await bridge.handleOutboxDispatch(body);
      sendJson(res, 200, { ok: true });
      return;
    }

    if (method === "GET" && path === "/api/runlark/sessions") {
      sendJson(res, 200, await client.listSessions());
      return;
    }

    if (method === "POST" && path === "/api/runlark/sessions") {
      const body = await readJsonBody(req);
      const sessionId = String(body.session_id ?? "");
      const taskId = String(body.task_id ?? `task_${sessionId}`);
      const cwd = String(body.cwd ?? process.cwd());
      if (!sessionId) {
        sendJson(res, 400, { error: "invalid_request", message: "session_id is required" });
        return;
      }
      const ensured = await bridge.ensureManagedSession({ sessionId, taskId, cwd });
      sendJson(res, 201, ensured);
      return;
    }

    const sessionMatch =
      /^\/api\/runlark\/sessions\/([^/]+)(?:\/(events|questions|send|stop|resume|archive))?$/u.exec(
        path,
      );
    if (sessionMatch) {
      const sessionId = decodeURIComponent(sessionMatch[1]);
      const action = sessionMatch[2];

      if (method === "GET" && !action) {
        const session = await client.getSession(sessionId);
        if (!session) {
          sendJson(res, 404, {
            error: "session_not_found",
            message: `Session not found: ${sessionId}`,
          });
          return;
        }
        sendJson(res, 200, session);
        return;
      }

      if (method === "GET" && action === "questions") {
        sendJson(res, 200, await client.listSessionQuestions(sessionId));
        return;
      }

      if (method === "GET" && action === "events") {
        const afterSeq = parseOptionalInt(url.searchParams.get("after_seq"));
        const waitMs = parseOptionalInt(url.searchParams.get("wait_ms"));
        sendJson(
          res,
          200,
          await client.getSessionEvents(sessionId, afterSeq, waitMs, reqAbort.signal),
        );
        return;
      }

      if (method === "POST" && action === "send") {
        const body = await readJsonBody(req);
        const prompt = String(body.prompt ?? "").trim();
        if (!prompt) {
          sendJson(res, 400, { error: "invalid_request", message: "prompt is required" });
          return;
        }
        const requestId = brandString<SessionRequestId>(
          typeof body.commandId === "string" && body.commandId
            ? body.commandId
            : `ui_send_${randomUUID()}`,
        );
        const mode = body.mode === "steer" ? "steer" : "queue";
        const result = await bridge.sendManagedPrompt({
          requestId,
          sessionId: brandSessionId(sessionId),
          mode,
          content: [{ type: "text", text: prompt }],
        });
        sendJson(res, 200, result);
        return;
      }

      if (method === "POST" && action === "stop") {
        const result = await bridge.stopManagedRun(sessionId);
        sendJson(res, 200, result);
        return;
      }

      if (method === "POST" && action === "resume") {
        const body = await readJsonBody(req);
        const afterSeq = typeof body.afterSeq === "number" ? body.afterSeq : undefined;
        const result = await bridge.resumeManagedSession(
          sessionId,
          afterSeq !== undefined ? { afterSeq } : {},
        );
        sendJson(res, 200, result);
        return;
      }

      if (method === "POST" && action === "archive") {
        await bridge.archiveManagedSession(sessionId);
        sendJson(res, 200, { ok: true });
        return;
      }
    }

    const runEventsMatch = /^\/api\/runlark\/runs\/([^/]+)\/events$/u.exec(path);
    if (method === "GET" && runEventsMatch) {
      const runId = decodeURIComponent(runEventsMatch[1]);
      const afterSeq = parseOptionalInt(url.searchParams.get("after_seq"));
      const waitMs = parseOptionalInt(url.searchParams.get("wait_ms"));
      sendJson(res, 200, await client.getEvents(runId, afterSeq, waitMs, reqAbort.signal));
      return;
    }

    const answerMatch = /^\/api\/runlark\/questions\/([^/]+)\/answer$/u.exec(path);
    if (method === "POST" && answerMatch) {
      const questionId = decodeURIComponent(answerMatch[1]);
      const body = await readJsonBody(req);
      const answer = String(body.answer ?? "").trim();
      if (!answer) {
        sendJson(res, 400, { error: "invalid_request", message: "answer is required" });
        return;
      }
      const commandId =
        typeof body.command_id === "string" && body.command_id
          ? body.command_id
          : `ui_ans_${questionId}`;
      const expectedRevision =
        typeof body.expected_revision === "number" ? body.expected_revision : undefined;
      const result = await client.answerQuestion(questionId, {
        command_id: commandId,
        answer,
        ...(expectedRevision !== undefined ? { expected_revision: expectedRevision } : {}),
      });
      if (typeof body.session_id === "string" && body.session_id) {
        await bridge.applyQuestionAnswer(body.session_id, questionId, answer);
      }
      sendJson(res, 200, result);
      return;
    }

    sendJson(res, 404, { error: "not_found", message: `Unknown route: ${method} ${path}` });
  } catch (error) {
    if (reqAbort.signal.aborted || res.writableEnded) {
      return;
    }
    sendJson(res, 500, {
      error: "bridge_error",
      message: error instanceof Error ? error.message : String(error),
    });
  }
}
