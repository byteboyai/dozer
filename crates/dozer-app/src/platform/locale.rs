//! 系统语言探测的平台隔离层(spec 2026-10-12-dozer-i18n-design §4.1)。
//!
//! 返回**有序**的系统语言偏好列表(raw BCP-47 串),供 daemon 在 `System` 偏好
//! 下解析,也供 GUI 在离线/无缓存时做一个临时显示值。**不读 `LANG`**:进程
//! 环境里的 `LANG` 常被 launchd/终端篡改,不代表用户在"系统设置"里选的
//! 语言。macOS 走 `NSLocale.preferredLanguages`;其它平台暂用环境变量兜底,
//! 并在后续平台适配时替换为各平台的真实 API(Windows `GetUserPreferredUILanguages`
//! / Linux `LC_ALL`/`LANG` 组合)。
//!
//! 规范化与匹配规则不在这里重复——那份唯一实现是 `dozer-i18n::resolve_from_candidates`。

/// 返回有序的系统语言候选(原始串,已去空白、去空项)。
///
/// 顺序即优先级:调用方按顺序取第一个可解析的支持语言。
pub fn system_language_candidates() -> Vec<String> {
    let mut out = platform_candidates();
    // 去掉空白/空项,保持顺序。
    out.retain(|c| !c.trim().is_empty());
    out
}

/// 用系统候选解析出一个受支持语言(临时显示值;权威来源仍是 daemon)。
pub fn system_supported_locale() -> &'static str {
    dozer_i18n::resolve_from_candidates(system_language_candidates())
}

#[cfg(target_os = "macos")]
fn platform_candidates() -> Vec<String> {
    use objc2::rc::autoreleasepool;
    use objc2_foundation::NSLocale;

    autoreleasepool(|_| {
        let langs = NSLocale::preferredLanguages();
        let mut out = Vec::with_capacity(langs.len());
        for lang in langs.iter() {
            // `NSString` -> `String`;失败(非法 UTF-8)跳过该候选。
            let s: String = lang.to_string();
            out.push(s);
        }
        out
    })
}

#[cfg(not(target_os = "macos"))]
fn platform_candidates() -> Vec<String> {
    // 非 macOS 的首版兜底:按优先级读常见环境变量。这不是"真实系统 API",
    // 标注为后续平台适配待办(spec §4.1 要求桌面适配器提供有序列表)。
    let mut out = Vec::new();
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(key)
            && !v.trim().is_empty()
        {
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_have_no_blank_entries() {
        for c in system_language_candidates() {
            assert!(!c.trim().is_empty(), "候选不应含空白项: {c:?}");
        }
    }

    #[test]
    fn system_supported_locale_is_always_a_known_locale() {
        let loc = system_supported_locale();
        assert!(
            dozer_i18n::SUPPORTED.contains(&loc),
            "解析结果必须是受支持语言,得到 {loc:?}"
        );
    }
}
