//! 项目信息(Project)面板:项目名、git 分支/脏标、验收次数。阶段 1 扩展化
//! 重构项目,设计见 `docs/superpowers/specs/2026-08-13-project-info-pane-v2-design.md`。

pub mod delete;
pub mod links;
pub mod scaffold;
pub mod update;
pub mod view;

pub use update::*;
pub use view::*;

use iced_widget::core::Rectangle;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use std::path::PathBuf;

/// 单个同步 scaffold 步骤(缓存目录/README/git 仓库/项目文档与 Agent
/// 记忆)在弹窗里的实时状态。`ScaffoldStepResult` 只有终态,这里补一层
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
}

/// "补总结"聚合进度行的状态。`Done` 不区分成功/失败——补总结内部每条都有
/// 自己的降级路径(headless 失败就走启发式,见 `dozerd` 侧),从这个面板的
/// 视角看永远是"处理完了 N/M 条",没有整体失败态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillStepState {
    Pending,
    Running(BackfillProgress),
    Done(BackfillProgress),
}

/// 一次"修复项目"弹窗跑的完整状态:4 个同步步骤 + 转录历史补录 + 补总结
/// 聚合进度。`steps` 里固定 5 项,顺序 = `scaffold::scaffold_steps()`
/// 的 4 项 + 追加的"agent 历史"转录补录(与既有 `spawn_scaffold_run` 的
/// 拼接顺序一致)。
#[derive(Debug, Clone, PartialEq)]
pub struct ScaffoldRunState {
    pub steps: Vec<(String, ScaffoldStepState)>,
    pub backfill: BackfillStepState,
}

impl ScaffoldRunState {
    /// 初始态:全部步骤 Pending,补总结也 Pending。
    fn pending() -> Self {
        let mut steps: Vec<(String, ScaffoldStepState)> = scaffold::scaffold_steps()
            .into_iter()
            .map(|s| (s.label.to_string(), ScaffoldStepState::Pending))
            .collect();
        steps.push(("agent 历史".to_string(), ScaffoldStepState::Pending));
        Self {
            steps,
            backfill: BackfillStepState::Pending,
        }
    }

    pub fn all_done(&self) -> bool {
        self.steps
            .iter()
            .all(|(_, s)| matches!(s, ScaffoldStepState::Done(_)))
            && matches!(self.backfill, BackfillStepState::Done(_))
    }
}

/// 挂在每个 Workspace 上的项目信息面板状态。
#[derive(Default)]
pub struct WorkspaceState {
    branch: Option<String>,
    dirty: bool,
    /// git remote 的 fetch URL 列表(`delivery::remote_url`)。空 = 无 remote/
    /// 非 git(面板据此显示"未设置")。
    remote_url: Vec<String>,
    /// 磁盘占用字节数(排除构建产物)。None=尚未算出来。
    disk_usage_bytes: Option<u64>,
    /// 项目描述(`.dozer/description.md` 内容)。None=尚未写入。
    description: Option<String>,
    /// 项目名称行内编辑态(None=未在编辑)。
    name_editing: Option<String>,
    /// 名称编辑框是否持有 iced 内部真实焦点,每帧由 `CaptureNameEditFocus`
    /// 写入。
    name_edit_focused: bool,
    /// 一次性标记:点项目名(`NameEditStart`)刚触发编辑时置真。
    name_edit_focus_pending: bool,
    /// 项目描述编辑态(None=未在编辑)。采用 iced 原生 `text_editor::Content`。
    description_editing: Option<iced_widget::text_editor::Content>,
    /// 描述编辑框是否持有 iced 内部真实焦点,每帧由 `CaptureDescriptionEditFocus`
    /// 写入(同 `name_edit_focused`,供 main.rs 原生输入路由闸门放行)。
    description_edit_focused: bool,
    /// 文档/Agent 记忆虚拟链接。
    links: links::LinksState,
    /// 已展开状态目录 → 其子项列表(就地展开/收起)。
    expanded_link_dirs: std::collections::HashMap<PathBuf, Vec<links::DirRow>>,
    /// 链接区(项目文档 / Agent 记忆)当前选中项路径。单击文件(`OpenLink`)/
    /// 目录(`LinkDirToggle`)/展开子项、右击行(`LinkContextMenu`)都会选中,
    /// 用来整行高亮——参考文件树 `files::WorkspaceState::tree_selected`。
    /// `None`=无选中(新开项目默认)。
    selected_link: Option<PathBuf>,
    error: Option<String>,
    pub(crate) scaffold_run: Option<ScaffoldRunState>,
    pub(crate) delete_pending: Option<delete::DeleteScope>,
}

impl WorkspaceState {
    /// 打开一个新项目时构造。`description`/`links` 由调用方在构造之前分别
    /// 调 `project_meta::load_description`/`links::load_or_discover` 拿到
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
    pub(crate) fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    /// 项目级脏标(同上)。
    pub(crate) fn dirty(&self) -> bool {
        self.dirty
    }

    /// 名称编辑框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn name_edit_focused(&self) -> bool {
        self.name_edit_focused
    }

    /// 每帧渲染循环读走 `CaptureNameEditFocus` 查到的真实焦点态后写进来。
    /// **只更新焦点镜像标记,不做提交判断**——落盘需要 `project_id`/
    /// `client`,`WorkspaceState` 自己拿不到,这个判断在
    /// `App::set_project_name_focused` 里做(见 Task 4)。
    pub fn set_name_edit_focused_flag(&mut self, focused: bool) {
        self.name_edit_focused = focused;
    }

    /// 读走(消费式)一次性聚焦标记。
    pub fn take_name_edit_focus_pending(&mut self) -> bool {
        std::mem::take(&mut self.name_edit_focus_pending)
    }

    /// 描述编辑框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn description_edit_focused(&self) -> bool {
        self.description_edit_focused
    }

    /// 每帧渲染循环读走 `CaptureDescriptionEditFocus` 查到的真实焦点态后写
    /// 进来(同 `set_name_edit_focused_flag`)。
    pub fn set_description_edit_focused_flag(&mut self, focused: bool) {
        self.description_edit_focused = focused;
    }

    /// 供内核 `project_preview_open_path`/`project_link_context_menu` 调用——
    /// 打开链接预览 / 打开删除右键菜单的同时把该路径标记为「选中」行(参考
    /// 文件树 `files::WorkspaceState::set_tree_selected`)。
    pub fn set_selected_link(&mut self, path: PathBuf) {
        self.selected_link = Some(path);
    }

    /// 供内核 `project_link_context_menu` 解析右击行的路径(按区 + 下标),
    /// 用于把该行标记为「选中」。返回 `None` 表示下标越界(菜单本就不该弹)。
    pub fn link_path_at(&self, target: links::LinkTarget, index: usize) -> Option<PathBuf> {
        self.links.list(target).get(index).map(|e| e.path.clone())
    }

    /// 供内核 `Workspace::blur_inputs` 调用——失焦时把当前编辑态直接写盘
    /// (描述保存不需要网络往返,不用等 `Message` 走一圈)。
    pub fn submit_description_edit_on_blur(&mut self, repo_path: &std::path::Path) {
        let Some(content) = self.description_editing.take() else {
            return;
        };
        let text = content.text().trim().to_string();
        if crate::project_meta::write_description(repo_path, &text).is_ok() {
            self.description = if text.is_empty() { None } else { Some(text) };
        }
        // 写失败这里不重试(失焦场景不适合弹错误态阻塞用户),下次进入面板
        // 仍能看到 `description` 字段的旧值,不会丢用户输入太久——这是已知
        // 的简化,写盘失败几率很低(权限问题会在其它写操作里更早暴露)。
    }
}

/// 名称编辑框真 `text_input` 的 `widget::Id`,供 `CaptureNameEditFocus` 匹配
/// 真实焦点态、main.rs 程序化聚焦与键盘路由查询。
pub fn name_field_id() -> Id {
    Id::new("project-name-edit-box")
}

/// 描述编辑框真 `text_editor` 的 `widget::Id`。菜单复制/粘贴靠
/// `interface.operate(focus)` 聚焦到它,故描述框也挂稳定 id。
pub fn description_field_id() -> Id {
    Id::new("project-description-edit-box")
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
/// `CaptureNameEditFocus`)。此前描述编辑框缺这道每帧查焦点的机制,main.rs
/// 键盘路由闸门里查不到它的真实焦点态,导致键入被当作"未聚焦任何原生输入"
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
/// 不挂内核 `App` 的 hover 动画表,`view` 通过调用方传入的 `ProjectPaneHover`
/// 取动画进度,进入/离开则以 `Message::ToolbarHover` 上报给内核(转发到
/// `HoverId`,同 `files::FilesToolbarTarget` 的既有先例)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectToolbarTarget {
    Docs,
    Memory,
    Remote,
}

/// 组合 git 刷新结果里跟 Project 有关的部分(`branch`/`dirty`/`remote_url`)、
/// daemon 改名结果。`GitRefreshed`/`NameRenamed` 由内核分发,带 `project_id`,
/// 走 `with_project`;其余是用户交互消息。
#[derive(Debug, Clone)]
pub enum Message {
    /// 占位:非交互态图标按钮(如 Git 远程仓库"＋",功能未接入前)的
    /// `icon_button_entry` 仍需要一个具体的 `on_select` 值,但 `interactive`
    /// 为假时该值不会真的被 `on_press` 调用——这个变体只为满足类型签名。
    Noop,
    /// 面板头部"＋"按钮 hover 动画上报,见 `ProjectToolbarTarget` 文档。
    ToolbarHover(ProjectToolbarTarget, bool),
    GitRefreshed(i64, Option<String>, bool, Vec<String>),
    /// 磁盘占用统计结果(排除构建产物后的字节数)。
    DiskUsageLoaded(i64, u64),
    /// daemon 改名结果。带 `project_id`,走 `with_project` 路由。
    NameRenamed(i64, Result<dozer_core::protocol::ProjectInfo, String>),
    /// 点项目名进入编辑态(`name_edit_focus_pending` 置位)。
    NameEditStart,
    /// 名称编辑框草稿变化(iced `text_input::on_input`)。
    NameEditInput(String),
    /// 回车提交(与失焦提交共用 `submit_name_edit`)。
    NameEditSubmit,
    /// 名称 / 描述编辑框被右键:内核拦截,不进 `update`——转发成顶层
    /// `Message::TextInputMenuOpen` 弹出通用输入框右键菜单(见 app.rs)。
    TextInputMenuOpen(crate::app::TextInputTarget),
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
    /// 行内右键:由内核拦截,把目标项(区 + 下标)记进 App 级右键菜单浮层态,
    /// 渲染删除菜单,见 `files::Message::ContextMenuOpen` 文档同款写法。
    LinkContextMenu {
        target: links::LinkTarget,
        index: usize,
    },
    /// 内核拦截处理,见 `files::Message::OpenFile` 文档同款写法。
    OpenLink(PathBuf),
    /// 仅选中(不展开、不打开):用于「项目文档 / Agent 记忆」里已展开目录的
    /// 子目录项——它们是只读单层展示,单击只高亮、不触发二次展开(展开已在
    /// 父级目录 `LinkDirToggle` 完成)。进 `update` 直接写 `selected_link`。
    LinkSelect {
        /// 保留在签名里与 `LinkDirToggle` 对齐;本期选中只按 `path`,未分流。
        #[allow(dead_code)]
        target: links::LinkTarget,
        path: PathBuf,
    },
    /// 单颗"＋"按钮:由内核 rfd 弹 OS 文件浏览器(根目录在项目根),选中的
    /// 文件/目录由内核判 `is_dir()` 定 `LinkKind`,再回送 `LinkAdd`。
    Pick(links::LinkTarget),
    /// footer-bar「修复项目」按钮:弹出逐步骤实时反馈弹窗,见
    /// `spawn_repair_run`。
    RepairProject,
    /// footer-bar「删除项目」按钮:打开三选一确认弹窗,默认选中最轻层级。
    DeleteProjectRequest,
    /// 弹窗内切换单选层级。
    DeleteProjectScopeSelect(delete::DeleteScope),
    /// 弹窗"取消"。
    DeleteProjectCancel,
    /// 弹窗"确认删除"。**由 `app.rs` 拦截处理**(需要关掉当前项目 tab，
    /// 单个 extension 的 `update` 够不到跨 `Workspace` 的操作，同
    /// `LinkContextMenu` 的既有先例)——`update()` 里这个分支是
    /// `unreachable!()`。
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
    /// 补总结轮询到新的 `(completed, total)`。`completed >= total` 时
    /// `update()` 把 `backfill` 置为 `Done`,否则 `Running`。
    SummaryBackfillProgress(i64, u32, u32),
    /// 弹窗"关闭"按钮(全部完成才可点)。
    ScaffoldPopupClose,
}

/// 磁盘占用统计的排除名单——跟 `crates/dozer-app/src/project.rs::HIDDEN`
/// (文件树"要不要显示这一行")语义不同,这里是"算不算项目真实内容",不复用
/// 那份常量。
pub const DISK_USAGE_EXCLUDE: [&str; 7] = [
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    ".venv",
    "__pycache__",
];

/// 递归求和 `root` 下所有文件大小,跳过名字命中 `exclude` 的目录(整个子树
/// 跳过,不下钻)。读不到的条目(权限/符号链接死链)跳过不计入,不中断整体
/// 计算。
pub fn dir_size_excluding(root: &std::path::Path, exclude: &[&str]) -> u64 {
    let Ok(rd) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if exclude.contains(&name.as_str()) {
                continue;
            }
            total += dir_size_excluding(&entry.path(), exclude);
        } else if let Ok(meta) = entry.metadata() {
            total += meta.len();
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_ws() -> WorkspaceState {
        WorkspaceState::default()
    }

    fn test_repo_path() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dozer-project-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn test_client() -> dozer_client::Client {
        dozer_client::Client::new(std::path::PathBuf::from("/tmp/dozer-project-test.sock"))
    }

    #[test]
    fn git_refreshed_updates_four_fields() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::GitRefreshed(
                1,
                Some("main".to_string()),
                true,
                vec!["https://x.git".to_string()],
            ),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.branch.as_deref(), Some("main"));
        assert!(ws.dirty);
        assert_eq!(ws.remote_url.as_slice(), ["https://x.git"]);
    }

    #[test]
    fn branch_and_dirty_accessors_read_current_state() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::GitRefreshed(1, Some("main".to_string()), true, vec![]),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.branch(), Some("main"));
        assert!(ws.dirty());
    }

    #[test]
    fn name_edit_start_prefills_current_name() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::NameEditStart,
            1,
            "旧名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.name_editing.as_deref(), Some("旧名字"));
    }

    #[test]
    fn name_edit_submit_same_name_closes_without_request() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::NameEditStart,
            1,
            "同名",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.name_edit_focus_pending, "点项目名应置一次性聚焦标记");
        update(
            &mut ws,
            Message::NameEditSubmit,
            1,
            "同名",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.name_editing.is_none());
        assert!(!ws.error.is_some(), "同名不改名不应报错");
    }

    #[test]
    fn name_renamed_ok_clears_editing_state() {
        let mut ws = new_ws();
        ws.name_editing = Some("新名字".into());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let project = dozer_core::protocol::ProjectInfo {
            id: 1,
            path: "/repo".into(),
            name: "新名字".into(),
            last_active_ms: 0,
            created_ms: 0,
            updated_ms: 0,
        };
        update(
            &mut ws,
            Message::NameRenamed(1, Ok(project)),
            1,
            "旧名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.name_editing.is_none());
        assert!(ws.error.is_none());
    }

    #[test]
    fn name_renamed_err_keeps_editing_state_and_sets_error() {
        let mut ws = new_ws();
        ws.name_editing = Some("新名字".into());
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::NameRenamed(1, Err("连接失败".into())),
            1,
            "旧名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.name_editing.as_deref(), Some("新名字"));
        assert!(ws.error.is_some());
    }

    #[test]
    fn dir_size_excluding_sums_files_and_skips_excluded_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "12345").unwrap(); // 5 bytes
        std::fs::create_dir(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target").join("big.bin"), vec![0u8; 1000]).unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src").join("b.txt"), "12").unwrap(); // 2 bytes
        let size = dir_size_excluding(dir.path(), &DISK_USAGE_EXCLUDE);
        assert_eq!(size, 7); // 5 + 2, target 整个跳过
    }

    #[test]
    fn dir_size_excluding_empty_dir_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(dir_size_excluding(dir.path(), &DISK_USAGE_EXCLUDE), 0);
    }

    #[test]
    fn disk_usage_loaded_sets_field() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::DiskUsageLoaded(1, 12345),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.disk_usage_bytes, Some(12345));
    }

    #[test]
    fn description_edit_start_prefills_from_field() {
        let mut ws = new_ws();
        ws.description = Some("已有描述".to_string());
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::DescriptionEditStart,
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        let content = ws.description_editing.expect("进入编辑态");
        assert_eq!(content.text(), "已有描述");
    }

    #[test]
    fn description_edit_submit_writes_to_disk_and_clears_editing() {
        let mut ws = new_ws();
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let action = iced_widget::text_editor::Action::Edit(iced_widget::text_editor::Edit::Paste(
            std::sync::Arc::new("新描述内容".to_string()),
        ));
        update(
            &mut ws,
            Message::DescriptionEditStart,
            1,
            "名字",
            dir.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        update(
            &mut ws,
            Message::DescriptionEditAction(action),
            1,
            "名字",
            dir.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        update(
            &mut ws,
            Message::DescriptionEditSubmit,
            1,
            "名字",
            dir.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.description_editing.is_none());
        assert_eq!(ws.description.as_deref(), Some("新描述内容"));
        let on_disk = crate::project_meta::load_description(dir.path());
        assert_eq!(on_disk.as_deref(), Some("新描述内容"));
    }

    #[test]
    fn description_edit_submit_empty_clears_field_and_file() {
        let mut ws = new_ws();
        let dir = tempfile::tempdir().unwrap();
        crate::project_meta::write_description(dir.path(), "旧描述").unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::DescriptionEditStart,
            1,
            "名字",
            dir.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        // 清空编辑内容:直接提交空编辑器。
        update(
            &mut ws,
            Message::DescriptionEditSubmit,
            1,
            "名字",
            dir.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.description_editing.is_none());
        assert!(ws.description.is_none());
        assert!(crate::project_meta::load_description(dir.path()).is_none());
    }

    #[test]
    fn link_add_appends_and_writes_disk() {
        let mut ws = new_ws();
        let repo = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::LinkAdd {
                target: links::LinkTarget::Docs,
                path: PathBuf::from("/repo/README.md"),
                kind: links::LinkKind::File,
            },
            1,
            "名字",
            repo.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.links.docs.len(), 1);
        let loaded = links::load(repo.path()).unwrap();
        assert_eq!(loaded.docs.len(), 1);
    }

    #[test]
    fn link_add_dedupes_existing_path() {
        let mut ws = new_ws();
        ws.links.docs.push(links::LinkEntry {
            path: PathBuf::from("/repo/README.md"),
            kind: links::LinkKind::File,
        });
        let repo = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::LinkAdd {
                target: links::LinkTarget::Docs,
                path: PathBuf::from("/repo/README.md"),
                kind: links::LinkKind::File,
            },
            1,
            "名字",
            repo.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.links.docs.len(), 1);
    }

    #[test]
    fn link_remove_deletes_and_writes_disk() {
        let mut ws = new_ws();
        ws.links.memory.push(links::LinkEntry {
            path: PathBuf::from("/home/.claude/memory"),
            kind: links::LinkKind::Dir,
        });
        let repo = tempfile::tempdir().unwrap();
        links::save(repo.path(), &ws.links).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::LinkRemove {
                target: links::LinkTarget::Memory,
                index: 0,
            },
            1,
            "名字",
            repo.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.links.memory.is_empty());
        assert!(links::load(repo.path()).unwrap().memory.is_empty());
    }

    #[test]
    fn link_dir_toggle_expands_then_collapses() {
        let mut ws = new_ws();
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join("docs")).unwrap();
        std::fs::write(repo.path().join("docs").join("a.md"), "").unwrap();
        let docs_path = repo.path().join("docs");
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::LinkDirToggle {
                target: links::LinkTarget::Docs,
                path: docs_path.clone(),
            },
            1,
            "名字",
            repo.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(
            ws.expanded_link_dirs.get(&docs_path).map(|r| r.len()),
            Some(1)
        );
        update(
            &mut ws,
            Message::LinkDirToggle {
                target: links::LinkTarget::Docs,
                path: docs_path.clone(),
            },
            1,
            "名字",
            repo.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(!ws.expanded_link_dirs.contains_key(&docs_path));
    }

    #[test]
    fn link_dir_toggle_selects_the_entry() {
        let mut ws = new_ws();
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join("docs")).unwrap();
        let docs_path = repo.path().join("docs");
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::LinkDirToggle {
                target: links::LinkTarget::Docs,
                path: docs_path.clone(),
            },
            1,
            "名字",
            repo.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.selected_link.as_deref(), Some(docs_path.as_path()));
    }

    #[test]
    fn link_select_only_marks_selection_without_expanding() {
        let mut ws = new_ws();
        let repo = tempfile::tempdir().unwrap();
        let child = repo.path().join("docs").join("sub");
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::LinkSelect {
                target: links::LinkTarget::Docs,
                path: child.clone(),
            },
            1,
            "名字",
            repo.path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.selected_link.as_deref(), Some(child.as_path()));
        assert!(ws.expanded_link_dirs.is_empty());
    }

    #[test]
    fn scaffold_done_is_a_pure_noop() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::ScaffoldDone,
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.scaffold_run.is_none());
    }

    #[test]
    fn repair_project_sets_pending_scaffold_run() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::RepairProject,
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        let run = ws.scaffold_run.expect("弹窗状态应已初始化");
        assert_eq!(run.steps.len(), 5);
        assert!(
            run.steps
                .iter()
                .all(|(_, s)| matches!(s, ScaffoldStepState::Pending))
        );
        assert_eq!(run.backfill, BackfillStepState::Pending);
    }

    #[test]
    fn scaffold_step_started_then_finished_updates_that_step_only() {
        let mut ws = new_ws();
        ws.scaffold_run = Some(ScaffoldRunState::pending());
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::ScaffoldStepStarted(1, 1),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        let run = ws.scaffold_run.as_ref().unwrap();
        assert_eq!(run.steps[0].1, ScaffoldStepState::Pending);
        assert_eq!(run.steps[1].1, ScaffoldStepState::Running);

        update(
            &mut ws,
            Message::ScaffoldStepFinished(1, 1, scaffold::ScaffoldStepResult::AlreadyOk),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        let run = ws.scaffold_run.as_ref().unwrap();
        assert_eq!(
            run.steps[1].1,
            ScaffoldStepState::Done(scaffold::ScaffoldStepResult::AlreadyOk)
        );
    }

    #[test]
    fn transcript_backfill_started_then_finished_updates_last_step_only() {
        let mut ws = new_ws();
        ws.scaffold_run = Some(ScaffoldRunState::pending());
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::TranscriptBackfillStarted(1),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        let run = ws.scaffold_run.as_ref().unwrap();
        let last = run.steps.len() - 1;
        assert_eq!(run.steps[last].1, ScaffoldStepState::Running);
        assert_eq!(run.steps[0].1, ScaffoldStepState::Pending);

        update(
            &mut ws,
            Message::TranscriptBackfillFinished(
                1,
                scaffold::ScaffoldStepResult::Created("导入 3 个历史文件".into()),
            ),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        let run = ws.scaffold_run.as_ref().unwrap();
        assert_eq!(
            run.steps[last].1,
            ScaffoldStepState::Done(scaffold::ScaffoldStepResult::Created(
                "导入 3 个历史文件".into()
            ))
        );
    }

    #[test]
    fn summary_backfill_progress_marks_done_when_completed_reaches_total() {
        let mut ws = new_ws();
        ws.scaffold_run = Some(ScaffoldRunState::pending());
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::SummaryBackfillProgress(1, 2, 5),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(
            ws.scaffold_run.as_ref().unwrap().backfill,
            BackfillStepState::Running(BackfillProgress {
                completed: 2,
                total: 5
            })
        );

        update(
            &mut ws,
            Message::SummaryBackfillProgress(1, 5, 5),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(
            ws.scaffold_run.as_ref().unwrap().backfill,
            BackfillStepState::Done(BackfillProgress {
                completed: 5,
                total: 5
            })
        );
    }

    #[test]
    fn summary_backfill_progress_of_zero_total_is_immediately_done() {
        let mut ws = new_ws();
        ws.scaffold_run = Some(ScaffoldRunState::pending());
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::SummaryBackfillProgress(1, 0, 0),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(
            ws.scaffold_run.as_ref().unwrap().backfill,
            BackfillStepState::Done(BackfillProgress {
                completed: 0,
                total: 0
            })
        );
    }

    #[test]
    fn popup_close_only_clears_state_when_all_done() {
        let mut ws = new_ws();
        ws.scaffold_run = Some(ScaffoldRunState::pending());
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::ScaffoldPopupClose,
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.scaffold_run.is_some(), "未完成时点关闭不应清空状态");
    }

    #[test]
    fn popup_close_clears_state_once_all_steps_and_backfill_are_done() {
        let mut ws = new_ws();
        let mut run = ScaffoldRunState::pending();
        for (_, state) in run.steps.iter_mut() {
            *state = ScaffoldStepState::Done(scaffold::ScaffoldStepResult::AlreadyOk);
        }
        run.backfill = BackfillStepState::Done(BackfillProgress {
            completed: 0,
            total: 0,
        });
        ws.scaffold_run = Some(run);
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::ScaffoldPopupClose,
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert!(ws.scaffold_run.is_none());
    }

    #[test]
    fn delete_project_request_then_scope_select_then_cancel() {
        let mut ws_state = WorkspaceState::new(None, links::LinksState::default());
        let noop_client = dozer_client::Client::new(std::path::PathBuf::from("/tmp/dozer.sock"));
        let handle = tokio::runtime::Handle::try_current()
            .unwrap_or_else(|_| tokio::runtime::Runtime::new().unwrap().handle().clone());
        let repo = std::path::Path::new("/tmp/demo");

        update(
            &mut ws_state,
            Message::DeleteProjectRequest,
            1,
            "demo",
            repo,
            &noop_client,
            &handle,
            |_| {},
        );
        assert_eq!(
            ws_state.delete_pending,
            Some(delete::DeleteScope::DozerOnly)
        );

        update(
            &mut ws_state,
            Message::DeleteProjectScopeSelect(delete::DeleteScope::WithProjectFiles),
            1,
            "demo",
            repo,
            &noop_client,
            &handle,
            |_| {},
        );
        assert_eq!(
            ws_state.delete_pending,
            Some(delete::DeleteScope::WithProjectFiles)
        );

        update(
            &mut ws_state,
            Message::DeleteProjectCancel,
            1,
            "demo",
            repo,
            &noop_client,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.delete_pending, None);
    }
}
