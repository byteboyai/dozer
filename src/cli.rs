use clap::{Args, Parser, Subcommand};
use clap_complete::engine::ArgValueCandidates;

use crate::completion;

#[derive(Parser)]
#[command(name = "dozer", about = "Dozer AI CLI", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Print version
    Version,
    /// Launch an agent CLI
    #[command(disable_help_flag = true)]
    Run {
        #[arg(add = ArgValueCandidates::new(completion::agents))]
        agent: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Agent management
    Agent(AgentArgs),
    /// MLX model management
    Model(ModelArgs),
    /// Check the dev environment
    Doctor,
    /// Configuration
    Config(ConfigArgs),
}

#[derive(Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub command: AgentCmd,
}

#[derive(Subcommand)]
pub enum AgentCmd {
    List,
    Doctor,
}

#[derive(Args)]
pub struct ModelArgs {
    #[command(subcommand)]
    pub command: ModelCmd,
}

#[derive(Subcommand)]
pub enum ModelCmd {
    List,
    Start {
        #[arg(add = ArgValueCandidates::new(completion::models))]
        id: String,
    },
    Stop {
        #[arg(add = ArgValueCandidates::new(completion::models))]
        id: String,
    },
    Restart {
        #[arg(add = ArgValueCandidates::new(completion::models))]
        id: String,
    },
    Status,
    Logs {
        #[arg(add = ArgValueCandidates::new(completion::models))]
        id: String,
    },
}

#[derive(Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCmd,
}

#[derive(Subcommand)]
pub enum ConfigCmd {
    Show,
    Edit,
}

pub fn parse() -> Cli {
    Cli::parse()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_model_start() {
        let cli = Cli::try_parse_from(["dozer", "model", "start", "qwen36"]).unwrap();
        match cli.command {
            Command::Model(m) => match m.command {
                ModelCmd::Start { id } => assert_eq!(id, "qwen36"),
                _ => panic!("wrong model subcommand"),
            },
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn run_collects_trailing_args() {
        let cli = Cli::try_parse_from(["dozer", "run", "claude", "--help", "-p"]).unwrap();
        match cli.command {
            Command::Run { agent, args } => {
                assert_eq!(agent, "claude");
                assert_eq!(args, vec!["--help", "-p"]);
            }
            _ => panic!("wrong command"),
        }
    }
}
