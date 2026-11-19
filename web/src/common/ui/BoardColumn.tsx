import { useState, type ReactNode } from "react";
import { taskStageLabel, type BoardTaskEntry, type TaskStage } from "../board-types";
import type { Locale, RunlarkStrings } from "../i18n";
import type { TeamStrings } from "../team-i18n";
import type { RunnerRegistration } from "../team-types";
import { BoardCard } from "./BoardCard";
import { DropArrowSvg } from "./SvgIcons";

export interface BoardColumnProps {
  readonly stage: TaskStage;
  readonly entries: readonly BoardTaskEntry[];
  readonly hasActiveFilter: boolean;
  readonly strings: RunlarkStrings;
  readonly teamStrings?: TeamStrings;
  readonly runners?: readonly RunnerRegistration[];
  readonly locale: Locale;
  readonly draggingTaskId: string | null;
  readonly draggingSourceStage: TaskStage | null;
  readonly moveDisabled: boolean;
  readonly onOpenTask: (taskId: string) => void;
  readonly onMoveTask: (taskId: string, targetStage: TaskStage, expectedRevision: number) => void;
  readonly onDragStartTask: (taskId: string) => void;
  readonly onDragEndTask: () => void;
}

export function BoardColumn({
  stage,
  entries,
  hasActiveFilter,
  strings,
  teamStrings,
  runners,
  locale,
  draggingTaskId,
  draggingSourceStage,
  moveDisabled,
  onOpenTask,
  onMoveTask,
  onDragStartTask,
  onDragEndTask,
}: BoardColumnProps): ReactNode {
  const [dropHover, setDropHover] = useState(false);
  const title = taskStageLabel(stage, strings);
  const isDropCandidate =
    draggingTaskId !== null && draggingSourceStage !== null && draggingSourceStage !== stage;

  return (
    <section
      className="rl-column"
      aria-labelledby={`rl-col-${stage}`}
      data-testid={`rl-board-col-${stage}`}
    >
      <header className="rl-column-header">
        <h2 id={`rl-col-${stage}`} className="rl-column-heading">
          {title}
        </h2>
        <span className="rl-badge rl-badge-neutral rl-mono" data-testid={`rl-col-count-${stage}`}>
          {entries.length}
        </span>
      </header>

      <div
        className="rl-column-dropzone"
        onDragOver={(e) => {
          if (isDropCandidate) {
            e.preventDefault();
            e.dataTransfer.dropEffect = "move";
            if (!dropHover) setDropHover(true);
          }
        }}
        onDragLeave={() => {
          if (dropHover) setDropHover(false);
        }}
        onDrop={(e) => {
          e.preventDefault();
          setDropHover(false);
          if (!draggingTaskId || !isDropCandidate) return;
          const dragged = entries.find((item) => item.task.id === draggingTaskId);
          if (dragged) {
            onMoveTask(draggingTaskId, stage, dragged.task.revision);
          } else {
            onMoveTask(draggingTaskId, stage, -1);
          }
          onDragEndTask();
        }}
      >
        {isDropCandidate ? (
          <div className="rl-drop-target" data-drop-hover={dropHover ? "true" : "false"}>
            <DropArrowSvg />
            <span>{strings.dropTargetHint(title)}</span>
          </div>
        ) : null}

        {entries.length === 0 ? (
          <p className="rl-status-line">
            {hasActiveFilter ? strings.emptyFiltered : strings.emptyColumn}
          </p>
        ) : (
          entries.map((entry) => (
            <BoardCard
              key={entry.task.id}
              entry={entry}
              strings={strings}
              {...(teamStrings ? { teamStrings } : {})}
              {...(runners ? { runners } : {})}
              locale={locale}
              isDragging={draggingTaskId === entry.task.id}
              moveDisabled={moveDisabled}
              onOpenTask={onOpenTask}
              onMoveTask={onMoveTask}
              onDragStartTask={onDragStartTask}
              onDragEndTask={onDragEndTask}
            />
          ))
        )}
      </div>
    </section>
  );
}
