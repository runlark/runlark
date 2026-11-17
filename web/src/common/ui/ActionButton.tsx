import { Button } from "@mantine/core";
import type { ReactNode } from "react";

export interface ActionButtonProps {
  readonly label: string;
  readonly variant?: "primary" | "secondary" | "danger";
  readonly type?: "button" | "submit";
  readonly disabled?: boolean;
  readonly disabledReasonId?: string;
  readonly onClick?: () => void;
  readonly testId?: string;
}

export function ActionButton({
  label,
  variant = "secondary",
  type = "button",
  disabled = false,
  disabledReasonId,
  onClick,
  testId,
}: ActionButtonProps): ReactNode {
  const toneClass =
    variant === "primary"
      ? "rl-btn rl-btn-primary"
      : variant === "danger"
        ? "rl-btn rl-btn-danger"
        : "rl-btn";

  return (
    <Button
      unstyled
      type={type}
      className={toneClass}
      disabled={disabled}
      aria-describedby={disabled && disabledReasonId ? disabledReasonId : undefined}
      onClick={onClick}
      data-testid={testId}
    >
      {label}
    </Button>
  );
}
