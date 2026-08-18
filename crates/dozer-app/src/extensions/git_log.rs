// crates/dozer-app/src/git_log.rs
//! spike(2026-08-06): 验证第三方 Rust 库 `gleisbau`(git-graph 的布局引擎,
//! 拆出来的独立 crate)能否喂出可在 iced Canvas 里原生画出的提交图数据,
//! 探路"要不要在 Dozer 里做一个类似 VS Code Git Graph 的面板"。
//!
//! 只验证数据链路是否走得通,不追求 curve/fork 的像素级还原:每条 track
//! 画一根直线,commit 是线上的一个圆点,父子关系用直线连接(不是贝塞尔)。
//! 验证通过、决定转正时,再补动画/交互/性能优化。
use crate::app::{App, HoverId};
use crate::delivery::WorktreeInfo;
use crate::theme;
use iced_widget::core::alignment;
use iced_widget::core::{Border, Element, Font, Length};
use iced_widget::{MouseArea, column, container, row, scrollable, text};
use std::path::{Path, PathBuf};

/// 首次打开面板拉多少个 commit——够看出分叉/合并的形状,又不至于让
/// revwalk + 分支归属分析在大仓库上明显卡顿。"加载更多"每次在当前基础上
/// 加这么多再整份重算(gleisbau 的 API 是"从头按 max_count 走一遍
/// revwalk",没有增量/游标接口,重算是唯一选项——见 build() 文档)。
pub const DEFAULT_MAX_COMMITS: usize = 200;
pub const LOAD_MORE_STEP: usize = 200;

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
    /// 这份快照实际请求的 `max_count`("加载更多"算下一次请求值用)。
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
/// gleisbau 没有增量/游标 API,"加载更多"就是拿更大的 `max_count` 再整份
/// 跑一遍,见 [`LOAD_MORE_STEP`]。
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
            Ok(CommitRow {
                short_sha,
                summary,
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
    LoadMore,
    /// 点 worktree 条带里的其它 worktree,切过去。内核(`app.rs`)在
    /// `Message::GitLog` 分发里拦截,转成 `Message::ProjectTabOpen`,
    /// 不会转发到 `update`(见其 `unreachable!` 分支)。
    ProjectTabOpen(PathBuf),
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
    /// 点某个分支——内核截获处理(同 `LoadMore`/`ProjectTabOpen` 的既有
    /// 例外模式),不会转发到 `update`(见其 `unreachable!` 分支)。
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
    /// "加载更多"发起前记下的选中提交,新快照落地后据此还原选中态。
    restore_after_load: Option<git2::Oid>,
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
}

impl State {
    /// "加载更多"按钮下一个请求的 `max_count`:有缓存则在当前基础上
    /// `+ LOAD_MORE_STEP`,否则回落 `DEFAULT_MAX_COMMITS`。
    pub fn next_load_more_count(&self) -> usize {
        self.cache
            .as_ref()
            .map(|c| c.max_count() + LOAD_MORE_STEP)
            .unwrap_or(DEFAULT_MAX_COMMITS)
    }

    /// 当前缓存的 `max_count`(无缓存则回落 `DEFAULT_MAX_COMMITS`)——用于
    /// "内容不变、只是要重新拉一遍"的场景(`.git` 引用变化触发的重建),
    /// 跟"加载更多"要的 `next_load_more_count()`(会 `+LOAD_MORE_STEP`)
    /// 是两回事,内核代码里不要混用。
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

    /// 当前选中的提交(`None` = 未选中)。内核发起"加载更多"前需要先读一次
    /// 这个值——`request_refresh` 会把它清空,内核得自己先存一份,请求
    /// 落地后再用 `set_restore_after_load` 传回来。
    pub fn selected(&self) -> Option<git2::Oid> {
        self.selected
    }

    /// `request_refresh` 落地新快照之前,内核用这个把"发起刷新前选中的
    /// 提交"记下来,新快照真正落地(`SnapshotLoaded` 处理完)时
    /// `update()` 会据此还原选中态(见其返回值 `Some(Message::SelectCommit)`
    /// 那条路径)。
    pub fn set_restore_after_load(&mut self, oid: Option<git2::Oid>) {
        self.restore_after_load = oid;
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
/// `LoadMore` 需要内核才知道的"当前聚焦项目路径",不在这里处理——传进来
/// 会直接 panic,调用方(`workspace.rs`)必须在转发前先拦掉这一种(见
/// 设计文档"内核转发不是无差别盲转"）。
///
/// 返回值:`Some(next)` = 这次处理还产生了一条要递归分发的后续消息(目前
/// 只有 `SnapshotLoaded` 落地后恢复选中提交这一种情况)。
pub fn update(
    state: &mut State,
    msg: Message,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) -> Option<Message> {
    match msg {
        // 卡片悬停由内核 `App::update` 拦截转发到 `set_hover`,不会到这。
        Message::Hover(_, _) => None,
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
            state.restore_after_load.take().map(Message::SelectCommit)
        }
        Message::LoadMore => {
            unreachable!(
                "LoadMore 由内核在 Message::GitLog 分支里直接处理(需要仓库路径),不会转发到这里"
            )
        }
        Message::ProjectTabOpen(_) => {
            unreachable!(
                "ProjectTabOpen 由内核在 Message::GitLog 分支里直接处理(切到对应 worktree),不会转发到这里"
            )
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

/// 同仓库其它 worktree 压成一行小字,标示当前提交图对应哪个 worktree 上下文。
/// 主 worktree + N 个链接 worktree 各自的分支会散落在同一条图上,这个条带帮
/// 用户分辨 `[→main]` 到底指谁。它渲染在文件夹路径下方(见 `view` 的
/// `header` 之后)。"本工作区"是状态展示,不是甲方动作,不能用 GOLD(CLAUDE.md
/// 硬性裁决,GOLD 专属甲方动作)——真正的动作是点其它 worktree 切过去,那些
/// 按钮才该用 GOLD。
fn worktree_strip<'a>(
    worktrees: &'a [WorktreeInfo],
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let others = worktrees
        .iter()
        .filter(|w| !w.is_current)
        .collect::<Vec<_>>();
    let mut chips: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = vec![];
    for o in others {
        let label = match (&o.branch, o.missing) {
            (Some(b), true) => format!("{b} (缺失)"),
            (Some(b), _) => b.clone(),
            (None, true) => "无分支 (缺失)".into(),
            (None, _) => "无分支".into(),
        };
        if o.missing {
            // 目录已经不在磁盘上,没有可切换的目标——保留纯展示文案。
            chips.push(
                text(label)
                    .size(theme::font::caption())
                    .color(byteui::theme::color::current().dim)
                    .into(),
            );
        } else {
            chips.push(
                iced_widget::button(
                    text(label)
                        .size(theme::font::caption())
                        .color(byteui::theme::color::current().gold),
                )
                .on_press(Message::ProjectTabOpen(o.path.clone()))
                .padding(0)
                .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                    background: None,
                    text_color: byteui::theme::color::current().gold,
                    ..iced_widget::button::Style::default()
                })
                .into(),
            );
        }
    }
    if chips.is_empty() {
        return container(iced_widget::Space::new())
            .height(Length::Shrink)
            .into();
    }
    row![
        iced_widget::Row::with_children(chips).spacing(12),
        iced_widget::Space::new().width(Length::Fill),
    ]
    .padding([4, 8])
    .width(Length::Fill)
    .height(Length::Shrink)
    .into()
}

/// 异步重建 Git Log 快照,`max_count` 由调用方决定(打开面板/引用变化用
/// `DEFAULT_MAX_COMMITS`,"加载更多"用 `State::next_load_more_count()`)。
/// 现有 `App::spawn_git_log_refresh` 的搬家版本,行为不变。
pub fn request_refresh(
    state: &mut State,
    repo_path: PathBuf,
    max_count: usize,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    state.selected = None;
    state.detail = None;
    state.restore_after_load = None;
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
/// "架构与数据流"第 6 节)。每行上下两行:上行图标(普通/合并)+ short_sha +
/// 时间戳 + refs 标签;下行 summary。整行可点选中(`Message::SelectCommit`),
/// 选中态统一卡片样式(对齐 Todo/Files 面板既有选中行视觉语言)。
fn commit_list_view<'a>(
    app: &App,
    snapshot: &'a GitLogSnapshot,
    selected: Option<git2::Oid>,
    head_branch: Option<&'a str>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut list = column![].spacing(8);
    for (i, row) in snapshot.rows.iter().enumerate() {
        let is_selected = selected == Some(row.oid);
        let icon_kind = if row.is_merge {
            byteui::interaction::icons::IconKind::GitMerge
        } else {
            byteui::interaction::icons::IconKind::GitCommitVertical
        };
        let refs_prefix = ref_labels_text(&row.refs, head_branch);
        // 上行:图标 + short_sha + 时间戳 + refs 标签
        let mut head_line = row![
            byteui::interaction::icons::view(
                icon_kind,
                crate::theme::icon_size::row(),
                byteui::theme::color::current().dim
            ),
            text(row.short_sha.clone())
                .size(theme::font::caption())
                .color(byteui::theme::color::current().dim)
                .font(Font::MONOSPACE),
            text(format_commit_time(row.time))
                .size(theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center);
        if !refs_prefix.is_empty() {
            head_line = head_line.push(
                text(refs_prefix)
                    .size(theme::font::caption_sm())
                    .color(byteui::theme::color::current().cyan),
            );
        }
        // 下行:summary 主体——左缩进到与 commit id(short_sha)对齐:跳过
        // 上行图标宽度 + 图标与 sha 之间的 `spacing(8)`。
        let comment_indent = crate::theme::icon_size::row() + 8.0;
        let summary_line = container(
            text(row.summary.clone())
                .size(theme::font::caption())
                .color(byteui::theme::color::current().cream),
        )
        .padding(iced_widget::core::Padding {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: comment_indent,
        });
        let line = column![head_line, summary_line].spacing(4);
        // 统一卡片样式:选中/一般/hover 三态(选中=金边、hover=金边+填充、
        // 一般态=描边),不再用左侧 3px 金竖条表示选中。内部间距与卡片内边距
        // 对齐 Agent 面板的 agent 卡片(`workspace.rs::agent_card`:行距 4、
        // `padding(10)`),避免 commit 卡片内部过挤。
        let hovered = app.hover_progress(HoverId::Commit(i)) > 0.0;
        let inner = container(line).padding(10).width(Length::Fill).style(
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
            .on_enter(Message::Hover(HoverId::Commit(i), true))
            .on_exit(Message::Hover(HoverId::Commit(i), false))
            .on_press(Message::SelectCommit(row.oid));
        list = list.push(area);
    }
    scrollable(list)
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
                .size(theme::font::caption())
                .color(byteui::theme::color::current().red),
        )
        .padding(8)
        .into(),
        Ok(detail) if detail.files.is_empty() => {
            container(
            text("无文件改动")
                .size(theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
                .padding(8)
                .into()
        }
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
                        .size(theme::font::caption())
                        .color(color)
                        .width(18),
                    text(f.path.clone())
                        .size(theme::font::caption())
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
    worktrees: &'a [WorktreeInfo],
    git_log_split: f32,
    git_log_file_diff_split: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let error = state.error.as_deref();
    // 面板内边距对齐文件树面板(`project_pane` region):header/body/footer
    // 的分隔线与内容容器统一按同一水平 inset 排布,避免 Git 面板自己另起
    // 一套 → 0 的 padding 与文件树/项目面板(8)错位。
    let pad = theme::region::project_pane().padding;
    let head =
        crate::homespace::home_panel_head(byteui::interaction::icons::IconKind::GitGraph, "Git");

    let loading = state.pending.is_some();
    let Some(snapshot) = state.cache.as_ref() else {
        let text_content = if loading {
            "加载中…"
        } else {
            "未打开项目"
        };
        return container(
            column![
                head,
                text(text_content)
                    .size(theme::font::caption())
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
                    .size(theme::font::caption())
                    .color(byteui::theme::color::current().dim)
            ]
            .spacing(8)
            .padding(pad),
        )
        .into();
    }

    let head_branch = snapshot.head_branch();
    // 顶部不再显示"本工作区:<分支>"(与 footbar 的分支名重复);仅当存在
    // 其它(关联)工作树时才把切换条挂上,避免单工作树时凭空多一条间隙。
    let mut left = column![head].spacing(8);
    if worktrees.iter().any(|w| !w.is_current) {
        left = left.push(worktree_strip(worktrees));
    }
    if let Some(err) = error {
        left = left.push(
            text(format!("git log 读取失败: {err}"))
                .size(theme::font::caption())
                .color(byteui::theme::color::current().red),
        );
    }
    left = left.push(commit_list_view(app, snapshot, state.selected, head_branch));
    left = left.push(git_panel_footer_bar(state, head_branch, loading));
    let left_with_picker = iced_widget::stack![
        container(left).width(Length::Fill).height(Length::Fill),
        branch_picker_view(state, head_branch),
    ];

    let (list_portion, content_portion) = crate::workspace::split_portions(git_log_split);
    let right: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if let Some(detail) = state.detail.as_ref() {
            let (top_portion, bottom_portion) =
                crate::workspace::split_portions(git_log_file_diff_split);
            column![
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
                .size(theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
                .padding(12)
                .into()
        };

    row![
        container(left_with_picker).width(Length::FillPortion(list_portion)),
        crate::app::divider_bar(
            crate::app::Divider::GitLogSplit,
            byteui::theme::color::current().bg,
            byteui::theme::color::current().bg,
            Message::ColumnDragStart,
        ),
        container(right).width(Length::FillPortion(content_portion)),
    ]
    .width(Length::Fill)
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
                .size(theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
            .padding(8)
            .into();
    };
    let Some(entry) = detail.files.iter().find(|f| f.path == path) else {
        return container(
            text("未选中文件")
                .size(theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
            .padding(8)
            .into();
    };
    let mut content = column![
        text(entry.path.clone())
            .size(theme::font::caption())
            .color(byteui::theme::color::current().dim)
    ]
    .spacing(4);
    if entry.patch.is_empty() {
        content = content.push(
            text("(无 diff 内容)")
                .size(theme::font::caption())
                .color(byteui::theme::color::current().dim),
        );
    } else {
        content = content.push(crate::diff_render::colored_diff_lines(&entry.patch));
    }
    if entry.truncated {
        content = content.push(
            text("… diff 过长,已截断显示")
                .size(theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
        );
    }
    scrollable(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// 左侧面板底部固定展示:当前分支名 + 展开箭头,点击发
/// 左侧面板底部 footbar:完全照抄文件树面板的 `git_footer_bar` 结构——顶部
/// 一条 1px 分隔线 + 一行(左:`GitBranch` 图标 + 当前分支名;右:加载更多 +
/// 分支切换 chevron),`spacing(6)`、`align_y(Center)`、外层 `padding([6,0])`、
/// 背景透明。分支切换走 `BranchPickerOpen`/`BranchPickerClose`;下拉层仍是
/// 左侧面板局部 `stack!`(`branch_picker_view`)。
fn git_panel_footer_bar<'a>(
    state: &'a State,
    head_branch: Option<&'a str>,
    loading: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let branch_label = text(head_branch.unwrap_or("(无分支)"))
        .size(theme::font::label())
        .color(byteui::theme::color::current().cream);

    let switch = iced_widget::button(byteui::interaction::icons::view(
        if state.branch_picker_open {
            byteui::interaction::icons::IconKind::ChevronUp
        } else {
            byteui::interaction::icons::IconKind::ChevronDown
        },
        crate::theme::icon_size::row(),
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

    let load_more = iced_widget::button(
        text("加载更多提交 (+200)")
            .size(theme::font::label())
            .color(byteui::theme::color::current().cream),
    )
    .on_press_maybe((!loading).then_some(Message::LoadMore))
    .padding([4, 10])
    .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
        background: Some(byteui::theme::color::current().bg.into()),
        text_color: byteui::theme::color::current().cream,
        border: Border {
            color: byteui::theme::color::current().border,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..iced_widget::button::Style::default()
    });

    let bar = row![
        byteui::interaction::icons::view(
            byteui::interaction::icons::IconKind::GitBranch,
            crate::theme::icon_size::row(),
            byteui::theme::color::current().cream,
        ),
        branch_label,
        iced_widget::space::horizontal(),
        load_more,
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
                .size(theme::font::body())
                .color(byteui::theme::color::current().dim)
                .into(),
        );
    }
    if state.branches.is_empty() {
        items.push(
            text("暂无本地分支")
                .size(theme::font::body())
                .color(byteui::theme::color::current().dim)
                .into(),
        );
    }
    // dirty(有未提交改动)时锁定除当前分支外的其余分支;切换请求进行中时
    // 全部锁定——跟 `git_panel_footer_bar` 的 `branch_switch_pending` 禁用
    // 语义一致。单项统一走 `crate::menu::item_row_fill`:同一套 hover/
    // 锁定样式,但整行撑满 Git 面板宽度(窄面板里好用)。
    for name in &state.branches {
        let is_current = Some(name.as_str()) == head_branch;
        let locked = state.branch_switch_pending || (state.dirty && !is_current);
        let color = if is_current {
            byteui::theme::color::current().gold
        } else if locked {
            byteui::theme::color::current().dim
        } else {
            byteui::theme::color::current().cream
        };
        let label = if is_current && state.dirty {
            format!("{name} (Uncommitted)")
        } else {
            name.clone()
        };
        items.push(crate::menu::item_row_fill(
            None,
            label,
            color,
            (!locked && !is_current).then(|| Message::BranchSwitch(name.clone())),
        ));
    }
    // 面板壳走 `crate::menu::shell`(context_menu 表面 = 文件树右键菜单基准)。
    let panel: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        crate::menu::shell(items, Length::Fill);
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
    let positioned = column![
        iced_widget::Space::new().height(Length::Fill),
        panel,
    ]
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
        assert!(
            result.is_none(),
            "restore_after_load 为 None 时不该产生后续消息"
        );
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

    #[tokio::test]
    async fn snapshot_loaded_returns_select_commit_when_restore_after_load_set() {
        let repo_path = PathBuf::from("/tmp/repo");
        let oid = git2::Oid::from_bytes(&[7; 20]).unwrap();
        let mut state = State {
            pending: Some((repo_path.clone(), 10)),
            restore_after_load: Some(oid),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let result = update(
            &mut state,
            Message::SnapshotLoaded(repo_path.clone(), 10, Ok(snapshot_at(&repo_path, 10))),
            &handle,
            |_| {},
        );
        match result {
            Some(Message::SelectCommit(got)) => assert_eq!(got, oid),
            other => panic!("期望 Some(SelectCommit(oid)),实际 {other:?}"),
        }
        assert!(state.restore_after_load.is_none(), "取用后应清空");
    }

    #[test]
    fn next_load_more_count_with_cache_adds_step() {
        let state = State {
            cache: Some(snapshot_at(Path::new("/tmp/repo"), 200)),
            ..State::default()
        };
        assert_eq!(state.next_load_more_count(), 200 + LOAD_MORE_STEP);
    }

    #[test]
    fn next_load_more_count_without_cache_falls_back_to_default() {
        let state = State::default();
        assert_eq!(state.next_load_more_count(), DEFAULT_MAX_COMMITS);
    }

    #[test]
    fn cache_max_count_reflects_current_cache_without_adding_step() {
        let with_cache = State {
            cache: Some(snapshot_at(Path::new("/tmp/repo"), 37)),
            ..State::default()
        };
        assert_eq!(
            with_cache.cache_max_count(),
            37,
            "不该像 next_load_more_count 那样 +LOAD_MORE_STEP"
        );
        assert_eq!(State::default().cache_max_count(), DEFAULT_MAX_COMMITS);
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

    #[test]
    fn selected_and_set_restore_after_load_roundtrip() {
        let mut state = State::default();
        assert_eq!(state.selected(), None);
        let oid = git2::Oid::from_bytes(&[9; 20]).unwrap();
        state.selected = Some(oid);
        assert_eq!(state.selected(), Some(oid));
        state.set_restore_after_load(Some(oid));
        assert_eq!(state.restore_after_load, Some(oid));
        state.set_restore_after_load(None);
        assert_eq!(state.restore_after_load, None);
    }

    #[tokio::test]
    async fn request_refresh_resets_selection_and_records_pending() {
        let repo_path = PathBuf::from("/tmp/repo");
        let mut state = State {
            selected: Some(git2::Oid::from_bytes(&[8; 20]).unwrap()),
            detail: Some(Ok(CommitDetail { files: Vec::new() })),
            restore_after_load: Some(git2::Oid::from_bytes(&[8; 20]).unwrap()),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        request_refresh(&mut state, repo_path.clone(), 50, &handle, |_| {});
        assert!(state.selected.is_none());
        assert!(state.detail.is_none());
        assert!(state.restore_after_load.is_none());
        assert_eq!(state.pending, Some((repo_path, 50)));
    }
}
