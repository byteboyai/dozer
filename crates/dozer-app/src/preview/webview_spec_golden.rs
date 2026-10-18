//! S5b-2 P2 的守恒依据:`PreviewPane` 产出的 webview 期望清单(`desired_*_webviews`)
//! 在抽库前后必须**逐字相同**。清单决定 `dozer-app` 的 wry 池创建 / 销毁 / 摆放
//! 哪些 webview,抽库若悄悄改变它,界面会表现为「页面不见了 / 多了一个」。
//!
//! 黄金文件 `tests/fixtures/preview_webview_specs_golden.txt` 在抽库之前于现状上
//! 生成一次(`UPDATE_GOLDEN=1 cargo test ... webview_spec_golden`),之后**不得
//! 重新生成**;抽库后这条测试必须原样通过。

use super::*;
use crate::app::PanelKind;
use std::path::PathBuf;

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/preview_webview_specs_golden.txt"
);

fn open(pane: &mut PreviewPane, path: &str) -> usize {
    pane.open_path(PathBuf::from(path))
}

/// 一组覆盖面:空、代码、渲染型(含 reload_nonce)、JSON、表格、加载中、挂起、多页签。
fn scenarios() -> Vec<(&'static str, PreviewPane)> {
    let mut out = Vec::new();
    out.push(("blank_only", PreviewPane::default()));

    let mut p = PreviewPane::default();
    open(&mut p, "/tmp/p2golden/a.rs");
    out.push(("one_code_tab", p));

    let mut p = PreviewPane::default();
    open(&mut p, "/tmp/p2golden/readme.md");
    open(&mut p, "/tmp/p2golden/page.html");
    open(&mut p, "/tmp/p2golden/pic.png");
    out.push(("rendered_tabs", p));

    let mut p = PreviewPane::default();
    let md = open(&mut p, "/tmp/p2golden/readme.md");
    p.tabs_mut()
        .iter_mut()
        .find(|t| t.id == md)
        .unwrap()
        .reload_nonce = 2;
    out.push(("rendered_with_reload_nonce", p));

    let mut p = PreviewPane::default();
    open(&mut p, "/tmp/p2golden/data.json");
    open(&mut p, "/tmp/p2golden/log.jsonl");
    out.push(("json_and_streamed", p));

    let mut p = PreviewPane::default();
    open(&mut p, "/tmp/p2golden/sheet.csv");
    open(&mut p, "/tmp/p2golden/book.xlsx");
    out.push(("tabular_tabs", p));

    let mut p = PreviewPane::default();
    let a = open(&mut p, "/tmp/p2golden/a.rs");
    let b = open(&mut p, "/tmp/p2golden/b.py");
    let c = open(&mut p, "/tmp/p2golden/c.md");
    open(&mut p, "/tmp/p2golden/blob.bin");
    let _ = p.begin_load(a, PreviewLoadStage::Profiling);
    p.suspend_tab(b);
    let _ = p.begin_load(c, PreviewLoadStage::Profiling);
    out.push(("mixed_loading_and_suspended", p));

    let mut p = PreviewPane::default();
    for name in ["x1.rs", "x2.rs", "x3.md", "x4.json", "x5.csv", "x6.png"] {
        open(&mut p, &format!("/tmp/p2golden/{name}"));
    }
    p.select(3);
    out.push(("six_tabs_middle_active", p));
    out
}

fn render() -> String {
    let mut text = String::new();
    for (name, pane) in scenarios() {
        for (panel, label, project_id) in [
            (PanelKind::Files, "Files", 7_i64),
            (PanelKind::Project, "Project", 7_i64),
        ] {
            for locale in ["en", "zh-CN"] {
                text.push_str(&format!("### {name} / {label} / {locale}\n"));
                text.push_str(&format!(
                    "webviews: {:#?}\n",
                    pane.desired_webviews(project_id, panel, locale)
                ));
                text.push_str(&format!(
                    "editors: {:#?}\n",
                    pane.desired_editor_webviews(project_id, panel, locale)
                ));
                text.push_str(&format!(
                    "json: {:#?}\n",
                    pane.desired_json_webviews(project_id, panel, locale)
                ));
                text.push_str(&format!(
                    "tabular: {:#?}\n",
                    pane.desired_tabular_webviews(project_id, panel, locale)
                ));
            }
        }
    }
    text
}

#[test]
fn preview_webview_specs_match_the_golden() {
    let actual = render();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(GOLDEN, &actual).expect("写黄金文件");
        return;
    }
    let expected = std::fs::read_to_string(GOLDEN)
        .expect("缺少黄金文件:先在抽库之前用 UPDATE_GOLDEN=1 生成并提交");
    assert!(
        actual == expected,
        "webview 期望清单与黄金不一致(抽库不得改变它)。首个差异行:\n{}",
        actual
            .lines()
            .zip(expected.lines())
            .enumerate()
            .find(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| format!("line {}: actual={a:?} expected={b:?}", i + 1))
            .unwrap_or_else(|| format!(
                "长度不同:actual {} 行,expected {} 行",
                actual.lines().count(),
                expected.lines().count()
            ))
    );
}

/// 生成黄金依赖清单是确定的:同一进程内渲染两次必须相同(排除随机 token / 时间戳)。
#[test]
fn rendering_the_specs_twice_gives_identical_text() {
    assert_eq!(render(), render());
}
