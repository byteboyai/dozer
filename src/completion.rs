//! Dynamic shell-completion candidates sourced from the user's config.
//!
//! The pure `*_pairs` helpers turn a [`Config`] into `(value, help)` pairs and
//! are unit-tested. The public functions wrap them with a side-effect-free
//! config load and adapt them to clap_complete's [`CompletionCandidate`].

use crate::config::{self, Config};
use clap_complete::engine::CompletionCandidate;

/// Agent names with their launch command as help text.
fn agent_pairs(cfg: &Config) -> Vec<(String, String)> {
    cfg.agents
        .iter()
        .map(|(name, cmd)| (name.clone(), cmd.clone()))
        .collect()
}

/// Model ids with their display name as help text.
fn model_pairs(cfg: &Config) -> Vec<(String, String)> {
    cfg.models
        .iter()
        .map(|(id, m)| (id.clone(), m.name.clone()))
        .collect()
}

fn to_candidates(pairs: Vec<(String, String)>) -> Vec<CompletionCandidate> {
    pairs
        .into_iter()
        .map(|(value, help)| CompletionCandidate::new(value).help(Some(help.into())))
        .collect()
}

/// Candidate agent names for `dozer run <agent>`.
pub fn agents() -> Vec<CompletionCandidate> {
    to_candidates(agent_pairs(&config::try_load()))
}

/// Candidate model ids for `dozer model start|stop|restart|logs <id>`.
pub fn models() -> Vec<CompletionCandidate> {
    to_candidates(model_pairs(&config::try_load()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Config {
        config::parse(
            r#"
[agents]
claude = "claude"
aider = "aider"

[models.qwen36]
name = "Qwen3.6"
command = "mlx_lm.server"
model = "/m/q"
host = "127.0.0.1"
port = 7101
"#,
        )
        .unwrap()
    }

    #[test]
    fn agent_pairs_are_name_and_command_sorted() {
        // BTreeMap keeps keys sorted, so `aider` precedes `claude`.
        assert_eq!(
            agent_pairs(&sample()),
            vec![
                ("aider".to_string(), "aider".to_string()),
                ("claude".to_string(), "claude".to_string()),
            ]
        );
    }

    #[test]
    fn model_pairs_are_id_and_display_name() {
        assert_eq!(
            model_pairs(&sample()),
            vec![("qwen36".to_string(), "Qwen3.6".to_string())]
        );
    }

    #[test]
    fn empty_config_yields_no_candidates() {
        let cfg = Config::default();
        assert!(agent_pairs(&cfg).is_empty());
        assert!(model_pairs(&cfg).is_empty());
    }
}
