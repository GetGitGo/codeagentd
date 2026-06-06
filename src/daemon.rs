use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::workspace::Workspace;
use thiserror::Error;

const STOP_TIMEOUT: Duration = Duration::from_secs(20);
/// Extra time after main process exits for MCP child processes to reap.
const MCP_REAP_GRACE: Duration = Duration::from_millis(800);
const START_WAIT: Duration = Duration::from_secs(90);

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("codeagentd already running (pid {pid})")]
    AlreadyRunning { pid: u32 },
    #[error("codeagentd is not running")]
    NotRunning,
    #[error("failed to read pid file: {0}")]
    ReadPid(#[from] std::io::Error),
    #[error("invalid pid in {path}: {contents}")]
    InvalidPid { path: PathBuf, contents: String },
    #[error("failed to start daemon: {0}")]
    Start(String),
    #[error("daemon exited before ready")]
    StartFailed,
}

pub fn install_pid_file(ws: &Workspace) -> Result<(), std::io::Error> {
    if let Some(parent) = ws.pid_file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&ws.pid_file, std::process::id().to_string())
}

pub fn remove_pid_file(ws: &Workspace) {
    let _ = fs::remove_file(&ws.pid_file);
}

fn cleanup_after_stop(ws: &Workspace, config: &Path) {
    let work_dir = match crate::config::Config::load(config) {
        Ok(cfg) => cfg.work_dir,
        Err(_) => ws.default_work_dir.clone(),
    };
    ws.cleanup_runtime_artifacts(&work_dir);
}

pub fn start(config: &Path) -> Result<(), DaemonError> {
    let ws = Workspace::from_settings_path(config);
    ws.ensure_state_dirs()
        .map_err(|e| DaemonError::Start(e.to_string()))?;

    if let Some(pid) = read_pid_if_alive(&ws.pid_file)? {
        return Err(DaemonError::AlreadyRunning { pid });
    }
    cleanup_stale_pid(&ws.pid_file);

    let exe = std::env::current_exe().map_err(|e| DaemonError::Start(e.to_string()))?;
    let config_abs = fs::canonicalize(config).unwrap_or_else(|_| config.to_path_buf());
    let cwd = fs::canonicalize(&ws.workspace_root).unwrap_or(ws.workspace_root.clone());

    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&ws.log_file)
        .map_err(|e| DaemonError::Start(e.to_string()))?;

    let child = Command::new(exe)
        .current_dir(&cwd)
        .arg("run")
        .arg("--config")
        .arg(&config_abs)
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| DaemonError::Start(e.to_string()))?)
        .stderr(log)
        .spawn()
        .map_err(|e| DaemonError::Start(e.to_string()))?;

    let pid = child.id();
    fs::write(&ws.pid_file, pid.to_string()).map_err(DaemonError::ReadPid)?;

    if wait_until_ready(config, START_WAIT) {
        println!("codeagentd started (pid {pid})");
        Ok(())
    } else {
        cleanup_stale_pid(&ws.pid_file);
        if process_alive(pid) {
            let _ = signal_pid(pid, "TERM");
        }
        Err(DaemonError::StartFailed)
    }
}

pub fn stop(config: &Path) -> Result<(), DaemonError> {
    let ws = Workspace::from_settings_path(config);
    let pid = match read_pid_if_alive(&ws.pid_file)? {
        Some(pid) => pid,
        None => {
            cleanup_stale_pid(&ws.pid_file);
            cleanup_after_stop(&ws, config);
            return Err(DaemonError::NotRunning);
        }
    };

    if !signal_pid(pid, "TERM") {
        cleanup_stale_pid(&ws.pid_file);
        cleanup_after_stop(&ws, config);
        return Err(DaemonError::NotRunning);
    }

    let deadline = std::time::Instant::now() + STOP_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if !process_alive(pid) {
            thread::sleep(MCP_REAP_GRACE);
            cleanup_stale_pid(&ws.pid_file);
            cleanup_process_group(pid);
            cleanup_after_stop(&ws, config);
            println!("codeagentd stopped (pid {pid})");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }

    // Force-kill the whole process group (codeagentd + MCP children).
    let _ = signal_process_group(pid, "TERM");
    thread::sleep(Duration::from_millis(500));
    let _ = signal_process_group(pid, "KILL");
    let _ = signal_pid(pid, "KILL");
    thread::sleep(Duration::from_millis(300));
    cleanup_stale_pid(&ws.pid_file);
    cleanup_process_group(pid);

    if process_alive(pid) {
        return Err(DaemonError::Start(format!(
            "failed to stop pid {pid} after timeout"
        )));
    }

    thread::sleep(MCP_REAP_GRACE);
    cleanup_after_stop(&ws, config);
    println!("codeagentd stopped (pid {pid}, killed)");
    Ok(())
}

pub fn restart(config: &Path) -> Result<(), DaemonError> {
    match stop(config) {
        Ok(()) | Err(DaemonError::NotRunning) => {}
        Err(e) => return Err(e),
    }
    start(config)
}

fn read_pid_if_alive(pid_file: &Path) -> Result<Option<u32>, DaemonError> {
    if !pid_file.is_file() {
        return Ok(None);
    }
    let contents = fs::read_to_string(pid_file).map_err(DaemonError::ReadPid)?;
    let pid: u32 = contents
        .trim()
        .parse()
        .map_err(|_| DaemonError::InvalidPid {
            path: pid_file.to_path_buf(),
            contents,
        })?;
    if process_alive(pid) {
        Ok(Some(pid))
    } else {
        Ok(None)
    }
}

fn cleanup_stale_pid(pid_file: &Path) {
    let _ = fs::remove_file(pid_file);
}

fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn signal_pid(pid: u32, sig: &str) -> bool {
    Command::new("kill")
        .args([format!("-{sig}"), pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Signal every process in codeagentd's process group (negative pgid).
#[cfg(unix)]
fn signal_process_group(pgid: u32, sig: &str) -> bool {
    Command::new("kill")
        .args([format!("-{sig}"), format!("-{pgid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn signal_process_group(pid: u32, sig: &str) -> bool {
    signal_pid(pid, sig)
}

/// Reap any MCP subprocesses that outlived the main daemon.
#[cfg(unix)]
fn cleanup_process_group(pgid: u32) {
    let _ = signal_process_group(pgid, "KILL");
}

#[cfg(not(unix))]
fn cleanup_process_group(_pgid: u32) {}

fn wait_until_ready(config: &Path, timeout: Duration) -> bool {
    let listen_addr = match crate::config::Config::load(config) {
        Ok(c) => c.listen_addr,
        Err(_) => return false,
    };

    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect_timeout(&listen_addr, Duration::from_millis(500)).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(300));
    }
    false
}
