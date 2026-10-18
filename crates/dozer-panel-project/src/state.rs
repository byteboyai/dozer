//! Project 面板状态类型:每个 workspace 一份的 [`WorkspaceState`]、消息与焦点捕获。
//!
//! 逐字搬自 Dozer `extensions/project.rs`(只换可见性与两处归属):
//! 提示发件箱改用 `dozer-panel-host` 的中性 [`NoticeOutbox`],请求身份改用
//! [`RequestTrackers`],名字/描述编辑框的 `widget::Id` 改由 [`crate::adapter::TextInputTarget`]
//! 上报给产品。

use dozer_model::LocalizedText;
use dozer_panel_host::{Notice, NoticeOutbox, RequestTrackers};
use iced_widget::core::Rectangle;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use std::path::PathBuf;

use crate::adapter::TextInputTarget;
use crate::{delete, links, scaffold};

/// 单个同步 scaffold 步骤(缓存目录/README/git 仓库/项目文档)
/// 在弹窗里的实时状态。`ScaffoldStepResult` 只有终态,这里补一层
/// pending/running。
#[derive(Debug, Clone, PartialEq)]
pub enum ScaffoldStepState {
    Pending,
    Running,
    Done(scaffold::ScaffoldStepResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackfillProgress {
    pub completed: u32,
    pub total: u32,
    /// 成功 / 失败 / 跳过计数(spec 2026-09-26 第 8 节:修复项目显示
    /// "成功 X / 失败 Y / 跳过 Z")。旧 `completed/total` 仍作为"处理完
    /// N/M"的总进度保留。
    pub succeeded: u32,
    pub failed: u32,
    pub skipped: u32,
}

/// "补总结"聚合进度行的状态。`Done` 现在区分成功/失败/跳过——不再像旧版
/// 那样"每条都有降级路径、永远算处理完"(新版失败不写启发式,见 dozerd 侧
/// summary_service)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackfillStepState {
    Pending,
    Running(BackfillProgress),
    Done(BackfillProgress),
    Failed(LocalizedText),
}

/// 一次"修复项目"弹窗跑的完整状态:4 个同步步骤 + 转录历史补录 + 补总结
/// 聚合进度。`steps` 里固定 5 项,顺序 = `scaffold::scaffold_steps()`
/// 的 4 项 + 追加的"agent 历史"转录补录(与既有 `spawn_scaffold_run` 的
/// 拼接顺序一致)。
#[derive(Debug, Clone, PartialEq)]
pub struct ScaffoldRunState {
    pub steps: Vec<(&'static str, ScaffoldStepState)>,
    pub backfill: BackfillStepState,
}

impl ScaffoldRunState {
    /// 初始态:全部步骤 Pending,补总结也 Pending。`transcript` 为假时不追加
    /// "agent 历史"那一步(产品没有转录能力)。
    pub fn pending(transcript: bool) -> Self {
        let mut steps: Vec<(&'static str, ScaffoldStepState)> = scaffold::scaffold_steps()
            .into_iter()
            .map(|s| (s.label_key, ScaffoldStepState::Pending))
            .collect();
        if transcript {
            steps.push(("scaffold.step.agent_history", ScaffoldStepState::Pending));
        }
        Self {
            steps,
            backfill: BackfillStepState::Pending,
        }
    }

    pub fn all_done(&self) -> bool {
        self.steps
            .iter()
            .all(|(_, s)| matches!(s, ScaffoldStepState::Done(_)))
            && matches!(
                self.backfill,
                BackfillStepState::Done(_) | BackfillStepState::Failed(_)
            )
    }
}

/// 新建共享记忆的表单草稿。四个字段分别对应 `write_memory` 的入参,
/// 用具名结构体而不是四个散落的 `String` 局部变量,避免以后传参时顺序
/// 传错(`title`/`kind`/`description` 三个 `String` 相邻,属于 CLAUDE.md
/// 参数结构体裁决要规避的形状)。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemoryDraft {
    pub title: String,
    pub kind: String,
    pub description: String,
    pub body: String,
}

/// 挂在每个 Workspace 上的项目信息面板状态。
#[derive(Default)]
pub struct WorkspaceState {
    pub(crate) requests: RequestTrackers,
    pub(crate) branch: Option<String>,
    pub(crate) dirty: bool,
    /// git remote 的 fetch URL 列表(`bytegit::Repo::remotes`,去重后)。空 = 无 remote/
    /// 非 git(面板据此显示"未设置")。
    pub(crate) remote_url: Vec<String>,
    /// 磁盘占用字节数(排除构建产物)。None=尚未算出来。
    pub(crate) disk_usage_bytes: Option<u64>,
    /// 项目描述(`.dozer/description.md` 内容)。None=尚未写入。
    pub(crate) description: Option<String>,
    /// 项目名称行内编辑态(None=未在编辑)。
    pub(crate) name_editing: Option<String>,
    /// 名称编辑框是否持有 iced 内部真实焦点,每帧由 `CaptureNameEditFocus`
    /// 写入。
    name_edit_focused: bool,
    /// 一次性标记:点项目名(`NameEditStart`)刚触发编辑时置真。
    pub(crate) name_edit_focus_pending: bool,
    /// 项目描述编辑态(None=未在编辑)。采用 iced 原生 `text_editor::Content`。
    pub(crate) description_editing: Option<iced_widget::text_editor::Content>,
    /// 描述编辑框是否持有 iced 内部真实焦点,每帧由 `CaptureDescriptionEditFocus`
    /// 写入(同 `name_edit_focused`,供产品原生输入路由闸门放行)。
    description_edit_focused: bool,
    /// 项目文档虚拟链接。
    pub(crate) links: links::LinksState,
    /// 已展开状态目录 → 其子项列表(就地展开/收起)。
    pub(crate) expanded_link_dirs: std::collections::HashMap<PathBuf, Vec<links::DirRow>>,
    /// 链接区(项目文档)当前选中项路径。单击文件(`OpenLink`)/
    /// 目录(`LinkDirToggle`)/展开子项、右击行(`LinkContextMenu`)都会选中,
    /// 用来整行高亮——参考文件树 `files::WorkspaceState::tree_selected`。
    /// `None`=无选中(新开项目默认)。
    pub(crate) selected_link: Option<PathBuf>,
    /// 共享记忆列表(不含正文),`RequestMemoriesRefresh` 拉取后写入。
    pub(crate) memories: Vec<dozer_model::MemoryInfo>,
    /// 待发提示;产品每条消息后统一排空成自己的 Toast。
    pub(crate) outbox: NoticeOutbox,
    /// 新建记忆的表单草稿(`None` = 表单未打开)。
    pub(crate) memory_create_draft: Option<MemoryDraft>,
    /// 当前打开的记忆详情(`None` = 未打开详情面板)。
    pub(crate) memory_detail: Option<dozer_model::MemoryDetail>,
    /// 详情页的编辑草稿(`None` = 只读展示态,未进入编辑)。
    pub(crate) memory_edit_draft: Option<MemoryDraft>,
    /// 待删除确认的记忆 id(`None` = 未在确认删除)。同 `delete_pending`
    /// 的既有先例(项目删除的二次确认),不用弹独立模态框。
    pub(crate) memory_delete_pending: Option<i64>,
    pub(crate) error: Option<LocalizedText>,
    pub(crate) scaffold_run: Option<ScaffoldRunState>,
    pub(crate) delete_pending: Option<delete::DeleteScope>,
}

impl WorkspaceState {
    pub fn memories(&self) -> &[dozer_model::MemoryInfo] {
        &self.memories
    }

    pub fn take_outbox(&mut self) -> Vec<Notice> {
        self.outbox.take()
    }

    /// 打开一个新项目时构造。`description`/`links` 由调用方在构造之前分别
    /// 调 `meta::load_description`/`links::load_or_discover` 拿到
    /// (同现有 `files::WorkspaceState::new(FileTree::new(..))` 那种"调用方
    /// 先算好再传入"的既有模式)。
    pub fn new(description: Option<String>, links: links::LinksState) -> Self {
        Self {
            description,
            links,
            ..Self::default()
        }
    }

    /// 项目级分支名(Project 面板/Agent 卡片工作区行共用读口)。
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    /// 项目级脏标(同上)。
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// 名称编辑框是否持有 iced 真实焦点(产品键盘路由用)。
    pub fn name_edit_focused(&self) -> bool {
        self.name_edit_focused
    }

    /// 每帧渲染循环读走 `CaptureNameEditFocus` 查到的真实焦点态后写进来。
    /// **只更新焦点镜像标记,不做提交判断**——落盘需要 `project_id`/服务束,
    /// 这个判断在产品侧做。
    pub fn set_name_edit_focused_flag(&mut self, focused: bool) {
        self.name_edit_focused = focused;
    }

    /// 读走(消费式)一次性聚焦标记。
    pub fn take_name_edit_focus_pending(&mut self) -> bool {
        std::mem::take(&mut self.name_edit_focus_pending)
    }

    /// 描述编辑框是否持有 iced 真实焦点(产品键盘路由用)。
    pub fn description_edit_focused(&self) -> bool {
        self.description_edit_focused
    }

    /// 每帧渲染循环读走 `CaptureDescriptionEditFocus` 查到的真实焦点态后写
    /// 进来(同 `set_name_edit_focused_flag`)。
    pub fn set_description_edit_focused_flag(&mut self, focused: bool) {
        self.description_edit_focused = focused;
    }

    /// 供产品内核 `project_preview_open_path`/`project_link_context_menu` 调用——
    /// 打开链接预览 / 打开删除右键菜单的同时把该路径标记为「选中」行(参考
    /// 文件树 `files::WorkspaceState::set_tree_selected`)。
    pub fn set_selected_link(&mut self, path: PathBuf) {
        self.selected_link = Some(path);
    }

    /// 供产品内核解析右击行的路径(按区 + 下标),
    /// 用于把该行标记为「选中」。返回 `None` 表示下标越界(菜单本就不该弹)。
    pub fn link_path_at(&self, target: links::LinkTarget, index: usize) -> Option<PathBuf> {
        self.links.list(target).get(index).map(|e| e.path.clone())
    }

    /// 供产品内核 `Workspace::blur_inputs` 调用——失焦时把当前编辑态直接写盘
    /// (描述保存不需要网络往返,不用等 `Message` 走一圈)。
    pub fn submit_description_edit_on_blur(&mut self, repo_path: &std::path::Path) {
        let Some(content) = self.description_editing.take() else {
            return;
        };
        let text = content.text().trim().to_string();
        if crate::meta::write_description(repo_path, &text).is_ok() {
            self.description = if text.is_empty() { None } else { Some(text) };
        }
        // 写失败这里不重试(失焦场景不适合弹错误态阻塞用户),下次进入面板
        // 仍能看到 `description` 字段的旧值,不会丢用户输入太久——这是已知
        // 的简化,写盘失败几率很低(权限问题会在其它写操作里更早暴露)。
    }
}

/// 名称编辑框真 `text_input` 的 `widget::Id`,供 `CaptureNameEditFocus` 匹配
/// 真实焦点态、产品程序化聚焦与键盘路由查询。
pub fn name_field_id() -> Id {
    Id::new("project-name-edit-box")
}

/// 描述编辑框真 `text_editor` 的 `widget::Id`。菜单复制/粘贴靠
/// `interface.operate(focus)` 聚焦到它,故描述框也挂稳定 id。
pub fn description_field_id() -> Id {
    Id::new("project-description-edit-box")
}

/// 名字 / 描述编辑框的右键菜单目标(产品把它转成自己的 `TextInputTarget`)。
pub fn name_edit_target() -> TextInputTarget {
    TextInputTarget {
        id: name_field_id(),
        secure: false,
    }
}

/// 描述编辑框的右键菜单目标。
pub fn description_edit_target() -> TextInputTarget {
    TextInputTarget {
        id: description_field_id(),
        secure: false,
    }
}

static NAME_EDIT_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式)上一帧捕获到的名称编辑框真 `text_input` 焦点态
/// (同 `extensions::files::take_search_focused` 的消费式复位手法,避免
/// 编辑框不可见的帧卡死上一次 `true` 永久堵死终端键盘转发)。
pub fn take_name_edit_focused() -> bool {
    std::mem::replace(&mut *NAME_EDIT_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍,把命中 `name_field_id` 的真
/// `text_input` 是否持有 iced 焦点写进 `NAME_EDIT_FOCUSED`。`traverse`
/// 必须调用传入闭包(见 [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureNameEditFocus;
impl Operation<()> for CaptureNameEditFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&name_field_id()) {
            *NAME_EDIT_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

static DESCRIPTION_EDIT_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式)上一帧捕获到的描述编辑框真 `text_editor` 焦点态
/// (同 `take_name_edit_focused`)。
pub fn take_description_edit_focused() -> bool {
    std::mem::replace(&mut *DESCRIPTION_EDIT_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍,把命中 `description_field_id` 的真
/// `text_editor` 是否持有 iced 焦点写进 `DESCRIPTION_EDIT_FOCUSED`(同
/// `CaptureNameEditFocus`)。此前描述编辑框缺这道每帧查焦点的机制,产品键盘
/// 路由闸门里查不到它的真实焦点态,导致键入被当作"未聚焦任何原生输入"
/// 转发进了 PTY 终端而不是交给 `text_editor` 自己——这个 Operation 补上
/// 这道缺失的信号源。
pub struct CaptureDescriptionEditFocus;
impl Operation<()> for CaptureDescriptionEditFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&description_field_id()) {
            *DESCRIPTION_EDIT_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

/// 面板头部带 hover 动画的"＋"图标按钮标识。本面板只有 `WorkspaceState`,
/// 不挂产品内核的 hover 动画表;`view` 的 hover 进度由调用方传入,进入/离开则
/// 以 `Message::ToolbarHover` 上报给产品(转发到它自己的 `HoverId`,同
/// `files::FilesToolbarTarget` 的既有先例)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectToolbarTarget {
    Docs,
    Remote,
    /// 「共享记忆」标题行"＋"按钮(新建记忆)。
    Memory,
}

/// 组合 git 刷新结果里跟 Project 有关的部分(`branch`/`dirty`/`remote_url`)、
/// daemon 改名结果。`GitRefreshed`/`NameRenamed` 由产品内核分发,带 `project_id`,
/// 走"按项目路由";其余是用户交互消息。
#[derive(Debug, Clone)]
pub enum Message {
    AsyncResult(bytehost_core::request::RequestToken, Box<Message>),
    /// 占位:非交互态图标按钮(如 Git 远程仓库"＋",功能未接入前)的
    /// `icon_button_entry` 仍需要一个具体的 `on_select` 值,但 `interactive`
    /// 为假时该值不会真的被 `on_press` 调用——这个变体只为满足类型签名。
    Noop,
    /// 面板头部"＋"按钮 hover 动画上报,见 [`ProjectToolbarTarget`] 文档。
    ToolbarHover(ProjectToolbarTarget, bool),
    GitRefreshed(i64, Option<String>, bool, Vec<String>),
    /// 磁盘占用统计结果(排除构建产物后的字节数)。
    DiskUsageLoaded(i64, u64),
    /// daemon 改名结果。带 `project_id`,走"按项目路由"。
    NameRenamed(i64, Result<dozer_model::ProjectInfo, Option<String>>),
    /// 点项目名进入编辑态(`name_edit_focus_pending` 置位)。
    NameEditStart,
    /// 名称编辑框草稿变化(iced `text_input::on_input`)。
    NameEditInput(String),
    /// 回车提交(与失焦提交共用 `submit_name_edit`)。
    NameEditSubmit,
    /// 名称 / 描述编辑框被右键:产品内核拦截,不进 `update`——转发成产品自己的
    /// 顶层"通用输入框右键菜单"消息(见产品侧文档)。
    TextInputMenuOpen(TextInputTarget),
    DescriptionEditStart,
    DescriptionEditAction(iced_widget::text_editor::Action),
    /// 预留:当前描述靠 `submit_description_edit_on_blur` 直接写盘(见该文档
    /// 注释),这个变体/`update` 分支留给以后可能加的显式"保存"按钮,现在还没
    /// 生产代码构造它,故 `#[allow(dead_code)]`。
    #[allow(dead_code)]
    DescriptionEditSubmit,
    LinkAdd {
        target: links::LinkTarget,
        path: PathBuf,
        kind: links::LinkKind,
    },
    LinkRemove {
        target: links::LinkTarget,
        index: usize,
    },
    LinkDirToggle {
        /// 保留在消息签名里(与 `LinkAdd`/`LinkRemove` 对齐);本期展开/收起
        /// 只按 `path` 操作,还没按 `target` 分流,故 `#[allow(dead_code)]`。
        #[allow(dead_code)]
        target: links::LinkTarget,
        path: PathBuf,
    },
    /// 行内右键:由产品内核拦截,把目标项(区 + 下标)记进产品级右键菜单浮层态,
    /// 渲染删除菜单,见 `files::Message::ContextMenuOpen` 文档同款写法。
    LinkContextMenu {
        target: links::LinkTarget,
        index: usize,
    },
    /// 产品内核拦截处理,见 `files::Message::OpenFile` 文档同款写法。
    OpenLink(PathBuf),
    /// 仅选中(不展开、不打开):用于「项目文档」里已展开目录的
    /// 子目录项——它们是只读单层展示,单击只高亮、不触发二次展开(展开已在
    /// 父级目录 `LinkDirToggle` 完成)。进 `update` 直接写 `selected_link`。
    LinkSelect {
        /// 保留在签名里与 `LinkDirToggle` 对齐;本期选中只按 `path`,未分流。
        #[allow(dead_code)]
        target: links::LinkTarget,
        path: PathBuf,
    },
    /// 单颗"＋"按钮:由产品内核 rfd 弹 OS 文件浏览器(根目录在项目根),选中的
    /// 文件/目录由内核判 `is_dir()` 定 `LinkKind`,再回送 `LinkAdd`。
    Pick(links::LinkTarget),
    /// footer-bar「修复项目」按钮:弹出逐步骤实时反馈弹窗,见
    /// [`crate::spawn_repair_run`]。
    RepairProject,
    /// footer-bar「删除项目」按钮:打开三选一确认弹窗,默认选中最轻层级。
    DeleteProjectRequest,
    /// 弹窗内切换单选层级。
    DeleteProjectScopeSelect(delete::DeleteScope),
    /// 弹窗"取消"。
    DeleteProjectCancel,
    /// 弹窗"确认删除"。**由产品内核拦截处理**(需要关掉当前项目 tab,
    /// 单个面板的 `update` 够不到跨 workspace 的操作,同 `LinkContextMenu`
    /// 的既有先例)——`update()` 里这个分支是 `unreachable!()`。
    DeleteProjectConfirm,
    /// 静默 scaffold 跑(打开项目 tab 时触发,不弹窗)完成。目前没有任何
    /// UI 需要消费这个结果——四个同步步骤 + 转录补录的副作用已经落地,这
    /// 条消息只是给 `update()` 一个"忽略"分支占位,不驱动任何状态。
    ScaffoldDone,
    /// "修复项目"弹窗:第 `idx` 个同步步骤(下标对应
    /// `ScaffoldRunState.steps`)进入 Running。
    ScaffoldStepStarted(i64, usize),
    /// 同上,携带该步骤终态。
    ScaffoldStepFinished(i64, usize, scaffold::ScaffoldStepResult),
    /// "agent 历史"转录补录步骤(固定是 `steps` 的最后一项)开始。
    TranscriptBackfillStarted(i64),
    /// 同上,携带终态。
    TranscriptBackfillFinished(i64, scaffold::ScaffoldStepResult),
    /// 补总结轮询到新的进度:携带 completed/total/succeeded/failed/skipped
    /// (新版 V2 批次计数;completed 是 succeeded+failed+skipped+cancelled 的
    /// 终态数)。completed 追平 total 时 update() 把 backfill 置为 Done。
    #[allow(clippy::type_complexity)]
    SummaryBackfillProgress(i64, u32, u32, u32, u32, u32),
    SummaryBackfillFailed(i64, LocalizedText),
    /// 弹窗"关闭"按钮(全部完成才可点)。
    ScaffoldPopupClose,
    /// 共享记忆列表拉取结果。
    MemoriesLoaded(Vec<dozer_model::MemoryInfo>),
    /// 记忆列表读取失败:**不改任何列表**。`Unavailable` 静默(产品顶栏徽标已表达"dozerd 不可用");
    /// 其余弹带 key 的 Error 提示,相同文本 60 秒内只提示一次(见 `NoticeOutbox::push_keyed_cooldown`)。
    MemoriesLoadFailed(dozer_services::ServiceErrorKind, String),
    /// 打开"新建记忆"表单。
    MemoryCreateStart,
    /// 新建表单四个字段的输入变化。
    MemoryCreateTitleInput(String),
    MemoryCreateKindInput(String),
    MemoryCreateDescriptionInput(String),
    MemoryCreateBodyInput(String),
    /// 提交新建表单。
    MemoryCreateSubmit,
    /// 取消新建。
    MemoryCreateCancel,
    /// 一次写操作(新建/编辑)完成后的结果——成功与否都触发一次列表刷新,
    /// 同 Todo 面板 `Message::Mutated` 的既有先例。
    MemoryMutated(Result<(), String>),
    /// 点列表行,打开详情(先设 loading 态,再异步拉正文)。
    MemoryDetailOpen(i64),
    /// 详情拉取完成。
    MemoryDetailLoaded(dozer_model::MemoryDetail),
    /// 读取记忆详情失败(打开一条记忆,或编辑成功后刷新详情)。沿用该面板
    /// 既有的内联 `error` 展示("保存记忆失败"同款)。
    MemoryDetailLoadFailed(String),
    /// 关闭详情面板。
    MemoryDetailClose,
    /// 进入编辑态(用当前详情内容预填草稿)。
    MemoryEditStart,
    MemoryEditTitleInput(String),
    MemoryEditKindInput(String),
    MemoryEditDescriptionInput(String),
    MemoryEditBodyInput(String),
    MemoryEditSubmit,
    MemoryEditCancel,
    /// 点删除按钮,进入二次确认态。
    MemoryDeleteRequest(i64),
    MemoryDeleteCancel,
    MemoryDeleteConfirm,
}

/// 把一次 `get_memory` 的结果变成要回给 `update` 的消息:成功 → `MemoryDetailLoaded`,
/// 失败 → `MemoryDetailLoadFailed`(而不是像过去那样在 `if let Ok(..)` 里静默丢掉)。
pub(crate) fn memory_detail_message<E: std::fmt::Display>(
    res: Result<dozer_model::MemoryDetail, E>,
) -> Message {
    match res {
        Ok(detail) => Message::MemoryDetailLoaded(detail),
        Err(e) => Message::MemoryDetailLoadFailed(e.to_string()),
    }
}

impl Message {
    /// 内层消息(剥掉任意层 `AsyncResult`)的载荷,测试用。
    #[cfg(test)]
    pub(crate) fn result_payload(self) -> Self {
        match self {
            Self::AsyncResult(_, inner) => inner.result_payload(),
            other => other,
        }
    }

    /// 这条消息携带的 `project_id`(只有装进 `AsyncResult` 的回包会带)。
    pub fn result_project(&self) -> Option<i64> {
        match self {
            Self::AsyncResult(token, _) => dozer_panel_host::project_of(token.scope()),
            _ => None,
        }
    }
}
