use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub source_root: String,
    pub compile_db_dir: Option<String>,
    pub compile_db_entries: Option<usize>,
    pub main_sources: Option<Vec<String>>,
    pub tree_sitter_project: Option<String>,
    pub tree_sitter_registry_root: Option<String>,
    pub tree_sitter_main_paths: Option<Vec<String>>,
    pub mcp_ready: bool,
    pub tool_count: usize,
    pub model: String,
}

pub async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        source_root: state.config.source_root.display().to_string(),
        compile_db_dir: state
            .compile_db
            .as_ref()
            .map(|c| c.compile_db_dir.display().to_string()),
        compile_db_entries: state.compile_db.as_ref().map(|c| c.entry_count),
        main_sources: state.compile_db.as_ref().map(|c| {
            c.main_sources
                .iter()
                .map(|p| p.display().to_string())
                .collect()
        }),
        tree_sitter_project: state.ts_context.as_ref().map(|t| t.project_name.clone()),
        tree_sitter_registry_root: state
            .ts_context
            .as_ref()
            .map(|t| t.registry_root.display().to_string()),
        tree_sitter_main_paths: state
            .ts_context
            .as_ref()
            .map(|t| t.main_entry_paths.clone()),
        mcp_ready: state.tool_count > 0,
        tool_count: state.tool_count,
        model: state.config.deepseek_model.clone(),
    })
}
