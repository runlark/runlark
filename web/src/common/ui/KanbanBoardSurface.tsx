import { useEffect, useId, useState, type ReactNode } from "react";
import { TASK_STAGES, taskStageLabel, type BoardTaskEntry, type TaskStage } from "../board-types";
import type { Locale, RunlarkStrings } from "../i18n";
import { ActionButton } from "./ActionButton";
import { BoardColumn } from "./BoardColumn";

export interface KanbanBoardSurfaceProps {
  readonly columns: Readonly<Record<TaskStage, readonly BoardTaskEntry[]>>;
  readonly mobileStage: TaskStage;
  readonly onMobileStageChange: (stage: TaskStage) => void;
  readonly hasActiveFilter: boolean;
  readonly strings: RunlarkStrings;
  readonly locale: Locale;
  readonly moveDisabled: boolean;
  readonly announcement: string;
  readonly onOpenTask: (taskId: string) => void;
  readonly onMoveTask: (taskId: string, targetStage: TaskStage, expectedRevision: number) => void;
}

export function KanbanBoardSurface({
  columns,
  mobileStage,
  onMobileStageChange,
  hasActiveFilter,
  strings,
  locale,
  moveDisabled,
  announcement,
  onOpenTask,
  onMoveTask,
}: KanbanBoardSurfaceProps): ReactNode {
  const [draggingTaskId, setDraggingTaskId] = useState<string | null>(null);
  const [isMobileViewport, setIsMobileViewport] = useState(false);
  const mobileSelectId = useId();

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
      return undefined;
    }
    const mq = window.matchMedia("(max-width: 47.99em)");
    const sync = (): void => setIsMobileViewport(mq.matches);
    sync();
    mq.addEventListener("change", sync);
    return () => mq.removeEventListener("change", sync);
  }, []);

  useEffect(() => {
    if (!draggingTaskId || typeof window === "undefined") {
      return undefined;
    }
    const clearDrag = (): void => setDraggingTaskId(null);
    window.addEventListener("dragend", clearDrag);
    window.addEventListener("drop", clearDrag);
    return () => {
      window.removeEventListener("dragend", clearDrag);
      window.removeEventListener("drop", clearDrag);
    };
  }, [draggingTaskId]);

  const allEntries = TASK_STAGES.flatMap((stage) => columns[stage]);
  const draggedEntry = draggingTaskId
    ? allEntries.find((e) => e.task.id === draggingTaskId)
    : undefined;
  const draggingSourceStage = draggedEntry?.task.stage ?? null;

  const mobileIndex = TASK_STAGES.indexOf(mobileStage);
  const prevStage = mobileIndex > 0 ? TASK_STAGES[mobileIndex - 1] : undefined;
  const nextStage = mobileIndex < TASK_STAGES.length - 1 ? TASK_STAGES[mobileIndex + 1] : undefined;

  const visibleStages = isMobileViewport ? [mobileStage] : TASK_STAGES;

  const handleMove = (taskId: string, targetStage: TaskStage, revHint: number): void => {
    const found = allEntries.find((e) => e.task.id === taskId);
    onMoveTask(taskId, targetStage, found ? found.task.revision : revHint);
  };

  return (
    <>
      <output className="sr-only" aria-live="polite" data-testid="rl-board-live">
        {announcement}
      </output>

      <div className="rl-mobile-stage-bar" data-testid="rl-mobile-stage-controls">
        <ActionButton
          variant="secondary"
          label={strings.mobilePrevStage}
          disabled={!prevStage}
          onClick={() => {
            if (prevStage) onMobileStageChange(prevStage);
          }}
        />
        <label htmlFor={mobileSelectId} className="sr-only">
          {strings.mobileStageSelect}
        </label>
        <select
          id={mobileSelectId}
          className="rl-toolbar-select"
          value={mobileStage}
          onChange={(e) => onMobileStageChange(e.currentTarget.value as TaskStage)}
        >
          {TASK_STAGES.map((stage) => (
            <option key={stage} value={stage}>
              {taskStageLabel(stage, strings)} ({columns[stage].length})
            </option>
          ))}
        </select>
        <ActionButton
          variant="secondary"
          label={strings.mobileNextStage}
          disabled={!nextStage}
          onClick={() => {
            if (nextStage) onMobileStageChange(nextStage);
          }}
        />
      </div>

      <div className="rl-board-surface" data-testid="rl-board-surface">
        <div className="rl-board-grid">
          {visibleStages.map((stage) => (
            <BoardColumn
              key={stage}
              stage={stage}
              entries={columns[stage]}
              hasActiveFilter={hasActiveFilter}
              strings={strings}
              locale={locale}
              draggingTaskId={draggingTaskId}
              draggingSourceStage={draggingSourceStage}
              moveDisabled={moveDisabled}
              onOpenTask={onOpenTask}
              onMoveTask={handleMove}
              onDragStartTask={(id) => setDraggingTaskId(id)}
              onDragEndTask={() => setDraggingTaskId(null)}
            />
          ))}
        </div>
      </div>
    </>
  );
}
