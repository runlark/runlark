use crate::routes::error::{ApiError, api_error};
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use runlark_wasm::{PackageInspection, PackagePin, ValidationReport, WasmError};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct PluginPathRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct ValidatePluginResponse {
    pub inspection: PackageInspection,
    pub validation: ValidationReport,
}

/// Lists all workflow/validator packages retained in the digest-pinned registry.
///
/// # Errors
/// Returns `500 Internal Server Error` if the registry lock fails.
pub async fn list_plugins(
    State(state): State<AppState>,
) -> Result<Json<Vec<PackagePin>>, ApiError> {
    let pins = state
        .verification
        .list_packages()
        .map_err(wasm_err_to_api)?;
    Ok(Json(pins))
}

/// Installs and pins a workflow/validator package directory into the daemon registry and CAS.
///
/// # Errors
/// Returns `400` if `path` is blank, `409` on digest/WIT mismatch, or `404`/`500` on missing/I/O.
pub async fn install_plugin(
    State(state): State<AppState>,
    Json(req): Json<PluginPathRequest>,
) -> Result<(StatusCode, Json<PackagePin>), ApiError> {
    if req.path.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_plugin_path",
            "path is required",
        ));
    }
    let path = PathBuf::from(&req.path);
    let pin = state
        .verification
        .install_package_dir(&path)
        .map_err(wasm_err_to_api)?;
    Ok((StatusCode::CREATED, Json(pin)))
}

/// Validates a workflow/validator package directory end-to-end against WIT + Wasmtime.
///
/// # Errors
/// Returns `400` if `path` is blank, `409` on digest/WIT/capability failure, or `500` on trap.
pub async fn validate_plugin(
    Json(req): Json<PluginPathRequest>,
) -> Result<Json<ValidatePluginResponse>, ApiError> {
    if req.path.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_plugin_path",
            "path is required",
        ));
    }
    let path = PathBuf::from(&req.path);
    let (inspection, validation) =
        runlark_wasm::validate_package_path(&path).map_err(wasm_err_to_api)?;
    Ok(Json(ValidatePluginResponse {
        inspection,
        validation,
    }))
}

fn wasm_err_to_api(err: WasmError) -> ApiError {
    match err {
        WasmError::PackageNotFound(msg) => {
            api_error(StatusCode::NOT_FOUND, "package_not_found", msg)
        }
        WasmError::DigestMismatch { .. }
        | WasmError::IncompatibleComponent(_)
        | WasmError::InvalidManifest(_)
        | WasmError::WitContract(_) => api_error(
            StatusCode::CONFLICT,
            "invalid_plugin_package",
            err.to_string(),
        ),
        other => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "wasm_error",
            other.to_string(),
        ),
    }
}
