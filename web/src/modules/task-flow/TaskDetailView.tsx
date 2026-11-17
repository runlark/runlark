import { wrap } from "@reatom/core";
import { reatomComponent } from "@reatom/react";
import { useRef, type KeyboardEvent, type ReactNode } from "react";
import { getStrings } from "@/common/i18n";
import { ActionButton } from "@/common/ui/ActionButton";
import {
  AcceptancePrTab,
  DiffArtifactsTab,
  EventsChatTab,
  RunBudgetTab,
} from "@/common/ui/TaskDetailPanels";
import { TaskWorkflowPanel } from "@/common/ui/TaskWorkflowPanel";
import type { TaskDetailTab, TaskFlowModel } from "./model";

const TAB_ORDER: readonly TaskDetailTab[] = [
  "workflow",
  "run_budget",
  "diff_artifacts",
  "acceptance_pr",
  "events_chat",
];

export const TaskDetailView = reatomComponent<{ readonly model: TaskFlowModel }>(
  ({ model }): ReactNode => {
    const detail = model.selectedTaskDetail();
    const strings = getStrings(model.locale());
    const locale = model.locale();
    const activeTab = model.activeTaskTab();
    const pending = model.actionPending();
    const err = model.actionError();
    const history = model.selectedRunHistory();
    const diff = model.selectedRunDiff();
    const events = model.selectedRunEvents();
    const chatLog = model.chatLog();
    const prUnknown = model.prUnknownReason();
    const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);

    if (!detail) {
      return (
        <article className="rl-card">
          <p className="rl-status-line">{strings.loadingTask}</p>
        </article>
      );
    }

    const tabLabels: Readonly<Record<TaskDetailTab, string>> = {
      workflow: strings.taskTabWorkflow,
      run_budget: strings.taskTabRunBudget,
      diff_artifacts: strings.taskTabDiffArtifacts,
      acceptance_pr: strings.taskTabAcceptancePr,
      events_chat: strings.taskTabEventsChat,
    };

    const handleTabKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
      const idx = TAB_ORDER.indexOf(activeTab);
      let nextIdx = idx;
      if (event.key === "ArrowRight") nextIdx = (idx + 1) % TAB_ORDER.length;
      else if (event.key === "ArrowLeft") nextIdx = (idx - 1 + TAB_ORDER.length) % TAB_ORDER.length;
      else if (event.key === "Home") nextIdx = 0;
      else if (event.key === "End") nextIdx = TAB_ORDER.length - 1;
      else return;
      event.preventDefault();
      const nextTab = TAB_ORDER[nextIdx];
      if (nextTab) {
        wrap(() => model.activeTaskTab.set(nextTab))();
        tabRefs.current[nextIdx]?.focus();
      }
    };

    const runId = detail.latest_run?.id ?? detail.task.latest_run_id ?? detail.task.active_run_id;
    const sessionId = detail.latest_run?.session_id ?? detail.task.active_session_id ?? "";
    const sessionRev = detail.session_revision ?? 1;

    return (
      <div className="rl-field" style={{ gap: "0.875rem" }} data-testid="rl-task-detail">
        <div className="rl-page-header">
          <div>
            <h1 className="rl-page-h1">{detail.task.title}</h1>
            <p className="rl-page-subtitle rl-mono">{detail.task.id}</p>
          </div>
          <ActionButton
            variant="secondary"
            label={strings.backToBoardBtn}
            onClick={wrap(() => model.closeTask())}
            testId="rl-back-to-board"
          />
        </div>

        {err ? (
          <div className="rl-error-box" role="alert" data-testid="rl-task-error">
            {err}
          </div>
        ) : null}

        <div
          role="tablist"
          aria-label={detail.task.title}
          className="rl-tabs-list"
          tabIndex={-1}
          onKeyDown={handleTabKeyDown}
        >
          {TAB_ORDER.map((tab, i) => (
            <button
              key={tab}
              ref={(el) => {
                tabRefs.current[i] = el;
              }}
              id={`rl-tab-${tab}`}
              type="button"
              role="tab"
              aria-selected={activeTab === tab}
              tabIndex={activeTab === tab ? 0 : -1}
              className="rl-tab-btn"
              onClick={wrap(() => model.activeTaskTab.set(tab))}
              data-testid={`rl-task-tab-${tab}`}
            >
              {tabLabels[tab]}
            </button>
          ))}
        </div>

        <div role="tabpanel" aria-labelledby="rl-tab-workflow" hidden={activeTab !== "workflow"}>
          <TaskWorkflowPanel
            key={`${detail.task.id}:${String(detail.task.revision)}`}
            detail={detail}
            strings={strings}
            actionPending={pending}
            onClarify={wrap((text) => void model.clarifyTask(detail.task.id, text))}
            onSavePlan={wrap(
              (text, approve) =>
                void model.savePlan(detail.task.id, text, approve, detail.task.revision),
            )}
            onStartRun={wrap(() => void model.startTask(detail.task.id))}
            onAnswerQuestion={wrap(
              (qid, answer) => void model.answerQuestion(qid, answer, sessionId, sessionRev),
            )}
          />
        </div>

        <div
          role="tabpanel"
          aria-labelledby="rl-tab-run_budget"
          hidden={activeTab !== "run_budget"}
        >
          <RunBudgetTab
            detail={detail}
            history={history}
            strings={strings}
            locale={locale}
            actionPending={pending}
            onStopRun={wrap(() => {
              if (runId && sessionId) void model.stopRun(sessionId, runId, sessionRev);
            })}
            onResumeRun={wrap(() => {
              if (runId && sessionId) void model.resumeSession(sessionId, runId, sessionRev);
            })}
            onVerifyRun={wrap(() => {
              if (runId) void model.verifyRun(runId, detail.task.id);
            })}
          />
        </div>

        <div
          role="tabpanel"
          aria-labelledby="rl-tab-diff_artifacts"
          hidden={activeTab !== "diff_artifacts"}
        >
          <DiffArtifactsTab detail={detail} diff={diff} strings={strings} />
        </div>

        <div
          role="tabpanel"
          aria-labelledby="rl-tab-acceptance_pr"
          hidden={activeTab !== "acceptance_pr"}
        >
          <AcceptancePrTab
            key={`${detail.task.id}:${String(detail.task.revision)}`}
            detail={detail}
            strings={strings}
            actionPending={pending}
            prUnknownReason={prUnknown}
            onAccept={wrap((input) => void model.acceptTask(detail.task.id, input))}
            onReject={wrap((reason) => void model.rejectTask(detail.task.id, reason))}
            onCreatePr={wrap((input) => void model.createPr(detail.task.id, input))}
          />
        </div>

        <div
          role="tabpanel"
          aria-labelledby="rl-tab-events_chat"
          hidden={activeTab !== "events_chat"}
        >
          <EventsChatTab
            events={events}
            loading={model.loading()}
            chatLog={chatLog}
            strings={strings}
            actionPending={pending}
            onDispatchChat={wrap((cmd) => void model.dispatchChat(cmd))}
          />
        </div>
      </div>
    );
  },
  "TaskDetailView",
);
