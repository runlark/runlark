export class DaemonHttpError extends Error {
  readonly status: number;
  readonly body: unknown;

  constructor(status: number, body: unknown, message: string) {
    super(message);
    this.name = "DaemonHttpError";
    this.status = status;
    this.body = body;
  }
}

export interface ChatDispatchPayload {
  input: string;
  project_id?: string;
  session_id?: string;
  run_id?: string;
  actor?: string;
  command_id?: string;
}

export interface ChatDispatchResult {
  command: string;
  summary: string;
  data: unknown;
}

export interface RecordAttemptPayload {
  command_id?: string;
  attempt_id?: string;
  workflow_stage?: string;
  outcome?: { status: string; error?: string; reason?: string };
  cost_micro_usd: number | null;
  unknown_cost_reason?: string | null;
  repair_reason?: string | null;
  expected_revision?: number;
}

export class DaemonBoardClient {
  private readonly baseUrl: string;

  constructor(baseUrl: string) {
    this.baseUrl = baseUrl.replace(/\/+$/u, "");
  }

  private async requestWithStatus<T>(
    method: string,
    path: string,
    body?: unknown,
    signal?: AbortSignal,
  ): Promise<{ readonly status: number; readonly data: T }> {
    const res = await fetch(`${this.baseUrl}${path}`, {
      method,
      headers: body !== undefined ? { "Content-Type": "application/json" } : undefined,
      body: body !== undefined ? JSON.stringify(body) : undefined,
      ...(signal ? { signal } : {}),
    });
    const text = await res.text();
    let parsed: unknown = null;
    if (text.trim().length > 0) {
      try {
        parsed = JSON.parse(text);
      } catch {
        parsed = text;
      }
    }
    if (!res.ok) {
      const msg =
        typeof parsed === "object" &&
        parsed !== null &&
        "message" in parsed &&
        typeof (parsed as { message?: unknown }).message === "string"
          ? (parsed as { message: string }).message
          : `Daemon request ${method} ${path} failed (${res.status})`;
      throw new DaemonHttpError(res.status, parsed, msg);
    }
    return { status: res.status, data: parsed as T };
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
    signal?: AbortSignal,
  ): Promise<T> {
    const { data } = await this.requestWithStatus<T>(method, path, body, signal);
    return data;
  }

  getBoard(projectId?: string, signal?: AbortSignal): Promise<unknown> {
    const q = projectId ? `?project_id=${encodeURIComponent(projectId)}` : "";
    return this.request("GET", `/api/v1/board${q}`, undefined, signal);
  }

  getSchedulerStatus(signal?: AbortSignal): Promise<unknown> {
    return this.request("GET", "/api/v1/scheduler/status", undefined, signal);
  }

  listProjects(signal?: AbortSignal): Promise<unknown> {
    return this.request("GET", "/api/v1/projects", undefined, signal);
  }

  createProject(body: Record<string, unknown>, signal?: AbortSignal): Promise<unknown> {
    return this.request("POST", "/api/v1/projects", body, signal);
  }

  getProject(id: string, signal?: AbortSignal): Promise<unknown> {
    return this.request("GET", `/api/v1/projects/${encodeURIComponent(id)}`, undefined, signal);
  }

  listTasks(projectId?: string, stage?: string, signal?: AbortSignal): Promise<unknown> {
    const params = new URLSearchParams();
    if (projectId) params.set("project_id", projectId);
    if (stage) params.set("stage", stage);
    const q = params.toString() ? `?${params.toString()}` : "";
    return this.request("GET", `/api/v1/tasks${q}`, undefined, signal);
  }

  createTask(body: Record<string, unknown>, signal?: AbortSignal): Promise<unknown> {
    return this.request("POST", "/api/v1/tasks", body, signal);
  }

  getTask(id: string, signal?: AbortSignal): Promise<unknown> {
    return this.request("GET", `/api/v1/tasks/${encodeURIComponent(id)}`, undefined, signal);
  }

  taskActionWithStatus(
    id: string,
    action: "move" | "clarify" | "plan" | "start" | "accept" | "reject" | "pr",
    body: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<{ readonly status: number; readonly data: unknown }> {
    return this.requestWithStatus(
      "POST",
      `/api/v1/tasks/${encodeURIComponent(id)}/${action}`,
      body,
      signal,
    );
  }

  taskAction(
    id: string,
    action: "move" | "clarify" | "plan" | "start" | "accept" | "reject" | "pr",
    body: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<unknown> {
    return this.request("POST", `/api/v1/tasks/${encodeURIComponent(id)}/${action}`, body, signal);
  }

  getRunHistory(runId: string, signal?: AbortSignal): Promise<unknown> {
    return this.request(
      "GET",
      `/api/v1/runs/${encodeURIComponent(runId)}/history`,
      undefined,
      signal,
    );
  }

  getRunDiff(runId: string, signal?: AbortSignal): Promise<unknown> {
    return this.request("GET", `/api/v1/runs/${encodeURIComponent(runId)}/diff`, undefined, signal);
  }

  async getArtifact(
    digest: string,
    meta: boolean,
    signal?: AbortSignal,
  ): Promise<{ readonly contentType: string; readonly body: Buffer }> {
    const q = meta ? "?meta=true" : "";
    const path = `/api/v1/artifacts/${encodeURIComponent(digest)}${q}`;
    const res = await fetch(`${this.baseUrl}${path}`, {
      method: "GET",
      ...(signal ? { signal } : {}),
    });
    const buf = Buffer.from(await res.arrayBuffer());
    if (!res.ok) {
      let parsed: unknown = null;
      const text = buf.toString("utf8");
      if (text.trim().length > 0) {
        try {
          parsed = JSON.parse(text);
        } catch {
          parsed = text;
        }
      }
      const msg =
        typeof parsed === "object" &&
        parsed !== null &&
        "message" in parsed &&
        typeof (parsed as { message?: unknown }).message === "string"
          ? (parsed as { message: string }).message
          : `Daemon request GET ${path} failed (${res.status})`;
      throw new DaemonHttpError(res.status, parsed, msg);
    }
    return {
      contentType: res.headers.get("content-type") ?? "application/octet-stream",
      body: buf,
    };
  }

  verifyRun(runId: string, body: Record<string, unknown>, signal?: AbortSignal): Promise<unknown> {
    return this.request("POST", `/api/v1/runs/${encodeURIComponent(runId)}/verify`, body, signal);
  }

  recordRunAttempt(
    runId: string,
    body: RecordAttemptPayload,
    signal?: AbortSignal,
  ): Promise<unknown> {
    return this.request("POST", `/api/v1/runs/${encodeURIComponent(runId)}/attempts`, body, signal);
  }

  dispatchChat(body: ChatDispatchPayload, signal?: AbortSignal): Promise<ChatDispatchResult> {
    return this.request("POST", "/api/v1/chat/dispatch", body, signal);
  }

  listPlugins(signal?: AbortSignal): Promise<unknown> {
    return this.request("GET", "/api/v1/plugins", undefined, signal);
  }

  installPlugin(body: Record<string, unknown>, signal?: AbortSignal): Promise<unknown> {
    return this.request("POST", "/api/v1/plugins", body, signal);
  }

  validatePlugin(body: Record<string, unknown>, signal?: AbortSignal): Promise<unknown> {
    return this.request("POST", "/api/v1/plugins/validate", body, signal);
  }

  teamLogin(body: Record<string, unknown>, signal?: AbortSignal): Promise<unknown> {
    return this.request("POST", "/api/v1/team/login", body, signal);
  }

  getTeamStatus(signal?: AbortSignal): Promise<unknown> {
    return this.request("GET", "/api/v1/team/status", undefined, signal);
  }

  syncTeam(body?: Record<string, unknown>, signal?: AbortSignal): Promise<unknown> {
    return this.request("POST", "/api/v1/team/sync", body ?? {}, signal);
  }

  createWorkspace(body: Record<string, unknown>, signal?: AbortSignal): Promise<unknown> {
    return this.request("POST", "/api/v1/team/workspaces", body, signal);
  }

  getWorkspace(wid: string, signal?: AbortSignal): Promise<unknown> {
    return this.request(
      "GET",
      `/api/v1/team/workspaces/${encodeURIComponent(wid)}`,
      undefined,
      signal,
    );
  }

  listWorkspaceMembers(wid: string, signal?: AbortSignal): Promise<unknown> {
    return this.request(
      "GET",
      `/api/v1/team/workspaces/${encodeURIComponent(wid)}/members`,
      undefined,
      signal,
    );
  }

  updateWorkspaceMemberRole(
    wid: string,
    uid: string,
    body: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<unknown> {
    return this.request(
      "PATCH",
      `/api/v1/team/workspaces/${encodeURIComponent(wid)}/members/${encodeURIComponent(uid)}`,
      body,
      signal,
    );
  }

  listInvitations(wid: string, signal?: AbortSignal): Promise<unknown> {
    return this.request(
      "GET",
      `/api/v1/team/workspaces/${encodeURIComponent(wid)}/invitations`,
      undefined,
      signal,
    );
  }

  createInvitation(
    wid: string,
    body: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<unknown> {
    return this.request(
      "POST",
      `/api/v1/team/workspaces/${encodeURIComponent(wid)}/invitations`,
      body,
      signal,
    );
  }

  revokeInvitation(wid: string, inviteId: string, signal?: AbortSignal): Promise<unknown> {
    return this.request(
      "POST",
      `/api/v1/team/workspaces/${encodeURIComponent(wid)}/invitations/${encodeURIComponent(inviteId)}/revoke`,
      {},
      signal,
    );
  }

  acceptInvitation(body: Record<string, unknown>, signal?: AbortSignal): Promise<unknown> {
    return this.request("POST", "/api/v1/team/invitations/accept", body, signal);
  }

  promoteProjectToTeam(
    projectId: string,
    body: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<unknown> {
    return this.request(
      "POST",
      `/api/v1/projects/${encodeURIComponent(projectId)}/promote-team`,
      body,
      signal,
    );
  }

  listDrafts(projectId?: string, signal?: AbortSignal): Promise<unknown> {
    const q = projectId ? `?project_id=${encodeURIComponent(projectId)}` : "";
    return this.request("GET", `/api/v1/team/drafts${q}`, undefined, signal);
  }

  createDraft(body: Record<string, unknown>, signal?: AbortSignal): Promise<unknown> {
    return this.request("POST", "/api/v1/team/drafts", body, signal);
  }

  retryDraft(
    draftId: string,
    body?: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<unknown> {
    return this.request(
      "POST",
      `/api/v1/team/drafts/${encodeURIComponent(draftId)}/retry`,
      body ?? {},
      signal,
    );
  }

  deleteDraft(draftId: string, signal?: AbortSignal): Promise<{ readonly status: number }> {
    return this.requestWithStatus(
      "DELETE",
      `/api/v1/team/drafts/${encodeURIComponent(draftId)}`,
      undefined,
      signal,
    );
  }

  listTaskComments(taskId: string, signal?: AbortSignal): Promise<unknown> {
    return this.request(
      "GET",
      `/api/v1/tasks/${encodeURIComponent(taskId)}/comments`,
      undefined,
      signal,
    );
  }

  addTaskComment(
    taskId: string,
    body: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<{ readonly status: number; readonly data: unknown }> {
    return this.requestWithStatus(
      "POST",
      `/api/v1/tasks/${encodeURIComponent(taskId)}/comments`,
      body,
      signal,
    );
  }

  listRunners(workspaceId?: string, signal?: AbortSignal): Promise<unknown> {
    const q = workspaceId ? `?workspace_id=${encodeURIComponent(workspaceId)}` : "";
    return this.request("GET", `/api/v1/team/runners${q}`, undefined, signal);
  }

  registerRunner(body: Record<string, unknown>, signal?: AbortSignal): Promise<unknown> {
    return this.request("POST", "/api/v1/team/runners/register", body, signal);
  }

  getRunnerStatus(workspaceId?: string, signal?: AbortSignal): Promise<unknown> {
    const q = workspaceId ? `?workspace_id=${encodeURIComponent(workspaceId)}` : "";
    return this.request("GET", `/api/v1/team/runners/status${q}`, undefined, signal);
  }

  assignRunner(
    runId: string,
    body: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<unknown> {
    return this.request(
      "POST",
      `/api/v1/team/runs/${encodeURIComponent(runId)}/assign`,
      body,
      signal,
    );
  }

  cancelUnreachableRun(
    runId: string,
    body: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<unknown> {
    return this.request(
      "POST",
      `/api/v1/team/runs/${encodeURIComponent(runId)}/cancel-unreachable`,
      body,
      signal,
    );
  }
}
