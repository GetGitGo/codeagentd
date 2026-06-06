mod agent;
mod api;
mod cli;
mod compile_db;
mod config;
mod daemon;
mod mcp;
mod process_util;
mod state;
mod workspace;

use std::path::Path;

use tracing_subscriber::EnvFilter;

use crate::agent::build_agent;
use crate::api::router;
use crate::cli::Command;
use crate::compile_db::prepare;
use crate::config::Config;
use crate::daemon::{install_pid_file, remove_pid_file};
use crate::workspace::Workspace;
use crate::mcp::{init as init_mcp, mcp_holder, shutdown_holder};
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = cli::parse().map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })?;

    match cli.command {
        Command::Start => return daemon::start(&cli.config).map_err(Into::into),
        Command::Stop => return daemon::stop(&cli.config).map_err(Into::into),
        Command::Restart => return daemon::restart(&cli.config).map_err(Into::into),
        Command::Run => run_server(&cli.config).await,
    }
}

async fn run_server(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let ws = Workspace::from_settings_path(config_path);
    ws.ensure_state_dirs()?;
    install_pid_file(&ws)?;

    let config = Config::load(config_path)?;
    tracing::info!(
        config = %config_path.display(),
        source_root = %config.source_root.display(),
        work_dir = %config.work_dir.display(),
        listen = %config.listen_addr,
        model = %config.deepseek_model,
        pid = std::process::id(),
        "starting codeagentd"
    );

    let work_dir = config.work_dir.clone();
    let result = run_server_inner(&config).await;
    remove_pid_file(&ws);
    ws.cleanup_runtime_artifacts(&work_dir);
    result
}

async fn run_server_inner(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    process_util::become_session_leader();

    let compile_db = prepare(
        &config.source_root,
        &config.work_dir,
        &config.compile_commands,
        config.remote_build_prefix.as_deref(),
    )?;
    tracing::info!(
        compile_db_dir = %compile_db.compile_db_dir.display(),
        entries = compile_db.entry_count,
        "compile_commands ready for mcp-cpp"
    );

    let mcp = init_mcp(&config.source_root, true).await?;
    mcp.warmup(&compile_db).await;
    let tool_count = mcp.tool_count;
    let mcp_cell = mcp_holder(mcp);
    let tool_server = {
        let guard = mcp_cell.lock().await;
        guard
            .as_ref()
            .expect("mcp runtime present")
            .tool_server
            .clone()
    };

    let agent = build_agent(config, Some(&compile_db), tool_server)?;
    let state = AppState::new(config.clone(), Some(compile_db), tool_count, agent);

    let app = router(state);
    let listener = tokio::net::TcpListener::bind(config.listen_addr).await?;
    tracing::info!(
        addr = %config.listen_addr,
        tools = tool_count,
        pgid = std::process::id(),
        "codeagentd ready; multi-user requests will queue on agent mutex"
    );

    let mcp_for_signal = mcp_cell.clone();
    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown_holder(&mcp_for_signal).await;
        })
        .await;

    shutdown_holder(&mcp_cell).await;
    serve_result?;
    tracing::info!("codeagentd stopped");
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("failed to install SIGTERM handler");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("shutdown signal received (ctrl-c)");
        }
        _ = async {
            #[cfg(unix)]
            {
                if let Some(()) = sigterm.recv().await {
                    tracing::info!("shutdown signal received (sigterm)");
                }
            }
            #[cfg(not(unix))]
            {
                std::future::pending::<()>().await;
            }
        } => {}
    }
}
