import type { HttpClientOptions } from "./runlark-http";
import {
  TeamApiError,
  type AcceptInvitationLocalResponse,
  type AddCommentResponse,
  type CreateInvitationResponse,
  type DraftsListResponse,
  type LocalRunnerStatusResponse,
  type OfflineDraft,
  type PromoteTeamResponse,
  type RunOwnershipResponse,
  type RunnerRegistration,
  type TaskComment,
  type TeamApiClient,
  type TeamCommandSubmissionResult,
  type TeamLoginResponse,
  type TeamStatusResponse,
  type TeamSyncSummary,
  type VerifiedArtifactInspection,
  type WorkspaceDetailResponse,
  type WorkspaceInvitation,
  type WorkspaceMember,
  type WorkspaceRole,
} from "./team-types";

const MAX_ARTIFACT_PREVIEW_CHARS = 32_768;

interface RawErrorBody {
  readonly error?: string;
  readonly server_code?: string;
  readonly message?: string;
  readonly expected_revision?: number;
  readonly actual_revision?: number;
  readonly current_revision?: number;
}

async function parseErrorResponse(res: Response): Promise<TeamApiError> {
  const text = await res.text();
  if (!text.trim()) {
    return new TeamApiError({
      message: `HTTP ${String(res.status)}`,
      httpStatus: res.status,
      code: res.status === 503 ? "team_server_offline" : `http_${String(res.status)}`,
    });
  }
  try {
    const parsed = JSON.parse(text) as RawErrorBody;
    return new TeamApiError({
      message: parsed.message ?? parsed.error ?? text,
      httpStatus: res.status,
      code: parsed.error ?? `http_${String(res.status)}`,
      serverCode: parsed.server_code ?? null,
      expectedRevision: parsed.expected_revision ?? null,
      actualRevision: parsed.actual_revision ?? parsed.current_revision ?? null,
    });
  } catch {
    return new TeamApiError({
      message: text,
      httpStatus: res.status,
      code: res.status === 503 ? "team_server_offline" : `http_${String(res.status)}`,
    });
  }
}

export function createTeamApiClient(options: HttpClientOptions = {}): TeamApiClient {
  const mode = options.mode ?? "dsh-proxy";
  const baseUrl = (options.baseUrl ?? "/api/runlark").replace(/\/+$/u, "");
  const fetchFn = options.fetchImpl ?? globalThis.fetch.bind(globalThis);
  const apiPrefix = mode === "daemon" ? `${baseUrl}/api/v1` : baseUrl;

  async function doFetch(path: string, init?: RequestInit): Promise<Response> {
    try {
      return await fetchFn(`${apiPrefix}${path}`, init);
    } catch (error: unknown) {
      if (error instanceof DOMException && error.name === "AbortError") {
        throw error;
      }
      if (error instanceof Error && error.name === "AbortError") {
        throw error;
      }
      throw new TeamApiError({
        message: error instanceof Error ? error.message : String(error),
        httpStatus: 503,
        code: "team_server_offline",
        isOffline: true,
      });
    }
  }

  async function reqJson<T>(path: string, init?: RequestInit): Promise<T> {
    const res = await doFetch(path, init);
    if (!res.ok) {
      throw await parseErrorResponse(res);
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
    teamLogin(input) {
      return postJson<TeamLoginResponse>("/team/login", input);
    },

    getTeamStatus(signal) {
      return reqJson<TeamStatusResponse>("/team/status", signal ? { signal } : undefined);
    },

    syncTeam(workspaceId) {
      return postJson<TeamSyncSummary>(
        "/team/sync",
        workspaceId ? { workspace_id: workspaceId } : {},
      );
    },

    createWorkspace(input) {
      return postJson<WorkspaceDetailResponse>("/team/workspaces", input);
    },

    getWorkspace(wid, signal) {
      return reqJson<WorkspaceDetailResponse>(
        `/team/workspaces/${encodeURIComponent(wid)}`,
        signal ? { signal } : undefined,
      );
    },

    listMembers(wid, signal) {
      return reqJson<readonly WorkspaceMember[]>(
        `/team/workspaces/${encodeURIComponent(wid)}/members`,
        signal ? { signal } : undefined,
      );
    },

    updateMemberRole(wid, uid, role: WorkspaceRole) {
      return reqJson<WorkspaceMember>(
        `/team/workspaces/${encodeURIComponent(wid)}/members/${encodeURIComponent(uid)}`,
        {
          method: "PATCH",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ role }),
        },
      );
    },

    listInvitations(wid, signal) {
      return reqJson<readonly WorkspaceInvitation[]>(
        `/team/workspaces/${encodeURIComponent(wid)}/invitations`,
        signal ? { signal } : undefined,
      );
    },

    createInvitation(wid, input) {
      return postJson<CreateInvitationResponse>(
        `/team/workspaces/${encodeURIComponent(wid)}/invitations`,
        input,
      );
    },

    revokeInvitation(wid, inviteId) {
      return postJson<WorkspaceInvitation>(
        `/team/workspaces/${encodeURIComponent(wid)}/invitations/${encodeURIComponent(inviteId)}/revoke`,
        {},
      );
    },

    acceptInvitation(input) {
      return postJson<AcceptInvitationLocalResponse>("/team/invitations/accept", input);
    },

    promoteProjectToTeam(projectId, input) {
      return postJson<PromoteTeamResponse>(
        `/projects/${encodeURIComponent(projectId)}/promote-team`,
        input,
      );
    },

    listDrafts(projectId, signal) {
      const q = projectId ? `?project_id=${encodeURIComponent(projectId)}` : "";
      return reqJson<DraftsListResponse>(`/team/drafts${q}`, signal ? { signal } : undefined);
    },

    createDraft(input) {
      return postJson<OfflineDraft>("/team/drafts", input);
    },

    retryDraft(draftId, input = {}) {
      return postJson<OfflineDraft>(`/team/drafts/${encodeURIComponent(draftId)}/retry`, input);
    },

    async deleteDraft(draftId) {
      const res = await doFetch(`/team/drafts/${encodeURIComponent(draftId)}`, {
        method: "DELETE",
      });
      if (!res.ok) {
        throw await parseErrorResponse(res);
      }
    },

    listTaskComments(taskId, signal) {
      return reqJson<readonly TaskComment[]>(
        `/tasks/${encodeURIComponent(taskId)}/comments`,
        signal ? { signal } : undefined,
      );
    },

    addTaskComment(taskId, input) {
      return postJson<AddCommentResponse>(`/tasks/${encodeURIComponent(taskId)}/comments`, input);
    },

    listRunners(workspaceId, signal) {
      const q = workspaceId ? `?workspace_id=${encodeURIComponent(workspaceId)}` : "";
      return reqJson<readonly RunnerRegistration[]>(
        `/team/runners${q}`,
        signal ? { signal } : undefined,
      );
    },

    registerRunner(input) {
      return postJson<RunnerRegistration>("/team/runners/register", input);
    },

    getRunnerStatus(workspaceId, signal) {
      const q = workspaceId ? `?workspace_id=${encodeURIComponent(workspaceId)}` : "";
      return reqJson<LocalRunnerStatusResponse>(
        `/team/runners/status${q}`,
        signal ? { signal } : undefined,
      );
    },

    assignRunner(runId, input) {
      return postJson<RunOwnershipResponse>(
        `/team/runs/${encodeURIComponent(runId)}/assign`,
        input,
      );
    },

    cancelUnreachableRun(runId, input) {
      return postJson<TeamCommandSubmissionResult>(
        `/team/runs/${encodeURIComponent(runId)}/cancel-unreachable`,
        input,
      );
    },

    async inspectArtifact(digest, signal): Promise<VerifiedArtifactInspection> {
      const res = await doFetch(
        `/artifacts/${encodeURIComponent(digest)}`,
        signal ? { signal } : undefined,
      );
      if (res.ok) {
        const mediaType = res.headers.get("content-type") ?? "text/plain";
        const rawText = await res.text();
        const truncated = rawText.length > MAX_ARTIFACT_PREVIEW_CHARS;
        return {
          status: "verified",
          digest,
          mediaType,
          sizeBytes: rawText.length,
          textPreview: truncated ? rawText.slice(0, MAX_ARTIFACT_PREVIEW_CHARS) : rawText,
          truncated,
        };
      }
      const err = await parseErrorResponse(res);
      if (err.httpStatus === 404 || err.code === "artifact_not_found") {
        return { status: "missing", digest, message: err.message };
      }
      if (err.code === "corrupt_artifact") {
        return { status: "corrupted", digest, message: err.message };
      }
      return { status: "error", digest, message: err.message };
    },
  };
}
