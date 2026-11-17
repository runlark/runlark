import type { IncomingMessage, ServerResponse } from "node:http";
import { DaemonBoardClient, DaemonHttpError } from "./daemon_board_client.ts";
import type { ChatDispatchPayload } from "./daemon_board_client.ts";

type SendJsonFn = (res: ServerResponse, status: number, body: unknown) => void;
type ReadJsonFn = (req: IncomingMessage) => Promise<Record<string, unknown>>;

export async function handleRunlarkBoardWebRequest(
  req: IncomingMessage,
  res: ServerResponse,
  boardClient: DaemonBoardClient,
  readJsonBody: ReadJsonFn,
  sendJson: SendJsonFn,
  signal: AbortSignal,
): Promise<boolean> {
  const method = req.method ?? "GET";
  const url = new URL(req.url ?? "/api/runlark", "http://127.0.0.1");
  const path = url.pathname;

  try {
    if (method === "GET" && path === "/api/runlark/board") {
      const projectId = url.searchParams.get("project_id") ?? undefined;
      sendJson(res, 200, await boardClient.getBoard(projectId, signal));
      return true;
    }

    if (method === "GET" && path === "/api/runlark/scheduler/status") {
      sendJson(res, 200, await boardClient.getSchedulerStatus(signal));
      return true;
    }

    if (method === "GET" && path === "/api/runlark/projects") {
      sendJson(res, 200, await boardClient.listProjects(signal));
      return true;
    }

    if (method === "POST" && path === "/api/runlark/projects") {
      const body = await readJsonBody(req);
      sendJson(res, 201, await boardClient.createProject(body, signal));
      return true;
    }

    const projectMatch = /^\/api\/runlark\/projects\/([^/]+)$/u.exec(path);
    if (method === "GET" && projectMatch) {
      sendJson(res, 200, await boardClient.getProject(decodeURIComponent(projectMatch[1]), signal));
      return true;
    }

    if (method === "GET" && path === "/api/runlark/tasks") {
      const projectId = url.searchParams.get("project_id") ?? undefined;
      const stage = url.searchParams.get("stage") ?? undefined;
      sendJson(res, 200, await boardClient.listTasks(projectId, stage, signal));
      return true;
    }

    if (method === "POST" && path === "/api/runlark/tasks") {
      const body = await readJsonBody(req);
      sendJson(res, 201, await boardClient.createTask(body, signal));
      return true;
    }

    const taskMatch =
      /^\/api\/runlark\/tasks\/([^/]+)(?:\/(move|clarify|plan|start|accept|reject|pr))?$/u.exec(
        path,
      );
    if (taskMatch) {
      const taskId = decodeURIComponent(taskMatch[1]);
      const action = taskMatch[2] as
        | "move"
        | "clarify"
        | "plan"
        | "start"
        | "accept"
        | "reject"
        | "pr"
        | undefined;
      if (method === "GET" && !action) {
        sendJson(res, 200, await boardClient.getTask(taskId, signal));
        return true;
      }
      if (method === "POST" && action) {
        const body = await readJsonBody(req);
        const { status, data } = await boardClient.taskActionWithStatus(
          taskId,
          action,
          body,
          signal,
        );
        sendJson(res, status, data);
        return true;
      }
    }

    if (method === "POST" && path === "/api/runlark/chat") {
      const body = await readJsonBody(req);
      const result = await boardClient.dispatchChat(body as unknown as ChatDispatchPayload, signal);
      sendJson(res, 200, result);
      return true;
    }

    const runActionMatch = /^\/api\/runlark\/runs\/([^/]+)\/(diff|history|verify)$/u.exec(path);
    if (runActionMatch) {
      const runId = decodeURIComponent(runActionMatch[1]);
      const action = runActionMatch[2];
      if (method === "GET" && action === "diff") {
        sendJson(res, 200, await boardClient.getRunDiff(runId, signal));
        return true;
      }
      if (method === "GET" && action === "history") {
        sendJson(res, 200, await boardClient.getRunHistory(runId, signal));
        return true;
      }
      if (method === "POST" && action === "verify") {
        const body = await readJsonBody(req);
        sendJson(res, 200, await boardClient.verifyRun(runId, body, signal));
        return true;
      }
    }

    if (method === "GET" && path === "/api/runlark/plugins") {
      sendJson(res, 200, await boardClient.listPlugins(signal));
      return true;
    }

    if (method === "POST" && path === "/api/runlark/plugins") {
      const body = await readJsonBody(req);
      sendJson(res, 201, await boardClient.installPlugin(body, signal));
      return true;
    }

    if (method === "POST" && path === "/api/runlark/plugins/validate") {
      const body = await readJsonBody(req);
      sendJson(res, 200, await boardClient.validatePlugin(body, signal));
      return true;
    }

    return false;
  } catch (error) {
    if (signal.aborted || res.writableEnded) {
      return true;
    }
    if (error instanceof DaemonHttpError) {
      sendJson(res, error.status, error.body ?? { error: "daemon_error", message: error.message });
      return true;
    }
    throw error;
  }
}
