//! 会话面板自带的文案解析:资源随库(`locales/{en,zh-CN}.json`,owner `dozer.conversations`),locale 由宿主注入。
//! 解析复用中性纯库 `dozer-i18n`,不依赖 `dozer-app`。

use std::sync::OnceLock;

use dozer_i18n::{Resolver, ResourceBundle, ResourceSet};
use dozer_model::LocalizedText;

pub use crate::CONVERSATIONS_OWNER;

const EN_JSON: &str = include_str!("../locales/en.json");
const ZH_CN_JSON: &str = include_str!("../locales/zh-CN.json");

fn resolver() -> &'static Resolver {
    static RESOLVER: OnceLock<Resolver> = OnceLock::new();
    RESOLVER.get_or_init(|| {
        let mut set = ResourceSet::new(CONVERSATIONS_OWNER);
        // 资源与 Rust 同源静态打入二进制;装载失败在开发期即暴露(fail fast)。
        set.insert_bundle(
            "en",
            ResourceBundle::from_json(EN_JSON).expect("conversations en.json 必须合法"),
        );
        set.insert_bundle(
            "zh-CN",
            ResourceBundle::from_json(ZH_CN_JSON).expect("conversations zh-CN.json 必须合法"),
        );
        let mut resolver = Resolver::new();
        resolver.insert(set);
        resolver
    })
}

/// 解析一条 `dozer.conversations` 文案。`fallback` 是英文兜底。
pub fn t(key: &str, fallback: &str, locale: &str) -> String {
    resolver()
        .resolve(
            &LocalizedText::keyed(CONVERSATIONS_OWNER, key, fallback),
            locale,
        )
        .text
}

/// 同 [`t`],带具名参数。
pub fn t_args(key: &str, fallback: &str, locale: &str, args: &[(&str, &str)]) -> String {
    let mut text = LocalizedText::keyed(CONVERSATIONS_OWNER, key, fallback);
    for (name, value) in args {
        text = text.with_arg(*name, *value);
    }
    resolver().resolve(&text, locale).text
}

/// Deferred panel-owned text, rendered by the host's registered panel resources.
pub fn keyed(key: &str, fallback: &str) -> LocalizedText {
    LocalizedText::keyed(CONVERSATIONS_OWNER, key, fallback)
}
pub fn keyed_args(key: &str, fallback: &str, args: &[(&str, &str)]) -> LocalizedText {
    let mut t = keyed(key, fallback);
    for (k, v) in args {
        t = t.with_arg(*k, *v);
    }
    t
}
pub fn render(text: &LocalizedText, locale: &str) -> String {
    resolver().resolve(text, locale).text
}
