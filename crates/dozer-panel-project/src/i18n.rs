//! Project 面板自带的文案解析:资源随库(`locales/{en,zh-CN}.json`,owner `dozer.project`),locale 由宿主注入。
//! 解析复用中性纯库 `dozer-i18n`,不依赖 `dozer-app`。

use std::sync::OnceLock;

use dozer_i18n::{Resolver, ResourceBundle, ResourceSet};
use dozer_model::LocalizedText;

pub use crate::PROJECT_OWNER;

const EN_JSON: &str = include_str!("../locales/en.json");
const ZH_CN_JSON: &str = include_str!("../locales/zh-CN.json");

fn resolver() -> &'static Resolver {
    static RESOLVER: OnceLock<Resolver> = OnceLock::new();
    RESOLVER.get_or_init(|| {
        let mut set = ResourceSet::new(PROJECT_OWNER);
        // 资源与 Rust 同源静态打入二进制;装载失败在开发期即暴露(fail fast)。
        set.insert_bundle(
            "en",
            ResourceBundle::from_json(EN_JSON).expect("project en.json 必须合法"),
        );
        set.insert_bundle(
            "zh-CN",
            ResourceBundle::from_json(ZH_CN_JSON).expect("project zh-CN.json 必须合法"),
        );
        let mut resolver = Resolver::new();
        resolver.insert(set);
        resolver
    })
}

/// 解析一条 `dozer.project` 文案。`fallback` 是英文兜底。
pub fn t(key: &str, fallback: &str, locale: &str) -> String {
    resolver()
        .resolve(&LocalizedText::keyed(PROJECT_OWNER, key, fallback), locale)
        .text
}

/// 同 [`t`],带具名参数。
pub fn t_args(key: &str, fallback: &str, locale: &str, args: &[(&str, &str)]) -> String {
    let mut text = LocalizedText::keyed(PROJECT_OWNER, key, fallback);
    for (name, value) in args {
        text = text.with_arg(*name, *value);
    }
    resolver().resolve(&text, locale).text
}

/// 按当前 locale 解析一条**任意 owner** 的 `LocalizedText`(状态里存的 keyed 文案)。
/// 本面板自有的文案走 [`t`];其他 owner(共享记忆等)在本地解析不到时回落英文兜底,
/// 与产品侧解析器的行为一致。
pub fn t_from(text: &LocalizedText, locale: &str) -> String {
    resolver().resolve(text, locale).text
}

/// 构造一条本面板 `LocalizedText`(后台任务 / 生产端拿不到 locale 时用),渲染端按当前 locale 解析。
pub fn keyed(key: &str, fallback: &str) -> LocalizedText {
    LocalizedText::keyed(PROJECT_OWNER, key, fallback)
}

/// 同 [`keyed`],带具名参数。
pub fn keyed_args(key: &str, fallback: &str, args: &[(&str, &str)]) -> LocalizedText {
    let mut text = LocalizedText::keyed(PROJECT_OWNER, key, fallback);
    for (name, value) in args {
        text = text.with_arg(*name, *value);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_catalogs_resolve_in_both_languages() {
        assert_eq!(t("title", "Project", "en"), "Project");
        assert!(!t("title", "Project", "zh-CN").is_empty());
        assert_eq!(
            t("no.such.key", "Fallback text", "zh-CN"),
            "Fallback text",
            "a missing key falls back to the English text"
        );
    }

    #[test]
    fn named_arguments_are_interpolated() {
        let s = t_args(
            "error.rename_failed",
            "Rename failed: {error}",
            "en",
            &[("error", "boom")],
        );
        assert_eq!(s, "Rename failed: boom");
    }

    #[test]
    fn keyed_texts_carry_the_owner_and_args() {
        assert_eq!(keyed("title", "Project").owner, PROJECT_OWNER);
        let text = keyed_args(
            "scaffold.result.docs_added",
            "Added {n} project docs",
            &[("n", "2")],
        );
        assert_eq!(text.owner, PROJECT_OWNER);
    }
}

/// 语言无关的稳定渲染值(英文 + 已插值参数、折叠空白):测试与去重比较用,
/// 与 Dozer `i18n::host_render_stable` + `toast::normalize` 的口径一致。
#[cfg(test)]
pub fn stable(text: &LocalizedText) -> String {
    resolver()
        .resolve(text, "en")
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// 按中文解析一条本面板文案(测试用)。
#[cfg(test)]
pub fn resolve_zh(text: &LocalizedText) -> String {
    resolver().resolve(text, "zh-CN").text
}
