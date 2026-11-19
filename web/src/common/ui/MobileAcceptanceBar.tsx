import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from "react";
import { formatMicroUsd, type BoardTaskEntry } from "../board-types";
import type { Locale, RunlarkStrings } from "../i18n";
import type { TeamStrings } from "../team-i18n";
import type { VerifiedArtifactInspection } from "../team-types";
import { ActionButton } from "./ActionButton";
import { StatusBadge } from "./StatusBadge";
import type { AcceptTaskBoundInput } from "./TaskDetailPanels";

export interface MobileAcceptanceBarProps {
  readonly detail: BoardTaskEntry;
  readonly locale: Locale;
  readonly strings: RunlarkStrings;
  readonly teamStrings: TeamStrings;
  readonly evidenceInspection: VerifiedArtifactInspection | undefined;
  readonly offlineBlocked: boolean;
  readonly busy: boolean;
  readonly onInspectEvidence: (digest: string) => void;
  readonly onAccept: (input: AcceptTaskBoundInput) => void;
  readonly onReject: (reason: string) => void;
}

export function MobileAcceptanceBar({
  detail,
  locale,
  strings,
  teamStrings,
  evidenceInspection,
  offlineBlocked,
  busy,
  onInspectEvidence,
  onAccept,
  onReject,
}: MobileAcceptanceBarProps): ReactNode {
  const { task, latest_run: run, artifacts, cost_summary: cost } = detail;
  const [mode, setMode] = useState<"accept" | "reject" | null>(null);
  const [comment, setComment] = useState("");
  const [rejectReason, setRejectReason] = useState("");
  const [rejectErr, setRejectErr] = useState<string | null>(null);

  const dialogRef = useRef<HTMLDialogElement | null>(null);
  const cancelBtnRef = useRef<HTMLButtonElement | null>(null);
  const headingId = useId();
  const commentId = useId();
  const rejectId = useId();
  const rejectErrId = useId();

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (mode !== null) {
      if (!dialog.open) {
        dialog.showModal();
      }
      cancelBtnRef.current?.focus();
    } else if (dialog.open) {
      dialog.close();
    }
  }, [mode]);

  if (task.stage !== "acceptance") {
    return null;
  }

  const evidenceArtifact = [...artifacts].reverse().find((a) => a.label === "evidence_bundle");
  const evidenceCorrupted = evidenceInspection?.status === "corrupted";
  const canAccept =
    !busy && !offlineBlocked && !evidenceCorrupted && evidenceArtifact !== undefined;
  const canReject = !busy && !offlineBlocked;

  const handleConfirmAccept = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (!canAccept) return;
    onAccept({
      ...(comment.trim() ? { comment: comment.trim() } : {}),
      expected_revision: task.revision,
      plan_version: task.plan_version,
      ...(run?.head_sha ? { head_sha: run.head_sha } : {}),
      ...(run?.base_sha ? { base_sha: run.base_sha } : {}),
      ...(run?.policy_digest ? { policy_digest: run.policy_digest } : {}),
      ...(evidenceArtifact ? { evidence_digest: evidenceArtifact.digest_sha256 } : {}),
    });
    setComment("");
    setMode(null);
  };

  const handleConfirmReject = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (!canReject) return;
    if (!rejectReason.trim()) {
      setRejectErr(strings.rejectReasonRequired);
      return;
    }
    setRejectErr(null);
    onReject(rejectReason.trim());
    setRejectReason("");
    setMode(null);
  };

  return (
    <section
      className="rl-mobile-acceptance-bar"
      aria-label={teamStrings.mobileAcceptanceHeading}
      data-testid="rl-mobile-acceptance-bar"
    >
      <div className="rl-artifact-head">
        <span className="rl-comment-author">{teamStrings.mobileAcceptanceHeading}</span>
        <span className="rl-figure">
          v{String(task.plan_version)} · {formatMicroUsd(cost.known_micro_usd, locale)}
        </span>
      </div>

      <div className="rl-sync-strip-group">
        <span className="rl-meta-value">
          {run?.base_sha?.slice(0, 8) ?? "-"}..{run?.head_sha?.slice(0, 8) ?? "-"}
        </span>
        {evidenceArtifact ? (
          <StatusBadge
            status={evidenceInspection?.status === "corrupted" ? "failed" : "completed"}
            customTone={
              evidenceInspection?.status === "verified"
                ? "success"
                : evidenceInspection?.status === "corrupted"
                  ? "danger"
                  : "attention"
            }
            customLabel={
              evidenceInspection?.status === "verified"
                ? teamStrings.casVerifiedBadge
                : evidenceInspection?.status === "corrupted"
                  ? teamStrings.casCorruptedBadge
                  : `CAS ${evidenceArtifact.digest_sha256.slice(0, 10)}`
            }
          />
        ) : null}
        {evidenceArtifact && evidenceInspection?.status !== "verified" ? (
          <ActionButton
            variant="secondary"
            label={teamStrings.casInspectBtn}
            onClick={() => onInspectEvidence(evidenceArtifact.digest_sha256)}
            testId="rl-mobile-inspect-evidence"
          />
        ) : null}
        {cost.unknown_attempts > 0 ? (
          <StatusBadge
            status="unknown"
            customTone="unknown"
            customLabel={strings.unknownCostAttempts(cost.unknown_attempts)}
          />
        ) : null}
      </div>

      {offlineBlocked ? (
        <p className="rl-status-line" data-testid="rl-mobile-acceptance-offline">
          {teamStrings.mobileAcceptanceOfflineHint}
        </p>
      ) : null}

      <div className="rl-mobile-acceptance-actions">
        <ActionButton
          variant="primary"
          label={strings.acceptBtn}
          disabled={!canAccept}
          onClick={() => setMode("accept")}
          testId="rl-mobile-accept-open"
        />
        <ActionButton
          variant="danger"
          label={strings.rejectBtn}
          disabled={!canReject}
          onClick={() => {
            setRejectErr(null);
            setMode("reject");
          }}
          testId="rl-mobile-reject-open"
        />
      </div>

      <dialog
        ref={dialogRef}
        className="rl-dialog"
        aria-labelledby={headingId}
        onCancel={(e) => {
          e.preventDefault();
          if (!busy) setMode(null);
        }}
        data-testid="rl-mobile-acceptance-dialog"
      >
        {mode === "accept" ? (
          <form onSubmit={handleConfirmAccept} className="rl-field" style={{ gap: "0.75rem" }}>
            <h2 id={headingId} className="rl-section-heading">
              {teamStrings.mobileAcceptDialogTitle}
            </h2>
            <p className="rl-meta-value">
              {teamStrings.mobileRevisionPlanLabel(task.revision, task.plan_version)} ·{" "}
              {run?.head_sha?.slice(0, 8) ?? "-"} ·{" "}
              {evidenceArtifact?.digest_sha256.slice(0, 12) ?? "-"}
            </p>
            <div className="rl-field">
              <label htmlFor={commentId} className="rl-label">
                {strings.acceptCommentLabel}
              </label>
              <input
                id={commentId}
                type="text"
                className="rl-input"
                value={comment}
                disabled={busy}
                onChange={(e) => setComment(e.currentTarget.value)}
                data-testid="rl-mobile-accept-comment"
              />
            </div>
            <div className="rl-actions-row">
              <button
                ref={cancelBtnRef}
                type="button"
                className="rl-btn"
                disabled={busy}
                onClick={() => setMode(null)}
              >
                {teamStrings.cancelBtn}
              </button>
              <ActionButton
                type="submit"
                variant="primary"
                label={teamStrings.mobileAcceptConfirmBtn}
                disabled={!canAccept}
                testId="rl-mobile-accept-confirm"
              />
            </div>
          </form>
        ) : (
          <form
            onSubmit={handleConfirmReject}
            className="rl-field"
            style={{ gap: "0.75rem" }}
            noValidate
          >
            <h2 id={headingId} className="rl-section-heading">
              {teamStrings.mobileRejectDialogTitle}
            </h2>
            <div className="rl-field">
              <label htmlFor={rejectId} className="rl-label">
                {strings.rejectReasonLabel}
              </label>
              <input
                id={rejectId}
                type="text"
                className="rl-input"
                value={rejectReason}
                disabled={busy}
                aria-invalid={rejectErr ? "true" : "false"}
                aria-describedby={rejectErr ? rejectErrId : undefined}
                onChange={(e) => {
                  setRejectReason(e.currentTarget.value);
                  if (rejectErr) setRejectErr(null);
                }}
                data-testid="rl-mobile-reject-reason"
              />
            </div>
            {rejectErr ? (
              <div id={rejectErrId} className="rl-error-box" role="alert">
                {rejectErr}
              </div>
            ) : null}
            <div className="rl-actions-row">
              <button
                ref={cancelBtnRef}
                type="button"
                className="rl-btn"
                disabled={busy}
                onClick={() => setMode(null)}
              >
                {teamStrings.cancelBtn}
              </button>
              <ActionButton
                type="submit"
                variant="danger"
                label={strings.rejectBtn}
                disabled={!canReject}
                testId="rl-mobile-reject-confirm"
              />
            </div>
          </form>
        )}
      </dialog>
    </section>
  );
}
