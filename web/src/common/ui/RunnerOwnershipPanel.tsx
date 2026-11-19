import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from "react";
import { normalizeRunStatusValue, type RunRecord } from "../board-types";
import type { TeamStrings } from "../team-i18n";
import type { RunOwnership, RunnerRegistration } from "../team-types";
import { ActionButton } from "./ActionButton";
import { StatusBadge } from "./StatusBadge";

export interface RunnerOwnershipPanelProps {
  readonly run: RunRecord | null;
  readonly ownership: RunOwnership | null;
  readonly runners: readonly RunnerRegistration[];
  readonly offlineBlocked: boolean;
  readonly busy: boolean;
  readonly strings: TeamStrings;
  readonly onAssignRunner: (runId: string, runnerId: string, expectedRevision: number) => void;
  readonly onCancelUnreachable: (runId: string, reason: string, expectedRevision: number) => void;
}

export function RunnerOwnershipPanel({
  run,
  ownership,
  runners,
  offlineBlocked,
  busy,
  strings,
  onAssignRunner,
  onCancelUnreachable,
}: RunnerOwnershipPanelProps): ReactNode {
  const onlineRunners = runners.filter((r) => r.status === "online");
  const [selectedRunnerId, setSelectedRunnerId] = useState(onlineRunners[0]?.id ?? "");
  const [cancelOpen, setCancelOpen] = useState(false);
  const [reason, setReason] = useState("");
  const [reasonError, setReasonError] = useState<string | null>(null);

  const selectId = useId();
  const dialogHeadingId = useId();
  const reasonId = useId();
  const reasonErrId = useId();
  const dialogRef = useRef<HTMLDialogElement | null>(null);
  const cancelBtnRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    if (!selectedRunnerId && onlineRunners[0]) {
      setSelectedRunnerId(onlineRunners[0].id);
    }
  }, [selectedRunnerId, onlineRunners]);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (cancelOpen) {
      if (!dialog.open) {
        dialog.showModal();
      }
      cancelBtnRef.current?.focus();
    } else if (dialog.open) {
      dialog.close();
    }
  }, [cancelOpen]);

  if (!run) {
    return null;
  }

  const runStatus = normalizeRunStatusValue(run.status);
  const isTerminal = runStatus === "completed" || runStatus === "stopped" || runStatus === "failed";
  const ownerId = ownership?.assigned_runner_id ?? null;
  const leaseEpoch = ownership?.lease_epoch ?? 0;
  const unreachable = ownership?.runner_unreachable === true;
  const ownerReg = ownerId ? runners.find((r) => r.id === ownerId) : undefined;
  const ownerLabel = ownerReg ? `${ownerReg.name} (${ownerReg.id})` : (ownerId ?? "-");

  const effectiveRunnerId = onlineRunners.some((r) => r.id === selectedRunnerId)
    ? selectedRunnerId
    : (onlineRunners[0]?.id ?? "");
  const canAssign =
    !busy && !offlineBlocked && !isTerminal && ownerId === null && effectiveRunnerId.length > 0;
  const canCancelUnreachable =
    !busy && !offlineBlocked && !isTerminal && ownerId !== null && unreachable;

  const handleCancelSubmit = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (!reason.trim()) {
      setReasonError(strings.cancelUnreachableReasonRequired);
      return;
    }
    setReasonError(null);
    onCancelUnreachable(run.id, reason.trim(), run.revision);
    setReason("");
    setCancelOpen(false);
  };

  return (
    <article className="rl-card" data-testid="rl-runner-ownership-panel">
      <header className="rl-card-header">
        <h3 className="rl-section-heading">{strings.runnerOwnershipHeading}</h3>
        {unreachable ? (
          <StatusBadge
            status="unknown"
            customTone="unknown"
            customLabel={strings.runnerUnreachableBadge}
          />
        ) : ownerId ? (
          <StatusBadge
            status="running"
            customTone={ownerReg?.status === "online" ? "success" : "neutral"}
            customLabel={strings.runnerExclusiveOwner(ownerLabel, leaseEpoch)}
          />
        ) : (
          <StatusBadge
            status="queued"
            customTone="neutral"
            customLabel={strings.runnerUnassignedLabel}
          />
        )}
      </header>

      {ownerId ? (
        <div className="rl-field" style={{ gap: "0.375rem" }}>
          <p className="rl-status-line" data-testid="rl-runner-exclusive-notice">
            {strings.runnerExclusiveOwner(ownerLabel, leaseEpoch)} · {strings.noAutoTakeoverNotice}
          </p>
          {canCancelUnreachable ? (
            <div className="rl-actions-row">
              <ActionButton
                variant="danger"
                label={strings.cancelUnreachableBtn}
                disabled={busy || offlineBlocked}
                onClick={() => {
                  setReasonError(null);
                  setCancelOpen(true);
                }}
                testId="rl-cancel-unreachable-open-btn"
              />
            </div>
          ) : null}
        </div>
      ) : !isTerminal ? (
        <div className="rl-field" style={{ gap: "0.5rem" }}>
          {onlineRunners.length === 0 ? (
            <p className="rl-status-line">{strings.noOnlineRunnersHint}</p>
          ) : (
            <div className="rl-actions-row">
              <label htmlFor={selectId} className="rl-sr-only">
                {strings.assignRunnerSelectLabel}
              </label>
              <select
                id={selectId}
                className="rl-toolbar-select"
                value={effectiveRunnerId}
                disabled={busy || offlineBlocked}
                onChange={(e) => setSelectedRunnerId(e.currentTarget.value)}
                data-testid="rl-assign-runner-select"
              >
                {onlineRunners.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.name} ({r.id})
                  </option>
                ))}
              </select>
              <ActionButton
                variant="primary"
                label={strings.assignRunnerBtn}
                disabled={!canAssign}
                onClick={() => onAssignRunner(run.id, effectiveRunnerId, run.revision)}
                testId="rl-assign-runner-btn"
              />
            </div>
          )}
        </div>
      ) : null}

      <dialog
        ref={dialogRef}
        className="rl-dialog"
        aria-labelledby={dialogHeadingId}
        onCancel={(e) => {
          e.preventDefault();
          if (!busy) setCancelOpen(false);
        }}
        data-testid="rl-cancel-unreachable-dialog"
      >
        <form
          onSubmit={handleCancelSubmit}
          className="rl-field"
          style={{ gap: "0.75rem" }}
          noValidate
        >
          <h2 id={dialogHeadingId} className="rl-section-heading">
            {strings.cancelUnreachableDialogTitle}
          </h2>
          <p className="rl-status-line">
            {strings.cancelUnreachableDialogBody(ownerLabel, leaseEpoch)}
          </p>
          <div className="rl-field">
            <label htmlFor={reasonId} className="rl-label">
              {strings.cancelUnreachableReasonLabel}
            </label>
            <input
              id={reasonId}
              type="text"
              className="rl-input"
              value={reason}
              disabled={busy}
              aria-invalid={reasonError ? "true" : "false"}
              aria-describedby={reasonError ? reasonErrId : undefined}
              onChange={(e) => {
                setReason(e.currentTarget.value);
                if (reasonError) setReasonError(null);
              }}
              data-testid="rl-cancel-unreachable-reason"
            />
          </div>
          {reasonError ? (
            <div id={reasonErrId} className="rl-error-box" role="alert">
              {reasonError}
            </div>
          ) : null}
          <div className="rl-actions-row">
            <button
              ref={cancelBtnRef}
              type="button"
              className="rl-btn"
              disabled={busy}
              onClick={() => setCancelOpen(false)}
              data-testid="rl-cancel-unreachable-dismiss"
            >
              {strings.cancelBtn}
            </button>
            <ActionButton
              type="submit"
              variant="danger"
              label={strings.cancelUnreachableConfirmBtn}
              disabled={busy}
              testId="rl-cancel-unreachable-confirm"
            />
          </div>
        </form>
      </dialog>
    </article>
  );
}
