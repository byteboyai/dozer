//! "默认 agent"最小配置:本地 TOML 文件,用户手动改,没有 GUI 入口
//! (spec 2026-08-28)。每次补总结请求时惰性读取,不缓存,不需要重启
//! dozerd 生效。

use dozer_core::protocol::AgentKind;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct DefaultAgentConfig {
    default_agent: AgentKind,
}

/// 生产入口,固定用 `dozer_core::paths::config_dir().join("config.toml")`。
pub fn load_default_agent() -> AgentKind {
    load_default_agent_from(&dozer_core::paths::config_dir().join("config.toml"))
}

/// `path` 显式传入版本,测试用。文件不存在、读取失败、内容非法(缺字段/
/// `default_agent` 值不是四家已知 agent 之一)都回落到 `AgentKind::Claude`
/// 并记 `tracing::warn!`——不 panic,不阻塞补总结流程。
pub fn load_default_agent_from(path: &Path) -> AgentKind {
    let fallback = AgentKind::Claude;
    let Ok(text) = std::fs::read_to_string(path) else {
        tracing::warn!(path = %path.display(), "default_agent 配置文件不存在,回退到 Claude");
        return fallback;
    };
    match toml::from_str::<DefaultAgentConfig>(&text) {
        Ok(cfg) => cfg.default_agent,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "default_agent 配置解析失败,回退到 Claude");
            fallback
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_falls_back_to_claude() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        assert_eq!(load_default_agent_from(&path), AgentKind::Claude);
    }

    #[test]
    fn valid_config_returns_configured_agent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "default_agent = \"opencode\"\n").unwrap();
        assert_eq!(load_default_agent_from(&path), AgentKind::Opencode);
    }

    #[test]
    fn malformed_toml_falls_back_to_claude() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not valid toml {{{").unwrap();
        assert_eq!(load_default_agent_from(&path), AgentKind::Claude);
    }

    #[test]
    fn unknown_agent_value_falls_back_to_claude() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // `AgentKind` 反序列化遇到不认识的字符串会报错(不是静默变 Unknown,
        // 因为 `AgentKind` 没有 `#[serde(other)]`),走 malformed 同一条
        // 回退路径。
        std::fs::write(&path, "default_agent = \"chatgpt\"\n").unwrap();
        assert_eq!(load_default_agent_from(&path), AgentKind::Claude);
    }

    #[test]
    fn all_four_supported_agents_parse() {
        for (raw, expected) in [
            ("claude", AgentKind::Claude),
            ("codebuddy", AgentKind::Codebuddy),
            ("opencode", AgentKind::Opencode),
            ("v8agent", AgentKind::V8agent),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            std::fs::write(&path, format!("default_agent = \"{raw}\"\n")).unwrap();
            assert_eq!(load_default_agent_from(&path), expected);
        }
    }
}
