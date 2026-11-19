use crate::repair::{
    PipelineError, VerifyPipelineOptions, VerifyPipelineOutcome, WorkflowPipeline,
};
use crate::routes::error::{ApiError, api_error, storage_error_to_http};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use runlark_domain::{RunId, Timestamp};
use runlark_local::SessionStore;
use runlark_wasm::{CheckEvidence, ValidationInput, ValidationReport, WasmError, WorkflowStage};
use serde::Deserialize;

/// Request payload for `POST /api/v1/runs/{id}/verify`.
#[derive(Debug, Deserialize)]
pub struct VerifyRunRequest {
    #[serde(default = "default_package_name")]
    pub package_name: String,
    pub task_id: String,
    #[serde(default = "default_stage")]
    pub stage: WorkflowStage,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub checks: Vec<CheckEvidence>,
    #[serde(default)]
    pub diff_bytes: u64,
    #[serde(default)]
    pub repair_attempt: u32,
}

fn default_package_name() -> String {
    "solo-default".to_string()
}

const fn default_stage() -> WorkflowStage {
    WorkflowStage::Validate
}

/// Executes the digest-pinned WASM validator for a persisted run under bounded Wasmtime limits.
///
/// # Errors
/// Returns `404 Not Found` if the run or workflow package is missing,
/// `422 Unprocessable Entity` if the validator rejects invalid input,
/// or `500 Internal Server Error` on storage or Wasmtime failure.
pub async fn verify_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<VerifyRunRequest>,
) -> Result<Json<ValidationReport>, ApiError> {
    let conn = state.get_connection().map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            e.to_string(),
        )
    })?;
    let run_id = RunId::from(id);
    let run = {
        SessionStore::get_run(&conn, &run_id)
            .map_err(storage_error_to_http)?
            .ok_or_else(|| {
                api_error(
                    StatusCode::NOT_FOUND,
                    "run_not_found",
                    format!("Run not found: {run_id}"),
                )
            })?
    };

    if payload.changed_files.is_empty()
        && payload.checks.is_empty()
        && payload.diff_bytes == 0
        && run.task_id.is_some()
    {
        drop(conn);
        let outcome = WorkflowPipeline::verify_and_advance(
            &state,
            &run_id,
            VerifyPipelineOptions {
                package_name: Some(payload.package_name),
                ..VerifyPipelineOptions::default()
            },
            Timestamp(AppState::current_timestamp_ms()),
        )
        .await
        .map_err(pipeline_error_to_http)?;
        let report = match outcome {
            VerifyPipelineOutcome::PassedForAcceptance { report, .. }
            | VerifyPipelineOutcome::RepairScheduled { report, .. }
            | VerifyPipelineOutcome::BudgetHalted { report, .. } => report,
        };
        return Ok(Json(report));
    }

    let input = ValidationInput {
        task_id: payload.task_id,
        run_id: run_id.0.clone(),
        stage: payload.stage,
        changed_files: payload.changed_files,
        checks: payload.checks,
        diff_bytes: payload.diff_bytes,
        repair_attempt: payload.repair_attempt,
    };

    let report = state
        .verification
        .verify_run_with_digest(
            &run_id,
            run.workflow_digest.as_ref(),
            &payload.package_name,
            &input,
        )
        .map_err(wasm_error_to_http)?;
    Ok(Json(report))
}

fn pipeline_error_to_http(err: PipelineError) -> ApiError {
    match err {
        PipelineError::Storage(e) => storage_error_to_http(e),
        PipelineError::Wasm(e) => wasm_error_to_http(e),
        PipelineError::Transition(e) => {
            api_error(StatusCode::CONFLICT, "transition_conflict", e.to_string())
        }
        PipelineError::NotFound(msg) => api_error(StatusCode::NOT_FOUND, "not_found", msg),
        PipelineError::TeamFencing(msg) => {
            api_error(StatusCode::CONFLICT, "team_fencing_conflict", msg)
        }
    }
}

fn wasm_error_to_http(err: WasmError) -> ApiError {
    match err {
        WasmError::PackageNotFound(pkg) => api_error(
            StatusCode::NOT_FOUND,
            "workflow_package_not_found",
            format!("Workflow package not found: {pkg}"),
        ),
        WasmError::ValidatorRejected(msg) => {
            api_error(StatusCode::UNPROCESSABLE_ENTITY, "validator_rejected", msg)
        }
        WasmError::DigestMismatch { .. } | WasmError::IncompatibleComponent(_) => api_error(
            StatusCode::CONFLICT,
            "package_verification_failed",
            err.to_string(),
        ),
        other => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "wasm_execution_error",
            other.to_string(),
        ),
    }
}
