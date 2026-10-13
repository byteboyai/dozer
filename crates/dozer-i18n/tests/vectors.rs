//! Rust 侧读取共享插值向量,确认与 TS 解析器同结果(spec §6.2 / L06)。
//!
//! 向量文件在 `vectors/interpolation.json`,TS 测试读同一文件。

use std::collections::BTreeMap;

use dozer_i18n::interpolate;

#[derive(serde::Deserialize)]
struct Vectors {
    interpolate: Vec<Case>,
}

#[derive(serde::Deserialize)]
struct Case {
    name: String,
    template: String,
    args: BTreeMap<String, String>,
    expect: Option<String>,
}

#[test]
fn interpolation_vectors_match() {
    let raw = include_str!("../vectors/interpolation.json");
    let vectors: Vectors = serde_json::from_str(raw).expect("向量文件应为合法 JSON");
    assert!(!vectors.interpolate.is_empty(), "向量不应为空");

    for case in &vectors.interpolate {
        let got = interpolate(&case.template, &case.args).ok();
        assert_eq!(
            got, case.expect,
            "向量 `{}` 不符:template={:?} args={:?}",
            case.name, case.template, case.args
        );
    }
}
