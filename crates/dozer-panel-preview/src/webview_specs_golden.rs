//! 预览 webview 期望清单的黄金(S5b-2 P3)。
//!
//! 这份黄金在搬迁**之前**于 `dozer-app` 现状上生成,库侧的 `desired_*_webviews`
//! 与它逐字同构。Dozer 的 `preview_webview_specs_golden.txt` 继续钉住装配侧的
//! 最终形态;这里钉住"库产出的清单没在搬动中变形"。

use super::*;
use std::path::PathBuf;

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/preview_webview_specs_golden.txt"
);

fn open(pane: &mut PreviewPane, path: &str) -> usize {
    pane.open_path(PathBuf::from(path))
}

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
            (ViewerPanel::Files, "Files", 7_i64),
            (ViewerPanel::Project, "Project", 7_i64),
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
fn webview_specs_golden_is_stable() {
    let actual = render();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(GOLDEN, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(GOLDEN).unwrap();
    assert_eq!(actual, expected);
}
