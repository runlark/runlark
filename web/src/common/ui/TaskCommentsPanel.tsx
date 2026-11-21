import { useId, useState, type FormEvent, type KeyboardEvent, type ReactNode } from "react";
import type { TeamStrings } from "../team-i18n";
import type { OfflineDraft, TaskComment } from "../team-types";
import { ActionButton } from "./ActionButton";
import { StatusBadge } from "./StatusBadge";

export interface TaskCommentsPanelProps {
  readonly taskId: string;
  readonly comments: readonly TaskComment[];
  readonly commentDrafts: readonly OfflineDraft[];
  readonly queuedOfflineNotice: boolean;
  readonly busy: boolean;
  readonly strings: TeamStrings;
  readonly onPostComment: (body: string) => Promise<boolean> | boolean;
  readonly onRetryDraft: (draftId: string) => void;
  readonly onDeleteDraft: (draftId: string) => void;
}

function parseDraftBody(payloadJson: string): string {
  try {
    const parsed = JSON.parse(payloadJson) as { readonly body?: unknown };
    return typeof parsed.body === "string" ? parsed.body : payloadJson;
  } catch {
    return payloadJson;
  }
}

export function TaskCommentsPanel({
  taskId,
  comments,
  commentDrafts,
  queuedOfflineNotice,
  busy,
  strings,
  onPostComment,
  onRetryDraft,
  onDeleteDraft,
}: TaskCommentsPanelProps): ReactNode {
  const [body, setBody] = useState("");
  const [validationError, setValidationError] = useState<string | null>(null);
  const textareaId = useId();
  const errId = useId();

  const submitBody = async (): Promise<void> => {
    const trimmed = body.trim();
    if (!trimmed) {
      setValidationError(strings.fieldRequiredError);
      return;
    }
    setValidationError(null);
    const ok = await onPostComment(trimmed);
    if (ok) {
      setBody("");
    }
  };

  const handleSubmit = (e: FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    void submitBody();
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>): void => {
    if (e.nativeEvent.isComposing) return;
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter" && !busy) {
      e.preventDefault();
      void submitBody();
    }
  };

  return (
    <div
      className="rl-field"
      style={{ gap: "0.75rem" }}
      data-testid={`rl-comments-panel-${taskId}`}
    >
      <article className="rl-card">
        <header className="rl-card-header">
          <h2 className="rl-section-heading">{strings.commentsHeading(comments.length)}</h2>
          <span className="rl-meta-label">{strings.commentsImmutableHint}</span>
        </header>

        {comments.length === 0 ? (
          <p className="rl-status-line">{strings.commentsEmpty}</p>
        ) : (
          <ol className="rl-team-list" data-testid="rl-comments-list">
            {comments.map((cmt) => (
              <li key={cmt.id} className="rl-comment-item" data-testid={`rl-comment-${cmt.id}`}>
                <div className="rl-comment-head">
                  <span className="rl-comment-author">
                    {cmt.author_display_name}{" "}
                    <span className="rl-meta-value">({cmt.author_user_id})</span>
                  </span>
                  <span className="rl-meta-value">
                    {cmt.run_id ? `${cmt.run_id} · ` : ""}
                    {new Date(cmt.created_at).toISOString()}
                  </span>
                </div>
                <p className="rl-comment-body">{cmt.body}</p>
              </li>
            ))}
          </ol>
        )}

        {queuedOfflineNotice ? (
          <p
            className="rl-notice-unknown"
            style={{ margin: 0 }}
            data-testid="rl-comment-queued-notice"
          >
            {strings.commentQueuedOfflineNotice}
          </p>
        ) : null}

        <form onSubmit={handleSubmit} className="rl-field" noValidate data-testid="rl-comment-form">
          <label htmlFor={textareaId} className="rl-label">
            {strings.commentBodyLabel}
          </label>
          <textarea
            id={textareaId}
            rows={3}
            className="rl-input"
            value={body}
            disabled={busy}
            aria-invalid={validationError ? "true" : "false"}
            aria-describedby={validationError ? errId : undefined}
            onKeyDown={handleKeyDown}
            onChange={(e) => {
              setBody(e.currentTarget.value);
              if (validationError) setValidationError(null);
            }}
            data-testid="rl-comment-input"
          />
          {validationError ? (
            <div id={errId} className="rl-error-box" role="alert">
              {validationError}
            </div>
          ) : null}
          <div className="rl-actions-row">
            <ActionButton
              type="submit"
              variant="primary"
              label={strings.commentSubmitBtn}
              disabled={busy}
              testId="rl-comment-submit"
            />
          </div>
        </form>
      </article>

      {commentDrafts.length > 0 ? (
        <article className="rl-card" data-testid="rl-comment-drafts-card">
          <h3 className="rl-section-heading">{strings.commentDraftsHeading}</h3>
          <ul className="rl-team-list">
            {commentDrafts.map((draft) => {
              const isConflict = draft.sync_state.state === "conflict";
              const label =
                draft.sync_state.state === "conflict"
                  ? strings.draftStateConflict(draft.sync_state.server_revision)
                  : draft.sync_state.state === "synced"
                    ? strings.draftStateSynced
                    : strings.draftStatePending;
              return (
                <li
                  key={draft.id}
                  className={
                    isConflict ? "rl-draft-block rl-draft-block-conflict" : "rl-draft-block"
                  }
                  data-testid={`rl-comment-draft-${draft.id}`}
                >
                  <div className="rl-artifact-head">
                    <span className="rl-meta-value">{draft.id}</span>
                    <StatusBadge
                      status="queued"
                      customTone={isConflict ? "danger" : "attention"}
                      customLabel={label}
                    />
                  </div>
                  <p className="rl-comment-body">{parseDraftBody(draft.payload_json)}</p>
                  {draft.sync_state.state !== "synced" ? (
                    <div className="rl-actions-row">
                      <ActionButton
                        variant="secondary"
                        label={strings.draftRetryBtn}
                        disabled={busy}
                        onClick={() => onRetryDraft(draft.id)}
                        testId={`rl-comment-draft-retry-${draft.id}`}
                      />
                      <ActionButton
                        variant="danger"
                        label={strings.draftDiscardBtn}
                        disabled={busy}
                        onClick={() => onDeleteDraft(draft.id)}
                        testId={`rl-comment-draft-delete-${draft.id}`}
                      />
                    </div>
                  ) : null}
                </li>
              );
            })}
          </ul>
        </article>
      ) : null}
    </div>
  );
}
