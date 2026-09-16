//! Todo 面板(设计 2026-08-06;SQLite 迁移计划 2026-09-01;已并入
//! `extensions::todo`):任务列表的唯一权威源在 dozerd 侧 `dozer.db` 的
//! SQLite `todos` 表(TodoStore 归档)。本模块只持有每个 `Workspace` 上的
//! 渲染态(`WorkspaceState`)与视图/更新逻辑,List/Add/Toggle 经 `Client`
//! 走 UDS 与 dozerd 同步;不再直接读写 `.dozer/todo.md`/`todo_meta.json`
//! (MARKDOWN 整文件视图已随迁移移除)。

use crate::app::{App, HoverId};
use crate::theme;
use crate::workspace::{Workspace, agent_icon};
use byteui::interaction::icons;
use dozer_client::Client;
use dozer_core::protocol::{AgentKind, CategoryInfo, TodoInfo};
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Border, Color, Element, Length, Padding, Rectangle, mouse};
use iced_widget::{
    MouseArea, button, column, container, rich_text, row, scrollable, space, span, text,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoState {
    Pending,
    InProgress,
    Done,
    /// 搁置(用户主动"拿回/暂停",服务端存储位 `paused`)。优先级在
    /// `Done` 之下、`InProgress` 之上:一个既完成又(残留)搁置的按完成算;
    /// 派发但被用户搁置的按搁置算——搁置即"停手追那家伙",不残留进行中。
    Suspended,
}

/// 右区当前展示的视图模式。`List` 是现有的一列一列的任务列表(卡片逐行
/// 堆叠/可拖拽排序);`Kanban` 是看板视图,按状态分列展示同批任务——**一期
/// 只占位,暂无渲染实现**(见下方 `kanban_placeholder`),先让用户在两种视图
/// 间切换的入口可用而不报错。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TodoView {
    #[default]
    List,
    Kanban,
}

/// `done` 为真直接 `Done`（已完成优先，不管有没有派发/搁置记录）；
/// 否则若 `paused` 为真 → `Suspended`（搁置即停手，覆盖派发/进行中）；
/// 否则看派发记录：存在且 session 存活 → `InProgress`；否则 → `Pending`。
/// 四种派生状态:Pending / InProgress / Suspended / Done。
/// `plan_date`/`completed_at`/`paused` 的权威值现在都在 `TodoInfo` 自身上
/// （不再走 sidecar）。
pub fn todo_display_state(item: &TodoInfo, target_alive: bool) -> TodoState {
    if item.done {
        return TodoState::Done;
    }
    if item.paused {
        return TodoState::Suspended;
    }
    if item.dispatch_session_id.is_some() && target_alive {
        TodoState::InProgress
    } else {
        TodoState::Pending
    }
}

/// 正在"谈/进行"（等于总能在"活动段"里被拖拽重排的子集）：`!done && !paused`。
/// 搁置（`paused`）是中间第三个不可拖放的段，完成是沉底段——都不可拖。
fn is_active_todo(item: &TodoInfo) -> bool {
    !item.done && !item.paused
}

/// 分类树的当前过滤选中态。`All`/`Uncategorized` 是钉在树顶的两个伪
/// 节点(不对应真实 `CategoryInfo` 行),`Node(id)` 才是用户自建的真实
/// 分类节点。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CategoryFilter {
    #[default]
    All,
    Uncategorized,
    Node(i64),
}

/// 搜索框左前「按状态筛选」下拉的选中态:`All` = 不看状态(等于是三段都
/// 算),`Status(s)` = 只看这一种派生状态的卡片。四种状态(Pending/
/// InProgress/Suspended/Done)看着多,但语义单一——只决定"展示哪一段",
/// 不改变任何落盘/拖拽行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusFilter {
    #[default]
    All,
    Status(TodoState),
}

/// 分类树左侧面板一行的拍平展示(镜像 `project.rs::TreeRow` 的"扁平
/// 存储 + 展开集 → 拍平成行"模式,只是节点数据源从文件系统换成
/// `CategoryInfo`)。
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryTreeRow {
    pub id: i64,
    pub name: String,
    pub depth: usize,
    pub has_children: bool,
    pub expanded: bool,
}

/// Todo 面板右区固定展示列表视图(MARKDOWN tab 已随存储迁移移除),不再
/// 有视图切换,故不保留视图模式状态。
///
/// 鼠标拖拽排序进行态:只记被拖起的待办任务和当前光标悬停到的目标待办,
/// 都用 `items` 里的下标(item-index)表示,**不**用"待办块"相对 rank。
/// 好处是过滤/搜索视图下也成立——展示置换在 `todo_list_view` 里直接对
/// 可见待办子序列(按 item-index)做。`target_idx == usize::MAX` 表示
/// "拖到待办块末尾(已完成之前)"——光标悬停到已完成卡片时取这个值。
/// `source_idx == target_idx` 即还没真的移动过(纯点击),不算重排。已完成
/// 任务永远不参与拖拽:`source_idx` 只能来自待办,悬停已完成只改变
/// `target_idx`(夹到末尾)。排序落盘经 `client.reorder_todo`(Task 7 接入,
/// 见 `Message::DragEnd`)。
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
/// 纯前端过滤：状态相等匹配 + 关键字对 `TodoInfo.text` 做大小写不敏感
/// 的子串匹配（空 `query` 不过滤）。作用在"已经解析+推导好状态"的
/// 内存列表上，不碰数据库（design 第 7 节）。
/// 纯前端关键字过滤：对 `TodoInfo.text` 做大小写不敏感子串匹配（空
/// `query` 不过滤）。作用在"已经解析+推导好状态"的内存列表上，不碰
/// 数据库（design 第 7 节）。左栏"按状态分类"筛选维度移除后，可视
/// 范围只由「关键字 × 自定义分类」两口径取交集决定。
pub fn filter_todos(items: &[TodoInfo], query: &str) -> Vec<usize> {
    let query_lower = query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            query_lower.is_empty() || item.text.to_lowercase().contains(&query_lower)
        })
        .map(|(i, _)| i)
        .collect()
}

/// 按 `parent_id` 把 `categories` 拼成树,按 `expanded` 展开态深度优先
/// 拍平成可见行(未展开节点的子孙不出现在结果里,但节点自身若有子节点
/// 仍会标 `has_children = true`,供左侧渲染箭头)。同级顺序按
/// `CategoryInfo.rank` 升序。
pub fn visible_category_rows(
    categories: &[CategoryInfo],
    expanded: &std::collections::HashSet<i64>,
) -> Vec<CategoryTreeRow> {
    let mut children_of: std::collections::HashMap<Option<i64>, Vec<&CategoryInfo>> =
        std::collections::HashMap::new();
    for c in categories {
        children_of.entry(c.parent_id).or_default().push(c);
    }
    for siblings in children_of.values_mut() {
        siblings.sort_by_key(|c| c.rank);
    }
    let mut rows = Vec::new();
    fn walk(
        parent: Option<i64>,
        depth: usize,
        children_of: &std::collections::HashMap<Option<i64>, Vec<&CategoryInfo>>,
        expanded: &std::collections::HashSet<i64>,
        rows: &mut Vec<CategoryTreeRow>,
    ) {
        let Some(siblings) = children_of.get(&parent) else {
            return;
        };
        for c in siblings {
            let has_children = children_of
                .get(&Some(c.id))
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            let is_expanded = expanded.contains(&c.id);
            rows.push(CategoryTreeRow {
                id: c.id,
                name: c.name.clone(),
                depth,
                has_children,
                expanded: is_expanded,
            });
            if is_expanded {
                walk(Some(c.id), depth + 1, children_of, expanded, rows);
            }
        }
    }
    walk(None, 0, &children_of, expanded, &mut rows);
    rows
}

/// `root` 的全部子孙节点 id(不含 `root` 自己)。给"选中父节点汇总子孙
/// 任务"和"reparent 目标合法性校验"(GUI 侧提前拦截,dozerd 侧
/// `CategoryStore::reparent` 仍会再校验一次,双保险)复用。
pub fn category_descendants(
    categories: &[CategoryInfo],
    root: i64,
) -> std::collections::HashSet<i64> {
    let mut children_of: std::collections::HashMap<i64, Vec<i64>> =
        std::collections::HashMap::new();
    for c in categories {
        if let Some(p) = c.parent_id {
            children_of.entry(p).or_default().push(c.id);
        }
    }
    let mut out = std::collections::HashSet::new();
    let mut frontier = vec![root];
    while let Some(node) = frontier.pop() {
        if let Some(children) = children_of.get(&node) {
            for &child in children {
                if out.insert(child) {
                    frontier.push(child);
                }
            }
        }
    }
    out
}

/// 按当前分类过滤选中态,返回符合条件的任务 id 集合(不是下标——调用方
/// 若需要按下标跟既有 `filter_todos` 的结果取交集,自己按 `todos[i].id`
/// 是否在这个集合里判断)。`Node(id)` 汇总该节点及其全部子孙下的任务。
pub fn filter_todos_by_category(
    todos: &[TodoInfo],
    categories: &[CategoryInfo],
    filter: CategoryFilter,
) -> std::collections::HashSet<i64> {
    match filter {
        CategoryFilter::All => todos.iter().map(|t| t.id).collect(),
        CategoryFilter::Uncategorized => todos
            .iter()
            .filter(|t| t.category_id.is_none())
            .map(|t| t.id)
            .collect(),
        CategoryFilter::Node(root) => {
            let mut allowed = category_descendants(categories, root);
            allowed.insert(root);
            todos
                .iter()
                .filter(|t| t.category_id.is_some_and(|cid| allowed.contains(&cid)))
                .map(|t| t.id)
                .collect()
        }
    }
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

/// Todo 面板挂在每个 `Workspace` 上的状态。
#[derive(Default)]
pub struct WorkspaceState {
    items: Vec<TodoInfo>,
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
    /// 状态下拉展开态(卡片下标):点卡片左下角状态按钮弹出四态(待办/进行中/
    /// 搁置/已完成)选择层。与 `dispatch_open`/`calendar_open` 同一种"同
    /// 时只能有一个"模型——展开状态下拉后,其它同样的弹层都认为关闭。
    status_open: Option<usize>,
    /// 状态下拉浮层弹出锚点(逻辑像素,取点击状态按钮时的光标位置)。与
    /// `dispatch_anchor` 同款窗口级 overlay 定位手法;关闭时清空。
    status_anchor: Option<(f32, f32)>,
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
    /// 鼠标拖拽排序进行态(`None` = 没在拖)。见 `TodoDrag`。视图层据此对
    /// 待办子序列做展示置换并改光标为抓取态；落盘只在 `DragEnd` 时一次性
    /// 发生。已完成任务不可拖动(见 `RowSelect`/`DragMove` 的不变量)。
    drag: Option<TodoDrag>,
    /// 当前项目全部分类节点,随 `ListTodos` 同一轮轮询一并拉取
    /// (`request_categories_refresh`)。
    categories: Vec<CategoryInfo>,
    /// 展开的分类节点 id,纯 UI 态,不落盘(对齐 `FileTree::expanded`
    /// 同样"只在内存里"的处理)。
    category_expanded: std::collections::HashSet<i64>,
    /// 当前选中的分类过滤节点,默认"全部"。
    category_selected: CategoryFilter,
    /// 搜索框左前「按状态」下拉筛选中项,默认"全部"(不做状态维度过滤)。
    /// 与 `category_selected` 并列两维:右区列表 =关键词∩分类∩状态,三段
    /// 展示随选中态收敛(详见 `todo_list_view`)。
    status_filter: StatusFilter,
    /// 搜索框左前的状态筛选浮层是否展开(`true` 时点状态 segment 弹出
    /// "全部 + 四种状态"选择层)。用了同卡内状态按钮一样的"锚点 + 窗口级
    /// overlay"展开模式,见 `status_filter_anchor`。
    status_filter_open: bool,
    /// 状态筛选浮层弹出锚点(逻辑像素,取点击"全部/某状态"segment 时的光标
    /// 位置)。关闭后清空。
    status_filter_anchor: Option<(f32, f32)>,
    /// 分类树行内改名态(分类 id, 草稿字符串),镜像
    /// `files::TreeEdit`——单行文本,不用 `text_editor::Content`。
    category_renaming: Option<(i64, String)>,
    /// 改名框是否持有 iced 真实焦点,镜像 `files.rs::tree_edit_focused`。
    category_rename_focused: bool,
    /// 一次性聚焦标记,镜像 `files.rs::tree_edit_focus_pending`。
    category_rename_focus_pending: bool,
    /// 右区视图模式:列表视图 `List`(默认)/ 看板视图 `Kanban`。
    view: TodoView,
    /// 详情弹窗展开态(卡片下标),`None` = 未展开。跟 `dispatch_open`/
    /// `calendar_open` 同一种"同时只能有一个"模型。
    detail_open: Option<usize>,
    /// 详情弹窗拉到的回合列表(`GetTodoDetail` 应答),弹窗关闭时清空。
    detail_turns: Vec<dozer_core::protocol::TurnRecord>,
    /// 回复框草稿(`text_input` 的 value)。
    detail_reply_draft: String,
    /// 回复框是否持有 iced 内部真实焦点,每帧由 `CaptureDetailReplyFocus`
    /// 写入,镜像 `category_rename_focused`。
    detail_reply_focused: bool,
    /// "处理"按钮是否正在等待 `ProcessTodoNow` RPC 返回——耗时可能到 10
    /// 分钟,期间按钮显示 loading 态、禁用重复提交。
    detail_processing: bool,
    /// "清空列表"确认弹窗是否展开。点 footbar「清空列表」按钮先弹这个
    /// 确认框(危险操作,不可撤销),取消/遮罩收起,确认才真正触发
    /// `Message::ClearListConfirm`。
    clear_confirm: bool,
}

impl WorkspaceState {
    /// 派发选择层是否打开(内核 `App::todo_dispatch_open` 键盘/UI 状态查询用)。
    pub fn dispatch_popup_open(&self) -> bool {
        self.dispatch_open.is_some()
    }

    /// 状态下拉选择层是否打开(内核 `App::todo_status_open` 键盘/UI 状态
    /// 查询用)。
    pub fn status_popup_open(&self) -> bool {
        self.status_open.is_some()
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

    /// 只读当前已加载的任务列表,给视图层渲染与内核处理按下标取任务用。
    pub fn items(&self) -> &[TodoInfo] {
        &self.items
    }

    /// 反查:这个 `session_id` 是不是某条 Todo 任务铸造出来的会话,是的话
    /// 返回该任务原文——给 Agent 卡片"当前工作内容"当主选数据源用。注意
    /// `dispatch_session_id` 现在是"首次真正处理时铸造"的值(2026-09-02
    /// 起,指派与执行解耦),任务被指派但还没真正处理过时该字段是 `None`,
    /// 这个反查天然不会命中——不代表这个反查逻辑本身需要改。
    pub fn task_title_for_session<'a>(&'a self, session_id: &str) -> Option<&'a str> {
        self.items
            .iter()
            .find(|item| item.dispatch_session_id.as_deref() == Some(session_id))
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

    /// 关闭状态下拉选择层(选中某一态并落地后,或 Esc / 点外部)。
    pub fn close_status_popup(&mut self) {
        self.status_open = None;
        self.status_anchor = None;
    }

    /// 记录状态下拉选择层浮层弹出锚点(点击状态按钮时的光标逻辑坐标),供
    /// 窗口级 overlay 定位用。`app.rs::todo_message` 在 `StatusOpen` 时写入。
    pub fn set_status_anchor(&mut self, anchor: (f32, f32)) {
        self.status_anchor = Some(anchor);
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

    /// 失焦退出任务内容编辑态。返回 `Some((id, new_text))` 表示有改动需要
    /// 提交,调用方(`App::set_todo_content_focused`)据此发起
    /// `client.edit_todo_text`;返回 `None` 表示无改动或草稿为空,纯丢弃。
    pub fn commit_content_edit(&mut self) -> Option<(i64, String)> {
        let (idx, draft) = self.editing_content.take()?;
        let new_text = draft.text().trim().to_string();
        if new_text.is_empty() {
            return None;
        }
        let item = self.items.get(idx)?;
        if item.text == new_text {
            return None;
        }
        Some((item.id, new_text))
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

    /// 生效词和草稿都清空(切显示分类时调用,见 `Message::CategorySelect`
    /// 处理器)——只清 `search` 会留下草稿里的旧关键词,用户以为搜索框已经
    /// 清空,其实再次回车/失焦提交时会把旧词重新落成生效过滤,不是真正的
    /// 重置。切回原来那个用关键词搜过的分类也一样清空,不做"记住每个分类
    /// 各自的搜索词"那套(需求原话:哪怕切回去也要重置)。
    pub fn clear_search(&mut self) {
        self.search.clear();
        self.search_draft.clear();
    }

    /// 当前项目全部分类节点(左侧树渲染 + 过滤计算用)。
    pub fn categories(&self) -> &[CategoryInfo] {
        &self.categories
    }

    /// 展开的分类节点 id 集合(左侧树渲染用)。
    pub fn category_expanded(&self) -> &std::collections::HashSet<i64> {
        &self.category_expanded
    }

    /// 当前选中的分类过滤节点。
    pub fn category_selected(&self) -> CategoryFilter {
        self.category_selected
    }

    /// 当前搜索框左前按「状态」筛选中项(read-only,视图 label/过滤计算用)。
    pub fn status_filter(&self) -> StatusFilter {
        self.status_filter
    }

    /// 按状态筛选浮层当前是否展开(main.rs Esc 键盘路由 + 视图 overlay 用)。
    pub fn status_filter_popup_open(&self) -> bool {
        self.status_filter_open
    }

    /// 打开状态筛选浮层并记录弹出锚点(锚点由 `app.rs` 在收到
    /// `Message::StatusFilterOpen` 时先写,这里只翻 bool;同一套"切任何一
    /// 个都先关其它同款浮层"的模型,见 `close_*_popup`)。
    pub fn open_status_filter(&mut self) {
        self.status_filter_open = true;
    }

    /// 关闭状态筛选浮层(选中某一项、Esc 或点外部)。
    pub fn close_status_filter_popup(&mut self) {
        self.status_filter_open = false;
        self.status_filter_anchor = None;
    }

    /// 记录状态筛选浮层弹出锚点(点"全部/待办/..."segment 时的光标位置),
    /// 窗口级 overlay 靠它定位;由 `app.rs::todo_message` 在
    /// `StatusFilterOpen` 时写入(同一套 `set_status_anchor` 手法)。
    pub fn set_status_filter_anchor(&mut self, anchor: (f32, f32)) {
        self.status_filter_anchor = Some(anchor);
    }

    /// 右区当前视图模式(列表/看板),内容渲染与 tab 高亮共用。
    pub fn view(&self) -> TodoView {
        self.view
    }

    /// 展开/收起某个分类节点(点左侧树箭头)。
    fn toggle_category_expanded(&mut self, id: i64) {
        if !self.category_expanded.remove(&id) {
            self.category_expanded.insert(id);
        }
    }

    /// 当前行内改名的分类(id, 草稿)。
    pub fn category_renaming(&self) -> Option<(i64, &str)> {
        self.category_renaming
            .as_ref()
            .map(|(id, draft)| (*id, draft.as_str()))
    }

    pub fn category_rename_focused(&self) -> bool {
        self.category_rename_focused
    }

    pub fn set_category_rename_focused_flag(&mut self, focused: bool) {
        self.category_rename_focused = focused;
    }

    pub fn take_category_rename_focus_pending(&mut self) -> bool {
        std::mem::take(&mut self.category_rename_focus_pending)
    }

    /// 草稿变化时调用(`on_input`)。
    fn set_category_rename_draft(&mut self, text: String) {
        if let Some((_, draft)) = self.category_renaming.as_mut() {
            *draft = text;
        }
    }

    /// 提交(回车/失焦边缘触发共用)。返回 `Some((id, new_name))` 表示有
    /// 改动需要落盘(空白/未变都视为无改动,直接丢弃草稿)。
    fn commit_category_rename(&mut self) -> Option<(i64, String)> {
        let (id, draft) = self.category_renaming.take()?;
        let new_name = draft.trim().to_string();
        if new_name.is_empty() {
            return None;
        }
        let current = self.categories.iter().find(|c| c.id == id)?;
        if current.name == new_name {
            return None;
        }
        Some((id, new_name))
    }

    /// `app.rs::set_category_rename_focused` 失焦边缘触发用的公开入口,
    /// 转发到 `commit_category_rename`。
    pub(crate) fn commit_category_rename_for_blur(&mut self) -> Option<(i64, String)> {
        self.commit_category_rename()
    }

    /// 详情弹窗是否打开(内核键盘 Esc 关闭用,同 `dispatch_popup_open`)。
    pub fn detail_popup_open(&self) -> bool {
        self.detail_open.is_some()
    }

    /// "清空列表"确认弹窗是否打开(窗口级 overlay 挂载判据,同
    /// `detail_popup_open`)。
    pub fn clear_confirm_open(&self) -> bool {
        self.clear_confirm
    }

    pub fn detail_turns(&self) -> &[dozer_core::protocol::TurnRecord] {
        &self.detail_turns
    }

    pub fn detail_reply_draft(&self) -> &str {
        &self.detail_reply_draft
    }

    pub fn detail_processing(&self) -> bool {
        self.detail_processing
    }

    pub fn detail_reply_focused(&self) -> bool {
        self.detail_reply_focused
    }

    pub fn detail_open_idx(&self) -> Option<usize> {
        self.detail_open
    }

    /// 详情弹窗最后一次乐观插入的人类回合内容(`push_optimistic_human_turn`
    /// 刚插入的那条),供 `App::todo_detail_process` 转发进 `ProcessTodoNow`
    /// RPC 的 `human_reply` 参数。`detail_turns` 里最后一条一定是刚插入的
    /// 人类回合(`DetailReplySubmit` 处理顺序:先插入本地乐观回合,`app.rs`
    /// 再读这个值发 RPC)。
    pub fn last_reply_text(&self) -> Option<String> {
        self.detail_turns
            .last()
            .filter(|t| t.role == "human")
            .map(|t| t.content.clone())
    }

    pub fn set_detail_reply_focused_flag(&mut self, focused: bool) {
        self.detail_reply_focused = focused;
    }

    /// `App::todo_detail_open` 拉到 `GetTodoDetail` 应答后写回本地状态。
    pub fn open_detail(&mut self, idx: usize, turns: Vec<dozer_core::protocol::TurnRecord>) {
        self.detail_open = Some(idx);
        self.detail_turns = turns;
        self.detail_reply_draft.clear();
    }

    pub fn close_detail(&mut self) {
        self.detail_open = None;
        self.detail_turns.clear();
        self.detail_reply_draft.clear();
        self.detail_processing = false;
    }

    /// 乐观本地插入一条人类回合(提交回复时,不等 RPC 回来就先看到)。
    pub fn push_optimistic_human_turn(&mut self, content: String) {
        let next_index = self
            .detail_turns
            .last()
            .map(|t| t.turn_index + 1)
            .unwrap_or(0);
        self.detail_turns.push(dozer_core::protocol::TurnRecord {
            turn_index: next_index,
            role: "human".into(),
            content,
            tool_calls: vec![],
            thinking: false,
            thinking_text: None,
            ts: None,
            is_error: false,
            tokens_in: 0,
            tokens_out: 0,
            tokens_cache_read: 0,
            tokens_cache_write: 0,
        });
        self.detail_processing = true;
    }

    /// `ProcessTodoNow` RPC 权威结果回来后,用服务端最新回合列表整体替换
    /// (替换掉乐观插入的那条,避免和服务端最终写入的 `turn_index`/
    /// `message_key` 不一致)。
    pub fn replace_detail_turns(&mut self, turns: Vec<dozer_core::protocol::TurnRecord>) {
        self.detail_turns = turns;
        self.detail_processing = false;
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

/// Todo 面板挂在 `App` 上的本地元数据 sidecar 已随 SQLite 迁移整体删除;
/// 派发记录/计划时间/完成时间一律存 `dozerd::todo::TodoStore`(经 `Client`),
/// 权威字段在 `TodoInfo` 上。

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
    /// 选定 agent 类型完成指派(纯记录,不触发执行)。内核拦截,转发到
    /// `App::todo_assign_agent`——真正的 RPC 调用在 `app.rs`,`todo::update`
    /// 只负责关掉选择层。
    AssignAgent(usize, dozer_core::protocol::AgentKind),
    /// 点卡片"详情"按钮,打开任务详情弹窗。内核拦截,转发到
    /// `App::todo_detail_open`(发 `GetTodoDetail` RPC 拉取回合列表)。
    DetailOpen(usize),
    /// 关闭详情弹窗(Esc / 点外部 / 点关闭按钮)。
    DetailClose,
    /// 详情弹窗回复框草稿变化(`text_input::on_input`,给全量当前字符串)。
    DetailReplyInput(String),
    /// 点"处理"按钮:内核拦截,转发到 `App::todo_detail_process`(乐观插入
    /// 一条本地回合 + 发 `ProcessTodoNow` RPC)。`todo::update` 只清空
    /// 草稿、置处理中标记。
    DetailReplySubmit,
    /// 点卡片左下角状态按钮:弹出从"待办/进行中/搁置/已完成"四态选一的
    /// 状态下拉选择层(`status_open` 记下标,`status_anchor` 记弹出锚点)。
    StatusOpen(usize),
    /// 关闭状态下拉选择层(选中并落地后 / Esc / 点弹层外)。只清浮层,不动
    /// 任务。
    StatusClose,
    /// 状态下拉里选了某一态:`state` 是用户想切到的目标状态(含派生的
    /// `InProgress`,见 `update` 里对三存储态 + InProgress 的分别处理)。
    StatusPick(usize, TodoState),
    /// 点搜索框左前「按状态」segment → 弹出状态筛选浮层(全部/待办/进行中/
    /// 搁置/已完成)。内核 `app.rs` 拦截写在 `status_filter_anchor`;本消息
    /// 只在 `todo::update` 里把 `status_filter_open` 翻真。
    StatusFilterOpen,
    /// 关闭状态筛选浮层(Esc / 点外部 / 选中一项后)。
    StatusFilterClose,
    /// 状态筛选浮层选中某一项:只改 `status_filter`(纯本地过滤轴,不落盘、不
    /// 清搜索关键词),随后关闭浮层。
    StatusFilterPick(StatusFilter),
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
    /// 悬停某张任务卡(由 `todo_card` 外层的 `MouseArea::on_enter/on_exit`
    /// 构造),转交内核的悬停动画表(`app.rs::set_hover`),与全应用其它卡片
    /// 用同一套 hover 机制。
    Hover(HoverId, bool),
    /// 点 footbar 的"清空列表"按钮:弹出确认框(危险操作,不可撤销),
    /// 不直接清空,见 `clear_confirm_popup`。
    ClearListRequest,
    /// 确认弹窗"取消"/点遮罩,收起弹窗,不清空。
    ClearListCancel,
    /// 确认弹窗"清空"。**清空本身尚未实现**:`update` 里只收起弹窗,是
    /// no-op,仅占位——弹窗流程已就位,后续接入清空逻辑时在此落地。
    ClearListConfirm,
    /// 拉取列表的异步结果(轮询、或任一写操作成功后的刷新都落这里)。
    Loaded(Vec<TodoInfo>),
    /// 写操作(增/改/勾选/排序/计划日期/派发)的异步确认;不管成功失败都
    /// 触发一次 `Loaded` 刷新——成功时拿到权威的最新状态,失败时也借这次
    /// 刷新纠正掉之前的乐观更新。
    Mutated(Result<(), String>),
    /// 拉取分类树的异步结果(轮询、或任一分类写操作成功后的刷新都落
    /// 这里),与 `Loaded` 同构。
    CategoriesLoaded(Vec<CategoryInfo>),
    /// 分类写操作(增/改/删/reparent/上移下移/挂任务分类)的异步确认;
    /// 不管成功失败都触发一次 `CategoriesLoaded` + `Loaded` 双刷新——
    /// `SetTodoCategory` 改的是任务的 `category_id`,单刷分类树看不到
    /// 任务列表那边的变化,所以两份列表一起刷,与 `Mutated` 只刷
    /// `Loaded` 的原因不同(那边改的字段只影响任务本身)。
    CategoryMutated(Result<(), String>),
    /// 点左侧树箭头,展开/收起该节点。
    CategoryToggleExpand(i64),
    /// 点左侧树某一行(或"全部"/"未分类"伪节点),切换当前过滤。
    CategorySelect(CategoryFilter),
    /// 切换右区视图模式(列表视图/看板视图)。看板暂占位(CategoryFilter 里
    /// 两个伪节点的空壳),仅记录选中态,列表内容仍正常渲染。
    SelectView(TodoView),
    /// 右键某个分类节点,打开其右键菜单。`Some(id)` 是真实分类节点;
    /// `None` 表示右键的是钉住的"全部"/"未分类"伪节点——伪节点菜单只
    /// 含"新建分类"(新建顶层分类)。内核拦截转发成
    /// `App::todo_category_context_menu`(同 `ProjectLinkMenu` 的既有
    /// 接线方式),不进 `todo::update`。
    CategoryContextMenuOpen(Option<i64>),
    /// 右键菜单"新建子分类":先用默认名新建(RPC 返回真实 id),再立刻
    /// 进入该节点的改名态,让用户直接输入真实名字。
    CategoryNewChild(i64),
    /// 右键菜单"新建同级分类":`parent_id` 是被右键节点的父节点(`None`
    /// 表示新建一个顶层分类,对应"未分类"/空白处右键 → 全局"新建分类"
    /// 入口)。
    CategoryNewSibling(Option<i64>),
    /// 右键菜单"删除"。
    CategoryDelete(i64),
    /// 右键菜单"重命名"/新建后自动触发:进入行内改名态。
    CategoryRenameStart(i64),
    /// 改名框草稿变化(`text_input::on_input`,给全量当前字符串)。
    CategoryRenameEdit(String),
    /// 改名框提交(回车 `on_submit`;失焦边缘触发走
    /// `App::set_category_rename_focused`)。
    CategoryRenameSubmit,
    /// 与前一个/后一个同级节点交换顺序。
    CategoryMoveSibling(i64, dozer_core::protocol::CategoryMoveDirection),
    /// 打开"移动到..."选择器(内核拦截,转发到
    /// `App::todo_category_picker_open_for_category`)。
    CategoryReparentPickerOpen(i64),
    /// 点任务卡片的分类 chip,打开分类选择器(内核拦截,转发到
    /// `App::todo_category_picker_open`)。
    CategoryPickerOpenForTodo(i64),
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

pub fn category_rename_field_id() -> Id {
    Id::new("todo-category-rename-field")
}

static CATEGORY_RENAME_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

pub fn take_category_rename_focused() -> bool {
    std::mem::replace(&mut *CATEGORY_RENAME_FOCUSED.lock().unwrap(), false)
}

pub struct CaptureCategoryRenameFocus;
impl Operation<()> for CaptureCategoryRenameFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&category_rename_field_id()) {
            *CATEGORY_RENAME_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

pub fn detail_reply_field_id() -> Id {
    Id::new("todo-detail-reply-field")
}

static DETAIL_REPLY_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

pub fn take_detail_reply_focused() -> bool {
    std::mem::replace(&mut *DETAIL_REPLY_FOCUSED.lock().unwrap(), false)
}

pub struct CaptureDetailReplyFocus;
impl Operation<()> for CaptureDetailReplyFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&detail_reply_field_id()) {
            *DETAIL_REPLY_FOCUSED.lock().unwrap() = state.is_focused();
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

/// 乐观新增(`AddSubmit`)时,服务端真实 id 还没回来前的占位值。
/// 真实 id 从 1 起(`AUTOINCREMENT`),用 0 保证不会跟真实任务撞车。
/// `Mutated` 触发的刷新会用服务端权威列表整体替换掉带这个 id 的乐观行。
const OPTIMISTIC_TODO_ID: i64 = 0;

/// 现有 `Workspace::spawn_bookmarks_refresh`(见 `extensions::browser::
/// request_bookmarks_refresh`)同款手法的搬家版本:异步拉取某项目的任务
/// 列表。轮询(`App::poll_todo_if_visible`)和任一写操作成功后都调它。
pub fn request_todos_refresh(
    project_id: i64,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let todos = client.list_todos(project_id).await.unwrap_or_default();
        emit(Message::Loaded(todos));
    });
}

/// `request_todos_refresh` 的分类树版本:异步拉取某项目的全部分类节点。
pub fn request_categories_refresh(
    project_id: i64,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let categories = client.list_categories(project_id).await.unwrap_or_default();
        emit(Message::CategoriesLoaded(categories));
    });
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

/// 处理除 `DispatchToExisting` 之外的消息。`DispatchToExisting` 涉及终端
/// 会话读写,内核会先拦截,不会转发到这里。写操作(增/改/勾选/排序/计划
/// 日期/派发)一律走异步 `Client`,结果经 `Message::Loaded`/`Mutated`
/// 落回 `ws_state.items`。
pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + Sync + 'static,
) {
    match msg {
        // 卡片悬停由内核 `App::update` 拦截转发到 `set_hover`,不会到这。
        Message::Hover(_, _) => {}
        // footbar"清空列表"按钮:弹出确认框,不直接清空。
        Message::ClearListRequest => ws_state.clear_confirm = true,
        Message::ClearListCancel => ws_state.clear_confirm = false,
        // 确认弹窗"清空":清空本身尚未实现,先收起弹窗,仅占位。
        Message::ClearListConfirm => ws_state.clear_confirm = false,
        // `TextInputMenuOpen` 由内核拦截映射为右键菜单,不进这里。
        Message::TextInputMenuOpen(_) => {}
        // `ToggleListCollapse` 由内核拦截映射为列表列收起/展开,不进这里。
        Message::ToggleListCollapse => {}
        Message::Loaded(todos) => ws_state.items = todos,
        Message::Mutated(res) => {
            if let Err(e) = res {
                tracing::warn!("Todo 写操作失败: {e}");
            }
            request_todos_refresh(project_id, client, handle, emit);
        }
        Message::CategoriesLoaded(categories) => ws_state.categories = categories,
        Message::CategoryMutated(res) => {
            if let Err(e) = res {
                tracing::warn!("分类写操作失败: {e}");
            }
            let client1 = client.clone();
            let client2 = client.clone();
            let handle1 = handle.clone();
            let handle2 = handle.clone();
            let emit = std::sync::Arc::new(emit);
            let emit1 = emit.clone();
            let emit2 = emit;
            request_categories_refresh(project_id, &client1, &handle1, move |m| emit1(m));
            handle2.spawn(async move {
                let todos = client2.list_todos(project_id).await.unwrap_or_default();
                emit2(Message::Loaded(todos));
            });
        }
        Message::CategoryToggleExpand(id) => ws_state.toggle_category_expanded(id),
        Message::SelectView(view) => ws_state.view = view,
        Message::CategorySelect(filter) => {
            ws_state.category_selected = filter;
            // 切显示分类重置搜索关键词过滤(原在切换状态分类时做,现左栏只留
            // 自定义分类这一口径,故改到切换分类这里):哪怕切回原来那个用
            // 关键词搜过的分类也要重置,不做"记住每个分类各自搜索词"那套。
            ws_state.clear_search();
        }
        Message::CategoryContextMenuOpen(_) => {}
        Message::CategoryNewChild(parent_id) => {
            let client = client.clone();
            let project_id_owned = project_id;
            handle.spawn(async move {
                let res = client
                    .add_category(project_id_owned, Some(parent_id), "新分类")
                    .await;
                match res {
                    Ok(category) => {
                        emit(Message::CategoryRenameStart(category.id));
                        emit(Message::CategoryMutated(Ok(())));
                    }
                    Err(e) => emit(Message::CategoryMutated(Err(e.to_string()))),
                }
            });
        }
        Message::CategoryNewSibling(parent_id) => {
            let client = client.clone();
            let project_id_owned = project_id;
            handle.spawn(async move {
                let res = client
                    .add_category(project_id_owned, parent_id, "新分类")
                    .await;
                match res {
                    Ok(category) => {
                        emit(Message::CategoryRenameStart(category.id));
                        emit(Message::CategoryMutated(Ok(())));
                    }
                    Err(e) => emit(Message::CategoryMutated(Err(e.to_string()))),
                }
            });
        }
        Message::CategoryDelete(id) => {
            let client = client.clone();
            handle.spawn(async move {
                let res = client.delete_category(id).await.map_err(|e| e.to_string());
                emit(Message::CategoryMutated(res));
            });
        }
        Message::CategoryRenameStart(id) => {
            let Some(current) = ws_state.categories.iter().find(|c| c.id == id) else {
                return;
            };
            ws_state.category_renaming = Some((id, current.name.clone()));
            ws_state.category_rename_focus_pending = true;
        }
        Message::CategoryRenameEdit(text) => ws_state.set_category_rename_draft(text),
        Message::CategoryRenameSubmit => {
            if let Some((id, new_name)) = ws_state.commit_category_rename() {
                let client = client.clone();
                handle.spawn(async move {
                    let res = client
                        .rename_category(id, &new_name)
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    emit(Message::CategoryMutated(res));
                });
            }
        }
        Message::CategoryMoveSibling(id, direction) => {
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .move_category_sibling(id, direction)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::CategoryMutated(res));
            });
        }
        Message::CategoryReparentPickerOpen(_) => {}
        Message::CategoryPickerOpenForTodo(_) => {}
        Message::Toggle(idx) => {
            let Some(item) = ws_state.items.get(idx) else {
                return;
            };
            let id = item.id;
            let target = !item.done;
            // 乐观更新:本地立即翻转,给出与迁移前同等的零延迟反馈。
            if let Some(item) = ws_state.items.get_mut(idx) {
                item.done = target;
                // 同步完成/取消完成的时间戳(`completed_at_for_toggle`:完成
                // → `Some(now)`,取消 → `None`),让 Done 徽章的 "MM-DD" 在
                // 服务端往返回来之前就能显示(与服务端 TodoStore 口径一致)。
                item.completed_at_ms =
                    completed_at_for_toggle(target, std::time::SystemTime::now()).map(|t| {
                        t.duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64
                    });
            }
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .toggle_todo(id, target)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::Mutated(res));
            });
        }
        Message::AddEdit(action) => ws_state.add_draft.perform(action),
        Message::AddSubmit => {
            let text = ws_state.add_draft.text();
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            ws_state.add_draft = iced_widget::text_editor::Content::new();
            // 乐观置顶:插入一条临时 id 的条目,立即触发既有的"新增闪光+
            // 滚回顶部"效果,不等服务端往返。
            ws_state.items.insert(
                0,
                TodoInfo {
                    id: OPTIMISTIC_TODO_ID,
                    project_id,
                    text: text.clone(),
                    done: false,
                    paused: false,
                    rank: 0,
                    created_ms: 0,
                    completed_at_ms: None,
                    plan_date: None,
                    dispatch_session_id: None,
                    dispatch_at_ms: None,
                    category_id: None,
                    assigned_agent: None,
                },
            );
            ws_state.start_flash(0);
            let client = client.clone();
            let project_id_owned = project_id;
            handle.spawn(async move {
                let res = client
                    .add_todo(project_id_owned, &text)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::Mutated(res));
            });
        }
        // 高度拖拽在 app 层 `todo_message` 已早退,不会到这里;保留 arm 仅
        // 为 match 穷尽。
        Message::AddResizeStart => {}
        Message::RowSelect(idx) => {
            ws_state.selected_row = idx;
            // 用户手动选中(点卡片空白处)会打断"新增闪光":否则 2 秒计时到点
            // 会把用户刚主动选的卡片又自动取消选中。`take_scroll_to_top` 未定
            // 时用户主动点才会走到这里,新增闪光阶段不处理(见 `Message::Toggle`
            // 之上对闪光来源的约定)。
            ws_state.flash = None;
            // 活动卡片被按下即"准备拖":记下它的 item-index 作为拖拽源。
            // 搁置/完成不参与拖拽(只有活动段才进 `drag`)。注意这跟选中态是
            // 两件独立的事——纯点击(不移动)也会落到这里,但松手时
            // source==target 不写盘,只是正常选中切换(同 `TabDragMove`
            // 的"按住=准备拖,移动才换位"语义)。
            if let Some(i) = idx
                && let Some(item) = ws_state.items.get(i)
                && is_active_todo(item)
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
            // 活动段尾部下标:最后一个活动(非 done && 非 paused)项的
            // item-index。搁置/完成都夹不到活动段的"之后"(活动段是整它自己
            // 那一段),落到非活动行就一律取"最后一个活动"当落点。
            let last_active = ws_state
                .items
                .iter()
                .enumerate()
                .rfind(|(_, it)| is_active_todo(it))
                .map(|(i, _)| i);
            let target = match ws_state.items.get(over_idx) {
                Some(it) if is_active_todo(it) => over_idx,
                // 走到这个分支时表示悬停到搁置/完成(或在它之上)。为了让用户把
                // 活动拖到"搁置段之前",落点取最后一个活动项的 idx(下面换算成
                // after_id);若根本没有可落的活动行则走 usize::MAX(交服务端
                // 按"待办块挪到最前"处理)。注意 source 等于该 last_active(正要
                // 把它挪到自己之后)会让 last_active 与 itself 结算成 no-op。
                _ => last_active.unwrap_or(usize::MAX),
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
                return;
            }
            let Some(source_item) = ws_state.items.get(drag.source_idx) else {
                return;
            };
            let id = source_item.id;
            // 落点 target 只在"另一个活动项"上(DragMove 里已保证夹在活动段
            // 内、且 source != target 已在上方早退),after_id 直接取它的 id,
            // 让服务端把本任务挪到它之后。活动段/搁置段边界由 DragMove 夹好,
            // 这里不需要再区分 usize::MAX。
            let after_id = ws_state.items.get(drag.target_idx).map(|it| it.id);
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .reorder_todo(id, after_id)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::Mutated(res));
            });
        }
        Message::DispatchOpen(idx) => {
            // 与卡片浮层互斥:开派发层顺手收起状态筛选、日历、卡片状态下拉。
            ws_state.close_status_filter_popup();
            ws_state.calendar_open = None;
            ws_state.status_open = None;
            ws_state.dispatch_open = Some(idx);
        }
        Message::DispatchClose => {
            ws_state.dispatch_open = None;
            ws_state.dispatch_anchor = None;
        }
        Message::AssignAgent(_, _) => {
            // 真正的 RPC 调用在 `App::todo_assign_agent`(app.rs),这里
            // 只负责关掉选择层——与 `DispatchClose` 同款收尾。
            ws_state.dispatch_open = None;
            ws_state.dispatch_anchor = None;
        }
        Message::DetailClose => ws_state.close_detail(),
        Message::DetailReplyInput(text) => ws_state.detail_reply_draft = text,
        Message::DetailReplySubmit => {
            // 乐观插入 + 置处理中标记在这里做(纯本地状态);真正发
            // `ProcessTodoNow` RPC 在 `App::todo_detail_process`(app.rs),
            // 那边会读 `detail_reply_draft` 拿文本、读 `items()[idx].id`
            // 拿任务 id。
            let draft = std::mem::take(&mut ws_state.detail_reply_draft);
            if !draft.trim().is_empty() {
                ws_state.push_optimistic_human_turn(draft);
            }
        }
        Message::DetailOpen(_) => {
            // 真正拉 `GetTodoDetail` 在 `App::todo_detail_open`(app.rs),
            // `todo::update` 不处理这条(no-op arm 保持 match 穷尽,同
            // `AddResizeStart` 的既有模式)。
        }
        Message::StatusOpen(idx) => {
            // 同一时刻只允许一个卡片弹层(状态/日历/派发互斥):打开状态下拉时
            // 顺手把另外两个收起,避免叠两层卡片浮层。
            ws_state.close_status_filter_popup();
            ws_state.dispatch_open = None;
            ws_state.calendar_open = None;
            ws_state.status_open = Some(idx);
        }
        Message::StatusClose => ws_state.close_status_popup(),
        Message::StatusPick(idx, target) => {
            // 关闭状态下拉;无论目标是什么都先收起浮层,再做分支处理。
            ws_state.close_status_popup();
            let Some(item) = ws_state.items.get(idx) else {
                return;
            };
            // 需要把状态落到 dozerd 的存储态。InProgress 不是存储位:真正
            // "进进行中"要一次指派(选到某个存活会话);这里要么把这个分支
            // 转给派发选择层(待办/搁置想进进行中),要么撤销搁置先变"待办"。
            let stored_target = match target {
                TodoState::Pending => Some(dozer_core::protocol::TodoStoredStatus::Todo),
                TodoState::Done => Some(dozer_core::protocol::TodoStoredStatus::Done),
                TodoState::Suspended => Some(dozer_core::protocol::TodoStoredStatus::Suspended),
                TodoState::InProgress => None,
            };
            // InProgress:当前没存活会话(从 Pending/Suspended 想转)就交给
            // 派发选择层;已经 InProgress 的选它不做事。搁置(拿回暂停)想
            // 进"进行中"先把 stored 位撤回待办(撤销暂停),下一拍用户再走
            // "指派"一个存活会话；这里不连做两次写只因想分清"取消搁置"跟
            // "指派会话"两件原子动作。
            if target == TodoState::InProgress {
                if item.paused {
                    let client = client.clone();
                    let id = item.id;
                    handle.spawn(async move {
                        let res = client
                            .set_todo_status(id, dozer_core::protocol::TodoStoredStatus::Todo)
                            .await
                            .map(|_| ())
                            .map_err(|e| e.to_string());
                        emit(Message::Mutated(res));
                    });
                } else {
                    // 非搁置想进进行中 = 想指派一个会话 → 把状态浮层让位给
                    // 派发浮层,并沿用它自己的弹出锚点(刚记录的那次点击)。
                    if let Some(a) = ws_state.status_anchor.take() {
                        ws_state.dispatch_anchor = Some(a);
                    }
                    ws_state.dispatch_open = Some(idx);
                }
                ws_state.status_open = None;
                ws_state.status_anchor = None;
                return;
            }
            let Some(stored_target) = stored_target else {
                return;
            };
            let client = client.clone();
            let id = item.id;
            handle.spawn(async move {
                let res = client
                    .set_todo_status(id, stored_target)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::Mutated(res));
            });
        }
        Message::StatusFilterOpen => {
            // 与卡片浮层互斥:开搜索框筛选浮层时先收起派发层/日历/卡片状态。
            ws_state.status_open = None;
            ws_state.dispatch_open = None;
            ws_state.calendar_open = None;
            ws_state.open_status_filter();
        }
        Message::StatusFilterClose => ws_state.close_status_filter_popup(),
        Message::StatusFilterPick(filter) => {
            // 纯本地筛选轴:只改选中项,不落盘、不动搜索关键词。选中某项或
            // "全部"后顺带关闭浮层。
            ws_state.status_filter = filter;
            ws_state.close_status_filter_popup();
        }
        Message::CalendarOpen(idx) => {
            // 与 StatusOpen/派发同样"同时只能有一个浮层":开日历时收起状态
            // 提层/派发层,以及搜索框的状态筛选浮层。
            ws_state.close_status_filter_popup();
            ws_state.status_open = None;
            ws_state.status_anchor = None;
            ws_state.dispatch_open = None;
            // 打开日历:默认停在"当前月",若任务已有计划日期且能解析成 MM-DD,
            // 则把视图拨到该月(年份取当前年——plan_date 只有月日,无年份)。
            let (now_y, now_m, _) = today_ymd();
            let (y, m) = ws_state
                .items
                .get(idx)
                .and_then(|item| item.plan_date.clone())
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
            if let Some(item) = ws_state.items.get(idx) {
                let id = item.id;
                let client = client.clone();
                handle.spawn(async move {
                    let res = client
                        .set_todo_plan_date(id, Some(&day))
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    emit(Message::Mutated(res));
                });
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
            let is_enter = matches!(
                action,
                iced_widget::text_editor::Action::Edit(iced_widget::text_editor::Edit::Enter)
            );
            if let Some((idx, draft)) = ws_state.editing_content.as_mut() {
                draft.perform(action.clone());
                if is_enter {
                    let idx = *idx;
                    let new_text = draft.text().trim().to_string();
                    let old_item = ws_state.items.get(idx).cloned();
                    ws_state.editing_content = None;
                    if let Some(old_item) = old_item
                        && !new_text.is_empty()
                        && old_item.text != new_text
                    {
                        let id = old_item.id;
                        let client = client.clone();
                        handle.spawn(async move {
                            let res = client
                                .edit_todo_text(id, &new_text)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string());
                            emit(Message::Mutated(res));
                        });
                    }
                }
            }
        }
    }
}

/// Todo 面板渲染成两个独立的边框 pane(镜像 Files/Project 面板已有的
/// "侧栏 + 内容区，中间一条可拖拽分隔线"两栏模式，不再是单个面板内部一个
/// `row![sidebar, body]`)——调用方(`app.rs` 的 `PanelKind::Todo` 分支)负责
/// 拼 `row![sidebar_pane, divider_bar(Divider::TodoSplit, ..), content_pane]`。
/// 左栏：面板头 + 分类导航。右栏：列表视图主体 + 底部新增输入。
#[allow(clippy::too_many_arguments)]
pub fn view<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
    ws: &Workspace,
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

    // ---- 状态推导（每张任务卡片的状态标/派发判断共用,一次算好） ----
    let states: Vec<TodoState> = ws_state
        .items
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
        .collect();

    // ---- 左栏 pane：header + 分类导航 + 底部「清空列表」栏 ----
    // 去掉按任务状态分类的列表(全部/待办/进行中/已完成),只保留下方的
    // 自定义分类导航。原 content pane 右下角的 `todo_clear_footer_bar` 整体
    // 迁到左栏:分类导航之下、靠底。计数与「清空列表」不再堆在内容区右下方,
    // 改由左栏底部统一呈现——内容区底部只留「新增任务」输入框。
    // ---- 分类树导航:左栏唯一的多类别入口 ----
    let category_nav = category_tree_nav(app, ws_state);
    let sidebar_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(
            column![
                header,
                category_nav,
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

    // ---- 右栏 pane：视图切换 tab + 收起/展开列表列按钮 + 视图主体 ----
    // 右栏内容区按 `TodoView` 渲染:列表视图(现有逐行列表)与看板视图
    // (一期仅占位,见 `kanban_placeholder`)。顶部一行左侧是这两个视图切换
    // tab、右端是收起/展开列表列按钮。收起/展开按钮消息为本地
    // `Message::ToggleListCollapse`,由内核 `App::update` 拦截转发成顶层
    // `Message::TogglePanelListCollapse`。
    let collapse = app.list_collapse_button(
        crate::app::PanelKind::Todo,
        app.list_collapsed(crate::app::PanelKind::Todo),
        crate::app::HoverId::TodoListCollapse,
        "收起",
        "展开",
        Message::ToggleListCollapse,
        move |hovered| Message::Hover(crate::app::HoverId::TodoListCollapse, hovered),
    );
    // 右区顶栏:左侧放「列表视图/看板视图」两个视图切换 tab,右端放
    // 收起/展开列表列按钮。中间 `Fill` 空间把右侧按钮顶到行尾,也让视图
    // tab 左贴内容区(都与下方任务卡片取平)。`collapse` 右缘对齐 20px 右边
    // 距(2026-08-28 用户反馈:改之前 collapse 紧贴 list 右边,没跟卡片右
    // 对齐)。
    let top_row = row![
        todo_view_tab(
            "列表视图",
            TodoView::List,
            ws_state.view() == TodoView::List
        ),
        todo_view_tab(
            "看板视图",
            TodoView::Kanban,
            ws_state.view() == TodoView::Kanban
        ),
        space::Space::new().width(Length::Fill),
        collapse,
    ]
    .width(Length::Fill)
    .align_y(iced_widget::core::Alignment::Center)
    .spacing(4)
    .padding([4, 20]);
    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match ws_state.view() {
            TodoView::List => todo_list_view(app, ws_state, &states),
            // 看板视图一期仅占位:切换有入口、渲染不崩,内容留空待后续实现。
            TodoView::Kanban => kanban_placeholder(),
        };
    let content_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(column![top_row, crate::app::tab_divider(), body].height(Length::Fill))
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

/// 右区顶部一个视图模式切换 tab(纯文字 pill):选中态 cream 文字 + `card`
/// 实底 + 1px `theme.border` 描边,未选中态静止为 `dim` 文字。
///
/// 视觉对齐文件/浏览器/终端那套共享 `tab_widget::panel_tab` 的激活态
/// (cream 文字 + `card` 底 + 1px 中性描边,非原来自成一派的金描边):因此两
/// 颗「列表视图/看板视图」切换钮与 readme 预览/浏览器打开的文件页签长相一
/// 致 —— 同一行里选中那颗 = `panel_tab` 激活页签,未选中的 = 未激活页签
/// (hover 时浮现 `tab_hover` 胶囊、文字 `dim`→`gold`)。区别只在它是无关闭
/// × 的互斥选择开关(视图切换没有"关掉列表视图"这种语义),故不复用
/// `panel_tab` 那个关不掉关闭按钮的带 close 形状,只 `button::status` 做同一套
/// 静止/hover 两态。点击下发 `Message::SelectView`,切换列表/看板视图。
fn todo_view_tab<'a>(
    label: &'static str,
    view: TodoView,
    active: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let theme = byteui::theme::color::current();
    // 文字颜色不固定在构造期:不显式 `.color()`(默认 `None` = 继承父级),
    // 这样才会读 `button::Style.text_color`,按下/hover 驱动 `dim`→`gold`
    // (同 `panel_tab` 的标题染色)。之前误加 `.color(Color::TRANSPARENT)`
    // ——iced `Text::color()` 一旦调用就是固定值、不是"占位待继承",导致
    // 文字恒透明不可见(2026-09-04 用户反馈肉眼看不到 tab 文字)。
    let content = text(label).size(byteui::theme::font::body());
    let btn = button(content)
        .on_press(Message::SelectView(view))
        .width(Length::Shrink)
        .padding([5, 14])
        .style(move |_t: &iced_widget::Theme, status| {
            if active {
                // 选中 → 同 `panel_tab` 激活态:CARD 实底 + 1px `theme.border`。
                button::Style {
                    background: Some(theme.card.into()),
                    text_color: theme.cream,
                    border: Border {
                        color: theme.border,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..button::Style::default()
                }
            } else {
                // 未选中 → 同 `panel_tab` 未激活态:静止透明 dim;hover/press
                // 浮现 `tab_hover` 胶囊、文字 `dim`→`gold`。
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: if hovered {
                        Some(theme.tab_hover.into())
                    } else {
                        None
                    },
                    text_color: if hovered { theme.gold } else { theme.dim },
                    border: Border {
                        radius: 6.0.into(),
                        ..Border::default()
                    },
                    ..button::Style::default()
                }
            }
        });
    btn.into()
}

/// 看板视图的一期占位:视觉几列状态分区我们不渲染任何任务(功能未实现),
/// 只在内容区中央给一行淡淡的提示文字,说明该视图待实现。等接入了真正
/// 的看板渲染(按状态分列的那批 `todo_card`)再替换这里。
fn kanban_placeholder<'a>() -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(
        text("看板视图待实现")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(iced_widget::core::alignment::Horizontal::Center)
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .into()
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
/// (顶部分隔线 + 左图标/文案 + 右侧操作按钮)。危险操作(清空整个列表
/// 不可撤销),点按钮先弹确认框(`clear_confirm_popup`)而非直接清空;
/// 按统一按钮规范(见 `dialog::action_button_border_color` 文档)走红字 +
/// 描边静止态 `border`、悬浮/按下态变 `gold`(此前固定奶油字 + 不响应
/// hover 的静态描边)。**清空本身尚未实现**:`ClearListConfirm` 在
/// `update` 里只收起弹窗,是 no-op,仅占位——弹窗流程已就位,后续接入
/// 清空逻辑时在此落地。
fn todo_clear_footer_bar<'a>(
    _ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let clear = button(
        row![
            icons::view(
                icons::IconKind::Trash,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().red,
            ),
            text("清空列表")
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().red),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .on_press(Message::ClearListRequest)
    .padding([4, 8])
    .style(|_t: &iced_widget::Theme, s| button::Style {
        background: Some(byteui::theme::color::current().bg.into()),
        border: Border {
            color: crate::dialog::action_button_border_color(s),
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: byteui::theme::color::current().red,
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

/// "清空列表"确认弹窗:窗口级居中浮层,视觉模板同
/// `extensions::project::project_delete_confirm_popup`/
/// `ssh.rs::delete_confirm_popup`(CARD 底 + 圆角描边 + 标题图标 +
/// 取消/确认两个圆角按钮)。标题图标用 Todo 面板自己的
/// `icons::IconKind::ListTodo`(同 `home_panel_head` 头部图标),不用
/// footbar 按钮的 `Trash`——图标标的是"这是 Todo 面板的弹窗",危险语义已
/// 由红色"清空"按钮本身表达,不需要标题图标重复。
pub fn clear_confirm_popup(
    _ws_state: &WorkspaceState,
    window_width: f32,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let title = row![
        icons::view(
            icons::IconKind::ListTodo,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().cream,
        ),
        text("清空列表")
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().cream),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let cancel = button(
        text("取消")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
    )
    .on_press(Message::ClearListCancel)
    .padding([6, 12])
    .style(crate::dialog::action_button_style(
        byteui::theme::color::current().dim,
    ));
    let confirm = button(
        text("清空")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().red),
    )
    .on_press(Message::ClearListConfirm)
    .padding([6, 12])
    .style(crate::dialog::action_button_style(
        byteui::theme::color::current().red,
    ));

    let dialog = container(
        column![
            title,
            text("这会清空当前项目的全部任务,操作不可撤销。")
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().dim),
            crate::dialog::actions(row![cancel, confirm].spacing(8)),
        ]
        .spacing(12),
    )
    // 宽度改用 `dialog::width`(整窗 1/3,2026-09-15 统一约定)——此前没给
    // 显式宽度,靠内容撑开。
    .width(crate::dialog::width(window_width))
    .padding(16)
    .style(crate::dialog::card_style);

    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

/// 顶部搜索框:真正的 `byteui::form::input_text`,形状与 Files 搜索框
/// (Stage 2)一致。草稿 `draft` 是 `text_input::on_input` 给的全量字符串,
/// 焦点态由 `CaptureTodoSearchFocus` 每帧查、`main.rs` 据此放行键盘给
/// 标准 iced 管线。`highlight` = 列表正被 `search` 过滤或搜索聚焦时持续
/// 金框提示(同 Files `search_box`)。左前区内嵌「按状态筛选」segment
/// (`view_with_prefix`),点它开 `Message::StatusFilterOpen` 弹"全部 + 四种
/// 状态"选择浮层(不再按分类——分类改由左侧分类树承担)。
fn todo_search_bar<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let highlight = ws_state.search_focused() || !ws_state.search.is_empty();
    let filter_label = status_filter_label(ws_state.status_filter());
    let filter_state = match ws_state.status_filter() {
        StatusFilter::All => None,
        StatusFilter::Status(st) => Some(st),
    };
    let prefix = status_filter_segment(filter_label, filter_state);
    let bar = byteui::form::search_box::view_with_prefix(
        "搜索任务…",
        &ws_state.search_draft,
        Some(todo_search_field_id()),
        highlight,
        Message::SearchInput,
        Message::SearchSubmit,
        app.hover_progress(HoverId::TodoSearchSubmit),
        |hovered| Message::Hover(HoverId::TodoSearchSubmit, hovered),
        Some(prefix),
    );
    byteui::interaction::context_menu::wrap(
        bar,
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: todo_search_field_id(),
            secure: false,
        })),
    )
}

/// 当前 `StatusFilter` 在搜索 segment 上显示的名字:`All` = "全部"(不做
/// 状态维度过滤),`Status(s)` = 那态的中文(fm `status_meta`,带它自己的色)。
fn status_filter_label(filter: StatusFilter) -> String {
    match filter {
        StatusFilter::All => "全部".to_string(),
        StatusFilter::Status(st) => status_meta(st).0.to_string(),
    }
}

/// 搜索框左前方的"状态"segment:显示当前过滤名 + 下箭头,点开窗口级浮层
/// (列出"全部 + 待办/进行中/搁置/已完成")。色调对齐分类左栏选中行(hover
/// 卡底 + 金描边);本身无边框,与外层 `search_box` 共用同一圈搜索框边框,
/// 点击不开走输入焦点——通过 `byteui::form::search_box::view_with_prefix`
/// 内嵌在输入位左前区。文案按当前过滤单项变色:选中状态时用那态的颜色,
/// "全部"用奶油,一眼能看出当前筛在哪个态。
fn status_filter_segment<'a>(
    label: String,
    active_state: Option<TodoState>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let text_color = active_state
        .map(|st| status_meta(st).1)
        .unwrap_or(colors.cream);
    button(
        row![
            text(label)
                .size(byteui::theme::font::body())
                .color(text_color),
            icons::view(
                icons::IconKind::ChevronDown,
                byteui::theme::icon_size::chevron(),
                colors.dim,
            ),
        ]
        .spacing(4)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .on_press(Message::StatusFilterOpen)
    .style(move |_t: &iced_widget::Theme, s: button::Status| {
        let hovered = matches!(s, button::Status::Hovered);
        button::Style {
            background: if hovered {
                Some(colors.card.into())
            } else {
                Some(colors.bg.into())
            },
            text_color,
            border: Border {
                color: if hovered { colors.gold } else { colors.border },
                width: 1.0,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        }
    })
    .into()
}

/// 列表视图主体：搜索栏 + 编号行列表 + 底部新增输入。
fn todo_list_view<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
    states: &[TodoState],
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let keyword_idx = filter_todos(&ws_state.items, &ws_state.search);
    let category_allowed_ids = filter_todos_by_category(
        &ws_state.items,
        ws_state.categories(),
        ws_state.category_selected(),
    );
    // 状态维度:由搜索框左前「状态」筛选项决定(从 `states` 取每条的状态)。
    // 与分类维度并列,二者连同关键词取交集(`All` 不设约束)。
    let status_allowed = match ws_state.status_filter() {
        StatusFilter::All => None,
        StatusFilter::Status(st) => Some(st),
    };
    let visible_idx: Vec<usize> = keyword_idx
        .into_iter()
        .filter(|&i| category_allowed_ids.contains(&ws_state.items[i].id))
        .filter(|&i| status_allowed.map(|st| states[i] == st).unwrap_or(true))
        .collect();

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
        // 三段布局:活动(待办/进行中,可拖拽)→ 搁置(拿回暂停,不可拖)→
        // 已完成(沉底,不可拖)。把 `visible_idx` 按每项 `is_active_todo`/
        // paused/done 拆开,各自保持原(items)次序。与 dozerd 落盘
        // `ORDER BY done, paused, rank` 的"活动-搁置-完成"顺序同构。
        let mut active_idx: Vec<usize> = Vec::new();
        let mut suspended_idx: Vec<usize> = Vec::new();
        let mut done_idx: Vec<usize> = Vec::new();
        for &i in &visible_idx {
            let it = &ws_state.items[i];
            if it.done {
                done_idx.push(i);
            } else if it.paused {
                suspended_idx.push(i);
            } else {
                active_idx.push(i);
            }
        }
        // 拖拽进行中不再对 active 段做展示置换——之前"每帧按新顺序
        // remove+insert 整个重排"会让被拖卡片之外的其它卡片瞬间跳位,
        // 没有任何过渡帧(用户反馈"动画不够流畅"的根因)。改成更常见的
        // "源卡片原位高亮 + 插入指示线"模式:active 子序列渲染顺序全程不变,
        // 被拖的那张卡片本身描边变金(`is_drag_source`),目标位置前插一条
        // 细的金色指示线提示"松手会落在这里"。真正的换位只在 `DragEnd`
        // 落盘时一次性发生。搁置/完成不可拖也不让拖到它们上头
        // (`DragMove` 只把落点夹在 active 段内)。
        let drag_source_idx = ws_state.drag.map(|d| d.source_idx);
        // 指示线该出现在 active 子序列的哪个展示位置之前:target_idx 对应
        // 的卡片在 active_idx 里的下标。落点是 active 段尾巴(悬停到搁置
        // 标题/完成段或末尾)时,指示线插这段最后一张之后。source ==
        // target(还没真的移动过)时不显示,跟换位逻辑"没移动不写盘"对齐。
        let (insert_before, insert_at_end) = match ws_state.drag {
            Some(drag) if drag.source_idx != drag.target_idx => {
                match active_idx.iter().position(|&x| x == drag.target_idx) {
                    Some(p) => (Some(p), false),
                    None => (None, true),
                }
            }
            _ => (None, false),
        };
        let grabbing = ws_state.drag.is_some();
        let mut shown = 0usize;
        // ---- 段一:活动(待办/进行中,可拖拽) ----
        for (display_no, &idx) in active_idx.iter().enumerate() {
            shown += 1;
            if insert_before == Some(display_no) {
                list = list.push(drag_insert_indicator());
            }
            list = list.push(todo_list_row(
                app,
                ws_state,
                states,
                shown,
                idx,
                grabbing,
                drag_source_idx == Some(idx),
            ));
        }
        if insert_at_end || insert_before == Some(active_idx.len()) {
            list = list.push(drag_insert_indicator());
        }
        // ---- 段二:搁置(已暂停,拿回待重启) ----
        if !suspended_idx.is_empty() {
            list = list.push(todo_segment_divider("搁置"));
            for &idx in &suspended_idx {
                shown += 1;
                list = list.push(todo_list_row(
                    app, ws_state, states, shown, idx, grabbing, false,
                ));
            }
        }
        // ---- 段三:已完成(沉底) ----
        if !done_idx.is_empty() {
            list = list.push(todo_segment_divider("已完成"));
            for &idx in &done_idx {
                shown += 1;
                list = list.push(todo_list_row(
                    app, ws_state, states, shown, idx, grabbing, false,
                ));
            }
        }
        let _ = shown;
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

/// `todo_list_view` 单行的渲染分派:内容编辑态 → `todo_content_edit_row`,
/// 否则 → `todo_card`。从 `todo_list_view` 的循环体里拆出来,好让 pending/
/// done 两段各自的 `for` 循环别重复这段查表+分支逻辑。
#[allow(clippy::too_many_arguments)]
fn todo_list_row<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
    states: &[TodoState],
    number: usize,
    idx: usize,
    grabbing: bool,
    is_drag_source: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let item = &ws_state.items[idx];
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
        selected: ws_state.selected_row == Some(idx),
        grabbing,
        is_drag_source,
        hovered,
        editing_draft,
        categories: ws_state.categories(),
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

/// 段标题(搁置 / 已完成):一段窄的暗色分隔条 + 缩进 caption 文字,用来
/// 把三段列表(活动 / 搁置 / 完成)在视觉上明确切开——前两段纯靠卡片排布
/// 分不出来,加个低频次、高信息量的分隔标题最省事。本身不响应任何输入。
fn todo_segment_divider(
    label: impl Into<String>,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    row![
        text(label.into())
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim),
        container(iced_widget::space::Space::new())
            .width(Length::Fill)
            .height(Length::Fixed(1.0))
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().border.into()),
                ..container::Style::default()
            }),
    ]
    .spacing(8)
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .padding([10, 0])
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
    item: &'a TodoInfo,
    state: TodoState,
    selected: bool,
    grabbing: bool,
    is_drag_source: bool,
    hovered: bool,
    editing_draft: Option<&'a iced_widget::text_editor::Content>,
    categories: &'a [CategoryInfo],
}

fn todo_card<'a>(
    args: TodoCardArgs<'a>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let TodoCardArgs {
        number,
        idx,
        item,
        state,
        selected,
        grabbing,
        is_drag_source,
        hovered,
        editing_draft,
        categories,
    } = args;
    let done = item.done;

    // ---- 顶部行：编号 + 日期徽章(calendar 图标 → 日历选择器)+ 状态文字 ----
    let number_text = text(format!("#{number:03}"))
        .size(byteui::theme::font::caption())
        .color(byteui::theme::color::current().dim);

    let date_label = match state {
        TodoState::Done => item
            .completed_at_ms
            .map(format_todo_month_day)
            .unwrap_or_else(|| "-".to_string()),
        _ => item.plan_date.clone().unwrap_or_else(|| "-".to_string()),
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

    // 状态不再作为顶部静态文字跟在序号后面(`#002 - 待办` 形式废除)——四态
    // 移到卡片底部左下的状态按钮(`todo_status_button`)以"当前状态"作button
    // 文本,点开状态下拉四选。顶部这行只留序号,分类 chip 与日期徽章续排。

    // 分类 chip:显示任务所属分类(找不到就是"未分类"),点击打开分类选择器。
    let category_label = categories
        .iter()
        .find(|c| Some(c.id) == item.category_id)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "未分类".to_string());
    let chip = button(
        text(category_label)
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim),
    )
    .on_press(Message::CategoryPickerOpenForTodo(item.id))
    .padding([2, 8])
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: Some(byteui::theme::color::current().card.into()),
        border: Border {
            color: byteui::theme::color::current().border,
            width: 1.0,
            radius: 10.0.into(),
        },
        ..button::Style::default()
    });

    let top_row = row![
        number_text,
        chip,
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

    // ---- 底部行：左=四态状态按钮(所有卡片都有),右=指派按钮(仅
    // "待办且未派发"会出现——指派本身就是把某条待办推进到"进行中")----
    let status_btn = todo_status_button(state, idx);

    let mut bottom = row![status_btn].spacing(12);
    // 详情按钮:只要这条任务已指派过/已留过会话(有可回看的往来、可继续
    // 人工触发处理)就出现,点击打开任务详情弹窗(`Message::DetailOpen`)。
    if item.assigned_agent.is_some() || item.dispatch_session_id.is_some() {
        let detail_btn = button(
            text("详情")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().gold),
        )
        .on_press(Message::DetailOpen(idx))
        .padding([6, 8])
        .style(move |_t: &iced_widget::Theme, s: button::Status| {
            let hovered = matches!(s, button::Status::Hovered);
            button::Style {
                background: if hovered {
                    Some(byteui::theme::color::current().card.into())
                } else {
                    None
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
                text_color: byteui::theme::color::current().gold,
                ..button::Style::default()
            }
        });
        bottom = bottom.push(detail_btn);
    }
    if state == TodoState::Pending && item.dispatch_session_id.is_none() {
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
        .style(move |_t: &iced_widget::Theme, s: button::Status| {
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
        bottom,
        iced_widget::space::Space::new()
            .width(Length::Fill)
            .height(Length::Shrink),
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

/// `todo_dispatch_overlay` 的原生菜单版本,纯数据组装——只列出有 headless
/// 适配器的四种 agent。仅 macOS 编译。
#[cfg(target_os = "macos")]
pub(crate) fn dispatch_items(idx: usize) -> Vec<crate::native_menu::Item<Message>> {
    [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::V8agent,
    ]
    .into_iter()
    .map(|agent| {
        crate::native_menu::Item::entry(
            Some(agent_icon(agent)),
            agent.label(),
            Message::AssignAgent(idx, agent),
        )
    })
    .collect()
}

/// `todo_status_overlay` 的原生菜单版本,纯数据组装——四态,文字用各自
/// `status_meta` 色。仅 macOS 编译。
#[cfg(target_os = "macos")]
pub(crate) fn status_items(idx: usize) -> Vec<crate::native_menu::Item<Message>> {
    use crate::native_menu::Item;
    [
        TodoState::Pending,
        TodoState::InProgress,
        TodoState::Suspended,
        TodoState::Done,
    ]
    .into_iter()
    .map(|st| {
        let (label, color) = status_meta(st);
        Item::Entry {
            icon: None,
            icon_color: None,
            label: label.into(),
            color,
            enabled: true,
            msg: Message::StatusPick(idx, st),
        }
    })
    .collect()
}

/// Todo 指派选择层(窗口级 overlay 版):选一个 agent 种类完成指派,不再
/// 要求"存在活着的 tab"(2026-09-02 起,指派与执行解耦——指派只是记录,
/// 真正执行靠分类轮询开关或详情弹窗手动"处理")。样式沿用
/// `crate::menu::item_row_fill` + `menu::shell`,定位靠 `dispatch_anchor`。
pub fn todo_dispatch_overlay<'a>(
    ws: &Workspace,
    window_size: (f32, f32),
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let ws_state = &ws.todo;
    let idx = ws_state.dispatch_open?;
    let anchor = ws_state.dispatch_anchor?;

    // 只列出有 headless 适配器的四种(与 `AgentKind::label()` 的四个可指派
    // 值一致,`dozerd::headless_agent::bare_program_name` 同一份覆盖面)。
    let candidates = [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::V8agent,
    ];
    let items: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> = candidates
        .into_iter()
        .map(|agent| {
            let icon = icons::view(
                agent_icon(agent),
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().body,
            );
            crate::menu::item_row(
                Some(icon),
                agent.label().to_string(),
                byteui::theme::color::current().body,
                Some(Message::AssignAgent(idx, agent)),
            )
        })
        .collect();
    let popup = crate::menu::shell_frosted(items, Length::Shrink);

    // 全窗口容器 + padding 把弹层推到锚点;窗口边界钳制,避免超出右下。
    let (ax, ay) = anchor;
    let window_w = window_size.0;
    let window_h = window_size.1;
    // 菜单估算尺寸:常宽约 220(图标 + 文字 + 内边距)、四条候选高约 4*28 + 内边距。
    let pop_w = 224.0_f32;
    let pop_h = 160.0_f32;
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

/// 任务状态文案与专属色,四态一个不落:
/// 待办=青 `CYAN`、进行中=金 `GOLD`、搁置=奶油 `CREAM`(比 DIM 略亮一点、
/// 强调"还没完,只是被拿回来放着")、完成=灰 `DIM`。状态按钮文本与状态下拉
/// 菜单各条目统一从这里取色,不各写一份颜色表。
fn status_meta(state: TodoState) -> (&'static str, Color) {
    match state {
        TodoState::Pending => ("待办", byteui::theme::color::current().cyan),
        TodoState::InProgress => ("进行中", byteui::theme::color::current().gold),
        TodoState::Suspended => ("搁置", byteui::theme::color::current().cream),
        TodoState::Done => ("已完成", byteui::theme::color::current().dim),
    }
}

/// 卡片底部左下的「状态」下拉按钮:文本 = 当前状态(待办/进行中/搁置/已完成,
/// 颜色随 `status_meta`),后跟向下箭头提示展开。点开弹 `todo_status_overlay`
/// 的四个选项。整体观感对齐同一行的「指派」按钮(BG 底 + BORDER 描边 +
/// 圆角 4),避免一张卡里两种按钮风格打架。
fn todo_status_button(
    state: TodoState,
    idx: usize,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (label, color) = status_meta(state);
    button(
        row![
            text(label).size(byteui::theme::font::label()).color(color),
            icons::view(
                icons::IconKind::ChevronDown,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().dim,
            ),
        ]
        .spacing(4)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .on_press(Message::StatusOpen(idx))
    .padding([6, 8])
    .style(move |_t: &iced_widget::Theme, s: button::Status| {
        let hovered = matches!(s, button::Status::Hovered);
        button::Style {
            background: if hovered {
                Some(byteui::theme::color::current().card.into())
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
            text_color: color,
            ..button::Style::default()
        }
    })
    .into()
}

/// Todo 状态下拉选择层(窗口级 overlay 版,设计风格复用 `todo_dispatch_overlay`
/// ——都是"点一个卡片按钮弹出的四选/选项列表",用同一套 `menu::shell` 壳保证
/// 观感一致):从上到下依次列出 待办/进行中/搁置/已完成 四态,每个条目文字用
/// 该态自己的 `status_meta` 色。点某条发 `StatusPick(idx, 该态)`(真实的存储
/// 落盘 / 转派发选择层的分支都在 `Message::StatusPick` 里处理)。返回 `None`
/// 表示 `status_open` 为真但锚点缺失(理论上到不了,调用方降级为只铺 dismiss
/// 收起层)。
pub fn todo_status_overlay<'a>(
    ws: &Workspace,
    window_size: (f32, f32),
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let ws_state = &ws.todo;
    let idx = ws_state.status_open?;
    let anchor = ws_state.status_anchor?;

    let candidates = [
        TodoState::Pending,
        TodoState::InProgress,
        TodoState::Suspended,
        TodoState::Done,
    ];
    let items: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> = candidates
        .into_iter()
        .map(|st| {
            let (label, color) = status_meta(st);
            crate::menu::item_row(None, label, color, Some(Message::StatusPick(idx, st)))
        })
        .collect();
    let popup = crate::menu::shell_frosted(items, Length::Shrink);

    // 全窗口容器 + padding 把弹层推到锚点;窗口边界钳制,避免超出右下。
    let (ax, ay) = anchor;
    let window_w = window_size.0;
    let window_h = window_size.1;
    // 菜单估算尺寸:常宽约 160(文字 + 内边距,比派发列表窄,因为没有图标列),
    // 高约每项 28 + 壳内边距;给足余量。
    let pop_w = 160.0_f32;
    let pop_h = (4.0_f32 * 28.0) + 16.0;
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

/// 搜索框左前「按状态筛选」的选择浮层(窗口级 overlay):从上到下列出
/// 「全部」 + 待办 / 进行中 / 搁置 / 已完成。"全部"不设任何状态约束(奶油
/// 文案);其余四种用各自 `status_meta` 专属色,并在前面缀一个当前正选中的
/// 状态用 **✓** 单字做选中标记,方便一眼看到现在筛在哪个态。点某条发
/// `Message::StatusFilterPick(filter)`(纯本地改 `status_filter`,不动关键词、
/// 不落盘),随后由该消息关闭浮层。`app.rs` 在状态详情浮层(卡片状态按钮)
/// 与日历/派发层之外单独判断,与它们互斥不得同时弹出。
///
/// 返回 `None`:要么浮层根本没打开,要么锚点还没记上(理论上到不了,调用方
/// 降级为只铺 dismiss 收起层)。
pub fn todo_status_filter_overlay<'a>(
    ws: &Workspace,
    window_size: (f32, f32),
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let ws_state = &ws.todo;
    if !ws_state.status_filter_open {
        return None;
    }
    let anchor = ws_state.status_filter_anchor?;
    // 记下当前正筛中的状态(“全部”时 `None”),用于在浮层里给对应项打 ✓。
    let checked_state = match ws_state.status_filter() {
        StatusFilter::All => None,
        StatusFilter::Status(st) => Some(st),
    };

    let mut items: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        Vec::new();
    let all_checked = checked_state.is_none();
    let marker = if all_checked { "✓ " } else { "  " };
    let cream = byteui::theme::color::current().cream;
    let all_btn = button(
        text(format!("{marker}全部"))
            .size(byteui::theme::font::body())
            .color(cream),
    )
    .on_press(Message::StatusFilterPick(StatusFilter::All))
    .width(Length::Fill)
    .padding([6, 10])
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: cream,
        ..button::Style::default()
    });
    items.push(all_btn.into());

    // 四种状态——带当前选中的先导记号(✓),其余补两个空格以对齐列宽。
    let order = [
        TodoState::Pending,
        TodoState::InProgress,
        TodoState::Suspended,
        TodoState::Done,
    ];
    for st in order {
        let (lbl, c) = status_meta(st);
        let checked = checked_state == Some(st);
        let lead = if checked { "✓ " } else { "  " };
        let row = button(
            text(format!("{lead}{lbl}"))
                .size(byteui::theme::font::body())
                .color(c),
        )
        .on_press(Message::StatusFilterPick(StatusFilter::Status(st)))
        .width(Length::Fill)
        .padding([6, 10])
        .style(move |_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: c,
            ..button::Style::default()
        });
        items.push(row.into());
    }

    let popup = container(column(items).spacing(2))
        .width(Length::Fixed(200.0))
        .padding(4)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().card.into()),
            border: Border {
                color: byteui::theme::color::current().border,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        });

    let (ax, ay) = anchor;
    let pop_w = 200.0_f32;
    let pop_h = 5.0_f32 * 34.0 + 10.0;
    let x = ax.min((window_size.0 - pop_w).max(0.0));
    let y = ay.min((window_size.1 - pop_h).max(0.0));
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
/// `window_size` 用于边界钳制。任务已存的 plan_date 直接读 `TodoInfo`。
pub fn todo_calendar_overlay<'a>(
    ws: &Workspace,
    window_size: (f32, f32),
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let ws_state = &ws.todo;
    let idx = ws_state.calendar_open?;
    let anchor = ws_state.calendar_anchor?;
    let item = ws_state.items.get(idx)?;
    let selected = item.plan_date.clone();
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

/// 分类树导航区:钉顶的"全部"/"未分类"伪节点 + 用户自建分类节点(可
/// 展开/收起、点选切过滤)。本函数只做展示 + 选中;右键菜单/增删改在
/// 后续任务接入。
fn category_tree_nav<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(2).padding([4, 8]);

    // "全部"/"未分类"伪节点可右键弹出顶层"新建分类"(见
    // `CategoryContextMenuOpen(None)` 的语义)。两者都新建的是顶层分类
    // (新建后并不会真挂在哪个名字下面——全部/未分类只是视图桶)。
    col = col.push(byteui::interaction::context_menu::wrap(
        category_pseudo_row(
            icons::IconKind::CircleSmall,
            "全部",
            ws_state.category_selected() == CategoryFilter::All,
            Message::CategorySelect(CategoryFilter::All),
        ),
        Some(Message::CategoryContextMenuOpen(None)),
    ));
    col = col.push(byteui::interaction::context_menu::wrap(
        category_pseudo_row(
            icons::IconKind::CircleSmall,
            "未分类",
            ws_state.category_selected() == CategoryFilter::Uncategorized,
            Message::CategorySelect(CategoryFilter::Uncategorized),
        ),
        Some(Message::CategoryContextMenuOpen(None)),
    ));

    let rows = visible_category_rows(ws_state.categories(), ws_state.category_expanded());
    for row in rows {
        if let Some((renaming_id, draft)) = ws_state.category_renaming()
            && renaming_id == row.id
        {
            col = col.push(category_rename_row(row.depth, draft));
            continue;
        }
        let active = ws_state.category_selected() == CategoryFilter::Node(row.id);
        let fg = if active {
            byteui::theme::color::current().cream
        } else {
            byteui::theme::color::current().dim
        };
        let chevron: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
            if row.has_children {
                icons::icon_button_entry(
                    if row.expanded {
                        icons::IconKind::ChevronDown
                    } else {
                        icons::IconKind::ChevronRight
                    },
                    byteui::theme::icon_size::row(),
                    active,
                    !active,
                    app.hover_progress(HoverId::TodoCategoryRow(row.id)),
                    false,
                    byteui::theme::geometry::tab_button_size(),
                    true,
                    Message::CategoryToggleExpand(row.id),
                    move |hovered| Message::Hover(HoverId::TodoCategoryRow(row.id), hovered),
                    if row.expanded { "收起" } else { "展开" },
                )
            } else {
                space::Space::new()
                    .width(byteui::theme::geometry::tab_button_size())
                    .into()
            };
        let label = button(
            row![
                chevron,
                text(row.name.clone())
                    .size(byteui::theme::font::body())
                    .color(fg),
            ]
            .spacing(4)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        )
        .on_press(Message::CategorySelect(CategoryFilter::Node(row.id)))
        .width(Length::Fill)
        .padding([6, 4 + (row.depth as u16) * 16])
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
        });
        col = col.push(byteui::interaction::context_menu::wrap(
            label.into(),
            Some(Message::CategoryContextMenuOpen(Some(row.id))),
        ));
    }
    col.into()
}

/// 分类树一行的行内改名输入框,镜像 `files.rs::tree_edit_row`(同款
/// `byteui::form::input_text::view` 单行 `text_input`,`on_submit` 直接
/// 回车提交,缩进用外层 `Padding::left` 换算,不拼进文本内容)。
fn category_rename_row<'a>(
    depth: usize,
    draft: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let indent_px = 4.0 + depth as f32 * 16.0; // 对齐 category_tree_nav 里真实行的缩进算法
    let field = container(byteui::form::input_text::view(
        "",
        draft,
        false,
        Some(category_rename_field_id()),
        false,
        Some(Message::CategoryRenameSubmit),
        false,
        Message::CategoryRenameEdit,
    ))
    .width(Length::Fill)
    .padding(Padding {
        left: indent_px,
        ..Padding::default()
    });
    byteui::interaction::context_menu::wrap(
        field.into(),
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: category_rename_field_id(),
            secure: false,
        })),
    )
}

/// "全部"/"未分类"两个不可删除/不可右键的伪节点行,视觉对齐真实分类
/// 节点但没有展开箭头。
fn category_pseudo_row<'a>(
    icon: icons::IconKind,
    label: &'a str,
    active: bool,
    on_press: Message,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
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
        ]
        .spacing(8)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .on_press(on_press)
    .width(Length::Fill)
    .padding([6, 10])
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

/// 完成时间(毫秒) → "MM-DD"（SUCCESS 徽章用，只取月日）。
fn format_todo_month_day(ms: u64) -> String {
    let secs = ms / 1000;
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

    fn todo_info(id: i64, text: &str, done: bool) -> TodoInfo {
        TodoInfo {
            id,
            project_id: 1,
            text: text.to_string(),
            done,
            paused: false,
            rank: 0,
            created_ms: 0,
            completed_at_ms: None,
            plan_date: None,
            dispatch_session_id: None,
            dispatch_at_ms: None,
            category_id: None,
            assigned_agent: None,
        }
    }

    #[test]
    fn display_state_done_wins_over_dispatch() {
        let mut item = todo_info(1, "任务A", true);
        item.dispatch_session_id = Some("s1".into());
        assert_eq!(todo_display_state(&item, true), TodoState::Done);
        item.done = false;
        assert_eq!(todo_display_state(&item, true), TodoState::InProgress);
        assert_eq!(todo_display_state(&item, false), TodoState::Pending);
        item.dispatch_session_id = None;
        assert_eq!(todo_display_state(&item, true), TodoState::Pending);
    }

    fn sample_states() -> Vec<TodoInfo> {
        vec![
            todo_info(1, "修复登录页闪烁", false),
            todo_info(2, "补 README 安装说明", true),
            todo_info(3, "给 claude 指派生成报告", false),
        ]
    }

    #[test]
    fn filter_all_with_empty_query_keeps_everything() {
        let items = sample_states();
        let hits = filter_todos(&items, "");
        assert_eq!(hits, vec![0, 1, 2]);
    }

    #[test]
    fn filter_by_keyword_case_insensitive_substring() {
        let items = sample_states();
        assert_eq!(filter_todos(&items, "CLAUDE"), vec![2]);
    }

    #[test]
    fn filter_keyword_is_substring_match_on_text() {
        let items = sample_states();
        assert_eq!(filter_todos(&items, "登录"), vec![0]);
        assert_eq!(filter_todos(&items, "不存在的关键词"), Vec::<usize>::new());
    }

    #[test]
    fn completed_at_for_toggle_reflects_done() {
        let now = std::time::SystemTime::now();
        assert_eq!(completed_at_for_toggle(true, now), Some(now));
        assert_eq!(completed_at_for_toggle(false, now), None);
    }

    #[test]
    fn calendar_parse_month_day_accepts_mm_dd_and_rejects_bad_input() {
        assert_eq!(parse_month_day("08-10"), Some((8, 10)));
        assert_eq!(parse_month_day("8-3"), Some((8, 3)));
        assert_eq!(parse_month_day("13-01"), None);
        assert_eq!(parse_month_day("08"), None);
        assert_eq!(parse_month_day("abc"), None);
    }

    #[test]
    fn calendar_days_in_month_handles_leap_years() {
        assert_eq!(days_in_month(2024, 2), 29); // 闰
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2024, 4), 30);
        assert_eq!(days_in_month(2024, 1), 31);
        assert_eq!(days_in_month(2024, 13), 0);
    }

    #[test]
    fn calendar_first_weekday_of_1970_jan_is_thursday() {
        assert_eq!(first_weekday_of_month(1970, 1), 4); // 周四
    }

    #[test]
    fn calendar_days_from_civil_round_trips() {
        use super::civil_from_days;
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    }

    #[test]
    fn format_month_day_from_ms() {
        // 1970-01-01 的毫秒。
        assert_eq!(format_todo_month_day(0), "01-01");
    }

    #[test]
    fn task_title_for_session_finds_dispatched_task() {
        let mut ws_state = WorkspaceState {
            items: vec![todo_info(1, "任务A", false), todo_info(2, "任务B", false)],
            ..Default::default()
        };
        ws_state.items[1].dispatch_session_id = Some("sess-1".to_string());
        assert_eq!(ws_state.task_title_for_session("sess-1"), Some("任务B"));
    }

    #[test]
    fn task_title_for_session_none_when_no_dispatch_matches() {
        let ws_state = WorkspaceState {
            items: vec![todo_info(1, "任务A", false)],
            ..Default::default()
        };
        assert_eq!(ws_state.task_title_for_session("sess-1"), None);
    }

    fn editing_state(text: &str) -> WorkspaceState {
        WorkspaceState {
            items: vec![todo_info(7, "旧文字", false)],
            editing_content: Some((0, iced_widget::text_editor::Content::with_text(text))),
            ..Default::default()
        }
    }

    #[test]
    fn commit_content_edit_returns_id_and_text_when_changed() {
        let mut ws_state = editing_state("新文字");
        let result = ws_state.commit_content_edit();
        assert_eq!(result, Some((7, "新文字".to_string())));
        assert!(ws_state.editing_content.is_none());
    }

    #[test]
    fn commit_content_edit_none_when_unchanged() {
        let mut ws_state = WorkspaceState {
            items: vec![todo_info(7, "同样的文字", false)],
            editing_content: Some((
                0,
                iced_widget::text_editor::Content::with_text("同样的文字"),
            )),
            ..Default::default()
        };
        assert_eq!(ws_state.commit_content_edit(), None);
    }

    #[test]
    fn commit_content_edit_none_when_draft_empty() {
        let mut ws_state = WorkspaceState {
            items: vec![todo_info(7, "旧文字", false)],
            editing_content: Some((0, iced_widget::text_editor::Content::new())),
            ..Default::default()
        };
        assert_eq!(ws_state.commit_content_edit(), None);
    }

    fn cat(id: i64, parent_id: Option<i64>, name: &str) -> CategoryInfo {
        CategoryInfo {
            id,
            project_id: 1,
            parent_id,
            name: name.into(),
            rank: 0,
            created_ms: 0,
            auto_poll_enabled: false,
        }
    }

    #[test]
    fn visible_category_rows_flattens_by_expanded_state() {
        // 前端(展开) -> UI, 性能(未展开无子节点)
        //   UI(未展开) -> 组件库(不可见,父未展开)
        // 后端(未展开) -> 数据库(不可见)
        let categories = vec![
            cat(1, None, "前端"),
            cat(2, Some(1), "UI"),
            cat(3, Some(1), "性能"),
            cat(4, Some(2), "组件库"),
            cat(5, None, "后端"),
            cat(6, Some(5), "数据库"),
        ];
        let mut expanded = std::collections::HashSet::new();
        expanded.insert(1);
        let rows = visible_category_rows(&categories, &expanded);
        let visible_ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        assert_eq!(
            visible_ids,
            vec![1, 2, 3, 5],
            "只展开了前端,UI/后端的子节点都不可见"
        );
        let front = rows.iter().find(|r| r.id == 1).unwrap();
        assert_eq!(front.depth, 0);
        assert!(front.has_children);
        assert!(front.expanded);
        let ui = rows.iter().find(|r| r.id == 2).unwrap();
        assert_eq!(ui.depth, 1);
        assert!(
            ui.has_children,
            "UI 有子节点组件库,即使未展开也要标记有子节点"
        );
        assert!(!ui.expanded);
        let perf = rows.iter().find(|r| r.id == 3).unwrap();
        assert!(!perf.has_children);
    }

    #[test]
    fn category_descendants_covers_multi_level_and_excludes_root() {
        let categories = vec![
            cat(1, None, "前端"),
            cat(2, Some(1), "UI"),
            cat(3, Some(2), "组件库"),
            cat(4, None, "后端"),
        ];
        let descendants = category_descendants(&categories, 1);
        assert_eq!(
            descendants,
            std::collections::HashSet::from([2, 3]),
            "根节点自己不算子孙,后端这条不相关分支不应该出现"
        );
        assert!(
            category_descendants(&categories, 3).is_empty(),
            "叶子节点没有子孙"
        );
    }

    #[test]
    fn filter_todos_by_category_all_returns_everything() {
        let todos = vec![todo_with_category(1, Some(10)), todo_with_category(2, None)];
        let categories = vec![cat(10, None, "分类")];
        let ids = filter_todos_by_category(&todos, &categories, CategoryFilter::All);
        assert_eq!(ids, std::collections::HashSet::from([1, 2]));
    }

    #[test]
    fn filter_todos_by_category_uncategorized_only() {
        let todos = vec![todo_with_category(1, Some(10)), todo_with_category(2, None)];
        let ids = filter_todos_by_category(&todos, &[], CategoryFilter::Uncategorized);
        assert_eq!(ids, std::collections::HashSet::from([2]));
    }

    #[test]
    fn filter_todos_by_category_node_includes_descendant_tasks() {
        // 前端(id 10) -> UI(id 11);任务 1 挂前端,任务 2 挂 UI,任务 3 未分类。
        // 选中"前端"应该看到任务 1 和任务 2(子孙汇总)。
        let todos = vec![
            todo_with_category(1, Some(10)),
            todo_with_category(2, Some(11)),
            todo_with_category(3, None),
        ];
        let categories = vec![cat(10, None, "前端"), cat(11, Some(10), "UI")];
        let ids = filter_todos_by_category(&todos, &categories, CategoryFilter::Node(10));
        assert_eq!(ids, std::collections::HashSet::from([1, 2]));
        // 选中叶子节点"UI"只看到任务 2。
        let ids = filter_todos_by_category(&todos, &categories, CategoryFilter::Node(11));
        assert_eq!(ids, std::collections::HashSet::from([2]));
    }

    fn todo_with_category(id: i64, category_id: Option<i64>) -> TodoInfo {
        TodoInfo {
            id,
            project_id: 1,
            text: format!("任务{id}"),
            done: false,
            paused: false,
            rank: 0,
            created_ms: 0,
            completed_at_ms: None,
            plan_date: None,
            dispatch_session_id: None,
            dispatch_at_ms: None,
            category_id,
            assigned_agent: None,
        }
    }
}
