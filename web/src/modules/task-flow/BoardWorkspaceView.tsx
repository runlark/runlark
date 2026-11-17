import { wrap } from "@reatom/core";
import { reatomComponent } from "@reatom/react";
import { useId, type ReactNode } from "react";
import { formatMicroUsd, type TaskKind } from "@/common/board-types";
import { getStrings } from "@/common/i18n";
import { ActionButton } from "@/common/ui/ActionButton";
import { CreateTaskDialog } from "@/common/ui/CreateTaskDialog";
import { KanbanBoardSurface } from "@/common/ui/KanbanBoardSurface";
import { StatusBadge } from "@/common/ui/StatusBadge";
import type { TaskFlowModel } from "./model";

export const BoardContentView = reatomComponent<{ readonly model: TaskFlowModel }>(
  ({ model }): ReactNode => {
    const strings = getStrings(model.locale());
    const locale = model.locale();
    const board = model.board();
    const columns = model.filteredColumns();
    const decisions = model.decisionEntries();
    const search = model.searchQuery();
    const kf = model.kindFilter();
    const decOnly = model.decisionsOnly();
    const pending = model.actionPending();
    const err = model.actionError();
    const dialogKind = model.createDialogKind();

    const searchId = useId();
    const kindId = useId();

    const slots = board?.slots ?? { max: 2, used: 0, available: 2 };
    const queuedCount = board?.queued_runs.length ?? 0;
    const cost = board?.cost_summary ?? {
      known_micro_usd: 0,
      held_micro_usd: 0,
      unknown_attempts: 0,
    };
    const hasFilter = search.trim().length > 0 || kf !== "all" || decOnly;

    return (
      <>
        <div className="rl-page-header">
          <div>
            <h1 className="rl-page-h1">{strings.boardHeading}</h1>
            <p className="rl-page-subtitle">{strings.boardSubtitle}</p>
          </div>
          <div className="rl-actions-row">
            <span className="rl-badge rl-badge-neutral rl-mono" data-testid="rl-slots-badge">
              {strings.slotsSummary(slots.used, slots.max, queuedCount)}
            </span>
            <span className="rl-badge rl-badge-neutral rl-mono" data-testid="rl-board-cost-known">
              {strings.knownCostLabel}: {formatMicroUsd(cost.known_micro_usd, locale)}
            </span>
            {cost.unknown_attempts > 0 ? (
              <StatusBadge
                status="unknown"
                customTone="unknown"
                customLabel={strings.unknownCostAttempts(cost.unknown_attempts)}
              />
            ) : null}
          </div>
        </div>

        {decisions.length > 0 ? (
          <div className="rl-attention-strip" data-testid="rl-board-attention">
            <span>{strings.attentionTitle(decisions.length)}</span>
            <ActionButton
              variant="secondary"
              label={strings.attentionAction}
              onClick={wrap(() => model.decisionsOnly.set(!decOnly))}
            />
          </div>
        ) : null}

        {err ? (
          <div className="rl-error-box" role="alert" data-testid="rl-board-error">
            {err}
          </div>
        ) : null}

        <form className="rl-toolbar" onSubmit={(e) => e.preventDefault()}>
          <div className="rl-toolbar-filters">
            <label htmlFor={searchId} className="sr-only">
              {strings.searchLabel}
            </label>
            <input
              id={searchId}
              type="search"
              className="rl-toolbar-input"
              value={search}
              placeholder={strings.searchPlaceholder}
              onChange={wrap((e: React.ChangeEvent<HTMLInputElement>) =>
                model.searchQuery.set(e.currentTarget.value),
              )}
              data-testid="rl-board-search"
            />

            <label htmlFor={kindId} className="sr-only">
              {strings.kindFilterLabel}
            </label>
            <select
              id={kindId}
              className="rl-toolbar-select"
              value={kf}
              onChange={wrap((e: React.ChangeEvent<HTMLSelectElement>) =>
                model.kindFilter.set(e.currentTarget.value as "all" | TaskKind),
              )}
              data-testid="rl-board-kind-filter"
            >
              <option value="all">{strings.kindAll}</option>
              <option value="standard">{strings.kindStandard}</option>
              <option value="quick_fix">{strings.kindQuickFix}</option>
            </select>

            <button
              type="button"
              className="rl-toggle-chip"
              aria-pressed={decOnly}
              onClick={wrap(() => model.decisionsOnly.set(!decOnly))}
              data-testid="rl-filter-decisions"
            >
              {strings.decisionsOnly} ({decisions.length})
            </button>
          </div>

          <div className="rl-toolbar-actions">
            <ActionButton
              variant="secondary"
              label={strings.refreshBtn}
              disabled={pending !== null}
              onClick={wrap(() => void model.refreshAll())}
            />
            <ActionButton
              variant="secondary"
              label={strings.newQuickFixBtn}
              disabled={pending !== null}
              onClick={wrap(() => model.openCreateDialog("quick_fix"))}
              testId="rl-btn-new-quick-fix"
            />
            <ActionButton
              variant="primary"
              label={strings.newTaskBtn}
              disabled={pending !== null}
              onClick={wrap(() => model.openCreateDialog("standard"))}
              testId="rl-btn-new-task"
            />
          </div>
        </form>

        <KanbanBoardSurface
          columns={columns}
          mobileStage={model.mobileStage()}
          onMobileStageChange={wrap((s) => model.mobileStage.set(s))}
          hasActiveFilter={hasFilter}
          strings={strings}
          locale={locale}
          moveDisabled={pending !== null}
          announcement={model.announcement()}
          onOpenTask={wrap((id) => void model.openTask(id))}
          onMoveTask={wrap((id, stage, rev) => void model.moveTask(id, stage, rev))}
        />

        <CreateTaskDialog
          open={dialogKind !== null}
          initialKind={dialogKind ?? "standard"}
          submitting={pending === "createTask"}
          submitError={err}
          strings={strings}
          onClose={wrap(() => model.closeCreateDialog())}
          onSubmit={wrap((input) => void model.createTask(input))}
        />
      </>
    );
  },
  "BoardContentView",
);
