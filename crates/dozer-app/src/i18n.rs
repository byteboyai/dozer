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

/// 面板 owner(spec §5):仍在 `dozer-app` 内的面板资源在 `locales/panels/<id>/`,
/// 独立 crate 的面板资源在其 crate 的 `locales/`。每项 `(owner_id, en, zh-CN)`。
///
/// Host 解析器需要聚合**所有可能经 Host 渲染的 owner**(如面板经 `outbox` 推来、
/// 由 Host Toast 呈现的 `LocalizedText`),因此独立 crate 的 `dozer.usage` 也在此
/// 登记;面板自身 view 则用其 crate 内 `i18n.rs` 的独立解析器(同名资源,不重复)。
const PANEL_OWNERS: &[(&str, &str, &str)] = &[
    (
        "dozer.todo",
        include_str!("../locales/panels/todo/en.json"),
        include_str!("../locales/panels/todo/zh-CN.json"),
    ),
    (
        "dozer.usage",
        include_str!("../../dozer-panel-usage/locales/en.json"),
        include_str!("../../dozer-panel-usage/locales/zh-CN.json"),
    ),
    (
        "dozer.code_health",
        include_str!("../locales/panels/codehealth/en.json"),
        include_str!("../locales/panels/codehealth/zh-CN.json"),
    ),
    (
        "dozer.git_log",
        include_str!("../locales/panels/git_log/en.json"),
        include_str!("../locales/panels/git_log/zh-CN.json"),
    ),
    (
        "dozer.ssh",
        include_str!("../locales/panels/ssh/en.json"),
        include_str!("../locales/panels/ssh/zh-CN.json"),
    ),
    (
        "dozer.web",
        include_str!("../locales/panels/web/en.json"),
        include_str!("../locales/panels/web/zh-CN.json"),
    ),
    (
        "dozer.conversations",
        include_str!("../locales/panels/conversations/en.json"),
        include_str!("../locales/panels/conversations/zh-CN.json"),
    ),
    (
        "dozer.agent",
        include_str!("../locales/panels/agent/en.json"),
        include_str!("../locales/panels/agent/zh-CN.json"),
    ),
    (
        "dozer.files",
        include_str!("../locales/panels/files/en.json"),
        include_str!("../locales/panels/files/zh-CN.json"),
    ),
    (
        "dozer.database",
        include_str!("../locales/panels/database/en.json"),
        include_str!("../locales/panels/database/zh-CN.json"),
    ),
    (
        "dozer.project",
        include_str!("../locales/panels/project/en.json"),
        include_str!("../locales/panels/project/zh-CN.json"),
    ),
    (
        "dozer.group_chat",
        include_str!("../locales/panels/group_chat/en.json"),
        include_str!("../locales/panels/group_chat/zh-CN.json"),
    ),
];

fn build_host_resolver() -> Resolver {
    let mut entries: Vec<(String, String, String)> = vec![
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
    ];
    for (owner, en, zh_cn) in PANEL_OWNERS {
        entries.push((owner.to_string(), "en".to_string(), en.to_string()));
        entries.push((owner.to_string(), "zh-CN".to_string(), zh_cn.to_string()));
    }
    resolver_from_json(entries).expect("内嵌语言包必须是合法扁平原")
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

/// 构造一条 Host `LocalizedText`(owner = `dozer.host`)但不解析:用于后台任务/
/// 生产端(拿不到 locale)把文案存进状态,渲染端再用 [`host_render`] 按当前
/// locale 解析,因此切换语言只需重绘(见 spec §4.3/Task 07)。
pub fn host_keyed(key: &str, fallback: &str) -> dozer_model::LocalizedText {
    dozer_model::LocalizedText::keyed(HOST_OWNER, key, fallback)
}

/// 同 [`host_keyed`],带具名参数。
pub fn host_keyed_args(
    key: &str,
    fallback: &str,
    args: &[(&str, &str)],
) -> dozer_model::LocalizedText {
    let mut t = dozer_model::LocalizedText::keyed(HOST_OWNER, key, fallback);
    for (name, value) in args {
        t = t.with_arg(*name, *value);
    }
    t
}

/// 按当前 locale 渲染一条 Host `LocalizedText`(Toast/错误呈现用)。
///
/// Toast 与错误提示把文案存成 `LocalizedText`(owner = `dozer.host`),渲染时
/// 才按 locale 解析,因此切换语言只需重绘、不改动已入队的提示内容(见 spec
/// §4.3/Task 07)。`LocalizedText` 的 `owner` 若非 host 也走 host 词表(目前
/// 所有 host 侧提示都绑 `dozer.host`),不 panic,未知 key 落 fallback。
pub fn host_render(text: &dozer_model::LocalizedText, locale: &str) -> String {
    host_resolver().resolve(text, locale).text
}

/// 按 Host 的**稳定兜底文案**(英文 fallback)渲染,忽略当前 locale:仅用于
/// 写日志/去重比较等需要语言无关稳定值的场景(见 spec「日志记录稳定事件")。
pub fn host_render_stable(text: &dozer_model::LocalizedText) -> String {
    host_resolver().resolve(text, dozer_model::Locale::EN).text
}

/// 便捷:按当前 locale 解析一条**面板**文案(owner 如 `dozer.todo`)。
pub fn panel_text(owner: &str, key: &str, fallback: &str, locale: &str) -> String {
    let t = dozer_model::LocalizedText::keyed(owner, key, fallback);
    host_resolver().resolve(&t, locale).text
}

/// 便捷:按当前 locale 解析一条带具名参数的面板文案模板。
pub fn panel_text_args(
    owner: &str,
    key: &str,
    fallback: &str,
    locale: &str,
    args: &[(&str, &str)],
) -> String {
    let mut t = dozer_model::LocalizedText::keyed(owner, key, fallback);
    for (name, value) in args {
        t = t.with_arg(*name, *value);
    }
    host_resolver().resolve(&t, locale).text
}

/// 构造一条**面板** `LocalizedText`(owner 如 `dozer.git_log`)但不解析:用于
/// 后台任务(拿不到 locale)把文案存进状态,渲染端再用 [`panel_text`] 或
/// [`render_localized`] 按当前 locale 解析。
pub fn panel_keyed(owner: &str, key: &str, fallback: &str) -> dozer_model::LocalizedText {
    dozer_model::LocalizedText::keyed(owner, key, fallback)
}

/// 构造一条**面板** `LocalizedText`(owner 如 `dozer.git_log`)但不解析:用于
/// 后台任务(拿不到 locale)把文案存进状态,渲染端再用 [`panel_text`] 或
/// [`render_localized`] 按当前 locale 解析。带具名参数版本。
pub fn panel_keyed_args(
    owner: &str,
    key: &str,
    fallback: &str,
    args: &[(&str, &str)],
) -> dozer_model::LocalizedText {
    let mut t = dozer_model::LocalizedText::keyed(owner, key, fallback);
    for (name, value) in args {
        t = t.with_arg(*name, *value);
    }
    t
}

/// 按当前 locale 渲染任意 owner 的 `LocalizedText`(面板状态里存的 keyed 文案)。
pub fn render_localized(text: &dozer_model::LocalizedText, locale: &str) -> String {
    host_resolver().resolve(text, locale).text
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

    #[test]
    fn host_resolver_renders_independent_crate_panel_owner() {
        // 独立 crate(dozer.usage)的风险:它经 `outbox` 推给 Host Toast 的
        // `LocalizedText` 由 Host 解析器渲染,故其 owner 必须登记在此,
        // 否则中文界面会回落到英文 fallback。
        let t = dozer_model::LocalizedText::keyed(
            "dozer.usage",
            "error.load_failed",
            "Failed to load usage: {message}",
        )
        .with_arg("message", "断线");
        assert_eq!(
            host_resolver().resolve(&t, "zh-CN").text,
            "用量统计失败: 断线"
        );
        assert_eq!(
            host_resolver().resolve(&t, "en").text,
            "Failed to load usage: 断线"
        );
    }
}
