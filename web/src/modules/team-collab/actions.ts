import { action, withAsync, wrap } from "@reatom/core";
import {
  TeamApiError,
  buildTaskDraftPayload,
  type DraftKind,
  type TeamApiClient,
  type WorkspaceRole,
} from "@/common/team-types";
import type { TeamCollabHooks, TeamCollabState } from "./model";

export interface TeamCollabActions {
  readonly refreshTeam: () => void;
  readonly dismissConflict: () => void;
  readonly selectWorkspace: (workspaceId: string) => void;
  readonly recordApiFailure: (error: unknown, targetLabel?: string) => void;
  readonly syncNow: (workspaceId?: string) => Promise<void>;
  readonly loginTeam: (input: {
    readonly server_url: string;
    readonly bearer_token: string;
    readonly workspace_id?: string;
  }) => Promise<void>;
  readonly createWorkspace: (input: {
    readonly name: string;
    readonly slug: string;
  }) => Promise<void>;
  readonly updateMemberRole: (userId: string, role: WorkspaceRole) => Promise<void>;
  readonly createInvitation: (input: {
    readonly invitee_hint: string;
    readonly role: WorkspaceRole;
  }) => Promise<void>;
  readonly revokeInvitation: (inviteId: string) => Promise<void>;
  readonly acceptInvitation: (input: {
    readonly server_url: string;
    readonly invite_token: string;
    readonly display_name: string;
    readonly user_id?: string;
  }) => Promise<void>;
  readonly promoteProject: (input: {
    readonly projectId: string;
    readonly workspaceId: string;
  }) => Promise<boolean>;
  readonly registerRunner: (name: string) => Promise<void>;
  readonly assignRunner: (
    runId: string,
    runnerId: string,
    expectedRevision: number,
  ) => Promise<void>;
  readonly cancelUnreachableRun: (
    runId: string,
    reason: string,
    expectedRevision: number,
  ) => Promise<void>;
  readonly addComment: (taskId: string, body: string, runId?: string) => Promise<boolean>;
  readonly createOfflineTaskDraft: (input: {
    readonly projectId: string;
    readonly title: string;
    readonly description: string;
    readonly kind: "standard" | "quick_fix";
  }) => Promise<boolean>;
  readonly createOfflinePlanDraft: (input: {
    readonly projectId: string;
    readonly taskId: string;
    readonly planText: string;
    readonly baseRevision: number;
  }) => Promise<boolean>;
  readonly retryDraft: (
    draftId: string,
    input?: { readonly expected_revision?: number; readonly payload_json?: string },
  ) => Promise<void>;
  readonly deleteDraft: (draftId: string) => Promise<void>;
  readonly inspectArtifact: (digest: string) => Promise<void>;
}

function toMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function createTeamCollabActions(
  api: TeamApiClient,
  state: TeamCollabState,
  hooks: TeamCollabHooks,
  prefix: string,
): TeamCollabActions {
  const bump = (): void => {
    state.syncVersion.set((v) => v + 1);
  };

  const handleFailure = (error: unknown, targetLabel?: string): void => {
    if (error instanceof TeamApiError) {
      if (error.isOffline) {
        state.connectionOverride.set("offline");
      } else if (error.isConflict) {
        state.connectionOverride.set("conflict");
        state.conflictNotice.set({
          message: error.message,
          serverCode: error.serverCode,
          actualRevision: error.actualRevision,
          targetLabel: targetLabel ?? null,
        });
      }
      state.actionError.set(error.message);
      return;
    }
    const msg = toMessage(error);
    if (msg.includes("team_server_offline") || msg.includes("503")) {
      state.connectionOverride.set("offline");
    } else if (msg.includes("409") || msg.toLowerCase().includes("conflict")) {
      state.connectionOverride.set("conflict");
      state.conflictNotice.set({
        message: msg,
        serverCode: null,
        actualRevision: null,
        targetLabel: targetLabel ?? null,
      });
    }
    state.actionError.set(msg);
  };

  const runTeamMutation = async (
    key: string,
    fn: () => Promise<void>,
    targetLabel?: string,
  ): Promise<boolean> => {
    state.actionPending.set(key);
    state.actionError.set(null);
    try {
      await wrap(fn());
      bump();
      if (hooks.onBoardInvalidated) {
        await wrap(hooks.onBoardInvalidated());
      }
      return true;
    } catch (error: unknown) {
      handleFailure(error, targetLabel);
      return false;
    } finally {
      if (state.actionPending() === key) {
        state.actionPending.set(null);
      }
    }
  };

  const refreshTeam = action(() => {
    bump();
  }, `${prefix}.refreshTeam`);

  const dismissConflict = action(() => {
    state.conflictNotice.set(null);
    if (state.connectionOverride() === "conflict" && state.conflictDraftCount() === 0) {
      state.connectionOverride.set("online");
    }
  }, `${prefix}.dismissConflict`);

  const selectWorkspace = action((workspaceId: string) => {
    state.selectedWorkspaceId.set(workspaceId);
    state.lastCreatedInvite.set(null);
    bump();
  }, `${prefix}.selectWorkspace`);

  const recordApiFailure = action((error: unknown, targetLabel?: string) => {
    handleFailure(error, targetLabel);
  }, `${prefix}.recordApiFailure`);

  const syncNow = action(async (workspaceId?: string) => {
    state.connectionOverride.set("reconnecting");
    state.actionPending.set("syncNow");
    state.actionError.set(null);
    try {
      const wid = workspaceId ?? state.effectiveWorkspaceId() ?? undefined;
      const summary = await wrap(api.syncTeam(wid));
      if (summary.outbox_conflicts > 0) {
        state.connectionOverride.set("conflict");
      } else {
        state.connectionOverride.set("online");
        state.conflictNotice.set(null);
      }
      state.announcement.set({
        kind: "synced",
        eventsApplied: summary.events_applied,
        outboxAcked: summary.outbox_acked,
      });
      bump();
      if (hooks.onBoardInvalidated) {
        await wrap(hooks.onBoardInvalidated());
      }
    } catch (error: unknown) {
      handleFailure(error, "sync");
    } finally {
      if (state.actionPending() === "syncNow") {
        state.actionPending.set(null);
      }
    }
  }, `${prefix}.syncNow`).extend(withAsync());

  const loginTeam = action(
    async (input: {
      readonly server_url: string;
      readonly bearer_token: string;
      readonly workspace_id?: string;
    }) => {
      await wrap(
        runTeamMutation("loginTeam", async () => {
          const res = await wrap(api.teamLogin(input));
          if (res.workspace_id) {
            state.selectedWorkspaceId.set(res.workspace_id);
          }
          state.connectionOverride.set("online");
          state.announcement.set({ kind: "logged_in", displayName: res.display_name });
        }),
      );
    },
    `${prefix}.loginTeam`,
  ).extend(withAsync());

  const createWorkspace = action(
    async (input: { readonly name: string; readonly slug: string }) => {
      await wrap(
        runTeamMutation("createWorkspace", async () => {
          const created = await wrap(api.createWorkspace(input));
          state.selectedWorkspaceId.set(created.workspace.id);
          state.connectionOverride.set("online");
          state.announcement.set({ kind: "workspace_created", name: created.workspace.name });
        }),
      );
    },
    `${prefix}.createWorkspace`,
  ).extend(withAsync());

  const updateMemberRole = action(async (userId: string, role: WorkspaceRole) => {
    const wid = state.effectiveWorkspaceId();
    if (!wid) return;
    await wrap(
      runTeamMutation(
        "updateMemberRole",
        async () => {
          await wrap(api.updateMemberRole(wid, userId, role));
          state.connectionOverride.set("online");
          state.announcement.set({ kind: "role_updated", userId, role });
        },
        userId,
      ),
    );
  }, `${prefix}.updateMemberRole`).extend(withAsync());

  const createInvitation = action(
    async (input: { readonly invitee_hint: string; readonly role: WorkspaceRole }) => {
      const wid = state.effectiveWorkspaceId();
      if (!wid) return;
      await wrap(
        runTeamMutation("createInvitation", async () => {
          const created = await wrap(api.createInvitation(wid, input));
          state.lastCreatedInvite.set(created);
          state.connectionOverride.set("online");
          state.announcement.set({
            kind: "invitation_created",
            inviteeHint: created.invitation.invitee_hint,
          });
        }),
      );
    },
    `${prefix}.createInvitation`,
  ).extend(withAsync());

  const revokeInvitation = action(async (inviteId: string) => {
    const wid = state.effectiveWorkspaceId();
    if (!wid) return;
    await wrap(
      runTeamMutation("revokeInvitation", async () => {
        await wrap(api.revokeInvitation(wid, inviteId));
        state.connectionOverride.set("online");
        state.announcement.set({ kind: "invitation_revoked", inviteId });
      }),
    );
  }, `${prefix}.revokeInvitation`).extend(withAsync());

  const acceptInvitation = action(
    async (input: {
      readonly server_url: string;
      readonly invite_token: string;
      readonly display_name: string;
      readonly user_id?: string;
    }) => {
      await wrap(
        runTeamMutation("acceptInvitation", async () => {
          const res = await wrap(api.acceptInvitation(input));
          state.selectedWorkspaceId.set(res.member.workspace_id);
          state.connectionOverride.set("online");
          state.announcement.set({
            kind: "invitation_accepted",
            workspaceId: res.member.workspace_id,
          });
        }),
      );
    },
    `${prefix}.acceptInvitation`,
  ).extend(withAsync());

  const promoteProject = action(
    async (input: { readonly projectId: string; readonly workspaceId: string }) =>
      await wrap(
        runTeamMutation(
          "promoteProject",
          async () => {
            const res = await wrap(
              api.promoteProjectToTeam(input.projectId, { workspace_id: input.workspaceId }),
            );
            state.selectedWorkspaceId.set(res.ack.workspace_id);
            state.connectionOverride.set("online");
            state.announcement.set({
              kind: "project_promoted",
              projectId: input.projectId,
              workspaceId: res.ack.workspace_id,
            });
          },
          input.projectId,
        ),
      ),
    `${prefix}.promoteProject`,
  ).extend(withAsync());

  const registerRunner = action(async (name: string) => {
    const wid = state.effectiveWorkspaceId() ?? undefined;
    await wrap(
      runTeamMutation("registerRunner", async () => {
        const reg = await wrap(api.registerRunner({ ...(wid ? { workspace_id: wid } : {}), name }));
        state.connectionOverride.set("online");
        state.announcement.set({ kind: "runner_registered", name: reg.name });
      }),
    );
  }, `${prefix}.registerRunner`).extend(withAsync());

  const assignRunner = action(async (runId: string, runnerId: string, expectedRevision: number) => {
    const wid = state.effectiveWorkspaceId() ?? undefined;
    await wrap(
      runTeamMutation(
        "assignRunner",
        async () => {
          const res = await wrap(
            api.assignRunner(runId, {
              ...(wid ? { workspace_id: wid } : {}),
              runner_id: runnerId,
              expected_revision: expectedRevision,
            }),
          );
          state.connectionOverride.set("online");
          state.announcement.set({
            kind: "runner_assigned",
            runnerId,
            leaseEpoch: res.ownership.lease_epoch,
          });
        },
        runId,
      ),
    );
  }, `${prefix}.assignRunner`).extend(withAsync());

  const cancelUnreachableRun = action(
    async (runId: string, reason: string, expectedRevision: number) => {
      const wid = state.effectiveWorkspaceId() ?? undefined;
      await wrap(
        runTeamMutation(
          "cancelUnreachableRun",
          async () => {
            await wrap(
              api.cancelUnreachableRun(runId, {
                ...(wid ? { workspace_id: wid } : {}),
                reason,
                expected_revision: expectedRevision,
              }),
            );
            state.connectionOverride.set("online");
            state.announcement.set({ kind: "run_cancelled_unreachable", runId });
          },
          runId,
        ),
      );
    },
    `${prefix}.cancelUnreachableRun`,
  ).extend(withAsync());

  const addComment = action(async (taskId: string, body: string, runId?: string) => {
    state.commentQueuedTaskId.set(null);
    return await wrap(
      runTeamMutation(
        "addComment",
        async () => {
          const res = await wrap(
            api.addTaskComment(taskId, { body, ...(runId ? { run_id: runId } : {}) }),
          );
          if (res.delivery === "queued_offline") {
            state.connectionOverride.set("offline");
            state.commentQueuedTaskId.set(taskId);
            state.announcement.set({ kind: "comment_queued_offline" });
          } else {
            state.connectionOverride.set("online");
            state.commentQueuedTaskId.set(null);
            state.announcement.set({ kind: "comment_posted" });
          }
        },
        taskId,
      ),
    );
  }, `${prefix}.addComment`).extend(withAsync());

  const createDraft = async (
    key: string,
    input: {
      readonly project_id: string;
      readonly task_id?: string;
      readonly kind: DraftKind;
      readonly payload_json: string;
      readonly base_revision?: number;
    },
  ): Promise<boolean> =>
    await wrap(
      runTeamMutation(key, async () => {
        const draft = await wrap(api.createDraft({ ...input, queue_sync: true }));
        state.announcement.set({ kind: "draft_saved", draftId: draft.id });
      }),
    );

  const createOfflineTaskDraft = action(
    async (input: {
      readonly projectId: string;
      readonly title: string;
      readonly description: string;
      readonly kind: "standard" | "quick_fix";
    }) =>
      await wrap(
        createDraft("createOfflineTaskDraft", {
          project_id: input.projectId,
          kind: "task_create",
          payload_json: buildTaskDraftPayload({
            title: input.title,
            description: input.description,
            kind: input.kind,
          }),
        }),
      ),
    `${prefix}.createOfflineTaskDraft`,
  ).extend(withAsync());

  const createOfflinePlanDraft = action(
    async (input: {
      readonly projectId: string;
      readonly taskId: string;
      readonly planText: string;
      readonly baseRevision: number;
    }) =>
      await wrap(
        createDraft("createOfflinePlanDraft", {
          project_id: input.projectId,
          task_id: input.taskId,
          kind: "plan_edit",
          payload_json: JSON.stringify({ plan_text: input.planText }),
          base_revision: input.baseRevision,
        }),
      ),
    `${prefix}.createOfflinePlanDraft`,
  ).extend(withAsync());

  const retryDraft = action(
    async (
      draftId: string,
      input?: { readonly expected_revision?: number; readonly payload_json?: string },
    ) => {
      await wrap(
        runTeamMutation(
          "retryDraft",
          async () => {
            const latest = await wrap(api.retryDraft(draftId, input));
            if (latest.sync_state.state === "conflict") {
              state.connectionOverride.set("conflict");
              state.conflictNotice.set({
                message: latest.sync_state.reason,
                serverCode: "revision_mismatch",
                actualRevision: latest.sync_state.server_revision,
                targetLabel: draftId,
              });
            } else if (latest.sync_state.state === "synced") {
              state.connectionOverride.set("online");
              state.conflictNotice.set(null);
              state.announcement.set({ kind: "draft_synced", draftId });
            }
          },
          draftId,
        ),
      );
    },
    `${prefix}.retryDraft`,
  ).extend(withAsync());

  const deleteDraft = action(async (draftId: string) => {
    await wrap(
      runTeamMutation("deleteDraft", async () => {
        await wrap(api.deleteDraft(draftId));
        state.announcement.set({ kind: "draft_discarded", draftId });
      }),
    );
  }, `${prefix}.deleteDraft`).extend(withAsync());

  const inspectArtifact = action(async (digest: string) => {
    state.inspectingDigest.set(digest);
    try {
      const result = await wrap(api.inspectArtifact(digest));
      state.artifactInspections.set({
        ...state.artifactInspections(),
        [digest]: result,
      });
    } catch (error: unknown) {
      if (error instanceof DOMException && error.name === "AbortError") {
        return;
      }
      state.artifactInspections.set({
        ...state.artifactInspections(),
        [digest]: { status: "error", digest, message: toMessage(error) },
      });
      handleFailure(error, digest);
    } finally {
      if (state.inspectingDigest() === digest) {
        state.inspectingDigest.set(null);
      }
    }
  }, `${prefix}.inspectArtifact`).extend(withAsync());

  return {
    refreshTeam,
    dismissConflict,
    selectWorkspace,
    recordApiFailure,
    syncNow,
    loginTeam,
    createWorkspace,
    updateMemberRole,
    createInvitation,
    revokeInvitation,
    acceptInvitation,
    promoteProject,
    registerRunner,
    assignRunner,
    cancelUnreachableRun,
    addComment,
    createOfflineTaskDraft,
    createOfflinePlanDraft,
    retryDraft,
    deleteDraft,
    inspectArtifact,
  };
}
