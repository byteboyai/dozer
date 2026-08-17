//! `.dozer/todo.md` 任务列表解析（Todo 面板 design，2026-08-06；已并入
//! `extensions::todo`）：
//! 标准两态 checkbox（`- [ ]`/`- [x]`），git 可追踪，agent 可直接读写。
//! 解析风格镜像 `goal.rs`——手写、宽松，不引入 markdown 库；格式意外
//! （多级缩进、非 checkbox 正文）一律忽略，不因为文件"长得不标准"而失败。

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::workspace::AddrEvent;
use crate::{icons, theme};
use iced_widget::core::{Border, Color, Element, Length, mouse};
use iced_widget::{MouseArea, button, column, container, rich_text, row, scrollable, span, text};

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
    /// 新增任务框是否处于自绘编辑态。本 app 每帧重建界面,原生 `text_input`
    /// 留不住焦点、也不参与 main.rs 的键盘路由裁决,不加这个标记的话打字
    /// 会同时漏进已聚焦的终端(agent 输入),见 `todo_footer_bar`。
    add_editing: bool,
    filter: TodoFilter,
    view_mode: TodoViewMode,
    selected_row: Option<usize>,
    /// 已生效的搜索关键词(列表过滤用)。打字期间只改草稿 `search_draft`,
    /// 回车/点右侧搜索按钮才落成这里(与文件树搜索 `search_query` 同款
    /// "草稿→提交"模型——本 app 的 iced 界面每帧重建、原生 `text_input`
    /// 留不住焦点,搜索必须用自绘输入 + main.rs 键盘拦截路由,见 design)。
    search: String,
    /// 搜索框编辑态草稿。`search_editing` 为真时按键经 main.rs 路由成
    /// `SearchEvent`,只动草稿,不重新过滤;回车/点搜索按钮才提交。
    search_draft: String,
    /// 搜索框是否处于自绘编辑态(main.rs 键盘路由用)。
    search_editing: bool,
    dispatch_open: Option<usize>,
    pending_dispatch: std::collections::HashMap<usize, String>,
    editing_plan_date: Option<(usize, String)>,
    /// 状态 pill 菜单展开态(卡片下标)，`None` = 未展开。跟 `dispatch_open`
    /// 同一种"同时只能有一个"模型，不做多卡片同时展开。
    state_pill_open: Option<usize>,
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

    /// 状态 pill 菜单是否打开(内核 `App::todo_state_pill_open` 键盘 Esc
    /// 关闭用,同 `dispatch_popup_open` 的既有模式——菜单打开后除了选中
    /// 一个选项之外没有别的关闭入口会把用户逼着做一次未必想要的状态
    /// 变更,Esc 必须能单独退出)。
    pub fn state_pill_menu_open(&self) -> bool {
        self.state_pill_open.is_some()
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

    /// 搜索框是否处于自绘编辑态(main.rs 键盘路由用)。
    pub fn search_editing(&self) -> bool {
        self.search_editing
    }

    /// 失焦退出搜索编辑态(`Workspace::blur_inputs` 用):草稿保留。
    pub fn cancel_search_edit(&mut self) {
        self.search_editing = false;
    }

    /// 新增任务框是否处于自绘编辑态(main.rs 键盘路由用)。
    pub fn add_editing(&self) -> bool {
        self.add_editing
    }

    /// 失焦退出新增任务编辑态(`Workspace::blur_inputs` 用):草稿保留,
    /// 与搜索框同款——半输入的任务文字不该因为点了别处就丢。
    pub fn cancel_add_edit(&mut self) {
        self.add_editing = false;
    }

    /// 计划时间行内编辑态是否打开(main.rs 键盘路由用)。
    pub fn plan_date_editing(&self) -> bool {
        self.editing_plan_date.is_some()
    }

    /// 失焦退出计划时间编辑态(`Workspace::blur_inputs` 用):直接丢弃
    /// 半输入。`editing_plan_date` 是点日期徽章才弹出的一次性行内编辑,
    /// 不是常驻输入框,行为对齐项目树重命名(`cancel_tree_edit`)而不是
    /// 搜索框。
    pub fn cancel_plan_date_edit(&mut self) {
        self.editing_plan_date = None;
    }

    /// 是否正在拖拽排序(main.rs 鼠标释放路由 + about_to_wait 持续重绘用)。
    pub fn drag_active(&self) -> bool {
        self.drag.is_some()
    }

    /// 取消进行中的拖拽排序(失焦/切面板时清状态,避免卡在拖拽中间)。
    pub fn cancel_drag(&mut self) {
        self.drag = None;
    }

    /// 草稿落成为生效的 `search` 过滤词;保留编辑态(便于连续改词)。
    pub fn commit_search(&mut self) {
        self.search = self.search_draft.clone();
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
    /// 点新增任务框进入自绘编辑态(`add_editing = true`),后续按键经
    /// main.rs 路由成 `AddEvent`,不再漏进终端(同 `SearchEditStart`)。
    AddEditStart,
    /// 编辑态下的按键:文本/退格改草稿,`AddrEvent::Submit` 落盘新任务
    /// (无独立"提交按钮"入口——新增任务只有回车这一条提交路径,不像
    /// 搜索框还有个放大镜按钮,故没有单独的 `AddSubmit` 消息)。
    AddEvent(AddrEvent),
    FilterSet(TodoFilter),
    ViewModeSet(TodoViewMode),
    RowSelect(Option<usize>),
    /// 点搜索框进入自绘编辑态(`search_editing = true`),后续按键经 main.rs
    /// 路由成 `SearchEvent`,不再漏进终端。
    SearchEditStart,
    /// 编辑态下的按键:只动草稿 `search_draft`,不重新过滤(需提交)。
    SearchEvent(AddrEvent),
    /// 回车 / 点右侧搜索按钮:把草稿落成生效的 `search` 过滤词。
    SearchSubmit,
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
    DispatchNew(usize, crate::workspace::PickerLaunch),
    PlanDateEditStart(usize),
    /// 编辑态下的按键:文本/退格改草稿,`AddrEvent::Submit` 落盘计划
    /// 时间,`AddrEvent::Cancel` 清空编辑态(同 `AddEvent`,回车是唯一
    /// 提交路径,没有单独的 `PlanDateSubmit` 消息)。
    PlanDateEvent(AddrEvent),
    /// pill 菜单选中"待办"/"已完成"时发出，`bool` 是**目标** `done` 值
    /// (显式设置，不是翻转)。当前 `done` 已经等于目标值时视为 no-op，
    /// 不重复写盘——见 `set_done()` 的实现注释。
    SetDone(usize, bool),
    /// 展开某张卡片的状态 pill 菜单(待办/已完成 二选一)。
    StatePillOpen(usize),
    /// 收起状态 pill 菜单(选中某项后，或点击外部)。
    StatePillClose,
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

/// `Toggle`/`SetDone` 共用的写盘逻辑：把任务行的 `[ ]`/`[x]` 改成
/// `target_done` 对应的目标值(不是翻转)。`item.done == target_done` 时
/// 直接 no-op 返回，不读写文件、不碰 `completed_at`——这是"进行中"态点
/// pill 菜单"待办"选项时的关键行为：`done` 本来就是 `false`，不应该因为
/// 用户点了这个选项就产生任何副作用。
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

/// `AddEvent(Submit)` 的写盘逻辑:把草稿追加成新任务行,空白草稿
/// (trim 后)no-op。
fn commit_add_task(ws_state: &mut WorkspaceState, project_path: &std::path::Path) {
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

/// `PlanDateEvent(Submit)` 的写盘逻辑:把草稿落成 `AppState` 里的计划
/// 时间元数据,并退出编辑态。
fn commit_plan_date(ws_state: &mut WorkspaceState, app_state: &mut AppState, project_id: i64) {
    if let Some((idx, draft)) = ws_state.editing_plan_date.clone()
        && let Some(text) = ws_state.items.get(idx).map(|item| item.text.clone())
    {
        app_state.set_plan_date(project_id, &text, draft);
    }
    ws_state.editing_plan_date = None;
}

/// 处理除 `DispatchToExisting`/`DispatchNew` 之外的消息,统一接收两块
/// 状态——`Toggle`/`PlanDateEditStart`/`PlanDateEvent` 需要读写
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
            let target = !item.done;
            set_done(ws_state, app_state, idx, project_id, project_path, target);
        }
        Message::SetDone(idx, target_done) => {
            set_done(
                ws_state,
                app_state,
                idx,
                project_id,
                project_path,
                target_done,
            );
            ws_state.state_pill_open = None;
        }
        Message::StatePillOpen(idx) => ws_state.state_pill_open = Some(idx),
        Message::StatePillClose => ws_state.state_pill_open = None,
        Message::AddEditStart => ws_state.add_editing = true,
        Message::AddEvent(ev) => {
            // 编辑态之外(失焦)的 `AddEvent` 一律忽略,避免草稿被污染
            // (同 `SearchEvent` 的既有约定)。
            if !ws_state.add_editing {
                return;
            }
            match ev {
                AddrEvent::Text(s) => ws_state.add_draft.push_str(&s),
                AddrEvent::Backspace => {
                    ws_state.add_draft.pop();
                }
                AddrEvent::Cancel => ws_state.add_editing = false,
                AddrEvent::Submit => commit_add_task(ws_state, project_path),
            }
        }
        Message::FilterSet(f) => ws_state.filter = f,
        Message::ViewModeSet(m) => ws_state.view_mode = m,
        Message::RowSelect(idx) => {
            ws_state.selected_row = idx;
            // 待办卡片被按下即"准备拖":记下它的 item-index 作为拖拽源。
            // 已完成不参与拖拽(只有待办才进 `drag`)。注意这跟选中态是两件
            // 独立的事——纯点击(不移动)也会落到这里,但松手时
            // source==target 不写盘,只是正常选中切换(同 `TabDragMove`
            // 的"按住=准备拖,移动才换位"语义)。
            if let Some(i) = idx {
                if let Some(item) = ws_state.items.get(i) {
                    if !item.done {
                        ws_state.drag = Some(TodoDrag {
                            source_idx: i,
                            target_idx: i,
                        });
                    }
                }
            }
        }
        Message::SearchEditStart => ws_state.search_editing = true,
        Message::SearchEvent(ev) => {
            // 编辑态之外(失焦)的 `SearchEvent` 一律忽略,避免草稿被污染。
            if !ws_state.search_editing {
                return;
            }
            match ev {
                AddrEvent::Text(s) => ws_state.search_draft.push_str(&s),
                AddrEvent::Backspace => {
                    ws_state.search_draft.pop();
                }
                AddrEvent::Cancel => ws_state.search_editing = false,
                AddrEvent::Submit => ws_state.commit_search(),
            }
        }
        Message::SearchSubmit => {
            ws_state.commit_search();
            ws_state.search_editing = false;
        }
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
            }
        }
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
        Message::PlanDateEvent(ev) => {
            // 编辑态之外(已提交/已取消)的 `PlanDateEvent` 一律忽略。
            if ws_state.editing_plan_date.is_none() {
                return;
            }
            match ev {
                AddrEvent::Text(s) => {
                    if let Some((_, draft)) = ws_state.editing_plan_date.as_mut() {
                        draft.push_str(&s);
                    }
                }
                AddrEvent::Backspace => {
                    if let Some((_, draft)) = ws_state.editing_plan_date.as_mut() {
                        draft.pop();
                    }
                }
                AddrEvent::Cancel => ws_state.editing_plan_date = None,
                AddrEvent::Submit => commit_plan_date(ws_state, app_state, project_id),
            }
        }
        Message::DispatchToExisting(..) | Message::DispatchNew(..) => {
            unreachable!(
                "DispatchToExisting/DispatchNew 由内核在 Message::Todo 分支里直接处理\
                 (需要终端会话读写能力),不会转发到这里"
            )
        }
    }
}

/// Todo 面板渲染成两个独立的边框 pane(镜像 Files/Project 面板已有的
/// "侧栏 + 内容区，中间一条可拖拽分隔线"两栏模式，不再是单个面板内部一个
/// `row![sidebar, body]`)——调用方(`app.rs` 的 `LeftView::Todo` 分支)负责
/// 拼 `row![sidebar_pane, divider_bar(Divider::TodoSplit, ..), content_pane]`。
/// 左栏：面板头 + 分类导航。右栏：列表/MARKDOWN 视图切换 tab + 视图
/// 主体。
#[allow(clippy::too_many_arguments)]
pub fn view<'a>(
    app_state: &'a AppState,
    ws_state: &'a WorkspaceState,
    project_id: i64,
    tabs: &[SessionTabSummary],
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
    let header = container(crate::homespace::home_panel_head(
        icons::IconKind::ListTodo,
        "Todo",
    ))
    .padding([20, 20]);

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
                .map(|d| tabs.iter().any(|t| t.session_id == d.session_id && t.alive))
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

    // ---- 左栏 pane：header + 分类导航 ----
    let mut nav = column![].spacing(4).padding([12, 8]);
    for (filter, count) in counts {
        nav = nav.push(todo_category_button(filter, count, ws_state.filter));
    }
    let sidebar_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(column![header, nav].height(Length::Fill))
            .width(sidebar_width)
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(theme::color::BG.into()),
                border: sidebar_outer,
                ..container::Style::default()
            })
            .into();

    // ---- 右栏 pane：tab 段 + 视图主体 ----
    let tabs_bar = todo_view_tabs(ws_state.view_mode);
    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match ws_state.view_mode {
            TodoViewMode::List => todo_list_view(app_state, ws_state, project_id, &states, tabs),
            TodoViewMode::Markdown => todo_markdown_view(project_path),
        };
    let content_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(column![tabs_bar, crate::app::tab_divider(), body].height(Length::Fill))
            .width(content_width)
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(theme::color::BG.into()),
                border: content_outer,
                ..container::Style::default()
            })
            .into();

    (sidebar_pane, content_pane)
}

/// 底部快速新建栏，结构对齐 `project.rs::project_footer_bar`(1px BORDER
/// 分隔线 + `padding([6, 8])`)。列表视图使用。自绘输入(键盘走 main.rs
/// 拦截层路由成 `AddEvent`,不用原生 `text_input`——本 app 每帧重建界面,
/// 原生输入留不住焦点也不参与键盘路由裁决,打字会同时漏进已聚焦的终端,
/// 见 `todo_search_bar` 同款说明)。整体是 `button`,点击(`AddEditStart`)
/// 进编辑态。
fn todo_footer_bar<'a>(
    add_draft: &'a str,
    editing: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let field: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if add_draft.is_empty() && !editing {
            text("Initiate new task protocol..")
                .size(theme::font::body())
                .color(theme::color::DIM)
                .into()
        } else {
            let caret = if editing { "▏" } else { "" };
            text(format!("{add_draft}{caret}"))
                .size(theme::font::body())
                .color(theme::color::CREAM)
                .into()
        };

    let add_row = button(
        row![
            icons::view(
                icons::IconKind::SquarePlus,
                crate::theme::icon_size::row(),
                theme::color::GOLD
            ),
            field,
        ]
        .spacing(8)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .on_press(Message::AddEditStart)
    .width(Length::Fill)
    .padding(0)
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: Some(theme::color::BG.into()),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 0.0.into(),
        },
        text_color: theme::color::CREAM,
        ..button::Style::default()
    });

    let top_line = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::BORDER.into()),
            ..container::Style::default()
        });

    container(column![top_line, add_row].spacing(4))
        .width(Length::Fill)
        .padding([6, 8])
        .into()
}

/// 顶部搜索框:自绘输入(键盘走 main.rs 拦截层路由成 `SearchEvent`,不用
/// iced 原生 `text_input`——本 app 每帧重建界面,原生输入留不住焦点,打字
/// 会漏进已聚焦的终端)。左侧是输入框本体(点 `SearchEditStart` 进编辑态),
/// 右侧是提交按钮(回车 / 点它把草稿落成生效的 `search` 过滤词)。编辑态/
/// 已过滤时整框 GOLD 边框表示焦点归属 / 当前被搜索词收窄。
fn todo_search_bar<'a>(
    draft: &'a str,
    editing: bool,
    active: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let body = if draft.is_empty() && !editing {
        text("搜索任务…")
            .size(theme::font::body())
            .color(theme::color::DIM)
    } else {
        let caret = if editing { "▏" } else { "" };
        text(format!("{draft}{caret}"))
            .size(theme::font::body())
            .color(theme::color::CREAM)
    };
    let box_btn = button(body)
        .on_press(Message::SearchEditStart)
        .width(Length::Fill)
        .padding([6, 8])
        .style(move |_t: &iced_widget::Theme, _s| button::Style {
            background: Some(theme::color::BG.into()),
            border: Border {
                color: if editing || active {
                    theme::color::GOLD
                } else {
                    theme::color::BORDER
                },
                width: 1.0,
                radius: 4.0.into(),
            },
            text_color: theme::color::CREAM,
            ..button::Style::default()
        });
    let submit = button(icons::view(
        icons::IconKind::Search,
        crate::theme::icon_size::row(),
        theme::color::GOLD,
    ))
    .on_press(Message::SearchSubmit)
    .padding(6)
    .style(|_t, _s| button::Style {
        background: Some(theme::color::BG.into()),
        border: Border {
            color: theme::color::BORDER,
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: theme::color::GOLD,
        ..button::Style::default()
    });
    row![box_btn, submit]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center)
        .into()
}

/// 列表视图主体：搜索栏 + 编号行列表 + 底部新增输入。
fn todo_list_view<'a>(
    app_state: &'a AppState,
    ws_state: &'a WorkspaceState,
    project_id: i64,
    states: &[TodoState],
    tabs: &[SessionTabSummary],
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let visible_idx = filter_todos(&ws_state.items, states, ws_state.filter, &ws_state.search);

    let existing_tabs: Vec<(&str, String)> = tabs
        .iter()
        .filter(|t| t.alive)
        .map(|t| (t.session_id.as_str(), t.title.clone()))
        .collect();

    // 搜索框的水平/垂直间距对齐任务卡片的间距规格(卡片列表 `list` 是
    // `spacing(8)` + `padding([0, 20])`):左右 20、上下 8,不再贴边顶到
    // tab 分隔线与首张卡片。
    let search = container(todo_search_bar(
        &ws_state.search_draft,
        ws_state.search_editing,
        !ws_state.search.is_empty(),
    ))
    .padding([8, 20])
    .width(Length::Fill);

    let mut list = column![].spacing(8).padding([0, 20]);
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
                ws_state,
                project_id,
                states,
                &existing_tabs,
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
                ws_state,
                project_id,
                states,
                &existing_tabs,
                pending_len + i + 1,
                idx,
                grabbing,
                false,
            ));
        }
    }

    column![
        search,
        scrollable(list).height(Length::Fill),
        todo_footer_bar(&ws_state.add_draft, ws_state.add_editing),
    ]
    .height(Length::Fill)
    .into()
}

/// MARKDOWN 占位：只读展示 `.dozer/todo.md` 原始源码。
fn todo_markdown_view<'a>(
    project_path: Option<&Path>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let src = project_path
        .map(todo_path)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| "# 暂无 .dozer/todo.md".to_string());
    let body = container(
        text(src)
            .size(theme::font::body())
            .color(theme::color::CREAM),
    )
    .padding([12, 20])
    .width(Length::Fill);
    scrollable(body).height(Length::Fill).into()
}

/// `todo_list_view` 单行的渲染分派:计划时间编辑态 → `todo_plan_date_edit_row`,
/// 否则 → `todo_card`。从 `todo_list_view` 的循环体里拆出来,好让 pending/
/// done 两段各自的 `for` 循环别重复这段查表+分支逻辑。
#[allow(clippy::too_many_arguments)]
fn todo_list_row<'a, 'b>(
    app_state: &'a AppState,
    ws_state: &'a WorkspaceState,
    project_id: i64,
    states: &[TodoState],
    // 独立生命周期 `'b`,不绑定到返回值的 `'a`:`todo_card` 分支返回
    // `Element<'static, ..>`(内部已经把要用的数据 clone 出来,不持有
    // 任何借用),调用方传进来的 `existing_tabs` 常是函数体内构造的短命
    // 局部 `Vec`,跟 `'a` 混在一起会逼编译器把这个短生命周期错误地传染
    // 给整个返回值。
    existing_tabs: &'b [(&'b str, String)],
    number: usize,
    idx: usize,
    grabbing: bool,
    is_drag_source: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let item = &ws_state.items[idx];
    let key = todo_line_key(&item.text);
    let meta = app_state.meta_for(project_id, key);
    let dispatch = meta.and_then(|m| m.dispatch.as_ref());
    if let Some((editing_idx, draft)) = &ws_state.editing_plan_date
        && *editing_idx == idx
    {
        return todo_plan_date_edit_row(item, draft);
    }
    todo_card(
        number,
        idx,
        item,
        states[idx],
        meta,
        dispatch,
        ws_state.selected_row == Some(idx),
        ws_state.dispatch_open == Some(idx),
        ws_state.state_pill_open == Some(idx),
        existing_tabs,
        grabbing,
        is_drag_source,
    )
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
            background: Some(theme::color::GOLD.into()),
            border: Border {
                radius: 2.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// 统一卡片组件：列表视图使用的边框卡片视觉，取代原来的
/// `todo_row`(扁平高亮行)。结构自上而下：编号 + 日期徽章 → checkbox +
/// 任务文字 → 派发按钮(仅待办未派发时) + 状态 pill。选中态左侧加 3px
/// 金色竖条(对齐原 `todo_row` 的 `accent` 处理)。
#[allow(clippy::too_many_arguments)]
fn todo_card<'a>(
    number: usize,
    idx: usize,
    item: &'a TodoItem,
    state: TodoState,
    meta: Option<&'a TodoTaskMeta>,
    dispatch: Option<&'a DispatchRecord>,
    selected: bool,
    dispatch_open: bool,
    state_pill_open: bool,
    existing_tabs: &'a [(&'a str, String)],
    grabbing: bool,
    is_drag_source: bool,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let done = item.done;

    // ---- 顶部行：编号 + 日期徽章 ----
    let number_text = text(format!("#{number:03}"))
        .size(theme::font::caption())
        .color(theme::color::DIM);

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
                    icons::IconKind::History,
                    crate::theme::icon_size::row(),
                    theme::color::DIM
                ),
                text(date_label)
                    .size(theme::font::caption())
                    .color(theme::color::DIM),
            ]
            .spacing(4)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        )
        .interaction(mouse::Interaction::Pointer)
        .on_press(Message::PlanDateEditStart(idx))
        .into();

    let top_row = row![
        number_text,
        iced_widget::space::Space::new()
            .width(Length::Fill)
            .height(Length::Shrink),
        date_badge,
    ]
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .spacing(6);

    // ---- 中部：checkbox + 任务文字（勾选/删除线处理与原 todo_row 一致）----
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
    let label_area: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> =
        MouseArea::new(container(label).width(Length::Fill))
            .interaction(mouse::Interaction::Pointer)
            .on_press(Message::RowSelect(if selected { None } else { Some(idx) }))
            .into();

    let body_row = row![checkbox, label_area]
        .spacing(10)
        .align_y(iced_widget::core::alignment::Vertical::Center);

    // ---- 底部行：派发按钮(仅待办未派发) + 状态 pill ----
    let mut bottom = row![]
        .spacing(8)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    if state == TodoState::Pending && dispatch.is_none() {
        let dispatch_btn = button(icons::view(
            icons::IconKind::BotMessageSquare,
            crate::theme::icon_size::row(),
            theme::color::GOLD,
        ))
        .on_press(Message::DispatchOpen(idx))
        .padding(6)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            border: Border {
                color: theme::color::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..button::Style::default()
        });
        bottom = bottom.push(dispatch_btn);
    }
    bottom = bottom.push(state_pill(idx, state));

    let bottom_row = row![
        iced_widget::space::Space::new()
            .width(Length::Fill)
            .height(Length::Shrink),
        bottom,
    ];

    let card_body = column![top_row, body_row, bottom_row].spacing(8);

    let accent = container(iced_widget::space::Space::new())
        .width(Length::Fixed(3.0))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: if selected {
                Some(theme::color::GOLD.into())
            } else {
                None
            },
            ..container::Style::default()
        });

    let inner = row![accent, container(card_body).padding(10).width(Length::Fill)].spacing(0);

    let card = container(inner)
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::CARD.into()),
            // 正在被拖起的那张卡片描边变金、加粗——跟"插入指示线"配合给
            // 出"这张卡片被拿起来了/会落在指示线那里"的反馈,不再靠其它
            // 卡片瞬间跳位来表达换位(见 `todo_list_view` 的改版说明)。
            border: if is_drag_source {
                Border {
                    color: theme::color::GOLD,
                    width: 1.5,
                    radius: 6.0.into(),
                }
            } else {
                Border {
                    color: theme::color::BORDER,
                    width: 1.0,
                    radius: 6.0.into(),
                }
            },
            ..container::Style::default()
        });

    let mut stacked = column![card];
    if dispatch_open {
        stacked = stacked.push(todo_dispatch_popup(idx, existing_tabs));
    }
    if state_pill_open {
        stacked = stacked.push(state_pill_menu(idx));
    }
    // 拖拽换位感应层:只补一个 `on_move`(光标移动过本卡就发 `DragMove`),
    // 子按钮(勾选/派发/pill)照常各自吞"按下"事件——同 `tab_drag_surface`
    // 的那套。按下=准备拖由 `RowSelect` 置位,这里 `on_move` 只认"正在拖"
    // 的时刻(`DragMove` 内部 no-op 检查)。拖拽中整张卡显示抓取光标。
    let area = MouseArea::new(stacked).on_move(move |_| Message::DragMove(idx));
    if grabbing {
        area.interaction(mouse::Interaction::Grabbing).into()
    } else {
        area.into()
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

/// 状态 pill：三态统一成同一种紧凑圆角形状，文字/颜色随 `state` 变。
/// `Pending`/`Done` 可点击(发 `StatePillOpen`，弹出二选一菜单)；
/// `InProgress` 是推导值，不接受直接设置，pill 只读展示，不挂 `on_press`。
fn state_pill(
    idx: usize,
    state: TodoState,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (label, border_color, text_color, bg) = match state {
        TodoState::Pending => ("待办", theme::color::BORDER, theme::color::DIM, None),
        TodoState::InProgress => ("进行中", theme::color::GREEN, theme::color::GREEN, None),
        TodoState::Done => (
            "已完成",
            theme::color::GREEN,
            theme::color::BG,
            Some(theme::color::GREEN),
        ),
    };
    let mut btn = button(text(label).size(theme::font::caption()).color(text_color))
        .padding([4, 10])
        .style(move |_t: &iced_widget::Theme, _s| button::Style {
            background: bg.map(Into::into),
            text_color,
            border: Border {
                color: border_color,
                width: 1.0,
                radius: 10.0.into(),
            },
            ..button::Style::default()
        });
    if state != TodoState::InProgress {
        btn = btn.on_press(Message::StatePillOpen(idx));
    }
    btn.into()
}

/// pill 菜单：待办/已完成 二选一，选中发 `SetDone(idx, 目标值)`。样式镜像
/// 现有 `todo_dispatch_popup`(CARD 底 + BORDER 描边)。
fn state_pill_menu(
    idx: usize,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let option = |label: &'static str, target_done: bool, color: Color| {
        button(text(label).size(theme::font::body()).color(color))
            .on_press(Message::SetDone(idx, target_done))
            .width(Length::Fill)
            .padding([6, 12])
            .style(move |_t: &iced_widget::Theme, _s| button::Style {
                background: None,
                text_color: color,
                ..button::Style::default()
            })
    };
    container(
        column![
            option("待办", false, theme::color::DIM),
            option("已完成", true, theme::color::GREEN),
        ]
        .spacing(2),
    )
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

/// 计划时间内联编辑态：任务文本 + 一个自绘输入框，回车提交。自绘原因同
/// `todo_footer_bar`(原生 `text_input` 不参与 main.rs 键盘路由裁决,打字
/// 会漏进终端);键盘走 main.rs 拦截层路由成 `PlanDateEvent`。这行只在
/// `editing_plan_date` 命中时才会被渲染出来,不需要额外的点击进入态。
fn todo_plan_date_edit_row<'a>(
    item: &'a TodoItem,
    draft: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let field = if draft.is_empty() {
        text("计划时间，如 08-10")
            .size(theme::font::caption())
            .color(theme::color::DIM)
    } else {
        text(format!("{draft}▏"))
            .size(theme::font::caption())
            .color(theme::color::CREAM)
    };
    row![
        text(item.text.clone())
            .size(theme::font::body())
            .color(theme::color::CREAM),
        container(field)
            .width(Length::Fixed(140.0))
            .padding([2, 6])
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(theme::color::CARD.into()),
                border: Border {
                    color: theme::color::GOLD,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..container::Style::default()
            }),
    ]
    .spacing(10)
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .padding([10, 20])
    .into()
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
        theme::color::CREAM
    } else {
        theme::color::DIM
    };
    button(
        row![
            icons::view(
                icon,
                crate::theme::icon_size::row(),
                if active {
                    theme::color::GOLD
                } else {
                    theme::color::DIM
                }
            ),
            text(label).size(theme::font::body()).color(fg),
            iced_widget::space::Space::new()
                .width(Length::Fill)
                .height(Length::Shrink),
            text(format!("{count}"))
                .size(theme::font::caption())
                .color(if active {
                    theme::color::GOLD
                } else {
                    theme::color::DIM
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
            Some(theme::color::CARD.into())
        } else {
            None
        },
        text_color: fg,
        border: Border {
            color: if active {
                theme::color::GOLD
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
    .padding([8, 20])
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
        theme::color::CREAM
    } else {
        theme::color::DIM
    };
    let icon_color = if active {
        theme::color::GOLD
    } else {
        theme::color::DIM
    };
    button(
        row![
            icons::view(icon, crate::theme::icon_size::row(), icon_color),
            text(label).size(theme::font::caption()).color(fg),
        ]
        .spacing(6)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .on_press(Message::ViewModeSet(mode))
    .padding([6, 12])
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: if active {
            Some(theme::color::CARD.into())
        } else {
            None
        },
        text_color: fg,
        border: Border {
            color: if active {
                theme::color::BORDER
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
    fn update_set_done_pending_to_done_writes_and_stamps_completed_at() {
        let (_dir, root) = project_dir_with_todo("- [ ] 任务A\n");
        let mut ws_state = ws_with_item("任务A", false);
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::SetDone(0, true),
            1,
            &root,
        );
        assert!(ws_state.items[0].done);
        let key = todo_line_key("任务A");
        assert!(app_state.meta_for(1, key).unwrap().completed_at.is_some());
        let content = std::fs::read_to_string(todo_path(&root)).unwrap();
        assert!(content.contains("- [x] 任务A"));
    }

    #[test]
    fn update_set_done_noop_when_already_target_value() {
        // 模拟"进行中"态点"待办"：done 已经是 false，SetDone(idx, false)
        // 必须整个是 no-op(不读写文件、不碰 completed_at)，否则会把还在
        // 执行的任务误标记。
        let (_dir, root) = project_dir_with_todo("- [ ] 任务A\n");
        let mut ws_state = ws_with_item("任务A", false);
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::SetDone(0, false),
            1,
            &root,
        );
        assert!(!ws_state.items[0].done);
        let key = todo_line_key("任务A");
        assert!(
            app_state.meta_for(1, key).is_none(),
            "no-op 不该写 completed_at"
        );
        let content = std::fs::read_to_string(todo_path(&root)).unwrap();
        assert_eq!(content, "- [ ] 任务A\n", "no-op 不该改动磁盘文件");
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
    fn update_set_done_done_to_pending_clears_completed_at() {
        let (_dir, root) = project_dir_with_todo("- [x] 任务A\n");
        let mut ws_state = ws_with_item("任务A", true);
        let mut app_state = AppState::default();
        app_state.set_completed_at(1, "任务A", true);
        update(
            &mut ws_state,
            &mut app_state,
            Message::SetDone(0, false),
            1,
            &root,
        );
        assert!(!ws_state.items[0].done);
        let key = todo_line_key("任务A");
        assert!(app_state.meta_for(1, key).unwrap().completed_at.is_none());
        let content = std::fs::read_to_string(todo_path(&root)).unwrap();
        assert!(content.contains("- [ ] 任务A"));
    }

    #[test]
    fn update_add_submit_appends_and_clears_draft() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState {
            add_draft: "新任务".to_string(),
            add_editing: true,
            ..WorkspaceState::default()
        };
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::AddEvent(AddrEvent::Submit),
            1,
            &root,
        );
        assert!(ws_state.add_draft.is_empty());
        assert_eq!(ws_state.items.len(), 1);
        assert_eq!(ws_state.items[0].text, "新任务");
    }

    #[test]
    fn update_add_submit_empty_draft_is_noop() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState {
            add_editing: true,
            ..WorkspaceState::default()
        };
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::AddEvent(AddrEvent::Submit),
            1,
            &root,
        );
        assert!(ws_state.items.is_empty());
    }

    #[test]
    fn update_add_event_ignored_outside_editing() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::AddEvent(AddrEvent::Text("x".to_string())),
            1,
            &root,
        );
        assert!(ws_state.add_draft.is_empty());
    }

    #[test]
    fn update_add_edit_start_then_event_builds_draft_and_submits() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::AddEditStart,
            1,
            &root,
        );
        assert!(ws_state.add_editing());
        update(
            &mut ws_state,
            &mut app_state,
            Message::AddEvent(AddrEvent::Text("新任务".to_string())),
            1,
            &root,
        );
        assert_eq!(ws_state.add_draft, "新任务");
        update(
            &mut ws_state,
            &mut app_state,
            Message::AddEvent(AddrEvent::Submit),
            1,
            &root,
        );
        assert!(ws_state.add_draft.is_empty());
        assert_eq!(ws_state.items.len(), 1);
        assert_eq!(ws_state.items[0].text, "新任务");
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
            Message::SearchEditStart,
            1,
            &root,
        );
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchEvent(AddrEvent::Text("关键字".to_string())),
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
    fn update_state_pill_open_and_close_toggle_field() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::StatePillOpen(3),
            1,
            &root,
        );
        assert_eq!(ws_state.state_pill_open, Some(3));
        update(
            &mut ws_state,
            &mut app_state,
            Message::StatePillClose,
            1,
            &root,
        );
        assert_eq!(ws_state.state_pill_open, None);
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
            Message::PlanDateEvent(AddrEvent::Submit),
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
    fn update_plan_date_event_ignored_without_editing_state() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = ws_with_item("任务A", false);
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::PlanDateEvent(AddrEvent::Text("0".to_string())),
            1,
            &root,
        );
        assert!(ws_state.editing_plan_date.is_none());
    }

    #[test]
    fn update_plan_date_event_builds_draft_and_submits() {
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
        update(
            &mut ws_state,
            &mut app_state,
            Message::PlanDateEvent(AddrEvent::Text("08-10".to_string())),
            1,
            &root,
        );
        assert_eq!(ws_state.editing_plan_date, Some((0, "08-10".to_string())));
        update(
            &mut ws_state,
            &mut app_state,
            Message::PlanDateEvent(AddrEvent::Submit),
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
    fn take_pending_dispatch_removes_and_returns_once() {
        let mut ws_state = WorkspaceState::default();
        ws_state.pending_dispatch.insert(7, "任务A".to_string());
        assert_eq!(ws_state.take_pending_dispatch(7), Some("任务A".to_string()));
        assert_eq!(ws_state.take_pending_dispatch(7), None, "取过一次就没了");
    }
}
