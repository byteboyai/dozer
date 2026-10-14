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

/// `set_locale` 的结果(spec §4.2):提交成功返回新快照;期望 revision 不匹配
/// 时返回当前最新快照并标记冲突,提示重新选择,不自动覆盖另一个窗口的操作。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum LocaleWriteOutcome {
    /// 提交成功,`snapshot` 为落盘后的权威快照。
    Applied { snapshot: LocaleSnapshot },
    /// revision 冲突;`snapshot` 为服务端当前最新快照,调用方应据此重新选择。
    Conflict { snapshot: LocaleSnapshot },
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
    /// 裸字符串转成 `LocalizedText` 时使用的默认 owner(host 词表)。
    pub const DEFAULT_OWNER: &'static str = "dozer.host";

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

/// 内置 webview 推送 envelope 的语言上下文(spec §8)。
///
/// 追加到既有 envelope 的可选字段,**不改变** `payload` 字段意义:
/// - `ui_locale`:当前生效语言(`en`/`zh-CN`),供前端查表渲染。
/// - `locale_revision`:GUI **本地显示代**(`LocaleController::display_generation`),
///   仅在生效语言真正变化时递增;**不与** daemon 的持久化 revision 比大小。
///
/// 序列化形状与 spec §8 示例一致(`{"ui_locale":"zh-CN","locale_revision":3}`),
/// 由各 envelope 以 `#[serde(flatten)]` 内联。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocaleStamp {
    pub ui_locale: Locale,
    pub locale_revision: u64,
}

/// 携带语言上下文的推送载荷载体(非线上形状,仅供宿主本地比较)。
///
/// `SurfaceExecutor<T>` 的逐帧去重比较的是 `T` 本身;把语言上下文并入 `T`,就能让
/// "业务数据不变、仅语言变"也触发一次推送(spec §8:**业务数据没变也要推送**)。
/// 载荷 → 注入脚本的转换(`build_script`)从本载体同时取 `stamp` 与 `payload`。
#[derive(Debug, Clone, PartialEq)]
pub struct Stamped<T> {
    pub stamp: LocaleStamp,
    pub payload: T,
}

impl<T> Stamped<T> {
    pub fn new(stamp: LocaleStamp, payload: T) -> Self {
        Stamped { stamp, payload }
    }

    pub fn into_parts(self) -> (LocaleStamp, T) {
        (self.stamp, self.payload)
    }
}

/// 裸字符串按 host owner 包成无 key 的 literals:让 `impl Into<LocalizedText>`
/// 的入口(如 `Outbox::push`)能直接收 `&str`/`String`。迁移期便利,渲染时直接
/// 显示原文本;已迁到资源 key 的调用点仍应用 `keyed`/`host_text*`。
impl From<&str> for LocalizedText {
    fn from(value: &str) -> Self {
        LocalizedText::literal(Self::DEFAULT_OWNER, value)
    }
}

impl From<String> for LocalizedText {
    fn from(value: String) -> Self {
        LocalizedText::literal(Self::DEFAULT_OWNER, value)
    }
}
