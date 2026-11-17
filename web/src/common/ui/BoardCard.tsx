import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  TASK_STAGES,
  formatMicroUsd,
  hasOwnPendingDecision,
  localizedRunStatusLabel,
  normalizeRunStatusValue,
  taskStageLabel,
  workflowStageLabel,
  type BoardTaskEntry,
  type TaskStage,
} from "../board-types";
import type { Locale, RunlarkStrings } from "../i18n";
import { StatusBadge } from "./StatusBadge";
import { DragGripSvg, MoreDotsSvg, PinIconSvg } from "./SvgIcons";

export interface BoardCardProps {
  readonly entry: BoardTaskEntry;
  readonly strings: RunlarkStrings;
  readonly locale: Locale;
  readonly isDragging: boolean;
  readonly moveDisabled: boolean;
  readonly onOpenTask: (taskId: string) => void;
  readonly onMoveTask: (taskId: string, targetStage: TaskStage, expectedRevision: number) => void;
  readonly onDragStartTask: (taskId: string) => void;
  readonly onDragEndTask: () => void;
}

export function BoardCard({
  entry,
  strings,
  locale,
  isDragging,
  moveDisabled,
  onOpenTask,
  onMoveTask,
  onDragStartTask,
  onDragEndTask,
}: BoardCardProps): ReactNode {
  const [menuOpen, setMenuOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const itemRefs = useRef<(HTMLButtonElement | null)[]>([]);

  useEffect(() => {
    if (!menuOpen) return undefined;
    const onPointerDown = (event: PointerEvent): void => {
      const target = event.target as Node | null;
      if (target && !menuRef.current?.contains(target) && !triggerRef.current?.contains(target)) {
        setMenuOpen(false);
      }
    };
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        setMenuOpen(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [menuOpen]);

  const { task, latest_run: run, cost_summary: cost } = entry;
  const ownDecision = hasOwnPendingDecision(entry);
  const runStatus = normalizeRunStatusValue(run?.status);

  if (isDragging) {
    return (
      <div className="rl-drop-source" data-testid={`rl-card-dragging-${task.id}`}>
        <PinIconSvg />
        <span>{task.title}</span>
      </div>
    );
  }

  const handleMenuKeyDown = (event: React.KeyboardEvent<HTMLDivElement>): void => {
    const enabled = itemRefs.current.filter(
      (el): el is HTMLButtonElement => el !== null && !el.disabled,
    );
    if (enabled.length === 0) return;
    const idx = enabled.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === "ArrowDown") {
      event.preventDefault();
      enabled[(idx + 1) % enabled.length]?.focus();
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      enabled[(idx - 1 + enabled.length) % enabled.length]?.focus();
    } else if (event.key === "Home") {
      event.preventDefault();
      enabled[0]?.focus();
    } else if (event.key === "End") {
      event.preventDefault();
      enabled[enabled.length - 1]?.focus();
    }
  };

  return (
    <article
      className={ownDecision ? "rl-board-card rl-board-card-decision" : "rl-board-card"}
      data-testid={`rl-board-card-${task.id}`}
      data-decision-own={ownDecision ? "true" : "false"}
    >
      <div className="rl-board-card-top">
        <button
          type="button"
          className="rl-card-title-btn"
          onClick={() => onOpenTask(task.id)}
          data-testid={`rl-card-open-${task.id}`}
        >
          {task.title}
        </button>

        <div className="rl-card-controls">
          <button
            type="button"
            className="rl-icon-btn rl-drag-handle"
            draggable={!moveDisabled}
            aria-label={strings.cardDragHandle(task.title)}
            onDragStart={(e) => {
              e.dataTransfer.setData("text/plain", task.id);
              e.dataTransfer.effectAllowed = "move";
              onDragStartTask(task.id);
            }}
            onDragEnd={onDragEndTask}
            data-testid={`rl-card-drag-${task.id}`}
          >
            <DragGripSvg />
          </button>

          <div className="rl-menu-anchor">
            <button
              ref={triggerRef}
              type="button"
              className="rl-icon-btn"
              aria-label={strings.cardMoveMenu(task.title)}
              aria-haspopup="menu"
              aria-expanded={menuOpen}
              onClick={() => setMenuOpen((prev) => !prev)}
              data-testid={`rl-card-menu-${task.id}`}
            >
              <MoreDotsSvg />
            </button>

            {menuOpen ? (
              <div
                ref={menuRef}
                role="menu"
                aria-label={strings.cardMoveMenu(task.title)}
                className="rl-menu-panel"
                tabIndex={-1}
                onKeyDown={handleMenuKeyDown}
              >
                <button
                  ref={(el) => {
                    itemRefs.current[0] = el;
                  }}
                  type="button"
                  role="menuitem"
                  className="rl-menu-item"
                  onClick={() => {
                    setMenuOpen(false);
                    onOpenTask(task.id);
                  }}
                >
                  <span>{strings.openTaskLabel}</span>
                </button>
                {TASK_STAGES.map((stage, i) => {
                  const isCurrent = stage === task.stage;
                  const stageText = taskStageLabel(stage, strings);
                  return (
                    <button
                      key={stage}
                      ref={(el) => {
                        itemRefs.current[i + 1] = el;
                      }}
                      type="button"
                      role="menuitem"
                      className="rl-menu-item"
                      disabled={moveDisabled || isCurrent}
                      onClick={() => {
                        setMenuOpen(false);
                        onMoveTask(task.id, stage, task.revision);
                      }}
                      data-testid={`rl-move-${task.id}-${stage}`}
                    >
                      <span>{strings.moveToLabel(stageText)}</span>
                      {isCurrent ? (
                        <span className="rl-meta-label">{strings.moveDisabledHint}</span>
                      ) : null}
                    </button>
                  );
                })}
              </div>
            ) : null}
          </div>
        </div>
      </div>

      <div className="rl-card-meta-row">
        <span className="rl-meta-value">{task.id}</span>
        <StatusBadge
          status="queued"
          customTone={task.kind === "quick_fix" ? "attention" : "neutral"}
          customLabel={task.kind === "quick_fix" ? strings.kindQuickFix : strings.kindStandard}
        />
      </div>

      <div className="rl-card-meta-row">
        <span className="rl-meta-label">{workflowStageLabel(task.workflow_stage, strings)}</span>
        {run ? (
          <StatusBadge
            status={runStatus}
            customLabel={localizedRunStatusLabel(runStatus, strings)}
          />
        ) : null}
      </div>

      {ownDecision ? (
        <StatusBadge status="waiting_for_approval" customLabel={strings.decisionPendingLabel} />
      ) : null}

      <div className="rl-card-meta-row">
        <span className="rl-figure" data-testid={`rl-card-cost-${task.id}`}>
          {formatMicroUsd(cost.known_micro_usd, locale)}
        </span>
        {cost.unknown_attempts > 0 ? (
          <StatusBadge
            status="unknown"
            customTone="unknown"
            customLabel={strings.unknownCostAttempts(cost.unknown_attempts)}
          />
        ) : run ? (
          <span className="rl-meta-value">
            {strings.repairAttemptsLabel}: {run.repair_attempts}/{run.budget_policy.max_attempts}
          </span>
        ) : null}
      </div>
    </article>
  );
}
