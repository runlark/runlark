import {
  abortVar,
  atom,
  computed,
  withAsyncData,
  wrap,
  type Atom,
  type Computed,
} from "@reatom/core";
import {
  TeamApiError,
  canRoleManageMembers,
  type CreateInvitationResponse,
  type DraftsListResponse,
  type LocalRunnerStatusResponse,
  type OfflineDraft,
  type RunnerRegistration,
  type TeamAnnouncement,
  type TeamApiClient,
  type TeamConflictNotice,
  type TeamConnectionState,
  type TeamStatusResponse,
  type VerifiedArtifactInspection,
  type WorkspaceDetailResponse,
  type WorkspaceInvitation,
  type WorkspaceMember,
  type WorkspaceRole,
} from "@/common/team-types";
import { createTeamCollabActions, type TeamCollabActions } from "./actions";

const EMPTY_TEAM_STATUS: TeamStatusResponse = {
  sessions: [],
  workspaces: [],
  project_bindings: [],
  unresolved_drafts_count: 0,
  pending_outbox_count: 0,
  conflict_outbox_count: 0,
};

const EMPTY_DRAFTS: DraftsListResponse = {
  drafts: [],
  outbox: [],
};

interface StatusEnvelope {
  readonly status: TeamStatusResponse;
  readonly offline: boolean;
}

interface InvitationsEnvelope {
  readonly items: readonly WorkspaceInvitation[];
  readonly forbidden: boolean;
}

export interface TeamCollabHooks {
  readonly onBoardInvalidated?: () => Promise<void>;
}

export interface AsyncResourceView<T> {
  readonly data: () => T;
  readonly ready: () => boolean;
  readonly retry: () => void;
}

export interface TeamCollabState {
  readonly syncVersion: Atom<number>;
  readonly selectedWorkspaceId: Atom<string | null>;
  readonly connectionOverride: Atom<TeamConnectionState>;
  readonly connectionState: Computed<TeamConnectionState>;
  readonly conflictNotice: Atom<TeamConflictNotice | null>;
  readonly actionPending: Atom<string | null>;
  readonly actionError: Atom<string | null>;
  readonly announcement: Atom<TeamAnnouncement | null>;
  readonly lastCreatedInvite: Atom<CreateInvitationResponse | null>;
  readonly commentQueuedTaskId: Atom<string | null>;
  readonly invitationsForbidden: Computed<boolean>;
  readonly inspectingDigest: Atom<string | null>;
  readonly artifactInspections: Atom<Readonly<Record<string, VerifiedArtifactInspection>>>;
  readonly teamStatusResource: AsyncResourceView<TeamStatusResponse>;
  readonly workspaceDetailResource: AsyncResourceView<WorkspaceDetailResponse | null>;
  readonly invitationsResource: AsyncResourceView<readonly WorkspaceInvitation[]>;
  readonly draftsResource: AsyncResourceView<DraftsListResponse>;
  readonly runnerStatusResource: AsyncResourceView<LocalRunnerStatusResponse | null>;
  readonly effectiveWorkspaceId: Computed<string | null>;
  readonly members: Computed<readonly WorkspaceMember[]>;
  readonly runners: Computed<readonly RunnerRegistration[]>;
  readonly drafts: Computed<readonly OfflineDraft[]>;
  readonly currentUserRole: Computed<WorkspaceRole | null>;
  readonly canManageMembers: Computed<boolean>;
  readonly unresolvedDraftCount: Computed<number>;
  readonly conflictDraftCount: Computed<number>;
}

export type TeamCollabModel = TeamCollabState & TeamCollabActions;

export function reatomTeamCollabModel(
  api: TeamApiClient,
  hooks: TeamCollabHooks = {},
): TeamCollabModel {
  const prefix = "teamCollab";
  const syncVersion = atom<number>(0, `${prefix}.syncVersion`);
  const selectedWorkspaceId = atom<string | null>(null, `${prefix}.selectedWorkspaceId`);
  const connectionOverride = atom<TeamConnectionState>("idle", `${prefix}.connectionOverride`);
  const conflictNotice = atom<TeamConflictNotice | null>(null, `${prefix}.conflictNotice`);
  const actionPending = atom<string | null>(null, `${prefix}.actionPending`);
  const actionError = atom<string | null>(null, `${prefix}.actionError`);
  const announcement = atom<TeamAnnouncement | null>(null, `${prefix}.announcement`);
  const lastCreatedInvite = atom<CreateInvitationResponse | null>(
    null,
    `${prefix}.lastCreatedInvite`,
  );
  const commentQueuedTaskId = atom<string | null>(null, `${prefix}.commentQueuedTaskId`);
  const inspectingDigest = atom<string | null>(null, `${prefix}.inspectingDigest`);
  const artifactInspections = atom<Readonly<Record<string, VerifiedArtifactInspection>>>(
    {},
    `${prefix}.artifactInspections`,
  );

  let lastStatus: TeamStatusResponse = EMPTY_TEAM_STATUS;
  let lastDrafts: DraftsListResponse = EMPTY_DRAFTS;
  const lastWorkspaceDetailByWs = new Map<string, WorkspaceDetailResponse>();
  const lastInvitationsByWs = new Map<string, readonly WorkspaceInvitation[]>();
  const lastRunnerStatusByWs = new Map<string, LocalRunnerStatusResponse>();

  const teamStatusEnvelope = computed(async (): Promise<StatusEnvelope> => {
    syncVersion();
    const { signal } = abortVar.require();
    try {
      const next = await wrap(api.getTeamStatus(signal));
      lastStatus = next;
      return { status: next, offline: false };
    } catch (error: unknown) {
      if (error instanceof TeamApiError && error.httpStatus === 404) {
        return { status: EMPTY_TEAM_STATUS, offline: false };
      }
      if (error instanceof TeamApiError && error.isOffline) {
        return { status: lastStatus, offline: true };
      }
      throw error;
    }
  }, `${prefix}.teamStatus`).extend(
    withAsyncData({ initState: { status: EMPTY_TEAM_STATUS, offline: false } as StatusEnvelope }),
  );

  const teamStatusResource: AsyncResourceView<TeamStatusResponse> = {
    data: () => teamStatusEnvelope.data().status,
    ready: () => teamStatusEnvelope.ready(),
    retry: () => teamStatusEnvelope.retry(),
  };

  const connectionState = computed<TeamConnectionState>(() => {
    if (teamStatusEnvelope.data().offline) return "offline";
    return connectionOverride();
  }, `${prefix}.connectionState`);

  const effectiveWorkspaceId = computed<string | null>(() => {
    const explicit = selectedWorkspaceId();
    if (explicit) return explicit;
    const status = teamStatusResource.data();
    const firstSessionWs = status.sessions.find(
      (s) => s.default_workspace_id,
    )?.default_workspace_id;
    if (firstSessionWs) return firstSessionWs;
    return status.workspaces[0]?.workspace.id ?? null;
  }, `${prefix}.effectiveWorkspaceId`);

  const workspaceDetailResource = computed(async () => {
    syncVersion();
    const wid = effectiveWorkspaceId();
    if (!wid) return null;
    const { signal } = abortVar.require();
    try {
      const next = await wrap(api.getWorkspace(wid, signal));
      lastWorkspaceDetailByWs.set(wid, next);
      return next;
    } catch (error: unknown) {
      if (error instanceof TeamApiError && error.httpStatus === 404) {
        return null;
      }
      if (error instanceof TeamApiError && error.isOffline) {
        return lastWorkspaceDetailByWs.get(wid) ?? null;
      }
      throw error;
    }
  }, `${prefix}.workspaceDetail`).extend(
    withAsyncData({ initState: null as WorkspaceDetailResponse | null }),
  );

  const invitationsEnvelope = computed(async (): Promise<InvitationsEnvelope> => {
    syncVersion();
    const wid = effectiveWorkspaceId();
    if (!wid) {
      return { items: [], forbidden: false };
    }
    const { signal } = abortVar.require();
    try {
      const list = await wrap(api.listInvitations(wid, signal));
      lastInvitationsByWs.set(wid, list);
      return { items: list, forbidden: false };
    } catch (error: unknown) {
      if (error instanceof TeamApiError && error.httpStatus === 403) {
        return { items: [], forbidden: true };
      }
      if (error instanceof TeamApiError && error.httpStatus === 404) {
        return { items: [], forbidden: false };
      }
      if (error instanceof TeamApiError && error.isOffline) {
        return { items: lastInvitationsByWs.get(wid) ?? [], forbidden: false };
      }
      throw error;
    }
  }, `${prefix}.invitations`).extend(
    withAsyncData({ initState: { items: [], forbidden: false } as InvitationsEnvelope }),
  );

  const invitationsResource: AsyncResourceView<readonly WorkspaceInvitation[]> = {
    data: () => invitationsEnvelope.data().items,
    ready: () => invitationsEnvelope.ready(),
    retry: () => invitationsEnvelope.retry(),
  };

  const invitationsForbidden = computed<boolean>(
    () => invitationsEnvelope.data().forbidden,
    `${prefix}.invitationsForbidden`,
  );

  const draftsResource = computed(async () => {
    syncVersion();
    const { signal } = abortVar.require();
    try {
      const next = await wrap(api.listDrafts(undefined, signal));
      lastDrafts = next;
      return next;
    } catch (error: unknown) {
      if (error instanceof TeamApiError && error.httpStatus === 404) {
        return EMPTY_DRAFTS;
      }
      if (error instanceof TeamApiError && error.isOffline) {
        return lastDrafts;
      }
      throw error;
    }
  }, `${prefix}.drafts`).extend(withAsyncData({ initState: EMPTY_DRAFTS }));

  const runnerStatusResource = computed(async () => {
    syncVersion();
    const wid = effectiveWorkspaceId();
    if (!wid) return null;
    const { signal } = abortVar.require();
    try {
      const next = await wrap(api.getRunnerStatus(wid, signal));
      lastRunnerStatusByWs.set(wid, next);
      return next;
    } catch (error: unknown) {
      if (error instanceof TeamApiError && error.httpStatus === 404) {
        return null;
      }
      if (error instanceof TeamApiError && error.isOffline) {
        return lastRunnerStatusByWs.get(wid) ?? null;
      }
      throw error;
    }
  }, `${prefix}.runnerStatus`).extend(
    withAsyncData({ initState: null as LocalRunnerStatusResponse | null }),
  );

  const members = computed<readonly WorkspaceMember[]>(
    () => workspaceDetailResource.data()?.members ?? [],
    `${prefix}.members`,
  );

  const runners = computed<readonly RunnerRegistration[]>(
    () => runnerStatusResource.data()?.workspace_runners ?? [],
    `${prefix}.runners`,
  );

  const drafts = computed<readonly OfflineDraft[]>(
    () => draftsResource.data().drafts,
    `${prefix}.draftsList`,
  );

  const currentUserRole = computed<WorkspaceRole | null>(() => {
    const status = teamStatusResource.data();
    const wid = effectiveWorkspaceId();
    const session =
      status.sessions.find((s) => s.default_workspace_id === wid) ?? status.sessions[0];
    if (!session) return null;
    return members().find((m) => m.user_id === session.user_id)?.role ?? null;
  }, `${prefix}.currentUserRole`);

  const canManageMembers = computed<boolean>(
    () => canRoleManageMembers(currentUserRole()),
    `${prefix}.canManageMembers`,
  );

  const unresolvedDraftCount = computed<number>(() => {
    const listCount = drafts().filter((d) => d.sync_state.state !== "synced").length;
    return Math.max(listCount, teamStatusResource.data().unresolved_drafts_count);
  }, `${prefix}.unresolvedDraftCount`);

  const conflictDraftCount = computed<number>(() => {
    const listCount = drafts().filter((d) => d.sync_state.state === "conflict").length;
    return Math.max(listCount, teamStatusResource.data().conflict_outbox_count);
  }, `${prefix}.conflictDraftCount`);

  const state: TeamCollabState = {
    syncVersion,
    selectedWorkspaceId,
    connectionOverride,
    connectionState,
    conflictNotice,
    actionPending,
    actionError,
    announcement,
    lastCreatedInvite,
    commentQueuedTaskId,
    invitationsForbidden,
    inspectingDigest,
    artifactInspections,
    teamStatusResource,
    workspaceDetailResource,
    invitationsResource,
    draftsResource,
    runnerStatusResource,
    effectiveWorkspaceId,
    members,
    runners,
    drafts,
    currentUserRole,
    canManageMembers,
    unresolvedDraftCount,
    conflictDraftCount,
  };

  const actions = createTeamCollabActions(api, state, hooks, prefix);

  return {
    ...state,
    ...actions,
  };
}
