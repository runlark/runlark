import { wrap } from "@reatom/core";
import { reatomComponent, useWrap } from "@reatom/react";
import { useEffect, type ReactNode } from "react";
import { ActionButton } from "@/common/ui/ActionButton";
import { EventFeedList } from "@/common/ui/EventFeedList";
import { QuestionDecisionCard } from "@/common/ui/QuestionDecisionCard";
import { SessionHeaderCard } from "@/common/ui/SessionHeaderCard";
import { StatusBadge } from "@/common/ui/StatusBadge";
import type { SessionCardModel } from "./model";

export interface SessionCardViewProps {
  readonly model: SessionCardModel;
  readonly defaultTaskId?: string;
  readonly defaultCwd?: string;
  readonly autoStartSync?: boolean;
  readonly pollWaitMs?: number;
}

export const SessionCardView = reatomComponent<SessionCardViewProps>(
  ({ model, defaultTaskId, defaultCwd, autoStartSync = true, pollWaitMs }): ReactNode => {
    const startSync = useWrap(model.startLiveSync, "SessionCardView.startLiveSync");

    useEffect(() => {
      if (!autoStartSync) {
        return undefined;
      }
      return startSync(pollWaitMs !== undefined ? { waitMs: pollWaitMs } : undefined);
    }, [startSync, autoStartSync, pollWaitMs]);

    const currentSession = model.session();
    const isLoading = model.loading();
    const connState = model.connectionState();
    const pending = model.actionPending();
    const actError = model.actionError();
    const qError = model.questionError();
    const evtError = model.streamError();
    const pendingQuestion = model.activeQuestion();
    const eventList = model.events();
    const seq = model.lastSeq();

    if (isLoading && !currentSession) {
      return (
        <article className="rl-card" data-testid="rl-session-loading">
          <header className="rl-card-header">
            <h2 className="rl-card-title">Runlark Session</h2>
            <StatusBadge status="unknown" customLabel="Connecting" />
          </header>
          <p className="rl-status-line">Loading durable session state from SQLite...</p>
        </article>
      );
    }

    if (!currentSession) {
      const taskId = defaultTaskId ?? `task_${model.sessionId}`;
      const cwd = defaultCwd ?? "/";
      return (
        <article className="rl-card" data-testid="rl-session-unmanaged">
          <header className="rl-card-header">
            <h2 className="rl-card-title">Unmanaged DSH Session</h2>
            <StatusBadge
              status="unknown"
              customLabel={connState === "error" ? "Daemon unreachable" : "Not managed"}
            />
          </header>
          <dl className="rl-meta-grid">
            <div className="rl-meta-item rl-meta-item-wide">
              <dt className="rl-meta-label">Session ID</dt>
              <dd className="rl-meta-value">{model.sessionId}</dd>
            </div>
            <div className="rl-meta-item rl-meta-item-wide">
              <dt className="rl-meta-label">Workspace CWD</dt>
              <dd className="rl-meta-value">{cwd}</dd>
            </div>
          </dl>
          {evtError ? (
            <div className="rl-error-box" role="alert">
              {evtError}
            </div>
          ) : null}
          {actError ? (
            <div className="rl-error-box" role="alert">
              {actError}
            </div>
          ) : null}
          <footer className="rl-actions-row">
            <ActionButton
              variant="primary"
              label={pending === "attach" ? "Managing..." : "Manage with Runlark"}
              disabled={pending !== null}
              onClick={wrap(() => void model.attachSession(taskId, cwd))}
              testId="rl-btn-attach"
            />
            <ActionButton
              variant="secondary"
              label="Retry sync"
              disabled={pending !== null}
              onClick={wrap(() => void model.refresh())}
              testId="rl-btn-retry"
            />
          </footer>
        </article>
      );
    }

    return (
      <>
        <SessionHeaderCard
          session={currentSession}
          connectionState={connState}
          actionPending={pending}
          actionError={actError}
          onSendPrompt={wrap((prompt: string) => void model.sendPrompt(prompt))}
          onStopRun={wrap(() => void model.stopRun())}
          onResumeSession={wrap(() => void model.resumeSession())}
          onArchiveSession={wrap(() => void model.archiveSession())}
          onRefresh={wrap(() => void model.refresh())}
        />
        {pendingQuestion ? (
          <QuestionDecisionCard
            key={pendingQuestion.question_id}
            question={pendingQuestion}
            submitting={pending === "answer"}
            submitError={qError}
            onSubmitAnswer={wrap(
              (questionId: string, answer: string) => void model.answerQuestion(questionId, answer),
            )}
          />
        ) : null}
        <EventFeedList
          events={eventList}
          loading={isLoading}
          streamError={evtError}
          lastSeq={seq}
        />
      </>
    );
  },
  "SessionCardView",
);
