use axum::Json;
use serde::Serialize;

/// Health probe payload.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub service: &'static str,
}

/// Returns `200 OK` when the collaboration server process is live.
pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "runlark-server",
    })
}
