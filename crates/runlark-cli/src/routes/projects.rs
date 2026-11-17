use crate::routes::error::{ApiError, api_error, deterministic_id, storage_error_to_http};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use runlark_domain::{
    BoardCommand, BudgetPolicy, CommandId, CreateProjectCommand, PolicyDigest, Project, ProjectId,
    Timestamp,
};
use runlark_local::{BoardCommandOutcome, BoardStore, ProjectStore};
use runlark_wasm::sha256_hex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub command_id: Option<String>,
    pub project_id: Option<String>,
    pub name: String,
    pub repo_root: String,
    #[serde(default = "default_branch")]
    pub default_branch: String,
    #[serde(default)]
    pub budget_policy: BudgetPolicy,
    pub policy_digest: Option<String>,
}

fn default_branch() -> String {
    "main".to_string()
}

#[derive(Debug, Serialize)]
pub struct ProjectResponse {
    pub project_id: String,
    pub name: String,
    pub repo_root: String,
    pub default_branch: String,
    pub max_concurrent_runs: u32,
    pub budget_policy: BudgetPolicy,
    pub policy_digest: String,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

impl From<Project> for ProjectResponse {
    fn from(p: Project) -> Self {
        Self {
            project_id: p.id.0,
            name: p.name,
            repo_root: p.repo_root,
            default_branch: p.default_branch,
            max_concurrent_runs: p.max_concurrent_runs,
            budget_policy: p.budget_policy,
            policy_digest: p.policy_digest.0,
            revision: p.revision.0,
            created_at: p.created_at.0,
            updated_at: p.updated_at.0,
        }
    }
}

/// Lists all registered projects.
///
/// # Errors
/// Returns `500 Internal Server Error` if `SQLite` query fails.
pub async fn list_projects(
    State(state): State<AppState>,
) -> Result<Json<Vec<ProjectResponse>>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let projects = ProjectStore::list_projects(&conn).map_err(storage_error_to_http)?;
    Ok(Json(
        projects.into_iter().map(ProjectResponse::from).collect(),
    ))
}

/// Registers a new project workspace via `BoardCommand::CreateProject`.
///
/// # Errors
/// Returns `400 Bad Request` for blank fields, `409 Conflict` on duplicate ID/command conflict,
/// or `500 Internal Server Error` if persistence fails.
pub async fn create_project(
    State(state): State<AppState>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectResponse>), ApiError> {
    if req.name.trim().is_empty() || req.repo_root.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_project",
            "name and repo_root are required",
        ));
    }

    let mut conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;

    let explicit_cmd_id = req.command_id.filter(|s| !s.trim().is_empty());
    let command_id = CommandId::from(
        explicit_cmd_id
            .clone()
            .unwrap_or_else(|| format!("cmd_proj_{}", Uuid::new_v4().simple())),
    );
    let project_id = ProjectId::from(
        req.project_id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| match &explicit_cmd_id {
                Some(cid) => deterministic_id("proj", cid),
                None => format!("proj_{}", Uuid::new_v4().simple()),
            }),
    );

    let policy_digest = if let Some(raw) = req.policy_digest.filter(|s| !s.trim().is_empty()) {
        PolicyDigest::from(raw)
    } else {
        let policy_bytes = serde_json::to_vec(&req.budget_policy).map_err(|e| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "serialization_error",
                e.to_string(),
            )
        })?;
        PolicyDigest::from(sha256_hex(&policy_bytes))
    };

    let now = Timestamp(AppState::current_timestamp_ms());
    let cmd = BoardCommand::CreateProject(CreateProjectCommand {
        command_id,
        project_id: project_id.clone(),
        name: req.name,
        repo_root: req.repo_root,
        default_branch: req.default_branch,
        budget_policy: req.budget_policy,
        policy_digest,
        created_at: now,
    });

    let outcome =
        BoardStore::apply_board_command(&mut conn, &cmd, now).map_err(storage_error_to_http)?;
    let project = ProjectStore::get_project(&conn, &project_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                "project missing after creation",
            )
        })?;

    let status = match outcome {
        BoardCommandOutcome::Applied { .. } => StatusCode::CREATED,
        BoardCommandOutcome::AlreadyExists { .. } => StatusCode::OK,
    };
    Ok((status, Json(ProjectResponse::from(project))))
}

/// Fetches a single project by ID.
///
/// # Errors
/// Returns `404 Not Found` if the project does not exist, or `500` on storage failure.
pub async fn get_project(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProjectResponse>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let project_id = ProjectId::from(id);
    let project = ProjectStore::get_project(&conn, &project_id)
        .map_err(storage_error_to_http)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "project_not_found",
                format!("Project not found: {project_id}"),
            )
        })?;
    Ok(Json(ProjectResponse::from(project)))
}
