//! 预览核心自带的文案解析:资源随库(`locales/{en,zh-CN}.json`,owner `dozer.preview`),
//! locale 由调用方传入。解析复用中性纯库 `dozer-i18n`,不依赖 `dozer-app`(同 `dozer-panel-todo::i18n`)。
//!
//! 函数名与签名对应 `dozer-app::i18n` 里的 `panel_text` / `panel_text_args` / `panel_keyed_args` /
//! `render_localized`,所以搬进来的模块(`router` / `file_policy` / `loading` / `text_save`)只需要把
//! `crate::i18n::` 解析到这里,代码本身不用改。`dozer-app` 里没搬走的预览代码仍通过宿主词表读取
//! **同一份**静态资源(`dozer-app/src/i18n.rs` 的 `include_str!` 指向本 crate 的 `locales/`)。

use std::sync::OnceLock;

use dozer_i18n::{Resolver, ResourceBundle, ResourceSet};
use dozer_model::LocalizedText;

use crate::PREVIEW_OWNER;

const EN_JSON: &str = include_str!("../locales/en.json");
const ZH_CN_JSON: &str = include_str!("../locales/zh-CN.json");

fn resolver() -> &'static Resolver {
    static RESOLVER: OnceLock<Resolver> = OnceLock::new();
    RESOLVER.get_or_init(|| {
        let mut set = ResourceSet::new(PREVIEW_OWNER);
        // 资源与 Rust 同源静态打入二进制;装载失败在开发期即暴露(fail fast)。
        set.insert_bundle(
            "en",
            ResourceBundle::from_json(EN_JSON).expect("preview en.json 必须合法"),
        );
        set.insert_bundle(
            "zh-CN",
            ResourceBundle::from_json(ZH_CN_JSON).expect("preview zh-CN.json 必须合法"),
        );
        let mut resolver = Resolver::new();
        resolver.insert(set);
        resolver
    })
}

/// 按 locale 解析一条面板文案。`owner` 恒为 [`PREVIEW_OWNER`](保持与 `dozer-app::i18n::panel_text`
/// 相同的形参,调用点不用改);未知 owner / key 落英文 `fallback`。
pub fn panel_text(owner: &str, key: &str, fallback: &str, locale: &str) -> String {
    resolver()
        .resolve(&LocalizedText::keyed(owner, key, fallback), locale)
        .text
}

/// 同 [`panel_text`],带具名参数。
pub fn panel_text_args(
    owner: &str,
    key: &str,
    fallback: &str,
    locale: &str,
    args: &[(&str, &str)],
) -> String {
    let mut text = LocalizedText::keyed(owner, key, fallback);
    for (name, value) in args {
        text = text.with_arg(*name, *value);
    }
    resolver().resolve(&text, locale).text
}

/// 构造一条 `LocalizedText`(不解析):后台任务拿不到 locale 时先存起来,渲染端再解析。
pub fn panel_keyed_args(
    owner: &str,
    key: &str,
    fallback: &str,
    args: &[(&str, &str)],
) -> LocalizedText {
    let mut text = LocalizedText::keyed(owner, key, fallback);
    for (name, value) in args {
        text = text.with_arg(*name, *value);
    }
    text
}

/// 按 locale 渲染一条 `LocalizedText`。
pub fn render_localized(text: &LocalizedText, locale: &str) -> String {
    resolver().resolve(text, locale).text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_keys_and_unknown_owners_fall_back_to_the_english_text() {
        assert_eq!(
            panel_text(PREVIEW_OWNER, "no.such.key", "Fallback text", "zh-CN"),
            "Fallback text"
        );
        assert_eq!(panel_text("dozer.other", "x", "Fallback", "en"), "Fallback");
    }

    #[test]
    fn named_arguments_are_interpolated_and_keyed_text_renders_later() {
        let keyed = panel_keyed_args(PREVIEW_OWNER, "k", "Hello {who}", &[("who", "World")]);
        assert_eq!(render_localized(&keyed, "en"), "Hello World");
        assert_eq!(
            panel_text_args(PREVIEW_OWNER, "k", "Hello {who}", "en", &[("who", "Bob")]),
            "Hello Bob"
        );
    }
}
