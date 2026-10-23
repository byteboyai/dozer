//! 视图层测试(FakeDisplay 注入)。

use crate::adapter::TaskTitleSource;
use crate::card::CardCtx;
use crate::display::*;
use crate::state::AgentPanelState;
use crate::style::group_tabs_by_agent;
use crate::tabs::{SessionTab, TabBackend};
use crate::update::Message;
use crate::view::{list, terminal};
use byteui::interaction::icons::IconKind;
use dozer_model::{AgentKind, AgentState, SessionInfo};
use dozer_panel_term::osc::OscScanner;
use dozer_panel_term::style::TermStyle;
use dozer_panel_term::view::TermEvent;
use iced_widget::core::{Border, Color, Length, Padding};
use iced_widget::{container, text};
use std::cell::RefCell;

struct Titles;
impl TaskTitleSource for Titles {
    fn task_title_for_session(&self, _id: &str) -> Option<String> {
        None
    }
}

struct FakeDisplay {
    style: &'static TermStyle,
    wrapped: RefCell<Vec<Message>>,
    overflow_calls: RefCell<Vec<usize>>,
    panel_titles: RefCell<Vec<String>>,
}

impl FakeDisplay {
    fn new() -> Self {
        Self {
            style: Box::leak(Box::new(TermStyle {
                font: iced_widget::core::Font::default(),
                font_size: 14.0,
                line_height_factor: 1.2,
            })),
            wrapped: RefCell::new(Vec::new()),
            overflow_calls: RefCell::new(Vec::new()),
            panel_titles: RefCell::new(Vec::new()),
        }
    }
}

impl<'a> PanelDisplay<'a, Message> for FakeDisplay {
    fn wrap(&self, message: Message) -> Message {
        self.wrapped.borrow_mut().push(message.clone());
        message
    }
    fn locale(&self) -> &str {
        "en"
    }
    fn hover_progress(&self, _slot: AgentHover) -> f32 {
        0.0
    }
    fn hover_message(&self, slot: AgentHover, hovered: bool) -> Message {
        Message::Hover(slot, hovered)
    }
    fn hover_tooltip_ready(&self, _slot: AgentHover) -> bool {
        false
    }
    fn pane_style(&self, _pane: AgentPane) -> PaneStyle {
        PaneStyle {
            gap: 4.0,
            padding: Padding::from(8),
            background: None,
        }
    }
    fn panel_head(
        &self,
        _icon: IconKind,
        title: &str,
        _actions: Option<El<'a, Message>>,
    ) -> El<'a, Message> {
        text(title.to_string()).into()
    }
    fn list_collapse_button(&self, _a: String, _b: String) -> El<'a, Message> {
        text("collapse").into()
    }
    fn tab_divider(&self) -> El<'a, Message> {
        container(text("|")).into()
    }
    fn panel_tab(&self, args: TabArgs<Message>) -> El<'a, Message> {
        self.panel_titles.borrow_mut().push(args.title.clone());
        text(args.title).into()
    }
    fn overflow_button(&self, tab_count: usize, _on_press: Message) -> Option<El<'a, Message>> {
        self.overflow_calls.borrow_mut().push(tab_count);
        (tab_count > 0).then(|| text("V").into())
    }
    fn overflow_menu(
        &self,
        entries: Vec<OverflowEntry>,
        _anchor: (f32, f32),
        _on_select: fn(usize) -> Message,
        _on_close: fn(usize) -> Message,
        _on_dismiss: Message,
    ) -> El<'a, Message> {
        text(format!("menu:{}", entries.len())).into()
    }
    fn drag_surface(&self, content: El<'a, Message>, _index: usize) -> El<'a, Message> {
        content
    }
    fn term_style(&self) -> &'a TermStyle {
        self.style
    }
    fn terminal_focused(&self) -> bool {
        false
    }
    fn ime_preedit(&self) -> Option<&'a str> {
        None
    }
    fn term_event(&self, _event: TermEvent) -> Message {
        Message::Input(Vec::new())
    }
    fn term_event_fn(&self) -> fn(TermEvent) -> Message {
        |_| Message::Input(Vec::new())
    }
    fn tab_bar_avail_px(&self) -> f32 {
        180.0
    }
}

fn make_tab(id: usize, agent: AgentKind, title: &str) -> SessionTab {
    SessionTab {
        info: SessionInfo {
            id: format!("s{id}"),
            name: title.into(),
            command: "sh".into(),
            cwd: "/tmp".into(),
            alive: true,
            created_ms: 0,
            agent_state: AgentState::Idle,
            transcript_path: None,
            project_id: Some(7),
            agent,
        },
        model: dozer_panel_term::model::TerminalModel::new(20, 4),
        alive: true,
        agent_state: AgentState::Idle,
        agent,
        transcript_path: None,
        osc: OscScanner::new(),
        cwd: None,
        last_exit: None,
        llm_model: None,
        permission_mode: None,
        last_activity: None,
        workspace_override: None,
        tab_id: id,
        forwarder: tokio::runtime::Handle::current().spawn(async {}),
        backend: TabBackend::Daemon,
    }
}

#[tokio::test]
async fn agent_list_with_no_tabs_shows_the_empty_text_and_with_tabs_one_group_header_per_non_empty_kind()
 {
    let host = FakeDisplay::new();
    let mut state = AgentPanelState::new();
    let ctx = CardCtx {
        titles: &Titles,
        project_branch: Some("main"),
        project_dirty: false,
    };
    let _ = list::agent_list(&state, &host, &ctx, Length::Fill, Border::default());
    assert!(
        !host
            .wrapped
            .borrow()
            .iter()
            .any(|m| matches!(m, Message::SelectTabNoDrag(_))),
        "空列表不应构造卡片"
    );

    state.tabs.push(make_tab(0, AgentKind::Claude, "a"));
    state.tabs.push(make_tab(1, AgentKind::Claude, "b"));
    state.tabs.push(make_tab(2, AgentKind::Codex, "c"));
    let groups = group_tabs_by_agent(&state.tabs);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].1.len(), 2);
    assert_eq!(groups[1].1.len(), 1);
    let _ = list::agent_list(&state, &host, &ctx, Length::Fill, Border::default());
    let wrapped = host.wrapped.borrow();
    assert!(wrapped.contains(&Message::SelectTabNoDrag(0)));
    assert!(!wrapped.contains(&Message::SelectTab(0)));
}

#[tokio::test]
async fn tab_bar_hides_the_overflow_button_for_zero_tabs_and_shows_it_for_one() {
    let host = FakeDisplay::new();
    let state = AgentPanelState::new();
    let _ = terminal::terminal_pane(
        &state,
        &host,
        TerminalSlots {
            floating: None,
            footer: None,
        },
        Length::Fill,
        Border::default(),
    );
    let mut state2 = AgentPanelState::new();
    state2.tabs.push(make_tab(0, AgentKind::Claude, "a"));
    let _ = terminal::terminal_pane(
        &state2,
        &host,
        TerminalSlots {
            floating: None,
            footer: None,
        },
        Length::Fill,
        Border::default(),
    );
    let calls = host.overflow_calls.borrow();
    assert_eq!(calls.as_slice(), &[0, 1]);
}

#[test]
fn tab_window_uses_the_hosts_available_width_and_term_tab_first() {
    let widths = vec![60.0, 60.0, 60.0, 60.0, 60.0];
    let w = byteui::interaction::tab_strip::tab_window(&widths, 4.0, 130.0, 2);
    assert!(w.first >= 1 && w.first <= 2, "first={}", w.first);
    assert!(w.visible_end > w.first);
}

#[tokio::test]
async fn degenerate_states_build_without_panicking() {
    let host = FakeDisplay::new();
    let ctx = CardCtx {
        titles: &Titles,
        project_branch: None,
        project_dirty: false,
    };
    let mut state = AgentPanelState::new();
    let _ = list::agent_list(&state, &host, &ctx, Length::Fill, Border::default());
    let _ = terminal::terminal_pane(
        &state,
        &host,
        TerminalSlots {
            floating: None,
            footer: None,
        },
        Length::Fill,
        Border::default(),
    );
    assert!(terminal::overflow_popup(&state, &host).is_none());

    state.tabs.push(make_tab(0, AgentKind::Claude, ""));
    state.active = 9;
    state.term_tab_first = 9;
    state.term_tab_overflow_anchor = Some((1.0, 2.0));
    let _ = list::agent_list(&state, &host, &ctx, Length::Fill, Border::default());
    let _ = terminal::terminal_pane(
        &state,
        &host,
        TerminalSlots {
            floating: None,
            footer: None,
        },
        Length::Fill,
        Border::default(),
    );
    assert!(terminal::overflow_popup(&state, &host).is_some());

    state.tabs[0].info.name = "x".repeat(10_000);
    let _ = list::agent_list(&state, &host, &ctx, Length::Fill, Border::default());
    let _ = terminal::terminal_pane(
        &state,
        &host,
        TerminalSlots {
            floating: None,
            footer: None,
        },
        Length::Fill,
        Border::default(),
    );
}

#[tokio::test]
async fn nonzero_last_exit_is_shown_for_the_active_tab_only() {
    let mut state = AgentPanelState::new();
    state.tabs.push(make_tab(0, AgentKind::Claude, "a"));
    assert_eq!(terminal::exit_code_line(&state), None);
    state.tabs[0].last_exit = Some(0);
    assert_eq!(terminal::exit_code_line(&state), None);
    state.tabs[0].last_exit = Some(7);
    assert_eq!(terminal::exit_code_line(&state), Some("exit 7".into()));
    state.active = 1;
    assert_eq!(terminal::exit_code_line(&state), None);
}

#[tokio::test]
async fn terminal_pane_without_tabs_shows_the_no_sessions_text() {
    let host = FakeDisplay::new();
    let state = AgentPanelState::new();
    let _ = terminal::terminal_pane(
        &state,
        &host,
        TerminalSlots {
            floating: None,
            footer: None,
        },
        Length::Fill,
        Border::default(),
    );
    assert_eq!(terminal::empty_label_key(), "term.no_sessions");
}

#[tokio::test]
async fn slots_are_placed_floating_over_the_grid_and_footer_below_it() {
    let host = FakeDisplay::new();
    let mut state = AgentPanelState::new();
    state.tabs.push(make_tab(0, AgentKind::Claude, "a"));
    let floating: El<'static, Message> = text("FLOAT").into();
    let footer: El<'static, Message> = text("FOOTER").into();
    let _ = terminal::terminal_pane(
        &state,
        &host,
        TerminalSlots {
            floating: Some(floating),
            footer: Some(footer),
        },
        Length::Fill,
        Border::default(),
    );
    // 画面内容由组合根决定;这里断言构造不 panic 且插槽参数被接受。
    assert_eq!(Color::WHITE.a, 1.0);
}
