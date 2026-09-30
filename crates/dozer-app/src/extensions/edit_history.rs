//! Agent 修改历史弹窗(v0.1 意向文档 §18 Change History):读 dozerd 的
//! `file_edit_history`,提供 Diff / Locate / Revert / Ask Agent。独立原生窗口宿主
//! 见 `platform/edit_history_overlay.rs`。设计见
//! `docs/superpowers/specs/2026-09-30-agent-native-file-editor-phase3-context-history-design.md`。

use byteui::interaction::icons;
use dozer_core::protocol::{FileEditHistoryInfo, MutationOutcome};
use iced_widget::core::{Alignment, Element, Length};
use iced_widget::{button, column, container, row, scrollable, text};

/// 列表栏宽与底部操作栏高。视图与宿主里的 diff 区几何计算共用这两个常量。
pub const LIST_WIDTH: f32 = 300.0;
pub const ACTIONS_HEIGHT: f32 = 36.0;
/// 默认加载条数上限。
pub const DEFAULT_LIMIT: u32 = 200;

#[derive(Debug, Clone, PartialEq)]
pub enum RevertStatus {
    Pending,
    Done,
    Failed(String),
}

pub struct State {
    project_id: i64,
    filter: Option<String>,
    /// `None` = 加载中。
    entries: Option<Result<Vec<FileEditHistoryInfo>, String>>,
    selected: Option<i64>,
    /// 打开弹窗时终端是否可见,决定 Ask Agent 是否置灰(弹窗持有期间主窗口被
    /// 遮罩挡住,可见性不会变)。
    agent_terminal_visible: bool,
    diff_webview_ready: bool,
    diff_sent_for: Option<i64>,
    revert: Option<(i64, RevertStatus)>,
}

impl State {
    pub fn new(project_id: i64, filter: Option<String>, agent_terminal_visible: bool) -> Self {
        Self {
            project_id,
            filter,
            entries: None,
            selected: None,
            agent_terminal_visible,
            diff_webview_ready: false,
            diff_sent_for: None,
            revert: None,
        }
    }
    pub fn project_id(&self) -> i64 {
        self.project_id
    }
    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }
    pub fn entries(&self) -> Option<&Result<Vec<FileEditHistoryInfo>, String>> {
        self.entries.as_ref()
    }
    pub fn selected(&self) -> Option<i64> {
        self.selected
    }
    pub fn entry(&self, id: i64) -> Option<&FileEditHistoryInfo> {
        match self.entries.as_ref()? {
            Ok(v) => v.iter().find(|e| e.id == id),
            Err(_) => None,
        }
    }
    pub fn selected_entry(&self) -> Option<&FileEditHistoryInfo> {
        self.entry(self.selected?)
    }
    pub fn agent_terminal_visible(&self) -> bool {
        self.agent_terminal_visible
    }
    pub fn revert_status(&self) -> Option<&(i64, RevertStatus)> {
        self.revert.as_ref()
    }
    pub fn diff_webview_ready(&self) -> bool {
        self.diff_webview_ready
    }
    pub fn set_diff_webview_ready(&mut self, v: bool) {
        self.diff_webview_ready = v;
    }
    pub fn diff_sent_for(&self) -> Option<i64> {
        self.diff_sent_for
    }
    pub fn set_diff_sent_for(&mut self, id: i64) {
        self.diff_sent_for = Some(id);
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Close,
    /// `project_id`/`filter` 用于核对结果落地时还是不是当前请求(弹窗重开或
    /// 切了过滤后,迟到的旧结果直接丢弃)。
    Loaded {
        project_id: i64,
        filter: Option<String>,
        result: Result<Vec<FileEditHistoryInfo>, String>,
    },
    Select(i64),
    SetFilter(Option<String>),
    /// App 拦截(需要预览句柄)。
    Locate(i64),
    /// App 拦截(需要终端句柄)。
    AskAgent(i64),
    Revert(i64),
    RevertDone {
        id: i64,
        result: Result<MutationOutcome, String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Followup {
    None,
    Reload,
    Revert(FileEditHistoryInfo),
}

/// 撤销失败的用户可读原因。
fn revert_failure(outcome: MutationOutcome) -> Option<String> {
    match outcome {
        MutationOutcome::Applied { .. } => None,
        MutationOutcome::Conflict { .. } => Some("文件已在此后被修改,无法撤销".into()),
        MutationOutcome::NotFound => Some("文件已不存在,无法撤销".into()),
        MutationOutcome::PathOutOfBounds => Some("路径越界,无法撤销".into()),
        MutationOutcome::Unwritable { reason } => Some(format!("无法写入: {reason}")),
    }
}

/// 纯状态转移(可单测);IO 由 `update` 按 `Followup` 执行。
pub fn apply(state: &mut Option<State>, msg: Message) -> Followup {
    if let Message::Close = msg {
        *state = None;
        return Followup::None;
    }
    let Some(s) = state.as_mut() else {
        return Followup::None; // 弹窗已关,迟到的消息忽略
    };
    match msg {
        Message::Close => unreachable!("上面已处理"),
        Message::Loaded {
            project_id,
            filter,
            result,
        } => {
            if s.project_id != project_id || s.filter != filter {
                return Followup::None;
            }
            let first = match &result {
                Ok(v) => v.first().map(|e| e.id),
                Err(_) => None,
            };
            let keep = match (&result, s.selected) {
                (Ok(v), Some(id)) if v.iter().any(|e| e.id == id) => Some(id),
                _ => first,
            };
            if keep != s.selected {
                s.diff_sent_for = None;
            }
            s.selected = keep;
            s.entries = Some(result);
            Followup::None
        }
        Message::Select(id) => {
            let old_path = s.selected_entry().map(|e| e.target_path.clone());
            let new_path = s.entry(id).map(|e| e.target_path.clone());
            s.selected = Some(id);
            s.diff_sent_for = None;
            // URL 由文件路径决定:路径变了 webview 会重载,`ready` 要重置。
            if old_path != new_path {
                s.diff_webview_ready = false;
            }
            Followup::None
        }
        Message::SetFilter(filter) => {
            s.filter = filter;
            s.entries = None;
            s.selected = None;
            s.diff_sent_for = None;
            s.diff_webview_ready = false;
            Followup::Reload
        }
        Message::Locate(_) | Message::AskAgent(_) => Followup::None,
        Message::Revert(id) => {
            let Some(entry) = s.entry(id).cloned() else {
                return Followup::None;
            };
            s.revert = Some((id, RevertStatus::Pending));
            Followup::Revert(entry)
        }
        Message::RevertDone { id, result } => match result {
            Ok(outcome) => match revert_failure(outcome) {
                None => {
                    s.revert = Some((id, RevertStatus::Done));
                    Followup::Reload
                }
                Some(reason) => {
                    s.revert = Some((id, RevertStatus::Failed(reason)));
                    Followup::None
                }
            },
            Err(e) => {
                s.revert = Some((id, RevertStatus::Failed(e)));
                Followup::None
            }
        },
    }
}

pub fn request_load(
    project_id: i64,
    filter: Option<String>,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let result = client
            .list_file_edit_history(project_id, filter.as_deref(), DEFAULT_LIMIT)
            .await
            .map_err(|e| e.to_string());
        emit(Message::Loaded {
            project_id,
            filter,
            result,
        });
    });
}

/// 执行 `apply` 并按 `Followup` 做 IO。Revert:反向调用 `apply_precise_edit`——
/// `expected_text` = 该条 `new_text`,`new_text` = 该条 `old_text`,坐标用该条
/// **修改后**的区间(起点不变,结束坐标取 `new_end_*`)。署名 `actor = "dozer"`
/// (human 经 GUI 触发,不冒充 agent);走同一套 Conflict Detection,磁盘在那次
/// 修改之后又变过就会得到 `Conflict` 而不是盲目覆盖。
pub fn update(
    state: &mut Option<State>,
    msg: Message,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match apply(state, msg) {
        Followup::None => {}
        Followup::Reload => {
            let Some(s) = state.as_ref() else { return };
            request_load(s.project_id, s.filter.clone(), client, handle, emit);
        }
        Followup::Revert(e) => {
            let client = client.clone();
            handle.spawn(async move {
                let result = client
                    .apply_precise_edit(
                        e.project_id,
                        &e.target_path,
                        e.start_line,
                        e.start_col,
                        e.new_end_line,
                        e.new_end_col,
                        &e.new_text,
                        &e.old_text,
                        &format!("撤销:{}", e.summary),
                        "dozer",
                        "dozer-gui",
                    )
                    .await
                    .map_err(|err| err.to_string());
                emit(Message::RevertDone { id: e.id, result });
            });
        }
    }
}

fn flat(
    color: iced_widget::core::Color,
) -> impl Fn(&iced_widget::Theme, button::Status) -> button::Style {
    move |_t, _s| button::Style {
        background: None,
        text_color: color,
        ..button::Style::default()
    }
}

fn when(ms: u64) -> String {
    crate::extensions::file_history::format_commit_time((ms / 1000) as i64)
}

pub fn edit_history_card(
    state: &State,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();

    let mut title = row![
        icons::view(
            icons::IconKind::History,
            byteui::theme::icon_size::row(),
            colors.cream
        ),
        text("Agent 修改历史")
            .size(byteui::theme::font::subtitle())
            .color(colors.cream),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    if let Some(f) = state.filter() {
        title = title.push(
            button(
                text(format!("过滤: {f}  ×"))
                    .size(byteui::theme::font::label())
                    .color(colors.gold),
            )
            .padding(0)
            .on_press(Message::SetFilter(None))
            .style(flat(colors.gold)),
        );
    }
    let title = title.push(iced_widget::space::horizontal()).push(
        button(
            text("×")
                .size(byteui::theme::font::subtitle())
                .color(colors.dim),
        )
        .on_press(Message::Close)
        .style(flat(colors.dim)),
    );

    let body = row![
        container(list_view(state))
            .width(Length::Fixed(LIST_WIDTH))
            .height(Length::Fill),
        detail_view(state),
    ]
    .spacing(12)
    .height(Length::Fill);

    container(column![title, body].spacing(12))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(16)
        .style(crate::dialog::card_style)
        .into()
}

fn list_view(state: &State) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let msg = |s: String, c| -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
        text(s).size(byteui::theme::font::body()).color(c).into()
    };
    match state.entries() {
        None => msg("加载中…".into(), colors.dim),
        Some(Err(e)) => msg(format!("加载失败: {e}"), colors.red),
        Some(Ok(v)) if v.is_empty() => msg("还没有 agent 修改记录".into(), colors.dim),
        Some(Ok(v)) => {
            let mut col = column![].spacing(4);
            for e in v {
                let selected = state.selected() == Some(e.id);
                let c = if selected { colors.gold } else { colors.cream };
                col = col.push(
                    button(
                        column![
                            text(e.summary.clone())
                                .size(byteui::theme::font::body())
                                .color(c),
                            text(format!(
                                "{} · {} · {}",
                                e.target_path,
                                e.actor,
                                when(e.created_ms)
                            ))
                            .size(byteui::theme::font::label())
                            .color(colors.dim),
                        ]
                        .spacing(2),
                    )
                    .padding(6)
                    .width(Length::Fill)
                    .on_press(Message::Select(e.id))
                    .style(flat(c)),
                );
            }
            scrollable(col).height(Length::Fill).into()
        }
    }
}

fn detail_view(state: &State) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let Some(e) = state.selected_entry() else {
        return container(
            text("选择左侧一条修改查看详情")
                .size(byteui::theme::font::body())
                .color(colors.dim),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    };
    let header = text(format!(
        "{}:{}:{}",
        e.target_path, e.start_line, e.start_col
    ))
    .size(byteui::theme::font::label())
    .color(colors.cream);

    // diff 占位:Task 7 的 wry webview 叠在这块区域上,几何见宿主的 `diff_area_bounds`。
    let diff_slot = container(text("")).width(Length::Fill).height(Length::Fill);

    let action = |label: &'static str, msg: Option<Message>, c| {
        let mut b = button(text(label).size(byteui::theme::font::label()).color(c))
            .padding([4, 10])
            .style(flat(c));
        if let Some(m) = msg {
            b = b.on_press(m);
        }
        b
    };
    let ask = if state.agent_terminal_visible() {
        action("Ask Agent", Some(Message::AskAgent(e.id)), colors.gold)
    } else {
        action("Ask Agent", None, colors.dim)
    };
    let mut actions = row![
        action("Locate", Some(Message::Locate(e.id)), colors.gold),
        ask,
        action("Revert", Some(Message::Revert(e.id)), colors.gold),
        action(
            "只看此文件",
            Some(Message::SetFilter(Some(e.target_path.clone()))),
            colors.cream
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    match state.revert_status() {
        Some((id, RevertStatus::Pending)) if *id == e.id => {
            actions = actions.push(
                text("撤销中…")
                    .size(byteui::theme::font::label())
                    .color(colors.dim),
            );
        }
        Some((id, RevertStatus::Done)) if *id == e.id => {
            actions = actions.push(
                text("已撤销")
                    .size(byteui::theme::font::label())
                    .color(colors.green),
            );
        }
        Some((id, RevertStatus::Failed(m))) if *id == e.id => {
            actions = actions.push(
                text(m.clone())
                    .size(byteui::theme::font::label())
                    .color(colors.red),
            );
        }
        _ => {}
    }

    column![
        header,
        diff_slot,
        container(actions)
            .height(Length::Fixed(ACTIONS_HEIGHT))
            .align_y(iced_widget::core::alignment::Vertical::Center),
    ]
    .spacing(8)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: i64, path: &str) -> FileEditHistoryInfo {
        FileEditHistoryInfo {
            id,
            project_id: 1,
            target_path: path.into(),
            actor: "claude".into(),
            session_id: "s".into(),
            start_line: 2,
            start_col: 1,
            end_line: 2,
            end_col: 9,
            new_end_line: 2,
            new_end_col: 9,
            old_text: "old".into(),
            new_text: "new".into(),
            summary: "改".into(),
            created_ms: id as u64,
        }
    }

    fn open(filter: Option<&str>) -> Option<State> {
        Some(State::new(1, filter.map(String::from), true))
    }

    #[test]
    fn loaded_selects_first_and_drops_stale_results() {
        let mut st = open(None);
        // 过期:项目不同 / 过滤不同 → 丢弃。
        apply(
            &mut st,
            Message::Loaded {
                project_id: 2,
                filter: None,
                result: Ok(vec![info(1, "a")]),
            },
        );
        assert!(st.as_ref().unwrap().entries().is_none());
        apply(
            &mut st,
            Message::Loaded {
                project_id: 1,
                filter: Some("x".into()),
                result: Ok(vec![info(1, "a")]),
            },
        );
        assert!(st.as_ref().unwrap().entries().is_none());
        // 有效。
        apply(
            &mut st,
            Message::Loaded {
                project_id: 1,
                filter: None,
                result: Ok(vec![info(5, "a"), info(4, "b")]),
            },
        );
        let s = st.as_ref().unwrap();
        assert_eq!(s.selected(), Some(5));
        assert_eq!(s.selected_entry().unwrap().target_path, "a");
    }

    #[test]
    fn reload_keeps_selection_when_still_present_else_selects_first() {
        let mut st = open(None);
        apply(
            &mut st,
            Message::Loaded {
                project_id: 1,
                filter: None,
                result: Ok(vec![info(5, "a"), info(4, "b")]),
            },
        );
        apply(&mut st, Message::Select(4));
        apply(
            &mut st,
            Message::Loaded {
                project_id: 1,
                filter: None,
                result: Ok(vec![info(6, "a"), info(5, "a"), info(4, "b")]),
            },
        );
        assert_eq!(st.as_ref().unwrap().selected(), Some(4));
        apply(
            &mut st,
            Message::Loaded {
                project_id: 1,
                filter: None,
                result: Ok(vec![info(9, "z")]),
            },
        );
        assert_eq!(st.as_ref().unwrap().selected(), Some(9));
    }

    #[test]
    fn set_filter_clears_entries_and_requests_reload() {
        let mut st = open(None);
        apply(
            &mut st,
            Message::Loaded {
                project_id: 1,
                filter: None,
                result: Ok(vec![info(1, "a")]),
            },
        );
        assert_eq!(
            apply(&mut st, Message::SetFilter(Some("a".into()))),
            Followup::Reload
        );
        let s = st.as_ref().unwrap();
        assert_eq!(s.filter(), Some("a"));
        assert!(s.entries().is_none(), "切换过滤后先清空,等新结果");
        assert_eq!(s.selected(), None);
    }

    #[test]
    fn select_resets_diff_bookkeeping_only_when_path_changes() {
        let mut st = open(None);
        apply(
            &mut st,
            Message::Loaded {
                project_id: 1,
                filter: None,
                result: Ok(vec![info(3, "a"), info(2, "a"), info(1, "b")]),
            },
        );
        st.as_mut().unwrap().set_diff_webview_ready(true);
        st.as_mut().unwrap().set_diff_sent_for(3);
        apply(&mut st, Message::Select(2)); // 同一路径:webview 不重载
        let s = st.as_ref().unwrap();
        assert!(s.diff_webview_ready());
        assert_eq!(s.diff_sent_for(), None, "换了条目就要重新推内容");
        apply(&mut st, Message::Select(1)); // 换路径:URL 会变,webview 重载
        assert!(!st.as_ref().unwrap().diff_webview_ready());
    }

    #[test]
    fn revert_lifecycle() {
        let mut st = open(None);
        apply(
            &mut st,
            Message::Loaded {
                project_id: 1,
                filter: None,
                result: Ok(vec![info(7, "a")]),
            },
        );
        let f = apply(&mut st, Message::Revert(7));
        assert!(matches!(f, Followup::Revert(ref e) if e.id == 7));
        assert!(matches!(
            st.as_ref().unwrap().revert_status(),
            Some((7, RevertStatus::Pending))
        ));

        let applied = MutationOutcome::Applied {
            new_start_line: 2,
            new_start_col: 1,
            new_end_line: 2,
            new_end_col: 4,
            history_id: 8,
        };
        assert_eq!(
            apply(
                &mut st,
                Message::RevertDone {
                    id: 7,
                    result: Ok(applied)
                }
            ),
            Followup::Reload
        );
        assert!(matches!(
            st.as_ref().unwrap().revert_status(),
            Some((7, RevertStatus::Done))
        ));

        apply(&mut st, Message::Revert(7));
        apply(
            &mut st,
            Message::RevertDone {
                id: 7,
                result: Ok(MutationOutcome::Conflict {
                    actual_text: "别的".into(),
                }),
            },
        );
        match st.as_ref().unwrap().revert_status() {
            Some((7, RevertStatus::Failed(m))) => assert!(m.contains("已在此后被修改"), "{m}"),
            other => panic!("{other:?}"),
        }
        apply(&mut st, Message::Revert(7));
        apply(
            &mut st,
            Message::RevertDone {
                id: 7,
                result: Err("连接断了".into()),
            },
        );
        assert!(
            matches!(st.as_ref().unwrap().revert_status(), Some((7, RevertStatus::Failed(m))) if m.contains("连接断了"))
        );
    }

    #[test]
    fn revert_of_unknown_entry_is_a_noop() {
        let mut st = open(None);
        assert_eq!(apply(&mut st, Message::Revert(99)), Followup::None);
    }

    #[test]
    fn close_drops_state_and_late_results_are_ignored() {
        let mut st = open(None);
        apply(&mut st, Message::Close);
        assert!(st.is_none());
        assert_eq!(
            apply(
                &mut st,
                Message::Loaded {
                    project_id: 1,
                    filter: None,
                    result: Ok(vec![])
                }
            ),
            Followup::None
        );
    }
}
