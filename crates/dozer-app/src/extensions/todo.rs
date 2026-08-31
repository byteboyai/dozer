//! `.dozer/todo.md` 任务列表解析（Todo 面板 design，2026-08-06；已并入
//! `extensions::todo`）：
//! 标准两态 checkbox（`- [ ]`/`- [x]`），git 可追踪，agent 可直接读写。
//! 解析风格镜像 `goal.rs`——手写、宽松，不引入 markdown 库；格式意外
//! （多级缩进、非 checkbox 正文）一律忽略，不因为文件"长得不标准"而失败。

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::app::{App, HoverId};
use crate::theme;
use crate::workspace::{AddrEvent, Workspace, agent_icon, tab_title};
use byteui::interaction::icons;
use dozer_core::protocol::AgentKind;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Border, Color, Element, Length, Padding, Rectangle, mouse};
use iced_widget::{
    MouseArea, button, column, container, rich_text, row, scrollable, space, span, text,
};

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

/// 在第一个 `- [ ]`/`- [x]` checkbox 行之前插入一条新任务：新任务"置顶"到
/// 任务列表顶部（待办块最前），符合"新增即置顶 + 列表顶部可见"的交互。
/// 文件里一条任务都没有时，插入在文件末尾（保留原有内容，末尾补一个换行
/// 再接新行，避免跟最后一行内容粘连）；headline 等非 checkbox 行保持在
/// 新任务之前（不挤到标题上面去）。
pub fn prepend_todo_item(content: &str, text: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let first_item_idx = lines
        .iter()
        .position(|l| l.trim_start().starts_with("- [ ]") || l.trim_start().starts_with("- [x]"));
    let insert_at = first_item_idx.unwrap_or(lines.len());
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

/// 把"待办块"里第 `from` 个待办行(按文件里待办行的出现次序,0-based)移动
/// 到第 `to` 个待办位(任意合法 rank,不必相邻)。已完成行保持原位置、整体
/// 仍在待办之后(视图再沉底);非 checkbox 行(标题/正文/空行)位置完全不动,
/// 只动 checkbox 待办行的先后。返回重建后的全文;`from`/`to` 越界或相等
/// 返回 `None`。这是鼠标拖拽排序的落盘内核——拖拽在 `DragEnd` 时只调
/// 一次,把整段拖拽累积成的 source→target 一次性落到 `.dozer/todo.md`,
/// 拖拽过程中不碰文件(视图层靠 item-index 置换即时反馈)。
pub fn move_pending_to(content: &str, from: usize, to: usize) -> Option<String> {
    if from == to {
        return None;
    }
    let lines: Vec<&str> = content.lines().collect();
    // 标记每行的类别,并收集待办行原文(保持文件次序)。
    let mut kinds: Vec<Option<bool>> = Vec::with_capacity(lines.len());
    let mut pending_lines: Vec<String> = Vec::new();
    for line in &lines {
        let t = line.trim_start();
        if t.starts_with("- [ ]") {
            kinds.push(Some(true));
            pending_lines.push(line.to_string());
        } else if t.starts_with("- [x]") {
            kinds.push(Some(false));
        } else {
            kinds.push(None);
        }
    }
    if from >= pending_lines.len() || to >= pending_lines.len() {
        return None;
    }
    // 把 from 处的待办行搬到 to 位:先摘下,再插回(中间行整体顺移)。
    let moved = pending_lines.remove(from);
    pending_lines.insert(to, moved);
    // 重建:遍历原行,checkbox 行按"待办块(新序)+ 已完成块(原序)"填充,
    // 非 checkbox 行原样保留。已完成行用原文(line)。
    let mut out = String::with_capacity(content.len());
    let mut pi = 0usize;
    for (i, line) in lines.iter().enumerate() {
        match kinds[i] {
            None => out.push_str(line),
            Some(true) => {
                out.push_str(&pending_lines[pi]);
                pi += 1;
            }
            Some(false) => out.push_str(line),
        }
        out.push('\n');
    }
    Some(out)
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

/// Todo 面板右区的两种展示形态(对应截图顶部 列表 / MARKDOWN 两个 tab)。
/// `List` 完整实现;`Markdown` 为只读占位视图(见 design)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TodoViewMode {
    #[default]
    List,
    Markdown,
}

/// 鼠标拖拽排序进行态:只记被拖起的待办任务和当前光标悬停到的目标待办,
/// 都用 `items` 里的下标(item-index)表示,**不**用"待办块"相对 rank。
/// 好处是过滤/搜索视图下也成立——展示置换在 `todo_list_view` 里直接对
/// 可见待办子序列(按 item-index)做,只有松手写盘时才把两端 item-index
/// 折算成文件里的待办 rank 交给 `move_pending_to`(见 `DragEnd`)。
/// `target_idx == usize::MAX` 表示"拖到待办块末尾(已完成之前)"——光标
/// 悬停到已完成卡片时取这个值。`source_idx == target_idx` 即还没真的
/// 移动过(纯点击),`DragEnd` 时不会写盘。已完成任务永远不参与拖拽:
/// `source_idx` 只能来自待办,悬停已完成只改变 `target_idx`(夹到末尾)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TodoDrag {
    pub source_idx: usize,
    pub target_idx: usize,
}

/// 新增任务后"置顶 + 选中保持"的计时记录:`idx` 是新增后那条任务在
/// `items` 里的下标,`until` 是自动清除其 `selected_row` 高亮的时刻
/// (`t + ADD_SELECT_HIGHLIGHT`)。`next_flash_wake`/`advance_flash` 据此
/// 恰好到点清除,不空转也不永远选中。
#[derive(Debug, Clone, Copy)]
struct Flash {
    idx: usize,
    until: std::time::Instant,
}

/// 新增任务后卡片选中高亮保持的时长。之后 `selected_row` 自动清除,
/// 除非用户在这期间已经手动点了别的卡片(见 `RowSelect` 里对 `flash`
/// 的清除)。
pub(crate) const ADD_SELECT_HIGHLIGHT: std::time::Duration = std::time::Duration::from_secs(2);

/// Todo 列表滚动容器的 `scrollable::Id`:main.rs 在新增任务置顶后据此发
/// `scrollable::scroll_to` 滚回顶部(`App::take_todo_scroll_to_top`),
/// 让新任务在列表顶部可见。取值只要在整棵 widget 树里唯一即可。
pub const TODO_LIST_SCROLL_ID: &str = "todo-list";
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
    /// 新增任务框草稿。类型从 `String` 换成 `iced_widget::text_editor::
    /// Content`(实现 `Default`/`Clone`)——真正的 `text_editor` 自己管理
    /// 光标/选区,不再需要应用层维护 `add_cursor` 字符下标。
    add_draft: iced_widget::text_editor::Content,
    /// 新增任务框是否持有 iced 内部真实焦点。**不是**应用层手动置位的
    /// 镜像——每帧渲染循环里 `CaptureAddFocus` 问一遍 iced 真相后立刻写
    /// 进这里(`set_add_focused`)。
    add_focused: bool,
    /// 新增任务框高度(逻辑像素)。框顶的拖拽手柄向上拉时由 app 层换算写回
    /// (见 `app.rs::RowDrag` 的 `TodoAddGrow` 分支),0 表示"未拖过、用默认
    /// 高",视图侧一律 `max(ADD_INPUT_MIN_HEIGHT)` 兜底——这样 `#[derive(
    /// Default)]` 给的 0 也不会渲染成 0 高框。
    add_input_height: f32,
    filter: TodoFilter,
    view_mode: TodoViewMode,
    selected_row: Option<usize>,
    /// 新增任务后"置顶 + 选中保持 2 秒"的计时态。`Some(Flash)` 表示刚新增
    /// 了一条任务,其卡片的 `selected_row` 高亮要在 `flash.until` 时刻自动
    /// 清除;期间的挂起唤醒由 `next_flash_wake` 驱动(main.rs 据此排下次
    /// 重绘,见 `App::advance_flash`)。用户手动点了其它卡片会清除本字段,
    /// 不再让 2 秒倒计时去抢用户的主动选中。
    flash: Option<Flash>,
    /// 新增任务后请求"下滑列表到底/置顶"的一次性滚动位标记。新增置顶后
    /// 让列表滚回顶部使新任务可见;由 main.rs 在下一帧 `interface.operate`
    /// 消耗(见 `App::take_todo_scroll_to_top`),置位后一直为 `true` 直到
    /// 被取走,避免主事件循环与渲染循环的帧序差异漏掉这次滚动。
    scroll_to_top: bool,
    /// 已生效的搜索关键词(列表过滤用)。打字期间只改草稿 `search_draft`,
    /// 回车/点右侧搜索按钮才落成这里(与文件树搜索 `search_query` 同款
    /// "草稿→提交"模型)。
    search: String,
    /// 搜索框草稿(iced `text_input` 的 `value`)。
    search_draft: String,
    /// 搜索框是否持有 iced 内部真实焦点。**不是**应用层手动置位的镜像——
    /// 每帧渲染循环里 `CaptureTodoSearchFocus` 问一遍 iced 真相后立刻写
    /// 进这里(`set_search_focused`),`main.rs` 键盘路由读它决定要不要
    /// 放行给标准 iced 管线。
    search_focused: bool,
    dispatch_open: Option<usize>,
    /// 派发选择层浮层弹出锚点(逻辑像素,取点击"指派"按钮时的光标位置)。
    /// 窗口级 overlay 靠它定位到按钮旁边;关闭时清空。
    dispatch_anchor: Option<(f32, f32)>,
    /// 日历日期选择器展开态(卡片下标),`None` = 未展开。跟 `dispatch_open`
    /// 同一种"同时只能有一个"模型。
    calendar_open: Option<usize>,
    /// 日历当前展示的 (年, 月)。打开时初始化成当前月,上一月/下一月导航
    /// 只改这个视图态,不落盘。
    calendar_view: (i32, u32),
    /// 日历浮层弹出锚点(逻辑像素,取点击日历按钮时的光标位置)。窗口级
    /// overlay 靠它定位到按钮旁边;关闭时清空。
    calendar_anchor: Option<(f32, f32)>,
    /// 任务内容行内编辑态(卡片下标, 草稿)。点卡片任务文字进入,失焦或
    /// 回车落盘改写任务文字(`commit_content_edit`)。草稿用
    /// `iced_widget::text_editor::Content`(多行,随内容自然撑高),不再用
    /// 单行 `String`——编辑态的输入框改走 `byteui::form::text_area`。
    editing_content: Option<(usize, iced_widget::text_editor::Content)>,
    /// 任务内容编辑框是否持有 iced 内部真实焦点,每帧由
    /// `CaptureContentEditFocus` 写入。
    content_edit_focused: bool,
    /// 一次性标记:`editing_content` 刚从 `None` 变成 `Some`(点卡片文字
    /// 刚触发编辑)时置真,main.rs 渲染循环取走后用 `operation::
    /// focusable::focus` 强制聚焦(同 `files::tree_edit_focus_pending`
    /// 的既有手法——点卡片文字这个点击落在旧的文字 `MouseArea` 上,不是
    /// 新出现的 `text_input` 本身,不会自动带焦点)。
    content_edit_focus_pending: bool,
    /// MARKDOWN 视图是否处于整文件编辑态(main.rs 键盘路由用)。
    markdown_editing: bool,
    /// MARKDOWN 编辑草稿:进入编辑态时从 `.dozer/todo.md` 全文载入,失焦
    /// 写盘、Esc 丢弃。
    markdown_draft: String,
    /// 鼠标拖拽排序进行态(`None` = 没在拖)。见 `TodoDrag`。视图层据此对
    /// 待办子序列做展示置换并改光标为抓取态；落盘只在 `DragEnd` 时一次性
    /// 发生。已完成任务不可拖动(见 `RowSelect`/`DragMove` 的不变量)。
    drag: Option<TodoDrag>,
}

impl WorkspaceState {
    /// 派发选择层是否打开(内核 `App::todo_dispatch_open` 键盘/UI 状态查询用)。
    pub fn dispatch_popup_open(&self) -> bool {
        self.dispatch_open.is_some()
    }

    /// 日历日期选择器是否打开(内核 `App::todo_calendar_open` 键盘 Esc
    /// 关闭用,同 `dispatch_popup_open` 的既有模式)。
    pub fn calendar_popup_open(&self) -> bool {
        self.calendar_open.is_some()
    }

    /// 开场新增任务选中闪光:`selected_row` 置为新增后的下标并保持
    /// `ADD_SELECT_HIGHLIGHT`;同时置滚动位,让列表滚回顶部使新任务可见。
    /// `scroll_to_top` 由 main.rs 下一帧 `interface.operate` 消费(一次性)。
    fn start_flash(&mut self, idx: usize) {
        self.selected_row = Some(idx);
        self.flash = Some(Flash {
            idx,
            until: std::time::Instant::now() + ADD_SELECT_HIGHLIGHT,
        });
        self.scroll_to_top = true;
    }

    /// 距新增闪光自动清除的剩余时间:main.rs 据此排下次唤醒,做到"恰好 2s
    /// 才重绘一次清除高亮",不空转也不延迟(同 `App::next_tooltip_wake` 的
    /// 定时范式)。未在闪光或已到点返回 `None`。
    pub fn next_flash_wake(&self) -> Option<std::time::Duration> {
        self.flash
            .and_then(|f| ADD_SELECT_HIGHLIGHT.checked_sub(f.until.elapsed()))
    }

    /// 推进闪光倒计时:到点且用户尚未手动改选就清除 `selected_row` 高亮,
    /// 同时熄灭闪光。每帧由 main.rs `new_events` 调用(见 `App::advance_flash`)。
    pub fn advance_flash(&mut self) {
        let Some(flash) = self.flash else {
            return;
        };
        if flash.until <= std::time::Instant::now() {
            if self.selected_row == Some(flash.idx) {
                self.selected_row = None;
            }
            self.flash = None;
        }
    }

    /// 取走"滚回列表顶部"的一次性滚动位并复位(供 main.rs 渲染循环在
    /// `interface.operate` 前查询)。返回 `true` 表示本帧应执行一次滚动。
    pub fn take_scroll_to_top(&mut self) -> bool {
        let pending = self.scroll_to_top;
        self.scroll_to_top = false;
        pending
    }

    /// 只读当前已解析的任务列表,给内核派发(`DispatchToExisting`)时按
    /// 下标取任务文本用。
    pub fn items(&self) -> &[TodoItem] {
        &self.items
    }

    /// 反查:这个 `session_id` 是不是某条 Todo 任务派发出来的会话,是的话
    /// 返回该任务原文——给 Agent 卡片"当前工作内容"当主选数据源用
    /// (`agent_card`)。`TodoTaskMeta` 只存哈希后的 `todo_line_key`,不存
    /// 原文,所以要拿着内存里的 `items` 逐条算 key 去 `app_meta` 里核对
    /// `dispatch.session_id`,O(n) 扫描,n 是任务条数(通常几十条以内,
    /// 每帧调一次不构成性能问题,不值得为它单独建反向索引)。没有任何
    /// 任务派发到这个 session(手动开的终端/agent)时返回 `None`,调用方
    /// 按既定口径 fallback 到 transcript 最后活动摘要。
    pub fn task_title_for_session<'a>(
        &'a self,
        app_meta: &AppState,
        project_id: i64,
        session_id: &str,
    ) -> Option<&'a str> {
        self.items
            .iter()
            .find(|item| {
                app_meta
                    .meta_for(project_id, todo_line_key(&item.text))
                    .and_then(|m| m.dispatch.as_ref())
                    .is_some_and(|d| d.session_id == session_id)
            })
            .map(|item| item.text.as_str())
    }

    /// 关闭派发选择层(选中目标后,或 Esc)。
    pub fn close_dispatch_popup(&mut self) {
        self.dispatch_open = None;
        self.dispatch_anchor = None;
    }

    /// 记录派发选择层浮层弹出锚点(点击"指派"按钮时的光标逻辑坐标),供
    /// 窗口级 overlay 定位用。`app.rs::todo_message` 在 `DispatchOpen` 时写入。
    pub fn set_dispatch_anchor(&mut self, anchor: (f32, f32)) {
        self.dispatch_anchor = Some(anchor);
    }

    /// 关闭日历选择器(选中日期后,或 Esc / 点外部)。
    pub fn close_calendar_popup(&mut self) {
        self.calendar_open = None;
        self.calendar_anchor = None;
    }

    /// 记录日历浮层弹出锚点(点击按钮时的光标逻辑坐标),供窗口级 overlay
    /// 定位用。`app.rs::todo_message` 在 `CalendarOpen` 时写入。
    pub fn set_calendar_anchor(&mut self, anchor: (f32, f32)) {
        self.calendar_anchor = Some(anchor);
    }

    /// 上次成功读取时 `.dozer/todo.md` 的 mtime,轮询靠比较它决定要不要
    /// 重读(`App::poll_todo_if_visible`)。`None` = 还没读过,或文件不存在。
    pub fn mtime(&self) -> Option<std::time::SystemTime> {
        self.mtime
    }

    /// 搜索框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn search_focused(&self) -> bool {
        self.search_focused
    }

    /// 每帧渲染循环读走 `CaptureTodoSearchFocus` 查到的真实焦点态后写
    /// 进来。
    pub fn set_search_focused(&mut self, focused: bool) {
        self.search_focused = focused;
    }

    /// 新增任务框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn add_focused(&self) -> bool {
        self.add_focused
    }

    /// 每帧渲染循环读走 `CaptureAddFocus` 查到的真实焦点态后写进来。
    pub fn set_add_focused(&mut self, focused: bool) {
        self.add_focused = focused;
    }

    /// 新增任务框有效高度:0(默认/未拖过)按最小高兜底,避免每帧重建时
    /// 渲染成 0 高框。见 `add_input_height` 字段注释。
    pub fn add_input_height(&self) -> f32 {
        self.add_input_height.max(ADD_INPUT_MIN_HEIGHT)
    }

    /// 拖拽置高(`app.rs::RowDrag` 的 `TodoAddGrow` 分支写回),钳到
    /// `[ADD_INPUT_MIN_HEIGHT, ADD_INPUT_MAX_HEIGHT]`。
    pub fn set_add_input_height(&mut self, h: f32) {
        self.add_input_height = h.clamp(ADD_INPUT_MIN_HEIGHT, ADD_INPUT_MAX_HEIGHT);
    }

    /// 任务内容编辑框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn content_edit_focused(&self) -> bool {
        self.content_edit_focused
    }

    /// 每帧渲染循环读走 `CaptureContentEditFocus` 查到的真实焦点态后写
    /// 进来。**只更新焦点镜像标记,不做落盘/丢弃判断**——是否该落盘取决
    /// 于"有没有打开的项目",`WorkspaceState` 自己拿不到 `project_path`,
    /// 这个判断在 `App::set_todo_content_focused` 里做(见 main.rs 接线
    /// 部分)。
    pub fn set_content_edit_focused_flag(&mut self, focused: bool) {
        self.content_edit_focused = focused;
    }

    /// 读走(消费式)一次性聚焦标记,同 `files::take_tree_edit_focus_pending`
    /// 的既有手法。
    pub fn take_content_edit_focus_pending(&mut self) -> bool {
        std::mem::take(&mut self.content_edit_focus_pending)
    }

    /// 失焦退出任务内容编辑态(`App::set_todo_content_focused` 无项目时用):
    /// 直接丢弃半输入。内容编辑是点卡片文字才弹出的一次性行内编辑,行为对齐
    /// 项目树重命名(`cancel_tree_edit`)而不是搜索框。
    pub fn cancel_content_edit(&mut self) {
        self.editing_content = None;
    }

    /// 失焦退出任务内容编辑态并**写盘保存**(与回车提交同一条
    /// `commit_content_edit` 落盘路径):改动且非空才写,否则丢弃。
    /// `App::set_todo_content_focused` 在真实焦点从真变假那一刻走这条,让
    /// "点别处"也等价于"按回车提交",不丢用户刚改的任务文字(见用户反馈:
    /// 内容编辑失焦应保存)。
    pub fn commit_content_edit(&mut self, project_path: &std::path::Path) {
        commit_content_edit(self, project_path);
    }

    /// MARKDOWN 视图是否处于整文件编辑态(main.rs 键盘路由用)。
    pub fn markdown_editing(&self) -> bool {
        self.markdown_editing
    }

    /// 失焦退出 MARKDOWN 编辑态(`Workspace::blur_inputs` 用):把草稿写回
    /// `.dozer/todo.md` 并刷新列表(整文件编辑没有"回车提交",失焦即提交;
    /// Esc 才是丢弃,见 `MarkdownEvent(Cancel)`)。
    pub fn cancel_markdown_edit(&mut self, project_path: &std::path::Path) {
        if !self.markdown_editing {
            return;
        }
        self.markdown_editing = false;
        let path = todo_path(project_path);
        if let Err(e) = std::fs::write(&path, &self.markdown_draft) {
            tracing::warn!("写入 todo.md 失败: {e}");
            return;
        }
        reload_from_disk(self, project_path);
    }

    /// 是否正在拖拽排序(main.rs 鼠标释放路由 + about_to_wait 持续重绘用)。
    pub fn drag_active(&self) -> bool {
        self.drag.is_some()
    }

    /// 取消进行中的拖拽排序(失焦/切面板时清状态,避免卡在拖拽中间)。
    pub fn cancel_drag(&mut self) {
        self.drag = None;
    }

    /// 草稿落成为生效的 `search` 过滤词(回车 / 点右侧搜索按钮时调用)。
    /// 焦点仍留在 iced `text_input` 上,不主动清空草稿(与 Files 搜索框
    /// Stage 2 一致)。
    pub fn commit_search(&mut self) {
        self.search = self.search_draft.clone();
    }

    /// 生效词和草稿都清空(切分类时调用,见 `Message::FilterSet` 处理器)——
    /// 只清 `search` 会留下草稿里的旧关键词,用户以为搜索框已经清空,其实
    /// 再次回车/失焦提交时会把旧词重新落成生效过滤,不是真正的重置。切
    /// 回原来那个用关键词搜过的分类也一样清空,不做"记住每个分类各自的
    /// 搜索词"那套(需求原话:哪怕切回去也要重置)。
    pub fn clear_search(&mut self) {
        self.search.clear();
        self.search_draft.clear();
    }
}

/// 搜索框稳定的 iced widget id。
pub fn todo_search_field_id() -> Id {
    Id::new("todo-search-box")
}

static TODO_SEARCH_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式),同 `extensions::files::take_search_focused` 的
/// 消费式复位手法,避免搜索框不可见的帧卡死上一次 `true` 永久堵死终端
/// 键盘转发。
pub fn take_todo_search_focused() -> bool {
    std::mem::replace(&mut *TODO_SEARCH_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍。`traverse` 必须调用传入闭包(见
/// [[dozer-operation-traverse-noop-bug]]——同 `extensions::files::
/// CaptureSearchFocus` 修复过的手法,这里从一开始就写对)。
pub struct CaptureTodoSearchFocus;
impl Operation<()> for CaptureTodoSearchFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&todo_search_field_id()) {
            *TODO_SEARCH_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
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

/// Todo 面板自己的消息类型。`DispatchToExisting` 涉及终端
/// 会话读写,内核在到达 `update` 之前就会拦截处理,不会真的传进
/// `update`——传进来会 `unreachable!`(同 Git Log 试点 `LoadMore` 的
/// 处理方式)。
#[derive(Debug, Clone)]
pub enum Message {
    Toggle(usize),
    /// 新增任务框编辑事件(iced `text_editor::on_action`,真正的
    /// `Action` 由组件自己产生,应用层只负责 `content.perform(action)`
    /// 落地——光标/选区/IME 全部交给 iced,不再是"文本/退格"这种自绘
    /// 事件分解)。
    AddEdit(iced_widget::text_editor::Action),
    /// 点新增任务框右侧 circle-arrow-up 提交按钮:把草稿落盘成新任务
    /// (与 `text_editor` 内置的 Enter 换行不冲突——提交只走按钮,不认
    /// 回车,见 `todo_footer_bar` 的按钮说明不变)。
    AddSubmit,
    /// 点新增任务框顶部的拖拽手柄:只在 app 层接管(`todo_message` 里置
    /// `dragging_row = TodoAddGrow`),真正的高度换算发生在 `app.rs::update`
    /// 的 `RowDrag` 分支——和 `Divider`/`RowDivider` 那套拖拽同构,只是目标
    /// 状态落在 `WorkspaceState::add_input_height` 而非 `PanelDims`。`todo::
    /// update` 收不到这条(早退),这里仍给个 no-op arm 保持 match 穷尽。
    AddResizeStart,
    FilterSet(TodoFilter),
    ViewModeSet(TodoViewMode),
    RowSelect(Option<usize>),
    /// 搜索框草稿变化(iced `text_input::on_input`,每次给全量当前字符串)。
    SearchInput(String),
    /// 回车 / 点右侧搜索按钮:把草稿落成生效的 `search` 过滤词。
    SearchSubmit,
    /// 内容侧"收起/展开列表列"按钮:内核拦截,不进 `update`——转发成顶层
    /// `Message::TogglePanelListCollapse(PanelKind::Todo)`(见 app.rs)。
    ToggleListCollapse,
    /// 新增框 / 搜索框 / 内容编辑框被右键:内核拦截,不进 `update`——转发成
    /// 顶层 `Message::TextInputMenuOpen` 弹出通用输入框右键菜单(见 app.rs)。
    TextInputMenuOpen(crate::app::TextInputTarget),
    /// 光标移动到了第 `idx` 个任务卡片上(由 `todo_card` 外层的
    /// `MouseArea::on_move` 构造)。若当前正在拖拽待办,更新目标位
    /// `target_idx`(悬停到已完成卡片时夹到待办块末尾,见 `update`)。
    /// 只有"正在拖"时才生效,纯悬停不会动任何东西。
    DragMove(usize),
    /// 松开左键,结束拖拽并把新顺序写盘(`move_pending_to` + reload)。
    /// 构造方为 main.rs 的 `MouseInput{Released}` 分支(同 `TabDragEnd`)。
    /// `source_idx == target_idx`(没真移动过)是 no-op,不写盘。
    DragEnd,
    DispatchOpen(usize),
    DispatchClose,
    DispatchToExisting(usize, String),
    /// 点卡片计划日期徽章 → 弹出日历日期选择器(取代原来的行内文本编辑)。
    CalendarOpen(usize),
    /// 关闭日历选择器(Esc / 点外部 / 选中日期后)。
    CalendarClose,
    /// 日历上一月 / 下一月导航。
    CalendarPrevMonth,
    CalendarNextMonth,
    /// 日历里选中某一天,`day` 是 "MM-DD" 文本(与 `plan_date` 存储格式一致)。
    CalendarPick(usize, String),
    /// 点卡片任务文字 → 进入内容行内编辑态(`editing_content` 置位 +
    /// `content_edit_focus_pending` 置位,main.rs 据此程序化聚焦)。
    ContentEditStart(usize),
    /// 内容编辑框草稿变化(iced `text_editor::on_action`,真正的 `Action`
    /// 由组件自己产生,应用层只负责 `content.perform(action)` 落地——光标/
    /// 选区/IME 全部交给 iced,与 `AddEdit` 同构)。回车(`Edit::Enter`)在
    /// update 里被拦截为提交:落盘改写任务文字,与失焦落盘共用
    /// `commit_content_edit` 一条路径。
    ContentEdit(iced_widget::text_editor::Action),
    /// 点 MARKDOWN 视图主体 → 进入整文件编辑态(`markdown_editing` 置位)。
    MarkdownEditStart,
    /// MARKDOWN 编辑态下的按键:`Text`(含回车翻成的 `"\n"`)/`Backspace`
    /// 改草稿,`Cancel` 丢弃退出;提交走失焦 `cancel_markdown_edit` 写盘
    /// (回车是换行不是提交——main.rs 把回车翻成 `Text("\n")`)。
    MarkdownEvent(AddrEvent),
    /// 悬停某张任务卡(由 `todo_card` 外层的 `MouseArea::on_enter/on_exit`
    /// 构造),转交内核的悬停动画表(`app.rs::set_hover`),与全应用其它卡片
    /// 用同一套 hover 机制。
    Hover(HoverId, bool),
    /// 点 footbar 的"清空列表"按钮。**功能尚未实现**:`update` 里是
    /// no-op,仅占位——UI 已就位,后续接入清空逻辑时在此落地。
    ClearList,
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

/// `Toggle` 的写盘逻辑：把任务行的 `[ ]`/`[x]` 改成
/// `target_done` 对应的目标值(不是翻转)。`item.done == target_done` 时
/// 直接 no-op 返回,不读写文件、不碰 `completed_at`。
fn set_done(
    ws_state: &mut WorkspaceState,
    app_state: &mut AppState,
    idx: usize,
    project_id: i64,
    project_path: &std::path::Path,
    target_done: bool,
) {
    let Some(item) = ws_state.items.get(idx) else {
        return;
    };
    if item.done == target_done {
        return;
    }
    let old_line = format!("- [{}] {}", if item.done { "x" } else { " " }, item.text);
    let new_line = format!("- [{}] {}", if target_done { "x" } else { " " }, item.text);
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
    // 文本没变(正常场景)才更新 completed_at;如果文本变了(文件可能在
    // 重读期间被 agent 并发改过),跳过,避免把完成时间错记到另一条任务上。
    if let Some(after) = ws_state.items.get(idx)
        && after.text == before_text
    {
        app_state.set_completed_at(project_id, &after.text, after.done);
    }
}

/// 任务内容编辑/添加框的真 `text_input`/`text_editor` 的 `widget::Id`。
/// `add_field_id` 由 `CaptureAddFocus` 的 `focusable` 钩子匹配真实焦点态
/// (见 `CaptureAddFocus`);`content_field_id` 挂任务内容编辑框的真
/// `text_input` 上,由 `CaptureContentEditFocus` 匹配,供键盘路由问焦点、
/// 鼠标点击与 `ContentEditStart` 触发后的一帧程序化聚焦使用。
pub fn add_field_id() -> Id {
    Id::new("todo-add-field")
}
pub fn content_field_id() -> Id {
    Id::new("todo-content-field")
}

/// 每帧 `interface.operate` 把字段屏幕 bounds 写进来,鼠标点击时读取。
static CONTENT_EDIT_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式)任务内容编辑框上一帧是否持有 iced 内部真实焦点,同
/// `files::take_tree_edit_focused` 的桥接手法(含消费式复位,避免编辑框
/// 不可见的帧卡死上一次 `true` 永久堵死终端键盘转发)。
pub fn take_content_edit_focused() -> bool {
    std::mem::replace(&mut *CONTENT_EDIT_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍,把命中 `content_field_id` 的真
/// `text_input` 当前是否持有 iced 焦点写进 `CONTENT_EDIT_FOCUSED`。
/// `traverse` 必须调用传入的 `operate` 闭包才能继续递归子节点(同
/// `extensions::files::CaptureSearchFocus` 修复过的容器跳过问题)。
pub struct CaptureContentEditFocus;
impl Operation<()> for CaptureContentEditFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&content_field_id()) {
            *CONTENT_EDIT_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

static ADD_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式),同 `extensions::files::take_search_focused` 的
/// 消费式复位手法,避免新增框不可见的帧卡死上一次 `true` 永久堵死终端
/// 键盘转发。
pub fn take_add_focused() -> bool {
    std::mem::replace(&mut *ADD_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍,把 `add_field_id`(真正的
/// `text_editor`)的真实焦点态问进 `static`。`traverse` 必须调用传入闭包(见
/// [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureAddFocus;
impl Operation<()> for CaptureAddFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&add_field_id()) {
            *ADD_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

/// `AddEvent(Submit)` 的写盘逻辑:把草稿追加成新任务行,空白草稿
/// (trim 后)no-op。
fn commit_add_task(ws_state: &mut WorkspaceState, project_path: &std::path::Path) {
    let text = ws_state.add_draft.text();
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    let path = todo_path(project_path);
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let new_content = prepend_todo_item(&content, &text);
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
    ws_state.add_draft = iced_widget::text_editor::Content::new();
    reload_from_disk(ws_state, project_path);
    // 新任务置顶落盘后:选中并滚动到列表顶部,保持 2 秒的选中态提示用户
    // "这条就是刚加的"。
    let flash_idx = ws_state
        .items
        .iter()
        .position(|item| item.text == text)
        .unwrap_or(0);
    ws_state.start_flash(flash_idx);
}

/// 任务内容提交(`ContentEdit` 回车的写盘逻辑):把草稿改写进 `.dozer/todo.md`
/// 里对应的任务行(文本变了才写),并刷新列表。空白草稿(trim 后)丢弃不写。
fn commit_content_edit(ws_state: &mut WorkspaceState, project_path: &std::path::Path) {
    let Some((idx, draft)) = ws_state.editing_content.clone() else {
        return;
    };
    let new_text = draft.text().trim().to_string();
    if new_text.is_empty() {
        ws_state.editing_content = None;
        return;
    }
    let Some(item) = ws_state.items.get(idx) else {
        ws_state.editing_content = None;
        return;
    };
    if item.text == new_text {
        ws_state.editing_content = None;
        return;
    }
    let old_line = format!("- [{}] {}", if item.done { "x" } else { " " }, item.text);
    let new_line = format!("- [{}] {}", if item.done { "x" } else { " " }, new_text);
    let path = todo_path(project_path);
    let Ok(content) = std::fs::read_to_string(&path) else {
        ws_state.editing_content = None;
        return;
    };
    match replace_todo_line(&content, &old_line, &new_line) {
        Some(new_content) => {
            if let Err(e) = std::fs::write(&path, &new_content) {
                tracing::warn!("写入 todo.md 失败: {e}");
            }
        }
        None => {
            // 冲突:文件已经变了,放弃这次写入,直接重读展示最新状态。
        }
    }
    ws_state.editing_content = None;
    reload_from_disk(ws_state, project_path);
}

/// 本地时区无关的"今天" (年, 月, 日),用 `SystemTime::now()` 的 UTC 秒数
/// 经 `civil_from_days` 换算。只用于日历默认停在当前月,时区偏差一天内无感。
fn today_ymd() -> (i32, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86400) as i64;
    let (y, m, d) = civil_from_days(days);
    (y as i32, m, d)
}

/// 解析 "MM-DD"(允许 `8-3` 这种缺前导零写法,兼容用户手敲的计划日期),
/// 返回 (月, 日);解析失败返回 `None`。
fn parse_month_day(s: &str) -> Option<(u32, u32)> {
    let (mm, dd) = s.split_once('-')?;
    let m: u32 = mm.trim().parse().ok()?;
    let d: u32 = dd.trim().parse().ok()?;
    (1..=12).contains(&m).then_some((m, d))
}

/// `civil_from_days` 的逆运算:把 (年, 月, 日) 换算回"自 1970-01-01 的天数",
/// 给日历算"某月 1 号是星期几"和"某月有多少天"用。只覆盖 1970..=2100,
/// 与 `civil_from_days` 同范围。
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64;
    let m = m as i64;
    let d = d as i64;
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// 某年某月有多少天。
fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
            if leap { 29 } else { 28 }
        }
        _ => 0,
    }
}

/// 某年某月 1 号是星期几:0 = 周日,1 = 周一 … 6 = 周六(1970-01-01 是周四)。
fn first_weekday_of_month(y: i32, m: u32) -> u32 {
    (days_from_civil(y, m, 1) + 4).rem_euclid(7) as u32
}

/// 处理除 `DispatchToExisting` 之外的消息,统一接收两块
/// 状态——`Toggle`/`CalendarPick`/`ContentEdit` 需要读写
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
        // 卡片悬停由内核 `App::update` 拦截转发到 `set_hover`,不会到这。
        Message::Hover(_, _) => {}
        // footbar"清空列表"按钮:功能尚未实现,仅占位。
        Message::ClearList => {}
        // `TextInputMenuOpen` 由内核拦截映射为右键菜单,不进这里。
        Message::TextInputMenuOpen(_) => {}
        // `ToggleListCollapse` 由内核拦截映射为列表列收起/展开,不进这里。
        Message::ToggleListCollapse => {}
        Message::Toggle(idx) => {
            let Some(item) = ws_state.items.get(idx) else {
                return;
            };
            let target = !item.done;
            set_done(ws_state, app_state, idx, project_id, project_path, target);
        }
        Message::AddEdit(action) => ws_state.add_draft.perform(action),
        Message::AddSubmit => commit_add_task(ws_state, project_path),
        // 高度拖拽在 app 层 `todo_message` 已早退,不会到这里;保留 arm 仅
        // 为 match 穷尽。
        Message::AddResizeStart => {}
        Message::FilterSet(f) => {
            ws_state.filter = f;
            // 切分类重置搜索关键词过滤(需求:哪怕切回原来那个用关键词
            // 搜过的分类也要重置,不做"记住每个分类各自搜索词"那套)。
            ws_state.clear_search();
        }
        Message::ViewModeSet(m) => ws_state.view_mode = m,
        Message::RowSelect(idx) => {
            ws_state.selected_row = idx;
            // 用户手动选中(点卡片空白处)会打断"新增闪光":否则 2 秒计时到点
            // 会把用户刚主动选的卡片又自动取消选中。`take_scroll_to_top` 未定
            // 时用户主动点才会走到这里,新增闪光阶段不处理(见 `Message::Toggle`
            // 之上对闪光来源的约定)。
            ws_state.flash = None;
            // 待办卡片被按下即"准备拖":记下它的 item-index 作为拖拽源。
            // 已完成不参与拖拽(只有待办才进 `drag`)。注意这跟选中态是两件
            // 独立的事——纯点击(不移动)也会落到这里,但松手时
            // source==target 不写盘,只是正常选中切换(同 `TabDragMove`
            // 的"按住=准备拖,移动才换位"语义)。
            if let Some(i) = idx
                && let Some(item) = ws_state.items.get(i)
                && !item.done
            {
                ws_state.drag = Some(TodoDrag {
                    source_idx: i,
                    target_idx: i,
                });
            }
        }
        Message::SearchInput(s) => ws_state.search_draft = s,
        Message::SearchSubmit => ws_state.commit_search(),
        Message::DragMove(over_idx) => {
            // 只有"正在拖"才生效;纯悬停不会动任何东西。
            let Some(drag) = ws_state.drag else {
                return;
            };
            // 悬停到待办卡片 → 目标取该卡片 item-index;悬停到已完成卡片
            // → 目标夹到待办块末尾(usize::MAX 哨兵,`DragEnd` 时折算成
            // 最后一个待办 rank)。已完成不可被拖到(只会改变落点)。
            let target = match ws_state.items.get(over_idx) {
                Some(it) if !it.done => over_idx,
                _ => usize::MAX,
            };
            if target != drag.target_idx {
                ws_state.drag = Some(TodoDrag {
                    source_idx: drag.source_idx,
                    target_idx: target,
                });
            }
        }
        Message::DragEnd => {
            let Some(drag) = ws_state.drag.take() else {
                return;
            };
            if drag.source_idx == drag.target_idx {
                return; // 没真移动过(纯点击),no-op
            }
            // 把两端的 item-index 折算成文件里的待办 rank(只在写盘时算一次)。
            let total_pending = ws_state.items.iter().filter(|it| !it.done).count();
            let pending_rank = |items: &[TodoItem], idx: usize| -> Option<usize> {
                items
                    .iter()
                    .enumerate()
                    .filter(|(_, it)| !it.done)
                    .position(|(i, _)| i == idx)
            };
            let Some(source_rank) = pending_rank(&ws_state.items, drag.source_idx) else {
                return;
            };
            let target_rank = if drag.target_idx == usize::MAX {
                total_pending.saturating_sub(1)
            } else {
                match pending_rank(&ws_state.items, drag.target_idx) {
                    Some(r) => r,
                    None => total_pending.saturating_sub(1),
                }
            };
            if source_rank == target_rank {
                return;
            }
            let path = todo_path(project_path);
            let Ok(content) = std::fs::read_to_string(&path) else {
                return;
            };
            if let Some(new_content) = move_pending_to(&content, source_rank, target_rank)
                && std::fs::write(&path, new_content).is_ok()
            {
                reload_from_disk(ws_state, project_path);
                // 拖拽完成后保持被拖任务选中:重排后它位于待办块第 `target_rank`
                // 位(`move_pending_to` 把源从 `source_rank` 摘出插到
                // `target_rank`,落点恒为该 rank),按重载后的 `items` 找回它的
                // 新 item-index 更新 `selected_row`。选中的是"被拖的那条"而
                // 不是它挪走后占住源位/目标位的邻居。
                ws_state.selected_row = ws_state
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, it)| !it.done)
                    .nth(target_rank)
                    .map(|(i, _)| i);
            }
        }
        Message::DispatchOpen(idx) => ws_state.dispatch_open = Some(idx),
        Message::DispatchClose => ws_state.dispatch_open = None,
        Message::CalendarOpen(idx) => {
            // 打开日历:默认停在"当前月",若任务已有计划日期且能解析成 MM-DD,
            // 则把视图拨到该月(年份取当前年——plan_date 只有月日,无年份)。
            let (now_y, now_m, _) = today_ymd();
            let (y, m) = ws_state
                .items
                .get(idx)
                .and_then(|item| {
                    let key = todo_line_key(&item.text);
                    app_state
                        .meta_for(project_id, key)
                        .and_then(|m| m.plan_date.clone())
                })
                .and_then(|s| parse_month_day(&s))
                .map(|(mm, _)| (now_y, mm))
                .unwrap_or((now_y, now_m));
            ws_state.calendar_open = Some(idx);
            ws_state.calendar_view = (y, m);
        }
        Message::CalendarClose => ws_state.close_calendar_popup(),
        Message::CalendarPrevMonth => {
            let (y, m) = ws_state.calendar_view;
            ws_state.calendar_view = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
        }
        Message::CalendarNextMonth => {
            let (y, m) = ws_state.calendar_view;
            ws_state.calendar_view = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
        }
        Message::CalendarPick(idx, day) => {
            if let Some(text) = ws_state.items.get(idx).map(|item| item.text.clone()) {
                app_state.set_plan_date(project_id, &text, day);
            }
            ws_state.close_calendar_popup();
        }
        Message::ContentEditStart(idx) => {
            // 已经在编辑同一张卡:保持草稿(重击只用于鼠标定位,不重置,
            // 否则点一下就把刚改了一半的文字丢掉)。新卡进入编辑态则重置草稿。
            if ws_state
                .editing_content
                .as_ref()
                .is_some_and(|(eidx, _)| *eidx == idx)
            {
                return;
            }
            if let Some(item) = ws_state.items.get(idx) {
                ws_state.editing_content = Some((
                    idx,
                    iced_widget::text_editor::Content::with_text(&item.text),
                ));
            }
            // 点卡片文字这个点击落在旧的文字 `MouseArea` 上,真 `text_editor`
            // 下一帧才出现、不会自己拿聚焦,置位一次性聚焦标记。
            ws_state.content_edit_focus_pending = true;
        }
        Message::ContentEdit(action) => {
            // 内容编辑器内回车=提交(与失焦落盘共用 `commit_content_edit`):
            // `text_editor` 把 Enter 报成 `Edit::Enter`,在这里就地落盘,不再
            // 走单独的 `ContentSubmit` 消息(update() 无返回值,无法重发消息)。
            if matches!(
                action,
                iced_widget::text_editor::Action::Edit(iced_widget::text_editor::Edit::Enter)
            ) {
                commit_content_edit(ws_state, project_path);
                return;
            }
            if let Some((_, content)) = ws_state.editing_content.as_mut() {
                content.perform(action);
            }
        }
        Message::MarkdownEditStart => {
            let path = todo_path(project_path);
            ws_state.markdown_draft = std::fs::read_to_string(&path).unwrap_or_default();
            ws_state.markdown_editing = true;
        }
        Message::MarkdownEvent(ev) => {
            if !ws_state.markdown_editing {
                return;
            }
            match ev {
                AddrEvent::Text(s) => ws_state.markdown_draft.push_str(&s),
                AddrEvent::Backspace => {
                    ws_state.markdown_draft.pop();
                }
                // 回车在 main.rs 已被翻成 `Text("\n")`(多行文本换行),这里
                // `Submit` 只兜底(理论不到),同样当换行处理。
                AddrEvent::Submit => ws_state.markdown_draft.push('\n'),
                AddrEvent::Cancel => {
                    ws_state.markdown_editing = false;
                }
            }
        }
        Message::DispatchToExisting(..) => {
            unreachable!(
                "DispatchToExisting 由内核在 Message::Todo 分支里直接处理\
                 (需要终端会话读写能力),不会转发到这里"
            )
        }
    }
}

/// Todo 面板渲染成两个独立的边框 pane(镜像 Files/Project 面板已有的
/// "侧栏 + 内容区，中间一条可拖拽分隔线"两栏模式，不再是单个面板内部一个
/// `row![sidebar, body]`)——调用方(`app.rs` 的 `PanelKind::Todo` 分支)负责
/// 拼 `row![sidebar_pane, divider_bar(Divider::TodoSplit, ..), content_pane]`。
/// 左栏：面板头 + 分类导航。右栏：列表/MARKDOWN 视图切换 tab + 视图
/// 主体。
#[allow(clippy::too_many_arguments)]
pub fn view<'a>(
    app_state: &'a AppState,
    app: &App,
    ws_state: &'a WorkspaceState,
    ws: &Workspace,
    project_id: i64,
    project_path: Option<&Path>,
    sidebar_width: Length,
    sidebar_outer: Border,
    content_width: Length,
    content_outer: Border,
) -> (
    Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
) {
    // ---- header（挂在左栏，同 Project/Files 面板"头在列表侧"的既有惯例） ----
    // 内边距对齐文件树面板(body 用 `project_pane` region 的 `padding` 把头
    // 及其自带分割线整体内缩):不再用 `[20,20]` 额外撑高头部、也不让分割线
    // 被大 padding 顶下去,与文件面板头部高度/分割线位置一致。
    let header = container(crate::homespace::home_panel_head(
        icons::IconKind::ListTodo,
        "Todo",
    ))
    .padding(theme::region::project_pane().padding);

    // ---- 状态推导（一次算好，侧栏计数 + 列表渲染共用） ----
    let states: Vec<TodoState> = ws_state
        .items
        .iter()
        .map(|item| {
            let key = todo_line_key(&item.text);
            let dispatch = app_state
                .meta_for(project_id, key)
                .and_then(|m| m.dispatch.as_ref());
            let target_alive = dispatch
                .map(|d| {
                    ws.tabs
                        .iter()
                        .chain(ws.ssh_tabs.iter())
                        .any(|t| t.alive && t.info.id == d.session_id)
                })
                .unwrap_or(false);
            todo_display_state(item, dispatch, target_alive)
        })
        .collect();
    let counts = [
        (TodoFilter::All, ws_state.items.len()),
        (
            TodoFilter::Pending,
            states.iter().filter(|s| **s == TodoState::Pending).count(),
        ),
        (
            TodoFilter::InProgress,
            states
                .iter()
                .filter(|s| **s == TodoState::InProgress)
                .count(),
        ),
        (
            TodoFilter::Done,
            states.iter().filter(|s| **s == TodoState::Done).count(),
        ),
    ];

    // ---- 左栏 pane：header + 分类导航 + 底部「任务计数 / 清空列表」栏 ----
    // 原 content pane 右下角的 `todo_clear_footer_bar` 整体迁到左栏:分类导航
    // 之下、靠底。计数(左)与「清空列表」(右)不再堆在内容区右下方,改由左栏
    // 底部统一呈现——内容区底部只留「新增任务」输入框。
    let mut nav = column![].spacing(4).padding([12, 8]);
    for (filter, count) in counts {
        nav = nav.push(todo_category_button(filter, count, ws_state.filter));
    }
    let sidebar_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(
            column![
                header,
                nav,
                space::Space::new().height(Length::Fill),
                todo_clear_footer_bar(ws_state),
            ]
            .height(Length::Fill),
        )
        .width(sidebar_width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().bg.into()),
            border: sidebar_outer,
            ..container::Style::default()
        })
        .into();

    // ---- 右栏 pane：tab 段 + 视图主体 ----
    // 视图切换 tab 右侧钉一个"收起/展开列表列"按钮(内容侧,收起左列后仍在
    // 此可见以便恢复)。按钮消息为本地 `Message::ToggleListCollapse`,由内核
    // `App::update` 拦截转发成顶层 `Message::TogglePanelListCollapse`。
    let collapse = app.list_collapse_button(
        crate::app::PanelKind::Todo,
        app.list_collapsed(crate::app::PanelKind::Todo),
        crate::app::HoverId::TodoListCollapse,
        "收起",
        "展开",
        Message::ToggleListCollapse,
        move |hovered| Message::Hover(crate::app::HoverId::TodoListCollapse, hovered),
    );
    // 中间塞一块 `Fill` 空间把 `collapse` 顶到行右端,右缘对齐 `padding`
    // 的 20px 右内边距——跟下面任务卡片/搜索框的右侧留白(同为 20px)取平
    // (2026-08-28 用户反馈:改之前 collapse 紧贴在 tab 右边,没有跟卡片
    // 右对齐)。
    let tabs_bar = row![
        todo_view_tabs(ws_state.view_mode),
        space::Space::new().width(Length::Fill),
        collapse
    ]
    .width(Length::Fill)
    .align_y(iced_widget::core::Alignment::Center)
    .spacing(4)
    .padding([4, 20]);
    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match ws_state.view_mode {
            TodoViewMode::List => todo_list_view(app_state, app, ws_state, project_id, &states),
            TodoViewMode::Markdown => todo_markdown_view(project_path, ws_state),
        };
    let content_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(column![tabs_bar, crate::app::tab_divider(), body].height(Length::Fill))
            .width(content_width)
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().bg.into()),
                border: content_outer,
                ..container::Style::default()
            })
            .into();

    (sidebar_pane, content_pane)
}

/// 新增任务框高度上/下限(逻辑像素)。下限即默认高(约 5 行正文,保证多行
/// 任务内容可见);上限让列表区至少留出约 140px,且 `app.rs::RowDrag` 的
/// `TodoAddGrow` 分支会再按窗口高夹一道,这里给的是硬上限(窗口极矮时由
/// 那里兜底)。
pub const ADD_INPUT_MIN_HEIGHT: f32 = 120.0;
pub const ADD_INPUT_MAX_HEIGHT: f32 = 400.0;

/// 顶部宽 8px 的细窄拖拽手柄:把光标变 `ResizingRow`,按下经
/// `Message::AddResizeStart` 交给 app 层接管高度换算。视觉上只是顶边框上
/// 一道 1px 亮线(像输入框可被向上拉起的"抓手"),平时几乎隐形,拖拽时靠
/// 光标变化提示可拖。
fn todo_resize_handle<'a>() -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let grip = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..container::Style::default()
        });
    MouseArea::new(
        container(column![grip].spacing(0))
            .width(Length::Fill)
            .height(Length::Fixed(8.0)),
    )
    .interaction(mouse::Interaction::ResizingRow)
    .on_press(Message::AddResizeStart)
    .into()
}

/// 底部快速新建栏。结构对齐 `project.rs::project_footer_bar`(1px BORDER
/// 分隔线),但左右间距对齐任务卡片的 20px、输入框加高到约 3 行文字,**提交
/// 按钮嵌在输入框边框内**(右下方、无独立边框,只是框里一枚 circle-arrow-up
/// 图标——视觉上按钮"在输入框内",且始终贴输入框右下角)。框顶还有一道可向上拖的 8px 手柄
/// (`todo_resize_handle`),拉高输入框(高度落在 `WorkspaceState::
/// add_input_height`)。输入框已是真正的 iced `text_editor`(Stage 4 添加框
/// 迁移,与 Todo 搜索框同期),点击/光标/IME 全由原生管线接管;键盘路由靠
/// main.rs 每帧 `CaptureAddFocus` 问真实焦点态裁决。
fn todo_footer_bar<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let field = byteui::form::text_area::view(
        &ws_state.add_draft,
        "添加新任务",
        Some(add_field_id()),
        true,
        Some(ws_state.add_input_height()),
        Message::AddEdit,
    );
    let field = byteui::interaction::context_menu::wrap(
        field,
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: add_field_id(),
            secure: false,
        })),
    );

    // 提交按钮:circle-arrow-up,嵌在输入框右边框内、无独立边框(视觉上"在
    // 框里"),点它提交(与搜索框按钮同款,但不认回车——`text_editor` 内置
    // Enter 换行,回车不提交新任务)。它是输入框容器内右侧的按钮,会自己
    // 吃掉点击,不会把焦点让给别处。
    //
    // 图标配色对齐其它 icon 按钮(agent 面板"＋"、文件树搜索等):静止
    // DIM、hover 平滑过渡到 GOLD,由 `HoverId::TodoAddSubmit` + 外层
    // `MouseArea` 驱动同一套悬停动画(不再是恒 GOLD 的硬编码)。
    let submit = icons::icon_button_entry(
        icons::IconKind::CircleArrowUp,
        byteui::theme::icon_size::row(),
        false,
        false,
        app.hover_progress(HoverId::TodoAddSubmit),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::AddSubmit,
        |hovered| Message::Hover(HoverId::TodoAddSubmit, hovered),
        "提交",
    );

    // 输入框本体:单个带边框的容器,把"文字区 + 提交按钮"一起包进边框内。
    // 不再需要 `MouseArea`/`AddEditStart`——`text_editor` 是真控件,点击
    // 命中范围内就由 iced 标准鼠标管线自己处理聚焦(边框金/灰由下面
    // `editing = add_focused()` 驱动)。高度仍可经顶部拖拽手柄放大
    // (`add_input_height`,拖拽逻辑在 `app.rs::RowDrag`,本计划不改),
    // `text_area` 的 `height` 参数按这个值定高。内部一行
    // 两格:左格文字(占满高度、靠顶左对齐)、右格提交按钮(占满高度、靠底)。
    let editing = ws_state.add_focused();
    let input_box = container(
        row![
            container(field)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Top)
                .align_x(iced_widget::core::alignment::Horizontal::Left),
            container(submit)
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Bottom),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fixed(ws_state.add_input_height()))
    .padding([10, 12])
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: Some(byteui::theme::color::current().bg.into()),
        border: Border {
            color: if editing {
                byteui::theme::color::current().gold
            } else {
                byteui::theme::color::current().border
            },
            width: 1.0,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    });

    // 输入框自身已带 1px 边框,作为与列表区之间的唯一分割线;不再额外画
    // 一道 `top_line`,避免输入框上方出现两条并列分割线。
    container(column![todo_resize_handle(), input_box].spacing(4))
        .width(Length::Fill)
        .padding([8, 20])
        .into()
}

/// 左栏底部栏(位于分类导航之下、靠底):仅右侧"清空列表"按钮,不再展示
/// 左侧任务计数与图标。已从 content pane 右下角迁到左栏(见 `view`)。样式
/// 对齐 `files.rs` 的 `git_footer_bar`
/// (顶部分隔线 + 左图标/文案 + 右侧操作按钮)。**清空功能尚未实现**:
/// `清空列表` 走 `Message::ClearList`,在 `update` 里是 no-op,这里只负责
/// 把 UI 摆出来。
fn todo_clear_footer_bar<'a>(
    _ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let clear = button(
        row![
            icons::view(
                icons::IconKind::Trash,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().cream,
            ),
            text("清空列表")
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().cream),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .on_press(Message::ClearList)
    .padding([4, 8])
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: Some(byteui::theme::color::current().bg.into()),
        border: Border {
            color: byteui::theme::color::current().border,
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: byteui::theme::color::current().cream,
        ..button::Style::default()
    });

    let bar = row![space::horizontal(), clear]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center);

    let top_line = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..container::Style::default()
        });

    // 顶部分割线左右内缩对齐面板 header 的分割线(`home_panel_head` 被
    // `project_pane().padding` 整体内缩),否则底部 footbar 分割线会比头部
    // 的更长、两端对齐不上。竖直 6px 间距沿用原 `[6,0]` 的观感。
    let pp = theme::region::project_pane().padding;
    container(column![top_line, bar].spacing(4))
        .width(Length::Fill)
        .padding(Padding {
            top: 6.0,
            right: pp.right,
            bottom: 6.0,
            left: pp.left,
        })
        .style(|_t: &iced_widget::Theme| container::Style {
            background: None,
            ..container::Style::default()
        })
        .into()
}

/// 顶部搜索框:真正的 `byteui::form::input_text`,形状与 Files 搜索框
/// (Stage 2)一致。草稿 `draft` 是 `text_input::on_input` 给的全量字符串,
/// 焦点态由 `CaptureTodoSearchFocus` 每帧查、`main.rs` 据此放行键盘给
/// 标准 iced 管线。`active` = 列表正被 `search` 过滤时持续金框提示(同
/// Files `search_box` 的 `highlight`)。
fn todo_search_bar<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let highlight = ws_state.search_focused() || !ws_state.search.is_empty();
    let bar = byteui::form::search_box::view(
        "搜索任务…",
        &ws_state.search_draft,
        Some(todo_search_field_id()),
        highlight,
        Message::SearchInput,
        Message::SearchSubmit,
        app.hover_progress(HoverId::TodoSearchSubmit),
        |hovered| Message::Hover(HoverId::TodoSearchSubmit, hovered),
    );
    byteui::interaction::context_menu::wrap(
        bar,
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: todo_search_field_id(),
            secure: false,
        })),
    )
}

/// 列表视图主体：搜索栏 + 编号行列表 + 底部新增输入。
fn todo_list_view<'a>(
    app_state: &'a AppState,
    app: &App,
    ws_state: &'a WorkspaceState,
    project_id: i64,
    states: &[TodoState],
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let visible_idx = filter_todos(&ws_state.items, states, ws_state.filter, &ws_state.search);

    // 搜索框的水平/垂直间距对齐任务卡片的间距规格(卡片列表 `list` 是
    // `spacing(8)` + `padding([0, 20])`):左右 20、上下 8,不再贴边顶到
    // tab 分隔线与首张卡片。
    let search = container(todo_search_bar(app, ws_state))
        .padding([8, 20])
        .width(Length::Fill);

    let mut list = column![].spacing(8).padding([0, 20]);
    if visible_idx.is_empty() {
        list = list.push(
            container(
                text("没有匹配的任务")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().dim),
            )
            .padding([20, 20]),
        );
    } else {
        // 待办在前、已完成沉底:把 `visible_idx` 拆成两段,各自保持原(items)
        // 次序后拼接。与落盘时"待办块 + 已完成块"的归一化一致。
        let mut pending_idx: Vec<usize> = Vec::new();
        let mut done_idx: Vec<usize> = Vec::new();
        for &i in &visible_idx {
            if ws_state.items[i].done {
                done_idx.push(i);
            } else {
                pending_idx.push(i);
            }
        }
        // 拖拽进行中不再对 pending_idx 做展示置换——之前"每帧按新顺序
        // remove+insert 整个重排"会让被拖卡片之外的其它卡片瞬间跳位,
        // 没有任何过渡帧(用户反馈"动画不够流畅"的根因)。改成更常见的
        // "源卡片原位高亮 + 插入指示线"模式:待办子序列渲染顺序全程不变,
        // 被拖的那张卡片本身描边变金(`is_drag_source`),目标位置前插一条
        // 细的金色指示线提示"松手会落在这里"。真正的换位只在 `DragEnd`
        // 落盘时一次性发生,视觉上不再有中间态的"其它卡片被顶开"。
        let drag_source_idx = ws_state.drag.map(|d| d.source_idx);
        // 指示线该出现在 pending 子序列的哪个展示位置之前:target_idx 对应
        // 的卡片当前在 pending_idx 里的下标(`usize::MAX` 哨兵表示插到
        // pending 块末尾,单独用 insert_at_end 标记,不落进这个 Option)。
        // source_idx == target_idx(还没真的移动过)时不显示指示线,跟换位
        // 逻辑本身"没移动不写盘"的既有语义对齐。
        let (insert_before, insert_at_end) = match ws_state.drag {
            Some(drag) if drag.source_idx != drag.target_idx => {
                if drag.target_idx == usize::MAX {
                    (None, true)
                } else {
                    (
                        pending_idx.iter().position(|&x| x == drag.target_idx),
                        false,
                    )
                }
            }
            _ => (None, false),
        };
        let grabbing = ws_state.drag.is_some();
        let pending_len = pending_idx.len();
        for (display_no, &idx) in pending_idx.iter().enumerate() {
            if insert_before == Some(display_no) {
                list = list.push(drag_insert_indicator());
            }
            list = list.push(todo_list_row(
                app_state,
                app,
                ws_state,
                project_id,
                states,
                display_no + 1,
                idx,
                grabbing,
                drag_source_idx == Some(idx),
            ));
        }
        if insert_at_end || insert_before == Some(pending_len) {
            list = list.push(drag_insert_indicator());
        }
        for (i, &idx) in done_idx.iter().enumerate() {
            list = list.push(todo_list_row(
                app_state,
                app,
                ws_state,
                project_id,
                states,
                pending_len + i + 1,
                idx,
                grabbing,
                false,
            ));
        }
    }

    column![
        search,
        // 任务列表滚动条对齐全应用统一滚动条规范(几何 + 外观,见
        // `byteui::interaction::scrollbar`),不再是 iced 默认滚动条。`.id` 是新增任务后
        // "滚回顶部使新任务可见"的定位锚点(main.rs `interface.operate`
        // 拿这个 Id 发 `scrollable::scroll_to`,见 `App::take_todo_scroll_to_top`)。
        scrollable(list)
            .id(iced_widget::Id::new(TODO_LIST_SCROLL_ID))
            .height(Length::Fill)
            .direction(scrollable::Direction::Vertical(
                byteui::interaction::scrollbar::scrollbar(),
            ))
            .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
        todo_footer_bar(app, ws_state),
    ]
    .height(Length::Fill)
    .into()
}

/// MARKDOWN 视图:整文件编辑 `.dozer/todo.md`。未编辑时展示当前源码,
/// 点击主体进入编辑态(`MarkdownEditStart`);编辑态下按键经 main.rs 路由成
/// `MarkdownEvent`(回车=换行),失焦写盘(`cancel_markdown_edit`)、Esc 丢弃。
/// 自绘输入原因同 `todo_search_bar`(原生 `text_input` 留不住焦点、不参与
/// main.rs 键盘路由裁决)。
fn todo_markdown_view<'a>(
    project_path: Option<&Path>,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let content: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if ws_state.markdown_editing {
            let caret = "▏";
            if ws_state.markdown_draft.is_empty() {
                text(caret)
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().dim)
                    .into()
            } else {
                text(format!("{}{caret}", ws_state.markdown_draft))
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().cream)
                    .into()
            }
        } else {
            let src = project_path
                .map(todo_path)
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_else(|| "# 暂无 .dozer/todo.md".to_string());
            text(src)
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().cream)
                .into()
        };

    let body = container(content).padding([12, 20]).width(Length::Fill);

    // MARKDOWN 视图滚动条同样对齐全应用统一滚动条规范。
    let area = MouseArea::new(
        scrollable(body)
            .height(Length::Fill)
            .direction(scrollable::Direction::Vertical(
                byteui::interaction::scrollbar::scrollbar(),
            ))
            .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
    )
    .interaction(mouse::Interaction::Pointer);
    if ws_state.markdown_editing {
        // 编辑态下点主体不重载草稿(避免把刚打的字冲掉),纯 no-op。
        area.into()
    } else {
        area.on_press(Message::MarkdownEditStart).into()
    }
}

/// `todo_list_view` 单行的渲染分派:内容编辑态 → `todo_content_edit_row`,
/// 否则 → `todo_card`。从 `todo_list_view` 的循环体里拆出来,好让 pending/
/// done 两段各自的 `for` 循环别重复这段查表+分支逻辑。
#[allow(clippy::too_many_arguments)]
fn todo_list_row<'a>(
    app_state: &'a AppState,
    app: &App,
    ws_state: &'a WorkspaceState,
    project_id: i64,
    states: &[TodoState],
    number: usize,
    idx: usize,
    grabbing: bool,
    is_drag_source: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let item = &ws_state.items[idx];
    let key = todo_line_key(&item.text);
    let meta = app_state.meta_for(project_id, key);
    let dispatch = meta.and_then(|m| m.dispatch.as_ref());
    let hovered = app.hover_target(HoverId::TodoCard(idx));
    // 内容编辑态:不再把整张卡替换成独立的编辑行,而是把 `editing_draft` 传进
    // `todo_card`,由卡片原地保留边框/背景、只把内容文字换成带 BORDER 描边的
    // 输入框(见 `todo_card` 内 `label_area` 的分支)。
    let editing_draft = if let Some((editing_idx, draft)) = &ws_state.editing_content
        && *editing_idx == idx
    {
        Some(draft)
    } else {
        None
    };
    todo_card(TodoCardArgs {
        number,
        idx,
        item,
        state: states[idx],
        meta,
        dispatch,
        selected: ws_state.selected_row == Some(idx),
        grabbing,
        is_drag_source,
        hovered,
        editing_draft,
    })
}

/// 拖拽换位的"插入指示线":一条细的金色横条,插在"松手会落到这里"的
/// 展示位置——取代之前逐帧重排其它卡片的做法(见 `todo_list_view`)。
/// 高度和左右 padding 跟卡片间距(`spacing(8)`)对齐,视觉上像卡片之间
/// 多出的一道缝被点亮,而不是新插了一整行。
fn drag_insert_indicator() -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer>
{
    container(iced_widget::space::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(3.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().gold.into()),
            border: Border {
                radius: 2.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// 统一卡片组件：列表视图使用的边框卡片视觉，取代原来的
/// `todo_row`(扁平高亮行)。结构自上而下：编号 + 状态(`#002 - 待办` 形式,
/// 状态紧跟序号)+ 日期徽章(calendar 图标 → 日历选择器)→ checkbox + 任务文字
/// (点文字进入内容编辑)→ 指派文本按钮(仅待办未派发时)。选中/一般/hover 三态
/// 走统一卡片样式(选中=金边、hover=金边+填充、一般态=描边)。
/// `todo_card` 的参数对象:11 个位置参数里 `selected`/`grabbing`/
/// `is_drag_source`/`hovered` 四个连续 `bool`,顺序传错编译器发现不了
/// (Rust Design Patterns:Builder,用具名字段替代同类型位置参数)。
struct TodoCardArgs<'a> {
    number: usize,
    idx: usize,
    item: &'a TodoItem,
    state: TodoState,
    meta: Option<&'a TodoTaskMeta>,
    dispatch: Option<&'a DispatchRecord>,
    selected: bool,
    grabbing: bool,
    is_drag_source: bool,
    hovered: bool,
    editing_draft: Option<&'a iced_widget::text_editor::Content>,
}

fn todo_card<'a>(
    args: TodoCardArgs<'a>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let TodoCardArgs {
        number,
        idx,
        item,
        state,
        meta,
        dispatch,
        selected,
        grabbing,
        is_drag_source,
        hovered,
        editing_draft,
    } = args;
    let done = item.done;

    // ---- 顶部行：编号 + 日期徽章(calendar 图标 → 日历选择器)+ 状态文字 ----
    let number_text = text(format!("#{number:03}"))
        .size(byteui::theme::font::caption())
        .color(byteui::theme::color::current().dim);

    let date_label = match state {
        TodoState::Done => meta
            .and_then(|m| m.completed_at)
            .map(format_todo_month_day)
            .unwrap_or_else(|| "-".to_string()),
        _ => meta
            .and_then(|m| m.plan_date.clone())
            .unwrap_or_else(|| "-".to_string()),
    };
    let date_badge: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> =
        MouseArea::new(
            row![
                icons::view(
                    icons::IconKind::Calendar,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().dim
                ),
                text(date_label)
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().dim),
            ]
            .spacing(4)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        )
        .interaction(mouse::Interaction::Pointer)
        .on_press(Message::CalendarOpen(idx))
        .into();

    // 状态:静态文字(不再是可点击 pill),颜色随三态走——待办=青、进行中=金、
    // 完成=灰。紧跟在序号之后(如 `#002 - 待办`),日期徽章仍居右。
    let status_label = state_label(state);

    let status_sep = text(" - ")
        .size(byteui::theme::font::caption())
        .color(byteui::theme::color::current().dim);

    let top_row = row![
        number_text,
        status_sep,
        status_label,
        iced_widget::space::Space::new()
            .width(Length::Fill)
            .height(Length::Shrink),
        date_badge,
    ]
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .spacing(8);

    // ---- 中部：checkbox + 任务文字（勾选/删除线处理与原 todo_row 一致）----
    let box_color = if done {
        byteui::theme::color::current().border
    } else {
        byteui::theme::color::current().dim
    };
    let checkbox = button(
        container(if done {
            text("✓")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim)
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
                Some(byteui::theme::color::current().border.into())
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
        text_color: byteui::theme::color::current().cream,
        ..button::Style::default()
    });

    // 任务内容文字:未完成用主题 `body`(#9AB4C4,与正文层级一致,不再用
    // 奶油色高亮整句),已完成保持 `dim` + 删除线。进入内容编辑态时文字
    // 仍走奶油色(见 `label_area` 编辑分支),这里只负责非编辑态静态内容。
    let label_color = if done {
        byteui::theme::color::current().dim
    } else {
        byteui::theme::color::current().body
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
                .size(byteui::theme::font::body())
                .color(label_color)
                .strikethrough(true)
        ];
        rich.into()
    } else {
        text(item.text.clone())
            .size(byteui::theme::font::body())
            .color(label_color)
            .into()
    };
    // 点任务文字 → 进入内容行内编辑态(取代原来的"选中"——选中/拖拽仍由卡片
    // 外层的 `RowSelect` 承担,点文字只负责编辑)。编辑态下只把内容文字原地换成
    // 自绘输入框:卡片边框/背景/勾选/日期/指派全部保持原样,输入框尺寸对齐原
    // 内容(同字号 body + 同宽 Fill),仅加 #1c3440(=BORDER)描边、不另设背景
    // (透出卡片底),避免整卡被替换成另一个带金边的大框。
    let label_area: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if let Some(draft) = editing_draft {
            // 真正的 iced `text_editor`(`byteui::form::text_area`,`bare: true`
            // 不画自身背景/描边,把外框交回下面这个外层 `container` 复刻旧版
            // "只有描边、不透底"的观感)。`height: None` 走 iced 的
            // `Length::Shrink`,随内容自然撑高——多行任务文字编辑时不再被
            // 压成单行(见 `byteui::form::text_area` 头部注释)。`content_field_id`
            // 从旧版 `container` 挪到真 `text_editor` 上,`CaptureContentEditFocus`
            // 的 `focusable` 钩子才能认出它。
            container(byteui::interaction::context_menu::wrap(
                byteui::form::text_area::view(
                    draft,
                    "任务内容…",
                    Some(content_field_id()),
                    true,
                    None,
                    Message::ContentEdit,
                ),
                Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
                    id: content_field_id(),
                    secure: false,
                })),
            ))
            .width(Length::Fill)
            .padding([10, 12])
            .style(|_t: &iced_widget::Theme| container::Style {
                background: None,
                border: Border {
                    color: byteui::theme::color::current().border,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..container::Style::default()
            })
            .into()
        } else {
            MouseArea::new(container(label).width(Length::Fill))
                .interaction(mouse::Interaction::Pointer)
                .on_press(Message::ContentEditStart(idx))
                .into()
        };

    let body_row = row![checkbox, label_area]
        .spacing(10)
        .align_y(iced_widget::core::alignment::Vertical::Center);

    // ---- 底部行：指派文本按钮(仅待办未派发) ----
    let mut bottom = row![]
        .spacing(8)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    if state == TodoState::Pending && dispatch.is_none() {
        // 样式对齐 `project.rs::project_footer_bar` 的「修复项目」按钮:
        // BG 底 + 1px BORDER 描边 + 圆角 4 + CREAM 文字,label 字号 + 内边距
        // [6,8]。文字后跟 ChevronRight 图标,提示点击会弹出可指派的 agent
        // 列表(窗口级 overlay)。
        let assign_btn = button(
            row![
                text("指派")
                    .size(byteui::theme::font::label())
                    .color(byteui::theme::color::current().cream),
                icons::view(
                    icons::IconKind::ChevronRight,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().cream,
                ),
            ]
            .spacing(4)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        )
        .on_press(Message::DispatchOpen(idx))
        .padding([6, 8])
        .style(|_t: &iced_widget::Theme, s: button::Status| {
            let hovered = matches!(s, button::Status::Hovered);
            button::Style {
                background: if hovered {
                    Some(byteui::theme::color::current().tab_hover.into())
                } else {
                    Some(byteui::theme::color::current().bg.into())
                },
                border: Border {
                    color: if hovered {
                        byteui::theme::color::current().gold
                    } else {
                        byteui::theme::color::current().border
                    },
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: byteui::theme::color::current().cream,
                ..button::Style::default()
            }
        });
        bottom = bottom.push(assign_btn);
    }

    let bottom_row = row![
        iced_widget::space::Space::new()
            .width(Length::Fill)
            .height(Length::Shrink),
        bottom,
    ];

    let card_body = column![top_row, body_row, bottom_row].spacing(8);

    // 统一卡片样式:选中=金边(无背景)、hover=金边+填充、一般态=描边(无
    // 背景)——与 Agent 卡片三态对齐,不再用左侧 3px 金竖条表示选中。
    let inner = container(card_body).padding(10).width(Length::Fill);

    let card = container(inner)
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| {
            // 正在被拖起的那张卡片描边变金、加粗——跟"插入指示线"配合给
            // 出"这张卡片被拿起来了/会落在指示线那里"的反馈,不再靠其它
            // 卡片瞬间跳位来表达换位(见 `todo_list_view` 的改版说明)。
            if is_drag_source {
                container::Style {
                    background: Some(byteui::theme::color::current().card.into()),
                    border: Border {
                        color: byteui::theme::color::current().gold,
                        width: 1.5,
                        radius: byteui::interaction::cards::CARD_RADIUS.into(),
                    },
                    ..container::Style::default()
                }
            } else {
                byteui::interaction::cards::container_card(
                    selected,
                    hovered,
                    byteui::theme::color::current().card,
                )
            }
        });

    let stacked = column![card];
    // 拖拽换位感应层:补 `on_move`(光标移动过本卡就发 `DragMove`)+
    // `on_press`(`RowSelect` 选中并武装拖拽——点文字/勾选/日期/指派 这些
    // 子元素各自吞掉自己的"按下",只有落在卡片空白处才走到这里)。拖拽中
    // 整张卡显示抓取光标。
    let area = MouseArea::new(stacked)
        .on_move(move |_| Message::DragMove(idx))
        .on_enter(Message::Hover(HoverId::TodoCard(idx), true))
        .on_exit(Message::Hover(HoverId::TodoCard(idx), false))
        .on_press(Message::RowSelect(if selected { None } else { Some(idx) }));
    if grabbing {
        area.interaction(mouse::Interaction::Grabbing).into()
    } else {
        area.into()
    }
}

/// Todo 派发选择层(窗口级 overlay 版):列出当前项目存活的 agent tab(不再
/// 提供"新建 agent 会话"入口——见 2026-08-17 优化),每个条目前带该 agent
/// 的品牌图标。样式统一走 `crate::menu::item_row_fill` + `menu::shell`(基准
/// 即文件树右键菜单),定位靠 `dispatch_anchor`(点"指派"按钮时的光标,等价于
/// "按钮旁边")。返回 `None` 表示没有可弹的层(`dispatch_open` 为真但锚点缺失,
/// 理论上不会到——调用方降级为只铺 dismiss 收起层)。
pub fn todo_dispatch_overlay<'a>(
    ws: &Workspace,
    window_size: (f32, f32),
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let ws_state = &ws.todo;
    let idx = ws_state.dispatch_open?;
    let anchor = ws_state.dispatch_anchor?;
    // 当前项目存活的终端/SSH 会话都是可派发目标;session_id 与展示标题沿用
    // `app.rs` 原 `SessionTabSummary` 的算法(`info.id` + `tab_title`)。
    let tabs: Vec<(String, AgentKind, String)> = ws
        .tabs
        .iter()
        .chain(ws.ssh_tabs.iter())
        .filter(|t| t.alive)
        .map(|t| {
            (
                t.info.id.clone(),
                t.agent,
                tab_title(t.agent, t.cwd.as_deref(), &t.info.name),
            )
        })
        .collect();

    let items: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> = tabs
        .iter()
        .map(|(session_id, agent, title)| {
            let icon = icons::view(
                agent_icon(*agent),
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().cream,
            );
            crate::menu::item_row(
                Some(icon),
                title.clone(),
                byteui::theme::color::current().cream,
                Some(Message::DispatchToExisting(idx, session_id.clone())),
            )
        })
        .collect();
    let popup = crate::menu::shell(items, Length::Shrink);

    // 全窗口容器 + padding 把弹层推到锚点;窗口边界钳制,避免超出右下。
    let (ax, ay) = anchor;
    let window_w = window_size.0;
    let window_h = window_size.1;
    // 菜单估算尺寸:常宽约 220(图标 + 文字 + 内边距)、高约每项 28 + 内边距。
    let pop_w = 224.0_f32;
    let pop_h = 240.0_f32;
    let x = ax.min((window_w - pop_w).max(0.0));
    let y = ay.min((window_h - pop_h).max(0.0));
    Some(
        container(popup)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: y,
                left: x,
                right: 0.0,
                bottom: 0.0,
            })
            .into(),
    )
}

/// 状态静态文字：三态只显示彩色文字,没有按钮/菜单行为(状态不再可点击切换,
/// 只能靠 checkbox 勾选翻转)。颜色:待办=青 `CYAN`、进行中=金 `GOLD`、
/// 完成=灰 `DIM`。
fn state_label(
    state: TodoState,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (label, color) = match state {
        TodoState::Pending => ("待办", byteui::theme::color::current().cyan),
        TodoState::InProgress => ("进行中", byteui::theme::color::current().gold),
        TodoState::Done => ("已完成", byteui::theme::color::current().dim),
    };
    text(label)
        .size(byteui::theme::font::caption())
        .color(color)
        .into()
}

/// 日历日期选择器：点卡片日期徽章弹出,展示 `calendar_view` 那个月,上一月/
/// 下一月导航,点某天把 `plan_date` 写成 "MM-DD" 并关闭。样式对齐
/// `todo_dispatch_overlay`(CARD 底 + BORDER 描边)。
fn todo_calendar_popup(
    idx: usize,
    view: (i32, u32),
    selected: Option<String>,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (y, m) = view;
    let first_wd = first_weekday_of_month(y, m);
    let dim = days_in_month(y, m);
    // 没有 plan_date 时默认高亮"今天"(仅当当前视图月就是当前月);有
    // plan_date 则高亮那天的 MM-DD。满足"弹出时默认选中当天日期"。
    let selected_md = selected.as_deref().and_then(parse_month_day).or_else(|| {
        let (ty, tm, td) = today_ymd();
        if tm == m && ty == y {
            Some((tm, td))
        } else {
            None
        }
    });

    let nav_style = |_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: byteui::theme::color::current().cream,
        ..button::Style::default()
    };
    let prev = button(
        text("‹")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
    )
    .on_press(Message::CalendarPrevMonth)
    .padding([2, 8])
    .style(nav_style);
    let next = button(
        text("›")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
    )
    .on_press(Message::CalendarNextMonth)
    .padding([2, 8])
    .style(nav_style);
    let title = text(format!("{y}-{m:02}"))
        .size(byteui::theme::font::caption())
        .color(byteui::theme::color::current().cream);
    let header = row![prev, title, next]
        .spacing(6)
        .align_y(iced_widget::core::alignment::Vertical::Center);

    let weekday_labels = ["日", "一", "二", "三", "四", "五", "六"];
    let mut weekday_row = row![].spacing(0);
    for w in weekday_labels {
        weekday_row = weekday_row.push(
            container(
                text(w)
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().dim),
            )
            .width(Length::Fixed(28.0))
            .center_x(Length::Fill),
        );
    }

    let mut rows = column![].spacing(2);
    let mut day: u32 = 1;
    'outer: for r in 0..6u32 {
        let mut row_el = row![].spacing(0);
        for c in 0..7u32 {
            let cell = r * 7 + c;
            if cell < first_wd || day > dim {
                row_el = row_el.push(
                    container(iced_widget::space::Space::new())
                        .width(Length::Fixed(28.0))
                        .height(Length::Fixed(24.0)),
                );
            } else {
                let d = day;
                let is_sel = selected_md == Some((m, d));
                let cell_btn = button(
                    text(format!("{d}"))
                        .size(byteui::theme::font::caption())
                        .color(if is_sel {
                            byteui::theme::color::current().gold
                        } else {
                            byteui::theme::color::current().cream
                        }),
                )
                .on_press(Message::CalendarPick(idx, format!("{m:02}-{d:02}")))
                .width(Length::Fixed(28.0))
                .height(Length::Fixed(24.0))
                .padding(0)
                .style(move |_t: &iced_widget::Theme, _s| button::Style {
                    background: if is_sel {
                        Some(byteui::theme::color::current().card.into())
                    } else {
                        None
                    },
                    border: Border {
                        color: if is_sel {
                            byteui::theme::color::current().gold
                        } else {
                            Color::TRANSPARENT
                        },
                        width: if is_sel { 1.0 } else { 0.0 },
                        radius: 4.0.into(),
                    },
                    text_color: if is_sel {
                        byteui::theme::color::current().gold
                    } else {
                        byteui::theme::color::current().cream
                    },
                    ..button::Style::default()
                });
                row_el = row_el.push(cell_btn);
                day += 1;
                if day > dim {
                    rows = rows.push(row_el);
                    break 'outer;
                }
            }
        }
        rows = rows.push(row_el);
    }

    container(column![header, weekday_row, rows].spacing(4))
        .width(Length::Shrink)
        .padding(8)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().card.into()),
            border: Border {
                color: byteui::theme::color::current().border,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// Todo 日历日期选择器的窗口级 overlay 版本:不再挂在卡片下方,而是铺在
/// 整张窗口之上、定位到点击按钮时的光标锚点(像右键菜单一样出现在按钮旁
/// 边)。返回 `None` 表示没有可弹的日历(`calendar_open` 为真但锚点/项目
/// 缺失,理论上不会到——调用方降级为只铺 dismiss 收起层,避免卡在打开态)。
///
/// `app_state` 取 plan_date、`window_size` 用于边界钳制——二者都是 `App`
/// 的私有字段,不能让本模块直接碰 `App`,沿用 `todo::update`/`view` 把
/// `app_state` 当参数传进来的约定。
pub fn todo_calendar_overlay<'a>(
    app_state: &AppState,
    ws: &Workspace,
    window_size: (f32, f32),
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let ws_state = &ws.todo;
    let idx = ws_state.calendar_open?;
    let anchor = ws_state.calendar_anchor?;
    let project_id = ws.project.as_ref()?.id;
    let item = ws_state.items.get(idx)?;
    let key = todo_line_key(&item.text);
    let selected = app_state
        .meta_for(project_id, key)
        .and_then(|m| m.plan_date.clone());
    let popup = todo_calendar_popup(idx, ws_state.calendar_view, selected);

    // 全窗口容器 + padding 把弹层推到锚点;窗口边界钳制,避免日历超出右下。
    let (ax, ay) = anchor;
    let window_w = window_size.0;
    let window_h = window_size.1;
    // 日历估算尺寸:7 列 × 28px + 内边距(8×2)≈ 212 宽;标题 + 星期行 + 6
    // 行 × 24px + 间距 + 内边距 ≈ 240 高。
    let pop_w = 216.0_f32;
    let pop_h = 240.0_f32;
    let x = ax.min((window_w - pop_w).max(0.0));
    let y = ay.min((window_h - pop_h).max(0.0));
    Some(
        container(popup)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: y,
                left: x,
                right: 0.0,
                bottom: 0.0,
            })
            .into(),
    )
}

/// 左栏分类导航项（= 原 filter 段，竖排）：图标 + 标签 + 右侧计数，选中态
/// 金框 + CREAM 字 + CARD 底。点击 → `FilterSet`。
fn todo_category_button<'a>(
    filter: TodoFilter,
    count: usize,
    current: TodoFilter,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (icon, label) = match filter {
        TodoFilter::All => (icons::IconKind::CircleSmall, "全部任务"),
        TodoFilter::Pending => (icons::IconKind::CircleSmall, "待办任务"),
        TodoFilter::InProgress => (icons::IconKind::CircleSmall, "进行中任务"),
        TodoFilter::Done => (icons::IconKind::CircleSmall, "已完成任务"),
    };
    let active = filter == current;
    let fg = if active {
        byteui::theme::color::current().cream
    } else {
        byteui::theme::color::current().dim
    };
    button(
        row![
            icons::view(
                icon,
                byteui::theme::icon_size::row(),
                if active {
                    byteui::theme::color::current().gold
                } else {
                    byteui::theme::color::current().dim
                }
            ),
            text(label).size(byteui::theme::font::body()).color(fg),
            iced_widget::space::Space::new()
                .width(Length::Fill)
                .height(Length::Shrink),
            text(format!("{count}"))
                .size(byteui::theme::font::caption())
                .color(if active {
                    byteui::theme::color::current().gold
                } else {
                    byteui::theme::color::current().dim
                }),
        ]
        .spacing(8)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .on_press(Message::FilterSet(filter))
    .width(Length::Fill)
    .padding([8, 10])
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: if active {
            Some(byteui::theme::color::current().card.into())
        } else {
            None
        },
        text_color: fg,
        border: Border {
            color: if active {
                byteui::theme::color::current().gold
            } else {
                Color::TRANSPARENT
            },
            width: if active { 1.0 } else { 0.0 },
            radius: 6.0.into(),
        },
        ..button::Style::default()
    })
    .into()
}

/// 右区顶部 tab 段：列表 / MARKDOWN。视觉对齐全应用统一的"标准 tab"
/// 样式(`app::panel_tab` 的选中态：`CARD` 底 + `BORDER` 1px 描边 + 6 圆角，
/// 项目页签/终端会话 tab 都是这一套)，不再是这个面板自己发明的下划线
/// 样式。点击 → `ViewModeSet`。
fn todo_view_tabs<'a>(
    current: TodoViewMode,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    row![
        todo_tab(
            icons::IconKind::ListTodo,
            "列表",
            TodoViewMode::List,
            current
        ),
        todo_tab(
            icons::IconKind::FileText,
            "MARKDOWN",
            TodoViewMode::Markdown,
            current
        ),
    ]
    .spacing(4)
    // 上下从 8 收到 4(验收反馈:这里不是 `panel_tab`,是独立实现的视图
    // 切换 tab,原高度比左栏 header 分割线低了近 8px,对不齐)。左右不再
    // 单独留白(2026-08-28 起改 0)——外层 `tabs_bar` 已经有 20px 左右
    // padding,这里再叠一层会让"列表"tab 比下面的搜索框/任务卡片多缩进
    // 一截,对不齐左边。
    .padding([4, 0])
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .into()
}

/// 单个视图切换 tab：图标 + 标签，选中态 `CARD` 底 + `BORDER` 描边圆角
/// (标准 tab 视觉，见 `todo_view_tabs` 文档)。这是纯粹的视图模式切换，没有
/// 可关闭语义,不套 `tabs::tab_core`(那是为可关闭 tab 设计的交互内核,
/// 强套需要传一个永远不触发的 `on_close` 并额外处理"×"淡入的悬停态，
/// 削足适履)。
fn todo_tab<'a>(
    icon: icons::IconKind,
    label: &'a str,
    mode: TodoViewMode,
    current: TodoViewMode,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let active = mode == current;
    let fg = if active {
        byteui::theme::color::current().cream
    } else {
        byteui::theme::color::current().dim
    };
    let icon_color = if active {
        byteui::theme::color::current().gold
    } else {
        byteui::theme::color::current().dim
    };
    button(
        row![
            icons::view(icon, byteui::theme::icon_size::row(), icon_color),
            text(label).size(byteui::theme::font::caption()).color(fg),
        ]
        .spacing(6)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .on_press(Message::ViewModeSet(mode))
    .padding([6, 12])
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: if active {
            Some(byteui::theme::color::current().card.into())
        } else {
            None
        },
        text_color: fg,
        border: Border {
            color: if active {
                byteui::theme::color::current().border
            } else {
                Color::TRANSPARENT
            },
            width: 1.0,
            radius: 6.0.into(),
        },
        ..button::Style::default()
    })
    .into()
}

/// `SystemTime` → "MM-DD"（SUCCESS 徽章用，只取月日）。
fn format_todo_month_day(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86400) as i64;
    let (_y, m, d) = civil_from_days(days);
    format!("{m:02}-{d:02}")
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
    fn move_pending_to_reorders_pending_only_done_kept_in_place() {
        // 待办 A、已完成 X、待办 B(混排);把第 0 个待办(A)移到第 1 位应与
        // 第 1 个待办(B)交换——只动待办之间的相对次序,已完成 X 留在原位
        // (列表视图再把它沉到最底,见 `todo_list_view` 的 pending/done 分区)。
        let md = "- [ ] A\n- [x] X\n- [ ] B\n";
        let moved = move_pending_to(md, 0, 1).expect("应可下移");
        assert_eq!(moved, "- [ ] B\n- [x] X\n- [ ] A\n");
        // 视图层分区后展示次序应为 B、A(待办)、X(已完成沉底)。
        let items = parse_todo(&moved);
        let pending: Vec<&str> = items
            .iter()
            .filter(|it| !it.done)
            .map(|it| it.text.as_str())
            .collect();
        let done: Vec<&str> = items
            .iter()
            .filter(|it| it.done)
            .map(|it| it.text.as_str())
            .collect();
        assert_eq!(pending, vec!["B", "A"]);
        assert_eq!(done, vec!["X"]);
        // 非 checkbox 行(标题/正文)位置不动。
        let md2 = "# 标题\n\n- [ ] A\n正文\n- [x] X\n- [ ] B\n";
        let moved2 = move_pending_to(md2, 1, 0).expect("应可上移");
        assert_eq!(moved2, "# 标题\n\n- [ ] B\n正文\n- [x] X\n- [ ] A\n");
    }

    #[test]
    fn move_pending_to_out_of_range_is_noop() {
        let md = "- [ ] A\n- [ ] B\n";
        assert!(move_pending_to(md, 0, 0).is_none()); // from==to,no-op
        assert!(move_pending_to(md, 0, 5).is_none()); // to 越界(已在顶上移)
        assert!(move_pending_to(md, 1, 2).is_none()); // to 越界(已在底下移)
        assert!(move_pending_to(md, 5, 0).is_none()); // from 越界
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
    fn prepend_todo_item_to_empty_list() {
        let content = "# Todo\n";
        assert_eq!(
            prepend_todo_item(content, "新任务"),
            "# Todo\n- [ ] 新任务\n"
        );
    }

    #[test]
    fn prepend_todo_item_before_first_existing_item() {
        let content = "# Todo\n\n- [ ] 任务A\n- [x] 任务B\n";
        assert_eq!(
            prepend_todo_item(content, "任务C"),
            "# Todo\n\n- [ ] 任务C\n- [ ] 任务A\n- [x] 任务B\n"
        );
    }

    #[test]
    fn prepend_todo_item_keeps_headline_above() {
        let content = "# Todo\n正文行\n- [ ] 任务A\n";
        assert_eq!(
            prepend_todo_item(content, "任务B"),
            "# Todo\n正文行\n- [ ] 任务B\n- [ ] 任务A\n"
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
    fn update_drag_end_reorders_pending_in_file() {
        // 待办 A、B、C;把 A(下标 0)拖到 C 的位置(下标 2),`DragEnd` 应把
        // 待办块重排成 B、C、A 并写盘。已完成行不参与(这里没有)。
        let (_dir, root) = project_dir_with_todo("- [ ] A\n- [ ] B\n- [ ] C\n");
        let mut ws_state = WorkspaceState {
            items: vec![
                TodoItem {
                    text: "A".into(),
                    done: false,
                },
                TodoItem {
                    text: "B".into(),
                    done: false,
                },
                TodoItem {
                    text: "C".into(),
                    done: false,
                },
            ],
            ..WorkspaceState::default()
        };
        let mut app_state = AppState::default();
        ws_state.drag = Some(TodoDrag {
            source_idx: 0,
            target_idx: 2,
        });
        update(&mut ws_state, &mut app_state, Message::DragEnd, 1, &root);
        let content = std::fs::read_to_string(todo_path(&root)).unwrap();
        assert_eq!(content, "- [ ] B\n- [ ] C\n- [ ] A\n");
        assert!(ws_state.drag.is_none(), "DragEnd 应清掉拖拽态");
    }

    #[test]
    fn update_drag_end_keeps_dragged_task_selected() {
        // 拖拽完成后被拖的任务要保持选中:把 A(下标 0)拖到 C 的位置(下标 2),
        // 重排成 B、C、A 后 `selected_row` 应指向 A 的新下标 2,而不是它挪走后
        // 占住源位的那条 B(下标 0)。
        let (_dir, root) = project_dir_with_todo("- [ ] A\n- [ ] B\n- [ ] C\n");
        let mut ws_state = WorkspaceState {
            items: vec![
                TodoItem {
                    text: "A".into(),
                    done: false,
                },
                TodoItem {
                    text: "B".into(),
                    done: false,
                },
                TodoItem {
                    text: "C".into(),
                    done: false,
                },
            ],
            ..WorkspaceState::default()
        };
        let mut app_state = AppState::default();
        ws_state.drag = Some(TodoDrag {
            source_idx: 0,
            target_idx: 2,
        });
        update(&mut ws_state, &mut app_state, Message::DragEnd, 1, &root);
        assert_eq!(
            ws_state.selected_row,
            Some(2),
            "被拖的 A 应选中,其新 item-index 是 2"
        );
        assert_eq!(ws_state.items[2].text, "A");
    }

    #[test]
    fn update_drag_end_keeps_selected_when_dragged_to_pending_end() {
        // 拖到待办块末尾(`target_idx == usize::MAX`,悬停已完成卡片):被拖的
        // A 应落到最后一个待办位,并保持选中。
        let (_dir, root) = project_dir_with_todo("- [ ] A\n- [ ] B\n- [ ] C\n- [x] D\n");
        let mut ws_state = WorkspaceState {
            items: vec![
                TodoItem {
                    text: "A".into(),
                    done: false,
                },
                TodoItem {
                    text: "B".into(),
                    done: false,
                },
                TodoItem {
                    text: "C".into(),
                    done: false,
                },
                TodoItem {
                    text: "D".into(),
                    done: true,
                },
            ],
            ..WorkspaceState::default()
        };
        let mut app_state = AppState::default();
        ws_state.drag = Some(TodoDrag {
            source_idx: 0,
            target_idx: usize::MAX,
        });
        update(&mut ws_state, &mut app_state, Message::DragEnd, 1, &root);
        assert_eq!(
            ws_state.selected_row,
            Some(2),
            "拖到待办块末尾后 A 位于最后一个待办位(下标 2)"
        );
        assert_eq!(ws_state.items[2].text, "A");
    }

    #[test]
    fn update_drag_end_noop_when_not_moved() {
        // 光标没真移动过(source==target),`DragEnd` 是 no-op,不碰磁盘文件。
        let (_dir, root) = project_dir_with_todo("- [ ] A\n- [ ] B\n");
        let mut ws_state = WorkspaceState {
            items: vec![
                TodoItem {
                    text: "A".into(),
                    done: false,
                },
                TodoItem {
                    text: "B".into(),
                    done: false,
                },
            ],
            ..WorkspaceState::default()
        };
        let mut app_state = AppState::default();
        ws_state.drag = Some(TodoDrag {
            source_idx: 0,
            target_idx: 0,
        });
        update(&mut ws_state, &mut app_state, Message::DragEnd, 1, &root);
        let content = std::fs::read_to_string(todo_path(&root)).unwrap();
        assert_eq!(content, "- [ ] A\n- [ ] B\n", "没移动不该改文件");
        assert!(ws_state.drag.is_none());
    }

    #[test]
    fn update_add_submit_prepends_and_clears_draft() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState {
            add_draft: iced_widget::text_editor::Content::with_text("新任务"),
            ..WorkspaceState::default()
        };
        let mut app_state = AppState::default();
        update(&mut ws_state, &mut app_state, Message::AddSubmit, 1, &root);
        assert!(ws_state.add_draft.text().is_empty());
        assert_eq!(ws_state.items.len(), 1);
        assert_eq!(ws_state.items[0].text, "新任务");
        // 新增后应置顶闪光:卡片保持选中、滚动位被武装(等 main.rs 消费)。
        assert_eq!(ws_state.selected_row, Some(0));
        assert!(ws_state.flash.is_some());
        assert!(ws_state.take_scroll_to_top());
        // 取走后滚动位复位,重启闪光倒计时(未到点前不立即清除)。
        assert!(!ws_state.take_scroll_to_top());
        assert!(ws_state.next_flash_wake().is_some());
    }

    #[test]
    fn flash_clears_selection_after_expiry() {
        let mut ws_state = WorkspaceState::default();
        ws_state.start_flash(2);
        assert_eq!(ws_state.selected_row, Some(2));
        // 未到点:advance 不应清除。
        ws_state.advance_flash();
        assert_eq!(ws_state.selected_row, Some(2));
        assert!(ws_state.next_flash_wake().is_some());
        // 把闪光计时拨到过去,模拟 2s 已过。
        ws_state.flash = Some(Flash {
            idx: 2,
            until: std::time::Instant::now() - std::time::Duration::from_millis(1),
        });
        ws_state.advance_flash();
        assert_eq!(ws_state.selected_row, None);
        assert!(ws_state.next_flash_wake().is_none());
    }

    #[test]
    fn flash_does_not_clear_user_changed_selection() {
        let mut ws_state = WorkspaceState::default();
        ws_state.start_flash(2);
        // 用户在闪光期间手动选了别的卡片(或取消):到点不再抢回/清除。
        ws_state.selected_row = Some(5);
        ws_state.flash = Some(Flash {
            idx: 2,
            until: std::time::Instant::now() - std::time::Duration::from_millis(1),
        });
        ws_state.advance_flash();
        assert_eq!(ws_state.selected_row, Some(5));
        assert!(ws_state.next_flash_wake().is_none());
    }

    #[test]
    fn row_select_clears_flash() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState::default();
        // 先模拟一次新增闪光。
        ws_state.start_flash(0);
        assert!(ws_state.next_flash_wake().is_some());
        let mut app_state = AppState::default();
        // 用户随后手动点卡片空白处选中:应立即熄灭闪光,不再让 2s 倒计时
        // 干扰用户主动选中。
        update(
            &mut ws_state,
            &mut app_state,
            Message::RowSelect(Some(0)),
            1,
            &root,
        );
        assert!(ws_state.next_flash_wake().is_none());
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
    fn update_add_edit_builds_draft_and_submits() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState {
            add_draft: iced_widget::text_editor::Content::with_text("新任务"),
            ..WorkspaceState::default()
        };
        let mut app_state = AppState::default();
        assert_eq!(ws_state.add_draft.text(), "新任务");
        update(&mut ws_state, &mut app_state, Message::AddSubmit, 1, &root);
        assert!(ws_state.add_draft.text().is_empty());
        assert_eq!(ws_state.items.len(), 1);
        assert_eq!(ws_state.items[0].text, "新任务");
    }

    #[test]
    fn set_add_focused_updates_accessor() {
        let mut ws_state = WorkspaceState::default();
        assert!(!ws_state.add_focused());
        ws_state.set_add_focused(true);
        assert!(ws_state.add_focused());
        ws_state.set_add_focused(false);
        assert!(!ws_state.add_focused());
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
            Message::SearchInput("关键字".to_string()),
            1,
            &root,
        );
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchSubmit,
            1,
            &root,
        );
        assert_eq!(ws_state.search, "关键字");
    }

    #[test]
    fn filter_set_clears_search_even_switching_back_to_same_filter() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchInput("关键字".to_string()),
            1,
            &root,
        );
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchSubmit,
            1,
            &root,
        );
        assert_eq!(ws_state.search, "关键字");

        // 切到另一个分类:搜索词清空。
        update(
            &mut ws_state,
            &mut app_state,
            Message::FilterSet(TodoFilter::Done),
            1,
            &root,
        );
        assert!(ws_state.search.is_empty());
        assert!(ws_state.search_draft.is_empty());

        // 再切回原来那个分类(All):即使是搜过词的那个分类,也不恢复。
        update(
            &mut ws_state,
            &mut app_state,
            Message::FilterSet(TodoFilter::All),
            1,
            &root,
        );
        assert!(ws_state.search.is_empty());
        assert!(ws_state.search_draft.is_empty());
    }

    #[test]
    fn set_search_focused_updates_accessor() {
        let mut ws_state = WorkspaceState::default();
        assert!(!ws_state.search_focused());
        ws_state.set_search_focused(true);
        assert!(ws_state.search_focused());
        ws_state.set_search_focused(false);
        assert!(!ws_state.search_focused());
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
    fn app_state_record_dispatch_then_meta_for_finds_it() {
        let mut app_state = AppState::default();
        app_state.record_dispatch(1, "任务A", "sess-1".to_string());
        let key = todo_line_key("任务A");
        let meta = app_state.meta_for(1, key).unwrap();
        assert_eq!(meta.dispatch.as_ref().unwrap().session_id, "sess-1");
    }

    #[test]
    fn task_title_for_session_finds_dispatched_task() {
        let ws_state = ws_with_item("修复登录 bug", false);
        let mut app_meta = AppState::default();
        app_meta.record_dispatch(1, "修复登录 bug", "sess-1".to_string());

        assert_eq!(
            ws_state.task_title_for_session(&app_meta, 1, "sess-1"),
            Some("修复登录 bug")
        );
    }

    #[test]
    fn task_title_for_session_none_when_no_dispatch_matches() {
        let ws_state = ws_with_item("修复登录 bug", false);
        let app_meta = AppState::default();
        // 没有任何派发记录:手动开的终端/agent 应该拿不到任务标题,
        // 调用方据此 fallback 到 transcript 最后活动摘要。
        assert_eq!(
            ws_state.task_title_for_session(&app_meta, 1, "sess-1"),
            None
        );
    }

    #[test]
    fn task_title_for_session_ignores_other_project_or_session() {
        let ws_state = ws_with_item("修复登录 bug", false);
        let mut app_meta = AppState::default();
        app_meta.record_dispatch(1, "修复登录 bug", "sess-1".to_string());

        assert_eq!(
            ws_state.task_title_for_session(&app_meta, 2, "sess-1"),
            None,
            "同一份派发记录挂在别的 project_id 下不该命中"
        );
        assert_eq!(
            ws_state.task_title_for_session(&app_meta, 1, "sess-2"),
            None,
            "session_id 对不上不该命中"
        );
    }

    #[test]
    fn calendar_parse_month_day_accepts_mm_dd_and_rejects_bad_input() {
        assert_eq!(parse_month_day("08-10"), Some((8, 10)));
        assert_eq!(parse_month_day("8-3"), Some((8, 3)));
        assert_eq!(parse_month_day("13-01"), None, "月份越界");
        assert_eq!(parse_month_day("abc"), None);
        assert_eq!(parse_month_day("08"), None);
    }

    #[test]
    fn calendar_days_in_month_handles_leap_years() {
        assert_eq!(days_in_month(2024, 2), 29, "闰年 2 月 29 天");
        assert_eq!(days_in_month(2023, 2), 28, "平年 2 月 28 天");
        assert_eq!(days_in_month(2023, 4), 30);
        assert_eq!(days_in_month(2023, 1), 31);
    }

    #[test]
    fn calendar_first_weekday_of_1970_jan_is_thursday() {
        // 1970-01-01 是周四(0=周日 … 4=周四)。
        assert_eq!(first_weekday_of_month(1970, 1), 4);
    }

    #[test]
    fn calendar_days_from_civil_round_trips() {
        for (y, m, d) in [
            (1970, 1, 1),
            (2000, 2, 29),
            (2024, 3, 1),
            (2026, 8, 17),
            (2099, 12, 31),
        ] {
            let (yy, mm, dd) = civil_from_days(days_from_civil(y, m, d));
            assert_eq!((y as i64, m, d), (yy, mm, dd));
        }
    }

    #[test]
    fn update_content_edit_submit_rewrites_line() {
        let (_dir, root) = project_dir_with_todo("- [ ] 旧任务\n- [ ] 其它\n");
        let mut ws_state = WorkspaceState {
            items: vec![
                TodoItem {
                    text: "旧任务".into(),
                    done: false,
                },
                TodoItem {
                    text: "其它".into(),
                    done: false,
                },
            ],
            ..WorkspaceState::default()
        };
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::ContentEditStart(0),
            1,
            &root,
        );
        assert!(ws_state.editing_content.is_some());
        // 原生 `text_editor` 语义:先 `SelectAll` 选中整段,再 `Edit::Paste`
        // 整体改写为新文字(不再是单行 `text_input` 的"整缓冲替换")。
        update(
            &mut ws_state,
            &mut app_state,
            Message::ContentEdit(iced_widget::text_editor::Action::SelectAll),
            1,
            &root,
        );
        update(
            &mut ws_state,
            &mut app_state,
            Message::ContentEdit(iced_widget::text_editor::Action::Edit(
                iced_widget::text_editor::Edit::Paste("新".to_string().into()),
            )),
            1,
            &root,
        );
        // 回车提交:`text_editor` 报 `Edit::Enter`,`update` 就地落盘。
        update(
            &mut ws_state,
            &mut app_state,
            Message::ContentEdit(iced_widget::text_editor::Action::Edit(
                iced_widget::text_editor::Edit::Enter,
            )),
            1,
            &root,
        );
        assert!(ws_state.editing_content.is_none());
        let content = std::fs::read_to_string(todo_path(&root)).unwrap();
        assert!(content.contains("- [ ] 新\n"), "内容应改写: {content}");
        assert_eq!(ws_state.items[0].text, "新");
    }

    #[test]
    fn update_content_edit_cancel_discards() {
        let (_dir, root) = project_dir_with_todo("- [ ] 旧任务\n");
        let mut ws_state = ws_with_item("旧任务", false);
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::ContentEditStart(0),
            1,
            &root,
        );
        update(
            &mut ws_state,
            &mut app_state,
            Message::ContentEdit(iced_widget::text_editor::Action::Edit(
                iced_widget::text_editor::Edit::Paste("x".to_string().into()),
            )),
            1,
            &root,
        );
        // `text_editor` 没有 `Cancel` 消息:丢弃半输入走"失焦清空"路径
        // (没有打开项目时 `App::set_todo_content_focused` 调 `cancel_content_edit`)。
        ws_state.cancel_content_edit();
        assert!(ws_state.editing_content.is_none());
        let content = std::fs::read_to_string(todo_path(&root)).unwrap();
        assert_eq!(content, "- [ ] 旧任务\n", "取消不该改动文件");
    }

    #[test]
    fn update_calendar_pick_writes_plan_date() {
        let (_dir, root) = project_dir_with_todo("- [ ] 任务A\n");
        let mut ws_state = ws_with_item("任务A", false);
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::CalendarOpen(0),
            1,
            &root,
        );
        assert!(ws_state.calendar_popup_open());
        update(
            &mut ws_state,
            &mut app_state,
            Message::CalendarPick(0, "08-10".to_string()),
            1,
            &root,
        );
        assert!(!ws_state.calendar_popup_open(), "选中日期后关闭日历");
        let key = todo_line_key("任务A");
        assert_eq!(
            app_state.meta_for(1, key).unwrap().plan_date.as_deref(),
            Some("08-10")
        );
    }
}
