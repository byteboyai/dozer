use crate::config::Config;
use crate::error::ShyError;
use crate::process::is_installed;
use anyhow::Context;
use std::process::Command;

pub fn status_line(name: &str, installed: bool) -> String {
    let mark = if installed {
        "✓"
    } else {
        "✗ Not Installed"
    };
    format!("{name:<16}{mark}")
}

pub fn list(cfg: &Config) {
    println!("Available Agents");
    for (name, cmd) in &cfg.agents {
        println!("{}", status_line(name, is_installed(cmd)));
    }
}

pub fn doctor(cfg: &Config) {
    for (name, cmd) in &cfg.agents {
        println!("{}", status_line(name, is_installed(cmd)));
    }
}

pub fn run(cfg: &Config, agent: &str, args: &[String]) -> anyhow::Result<()> {
    let cmd = cfg
        .agents
        .get(agent)
        .ok_or_else(|| ShyError::AgentNotFound(agent.to_string()))?;
    let status = Command::new(cmd)
        .args(args)
        .status()
        .with_context(|| format!("launching agent '{agent}' ({cmd})"))?;
    if !status.success()
        && let Some(code) = status.code()
    {
        std::process::exit(code);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_shows_check() {
        assert_eq!(status_line("claude", true), "claude          ✓");
    }

    #[test]
    fn missing_shows_not_installed() {
        assert_eq!(
            status_line("aider", false),
            "aider           ✗ Not Installed"
        );
    }
}
