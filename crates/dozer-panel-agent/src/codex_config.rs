//! Codex 启动到第一条消息之间不触发任何 hook(实测 codex-cli 0.163:`SessionStart` 在第一条
//! 消息发出时才来),卡片此时没有任何模型数据来源。这里读 Codex 配置里的默认模型当兜底;
//! 第一个 hook 事件到达后,转录文件里的真实模型会覆盖它(`card::apply_refresh`)。

use dozer_model::AgentKind;
use std::path::PathBuf;

/// 顶层(第一个 `[表]` 之前)的 `model = "…"`。不认 `model_provider` 等同前缀的键,
/// 也不读 `[profiles.*]` 里的 `model`(用户没选 profile 时它们不生效)。
pub fn parse_default_model(config: &str) -> Option<String> {
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            return None;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "model" {
            continue;
        }
        let value = value.split('#').next().unwrap_or("").trim();
        let unquoted = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))?;
        return (!unquoted.is_empty()).then(|| unquoted.to_string());
    }
    None
}

fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex")))?;
    Some(home.join("config.toml"))
}

/// 读不到文件、没配 `model`,都是 `None`(不报错:这只是兜底)。
pub fn default_model() -> Option<String> {
    parse_default_model(&std::fs::read_to_string(config_path()?).ok()?)
}

/// 新页签的初始模型:只有 Codex 需要兜底(Claude 等的 `SessionStart` 在启动时就到)。
pub fn initial_llm_model(
    agent: AgentKind,
    read: impl FnOnce() -> Option<String>,
) -> Option<String> {
    (agent == AgentKind::Codex).then(read).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_top_level_model() {
        assert_eq!(
            parse_default_model("model = \"gpt-6.1-sol\"\nmodel_provider = \"x\"\n").as_deref(),
            Some("gpt-6.1-sol")
        );
    }

    #[test]
    fn ignores_lookalike_keys_and_comments() {
        let cfg =
            "# model = \"no\"\nmodel_provider = \"openai\"\nmodel_reasoning_effort = \"high\"\n";
        assert_eq!(parse_default_model(cfg), None);
        assert_eq!(
            parse_default_model("model = 'a-b'   # trailing\n").as_deref(),
            Some("a-b")
        );
    }

    #[test]
    fn does_not_read_models_inside_tables() {
        let cfg = "[profiles.fast]\nmodel = \"small\"\n";
        assert_eq!(parse_default_model(cfg), None);
        // 顶层的在前、表里的在后:取顶层。
        let cfg = "model = \"top\"\n[profiles.fast]\nmodel = \"small\"\n";
        assert_eq!(parse_default_model(cfg).as_deref(), Some("top"));
    }

    #[test]
    fn empty_or_unquoted_values_are_not_models() {
        assert_eq!(parse_default_model("model = \"\"\n"), None);
        assert_eq!(parse_default_model("model = gpt\n"), None);
        assert_eq!(parse_default_model(""), None);
    }

    #[test]
    fn only_codex_gets_a_fallback_model() {
        assert_eq!(
            initial_llm_model(AgentKind::Codex, || Some("m".into())).as_deref(),
            Some("m")
        );
        assert_eq!(
            initial_llm_model(AgentKind::Claude, || Some("m".into())),
            None
        );
        assert_eq!(initial_llm_model(AgentKind::Codex, || None), None);
    }
}
