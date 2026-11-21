import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from "react";
import type { ProjectSummary } from "../board-types";
import type { TeamStrings } from "../team-i18n";
import type { TeamWorkspaceRecord } from "../team-types";
import { ActionButton } from "./ActionButton";

export interface PromoteTeamDialogProps {
  readonly open: boolean;
  readonly project: ProjectSummary | null;
  readonly blockingRunIds: readonly string[];
  readonly workspaces: readonly TeamWorkspaceRecord[];
  readonly submitting: boolean;
  readonly submitError: string | null;
  readonly strings: TeamStrings;
  readonly onClose: () => void;
  readonly onPromote: (input: { readonly projectId: string; readonly workspaceId: string }) => void;
}

export function PromoteTeamDialog({
  open,
  project,
  blockingRunIds,
  workspaces,
  submitting,
  submitError,
  strings,
  onClose,
  onPromote,
}: PromoteTeamDialogProps): ReactNode {
  const dialogRef = useRef<HTMLDialogElement | null>(null);
  const headingId = useId();

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && project) {
      if (!dialog.open) {
        dialog.showModal();
      }
    } else if (dialog.open) {
      dialog.close();
    }
  }, [open, project]);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog || !open) return undefined;
    const onBackdropClick = (e: MouseEvent): void => {
      if (e.target === dialog && !submitting) {
        onClose();
      }
    };
    dialog.addEventListener("click", onBackdropClick);
    return () => {
      dialog.removeEventListener("click", onBackdropClick);
    };
  }, [open, submitting, onClose]);

  return (
    <dialog
      ref={dialogRef}
      className="rl-dialog"
      aria-labelledby={headingId}
      onCancel={(e) => {
        e.preventDefault();
        if (!submitting) onClose();
      }}
      data-testid="rl-promote-team-dialog"
    >
      <PromoteFormBody
        key={open && project ? `promote-${project.project_id}` : "closed"}
        headingId={headingId}
        project={project}
        blockingRunIds={blockingRunIds}
        workspaces={workspaces}
        submitting={submitting}
        submitError={submitError}
        strings={strings}
        onClose={onClose}
        onPromote={onPromote}
      />
    </dialog>
  );
}

function PromoteFormBody({
  headingId,
  project,
  blockingRunIds,
  workspaces,
  submitting,
  submitError,
  strings,
  onClose,
  onPromote,
}: Omit<PromoteTeamDialogProps, "open"> & { readonly headingId: string }): ReactNode {
  const [workspaceId, setWorkspaceId] = useState(workspaces[0]?.workspace.id ?? "");
  const [validationError, setValidationError] = useState<string | null>(null);
  const wsInputId = useId();
  const blockedHintId = useId();
  const errId = useId();

  const blocked = blockingRunIds.length > 0;
  const effectiveWorkspaceId =
    workspaces.length > 0
      ? workspaces.some((w) => w.workspace.id === workspaceId)
        ? workspaceId
        : (workspaces[0]?.workspace.id ?? "")
      : workspaceId;
  const displayError = validationError ?? submitError;

  const handleSubmit = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (!project || blocked) return;
    if (!effectiveWorkspaceId.trim()) {
      setValidationError(strings.fieldRequiredError);
      return;
    }
    setValidationError(null);
    onPromote({ projectId: project.project_id, workspaceId: effectiveWorkspaceId.trim() });
  };

  return (
    <form onSubmit={handleSubmit} className="rl-field" style={{ gap: "0.75rem" }} noValidate>
      <h2 id={headingId} className="rl-section-heading">
        {strings.promoteDialogTitle}
      </h2>

      {project ? (
        <p className="rl-meta-value">
          {project.name} ({project.project_id})
        </p>
      ) : null}

      {blocked ? (
        <div
          id={blockedHintId}
          className="rl-error-box"
          role="alert"
          data-testid="rl-promote-blocked-notice"
        >
          <p style={{ margin: 0 }}>{strings.promoteBlockedNotice(blockingRunIds.length)}</p>
          <p className="rl-meta-value" style={{ marginTop: "0.25rem" }}>
            {blockingRunIds.join(", ")}
          </p>
        </div>
      ) : (
        <p className="rl-notice-unknown" style={{ margin: 0 }}>
          {strings.promoteSanitizeNotice}
        </p>
      )}

      <div className="rl-field">
        <label htmlFor={wsInputId} className="rl-label">
          {strings.promoteWorkspaceLabel}
        </label>
        {workspaces.length > 0 ? (
          <select
            id={wsInputId}
            className="rl-toolbar-select"
            value={effectiveWorkspaceId}
            disabled={submitting || blocked}
            onChange={(e) => {
              setWorkspaceId(e.currentTarget.value);
              if (validationError) setValidationError(null);
            }}
            data-testid="rl-promote-workspace-select"
          >
            {workspaces.map((w) => (
              <option key={w.workspace.id} value={w.workspace.id}>
                {w.workspace.name} ({w.workspace.id})
              </option>
            ))}
          </select>
        ) : (
          <input
            id={wsInputId}
            type="text"
            className="rl-input rl-mono"
            value={workspaceId}
            disabled={submitting || blocked}
            aria-invalid={displayError ? "true" : "false"}
            aria-describedby={displayError ? errId : undefined}
            onChange={(e) => {
              setWorkspaceId(e.currentTarget.value);
              if (validationError) setValidationError(null);
            }}
            data-testid="rl-promote-workspace-input"
          />
        )}
      </div>

      {displayError ? (
        <div id={errId} className="rl-error-box" role="alert" data-testid="rl-promote-error">
          {displayError}
        </div>
      ) : null}

      <div className="rl-actions-row">
        <ActionButton
          variant="secondary"
          label={strings.cancelBtn}
          disabled={submitting}
          onClick={onClose}
          testId="rl-promote-cancel-btn"
        />
        <ActionButton
          type="submit"
          variant="primary"
          label={strings.promoteConfirmBtn}
          disabled={submitting || blocked || !project}
          {...(blocked ? { disabledReasonId: blockedHintId } : {})}
          testId="rl-promote-confirm-btn"
        />
      </div>
    </form>
  );
}
