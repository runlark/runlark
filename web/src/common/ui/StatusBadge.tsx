import { Badge } from "@mantine/core";
import type { ReactNode } from "react";
import type { RunStatus } from "../runlark-types";

export type SemanticTone = "neutral" | "attention" | "success" | "danger" | "unknown";

export interface StatusPresentation {
  readonly label: string;
  readonly tone: SemanticTone;
}

const RUN_STATUS_PRESENTATION: Readonly<Record<RunStatus, StatusPresentation>> = {
  queued: { label: "Queued", tone: "neutral" },
  running: { label: "Running", tone: "attention" },
  waiting_for_approval: { label: "Waiting for approval", tone: "attention" },
  cancel_requested: { label: "Cancel requested", tone: "attention" },
  stopped: { label: "Stopped", tone: "neutral" },
  completed: { label: "Completed", tone: "success" },
  failed: { label: "Failed", tone: "danger" },
  interrupted: { label: "Interrupted · unknown", tone: "unknown" },
  unknown: { label: "Unknown", tone: "unknown" },
};

export interface StatusBadgeProps {
  readonly status: RunStatus;
  readonly customLabel?: string;
  readonly customTone?: SemanticTone;
}

export function StatusBadge({ status, customLabel, customTone }: StatusBadgeProps): ReactNode {
  const meta = RUN_STATUS_PRESENTATION[status] ?? RUN_STATUS_PRESENTATION.unknown;
  const tone = customTone ?? meta.tone;
  const label = customLabel ?? meta.label;

  return (
    <Badge
      unstyled
      component="span"
      className={`rl-badge rl-badge-${tone}`}
      data-run-status={status}
      data-tone={tone}
    >
      <span className="rl-badge-dot" aria-hidden="true" />
      <span>{label}</span>
    </Badge>
  );
}
