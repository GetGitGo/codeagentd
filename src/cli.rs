use std::path::PathBuf;

use crate::workspace::Workspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Run,
    Start,
    Stop,
    Restart,
}

pub struct Cli {
    pub command: Command,
    pub config: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("unknown command: {0} (use: start | stop | restart | run)")]
    Unknown(String),
    #[error("--config requires a path argument")]
    MissingConfigPath,
}

pub fn parse() -> Result<Cli, CliError> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let (command, rest) = match args.first().map(|s| s.as_str()) {
        None => (Command::Run, &args[..]),
        Some("run") => (Command::Run, &args[1..]),
        Some("start") => (Command::Start, &args[1..]),
        Some("stop") => (Command::Stop, &args[1..]),
        Some("restart") => (Command::Restart, &args[1..]),
        Some(s) if s.starts_with('-') => (Command::Run, &args[..]),
        Some(s) => return Err(CliError::Unknown(s.to_string())),
    };

    let mut config = None;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--config" => {
                i += 1;
                config = Some(
                    rest.get(i)
                        .ok_or(CliError::MissingConfigPath)?
                        .clone(),
                );
            }
            arg => {
                if let Some(path) = arg.strip_prefix("--config=") {
                    config = Some(path.to_string());
                }
            }
        }
        i += 1;
    }

    let config = config
        .map(PathBuf::from)
        .or_else(|| std::env::var("CODEAGENTD_CONFIG").ok().map(PathBuf::from))
        .unwrap_or_else(Workspace::default_settings_path);

    Ok(Cli { command, config })
}
