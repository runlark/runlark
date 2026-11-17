use crate::routes::board::{BoardTaskEntry, load_enriched_task};
use crate::routes::error::{ApiError, api_error, deterministic_id, storage_error_to_http};
pub use crate::routes::task_planning::{
    ClarifyTaskRequest, ClarifyTaskResponse, PlanTaskRequest, clarify_task, plan_task,
};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use runlark_domain::{
    ActorRole, BoardCommand, BudgetPolicy, CommandId, CreateTaskCommand, MoveTaskStageCommand,
    ProjectId, Revision, Task, TaskId, TaskKind, TaskStage, Timestamp,
};
use runlark_local::{BoardCommandOutcome, BoardStore, ProjectStore, RunQueries, TaskStore};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    pub project_id: Option<String>,
    pub stage: Option<TaskStage>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    pub command_id: Option<String>,
    pub task_id: Option<String>,
    pub project_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub kind: TaskKind,
    pub base_ref: Option<String>,
    pub budget_override: Option<BudgetPolicy>,
}

#[derive(Debug, Deserialize)]
pub struct MoveTaskRequest {
    pub command_id: Option<String>,
    pub stage: TaskStage,
    #[serde(default, deserialize_with = "deserialize_optional_actor_role")]
    pub actor: Option<ActorRole>,
    pub actor_name: Option<String>,
    pub expected_revision: Option<u64>,
}

fn deserialize_optional_actor_role<'de, D>(deserializer: D) -> Result<Option<ActorRole>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ActorOrLabel {
        Role(ActorRole),
        Label(String),
    }

    let raw = Option::<ActorOrLabel>::deserialize(deserializer)?;
    Ok(raw.map(|value| match value {
        ActorOrLabel::Role(role) => role,
        ActorOrLabel::Label(label) => match label.trim().to_ascii_lowercase().as_str() {
            "agent" => ActorRole::Agent("agent".to_string()),
            "system" => ActorRole::System,
            "" => ActorRole::Human("human".to_string()),
            _ => ActorRole::Human(label),
        },
    }))
}

/// Lists tasks optionally filtered by `project_id` and `stage`.
///
/// # Errors
/// Returns `500 Internal Server Error` if querying `SQLite` fails.
pub async fn list_tasks(
    State(state): State<AppState>,
    Query(query): Query<ListTasksQuery>,
) -> Result<Json<Vec<Task>>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let pid = query
        .project_id
        .filter(|s| !s.trim().is_empty())
        .map(ProjectId::from);
    let tasks =
        TaskStore::list_tasks(&conn, pid.as_ref(), query.stage).map_err(storage_error_to_http)?;
    Ok(Json(tasks))
}

/// Creates a new Kanban task via `BoardCommand::CreateTask`.
///
/// # Errors
/// Returns `400` on blank title, `404` if project missing, `409` on command conflict.
pub async fn create_task(
    State(state): State<AppState>,
    Json(req): Json<CreateTaskRequest>,
) -> Result<(StatusCode, Json<Task>), ApiError> {
    if req.title.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_task",
            "title is required",
        ));
    }

    let mut conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;

    let explicit_pid = req
        .project_id
        .filter(|s| !s.trim().is_empty())
        .map(ProjectId::from);
    let project_id = RunQueries::resolve_default_project_id(&conn, explicit_pid.as_ref())
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "project_not_found",
                "no project registered; create a project first",
            )
        })?;
    if ProjectStore::get_project(&conn, &project_id)
        .map_err(storage_error_to_http)?
        .is_none()
    {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "project_not_found",
            format!("Project not found: {project_id}"),
        ));
    }

    let explicit_cmd_id = req.command_id.filter(|s| !s.trim().is_empty());
    let command_id = CommandId::from(
        explicit_cmd_id
            .clone()
            .unwrap_or_else(|| format!("cmd_task_{}", Uuid::new_v4().simple())),
    );
    let task_id = TaskId::from(
        req.task_id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| match &explicit_cmd_id {
                Some(cid) => deterministic_id("task", cid),
                None => format!("task_{}", Uuid::new_v4().simple()),
            }),
    );
    let now = Timestamp(AppState::current_timestamp_ms());
    let cmd = BoardCommand::CreateTask(CreateTaskCommand {
        command_id,
        task_id: task_id.clone(),
        project_id,
        title: req.title,
        description: req.description,
        kind: req.kind,
        base_ref: req.base_ref,
        budget_override: req.budget_override,
        created_at: now,
    });

    let outcome =
        BoardStore::apply_board_command(&mut conn, &cmd, now).map_err(storage_error_to_http)?;
    let task = TaskStore::get_task(&conn, &task_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                "task missing after creation",
            )
        })?;
    let status = match outcome {
        BoardCommandOutcome::Applied { .. } => StatusCode::CREATED,
        BoardCommandOutcome::AlreadyExists { .. } => StatusCode::OK,
    };
    Ok((status, Json(task)))
}

/// Retrieves a single enriched task summary by ID.
///
/// # Errors
/// Returns `404` if the task does not exist.
pub async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<BoardTaskEntry>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let task_id = TaskId::from(id);
    let Json(updated) = load_task_json(&conn, &task_id)?;
    Ok(Json(load_enriched_task(&conn, &updated)?))
}

fn load_task_json(
    conn: &runlark_local::Connection,
    task_id: &TaskId,
) -> Result<Json<Task>, ApiError> {
    let task = TaskStore::get_task(conn, task_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "task_not_found",
                format!("Task not found: {task_id}"),
            )
        })?;
    Ok(Json(task))
}

/// Moves a task across Kanban board columns (`Inbox -> Ready -> InProgress -> Acceptance -> Done`).
///
/// # Errors
/// Returns `404` if task missing, `409` on revision/guard/self-acceptance violations.
pub async fn move_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<MoveTaskRequest>,
) -> Result<Json<Task>, ApiError> {
    let mut conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let task_id = TaskId::from(id);
    let task = TaskStore::get_task(&conn, &task_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "task_not_found",
                format!("Task not found: {task_id}"),
            )
        })?;

    let actor = req
        .actor
        .unwrap_or_else(|| ActorRole::Human(req.actor_name.unwrap_or_else(|| "human".to_string())));
    let explicit_cmd_id = req.command_id.filter(|s| !s.trim().is_empty());
    let command_id = CommandId::from(
        explicit_cmd_id
            .clone()
            .unwrap_or_else(|| format!("cmd_move_{}", Uuid::new_v4().simple())),
    );
    let prior_rev = if req.expected_revision.is_none() && explicit_cmd_id.is_some() {
        BoardStore::get_command_payload(&conn, &command_id)
            .map_err(storage_error_to_http)?
            .and_then(|json| serde_json::from_str::<BoardCommand>(&json).ok())
            .and_then(|c| match c {
                BoardCommand::MoveTaskStage(prev) => Some(prev.expected_revision),
                _ => None,
            })
    } else {
        None
    };
    let expected_revision =
        prior_rev.unwrap_or_else(|| Revision(req.expected_revision.unwrap_or(task.revision.0)));
    let now = Timestamp(AppState::current_timestamp_ms());
    let cmd = BoardCommand::MoveTaskStage(MoveTaskStageCommand {
        command_id,
        task_id: task_id.clone(),
        target_stage: req.stage,
        actor,
        expected_revision,
        created_at: now,
    });
    BoardStore::apply_board_command(&mut conn, &cmd, now).map_err(storage_error_to_http)?;
    let updated = TaskStore::get_task(&conn, &task_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                "task missing after move",
            )
        })?;
    Ok(Json(updated))
}
