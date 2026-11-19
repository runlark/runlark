import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from "react";
import type { TeamStrings } from "../team-i18n";
import type { LocalRunnerStatusResponse, OfflineDraft, RunnerRegistration } from "../team-types";
import { ActionButton } from "./ActionButton";
import { StatusBadge } from "./StatusBadge";

export interface TeamRunnersDraftsPanelProps {
  readonly workspaceId: string | null;
  readonly runnerStatus: LocalRunnerStatusResponse | null;
  readonly runners: readonly RunnerRegistration[];
  readonly drafts: readonly OfflineDraft[];
  readonly busy: boolean;
  readonly strings: TeamStrings;
  readonly onRegisterRunner: (name: string) => void;
  readonly onRetryDraft: (
    draftId: string,
    input: { readonly expected_revision?: number; readonly payload_json?: string },
  ) => void;
  readonly onDeleteDraft: (draftId: string) => void;
}

export function TeamRunnersDraftsPanel({
  workspaceId,
  runnerStatus,
  runners,
  drafts,
  busy,
  strings,
  onRegisterRunner,
  onRetryDraft,
  onDeleteDraft,
}: TeamRunnersDraftsPanelProps): ReactNode {
  const [runnerName, setRunnerName] = useState("");
  const [runnerErr, setRunnerErr] = useState<string | null>(null);
  const [discardTarget, setDiscardTarget] = useState<OfflineDraft | null>(null);

  const runnerNameId = useId();
  const runnerErrId = useId();
  const discardHeadingId = useId();
  const dialogRef = useRef<HTMLDialogElement | null>(null);
  const cancelBtnRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (discardTarget !== null) {
      if (!dialog.open) {
        dialog.showModal();
      }
      cancelBtnRef.current?.focus();
    } else if (dialog.open) {
      dialog.close();
    }
  }, [discardTarget]);

  const handleRegister = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (!runnerName.trim()) {
      setRunnerErr(strings.fieldRequiredError);
      return;
    }
    setRunnerErr(null);
    onRegisterRunner(runnerName.trim());
    setRunnerName("");
  };

  return (
    <div className="rl-team-grid">
      <article className="rl-card" data-testid="rl-team-runners-card">
        <h2 className="rl-section-heading">{strings.runnersHeading(runners.length)}</h2>
        <p className="rl-status-line" data-testid="rl-local-runner-summary">
          {runnerStatus?.local_runner_id
            ? strings.localRunnerSummary(
                runnerStatus.local_runner_id,
                runnerStatus.local_runner_name ?? runnerStatus.local_runner_id,
                runnerStatus.buffered_events_count,
              )
            : strings.noLocalRunner}
        </p>

        {workspaceId ? (
          <form
            onSubmit={handleRegister}
            className="rl-actions-row"
            noValidate
            data-testid="rl-register-runner-form"
          >
            <div className="rl-field" style={{ flex: "1 1 14rem" }}>
              <label htmlFor={runnerNameId} className="rl-label">
                {strings.runnerNameLabel}
              </label>
              <input
                id={runnerNameId}
                type="text"
                className="rl-input"
                value={runnerName}
                disabled={busy}
                aria-invalid={runnerErr ? "true" : "false"}
                aria-describedby={runnerErr ? runnerErrId : undefined}
                onChange={(e) => {
                  setRunnerName(e.currentTarget.value);
                  if (runnerErr) setRunnerErr(null);
                }}
                data-testid="rl-register-runner-input"
              />
              {runnerErr ? (
                <div id={runnerErrId} className="rl-error-box" role="alert">
                  {runnerErr}
                </div>
              ) : null}
            </div>
            <ActionButton
              type="submit"
              variant="secondary"
              label={strings.registerRunnerBtn}
              disabled={busy}
              testId="rl-register-runner-submit"
            />
          </form>
        ) : null}

        {runners.length === 0 ? (
          <p className="rl-status-line">{strings.runnersEmpty}</p>
        ) : (
          <ul className="rl-team-list" data-testid="rl-runners-list">
            {runners.map((r) => {
              const tone =
                r.status === "online"
                  ? "success"
                  : r.status === "unreachable"
                    ? "unknown"
                    : "neutral";
              return (
                <li key={r.id} className="rl-team-row" data-testid={`rl-runner-row-${r.id}`}>
                  <div className="rl-team-row-info">
                    <span className="rl-comment-author">{r.name}</span>
                    <span className="rl-meta-value">
                      {r.id} · {r.owner_user_id}
                    </span>
                  </div>
                  <StatusBadge
                    status={r.status === "unreachable" ? "unknown" : "running"}
                    customTone={tone}
                    customLabel={strings.runnerStatusLabel(r.status)}
                  />
                </li>
              );
            })}
          </ul>
        )}
      </article>

      <article className="rl-card" data-testid="rl-team-drafts-card">
        <h2 className="rl-section-heading">{strings.draftsHeading(drafts.length)}</h2>
        {drafts.length === 0 ? (
          <p className="rl-status-line">{strings.draftsEmpty}</p>
        ) : (
          <ul className="rl-team-list" data-testid="rl-drafts-list">
            {drafts.map((draft) => (
              <DraftQueueRow
                key={`${draft.id}:${draft.sync_state.state}:${String(draft.updated_at)}`}
                draft={draft}
                busy={busy}
                strings={strings}
                onRetry={onRetryDraft}
                onRequestDiscard={setDiscardTarget}
              />
            ))}
          </ul>
        )}
      </article>

      <dialog
        ref={dialogRef}
        className="rl-dialog"
        aria-labelledby={discardHeadingId}
        onCancel={(e) => {
          e.preventDefault();
          if (!busy) setDiscardTarget(null);
        }}
        data-testid="rl-discard-draft-dialog"
      >
        <div className="rl-field" style={{ gap: "0.75rem" }}>
          <h2 id={discardHeadingId} className="rl-section-heading">
            {strings.discardDraftDialogTitle}
          </h2>
          <p className="rl-status-line">{strings.discardDraftDialogBody}</p>
          {discardTarget ? (
            <p className="rl-meta-value">
              {discardTarget.id} · {strings.draftKindLabel(discardTarget.kind)}
            </p>
          ) : null}
          <div className="rl-actions-row">
            <button
              ref={cancelBtnRef}
              type="button"
              className="rl-btn"
              disabled={busy}
              onClick={() => setDiscardTarget(null)}
              data-testid="rl-discard-draft-cancel"
            >
              {strings.cancelBtn}
            </button>
            <ActionButton
              variant="danger"
              label={strings.discardDraftConfirmBtn}
              disabled={busy || !discardTarget}
              onClick={() => {
                if (discardTarget) {
                  onDeleteDraft(discardTarget.id);
                  setDiscardTarget(null);
                }
              }}
              testId="rl-discard-draft-confirm"
            />
          </div>
        </div>
      </dialog>
    </div>
  );
}

function DraftQueueRow({
  draft,
  busy,
  strings,
  onRetry,
  onRequestDiscard,
}: {
  readonly draft: OfflineDraft;
  readonly busy: boolean;
  readonly strings: TeamStrings;
  readonly onRetry: (
    draftId: string,
    input: { readonly expected_revision?: number; readonly payload_json?: string },
  ) => void;
  readonly onRequestDiscard: (draft: OfflineDraft) => void;
}): ReactNode {
  const isConflict = draft.sync_state.state === "conflict";
  const suggestedRev =
    draft.sync_state.state === "conflict" && draft.sync_state.server_revision !== null
      ? String(draft.sync_state.server_revision)
      : draft.base_revision !== null
        ? String(draft.base_revision)
        : "";
  const [revInput, setRevInput] = useState(suggestedRev);
  const [payloadInput, setPayloadInput] = useState(draft.payload_json);
  const revId = useId();
  const payloadId = useId();

  const tone =
    draft.sync_state.state === "synced"
      ? "success"
      : isConflict
        ? "danger"
        : draft.sync_state.state === "pending_sync"
          ? "attention"
          : "neutral";

  const stateLabel =
    draft.sync_state.state === "synced"
      ? strings.draftStateSynced
      : draft.sync_state.state === "conflict"
        ? strings.draftStateConflict(draft.sync_state.server_revision)
        : draft.sync_state.state === "pending_sync"
          ? strings.draftStatePending
          : strings.draftStateLocal;

  const canRetry = !busy && draft.sync_state.state !== "synced" && draft.kind !== "decision_intent";

  return (
    <li
      className={isConflict ? "rl-draft-block rl-draft-block-conflict" : "rl-draft-block"}
      data-testid={`rl-draft-row-${draft.id}`}
      data-draft-state={draft.sync_state.state}
    >
      <div className="rl-artifact-head">
        <span className="rl-comment-author">
          {strings.draftKindLabel(draft.kind)}{" "}
          <span className="rl-meta-value">
            {draft.id}
            {draft.task_id ? ` · ${draft.task_id}` : ""}
          </span>
        </span>
        <StatusBadge status="queued" customTone={tone} customLabel={stateLabel} />
      </div>

      {draft.sync_state.state === "conflict" ? (
        <p className="rl-error-box" role="alert">
          {draft.sync_state.reason}
        </p>
      ) : null}

      {isConflict ? (
        <div className="rl-field" style={{ gap: "0.5rem" }}>
          <div className="rl-field">
            <label htmlFor={revId} className="rl-label">
              {strings.draftExpectedRevLabel}
            </label>
            <input
              id={revId}
              type="number"
              min={1}
              className="rl-input rl-mono"
              value={revInput}
              disabled={busy}
              onChange={(e) => setRevInput(e.currentTarget.value)}
              data-testid={`rl-draft-rev-${draft.id}`}
            />
          </div>
          <div className="rl-field">
            <label htmlFor={payloadId} className="rl-label">
              {strings.draftPayloadLabel}
            </label>
            <textarea
              id={payloadId}
              rows={3}
              className="rl-input rl-mono"
              value={payloadInput}
              disabled={busy}
              onChange={(e) => setPayloadInput(e.currentTarget.value)}
              data-testid={`rl-draft-payload-${draft.id}`}
            />
          </div>
        </div>
      ) : (
        <pre className="rl-artifact-preview">{draft.payload_json}</pre>
      )}

      <div className="rl-actions-row">
        {canRetry ? (
          <ActionButton
            variant="primary"
            label={strings.draftRetryBtn}
            disabled={busy}
            onClick={() => {
              const parsedRev = revInput.trim() ? Number.parseInt(revInput.trim(), 10) : Number.NaN;
              onRetry(draft.id, {
                ...(Number.isFinite(parsedRev) && parsedRev > 0
                  ? { expected_revision: parsedRev }
                  : {}),
                ...(payloadInput !== draft.payload_json ? { payload_json: payloadInput } : {}),
              });
            }}
            testId={`rl-draft-retry-${draft.id}`}
          />
        ) : null}
        {draft.sync_state.state !== "synced" ? (
          <ActionButton
            variant="danger"
            label={strings.draftDiscardBtn}
            disabled={busy}
            onClick={() => onRequestDiscard(draft)}
            testId={`rl-draft-discard-${draft.id}`}
          />
        ) : null}
      </div>
    </li>
  );
}
