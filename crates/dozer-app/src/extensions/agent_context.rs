//! Agent 上下文列表(v0.1 意向文档 §10 Context Scope):终端下方的一条,列出
//! human 显式送进来的文件/目录。数据权威在 dozerd(`agent_context_items` 表),
//! 本模块只持有每项目一份的展示状态。设计见
//! `docs/superpowers/specs/2026-09-30-agent-native-file-editor-phase3-context-history-design.md`。

use std::path::PathBuf;

use byteui::interaction::icons;
use dozer_core::protocol::ContextItemInfo;
use iced_widget::core::{Alignment, Element, Length};
use iced_widget::{button, column, container, row, scrollable, text};

/// 条的固定高度。折叠/展开都不改变它:展开的列表用 `stack!` 浮在终端底部
/// 之上,不推挤终端,所以 PTY 网格不会因折叠/展开重算(见 plan「有意偏差 1」)。
pub const STRIP_HEIGHT: f32 = 28.0;
/// 展开列表的最大高度,超出滚动。
const EXPANDED_MAX_HEIGHT: f32 = 168.0;

/// 终端几何要为条预留的总高度 = 条高 + 终端 pane column 的子元素间距。
/// `terminal_pane_pixel_size` 与 `terminal_pane` 渲染共用这一个值。
pub fn strip_reserved_height() -> f32 {
    STRIP_HEIGHT + crate::theme::region::terminal_pane().gap
}

/// 一项上下文 + 它在磁盘上是否已不存在(刷新时在后台线程判定,视图层不读盘)。
#[derive(Debug, Clone, PartialEq)]
pub struct ContextEntry {
    pub info: ContextItemInfo,
    pub missing: bool,
}

#[derive(Debug, Default)]
pub struct State {
    items: Vec<ContextEntry>,
    expanded: bool,
    notice: Option<String>,
}

impl State {
    pub fn items(&self) -> &[ContextEntry] {
        &self.items
    }
    pub fn expanded(&self) -> bool {
        self.expanded
    }
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }
    pub fn entry(&self, id: i64) -> Option<&ContextEntry> {
        self.items.iter().find(|e| e.info.id == id)
    }
}

/// 所有消息都由 `App` 以 `Message::AgentContext(project_id, msg)` 信封携带
/// 项目 id 路由到对应 `Workspace`——异步结果不会写进"当前聚焦项目"。
#[derive(Debug, Clone)]
pub enum Message {
    Loaded(Result<Vec<ContextEntry>, String>),
    ToggleExpanded,
    Remove(i64),
    Added(Result<ContextItemInfo, String>),
    Removed(Result<(), String>),
    /// 点某一项:App 拦截(需要预览/文件树句柄),不进 `apply`。
    Open(i64),
    /// 打开修改历史弹窗:`None` = 全部;`Some(item_id)` = 预过滤到该项。App 拦截。
    OpenHistory(Option<i64>),
    DismissNotice,
}

/// `apply` 之后调用方还要做的 IO。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followup {
    None,
    Refresh,
    RemoveItem(i64),
}

/// 纯状态转移(可单测);IO 由 `update` 按 `Followup` 执行。
pub fn apply(state: &mut State, msg: Message) -> Followup {
    match msg {
        Message::Loaded(Ok(items)) => {
            state.items = items;
            Followup::None
        }
        Message::Loaded(Err(e)) => {
            state.notice = Some(format!("加载上下文列表失败: {e}"));
            Followup::None
        }
        Message::ToggleExpanded => {
            state.expanded = !state.expanded;
            Followup::None
        }
        Message::Remove(id) => Followup::RemoveItem(id),
        Message::Removed(res) => {
            if let Err(e) = res {
                state.notice = Some(format!("移除失败: {e}"));
            }
            Followup::Refresh
        }
        Message::Added(res) => {
            if let Err(e) = res {
                // 落库失败时终端粘贴已经发生(见 Task 5),这里如实告知。
                state.notice = Some(format!("已发送到终端,但未能记录到上下文列表: {e}"));
            }
            Followup::Refresh
        }
        Message::Open(_) | Message::OpenHistory(_) => Followup::None,
        Message::DismissNotice => {
            state.notice = None;
            Followup::None
        }
    }
}

/// 拉取列表并在后台线程判定每项是否已缺失,完成后 `emit(Loaded)`。
pub fn request_refresh(
    project_id: i64,
    root: PathBuf,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let res = match client.list_context_items(project_id).await {
            Err(e) => Err(e.to_string()),
            Ok(items) => tokio::task::spawn_blocking(move || {
                items
                    .into_iter()
                    .map(|info| {
                        let missing = !root.join(&info.entity_ref).exists();
                        ContextEntry { info, missing }
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .map_err(|e| e.to_string()),
        };
        emit(Message::Loaded(res));
    });
}

/// 把文件/目录加入项目上下文,完成后 `emit(Added)`。`relative` 不带尾部 `/`。
pub fn request_add(
    project_id: i64,
    is_dir: bool,
    relative: String,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    let kind = if is_dir { "dir" } else { "file" };
    handle.spawn(async move {
        let res = client
            .add_context_item(project_id, kind, &relative)
            .await
            .map_err(|e| e.to_string());
        emit(Message::Added(res));
    });
}

/// 执行 `apply` 并按 `Followup` 做 IO。`emit` 需 `Clone`(Refresh 与
/// RemoveItem 各自 spawn 一次);`App` 里的 `emit` 只捕获 `proxy`(Clone)与
/// `project_id`(Copy),天然满足。
pub fn update(
    state: &mut State,
    project_id: i64,
    root: PathBuf,
    msg: Message,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Clone + Send + 'static,
) {
    match apply(state, msg) {
        Followup::None => {}
        Followup::Refresh => request_refresh(project_id, root, client, handle, emit),
        Followup::RemoveItem(id) => {
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .remove_context_item(project_id, id)
                    .await
                    .map_err(|e| e.to_string());
                emit(Message::Removed(res));
            });
        }
    }
}

fn flat_button(
    color: iced_widget::core::Color,
) -> impl Fn(&iced_widget::Theme, button::Status) -> button::Style {
    move |_t, _s| button::Style {
        background: None,
        text_color: color,
        ..button::Style::default()
    }
}

/// 终端下方恒定高度的一条:`Agent 上下文 · N 项`(点击展开/收起) + 提示 +
/// `修改历史`。字体走系统默认(非代码/终端场景)。
pub fn strip(state: &State) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let arrow = if state.expanded {
        icons::IconKind::ChevronDown
    } else {
        icons::IconKind::ChevronUp
    };
    let toggle = button(
        row![
            icons::view(arrow, byteui::theme::icon_size::row(), colors.dim),
            text(format!("Agent 上下文 · {} 项", state.items.len()))
                .size(byteui::theme::font::label())
                .color(colors.cream),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(0)
    .on_press(Message::ToggleExpanded)
    .style(flat_button(colors.cream));

    let mut bar = row![toggle].spacing(12).align_y(Alignment::Center);
    if let Some(n) = state.notice() {
        bar = bar.push(
            button(
                text(n.to_string())
                    .size(byteui::theme::font::label())
                    .color(colors.red),
            )
            .padding(0)
            .on_press(Message::DismissNotice)
            .style(flat_button(colors.red)),
        );
    }
    bar = bar.push(iced_widget::space::horizontal()).push(
        button(
            text("修改历史")
                .size(byteui::theme::font::label())
                .color(colors.gold),
        )
        .padding(0)
        .on_press(Message::OpenHistory(None))
        .style(flat_button(colors.gold)),
    );
    container(bar)
        .width(Length::Fill)
        .height(Length::Fixed(STRIP_HEIGHT))
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

/// 展开态的逐项列表(折叠、或列表为空时返回 `None`)。由 `terminal_pane` 用
/// `stack!` 浮在终端底部、条的正上方。缺失项灰显并标"缺失"。
pub fn expanded_panel(
    state: &State,
) -> Option<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    if !state.expanded || state.items.is_empty() {
        return None;
    }
    let colors = byteui::theme::color::current();
    let mut list = column![].spacing(4);
    for e in &state.items {
        let kind_icon = if e.info.entity_kind == "dir" {
            icons::IconKind::Folder
        } else {
            icons::IconKind::FileText
        };
        let label_color = if e.missing { colors.dim } else { colors.cream };
        let label = if e.missing {
            format!("{}(缺失)", e.info.entity_ref)
        } else {
            e.info.entity_ref.clone()
        };
        let mut name = button(
            text(label)
                .size(byteui::theme::font::label())
                .color(label_color),
        )
        .padding(0)
        .width(Length::Fill)
        .style(flat_button(label_color));
        if !e.missing {
            name = name.on_press(Message::Open(e.info.id));
        }
        list = list.push(
            row![
                icons::view(kind_icon, byteui::theme::icon_size::row(), label_color),
                name,
                button(
                    text("历史")
                        .size(byteui::theme::font::label())
                        .color(colors.gold)
                )
                .padding(0)
                .on_press(Message::OpenHistory(Some(e.info.id)))
                .style(flat_button(colors.gold)),
                button(
                    text("移除")
                        .size(byteui::theme::font::label())
                        .color(colors.dim)
                )
                .padding(0)
                .on_press(Message::Remove(e.info.id))
                .style(flat_button(colors.dim)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }
    Some(
        container(scrollable(list))
            .width(Length::Fill)
            .max_height(EXPANDED_MAX_HEIGHT)
            .padding(8)
            .style(crate::dialog::card_style)
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i64, r: &str, missing: bool) -> ContextEntry {
        ContextEntry {
            info: ContextItemInfo {
                id,
                project_id: 1,
                entity_kind: "file".into(),
                entity_ref: r.into(),
                created_ms: id as u64,
            },
            missing,
        }
    }

    #[test]
    fn loaded_ok_replaces_items_and_err_sets_notice() {
        let mut s = State::default();
        assert_eq!(
            apply(&mut s, Message::Loaded(Ok(vec![entry(1, "a", false)]))),
            Followup::None
        );
        assert_eq!(s.items().len(), 1);
        apply(&mut s, Message::Loaded(Err("boom".into())));
        assert!(s.notice().unwrap().contains("boom"));
        assert_eq!(s.items().len(), 1, "加载失败不清空已有列表");
    }

    #[test]
    fn toggle_flips_expanded() {
        let mut s = State::default();
        assert!(!s.expanded());
        apply(&mut s, Message::ToggleExpanded);
        assert!(s.expanded());
        apply(&mut s, Message::ToggleExpanded);
        assert!(!s.expanded());
    }

    #[test]
    fn remove_requests_io_and_removed_refreshes() {
        let mut s = State::default();
        assert_eq!(apply(&mut s, Message::Remove(5)), Followup::RemoveItem(5));
        assert_eq!(apply(&mut s, Message::Removed(Ok(()))), Followup::Refresh);
        assert_eq!(
            apply(&mut s, Message::Removed(Err("x".into()))),
            Followup::Refresh
        );
        assert!(s.notice().unwrap().contains("x"));
    }

    #[test]
    fn added_failure_still_refreshes_and_tells_the_user_it_was_pasted() {
        let mut s = State::default();
        let info = ContextItemInfo {
            id: 1,
            project_id: 1,
            entity_kind: "file".into(),
            entity_ref: "a".into(),
            created_ms: 1,
        };
        assert_eq!(apply(&mut s, Message::Added(Ok(info))), Followup::Refresh);
        assert!(s.notice().is_none());
        assert_eq!(
            apply(&mut s, Message::Added(Err("db 挂了".into()))),
            Followup::Refresh
        );
        let n = s.notice().unwrap();
        assert!(n.contains("已发送到终端") && n.contains("db 挂了"), "{n}");
    }

    #[test]
    fn open_messages_are_not_handled_here_and_dismiss_clears_notice() {
        let mut s = State::default();
        assert_eq!(apply(&mut s, Message::Open(1)), Followup::None);
        assert_eq!(apply(&mut s, Message::OpenHistory(None)), Followup::None);
        apply(&mut s, Message::Loaded(Err("e".into())));
        apply(&mut s, Message::DismissNotice);
        assert!(s.notice().is_none());
    }

    #[test]
    fn entry_lookup_and_reserved_height() {
        let mut s = State::default();
        apply(
            &mut s,
            Message::Loaded(Ok(vec![entry(1, "a", false), entry(2, "b", true)])),
        );
        assert!(s.entry(2).unwrap().missing);
        assert!(s.entry(9).is_none());
        assert!(
            strip_reserved_height() > STRIP_HEIGHT,
            "必须包含 column 间距"
        );
    }
}
