use rig_core::agent::Agent;
use rig_core::client::{CompletionClient, ProviderClientError};
use rig_core::providers::openai;
use rig_core::tool::server::ToolServerHandle;

use crate::agent::preamble::build_preamble;
use crate::compile_db::CompileDbContext;
use crate::config::Config;

pub type CodeAgent = Agent<openai::CompletionModel>;

pub fn build_agent(
    config: &Config,
    compile_db: Option<&CompileDbContext>,
    tool_server: ToolServerHandle,
) -> Result<CodeAgent, ProviderClientError> {
    let client = openai::Client::builder()
        .api_key(&config.deepseek_api_key)
        .base_url(&config.deepseek_base_url)
        .build()?
        .completions_api();

    let preamble = build_preamble(&config.source_root, compile_db);

    let agent = client
        .agent(&config.deepseek_model)
        .preamble(&preamble)
        .tool_server_handle(tool_server)
        .default_max_turns(1024)
        .build();

    Ok(agent)
}
