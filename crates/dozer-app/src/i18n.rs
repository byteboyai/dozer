//! App 侧 i18n 资源装载(spec 2026-10-12-dozer-i18n-design §5/§7)。
//!
//! 本模块把 Host 词表在**编译期**嵌入二进制(`include_str!`),构建离线可用,
//! 不产生启动网络依赖。查表与插值在 `dozer-i18n`,这里只负责把资源按 owner
//! 组装成 `Resolver` 并暴露一个进程内单例。
//!
//! Task 02 只建立最小 Host 词表(语言设置、面板标题、通用保存失败);面板标题
//! 接入 `PanelDescriptor`/`product` 与设置页在 Task 04/05 完成。此处先提供
//! `host_resolver()` 作为后续任务的接缝,并证明资源确实来自 zh-CN/en 语言包。

use std::sync::OnceLock;

use dozer_i18n::{Resolver, resolver_from_json};

/// Host owner id(spec §6.1)。
pub const HOST_OWNER: &str = "dozer.host";

/// 编译期嵌入的 Host 语言包。
const HOST_EN: &str = include_str!("../locales/host/en.json");
const HOST_ZH_CN: &str = include_str!("../locales/host/zh-CN.json");

fn build_host_resolver() -> Resolver {
    resolver_from_json([
        (
            HOST_OWNER.to_string(),
            "en".to_string(),
            HOST_EN.to_string(),
        ),
        (
            HOST_OWNER.to_string(),
            "zh-CN".to_string(),
            HOST_ZH_CN.to_string(),
        ),
    ])
    .expect("内嵌 Host 语言包必须是合法扁平原")
}

/// 进程内 Host 词表解析器单例。
pub fn host_resolver() -> &'static Resolver {
    static RESOLVER: OnceLock<Resolver> = OnceLock::new();
    RESOLVER.get_or_init(build_host_resolver)
}

/// 便捷:按当前 locale 解析一条 Host 文案。
pub fn host_text(key: &str, fallback: &str, locale: &str) -> String {
    let t = dozer_model::LocalizedText::keyed(HOST_OWNER, key, fallback);
    host_resolver().resolve(&t, locale).text
}

/// 便捷:按当前 locale 解析一条带具名参数的 Host 文案模板。
///
/// 必须走这条(而不是先取模板再单独插值):`Resolver::resolve` 只有在**模板
/// 参数齐全**时才认语言包里的值,否则含 `{name}` 的条目会被判为不可渲染并回落。
/// 缺参时返回 fallback(经插值),绝不 panic。
pub fn host_text_args(key: &str, fallback: &str, locale: &str, args: &[(&str, &str)]) -> String {
    let mut t = dozer_model::LocalizedText::keyed(HOST_OWNER, key, fallback);
    for (name, value) in args {
        t = t.with_arg(*name, *value);
    }
    host_resolver().resolve(&t, locale).text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_bundle_parses_and_resolves_from_locale_pack() {
        let r = host_resolver();
        let title = dozer_model::LocalizedText::keyed(HOST_OWNER, "settings.title", "Settings");
        // 中文确实来自 zh-CN 资源,不是 fallback。
        assert_eq!(r.resolve(&title, "zh-CN").text, "设置");
        assert_eq!(r.resolve(&title, "en").text, "Settings");
    }

    #[test]
    fn host_panel_titles_localized() {
        assert_eq!(host_text("panel.files", "Files", "zh-CN"), "文件");
        assert_eq!(host_text("panel.files", "Files", "en"), "Files");
        assert_eq!(
            host_text("panel.code_health", "Code Health", "zh-CN"),
            "代码健康度"
        );
    }

    #[test]
    fn host_save_error_interpolates() {
        let t = dozer_model::LocalizedText::keyed(
            HOST_OWNER,
            "error.save_failed",
            "Save failed: {message}",
        )
        .with_arg("message", "disk full");
        assert_eq!(
            host_resolver().resolve(&t, "zh-CN").text,
            "保存失败：disk full"
        );
        assert_eq!(
            host_resolver().resolve(&t, "en").text,
            "Save failed: disk full"
        );
    }

    #[test]
    fn unknown_locale_falls_back_to_english() {
        assert_eq!(host_text("settings.title", "Settings", "fr"), "Settings");
    }

    #[test]
    fn host_text_args_resolves_pack_template_with_params() {
        assert_eq!(
            host_text_args(
                "time.minutes_ago",
                "{n} minutes ago",
                "zh-CN",
                &[("n", "3")]
            ),
            "3 分钟前"
        );
        assert_eq!(
            host_text_args("time.minutes_ago", "{n} minutes ago", "en", &[("n", "3")]),
            "3 minutes ago"
        );
    }
}
