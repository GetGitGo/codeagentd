use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::agent::CodeAgent;
use crate::compile_db::CompileDbContext;
use crate::config::Config;
use crate::tree_sitter::TreeSitterContext;

pub struct AppState {
    pub settings_path: PathBuf,
    pub config: Arc<Config>,
    pub compile_db: Option<CompileDbContext>,
    pub ts_context: Option<TreeSitterContext>,
    pub tool_count: usize,
    pub agent: Mutex<CodeAgent>,
}

impl AppState {
    pub fn new(
        settings_path: PathBuf,
        config: Config,
        compile_db: Option<CompileDbContext>,
        ts_context: Option<TreeSitterContext>,
        tool_count: usize,
        agent: CodeAgent,
    ) -> Arc<Self> {
        Arc::new(Self {
            settings_path,
            config: Arc::new(config),
            compile_db,
            ts_context,
            tool_count,
            agent: Mutex::new(agent),
        })
    }
}
