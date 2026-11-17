import type {
  BoardApiClient,
  BoardResponse,
  BoardTaskEntry,
  ChatDispatchResult,
  CreatePrOutcome,
  InstalledPlugin,
  ProjectSummary,
  RunDiffResponse,
  RunHistoryResponse,
  RunRecord,
  TaskKind,
  TaskRecord,
  TaskStage,
  WorkflowStage,
} from "./board-types";
import { createRunlarkApiClient, type HttpClientOptions } from "./runlark-http";

async function readApiError(res: Response): Promise<string> {
  const text = await res.text();
  if (!text) return `HTTP ${String(res.status)}`;
  try {
    const parsed = JSON.parse(text) as { message?: string; error?: string };
    return parsed.message ?? parsed.error ?? text;
  } catch {
    return text;
  }
}

export function createBoardApiClient(options: HttpClientOptions = {}): BoardApiClient {
  const mode = options.mode ?? "dsh-proxy";
  const baseUrl = (options.baseUrl ?? "/api/runlark").replace(/\/+$/u, "");
  const fetchFn = options.fetchImpl ?? globalThis.fetch.bind(globalThis);
  const apiPrefix = mode === "daemon" ? `${baseUrl}/api/v1` : baseUrl;
  const sessionClient = createRunlarkApiClient(options);

  async function reqJson<T>(path: string, init?: RequestInit): Promise<T> {
    const res = await fetchFn(`${apiPrefix}${path}`, init);
    if (!res.ok) {
      throw new Error(await readApiError(res));
    }
    return (await res.json()) as T;
  }

  function postJson<T>(path: string, body: unknown): Promise<T> {
    return reqJson<T>(path, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  }

  return {
    getBoard(projectId, signal) {
      const q = projectId ? `?project_id=${encodeURIComponent(projectId)}` : "";
      return reqJson<BoardResponse>(`/board${q}`, signal ? { signal } : undefined);
    },

    listProjects(signal) {
      return reqJson<readonly ProjectSummary[]>("/projects", signal ? { signal } : undefined);
    },

    createProject(input) {
      return postJson<ProjectSummary>("/projects", input);
    },

    getTask(taskId, signal) {
      return reqJson<BoardTaskEntry>(
        `/tasks/${encodeURIComponent(taskId)}`,
        signal ? { signal } : undefined,
      );
    },

    createTask(input: {
      readonly project_id?: string;
      readonly title: string;
      readonly description: string;
      readonly kind: TaskKind;
    }) {
      return postJson<TaskRecord>("/tasks", input);
    },

    moveTask(taskId, stage: TaskStage, expectedRevision: number) {
      return postJson<TaskRecord>(`/tasks/${encodeURIComponent(taskId)}/move`, {
        stage,
        actor: "human",
        expected_revision: expectedRevision,
      });
    },

    clarifyTask(taskId, clarification, advanceToPlan = true) {
      return postJson<{ readonly task: TaskRecord; readonly workflow_stage: WorkflowStage }>(
        `/tasks/${encodeURIComponent(taskId)}/clarify`,
        { clarification, advance_to_plan: advanceToPlan },
      );
    },

    planTask(taskId, input) {
      return postJson<TaskRecord>(`/tasks/${encodeURIComponent(taskId)}/plan`, input);
    },

    startTask(taskId, input = {}) {
      return postJson<{
        readonly task: TaskRecord;
        readonly run: RunRecord;
        readonly admitted: boolean;
      }>(`/tasks/${encodeURIComponent(taskId)}/start`, input);
    },

    acceptTask(taskId, input = {}) {
      return postJson<{
        readonly decision_id: string;
        readonly task: TaskRecord;
        readonly run: RunRecord;
      }>(`/tasks/${encodeURIComponent(taskId)}/accept`, input);
    },

    rejectTask(taskId, input) {
      return postJson<{
        readonly decision_id: string;
        readonly task: TaskRecord;
        readonly run: RunRecord;
      }>(`/tasks/${encodeURIComponent(taskId)}/reject`, input);
    },

    createTaskPr(taskId, input) {
      return postJson<CreatePrOutcome>(`/tasks/${encodeURIComponent(taskId)}/pr`, input);
    },

    getRunHistory(runId, signal) {
      return reqJson<RunHistoryResponse>(
        `/runs/${encodeURIComponent(runId)}/history`,
        signal ? { signal } : undefined,
      );
    },

    getRunDiff(runId, signal) {
      return reqJson<RunDiffResponse>(
        `/runs/${encodeURIComponent(runId)}/diff`,
        signal ? { signal } : undefined,
      );
    },

    verifyRun(runId, taskId) {
      return postJson<unknown>(`/runs/${encodeURIComponent(runId)}/verify`, { task_id: taskId });
    },

    stopRun(input) {
      return sessionClient.stopRun(input);
    },

    resumeSession(input) {
      return sessionClient.resumeSession(input);
    },

    answerQuestion(input) {
      return sessionClient.answerQuestion(input);
    },

    dispatchChat(input) {
      const path = mode === "daemon" ? "/chat/dispatch" : "/chat";
      return postJson<ChatDispatchResult>(path, input);
    },

    listPlugins(signal) {
      return reqJson<readonly InstalledPlugin[]>("/plugins", signal ? { signal } : undefined);
    },

    installPlugin(path) {
      return postJson<InstalledPlugin>("/plugins", { path });
    },
  };
}
