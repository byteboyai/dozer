// crates/dozer-app/src/extensions/conversations.rs
//! 对话(Conversations)面板列表侧:当前项目全部 session 的搜索/agent 筛选/
//! 客户端分页/卡片列表。命名用复数——`crate::conversation` 已经是历史对话
//! 展示用的中间表示数据层(`SessionRow`/`ConversationMeta` 等,Usage 面板
//! 也用),这里是另一回事,用复数避免和它读混。
//!
//! 详情审阅侧(`ReviewView`/`dozer://review-trace` webview host)不在这个
//! 模块里——它是内核共享基础设施,同时服务核心 Agent 面板的
//! `ReviewSource::Session` 和本面板的 `ReviewSource::Conversation`,继续留在
//! `workspace.rs`(见 docs/superpowers/specs/2026-09-15-conversations-extension-pilot-design.md)。

use crate::app::App;
use crate::app::{HoverId, ProjectId, TextInputTarget};
use crate::conversation::SessionRow;
use crate::homespace::home_panel_head;
use crate::theme;
use crate::workspace::{agent_dot_color, agent_icon, lh, relative_time_text};
use byteui::interaction::icons;
use byteui::interaction::icons::IconKind;
use dozer_core::protocol::AgentKind;
use dozer_core::protocol::TodoInfo;
use iced_widget::core::Rectangle;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Border, Element, Length};
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable, text};
use std::path::PathBuf;
use std::sync::Mutex;

/// 会话列表(对话面板扁平列表)一页显示的条数,与 Git Log commit 列表的
/// `COMMIT_PAGE_SIZE` 保持一致。首帧 1 页,点"更多..."页数递增。
pub(crate) const CONVERSATION_PAGE_SIZE: usize = 20;

/// 按 `pages`(点过几次"更多",0 起)算出当前应该显示到第几条。抽成纯函数
/// 方便 headless 单测;视图只把返回值画出来。
pub(crate) fn conversation_visible_count(pages: usize) -> usize {
    (pages + 1) * CONVERSATION_PAGE_SIZE
}

/// 会话列表关键字 + agent 过滤:标题(`display_title`)大小写不敏感子串匹配
/// (空关键字不过滤标题这一维)叠加 agent 精确匹配(`None` = 不限)。
///
/// `pub(crate)` 而不是私有:Task 3 之前,`workspace.rs` 里还没搬走的
/// `conversation_list_pane` 需要跨模块调用它;Task 3 把调用方也搬进本模块
/// 后,两者同处一个文件,可见性不需要再改回私有——保持 `pub(crate)` 不动即可。
pub(crate) fn filter_sessions<'a>(
    rows: &'a [SessionRow],
    query: &str,
    agent: Option<AgentKind>,
) -> Vec<&'a SessionRow> {
    let needle = query.to_lowercase();
    rows.iter()
        .filter(|r| agent.map(|a| r.agent == a).unwrap_or(true))
        .filter(|r| {
            query.is_empty()
                || r.display_title.to_lowercase().contains(&needle)
                || r.summary
                    .as_deref()
                    .is_some_and(|s| s.to_lowercase().contains(&needle))
        })
        .collect()
}

/// 会话列表里出现过的 agent 种类,去重,固定展示顺序。供底部 footbar 画筛选
/// chip;返回空 = 没有会话数据,footbar 不渲染。`pub(crate)` 理由同
/// `filter_sessions`。
pub(crate) fn conversation_agents_present(rows: &[SessionRow]) -> Vec<AgentKind> {
    const ORDER: [AgentKind; 7] = [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::Codex,
        AgentKind::Kilo,
        AgentKind::V8agent,
        AgentKind::Unknown,
    ];
    ORDER
        .into_iter()
        .filter(|k| rows.iter().any(|r| r.agent == *k))
        .collect()
}

/// 会话列表搜索框(iced 原生 `text_input`)的 `widget::Id`:main.rs 每帧
/// `interface.operate` 用 `CaptureConversationSearchFocus` 问真实焦点态。
pub fn conversation_search_field_id() -> Id {
    Id::new("conversation-search-box")
}

static CONVERSATION_SEARCH_FOCUSED: std::sync::LazyLock<Mutex<bool>> =
    std::sync::LazyLock::new(|| Mutex::new(false));

/// 读走并复位(消费式),避免搜索框不可见的帧卡死上一次 `true` 永久堵死终端
/// 键盘转发。
pub fn take_conversation_search_focused() -> bool {
    std::mem::replace(&mut *CONVERSATION_SEARCH_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍。`traverse` 必须调用传入闭包(见
/// [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureConversationSearchFocus;
impl Operation<()> for CaptureConversationSearchFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&conversation_search_field_id()) {
            *CONVERSATION_SEARCH_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

/// 挂在每个 Workspace 上的对话面板列表侧状态,对应现有 `Workspace` 上 7 个
/// 平铺的 `conversation_*` 字段。
#[derive(Default)]
pub struct WorkspaceState {
    sessions: Option<Vec<SessionRow>>,
    pages: usize,
    search: String,
    search_draft: String,
    search_focused: bool,
    agent_filter: Option<AgentKind>,
    agent_picker_open: bool,
}

impl WorkspaceState {
    /// 供内核 `App::conversation_session_open` 按 `conversation_id` 查找
    /// 对应行的总结信息用。
    pub fn sessions(&self) -> Option<&[SessionRow]> {
        self.sessions.as_deref()
    }

    /// 会话列表搜索框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn search_focused(&self) -> bool {
        self.search_focused
    }

    /// 每帧渲染循环读走 `CaptureConversationSearchFocus` 查到的真实焦点态后
    /// 写回这里。
    pub fn set_search_focused(&mut self, focused: bool) {
        self.search_focused = focused;
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    /// 会话列表刷新结果:当前项目全部 session(联查总结),按最后活跃时间
    /// 倒序。按自带的 `ProjectId` 走 `with_project` 路由。
    SessionsRefreshed(ProjectId, Result<Vec<SessionRow>, String>),
    /// 点击某一行打开该 session 的详情审阅。由内核 `App::update` 直接拦截
    /// 处理(要写进跨面板共享的 `ws.review`),不会到达 `update()`。
    SessionOpen(String, AgentKind),
    /// 详情页"加载更多"追加下一页回合。同上,内核直接拦截处理。
    DetailLoadMore(String, i64),
    /// 列表底部"更多..."翻页,纯客户端状态。
    ListMore,
    /// 搜索框草稿变化。
    SearchInput(String),
    /// 回车/点搜索按钮,草稿落成生效过滤词并重置分页。
    SearchSubmit,
    /// agent 筛选下拉某一项点击,`None` = 全部;重置分页并收起下拉。
    AgentFilterSelect(Option<AgentKind>),
    /// 展开 agent 筛选下拉。
    AgentPickerOpen,
    /// 收起 agent 筛选下拉。
    AgentPickerClose,
    /// 搜索框/翻页按钮的 hover 进度条动画。由内核直接拦截转发给
    /// `App::set_hover`(同 `usage::Message::Hover` 的既有写法),不会到达
    /// `update()`。
    Hover(HoverId, bool),
    /// 搜索框右键菜单目标。由内核直接拦截转发给顶层
    /// `Message::TextInputMenuOpen`(同 `files::Message::TextInputMenuOpen`
    /// 的既有写法),不会到达 `update()`。
    TextInputMenuOpen(TextInputTarget),
}

pub fn update(ws_state: &mut WorkspaceState, msg: Message) {
    match msg {
        Message::SessionsRefreshed(_, result) => {
            ws_state.sessions = Some(match result {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::warn!(error = %e, "对话会话列表查询失败");
                    Vec::new()
                }
            });
        }
        Message::ListMore => ws_state.pages += 1,
        Message::SearchInput(s) => ws_state.search_draft = s,
        Message::SearchSubmit => {
            ws_state.search = ws_state.search_draft.clone();
            ws_state.pages = 0;
        }
        Message::AgentFilterSelect(agent) => {
            ws_state.agent_filter = agent;
            ws_state.pages = 0;
            ws_state.agent_picker_open = false;
        }
        Message::AgentPickerOpen => ws_state.agent_picker_open = true,
        Message::AgentPickerClose => ws_state.agent_picker_open = false,
        Message::SessionOpen(..)
        | Message::DetailLoadMore(..)
        | Message::Hover(..)
        | Message::TextInputMenuOpen(..) => {
            unreachable!("由内核 App::update 直接拦截处理,不会转发到这里")
        }
    }
}

/// 加载(或刷新)当前项目的会话列表(联查总结,标题+摘要预览)。签名/实现
/// 照搬现有 `Workspace::spawn_conversations_refresh`,`client` 参数传入方式
/// 对齐 Usage 试点既有 `usage::spawn_refresh`。
pub fn spawn_refresh(
    project_id: i64,
    cwd: PathBuf,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let result = client
            .list_conversations_with_summaries(&cwd.to_string_lossy(), None, 500, 0)
            .await
            .map(|rows| {
                rows.iter()
                    .map(|(c, s)| SessionRow::from_row(c, s.as_ref()))
                    .collect()
            })
            .map_err(|e| e.to_string());
        emit(Message::SessionsRefreshed(project_id, result));
    });
}

/// 会话列表可收纳的最小宽度,同 `project::footer_min_width` 的估算口径:
/// 拖 `Divider::RightPairSplit`(`right_view == Conversations` 时)低于
/// 此宽度直接收起(见 `apply_column_drag` 该分支)。按 `footer_bar` 未筛选
/// 态(标签"全部",2 字)估:图标 + 文案 + 展开箭头按钮(`padding(6)` 的
/// 方形图标按钮)、三段间 `spacing(6)`,外层 padding 用
/// `conversation_list_pane().padding`(`view` 顶层用的同一份)。筛选到某个
/// agent 名后标签可能变长,这里只保底不筛选态,近似值。
pub(crate) fn list_min_width() -> f32 {
    let icon = byteui::theme::icon_size::row();
    let label_px = byteui::theme::font::label() as f32;
    let switch = icon + 12.0;
    let region = theme::region::conversation_list_pane();
    icon + label_px * 2.0 + switch + 6.0 * 3.0 + region.padding.left + region.padding.right
}

/// 会话列表底部 agent 筛选栏:样式对齐文件树面板的分支切换下拉——左边
/// 图标 + 当前筛选(agent 名称,不筛选时"全部"),右边一个展开/收起下拉
/// 的箭头按钮。`agents` 为空(没有会话数据)时不渲染整条 bar。
fn footer_bar<'a>(
    ws_state: &WorkspaceState,
    agents: &[AgentKind],
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if agents.is_empty() {
        return column![].into();
    }

    let current_label = ws_state
        .agent_filter
        .map(|a| a.label().to_string())
        .unwrap_or_else(|| "全部".to_string());
    let switch = button(icons::view(
        if ws_state.agent_picker_open {
            IconKind::ChevronUp
        } else {
            IconKind::ChevronDown
        },
        byteui::theme::icon_size::row(),
        byteui::theme::color::current().cream,
    ))
    .on_press(if ws_state.agent_picker_open {
        Message::AgentPickerClose
    } else {
        Message::AgentPickerOpen
    })
    .padding(6)
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: byteui::theme::color::current().cream,
        border: iced_widget::core::Border {
            color: byteui::theme::color::current().border,
            width: 0.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    });

    let bar = row![
        icons::view(
            IconKind::Bot,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().cream,
        ),
        text(current_label)
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().cream),
        iced_widget::space::horizontal(),
        switch,
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let top_line = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..container::Style::default()
        });

    container(column![top_line, bar].spacing(4))
        .width(Length::Fill)
        .padding([6, 0])
        .style(|_t: &iced_widget::Theme| container::Style {
            background: None,
            ..container::Style::default()
        })
        .into()
}

/// `footer_bar` 展开的下拉弹出层:列出"全部" + `agents`,点某项即
/// `AgentFilterSelect` 切换并收起(本地 `stack!` 叠在面板自己内容之上,不是
/// 全窗 overlay)。未展开时返回零高度元素。
fn agent_picker_view(
    ws_state: &WorkspaceState,
    agents: &[AgentKind],
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !ws_state.agent_picker_open {
        return iced_widget::Space::new().into();
    }
    let is_all_current = ws_state.agent_filter.is_none();
    let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        Vec::new();
    items.push(crate::menu::item_row_fill(
        None,
        "全部",
        if is_all_current {
            byteui::theme::color::current().gold
        } else {
            byteui::theme::color::current().body
        },
        (!is_all_current).then_some(Message::AgentFilterSelect(None)),
    ));
    for &agent in agents {
        let is_current = ws_state.agent_filter == Some(agent);
        let color = if is_current {
            byteui::theme::color::current().gold
        } else {
            byteui::theme::color::current().body
        };
        let leading = icons::view(
            agent_icon(agent),
            byteui::theme::icon_size::row(),
            agent_dot_color(agent),
        );
        items.push(crate::menu::item_row_fill(
            Some(leading),
            agent.label(),
            color,
            (!is_current).then_some(Message::AgentFilterSelect(Some(agent))),
        ));
    }
    let panel: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        crate::menu::shell_frosted(items, Length::Fill);

    let dismiss = MouseArea::new(
        iced_widget::Space::new()
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::AgentPickerClose);
    let positioned = column![iced_widget::Space::new().height(Length::Fill), panel]
        .width(Length::Fill)
        .height(Length::Fill);
    iced_widget::stack![dismiss, positioned]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// 对话列表面板(右面板区"对话"视图的列表侧):当前项目全部 session 的
/// 列表(标题+总结预览),按最后活跃时间倒序。点一行 → `SessionOpen` 驱动
/// 内核加载右侧审阅内容。列表按 `conversation_visible_count` 客户端分页。
///
/// `todo_items`/`open_transcript_paths` 由内核跨读传入(见设计文档"关键
/// 语义确认"——这不是 extension 间耦合,`conversations` 模块本身不持有
/// `todo::WorkspaceState`,只是内核在组装这次 `view` 调用时顺带传了两份
/// 只读数据)。
pub fn view<'a>(
    app: &'a App,
    ws_state: &'a WorkspaceState,
    todo_items: &[TodoInfo],
    open_transcript_paths: &[String],
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::conversation_list_pane();
    let mut content =
        column![home_panel_head(IconKind::BotMessageSquare, "会话"),].spacing(region.gap);

    let search_active = ws_state.search_focused || !ws_state.search.is_empty();
    let search_box = byteui::form::search_box::view(
        "搜索会话标题/摘要…",
        &ws_state.search_draft,
        Some(conversation_search_field_id()),
        search_active,
        Message::SearchInput,
        Message::SearchSubmit,
        app.hover_progress(HoverId::ConversationSearchSubmit),
        |hovered| Message::Hover(HoverId::ConversationSearchSubmit, hovered),
    );
    content = content.push(byteui::interaction::context_menu::wrap(
        search_box,
        Some(Message::TextInputMenuOpen(TextInputTarget {
            id: conversation_search_field_id(),
            secure: false,
        })),
    ));

    let Some(rows) = ws_state.sessions.as_ref() else {
        content = content.push(lh(text("加载中…")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim)));
        return container(content.padding(region.padding))
            .width(width)
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: region.background.map(Into::into),
                border: outer,
                ..container::Style::default()
            })
            .into();
    };

    let agents_present = conversation_agents_present(rows);
    let filtered = filter_sessions(rows, &ws_state.search, ws_state.agent_filter);
    if rows.is_empty() {
        content = content.push(lh(text("暂无对话记录")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim)));
    } else if filtered.is_empty() {
        content = content.push(lh(text("无匹配结果")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim)));
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let mut cards = column![].spacing(region.gap);
    let visible = conversation_visible_count(ws_state.pages);
    for g in filtered.iter().take(visible) {
        let current = crate::conversation::is_current_conversation_id(
            &g.conversation_id,
            open_transcript_paths,
        );
        let agent_label = g.agent.label();
        let task_suffix = g
            .task_id
            .and_then(|task_id| {
                todo_items
                    .iter()
                    .find(|t| t.id == task_id)
                    .map(|t| t.text.as_str())
            })
            .map(|text| {
                let truncated: String = text.chars().take(30).collect();
                if text.chars().count() > 30 {
                    format!(" · 关联任务:{truncated}…")
                } else {
                    format!(" · 关联任务:{truncated}")
                }
            })
            .unwrap_or_default();
        let sub = if current {
            format!(
                "● 当前 · {agent_label} · {}{task_suffix}",
                relative_time_text(g.last_ts, now_ms)
            )
        } else {
            format!(
                "{agent_label} · {}{task_suffix}",
                relative_time_text(g.last_ts, now_ms)
            )
        };
        let sub_color = if current {
            byteui::theme::color::current().green
        } else {
            byteui::theme::color::current().dim
        };
        let row_btn = button(
            row![
                icons::view(
                    agent_icon(g.agent),
                    byteui::theme::icon_size::row(),
                    agent_dot_color(g.agent),
                ),
                column![
                    lh(text(g.display_title.clone())
                        .size(byteui::theme::font::body())
                        .color(byteui::theme::color::current().cream)),
                    lh(text(sub)
                        .size(byteui::theme::font::caption_sm())
                        .color(sub_color)),
                ]
                .spacing(4),
            ]
            .spacing(8)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .on_press(Message::SessionOpen(g.conversation_id.clone(), g.agent))
        .width(Length::Fill)
        .padding(10)
        .style(byteui::interaction::cards::button_card(
            current,
            byteui::theme::color::current().card,
        ));
        cards = cards.push(row_btn);
    }
    if filtered.len() > visible {
        let more_button = icons::icon_button_entry(
            icons::IconKind::Ellipsis,
            byteui::theme::icon_size::row(),
            false,
            false,
            app.hover_progress(HoverId::ConversationListMore),
            false,
            byteui::theme::geometry::tab_button_size(),
            true,
            Message::ListMore,
            |hovered| Message::Hover(HoverId::ConversationListMore, hovered),
            "更多",
        );
        cards = cards.push(
            container(more_button)
                .width(Length::Fill)
                .align_x(iced_widget::core::alignment::Horizontal::Center),
        );
    }
    content = content.push(
        Scrollable::new(cards)
            .width(Length::Fill)
            .height(Length::Fill)
            .direction(scrollable::Direction::Vertical(
                byteui::interaction::scrollbar::scrollbar(),
            ))
            .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
    );
    content = content.push(footer_bar(ws_state, &agents_present));

    let panel = container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        });

    iced_widget::stack![panel, agent_picker_view(ws_state, &agents_present)]
        .width(width)
        .height(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_submit_commits_draft_and_resets_pages() {
        let mut ws_state = WorkspaceState {
            search_draft: "foo".into(),
            pages: 3,
            ..WorkspaceState::default()
        };
        update(&mut ws_state, Message::SearchSubmit);
        assert_eq!(ws_state.search, "foo");
        assert_eq!(ws_state.pages, 0);
    }

    #[test]
    fn agent_filter_select_resets_pages_and_closes_picker() {
        let mut ws_state = WorkspaceState {
            pages: 2,
            agent_picker_open: true,
            ..WorkspaceState::default()
        };
        update(
            &mut ws_state,
            Message::AgentFilterSelect(Some(AgentKind::Claude)),
        );
        assert_eq!(ws_state.agent_filter, Some(AgentKind::Claude));
        assert_eq!(ws_state.pages, 0);
        assert!(!ws_state.agent_picker_open);
    }

    #[test]
    fn sessions_refreshed_err_lands_as_empty_vec_not_none() {
        let mut ws_state = WorkspaceState::default();
        update(
            &mut ws_state,
            Message::SessionsRefreshed(1, Err("boom".into())),
        );
        assert_eq!(ws_state.sessions(), Some([].as_slice()));
    }

    #[test]
    fn filter_sessions_matches_title_case_insensitive() {
        let row = SessionRow {
            conversation_id: "c1".into(),
            agent: AgentKind::Claude,
            last_ts: 0,
            display_title: "Fix Login Bug".into(),
            summary: None,
            summary_status: None,
            task_id: None,
        };
        let rows = vec![row];
        assert_eq!(filter_sessions(&rows, "login", None).len(), 1);
        assert_eq!(filter_sessions(&rows, "nomatch", None).len(), 0);
    }

    #[test]
    fn conversation_visible_count_starts_at_one_page() {
        assert_eq!(conversation_visible_count(0), CONVERSATION_PAGE_SIZE);
    }

    #[test]
    fn conversation_visible_count_grows_by_page_size() {
        assert_eq!(conversation_visible_count(1), 2 * CONVERSATION_PAGE_SIZE);
        assert_eq!(conversation_visible_count(2), 3 * CONVERSATION_PAGE_SIZE);
    }

    fn session_row(title: &str, agent: AgentKind) -> SessionRow {
        SessionRow {
            conversation_id: title.to_string(),
            agent,
            last_ts: 0,
            display_title: title.to_string(),
            summary: None,
            summary_status: None,
            task_id: None,
        }
    }

    #[test]
    fn filter_sessions_empty_query_and_no_agent_returns_all() {
        let rows = vec![
            session_row("修复登录 bug", AgentKind::Claude),
            session_row("重构解析器", AgentKind::Codebuddy),
        ];
        assert_eq!(filter_sessions(&rows, "", None).len(), 2);
    }

    #[test]
    fn filter_sessions_matches_title_case_insensitive_substring() {
        let rows = vec![
            session_row("Fix Login Bug", AgentKind::Claude),
            session_row("重构解析器", AgentKind::Codebuddy),
        ];
        let filtered = filter_sessions(&rows, "login", None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].display_title, "Fix Login Bug");
    }

    #[test]
    fn filter_sessions_matches_summary_case_insensitive_substring() {
        let mut with_summary = session_row("修复登录 bug", AgentKind::Claude);
        with_summary.summary = Some("Rewrote the OAuth callback handler".to_string());
        let rows = vec![
            with_summary,
            session_row("重构解析器", AgentKind::Codebuddy),
        ];
        let filtered = filter_sessions(&rows, "oauth", None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].display_title, "修复登录 bug");
    }

    #[test]
    fn filter_sessions_no_summary_does_not_match_on_summary_query() {
        let rows = vec![session_row("修复登录 bug", AgentKind::Claude)];
        assert!(filter_sessions(&rows, "oauth", None).is_empty());
    }

    #[test]
    fn filter_sessions_filters_by_agent() {
        let rows = vec![
            session_row("会话 A", AgentKind::Claude),
            session_row("会话 B", AgentKind::Codebuddy),
        ];
        let filtered = filter_sessions(&rows, "", Some(AgentKind::Codebuddy));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].display_title, "会话 B");
    }

    #[test]
    fn filter_sessions_combines_query_and_agent() {
        let rows = vec![
            session_row("修复登录 bug", AgentKind::Claude),
            session_row("修复登录 bug", AgentKind::Codebuddy),
        ];
        let filtered = filter_sessions(&rows, "登录", Some(AgentKind::Claude));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].agent, AgentKind::Claude);
    }

    #[test]
    fn filter_sessions_no_match_yields_empty() {
        let rows = vec![session_row("会话 A", AgentKind::Claude)];
        assert!(filter_sessions(&rows, "不存在", None).is_empty());
    }

    #[test]
    fn conversation_agents_present_dedups_and_orders_stably() {
        let rows = vec![
            session_row("a", AgentKind::Opencode),
            session_row("b", AgentKind::Claude),
            session_row("c", AgentKind::Claude),
        ];
        assert_eq!(
            conversation_agents_present(&rows),
            vec![AgentKind::Claude, AgentKind::Opencode]
        );
    }

    #[test]
    fn conversation_agents_present_empty_for_no_rows() {
        assert!(conversation_agents_present(&[]).is_empty());
    }
}
