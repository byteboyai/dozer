// crates/dozer-app/src/git_log.rs
//! spike(2026-08-06): 验证第三方 Rust 库 `gleisbau`(git-graph 的布局引擎,
//! 拆出来的独立 crate)能否喂出可在 iced Canvas 里原生画出的提交图数据,
//! 探路"要不要在 Dozer 里做一个类似 VS Code Git Graph 的面板"。
//!
//! 只验证数据链路是否走得通,不追求 curve/fork 的像素级还原:每条 track
//! 画一根直线,commit 是线上的一个圆点,父子关系用直线连接(不是贝塞尔)。
//! 验证通过、决定转正时,再补动画/交互/性能优化。
use crate::app::{App, HoverId};
use crate::theme;
use iced_widget::core::alignment;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Border, Element, Font, Length, Rectangle};
use iced_widget::{MouseArea, column, container, row, scrollable, text};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 首次打开面板拉多少个 commit——够看出分叉/合并的形状,又不至于让
/// revwalk + 分支归属分析在大仓库上明显卡顿(gleisbau 的 API 是"从头按
/// max_count 走一遍 revwalk",没有增量/游标接口——见 build() 文档)。这是
/// 面板能看到的 commit 总数上限,不再提供"问 git 要更多"的入口(2026-08-20
/// 移除,见 `COMMIT_PAGE_SIZE` 之下)。
pub const DEFAULT_MAX_COMMITS: usize = 200;

/// commit 列表一页显示的条数(客户端分页,不触发 git 重新 revwalk——同
/// `homespace::PROJECT_PAGE_SIZE` 的"更多..."翻页手法,只是在已经拉到的
/// `DEFAULT_MAX_COMMITS` 缓存里逐页展开)。
const COMMIT_PAGE_SIZE: usize = 20;

/// 一个 commit 指向的引用(分支/远程分支/tag),供图上显示彩色标签。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    LocalBranch,
    RemoteBranch,
    Tag,
}

/// 派生 `Debug + Clone`——`Message::GitLogSnapshotLoaded` 要装
/// `Result<GitLogSnapshot, _>`,`Message` 本身 `derive(Debug, Clone)`(见
/// `CommitDetail` 上同理由的注释)。
#[derive(Debug, Clone)]
pub struct CommitRow {
    short_sha: String,
    summary: String,
    /// commit 作者名(`git_commit.author().name()`),commit 列表行用它替代
    /// 原先展示的 `short_sha`(用户需求:把 commit id 改为 commit user)。
    /// 极少数取不到作者名的 commit 为 `None`,列表回退显示 `short_sha`。
    author: Option<String>,
    /// 指向这个 commit 的分支/tag(可能为空)。
    refs: Vec<RefLabel>,
    /// 这个 commit 的完整 40 位 oid,选中详情用——`short_sha` 只够显示,
    /// 不够拿去 `git2::Repository::find_commit`。
    oid: git2::Oid,
    /// author time,Unix 秒——commit 列表行展示用(见 `format_commit_time`)。
    time: i64,
    /// 这是不是合并提交(真实 git parent 数 `>= 2`)。见 spec §6——2026-08-17
    /// 重构后 commit 列表不再画分支拓扑,合并提交只靠这个 bool + `git-merge`
    /// 图标区分,不需要 `column`/`color_idx`/`parents` 那套布局字段。
    is_merge: bool,
}

/// 派生 `Debug + Clone`,理由同 [`CommitRow`]。
#[derive(Debug, Clone)]
pub struct GitLogSnapshot {
    repo_path: PathBuf,
    rows: Vec<CommitRow>,
    /// 当前 HEAD 所在的本地分支名(detached HEAD 时为 `None`)——列表里给这
    /// 个分支的标签加个 `→` 前缀区分"这是我现在checkout的那条"。
    head_branch: Option<String>,
    /// 这份快照实际请求的 `max_count`。
    max_count: usize,
}

impl GitLogSnapshot {
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    pub fn max_count(&self) -> usize {
        self.max_count
    }

    pub fn head_branch(&self) -> Option<&str> {
        self.head_branch.as_deref()
    }
}

/// `Settings` 无内置构造函数——`BranchSettingsDef::simple()` 只给"分支命名
/// 分类"这半份(persistence/order/颜色分类的字符串正则源),真正的
/// `Settings`(含格式/字符集/合并模式等)得手工拼。选 `simple()` 而非
/// `git_flow()`:Dozer worktree 分支名随意,不套 gitflow 那套 main/develop/
/// feature 假设更稳妥(见调研备忘)。
fn default_settings() -> Result<gleisbau::settings::Settings, String> {
    use gleisbau::settings::{
        BranchOrder, BranchSettings, BranchSettingsDef, Characters, MergePatterns, Settings,
    };
    let branches = BranchSettings::from(BranchSettingsDef::simple()).map_err(|e| e.to_string())?;
    Ok(Settings {
        reverse_commit_order: false,
        debug: false,
        compact: false,
        colored: false,
        include_remote: true,
        format: gleisbau::print::format::CommitFormat::OneLine,
        wrapping: None,
        characters: Characters::thin(),
        branch_order: BranchOrder::ShortestFirst(true),
        branches,
        merge_patterns: MergePatterns::default(),
    })
}

/// 对 `repo_path` 跑一次 `gleisbau` 布局,产出可渲染快照。函数本身是同步
/// 阻塞的(revwalk + 分支归属分析在提交/分支数量大时可到秒级),调用方
/// (`workspace.rs` 的 `App::spawn_git_log_refresh`)必须扔进
/// `tokio::task::spawn_blocking`,不能直接摆在 UI 线程的 `update()` 里跑。
pub fn build(repo_path: &Path, max_count: usize) -> Result<GitLogSnapshot, String> {
    let repository = gleisbau::get_repo(repo_path, false).map_err(|e| e.message().to_string())?;
    let settings = std::rc::Rc::new(default_settings()?);
    let graph = gleisbau::graph::Builder::new()
        .with_repository(repository)
        .with_settings(settings)
        .with_max_count(max_count)
        .build()?;

    let head_branch = graph.head.is_branch.then(|| graph.head.name.clone());

    let rows = graph
        .tracks
        .commits
        .iter()
        .map(|commit| {
            let git_commit = graph
                .commit(commit.oid)
                .map_err(|e| e.message().to_string())?;
            let full_sha = commit.oid.to_string();
            let short_sha = full_sha.chars().take(7).collect();
            let summary = git_commit
                .summary()
                .ok()
                .flatten()
                .unwrap_or("")
                .to_string();
            let refs = graph
                .labels
                .get_labels(&commit.oid)
                .map(|labels| {
                    labels
                        .iter()
                        .map(|l| RefLabel {
                            name: l.name.clone(),
                            kind: match l.kind {
                                gleisbau::print::label::LabelType::LocalBranch => {
                                    RefKind::LocalBranch
                                }
                                gleisbau::print::label::LabelType::RemoteBranch => {
                                    RefKind::RemoteBranch
                                }
                                gleisbau::print::label::LabelType::Tag => RefKind::Tag,
                            },
                        })
                        .collect()
                })
                .unwrap_or_default();
            let time = git_commit.time().seconds();
            let is_merge = git_commit.parent_count() >= 2;
            let author = git_commit.author().name().ok().map(|n| n.to_string());
            Ok(CommitRow {
                short_sha,
                summary,
                author,
                refs,
                oid: commit.oid,
                time,
                is_merge,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(GitLogSnapshot {
        repo_path: repo_path.to_path_buf(),
        rows,
        head_branch,
        max_count,
    })
}

/// 单个改动文件:哪个文件、什么类型的改动、这个文件自己的 unified diff
/// 文本(不是整个提交的 diff——按文件拆开,方便 UI 逐文件展开)。
/// 派生 `Clone`(`Message::GitLogDetailLoaded` 要装 `Result<CommitDetail, _>`,
/// `Message` 本身 `derive(Debug, Clone)`——workspace.rs 的 `Message`,
/// 这两个结构体的字段类型都天然 `Debug + Clone`,一起派生即可)。
#[derive(Debug, Clone)]
pub struct DiffFileEntry {
    pub path: String,
    pub status: git2::Delta,
    pub patch: String,
    /// `patch` 是否因为过大被截断——超过 [`MAX_PATCH_CHARS`] 就不再追加
    /// 正文,只留一行提示。大 diff(几千行)一次性喂给 `text()` widget 排版,
    /// 布局开销肉眼可见("打开一次提交详情也有些卡顿"),而详情面板本来就
    /// 不是给通读整份 diff 用的,截断只影响展示,不影响 diff 计算的正确性。
    pub truncated: bool,
}

/// 单个文件 `patch` 文本的字符数上限,超过就截断(见 [`DiffFileEntry::truncated`])。
const MAX_PATCH_CHARS: usize = 20_000;

#[derive(Debug, Clone)]
pub struct CommitDetail {
    pub files: Vec<DiffFileEntry>,
}

/// Git Log 模块自己的消息类型——内核(`workspace.rs`)只认一个包装变体
/// `Message::GitLog(extensions::git_log::Message)`,这个模块本身不 import
/// 顶层 `Message`,不知道自己被包在哪个外层类型里。
#[derive(Debug, Clone)]
pub enum Message {
    SelectCommit(git2::Oid),
    /// commit 列表客户端翻页"更多"图标按钮:只在已缓存的 `cache` 里往下
    /// 多展开一页(`COMMIT_PAGE_SIZE` 条),不问 git 要新数据。
    CommitListMore,
    /// commit 搜索框草稿变化(iced `text_input::on_input`)。
    SearchInput(String),
    /// 回车 / 点搜索按钮:把草稿落成生效的 `search` 过滤词,同时把翻页
    /// 重置回第 1 页(过滤后结果变少,停在旧页码没有意义)。
    SearchSubmit,
    /// commit 搜索框被右键:内核拦截,不进 `update`——转发成顶层
    /// `Message::TextInputMenuOpen` 弹出通用输入框右键菜单(见 app.rs)。
    TextInputMenuOpen(crate::app::TextInputTarget),
    DetailLoaded(PathBuf, git2::Oid, Result<CommitDetail, String>),
    SnapshotLoaded(PathBuf, usize, Result<GitLogSnapshot, String>),
    /// 点文件列表某一行,选中它(右下面板据此展示该文件的 diff)。
    SelectFile(String),
    /// 展开左侧面板底部的分支切换下拉(首次展开时内核顺带异步查一次
    /// `delivery::local_branches`)。
    BranchPickerOpen,
    BranchPickerClose,
    /// 内核异步查完本地分支列表 + 工作区 dirty 状态后落地(仓库路径核对
    /// 一致才接受)。`bool` = 工作区是否有未提交改动(`delivery::is_dirty`)。
    BranchesLoaded(PathBuf, Vec<String>, bool),
    /// 点某个分支——内核截获处理(同 `ProjectTabOpen` 的既有例外模式),
    /// 不会转发到 `update`(见其 `unreachable!` 分支)。
    BranchSwitch(String),
    BranchSwitchDone(Result<(), String>),
    /// 三栏布局里左右分割线开始拖拽——内核截获,转成 app 级
    /// `ColumnDragStart(GitLogSplit)`(分割线拖拽是 app 级 PanelDims 状态,
    /// 扩展自己发不了 app::Message,靠这条例外消息让内核代发)。
    ColumnDragStart,
    /// 三栏布局里右侧上下分割线开始拖拽——内核截获,转成 app 级
    /// `RowDragStart(GitLogFileDiffSplit)`。
    RowDragStart,
    /// 卡片(commit 行 / diff 文件行)的鼠标悬停进/出:内核 `App::update`
    /// 里拦截,转成 `Message::Hover(HoverId, bool)` 驱动统一的卡片
    /// 悬停动画,不会转发到 `update`。
    Hover(HoverId, bool),
}

/// Git Log 面板的全部状态。现在挂在 `App`(不按项目分,见设计文档"非
/// 目标"——这次纯重构不改这个现状),以后要改成按项目分的话,类型本身
/// 不用变,只是挪个持有位置。
#[derive(Default)]
pub struct State {
    cache: Option<GitLogSnapshot>,
    error: Option<String>,
    selected: Option<git2::Oid>,
    detail: Option<Result<CommitDetail, String>>,
    /// 右上文件列表当前选中的文件路径(`CommitDetail.files[].path`)。切
    /// commit 时先清空,新 `detail` 落地后预选第一个改动文件。
    selected_file: Option<String>,
    /// 最近一次派发的 `build` 请求 (repo_path, max_count)——落地时核对
    /// 还对不对得上"现在真正需要的",不对就丢弃。
    pending: Option<(PathBuf, usize)>,
    /// 面板底部分支下拉是否展开。
    branch_picker_open: bool,
    /// 当前仓库的本地分支列表(`delivery::local_branches` 结果缓存,内核在
    /// `BranchPickerOpen` 首次展开时异步查一次)。
    branches: Vec<String>,
    /// 当前仓库工作区是否有未提交改动(`delivery::is_dirty` 结果,随
    /// `BranchesLoaded` 一起落地)。dirty 时锁定除当前分支外的其余分支,
    /// 语义跟 `files.rs::branch_picker_popup` 的 dirty-lock 一致。
    dirty: bool,
    /// 分支切换请求进行中(禁用下拉交互、显示"切换中…")。
    branch_switch_pending: bool,
    /// commit 列表客户端分页已经点开的"更多"次数(0 = 只显示第一页
    /// `COMMIT_PAGE_SIZE` 条)。`#[derive(Default)]` 落到 0,天然就是
    /// "第一页",不需要额外的构造器初始化(同 `commit_visible_count` 的
    /// "+1 折算"注释)。
    pages: usize,
    /// commit 搜索框已提交生效的过滤词(空串 = 不过滤,按摘要大小写不敏感
    /// 子串匹配)。`git_log::State` 不按项目分(见结构体顶部注释),切项目
    /// 不清空——同 `pages` 目前也不在切项目时重置的既有现状,不在这次改动
    /// 里单独修。
    search: String,
    /// 搜索框编辑态草稿——同 `workspace::Workspace` 会话搜索的 draft/committed
    /// 分离手法,打字期间只改草稿,回车/点搜索按钮才落成 `search`。
    search_draft: String,
    /// 搜索框是否持有 iced 真实焦点。**不是**应用层手动置位的镜像——每帧
    /// 渲染循环里 `CaptureSearchFocus` 问一遍 iced 真相后立刻写进这里
    /// (`main.rs` 键盘路由读它决定要不要把按键放行)。
    search_focused: bool,
}

impl State {
    /// commit 列表当前应该显示到第几条(客户端分页,见 `COMMIT_PAGE_SIZE`
    /// 上方注释)。`pages` 是"点过几次'更多'"(0-based),`+1` 折算成
    /// "当前共几页"再乘页大小。
    pub fn commit_visible_count(&self) -> usize {
        (self.pages + 1) * COMMIT_PAGE_SIZE
    }

    /// 当前缓存的 `max_count`(无缓存则回落 `DEFAULT_MAX_COMMITS`)——用于
    /// "内容不变、只是要重新拉一遍"的场景(`.git` 引用变化触发的重建)。
    pub fn cache_max_count(&self) -> usize {
        self.cache
            .as_ref()
            .map(|c| c.max_count())
            .unwrap_or(DEFAULT_MAX_COMMITS)
    }

    /// 当前缓存快照所属的仓库路径(`None` = 还没有缓存)。内核靠它判断
    /// "缓存是不是已经属于当前聚焦项目",不用时不重建。
    pub fn cache_repo_path(&self) -> Option<&Path> {
        self.cache.as_ref().map(|c| c.repo_path())
    }

    /// 搜索框是否持有 iced 真实焦点(`App::git_log_search_focused` 转发)。
    pub fn search_focused(&self) -> bool {
        self.search_focused
    }

    /// 每帧渲染循环调用(`App::set_git_log_search_focused` 转发),见字段
    /// 上的文档。
    pub(crate) fn set_search_focused(&mut self, focused: bool) {
        self.search_focused = focused;
    }

    /// 分支列表是否还没查过(`BranchPickerOpen` 首次展开时,内核据此判断
    /// 要不要发起异步查询——避免每次展开都重新查一遍)。
    pub fn branches_is_empty(&self) -> bool {
        self.branches.is_empty()
    }

    /// 设置"分支切换请求进行中"标记(内核在 `BranchSwitch` 截获时置位,落地
    /// `BranchSwitchDone` 时由 `update()` 清)。
    pub(crate) fn set_branch_switch_pending(&mut self, pending: bool) {
        self.branch_switch_pending = pending;
    }
}

/// 处理 `SelectCommit`/`DetailLoaded`/`SnapshotLoaded` 三种消息。
pub fn update(
    state: &mut State,
    msg: Message,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) -> Option<Message> {
    match msg {
        // 卡片悬停由内核 `App::update` 拦截转发到 `set_hover`,不会到这。
        Message::Hover(_, _) => None,
        Message::TextInputMenuOpen(_) => None,
        Message::SelectCommit(oid) => {
            state.selected = Some(oid);
            state.detail = None;
            state.selected_file = None;
            let repo_path = state.cache.as_ref().map(|c| c.repo_path().to_path_buf())?;
            handle.spawn(async move {
                let repo_path2 = repo_path.clone();
                let result = tokio::task::spawn_blocking(move || commit_detail(&repo_path2, oid))
                    .await
                    .unwrap_or_else(|e| Err(format!("详情加载任务失败: {e}")));
                emit(Message::DetailLoaded(repo_path, oid, result));
            });
            None
        }
        Message::SelectFile(path) => {
            state.selected_file = Some(path);
            None
        }
        Message::DetailLoaded(repo_path, oid, result) => {
            let still_current = state.cache.as_ref().map(|c| c.repo_path())
                == Some(repo_path.as_path())
                && state.selected == Some(oid);
            if still_current {
                state.selected_file = match &result {
                    Ok(detail) => detail.files.first().map(|f| f.path.clone()),
                    Err(_) => None,
                };
                state.detail = Some(result);
            }
            // 否则:项目已切换,或用户点了别的提交——这份结果过期了,丢弃。
            None
        }
        Message::SnapshotLoaded(repo_path, max_count, result) => {
            let still_pending = state.pending.as_ref().map(|(p, m)| (p.as_path(), *m))
                == Some((repo_path.as_path(), max_count));
            if !still_pending {
                return None;
            }
            state.pending = None;
            match result {
                Ok(snapshot) => {
                    state.cache = Some(snapshot);
                    state.error = None;
                }
                Err(err) => {
                    state.cache = None;
                    state.error = Some(err);
                }
            }
            None
        }
        Message::CommitListMore => {
            state.pages += 1;
            None
        }
        Message::SearchInput(s) => {
            state.search_draft = s;
            None
        }
        Message::SearchSubmit => {
            state.search = state.search_draft.clone();
            state.pages = 0;
            None
        }
        Message::BranchPickerOpen => {
            state.branch_picker_open = true;
            None
        }
        Message::BranchPickerClose => {
            state.branch_picker_open = false;
            None
        }
        Message::BranchesLoaded(repo_path, branches, dirty) => {
            let matches = state.cache.as_ref().map(|c| c.repo_path()) == Some(repo_path.as_path());
            if matches {
                state.branches = branches;
                state.dirty = dirty;
            }
            None
        }
        Message::BranchSwitch(_) => {
            unreachable!(
                "BranchSwitch 由内核在 Message::GitLog 分支里直接处理(需要仓库路径 + 真实 checkout IO),不会转发到这里"
            )
        }
        Message::BranchSwitchDone(result) => {
            state.branch_picker_open = false;
            state.branch_switch_pending = false;
            match result {
                Ok(()) => state.error = None,
                Err(e) => state.error = Some(e),
            }
            None
        }
        Message::ColumnDragStart | Message::RowDragStart => {
            unreachable!(
                "ColumnDragStart/RowDragStart 由内核在 Message::GitLog 分支里直接处理(转成 app 级拖拽消息),不会转发到这里"
            )
        }
    }
}

/// 异步重建 Git Log 快照,`max_count` 由调用方决定(通常是
/// `DEFAULT_MAX_COMMITS`)。现有 `App::spawn_git_log_refresh` 的搬家版本,
/// 行为不变。
pub fn request_refresh(
    state: &mut State,
    repo_path: PathBuf,
    max_count: usize,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    state.selected = None;
    state.detail = None;
    state.pending = Some((repo_path.clone(), max_count));
    handle.spawn(async move {
        let repo_path2 = repo_path.clone();
        let result = tokio::task::spawn_blocking(move || build(&repo_path2, max_count))
            .await
            .unwrap_or_else(|e| Err(format!("Git Log 加载任务失败: {e}")));
        emit(Message::SnapshotLoaded(repo_path, max_count, result));
    });
}

/// 取某个提交改动了哪些文件、每个文件的 diff 文本。合并提交(≥2 parent)
/// 相对**第一父**算(与 `git show` 默认行为一致,不做三方 diff——spec D5)。
/// 根提交(无 parent)相对空树算,等价于"全部文件都是新增"。
pub fn commit_detail(repo_path: &Path, oid: git2::Oid) -> Result<CommitDetail, String> {
    let repo = git2::Repository::open(repo_path).map_err(|e| e.message().to_string())?;
    let commit = repo.find_commit(oid).map_err(|e| e.message().to_string())?;
    let new_tree = commit.tree().map_err(|e| e.message().to_string())?;
    let old_tree = match commit.parent(0) {
        Ok(parent) => Some(parent.tree().map_err(|e| e.message().to_string())?),
        Err(_) => None, // 根提交,相对空树
    };
    let diff = repo
        .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)
        .map_err(|e| e.message().to_string())?;

    let mut files: Vec<DiffFileEntry> = diff
        .deltas()
        .filter_map(|delta| {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())?
                .to_string_lossy()
                .into_owned();
            Some(DiffFileEntry {
                path,
                status: delta.status(),
                patch: String::new(), // 下面按文件路径回填
                truncated: false,
            })
        })
        .collect();

    // git2 的 `Diff::print` 是整份 diff 一次性回调、按行给,不是按文件给
    // 一整块文本——这里按 `DiffLine::origin_value()` 是不是文件头
    // (`FileHeader`)切分,把每一行追加到当前文件对应的 `patch` 里。
    let mut current_path: Option<String> = None;
    diff.print(git2::DiffFormat::Patch, |delta, _hunk, line| {
        if matches!(line.origin_value(), git2::DiffLineType::FileHeader) {
            current_path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().into_owned());
        }
        if let Some(path) = &current_path
            && let Some(entry) = files.iter_mut().find(|f| &f.path == path)
            && !entry.truncated
        {
            if entry.patch.len() >= MAX_PATCH_CHARS {
                entry.truncated = true;
                entry.patch.push_str("\n… diff 过长,已截断显示\n");
            } else {
                let prefix = match line.origin() {
                    '+' | '-' | ' ' => line.origin().to_string(),
                    _ => String::new(),
                };
                entry.patch.push_str(&prefix);
                entry
                    .patch
                    .push_str(&String::from_utf8_lossy(line.content()));
            }
        }
        true
    })
    .map_err(|e| e.message().to_string())?;

    Ok(CommitDetail { files })
}

/// commit 搜索框(iced 原生 `text_input`)的 `widget::Id`:main.rs 每帧
/// `interface.operate` 用 `CaptureSearchFocus` 问真实焦点态。
pub fn search_field_id() -> Id {
    Id::new("git-log-search-box")
}

static SEARCH_FOCUSED: std::sync::LazyLock<Mutex<bool>> =
    std::sync::LazyLock::new(|| Mutex::new(false));

/// 读走并复位(消费式),同 `extensions::files::take_search_focused` 的
/// 消费式复位手法,避免搜索框不可见的帧卡死上一次 `true` 永久堵死终端
/// 键盘转发。
pub fn take_search_focused() -> bool {
    std::mem::replace(&mut *SEARCH_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍。`traverse` 必须调用传入闭包(见
/// [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureSearchFocus;
impl Operation<()> for CaptureSearchFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&search_field_id()) {
            *SEARCH_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

/// commit 列表关键字过滤:摘要(commit 消息第一行,`CommitRow::summary`)
/// 大小写不敏感子串匹配,空关键字不过滤。拆成纯函数(同
/// `workspace::filter_turn_groups`)方便 headless 单测。
fn filter_commit_rows<'a>(rows: &'a [CommitRow], query: &str) -> Vec<&'a CommitRow> {
    if query.is_empty() {
        return rows.iter().collect();
    }
    let needle = query.to_lowercase();
    rows.iter()
        .filter(|r| r.summary.to_lowercase().contains(&needle))
        .collect()
}

/// commit 列表上方的搜索框:形状与会话列表搜索框
/// (`workspace::conversation_list_pane`)一致——真正的 iced `text_input`
/// (`byteui::form::input_text`)+ 右侧搜索图标按钮。不会边输入边过滤,
/// 敲回车/点搜索按钮后由 `Message::SearchSubmit` 把草稿落成生效的
/// `search`。`highlight` 传 `search_active`:即使当前没聚焦,只要列表被
/// 搜索词过滤中就持续金框提示。
fn commit_search_box<'a>(
    app: &App,
    state: &'a State,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let search_active = state.search_focused() || !state.search.is_empty();
    let box_el = byteui::form::search_box::view(
        "搜索提交内容…",
        &state.search_draft,
        Some(search_field_id()),
        search_active,
        Message::SearchInput,
        Message::SearchSubmit,
        app.hover_progress(HoverId::GitLogSearchSubmit),
        |hovered| Message::Hover(HoverId::GitLogSearchSubmit, hovered),
    );
    byteui::interaction::context_menu::wrap(
        box_el,
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: search_field_id(),
            secure: false,
        })),
    )
}

/// 把一行 commit 的 `refs` 拼成形如 `[main][origin/main]` 的前缀文本;当前
/// HEAD 所在的本地分支加 `→` 标记(`[→main]`)。空 `refs` 返回空字符串。
/// 不在这里上色——canvas 文本整体只有一个 `Color`,没法给子串单独上色,
/// 颜色区分留给后续真的做背景色块 pill 时再处理(见 Task 4)。
fn ref_labels_text(refs: &[RefLabel], head_branch: Option<&str>) -> String {
    refs.iter()
        .map(|r| {
            let marker = if head_branch == Some(r.name.as_str()) {
                "→"
            } else {
                ""
            };
            format!("[{marker}{}]", r.name)
        })
        .collect::<Vec<_>>()
        .join("")
}

/// commit 线性列表(替代原 Canvas 拓扑图,2026-08-17 重构——见 spec
/// "架构与数据流"第 6 节)。每张卡片四行:首行图标(普通/合并)+ 作者名
/// (无作者时回退 short_sha,灰显等宽字);其下缩进三行依次是 commit 摘要、
/// 时间戳、分支标签(refs,带 HEAD 的 `→` 标记,青色;无 ref 时回退 short_sha
/// 灰显)。整行可点选中(`Message::SelectCommit`),选中态统一卡片样式(对齐
/// Todo/Files 面板既有选中行视觉语言)。
///
/// `visible_count`(`State::commit_visible_count`)客户端分页:只画前
/// `visible_count` 条,画不完时列表末尾补一个"更多"图标按钮(同
/// `homespace::home_project_list_view` 的处理方式)——不是把整个已缓存的
/// `snapshot.rows`(最多到 `DEFAULT_MAX_COMMITS`)一次性全画出来,大仓库
/// 几百条 commit 一次性铺开会让这块 `scrollable` 明显变沉。
fn commit_list_view<'a>(
    app: &App,
    rows: &[&'a CommitRow],
    selected: Option<git2::Oid>,
    head_branch: Option<&'a str>,
    visible_count: usize,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut list = column![].spacing(8);
    for row in rows.iter().take(visible_count) {
        let is_selected = selected == Some(row.oid);
        let icon_kind = if row.is_merge {
            byteui::interaction::icons::IconKind::GitMerge
        } else {
            byteui::interaction::icons::IconKind::GitCommitVertical
        };
        let branch_text = ref_labels_text(&row.refs, head_branch);
        let author_or_id = row.author.clone().unwrap_or_else(|| row.short_sha.clone());
        // 新四行布局(2026-08-21 调整,对应需求"commit 卡片按 icon+作者 /
        // 摘要 / 时间 / 分支 四行显示"):
        //   第一行:图标 + 作者名(无作者回退 short_sha),灰显等宽字。
        //   第二行:commit 摘要,左缩进到图标之后。
        //   第三行:时间戳(UTC,`YYYY-MM-DD HH:MM:SS`),缩进对齐。
        //   第四行:分支标签(带 HEAD 的 `→` 标记,青色;无 ref 时回退 short_sha)。
        let line1 = row![
            byteui::interaction::icons::view(
                icon_kind,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().dim
            ),
            text(author_or_id)
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim)
                .font(Font::MONOSPACE),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center);
        // 第 2~4 行统一左缩进到"图标之后":跳过图标宽度 + 图标与首行的
        // `spacing(8)`,与首行作者名左缘对齐。
        let indent = byteui::theme::icon_size::row() + 8.0;
        let summary_line = container(
            text(row.summary.clone())
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().cream),
        )
        .padding(iced_widget::core::Padding {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: indent,
        });
        let time_line = container(
            text(format_commit_time(row.time))
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
        )
        .padding(iced_widget::core::Padding {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: indent,
        });
        let branch_color = if row.refs.is_empty() {
            byteui::theme::color::current().dim
        } else {
            byteui::theme::color::current().cyan
        };
        let branch_line = container(
            text(branch_text)
                .size(byteui::theme::font::caption_sm())
                .color(branch_color)
                .font(Font::MONOSPACE),
        )
        .padding(iced_widget::core::Padding {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: indent,
        });
        let line = column![line1, summary_line, time_line, branch_line].spacing(4);
        // 统一卡片样式:选中/一般/hover 三态(选中=金边、hover=金边+填充、
        // 一般态=描边),不再用左侧 3px 金竖条表示选中。内部间距与卡片内边距
        // 对齐 Agent 面板的 agent 卡片(`workspace.rs::agent_card`:行距 4、
        // `padding(10)`),避免 commit 卡片内部过挤。原先
        // `MouseArea`+`container_card`+`hover_progress` 那套是指数衰减动画,
        // 鼠标移开后高亮会拖尾残留(验收反馈:hover 要"停"2s 才消退)——改回
        // agent 卡片同款原生 `button`+`button_card`,亮灭直接由 iced 自己的
        // `button::Status` 驱动,没有额外状态、没有拖尾。
        let card = iced_widget::button(container(line).padding(10))
            .on_press(Message::SelectCommit(row.oid))
            .width(Length::Fill)
            .style(byteui::interaction::cards::button_card(
                is_selected,
                byteui::theme::color::current().card,
            ));
        list = list.push(card);
    }
    // 还有没画出来的 commit 时,在列表末尾加一个居中的"更多"图标按钮
    // (Lucide ellipsis,无外边框/背景,hover DIM→GOLD)——点它翻下一页
    // (`Message::CommitListMore`,纯客户端状态,不问 git 要新数据)。
    if rows.len() > visible_count {
        let more_button = byteui::interaction::icons::icon_button_entry(
            byteui::interaction::icons::IconKind::Ellipsis,
            byteui::theme::icon_size::row(),
            false,
            false,
            app.hover_progress(HoverId::CommitListMore),
            false,
            byteui::theme::geometry::tab_button_size(),
            true,
            Message::CommitListMore,
            |hovered| Message::Hover(HoverId::CommitListMore, hovered),
            "更多",
        );
        list = list.push(
            container(more_button)
                .width(Length::Fill)
                .align_x(alignment::Horizontal::Center),
        );
    }
    scrollable(list)
        .direction(scrollable::Direction::Vertical(
            byteui::interaction::scrollbar::scrollbar(),
        ))
        .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style())
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// 右上文件列表:选中 commit 改动的每个文件一行(状态字符 + 路径),点击
/// 发 `Message::SelectFile`,选中态同 `commit_list_view` 的金边(统一卡片样式:
/// 选中=金边、hover=金边+填充、一般态=描边)。
fn file_list_view<'a>(
    app: &App,
    detail: &'a Result<CommitDetail, String>,
    selected_file: Option<&'a str>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    match detail {
        Err(err) => container(
            text(format!("详情加载失败: {err}"))
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().red),
        )
        .padding(8)
        .into(),
        Ok(detail) if detail.files.is_empty() => container(
            text("无文件改动")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
        .padding(8)
        .into(),
        Ok(detail) => {
            let mut list = column![].spacing(2);
            for (i, f) in detail.files.iter().enumerate() {
                let is_selected = selected_file == Some(f.path.as_str());
                let color = match f.status {
                    git2::Delta::Added => byteui::theme::color::current().green,
                    git2::Delta::Deleted => byteui::theme::color::current().red,
                    _ => byteui::theme::color::current().cyan,
                };
                let line = row![
                    text(status_glyph(f.status))
                        .size(byteui::theme::font::body())
                        .color(color)
                        .width(18),
                    text(f.path.clone())
                        .size(byteui::theme::font::body())
                        .color(byteui::theme::color::current().cream),
                ]
                .spacing(4);
                // 统一卡片样式:选中/一般/hover 三态(选中=金边、hover=金边+填充、
                // 一般态=描边),不再用左侧 3px 金竖条表示选中。
                let hovered = app.hover_progress(HoverId::GitFile(i)) > 0.0;
                let inner = container(line).padding([2, 8]).width(Length::Fill).style(
                    move |_t: &iced_widget::Theme| {
                        byteui::interaction::cards::container_card(
                            is_selected,
                            hovered,
                            byteui::theme::color::current().card,
                        )
                    },
                );
                let area = MouseArea::new(inner)
                    .interaction(iced_widget::core::mouse::Interaction::Pointer)
                    .on_enter(Message::Hover(HoverId::GitFile(i), true))
                    .on_exit(Message::Hover(HoverId::GitFile(i), false))
                    .on_press(Message::SelectFile(f.path.clone()));
                list = list.push(area);
            }
            scrollable(list)
                .direction(scrollable::Direction::Vertical(
                    byteui::interaction::scrollbar::scrollbar(),
                ))
                .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style())
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
    }
}

/// 渲染整块 Git Log 面板(三栏:左 commit 列表 | 右上文件列表 / 右下 diff
/// 内容,两条分割线都可拖拽)。纯函数——不碰 `App`/`Workspace` 内部状态,
/// 两条 split 比例由内核(`app.rs`)持有并传进来(与 Todo/Project 面板"内核
/// 传 split 值进来"的既有模式一致)。
pub fn view<'a>(
    app: &App,
    state: &'a State,
    git_log_split: f32,
    git_log_file_diff_split: f32,
    mirror: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let error = state.error.as_deref();
    // 面板内边距对齐文件树面板(`project_pane` region):header/body/footer
    // 的分隔线与内容容器统一按同一水平 inset 排布,避免 Git 面板自己另起
    // 一套 → 0 的 padding 与文件树/项目面板(8)错位。
    let pad = theme::region::project_pane().padding;
    let head = crate::chrome::homespace::home_panel_head(
        byteui::interaction::icons::IconKind::GitGraph,
        "Git",
    );

    let loading = state.pending.is_some();
    let Some(snapshot) = state.cache.as_ref() else {
        if loading {
            return container(
                column![
                    head,
                    byteui::feedback::math_curve::loading_hint(
                        byteui::feedback::math_curve::Curve::RoseThree,
                        "加载中…",
                        48.0,
                    ),
                ]
                .spacing(8)
                .padding(pad)
                .height(Length::Fill),
            )
            .into();
        }
        return container(
            column![
                head,
                text("未打开项目")
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().dim)
            ]
            .spacing(8)
            .padding(pad),
        )
        .into();
    };
    if snapshot.rows.is_empty() {
        return container(
            column![
                head,
                text("没有可显示的提交")
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().dim)
            ]
            .spacing(8)
            .padding(pad),
        )
        .into();
    }

    let head_branch = snapshot.head_branch();
    let mut left = column![head].spacing(8);
    if let Some(err) = error {
        left = left.push(
            text(format!("git log 读取失败: {err}"))
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().red),
        );
    }
    left = left.push(commit_search_box(app, state));
    let filtered = filter_commit_rows(&snapshot.rows, &state.search);
    if filtered.is_empty() {
        left = left.push(
            text("无匹配结果")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim),
        );
    } else {
        left = left.push(commit_list_view(
            app,
            &filtered,
            state.selected,
            head_branch,
            state.commit_visible_count(),
        ));
    }
    left = left.push(git_panel_footer_bar(state, head_branch));
    let left_with_picker = iced_widget::stack![
        container(left).width(Length::Fill).height(Length::Fill),
        branch_picker_view(state, head_branch),
    ];

    let (list_portion, content_portion) = crate::workspace::split_portions(git_log_split);
    let right: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if let Some(detail) = state.detail.as_ref() {
            // 文件列表上方头部:改动文件计数 + 一条 1px 分割线(见需求
            // "右侧文件列表上方新增头部统计文件数量")。计数直接取当前选中
            // 提交 `detail` 的文件数;加载失败时记 0(此时 file_list_view
            // 会另显示错误文案,头部只是个中性计数)。
            let file_count = match detail {
                Ok(d) => d.files.len(),
                Err(_) => 0,
            };
            let header = container(
                column![
                    text(format!("{} 个修改的文件", file_count))
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().dim),
                    container(iced_widget::Space::new())
                        .width(Length::Fill)
                        .height(Length::Fixed(1.0))
                        .style(|_t: &iced_widget::Theme| container::Style {
                            background: Some(byteui::theme::color::current().border.into()),
                            ..container::Style::default()
                        }),
                ]
                .spacing(6),
            )
            .padding([4, 8]);
            let (top_portion, bottom_portion) =
                crate::workspace::split_portions(git_log_file_diff_split);
            column![
                header,
                container(file_list_view(app, detail, state.selected_file.as_deref()))
                    .height(Length::FillPortion(top_portion)),
                crate::app::horizontal_divider_bar(
                    byteui::theme::color::current().bg,
                    byteui::theme::color::current().bg,
                    Message::RowDragStart,
                ),
                container(diff_pane_view(detail, state.selected_file.as_deref()))
                    .height(Length::FillPortion(bottom_portion)),
            ]
            .height(Length::Fill)
            .into()
        } else {
            container(
                text("选择一个提交查看改动")
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().dim),
            )
            .padding(12)
            .into()
        };

    let left_box = container(left_with_picker).width(Length::FillPortion(list_portion));
    let right_box = container(right).width(Length::FillPortion(content_portion));
    let divider = crate::app::divider_bar(
        crate::app::Divider::GitLogSplit,
        byteui::theme::color::current().bg,
        byteui::theme::color::current().bg,
        Message::ColumnDragStart,
    );
    let body = if mirror {
        row![right_box, divider, left_box]
    } else {
        row![left_box, divider, right_box]
    };
    body.width(Length::Fill)
        .height(Length::Fill)
        .padding(pad)
        .into()
}

/// 右下 diff 内容面板:`selected_file` 对应文件的 patch,逐行染色(复用
/// `diff_render::colored_diff_lines`)。找不到该路径(比如换 commit 那一瞬间
/// `selected_file` 还没跟上新 `detail`)或未选中任何文件时展示占位文案,
/// 不 panic。
fn diff_pane_view<'a>(
    detail: &'a Result<CommitDetail, String>,
    selected_file: Option<&'a str>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Ok(detail) = detail else {
        // 错误态已经在 file_list_view 里展示过一次,这里不重复展示错误
        // 文案,给个中性占位即可。
        return container(iced_widget::Space::new()).into();
    };
    let Some(path) = selected_file else {
        return container(
            text("未选中文件")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
        .padding(8)
        .into();
    };
    let Some(entry) = detail.files.iter().find(|f| f.path == path) else {
        return container(
            text("未选中文件")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
        .padding(8)
        .into();
    };
    let mut content = column![
        text(entry.path.clone())
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().cream)
    ]
    .spacing(4);
    if entry.patch.is_empty() {
        content = content.push(
            text("(无 diff 内容)")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim),
        );
    } else {
        content = content.push(crate::extensions::diff_render::colored_diff_lines(
            &entry.patch,
        ));
    }
    if entry.truncated {
        content = content.push(
            text("… diff 过长,已截断显示")
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
        );
    }
    scrollable(content)
        .direction(scrollable::Direction::Vertical(
            byteui::interaction::scrollbar::scrollbar(),
        ))
        .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style())
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// 左侧面板底部 footbar:完全照抄文件树面板的 `git_footer_bar` 结构——顶部
/// 一条 1px 分隔线 + 一行(左:`GitBranch` 图标 + 当前分支名;右:分支切换
/// chevron),`spacing(6)`、`align_y(Center)`、外层 `padding([6,0])`、背景
/// 透明。分支切换走 `BranchPickerOpen`/`BranchPickerClose`;下拉层仍是左侧
/// 面板局部 `stack!`(`branch_picker_view`)。
fn git_panel_footer_bar<'a>(
    state: &'a State,
    head_branch: Option<&'a str>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let branch_label = text(head_branch.unwrap_or("(无分支)"))
        .size(byteui::theme::font::label())
        .color(byteui::theme::color::current().cream);

    let switch = iced_widget::button(byteui::interaction::icons::view(
        if state.branch_picker_open {
            byteui::interaction::icons::IconKind::ChevronUp
        } else {
            byteui::interaction::icons::IconKind::ChevronDown
        },
        byteui::theme::icon_size::row(),
        byteui::theme::color::current().cream,
    ))
    .on_press_maybe(
        (!state.branch_switch_pending).then_some(if state.branch_picker_open {
            Message::BranchPickerClose
        } else {
            Message::BranchPickerOpen
        }),
    )
    .padding(6)
    .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
        background: None,
        text_color: byteui::theme::color::current().cream,
        border: Border {
            color: byteui::theme::color::current().border,
            width: 0.0,
            radius: 4.0.into(),
        },
        ..iced_widget::button::Style::default()
    });

    let bar = row![
        byteui::interaction::icons::view(
            byteui::interaction::icons::IconKind::GitBranch,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().cream,
        ),
        branch_label,
        iced_widget::space::horizontal(),
        switch,
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);

    let top_line = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..container::Style::default()
        });

    container(column![top_line, bar].spacing(4))
        .width(Length::Fill)
        .padding([6, 0])
        .style(|_t: &iced_widget::Theme| container::Style {
            background: None,
            ..container::Style::default()
        })
        .into()
}

/// 分支下拉展开层:局部 `stack!`(不是 window-wide overlay,只覆盖左侧
/// Git 面板范围),视觉风格照抄 `files.rs::branch_picker_popup`(CARD 底/
/// BORDER 描边/当前分支 GOLD 高亮),但状态完全独立(不读 `files::
/// WorkspaceState`)。`branch_switch_pending` 时全部禁用并显示"切换中…"。
fn branch_picker_view<'a>(
    state: &'a State,
    head_branch: Option<&'a str>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !state.branch_picker_open {
        return iced_widget::Space::new().into();
    }
    // 分支下拉里出现的纯文字态(切换中/暂无分支)不套按钮,直接进 items。
    let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        Vec::new();
    if state.branch_switch_pending {
        items.push(
            text("切换中…")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim)
                .into(),
        );
    }
    if state.branches.is_empty() {
        items.push(
            text("暂无本地分支")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim)
                .into(),
        );
    }
    // dirty(有未提交改动)时锁定除当前分支外的其余分支;切换请求进行中时
    // 全部锁定——跟 `git_panel_footer_bar` 的 `branch_switch_pending` 禁用
    // 语义一致。单项统一走 `crate::chrome::menu::item_row_fill`:同一套 hover/
    // 锁定样式,但整行撑满 Git 面板宽度(窄面板里好用)。
    for name in &state.branches {
        let is_current = Some(name.as_str()) == head_branch;
        let locked = state.branch_switch_pending || (state.dirty && !is_current);
        let color = if is_current {
            byteui::theme::color::current().gold
        } else if locked {
            byteui::theme::color::current().dim
        } else {
            byteui::theme::color::current().body
        };
        let label = if is_current && state.dirty {
            format!("{name} (Uncommitted)")
        } else {
            name.clone()
        };
        items.push(crate::chrome::menu::item_row_fill(
            None,
            label,
            color,
            (!locked && !is_current).then(|| Message::BranchSwitch(name.clone())),
        ));
    }
    // 面板壳走 `crate::chrome::menu::shell`(context_menu 表面 = 文件树右键菜单基准)。
    let panel: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        crate::chrome::menu::shell_frosted(items, Length::Fill);
    // 下拉层锚定在左侧面板底部、footbar 正上方:全高 stack 铺一层透明
    // `dismiss` 用于"点外关闭",面板用 `Space::Fill` 顶到最底,从 footbar
    // 上方弹出(与文件树 `branch_picker_popup` 钉在 git 底栏上方的语义一致,
    // 而非 `height(Shrink)` 时浮到面板顶部)。
    let dismiss = iced_widget::MouseArea::new(
        iced_widget::Space::new()
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::BranchPickerClose);
    let positioned = column![iced_widget::Space::new().height(Length::Fill), panel,]
        .width(Length::Fill)
        .height(Length::Fill);
    iced_widget::stack![dismiss, positioned]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn status_glyph(status: git2::Delta) -> &'static str {
    match status {
        git2::Delta::Added => "+",
        git2::Delta::Deleted => "-",
        git2::Delta::Modified => "M",
        git2::Delta::Renamed => "R",
        git2::Delta::Copied => "C",
        _ => "?",
    }
}

/// commit 时间戳格式化,`YYYY-MM-DD HH:MM:SS`。不引入 `chrono`——用标准库
/// 手工做民用历换算(Howard Hinnant 的 `civil_from_days`,与 `todo.rs` 里
/// 那份同源)。展示的是 UTC(不依赖本地时区,也不引入时区库——commit 列表
/// 的时间戳是纯展示态,UTC 足够)。
fn format_commit_time(unix_secs: i64) -> String {
    let secs = unix_secs.max(0) as u64;
    let days = (secs / 86_400) as i64;
    let secs_of_day = secs % 86_400;
    let (h, m, s) = (
        secs_of_day / 3600,
        (secs_of_day / 60) % 60,
        secs_of_day % 60,
    );
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}")
}

/// Howard Hinnant 的 `civil_from_days` 算法:Unix epoch 起的天数 → (年, 月, 日)。
/// 范围覆盖 1970..=2100,足够 commit 时间戳用。与 `todo.rs` 的同名函数同源
/// (那个是模块私有,不便跨模块复用,这里照抄一份)。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
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

    /// spike 验证核心问题:`gleisbau` 能否对 Dozer 自己这个真实、有分叉/合并
    /// 历史的仓库跑出合理的 commit 列表 + refs 标签 + is_merge 标记。跑
    /// `cargo test -p dozer-app git_log::tests -- --nocapture` 看打印的前 20
    /// 行,人工核对 sha/refs/summary 是否符合直觉。
    #[test]
    fn build_against_real_repo() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crates/dozer-app 应有两层上级目录到仓库根");
        let snapshot =
            build(repo_root, DEFAULT_MAX_COMMITS).expect("gleisbau 应能解析 dozer 自己的仓库");

        assert!(!snapshot.rows.is_empty(), "真实仓库应至少有一个 commit");

        for (row_idx, row) in snapshot.rows.iter().take(20).enumerate() {
            println!(
                "row={row_idx} sha={} merge={} summary={:?}",
                row.short_sha, row.is_merge, row.summary
            );
        }

        // merge commit 理应至少出现一次——Dozer 仓库历史里确实有过 merge
        // (如 2429d15),不是纯线性历史。
        assert!(
            snapshot.rows.iter().any(|r| r.is_merge),
            "200 个 commit 窗口内应能看到至少一个 merge"
        );
    }

    #[test]
    fn build_populates_time_and_is_merge() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crates/dozer-app 应有两层上级目录到仓库根");
        let snapshot =
            build(repo_root, DEFAULT_MAX_COMMITS).expect("gleisbau 应能解析 dozer 自己的仓库");

        // 每一行的 time 都应该是合理的正数(Unix 秒,仓库不可能早于 2020 年)。
        let epoch_2020 = 1_577_836_800_i64;
        for row in &snapshot.rows {
            assert!(
                row.time > epoch_2020,
                "commit time 应晚于 2020-01-01: {}",
                row.time
            );
        }

        // Dozer 仓库历史里确实有过 merge(如 2429d15),is_merge 应该跟
        // 真实的 git parent 数一致(不能拿 `parents.len()` 比——那是按
        // `max_count` 窗口过滤后的 Vec,父提交恰好被窗口截断时两者会不一致,
        // 见 `CommitRow::is_merge` 字段注释)。
        let has_merge_row = snapshot.rows.iter().any(|r| r.is_merge);
        assert!(
            has_merge_row,
            "200 个 commit 窗口内应能看到至少一个 is_merge=true 的行"
        );
        let repo = git2::Repository::open(repo_root).expect("应能打开 dozer 自己的仓库");
        for row in &snapshot.rows {
            let commit = repo
                .find_commit(row.oid)
                .expect("snapshot 里的 oid 应能查到");
            assert_eq!(
                row.is_merge,
                commit.parent_count() >= 2,
                "is_merge 应与真实 git parent 数一致: {}",
                row.short_sha
            );
        }
    }

    #[test]
    fn build_marks_head_branch_and_labels() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crates/dozer-app 应有两层上级目录到仓库根");
        let snapshot = build(repo_root, 50).expect("gleisbau 应能解析 dozer 自己的仓库");

        // 当前 HEAD 分支(dogfooding 仓库跑测试时几乎总在 main,但不强行假设
        // 分支名——只断言"存在且第 0 行的 refs 里能找到它")。
        let head = snapshot.head_branch.clone().expect("应能取到当前分支名");
        let first = &snapshot.rows[0];
        assert!(
            first
                .refs
                .iter()
                .any(|r| r.name == head && r.kind == RefKind::LocalBranch),
            "HEAD 所在行的 refs 应包含当前分支: {:?}",
            first.refs
        );
    }

    #[test]
    fn build_respects_max_count() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let small = build(repo_root, 5).unwrap();
        let bigger = build(repo_root, 50).unwrap();
        assert!(small.rows.len() <= 5);
        assert!(bigger.rows.len() > small.rows.len());
        assert_eq!(small.max_count(), 5);
        assert_eq!(bigger.max_count(), 50);
        // 重算稳定性:更大窗口的前 N 行应与小窗口结果一致(同一份历史,只是
        // 走得更远,不应该导致已经算出来的部分变形)。
        for (a, b) in small.rows.iter().zip(bigger.rows.iter()) {
            assert_eq!(a.short_sha, b.short_sha);
            assert_eq!(a.is_merge, b.is_merge);
        }
    }

    #[test]
    fn commit_detail_uses_first_parent_diff() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crates/dozer-app 应有两层上级目录到仓库根");
        // 取最新提交(当前 HEAD 也就是第 0 行),真实仓库上它总有一个 parent,
        // diff 应该有 ≥1 个文件(至少动过 `git_log.rs` 或本测试自身)。
        // 不用 `max_count=1`:gleisbau 的 `with_max_count(1)` 会解析成 0 行,
        // 得给足额度(用面板默认值),只取第 0 行即 HEAD。
        let snapshot = build(repo_root, DEFAULT_MAX_COMMITS).expect("应能解析 dozer 自己的仓库");
        let head_oid = snapshot.rows[0].oid;
        let detail = commit_detail(repo_root, head_oid).expect("HEAD 提交的 diff 应能算出");
        assert!(!detail.files.is_empty(), "HEAD 对 parent 至少改动一个文件");
        for f in &detail.files {
            assert!(!f.path.is_empty());
        }
        // 至少一个文件真的算出了 diff 文本——否则 `detail_view` 渲染的 patch
        // 永远是空字符串,回归成"只有文件列表看不到 diff"(code review 发现)。
        assert!(
            detail.files.iter().any(|f| !f.patch.is_empty()),
            "至少一个改动文件应该有非空 patch 文本"
        );
    }

    /// tempdir 里造一个只有一次提交(根提交)的真 git 仓库,同 `delivery.rs`
    /// 的 `mkrepo` 惯例(用真 `git` CLI,不手搓 git2 底层对象)。
    fn mkrepo_with_one_commit() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let st = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(st.status.success(), "git {args:?}: {st:?}");
        };
        git(&["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        std::fs::write(repo.join("b.txt"), "two\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "root"]);
        (dir, repo)
    }

    #[test]
    fn commit_detail_handles_root_commit_as_all_added() {
        let (_dir, repo) = mkrepo_with_one_commit();
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).expect("应能解析临时仓库");
        assert_eq!(snapshot.rows.len(), 1, "临时仓库只有一个根提交");
        let root_oid = snapshot.rows[0].oid;

        let detail = commit_detail(&repo, root_oid).expect("根提交相对空树应该也能算出 diff");
        assert_eq!(detail.files.len(), 2, "根提交里的两个文件都应该出现");
        assert!(
            detail.files.iter().all(|f| f.status == git2::Delta::Added),
            "根提交相对空树,所有文件都该是 Added: {:?}",
            detail.files.iter().map(|f| f.status).collect::<Vec<_>>()
        );
        assert!(
            detail.files.iter().all(|f| !f.patch.is_empty()),
            "根提交的每个文件都该有非空 patch 文本"
        );
    }

    #[test]
    fn ref_labels_text_head_marker_and_join() {
        let refs = vec![
            RefLabel {
                name: "main".into(),
                kind: RefKind::LocalBranch,
            },
            RefLabel {
                name: "origin/main".into(),
                kind: RefKind::RemoteBranch,
            },
        ];
        // HEAD 在 main,只给 main 加箭头;远端分支不加。
        assert_eq!(ref_labels_text(&refs, Some("main")), "[→main][origin/main]");
        // 没有匹配的 HEAD 分支,全都不加箭头。
        assert_eq!(
            ref_labels_text(&refs, Some("feature")),
            "[main][origin/main]"
        );
        // 空 refs(一般提交)返回空串。
        assert_eq!(ref_labels_text(&[], Some("main")), "");
    }

    #[test]
    fn format_commit_time_matches_expected_layout() {
        // 2026-08-17 09:22:31 UTC(固定输入 → 固定输出;实现是 UTC,无时区歧义)。
        assert_eq!(format_commit_time(1_786_958_551), "2026-08-17 09:22:31");
        // epoch 0 边界。
        assert_eq!(format_commit_time(0), "1970-01-01 00:00:00");
        // 负数夹到 epoch。
        assert_eq!(format_commit_time(-5), "1970-01-01 00:00:00");
    }

    fn snapshot_at(repo_path: &Path, max_count: usize) -> GitLogSnapshot {
        GitLogSnapshot {
            repo_path: repo_path.to_path_buf(),
            rows: Vec::new(),
            head_branch: None,
            max_count,
        }
    }

    #[tokio::test]
    async fn select_commit_sets_selected_and_clears_detail() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            detail: Some(Ok(CommitDetail { files: Vec::new() })),
            ..State::default()
        };
        let oid = git2::Oid::from_bytes(&[1; 20]).unwrap();
        let handle = tokio::runtime::Handle::current();
        let result = update(&mut state, Message::SelectCommit(oid), &handle, |_| {});
        assert_eq!(state.selected, Some(oid));
        assert!(state.detail.is_none());
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn select_commit_without_cache_sets_selected_but_spawns_nothing() {
        let mut state = State::default();
        let oid = git2::Oid::from_bytes(&[2; 20]).unwrap();
        let handle = tokio::runtime::Handle::current();
        let result = update(&mut state, Message::SelectCommit(oid), &handle, |_| {
            panic!("无缓存时不该 emit 任何消息");
        });
        assert_eq!(state.selected, Some(oid));
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detail_loaded_writes_when_repo_path_and_selected_match() {
        let repo_path = PathBuf::from("/tmp/repo");
        let oid = git2::Oid::from_bytes(&[3; 20]).unwrap();
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            selected: Some(oid),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let result = update(
            &mut state,
            Message::DetailLoaded(repo_path, oid, Ok(CommitDetail { files: Vec::new() })),
            &handle,
            |_| {},
        );
        match &state.detail {
            Some(Ok(detail)) => assert!(detail.files.is_empty()),
            other => panic!("期望 Some(Ok(空 CommitDetail)),实际 {other:?}"),
        }
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detail_loaded_discarded_when_repo_path_mismatches() {
        let mut state = State {
            cache: Some(snapshot_at(Path::new("/tmp/a"), 10)),
            selected: Some(git2::Oid::from_bytes(&[4; 20]).unwrap()),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DetailLoaded(
                PathBuf::from("/tmp/b"),
                git2::Oid::from_bytes(&[4; 20]).unwrap(),
                Ok(CommitDetail { files: Vec::new() }),
            ),
            &handle,
            |_| {},
        );
        assert!(state.detail.is_none(), "仓库路径对不上,结果应被丢弃");
    }

    #[tokio::test]
    async fn detail_loaded_discarded_when_selected_mismatches() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            selected: Some(git2::Oid::from_bytes(&[5; 20]).unwrap()),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DetailLoaded(
                repo_path,
                git2::Oid::from_bytes(&[6; 20]).unwrap(),
                Ok(CommitDetail { files: Vec::new() }),
            ),
            &handle,
            |_| {},
        );
        assert!(state.detail.is_none(), "选中的提交对不上,结果应被丢弃");
    }

    #[tokio::test]
    async fn select_file_sets_selected_file() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let result = update(
            &mut state,
            Message::SelectFile("src/main.rs".to_string()),
            &handle,
            |_| {},
        );
        assert_eq!(state.selected_file.as_deref(), Some("src/main.rs"));
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detail_loaded_preselects_first_file() {
        let repo_path = PathBuf::from("/tmp/repo");
        let oid = git2::Oid::from_bytes(&[10; 20]).unwrap();
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            selected: Some(oid),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let detail = CommitDetail {
            files: vec![
                DiffFileEntry {
                    path: "a.rs".to_string(),
                    status: git2::Delta::Modified,
                    patch: "+x".to_string(),
                    truncated: false,
                },
                DiffFileEntry {
                    path: "b.rs".to_string(),
                    status: git2::Delta::Added,
                    patch: "+y".to_string(),
                    truncated: false,
                },
            ],
        };
        update(
            &mut state,
            Message::DetailLoaded(repo_path, oid, Ok(detail)),
            &handle,
            |_| {},
        );
        assert_eq!(
            state.selected_file.as_deref(),
            Some("a.rs"),
            "detail 落地后应预选第一个改动文件"
        );
    }

    #[tokio::test]
    async fn detail_loaded_with_no_files_clears_selected_file() {
        let repo_path = PathBuf::from("/tmp/repo");
        let oid = git2::Oid::from_bytes(&[11; 20]).unwrap();
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            selected: Some(oid),
            selected_file: Some("stale.rs".to_string()),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DetailLoaded(repo_path, oid, Ok(CommitDetail { files: Vec::new() })),
            &handle,
            |_| {},
        );
        assert_eq!(
            state.selected_file, None,
            "无改动文件时应清空 selected_file,不留旧值"
        );
    }

    #[tokio::test]
    async fn select_commit_clears_selected_file() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            selected_file: Some("old.rs".to_string()),
            ..State::default()
        };
        let oid = git2::Oid::from_bytes(&[12; 20]).unwrap();
        let handle = tokio::runtime::Handle::current();
        update(&mut state, Message::SelectCommit(oid), &handle, |_| {});
        assert_eq!(
            state.selected_file, None,
            "切 commit 时应先清空旧的 selected_file(等新 detail 落地才重选)"
        );
    }

    #[tokio::test]
    async fn branch_picker_open_and_close_toggle_flag() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        update(&mut state, Message::BranchPickerOpen, &handle, |_| {});
        assert!(state.branch_picker_open);
        update(&mut state, Message::BranchPickerClose, &handle, |_| {});
        assert!(!state.branch_picker_open);
    }

    #[tokio::test]
    async fn branches_loaded_lands_when_repo_path_matches_cache() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::BranchesLoaded(repo_path, vec!["main".to_string(), "dev".to_string()], true),
            &handle,
            |_| {},
        );
        assert_eq!(state.branches, vec!["main".to_string(), "dev".to_string()]);
        assert!(state.dirty, "dirty 标记应随分支列表一起落地");
    }

    #[tokio::test]
    async fn branches_loaded_discarded_when_repo_path_mismatches() {
        let mut state = State {
            cache: Some(snapshot_at(Path::new("/tmp/a"), 10)),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::BranchesLoaded(PathBuf::from("/tmp/b"), vec!["main".to_string()], true),
            &handle,
            |_| {},
        );
        assert!(state.branches.is_empty(), "仓库路径对不上,不该落地");
        assert!(!state.dirty, "仓库路径对不上,dirty 也不该落地");
    }

    #[tokio::test]
    async fn branch_switch_done_ok_clears_pending_and_closes_picker() {
        let mut state = State {
            branch_picker_open: true,
            branch_switch_pending: true,
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::BranchSwitchDone(Ok(())),
            &handle,
            |_| {},
        );
        assert!(!state.branch_picker_open);
        assert!(!state.branch_switch_pending);
        assert!(state.error.is_none());
    }

    #[tokio::test]
    async fn branch_switch_done_err_sets_error_and_clears_pending() {
        let mut state = State {
            branch_switch_pending: true,
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::BranchSwitchDone(Err("checkout 失败".to_string())),
            &handle,
            |_| {},
        );
        assert!(!state.branch_switch_pending);
        assert_eq!(state.error.as_deref(), Some("checkout 失败"));
    }

    #[tokio::test]
    async fn snapshot_loaded_lands_when_pending_matches() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            pending: Some((repo_path.clone(), 10)),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let result = update(
            &mut state,
            Message::SnapshotLoaded(repo_path.clone(), 10, Ok(snapshot_at(&repo_path, 10))),
            &handle,
            |_| {},
        );
        assert!(state.cache.is_some());
        assert!(state.error.is_none());
        assert!(state.pending.is_none());
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn snapshot_loaded_discarded_when_pending_mismatches() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            pending: Some((repo_path.clone(), 10)),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::SnapshotLoaded(repo_path.clone(), 20, Ok(snapshot_at(&repo_path, 20))),
            &handle,
            |_| {},
        );
        assert!(state.cache.is_none(), "max_count 对不上,不该落地");
        assert_eq!(state.pending, Some((repo_path, 10)), "pending 也不该被清掉");
    }

    #[tokio::test]
    async fn snapshot_loaded_error_clears_cache_and_sets_error() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            pending: Some((repo_path.clone(), 10)),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::SnapshotLoaded(repo_path, 10, Err("boom".to_string())),
            &handle,
            |_| {},
        );
        assert!(state.cache.is_none());
        assert_eq!(state.error.as_deref(), Some("boom"));
    }

    #[test]
    fn cache_max_count_reflects_current_cache() {
        let with_cache = State {
            cache: Some(snapshot_at(Path::new("/tmp/repo"), 37)),
            ..State::default()
        };
        assert_eq!(with_cache.cache_max_count(), 37);
        assert_eq!(State::default().cache_max_count(), DEFAULT_MAX_COMMITS);
    }

    #[test]
    fn commit_visible_count_starts_at_one_page() {
        assert_eq!(State::default().commit_visible_count(), COMMIT_PAGE_SIZE);
    }

    #[tokio::test]
    async fn commit_list_more_advances_one_page_per_click() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        assert!(update(&mut state, Message::CommitListMore, &handle, |_| {}).is_none());
        assert_eq!(state.commit_visible_count(), 2 * COMMIT_PAGE_SIZE);
        assert!(update(&mut state, Message::CommitListMore, &handle, |_| {}).is_none());
        assert_eq!(state.commit_visible_count(), 3 * COMMIT_PAGE_SIZE);
    }

    #[test]
    fn cache_repo_path_reflects_cache_presence() {
        let repo_path = PathBuf::from("/tmp/repo");
        let with_cache = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            ..State::default()
        };
        assert_eq!(with_cache.cache_repo_path(), Some(repo_path.as_path()));
        assert_eq!(State::default().cache_repo_path(), None);
    }

    #[tokio::test]
    async fn request_refresh_resets_selection_and_records_pending() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            selected: Some(git2::Oid::from_bytes(&[8; 20]).unwrap()),
            detail: Some(Ok(CommitDetail { files: Vec::new() })),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        request_refresh(&mut state, repo_path.clone(), 50, &handle, |_| {});
        assert!(state.selected.is_none());
        assert!(state.detail.is_none());
        assert_eq!(state.pending, Some((repo_path, 50)));
    }

    fn make_commit_row(summary: &str) -> CommitRow {
        CommitRow {
            short_sha: "abc1234".to_string(),
            summary: summary.to_string(),
            author: None,
            refs: Vec::new(),
            oid: git2::Oid::from_bytes(&[0; 20]).unwrap(),
            time: 0,
            is_merge: false,
        }
    }

    #[test]
    fn filter_commit_rows_empty_query_returns_all() {
        let rows = vec![
            make_commit_row("fix login bug"),
            make_commit_row("refactor parser"),
        ];
        assert_eq!(filter_commit_rows(&rows, "").len(), 2);
    }

    #[test]
    fn filter_commit_rows_matches_summary_case_insensitive_substring() {
        let rows = vec![
            make_commit_row("Fix Login Bug"),
            make_commit_row("refactor parser"),
        ];
        let filtered = filter_commit_rows(&rows, "login");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].summary, "Fix Login Bug");
    }

    #[test]
    fn filter_commit_rows_no_match_yields_empty() {
        let rows = vec![make_commit_row("fix login bug")];
        assert!(filter_commit_rows(&rows, "不存在").is_empty());
    }

    #[tokio::test]
    async fn search_submit_commits_draft_and_resets_pages() {
        let handle = tokio::runtime::Handle::current();
        let mut state = State {
            pages: 3,
            search_draft: "login".to_string(),
            ..State::default()
        };
        assert!(update(&mut state, Message::SearchSubmit, &handle, |_| {}).is_none());
        assert_eq!(state.search, "login");
        assert_eq!(state.pages, 0);
    }
}
