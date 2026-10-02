//! Todo 内容区 webview 推送协议:把 `WorkspaceState` 算成要序列化推给 webview
//! 的 `TodoViewPayload`,并解析 webview 发回的事件。前端只渲染与保存纯视图
//! 状态,**不做过滤(分类)、状态派生、排序**——这些全在 Rust。
//! 形态仿 `extensions/codehealth/protocol.rs`(声明式比较 + 固定单槽)。

use super::{
    CategoryFilter, Message, TodoState, WorkspaceState, filter_todos_by_category,
    format_todo_month_day, is_active_todo, is_optimistic_id, parse_month_day, todo_display_state,
    visible_category_rows,
};
use crate::workspace::{Workspace, agent_icon};
use dozer_core::protocol::{AgentKind, TodoInfo};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const TODO_PROTOCOL_VERSION: u32 = 1;

/// webview 加载后超过这个时长还没发 `ready` 就判失败,回落原生占位页。
pub const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// 来自 webview 的任务文本长度上限(字符数)。webview 内容不可信,防止异常
/// 超长文本落库。
pub const MAX_TEXT_CHARS: usize = 10_000;

// ---------------------------------------------------------------------------
// Rust → webview
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StateKey {
    Pending,
    InProgress,
    Suspended,
    Done,
}

impl From<TodoState> for StateKey {
    fn from(s: TodoState) -> Self {
        match s {
            TodoState::Pending => StateKey::Pending,
            TodoState::InProgress => StateKey::InProgress,
            TodoState::Suspended => StateKey::Suspended,
            TodoState::Done => StateKey::Done,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKey {
    Active,
    Paused,
    Done,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TodoCardVm {
    pub id: i64,
    pub text: String,
    pub state: StateKey,
    pub segment: SegmentKey,
    /// "MM-DD"
    pub plan_date: Option<String>,
    /// 已完成时的完成日期("MM-DD"或 "-"),否则 `None`。
    pub completed_label: Option<String>,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub assigned_agent: Option<AgentKind>,
    pub has_dispatch: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CategoryRowVm {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub depth: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentVm {
    pub kind: AgentKind,
    pub label: &'static str,
    /// Rust 内嵌的 SVG 原文(可信来源,前端用 innerHTML 渲染)。
    pub icon_svg: String,
    pub preserves_color: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct TodayVm {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TodoViewPayload {
    pub project_id: i64,
    /// `all` / `uncategorized` / `node:<id>`;前端据此在分类切换时重置搜索。
    pub category_key: String,
    /// 已按左栏选中的分类过滤,保持 dozerd 给出的顺序。
    pub items: Vec<TodoCardVm>,
    /// 全展开拍平,供分类选择器按 `depth` 缩进。
    pub categories: Vec<CategoryRowVm>,
    pub agents: Vec<AgentVm>,
    pub add_height_px: f32,
    pub today: TodayVm,
    /// 新增后 2 秒高亮的任务 id(沿用 `Flash` 计时)。
    pub selected_id: Option<i64>,
    /// 每次要求"滚回顶部"时递增。
    pub scroll_nonce: u32,
}

/// 指派选择器里的四个 headless agent(与 `dispatch_items` 一致)。
pub(crate) const DISPATCH_AGENTS: [AgentKind; 4] = [
    AgentKind::Claude,
    AgentKind::Codebuddy,
    AgentKind::Opencode,
    AgentKind::V8agent,
];

pub(crate) fn agent_vms() -> Vec<AgentVm> {
    DISPATCH_AGENTS
        .into_iter()
        .map(|agent| {
            let icon = agent_icon(agent);
            AgentVm {
                kind: agent,
                label: agent.display_label(),
                icon_svg: String::from_utf8_lossy(icon.bytes()).into_owned(),
                preserves_color: icon.preserves_original_color(),
            }
        })
        .collect()
}

/// 每张任务卡片的派生状态(需要看派发的目标 session 是否存活,所以要
/// `Workspace` 而不只是 `WorkspaceState`)。顺序与 `ws.todo.items` 一一对应。
pub(crate) fn derive_states(ws: &Workspace) -> Vec<TodoState> {
    ws.todo
        .items()
        .iter()
        .map(|item| {
            let target_alive = item
                .dispatch_session_id
                .as_ref()
                .map(|sid| {
                    ws.tabs
                        .iter()
                        .chain(ws.ssh_tabs.iter())
                        .any(|t| t.alive && t.info.id == *sid)
                })
                .unwrap_or(false);
            todo_display_state(item, target_alive)
        })
        .collect()
}

pub(crate) fn current_view_payload(
    ws: &Workspace,
    project_id: i64,
    today: (i32, u32, u32),
) -> TodoViewPayload {
    build_payload(&ws.todo, &derive_states(ws), project_id, today, agent_vms())
}

/// 纯函数,便于不构造 `Workspace` 地测试。`states` 与 `todo.items` 等长。
pub(crate) fn build_payload(
    todo: &WorkspaceState,
    states: &[TodoState],
    project_id: i64,
    today: (i32, u32, u32),
    agents: Vec<AgentVm>,
) -> TodoViewPayload {
    let allowed = filter_todos_by_category(&todo.items, &todo.categories, todo.category_selected);
    let category_name = |id: Option<i64>| {
        id.and_then(|cid| todo.categories.iter().find(|c| c.id == cid))
            .map(|c| c.name.clone())
    };
    let items = todo
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| allowed.contains(&item.id))
        .map(|(i, item)| {
            let segment = if item.done {
                SegmentKey::Done
            } else if item.paused {
                SegmentKey::Paused
            } else {
                SegmentKey::Active
            };
            TodoCardVm {
                id: item.id,
                text: item.text.clone(),
                state: states.get(i).copied().unwrap_or(TodoState::Pending).into(),
                segment,
                plan_date: item.plan_date.clone(),
                completed_label: item.done.then(|| {
                    item.completed_at_ms
                        .map(format_todo_month_day)
                        .unwrap_or_else(|| "-".to_string())
                }),
                category_id: item.category_id,
                category_name: category_name(item.category_id),
                assigned_agent: item.assigned_agent,
                has_dispatch: item.dispatch_session_id.is_some(),
            }
        })
        .collect();

    let all_expanded: std::collections::HashSet<i64> =
        todo.categories.iter().map(|c| c.id).collect();
    let categories = visible_category_rows(&todo.categories, &all_expanded)
        .into_iter()
        .map(|row| CategoryRowVm {
            parent_id: todo
                .categories
                .iter()
                .find(|c| c.id == row.id)
                .and_then(|c| c.parent_id),
            id: row.id,
            name: row.name,
            depth: row.depth,
        })
        .collect();

    TodoViewPayload {
        project_id,
        category_key: match todo.category_selected {
            CategoryFilter::All => "all".to_string(),
            CategoryFilter::Uncategorized => "uncategorized".to_string(),
            CategoryFilter::Node(id) => format!("node:{id}"),
        },
        items,
        categories,
        agents,
        add_height_px: todo.add_input_height(),
        today: TodayVm {
            year: today.0,
            month: today.1,
            day: today.2,
        },
        selected_id: todo.selected_id(),
        scroll_nonce: todo.scroll_nonce,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TodoPushEnvelope {
    pub protocol_version: u32,
    /// 单调递增;前端丢弃小于已应用值的推送。
    pub revision: u64,
    pub payload: TodoViewPayload,
}

pub fn encode_todo_push(revision: u64, payload: TodoViewPayload) -> String {
    serde_json::to_string(&TodoPushEnvelope {
        protocol_version: TODO_PROTOCOL_VERSION,
        revision,
        payload,
    })
    .unwrap_or_else(|_| "null".to_string())
}

// ---------------------------------------------------------------------------
// webview → Rust
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SetStatusTarget {
    Pending,
    Suspended,
    Done,
}

impl From<SetStatusTarget> for TodoState {
    fn from(t: SetStatusTarget) -> Self {
        match t {
            SetStatusTarget::Pending => TodoState::Pending,
            SetStatusTarget::Suspended => TodoState::Suspended,
            SetStatusTarget::Done => TodoState::Done,
        }
    }
}

/// webview 发回的事件。webview 内容不可信:文本长度、日期格式、agent 种类、
/// 浮点有限性都在 `route_event` 里校验。
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TodoWebviewEvent {
    Ready,
    Failed { reason: String },
    Add { text: String },
    Toggle { id: i64 },
    EditText { id: i64, text: String },
    Reorder { id: i64, after_id: Option<i64> },
    SetStatus { id: i64, state: SetStatusTarget },
    SetPlanDate { id: i64, date: String },
    AssignAgent { id: i64, agent: AgentKind },
    SetCategory { id: i64, category_id: Option<i64> },
    OpenDetail { id: i64 },
    AddHeight { px: f32 },
}

pub fn parse_todo_event(body: &str) -> Result<TodoWebviewEvent, String> {
    serde_json::from_str(body).map_err(|e| e.to_string())
}

/// `route_event` 的结果:交给 `App::todo_message` 的消息,或 `App` 上现有的
/// 下标式入口。
#[derive(Debug, Clone)]
pub enum Routed {
    Message(Message),
    AssignAgent { idx: usize, agent: AgentKind },
    OpenDetail { idx: usize },
}

fn index_of(items: &[TodoInfo], id: i64) -> Option<usize> {
    // 乐观新增的临时项(id <= 0)还不是真实任务:任何事件都不能落到它上面。
    if is_optimistic_id(id) {
        return None;
    }
    items.iter().position(|i| i.id == id)
}

fn clean_text(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty() && t.chars().count() <= MAX_TEXT_CHARS).then(|| t.to_string())
}

/// 把 webview 事件的 `id` 映射成**当前**下标,并校验入参,再复用现有的下标式
/// 消息与更新逻辑。`id` 已不存在(任务被别的 agent 删了)或入参非法一律返回
/// `None`(空操作),绝不能落到别的行。`Ready`/`Failed` 由 `App` 自己处理,这里
/// 返回 `None`。
pub(crate) fn route_event(items: &[TodoInfo], event: TodoWebviewEvent) -> Option<Routed> {
    use TodoWebviewEvent as Ev;
    match event {
        Ev::Ready | Ev::Failed { .. } => None,
        Ev::Add { text } => Some(Routed::Message(Message::AddText(clean_text(&text)?))),
        Ev::Toggle { id } => Some(Routed::Message(Message::Toggle(index_of(items, id)?))),
        Ev::EditText { id, text } => {
            index_of(items, id)?;
            Some(Routed::Message(Message::EditText(id, clean_text(&text)?)))
        }
        Ev::Reorder { id, after_id } => {
            let idx = index_of(items, id)?;
            if !is_active_todo(&items[idx]) || after_id == Some(id) {
                return None;
            }
            // `after_id` 必须仍然存在且是进行中的真实任务;否则 dozerd 会把卡片静默
            // 挪到末尾——放置期间那张卡被别的 agent 删掉 / 完成时应当空操作。
            if let Some(after) = after_id {
                let after_idx = index_of(items, after)?;
                if !is_active_todo(&items[after_idx]) {
                    return None;
                }
            }
            Some(Routed::Message(Message::ReorderTo { id, after_id }))
        }
        Ev::SetStatus { id, state } => {
            let idx = index_of(items, id)?;
            Some(Routed::Message(Message::StatusPick(idx, state.into())))
        }
        Ev::SetPlanDate { id, date } => {
            let idx = index_of(items, id)?;
            let (m, d) = parse_month_day(&date)?;
            if !(1..=31).contains(&d) {
                return None;
            }
            Some(Routed::Message(Message::CalendarPick(
                idx,
                format!("{m:02}-{d:02}"),
            )))
        }
        Ev::AssignAgent { id, agent } => {
            let idx = index_of(items, id)?;
            DISPATCH_AGENTS
                .contains(&agent)
                .then_some(Routed::AssignAgent { idx, agent })
        }
        Ev::SetCategory { id, category_id } => {
            index_of(items, id)?;
            Some(Routed::Message(Message::SetCategory(id, category_id)))
        }
        Ev::OpenDetail { id } => Some(Routed::OpenDetail {
            idx: index_of(items, id)?,
        }),
        Ev::AddHeight { px } => px
            .is_finite()
            .then_some(Routed::Message(Message::AddHeight(px))),
    }
}

// ---------------------------------------------------------------------------
// 声明式推送状态(同 codehealth::WebviewPushState)
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct WebviewPushState {
    ready: bool,
    last_sent: Option<TodoViewPayload>,
    revision: u64,
    /// webview 已进池但尚未 ready 的起点,用于判定加载超时。
    waiting_since: Option<Instant>,
    /// 加载失败/超时原因;`Some` 时内容区回落原生占位页(重试清除)。
    failed: Option<String>,
}

impl WebviewPushState {
    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
        if ready {
            // 新实例(或重新确认 ready)一律强制重发当前内容。
            self.last_sent = None;
            self.waiting_since = None;
        }
    }

    /// 每帧由消费点调用:`available` = webview 当前在池里。
    pub fn observe_availability(&mut self, available: bool, now: Instant) {
        if !available {
            self.ready = false;
            self.waiting_since = None;
            return;
        }
        if self.ready {
            self.waiting_since = None;
            return;
        }
        let since = *self.waiting_since.get_or_insert(now);
        if self.failed.is_none() && now.duration_since(since) > READY_TIMEOUT {
            self.failed = Some("面板页面加载超时".to_string());
        }
    }

    pub fn pending_push(&self, desired: &TodoViewPayload) -> Option<TodoViewPayload> {
        if !self.ready || self.last_sent.as_ref() == Some(desired) {
            return None;
        }
        Some(desired.clone())
    }

    /// 记录已送达并返回本次 revision(从 1 起)。
    pub fn mark_sent(&mut self, payload: TodoViewPayload) -> u64 {
        self.revision += 1;
        self.last_sent = Some(payload);
        self.revision
    }

    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    pub fn set_failed(&mut self, reason: String) {
        self.failed = Some(reason);
        self.ready = false;
        self.last_sent = None;
        self.waiting_since = None;
    }

    pub fn clear_failed(&mut self) {
        self.failed = None;
        self.waiting_since = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::todo::{CategoryFilter, Message};
    use dozer_core::protocol::CategoryInfo;
    use serde_json::json;

    fn todo(id: i64, text: &str) -> TodoInfo {
        TodoInfo {
            id,
            project_id: 1,
            text: text.to_string(),
            done: false,
            paused: false,
            rank: id,
            created_ms: 0,
            completed_at_ms: None,
            plan_date: None,
            dispatch_session_id: None,
            dispatch_at_ms: None,
            category_id: None,
            assigned_agent: None,
        }
    }

    fn category(id: i64, parent: Option<i64>, name: &str) -> CategoryInfo {
        serde_json::from_value(json!({
            "id": id, "project_id": 1, "parent_id": parent, "name": name,
            "rank": id, "created_ms": 0
        }))
        .expect("CategoryInfo")
    }

    fn state_with(items: Vec<TodoInfo>) -> WorkspaceState {
        WorkspaceState {
            items,
            ..WorkspaceState::default()
        }
    }

    // ---- payload ----

    #[test]
    fn payload_filters_by_selected_category_in_original_order() {
        let mut a = todo(1, "A");
        a.category_id = Some(10);
        let b = todo(2, "B");
        let mut c = todo(3, "C");
        c.category_id = Some(11); // 10 的子分类
        let mut ws = state_with(vec![a, b, c]);
        ws.categories = vec![category(10, None, "后端"), category(11, Some(10), "接口")];
        ws.category_selected = CategoryFilter::Node(10);
        let states = vec![TodoState::Pending; 3];
        let p = build_payload(&ws, &states, 7, (2026, 10, 2), vec![]);
        assert_eq!(p.project_id, 7);
        assert_eq!(p.category_key, "node:10");
        assert_eq!(p.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![1, 3]);
        assert_eq!(p.items[0].category_name.as_deref(), Some("后端"));
    }

    #[test]
    fn payload_marks_segments_and_uses_given_states() {
        let mut paused = todo(2, "搁置");
        paused.paused = true;
        let mut done = todo(3, "完成");
        done.done = true;
        done.completed_at_ms = Some(0);
        let ws = state_with(vec![todo(1, "进行"), paused, done]);
        let states = vec![TodoState::InProgress, TodoState::Suspended, TodoState::Done];
        let p = build_payload(&ws, &states, 1, (2026, 10, 2), vec![]);
        assert_eq!(p.items[0].segment, SegmentKey::Active);
        assert_eq!(p.items[1].segment, SegmentKey::Paused);
        assert_eq!(p.items[2].segment, SegmentKey::Done);
        assert_eq!(p.items[0].state, StateKey::InProgress);
        assert!(p.items[2].completed_label.is_some());
        assert!(p.items[0].completed_label.is_none());
    }

    #[test]
    fn payload_category_key_covers_all_filters() {
        let mut ws = state_with(vec![]);
        let key =
            |ws: &WorkspaceState| build_payload(ws, &[], 1, (2026, 10, 2), vec![]).category_key;
        ws.category_selected = CategoryFilter::All;
        assert_eq!(key(&ws), "all");
        ws.category_selected = CategoryFilter::Uncategorized;
        assert_eq!(key(&ws), "uncategorized");
        ws.category_selected = CategoryFilter::Node(42);
        assert_eq!(key(&ws), "node:42");
    }

    #[test]
    fn payload_flattens_categories_with_depth_and_parent() {
        let mut ws = state_with(vec![]);
        ws.categories = vec![category(10, None, "后端"), category(11, Some(10), "接口")];
        let p = build_payload(&ws, &[], 1, (2026, 10, 2), vec![]);
        assert_eq!(p.categories.len(), 2);
        assert_eq!(
            (
                p.categories[1].id,
                p.categories[1].depth,
                p.categories[1].parent_id
            ),
            (11, 1, Some(10))
        );
    }

    #[test]
    fn payload_carries_selected_id_scroll_nonce_and_height() {
        let mut ws = state_with(vec![todo(5, "新"), todo(6, "旧")]);
        ws.start_flash(0);
        let p = build_payload(&ws, &[TodoState::Pending; 2], 1, (2026, 10, 2), vec![]);
        assert_eq!(p.selected_id, Some(5));
        assert_eq!(p.scroll_nonce, 1);
        assert!(p.add_height_px > 0.0);
    }

    /// 字段名一旦改动就会破坏前端,用快照钉住形状。
    #[test]
    fn payload_serializes_stable_shape() {
        let mut item = todo(1, "写文档");
        item.plan_date = Some("10-05".into());
        let ws = state_with(vec![item]);
        let p = build_payload(&ws, &[TodoState::Pending], 3, (2026, 10, 2), vec![]);
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["project_id"], 3);
        assert_eq!(v["category_key"], "all");
        assert_eq!(v["today"], json!({"year": 2026, "month": 10, "day": 2}));
        assert_eq!(
            v["items"][0],
            json!({
                "id": 1, "text": "写文档", "state": "pending", "segment": "active",
                "plan_date": "10-05", "completed_label": null, "category_id": null,
                "category_name": null, "assigned_agent": null, "has_dispatch": false
            })
        );
        for key in [
            "categories",
            "agents",
            "add_height_px",
            "selected_id",
            "scroll_nonce",
        ] {
            assert!(v.get(key).is_some(), "缺字段 {key}");
        }
    }

    #[test]
    fn agents_are_the_four_headless_kinds_with_inline_svg() {
        let agents = agent_vms();
        let kinds: Vec<_> = agents.iter().map(|a| a.kind).collect();
        assert_eq!(
            kinds,
            vec![
                AgentKind::Claude,
                AgentKind::Codebuddy,
                AgentKind::Opencode,
                AgentKind::V8agent
            ]
        );
        assert!(agents.iter().all(|a| a.icon_svg.contains("<svg")));
    }

    // ---- events ----

    #[test]
    fn parses_every_event_kind() {
        let cases = [
            (r#"{"kind":"ready"}"#, TodoWebviewEvent::Ready),
            (
                r#"{"kind":"failed","reason":"x"}"#,
                TodoWebviewEvent::Failed { reason: "x".into() },
            ),
            (
                r#"{"kind":"add","text":"a"}"#,
                TodoWebviewEvent::Add { text: "a".into() },
            ),
            (
                r#"{"kind":"toggle","id":3}"#,
                TodoWebviewEvent::Toggle { id: 3 },
            ),
            (
                r#"{"kind":"edit_text","id":3,"text":"b"}"#,
                TodoWebviewEvent::EditText {
                    id: 3,
                    text: "b".into(),
                },
            ),
            (
                r#"{"kind":"reorder","id":3,"after_id":null}"#,
                TodoWebviewEvent::Reorder {
                    id: 3,
                    after_id: None,
                },
            ),
            (
                r#"{"kind":"reorder","id":3,"after_id":2}"#,
                TodoWebviewEvent::Reorder {
                    id: 3,
                    after_id: Some(2),
                },
            ),
            (
                r#"{"kind":"set_status","id":3,"state":"suspended"}"#,
                TodoWebviewEvent::SetStatus {
                    id: 3,
                    state: SetStatusTarget::Suspended,
                },
            ),
            (
                r#"{"kind":"set_plan_date","id":3,"date":"10-05"}"#,
                TodoWebviewEvent::SetPlanDate {
                    id: 3,
                    date: "10-05".into(),
                },
            ),
            (
                r#"{"kind":"assign_agent","id":3,"agent":"claude"}"#,
                TodoWebviewEvent::AssignAgent {
                    id: 3,
                    agent: AgentKind::Claude,
                },
            ),
            (
                r#"{"kind":"set_category","id":3,"category_id":null}"#,
                TodoWebviewEvent::SetCategory {
                    id: 3,
                    category_id: None,
                },
            ),
            (
                r#"{"kind":"open_detail","id":3}"#,
                TodoWebviewEvent::OpenDetail { id: 3 },
            ),
            (
                r#"{"kind":"add_height","px":120.0}"#,
                TodoWebviewEvent::AddHeight { px: 120.0 },
            ),
        ];
        for (body, expected) in cases {
            assert_eq!(parse_todo_event(body).unwrap(), expected, "{body}");
        }
    }

    #[test]
    fn rejects_unknown_kind_and_garbage() {
        assert!(parse_todo_event(r#"{"kind":"nope"}"#).is_err());
        assert!(parse_todo_event("not json").is_err());
    }

    // ---- routing ----

    fn items3() -> Vec<TodoInfo> {
        let mut paused = todo(2, "搁置");
        paused.paused = true;
        let mut done = todo(3, "完成");
        done.done = true;
        vec![todo(1, "进行"), paused, done]
    }

    // Review Focus 1:过期 id 必须空操作,绝不能落到别的行。
    #[test]
    fn route_event_unknown_id_is_noop() {
        let items = items3();
        for ev in [
            TodoWebviewEvent::Toggle { id: 999 },
            TodoWebviewEvent::EditText {
                id: 999,
                text: "x".into(),
            },
            TodoWebviewEvent::Reorder {
                id: 999,
                after_id: None,
            },
            TodoWebviewEvent::SetStatus {
                id: 999,
                state: SetStatusTarget::Done,
            },
            TodoWebviewEvent::SetPlanDate {
                id: 999,
                date: "10-05".into(),
            },
            TodoWebviewEvent::AssignAgent {
                id: 999,
                agent: AgentKind::Claude,
            },
            TodoWebviewEvent::SetCategory {
                id: 999,
                category_id: None,
            },
            TodoWebviewEvent::OpenDetail { id: 999 },
        ] {
            assert!(route_event(&items, ev.clone()).is_none(), "{ev:?}");
        }
    }

    #[test]
    fn route_event_maps_id_to_current_index() {
        let items = items3();
        let r = route_event(&items, TodoWebviewEvent::Toggle { id: 3 }).unwrap();
        assert!(matches!(r, Routed::Message(Message::Toggle(2))));
        let r = route_event(&items, TodoWebviewEvent::OpenDetail { id: 2 }).unwrap();
        assert!(matches!(r, Routed::OpenDetail { idx: 1 }));
        let r = route_event(
            &items,
            TodoWebviewEvent::SetStatus {
                id: 1,
                state: SetStatusTarget::Done,
            },
        )
        .unwrap();
        assert!(matches!(
            r,
            Routed::Message(Message::StatusPick(0, TodoState::Done))
        ));
    }

    #[test]
    fn route_add_trims_and_rejects_empty_and_oversized() {
        let items = items3();
        let r = route_event(
            &items,
            TodoWebviewEvent::Add {
                text: "  新任务\n".into(),
            },
        )
        .unwrap();
        assert!(matches!(r, Routed::Message(Message::AddText(ref t)) if t == "新任务"));
        assert!(route_event(&items, TodoWebviewEvent::Add { text: "   ".into() }).is_none());
        let huge = "字".repeat(MAX_TEXT_CHARS + 1);
        assert!(route_event(&items, TodoWebviewEvent::Add { text: huge.clone() }).is_none());
        assert!(route_event(&items, TodoWebviewEvent::EditText { id: 1, text: huge }).is_none());
    }

    #[test]
    fn route_set_plan_date_validates_month_day() {
        let items = items3();
        let ok = route_event(
            &items,
            TodoWebviewEvent::SetPlanDate {
                id: 1,
                date: "9-5".into(),
            },
        );
        // 规范化成 MM-DD
        assert!(
            matches!(ok, Some(Routed::Message(Message::CalendarPick(0, ref d))) if d == "09-05")
        );
        for bad in ["13-01", "00-10", "10-32", "abc", "10-00", ""] {
            assert!(
                route_event(
                    &items,
                    TodoWebviewEvent::SetPlanDate {
                        id: 1,
                        date: bad.into()
                    }
                )
                .is_none(),
                "{bad}"
            );
        }
    }

    #[test]
    fn route_assign_agent_only_accepts_the_four_headless_kinds() {
        let items = items3();
        let ok = route_event(
            &items,
            TodoWebviewEvent::AssignAgent {
                id: 1,
                agent: AgentKind::V8agent,
            },
        );
        assert!(matches!(
            ok,
            Some(Routed::AssignAgent {
                idx: 0,
                agent: AgentKind::V8agent
            })
        ));
        for bad in [
            AgentKind::Codex,
            AgentKind::Goose,
            AgentKind::Aider,
            AgentKind::Unknown,
        ] {
            assert!(
                route_event(&items, TodoWebviewEvent::AssignAgent { id: 1, agent: bad }).is_none()
            );
        }
    }

    #[test]
    fn route_reorder_only_for_active_items_and_not_onto_itself() {
        let items = items3();
        let ok = route_event(
            &items,
            TodoWebviewEvent::Reorder {
                id: 1,
                after_id: None,
            },
        );
        assert!(matches!(
            ok,
            Some(Routed::Message(Message::ReorderTo {
                id: 1,
                after_id: None
            }))
        ));
        // 搁置、已完成不可拖
        assert!(
            route_event(
                &items,
                TodoWebviewEvent::Reorder {
                    id: 2,
                    after_id: None
                }
            )
            .is_none()
        );
        assert!(
            route_event(
                &items,
                TodoWebviewEvent::Reorder {
                    id: 3,
                    after_id: None
                }
            )
            .is_none()
        );
        // 挪到自己之后无意义
        assert!(
            route_event(
                &items,
                TodoWebviewEvent::Reorder {
                    id: 1,
                    after_id: Some(1)
                }
            )
            .is_none()
        );
    }

    // 审阅 Important 3:放置位置上方的那张卡已被删掉 / 不再是进行中,必须空操作,
    // 不能让 dozerd 把卡片静默挪到末尾。
    #[test]
    fn route_reorder_with_stale_or_inactive_after_id_is_noop() {
        let items = items3(); // 1 进行 / 2 搁置 / 3 已完成
        for after in [999, 2, 3] {
            assert!(
                route_event(
                    &items,
                    TodoWebviewEvent::Reorder {
                        id: 1,
                        after_id: Some(after)
                    }
                )
                .is_none(),
                "after_id={after}"
            );
        }
    }

    #[test]
    fn route_reorder_after_another_active_item_is_still_routed() {
        let mut items = items3();
        items.push(todo(4, "又一个进行中"));
        let r = route_event(
            &items,
            TodoWebviewEvent::Reorder {
                id: 1,
                after_id: Some(4),
            },
        );
        assert!(matches!(
            r,
            Some(Routed::Message(Message::ReorderTo {
                id: 1,
                after_id: Some(4)
            }))
        ));
    }

    // 审阅 Important 4:乐观新增的临时项(id <= 0)不是真实任务,任何事件都不能落到它上面,
    // 也不能当作 `after_id`。
    #[test]
    fn optimistic_items_are_never_routed() {
        for temp in [0_i64, -1, -7] {
            let mut items = items3();
            items.insert(0, todo(temp, "乐观新增"));
            for ev in [
                TodoWebviewEvent::Toggle { id: temp },
                TodoWebviewEvent::EditText {
                    id: temp,
                    text: "x".into(),
                },
                TodoWebviewEvent::Reorder {
                    id: temp,
                    after_id: None,
                },
                TodoWebviewEvent::SetStatus {
                    id: temp,
                    state: SetStatusTarget::Done,
                },
                TodoWebviewEvent::SetPlanDate {
                    id: temp,
                    date: "10-05".into(),
                },
                TodoWebviewEvent::AssignAgent {
                    id: temp,
                    agent: AgentKind::Claude,
                },
                TodoWebviewEvent::SetCategory {
                    id: temp,
                    category_id: None,
                },
                TodoWebviewEvent::OpenDetail { id: temp },
            ] {
                assert!(
                    route_event(&items, ev.clone()).is_none(),
                    "temp={temp} {ev:?}"
                );
            }
            assert!(
                route_event(
                    &items,
                    TodoWebviewEvent::Reorder {
                        id: 1,
                        after_id: Some(temp)
                    }
                )
                .is_none(),
                "temp={temp} 不能当 after_id"
            );
        }
    }

    #[test]
    fn route_add_height_requires_finite() {
        let items = items3();
        assert!(matches!(
            route_event(&items, TodoWebviewEvent::AddHeight { px: 90.0 }),
            Some(Routed::Message(Message::AddHeight(_)))
        ));
        assert!(route_event(&items, TodoWebviewEvent::AddHeight { px: f32::NAN }).is_none());
        assert!(route_event(&items, TodoWebviewEvent::AddHeight { px: f32::INFINITY }).is_none());
    }

    #[test]
    fn route_ready_and_failed_are_handled_by_the_app_not_routed() {
        let items = items3();
        assert!(route_event(&items, TodoWebviewEvent::Ready).is_none());
        assert!(route_event(&items, TodoWebviewEvent::Failed { reason: "x".into() }).is_none());
    }

    // ---- push state ----

    fn payload(project_id: i64) -> TodoViewPayload {
        build_payload(&state_with(vec![]), &[], project_id, (2026, 10, 2), vec![])
    }

    #[test]
    fn push_state_waits_for_ready_then_sends_once() {
        let mut s = WebviewPushState::default();
        let p = payload(1);
        assert!(s.pending_push(&p).is_none(), "未 ready 不推送");
        s.set_ready(true);
        let to_send = s.pending_push(&p).expect("ready 后应推送");
        assert_eq!(s.mark_sent(to_send), 1);
        assert!(s.pending_push(&p).is_none(), "内容没变不重复推送");
        assert!(
            s.pending_push(&payload(2)).is_some(),
            "内容变了(换项目)要推送"
        );
    }

    // Review Focus 4:webview 被重建后 ready 会强制重发当前内容。
    #[test]
    fn push_state_set_ready_forces_resend() {
        let mut s = WebviewPushState::default();
        let p = payload(1);
        s.set_ready(true);
        let first = s.pending_push(&p).unwrap();
        s.mark_sent(first);
        s.set_ready(true);
        assert!(s.pending_push(&p).is_some());
        assert_eq!(s.mark_sent(p.clone()), 2, "revision 单调递增");
    }

    #[test]
    fn push_state_times_out_when_never_ready_and_blocks_push() {
        let mut s = WebviewPushState::default();
        let t0 = Instant::now();
        s.observe_availability(true, t0);
        assert!(s.failed().is_none());
        s.observe_availability(true, t0 + READY_TIMEOUT + Duration::from_secs(1));
        assert!(s.failed().is_some());
        s.set_ready(true);
        s.clear_failed();
        assert!(s.failed().is_none());
    }

    #[test]
    fn push_state_unavailable_resets_ready() {
        let mut s = WebviewPushState::default();
        s.set_ready(true);
        s.observe_availability(false, Instant::now());
        assert!(s.pending_push(&payload(1)).is_none());
    }

    #[test]
    fn encode_push_wraps_revision_and_version() {
        let out = encode_todo_push(5, payload(9));
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["protocol_version"], TODO_PROTOCOL_VERSION);
        assert_eq!(v["revision"], 5);
        assert_eq!(v["payload"]["project_id"], 9);
    }
}
