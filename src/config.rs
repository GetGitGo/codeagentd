use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use crate::workspace::Workspace;

pub const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:3000";
pub const DEFAULT_LLM_BASE_URL: &str = "https://api.deepseek.com";
pub const DEFAULT_LLM_MODEL: &str = "deepseek-v4-pro";

#[derive(Debug, Clone)]
pub struct Config {
    pub deepseek_api_key: String,
    pub deepseek_base_url: String,
    pub deepseek_model: String,
    /// Read-only C/C++ source tree (never written by codeagentd).
    pub source_root: PathBuf,
    /// Original compile_commands file; any path and filename. Never modified.
    pub compile_commands: PathBuf,
    pub remote_build_prefix: Option<String>,
    /// Writable scratch directory for remapped compile_commands copies.
    pub work_dir: PathBuf,
    pub listen_addr: SocketAddr,
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    listen_addr: Option<String>,
    work_dir: Option<String>,
    project: ProjectSection,
    llm: Option<LlmSection>,
}

#[derive(Debug, Deserialize)]
struct ProjectSection {
    source_root: String,
    compile_commands: Option<String>,
    remote_build_prefix: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LlmSection {
    api_key: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config file {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("missing required config: {0}")]
    Missing(&'static str),
    #[error("missing environment variable: {0}")]
    MissingEnv(&'static str),
    #[error("invalid {field}: {message}")]
    Invalid { field: &'static str, message: String },
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        dotenvy::dotenv().ok();

        let path = path.to_path_buf();
        let raw = std::fs::read_to_string(&path).map_err(|e| ConfigError::Read {
            path: path.clone(),
            source: e,
        })?;
        let file: FileConfig = toml::from_str(&raw).map_err(|e| ConfigError::Parse {
            path: path.clone(),
            source: e,
        })?;

        let workspace = Workspace::from_settings_path(&path);
        let base = workspace.project_base();

        let source_root = require_non_empty(&file.project.source_root, "project.source_root")?;
        let source_root =
            resolve_existing_dir(base, source_root, "project.source_root")?;

        let compile_commands = require_optional_nonempty(
            file.project.compile_commands.as_deref(),
            "project.compile_commands",
        )?;
        let compile_commands =
            resolve_existing_file(base, compile_commands, "project.compile_commands")?;
        validate_compile_commands_file(&compile_commands)?;

        let work_dir = resolve_work_dir(base, &workspace, file.work_dir.as_deref())?;

        let deepseek_api_key = file
            .llm
            .as_ref()
            .and_then(|l| l.api_key.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok())
            .filter(|s| !s.is_empty())
            .ok_or(ConfigError::MissingEnv("DEEPSEEK_API_KEY"))?;

        let deepseek_base_url = match optional_trimmed(file.llm.as_ref().and_then(|l| l.base_url.as_deref())) {
            Some(url) => validate_base_url(url)?,
            None => DEFAULT_LLM_BASE_URL.to_string(),
        };

        let deepseek_model = optional_trimmed(file.llm.as_ref().and_then(|l| l.model.as_deref()))
            .unwrap_or(DEFAULT_LLM_MODEL)
            .to_string();

        let listen_addr = match optional_trimmed(file.listen_addr.as_deref()) {
            Some(addr) => parse_listen_addr(addr)?,
            None => parse_listen_addr(DEFAULT_LISTEN_ADDR)?,
        };

        let remote_build_prefix =
            optional_trimmed(file.project.remote_build_prefix.as_deref()).map(str::to_string);

        Ok(Self {
            deepseek_api_key,
            deepseek_base_url,
            deepseek_model,
            source_root,
            compile_commands,
            remote_build_prefix,
            work_dir,
            listen_addr,
        })
    }
}

fn require_non_empty<'a>(value: &'a str, field: &'static str) -> Result<&'a str, ConfigError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(ConfigError::Missing(field))
    } else {
        Ok(trimmed)
    }
}

fn require_optional_nonempty<'a>(
    value: Option<&'a str>,
    field: &'static str,
) -> Result<&'a str, ConfigError> {
    match value {
        Some(v) if !v.trim().is_empty() => Ok(v.trim()),
        _ => Err(ConfigError::Missing(field)),
    }
}

fn optional_trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn parse_listen_addr(addr: &str) -> Result<SocketAddr, ConfigError> {
    addr.parse().map_err(|e: std::net::AddrParseError| ConfigError::Invalid {
        field: "listen_addr",
        message: e.to_string(),
    })
}

fn validate_base_url(url: &str) -> Result<String, ConfigError> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(url.to_string())
    } else {
        Err(ConfigError::Invalid {
            field: "llm.base_url",
            message: format!("must start with http:// or https://, got: {url}"),
        })
    }
}

fn validate_compile_commands_file(path: &Path) -> Result<(), ConfigError> {
    let bytes = std::fs::read(path).map_err(|e| ConfigError::Invalid {
        field: "project.compile_commands",
        message: format!("cannot read {}: {e}", path.display()),
    })?;
    let root: Value = serde_json::from_slice(&bytes).map_err(|e| ConfigError::Invalid {
        field: "project.compile_commands",
        message: format!("invalid JSON in {}: {e}", path.display()),
    })?;
    let Some(entries) = root.as_array() else {
        return Err(ConfigError::Invalid {
            field: "project.compile_commands",
            message: "must be a JSON array".into(),
        });
    };
    if entries.is_empty() {
        return Err(ConfigError::Invalid {
            field: "project.compile_commands",
            message: "must be a non-empty JSON array".into(),
        });
    }
    Ok(())
}

fn resolve_work_dir(
    base: &Path,
    workspace: &Workspace,
    value: Option<&str>,
) -> Result<PathBuf, ConfigError> {
    let path = match optional_trimmed(value) {
        Some(p) => resolve_path(base, p),
        None => workspace.default_work_dir.clone(),
    };
    std::fs::create_dir_all(&path).map_err(|e| ConfigError::Invalid {
        field: "work_dir",
        message: format!("cannot create {}: {e}", path.display()),
    })?;
    Ok(path)
}

fn resolve_path(base: &Path, path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        p
    } else {
        base.join(p)
    }
}

fn resolve_existing_dir(base: &Path, path: &str, field: &'static str) -> Result<PathBuf, ConfigError> {
    let p = resolve_path(base, path);
    std::fs::canonicalize(&p).map_err(|e| ConfigError::Invalid {
        field,
        message: format!("not a directory: {} ({e})", p.display()),
    })
}

fn resolve_existing_file(base: &Path, path: &str, field: &'static str) -> Result<PathBuf, ConfigError> {
    let p = resolve_path(base, path);
    if !p.is_file() {
        return Err(ConfigError::Invalid {
            field,
            message: format!("not a file: {}", p.display()),
        });
    }
    Ok(std::fs::canonicalize(&p).unwrap_or(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_settings(dir: &Path, body: &str) -> PathBuf {
        let state = dir.join(".codeagentd");
        fs::create_dir_all(&state).unwrap();
        let path = state.join("settings.toml");
        fs::write(&path, body).unwrap();
        path
    }

    fn touch_project_layout(root: &Path) {
        fs::create_dir_all(root.join("g122app")).unwrap();
        fs::write(
            root.join("g122_compile_commands.json"),
            br#"[{"directory":"/x","file":"/x/a.cpp","command":"c++ a.cpp"}]"#,
        )
        .unwrap();
    }

    #[test]
    fn rejects_missing_compile_commands() {
        let root = tempdir().unwrap();
        touch_project_layout(root.path());
        let path = write_settings(
            root.path(),
            r#"
[project]
source_root = "g122app"
"#,
        );
        let err = Config::load(&path).unwrap_err();
        assert!(matches!(err, ConfigError::Missing("project.compile_commands")));
    }

    #[test]
    fn rejects_empty_source_root() {
        let root = tempdir().unwrap();
        touch_project_layout(root.path());
        let path = write_settings(
            root.path(),
            r#"
[project]
source_root = "  "
compile_commands = "g122_compile_commands.json"
"#,
        );
        let err = Config::load(&path).unwrap_err();
        assert!(matches!(err, ConfigError::Missing("project.source_root")));
    }

    #[test]
    fn applies_defaults_for_optional_fields() {
        let root = tempdir().unwrap();
        touch_project_layout(root.path());
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");
        let path = write_settings(
            root.path(),
            r#"
[project]
source_root = "g122app"
compile_commands = "g122_compile_commands.json"
"#,
        );
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.listen_addr, "0.0.0.0:3000".parse().unwrap());
        assert_eq!(cfg.deepseek_base_url, DEFAULT_LLM_BASE_URL);
        assert_eq!(cfg.deepseek_model, DEFAULT_LLM_MODEL);
        assert_eq!(cfg.work_dir, root.path().join(".codeagentd/tmp"));
    }

    #[test]
    fn rejects_invalid_listen_addr() {
        let root = tempdir().unwrap();
        touch_project_layout(root.path());
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");
        let path = write_settings(
            root.path(),
            r#"
listen_addr = "not-an-addr"
[project]
source_root = "g122app"
compile_commands = "g122_compile_commands.json"
"#,
        );
        let err = Config::load(&path).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { field: "listen_addr", .. }));
    }

    #[test]
    fn rejects_invalid_base_url() {
        let root = tempdir().unwrap();
        touch_project_layout(root.path());
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");
        let path = write_settings(
            root.path(),
            r#"
[project]
source_root = "g122app"
compile_commands = "g122_compile_commands.json"
[llm]
base_url = "ftp://bad"
"#,
        );
        let err = Config::load(&path).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { field: "llm.base_url", .. }));
    }
}
