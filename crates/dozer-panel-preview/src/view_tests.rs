//! 视图测试:用一个记录型的假 `PanelDisplay` 搭 Element,断言「画了哪些部件 / 发了哪些消息」。

use std::cell::RefCell;
use std::path::PathBuf;

use iced_widget::core::{Border, Color, Length, Padding};

use crate::display::{
    ActionButtonStyle, Element, OverflowMenuArgs, PaneStyle, PanelDisplay, PreviewHover, TabArgs,
};
use crate::view::{self, ViewCtx};
use crate::*;

type M = String;

#[derive(Default)]
struct Host {
    wrapped: RefCell<Vec<String>>,
    overflow_entries: RefCell<Vec<(usize, bool, bool)>>,
    render_mode_buttons: RefCell<usize>,
    tabular_buttons: RefCell<usize>,
    json_buttons: RefCell<usize>,
    tabs: RefCell<Vec<(String, bool)>>,
}

fn blank<'a>() -> Element<'a, M> {
    iced_widget::Space::new().into()
}

impl PanelDisplay<M> for Host {
    fn wrap(&self, panel: ViewerPanel, message: Message) -> M {
        let s = format!("{panel:?}:{message:?}");
        self.wrapped.borrow_mut().push(s.clone());
        s
    }
    fn locale(&self) -> &str {
        "en"
    }
    fn product_name(&self) -> &str {
        "TestProduct"
    }
    fn render_text(&self, text: &dozer_model::LocalizedText) -> String {
        format!("{text:?}")
    }
    fn pane_style(&self) -> PaneStyle {
        PaneStyle {
            padding: Padding::new(4.0),
            gap: 4.0,
            background: None,
        }
    }
    fn hover_progress(&self, _: ViewerPanel, _: PreviewHover) -> f32 {
        0.0
    }
    fn hover_tooltip_ready(&self, _: ViewerPanel, _: PreviewHover) -> bool {
        false
    }
    fn hover_message(&self, panel: ViewerPanel, hover: PreviewHover, hovered: bool) -> M {
        format!("hover:{panel:?}:{hover:?}:{hovered}")
    }
    fn tab_bar_avail_px(&self, _: ViewerPanel) -> f32 {
        400.0
    }
    fn tab_display_width(&self, title: &str) -> f32 {
        title.len() as f32 * 8.0 + 30.0
    }
    fn tab_divider<'a>(&self) -> Element<'a, M> {
        blank()
    }
    fn panel_tab<'a>(&self, args: TabArgs<'a, M>) -> Element<'a, M> {
        self.tabs.borrow_mut().push((args.title, args.active));
        blank()
    }
    fn tab_gestures<'a>(
        &self,
        _: ViewerPanel,
        _: usize,
        tab: Element<'a, M>,
        _: Option<PathBuf>,
    ) -> Element<'a, M> {
        tab
    }
    fn overflow_button<'a>(
        &self,
        tab_count: usize,
        _: f32,
        _: M,
        _: Box<dyn Fn(bool) -> M + 'a>,
    ) -> Option<Element<'a, M>> {
        (tab_count > 0).then(blank)
    }
    fn render_mode_button<'a>(
        &self,
        _: bool,
        _: f32,
        _: M,
        _: Box<dyn Fn(bool) -> M + 'a>,
    ) -> Element<'a, M> {
        *self.render_mode_buttons.borrow_mut() += 1;
        blank()
    }
    fn tabular_mode_button<'a>(
        &self,
        _: bool,
        _: f32,
        _: M,
        _: Box<dyn Fn(bool) -> M + 'a>,
    ) -> Element<'a, M> {
        *self.tabular_buttons.borrow_mut() += 1;
        blank()
    }
    fn json_mode_button<'a>(
        &self,
        _: bool,
        _: f32,
        _: M,
        _: Box<dyn Fn(bool) -> M + 'a>,
    ) -> Element<'a, M> {
        *self.json_buttons.borrow_mut() += 1;
        blank()
    }
    fn collapse_button<'a>(&self, _: ViewerPanel) -> Element<'a, M> {
        blank()
    }
    fn overflow_menu<'a>(&self, args: OverflowMenuArgs<'a, M>) -> Element<'a, M> {
        *self.overflow_entries.borrow_mut() = args
            .entries
            .iter()
            .map(|e| (e.index, e.active, e.closable))
            .collect();
        blank()
    }
    fn action_button_style(&self, _: Color) -> ActionButtonStyle {
        Box::new(|_, _| iced_widget::button::Style::default())
    }
    fn find_font_size(&self) -> f32 {
        14.0
    }
    fn line_height_factor(&self) -> f32 {
        1.2
    }
}

fn build_pane(host: &Host, state: &PreviewPanelState, width: Length) {
    let ctx = ViewCtx { project: None };
    let _ = view::pane(
        &state.pane,
        state.tab_first,
        state.error.as_ref(),
        ViewerPanel::Files,
        host,
        &ctx,
        width,
        Border::default(),
    );
}

fn state_with(n: usize, dir: &std::path::Path) -> PreviewPanelState {
    let mut st = PreviewPanelState::default();
    for i in 0..n {
        let p = dir.join(format!("f{i}.md"));
        std::fs::write(&p, "x").unwrap();
        st.pane.open_path(p);
    }
    st
}

#[test]
fn pane_builds_for_zero_one_and_fifty_tabs_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    for n in [0, 1, 50] {
        let host = Host::default();
        let st = state_with(n, dir.path());
        build_pane(&host, &st, Length::Fill);
        // 占位 Blank + n 个文件;窗口只画装得下的那部分,但至少画出激活项所在的窗口。
        assert!(!host.tabs.borrow().is_empty());
    }
}

#[test]
fn degenerate_states_build_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::default();
    let mut st = state_with(3, dir.path());
    st.tab_first = 9_999; // 越界
    build_pane(&host, &st, Length::Fixed(0.0)); // 窄到 0
    let mut st = state_with(1, dir.path());
    st.pane.tabs_mut()[1].title = String::new(); // 空标题
    build_pane(&host, &st, Length::Fill);
    st.pane.tabs_mut()[1].title = "长".repeat(10_000); // 超长标题
    build_pane(&host, &st, Length::Fill);
    let mut st = state_with(2, dir.path());
    st.error = Some(crate::i18n::panel_keyed(
        PANEL_OWNER,
        "preview.file_unreadable",
        "x {path}",
    ));
    build_pane(&host, &st, Length::Fill);
}

#[test]
fn tab_titles_mark_the_active_tab_and_dirty_tabs_get_a_star() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::default();
    let mut st = state_with(2, dir.path());
    st.pane.tabs_mut()[1].dirty = true;
    build_pane(&host, &st, Length::Fill);
    let tabs = host.tabs.borrow();
    assert!(tabs.iter().any(|(t, _)| t == "f0.md *"));
    assert_eq!(
        tabs.iter().filter(|(_, a)| *a).count(),
        1,
        "恰好一个激活页签"
    );
}

#[test]
fn close_buttons_send_close_for_the_right_index_and_blank_alone_only_selects() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::default();
    let st = state_with(2, dir.path());
    build_pane(&host, &st, Length::Fill);
    let log = host.wrapped.borrow();
    assert!(log.iter().any(|m| m == "Files:CloseTab(1)"));
    assert!(log.iter().any(|m| m == "Files:CloseTab(2)"));
    drop(log);
    let host = Host::default();
    build_pane(&host, &PreviewPanelState::default(), Length::Fill);
    assert!(
        !host.wrapped.borrow().iter().any(|m| m.contains("CloseTab")),
        "单独的 Blank 占位页签 × 只选中"
    );
}

#[test]
fn render_mode_button_only_for_eligible_tabs_and_sends_toggle() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::default();
    let mut st = state_with(1, dir.path());
    build_pane(&host, &st, Length::Fill);
    assert_eq!(
        *host.render_mode_buttons.borrow(),
        1,
        "Markdown 可切代码/预览"
    );
    assert!(
        host.wrapped
            .borrow()
            .iter()
            .any(|m| m == "Files:ToggleRenderMode(1)")
    );
    // 激活 Blank 占位:没有按钮。
    st.pane.select(0);
    let host = Host::default();
    build_pane(&host, &st, Length::Fill);
    assert_eq!(*host.render_mode_buttons.borrow(), 0);
    assert_eq!(
        *host.tabular_buttons.borrow() + *host.json_buttons.borrow(),
        0
    );
}

#[test]
fn find_bar_shows_replace_row_only_when_replace_is_open_and_sends_the_documented_messages() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = state_with(1, dir.path());
    st.pane.open_find_on_active(false, Some(1));
    // 没有 find 会话时是空占位。
    let host = Host::default();
    let empty_pane = PreviewPane::default();
    let _ = view::find_bar(&host, ViewerPanel::Project, &empty_pane, true);
    assert!(host.wrapped.borrow().is_empty());
    // 原生档(show_replace=true)但替换行折叠:不发替换消息。
    let host = Host::default();
    let mut native = PreviewPane {
        find: Some(FindState {
            tab_id: 1,
            query: "q".into(),
            current: 0,
            count: 2,
            case_sensitive: false,
            replacement: String::new(),
            replace_open: false,
            query_focused: false,
            is_webview: false,
            pending_webview_exec: None,
            webview_pool_id: None,
        }),
        ..Default::default()
    };
    let _ = view::find_bar(&host, ViewerPanel::Files, &native, true);
    let log = host.wrapped.borrow().join(",");
    assert!(
        log.contains("FindCase(true)")
            && log.contains("FindGo(true)")
            && log.contains("FindGo(false)")
    );
    assert!(log.contains("FindReplaceToggle"));
    assert!(!log.contains("FindReplaceCurrent") && !log.contains("FindReplaceAll"));
    // 展开后出现替换行。
    native.find.as_mut().unwrap().replace_open = true;
    let host = Host::default();
    let _ = view::find_bar(&host, ViewerPanel::Files, &native, true);
    let log = host.wrapped.borrow().join(",");
    assert!(log.contains("FindReplaceCurrent") && log.contains("FindReplaceAll"));
    // webview 档(show_replace=false):永不出现替换相关部件。
    let host = Host::default();
    let _ = view::find_bar(&host, ViewerPanel::Files, &native, false);
    let log = host.wrapped.borrow().join(",");
    assert!(!log.contains("FindReplace"));
}

#[test]
fn large_file_search_bar_builds_in_every_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = state_with(1, dir.path());
    let id = st.pane.tabs()[1].id;
    st.pane.open_large_file_search(id);
    let host = Host::default();
    let _ = view::large_file_search_bar(&host, ViewerPanel::Files, &st.pane, id); // 空闲
    let (generation, _) = st.pane.start_large_file_search(id, "q".into()).unwrap();
    let _ = view::large_file_search_bar(&host, ViewerPanel::Files, &st.pane, id); // 进行中
    st.pane.set_large_file_search_progress(
        id,
        generation,
        PreviewLoadProgress {
            completed: 5,
            total: Some(10),
        },
    );
    let _ = view::large_file_search_bar(&host, ViewerPanel::Files, &st.pane, id); // 进度
    st.pane.set_large_file_search_results(
        id,
        generation,
        Ok(SearchOutcome {
            hits: vec![SearchHit {
                line: 3,
                column: 1,
                text: "q".into(),
            }],
            total_matches: 1,
            truncated: false,
            lines_scanned: 10,
        }),
    );
    let _ = view::large_file_search_bar(&host, ViewerPanel::Files, &st.pane, id); // 有结果
    let (g2, _) = st.pane.start_large_file_search(id, "none".into()).unwrap();
    st.pane.set_large_file_search_results(
        id,
        g2,
        Ok(SearchOutcome {
            hits: vec![],
            total_matches: 0,
            truncated: false,
            lines_scanned: 10,
        }),
    );
    let _ = view::large_file_search_bar(&host, ViewerPanel::Files, &st.pane, id); // 无结果
    let g3 = st.pane.start_large_file_search(id, "err".into()).unwrap().0;
    st.pane
        .set_large_file_search_results(id, g3, Err("boom".into()));
    let _ = view::large_file_search_bar(&host, ViewerPanel::Files, &st.pane, id); // 失败
    let log = host.wrapped.borrow().join(",");
    assert!(log.contains("LargeFileSearchGo(true)") && log.contains("LargeFileSearchClose"));
}

#[test]
fn overflow_menu_has_one_entry_per_tab_and_marks_the_active_one() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::default();
    let st = state_with(3, dir.path());
    assert!(view::overflow_menu(&host, ViewerPanel::Files, &st.pane, None).is_none());
    assert!(view::overflow_menu(&host, ViewerPanel::Files, &st.pane, Some((1.0, 2.0))).is_some());
    let entries = host.overflow_entries.borrow();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries.iter().filter(|e| e.1).count(), 1);
    assert!(entries[0].1 == (st.pane.active_idx() == 0));
    assert!(!entries[0].2, "Blank 占位不可关闭");
    assert!(entries[1..].iter().all(|e| e.2));
}

#[test]
fn fallback_page_offers_the_system_open_action_for_every_degraded_kind() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = state_with(1, dir.path());
    let host = Host::default();
    let tab = &mut st.pane.tabs_mut()[1];
    tab.backend = Some(PreviewBackend::Unsupported(UnsupportedBackend {
        reason: RouteReason::BackendDisabled,
    }));
    let tab = &st.pane.tabs()[1];
    assert!(view::fallback_page(&host, ViewerPanel::Files, tab).is_some());
    assert!(
        host.wrapped
            .borrow()
            .iter()
            .any(|m| m.contains("OpenExternal"))
    );
    // External 同样。
    let mut st = state_with(1, dir.path());
    st.pane.tabs_mut()[1].backend = Some(PreviewBackend::External(ExternalBackend {
        reason: RouteReason::ArchiveFallback,
    }));
    let host = Host::default();
    assert!(view::fallback_page(&host, ViewerPanel::Project, &st.pane.tabs()[1]).is_some());
    assert!(
        host.wrapped
            .borrow()
            .iter()
            .any(|m| m == "Project:OpenExternal(2)" || m.starts_with("Project:OpenExternal("))
    );
    // 正常 Ready 页签没有降级页。
    let st = state_with(1, dir.path());
    let host = Host::default();
    assert!(view::fallback_page(&host, ViewerPanel::Files, &st.pane.tabs()[0]).is_none());
}

#[test]
fn fallback_copy_uses_the_product_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = state_with(1, dir.path());
    st.pane.tabs_mut()[1].backend = Some(PreviewBackend::Unsupported(UnsupportedBackend {
        reason: RouteReason::BackendDisabled,
    }));
    let m = view_model::fallback_model(&st.pane.tabs()[1], "en", "TestProduct").unwrap();
    assert!(m.headline.contains("TestProduct") && !m.headline.contains("Dozer"));
    assert!(m.security_note.contains("TestProduct"));
    let zh = view_model::fallback_model(&st.pane.tabs()[1], "zh-CN", "Digger").unwrap();
    assert!(zh.headline.contains("Digger"));
}

#[test]
fn conflict_bar_actions_send_keep_and_reload() {
    let host = Host::default();
    let _ = view::conflict_bar(&host, ViewerPanel::Project, 7, false);
    let log = host.wrapped.borrow().join(",");
    assert!(log.contains("Project:ConflictKeep(7)") && log.contains("Project:ConflictReload(7)"));
}

#[test]
fn loading_view_present_for_every_active_stage_absent_at_idle() {
    for stage in PreviewLoadStage::active_stages() {
        let state = PreviewLoadState::starting(1, stage);
        assert!(
            view::loading_view::<M>(&state, None, "en").is_some(),
            "{stage:?}"
        );
    }
    assert!(view::loading_view::<M>(&PreviewLoadState::default(), None, "en").is_none());
}

#[test]
fn loading_view_marks_first_frame_for_matching_generation() {
    let state = PreviewLoadState::starting(5, PreviewLoadStage::Reading);
    let observe = LoadObservation::new(5);
    assert!(view::loading_view::<M>(&state, Some(&observe), "en").is_some());
    assert!(observe.first_frame_offset_ms().is_some());
    let stale = LoadObservation::new(9);
    let _ = view::loading_view::<M>(&state, Some(&stale), "en");
    assert!(stale.first_frame_offset_ms().is_none());
}
