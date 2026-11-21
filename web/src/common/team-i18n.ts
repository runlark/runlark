import type { Locale } from "./i18n";
import { TEAM_RU } from "./team-i18n-ru";
import type {
  DraftKind,
  InvitationStatus,
  RunnerStatus,
  TeamAnnouncement,
  WorkspaceRole,
} from "./team-types";

export interface TeamStrings {
  readonly teamHeading: string;
  readonly teamSubtitle: string;
  readonly ownershipSolo: string;
  readonly ownershipTeam: (workspaceId: string, epoch: number) => string;
  readonly syncOnline: (cursor: number) => string;
  readonly syncOffline: string;
  readonly syncReconnecting: string;
  readonly syncConflict: string;
  readonly syncNowBtn: string;
  readonly reconnectBtn: string;
  readonly draftsSummaryChip: (unresolved: number, conflicts: number) => string;
  readonly openTeamQueueBtn: string;
  readonly offlineGateNotice: string;
  readonly conflictBannerTitle: (rev: number | null) => string;
  readonly serverConnectionTitle: string;
  readonly serverUrlLabel: string;
  readonly bearerTokenLabel: string;
  readonly workspaceIdOptionalLabel: string;
  readonly connectServerBtn: string;
  readonly connectedSessionsHeading: string;
  readonly noConnectedSessions: string;
  readonly acceptInviteHeading: string;
  readonly inviteTokenLabel: string;
  readonly displayNameLabel: string;
  readonly userIdOptionalLabel: string;
  readonly acceptInviteBtn: string;
  readonly workspaceSelectorHeading: string;
  readonly activeWorkspaceLabel: string;
  readonly createWorkspaceHeading: string;
  readonly workspaceNameLabel: string;
  readonly workspaceSlugLabel: string;
  readonly createWorkspaceBtn: string;
  readonly membersHeading: (count: number) => string;
  readonly membersEmpty: string;
  readonly roleSelectLabel: (displayName: string) => string;
  readonly roleManageDeniedHint: string;
  readonly lastOwnerHint: string;
  readonly invitationsHeading: (count: number) => string;
  readonly invitationsEmpty: string;
  readonly inviteeHintLabel: string;
  readonly inviteRoleLabel: string;
  readonly createInviteBtn: string;
  readonly oneTimeTokenWarning: string;
  readonly revokeInviteBtn: string;
  readonly invitationsForbiddenHint: string;
  readonly runnersHeading: (count: number) => string;
  readonly runnersEmpty: string;
  readonly registerRunnerHeading: string;
  readonly runnerNameLabel: string;
  readonly registerRunnerBtn: string;
  readonly localRunnerSummary: (id: string, name: string, buffered: number) => string;
  readonly noLocalRunner: string;
  readonly draftsHeading: (count: number) => string;
  readonly draftsEmpty: string;
  readonly draftStateLocal: string;
  readonly draftStatePending: string;
  readonly draftStateConflict: (serverRev: number | null) => string;
  readonly draftStateSynced: string;
  readonly draftExpectedRevLabel: string;
  readonly draftPayloadLabel: string;
  readonly draftRetryBtn: string;
  readonly draftDiscardBtn: string;
  readonly discardDraftDialogTitle: string;
  readonly discardDraftDialogBody: string;
  readonly discardDraftConfirmBtn: string;
  readonly promoteToTeamBtn: string;
  readonly promoteDialogTitle: string;
  readonly promoteWorkspaceLabel: string;
  readonly promoteBlockedNotice: (activeCount: number) => string;
  readonly promoteSanitizeNotice: string;
  readonly promoteConfirmBtn: string;
  readonly runnerOwnershipHeading: string;
  readonly runnerUnassignedLabel: string;
  readonly runnerExclusiveOwner: (runnerLabel: string, epoch: number) => string;
  readonly noAutoTakeoverNotice: string;
  readonly assignRunnerSelectLabel: string;
  readonly assignRunnerBtn: string;
  readonly noOnlineRunnersHint: string;
  readonly runnerUnreachableBadge: string;
  readonly cancelUnreachableBtn: string;
  readonly cancelUnreachableDialogTitle: string;
  readonly cancelUnreachableDialogBody: (runnerLabel: string, epoch: number) => string;
  readonly cancelUnreachableReasonLabel: string;
  readonly cancelUnreachableReasonRequired: string;
  readonly cancelUnreachableConfirmBtn: string;
  readonly commentsHeading: (count: number) => string;
  readonly commentsEmpty: string;
  readonly commentsImmutableHint: string;
  readonly commentBodyLabel: string;
  readonly commentSubmitBtn: string;
  readonly commentQueuedOfflineNotice: string;
  readonly commentDraftsHeading: string;
  readonly saveTaskOfflineDraftBtn: string;
  readonly savePlanOfflineDraftBtn: string;
  readonly casInspectBtn: string;
  readonly casInspectingBtn: string;
  readonly casVerifiedBadge: string;
  readonly casMissingBadge: string;
  readonly casCorruptedBadge: string;
  readonly casTruncatedNotice: string;
  readonly mobileAcceptanceHeading: string;
  readonly mobileAcceptanceOfflineHint: string;
  readonly mobileAcceptDialogTitle: string;
  readonly mobileAcceptConfirmBtn: string;
  readonly mobileRejectDialogTitle: string;
  readonly mobileRevisionPlanLabel: (revision: number, planVersion: number) => string;
  readonly formatAnnouncement: (announcement: TeamAnnouncement | null) => string;
  readonly fieldRequiredError: string;
  readonly cancelBtn: string;
  readonly roleLabel: (role: WorkspaceRole) => string;
  readonly inviteStatusLabel: (status: InvitationStatus) => string;
  readonly runnerStatusLabel: (status: RunnerStatus) => string;
  readonly draftKindLabel: (kind: DraftKind) => string;
  readonly conflictCodeHint: (serverCode: string | null, fallback: string) => string;
}

export const TEAM_EN: TeamStrings = {
  teamHeading: "Team Collaboration",
  teamSubtitle: "Shared workspaces, roles, runners, offline drafts and conflict resolution",
  ownershipSolo: "Solo Local",
  ownershipTeam: (wid, epoch) => `Team · ${wid} · e${String(epoch)}`,
  syncOnline: (cursor) => `Online · cursor #${String(cursor)}`,
  syncOffline: "Offline · authoritative mutations locked",
  syncReconnecting: "Syncing with team server...",
  syncConflict: "Conflict (409) · review required",
  syncNowBtn: "Sync now",
  reconnectBtn: "Reconnect & sync",
  draftsSummaryChip: (u, c) =>
    c > 0 ? `${String(u)} drafts · ${String(c)} conflict(s)` : `${String(u)} offline draft(s)`,
  openTeamQueueBtn: "Open queue",
  offlineGateNotice:
    "Team server is unreachable. Authoritative transitions are locked; comments, task drafts and plan edits can be saved as offline drafts.",
  conflictBannerTitle: (rev) =>
    rev !== null
      ? `Revision conflict (409) — server is at rev ${String(rev)}. Sync or rebase before retrying.`
      : "Conflict (409) — refresh server state before retrying.",
  serverConnectionTitle: "Team Server & Invitations",
  serverUrlLabel: "Server URL",
  bearerTokenLabel: "Bearer token",
  workspaceIdOptionalLabel: "Workspace ID (optional)",
  connectServerBtn: "Connect server",
  connectedSessionsHeading: "Connected sessions",
  noConnectedSessions: "No team server session configured yet.",
  acceptInviteHeading: "Join via invitation token",
  inviteTokenLabel: "One-time invite token",
  displayNameLabel: "Display name",
  userIdOptionalLabel: "User ID (optional)",
  acceptInviteBtn: "Accept invitation",
  workspaceSelectorHeading: "Workspaces",
  activeWorkspaceLabel: "Active workspace",
  createWorkspaceHeading: "Create workspace",
  workspaceNameLabel: "Workspace name",
  workspaceSlugLabel: "Slug",
  createWorkspaceBtn: "Create workspace",
  membersHeading: (n) => `Members (${String(n)})`,
  membersEmpty: "No workspace members loaded yet.",
  roleSelectLabel: (name) => `Workspace role for ${name}`,
  roleManageDeniedHint: "Only Owners and Maintainers can change member roles.",
  lastOwnerHint: "Cannot demote the sole remaining Owner.",
  invitationsHeading: (n) => `Invitations (${String(n)})`,
  invitationsEmpty: "No invitations recorded for this workspace.",
  inviteeHintLabel: "Invitee email or handle",
  inviteRoleLabel: "Role",
  createInviteBtn: "Create invitation",
  oneTimeTokenWarning: "Shown once — copy and share securely:",
  revokeInviteBtn: "Revoke",
  invitationsForbiddenHint: "Only Owners and Maintainers can view or manage workspace invitations.",
  runnersHeading: (n) => `Workspace Runners (${String(n)})`,
  runnersEmpty: "No execution runners registered for this workspace.",
  registerRunnerHeading: "Register local runner",
  runnerNameLabel: "Runner display name",
  registerRunnerBtn: "Register runner",
  localRunnerSummary: (id, name, buffered) =>
    `Local runner: ${name} (${id}) · ${String(buffered)} buffered event(s)`,
  noLocalRunner: "This daemon is not yet registered as a workspace runner.",
  draftsHeading: (n) => `Offline Drafts & Conflict Queue (${String(n)})`,
  draftsEmpty: "No offline drafts or conflicts pending.",
  draftStateLocal: "Local draft",
  draftStatePending: "Pending sync",
  draftStateConflict: (rev) =>
    rev !== null ? `Conflict (409 · server rev ${String(rev)})` : "Conflict (409)",
  draftStateSynced: "Synced",
  draftExpectedRevLabel: "Expected revision",
  draftPayloadLabel: "Draft payload (JSON)",
  draftRetryBtn: "Rebase & retry",
  draftDiscardBtn: "Discard draft",
  discardDraftDialogTitle: "Discard offline draft?",
  discardDraftDialogBody:
    "Discarding removes this unsynced draft and its outbox entry from local SQLite storage.",
  discardDraftConfirmBtn: "Discard permanently",
  promoteToTeamBtn: "Promote to Team",
  promoteDialogTitle: "Promote Solo Project to Team",
  promoteWorkspaceLabel: "Target workspace ID",
  promoteBlockedNotice: (n) =>
    `Promotion blocked: ${String(n)} active or non-terminal run(s) must complete or stop before transferring ownership.`,
  promoteSanitizeNotice:
    "Local repository paths are stripped before upload. After server acknowledgement, the team server becomes authoritative.",
  promoteConfirmBtn: "Promote project",
  runnerOwnershipHeading: "Runner Assignment & Fencing",
  runnerUnassignedLabel: "Unassigned · waiting for an online runner",
  runnerExclusiveOwner: (label, epoch) =>
    `Exclusive owner: ${label} · lease epoch #${String(epoch)}`,
  noAutoTakeoverNotice: "Automatic takeover of active runs is forbidden.",
  assignRunnerSelectLabel: "Select online runner",
  assignRunnerBtn: "Assign runner",
  noOnlineRunnersHint: "No online runners available in this workspace.",
  runnerUnreachableBadge: "Runner unreachable · no auto-takeover",
  cancelUnreachableBtn: "Cancel unreachable run",
  cancelUnreachableDialogTitle: "Cancel unreachable runner's run?",
  cancelUnreachableDialogBody: (label, epoch) =>
    `Runner ${label} (lease epoch #${String(epoch)}) is unreachable. This explicitly cancels the run on the server without silently transferring execution to another runner.`,
  cancelUnreachableReasonLabel: "Cancellation reason",
  cancelUnreachableReasonRequired: "Enter a reason before cancelling an unreachable run.",
  cancelUnreachableConfirmBtn: "Confirm cancel run",
  commentsHeading: (n) => `Append-only Comments (${String(n)})`,
  commentsEmpty: "No comments on this task yet.",
  commentsImmutableHint: "Comments are append-only and preserved in durable history.",
  commentBodyLabel: "Add comment",
  commentSubmitBtn: "Post comment",
  commentQueuedOfflineNotice:
    "Team server unreachable — comment saved locally as an offline draft and queued for sync.",
  commentDraftsHeading: "Pending / Conflicted Comment Drafts",
  saveTaskOfflineDraftBtn: "Save as offline draft",
  savePlanOfflineDraftBtn: "Save plan edit as offline draft",
  casInspectBtn: "Verify & inspect",
  casInspectingBtn: "Verifying SHA-256...",
  casVerifiedBadge: "SHA-256 verified",
  casMissingBadge: "Not synced locally · unknown",
  casCorruptedBadge: "Corrupted · SHA-256 mismatch",
  casTruncatedNotice: "Preview truncated to 32 KiB.",
  mobileAcceptanceHeading: "Mobile Bound Acceptance",
  mobileAcceptanceOfflineHint: "Team server offline — acceptance requires server confirmation.",
  mobileAcceptDialogTitle: "Confirm bound acceptance",
  mobileAcceptConfirmBtn: "Confirm accept",
  mobileRejectDialogTitle: "Request changes",
  mobileRevisionPlanLabel: (rev, planVer) => `rev ${String(rev)} · plan v${String(planVer)}`,
  formatAnnouncement: (a) => {
    if (!a) return "";
    switch (a.kind) {
      case "synced":
        return `Synced ${String(a.eventsApplied)} events, ${String(a.outboxAcked)} outbox acked`;
      case "logged_in":
        return `Connected as ${a.displayName}`;
      case "workspace_created":
        return `Created workspace ${a.name}`;
      case "role_updated":
        return `Updated ${a.userId} role to ${a.role}`;
      case "invitation_created":
        return `Created invitation for ${a.inviteeHint}`;
      case "invitation_revoked":
        return `Revoked invitation ${a.inviteId}`;
      case "invitation_accepted":
        return `Joined workspace ${a.workspaceId}`;
      case "project_promoted":
        return `Promoted ${a.projectId} to team workspace ${a.workspaceId}`;
      case "runner_registered":
        return `Registered runner ${a.name}`;
      case "runner_assigned":
        return `Assigned runner ${a.runnerId} (epoch ${String(a.leaseEpoch)})`;
      case "run_cancelled_unreachable":
        return `Cancelled unreachable run ${a.runId}`;
      case "comment_queued_offline":
        return "Comment queued as offline draft";
      case "comment_posted":
        return "Comment posted";
      case "draft_saved":
        return `Saved offline draft ${a.draftId}`;
      case "draft_synced":
        return `Draft ${a.draftId} synced`;
      case "draft_discarded":
        return `Discarded draft ${a.draftId}`;
    }
  },
  fieldRequiredError: "This field is required.",
  cancelBtn: "Cancel",
  roleLabel: (role) => {
    switch (role) {
      case "owner":
        return "Owner";
      case "maintainer":
        return "Maintainer";
      case "member":
        return "Member";
      case "viewer":
        return "Viewer";
    }
  },
  inviteStatusLabel: (status) => {
    switch (status) {
      case "pending":
        return "Pending";
      case "accepted":
        return "Accepted";
      case "revoked":
        return "Revoked";
      case "expired":
        return "Expired";
    }
  },
  runnerStatusLabel: (status) => {
    switch (status) {
      case "online":
        return "Online";
      case "unreachable":
        return "Unreachable";
      case "drained":
        return "Drained";
    }
  },
  draftKindLabel: (kind) => {
    switch (kind) {
      case "task_create":
        return "New task";
      case "task_comment":
        return "Comment";
      case "plan_edit":
        return "Plan edit";
      case "decision_intent":
        return "Decision intent";
    }
  },
  conflictCodeHint: (code, fallback) => {
    switch (code) {
      case "solo_to_team_blocked":
        return "Promotion blocked: finish or stop all active runs before promoting to Team.";
      case "active_run_prevents_takeover":
        return "Active run is exclusively owned by another runner; automatic takeover is forbidden.";
      case "revision_mismatch":
        return "Server revision advanced while offline; sync and rebase before retrying.";
      case "last_owner_protected":
        return "Cannot demote the last remaining workspace Owner.";
      case "runner_not_online":
        return "Only online runners can be assigned to a run.";
      case "evidence_required":
        return "Verified evidence bundle is required before recording acceptance.";
      default:
        return fallback;
    }
  },
};

export function getTeamStrings(locale: Locale): TeamStrings {
  return locale === "ru" ? TEAM_RU : TEAM_EN;
}
