import type { ReactNode } from "react";
import { formatEventTime, summarizeEventPayload, type DaemonEventItem } from "../runlark-types";

export interface EventFeedListProps {
  readonly events: readonly DaemonEventItem[];
  readonly loading: boolean;
  readonly streamError: string | null;
  readonly lastSeq: number | null;
}

export function EventFeedList({
  events,
  loading,
  streamError,
  lastSeq,
}: EventFeedListProps): ReactNode {
  return (
    <section className="rl-card" aria-labelledby="rl-events-heading">
      <header className="rl-card-header">
        <h3 id="rl-events-heading" className="rl-section-heading">
          Live event stream
        </h3>
        <span className="rl-meta-value" aria-live="polite" data-testid="rl-event-cursor">
          {lastSeq === null ? "No events" : `seq #${String(lastSeq)} · ${String(events.length)}`}
        </span>
      </header>

      {streamError ? (
        <div className="rl-error-box" role="alert" data-testid="rl-stream-error">
          {streamError}
        </div>
      ) : null}

      {loading && events.length === 0 ? (
        <p className="rl-status-line">Loading run event history...</p>
      ) : events.length === 0 ? (
        <p className="rl-status-line" data-testid="rl-events-empty">
          No run events persisted in SQLite yet.
        </p>
      ) : (
        <ul className="rl-event-list" data-testid="rl-event-list">
          {events.map((event) => {
            const summary = summarizeEventPayload(event);
            const key = `${event.run_id ?? "run"}-${String(event.seq)}-${event.event_type}`;
            return (
              <li key={key} className="rl-event-row" data-event-seq={event.seq}>
                <span className="rl-event-seq">#{String(event.seq)}</span>
                <span className="rl-event-time">{formatEventTime(event.created_at)}</span>
                <span className="rl-event-type">{event.event_type}</span>
                {summary ? <p className="rl-event-detail">{summary}</p> : null}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
