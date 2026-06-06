use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::compile_db::CompileDbContext;

/// tree-sitter MCP project handle (name + registry root differ from source_root).
#[derive(Debug, Clone)]
pub struct TreeSitterContext {
    pub project_name: String,
    pub registry_root: PathBuf,
    /// Paths relative to `registry_root` for `get_file` / `get_symbols`.
    pub main_entry_paths: Vec<String>,
}

impl TreeSitterContext {
    pub fn from_registration(
        registration_json: &str,
        compile_db: Option<&CompileDbContext>,
    ) -> Result<Self, String> {
        let root: Value = serde_json::from_str(registration_json)
            .map_err(|e| format!("invalid registration JSON: {e}"))?;
        let project_name = root
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "registration missing name".to_string())?
            .to_string();
        let registry_root = root
            .get("root_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "registration missing root_path".to_string())?;
        let registry_root = std::fs::canonicalize(registry_root).unwrap_or_else(|_| {
            PathBuf::from(registry_root)
        });

        let main_entry_paths = compile_db
            .map(|db| entry_paths_relative_to(&registry_root, &db.main_sources))
            .unwrap_or_default();

        Ok(Self {
            project_name,
            registry_root,
            main_entry_paths,
        })
    }
}

pub fn project_name_from_path(project_root: &Path) -> String {
    project_root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "project".to_string())
}

pub fn entry_paths_relative_to(registry_root: &Path, files: &[PathBuf]) -> Vec<String> {
    files
        .iter()
        .filter_map(|p| path_relative_to(registry_root, p))
        .collect()
}

fn path_relative_to(base: &Path, file: &Path) -> Option<String> {
    let base = std::fs::canonicalize(base).ok()?;
    let file = std::fs::canonicalize(file).ok()?;
    let rel = file.strip_prefix(&base).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parses_registration_and_relative_paths() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("g122app");
        let main = src.join("app/main.cpp");
        fs::create_dir_all(main.parent().unwrap()).unwrap();
        fs::write(&main, "int main() {}").unwrap();

        let json = format!(
            r#"{{"name":"g122app","root_path":"{}"}}"#,
            dir.path().display()
        );
        let db = CompileDbContext {
            compile_db_dir: dir.path().to_path_buf(),
            entry_count: 1,
            source_files: vec![main.clone()],
            main_sources: vec![main],
        };
        let ctx = TreeSitterContext::from_registration(&json, Some(&db)).unwrap();
        assert_eq!(ctx.project_name, "g122app");
        assert_eq!(ctx.main_entry_paths, vec!["g122app/app/main.cpp"]);
    }
}
