use std::path::{Path, PathBuf};

use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct CompileDbContext {
    /// Directory containing `compile_commands.json` for mcp-cpp `build_directory`.
    pub compile_db_dir: PathBuf,
    pub entry_count: usize,
    /// All source files listed in compile_commands (local absolute paths).
    pub source_files: Vec<PathBuf>,
    /// Subset whose filename is `main.cpp` (typical program entry).
    pub main_sources: Vec<PathBuf>,
}

#[derive(Debug, Error)]
pub enum CompileDbError {
    #[error("failed to read compile_commands: {0}")]
    Read(#[from] std::io::Error),
    #[error("failed to parse compile_commands: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("compile_commands must be a non-empty array")]
    Empty,
}

/// Prepare a writable compile_commands copy under `work_dir`.
///
/// The original `source` file and `source_root` are never modified.
pub fn prepare(
    source_root: &Path,
    work_dir: &Path,
    source: &Path,
    remote_prefix: Option<&str>,
) -> Result<CompileDbContext, CompileDbError> {
    std::fs::create_dir_all(work_dir)?;

    let dest = work_dir.join("compile_commands.json");
    let original_bytes = std::fs::read(source)?;

    let (output, entry_count) = match remote_prefix {
        Some(prefix) => {
            let mut root: Value = serde_json::from_slice(&original_bytes)?;
            let entries = root
                .as_array_mut()
                .filter(|a| !a.is_empty())
                .ok_or(CompileDbError::Empty)?;
            let entry_count = entries.len();

            let remote = normalize_prefix(prefix);
            let local = normalize_prefix(&source_root.to_string_lossy());
            for entry in entries.iter_mut() {
                remap_entry(entry, &remote, &local);
            }
            (serde_json::to_vec_pretty(&root)?, entry_count)
        }
        None => {
            let root: Value = serde_json::from_slice(&original_bytes)?;
            let entry_count = root
                .as_array()
                .filter(|a| !a.is_empty())
                .ok_or(CompileDbError::Empty)?
                .len();
            (original_bytes, entry_count)
        }
    };

    std::fs::write(&dest, &output)?;
    install_cmake_shim(work_dir, source_root)?;

    let source_files = collect_source_files(&output)?;
    let main_sources = pick_main_sources(&source_files);

    tracing::info!(
        source = %source.display(),
        work_dir = %work_dir.display(),
        dest = %dest.display(),
        entries = entry_count,
        sources = source_files.len(),
        main_sources = main_sources.len(),
        remapped = remote_prefix.is_some(),
        "installed compile_commands copy (source and source_root untouched)"
    );

    Ok(CompileDbContext {
        compile_db_dir: work_dir.to_path_buf(),
        entry_count,
        source_files,
        main_sources,
    })
}

fn collect_source_files(output: &[u8]) -> Result<Vec<PathBuf>, CompileDbError> {
    let root: Value = serde_json::from_slice(output)?;
    let entries = root
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or(CompileDbError::Empty)?;
    let mut files = Vec::new();
    for entry in entries {
        if let Some(path) = entry.get("file").and_then(|v| v.as_str()) {
            files.push(PathBuf::from(path));
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn pick_main_sources(files: &[PathBuf]) -> Vec<PathBuf> {
    files
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n == "main.cpp"))
        .cloned()
        .collect()
}

/// mcp-cpp only auto-discovers CMake/Meson build dirs. Makefile projects ship an
/// external compile_commands.json — add a minimal CMakeCache.txt so the directory
/// is accepted as a build_directory.
fn install_cmake_shim(work_dir: &Path, source_root: &Path) -> Result<(), CompileDbError> {
    let source = std::fs::canonicalize(source_root).unwrap_or_else(|_| source_root.to_path_buf());
    let cache = format!(
        "CMAKE_SOURCE_DIR:PATH={}\nCMAKE_BUILD_TYPE:STRING=Release\nCMAKE_GENERATOR:INTERNAL=Unix Makefiles\n",
        source.display()
    );
    std::fs::write(work_dir.join("CMakeCache.txt"), cache)?;
    Ok(())
}

fn remap_entry(entry: &mut Value, remote: &str, local: &str) {
    if let Some(obj) = entry.as_object_mut() {
        for key in ["directory", "file", "command"] {
            if let Some(Value::String(s)) = obj.get_mut(key) {
                *s = s.replace(remote, local);
            }
        }
    }
}

fn normalize_prefix(prefix: &str) -> String {
    let mut p = prefix.replace('\\', "/");
    while p.ends_with('/') && p.len() > 1 {
        p.pop();
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    const REMOTE_JSON: &str = r#"[
      {
        "directory": "/build_server/project/src",
        "command": "g++ -I/build_server/project/include -c /build_server/project/src/main.cpp",
        "file": "/build_server/project/src/main.cpp"
      }
    ]"#;

    #[test]
    fn remap_writes_only_to_work_dir() {
        let source_root = tempdir().unwrap();
        let work_dir = tempdir().unwrap();
        let source_file = source_root.path().join("g122_compile_commands.json");
        fs::write(&source_file, REMOTE_JSON).unwrap();

        let ctx = prepare(
            source_root.path(),
            work_dir.path(),
            &source_file,
            Some("/build_server/project"),
        )
        .unwrap();

        assert_eq!(ctx.compile_db_dir, work_dir.path());
        assert_eq!(ctx.entry_count, 1);
        assert_eq!(ctx.source_files.len(), 1);
        assert_eq!(ctx.main_sources.len(), 1);
        assert!(work_dir.path().join("compile_commands.json").is_file());
        assert!(work_dir.path().join("CMakeCache.txt").is_file());
        assert_eq!(fs::read(&source_file).unwrap(), REMOTE_JSON.as_bytes());

        let remapped: Value =
            serde_json::from_slice(&fs::read(work_dir.path().join("compile_commands.json")).unwrap())
                .unwrap();
        let local = normalize_prefix(&source_root.path().to_string_lossy());
        assert!(remapped[0]["directory"]
            .as_str()
            .unwrap()
            .starts_with(&local));
    }

    #[test]
    fn copy_without_remap_preserves_bytes() {
        let source_root = tempdir().unwrap();
        let work_dir = tempdir().unwrap();
        let source_file = source_root.path().join("my-db.json");
        fs::write(&source_file, REMOTE_JSON).unwrap();

        prepare(source_root.path(), work_dir.path(), &source_file, None).unwrap();

        assert_eq!(
            fs::read(work_dir.path().join("compile_commands.json")).unwrap(),
            REMOTE_JSON.as_bytes()
        );
        assert_eq!(fs::read(&source_file).unwrap(), REMOTE_JSON.as_bytes());
    }
}
