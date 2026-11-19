import { useId, useState, type FormEvent, type ReactNode } from "react";
import {
  formatMicroUsd,
  localizedRunStatusLabel,
  normalizeRunStatusValue,
  type BoardTaskEntry,
  type ChatDispatchResult,
  type RunDiffResponse,
  type RunHistoryResponse,
} from "../board-types";
import type { DaemonEventItem } from "../runlark-types";
import type { Locale, RunlarkStrings } from "../i18n";
import { ActionButton } from "./ActionButton";
import { EventFeedList } from "./EventFeedList";
import { StatusBadge } from "./StatusBadge";

export interface RunBudgetTabProps {
  readonly detail: BoardTaskEntry;
  readonly history: RunHistoryResponse | null;
  readonly strings: RunlarkStrings;
  readonly locale: Locale;
  readonly actionPending: string | null;
  readonly offlineBlocked?: boolean;
  readonly onStopRun: () => void;
  readonly onResumeRun: () => void;
  readonly onVerifyRun: () => void;
}

export function RunBudgetTab({
  detail,
  history,
  strings,
  locale,
  actionPending,
  offlineBlocked = false,
  onStopRun,
  onResumeRun,
  onVerifyRun,
}: RunBudgetTabProps): ReactNode {
  const run = history?.run ?? detail.latest_run;
  const attempts = history?.attempts ?? detail.attempts;
  const cost = history?.cost_summary ?? detail.cost_summary;
  const runStatus = normalizeRunStatusValue(run?.status);
  const busy = actionPending !== null;
  const stopHintId = useId();
  const resumeHintId = useId();
  const verifyHintId = useId();
  const stopDisabled =
    busy || offlineBlocked || (runStatus !== "running" && runStatus !== "queued");
  const resumeDisabled =
    busy || offlineBlocked || runStatus === "completed" || runStatus === "stopped";
  const verifyDisabled = busy || offlineBlocked || runStatus === "queued";

  return (
    <div className="rl-field" style={{ gap: "0.75rem" }}>
      <article className="rl-card">
        <header className="rl-card-header">
          <h2 className="rl-section-heading">{strings.taskTabRunBudget}</h2>
          {run ? (
            <StatusBadge
              status={runStatus}
              customLabel={localizedRunStatusLabel(runStatus, strings)}
            />
          ) : null}
        </header>

        <dl className="rl-meta-grid">
          <div className="rl-meta-item">
            <dt className="rl-meta-label">{strings.knownCostLabel}</dt>
            <dd className="rl-meta-value rl-figure">
              {formatMicroUsd(cost.known_micro_usd, locale)}
            </dd>
          </div>
          <div className="rl-meta-item">
            <dt className="rl-meta-label">{strings.heldCostLabel}</dt>
            <dd className="rl-meta-value rl-figure">
              {formatMicroUsd(cost.held_micro_usd, locale)}
            </dd>
          </div>
          <div className="rl-meta-item">
            <dt className="rl-meta-label">{strings.unknownCostLabel}</dt>
            <dd className="rl-meta-value">
              {cost.unknown_attempts > 0 ? (
                <StatusBadge
                  status="unknown"
                  customTone="unknown"
                  customLabel={strings.unknownCostAttempts(cost.unknown_attempts)}
                />
              ) : (
                "0"
              )}
            </dd>
          </div>
          <div className="rl-meta-item">
            <dt className="rl-meta-label">{strings.repairAttemptsLabel}</dt>
            <dd className="rl-meta-value rl-figure">
              {run
                ? `${String(run.repair_attempts)} / ${String(run.budget_policy.max_attempts)}`
                : "-"}
            </dd>
          </div>
          <div className="rl-meta-item rl-meta-item-wide">
            <dt className="rl-meta-label">{strings.worktreeDigestLabel}</dt>
            <dd className="rl-meta-value">
              {run ? `${run.worktree_path || "-"} · ${run.workflow_digest ?? "-"}` : "-"}
            </dd>
          </div>
        </dl>

        {run ? (
          <div className="rl-field">
            <div className="rl-actions-row">
              <ActionButton
                variant="danger"
                label={strings.stopRunBtn}
                disabled={stopDisabled}
                disabledReasonId={stopHintId}
                onClick={onStopRun}
              />
              <ActionButton
                variant="secondary"
                label={strings.resumeRunBtn}
                disabled={resumeDisabled}
                disabledReasonId={resumeHintId}
                onClick={onResumeRun}
              />
              <ActionButton
                variant="primary"
                label={strings.verifyRunBtn}
                disabled={verifyDisabled}
                disabledReasonId={verifyHintId}
                onClick={onVerifyRun}
                testId="rl-task-verify-run"
              />
            </div>
            {stopDisabled && !busy ? (
              <span id={stopHintId} className="rl-meta-label">
                {strings.stopDisabledHint}
              </span>
            ) : null}
            {resumeDisabled && !busy ? (
              <span id={resumeHintId} className="rl-meta-label">
                {strings.resumeDisabledHint}
              </span>
            ) : null}
            {verifyDisabled && !busy ? (
              <span id={verifyHintId} className="rl-meta-label">
                {strings.verifyDisabledHint}
              </span>
            ) : null}
          </div>
        ) : null}
      </article>

      <article className="rl-card">
        <h3 className="rl-section-heading">{strings.attemptsHeading(attempts.length)}</h3>
        {attempts.length === 0 ? (
          <p className="rl-status-line">{strings.attemptsEmpty}</p>
        ) : (
          <ul className="rl-event-list">
            {attempts.map((att) => (
              <li key={att.id} className="rl-event-row">
                <span className="rl-event-seq">
                  #{String(att.attempt_no ?? att.attempt_number ?? 0)}
                </span>
                <span className="rl-event-type">{att.outcome.status}</span>
                <span className="rl-figure">
                  {att.cost_micro_usd === null
                    ? strings.unknownCostLabel
                    : formatMicroUsd(att.cost_micro_usd, locale)}
                </span>
                {att.unknown_cost_reason ? (
                  <p className="rl-event-detail">{att.unknown_cost_reason}</p>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </article>
    </div>
  );
}

export function DiffArtifactsTab({
  detail,
  diff,
  strings,
  artifactsSlot,
}: {
  readonly detail: BoardTaskEntry;
  readonly diff: RunDiffResponse | null;
  readonly strings: RunlarkStrings;
  readonly artifactsSlot?: ReactNode;
}): ReactNode {
  return (
    <div className="rl-field" style={{ gap: "0.75rem" }}>
      <article className="rl-card">
        <header className="rl-card-header">
          <h2 className="rl-section-heading">{strings.gitDiffHeading}</h2>
          {diff ? (
            <span className="rl-meta-value">
              {diff.base_sha.slice(0, 8)}..{diff.head_sha.slice(0, 8)} · {String(diff.diff_bytes)} B
            </span>
          ) : null}
        </header>
        {!diff || !diff.diff ? (
          <p className="rl-status-line">{strings.diffEmpty}</p>
        ) : (
          <pre className="rl-diff-pre" data-testid="rl-run-diff-pre">
            {diff.diff}
          </pre>
        )}
      </article>

      {artifactsSlot ?? (
        <article className="rl-card">
          <h3 className="rl-section-heading">
            {strings.casArtifactsHeading(detail.artifacts.length)}
          </h3>
          {detail.artifacts.length === 0 ? (
            <p className="rl-status-line">{strings.artifactsEmpty}</p>
          ) : (
            <ul className="rl-event-list">
              {detail.artifacts.map((art) => (
                <li key={`${art.label}-${art.digest_sha256}`} className="rl-event-row">
                  <span className="rl-event-type">{art.label}</span>
                  <span className="rl-figure">{String(art.size_bytes)} B</span>
                  <p className="rl-event-detail rl-mono">{art.digest_sha256}</p>
                </li>
              ))}
            </ul>
          )}
        </article>
      )}
    </div>
  );
}

export interface AcceptTaskBoundInput {
  readonly comment?: string;
  readonly expected_revision: number;
  readonly plan_version: number;
  readonly head_sha?: string;
  readonly base_sha?: string;
  readonly policy_digest?: string;
  readonly evidence_digest?: string;
}

export interface AcceptancePrTabProps {
  readonly detail: BoardTaskEntry;
  readonly strings: RunlarkStrings;
  readonly actionPending: string | null;
  readonly prUnknownReason: string | null;
  readonly offlineBlockedReason?: string | null;
  readonly onAccept: (input: AcceptTaskBoundInput) => void;
  readonly onReject: (reason: string) => void;
  readonly onCreatePr: (input: {
    readonly title?: string;
    readonly base_branch?: string;
    readonly draft: boolean;
  }) => void;
}

export function AcceptancePrTab({
  detail,
  strings,
  actionPending,
  prUnknownReason,
  offlineBlockedReason = null,
  onAccept,
  onReject,
  onCreatePr,
}: AcceptancePrTabProps): ReactNode {
  const { task, latest_run: run, decisions, artifacts } = detail;
  const [acceptComment, setAcceptComment] = useState("");
  const [rejectReason, setRejectReason] = useState("");
  const [rejectError, setRejectError] = useState<string | null>(null);
  const [prTitle, setPrTitle] = useState(task.title);
  const [prBase, setPrBase] = useState("main");
  const [prDraft, setPrDraft] = useState(false);

  const acceptInputId = useId();
  const acceptHintId = useId();
  const rejectInputId = useId();
  const rejectErrId = useId();
  const prTitleId = useId();
  const prBaseId = useId();
  const prDraftId = useId();
  const prHintId = useId();

  const busy = actionPending !== null;
  const evidenceArtifact = [...artifacts].reverse().find((a) => a.label === "evidence_bundle");
  const hasEvidence = evidenceArtifact !== undefined;
  const offlineBlocked = offlineBlockedReason !== null;
  const canAccept = !busy && !offlineBlocked && task.stage === "acceptance" && hasEvidence;
  const hasAcceptDecision = decisions.some((d) => d.kind === "accept");
  const canCreatePr = !busy && !offlineBlocked && hasAcceptDecision && task.stage === "done";

  const handleRejectSubmit = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (busy || offlineBlocked || task.stage !== "acceptance") return;
    if (!rejectReason.trim()) {
      setRejectError(strings.rejectReasonRequired);
      return;
    }
    setRejectError(null);
    onReject(rejectReason.trim());
    setRejectReason("");
  };

  return (
    <div className="rl-field" style={{ gap: "0.75rem" }}>
      <article className="rl-card rl-card-decision">
        <h2 className="rl-section-heading">{strings.acceptHeading}</h2>
        <dl className="rl-meta-grid">
          <div className="rl-meta-item">
            <dt className="rl-meta-label">{strings.planVersionLabel}</dt>
            <dd className="rl-meta-value">v{String(task.plan_version)}</dd>
          </div>
          <div className="rl-meta-item">
            <dt className="rl-meta-label">{strings.headBaseShaLabel}</dt>
            <dd className="rl-meta-value">
              {run?.head_sha?.slice(0, 8) ?? "-"} / {run?.base_sha?.slice(0, 8) ?? "-"}
            </dd>
          </div>
        </dl>

        <div className="rl-field">
          <label htmlFor={acceptInputId} className="rl-label">
            {strings.acceptCommentLabel}
          </label>
          <input
            id={acceptInputId}
            type="text"
            className="rl-input"
            value={acceptComment}
            disabled={busy}
            onChange={(e) => setAcceptComment(e.currentTarget.value)}
          />
        </div>

        <div className="rl-actions-row">
          <ActionButton
            variant="primary"
            label={strings.acceptBtn}
            disabled={!canAccept}
            disabledReasonId={acceptHintId}
            onClick={() =>
              onAccept({
                ...(acceptComment.trim() ? { comment: acceptComment.trim() } : {}),
                expected_revision: task.revision,
                plan_version: task.plan_version,
                ...(run?.head_sha ? { head_sha: run.head_sha } : {}),
                ...(run?.base_sha ? { base_sha: run.base_sha } : {}),
                ...(run?.policy_digest ? { policy_digest: run.policy_digest } : {}),
                ...(evidenceArtifact ? { evidence_digest: evidenceArtifact.digest_sha256 } : {}),
              })
            }
            testId="rl-task-accept-btn"
          />
          {!canAccept ? (
            <span id={acceptHintId} className="rl-meta-label">
              {offlineBlockedReason ?? strings.acceptDisabledHint}
            </span>
          ) : null}
        </div>

        <form onSubmit={handleRejectSubmit} className="rl-field" noValidate>
          <label htmlFor={rejectInputId} className="rl-label">
            {strings.rejectReasonLabel}
          </label>
          <input
            id={rejectInputId}
            type="text"
            className="rl-input"
            value={rejectReason}
            disabled={busy || offlineBlocked || task.stage !== "acceptance"}
            aria-invalid={rejectError ? "true" : undefined}
            aria-describedby={rejectError ? rejectErrId : undefined}
            onChange={(e) => {
              setRejectReason(e.currentTarget.value);
              if (rejectError) setRejectError(null);
            }}
          />
          {rejectError ? (
            <div id={rejectErrId} className="rl-error-box" role="alert">
              {rejectError}
            </div>
          ) : null}
          <div className="rl-actions-row">
            <ActionButton
              type="submit"
              variant="danger"
              label={strings.rejectBtn}
              disabled={busy || offlineBlocked || task.stage !== "acceptance"}
              testId="rl-task-reject-btn"
            />
          </div>
        </form>
      </article>

      <article className="rl-card">
        <h3 className="rl-section-heading">{strings.createPrHeading}</h3>
        {task.pr_url ? (
          <p className="rl-meta-value" data-testid="rl-task-pr-url">
            {task.pr_url}
          </p>
        ) : null}
        {prUnknownReason ? (
          <div className="rl-badge rl-badge-unknown">
            <span>{strings.prUnknownNotice}</span>
          </div>
        ) : null}

        <div className="rl-field">
          <label htmlFor={prTitleId} className="rl-label">
            {strings.prTitleLabel}
          </label>
          <input
            id={prTitleId}
            type="text"
            className="rl-input"
            value={prTitle}
            disabled={busy}
            onChange={(e) => setPrTitle(e.currentTarget.value)}
          />
        </div>

        <div className="rl-field">
          <label htmlFor={prBaseId} className="rl-label">
            {strings.prBaseBranchLabel}
          </label>
          <input
            id={prBaseId}
            type="text"
            className="rl-input"
            value={prBase}
            disabled={busy}
            onChange={(e) => setPrBase(e.currentTarget.value)}
          />
        </div>

        <label htmlFor={prDraftId} className="rl-option-label">
          <input
            id={prDraftId}
            type="checkbox"
            checked={prDraft}
            disabled={busy}
            onChange={(e) => setPrDraft(e.currentTarget.checked)}
          />
          <span>{strings.prDraftLabel}</span>
        </label>

        <div className="rl-actions-row">
          <ActionButton
            variant="primary"
            label={strings.createPrBtn}
            disabled={!canCreatePr}
            disabledReasonId={prHintId}
            onClick={() =>
              onCreatePr({
                ...(prTitle.trim() ? { title: prTitle.trim() } : {}),
                ...(prBase.trim() ? { base_branch: prBase.trim() } : {}),
                draft: prDraft,
              })
            }
            testId="rl-task-create-pr-btn"
          />
          {!canCreatePr ? (
            <span id={prHintId} className="rl-meta-label">
              {strings.prDisabledHint}
            </span>
          ) : null}
        </div>
      </article>

      <article className="rl-card">
        <h3 className="rl-section-heading">{strings.decisionsHeading(decisions.length)}</h3>
        {decisions.length === 0 ? (
          <p className="rl-status-line">{strings.decisionsEmpty}</p>
        ) : (
          <ul className="rl-event-list">
            {decisions.map((dec) => (
              <li key={dec.id} className="rl-event-row">
                <span className="rl-event-type">{dec.kind}</span>
                <span className="rl-meta-value">{dec.actor}</span>
                {dec.comment ? <p className="rl-event-detail">{dec.comment}</p> : null}
              </li>
            ))}
          </ul>
        )}
      </article>
    </div>
  );
}

export function EventsChatTab({
  events,
  loading,
  chatLog,
  strings,
  actionPending,
  onDispatchChat,
}: {
  readonly events: readonly DaemonEventItem[];
  readonly loading: boolean;
  readonly chatLog: readonly ChatDispatchResult[];
  readonly strings: RunlarkStrings;
  readonly actionPending: string | null;
  readonly onDispatchChat: (input: string) => void;
}): ReactNode {
  const [commandInput, setCommandInput] = useState("");
  const inputId = useId();
  const busy = actionPending !== null;

  const handleChat = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (!commandInput.trim()) return;
    onDispatchChat(commandInput.trim());
    setCommandInput("");
  };

  return (
    <div className="rl-field" style={{ gap: "0.75rem" }}>
      <article className="rl-card">
        <form onSubmit={handleChat} className="rl-field">
          <label htmlFor={inputId} className="rl-label">
            {strings.chatCommandLabel}
          </label>
          <div className="rl-actions-row">
            <input
              id={inputId}
              type="text"
              className="rl-input"
              style={{ flex: "1 1 16rem" }}
              value={commandInput}
              disabled={busy}
              placeholder={strings.chatCommandPlaceholder}
              onChange={(e) => setCommandInput(e.currentTarget.value)}
              data-testid="rl-chat-command-input"
            />
            <ActionButton
              type="submit"
              variant="primary"
              label={strings.chatDispatchBtn}
              disabled={busy || !commandInput.trim()}
              testId="rl-chat-command-submit"
            />
          </div>
        </form>

        {chatLog.length > 0 ? (
          <ul className="rl-event-list" data-testid="rl-chat-log">
            {chatLog.map((item, idx) => (
              <li key={`${item.command}-${String(idx)}`} className="rl-event-row">
                <span className="rl-event-type">/{item.command}</span>
                <p className="rl-event-detail">{item.summary}</p>
              </li>
            ))}
          </ul>
        ) : null}
      </article>

      <EventFeedList
        events={events}
        loading={loading}
        streamError={null}
        lastSeq={events.at(-1)?.seq ?? null}
        strings={strings}
      />
    </div>
  );
}
