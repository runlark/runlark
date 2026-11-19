export type WorkspaceRole = "viewer" | "member" | "maintainer" | "owner";
export type InvitationStatus = "pending" | "accepted" | "revoked" | "expired";
export type RunnerStatus = "online" | "unreachable" | "drained";
export type DraftKind = "task_create" | "task_comment" | "plan_edit" | "decision_intent";
export type SyncOutboxStatus = "pending" | "dispatched" | "acked" | "conflict" | "failed";
export type TeamConnectionState = "idle" | "online" | "offline" | "reconnecting" | "conflict";

export type ProjectOwnershipMode =
  | { readonly mode: "solo_local" }
  | {
      readonly mode: "team_managed";
      readonly workspace_id: string;
      readonly server_url: string;
      readonly ownership_epoch: number;
    };

export type DraftSyncState =
  | { readonly state: "local_draft" }
  | { readonly state: "pending_sync" }
  | {
      readonly state: "conflict";
      readonly server_revision: number | null;
      readonly reason: string;
    }
  | { readonly state: "synced" };

export interface WorkspaceRecord {
  readonly id: string;
  readonly name: string;
  readonly slug: string;
  readonly created_by: string;
  readonly revision: number;
  readonly created_at: number;
  readonly updated_at: number;
}

export interface WorkspaceMember {
  readonly workspace_id: string;
  readonly user_id: string;
  readonly display_name: string;
  readonly role: WorkspaceRole;
  readonly joined_at: number;
}

export interface WorkspaceInvitation {
  readonly id: string;
  readonly workspace_id: string;
  readonly invitee_hint: string;
  readonly role: WorkspaceRole;
  readonly status: InvitationStatus;
  readonly invited_by: string;
  readonly expires_at: number;
  readonly created_at: number;
  readonly accepted_at: number | null;
  readonly accepted_by: string | null;
}

export interface RunnerRegistration {
  readonly id: string;
  readonly workspace_id: string;
  readonly owner_user_id: string;
  readonly name: string;
  readonly status: RunnerStatus;
  readonly last_heartbeat_at: number;
  readonly created_at: number;
}

export interface RunOwnership {
  readonly assigned_runner_id: string | null;
  readonly lease_epoch: number;
  readonly runner_unreachable: boolean;
}

export interface TaskComment {
  readonly id: string;
  readonly task_id: string;
  readonly run_id: string | null;
  readonly author_user_id: string;
  readonly author_display_name: string;
  readonly body: string;
  readonly created_at: number;
}

export interface OfflineDraft {
  readonly id: string;
  readonly project_id: string;
  readonly task_id: string | null;
  readonly kind: DraftKind;
  readonly payload_json: string;
  readonly base_revision: number | null;
  readonly sync_state: DraftSyncState;
  readonly created_at: number;
  readonly updated_at: number;
}

export interface SyncOutboxEntry {
  readonly command_id: string;
  readonly workspace_id: string;
  readonly project_id: string | null;
  readonly draft_id: string | null;
  readonly command_type: string;
  readonly payload_json: string;
  readonly expected_revision: number | null;
  readonly status: SyncOutboxStatus;
  readonly last_error: string | null;
  readonly conflict_server_revision: number | null;
  readonly created_at: number;
  readonly updated_at: number;
}

export interface TeamSessionSummary {
  readonly server_url: string;
  readonly user_id: string;
  readonly display_name: string;
  readonly default_workspace_id: string | null;
  readonly runner_id: string | null;
  readonly runner_name: string | null;
}

export interface TeamWorkspaceRecord {
  readonly workspace: WorkspaceRecord;
  readonly server_url: string;
  readonly last_cursor_seq: number;
}

export interface ProjectSyncBinding {
  readonly project_id: string;
  readonly ownership_mode: ProjectOwnershipMode;
  readonly last_cursor_seq: number;
  readonly updated_at: number;
}

export interface TeamStatusResponse {
  readonly sessions: readonly TeamSessionSummary[];
  readonly workspaces: readonly TeamWorkspaceRecord[];
  readonly project_bindings: readonly ProjectSyncBinding[];
  readonly unresolved_drafts_count: number;
  readonly pending_outbox_count: number;
  readonly conflict_outbox_count: number;
}

export interface TeamLoginResponse {
  readonly user_id: string;
  readonly display_name: string;
  readonly server_url: string;
  readonly workspace_id: string | null;
}

export interface TeamSyncSummary {
  readonly workspaces_synced: number;
  readonly snapshots_applied: number;
  readonly events_applied: number;
  readonly outbox_acked: number;
  readonly outbox_conflicts: number;
  readonly cas_uploaded: number;
  readonly cas_downloaded: number;
}

export interface WorkspaceDetailResponse {
  readonly workspace: WorkspaceRecord;
  readonly members: readonly WorkspaceMember[];
}

export interface CreateInvitationResponse {
  readonly invitation: WorkspaceInvitation;
  readonly invite_token: string;
}

export interface AcceptInvitationLocalResponse {
  readonly member: WorkspaceMember;
  readonly server_url: string;
  readonly token_saved: boolean;
}

export interface ServerPromotionAck {
  readonly workspace_id: string;
  readonly project_id: string;
  readonly initial_cursor: number;
  readonly ownership_epoch: number;
  readonly acknowledged_at: number;
}

export interface PromoteTeamResponse {
  readonly ack: ServerPromotionAck;
  readonly binding: ProjectSyncBinding;
}

export interface DraftsListResponse {
  readonly drafts: readonly OfflineDraft[];
  readonly outbox: readonly SyncOutboxEntry[];
}

export type AddCommentResponse =
  | { readonly delivery: "synced"; readonly comment: TaskComment }
  | { readonly delivery: "queued_offline"; readonly draft: OfflineDraft };

export interface LocalRunnerStatusResponse {
  readonly workspace_id: string | null;
  readonly local_runner_id: string | null;
  readonly local_runner_name: string | null;
  readonly buffered_events_count: number;
  readonly workspace_runners: readonly RunnerRegistration[];
}

export interface RunOwnershipResponse {
  readonly run: {
    readonly id: string;
    readonly status: string;
    readonly revision: number;
  };
  readonly ownership: RunOwnership;
}

export interface TeamCommandSubmissionResult {
  readonly command_id: string;
  readonly idempotent_replay: boolean;
  readonly cursor_seq: number;
  readonly aggregate_type: string;
  readonly aggregate_id: string;
  readonly revision: number;
}

export type VerifiedArtifactInspection =
  | {
      readonly status: "verified";
      readonly digest: string;
      readonly mediaType: string;
      readonly sizeBytes: number;
      readonly textPreview: string;
      readonly truncated: boolean;
    }
  | {
      readonly status: "missing" | "corrupted" | "error";
      readonly digest: string;
      readonly message: string;
    };

export interface TeamConflictNotice {
  readonly message: string;
  readonly serverCode: string | null;
  readonly actualRevision: number | null;
  readonly targetLabel: string | null;
}

export type TeamAnnouncement =
  | { readonly kind: "synced"; readonly eventsApplied: number; readonly outboxAcked: number }
  | { readonly kind: "logged_in"; readonly displayName: string }
  | { readonly kind: "workspace_created"; readonly name: string }
  | { readonly kind: "role_updated"; readonly userId: string; readonly role: WorkspaceRole }
  | { readonly kind: "invitation_created"; readonly inviteeHint: string }
  | { readonly kind: "invitation_revoked"; readonly inviteId: string }
  | { readonly kind: "invitation_accepted"; readonly workspaceId: string }
  | { readonly kind: "project_promoted"; readonly projectId: string; readonly workspaceId: string }
  | { readonly kind: "runner_registered"; readonly name: string }
  | { readonly kind: "runner_assigned"; readonly runnerId: string; readonly leaseEpoch: number }
  | { readonly kind: "run_cancelled_unreachable"; readonly runId: string }
  | { readonly kind: "comment_queued_offline" }
  | { readonly kind: "comment_posted" }
  | { readonly kind: "draft_saved"; readonly draftId: string }
  | { readonly kind: "draft_synced"; readonly draftId: string }
  | { readonly kind: "draft_discarded"; readonly draftId: string };

export class TeamApiError extends Error {
  readonly httpStatus: number;
  readonly code: string;
  readonly serverCode: string | null;
  readonly actualRevision: number | null;
  readonly expectedRevision: number | null;
  readonly isOffline: boolean;
  readonly isConflict: boolean;

  constructor(params: {
    readonly message: string;
    readonly httpStatus: number;
    readonly code: string;
    readonly serverCode?: string | null;
    readonly actualRevision?: number | null;
    readonly expectedRevision?: number | null;
    readonly isOffline?: boolean;
  }) {
    super(params.message);
    this.name = "TeamApiError";
    this.httpStatus = params.httpStatus;
    this.code = params.code;
    this.serverCode = params.serverCode ?? null;
    this.actualRevision = params.actualRevision ?? null;
    this.expectedRevision = params.expectedRevision ?? null;
    this.isOffline =
      params.isOffline === true ||
      params.httpStatus === 503 ||
      params.code === "team_server_offline";
    this.isConflict = params.httpStatus === 409;
  }
}

export interface TeamApiClient {
  teamLogin(input: {
    readonly server_url: string;
    readonly bearer_token: string;
    readonly workspace_id?: string;
  }): Promise<TeamLoginResponse>;
  getTeamStatus(signal?: AbortSignal): Promise<TeamStatusResponse>;
  syncTeam(workspaceId?: string): Promise<TeamSyncSummary>;
  createWorkspace(input: {
    readonly server_url?: string;
    readonly name: string;
    readonly slug: string;
  }): Promise<WorkspaceDetailResponse>;
  getWorkspace(wid: string, signal?: AbortSignal): Promise<WorkspaceDetailResponse>;
  listMembers(wid: string, signal?: AbortSignal): Promise<readonly WorkspaceMember[]>;
  updateMemberRole(wid: string, uid: string, role: WorkspaceRole): Promise<WorkspaceMember>;
  listInvitations(wid: string, signal?: AbortSignal): Promise<readonly WorkspaceInvitation[]>;
  createInvitation(
    wid: string,
    input: { readonly invitee_hint: string; readonly role: WorkspaceRole },
  ): Promise<CreateInvitationResponse>;
  revokeInvitation(wid: string, inviteId: string): Promise<WorkspaceInvitation>;
  acceptInvitation(input: {
    readonly server_url: string;
    readonly invite_token: string;
    readonly display_name: string;
    readonly user_id?: string;
  }): Promise<AcceptInvitationLocalResponse>;
  promoteProjectToTeam(
    projectId: string,
    input: {
      readonly workspace_id: string;
      readonly server_url?: string;
      readonly bearer_token?: string;
    },
  ): Promise<PromoteTeamResponse>;
  listDrafts(projectId?: string, signal?: AbortSignal): Promise<DraftsListResponse>;
  createDraft(input: {
    readonly project_id: string;
    readonly task_id?: string;
    readonly kind: DraftKind;
    readonly payload_json: string;
    readonly base_revision?: number;
    readonly queue_sync?: boolean;
  }): Promise<OfflineDraft>;
  retryDraft(
    draftId: string,
    input?: { readonly expected_revision?: number; readonly payload_json?: string },
  ): Promise<OfflineDraft>;
  deleteDraft(draftId: string): Promise<void>;
  listTaskComments(taskId: string, signal?: AbortSignal): Promise<readonly TaskComment[]>;
  addTaskComment(
    taskId: string,
    input: { readonly body: string; readonly run_id?: string },
  ): Promise<AddCommentResponse>;
  listRunners(workspaceId?: string, signal?: AbortSignal): Promise<readonly RunnerRegistration[]>;
  registerRunner(input: {
    readonly workspace_id?: string;
    readonly name: string;
  }): Promise<RunnerRegistration>;
  getRunnerStatus(workspaceId?: string, signal?: AbortSignal): Promise<LocalRunnerStatusResponse>;
  assignRunner(
    runId: string,
    input: {
      readonly workspace_id?: string;
      readonly runner_id: string;
      readonly expected_revision?: number;
    },
  ): Promise<RunOwnershipResponse>;
  cancelUnreachableRun(
    runId: string,
    input: {
      readonly workspace_id?: string;
      readonly reason: string;
      readonly expected_revision?: number;
    },
  ): Promise<TeamCommandSubmissionResult>;
  inspectArtifact(digest: string, signal?: AbortSignal): Promise<VerifiedArtifactInspection>;
}

export function canRoleManageMembers(role: WorkspaceRole | null): boolean {
  return role === "owner" || role === "maintainer";
}

export function isTeamManagedMode(
  mode: ProjectOwnershipMode | null | undefined,
): mode is Extract<ProjectOwnershipMode, { readonly mode: "team_managed" }> {
  return mode !== null && mode !== undefined && mode.mode === "team_managed";
}

export function buildTaskDraftPayload(input: {
  readonly title: string;
  readonly description: string;
  readonly kind: "standard" | "quick_fix";
}): string {
  return JSON.stringify({
    title: input.title,
    description: input.description,
    kind: input.kind,
  });
}
