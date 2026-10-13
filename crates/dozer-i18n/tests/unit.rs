//! dozer-i18n 单元测试:locale 规范化、资源校验、解析 fallback 顺序。

use dozer_i18n::{
    BundleError, ResolveMiss, Resolver, ResourceBundle, ResourceSet, check_key_parity, interpolate,
    normalize_locale, placeholders, resolve_from_candidates, resolver_from_json,
};
use dozer_model::LocalizedText;

// ---------- locale 规范化(spec §4.1) ----------

#[test]
fn normalize_en_family() {
    assert_eq!(normalize_locale("en"), Some("en"));
    assert_eq!(normalize_locale("EN"), Some("en"));
    assert_eq!(normalize_locale("en-US"), Some("en"));
    assert_eq!(normalize_locale("en_US"), Some("en"));
    assert_eq!(normalize_locale("en-US-POSIX"), Some("en"));
    assert_eq!(normalize_locale("en@latin"), Some("en"));
}

#[test]
fn normalize_zh_simplified_variants() {
    assert_eq!(normalize_locale("zh"), Some("zh-CN"));
    assert_eq!(normalize_locale("zh-CN"), Some("zh-CN"));
    assert_eq!(normalize_locale("zh_SG"), Some("zh-CN"));
    assert_eq!(normalize_locale("zh-Hans"), Some("zh-CN"));
    assert_eq!(normalize_locale("zh-Hans-CN"), Some("zh-CN"));
}

#[test]
fn normalize_zh_traditional_not_mapped() {
    assert_eq!(normalize_locale("zh-TW"), None);
    assert_eq!(normalize_locale("zh-HK"), None);
    assert_eq!(normalize_locale("zh-MO"), None);
    assert_eq!(normalize_locale("zh-Hant"), None);
    assert_eq!(normalize_locale("zh-Hant-CN"), None);
}

#[test]
fn normalize_unknown_and_empty() {
    assert_eq!(normalize_locale(""), None);
    assert_eq!(normalize_locale("   "), None);
    assert_eq!(normalize_locale("C"), None);
    assert_eq!(normalize_locale("POSIX"), None);
    assert_eq!(normalize_locale("fr"), None);
}

#[test]
fn resolve_candidates_falls_back_to_en() {
    assert_eq!(resolve_from_candidates(["zh-Hant-TW", "zh-Hans"]), "zh-CN");
    assert_eq!(resolve_from_candidates(["fr", "de"]), "en");
    assert_eq!(resolve_from_candidates(std::iter::empty::<&str>()), "en");
}

// ---------- 资源校验 ----------

#[test]
fn bundle_rejects_duplicate_key() {
    let json = r#"{"a":"1","b":"2","a":"3"}"#;
    assert_eq!(
        ResourceBundle::from_json(json).unwrap_err(),
        BundleError::DuplicateKey("a".into())
    );
}

#[test]
fn bundle_rejects_non_string_and_empty() {
    assert_eq!(
        ResourceBundle::from_json(r#"{"a":1}"#).unwrap_err(),
        BundleError::NonStringValue("a".into())
    );
    assert_eq!(
        ResourceBundle::from_json(r#"{"a":""}"#).unwrap_err(),
        BundleError::EmptyValue("a".into())
    );
    assert_eq!(
        ResourceBundle::from_json(r#"["a"]"#).unwrap_err(),
        BundleError::NotAnObject
    );
}

#[test]
fn bundle_rejects_bad_templates() {
    assert!(matches!(
        ResourceBundle::from_json(r#"{"a":"oops {"}"#).unwrap_err(),
        BundleError::BadTemplate { .. }
    ));
    assert!(matches!(
        ResourceBundle::from_json(r#"{"a":"oops }"}"#).unwrap_err(),
        BundleError::BadTemplate { .. }
    ));
    assert!(matches!(
        ResourceBundle::from_json(r#"{"a":"{1x}"}"#).unwrap_err(),
        BundleError::BadTemplate { .. }
    ));
}

#[test]
fn bundle_accepts_literal_braces_and_valid_placeholders() {
    assert!(ResourceBundle::from_json(r#"{"a":"{{literal}} {name}"}"#).is_ok());
}

#[test]
fn key_parity_detects_missing_and_mismatched_placeholders() {
    let en = ResourceBundle::from_json(r#"{"a":"hi {n}","b":"only en"}"#).unwrap();
    let zh = ResourceBundle::from_json(r#"{"a":"你好 {n}","b":"也在这"}"#).unwrap();
    assert!(check_key_parity(&en, &zh).is_empty());

    let zh_bad = ResourceBundle::from_json(r#"{"a":"你好","b":"在"}"#).unwrap();
    let errs = check_key_parity(&en, &zh_bad);
    assert!(!errs.is_empty());
}

#[test]
fn placeholders_order_and_dedup() {
    assert_eq!(placeholders("共 {a} 项,再 {b},又 {a}"), vec!["a", "b"]);
    assert_eq!(placeholders("{{literal}} {x}"), vec!["x"]);
}

// ---------- 解析与 fallback ----------

fn set(owner: &str, en: &str, zh: &str) -> ResourceSet {
    let mut s = ResourceSet::new(owner);
    s.insert_bundle("en", ResourceBundle::from_json(en).unwrap());
    s.insert_bundle("zh-CN", ResourceBundle::from_json(zh).unwrap());
    s
}

#[test]
fn resolve_uses_current_locale_then_default_then_fallback() {
    let mut r = Resolver::new();
    r.insert(set(
        "dozer.host",
        r#"{"greet":"Hello {n}"}"#,
        r#"{"greet":"你好 {n}"}"#,
    ));

    let t = LocalizedText::keyed("dozer.host", "greet", "Hi {n}").with_arg("n", "World");
    assert_eq!(r.resolve(&t, "zh-CN").text, "你好 World");
    assert_eq!(r.resolve(&t, "en").text, "Hello World");
    // 未知 locale:回落到 owner 默认(en)。
    assert_eq!(r.resolve(&t, "fr").text, "Hello World");
}

#[test]
fn resolve_missing_key_uses_fallback_with_interpolation() {
    let mut r = Resolver::new();
    r.insert(set("dozer.host", r#"{}"#, r#"{}"#));

    let t = LocalizedText::keyed("dozer.host", "nope", "Fallback {n}").with_arg("n", "X");
    let res = r.resolve(&t, "zh-CN");
    assert_eq!(res.text, "Fallback X");
    assert_eq!(res.diagnostic.unwrap().reason, ResolveMiss::MissingKey);
}

#[test]
fn resolve_missing_arg_falls_back_then_safe() {
    let mut r = Resolver::new();
    r.insert(set(
        "dozer.host",
        r#"{"k":"Hi {n}"}"#,
        r#"{"k":"你好 {n}"}"#,
    ));

    // key 找到但缺参数 → 尝试 fallback;fallback 也缺参数 → 内置安全文案。
    let t = LocalizedText::keyed("dozer.host", "k", "Fallback {n}");
    let res = r.resolve(&t, "zh-CN");
    assert_eq!(res.text, dozer_i18n::SAFE_FALLBACK);
    assert_eq!(res.diagnostic.unwrap().reason, ResolveMiss::Unrenderable);
}

#[test]
fn resolve_unknown_owner_uses_fallback_not_panic() {
    let r = Resolver::new();
    let t = LocalizedText::keyed("dozer.unknown", "k", "Safe");
    let res = r.resolve(&t, "zh-CN");
    assert_eq!(res.text, "Safe");
    assert_eq!(res.diagnostic.unwrap().reason, ResolveMiss::NoResources);
}

#[test]
fn resolve_literal_without_key_uses_fallback() {
    let r = Resolver::new();
    let t = LocalizedText::literal("dozer.host", "Just text");
    assert_eq!(r.resolve(&t, "zh-CN").text, "Just text");
    // 空 fallback → 安全文案,不 panic。
    let empty = LocalizedText::literal("dozer.host", "");
    assert_eq!(r.resolve(&empty, "zh-CN").text, dozer_i18n::SAFE_FALLBACK);
}

#[test]
fn resolver_from_json_groups_owners() {
    let r = resolver_from_json(vec![
        ("dozer.host".into(), "en".into(), r#"{"a":"A"}"#.into()),
        ("dozer.host".into(), "zh-CN".into(), r#"{"a":"甲"}"#.into()),
    ])
    .unwrap();
    let t = LocalizedText::keyed("dozer.host", "a", "x");
    assert_eq!(r.resolve(&t, "zh-CN").text, "甲");
    assert_eq!(r.resolve(&t, "en").text, "A");
}

#[test]
fn interpolate_malicious_html_is_not_executed_by_parser() {
    // 解析器不消费 HTML;恶意参数原样进入结果,由渲染层负责转义(spec §6.2)。
    let mut args = std::collections::BTreeMap::new();
    args.insert("v".to_string(), "<script>alert(1)</script>".to_string());
    assert_eq!(
        interpolate("<b>{v}</b>", &args).unwrap(),
        "<b><script>alert(1)</script></b>"
    );
}
