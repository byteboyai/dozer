//! 项目信息(Project)面板:项目名、git 分支/脏标、验收次数。阶段 1 扩展化
//! 重构项目,设计见 `docs/superpowers/specs/2026-08-13-project-info-pane-v2-design.md`。

pub mod delete;
pub mod links;

use byteui::interaction::icons;
use dozer_core::protocol::ProjectInfo;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Border, Element, Length, Rectangle};
use iced_widget::{MouseArea, button, column, container, row, stack, text};
use std::path::PathBuf;

use crate::project_scaffold;

/// 单个同步 scaffold 步骤(缓存目录/README/git 仓库/项目文档与 Agent
/// 记忆)在弹窗里的实时状态。`ScaffoldStepResult` 只有终态,这里补一层
/// pending/running。
#[derive(Debug, Clone, PartialEq)]
pub enum ScaffoldStepState {
    Pending,
    Running,
    Done(project_scaffold::ScaffoldStepResult),
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
/// 聚合进度。`steps` 里固定 5 项,顺序 = `project_scaffold::scaffold_steps()`
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
        let mut steps: Vec<(String, ScaffoldStepState)> = project_scaffold::scaffold_steps()
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
    project_acceptance_count: Option<u64>,
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

/// 读走(非消费)上一帧捕获到的名称编辑框真 `text_input` 焦点态。
pub fn take_name_edit_focused() -> bool {
    *NAME_EDIT_FOCUSED.lock().unwrap()
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

/// 组合 git 刷新结果里跟 Project 有关的部分(`branch`/`dirty`/`remote_url`)、
/// 验收次数、daemon 改名结果。`GitRefreshed`/`AcceptanceCountLoaded`/
/// `NameRenamed` 由内核分发,带 `project_id`,走 `with_project`;其余是用户
/// 交互消息。
#[derive(Debug, Clone)]
pub enum Message {
    GitRefreshed(i64, Option<String>, bool, Vec<String>),
    AcceptanceCountLoaded(i64, Option<u64>),
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
    ScaffoldStepFinished(i64, usize, project_scaffold::ScaffoldStepResult),
    /// "agent 历史"转录补录步骤(固定是 `steps` 的最后一项)开始。
    TranscriptBackfillStarted(i64),
    /// 同上,携带终态。
    TranscriptBackfillFinished(i64, project_scaffold::ScaffoldStepResult),
    /// 补总结轮询到新的 `(completed, total)`。`completed >= total` 时
    /// `update()` 把 `backfill` 置为 `Done`,否则 `Running`。
    SummaryBackfillProgress(i64, u32, u32),
    /// 弹窗"关闭"按钮(全部完成才可点)。
    ScaffoldPopupClose,
}

/// 处理全部消息——本模块不触碰终端会话域,没有需要内核拦截、`update` 里
/// `unreachable!` 的消息(不像 Files/Acceptance);改名需要 daemon 往返,走
/// `handle`/`emit`。
#[allow(clippy::too_many_arguments)]
pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    current_name: &str,
    repo_path: &std::path::Path,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::GitRefreshed(_, branch, dirty, remote_url) => {
            ws_state.branch = branch;
            ws_state.dirty = dirty;
            ws_state.remote_url = remote_url;
        }
        Message::AcceptanceCountLoaded(_, n) => {
            ws_state.project_acceptance_count = n;
        }
        Message::DiskUsageLoaded(_, bytes) => {
            ws_state.disk_usage_bytes = Some(bytes);
        }
        Message::NameEditStart => {
            ws_state.name_editing = Some(current_name.to_string());
            ws_state.name_edit_focus_pending = true;
        }
        Message::NameEditInput(s) => {
            if let Some(buf) = &mut ws_state.name_editing {
                *buf = s;
            }
        }
        Message::NameEditSubmit => {
            submit_name_edit(
                ws_state,
                project_id,
                current_name,
                client.clone(),
                handle,
                emit,
            );
        }
        Message::NameRenamed(_, result) => match result {
            Ok(_) => {
                ws_state.name_editing = None;
                ws_state.error = None;
            }
            Err(e) => {
                ws_state.error = Some(format!("改名失败: {e}"));
                // 保留编辑态原始输入,允许重试。
            }
        },
        Message::DescriptionEditStart => {
            let initial = ws_state.description.clone().unwrap_or_default();
            ws_state.description_editing =
                Some(iced_widget::text_editor::Content::with_text(&initial));
        }
        Message::TextInputMenuOpen(_) => {
            unreachable!("由内核拦截处理,见 project::Message::TextInputMenuOpen 文档")
        }
        Message::DescriptionEditAction(action) => {
            if let Some(content) = &mut ws_state.description_editing {
                content.perform(action);
            }
        }
        Message::DescriptionEditSubmit => {
            let Some(content) = ws_state.description_editing.take() else {
                return;
            };
            let text = content.text().trim().to_string();
            match crate::project_meta::write_description(repo_path, &text) {
                Ok(()) => {
                    ws_state.description = if text.is_empty() { None } else { Some(text) };
                    ws_state.error = None;
                }
                Err(e) => {
                    ws_state.error = Some(format!("保存失败: {e}"));
                    ws_state.description_editing = Some(content); // 保留编辑态允许重试
                }
            }
        }
        Message::LinkAdd { target, path, kind } => {
            let already_present = ws_state.links.list(target).iter().any(|e| e.path == path);
            if !already_present {
                ws_state
                    .links
                    .list_mut(target)
                    .push(links::LinkEntry { path, kind });
                match links::save(repo_path, &ws_state.links) {
                    Ok(()) => ws_state.error = None,
                    Err(e) => {
                        ws_state.links.list_mut(target).pop();
                        ws_state.error = Some(format!("保存失败: {e}"));
                    }
                }
            }
        }
        Message::LinkRemove { target, index } => {
            let list = ws_state.links.list_mut(target);
            if index >= list.len() {
                return;
            }
            let removed = list.remove(index);
            // 记进 dismissed:否则文件还在磁盘上的话,"修复项目"的
            // `merge_rediscovered` 下次会把这条自动发现的记录重新加回来,
            // 删除操作就形同虚设(见 `links::LinksState.dismissed` 文档)。
            ws_state.links.dismissed.push(removed.path.clone());
            if let Err(e) = links::save(repo_path, &ws_state.links) {
                ws_state.links.dismissed.pop();
                ws_state.links.list_mut(target).insert(index, removed);
                ws_state.error = Some(format!("保存失败: {e}"));
            } else {
                ws_state.error = None;
            }
        }
        Message::LinkDirToggle { path, .. } => {
            ws_state.selected_link = Some(path.clone());
            if ws_state.expanded_link_dirs.remove(&path).is_none() {
                let rows = links::read_dir_row(&path);
                ws_state.expanded_link_dirs.insert(path, rows);
            }
        }
        Message::LinkSelect { path, .. } => {
            ws_state.selected_link = Some(path);
        }
        Message::OpenLink(_) => {
            unreachable!("由内核拦截处理,见 files::Message::OpenFile 文档")
        }
        Message::Pick(_) => {
            unreachable!("由内核拦截处理,见 files::Message::OpenFile 文档")
        }
        Message::LinkContextMenu { .. } => {
            unreachable!("由内核拦截处理,见 files::Message::ContextMenuOpen 文档")
        }
        Message::RepairProject => {
            ws_state.scaffold_run = Some(ScaffoldRunState::pending());
            spawn_repair_run(
                repo_path.to_path_buf(),
                project_id,
                client.clone(),
                handle,
                emit,
            );
        }
        Message::DeleteProjectRequest => {
            ws_state.delete_pending = Some(delete::DeleteScope::DozerOnly);
        }
        Message::DeleteProjectScopeSelect(scope) => {
            ws_state.delete_pending = Some(scope);
        }
        Message::DeleteProjectCancel => {
            ws_state.delete_pending = None;
        }
        Message::DeleteProjectConfirm => {
            unreachable!("由内核拦截处理,见 App::project_delete_confirm 文档")
        }
        Message::ScaffoldDone => {}
        Message::ScaffoldStepStarted(_, idx) => {
            if let Some(run) = &mut ws_state.scaffold_run
                && let Some((_, state)) = run.steps.get_mut(idx)
            {
                *state = ScaffoldStepState::Running;
            }
        }
        Message::ScaffoldStepFinished(_, idx, result) => {
            if let Some(run) = &mut ws_state.scaffold_run
                && let Some((_, state)) = run.steps.get_mut(idx)
            {
                *state = ScaffoldStepState::Done(result);
            }
        }
        Message::TranscriptBackfillStarted(_) => {
            if let Some(run) = &mut ws_state.scaffold_run {
                let last = run.steps.len() - 1;
                if let Some((_, state)) = run.steps.get_mut(last) {
                    *state = ScaffoldStepState::Running;
                }
            }
        }
        Message::TranscriptBackfillFinished(_, result) => {
            if let Some(run) = &mut ws_state.scaffold_run {
                let last = run.steps.len() - 1;
                if let Some((_, state)) = run.steps.get_mut(last) {
                    *state = ScaffoldStepState::Done(result);
                }
            }
        }
        Message::SummaryBackfillProgress(_, completed, total) => {
            if let Some(run) = &mut ws_state.scaffold_run {
                let progress = BackfillProgress { completed, total };
                run.backfill = if completed >= total {
                    BackfillStepState::Done(progress)
                } else {
                    BackfillStepState::Running(progress)
                };
            }
        }
        Message::ScaffoldPopupClose => {
            if matches!(&ws_state.scaffold_run, Some(run) if run.all_done()) {
                ws_state.scaffold_run = None;
            }
        }
    }
}

/// 静默 scaffold 跑(项目 tab 打开时触发,`app.rs::project_tab_opened`
/// 唯一调用点,不弹窗)。跑完四个同步步骤 + 转录历史补录,不产出任何
/// UI 可见结果——`emit(Message::ScaffoldDone)` 只是让调用方知道这批
/// spawn 任务已经跑完(目前没有消费方,`update()` 是空分支),不携带内容。
/// **不触发补总结**:补总结只在显式点击"修复项目"时跑(spec
/// 2026-08-28,避免每次静默打开项目都真实拉起 agent 进程)。
pub fn spawn_scaffold_run(
    repo_path: std::path::PathBuf,
    client: dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let cwd = repo_path.to_string_lossy().into_owned();
    handle.spawn(async move {
        let repo_path2 = repo_path.clone();
        let _ = tokio::task::spawn_blocking(move || project_scaffold::run_sync_steps(&repo_path2))
            .await;
        let _ = client.backfill_project_transcripts(&cwd).await;
        emit(Message::ScaffoldDone);
    });
}

const SUMMARY_BACKFILL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// "修复项目"按钮触发的完整跑法:4 个同步步骤逐个 Started/Finished、
/// 转录历史补录 Started/Finished、再触发补总结并轮询进度,全部实时
/// `emit` 给弹窗(spec 2026-08-28)。所有消息都携带 `project_id`,靠
/// `app.rs` 里按 `project_id` 查找 workspace 的路由分支落地(不依赖
/// "当前激活哪个 tab"),避免用户在补总结进行中切换项目 tab 时消息投递到
/// 错误的 workspace。
pub fn spawn_repair_run(
    repo_path: std::path::PathBuf,
    project_id: i64,
    client: dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let cwd = repo_path.to_string_lossy().into_owned();
    handle.spawn(async move {
        let steps = project_scaffold::scaffold_steps();
        for (idx, step) in steps.into_iter().enumerate() {
            emit(Message::ScaffoldStepStarted(project_id, idx));
            let repo_path3 = repo_path.clone();
            let result = tokio::task::spawn_blocking(move || (step.run)(&repo_path3))
                .await
                .unwrap_or_else(|e| {
                    project_scaffold::ScaffoldStepResult::Failed(format!("内部错误: {e}"))
                });
            emit(Message::ScaffoldStepFinished(project_id, idx, result));
        }

        emit(Message::TranscriptBackfillStarted(project_id));
        let backfill_result = match client.backfill_project_transcripts(&cwd).await {
            Ok(0) => project_scaffold::ScaffoldStepResult::AlreadyOk,
            Ok(n) => project_scaffold::ScaffoldStepResult::Created(format!("导入 {n} 个历史文件")),
            Err(e) => project_scaffold::ScaffoldStepResult::Failed(e.to_string()),
        };
        emit(Message::TranscriptBackfillFinished(
            project_id,
            backfill_result,
        ));

        if let Err(e) = client.backfill_session_summaries(&cwd).await {
            tracing::warn!(error = %e, "补总结请求发送失败,视为无需补");
            emit(Message::SummaryBackfillProgress(project_id, 0, 0));
            return;
        }
        loop {
            tokio::time::sleep(SUMMARY_BACKFILL_POLL_INTERVAL).await;
            match client.get_session_summary_backfill_status(&cwd).await {
                Ok((completed, total)) => {
                    emit(Message::SummaryBackfillProgress(
                        project_id, completed, total,
                    ));
                    if completed >= total {
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "查询补总结进度失败,停止轮询");
                    break;
                }
            }
        }
    });
}

/// 项目名称编辑的共享提交逻辑:回车提交(`NameEditSubmit`)与失焦提交
/// (`App::set_project_name_focused` 的边缘触发)两条路径共用,避免两份
/// 重复的 `client.rename_project` 调用(现状历史遗留,这次一并合并)。
/// 空名字/未改动直接退出编辑态,不发请求。
pub fn submit_name_edit(
    ws_state: &mut WorkspaceState,
    project_id: i64,
    current_name: &str,
    client: dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let Some(raw) = ws_state.name_editing.take() else {
        return;
    };
    let name = raw.trim().to_string();
    if name.is_empty() || name == current_name {
        return;
    }
    handle.spawn(async move {
        let result = client
            .rename_project(project_id, &name)
            .await
            .map_err(|e| e.to_string())
            .and_then(|opt| opt.ok_or_else(|| "项目不存在".to_string()));
        emit(Message::NameRenamed(project_id, result));
    });
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

/// 面板主入口(单栏,不与任何其它面板配对——同 GitLog/Usage)。`project` 为
/// `None` 时内核不会真正走到这里(`left_panel_area` 对 `PanelKind::Project`
/// 无条件调用本函数,但 `App::view()` 顶层只在有聚焦项目时才会渲染到这个
/// 分支),这里仍保留一次防御性判断,风格对齐 Files 试点。
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    project: Option<&'a ProjectInfo>,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(p) = project else {
        return container(iced_widget::Space::new())
            .width(width)
            .height(Length::Fill)
            .into();
    };

    let mut content = column![]
        .spacing(12)
        .padding(8)
        .width(Length::Fill)
        .height(Length::Fill);

    content = content.push(crate::homespace::home_panel_head(
        icons::IconKind::Briefcase,
        "项目",
    ));

    let editing = ws_state.name_edit_focused();
    let name_row: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if ws_state.name_editing.is_some() {
            container(byteui::interaction::context_menu::wrap(
                byteui::form::input_text::view(
                    "",
                    ws_state.name_editing.as_deref().unwrap_or(""),
                    false,
                    Some(name_field_id()),
                    false,
                    Some(Message::NameEditSubmit),
                    true,
                    Message::NameEditInput,
                ),
                Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
                    id: name_field_id(),
                    secure: false,
                })),
            ))
            .padding([8, 12])
            .width(Length::Fill)
            .style(
                move |_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: Some(byteui::theme::color::current().card.into()),
                    border: Border {
                        color: if editing {
                            byteui::theme::color::current().gold
                        } else {
                            byteui::theme::color::current().border
                        },
                        width: 1.5,
                        radius: 8.0.into(),
                    },
                    ..iced_widget::container::Style::default()
                },
            )
            .into()
        } else {
            button(
                text(p.name.clone())
                    .size(byteui::theme::font::title())
                    .color(byteui::theme::color::current().cream),
            )
            .on_press(Message::NameEditStart)
            .style(|_t, _s| iced_widget::button::Style {
                background: None,
                text_color: byteui::theme::color::current().cream,
                ..iced_widget::button::Style::default()
            })
            .into()
        };
    content = content.push(name_row);

    let description_block: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if let Some(editing) = &ws_state.description_editing {
            let editor: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
                iced_widget::text_editor(editing)
                    .id(description_field_id())
                    .placeholder("项目描述信息…")
                    .on_action(Message::DescriptionEditAction)
                    .height(Length::Fixed(96.0))
                    .style(|_t, _s| iced_widget::text_editor::Style {
                        background: byteui::theme::color::current().card.into(),
                        border: Border {
                            color: byteui::theme::color::current().gold,
                            width: 1.5,
                            radius: 8.0.into(),
                        },
                        placeholder: byteui::theme::color::current().dim,
                        value: byteui::theme::color::current().cream,
                        selection: byteui::theme::color::current().gold,
                    })
                    .into();
            byteui::interaction::context_menu::wrap(
                editor,
                Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
                    id: description_field_id(),
                    secure: false,
                })),
            )
        } else {
            let label = ws_state
                .description
                .clone()
                .unwrap_or_else(|| "点击添加项目描述…".to_string());
            let color = if ws_state.description.is_some() {
                byteui::theme::color::current().body
            } else {
                byteui::theme::color::current().dim
            };
            button(text(label).size(byteui::theme::font::body()).color(color))
                .on_press(Message::DescriptionEditStart)
                .padding([10, 12])
                .width(Length::Fill)
                .style(|_t, _s| iced_widget::button::Style {
                    background: Some(byteui::theme::color::current().desc_bg.into()),
                    border: Border {
                        radius: 8.0.into(),
                        ..Default::default()
                    },
                    text_color: byteui::theme::color::current().body,
                    ..iced_widget::button::Style::default()
                })
                .into()
        };
    content = content.push(description_block);

    if let Some(n) = ws_state.project_acceptance_count.filter(|n| *n > 0) {
        content = content.push(
            text(format!("{n} 次验收"))
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().gold),
        );
    }

    let usage_label = ws_state
        .disk_usage_bytes
        .map(|b| format!("文件存储 ({} MB)", b / 1_000_000))
        .unwrap_or_else(|| "文件存储".to_string());
    content = content.push(
        row![
            icons::view(
                icons::IconKind::CircleSmall,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().cream
            ),
            text(usage_label)
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().cream),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    );

    // 「项目文档」下的文件树项都包在 iced button 里,button 默认左内边距 10px;
    // 为与之左对齐,标签行统一左缩 10px,值文本缩进到与文件树文件名同列。
    let tree_indent = 10.0;
    let value_indent = tree_indent + byteui::theme::icon_size::row() + 6.0;
    content = content.push(
        container(
            row![
                icons::view(
                    icons::IconKind::FolderDot,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().dim
                ),
                text("项目根目录")
                    .size(byteui::theme::font::label())
                    .color(byteui::theme::color::current().dim),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .padding(iced_widget::core::Padding::new(0.0).left(tree_indent)),
    );
    content = content.push(
        row![
            iced_widget::Space::new().width(Length::Fixed(value_indent)),
            text(shorten_path(&p.path))
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().body),
        ]
        .align_y(iced_widget::core::Alignment::Center),
    );
    content = content.push(
        container(
            row![
                icons::view(
                    icons::IconKind::FolderRoot,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().dim
                ),
                text("Git 远程仓库")
                    .size(byteui::theme::font::label())
                    .color(byteui::theme::color::current().dim),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .padding(iced_widget::core::Padding::new(0.0).left(tree_indent)),
    );
    if ws_state.remote_url.is_empty() {
        content = content.push(
            row![
                iced_widget::Space::new().width(Length::Fixed(value_indent)),
                text("未设置")
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().dim),
            ]
            .align_y(iced_widget::core::Alignment::Center),
        );
    } else {
        for url in &ws_state.remote_url {
            content = content.push(
                row![
                    iced_widget::Space::new().width(Length::Fixed(value_indent)),
                    text(url.clone())
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().body),
                ]
                .align_y(iced_widget::core::Alignment::Center),
            );
        }
    }

    content = content.push(links_section(
        "项目文档",
        links::LinkTarget::Docs,
        &ws_state.links,
        &ws_state.expanded_link_dirs,
        &ws_state.selected_link,
    ));
    content = content.push(links_section(
        "Agent 记忆",
        links::LinkTarget::Memory,
        &ws_state.links,
        &ws_state.expanded_link_dirs,
        &ws_state.selected_link,
    ));

    if let Some(err) = &ws_state.error {
        content = content.push(
            text(format!("⚠ {err}"))
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().red),
        );
    }

    let body = column![content, project_footer_bar()].spacing(0);

    let base =
        container(body)
            .width(width)
            .height(Length::Fill)
            .style(
                move |_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: Some(byteui::theme::color::current().panel.into()),
                    border: outer,
                    ..iced_widget::container::Style::default()
                },
            );

    if ws_state.delete_pending.is_some() {
        let dismiss = MouseArea::new(
            container(column![])
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: Some(byteui::theme::color::current().scrim.into()),
                    ..iced_widget::container::Style::default()
                }),
        )
        .on_press(Message::DeleteProjectCancel);
        return stack![base, dismiss, project_delete_confirm_popup(ws_state)]
            .width(width)
            .height(Length::Fill)
            .into();
    }

    if ws_state.scaffold_run.is_some() {
        // 进行中不可通过点击遮罩关闭:遮罩本身不挂 `on_press`,只挡住底层
        // 交互(与 `delete_pending` 分支的可点击遮罩故意不同——必须等全部
        // 步骤完成才能关,见 spec"弹窗可取消性"一节)。
        let scrim = container(column![])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().scrim.into()),
                ..iced_widget::container::Style::default()
            });
        return stack![base, scrim, scaffold_progress_popup(ws_state)]
            .width(width)
            .height(Length::Fill)
            .into();
    }

    base.into()
}

/// 项目信息面板底部 footer-bar,结构与文件树面板的 `git_footer_bar` 一致:
/// 1px `BORDER` 分隔线 + `padding([6, 8])` 容器。当前放「修复项目 / 删除项目」
/// 两个并排圆角按钮:「修复项目」触发 `RepairProject`(见 `spawn_repair_run`,
/// 弹出逐步骤进度弹窗),「删除项目」触发 `DeleteProjectRequest` 打开三选一
/// 确认弹窗(见 `project_delete_confirm_popup`)。
fn project_footer_bar() -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let repair = button(
        text("修复项目")
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().cream),
    )
    .on_press(Message::RepairProject)
    .width(Length::Fill)
    .padding([6, 8])
    .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
        background: Some(byteui::theme::color::current().bg.into()),
        border: Border {
            color: byteui::theme::color::current().border,
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: byteui::theme::color::current().cream,
        ..iced_widget::button::Style::default()
    });

    let delete = button(
        text("删除项目")
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().red),
    )
    .on_press(Message::DeleteProjectRequest)
    .width(Length::Fill)
    .padding([6, 8])
    .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
        background: Some(byteui::theme::color::current().bg.into()),
        border: Border {
            color: byteui::theme::color::current().red,
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: byteui::theme::color::current().red,
        ..iced_widget::button::Style::default()
    });

    let bar = row![repair, delete]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center);

    let top_line = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..iced_widget::container::Style::default()
        });

    let col = column![top_line, bar].spacing(4);

    container(col)
        .width(Length::Fill)
        .padding([6, 8])
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: None,
            ..iced_widget::container::Style::default()
        })
        .into()
}

/// "修复项目"弹窗单行:左侧状态符号 + 步骤名 + 右侧简短详情文字。
fn scaffold_step_row<'a>(
    label: &'a str,
    state: &'a ScaffoldStepState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (glyph, glyph_color, detail): (&str, iced_widget::core::Color, String) = match state {
        ScaffoldStepState::Pending => ("○", byteui::theme::color::current().dim, String::new()),
        ScaffoldStepState::Running => (
            "…",
            byteui::theme::color::current().gold,
            "进行中".to_string(),
        ),
        ScaffoldStepState::Done(project_scaffold::ScaffoldStepResult::AlreadyOk) => (
            "✓",
            byteui::theme::color::current().green,
            "已是最新".to_string(),
        ),
        ScaffoldStepState::Done(project_scaffold::ScaffoldStepResult::Created(msg)) => {
            ("✓", byteui::theme::color::current().green, msg.clone())
        }
        ScaffoldStepState::Done(project_scaffold::ScaffoldStepResult::Failed(msg)) => {
            ("✗", byteui::theme::color::current().red, msg.clone())
        }
    };
    row![
        text(glyph)
            .size(byteui::theme::font::body())
            .color(glyph_color),
        text(label)
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().cream)
            .width(Length::Fixed(140.0)),
        text(detail)
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim),
    ]
    .spacing(8)
    .align_y(iced_widget::core::Alignment::Center)
    .into()
}

fn scaffold_backfill_row(
    state: &BackfillStepState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (glyph, glyph_color, detail) = match state {
        BackfillStepState::Pending => (
            "○".to_string(),
            byteui::theme::color::current().dim,
            String::new(),
        ),
        BackfillStepState::Running(p) => (
            "…".to_string(),
            byteui::theme::color::current().gold,
            format!("{}/{}", p.completed, p.total),
        ),
        BackfillStepState::Done(p) if p.total == 0 => (
            "✓".to_string(),
            byteui::theme::color::current().green,
            "无需补".to_string(),
        ),
        BackfillStepState::Done(p) => (
            "✓".to_string(),
            byteui::theme::color::current().green,
            format!("{}/{}", p.completed, p.total),
        ),
    };
    row![
        text(glyph)
            .size(byteui::theme::font::body())
            .color(glyph_color),
        text("补总结")
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().cream)
            .width(Length::Fixed(140.0)),
        text(detail)
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim),
    ]
    .spacing(8)
    .align_y(iced_widget::core::Alignment::Center)
    .into()
}

/// "修复项目"进度弹窗:视觉模板同 `project_delete_confirm_popup`(卡片 +
/// 底部按钮)。进行中时"关闭"按钮不可点(`on_press_maybe`),全部完成
/// (`ScaffoldRunState::all_done`)才激活。
fn scaffold_progress_popup(
    ws_state: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(run) = &ws_state.scaffold_run else {
        return container(column![]).into();
    };
    let mut rows = column![].spacing(10);
    for (label, state) in &run.steps {
        rows = rows.push(scaffold_step_row(label, state));
    }
    rows = rows.push(scaffold_backfill_row(&run.backfill));

    let done = run.all_done();
    let close_label = if done { "关闭" } else { "进行中…" };
    let close_btn = button(
        text(close_label)
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
    )
    .on_press_maybe(done.then_some(Message::ScaffoldPopupClose))
    .padding([6, 12])
    .style(
        move |_t: &iced_widget::Theme, _s| iced_widget::button::Style {
            background: Some(byteui::theme::color::current().card.into()),
            text_color: byteui::theme::color::current().cream,
            border: Border {
                color: if done {
                    byteui::theme::color::current().gold
                } else {
                    byteui::theme::color::current().border
                },
                width: 1.0,
                radius: 4.0.into(),
            },
            ..iced_widget::button::Style::default()
        },
    );

    let card = column![
        text("修复项目")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
        rows,
        container(close_btn).align_x(iced_widget::core::alignment::Horizontal::Right),
    ]
    .spacing(14);

    container(card)
        .width(Length::Fixed(360.0))
        .padding(16)
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(byteui::theme::color::current().card.into()),
            border: Border {
                color: byteui::theme::color::current().border,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..iced_widget::container::Style::default()
        })
        .into()
}

/// 「删除项目」三选一确认弹窗,视觉模板同 `ssh.rs::delete_confirm_popup`
/// (卡片 + 取消/确认按钮)。单选行复用 `ssh.rs::radio_dot` 的视觉语言
/// (选中态 GOLD 实心描边,未选中态空心 BORDER 描边)——ssh 的 `radio_dot`
/// 绑定在 `ssh::Message` 上、跨模块复用不了类型,这里就地画一份同样的视觉。
fn project_delete_confirm_popup(
    ws_state: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let selected = ws_state
        .delete_pending
        .unwrap_or(delete::DeleteScope::DozerOnly);

    let radio_row = |scope: delete::DeleteScope, label: &'static str| {
        let is_selected = selected == scope;
        let dot = container(iced_widget::Space::new())
            .width(Length::Fixed(10.0))
            .height(Length::Fixed(10.0))
            .style(
                move |_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: if is_selected {
                        Some(byteui::theme::color::current().gold.into())
                    } else {
                        None
                    },
                    border: if is_selected {
                        Border {
                            color: byteui::theme::color::current().gold,
                            width: 1.5,
                            radius: 5.0.into(),
                        }
                    } else {
                        Border {
                            color: byteui::theme::color::current().border,
                            width: 1.5,
                            radius: 5.0.into(),
                        }
                    },
                    ..iced_widget::container::Style::default()
                },
            );
        let ring = container(dot)
            .width(Length::Fixed(16.0))
            .height(Length::Fixed(16.0))
            .align_x(iced_widget::core::alignment::Horizontal::Center)
            .align_y(iced_widget::core::alignment::Vertical::Center);
        MouseArea::new(
            row![
                ring,
                text(label)
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().cream),
            ]
            .spacing(6)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        )
        .interaction(iced_widget::core::mouse::Interaction::Pointer)
        .on_press(Message::DeleteProjectScopeSelect(scope))
    };

    let cancel = button(
        text("取消")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
    )
    .on_press(Message::DeleteProjectCancel)
    .padding([6, 12])
    .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
        background: Some(byteui::theme::color::current().card.into()),
        text_color: byteui::theme::color::current().cream,
        border: Border {
            color: byteui::theme::color::current().border,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..iced_widget::button::Style::default()
    });
    let confirm = button(
        text("删除")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().red),
    )
    .on_press(Message::DeleteProjectConfirm)
    .padding([6, 12])
    .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
        background: Some(byteui::theme::color::current().card.into()),
        text_color: byteui::theme::color::current().red,
        border: Border {
            color: byteui::theme::color::current().red,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..iced_widget::button::Style::default()
    });

    let dialog = container(
        column![
            text("删除项目")
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().cream),
            text("选择删除范围,操作会把对应内容移入系统回收站(可找回)。")
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().dim),
            column![
                radio_row(delete::DeleteScope::DozerOnly, "只删 dozer 关联与缓存文件"),
                radio_row(
                    delete::DeleteScope::WithAgentCache,
                    "以上 + 所有 agent 缓存数据"
                ),
                radio_row(
                    delete::DeleteScope::WithProjectFiles,
                    "以上 + 项目文件与版本仓库"
                ),
            ]
            .spacing(8),
            row![cancel, confirm].spacing(8),
        ]
        .spacing(12),
    )
    .padding(16)
    .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
        background: Some(byteui::theme::color::current().card.into()),
        border: Border {
            color: byteui::theme::color::current().border,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..iced_widget::container::Style::default()
    });

    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

/// 把可能很长的绝对路径压缩成一行可读字符串:超过 `MAX` 个字符时保留首尾、
/// 中间用 `…` 代替,避免项目面板被长路径撑破。
fn shorten_path(p: &str) -> String {
    const MAX: usize = 48;
    let chars: Vec<char> = p.chars().collect();
    if chars.len() <= MAX {
        return p.to_string();
    }
    let keep = MAX - 1;
    let head_len = keep / 2;
    let tail_len = keep - head_len;
    let head: String = chars[..head_len].iter().collect();
    let tail: String = chars[chars.len() - tail_len..].iter().collect();
    format!("{head}…{tail}")
}

fn links_section<'a>(
    title: &'static str,
    target: links::LinkTarget,
    links_state: &'a links::LinksState,
    expanded: &'a std::collections::HashMap<PathBuf, Vec<links::DirRow>>,
    selected_link: &'a Option<PathBuf>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(6);
    col = col.push(
        row![
            icons::view(
                icons::IconKind::CircleSmall,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().cream
            ),
            text(title)
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().cream),
            iced_widget::space::horizontal(),
            button(
                text("+")
                    .size(byteui::theme::font::label())
                    .color(byteui::theme::color::current().dim)
            )
            .on_press(Message::Pick(target))
            .style(|_t, _s| iced_widget::button::Style {
                background: None,
                text_color: byteui::theme::color::current().dim,
                ..iced_widget::button::Style::default()
            }),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    );

    for (i, entry) in links_state.list(target).iter().enumerate() {
        let name = entry
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| entry.path.to_string_lossy().into_owned());
        let row_icon = if entry.kind == links::LinkKind::Dir {
            icons::IconKind::Folder
        } else {
            icons::icon_for_file(&name)
        };
        let click_msg = if entry.kind == links::LinkKind::Dir {
            Message::LinkDirToggle {
                target,
                path: entry.path.clone(),
            }
        } else {
            Message::OpenLink(entry.path.clone())
        };
        // 删除改由右键菜单(`LinkContextMenu`)触发,行内不再挂 × 按钮。
        let is_selected = selected_link.as_deref() == Some(entry.path.as_path());
        let row_btn = button(
            row![
                icons::view(
                    row_icon,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().dim
                ),
                text(name)
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().body),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .on_press(click_msg)
        .style(move |_t, _s| iced_widget::button::Style {
            background: if is_selected {
                Some(byteui::theme::color::current().card.into())
            } else {
                None
            },
            text_color: byteui::theme::color::current().body,
            ..iced_widget::button::Style::default()
        });
        col = col.push(
            MouseArea::new(row_btn).on_right_press(Message::LinkContextMenu { target, index: i }),
        );
        if entry.kind == links::LinkKind::Dir
            && let Some(rows) = expanded.get(&entry.path)
        {
            for row_entry in rows {
                // 展开子项同样可点选(参考文件树每行都可选中):文件→打开预览,
                // 目录→仅选中(只读单层,不二次展开);单击即高亮。
                let child_path = row_entry.path.clone();
                let child_click = if row_entry.is_dir {
                    Message::LinkSelect {
                        target,
                        path: child_path.clone(),
                    }
                } else {
                    Message::OpenLink(child_path.clone())
                };
                let child_is_selected = selected_link.as_deref() == Some(child_path.as_path());
                let child_btn = button(
                    row![
                        iced_widget::space::Space::new().width(Length::Fixed(20.0)),
                        icons::view(
                            if row_entry.is_dir {
                                icons::IconKind::Folder
                            } else {
                                icons::icon_for_file(&row_entry.name)
                            },
                            byteui::theme::icon_size::row(),
                            byteui::theme::color::current().dim
                        ),
                        text(row_entry.name.clone())
                            .size(byteui::theme::font::caption())
                            .color(byteui::theme::color::current().dim),
                    ]
                    .spacing(6)
                    .align_y(iced_widget::core::Alignment::Center),
                )
                .on_press(child_click)
                .style(move |_t, _s| iced_widget::button::Style {
                    background: if child_is_selected {
                        Some(byteui::theme::color::current().card.into())
                    } else {
                        None
                    },
                    text_color: byteui::theme::color::current().dim,
                    ..iced_widget::button::Style::default()
                });
                col = col.push(child_btn);
            }
        }
    }
    col.into()
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
    fn acceptance_count_loaded_sets_field() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::AcceptanceCountLoaded(1, Some(3)),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.project_acceptance_count, Some(3));
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
            Message::ScaffoldStepFinished(1, 1, project_scaffold::ScaffoldStepResult::AlreadyOk),
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
            ScaffoldStepState::Done(project_scaffold::ScaffoldStepResult::AlreadyOk)
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
                project_scaffold::ScaffoldStepResult::Created("导入 3 个历史文件".into()),
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
            ScaffoldStepState::Done(project_scaffold::ScaffoldStepResult::Created(
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
            *state = ScaffoldStepState::Done(project_scaffold::ScaffoldStepResult::AlreadyOk);
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
