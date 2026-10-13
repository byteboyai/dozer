//! 多语言(i18n)领域数据类型(spec 2026-10-12-dozer-i18n-design §4/§6)。
//!
//! 本模块承诺线上 JSON 形状:locale 偏好、daemon 持久化快照、以及渲染用的
//! `LocalizedText` 描述。仅依赖 serde,不含 IO、UI 或解析逻辑——查表与插值在
//! `dozer-i18n` crate,持久化在 `dozerd`,展示在调用方(spec §5)。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 受支持的界面语言。首批 `en` 与 `zh-CN`(spec §4.1)。
///
/// 线上形状为 BCP-47 风格短串(`"en"`/`"zh-CN"`);解析/规范化规则在
/// `dozer-i18n`(spec §4.1 匹配规则),这里只承载已解析结果。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Locale(pub String);

impl Locale {
    pub const EN: &'static str = "en";
    pub const ZH_CN: &'static str = "zh-CN";

    /// 默认/兜底语言:英语资源为官方文案基准(spec §6.1)。
    pub fn default_locale() -> Self {
        Locale(Self::EN.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 界面语言偏好:System(跟随系统)/ 显式选择某个受支持语言(spec §4.1)。
///
/// `Explicit` 携带规范化后的 `Locale`,不携带非法值时由解析层拒绝。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LocalePreference {
    /// 跟随系统;探测由桌面平台适配器提供候选列表,system 模式才采用。
    #[default]
    System,
    /// 用户显式选择;不受系统语言变化影响。
    Explicit { locale: Locale },
}

/// daemon 持久化的语言设置快照(spec §4.1/§4.2)。
///
/// - `schema_version`:持久化格式版本,用于向前兼容。
/// - `preference`:用户偏好。
/// - `active_locale`:`preference` 解析后的实际生效语言。
/// - `revision`:daemon 串行提交且落盘时递增;客户端用它做 CAS。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocaleSnapshot {
    pub schema_version: u32,
    pub preference: LocalePreference,
    pub active_locale: Locale,
    pub revision: u64,
}

impl LocaleSnapshot {
    /// 当前持久化 schema 版本。
    pub const SCHEMA_VERSION: u32 = 1;

    /// 无持久化文件时的默认快照:跟随系统,生效语言为 `en`,revision 0。
    pub fn default_snapshot() -> Self {
        LocaleSnapshot {
            schema_version: Self::SCHEMA_VERSION,
            preference: LocalePreference::System,
            active_locale: Locale::default_locale(),
            revision: 0,
        }
    }
}

/// 渲染一条界面文案所需的最小描述(spec §6.1)。
///
/// - `owner`:文案所有者,如 `dozer.host`/`dozer.todo`;由注册来源绑定,
///   不可由不可信调用方任意指定其他 owner。
/// - `key`:可选的资源 key;为 `None` 时直接用 `fallback`。
/// - `fallback`:必填非空的安全文案(官方为英文),解析失败时的最终兜底。
/// - `args`:具名插值参数(spec §6.2),`BTreeMap` 保证渲染确定性。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizedText {
    pub owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub fallback: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, String>,
}

impl LocalizedText {
    /// 只带 fallback 的文案(无 key、无参数)。
    pub fn literal(owner: impl Into<String>, fallback: impl Into<String>) -> Self {
        LocalizedText {
            owner: owner.into(),
            key: None,
            fallback: fallback.into(),
            args: BTreeMap::new(),
        }
    }

    /// 带 key 与 fallback 的文案。
    pub fn keyed(
        owner: impl Into<String>,
        key: impl Into<String>,
        fallback: impl Into<String>,
    ) -> Self {
        LocalizedText {
            owner: owner.into(),
            key: Some(key.into()),
            fallback: fallback.into(),
            args: BTreeMap::new(),
        }
    }

    /// 追加一个具名参数(链式)。
    pub fn with_arg(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.args.insert(name.into(), value.into());
        self
    }
}
