import { wrap } from "@reatom/core";
import { reatomComponent } from "@reatom/react";
import { useRef, type KeyboardEvent, type ReactNode } from "react";
import { getStrings } from "@/common/i18n";
import { getTeamStrings } from "@/common/team-i18n";
import { isTeamManagedMode } from "@/common/team-types";
import { ActionButton } from "@/common/ui/ActionButton";
import { CasArtifactEvidencePanel } from "@/common/ui/CasArtifactEvidencePanel";
import { MobileAcceptanceBar } from "@/common/ui/MobileAcceptanceBar";
import { RunnerOwnershipPanel } from "@/common/ui/RunnerOwnershipPanel";
import { TaskCommentsPanel } from "@/common/ui/TaskCommentsPanel";
import {
  AcceptancePrTab,
  DiffArtifactsTab,
  EventsChatTab,
  RunBudgetTab,
} from "@/common/ui/TaskDetailPanels";
import { TaskWorkflowPanel } from "@/common/ui/TaskWorkflowPanel";
import type { TeamCollabModel } from "@/modules/team-collab";
import type { TaskDetailTab, TaskFlowModel } from "./model";

const TAB_ORDER: readonly TaskDetailTab[] = [
  "workflow",
  "run_budget",
  "diff_artifacts",
  "acceptance_pr",
  "comments",
  "events_chat",
];

export interface TaskDetailViewProps {
  readonly model: TaskFlowModel;
  readonly teamModel: TeamCollabModel;
}

export const TaskDetailView = reatomComponent<TaskDetailViewProps>(
  ({ model, teamModel }): ReactNode => {
    const detail = model.selectedTaskDetail();
    const locale = model.locale();
    const strings = getStrings(locale);
    const teamStrings = getTeamStrings(locale);
    const activeTab = model.activeTaskTab();
    const pending = model.actionPending();
    const teamPending = teamModel.actionPending();
    const err = model.actionError() ?? teamModel.actionError();
    const board = model.board();
    const history = model.selectedRunHistory();
    const diff = model.selectedRunDiff();
    const events = model.selectedRunEvents();
    const chatLog = model.chatLog();
    const prUnknown = model.prUnknownReason();
    const inspections = teamModel.artifactInspections();
    const inspectingDigest = teamModel.inspectingDigest();
    const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);

    if (!detail) {
      return (
        <article className="rl-card">
          <p className="rl-status-line">{strings.loadingTask}</p>
        </article>
      );
    }

    const comments = detail.comments ?? [];
    const commentDrafts = teamModel
      .drafts()
      .filter((d) => d.kind === "task_comment" && d.task_id === detail.task.id);
    const isTeam = isTeamManagedMode(board?.ownership_mode ?? board?.project?.ownership);
    const isOfflineLocked = isTeam && teamModel.connectionState() === "offline";
    const runners = board?.runners ?? teamModel.runners();
    const run = history?.run ?? detail.latest_run;
    const runId = run?.id ?? detail.task.latest_run_id ?? detail.task.active_run_id;
    const sessionId = run?.session_id ?? detail.task.active_session_id ?? "";
    const sessionRev = detail.session_revision ?? 1;
    const ownership =
      detail.latest_run_ownership ?? (runId ? (board?.run_ownerships?.[runId] ?? null) : null);
    const evidenceArtifact = [...detail.artifacts]
      .reverse()
      .find((a) => a.label === "evidence_bundle");
    const evidenceInspection = evidenceArtifact
      ? inspections[evidenceArtifact.digest_sha256]
      : undefined;

    const tabLabels: Readonly<Record<TaskDetailTab, string>> = {
      workflow: strings.taskTabWorkflow,
      run_budget: strings.taskTabRunBudget,
      diff_artifacts: strings.taskTabDiffArtifacts,
      acceptance_pr: strings.taskTabAcceptancePr,
      comments:
        comments.length + commentDrafts.length > 0
          ? `${strings.taskTabComments} (${String(comments.length + commentDrafts.length)})`
          : strings.taskTabComments,
      events_chat: strings.taskTabEventsChat,
    };

    const handleTabKeyDown = wrap((event: KeyboardEvent<HTMLDivElement>): void => {
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
        model.activeTaskTab.set(nextTab);
        tabRefs.current[nextIdx]?.focus();
      }
    });

    const anyBusy = pending !== null || teamPending !== null;

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
            actionPending={pending ?? teamPending}
            offlineBlocked={isOfflineLocked}
            {...(isTeam
              ? {
                  savePlanDraftLabel: teamStrings.savePlanOfflineDraftBtn,
                  onSavePlanDraft: wrap(
                    (planText) =>
                      void teamModel.createOfflinePlanDraft({
                        projectId: detail.task.project_id,
                        taskId: detail.task.id,
                        planText,
                        baseRevision: detail.task.revision,
                      }),
                  ),
                }
              : {})}
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
          <div className="rl-field" style={{ gap: "0.75rem" }}>
            {isTeam ? (
              <RunnerOwnershipPanel
                run={run}
                ownership={ownership}
                runners={runners}
                offlineBlocked={isOfflineLocked}
                busy={anyBusy}
                strings={teamStrings}
                onAssignRunner={wrap(
                  (rid, runnerId, rev) => void teamModel.assignRunner(rid, runnerId, rev),
                )}
                onCancelUnreachable={wrap(
                  (rid, reason, rev) => void teamModel.cancelUnreachableRun(rid, reason, rev),
                )}
              />
            ) : null}
            <RunBudgetTab
              detail={detail}
              history={history}
              strings={strings}
              locale={locale}
              actionPending={pending}
              offlineBlocked={isOfflineLocked}
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
        </div>

        <div
          role="tabpanel"
          aria-labelledby="rl-tab-diff_artifacts"
          hidden={activeTab !== "diff_artifacts"}
        >
          <DiffArtifactsTab
            detail={detail}
            diff={diff}
            strings={strings}
            artifactsSlot={
              <CasArtifactEvidencePanel
                artifacts={detail.artifacts}
                inspections={inspections}
                inspectingDigest={inspectingDigest}
                strings={strings}
                teamStrings={teamStrings}
                onInspectArtifact={wrap((digest) => void teamModel.inspectArtifact(digest))}
                {...(isTeam ? { onSyncWorkspace: wrap(() => void teamModel.syncNow()) } : {})}
              />
            }
          />
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
            offlineBlockedReason={isOfflineLocked ? teamStrings.mobileAcceptanceOfflineHint : null}
            onAccept={wrap((input) => void model.acceptTask(detail.task.id, input))}
            onReject={wrap((reason) => void model.rejectTask(detail.task.id, reason))}
            onCreatePr={wrap((input) => void model.createPr(detail.task.id, input))}
          />
        </div>

        <div role="tabpanel" aria-labelledby="rl-tab-comments" hidden={activeTab !== "comments"}>
          <TaskCommentsPanel
            key={detail.task.id}
            taskId={detail.task.id}
            comments={comments}
            commentDrafts={commentDrafts}
            queuedOfflineNotice={teamModel.commentQueuedTaskId() === detail.task.id}
            busy={anyBusy}
            strings={teamStrings}
            onPostComment={wrap((body) =>
              teamModel.addComment(detail.task.id, body, runId ?? undefined),
            )}
            onRetryDraft={wrap((did) => void teamModel.retryDraft(did))}
            onDeleteDraft={wrap((did) => void teamModel.deleteDraft(did))}
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

        <MobileAcceptanceBar
          detail={detail}
          locale={locale}
          strings={strings}
          teamStrings={teamStrings}
          evidenceInspection={evidenceInspection}
          offlineBlocked={isOfflineLocked}
          busy={anyBusy}
          onInspectEvidence={wrap((digest) => void teamModel.inspectArtifact(digest))}
          onAccept={wrap((input) => void model.acceptTask(detail.task.id, input))}
          onReject={wrap((reason) => void model.rejectTask(detail.task.id, reason))}
        />
      </div>
    );
  },
  "TaskDetailView",
);
