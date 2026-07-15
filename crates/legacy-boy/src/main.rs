mod agent;
mod cli;
mod completion;
mod config;
mod config_cmd;
mod doctor;
mod error;
mod model;
mod paths;
mod process;

use cli::{AgentCmd, Command, ConfigCmd, ModelCmd};

fn main() {
    // Handle shell-completion requests (COMPLETE=<shell> dozer ...) before any
    // other setup. When this is a completion call it prints candidates and
    // exits; otherwise it returns and normal execution continues.
    clap_complete::CompleteEnv::with_factory(<cli::Cli as clap::CommandFactory>::command)
        .complete();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    if let Err(e) = run() {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let cli = cli::parse();
    match cli.command {
        Command::Version => println!("Dozer {}", env!("CARGO_PKG_VERSION")),
        Command::Run { agent, args } => {
            let cfg = config::load_or_init()?;
            agent::run(&cfg, &agent, &args)?;
        }
        Command::Agent(a) => {
            let cfg = config::load_or_init()?;
            match a.command {
                AgentCmd::List => agent::list(&cfg),
                AgentCmd::Doctor => agent::doctor(&cfg),
            }
        }
        Command::Model(m) => {
            let cfg = config::load_or_init()?;
            match m.command {
                ModelCmd::List => model::list(&cfg),
                ModelCmd::Start { id } => model::start(&cfg, &id)?,
                ModelCmd::Stop { id } => model::stop(&cfg, &id)?,
                ModelCmd::Restart { id } => model::restart(&cfg, &id)?,
                ModelCmd::Status => model::status(&cfg)?,
                ModelCmd::Logs { id } => model::logs(&id)?,
            }
        }
        Command::Doctor => {
            let cfg = config::load_or_init()?;
            doctor::run(&cfg);
        }
        Command::Config(c) => match c.command {
            ConfigCmd::Show => config_cmd::show()?,
            ConfigCmd::Edit => config_cmd::edit()?,
        },
    }
    Ok(())
}
