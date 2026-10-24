//! Agent 预览桥接的纯逻辑(S6e-3a):从活动预览页签构造 `PreviewContext`、
//! `PreviewMode` token,以及把 `PreviewCommand` 解析到具体面板/页签。
//!
//! 这些函数原在 `dozer-app` 的 `workspace/{hook,state}.rs` 里(`pub(crate)`),
//! Digger 用不了;抽到共享库后 Dozer 改引用、Digger 直接消费。库只依赖
//! `dozer-model` / `dozer-preview-core` 的数据类型,不读时钟、不碰 IO、
//! 不依赖 `dozer_client` / `dozer_core`。

use dozer_model::{PreviewCommand, PreviewCommandTarget, PreviewContext, PreviewTabularContext};
use dozer_preview_core::resources::ViewerPanel;

use crate::state::{PreviewPane, TabKind};
use crate::{PreviewMode, TextPosition, TextRange};

/// 由 CodeMirror host 镜像出的(路径, 1-based 选区, 可见行范围, revision,
/// 时间)组装 1-based `PreviewContext`。选区 `start==end` 视为无选区(光标)。
/// 选区文本不在这里取(host 侧才持有正文);可见行范围给 Agent 判断"当前
/// 在看哪一屏"。
pub fn preview_context_from_web_state(
    path: &str,
    selection: Option<TextRange>,
    selected_text: Option<String>,
    viewport: Option<(u32, u32)>,
    revision: u64,
    now_ms: u64,
) -> PreviewContext {
    let (start_line, start_col, end_line, end_col, has_selection) = match selection {
        Some(range) => (
            range.start.line,
            range.start.column,
            range.end.line,
            range.end.column,
            range.start != range.end,
        ),
        None => (1, 1, 1, 1, false),
    };
    // 1-based 最小合法位置兜底,避免 0 行。
    let (start_line, start_col) = if start_line == 0 {
        (1, 1)
    } else {
        (start_line, start_col)
    };
    let (end_line, end_col) = if end_line == 0 {
        (1, 1)
    } else {
        (end_line, end_col)
    };
    let (visible_start_line, visible_end_line) = match viewport {
        Some((a, b)) => (Some(a), Some(b)),
        None => (None, None),
    };
    PreviewContext {
        path: path.to_string(),
        start_line,
        start_col,
        end_line,
        end_col,
        has_selection,
        updated_at_ms: now_ms,
        revision,
        selected_text,
        visible_start_line,
        visible_end_line,
        ..Default::default()
    }
}

/// `PreviewMode` → `PreviewContext.mode` 的稳定 token。
pub fn mode_token(mode: PreviewMode) -> &'static str {
    match mode {
        PreviewMode::Code => "code",
        PreviewMode::Rendered => "rendered",
        PreviewMode::Source => "source",
        PreviewMode::Tree => "tree",
        PreviewMode::Text => "text",
        PreviewMode::Streamed => "streamed",
        PreviewMode::Tabular => "tabular",
        PreviewMode::External => "external",
        PreviewMode::Unsupported => "unsupported",
    }
}

/// 活动预览页签 → `PreviewContext`。只有文件页签才有上下文:CodeMirror host
/// 用镜像的 1-based 选区/revision;表格页签填 `tabular` 块;Blank / 图片 /
/// webview 渲染页返回 `None`。
pub fn build_preview_context(pane: &PreviewPane, now_ms: u64) -> Option<PreviewContext> {
    let tab = pane.tabs().get(pane.active_idx())?;
    let TabKind::File(path) = &tab.kind else {
        return None;
    };
    let path_str = path.to_string_lossy().into_owned();
    let (mut ctx, read_only) = if tab.uses_editor_host() {
        (
            preview_context_from_web_state(
                &path_str,
                tab.web_selection,
                tab.web_selected_text.clone(),
                tab.web_viewport,
                tab.web_revision,
                now_ms,
            ),
            tab.backend_read_only(),
        )
    } else if let Some(view) = tab.tabular_view() {
        let (selected_cell, selected_range) = tabular_selection(view.selection);
        let mut c = PreviewContext {
            path: path_str.clone(),
            updated_at_ms: now_ms,
            ..Default::default()
        };
        c.tabular = Some(PreviewTabularContext {
            sheet: view.active_sheet,
            scroll_row: view.scroll_row as u32,
            scroll_col: view.scroll_col as u32,
            selected_cell,
            selected_range,
            ..Default::default()
        });
        (c, false)
    } else {
        return None;
    };
    ctx.mode = tab.current_mode().map(|m| mode_token(m).to_string());
    ctx.read_only = read_only;
    Some(ctx)
}

/// 表格选中范围 → (单格, 范围)。单点(四值相等)写 `selected_cell`,
/// 其余写 `selected_range`。
fn tabular_selection(
    selection: Option<(usize, usize, usize, usize)>,
) -> (Option<(u32, u32)>, Option<(u32, u32, u32, u32)>) {
    match selection {
        Some((r1, c1, r2, c2)) if r1 == r2 && c1 == c2 => (Some((r1 as u32, c1 as u32)), None),
        Some((r1, c1, r2, c2)) => (None, Some((r1 as u32, c1 as u32, r2 as u32, c2 as u32))),
        None => (None, None),
    }
}

/// 把 `PreviewCommand` 的 target 解析到 (面板, 页签下标)。
/// `Tab` 按显式面板名;`Path` 先找 Files、再找 Project(与 Dozer 一致)。
pub fn resolve_command_target(
    files: &PreviewPane,
    project: &PreviewPane,
    cmd: &PreviewCommand,
) -> Option<(ViewerPanel, usize)> {
    match &cmd.target {
        PreviewCommandTarget::Tab { panel, tab_id } => Some((
            if panel == "project" {
                ViewerPanel::Project
            } else {
                ViewerPanel::Files
            },
            *tab_id,
        )),
        PreviewCommandTarget::Path { path } => {
            let want = std::path::Path::new(path);
            if let Some(id) = find_file_tab(files, want) {
                Some((ViewerPanel::Files, id))
            } else {
                find_file_tab(project, want).map(|id| (ViewerPanel::Project, id))
            }
        }
    }
}

fn find_file_tab(pane: &PreviewPane, want: &std::path::Path) -> Option<usize> {
    pane.tabs()
        .iter()
        .find(|t| matches!(&t.kind, TabKind::File(p) if p == want))
        .map(|t| t.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabular::TabularView;
    use dozer_model::PreviewCommandAction;
    use std::path::PathBuf;

    fn temp_file(name: &str, body: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("agent_bridge_{}_{name}", std::process::id()));
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn preview_context_from_web_state_maps_selection_and_viewport() {
        let range = TextRange {
            start: TextPosition { line: 3, column: 2 },
            end: TextPosition { line: 5, column: 4 },
        };
        let ctx = preview_context_from_web_state(
            "/repo/a.rs",
            Some(range),
            Some("selected\ntext".into()),
            Some((10, 40)),
            9,
            1_700_000_000_000,
        );
        assert_eq!(ctx.path, "/repo/a.rs");
        assert_eq!((ctx.start_line, ctx.start_col), (3, 2));
        assert_eq!(ctx.selected_text.as_deref(), Some("selected\ntext"));
        assert_eq!((ctx.end_line, ctx.end_col), (5, 4));
        assert!(ctx.has_selection);
        assert_eq!(ctx.revision, 9);
        assert_eq!(
            (ctx.visible_start_line, ctx.visible_end_line),
            (Some(10), Some(40))
        );
        assert_eq!(ctx.updated_at_ms, 1_700_000_000_000);
    }

    #[test]
    fn preview_context_from_web_state_cursor_only_and_missing_viewport() {
        let cursor = TextRange {
            start: TextPosition { line: 7, column: 1 },
            end: TextPosition { line: 7, column: 1 },
        };
        let ctx = preview_context_from_web_state("/repo/a.rs", Some(cursor), None, None, 0, 1);
        assert_eq!((ctx.start_line, ctx.start_col), (7, 1));
        assert_eq!((ctx.end_line, ctx.end_col), (7, 1));
        assert!(!ctx.has_selection, "start==end 视为光标无选区");
        assert_eq!(ctx.visible_start_line, None);

        let ctx = preview_context_from_web_state("/repo/a.rs", None, None, None, 0, 1);
        assert_eq!((ctx.start_line, ctx.start_col), (1, 1));
    }

    #[test]
    fn mode_token_covers_all_modes() {
        for mode in [
            PreviewMode::Code,
            PreviewMode::Rendered,
            PreviewMode::Source,
            PreviewMode::Tree,
            PreviewMode::Text,
            PreviewMode::Streamed,
            PreviewMode::Tabular,
            PreviewMode::External,
            PreviewMode::Unsupported,
        ] {
            assert!(!mode_token(mode).is_empty(), "{mode:?}");
        }
    }

    #[test]
    fn build_preview_context_is_none_for_blank_and_for_non_text_tabs() {
        let blank = PreviewPane::default();
        assert!(build_preview_context(&blank, 1).is_none());

        let png = temp_file("image.png", "not an image");
        let mut image_pane = PreviewPane::default();
        image_pane.open_path(png.clone());
        assert!(
            build_preview_context(&image_pane, 1).is_none(),
            "图片页签不该有文本上下文"
        );
        std::fs::remove_file(png).ok();
    }

    #[test]
    fn build_preview_context_for_a_code_tab_carries_selection_mode_and_read_only() {
        let path = temp_file("code.rs", "fn main() {}\n");
        let mut pane = PreviewPane::default();
        pane.open_path(path.clone());
        let tab = &mut pane.tabs[pane.active];
        assert!(tab.uses_editor_host());
        tab.web_selection = Some(TextRange {
            start: TextPosition { line: 2, column: 3 },
            end: TextPosition { line: 2, column: 7 },
        });
        tab.web_selected_text = Some("main".into());
        tab.web_revision = 11;
        let read_only = tab.backend_read_only();
        let ctx = build_preview_context(&pane, 42).expect("code tab has context");
        assert_eq!(ctx.path, path.to_string_lossy().into_owned());
        assert_eq!((ctx.start_line, ctx.start_col), (2, 3));
        assert_eq!((ctx.end_line, ctx.end_col), (2, 7));
        assert_eq!(ctx.selected_text.as_deref(), Some("main"));
        assert_eq!(ctx.revision, 11);
        assert_eq!(ctx.updated_at_ms, 42);
        assert_eq!(ctx.mode.as_deref(), Some("code"));
        assert_eq!(ctx.read_only, read_only);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn build_preview_context_for_a_tabular_tab_fills_the_tabular_block() {
        let path = PathBuf::from("/tmp/agent_bridge_table.csv");
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let mut view = TabularView::new(path.clone(), vec!["S1".into()], vec![None]);
        view.scroll_row = 3;
        view.scroll_col = 2;
        view.selection = Some((4, 1, 4, 1));
        let generation = pane.load_generation(id);
        let _ = pane.finish_tabular_load(id, generation, Ok(view));
        let ctx = build_preview_context(&pane, 7).expect("tabular tab has context");
        let tabular = ctx.tabular.expect("tabular block");
        assert_eq!(tabular.sheet, 0);
        assert_eq!((tabular.scroll_row, tabular.scroll_col), (3, 2));
        assert_eq!(tabular.selected_cell, Some((4, 1)));
        assert_eq!(tabular.selected_range, None);
        assert_eq!(ctx.mode.as_deref(), Some("tabular"));
        assert!(!ctx.read_only);

        // 多格选中 → selected_range。
        let mut view = TabularView::new(path.clone(), vec!["S1".into()], vec![None]);
        view.selection = Some((1, 2, 3, 4));
        let generation = pane.load_generation(id);
        let _ = pane.finish_tabular_load(id, generation, Ok(view));
        let ctx = build_preview_context(&pane, 7).expect("tabular tab has context");
        assert_eq!(ctx.tabular.unwrap().selected_range, Some((1, 2, 3, 4)));
    }

    fn cmd(target: PreviewCommandTarget) -> PreviewCommand {
        PreviewCommand {
            request_id: "r".into(),
            project_id: 1,
            target,
            action: PreviewCommandAction::Reveal { line: 1, column: 1 },
            expected_revision: None,
        }
    }

    #[test]
    fn resolve_target_prefers_files_over_project_for_the_same_path() {
        let path = temp_file("both.md", "# x");
        let mut files = PreviewPane::default();
        let files_id = files.open_path(path.clone());
        let mut project = PreviewPane::default();
        project.open_path(path.clone());
        let got = resolve_command_target(
            &files,
            &project,
            &cmd(PreviewCommandTarget::Path {
                path: path.to_string_lossy().into_owned(),
            }),
        );
        assert_eq!(got, Some((ViewerPanel::Files, files_id)));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn resolve_target_for_an_explicit_tab_uses_the_named_panel() {
        let files = PreviewPane::default();
        let project = PreviewPane::default();
        assert_eq!(
            resolve_command_target(
                &files,
                &project,
                &cmd(PreviewCommandTarget::Tab {
                    panel: "project".into(),
                    tab_id: 5,
                })
            ),
            Some((ViewerPanel::Project, 5))
        );
        assert_eq!(
            resolve_command_target(
                &files,
                &project,
                &cmd(PreviewCommandTarget::Tab {
                    panel: "files".into(),
                    tab_id: 2,
                })
            ),
            Some((ViewerPanel::Files, 2))
        );
    }

    #[test]
    fn resolve_target_returns_none_when_the_path_is_open_nowhere() {
        let files = PreviewPane::default();
        let project = PreviewPane::default();
        assert_eq!(
            resolve_command_target(
                &files,
                &project,
                &cmd(PreviewCommandTarget::Path {
                    path: "/tmp/never-opened-anywhere.md".into(),
                })
            ),
            None
        );
    }
}
