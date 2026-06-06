use std::path::Path;
use std::sync::Arc;

use process_wrap::tokio::{CommandWrap, KillOnDrop, ProcessGroup};
use tokio::sync::Mutex;
use rig_core::tool::rmcp::McpClientHandler;
use rig_core::tool::server::{ToolServer, ToolServerHandle};
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientInfo};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use serde_json::json;
use thiserror::Error;

use crate::compile_db::CompileDbContext;
use crate::tree_sitter::{project_name_from_path, TreeSitterContext};

#[derive(Debug, Error)]
pub enum McpBootstrapError {
    #[error("dependency `{0}` not found in PATH")]
    DependencyMissing(&'static str),
    #[error("failed to spawn MCP process: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("tree-sitter MCP error: {0}")]
    TreeSitter(String),
    #[error("mcp-cpp MCP error: {0}")]
    McpCpp(String),
}

/// MCP setup tools — registered at daemon boot, never exposed to the LLM.
const AGENT_HIDDEN_TOOLS: &[&str] = &["register_project_tool", "list_projects_tool"];

/// Misleading for Makefile + external compile_commands (scans source tree, not work_dir).
const AGENT_HIDDEN_WHEN_COMPILE_DB: &[&str] = &["get_project_details"];

/// Keeps MCP child processes alive for the lifetime of the daemon.
pub struct McpRuntime {
    pub tool_server: ToolServerHandle,
    pub tool_count: usize,
    pub ts_context: TreeSitterContext,
    ts_service: RunningService<rmcp::service::RoleClient, McpClientHandler>,
    cpp_service: RunningService<rmcp::service::RoleClient, McpClientHandler>,
}

pub type McpHolder = Arc<Mutex<Option<McpRuntime>>>;

pub fn mcp_holder(mcp: McpRuntime) -> McpHolder {
    Arc::new(Mutex::new(Some(mcp)))
}

/// Idempotent: safe to call from signal handler and again on process exit.
pub async fn shutdown_holder(holder: &McpHolder) {
    if let Some(mcp) = holder.lock().await.take() {
        mcp.shutdown().await;
    }
}

impl McpRuntime {
    /// Verify MCP tools against a known source file (does not block HTTP listen).
    pub async fn warmup(&self, compile_db: &CompileDbContext) {
        let abs_probe = compile_db
            .main_sources
            .first()
            .or_else(|| compile_db.source_files.first());
        let Some(abs_path) = abs_probe else {
            tracing::warn!("warmup skipped: no source files in compile_commands");
            return;
        };
        let abs_path_str = abs_path.display().to_string();
        let build_dir = compile_db.compile_db_dir.display().to_string();
        let ts = &self.ts_context;
        let ts_rel = ts
            .main_entry_paths
            .first()
            .cloned()
            .unwrap_or_else(|| abs_path_str.clone());

        let get_file_args = json!({
            "project": ts.project_name,
            "path": ts_rel,
            "max_lines": 5
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        match self
            .ts_service
            .peer()
            .call_tool(CallToolRequestParams::new("get_file").with_arguments(get_file_args))
            .await
        {
            Ok(r) if r.is_error != Some(true) => {
                tracing::info!(file = %ts_rel, "warmup: tree-sitter get_file ok");
            }
            Ok(r) => tracing::warn!(
                file = %ts_rel,
                error = %format_tool_result(&r),
                "warmup: tree-sitter get_file failed"
            ),
            Err(e) => tracing::warn!(file = %ts_rel, error = %e, "warmup: tree-sitter get_file call failed"),
        }

        let ts_args = json!({
            "project": ts.project_name,
            "file_path": ts_rel
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        match self
            .ts_service
            .peer()
            .call_tool(CallToolRequestParams::new("get_symbols").with_arguments(ts_args))
            .await
        {
            Ok(r) if r.is_error != Some(true) => {
                tracing::info!(file = %ts_rel, "warmup: tree-sitter get_symbols ok");
            }
            Ok(r) => tracing::warn!(
                file = %ts_rel,
                error = %format_tool_result(&r),
                "warmup: tree-sitter get_symbols failed (complex C++ may need mcp-cpp)"
            ),
            Err(e) => tracing::warn!(file = %ts_rel, error = %e, "warmup: tree-sitter get_symbols call failed"),
        }

        let cpp_args = json!({
            "query": "main",
            "build_directory": build_dir,
            "files": [abs_path_str],
            "wait_timeout": 30
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        match self
            .cpp_service
            .peer()
            .call_tool(CallToolRequestParams::new("search_symbols").with_arguments(cpp_args))
            .await
        {
            Ok(r) if r.is_error != Some(true) => {
                tracing::info!(file = %abs_path.display(), "warmup: mcp-cpp search_symbols (document) ok");
            }
            Ok(r) => tracing::warn!(
                file = %abs_path.display(),
                error = %format_tool_result(&r),
                "warmup: mcp-cpp search_symbols failed"
            ),
            Err(e) => tracing::warn!(file = %abs_path.display(), error = %e, "warmup: mcp-cpp call failed"),
        }
    }

    /// Gracefully stop both MCP servers (uvx tree-sitter + mcp-cpp).
    async fn shutdown(self) {
        tracing::info!("shutting down MCP servers");
        let (cpp, ts) = tokio::join!(self.cpp_service.cancel(), self.ts_service.cancel());
        if let Err(e) = cpp {
            tracing::warn!(error = %e, "mcp-cpp shutdown error");
        }
        if let Err(e) = ts {
            tracing::warn!(error = %e, "tree-sitter shutdown error");
        }
        tracing::info!("MCP servers stopped");
    }
}

pub async fn init(
    project_root: &Path,
    compile_db_ready: bool,
    compile_db: Option<&CompileDbContext>,
) -> Result<McpRuntime, McpBootstrapError> {
    check_dependency("uvx")?;
    check_dependency("mcp-cpp-server")?;

    let project_root = std::fs::canonicalize(project_root)
        .unwrap_or_else(|_| project_root.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let tool_server = ToolServer::new().run();
    let client_info = ClientInfo::default();

    let ts_service = connect_mcp(
        tool_server.clone(),
        client_info.clone(),
        spawn_uvx_tree_sitter()?,
        "tree-sitter",
    )
    .await
    .map_err(|e| McpBootstrapError::TreeSitter(e.to_string()))?;

    let registration = register_tree_sitter_project(&ts_service, &project_root).await?;
    let ts_context =
        TreeSitterContext::from_registration(&registration, compile_db).map_err(McpBootstrapError::TreeSitter)?;
    tracing::info!(
        project_root = %project_root,
        ts_project = %ts_context.project_name,
        ts_registry_root = %ts_context.registry_root.display(),
        main_entries = ts_context.main_entry_paths.len(),
        "tree-sitter project ready"
    );

    let cpp_service = connect_mcp(
        tool_server.clone(),
        client_info,
        spawn_mcp_cpp(&project_root)?,
        "mcp-cpp",
    )
    .await
    .map_err(|e| McpBootstrapError::McpCpp(e.to_string()))?;

    hide_agent_tools(&tool_server, compile_db_ready).await;

    let tool_count = tool_server.get_tool_defs(None).await.map(|t| t.len()).unwrap_or(0);

    tracing::info!(tool_count, "MCP tool federation ready");

    Ok(McpRuntime {
        tool_server,
        tool_count,
        ts_context,
        ts_service,
        cpp_service,
    })
}

fn check_dependency(bin: &'static str) -> Result<(), McpBootstrapError> {
    if which::which(bin).is_err() {
        return Err(McpBootstrapError::DependencyMissing(bin));
    }
    Ok(())
}

fn mcp_command_wrap(
    program: &str,
    configure: impl FnOnce(&mut tokio::process::Command),
) -> CommandWrap {
    let pgid = std::process::id();
    let mut wrap = CommandWrap::with_new(program, configure);
    wrap.wrap(KillOnDrop);
    #[cfg(unix)]
    wrap.wrap(ProcessGroup::attach_to(pgid));
    wrap
}

fn spawn_uvx_tree_sitter() -> Result<TokioChildProcess, std::io::Error> {
    TokioChildProcess::new(mcp_command_wrap("uvx", |cmd| {
        cmd.arg("mcp-server-tree-sitter");
    }))
}

fn spawn_mcp_cpp(project_root: &str) -> Result<TokioChildProcess, std::io::Error> {
    let root = project_root.to_string();
    TokioChildProcess::new(mcp_command_wrap("mcp-cpp-server", |cmd| {
        cmd.args(["--root", &root]);
    }))
}

async fn connect_mcp(
    tool_server: ToolServerHandle,
    client_info: ClientInfo,
    transport: TokioChildProcess,
    label: &str,
) -> Result<RunningService<rmcp::service::RoleClient, McpClientHandler>, rig_core::tool::rmcp::McpClientError>
{
    tracing::info!(label, "connecting MCP server");
    let handler = McpClientHandler::new(client_info, tool_server);
    handler.connect(transport).await
}

async fn register_tree_sitter_project(
    service: &RunningService<rmcp::service::RoleClient, McpClientHandler>,
    project_root: &str,
) -> Result<String, McpBootstrapError> {
    let name = project_name_from_path(Path::new(project_root));
    let args = json!({ "path": project_root, "name": name })
        .as_object()
        .cloned()
        .unwrap_or_default();
    let params = CallToolRequestParams::new("register_project_tool").with_arguments(args);
    let result = service
        .peer()
        .call_tool(params)
        .await
        .map_err(|e| McpBootstrapError::TreeSitter(e.to_string()))?;
    if result.is_error == Some(true) {
        return Err(McpBootstrapError::TreeSitter(format_tool_result(&result)));
    }
    Ok(format_tool_result(&result))
}

async fn hide_agent_tools(tool_server: &ToolServerHandle, compile_db_ready: bool) {
    for name in AGENT_HIDDEN_TOOLS {
        remove_agent_tool(tool_server, name).await;
    }
    if compile_db_ready {
        for name in AGENT_HIDDEN_WHEN_COMPILE_DB {
            remove_agent_tool(tool_server, name).await;
        }
    }
}

async fn remove_agent_tool(tool_server: &ToolServerHandle, name: &str) {
    match tool_server.remove_tool(name).await {
        Ok(()) => tracing::info!(tool = name, "hidden from agent tool list"),
        Err(e) => tracing::debug!(tool = name, error = %e, "tool not exposed to agent"),
    }
}

fn format_tool_result(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

mod which {
    pub fn which(bin: &str) -> Result<(), ()> {
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                if dir.join(bin).is_file() {
                    return Ok(());
                }
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            let candidate = format!("{home}/.cargo/bin/{bin}");
            if std::path::Path::new(&candidate).is_file() {
                return Ok(());
            }
        }
        Err(())
    }
}
