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

    const projectPromoteMatch = /^\/api\/runlark\/projects\/([^/]+)\/promote-team$/u.exec(path);
    if (method === "POST" && projectPromoteMatch) {
      const body = await readJsonBody(req);
      sendJson(
        res,
        200,
        await boardClient.promoteProjectToTeam(
          decodeURIComponent(projectPromoteMatch[1]),
          body,
          signal,
        ),
      );
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

    const taskCommentMatch = /^\/api\/runlark\/tasks\/([^/]+)\/comments$/u.exec(path);
    if (taskCommentMatch) {
      const taskId = decodeURIComponent(taskCommentMatch[1]);
      if (method === "GET") {
        sendJson(res, 200, await boardClient.listTaskComments(taskId, signal));
        return true;
      }
      if (method === "POST") {
        const body = await readJsonBody(req);
        const { status, data } = await boardClient.addTaskComment(taskId, body, signal);
        sendJson(res, status, data);
        return true;
      }
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

    if (
      await handleTeamWebRequest(
        method,
        path,
        url,
        req,
        res,
        boardClient,
        readJsonBody,
        sendJson,
        signal,
      )
    ) {
      return true;
    }

    return false;
  } catch (error) {
    if (signal.aborted || res.writableEnded) {
      return true;
    }
    if (error instanceof URIError) {
      sendJson(res, 400, {
        error: "invalid_uri",
        message: "Invalid percent-encoded path segment",
      });
      return true;
    }
    if (error instanceof DaemonHttpError) {
      sendJson(res, error.status, error.body ?? { error: "daemon_error", message: error.message });
      return true;
    }
    throw error;
  }
}

async function handleTeamWebRequest(
  method: string,
  path: string,
  url: URL,
  req: IncomingMessage,
  res: ServerResponse,
  boardClient: DaemonBoardClient,
  readJsonBody: ReadJsonFn,
  sendJson: SendJsonFn,
  signal: AbortSignal,
): Promise<boolean> {
  if (!path.startsWith("/api/runlark/team")) {
    return false;
  }
  if (method === "POST" && path === "/api/runlark/team/login") {
    sendJson(res, 200, await boardClient.teamLogin(await readJsonBody(req), signal));
    return true;
  }
  if (method === "GET" && path === "/api/runlark/team/status") {
    sendJson(res, 200, await boardClient.getTeamStatus(signal));
    return true;
  }
  if (method === "POST" && path === "/api/runlark/team/sync") {
    sendJson(res, 200, await boardClient.syncTeam(await readJsonBody(req), signal));
    return true;
  }
  if (method === "POST" && path === "/api/runlark/team/workspaces") {
    sendJson(res, 201, await boardClient.createWorkspace(await readJsonBody(req), signal));
    return true;
  }
  if (method === "POST" && path === "/api/runlark/team/invitations/accept") {
    sendJson(res, 200, await boardClient.acceptInvitation(await readJsonBody(req), signal));
    return true;
  }

  const wsMatch = /^\/api\/runlark\/team\/workspaces\/([^/]+)(?:\/(members|invitations))?$/u.exec(
    path,
  );
  if (wsMatch) {
    const wid = decodeURIComponent(wsMatch[1]);
    const sub = wsMatch[2];
    if (method === "GET" && !sub) {
      sendJson(res, 200, await boardClient.getWorkspace(wid, signal));
      return true;
    }
    if (method === "GET" && sub === "members") {
      sendJson(res, 200, await boardClient.listWorkspaceMembers(wid, signal));
      return true;
    }
    if (method === "POST" && sub === "invitations") {
      sendJson(res, 201, await boardClient.createInvitation(wid, await readJsonBody(req), signal));
      return true;
    }
  }

  const memberPatchMatch = /^\/api\/runlark\/team\/workspaces\/([^/]+)\/members\/([^/]+)$/u.exec(
    path,
  );
  if (method === "PATCH" && memberPatchMatch) {
    sendJson(
      res,
      200,
      await boardClient.updateWorkspaceMemberRole(
        decodeURIComponent(memberPatchMatch[1]),
        decodeURIComponent(memberPatchMatch[2]),
        await readJsonBody(req),
        signal,
      ),
    );
    return true;
  }

  const revokeMatch =
    /^\/api\/runlark\/team\/workspaces\/([^/]+)\/invitations\/([^/]+)\/revoke$/u.exec(path);
  if (method === "POST" && revokeMatch) {
    sendJson(
      res,
      200,
      await boardClient.revokeInvitation(
        decodeURIComponent(revokeMatch[1]),
        decodeURIComponent(revokeMatch[2]),
        signal,
      ),
    );
    return true;
  }

  if (method === "GET" && path === "/api/runlark/team/drafts") {
    const projectId = url.searchParams.get("project_id") ?? undefined;
    sendJson(res, 200, await boardClient.listDrafts(projectId, signal));
    return true;
  }
  if (method === "POST" && path === "/api/runlark/team/drafts") {
    sendJson(res, 201, await boardClient.createDraft(await readJsonBody(req), signal));
    return true;
  }
  const draftRetryMatch = /^\/api\/runlark\/team\/drafts\/([^/]+)\/retry$/u.exec(path);
  if (method === "POST" && draftRetryMatch) {
    sendJson(
      res,
      200,
      await boardClient.retryDraft(
        decodeURIComponent(draftRetryMatch[1]),
        await readJsonBody(req),
        signal,
      ),
    );
    return true;
  }
  const draftDeleteMatch = /^\/api\/runlark\/team\/drafts\/([^/]+)$/u.exec(path);
  if (method === "DELETE" && draftDeleteMatch) {
    const { status } = await boardClient.deleteDraft(
      decodeURIComponent(draftDeleteMatch[1]),
      signal,
    );
    res.writeHead(status);
    res.end();
    return true;
  }

  if (method === "GET" && path === "/api/runlark/team/runners") {
    const wid = url.searchParams.get("workspace_id") ?? undefined;
    sendJson(res, 200, await boardClient.listRunners(wid, signal));
    return true;
  }
  if (method === "POST" && path === "/api/runlark/team/runners/register") {
    sendJson(res, 200, await boardClient.registerRunner(await readJsonBody(req), signal));
    return true;
  }
  if (method === "GET" && path === "/api/runlark/team/runners/status") {
    const wid = url.searchParams.get("workspace_id") ?? undefined;
    sendJson(res, 200, await boardClient.getRunnerStatus(wid, signal));
    return true;
  }
  const runActionMatch = /^\/api\/runlark\/team\/runs\/([^/]+)\/(assign|cancel-unreachable)$/u.exec(
    path,
  );
  if (method === "POST" && runActionMatch) {
    const runId = decodeURIComponent(runActionMatch[1]);
    const body = await readJsonBody(req);
    const result =
      runActionMatch[2] === "assign"
        ? await boardClient.assignRunner(runId, body, signal)
        : await boardClient.cancelUnreachableRun(runId, body, signal);
    sendJson(res, 200, result);
    return true;
  }
  return false;
}
