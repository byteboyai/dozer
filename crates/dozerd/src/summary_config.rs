//! summary 专属配置与解析优先级(spec 2026-09-26 第 5 节)。
//!
//! 独立于 Todo 的 `default_agent`——两者并存:`default_agent` 继续决定 Todo
//! 派发用哪个 agent(不改行为),`[summary]` 段决定总结用哪个 provider。选择
//! 优先级:本次 UI 显式选择 → `[summary]` 段 → 旧 `default_agent`(兼容来源,
//! UI 明示)。均未配置时返回 `configuration_required`,**不静默选择 Claude**。

use dozer_core::protocol::AgentKind;
use serde::Deserialize;
use std::path::Path;

/// 单次模型调用的默认 deadline(秒),spec 第 5 节"初始单调用 120 秒"。
pub const DEFAULT_CALL_TIMEOUT_SECS: u64 = 120;
/// 瞬态错误最大自动重试次数(spec 第 5 节"最多自动重试 2 次")。
pub const DEFAULT_MAX_RETRIES: u32 = 2;
/// 切块输入预算的默认值(字符数,保守估算),由 Task 3 的切块逻辑使用。
pub const DEFAULT_INPUT_BUDGET_CHARS: usize = 96_000;

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
struct RawSummaryConfig {
    provider: Option<AgentKind>,
    model: Option<String>,
    timeout_secs: Option<u64>,
    max_retries: Option<u32>,
    input_budget_chars: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawRootConfig {
    default_agent: Option<AgentKind>,
    summary: Option<RawSummaryConfig>,
}

/// 已解析的 summary 配置。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SummaryConfig {
    pub provider: AgentKind,
    pub model: Option<String>,
    pub call_timeout_secs: u64,
    pub max_retries: u32,
    pub input_budget_chars: usize,
}

/// 解析来源,供 UI 明示"这个 provider 从哪来"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryConfigSource {
    /// 本次 UI 明确选择。
    Ui,
    /// `[summary]` 段。
    Summary,
    /// 旧 `default_agent`(兼容来源,UI 明示为兼容来源)。
    DefaultAgent,
}

/// provider 解析结果:`Configured` 携带配置与来源;`Required` 表示未配置,
/// 调用方应记录 `configuration_required`,不静默选择 Claude、不启动必败进程。
#[derive(Debug, Clone, PartialEq)]
pub enum SummaryProviderResolution {
    Configured(SummaryConfig, SummaryConfigSource),
    Required,
}

/// 生产入口,固定用 `dozer_core::paths::config_dir().join("config.toml")`。
pub fn resolve_provider(
    ui_choice: Option<AgentKind>,
    ui_model: Option<String>,
) -> SummaryProviderResolution {
    resolve_provider_from(
        ui_choice,
        ui_model,
        &dozer_core::paths::config_dir().join("config.toml"),
    )
}

/// `path` 显式传入版本,测试用。配置非法(TOML 解析失败)时**不**回退到
/// Claude,而是连同"配置非法"一起计入 `Required`——spec 第 5 节"配置非法
/// 必须报告错误",静默回退会掩盖问题。
pub fn resolve_provider_from(
    ui_choice: Option<AgentKind>,
    ui_model: Option<String>,
    path: &Path,
) -> SummaryProviderResolution {
    if let Some(provider) = ui_choice {
        return SummaryProviderResolution::Configured(
            SummaryConfig {
                provider,
                model: ui_model,
                call_timeout_secs: DEFAULT_CALL_TIMEOUT_SECS,
                max_retries: DEFAULT_MAX_RETRIES,
                input_budget_chars: DEFAULT_INPUT_BUDGET_CHARS,
            },
            SummaryConfigSource::Ui,
        );
    }
    let root: Option<RawRootConfig> = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str(&text).ok());
    let Some(root) = root else {
        return SummaryProviderResolution::Required;
    };
    if let Some(summary) = root.summary
        && let Some(provider) = summary.provider
    {
        return SummaryProviderResolution::Configured(
            SummaryConfig {
                provider,
                model: summary.model,
                call_timeout_secs: summary.timeout_secs.unwrap_or(DEFAULT_CALL_TIMEOUT_SECS),
                max_retries: summary.max_retries.unwrap_or(DEFAULT_MAX_RETRIES),
                input_budget_chars: summary
                    .input_budget_chars
                    .unwrap_or(DEFAULT_INPUT_BUDGET_CHARS),
            },
            SummaryConfigSource::Summary,
        );
    }
    // 兼容来源:旧 default_agent。无 default_agent 也落到 Required。
    match root.default_agent {
        Some(provider) => SummaryProviderResolution::Configured(
            SummaryConfig {
                provider,
                model: None,
                call_timeout_secs: DEFAULT_CALL_TIMEOUT_SECS,
                max_retries: DEFAULT_MAX_RETRIES,
                input_budget_chars: DEFAULT_INPUT_BUDGET_CHARS,
            },
            SummaryConfigSource::DefaultAgent,
        ),
        None => SummaryProviderResolution::Required,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &std::path::Path, text: &str) -> std::path::PathBuf {
        let path = dir.join("config.toml");
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn ui_choice_wins_over_all() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "default_agent = \"claude\"\n[summary]\nprovider = \"opencode\"\n",
        );
        let r = resolve_provider_from(Some(AgentKind::Codex), Some("gpt-5".into()), &path);
        match r {
            SummaryProviderResolution::Configured(cfg, src) => {
                assert_eq!(cfg.provider, AgentKind::Codex);
                assert_eq!(cfg.model.as_deref(), Some("gpt-5"));
                assert_eq!(src, SummaryConfigSource::Ui);
            }
            _ => panic!("应解析到 UI 选择"),
        }
    }

    #[test]
    fn summary_section_wins_over_default_agent() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "default_agent = \"claude\"\n[summary]\nprovider = \"opencode\"\nmodel = \"m1\"\n",
        );
        let r = resolve_provider_from(None, None, &path);
        match r {
            SummaryProviderResolution::Configured(cfg, src) => {
                assert_eq!(cfg.provider, AgentKind::Opencode);
                assert_eq!(cfg.model.as_deref(), Some("m1"));
                assert_eq!(src, SummaryConfigSource::Summary);
            }
            _ => panic!("应解析到 summary 段"),
        }
    }

    #[test]
    fn default_agent_is_compat_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "default_agent = \"goose\"\n");
        let r = resolve_provider_from(None, None, &path);
        match r {
            SummaryProviderResolution::Configured(cfg, src) => {
                assert_eq!(cfg.provider, AgentKind::Goose);
                assert_eq!(src, SummaryConfigSource::DefaultAgent);
            }
            _ => panic!("应解析到 default_agent 兼容来源"),
        }
    }

    #[test]
    fn missing_config_returns_required() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        assert_eq!(
            resolve_provider_from(None, None, &path),
            SummaryProviderResolution::Required
        );
    }

    #[test]
    fn malformed_config_returns_required_not_claude() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "not valid toml {{{");
        assert_eq!(
            resolve_provider_from(None, None, &path),
            SummaryProviderResolution::Required
        );
    }

    #[test]
    fn empty_config_returns_required() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "");
        assert_eq!(
            resolve_provider_from(None, None, &path),
            SummaryProviderResolution::Required
        );
    }
}
