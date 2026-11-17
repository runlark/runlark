import { useId, useState, type FormEvent, type ReactNode } from "react";
import type { DaemonQuestion } from "../runlark-types";
import { ActionButton } from "./ActionButton";
import { StatusBadge } from "./StatusBadge";

export interface QuestionDecisionCardProps {
  readonly question: DaemonQuestion;
  readonly submitting: boolean;
  readonly submitError: string | null;
  readonly onSubmitAnswer: (questionId: string, answer: string) => void;
}

export function QuestionDecisionCard({
  question,
  submitting,
  submitError,
  onSubmitAnswer,
}: QuestionDecisionCardProps): ReactNode {
  const hasOptions = question.options.length > 0;
  const [selectedOption, setSelectedOption] = useState<string>(question.options[0] ?? "");
  const [customAnswer, setCustomAnswer] = useState("");
  const [validationError, setValidationError] = useState<string | null>(null);

  const formHeadingId = useId();
  const customInputId = useId();
  const errorId = useId();
  const radioGroupName = useId();

  const handleSubmit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const trimmedCustom = customAnswer.trim();
    const chosen = trimmedCustom.length > 0 ? trimmedCustom : selectedOption.trim();
    if (!chosen) {
      setValidationError("Select an option or enter a response before submitting.");
      return;
    }
    setValidationError(null);
    onSubmitAnswer(question.question_id, chosen);
  };

  const displayError = validationError ?? submitError;

  return (
    <article
      className="rl-card rl-card-decision"
      aria-labelledby={formHeadingId}
      data-testid="rl-question-card"
    >
      <div className="rl-attention-strip">
        <span>Human decision required</span>
        <StatusBadge status="waiting_for_approval" />
      </div>

      <header className="rl-card-header">
        <h3 id={formHeadingId} className="rl-card-title">
          {question.prompt}
        </h3>
        <span className="rl-meta-value" data-testid="rl-question-id">
          {question.question_id}
        </span>
      </header>

      <form onSubmit={handleSubmit} className="rl-field" aria-labelledby={formHeadingId} noValidate>
        {hasOptions ? (
          <fieldset className="rl-fieldset">
            <legend className="rl-legend">Choose an option</legend>
            {question.options.map((option, idx) => {
              const optionId = `${radioGroupName}-${String(idx)}`;
              return (
                <label key={optionId} htmlFor={optionId} className="rl-option-label">
                  <input
                    id={optionId}
                    type="radio"
                    name={radioGroupName}
                    value={option}
                    checked={selectedOption === option && customAnswer.trim().length === 0}
                    onChange={() => {
                      setSelectedOption(option);
                      setCustomAnswer("");
                      if (validationError) {
                        setValidationError(null);
                      }
                    }}
                    disabled={submitting}
                    className="rl-option-radio"
                  />
                  <span>{option}</span>
                </label>
              );
            })}
          </fieldset>
        ) : null}

        <div className="rl-field">
          <label htmlFor={customInputId} className="rl-label">
            {hasOptions ? "Or write a custom answer" : "Answer"}
          </label>
          <input
            id={customInputId}
            name="customAnswer"
            type="text"
            className="rl-input"
            value={customAnswer}
            onChange={(e) => {
              setCustomAnswer(e.currentTarget.value);
              if (validationError) {
                setValidationError(null);
              }
            }}
            disabled={submitting}
            aria-invalid={displayError ? "true" : undefined}
            aria-describedby={displayError ? errorId : undefined}
            placeholder="Enter decision response"
          />
        </div>

        {displayError ? (
          <div id={errorId} className="rl-error-box" role="alert" data-testid="rl-question-error">
            {displayError}
          </div>
        ) : null}

        <footer className="rl-actions-row">
          <ActionButton
            type="submit"
            variant="primary"
            label={submitting ? "Submitting answer..." : "Submit answer"}
            disabled={submitting}
            testId="rl-btn-answer-submit"
          />
        </footer>
      </form>
    </article>
  );
}
