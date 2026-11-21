import type { ReactNode } from "react";
import type { TeamStrings } from "../team-i18n";
import {
  isTeamManagedMode,
  type ProjectOwnershipMode,
  type TeamAnnouncement,
  type TeamConflictNotice,
  type TeamConnectionState,
} from "../team-types";
import { ActionButton } from "./ActionButton";
import { StatusBadge } from "./StatusBadge";

export interface TeamSyncStatusBarProps {
  readonly ownershipMode: ProjectOwnershipMode;
  readonly cursorSeq: number;
  readonly connectionState: TeamConnectionState;
  readonly unresolvedDraftsCount: number;
  readonly conflictCount: number;
  readonly conflictNotice: TeamConflictNotice | null;
  readonly announcement: TeamAnnouncement | null;
  readonly syncing: boolean;
  readonly strings: TeamStrings;
  readonly onSyncNow: () => void;
  readonly onOpenTeamQueue: () => void;
  readonly onDismissConflict?: () => void;
}

export function TeamSyncStatusBar({
  ownershipMode,
  cursorSeq,
  connectionState,
  unresolvedDraftsCount,
  conflictCount,
  conflictNotice,
  announcement,
  syncing,
  strings,
  onSyncNow,
  onOpenTeamQueue,
  onDismissConflict,
}: TeamSyncStatusBarProps): ReactNode {
  const isTeam = isTeamManagedMode(ownershipMode);
  const hasQueue = unresolvedDraftsCount > 0 || conflictCount > 0;
  const hasConflict =
    connectionState === "conflict" || conflictCount > 0 || conflictNotice !== null;
  const isOffline = connectionState === "offline";

  if (!isTeam && !hasQueue && !hasConflict && !isOffline) {
    return null;
  }

  const stripClass = hasConflict
    ? "rl-sync-strip rl-sync-strip-conflict"
    : isOffline
      ? "rl-sync-strip rl-sync-strip-offline"
      : "rl-sync-strip";

  const statusTone = hasConflict
    ? "danger"
    : isOffline
      ? "unknown"
      : connectionState === "reconnecting" || syncing
        ? "attention"
        : "success";

  const statusLabel = hasConflict
    ? strings.syncConflict
    : isOffline
      ? strings.syncOffline
      : connectionState === "reconnecting" || syncing
        ? strings.syncReconnecting
        : strings.syncOnline(cursorSeq);

  return (
    <section
      className={stripClass}
      aria-label={strings.teamHeading}
      data-testid="rl-team-sync-strip"
      data-connection-state={connectionState}
    >
      <div className="rl-sync-strip-group">
        <StatusBadge
          status="queued"
          customTone="neutral"
          customLabel={
            isTeam
              ? strings.ownershipTeam(ownershipMode.workspace_id, ownershipMode.ownership_epoch)
              : strings.ownershipSolo
          }
        />
        <StatusBadge
          status={isOffline ? "unknown" : "running"}
          customTone={statusTone}
          customLabel={statusLabel}
        />
        {hasQueue ? (
          <span
            className={
              conflictCount > 0
                ? "rl-badge rl-badge-danger rl-mono"
                : "rl-badge rl-badge-attention rl-mono"
            }
            data-testid="rl-sync-drafts-chip"
          >
            {strings.draftsSummaryChip(unresolvedDraftsCount, conflictCount)}
          </span>
        ) : null}
      </div>

      <div className="rl-sync-strip-group">
        {hasQueue ? (
          <ActionButton
            variant="secondary"
            label={strings.openTeamQueueBtn}
            onClick={onOpenTeamQueue}
            testId="rl-sync-open-queue-btn"
          />
        ) : null}
        <ActionButton
          variant={isOffline || hasConflict ? "primary" : "secondary"}
          label={isOffline ? strings.reconnectBtn : strings.syncNowBtn}
          disabled={syncing}
          onClick={onSyncNow}
          testId="rl-sync-now-btn"
        />
      </div>

      {isOffline ? (
        <p className="rl-status-line" style={{ flexBasis: "100%" }} data-testid="rl-offline-notice">
          {strings.offlineGateNotice}
        </p>
      ) : null}

      {conflictNotice ? (
        <div
          className="rl-error-box"
          role="alert"
          style={{ flexBasis: "100%" }}
          data-testid="rl-conflict-alert"
        >
          <span>
            {strings.conflictBannerTitle(conflictNotice.actualRevision)}{" "}
            {strings.conflictCodeHint(conflictNotice.serverCode, conflictNotice.message)}
          </span>
          {onDismissConflict ? (
            <button
              type="button"
              className="rl-breadcrumb-link"
              style={{ marginInlineStart: "0.75rem" }}
              onClick={onDismissConflict}
            >
              {strings.cancelBtn}
            </button>
          ) : null}
        </div>
      ) : null}

      <output className="rl-sr-only" aria-live="polite" data-testid="rl-team-live">
        {strings.formatAnnouncement(announcement)}
      </output>
    </section>
  );
}
