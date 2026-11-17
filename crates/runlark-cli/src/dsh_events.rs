use crate::state::AppState;
use runlark_domain::{
    AnswerQuestionCommand, CommandId, QuestionId, QuestionStatus, RunId, RunStatus, RuntimeEvent,
    SessionCommand, SessionId, SessionStatus, Timestamp, TurnEndStatus,
};
use runlark_local::{
    CommandStore, Connection, DshCursorStore, EventStore, LocalStore, QuestionStore,
    RuntimeApplyOutcome, SessionStore, StorageError, StoredEvent,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Raw `SessionWireEvent` payload emitted by DSH `session/event` and `session/follow`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DshWireEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub seq: u64,
    #[serde(default)]
    pub time: u64,
    #[serde(default)]
    pub data: Value,
}

/// Envelope used when `@runlark/dsh-plugin` forwards `SessionWireEvent`s over HTTP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DshWireEventEnvelope {
    pub session_id: String,
    #[serde(default)]
    pub run_id: Option<String>,
    pub event: DshWireEvent,
}

/// Applies a domain `RuntimeEvent`, reconciles matching `dispatched` outbox commands,
/// and broadcasts persisted events to stream listeners.
///
/// # Errors
/// Returns `StorageError` if `SQLite` transaction or reconciliation fails.
pub fn apply_runtime_event_and_reconcile(
    conn: &mut Connection,
    state: &AppState,
    event: &RuntimeEvent,
    now: Timestamp,
) -> Result<RuntimeApplyOutcome, StorageError> {
    let outcome = LocalStore::apply_runtime_event(conn, event, now)?;

    let dispatched = CommandStore::list_dispatched_for_run(conn, &outcome.run.id)?;
    for cmd in dispatched {
        let should_ack = match event {
            RuntimeEvent::TurnEnded { .. } => true,
            RuntimeEvent::TurnStarted { .. } => matches!(
                cmd.command_type.as_str(),
                "send_prompt" | "resume_session" | "answer_question"
            ),
            RuntimeEvent::UserQuestionAsked { .. } => false,
        };
        if should_ack {
            CommandStore::mark_acked(conn, &cmd.command_id, now)?;
        }
    }

    state.publish_events(&outcome.session.id.0, &outcome.events);
    state.notify_scheduler();
    Ok(outcome)
}

fn is_terminal_status(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Stopped | RunStatus::Completed | RunStatus::Failed
    )
}

fn resolve_target_run_id(
    conn: &Connection,
    session_id: &SessionId,
    run_id_hint: Option<&str>,
    wire_event: &DshWireEvent,
    now: Timestamp,
) -> Result<Option<RunId>, StorageError> {
    if wire_event.event_type == "user/message"
        && let Some(rpc_id) = wire_event
            .data
            .get("source")
            .and_then(|s| s.get("rpcId"))
            .and_then(Value::as_str)
    {
        let cmd_id = CommandId::from(rpc_id);
        if let Some(cmd) = CommandStore::get_command(conn, &cmd_id)? {
            if matches!(cmd.status.as_str(), "pending" | "dispatched" | "failed") {
                CommandStore::mark_acked(conn, &cmd.command_id, now)?;
            }
            return Ok(Some(cmd.run_id));
        }
    }

    if let Some(hint) = run_id_hint {
        return Ok(Some(RunId::from(hint)));
    }

    if let Some(session) = SessionStore::get_session(conn, session_id)?
        && let Some(active) = session.active_run_id
    {
        return Ok(Some(active));
    }

    let runs = SessionStore::list_runs_for_session(conn, session_id)?;
    Ok(runs
        .into_iter()
        .rev()
        .find(|r| !is_terminal_status(r.status))
        .map(|r| r.id))
}

fn parse_turn_end_status(data: &Value) -> TurnEndStatus {
    let kind = data
        .get("reason")
        .and_then(|r| r.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("completed");

    match kind {
        "completed" | "max-tokens" => TurnEndStatus::Completed,
        "aborted" | "blocked" | "interrupted" | "forked" => TurnEndStatus::Stopped,
        "error" => {
            let msg = data
                .get("reason")
                .and_then(|r| r.get("error"))
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("DSH turn failed")
                .to_string();
            TurnEndStatus::Failed { error: msg }
        }
        other => TurnEndStatus::Failed {
            error: format!("DSH turn ended with reason '{other}'"),
        },
    }
}

fn parse_ask_user_question_call(data: &Value) -> Option<(QuestionId, String, Vec<String>)> {
    if data.get("name").and_then(Value::as_str) != Some("ask_user_question") {
        return None;
    }
    let call_id = data.get("callId").and_then(Value::as_str)?;
    let raw_args = data.get("arguments").and_then(Value::as_str)?;
    let parsed: Value = serde_json::from_str(raw_args).ok()?;
    let questions = parsed.get("questions")?.as_array()?;

    let mut prompts = Vec::new();
    let mut options = Vec::new();
    for item in questions {
        if let Some(q) = item.get("question").and_then(Value::as_str) {
            prompts.push(q);
        }
        if let Some(opts) = item.get("options").and_then(Value::as_array) {
            for opt in opts {
                if let Some(label) = opt.get("label").and_then(Value::as_str) {
                    options.push(label.to_string());
                }
            }
        }
    }
    if prompts.is_empty() {
        return None;
    }
    Some((QuestionId::from(call_id), prompts.join("\n"), options))
}

fn extract_question_reply(data: &Value) -> Option<(QuestionId, String)> {
    let source = data.get("source")?;
    if source.get("kind").and_then(Value::as_str) != Some("user-question-reply") {
        return None;
    }
    let call_id = source.get("callId").and_then(Value::as_str)?;
    let text: String = data
        .get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    if text.trim().is_empty() {
        return None;
    }

    let formatted = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| {
            let answers = v.get("answers")?.as_array()?;
            if answers.len() == 1 {
                let first = &answers[0];
                if let Some(custom) = first.get("custom").and_then(Value::as_str)
                    && !custom.trim().is_empty()
                {
                    return Some(custom.to_string());
                }
                if let Some(selected) = first.get("selected").and_then(Value::as_array) {
                    let labels: Vec<&str> = selected
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                        .collect();
                    if !labels.is_empty() {
                        return Some(labels.join(", "));
                    }
                }
            }
            None
        })
        .unwrap_or(text);

    if formatted.trim().is_empty() {
        return None;
    }

    Some((QuestionId::from(call_id), formatted))
}

fn wire_event_label(event_type: &str) -> &'static str {
    match event_type {
        "turn/start" => "DshTurnStarted",
        "turn/end" => "DshTurnEnded",
        "user/message" => "DshUserMessage",
        "assistant/message" => "DshAssistantMessage",
        "assistant/attempt" => "DshAssistantAttempt",
        "tool/call" => "DshToolCall",
        "tool/result" => "DshToolResult",
        _ => "DshWireEvent",
    }
}

fn record_wire_event(
    conn: &Connection,
    state: &AppState,
    session_id: &SessionId,
    run_id: &RunId,
    wire_event: &DshWireEvent,
    now: Timestamp,
) -> Result<StoredEvent, StorageError> {
    let payload = serde_json::to_string(wire_event)?;
    let stored = EventStore::append_wire_event(
        conn,
        run_id,
        wire_event_label(&wire_event.event_type),
        &payload,
        now,
    )?;
    state.publish_events(&session_id.0, std::slice::from_ref(&stored));
    Ok(stored)
}

fn handle_wire_turn_end(
    conn: &mut Connection,
    state: &AppState,
    session_id: &SessionId,
    run_id: RunId,
    run_opt: Option<&runlark_domain::Run>,
    wire_event: &DshWireEvent,
    now: Timestamp,
) -> Result<Vec<StoredEvent>, StorageError> {
    let mut emitted = vec![record_wire_event(
        conn, state, session_id, &run_id, wire_event, now,
    )?];
    let active_non_terminal = run_opt.is_some_and(|r| !is_terminal_status(r.status));
    let reason_kind = wire_event
        .data
        .get("reason")
        .and_then(|r| r.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("completed");
    // DSH crash-recovery emits synthetic turn/end(interrupted) when resuming a cold
    // crashed turn; ignore it unless the user explicitly requested cancellation.
    let is_synthetic_repair_close = reason_kind == "interrupted"
        && run_opt.is_some_and(|r| r.status != RunStatus::CancelRequested);
    if active_non_terminal && !is_synthetic_repair_close {
        let end_status = parse_turn_end_status(&wire_event.data);
        let outcome = apply_runtime_event_and_reconcile(
            conn,
            state,
            &RuntimeEvent::TurnEnded {
                session_id: session_id.clone(),
                run_id,
                outcome: end_status,
            },
            now,
        )?;
        emitted.extend(outcome.events);
    }
    Ok(emitted)
}

fn handle_wire_tool_call(
    conn: &mut Connection,
    state: &AppState,
    session_id: &SessionId,
    run_id: RunId,
    active_non_terminal: bool,
    wire_event: &DshWireEvent,
    now: Timestamp,
) -> Result<Vec<StoredEvent>, StorageError> {
    let mut emitted = vec![record_wire_event(
        conn, state, session_id, &run_id, wire_event, now,
    )?];
    if active_non_terminal
        && let Some((question_id, prompt, options)) = parse_ask_user_question_call(&wire_event.data)
        && QuestionStore::get_question(conn, &question_id)?.is_none()
    {
        let outcome = apply_runtime_event_and_reconcile(
            conn,
            state,
            &RuntimeEvent::UserQuestionAsked {
                session_id: session_id.clone(),
                run_id,
                question_id,
                prompt,
                options,
            },
            now,
        )?;
        emitted.extend(outcome.events);
    }
    Ok(emitted)
}

fn handle_wire_user_message(
    conn: &mut Connection,
    state: &AppState,
    session_id: &SessionId,
    run_id: RunId,
    active_non_terminal: bool,
    wire_event: &DshWireEvent,
    now: Timestamp,
) -> Result<Vec<StoredEvent>, StorageError> {
    let mut emitted = vec![record_wire_event(
        conn, state, session_id, &run_id, wire_event, now,
    )?];
    if active_non_terminal
        && let Some((question_id, answer)) = extract_question_reply(&wire_event.data)
        && let Some(question) = QuestionStore::get_question(conn, &question_id)?
        && question.status == QuestionStatus::Pending
    {
        let latest_sess = SessionStore::get_session(conn, session_id)?
            .ok_or_else(|| StorageError::SessionNotFound(session_id.clone()))?;
        let cmd = SessionCommand::AnswerQuestion(AnswerQuestionCommand {
            command_id: CommandId::from(format!("dsh_reply_{question_id}")),
            session_id: session_id.clone(),
            run_id,
            question_id,
            answer,
            expected_revision: latest_sess.current_revision,
            created_at: now,
        });
        if let runlark_local::CommandApplyOutcome::Applied { events, .. } =
            LocalStore::apply_command(conn, &cmd, now)?
        {
            CommandStore::mark_acked(conn, cmd.command_id(), now)?;
            state.publish_events(&session_id.0, &events);
            emitted.extend(events);
        }
    }
    Ok(emitted)
}

/// Processes one DSH `SessionWireEvent`, deduplicating against `dsh_cursors`, translating
/// lifecycle transitions (`turn/start`, `turn/end`, `ask_user_question`, `user-question-reply`),
/// reconciling dispatched outbox commands, and recording raw tool/message events.
///
/// # Errors
/// Returns `StorageError` on `SQLite` query/write errors or invalid domain transitions.
pub fn process_dsh_wire_event(
    conn: &mut Connection,
    state: &AppState,
    session_id: &SessionId,
    run_id_hint: Option<&str>,
    wire_event: &DshWireEvent,
    now: Timestamp,
) -> Result<Vec<StoredEvent>, StorageError> {
    if wire_event.seq > 0
        && let Some(cursor) = DshCursorStore::get_cursor(conn, session_id)?
        && wire_event.seq <= cursor
    {
        return Ok(Vec::new());
    }

    let Some(session) = SessionStore::get_session(conn, session_id)? else {
        return Err(StorageError::SessionNotFound(session_id.clone()));
    };

    if session.status == SessionStatus::Archived {
        if wire_event.seq > 0 {
            let _ = DshCursorStore::advance_cursor(conn, session_id, wire_event.seq, now)?;
        }
        return Ok(Vec::new());
    }

    let target_run_id = resolve_target_run_id(conn, session_id, run_id_hint, wire_event, now)?;
    let mut emitted = Vec::new();

    if let Some(run_id) = target_run_id {
        let run_opt = SessionStore::get_run(conn, &run_id)?;
        let active_non_terminal = run_opt
            .as_ref()
            .is_some_and(|r| !is_terminal_status(r.status));

        match wire_event.event_type.as_str() {
            "turn/start" => {
                if active_non_terminal {
                    let outcome = apply_runtime_event_and_reconcile(
                        conn,
                        state,
                        &RuntimeEvent::TurnStarted {
                            session_id: session_id.clone(),
                            run_id: run_id.clone(),
                        },
                        now,
                    )?;
                    emitted.extend(outcome.events);
                }
                emitted.push(record_wire_event(
                    conn, state, session_id, &run_id, wire_event, now,
                )?);
            }
            "turn/end" => {
                emitted.extend(handle_wire_turn_end(
                    conn,
                    state,
                    session_id,
                    run_id,
                    run_opt.as_ref(),
                    wire_event,
                    now,
                )?);
            }
            "tool/call" => {
                emitted.extend(handle_wire_tool_call(
                    conn,
                    state,
                    session_id,
                    run_id,
                    active_non_terminal,
                    wire_event,
                    now,
                )?);
            }
            "user/message" => {
                emitted.extend(handle_wire_user_message(
                    conn,
                    state,
                    session_id,
                    run_id,
                    active_non_terminal,
                    wire_event,
                    now,
                )?);
            }
            "assistant/message" | "tool/result" | "assistant/attempt" => {
                emitted.push(record_wire_event(
                    conn, state, session_id, &run_id, wire_event, now,
                )?);
            }
            _ => {}
        }

        // Advance DSH cursor only once a run was resolved so pre-run events can be
        // re-delivered rather than silently dropped.
        if wire_event.seq > 0 {
            let _ = DshCursorStore::advance_cursor(conn, session_id, wire_event.seq, now)?;
        }
    }

    Ok(emitted)
}
