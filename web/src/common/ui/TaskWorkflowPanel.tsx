import { useId, useState, type FormEvent, type ReactNode } from "react";
import {
  localizedRunStatusLabel,
  normalizeRunStatusValue,
  taskStageLabel,
  workflowStageLabel,
  type BoardTaskEntry,
} from "../board-types";
import type { RunlarkStrings } from "../i18n";
import { ActionButton } from "./ActionButton";
import { QuestionDecisionCard } from "./QuestionDecisionCard";
import { StatusBadge } from "./StatusBadge";

export interface TaskWorkflowPanelProps {
  readonly detail: BoardTaskEntry;
  readonly strings: RunlarkStrings;
  readonly actionPending: string | null;
  readonly offlineBlocked?: boolean;
  readonly savePlanDraftLabel?: string;
  readonly onClarify: (clarification: string) => void;
  readonly onSavePlan: (planText: string, approve: boolean) => void;
  readonly onSavePlanDraft?: (planText: string) => void;
  readonly onStartRun: () => void;
  readonly onAnswerQuestion: (questionId: string, answer: string) => void;
}

export function TaskWorkflowPanel({
  detail,
  strings,
  actionPending,
  offlineBlocked = false,
  savePlanDraftLabel,
  onClarify,
  onSavePlan,
  onSavePlanDraft,
  onStartRun,
  onAnswerQuestion,
}: TaskWorkflowPanelProps): ReactNode {
  const { task, latest_run: run, pending_questions: pendingQuestions } = detail;
  const [clarification, setClarification] = useState(task.clarification_notes ?? "");
  const [planText, setPlanText] = useState(task.plan_text ?? "");
  const clarifyId = useId();
  const planId = useId();
  const startHintId = useId();

  const busy = actionPending !== null;
  const runStatus = normalizeRunStatusValue(run?.status);
  const hasBlockingRun =
    run !== null &&
    (runStatus === "queued" ||
      runStatus === "running" ||
      runStatus === "waiting_for_approval" ||
      runStatus === "cancel_requested");
  const pendingQuestion = pendingQuestions.find((q) => q.status === "pending") ?? null;

  const handleClarify = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (busy || offlineBlocked || !clarification.trim()) return;
    onClarify(clarification.trim());
  };

  return (
    <div className="rl-field" style={{ gap: "0.75rem" }}>
      <article className="rl-card">
        <header className="rl-card-header">
          <div>
            <h2 className="rl-card-title">{task.title}</h2>
            {task.description ? <p className="rl-status-line">{task.description}</p> : null}
          </div>
          <div className="rl-actions-row">
            <StatusBadge
              status="queued"
              customTone="neutral"
              customLabel={taskStageLabel(task.stage, strings)}
            />
            <StatusBadge
              status="queued"
              customTone="attention"
              customLabel={workflowStageLabel(task.workflow_stage, strings)}
            />
            {run ? (
              <StatusBadge
                status={runStatus}
                customLabel={localizedRunStatusLabel(runStatus, strings)}
              />
            ) : null}
          </div>
        </header>

        <dl className="rl-meta-grid">
          <div className="rl-meta-item">
            <dt className="rl-meta-label">{strings.taskIdLabel}</dt>
            <dd className="rl-meta-value">{task.id}</dd>
          </div>
          <div className="rl-meta-item">
            <dt className="rl-meta-label">{strings.revisionPlanLabel}</dt>
            <dd className="rl-meta-value">
              rev {String(task.revision)} · plan v{String(task.plan_version)}
            </dd>
          </div>
          <div className="rl-meta-item">
            <dt className="rl-meta-label">{strings.kindMetaLabel}</dt>
            <dd className="rl-meta-value">
              {task.kind === "quick_fix" ? strings.kindQuickFix : strings.kindStandard}
            </dd>
          </div>
          <div className="rl-meta-item">
            <dt className="rl-meta-label">{strings.branchPrLabel}</dt>
            <dd className="rl-meta-value">
              {task.pr_url ?? task.head_ref ?? task.base_ref ?? "-"}
            </dd>
          </div>
        </dl>

        <div className="rl-actions-row">
          <ActionButton
            variant="primary"
            label={strings.startRunBtn}
            disabled={busy || offlineBlocked || hasBlockingRun}
            disabledReasonId={startHintId}
            onClick={onStartRun}
            testId="rl-task-start-run"
          />
          {hasBlockingRun ? (
            <span id={startHintId} className="rl-meta-label">
              {strings.startRunDisabledActive}
            </span>
          ) : null}
        </div>
      </article>

      {pendingQuestion ? (
        <QuestionDecisionCard
          question={pendingQuestion}
          submitting={actionPending === "answer" || offlineBlocked}
          submitError={null}
          strings={strings}
          onSubmitAnswer={onAnswerQuestion}
        />
      ) : null}

      <article className="rl-card">
        <form onSubmit={handleClarify} className="rl-field">
          <label htmlFor={clarifyId} className="rl-label">
            {strings.clarifyLabel}
          </label>
          <textarea
            id={clarifyId}
            className="rl-input"
            rows={2}
            value={clarification}
            disabled={busy || offlineBlocked}
            onChange={(e) => setClarification(e.currentTarget.value)}
          />
          <div className="rl-actions-row">
            <ActionButton
              type="submit"
              variant="secondary"
              label={strings.clarifySubmit}
              disabled={busy || offlineBlocked || !clarification.trim()}
            />
          </div>
        </form>

        <div className="rl-field">
          <label htmlFor={planId} className="rl-label">
            {strings.planLabel}
          </label>
          <textarea
            id={planId}
            className="rl-input"
            rows={4}
            value={planText}
            disabled={busy}
            onChange={(e) => setPlanText(e.currentTarget.value)}
          />
          <div className="rl-actions-row">
            <ActionButton
              variant="secondary"
              label={strings.planSave}
              disabled={busy || offlineBlocked || !planText.trim()}
              onClick={() => onSavePlan(planText.trim(), false)}
            />
            <ActionButton
              variant="primary"
              label={strings.planSaveApprove}
              disabled={busy || offlineBlocked || !planText.trim()}
              onClick={() => onSavePlan(planText.trim(), true)}
              testId="rl-task-plan-approve"
            />
            {onSavePlanDraft && savePlanDraftLabel ? (
              <ActionButton
                variant="secondary"
                label={savePlanDraftLabel}
                disabled={busy || !planText.trim()}
                onClick={() => onSavePlanDraft(planText.trim())}
                testId="rl-task-plan-draft-btn"
              />
            ) : null}
          </div>
        </div>
      </article>
    </div>
  );
}
