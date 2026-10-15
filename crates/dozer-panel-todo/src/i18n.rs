//! Todo 面板自带的文案解析:资源随库(`locales/{en,zh-CN}.json`,owner `dozer.todo`),locale 由宿主注入。
//! 解析复用中性纯库 `dozer-i18n`,不依赖 `dozer-app`。

use std::sync::OnceLock;

use dozer_i18n::{Resolver, ResourceBundle, ResourceSet};
use dozer_model::LocalizedText;

pub use crate::TODO_OWNER;

const EN_JSON: &str = include_str!("../locales/en.json");
const ZH_CN_JSON: &str = include_str!("../locales/zh-CN.json");

fn resolver() -> &'static Resolver {
    static RESOLVER: OnceLock<Resolver> = OnceLock::new();
    RESOLVER.get_or_init(|| {
        let mut set = ResourceSet::new(TODO_OWNER);
        // 资源与 Rust 同源静态打入二进制;装载失败在开发期即暴露(fail fast)。
        set.insert_bundle("en", ResourceBundle::from_json(EN_JSON).expect("todo en.json 必须合法"));
        set.insert_bundle("zh-CN", ResourceBundle::from_json(ZH_CN_JSON).expect("todo zh-CN.json 必须合法"));
        let mut resolver = Resolver::new();
        resolver.insert(set);
        resolver
    })
}

/// 解析一条 `dozer.todo` 文案。`fallback` 是英文兜底。
pub fn t(key: &str, fallback: &str, locale: &str) -> String {
    resolver().resolve(&LocalizedText::keyed(TODO_OWNER, key, fallback), locale).text
}

/// 同 [`t`],带具名参数。
pub fn t_args(key: &str, fallback: &str, locale: &str, args: &[(&str, &str)]) -> String {
    let mut text = LocalizedText::keyed(TODO_OWNER, key, fallback);
    for (name, value) in args {
        text = text.with_arg(*name, *value);
    }
    resolver().resolve(&text, locale).text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 随库的资源与 Dozer 宿主词表里的 `dozer.todo` 是同一份文件:这里直接读库里的中英文各一条。
    #[test]
    fn the_bundled_catalogs_resolve_in_both_languages() {
        assert_eq!(t("title", "Todo", "en"), "Todo");
        assert!(!t("title", "Todo", "zh-CN").is_empty());
        assert_eq!(t("no.such.key", "Fallback text", "zh-CN"), "Fallback text", "a missing key falls back to the English text");
    }

    #[test]
    fn named_arguments_are_interpolated() {
        let s = t_args("detail.assigned_to", "Assigned to: {agent}", "en", &[("agent", "Claude")]);
        assert_eq!(s, "Assigned to: Claude");
    }
}
