use std::path::{Path, PathBuf};

pub const STATE_DIR_NAME: &str = ".codeagentd";
pub const SETTINGS_FILE_NAME: &str = "settings.toml";

/// Layout:
/// ```text
/// <workspace_root>/
///   .codeagentd/
///     settings.toml
///     run/codeagentd.pid
///     logs/codeagentd.log
///     tmp/           # default compile_commands scratch (work_dir)
///   g122app/         # project paths resolve from workspace_root
/// ```
#[derive(Debug, Clone)]
pub struct Workspace {
    pub workspace_root: PathBuf,
    pub state_dir: PathBuf,
    pub settings: PathBuf,
    pub pid_file: PathBuf,
    pub log_file: PathBuf,
    pub default_work_dir: PathBuf,
}

impl Workspace {
    pub fn default_settings_path() -> PathBuf {
        PathBuf::from(STATE_DIR_NAME).join(SETTINGS_FILE_NAME)
    }

    pub fn from_settings_path(settings: &Path) -> Self {
        let settings = settings.to_path_buf();
        let state_dir = settings
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

        let workspace_root = if state_dir
            .file_name()
            .is_some_and(|n| n == STATE_DIR_NAME)
        {
            state_dir
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf()
        } else {
            state_dir.clone()
        };

        Self {
            default_work_dir: state_dir.join("tmp"),
            pid_file: state_dir.join("run/codeagentd.pid"),
            log_file: state_dir.join("logs/codeagentd.log"),
            state_dir,
            settings,
            workspace_root,
        }
    }

    /// Directory used to resolve relative `project.*` paths in settings.toml.
    pub fn project_base(&self) -> &Path {
        &self.workspace_root
    }

    pub fn ensure_state_dirs(&self) -> std::io::Result<()> {
        for dir in [
            &self.state_dir,
            self.pid_file.parent().unwrap_or(&self.state_dir),
            self.log_file.parent().unwrap_or(&self.state_dir),
            &self.default_work_dir,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// Remove scratch work_dir, log file, and empty run/logs directories. Keeps settings.toml.
    pub fn cleanup_runtime_artifacts(&self, work_dir: &Path) {
        if work_dir.exists() {
            match std::fs::remove_dir_all(work_dir) {
                Ok(()) => tracing::info!(path = %work_dir.display(), "removed work directory"),
                Err(e) => tracing::warn!(path = %work_dir.display(), error = %e, "failed to remove work directory"),
            }
        }

        if self.log_file.is_file() {
            match std::fs::remove_file(&self.log_file) {
                Ok(()) => tracing::info!(path = %self.log_file.display(), "removed log file"),
                Err(e) => tracing::warn!(path = %self.log_file.display(), error = %e, "failed to remove log file"),
            }
        }

        Self::remove_dir_if_empty(self.log_file.parent());
        Self::remove_dir_if_empty(self.pid_file.parent());
    }

    fn remove_dir_if_empty(dir: Option<&Path>) {
        let Some(dir) = dir else { return };
        if dir.as_os_str().is_empty() {
            return;
        }
        let _ = std::fs::remove_dir(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_removes_tmp_and_log() {
        let base = std::env::temp_dir().join(format!("codeagentd-ws-test-{}", std::process::id()));
        let state = base.join(".codeagentd");
        let tmp = state.join("tmp");
        let logs = state.join("logs");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(tmp.join("compile_commands.json"), b"[]").unwrap();
        let log = logs.join("codeagentd.log");
        std::fs::write(&log, b"log").unwrap();

        let ws = Workspace::from_settings_path(&state.join("settings.toml"));
        ws.cleanup_runtime_artifacts(&tmp);

        assert!(!tmp.exists());
        assert!(!log.exists());
        assert!(!logs.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn standard_layout() {
        let ws = Workspace::from_settings_path(Path::new(".codeagentd/settings.toml"));
        assert_eq!(ws.workspace_root, PathBuf::from("."));
        assert_eq!(ws.state_dir, PathBuf::from(".codeagentd"));
        assert_eq!(ws.settings, PathBuf::from(".codeagentd/settings.toml"));
        assert_eq!(ws.pid_file, PathBuf::from(".codeagentd/run/codeagentd.pid"));
        assert_eq!(ws.log_file, PathBuf::from(".codeagentd/logs/codeagentd.log"));
        assert_eq!(ws.default_work_dir, PathBuf::from(".codeagentd/tmp"));
    }
}
