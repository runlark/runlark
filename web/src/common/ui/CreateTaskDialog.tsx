import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from "react";
import type { TaskKind } from "../board-types";
import type { RunlarkStrings } from "../i18n";
import { ActionButton } from "./ActionButton";

export interface CreateTaskDialogProps {
  readonly open: boolean;
  readonly initialKind: TaskKind;
  readonly submitting: boolean;
  readonly submitError: string | null;
  readonly strings: RunlarkStrings;
  readonly onClose: () => void;
  readonly onSubmit: (input: {
    readonly title: string;
    readonly description: string;
    readonly kind: TaskKind;
  }) => void;
}

export function CreateTaskDialog({
  open,
  initialKind,
  submitting,
  submitError,
  strings,
  onClose,
  onSubmit,
}: CreateTaskDialogProps): ReactNode {
  const dialogRef = useRef<HTMLDialogElement | null>(null);
  const headingId = useId();

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open) {
      if (!dialog.open) {
        dialog.showModal();
      }
    } else if (dialog.open) {
      dialog.close();
    }
  }, [open]);

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
        if (submitting) {
          e.preventDefault();
          return;
        }
        e.preventDefault();
        onClose();
      }}
      data-testid="rl-create-task-dialog"
    >
      <CreateTaskFormBody
        key={open ? `open-${initialKind}` : "closed"}
        headingId={headingId}
        initialKind={initialKind}
        submitting={submitting}
        submitError={submitError}
        strings={strings}
        onClose={onClose}
        onSubmit={onSubmit}
      />
    </dialog>
  );
}

function CreateTaskFormBody({
  headingId,
  initialKind,
  submitting,
  submitError,
  strings,
  onClose,
  onSubmit,
}: Omit<CreateTaskDialogProps, "open"> & { readonly headingId: string }): ReactNode {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [kind, setKind] = useState<TaskKind>(initialKind);
  const [validationError, setValidationError] = useState<string | null>(null);

  const kindId = useId();
  const titleId = useId();
  const helpId = useId();
  const errId = useId();
  const descId = useId();

  const handleSubmit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const trimmed = title.trim();
    if (!trimmed) {
      setValidationError(strings.createTaskTitleRequired);
      return;
    }
    setValidationError(null);
    onSubmit({ title: trimmed, description: description.trim(), kind });
  };

  const displayError = validationError ?? submitError;

  return (
    <form onSubmit={handleSubmit} className="rl-field" style={{ gap: "0.75rem" }} noValidate>
      <h2 id={headingId} className="rl-section-heading">
        {strings.createTaskHeading}
      </h2>

      <div className="rl-field">
        <label htmlFor={kindId} className="rl-label">
          {strings.createTaskKindLabel}
        </label>
        <select
          id={kindId}
          className="rl-toolbar-select"
          value={kind}
          disabled={submitting}
          onChange={(e) => setKind(e.currentTarget.value as TaskKind)}
        >
          <option value="standard">{strings.kindStandard}</option>
          <option value="quick_fix">{strings.kindQuickFix}</option>
        </select>
      </div>

      <div className="rl-field">
        <label htmlFor={titleId} className="rl-label">
          {strings.createTaskTitleLabel}
        </label>
        <input
          id={titleId}
          type="text"
          className="rl-input"
          value={title}
          disabled={submitting}
          onChange={(e) => {
            setTitle(e.currentTarget.value);
            if (validationError) setValidationError(null);
          }}
          aria-invalid={displayError ? "true" : undefined}
          aria-describedby={displayError ? `${helpId} ${errId}` : helpId}
        />
        <span id={helpId} className="rl-meta-label">
          {strings.createTaskTitleHelp}
        </span>
      </div>

      <div className="rl-field">
        <label htmlFor={descId} className="rl-label">
          {strings.createTaskDescLabel}
        </label>
        <textarea
          id={descId}
          className="rl-input"
          rows={3}
          value={description}
          disabled={submitting}
          onChange={(e) => setDescription(e.currentTarget.value)}
        />
      </div>

      {displayError ? (
        <div id={errId} className="rl-error-box" role="alert">
          {displayError}
        </div>
      ) : null}

      <div className="rl-actions-row">
        <ActionButton
          type="submit"
          variant="primary"
          label={strings.createTaskSubmit}
          disabled={submitting}
          testId="rl-create-task-submit"
        />
        <ActionButton
          variant="secondary"
          label={strings.cancelBtn}
          disabled={submitting}
          onClick={onClose}
        />
      </div>
    </form>
  );
}
