//! Todo 面板状态类型与状态机:TodoState/TodoView/分类/状态过滤/拖拽/
//! WorkspaceState/Message/焦点捕获/刷新请求。

use crate::app::HoverId;

use dozer_client::Client;
use dozer_core::protocol::{CategoryInfo, TodoInfo};
use iced_widget::core::Rectangle;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};

use super::*;

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
pub(crate) fn is_active_todo(item: &TodoInfo) -> bool {
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
pub(crate) struct Flash {
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
/// Todo 面板挂在每个 `Workspace` 上的状态。
#[derive(Default)]
pub struct WorkspaceState {
    pub(crate) items: Vec<TodoInfo>,
    /// 新增任务框草稿。类型从 `String` 换成 `iced_widget::text_editor::
    /// Content`(实现 `Default`/`Clone`)——真正的 `text_editor` 自己管理
    /// 光标/选区,不再需要应用层维护 `add_cursor` 字符下标。
    pub(crate) add_draft: iced_widget::text_editor::Content,
    /// 新增任务框是否持有 iced 内部真实焦点。**不是**应用层手动置位的
    /// 镜像——每帧渲染循环里 `CaptureAddFocus` 问一遍 iced 真相后立刻写
    /// 进这里(`set_add_focused`)。
    pub(crate) add_focused: bool,
    /// 新增任务框高度(逻辑像素)。框顶的拖拽手柄向上拉时由 app 层换算写回
    /// (见 `app.rs::RowDrag` 的 `TodoAddGrow` 分支),0 表示"未拖过、用默认
    /// 高",视图侧一律 `max(ADD_INPUT_MIN_HEIGHT)` 兜底——这样 `#[derive(
    /// Default)]` 给的 0 也不会渲染成 0 高框。
    pub(crate) add_input_height: f32,
    pub(crate) selected_row: Option<usize>,
    /// 新增任务后"置顶 + 选中保持 2 秒"的计时态。`Some(Flash)` 表示刚新增
    /// 了一条任务,其卡片的 `selected_row` 高亮要在 `flash.until` 时刻自动
    /// 清除;期间的挂起唤醒由 `next_flash_wake` 驱动(main.rs 据此排下次
    /// 重绘,见 `App::advance_flash`)。用户手动点了其它卡片会清除本字段,
    /// 不再让 2 秒倒计时去抢用户的主动选中。
    pub(crate) flash: Option<Flash>,
    /// 新增任务后请求"下滑列表到底/置顶"的一次性滚动位标记。新增置顶后
    /// 让列表滚回顶部使新任务可见;由 main.rs 在下一帧 `interface.operate`
    /// 消耗(见 `App::take_todo_scroll_to_top`),置位后一直为 `true` 直到
    /// 被取走,避免主事件循环与渲染循环的帧序差异漏掉这次滚动。
    pub(crate) scroll_to_top: bool,
    /// 已生效的搜索关键词(列表过滤用)。打字期间只改草稿 `search_draft`,
    /// 回车/点右侧搜索按钮才落成这里(与文件树搜索 `search_query` 同款
    /// "草稿→提交"模型)。
    pub(crate) search: String,
    /// 搜索框草稿(iced `text_input` 的 `value`)。
    pub(crate) search_draft: String,
    /// 搜索框是否持有 iced 内部真实焦点。**不是**应用层手动置位的镜像——
    /// 每帧渲染循环里 `CaptureTodoSearchFocus` 问一遍 iced 真相后立刻写
    /// 进这里(`set_search_focused`),`main.rs` 键盘路由读它决定要不要
    /// 放行给标准 iced 管线。
    pub(crate) search_focused: bool,
    pub(crate) dispatch_open: Option<usize>,
    /// 派发选择层浮层弹出锚点(逻辑像素,取点击"指派"按钮时的光标位置)。
    /// 窗口级 overlay 靠它定位到按钮旁边;关闭时清空。
    pub(crate) dispatch_anchor: Option<(f32, f32)>,
    /// 状态下拉展开态(卡片下标):点卡片左下角状态按钮弹出四态(待办/进行中/
    /// 搁置/已完成)选择层。与 `dispatch_open`/`calendar_open` 同一种"同
    /// 时只能有一个"模型——展开状态下拉后,其它同样的弹层都认为关闭。
    pub(crate) status_open: Option<usize>,
    /// 状态下拉浮层弹出锚点(逻辑像素,取点击状态按钮时的光标位置)。与
    /// `dispatch_anchor` 同款窗口级 overlay 定位手法;关闭时清空。
    pub(crate) status_anchor: Option<(f32, f32)>,
    /// 日历日期选择器展开态(卡片下标),`None` = 未展开。跟 `dispatch_open`
    /// 同一种"同时只能有一个"模型。
    pub(crate) calendar_open: Option<usize>,
    /// 日历当前展示的 (年, 月)。打开时初始化成当前月,上一月/下一月导航
    /// 只改这个视图态,不落盘。
    pub(crate) calendar_view: (i32, u32),
    /// 日历浮层弹出锚点(逻辑像素,取点击日历按钮时的光标位置)。窗口级
    /// overlay 靠它定位到按钮旁边;关闭时清空。
    pub(crate) calendar_anchor: Option<(f32, f32)>,
    /// 任务内容行内编辑态(卡片下标, 草稿)。点卡片任务文字进入,失焦或
    /// 回车落盘改写任务文字(`commit_content_edit`)。草稿用
    /// `iced_widget::text_editor::Content`(多行,随内容自然撑高),不再用
    /// 单行 `String`——编辑态的输入框改走 `byteui::form::text_area`。
    pub(crate) editing_content: Option<(usize, iced_widget::text_editor::Content)>,
    /// 任务内容编辑框是否持有 iced 内部真实焦点,每帧由
    /// `CaptureContentEditFocus` 写入。
    pub(crate) content_edit_focused: bool,
    /// 一次性标记:`editing_content` 刚从 `None` 变成 `Some`(点卡片文字
    /// 刚触发编辑)时置真,main.rs 渲染循环取走后用 `operation::
    /// focusable::focus` 强制聚焦(同 `files::tree_edit_focus_pending`
    /// 的既有手法——点卡片文字这个点击落在旧的文字 `MouseArea` 上,不是
    /// 新出现的 `text_input` 本身,不会自动带焦点)。
    pub(crate) content_edit_focus_pending: bool,
    /// 鼠标拖拽排序进行态(`None` = 没在拖)。见 `TodoDrag`。视图层据此对
    /// 待办子序列做展示置换并改光标为抓取态；落盘只在 `DragEnd` 时一次性
    /// 发生。已完成任务不可拖动(见 `RowSelect`/`DragMove` 的不变量)。
    pub(crate) drag: Option<TodoDrag>,
    /// 当前项目全部分类节点,随 `ListTodos` 同一轮轮询一并拉取
    /// (`request_categories_refresh`)。
    pub(crate) categories: Vec<CategoryInfo>,
    /// 展开的分类节点 id,纯 UI 态,不落盘(对齐 `FileTree::expanded`
    /// 同样"只在内存里"的处理)。
    pub(crate) category_expanded: std::collections::HashSet<i64>,
    /// 当前选中的分类过滤节点,默认"全部"。
    pub(crate) category_selected: CategoryFilter,
    /// 搜索框左前「按状态」下拉筛选中项,默认"全部"(不做状态维度过滤)。
    /// 与 `category_selected` 并列两维:右区列表 =关键词∩分类∩状态,三段
    /// 展示随选中态收敛(详见 `todo_list_view`)。
    pub(crate) status_filter: StatusFilter,
    /// 搜索框左前的状态筛选浮层是否展开(`true` 时点状态 segment 弹出
    /// "全部 + 四种状态"选择层)。用了同卡内状态按钮一样的"锚点 + 窗口级
    /// overlay"展开模式,见 `status_filter_anchor`。
    pub(crate) status_filter_open: bool,
    /// 状态筛选浮层弹出锚点(逻辑像素,取点击"全部/某状态"segment 时的光标
    /// 位置)。关闭后清空。
    pub(crate) status_filter_anchor: Option<(f32, f32)>,
    /// 分类树行内改名态(分类 id, 草稿字符串),镜像
    /// `files::TreeEdit`——单行文本,不用 `text_editor::Content`。
    pub(crate) category_renaming: Option<(i64, String)>,
    /// 改名框是否持有 iced 真实焦点,镜像 `files.rs::tree_edit_focused`。
    pub(crate) category_rename_focused: bool,
    /// 一次性聚焦标记,镜像 `files.rs::tree_edit_focus_pending`。
    pub(crate) category_rename_focus_pending: bool,
    /// 右区视图模式:列表视图 `List`(默认)/ 看板视图 `Kanban`。
    pub(crate) view: TodoView,
    /// 详情弹窗展开态(卡片下标),`None` = 未展开。跟 `dispatch_open`/
    /// `calendar_open` 同一种"同时只能有一个"模型。
    pub(crate) detail_open: Option<usize>,
    /// 详情弹窗拉到的回合列表(`GetTodoDetail` 应答),弹窗关闭时清空。
    pub(crate) detail_turns: Vec<dozer_core::protocol::TurnRecord>,
    /// 回复框草稿(`text_input` 的 value)。
    pub(crate) detail_reply_draft: String,
    /// 回复框是否持有 iced 内部真实焦点,每帧由 `CaptureDetailReplyFocus`
    /// 写入,镜像 `category_rename_focused`。
    pub(crate) detail_reply_focused: bool,
    /// "处理"按钮是否正在等待 `ProcessTodoNow` RPC 返回——耗时可能到 10
    /// 分钟,期间按钮显示 loading 态、禁用重复提交。
    pub(crate) detail_processing: bool,
    /// "清空列表"确认弹窗是否展开。点 footbar「清空列表」按钮先弹这个
    /// 确认框(危险操作,不可撤销),取消/遮罩收起,确认才真正触发
    /// `Message::ClearListConfirm`。
    pub(crate) clear_confirm: bool,
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
    pub(crate) fn start_flash(&mut self, idx: usize) {
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
    pub(crate) fn toggle_category_expanded(&mut self, id: i64) {
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
    pub(crate) fn set_category_rename_draft(&mut self, text: String) {
        if let Some((_, draft)) = self.category_renaming.as_mut() {
            *draft = text;
        }
    }

    /// 提交(回车/失焦边缘触发共用)。返回 `Some((id, new_name))` 表示有
    /// 改动需要落盘(空白/未变都视为无改动,直接丢弃草稿)。
    pub(crate) fn commit_category_rename(&mut self) -> Option<(i64, String)> {
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
pub(crate) const OPTIMISTIC_TODO_ID: i64 = 0;

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
