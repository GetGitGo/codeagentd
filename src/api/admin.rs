use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use crate::daemon;
use crate::local_peer;
use crate::state::AppState;

#[derive(Serialize)]
pub struct RestartResponse {
    pub status: &'static str,
    pub message: &'static str,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub async fn restart(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<RestartResponse>, (StatusCode, Json<ErrorResponse>)> {
    if !local_peer::is_local_peer(&peer) {
        tracing::warn!(%peer, "reject restart from non-local peer");
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "restart is only allowed from 127.0.0.1 / ::1".into(),
            }),
        ));
    }
    tracing::warn!(%peer, "restart requested via HTTP API");
    daemon::spawn_restart(&state.settings_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(Json(RestartResponse {
        status: "restarting",
        message: "codeagentd restart spawned; service will be briefly unavailable",
    }))
}
