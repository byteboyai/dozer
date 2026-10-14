//! Usage 面板自带的文案解析(spec 2026-10-12-dozer-i18n-design §5)。
//!
//! 独立面板自行携带资源(本 crate `locales/{en,zh-CN}.json`,owner `dozer.usage`),
//! locale 由 Host 注入。解析复用中性纯库 `dozer-i18n`,不复制查表/插值实现,也不
//! 反向依赖 `dozer-app`。

use std::sync::Mutex;
use std::sync::OnceLock;

use dozer_i18n::{Resolver, ResourceBundle, ResourceSet};
use dozer_model::LocalizedText;

/// 本面板在词表中的 owner id。
pub const USAGE_OWNER: &str = "dozer.usage";

const EN_JSON: &str = include_str!("../locales/en.json");
const ZH_CN_JSON: &str = include_str!("../locales/zh-CN.json");

fn resolver() -> &'static Resolver {
    static RESOLVER: OnceLock<Resolver> = OnceLock::new();
    RESOLVER.get_or_init(|| {
        let mut set = ResourceSet::new(USAGE_OWNER);
        // 资源与 Rust 同源静态打入二进制;装载失败在开发期即暴露(fail fast)。
        set.insert_bundle(
            "en",
            ResourceBundle::from_json(EN_JSON).expect("usage en.json 必须合法"),
        );
        set.insert_bundle(
            "zh-CN",
            ResourceBundle::from_json(ZH_CN_JSON).expect("usage zh-CN.json 必须合法"),
        );
        let mut resolver = Resolver::new();
        resolver.insert(set);
        resolver
    })
}

/// 解析一条 `dozer.usage` 文案。`fallback` 是英文兜底(与资源缺失/不可渲染时的
/// 约定一致),与 Rust `panel_text` 及前端 `t` 的调用形态对齐。
pub fn t(key: &str, fallback: &str, locale: &str) -> String {
    let text = LocalizedText::keyed(USAGE_OWNER, key, fallback);
    resolver().resolve(&text, locale).text
}

/// 解析一条带参数的 `dozer.usage` 文案。
pub fn t_args(
    key: &str,
    fallback: &str,
    locale: &str,
    args: impl IntoIterator<Item = (&'static str, String)>,
) -> String {
    let mut text = LocalizedText::keyed(USAGE_OWNER, key, fallback);
    for (name, value) in args {
        text = text.with_arg(name, value);
    }
    resolver().resolve(&text, locale).text
}

/// 解析任意一条面板文案(供适配层的 `LocalizedText` 载荷使用)。
pub fn resolve(text: &LocalizedText, locale: &str) -> String {
    resolver().resolve(text, locale).text
}

/// 与 [`t`] 相同,但把结果**驻留**成 `&'static str`,供只收 `&'static str`
/// 的 public 组件(如 `byteui::feedback::math_curve::loading_hint` 的 label)。
/// 按 `(key, locale)` 缓存,不重复分配——不是每次调用都泄漏。
pub fn t_static(key: &str, fallback: &str, locale: &str) -> &'static str {
    use std::collections::HashMap;
    static CACHE: OnceLock<Mutex<HashMap<(String, String), &'static str>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let slot = (key.to_string(), locale.to_string());
    let mut guard = cache.lock().expect("i18n 静态文案缓存锁未中毒");
    if let Some(s) = guard.get(&slot) {
        return s;
    }
    let owned = t(key, fallback, locale);
    let leaked: &'static str = Box::leak(owned.into_boxed_str());
    guard.insert(slot, leaked);
    leaked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_known_keys_for_both_locales() {
        assert_eq!(t("title", "Usage", "en"), "Usage");
        assert_eq!(t("title", "Usage", "zh-CN"), "用量");
        assert_eq!(t("loading", "Calculating…", "zh-CN"), "统计中…");
    }

    #[test]
    fn falls_back_to_english_literal_for_unknown_key() {
        assert_eq!(t("nope.missing", "Fallback text", "zh-CN"), "Fallback text");
    }

    #[test]
    fn interpolates_named_args() {
        assert_eq!(
            t_args(
                "error.load_failed",
                "Failed to load usage: {message}",
                "en",
                [("message", "boom".to_string())],
            ),
            "Failed to load usage: boom"
        );
        assert_eq!(
            t_args(
                "error.load_failed",
                "Failed to load usage: {message}",
                "zh-CN",
                [("message", "断线".to_string())],
            ),
            "用量统计失败: 断线"
        );
    }

    #[test]
    fn en_and_zh_share_the_same_key_set() {
        let en = ResourceBundle::from_json(EN_JSON).unwrap();
        let zh = ResourceBundle::from_json(ZH_CN_JSON).unwrap();
        let errs = dozer_i18n::check_key_parity(&en, &zh);
        assert!(errs.is_empty(), "key/占位符不一致: {errs:?}");
    }
}
