use std::sync::Arc;

use tokio::sync::Mutex;

use crate::agent::CodeAgent;
use crate::compile_db::CompileDbContext;
use crate::config::Config;
pub struct AppState {
    pub config: Arc<Config>,
    pub compile_db: Option<CompileDbContext>,
    pub tool_count: usize,
    pub agent: Mutex<CodeAgent>,
}

impl AppState {
    pub fn new(
        config: Config,
        compile_db: Option<CompileDbContext>,
        tool_count: usize,
        agent: CodeAgent,
    ) -> Arc<Self> {
        Arc::new(Self {
            config: Arc::new(config),
            compile_db,
            tool_count,
            agent: Mutex::new(agent),
        })
    }
}
