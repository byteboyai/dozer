//! `.dozer/todo.md` 任务列表解析（Todo 面板 design，2026-08-06；已并入
//! `extensions::todo`）：
//! 标准两态 checkbox（`- [ ]`/`- [x]`），git 可追踪，agent 可直接读写。
//! 解析风格镜像 `goal.rs`——手写、宽松，不引入 markdown 库；格式意外
//! （多级缩进、非 checkbox 正文）一律忽略，不因为文件"长得不标准"而失败。

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::{icons, theme};
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{button, column, container, rich_text, row, scrollable, span, text, text_input};

#[derive(Debug, Clone, PartialEq)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
}

pub fn todo_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("todo.md")
}

/// 宽松解析：只认一级 `- [ ]`/`- [x]` 列表项（`trim` 后必须以这两个前缀
/// 之一开头——多级缩进的子项 `trim` 后前导空格会被吃掉，但因为前面还有
/// `- [ ]` 的兄弟节点占了行首，不会被误判成一级项，见 `parses_pending_
/// and_done_items` 与 `ignores_non_checkbox_lines_and_blank_file` 两个
/// 测试）；其余行（标题、正文）一律忽略，不因为格式意外而失败。文件不
/// 存在/为空 → 空列表，不是 `Option`（跟 `goal.rs::parse_goal` 不同——
/// todo 没有"整份文件代表一个目标"这种要么有要么没有的语义）。
pub fn parse_todo(md: &str) -> Vec<TodoItem> {
    let mut items = Vec::new();
    for line in md.lines() {
        // 只有顶格（列 0）的 `- [ ]`/`- [x]` 才算 Dozer 面板的一级任务；
        // 缩进的子任务属于 agent 自己的清单，不归面板管（见 design）。
        if let Some(rest) = line.strip_prefix("- [ ]") {
            items.push(TodoItem {
                text: rest.trim().to_string(),
                done: false,
            });
        } else if let Some(rest) = line.strip_prefix("- [x]") {
            items.push(TodoItem {
                text: rest.trim().to_string(),
                done: true,
            });
        }
    }
    items
}

/// 在 `content` 里找到与 `old_line` 逐字节相同的一行（第一次出现），
/// 替换成 `new_line`。找不到（文件已被 agent 并发改过）返回 `None`，
/// 调用方按"冲突，放弃这次写入，强制重读"处理，不是错误（见 design
/// 第 6 节）。
pub fn replace_todo_line(content: &str, old_line: &str, new_line: &str) -> Option<String> {
    let mut found = false;
    let mut out = String::with_capacity(content.len());
    for line in content.lines() {
        if !found && line == old_line {
            out.push_str(new_line);
            found = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    found.then_some(out)
}

/// 在最后一个 `- [ ]`/`- [x]` 行之后追加一条新任务；纯追加不依赖"找到
/// 匹配行"，冲突面比 `replace_todo_line` 小。文件里一条任务都没有时，
/// 追加在文件末尾（保留原有内容，末尾补一个换行再接新行，避免跟最后
/// 一行内容粘连）。
pub fn append_todo_item(content: &str, text: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let last_item_idx = lines
        .iter()
        .rposition(|l| l.trim_start().starts_with("- [ ]") || l.trim_start().starts_with("- [x]"));
    let insert_at = last_item_idx.map(|i| i + 1).unwrap_or(lines.len());
    let mut out = String::with_capacity(content.len() + text.len() + 8);
    for (i, line) in lines.iter().enumerate() {
        if i == insert_at {
            out.push_str("- [ ] ");
            out.push_str(text);
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
    }
    if insert_at == lines.len() {
        out.push_str("- [ ] ");
        out.push_str(text);
        out.push('\n');
    }
    out
}

/// 派发记录/计划时间/完成时间在 GUI 本地 sidecar 里用这个 key 关联到
/// 具体某条任务——不给 markdown 行发明稳定 id（那需要往文件里塞隐藏
/// 标记，agent 编辑时容易破坏），代价是"改了任务文字会跟丢这条的全部
/// 本地元数据"，v1 接受（design 非目标）。用文本 `trim` 后算哈希，不
/// 要求无碰撞，只要求"实践中够用"，同 `AgentKind` 分组等既有哈希用途
/// 的验收标准。
pub fn todo_line_key(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.trim().hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoState {
    Pending,
    InProgress,
    Done,
}

/// `done` 为真直接 `Done`（不管有没有派发记录——已完成的任务不需要
/// 再关心是谁做的）；否则看有没有派发记录，记录存在且目标 session
/// 仍存活（`target_alive`，调用方传 `ws.tabs.iter().any(|t| t.info.id
/// == dispatch.session_id && t.alive)`）→ `InProgress`；否则（没派发
/// 过，或派发目标已经退出）→ `Pending`。`plan_date`/`completed_at`
/// 不参与这个推导，跟三态是两件事（design 第 4/8 节）。
pub fn todo_display_state(
    item: &TodoItem,
    dispatch: Option<&DispatchRecord>,
    target_alive: bool,
) -> TodoState {
    if item.done {
        return TodoState::Done;
    }
    if dispatch.is_some() && target_alive {
        TodoState::InProgress
    } else {
        TodoState::Pending
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TodoFilter {
    #[default]
    All,
    Pending,
    InProgress,
    Done,
}

/// 纯前端过滤：状态相等匹配 + 关键字对 `TodoItem.text` 做大小写不敏感
/// 的子串匹配（空 `query` 不过滤）。作用在"已经解析+推导好状态"的
/// 内存列表上，不碰文件、不碰 sidecar（design 第 7 节）。
pub fn filter_todos(
    items: &[TodoItem],
    states: &[TodoState],
    filter: TodoFilter,
    query: &str,
) -> Vec<usize> {
    let query_lower = query.trim().to_lowercase();
    items
        .iter()
        .zip(states.iter())
        .enumerate()
        .filter(|(_, (_, state))| match filter {
            TodoFilter::All => true,
            TodoFilter::Pending => **state == TodoState::Pending,
            TodoFilter::InProgress => **state == TodoState::InProgress,
            TodoFilter::Done => **state == TodoState::Done,
        })
        .filter(|(_, (item, _))| {
            query_lower.is_empty() || item.text.to_lowercase().contains(&query_lower)
        })
        .map(|(i, _)| i)
        .collect()
}

/// `done` 翻转成 `completed_at` 该有的值：完成 → `Some(now)`，取消
/// 完成 → `None`。抽成纯函数是为了能不起 GUI/不碰文件单测这条转换
/// 规则本身。
pub fn completed_at_for_toggle(
    done: bool,
    now: std::time::SystemTime,
) -> Option<std::time::SystemTime> {
    done.then_some(now)
}

// ---- 以下为并入的 `todo_meta.rs`（Todo 面板 design 第 3/8 节） ----
// 派发记录 + 计划时间 + 完成时间。跟 `open_projects.rs` 同一挂靠模式——
// 纯运行时缓存，不进 git，不影响 `.dozer/todo.md` 本身的格式，读失败
// （不存在/损坏）一律回落空 map，不 panic。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DispatchRecord {
    pub session_id: String,
    pub dispatched_at: SystemTime,
}

/// 三个字段互相独立——只设 `plan_date` 不影响 `dispatch`，反之亦然。
/// `#[serde(default)]` 让老文件缺字段时补 `None` 而不是整份反序列化
/// 失败（同 `ShellLayout` 的既有惯例）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TodoTaskMeta {
    #[serde(default)]
    pub dispatch: Option<DispatchRecord>,
    #[serde(default)]
    pub plan_date: Option<String>,
    #[serde(default)]
    pub completed_at: Option<SystemTime>,
}

pub type TodoMetaState = HashMap<i64, HashMap<u64, TodoTaskMeta>>;

fn file_path() -> PathBuf {
    dozer_core::paths::config_dir().join("todo_meta.json")
}

pub fn meta_load() -> TodoMetaState {
    load_from(&file_path())
}

pub fn meta_save(state: &TodoMetaState) -> io::Result<()> {
    save_to(&file_path(), state)
}

fn load_from(path: &Path) -> TodoMetaState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_to(path: &Path, state: &TodoMetaState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state).expect("TodoMetaState 总能序列化");
    std::fs::write(path, json)
}

/// Todo 面板挂在每个 `Workspace` 上的状态。
#[derive(Default)]
pub struct WorkspaceState {
    items: Vec<TodoItem>,
    mtime: Option<std::time::SystemTime>,
    add_draft: String,
    filter: TodoFilter,
    search: String,
    dispatch_open: Option<usize>,
    pending_dispatch: std::collections::HashMap<usize, String>,
    editing_plan_date: Option<(usize, String)>,
}

impl WorkspaceState {
    /// 派发选择层是否打开(内核 `App::todo_dispatch_open` 键盘/UI 状态查询用)。
    pub fn dispatch_popup_open(&self) -> bool {
        self.dispatch_open.is_some()
    }

    /// "派发到新建"发起时记的 `tab_id → 任务文本` 映射,内核在
    /// `Message::TabAttached` 落地时用真正的 `session_id` 消费掉这条,
    /// 补记派发记录。未知 `tab_id` 返回 `None`,是 no-op。
    pub fn take_pending_dispatch(&mut self, tab_id: usize) -> Option<String> {
        self.pending_dispatch.remove(&tab_id)
    }

    /// 只读当前已解析的任务列表,给内核派发(`DispatchToExisting`/`Dispatch
    /// `New`)时按下标取任务文本用。
    pub fn items(&self) -> &[TodoItem] {
        &self.items
    }

    /// 关闭派发选择层(选中目标/新建后,或 Esc)。
    pub fn close_dispatch_popup(&mut self) {
        self.dispatch_open = None;
    }

    /// "派发到新建"发起时记一笔 `tab_id → text`,等 `on_tab_attached` 落地
    /// 时用真正的 `session_id` 补派发记录。
    pub fn insert_pending_dispatch(&mut self, tab_id: usize, text: String) {
        self.pending_dispatch.insert(tab_id, text);
    }

    /// 上次成功读取时 `.dozer/todo.md` 的 mtime,轮询靠比较它决定要不要
    /// 重读(`App::poll_todo_if_visible`)。`None` = 还没读过,或文件不存在。
    pub fn mtime(&self) -> Option<std::time::SystemTime> {
        self.mtime
    }
}

/// Todo 面板挂在 `App` 上的元数据(派发记录/计划时间/完成时间),按
/// `project_id` 分桶,整体持久化到 `todo_meta.json`。
#[derive(Default)]
pub struct AppState {
    meta: TodoMetaState,
}

impl AppState {
    pub fn load() -> Self {
        Self { meta: meta_load() }
    }

    fn save(&self) {
        if let Err(e) = meta_save(&self.meta) {
            tracing::warn!("写入 todo_meta.json 失败: {e}");
        }
    }

    /// 按 `project_id`+`todo_line_key` 查这条任务的元数据(派发记录/计划
    /// 时间/完成时间),渲染层(`view`)和三态推导都用这个。
    pub fn meta_for(&self, project_id: i64, key: u64) -> Option<&TodoTaskMeta> {
        self.meta.get(&project_id)?.get(&key)
    }

    /// 把一条派发记录写进去并落盘。`text` 用来算 `todo_line_key`——跟
    /// 查询用的 key 必须是同一套算法,否则写进去的记录永远查不到。
    pub fn record_dispatch(&mut self, project_id: i64, text: &str, session_id: String) {
        let key = todo_line_key(text);
        let entry = self.meta.entry(project_id).or_default();
        entry.insert(
            key,
            TodoTaskMeta {
                dispatch: Some(DispatchRecord {
                    session_id,
                    dispatched_at: std::time::SystemTime::now(),
                }),
                ..entry.get(&key).cloned().unwrap_or_default()
            },
        );
        self.save();
    }

    /// 写/清计划时间:`draft` 为空字符串时存 `None`。
    pub fn set_plan_date(&mut self, project_id: i64, text: &str, draft: String) {
        let key = todo_line_key(text);
        let entry = self.meta.entry(project_id).or_default();
        let meta = entry.entry(key).or_default();
        meta.plan_date = if draft.trim().is_empty() {
            None
        } else {
            Some(draft.trim().to_string())
        };
        self.save();
    }

    /// 勾选变完成 → 盖章当前时间;取消勾选 → 清空。
    pub fn set_completed_at(&mut self, project_id: i64, text: &str, done: bool) {
        let key = todo_line_key(text);
        let entry = self.meta.entry(project_id).or_default();
        let meta = entry.entry(key).or_default();
        meta.completed_at = completed_at_for_toggle(done, std::time::SystemTime::now());
        self.save();
    }
}

/// `view` 渲染派发相关 UI 需要的终端会话摘要,由内核从 `ws.tabs` 摘出来
/// 传入——`extensions::todo` 不知道 `SessionTab` 这个终端领域的类型。
pub struct SessionTabSummary {
    pub session_id: String,
    pub title: String,
    pub alive: bool,
}

/// Todo 面板自己的消息类型。`DispatchToExisting`/`DispatchNew` 涉及终端
/// 会话读写,内核在到达 `update` 之前就会拦截处理,不会真的传进
/// `update`——传进来会 `unreachable!`(同 Git Log 试点 `LoadMore` 的
/// 处理方式)。
#[derive(Debug, Clone)]
pub enum Message {
    Toggle(usize),
    AddInputChanged(String),
    AddSubmit,
    FilterSet(TodoFilter),
    SearchChanged(String),
    DispatchOpen(usize),
    DispatchClose,
    DispatchToExisting(usize, String),
    DispatchNew(usize, crate::workspace::PickerLaunch),
    PlanDateEditStart(usize),
    PlanDateChanged(String),
    PlanDateSubmit,
}

/// 重读 `.dozer/todo.md`,刷新 `items`/`mtime`。文件不存在/读失败按空
/// 列表处理,不 panic。现有 `Workspace::reload_todo_from_disk` 的搬家
/// 版本。
pub fn reload_from_disk(ws_state: &mut WorkspaceState, project_path: &std::path::Path) {
    let path = todo_path(project_path);
    ws_state.mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let md = std::fs::read_to_string(&path).unwrap_or_default();
    ws_state.items = parse_todo(&md);
}

/// 处理除 `DispatchToExisting`/`DispatchNew` 之外的 10 条消息,统一接收
/// 两块状态——`Toggle`/`PlanDateEditStart`/`PlanDateSubmit` 需要读写
/// `AppState`(不只是 Git Log/浏览器试点里"只有派发类消息碰跨领域状态"
/// 那么简单,写计划前重新核对现有代码才发现这点)。
pub fn update(
    ws_state: &mut WorkspaceState,
    app_state: &mut AppState,
    msg: Message,
    project_id: i64,
    project_path: &std::path::Path,
) {
    match msg {
        Message::Toggle(idx) => {
            let Some(item) = ws_state.items.get(idx) else {
                return;
            };
            let old_line = format!("- [{}] {}", if item.done { "x" } else { " " }, item.text);
            let new_line = format!("- [{}] {}", if item.done { " " } else { "x" }, item.text);
            let before_text = item.text.clone();
            let path = todo_path(project_path);
            let Ok(content) = std::fs::read_to_string(&path) else {
                return;
            };
            match replace_todo_line(&content, &old_line, &new_line) {
                Some(new_content) => {
                    if let Err(e) = std::fs::write(&path, &new_content) {
                        tracing::warn!("写入 todo.md 失败: {e}");
                        return;
                    }
                    reload_from_disk(ws_state, project_path);
                }
                None => {
                    // 冲突:文件已经变了,放弃这次写入,直接重读展示最新状态。
                    reload_from_disk(ws_state, project_path);
                    return;
                }
            }
            // 文本没变(正常勾选场景)才更新 completed_at;如果文本变了
            // (文件可能在重读期间被 agent 并发改过),跳过,避免把完成
            // 时间错记到另一条任务上。
            if let Some(after) = ws_state.items.get(idx)
                && after.text == before_text
            {
                app_state.set_completed_at(project_id, &after.text, after.done);
            }
        }
        Message::AddInputChanged(s) => ws_state.add_draft = s,
        Message::AddSubmit => {
            let text = ws_state.add_draft.trim().to_string();
            if text.is_empty() {
                return;
            }
            let path = todo_path(project_path);
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let new_content = append_todo_item(&content, &text);
            if let Some(parent) = path.parent()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                tracing::warn!("创建 .dozer 目录失败: {e}");
                return;
            }
            if let Err(e) = std::fs::write(&path, &new_content) {
                tracing::warn!("写入 todo.md 失败: {e}");
                return;
            }
            ws_state.add_draft.clear();
            reload_from_disk(ws_state, project_path);
        }
        Message::FilterSet(f) => ws_state.filter = f,
        Message::SearchChanged(s) => ws_state.search = s,
        Message::DispatchOpen(idx) => ws_state.dispatch_open = Some(idx),
        Message::DispatchClose => ws_state.dispatch_open = None,
        Message::PlanDateEditStart(idx) => {
            let existing = ws_state
                .items
                .get(idx)
                .map(|item| todo_line_key(&item.text))
                .and_then(|key| app_state.meta_for(project_id, key))
                .and_then(|m| m.plan_date.clone())
                .unwrap_or_default();
            ws_state.editing_plan_date = Some((idx, existing));
        }
        Message::PlanDateChanged(s) => {
            if let Some((_, draft)) = ws_state.editing_plan_date.as_mut() {
                *draft = s;
            }
        }
        Message::PlanDateSubmit => {
            if let Some((idx, draft)) = ws_state.editing_plan_date.clone()
                && let Some(text) = ws_state.items.get(idx).map(|item| item.text.clone())
            {
                app_state.set_plan_date(project_id, &text, draft);
            }
            ws_state.editing_plan_date = None;
        }
        Message::DispatchToExisting(..) | Message::DispatchNew(..) => {
            unreachable!(
                "DispatchToExisting/DispatchNew 由内核在 Message::Todo 分支里直接处理\
                 (需要终端会话读写能力),不会转发到这里"
            )
        }
    }
}

pub fn view<'a>(
    app_state: &'a AppState,
    ws_state: &'a WorkspaceState,
    project_id: i64,
    tabs: &[SessionTabSummary],
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let header = column![
        text("Todo")
            .size(theme::font::title())
            .color(theme::color::CREAM),
        text(format!("{} 条任务 · .dozer/todo.md", ws_state.items.len()))
            .size(theme::font::caption())
            .color(theme::color::DIM),
    ]
    .spacing(4)
    .padding([20, 20]);

    let states: Vec<TodoState> = ws_state
        .items
        .iter()
        .map(|item| {
            let key = todo_line_key(&item.text);
            let dispatch = app_state
                .meta_for(project_id, key)
                .and_then(|m| m.dispatch.as_ref());
            let target_alive = dispatch
                .map(|d| tabs.iter().any(|t| t.session_id == d.session_id && t.alive))
                .unwrap_or(false);
            todo_display_state(item, dispatch, target_alive)
        })
        .collect();
    let visible_idx = filter_todos(&ws_state.items, &states, ws_state.filter, &ws_state.search);

    let existing_tabs: Vec<(&str, String)> = tabs
        .iter()
        .filter(|t| t.alive)
        .map(|t| (t.session_id.as_str(), t.title.clone()))
        .collect();

    let toolbar = row![
        todo_filter_segment("全部", TodoFilter::All, ws_state.filter),
        todo_filter_segment("待办", TodoFilter::Pending, ws_state.filter),
        todo_filter_segment("进行中", TodoFilter::InProgress, ws_state.filter),
        todo_filter_segment("完成", TodoFilter::Done, ws_state.filter),
        text_input("搜索任务关键字…", &ws_state.search)
            .on_input(Message::SearchChanged)
            .size(theme::font::body())
            .width(Length::Fill)
            .style(
                |_t: &iced_widget::Theme, _s| iced_widget::text_input::Style {
                    background: theme::color::BG.into(),
                    border: Border::default(),
                    icon: theme::color::DIM,
                    placeholder: theme::color::DIM,
                    value: theme::color::CREAM,
                    selection: theme::color::GOLD,
                }
            ),
    ]
    .spacing(8)
    .padding([12, 20])
    .align_y(iced_widget::core::alignment::Vertical::Center);

    let mut list = column![].spacing(2);
    if visible_idx.is_empty() {
        list = list.push(
            container(
                text("没有匹配的任务")
                    .size(theme::font::body())
                    .color(theme::color::DIM),
            )
            .padding([20, 20]),
        );
    } else {
        for &idx in &visible_idx {
            let item = &ws_state.items[idx];
            let key = todo_line_key(&item.text);
            let meta = app_state.meta_for(project_id, key);
            let mut row = None;
            if let Some((editing_idx, draft)) = &ws_state.editing_plan_date
                && *editing_idx == idx
            {
                row = Some(todo_plan_date_edit_row(item, draft));
            }
            match row {
                Some(r) => list = list.push(r),
                None => {
                    list = list.push(todo_row(
                        idx,
                        item,
                        states[idx],
                        meta,
                        ws_state.dispatch_open == Some(idx),
                        &existing_tabs,
                    ));
                }
            }
        }
    }

    let add_row = text_input("＋新增任务…", &ws_state.add_draft)
        .on_input(Message::AddInputChanged)
        .on_submit(Message::AddSubmit)
        .size(theme::font::body())
        .padding([10, 20])
        .style(
            |_t: &iced_widget::Theme, _s| iced_widget::text_input::Style {
                background: theme::color::BG.into(),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                icon: theme::color::DIM,
                placeholder: theme::color::DIM,
                value: theme::color::CREAM,
                selection: theme::color::GOLD,
            },
        );

    let divider = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            border: Border {
                color: theme::color::BORDER,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        });

    let content = column![
        header,
        toolbar,
        scrollable(list).height(Length::Fill),
        divider,
        add_row,
    ]
    .height(Length::Fill);

    container(content)
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::BG.into()),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

/// 一条 Todo 任务行。勾选框+文本（完成态删除线+暗色）+ 右侧按状态显示：
/// 待办=派发按钮；进行中=绿点+"进行中"标签；完成=占位。派发选择层叠在行下方。
fn todo_row<'a>(
    idx: usize,
    item: &'a TodoItem,
    state: TodoState,
    meta: Option<&'a TodoTaskMeta>,
    dispatch_open: bool,
    existing_tabs: &'a [(&'a str, String)],
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let done = item.done;
    let box_color = if done {
        theme::color::BORDER
    } else {
        theme::color::DIM
    };
    let checkbox = button(
        container(if done {
            text("✓")
                .size(theme::font::caption())
                .color(theme::color::DIM)
                .into()
        } else {
            Element::from(iced_widget::space::Space::new())
        })
        .width(Length::Fixed(18.0))
        .height(Length::Fixed(18.0))
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: if done {
                Some(theme::color::BORDER.into())
            } else {
                None
            },
            border: Border {
                color: box_color,
                width: 1.5,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        }),
    )
    .on_press(Message::Toggle(idx))
    .padding(0)
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: theme::color::CREAM,
        ..button::Style::default()
    });

    let label_color = if done {
        theme::color::DIM
    } else {
        theme::color::CREAM
    };
    let label: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> = if done {
        let rich: iced_widget::text::Rich<
            '_,
            (),
            Message,
            iced_widget::Theme,
            iced_renderer::Renderer,
        > = rich_text![
            span(item.text.clone())
                .size(theme::font::body())
                .color(label_color)
                .strikethrough(true)
        ];
        rich.into()
    } else {
        text(item.text.clone())
            .size(theme::font::body())
            .color(label_color)
            .into()
    };

    let date_label: Option<Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        match state {
            TodoState::Done => meta.and_then(|m| m.completed_at).map(|t| {
                text(format!("完成于 {}", format_todo_time(t)))
                    .size(theme::font::caption())
                    .color(theme::color::DIM)
                    .into()
            }),
            _ => meta.and_then(|m| m.plan_date.as_deref()).map(|d| {
                button(
                    text(format!("计划 {d}"))
                        .size(theme::font::caption())
                        .color(theme::color::DIM),
                )
                .on_press(Message::PlanDateEditStart(idx))
                .padding(0)
                .style(|_t: &iced_widget::Theme, _s| button::Style {
                    background: None,
                    text_color: theme::color::DIM,
                    ..button::Style::default()
                })
                .into()
            }),
        };
    let mut middle = row![checkbox, label]
        .spacing(10)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    if let Some(label) = date_label {
        middle = middle.push(label);
    }

    let trailing: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match state {
            TodoState::Pending => button(
                row![
                    icons::view(
                        icons::IconKind::SquarePlus,
                        crate::theme::icon_size::row(),
                        theme::color::GOLD
                    ),
                    text("派发")
                        .size(theme::font::caption())
                        .color(theme::color::GOLD),
                ]
                .spacing(4)
                .align_y(iced_widget::core::alignment::Vertical::Center),
            )
            .on_press(Message::DispatchOpen(idx))
            .padding([5, 10])
            .style(|_t: &iced_widget::Theme, _s| button::Style {
                background: None,
                text_color: theme::color::GOLD,
                border: Border {
                    color: theme::color::BORDER,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..button::Style::default()
            })
            .into(),
            TodoState::InProgress => row![
                text("●")
                    .size(theme::font::dot_sm())
                    .color(theme::color::GREEN),
                text("进行中")
                    .size(theme::font::caption())
                    .color(theme::color::GREEN),
            ]
            .spacing(5)
            .into(),
            TodoState::Done => iced_widget::space::Space::new().into(),
        };

    let base: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> =
        row![middle, trailing]
            .spacing(10)
            .align_y(iced_widget::core::alignment::Vertical::Center)
            .padding([10, 20])
            .into();
    if dispatch_open {
        column![base, todo_dispatch_popup(idx, existing_tabs)].into()
    } else {
        base
    }
}

/// Todo 派发选择层：列出当前项目存活的 agent tab + 一个"新建"入口，样式
/// 对齐 `agent_picker_popup`（CARD 底 + BORDER 描边）。挂在触发它的那一行
/// 下方，不需要额外的坐标计算。
fn todo_dispatch_popup<'a>(
    idx: usize,
    existing_tabs: &'a [(&'a str, String)],
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(2);
    for (session_id, title) in existing_tabs {
        col = col.push(
            button(
                text(title.clone())
                    .size(theme::font::body())
                    .color(theme::color::CREAM),
            )
            .on_press(Message::DispatchToExisting(idx, session_id.to_string()))
            .width(Length::Fill)
            .padding([6, 12])
            .style(|_t: &iced_widget::Theme, _s| button::Style {
                background: None,
                text_color: theme::color::CREAM,
                ..button::Style::default()
            }),
        );
    }
    col = col.push(
        button(
            text("新建 agent 会话…")
                .size(theme::font::body())
                .color(theme::color::GOLD),
        )
        .on_press(Message::DispatchNew(
            idx,
            crate::workspace::PickerLaunch::Agent(None),
        ))
        .width(Length::Fill)
        .padding([6, 12])
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: theme::color::GOLD,
            ..button::Style::default()
        }),
    );
    container(col)
        .padding(6)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::CARD.into()),
            border: Border {
                color: theme::color::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// 计划时间内联编辑态：任务文本 + 一个 `text_input`，回车提交。
fn todo_plan_date_edit_row<'a>(
    item: &'a TodoItem,
    draft: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    row![
        text(item.text.clone())
            .size(theme::font::body())
            .color(theme::color::CREAM),
        text_input("计划时间，如 08-10", draft)
            .on_input(Message::PlanDateChanged)
            .on_submit(Message::PlanDateSubmit)
            .size(theme::font::caption())
            .width(Length::Fixed(140.0)),
    ]
    .spacing(10)
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .padding([10, 20])
    .into()
}

/// 筛选分段按钮，选中态高亮。
fn todo_filter_segment<'a>(
    label: &'a str,
    value: TodoFilter,
    current: TodoFilter,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let active = value == current;
    button(text(label).size(theme::font::caption()).color(if active {
        theme::color::CREAM
    } else {
        theme::color::DIM
    }))
    .on_press(Message::FilterSet(value))
    .padding([4, 10])
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: if active {
            Some(theme::color::CARD.into())
        } else {
            None
        },
        text_color: if active {
            theme::color::CREAM
        } else {
            theme::color::DIM
        },
        border: Border {
            radius: 5.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    })
    .into()
}

/// `SystemTime` → "MM-DD HH:MM"(UTC)。不引 `chrono`,用 civil-from-days
/// 算法(Howard Hinnant)手推公历年月日,再拼 HH:MM。只用于"完成于"这种
/// 粗粒度提示,UTC 而非本地时区,不追求夏令时/时区严格正确。
fn format_todo_time(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // days = 秒数 → 自 1970-01-01 的整数日;`secs` 已经是 `u64`(1970 前会被
    // 上面的 `unwrap_or_default()` 夹到 0),这里不会是负数。
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let (hour, minute) = (rem / 3600, (rem % 3600) / 60);
    let (_y, m, d) = civil_from_days(days);
    format!("{:02}-{:02} {:02}:{:02}", m, d, hour, minute)
}

/// civil-from-days：把"自 1970-01-01 的天数"换算成 (年, 月, 日)。
/// 用 Hinnant 经典公式,范围覆盖 1970..=2100,足够"完成于"提示用。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn todo_path_is_dot_dozer() {
        assert_eq!(
            todo_path(Path::new("/repo")),
            PathBuf::from("/repo/.dozer/todo.md")
        );
    }

    #[test]
    fn parses_pending_and_done_items() {
        let md = "# Todo\n\n- [ ] 修复登录页闪烁\n- [x] 补 README 安装说明\n";
        let items = parse_todo(md);
        assert_eq!(
            items,
            vec![
                TodoItem {
                    text: "修复登录页闪烁".to_string(),
                    done: false
                },
                TodoItem {
                    text: "补 README 安装说明".to_string(),
                    done: true
                },
            ]
        );
    }

    #[test]
    fn ignores_non_checkbox_lines_and_blank_file() {
        let md = "# Todo\n\n正文说明，不是任务。\n- 普通列表项也不算\n  - [ ] 缩进的不算一级\n";
        assert_eq!(parse_todo(md), Vec::new());
        assert_eq!(parse_todo(""), Vec::new());
    }

    #[test]
    fn replace_todo_line_hits_and_replaces() {
        let content = "# Todo\n\n- [ ] 任务A\n- [ ] 任务B\n";
        let out = replace_todo_line(content, "- [ ] 任务A", "- [x] 任务A").unwrap();
        assert_eq!(out, "# Todo\n\n- [x] 任务A\n- [ ] 任务B\n");
    }

    #[test]
    fn replace_todo_line_misses_returns_none() {
        let content = "# Todo\n\n- [ ] 任务A\n";
        assert_eq!(replace_todo_line(content, "- [ ] 不存在的行", "x"), None);
    }

    #[test]
    fn replace_todo_line_only_replaces_first_match() {
        // 已知限制：文件里有多行完全相同的文本时，只替换第一次出现。
        let content = "- [ ] 重复\n- [ ] 重复\n";
        let out = replace_todo_line(content, "- [ ] 重复", "- [x] 重复").unwrap();
        assert_eq!(out, "- [x] 重复\n- [ ] 重复\n");
    }

    #[test]
    fn append_todo_item_to_empty_list() {
        let content = "# Todo\n";
        assert_eq!(
            append_todo_item(content, "新任务"),
            "# Todo\n- [ ] 新任务\n"
        );
    }

    #[test]
    fn append_todo_item_after_last_existing_item() {
        let content = "# Todo\n\n- [ ] 任务A\n- [x] 任务B\n";
        assert_eq!(
            append_todo_item(content, "任务C"),
            "# Todo\n\n- [ ] 任务A\n- [x] 任务B\n- [ ] 任务C\n"
        );
    }

    #[test]
    fn todo_line_key_ignores_surrounding_whitespace_but_not_content() {
        assert_eq!(todo_line_key("  任务A  "), todo_line_key("任务A"));
        assert_ne!(todo_line_key("任务A"), todo_line_key("任务B"));
    }

    fn item(done: bool) -> TodoItem {
        TodoItem {
            text: "任务".to_string(),
            done,
        }
    }

    fn record() -> DispatchRecord {
        DispatchRecord {
            session_id: "sess".to_string(),
            dispatched_at: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn done_item_is_always_done_regardless_of_dispatch() {
        assert_eq!(
            todo_display_state(&item(true), None, false),
            TodoState::Done
        );
        assert_eq!(
            todo_display_state(&item(true), Some(&record()), true),
            TodoState::Done
        );
    }

    #[test]
    fn pending_without_dispatch_is_pending() {
        assert_eq!(
            todo_display_state(&item(false), None, false),
            TodoState::Pending
        );
    }

    #[test]
    fn pending_with_live_dispatch_is_in_progress() {
        assert_eq!(
            todo_display_state(&item(false), Some(&record()), true),
            TodoState::InProgress
        );
    }

    #[test]
    fn pending_with_dead_dispatch_falls_back_to_pending() {
        assert_eq!(
            todo_display_state(&item(false), Some(&record()), false),
            TodoState::Pending
        );
    }

    fn sample() -> (Vec<TodoItem>, Vec<TodoState>) {
        let items = vec![
            TodoItem {
                text: "修复登录页闪烁".to_string(),
                done: false,
            },
            TodoItem {
                text: "补 README 安装说明".to_string(),
                done: false,
            },
            TodoItem {
                text: "移除死代码".to_string(),
                done: true,
            },
        ];
        let states = vec![TodoState::Pending, TodoState::InProgress, TodoState::Done];
        (items, states)
    }

    #[test]
    fn filter_all_with_empty_query_keeps_everything() {
        let (items, states) = sample();
        assert_eq!(
            filter_todos(&items, &states, TodoFilter::All, ""),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn filter_by_state() {
        let (items, states) = sample();
        assert_eq!(filter_todos(&items, &states, TodoFilter::Done, ""), vec![2]);
        assert_eq!(
            filter_todos(&items, &states, TodoFilter::InProgress, ""),
            vec![1]
        );
    }

    #[test]
    fn filter_by_keyword_case_insensitive_substring() {
        let (items, states) = sample();
        assert_eq!(
            filter_todos(&items, &states, TodoFilter::All, "readme"),
            vec![1]
        );
        assert_eq!(
            filter_todos(&items, &states, TodoFilter::All, "登录"),
            vec![0]
        );
    }

    #[test]
    fn filter_combines_state_and_keyword() {
        let (items, states) = sample();
        // "README" 只在下标 1，且下标 1 是 InProgress——命中；换成 Done 就不命中了。
        assert_eq!(
            filter_todos(&items, &states, TodoFilter::InProgress, "readme"),
            vec![1]
        );
        assert!(filter_todos(&items, &states, TodoFilter::Done, "readme").is_empty());
    }

    #[test]
    fn done_gets_timestamp_undone_gets_none() {
        let now = SystemTime::now();
        assert_eq!(completed_at_for_toggle(true, now), Some(now));
        assert_eq!(completed_at_for_toggle(false, now), None);
    }

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        assert_eq!(load_from(&path), TodoMetaState::default());
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load_from(&path), TodoMetaState::default());
    }

    #[test]
    fn save_then_load_round_trips_and_fields_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todo_meta.json");
        let mut state = TodoMetaState::new();
        let mut per_project = HashMap::new();
        // 一条只设了 plan_date，一条只设了 dispatch——验证字段互相独立。
        per_project.insert(
            1,
            TodoTaskMeta {
                plan_date: Some("2026-08-10".to_string()),
                ..Default::default()
            },
        );
        per_project.insert(
            2,
            TodoTaskMeta {
                dispatch: Some(DispatchRecord {
                    session_id: "sess-abc".to_string(),
                    dispatched_at: SystemTime::UNIX_EPOCH,
                }),
                ..Default::default()
            },
        );
        state.insert(42, per_project);
        save_to(&path, &state).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded, state);
        assert!(loaded[&42][&1].dispatch.is_none());
        assert!(loaded[&42][&2].plan_date.is_none());
    }

    fn ws_with_item(text: &str, done: bool) -> WorkspaceState {
        WorkspaceState {
            items: vec![TodoItem {
                text: text.to_string(),
                done,
            }],
            ..WorkspaceState::default()
        }
    }

    fn project_dir_with_todo(md: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = todo_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, md).unwrap();
        let root = dir.path().to_path_buf();
        (dir, root)
    }

    #[test]
    fn reload_from_disk_populates_items_and_mtime() {
        let (_dir, root) = project_dir_with_todo("- [ ] 任务A\n");
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, &root);
        assert_eq!(ws_state.items.len(), 1);
        assert!(ws_state.mtime.is_some());
    }

    #[test]
    fn reload_from_disk_missing_file_yields_empty_items() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert!(ws_state.items.is_empty());
        assert!(ws_state.mtime.is_none());
    }

    #[test]
    fn update_toggle_flips_line_on_disk_and_sets_completed_at() {
        let (_dir, root) = project_dir_with_todo("- [ ] 任务A\n");
        let mut ws_state = ws_with_item("任务A", false);
        let mut app_state = AppState::default();
        update(&mut ws_state, &mut app_state, Message::Toggle(0), 1, &root);
        assert!(ws_state.items[0].done, "内存态应反映勾选后的完成态");
        let key = todo_line_key("任务A");
        assert!(app_state.meta_for(1, key).unwrap().completed_at.is_some());
        let content = std::fs::read_to_string(todo_path(&root)).unwrap();
        assert!(content.contains("- [x] 任务A"));
    }

    #[test]
    fn update_toggle_missing_original_line_reloads_without_setting_completed_at() {
        // 文件内容跟内存态对不上(模拟并发冲突):old_line 找不到。
        let (_dir, root) = project_dir_with_todo("- [x] 任务A(已经被改过)\n");
        let mut ws_state = ws_with_item("任务A", false);
        let mut app_state = AppState::default();
        update(&mut ws_state, &mut app_state, Message::Toggle(0), 1, &root);
        let key = todo_line_key("任务A");
        assert!(app_state.meta_for(1, key).is_none(), "冲突时不该记完成时间");
    }

    #[test]
    fn update_add_submit_appends_and_clears_draft() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState {
            add_draft: "新任务".to_string(),
            ..WorkspaceState::default()
        };
        let mut app_state = AppState::default();
        update(&mut ws_state, &mut app_state, Message::AddSubmit, 1, &root);
        assert!(ws_state.add_draft.is_empty());
        assert_eq!(ws_state.items.len(), 1);
        assert_eq!(ws_state.items[0].text, "新任务");
    }

    #[test]
    fn update_add_submit_empty_draft_is_noop() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        update(&mut ws_state, &mut app_state, Message::AddSubmit, 1, &root);
        assert!(ws_state.items.is_empty());
    }

    #[test]
    fn update_filter_and_search_set_fields() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::FilterSet(TodoFilter::Done),
            1,
            &root,
        );
        assert_eq!(ws_state.filter, TodoFilter::Done);
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchChanged("关键字".to_string()),
            1,
            &root,
        );
        assert_eq!(ws_state.search, "关键字");
    }

    #[test]
    fn update_dispatch_open_and_close_toggle_popup() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::DispatchOpen(2),
            1,
            &root,
        );
        assert!(ws_state.dispatch_popup_open());
        update(
            &mut ws_state,
            &mut app_state,
            Message::DispatchClose,
            1,
            &root,
        );
        assert!(!ws_state.dispatch_popup_open());
    }

    #[test]
    fn update_plan_date_edit_start_prefills_from_app_state() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = ws_with_item("任务A", false);
        let mut app_state = AppState::default();
        app_state.set_plan_date(1, "任务A", "08-10".to_string());
        update(
            &mut ws_state,
            &mut app_state,
            Message::PlanDateEditStart(0),
            1,
            &root,
        );
        assert_eq!(ws_state.editing_plan_date, Some((0, "08-10".to_string())));
    }

    #[test]
    fn update_plan_date_edit_start_no_existing_value_prefills_empty() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = ws_with_item("任务A", false);
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::PlanDateEditStart(0),
            1,
            &root,
        );
        assert_eq!(ws_state.editing_plan_date, Some((0, String::new())));
    }

    #[test]
    fn update_plan_date_submit_writes_app_state_and_clears_editing() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = ws_with_item("任务A", false);
        ws_state.editing_plan_date = Some((0, "08-10".to_string()));
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::PlanDateSubmit,
            1,
            &root,
        );
        assert!(ws_state.editing_plan_date.is_none());
        let key = todo_line_key("任务A");
        assert_eq!(
            app_state.meta_for(1, key).unwrap().plan_date.as_deref(),
            Some("08-10")
        );
    }

    #[test]
    fn app_state_record_dispatch_then_meta_for_finds_it() {
        let mut app_state = AppState::default();
        app_state.record_dispatch(1, "任务A", "sess-1".to_string());
        let key = todo_line_key("任务A");
        let meta = app_state.meta_for(1, key).unwrap();
        assert_eq!(meta.dispatch.as_ref().unwrap().session_id, "sess-1");
    }

    #[test]
    fn take_pending_dispatch_removes_and_returns_once() {
        let mut ws_state = WorkspaceState::default();
        ws_state.pending_dispatch.insert(7, "任务A".to_string());
        assert_eq!(ws_state.take_pending_dispatch(7), Some("任务A".to_string()));
        assert_eq!(ws_state.take_pending_dispatch(7), None, "取过一次就没了");
    }
}
