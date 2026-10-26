//! Panel-owned group-chat text resources (`locales/{en,zh-CN}.json`), resolved
//! through the neutral `dozer-i18n` library. No dependency on `dozer-app`.

use std::sync::OnceLock;

use dozer_i18n::{Resolver, ResourceBundle, ResourceSet};
use dozer_model::LocalizedText;

pub use crate::GROUP_CHAT_OWNER;

/// Host-word owner used by the webview timeout/fallback text.
pub const HOST_OWNER: &str = "dozer.host";

const EN_JSON: &str = include_str!("../locales/panels/group_chat/en.json");
const ZH_CN_JSON: &str = include_str!("../locales/panels/group_chat/zh-CN.json");

fn resolver() -> &'static Resolver {
    static RESOLVER: OnceLock<Resolver> = OnceLock::new();
    RESOLVER.get_or_init(|| {
        let mut set = ResourceSet::new(GROUP_CHAT_OWNER);
        set.insert_bundle(
            "en",
            ResourceBundle::from_json(EN_JSON).expect("group-chat en.json 必须合法"),
        );
        set.insert_bundle(
            "zh-CN",
            ResourceBundle::from_json(ZH_CN_JSON).expect("group-chat zh-CN.json 必须合法"),
        );
        let mut resolver = Resolver::new();
        resolver.insert(set);
        resolver
    })
}

pub fn t(key: &str, fallback: &str, locale: &str) -> String {
    resolver()
        .resolve(
            &LocalizedText::keyed(GROUP_CHAT_OWNER, key, fallback),
            locale,
        )
        .text
}

pub fn t_args(key: &str, fallback: &str, locale: &str, args: &[(&str, &str)]) -> String {
    let mut text = LocalizedText::keyed(GROUP_CHAT_OWNER, key, fallback);
    for (name, value) in args {
        text = text.with_arg(*name, *value);
    }
    resolver().resolve(&text, locale).text
}

pub fn keyed(key: &str, fallback: &str) -> LocalizedText {
    LocalizedText::keyed(GROUP_CHAT_OWNER, key, fallback)
}

pub fn keyed_args(key: &str, fallback: &str, args: &[(&str, &str)]) -> LocalizedText {
    let mut text = keyed(key, fallback);
    for (name, value) in args {
        text = text.with_arg(*name, *value);
    }
    text
}

pub fn render(text: &LocalizedText, locale: &str) -> String {
    resolver().resolve(text, locale).text
}
