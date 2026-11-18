use super::ChatDispatchResponse;
use crate::git_pr::{
    CreatePrOutcome, CreatePrRequest, GitPrError, PrCreationService, SystemPrRunner,
};
use crate::routes::error::{ApiError, api_error, run_status_str, storage_error_to_http};
use crate::routes::team_gate;
use crate::state::AppState;
use axum::http::StatusCode;
use runlark_domain::{
    AnswerQuestionCommand, CancelTeamRunCommand, CommandId, QuestionId, ResumeSessionCommand,
    RunId, SessionCommand, StopRunCommand, TaskId, TeamCommand, Timestamp,
};
use runlark_local::{CommandApplyOutcome, LocalStore, QuestionStore, SessionStore, TaskStore};
use serde_json::{Value, json};
use uuid::Uuid;

pub(super) async fn exec_stop_run(
    state: &AppState,
    run_id: &RunId,
    reason: Option<String>,
    command_id: Option<String>,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let (run, team_ctx) = {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        let run = SessionStore::get_run(&conn, run_id)
            .map_err(storage_error_to_http)?
            .ok_or_else(|| {
                api_error(
                    StatusCode::NOT_FOUND,
                    "run_not_found",
                    format!("Run not found: {run_id}"),
                )
            })?;
        drop(conn);
        let ctx = team_gate::resolve_run_team_context(state, &run)?;
        (run, ctx)
    };

    let now = Timestamp(AppState::current_timestamp_ms());
    let cmd_id = CommandId::from(
        command_id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("cmd_stop_{}", Uuid::new_v4().simple())),
    );
    if let Some(ctx) = team_ctx {
        let cancel_reason = reason
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Cancelled by operator".to_string());
        let team_cmd = TeamCommand::CancelRun(CancelTeamRunCommand {
            command_id: cmd_id,
            run_id: run_id.clone(),
            reason: cancel_reason,
            expected_revision: run.revision,
            created_at: now,
        });
        team_gate::submit_team_command_checked(state, &ctx, &team_cmd).await?;
    } else {
        let mut conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        let session = SessionStore::get_session(&conn, &run.session_id)
            .map_err(storage_error_to_http)?
            .ok_or_else(|| {
                api_error(
                    StatusCode::NOT_FOUND,
                    "session_not_found",
                    format!("Session not found: {}", run.session_id),
                )
            })?;
        let cmd = SessionCommand::StopRun(StopRunCommand {
            command_id: cmd_id,
            session_id: session.id.clone(),
            run_id: run_id.clone(),
            reason,
            expected_revision: session.current_revision,
            created_at: now,
        });
        let outcome =
            LocalStore::apply_command(&mut conn, &cmd, now).map_err(storage_error_to_http)?;
        if let CommandApplyOutcome::Applied { events, .. } = &outcome {
            state.publish_events(&session.id.0, events);
            state.notify_outbox();
            state.notify_dsh_subscriber();
            state.notify_scheduler();
        }
    }

    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let updated_run = SessionStore::get_run(&conn, run_id)
        .map_err(storage_error_to_http)?
        .unwrap_or(run);
    Ok((
        StatusCode::OK,
        ChatDispatchResponse {
            command: "stop".to_string(),
            summary: format!(
                "Stop requested for run {} (status: {})",
                updated_run.id,
                run_status_str(updated_run.status)
            ),
            data: serde_json::to_value(&updated_run).unwrap_or(Value::Null),
        },
    ))
}

pub(super) async fn exec_resume_run(
    state: &AppState,
    run_id: &RunId,
    command_id: Option<String>,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let run = {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        SessionStore::get_run(&conn, run_id)
            .map_err(storage_error_to_http)?
            .ok_or_else(|| {
                api_error(
                    StatusCode::NOT_FOUND,
                    "run_not_found",
                    format!("Run not found: {run_id}"),
                )
            })?
    };
    // In TeamManaged mode, DSH session resume is executed locally via the DSH outbox after
    // verifying server reachability/sync, then synced back as a fenced runner-state update if leased.
    let team_ctx = team_gate::resolve_run_team_context(state, &run)?;
    if let Some(ref ctx) = team_ctx {
        team_gate::verify_server_reachable_and_synced(state, ctx).await?;
    }
    let now = Timestamp(AppState::current_timestamp_ms());
    let expected_revision = load_session_revision(state, &run.session_id)?;
    let cmd = SessionCommand::ResumeSession(ResumeSessionCommand {
        command_id: CommandId::from(
            command_id.unwrap_or_else(|| format!("cmd_res_{}", Uuid::new_v4().simple())),
        ),
        session_id: run.session_id.clone(),
        run_id: run_id.clone(),
        after_seq: None,
        expected_revision,
        created_at: now,
    });
    let outcome = {
        let mut conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        LocalStore::apply_command(&mut conn, &cmd, now).map_err(storage_error_to_http)?
    };
    if let CommandApplyOutcome::Applied { events, .. } = &outcome {
        state.publish_events(&run.session_id.0, events);
        state.notify_outbox();
        state.notify_dsh_subscriber();
        state.notify_scheduler();
        if let Some(ref ctx) = team_ctx {
            team_gate::sync_team_run_resumed_if_leased(
                state,
                ctx,
                run_id,
                "session_resumed",
                &json!({ "run_id": run_id.0 }).to_string(),
            )
            .await;
        }
    }
    let updated_run = {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        SessionStore::get_run(&conn, run_id)
            .map_err(storage_error_to_http)?
            .unwrap_or(run)
    };
    Ok((
        StatusCode::OK,
        ChatDispatchResponse {
            command: "resume".to_string(),
            summary: format!(
                "Resumed run {} (status: {})",
                updated_run.id,
                run_status_str(updated_run.status)
            ),
            data: serde_json::to_value(&updated_run).unwrap_or(Value::Null),
        },
    ))
}

fn load_session_revision(
    state: &AppState,
    session_id: &runlark_domain::SessionId,
) -> Result<runlark_domain::Revision, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    Ok(SessionStore::get_session(&conn, session_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                format!("Session not found: {session_id}"),
            )
        })?
        .current_revision)
}

pub(super) async fn exec_answer_question(
    state: &AppState,
    question_id: &QuestionId,
    answer: &str,
    command_id: Option<String>,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let question = QuestionStore::get_question(&conn, question_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "question_not_found",
                format!("Question not found: {question_id}"),
            )
        })?;
    let session = SessionStore::get_session(&conn, &question.session_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                format!("Session not found: {}", question.session_id),
            )
        })?;
    drop(conn);

    let now = Timestamp(AppState::current_timestamp_ms());
    let cmd = AnswerQuestionCommand {
        command_id: CommandId::from(
            command_id.unwrap_or_else(|| format!("cmd_ans_{}", Uuid::new_v4().simple())),
        ),
        session_id: session.id.clone(),
        run_id: question.run_id.clone(),
        question_id: question_id.clone(),
        answer: answer.to_string(),
        expected_revision: session.current_revision,
        created_at: now,
    };
    // In TeamManaged mode, DSH approval answers gate on server reachability/sync, execute
    // via the local DSH outbox, and sync `Running` back via fenced runner reports if leased.
    let team_ctx = team_gate::resolve_session_command_team_context(
        state,
        &SessionCommand::AnswerQuestion(cmd.clone()),
    )?
    .map(|t| t.context);
    if let Some(ref ctx) = team_ctx {
        team_gate::verify_server_reachable_and_synced(state, ctx).await?;
    }

    let outcome = {
        let mut conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        LocalStore::answer_question(&mut conn, cmd, now).map_err(storage_error_to_http)?
    };
    if let CommandApplyOutcome::Applied { events, .. } = &outcome {
        state.publish_events(&session.id.0, events);
        state.notify_outbox();
        state.notify_dsh_subscriber();
        state.notify_scheduler();
        if let Some(ref ctx) = team_ctx {
            team_gate::sync_team_run_resumed_if_leased(
                state,
                ctx,
                &question.run_id,
                "question_answered",
                &json!({ "question_id": question_id.0 }).to_string(),
            )
            .await;
        }
    }
    Ok((
        StatusCode::OK,
        ChatDispatchResponse {
            command: "answer".to_string(),
            summary: format!("Answered question {question_id}"),
            data: json!({
                "question_id": question_id.0,
                "run_id": question.run_id.0,
                "answer": answer,
            }),
        },
    ))
}

pub(super) async fn exec_create_pr(
    state: &AppState,
    target: &RunId,
    title: Option<String>,
    draft: bool,
    command_id: Option<String>,
) -> Result<(StatusCode, ChatDispatchResponse), ApiError> {
    let (resolved_run_id, team_ctx) = {
        let conn = state.get_connection().map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                e.to_string(),
            )
        })?;
        if let Some(run) = SessionStore::get_run(&conn, target).map_err(storage_error_to_http)? {
            drop(conn);
            let ctx = team_gate::resolve_run_team_context(state, &run)?;
            (run.id, ctx)
        } else if let Some(task) = TaskStore::get_task(&conn, &TaskId::from(target.0.as_str()))
            .map_err(storage_error_to_http)?
        {
            drop(conn);
            let ctx = team_gate::resolve_project_team_context(state, &task.project_id)?;
            let run_id = task.latest_run_id.or(task.active_run_id).ok_or_else(|| {
                api_error(
                    StatusCode::CONFLICT,
                    "missing_run",
                    format!("Task {} has no run", task.id),
                )
            })?;
            (run_id, ctx)
        } else {
            return Err(api_error(
                StatusCode::NOT_FOUND,
                "run_not_found",
                format!("Run not found: {target}"),
            ));
        }
    };
    if let Some(ref ctx) = team_ctx {
        team_gate::verify_server_reachable_and_synced(state, ctx).await?;
    }

    let req = CreatePrRequest {
        command_id: CommandId::from(
            command_id.unwrap_or_else(|| format!("cmd_pr_{}", Uuid::new_v4().simple())),
        ),
        run_id: resolved_run_id,
        title,
        body: None,
        base_branch: None,
        draft,
        expected_revision: None,
    };
    let runner = SystemPrRunner::default();
    match PrCreationService::create_pr(state, &runner, &req).await {
        Ok(outcome) => {
            let status = match &outcome {
                CreatePrOutcome::Created { replayed: true, .. } => StatusCode::OK,
                CreatePrOutcome::Created {
                    replayed: false, ..
                } => StatusCode::CREATED,
                CreatePrOutcome::Unknown { .. } => StatusCode::ACCEPTED,
            };
            Ok((
                status,
                ChatDispatchResponse {
                    command: "pr".to_string(),
                    summary: format!("PR outcome for run {}", req.run_id),
                    data: serde_json::to_value(&outcome).unwrap_or(Value::Null),
                },
            ))
        }
        Err(GitPrError::Storage(se)) => Err(storage_error_to_http(se)),
        Err(GitPrError::NotFound(msg)) => Err(api_error(StatusCode::NOT_FOUND, "not_found", msg)),
        Err(GitPrError::Conflict(msg)) => Err(api_error(
            StatusCode::CONFLICT,
            "pr_precondition_failed",
            msg,
        )),
        Err(GitPrError::CommandFailed(msg)) => {
            Err(api_error(StatusCode::BAD_GATEWAY, "pr_command_failed", msg))
        }
        Err(GitPrError::UnknownOutcome(msg)) => Err(api_error(
            StatusCode::BAD_GATEWAY,
            "pr_outcome_unknown",
            msg,
        )),
    }
}
