//! Locale 规范化与匹配(spec 2026-10-12-dozer-i18n-design §4.1)。
//!
//! 规则:规范化大小写及 `_`/`-` 分隔,移除编码/修饰后缀;非法输入忽略;
//! `en`/`en-*` → `en`;`zh`/`zh-CN`/`zh-SG`/`zh-Hans` 及其扩展 → `zh-CN`;
//! 明确 `Hant`/`TW`/`HK`/`MO` 不自动映射简体;无匹配(含 `C`/`POSIX`)→ `en`。

/// 受支持语言(规范化后的规范形式)。
pub const SUPPORTED: &[&str] = &["en", "zh-CN"];

/// 把任意候选 locale 规范化成受支持的规范形式;无匹配返回 `None`。
///
/// 返回 `None` 表示"该候选不受支持",调用方继续查找后续候选或回落 `en`。
/// 与"非法输入"区别:两者都返回 `None`,因为规范化的结果都是"不可用"。
pub fn normalize_locale(raw: &str) -> Option<&'static str> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    // 规范化分隔符并小写。
    let lower = s.replace('_', "-").to_ascii_lowercase();
    // 移除编码/修饰后缀(`@` 之后)与多余修饰;取主干。
    let primary = lower.split('@').next().unwrap_or(&lower);
    let parts: Vec<&str> = primary.split('-').filter(|p| !p.is_empty()).collect();
    let lang = parts.first().copied()?;

    match lang {
        "en" => Some("en"),
        "zh" => {
            // 检查 script/region:显式 Hant 或 TW/HK/MO 不自动映射简体。
            let mut has_hant = false;
            let mut region_hant = false;
            let mut script_hans = false;
            for p in parts.iter().skip(1) {
                match *p {
                    "hant" | "traditional" => has_hant = true,
                    "hans" | "simplified" => script_hans = true,
                    "tw" | "hk" | "mo" => region_hant = true,
                    _ => {}
                }
            }
            if script_hans {
                return Some("zh-CN");
            }
            if has_hant || region_hant {
                // 属于中文但不映射简体:本候选不受支持,交由后续候选处理。
                return None;
            }
            Some("zh-CN")
        }
        // `C`/`POSIX` 及未知语言均不匹配。
        _ => None,
    }
}

/// 按有序候选列表解析出第一个受支持语言;全部无匹配回落 `en`(spec §4.1 规则 4)。
pub fn resolve_from_candidates<I: IntoIterator<Item = S>, S: AsRef<str>>(
    candidates: I,
) -> &'static str {
    for c in candidates {
        if let Some(loc) = normalize_locale(c.as_ref()) {
            return loc;
        }
    }
    "en"
}
