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

/// Todo 面板挂在每个 `Workspace` 上的状态。
#[derive(Default)]
pub struct WorkspaceState {
    /// 待发提示(失败/被拒等一次性反馈)。`App::update` 的包装函数每次处理完消息后
    /// 统一排空成 Toast,见 `extensions::toast::Outbox`。
    pub(crate) outbox: crate::extensions::toast::Outbox,
    pub(crate) items: Vec<TodoInfo>,
    /// 新增任务框高度(逻辑像素)。前端拖拽手柄调高后经 `AddHeight` 事件写回,
    /// 0 表示"未拖过、用默认高",视图侧一律 `max(ADD_INPUT_MIN_HEIGHT)` 兜底。
    pub(crate) add_input_height: f32,
    pub(crate) selected_row: Option<usize>,
    /// 新增任务后"置顶 + 选中保持 2 秒"的计时态。`Some(Flash)` 表示刚新增
    /// 了一条任务,其卡片的 `selected_row` 高亮要在 `flash.until` 时刻自动
    /// 清除;期间的挂起唤醒由 `next_flash_wake` 驱动(main.rs 据此排下次
    /// 重绘,见 `App::advance_flash`)。用户手动点了其它卡片会清除本字段,
    /// 不再让 2 秒倒计时去抢用户的主动选中。
    pub(crate) flash: Option<Flash>,
    /// 每次 `start_flash`(新增任务后要滚回顶部)时递增;随推送带给 webview,
    /// 前端据此滚回顶部。取代旧 iced `scrollable::scroll_to` 的一次性标记。
    pub(crate) scroll_nonce: u32,
    /// 当前项目全部分类节点,随 `ListTodos` 同一轮轮询一并拉取
    /// (`request_categories_refresh`)。
    pub(crate) categories: Vec<CategoryInfo>,
    /// 展开的分类节点 id,纯 UI 态,不落盘(对齐 `FileTree::expanded`
    /// 同样"只在内存里"的处理)。
    pub(crate) category_expanded: std::collections::HashSet<i64>,
    /// 当前选中的分类过滤节点,默认"全部"。
    pub(crate) category_selected: CategoryFilter,
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
    /// 取走待发提示(`App::drain_outboxes` 调用)。
    pub fn take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending> {
        self.outbox.take()
    }

    /// 开场新增任务选中闪光:`selected_row` 置为新增后的下标并保持
    /// `ADD_SELECT_HIGHLIGHT`;同时递增 `scroll_nonce`,让前端滚回顶部使
    /// 新任务可见。
    pub(crate) fn start_flash(&mut self, idx: usize) {
        self.selected_row = Some(idx);
        self.flash = Some(Flash {
            idx,
            until: std::time::Instant::now() + ADD_SELECT_HIGHLIGHT,
        });
        self.scroll_nonce = self.scroll_nonce.wrapping_add(1);
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

    /// 只读当前已加载的任务列表,给视图层渲染与内核处理按下标取任务用。
    pub fn items(&self) -> &[TodoInfo] {
        &self.items
    }

    /// 当前"选中高亮"的任务 id(新增后 2 秒高亮用),供推送给 webview。
    pub fn selected_id(&self) -> Option<i64> {
        self.selected_row
            .and_then(|i| self.items.get(i))
            .map(|it| it.id)
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

    /// 新增任务框有效高度:0(默认/未拖过)按最小高兜底,避免每帧重建时
    /// 渲染成 0 高框。见 `add_input_height` 字段注释。
    pub fn add_input_height(&self) -> f32 {
        self.add_input_height.max(ADD_INPUT_MIN_HEIGHT)
    }

    /// 前端拖拽手柄调高后经 `AddHeight` 事件写回,钳到
    /// `[ADD_INPUT_MIN_HEIGHT, ADD_INPUT_MAX_HEIGHT]`。
    pub fn set_add_input_height(&mut self, h: f32) {
        self.add_input_height = h.clamp(ADD_INPUT_MIN_HEIGHT, ADD_INPUT_MAX_HEIGHT);
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
            tool_result_call_id: None,
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

/// Todo 面板挂在 `App` 上的本地元数据 sidecar 已随 SQLite 迁移整体删除;
/// 派发记录/计划时间/完成时间一律存 `dozerd::todo::TodoStore`(经 `Client`),
/// 权威字段在 `TodoInfo` 上。

#[derive(Debug, Clone)]
pub enum Message {
    Toggle(usize),
    /// 内容侧"收起/展开列表列"按钮:内核拦截,不进 `update`——转发成顶层
    /// `Message::TogglePanelListCollapse(PanelKind::Todo)`(见 app.rs)。
    ToggleListCollapse,
    /// 新增框 / 搜索框 / 内容编辑框被右键:内核拦截,不进 `update`——转发成
    /// 顶层 `Message::TextInputMenuOpen` 弹出通用输入框右键菜单(见 app.rs)。
    TextInputMenuOpen(crate::app::TextInputTarget),
    /// 关闭详情弹窗(Esc / 点外部 / 点关闭按钮)。
    DetailClose,
    /// 详情弹窗回复框草稿变化(`text_input::on_input`,给全量当前字符串)。
    DetailReplyInput(String),
    /// 点"处理"按钮:内核拦截,转发到 `App::todo_detail_process`(乐观插入
    /// 一条本地回合 + 发 `ProcessTodoNow` RPC)。`todo::update` 只清空
    /// 草稿、置处理中标记。
    DetailReplySubmit,
    /// webview 收起/展开状态选择后选定某一态:`state` 是用户想切到的存储
    /// 状态(`pending`/`suspended`/`done`,见 `SetStatusTarget`)。由
    /// `route_event` 从卡片 id 反查下标后产生。
    StatusPick(usize, TodoState),
    /// webview 里选定的计划日期,`day` 是 "MM-DD" 文本(与 `plan_date` 存储
    /// 格式一致)。由 `route_event` 从卡片 id 反查下标后产生。
    CalendarPick(usize, String),
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
    /// webview `Add`:新增任务文本(已 trim、非空、未超长)。
    AddText(String),
    /// webview `EditText`:按 id 改文字。
    EditText(i64, String),
    /// webview `Reorder`:把 `id` 挪到 `after_id` 之后(`None` = 进行中段最前)。
    ReorderTo {
        id: i64,
        after_id: Option<i64>,
    },
    /// webview `SetCategory`:`None` = 未分类。
    SetCategory(i64, Option<i64>),
    /// webview `AddHeight`:新增框高度(px),由 `set_add_input_height` 钳制。
    AddHeight(f32),
    /// webview 加载失败后原生占位页的「重试」。
    ContentRetry,
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
