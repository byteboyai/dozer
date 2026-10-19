//! Files 面板自带的文案解析:资源随库(`locales/{en,zh-CN}.json`,owner `dozer.files`),locale 由宿主注入。
//! 解析复用中性纯库 `dozer-i18n`,不依赖 `dozer-app`。

use std::sync::OnceLock;

use dozer_i18n::{Resolver, ResourceBundle, ResourceSet};
use dozer_model::LocalizedText;

pub use crate::FILES_OWNER;

/// Host 归属的文案 owner(宿主词表里的键,如 `webview.load_timeout`);值与 Dozer `i18n::HOST_OWNER` 相同。
pub const HOST_OWNER: &str = "dozer.host";

const HOST_EN_JSON: &str = include_str!("../../../crates/dozer-app/locales/host/en.json");
const HOST_ZH_CN_JSON: &str = include_str!("../../../crates/dozer-app/locales/host/zh-CN.json");

const EN_JSON: &str = include_str!("../locales/en.json");
const ZH_CN_JSON: &str = include_str!("../locales/zh-CN.json");

fn resolver() -> &'static Resolver {
    static RESOLVER: OnceLock<Resolver> = OnceLock::new();
    RESOLVER.get_or_init(|| {
        let mut host = ResourceSet::new(HOST_OWNER);
        host.insert_bundle(
            "en",
            ResourceBundle::from_json(HOST_EN_JSON).expect("host en.json 必须合法"),
        );
        host.insert_bundle(
            "zh-CN",
            ResourceBundle::from_json(HOST_ZH_CN_JSON).expect("host zh-CN.json 必须合法"),
        );
        let mut set = ResourceSet::new(FILES_OWNER);
        // 资源与 Rust 同源静态打入二进制;装载失败在开发期即暴露(fail fast)。
        set.insert_bundle(
            "en",
            ResourceBundle::from_json(EN_JSON).expect("files en.json 必须合法"),
        );
        set.insert_bundle(
            "zh-CN",
            ResourceBundle::from_json(ZH_CN_JSON).expect("files zh-CN.json 必须合法"),
        );
        let mut resolver = Resolver::new();
        resolver.insert(host);
        resolver.insert(set);
        resolver
    })
}

/// 解析一条 `dozer.files` 文案。`fallback` 是英文兜底。
pub fn t(key: &str, fallback: &str, locale: &str) -> String {
    resolver()
        .resolve(&LocalizedText::keyed(FILES_OWNER, key, fallback), locale)
        .text
}

/// 同 [`t`],带具名参数。
pub fn t_args(key: &str, fallback: &str, locale: &str, args: &[(&str, &str)]) -> String {
    let mut text = LocalizedText::keyed(FILES_OWNER, key, fallback);
    for (name, value) in args {
        text = text.with_arg(*name, *value);
    }
    resolver().resolve(&text, locale).text
}

/// 构造一条本面板 `LocalizedText` 但不解析(后台任务用,渲染端再解析)。
pub fn keyed(key: &str, fallback: &str) -> LocalizedText {
    LocalizedText::keyed(FILES_OWNER, key, fallback)
}

/// 同 [`keyed`],带具名参数。
pub fn keyed_args(key: &str, fallback: &str, args: &[(&str, &str)]) -> LocalizedText {
    let mut text = LocalizedText::keyed(FILES_OWNER, key, fallback);
    for (name, value) in args {
        text = text.with_arg(*name, *value);
    }
    text
}

/// 按当前 locale 解析一条**任意 owner** 的 `LocalizedText`。本面板与 host 文案都能解析;
/// 本地解析不到的资源回落英文兜底,与产品侧解析器的行为一致。
pub fn t_from(text: &LocalizedText, locale: &str) -> String {
    resolver().resolve(text, locale).text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_catalogs_resolve_in_both_languages() {
        let en = t("title", "Files", "en");
        assert!(!en.is_empty());
        assert!(!t("title", "Files", "zh-CN").is_empty());
        assert_eq!(
            t("no.such.key", "Fallback text", "zh-CN"),
            "Fallback text",
            "a missing key falls back to the English text"
        );
    }
}
