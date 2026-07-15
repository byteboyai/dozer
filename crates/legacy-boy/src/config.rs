use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, PartialEq)]
pub struct Model {
    pub name: String,
    pub command: String,
    pub model: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, PartialEq, Default)]
pub struct Config {
    #[serde(default)]
    pub agents: BTreeMap<String, String>,
    #[serde(default)]
    pub models: BTreeMap<String, Model>,
}

pub fn parse(s: &str) -> anyhow::Result<Config> {
    Ok(toml::from_str(s)?)
}

use anyhow::Context;

pub fn default_template() -> &'static str {
    r#"[agents]
claude = "claude"
codex = "codex"
hermes = "hermes"
aider = "aider"
codebuddy = "codebuddy"

# [models.qwen36]
# name = "Qwen3.6"
# command = "mlx_lm.server"
# model = "/Users/you/models/Qwen3.6"
# host = "127.0.0.1"
# port = 7101
"#
}

/// Read the config without creating or modifying it. Returns an empty config if
/// the file is missing or unreadable. Used by shell completion, which must be
/// fast and free of side effects.
pub fn try_load() -> Config {
    crate::paths::config_file()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| parse(&s).ok())
        .unwrap_or_default()
}

pub fn load_or_init() -> anyhow::Result<Config> {
    let path = crate::paths::config_file()?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating config dir {}", parent.display()))?;
        }
        std::fs::write(&path, default_template())
            .with_context(|| format!("writing default config {}", path.display()))?;
        eprintln!("Created default config at {}", path.display());
    }
    let s = std::fs::read_to_string(&path)
        .with_context(|| format!("reading config {}", path.display()))?;
    parse(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agents_and_models() {
        let toml = r#"
[agents]
claude = "claude"

[models.qwen36]
name = "Qwen3.6"
command = "mlx_lm.server"
model = "/m/q"
host = "127.0.0.1"
port = 7101
"#;
        let cfg = parse(toml).unwrap();
        assert_eq!(cfg.agents.get("claude").unwrap(), "claude");
        let m = cfg.models.get("qwen36").unwrap();
        assert_eq!(m.port, 7101);
        assert_eq!(m.host, "127.0.0.1");
    }

    #[test]
    fn empty_config_parses_to_defaults() {
        let cfg = parse("").unwrap();
        assert!(cfg.agents.is_empty());
        assert!(cfg.models.is_empty());
    }

    #[test]
    fn default_template_is_valid() {
        let cfg = parse(default_template()).unwrap();
        assert!(cfg.agents.contains_key("claude"));
    }
}
