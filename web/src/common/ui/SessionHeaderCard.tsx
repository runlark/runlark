import { useId, useState, type FormEvent, type ReactNode } from "react";
import {
  normalizeSessionStatus,
  resolveDisplayRunId,
  resolveDisplayRunStatus,
  resolveDisplayWorktree,
  type DaemonSession,
} from "../runlark-types";
import { ActionButton } from "./ActionButton";
import { StatusBadge } from "./StatusBadge";

export interface SessionHeaderCardProps {
  readonly session: DaemonSession;
  readonly connectionState: "connecting" | "live" | "reconnecting" | "error";
  readonly actionPending: string | null;
  readonly actionError: string | null;
  readonly onSendPrompt: (prompt: string) => void;
  readonly onStopRun: () => void;
  readonly onResumeSession: () => void;
  readonly onArchiveSession: () => void;
  readonly onRefresh: () => void;
}

export function SessionHeaderCard({
  session,
  connectionState,
  actionPending,
  actionError,
  onSendPrompt,
  onStopRun,
  onResumeSession,
  onArchiveSession,
  onRefresh,
}: SessionHeaderCardProps): ReactNode {
  const [promptText, setPromptText] = useState("");
  const [promptError, setPromptError] = useState<string | null>(null);
  const promptId = useId();
  const promptErrId = useId();
  const stopHintId = useId();
  const resumeHintId = useId();
  const archiveHintId = useId();

  const isConnectionUnknown = connectionState === "error";
  const runStatus = resolveDisplayRunStatus(session, isConnectionUnknown);
  const sessionStatus = normalizeSessionStatus(session.status);
  const displayRunId = resolveDisplayRunId(session);
  const displayWorktree = resolveDisplayWorktree(session);
  const dispatchedUnknown = session.dispatched_commands_count ?? 0;

  const isArchived = sessionStatus === "archived";
  const isBusy = actionPending !== null;
  const canSend =
    !isArchived &&
    !isBusy &&
    runStatus !== "waiting_for_approval" &&
    runStatus !== "cancel_requested";
  const canStop =
    !isArchived &&
    !isBusy &&
    session.active_run_id !== null &&
    (runStatus === "running" || runStatus === "queued" || runStatus === "waiting_for_approval");
  const canResume =
    !isArchived && !isBusy && session.active_run_id !== null && runStatus !== "cancel_requested";
  const canArchive =
    !isArchived &&
    !isBusy &&
    (session.active_run_id === null ||
      runStatus === "stopped" ||
      runStatus === "completed" ||
      runStatus === "failed");

  const handleSubmit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const trimmed = promptText.trim();
    if (!trimmed) {
      setPromptError("Enter a prompt before sending.");
      return;
    }
    setPromptError(null);
    onSendPrompt(trimmed);
    setPromptText("");
  };

  return (
    <article className="rl-card" aria-labelledby="rl-session-heading">
      <header className="rl-card-header">
        <div>
          <h2 id="rl-session-heading" className="rl-card-title">
            Runlark Session
          </h2>
          <p className="rl-status-line">
            {connectionState === "live"
              ? "Live daemon sync"
              : connectionState === "reconnecting"
                ? "Reconnecting to event stream"
                : connectionState === "error"
                  ? "Daemon connection lost · state unknown"
                  : "Connecting to daemon"}
          </p>
        </div>
        <div className="rl-actions-row">
          <StatusBadge status={runStatus} />
          {dispatchedUnknown > 0 ? (
            <StatusBadge
              status="unknown"
              customTone="unknown"
              customLabel={`${String(dispatchedUnknown)} unknown delivery`}
            />
          ) : null}
        </div>
      </header>

      <dl className="rl-meta-grid">
        <div className="rl-meta-item">
          <dt className="rl-meta-label">Session ID</dt>
          <dd className="rl-meta-value" data-testid="rl-session-id">
            {session.session_id}
          </dd>
        </div>
        <div className="rl-meta-item">
          <dt className="rl-meta-label">Task ID</dt>
          <dd className="rl-meta-value" data-testid="rl-task-id">
            {session.task_id}
          </dd>
        </div>
        <div className="rl-meta-item">
          <dt className="rl-meta-label">Run ID</dt>
          <dd className="rl-meta-value" data-testid="rl-run-id">
            {displayRunId ?? "None"}
          </dd>
        </div>
        <div className="rl-meta-item">
          <dt className="rl-meta-label">Revision</dt>
          <dd className="rl-meta-value" data-testid="rl-revision">
            rev {String(session.current_revision)} · {sessionStatus}
          </dd>
        </div>
        <div className="rl-meta-item rl-meta-item-wide">
          <dt className="rl-meta-label">Worktree / CWD</dt>
          <dd className="rl-meta-value" data-testid="rl-worktree-path">
            {displayWorktree}
          </dd>
        </div>
      </dl>

      {actionError ? (
        <div className="rl-error-box" role="alert" data-testid="rl-action-error">
          {actionError}
        </div>
      ) : null}

      <form onSubmit={handleSubmit} className="rl-field" noValidate>
        <label htmlFor={promptId} className="rl-label">
          Send prompt
        </label>
        <input
          id={promptId}
          name="prompt"
          type="text"
          className="rl-input"
          value={promptText}
          onChange={(e) => {
            setPromptText(e.currentTarget.value);
            if (promptError) {
              setPromptError(null);
            }
          }}
          disabled={!canSend}
          aria-invalid={promptError ? "true" : undefined}
          aria-describedby={promptError ? promptErrId : undefined}
          placeholder="Instruct the managed run"
        />
        {promptError ? (
          <p id={promptErrId} className="rl-error-box" role="alert">
            {promptError}
          </p>
        ) : null}
        <div className="rl-actions-row">
          <ActionButton
            type="submit"
            variant="primary"
            label={actionPending === "send" ? "Sending..." : "Send"}
            disabled={!canSend}
            testId="rl-btn-send"
          />
          <ActionButton
            variant="danger"
            label={actionPending === "stop" ? "Stopping..." : "Stop"}
            disabled={!canStop}
            disabledReasonId={stopHintId}
            onClick={onStopRun}
            testId="rl-btn-stop"
          />
          <ActionButton
            variant="secondary"
            label={actionPending === "resume" ? "Resuming..." : "Resume"}
            disabled={!canResume}
            disabledReasonId={resumeHintId}
            onClick={onResumeSession}
            testId="rl-btn-resume"
          />
          <ActionButton
            variant="secondary"
            label={actionPending === "archive" ? "Archiving..." : "Archive"}
            disabled={!canArchive}
            disabledReasonId={archiveHintId}
            onClick={onArchiveSession}
            testId="rl-btn-archive"
          />
          <ActionButton
            variant="secondary"
            label="Refresh"
            disabled={isBusy}
            onClick={onRefresh}
            testId="rl-btn-refresh"
          />
        </div>
      </form>

      {!canStop || !canResume || !canArchive ? (
        <p className="rl-help-text">
          {!canStop ? <span id={stopHintId}>Stop requires an active run. </span> : null}
          {!canResume ? (
            <span id={resumeHintId}>Resume applies to non-terminal active runs. </span>
          ) : null}
          {!canArchive ? (
            <span id={archiveHintId}>Archive is locked while a run is active or cancelling.</span>
          ) : null}
        </p>
      ) : null}
    </article>
  );
}
