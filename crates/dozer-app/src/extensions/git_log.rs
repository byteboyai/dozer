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
    ///
    /// CodeMirror diff 接管后,`patch`/`truncated` 不再是 diff 面板的正文来源
    /// (正文改由 `old_blob`/`new_blob` 读出的双侧文本经 `SetDiffDocument`
    /// 推送);保留它们是为 `file_history` 同款纯文本兜底与既有测试。
    pub truncated: bool,
    /// 旧版本 blob oid(新增文件为 `None`)。CodeMirror diff 渲染用,与
    /// `patch`(unified patch 文本)并存,互不影响。
    pub old_blob: Option<git2::Oid>,
    /// 新版本 blob oid(删除文件为 `None`)。
    pub new_blob: Option<git2::Oid>,
}

/// 单个文件 `patch` 文本的字符数上限,超过就截断(见 [`DiffFileEntry::truncated`])。
const MAX_PATCH_CHARS: usize = 20_000;

/// 单侧 blob 内容的字节上限(old/new 各自判定),超过就判定"不可渲染"。
pub const MAX_DIFF_BLOB_BYTES: usize = 512 * 1024;

/// [`diff_blob_content`] 的结果:要么是可渲染的双侧文本,要么给出原因
/// (供 UI 占位文案使用)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffBlobContent {
    Text { old_text: String, new_text: String },
    NotRenderable { reason: String },
}

/// 给定原始字节,判断能否喂给 CodeMirror diff 渲染:超过
/// [`MAX_DIFF_BLOB_BYTES`] 或含二进制内容(NUL 字节 / 非法 UTF-8)都判定
/// "不可渲染"。`git_log`(commit vs commit)与 `file_history`(commit vs
/// 磁盘实时内容)共用同一份判定,不允许出现第二份可能漂移的实现。
pub(crate) fn classify_diff_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.len() > MAX_DIFF_BLOB_BYTES {
        return None;
    }
    if bytes.contains(&0u8) {
        return None;
    }
    std::str::from_utf8(bytes).ok().map(|s| s.to_string())
}

/// 按新增/删除文件语义把 `None` 侧当空字符串处理;非 `None` 侧任一超过
/// [`MAX_DIFF_BLOB_BYTES`] 或含二进制内容(NUL 字节 / 非法 UTF-8)都判定
/// "不可渲染"——不做部分截断渲染。
pub fn diff_blob_content(
    repo: &git2::Repository,
    old_blob: Option<git2::Oid>,
    new_blob: Option<git2::Oid>,
) -> Result<DiffBlobContent, String> {
    fn read_side(
        repo: &git2::Repository,
        oid: Option<git2::Oid>,
    ) -> Result<Option<String>, String> {
        let Some(oid) = oid else {
            return Ok(Some(String::new()));
        };
        let blob = repo.find_blob(oid).map_err(|e| e.message().to_string())?;
        Ok(classify_diff_bytes(blob.content()))
    }

    let old_text = read_side(repo, old_blob)?;
    let new_text = read_side(repo, new_blob)?;
    match (old_text, new_text) {
        (Some(old_text), Some(new_text)) => Ok(DiffBlobContent::Text { old_text, new_text }),
        _ => Ok(DiffBlobContent::NotRenderable {
            reason: "文件不是文本,或超过大小上限,不支持 diff 渲染".to_string(),
        }),
    }
}

#[derive(Debug, Clone)]
pub struct CommitDetail {
    pub files: Vec<DiffFileEntry>,
}

/// 文件列表上方的分类筛选维度。`All` = 不筛选(默认,也是切到不含当前分类
/// 的提交时的回落值,见 `Message::DetailLoaded`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileFilter {
    #[default]
    All,
    /// 就地改动(`Modified`,以及 `Typechange` 这类没有专属 tab 的 delta)。
    Modified,
    Added,
    Deleted,
    /// 重命名 / 复制(`Renamed`/`Copied`)。
    Renamed,
}

impl FileFilter {
    /// 这个筛选是否接纳某个 git delta。
    ///
    /// 分桶与 `status_glyph` 的字符一一对应(`+`→Added、`-`→Deleted、
    /// `M`→Modified、`R`/`C`→Renamed);`status_glyph` 落到 `?` 的其它 delta
    /// (如 `Typechange`)归入「修改」——它们本质上也是就地改动,不给它们单开
    /// 一个几乎不会出现的 tab。
    fn matches(self, status: git2::Delta) -> bool {
        match self {
            FileFilter::All => true,
            FileFilter::Added => status == git2::Delta::Added,
            FileFilter::Deleted => status == git2::Delta::Deleted,
            FileFilter::Renamed => matches!(status, git2::Delta::Renamed | git2::Delta::Copied),
            FileFilter::Modified => !matches!(
                status,
                git2::Delta::Added
                    | git2::Delta::Deleted
                    | git2::Delta::Renamed
                    | git2::Delta::Copied
            ),
        }
    }
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
    /// 点文件列表上方的分类 tab,切换文件列表的筛选维度。
    SetFileFilter(FileFilter),
    /// 选中文件的 blob 内容异步加载完成。`git2::Oid`/`String` 是加载发起时
    /// 的 commit/路径快照,落地前核对仍匹配当前选择,不匹配则丢弃(用户
    /// 手快切换选择后的迟到结果)。
    DiffContentLoaded(git2::Oid, String, Result<DiffBlobContent, String>),
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

/// 当前已加载、给 CodeMirror diff webview 用的内容——`commit`/`path` 是
/// 加载时的选择快照,`SelectFile`/`SelectCommit` 落地新结果前先核对这两个
/// 字段还对不对得上"现在真正选中的",不对就丢弃(stale-guard,同
/// `DetailLoaded` 的 `repo_path`/`oid` 核对手法)。
#[derive(Debug, Clone)]
pub struct LoadedDiff {
    pub commit: git2::Oid,
    pub path: String,
    pub content: DiffBlobContent,
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
    /// 文件列表上方的分类筛选(默认 `All` = 不筛选)。用户点 tab 切换;
    /// 新 `detail` 落地时若该分类一个文件都没有,`Message::DetailLoaded`
    /// 会把它回落成 `All`(否则会停在一个空列表上)。
    file_filter: FileFilter,
    /// 当前选中文件已加载的 diff 内容(CodeMirror webview 用)。切
    /// commit/切选中文件时先清空,新结果落地(`DiffContentLoaded`)且仍
    /// 匹配当前选择才重新填入。
    loaded_diff: Option<LoadedDiff>,
    /// 当前选中文件的 diff **加载失败**原因(`DiffContentLoaded` 携带
    /// `Err` 时落地,如并发操作导致仓库状态变化)。与 `loaded_diff` 互斥:
    /// 失败时 `loaded_diff` 为 `None`、这里为 `Some`,UI 展示错误占位而不
    /// 是无限"加载中";切 commit/文件时清空。
    diff_load_error: Option<String>,
    /// 当前挂载的 diff webview 是否已经真正 `ready`(JS 端 `__dozer.dispatch`
    /// 已注册)。只由 `Message::GitLogDiffWebviewEvent` 的 `Ready` 分支置
    /// true;`loaded_diff` 被清空(见 `SelectCommit`/`SelectFile`)时连带置回
    /// false——webview 会被 `desired_webviews()` 判定为不再需要而销毁,
    /// 下次重新挂载是全新实例,必须等它自己的 `Ready` 才能再发命令。
    diff_webview_ready: bool,
    /// 最近一次**已下发**(`take_git_log_diff_script` 在确认该 webview 确
    /// 实在本帧池里、可 `evaluate_script` 之后才更新)的内容对应的
    /// `(commit, path)`。跟 `loaded_diff` 的 `(commit, path)` 不一致就还
    /// 需要再推一次;webview 那一帧还没进池就不写,下一帧重试,内容不丢。
    diff_sent_for: Option<(git2::Oid, String)>,
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

    /// 是否该向 CodeMirror diff webview 推送内容,以及推什么。
    ///
    /// 返回 `Some((commit, path, old_text, new_text))` 的条件(全部满足):
    /// - 已加载出**可渲染文本**(二进制/超限/失败 → `None`,UI 走 iced 占位);
    /// - webview 已确认 `Ready`(`__dozer.dispatch` 已注册);
    /// - 当前 `(commit, path)` 尚未送达过(`diff_sent_for` 不同)。
    ///
    /// 纯状态判定,不碰 webview 池——调用方(`App::take_git_log_diff_script`)
    /// 拿去组 envelope 后,只有真正 `evaluate_script` 成功才写
    /// `set_diff_sent_for`,保证"webview 还没进池"时下一帧重试不丢内容。
    pub fn pending_diff_push(&self) -> Option<(git2::Oid, String, String, String)> {
        if !self.diff_webview_ready {
            return None;
        }
        let loaded = self.loaded_diff.as_ref()?;
        if self.diff_sent_for.as_ref() == Some(&(loaded.commit, loaded.path.clone())) {
            return None;
        }
        let DiffBlobContent::Text { old_text, new_text } = &loaded.content else {
            return None;
        };
        Some((
            loaded.commit,
            loaded.path.clone(),
            old_text.clone(),
            new_text.clone(),
        ))
    }

    /// 当前是否应该挂载 diff 的 CodeMirror webview,以及它绑定的文件路径。
    ///
    /// 只有"已加载出可渲染文本"(`DiffBlobContent::Text`)才挂——二进制/
    /// 超限/加载失败/未选中文件都返回 `None`,由 `diff_pane_view` 走 iced
    /// 占位(`NotRenderable` 原因文案),绝不让 webview 抢占占位区。
    /// `path` 供调用方组 `EditorHostBinding`(URL 里的 `doc`/`lang` 用),
    /// 与 `pending_diff_push` 的推送内容同源同快照。
    pub fn diff_webview_desired(&self) -> Option<&str> {
        let loaded = self.loaded_diff.as_ref()?;
        match &loaded.content {
            DiffBlobContent::Text { .. } => Some(loaded.path.as_str()),
            DiffBlobContent::NotRenderable { .. } => None,
        }
    }

    /// `take_git_log_diff_script` 确认内容已下发后写回(见字段文档)。
    pub(crate) fn set_diff_sent_for(&mut self, key: (git2::Oid, String)) {
        self.diff_sent_for = Some(key);
    }

    /// `GitLogDiffWebviewEvent` 的 `Ready` 分支置位(见字段文档)。
    pub(crate) fn set_diff_webview_ready(&mut self, ready: bool) {
        self.diff_webview_ready = ready;
    }

    /// `Ready` 分支另需清空送达标记,强制下一帧重发一次当前内容。
    pub(crate) fn clear_diff_sent_for(&mut self) {
        self.diff_sent_for = None;
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

    /// 内核(macOS 原生分支菜单,`App::update` 拦截 `BranchPickerOpen` 时)
    /// 需要的快照:`(HEAD 分支名, 本地分支列表, 工作区是否 dirty)`。
    /// `head_branch` 从 commit 快照取(detached HEAD 时为 `None`)。
    pub(crate) fn branch_picker_snapshot(&self) -> (Option<String>, Vec<String>, bool) {
        (
            self.cache
                .as_ref()
                .and_then(|c| c.head_branch())
                .map(str::to_string),
            self.branches.clone(),
            self.dirty,
        )
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
            state.loaded_diff = None;
            state.diff_load_error = None;
            state.diff_webview_ready = false;
            state.diff_sent_for = None;
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
            state.selected_file = Some(path.clone());
            state.loaded_diff = None;
            state.diff_load_error = None;
            state.diff_webview_ready = false;
            state.diff_sent_for = None;
            let commit = state.selected?;
            let Ok(detail) = state.detail.as_ref()? else {
                return None;
            };
            let entry = detail.files.iter().find(|f| f.path == path)?;
            let (old_blob, new_blob) = (entry.old_blob, entry.new_blob);
            let repo_path = state.cache.as_ref().map(|c| c.repo_path().to_path_buf())?;
            handle.spawn(async move {
                let repo_path2 = repo_path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let repo =
                        git2::Repository::open(&repo_path2).map_err(|e| e.message().to_string())?;
                    diff_blob_content(&repo, old_blob, new_blob)
                })
                .await
                .unwrap_or_else(|e| Err(format!("diff 内容加载任务失败: {e}")));
                emit(Message::DiffContentLoaded(commit, path, result));
            });
            None
        }
        Message::SetFileFilter(filter) => {
            state.file_filter = filter;
            None
        }
        Message::DiffContentLoaded(commit, path, result) => {
            if state.selected != Some(commit)
                || state.selected_file.as_deref() != Some(path.as_str())
            {
                return None; // stale:用户已经切换了选择
            }
            match result {
                Ok(content) => {
                    state.loaded_diff = Some(LoadedDiff {
                        commit,
                        path,
                        content,
                    });
                    state.diff_load_error = None;
                }
                Err(err) => {
                    state.loaded_diff = None;
                    state.diff_load_error = Some(err);
                    // 同 `SelectCommit`/`SelectFile`:内容不可用意味着 webview
                    // 即将被 `desired_webviews()` 判定为不需要而销毁,旧的
                    // "已 ready"/"已送达" 状态不能带到下一次成功加载后重新
                    // 挂载的新实例上。
                    state.diff_webview_ready = false;
                    state.diff_sent_for = None;
                }
            }
            None
        }
        Message::DetailLoaded(repo_path, oid, result) => {
            let still_current = state.cache.as_ref().map(|c| c.repo_path())
                == Some(repo_path.as_path())
                && state.selected == Some(oid);
            if still_current {
                // 新提交的文件构成可能不含当前筛选的分类(比如上个提交筛了
                // "新增",这个提交一个新增文件都没有)——那种情况下该分类
                // tab 已从 tab 栏消失,继续保留筛选只会得到一个空列表,故
                // 回落到"全部"。`All` 恒匹配,不会被这里重置。
                if let Ok(detail) = &result
                    && !detail
                        .files
                        .iter()
                        .any(|f| state.file_filter.matches(f.status))
                {
                    state.file_filter = FileFilter::All;
                }
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
            let old_blob = (!delta.old_file().id().is_zero()).then(|| delta.old_file().id());
            let new_blob = (!delta.new_file().id().is_zero()).then(|| delta.new_file().id());
            Some(DiffFileEntry {
                path,
                status: delta.status(),
                patch: String::new(), // 下面按文件路径回填
                truncated: false,
                old_blob,
                new_blob,
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
                .font(Font::default()),
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
                .font(Font::default()),
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

/// 文件列表上方的分类筛选 tab 栏。固定顺序「全部 / 修改 / 新增 / 删除 /
/// 重命名」,只渲染当前提交里实际存在的分类(计数 > 0);「全部」恒在,即使
/// 没有改动文件也保留,当兜底 tab。
///
/// 计数取自当前选中提交的文件数,点击发 `Message::SetFileFilter`。视觉对齐
/// `codehealth` 的 `filter_buttons`(选中 = CARD 实底 + 金边 + 奶油字,未选中
/// = 无底 + 暗字),并补上本仓库统一的 dim→gold hover 反馈。
fn file_filter_tabs<'a>(
    detail: &CommitDetail,
    current: FileFilter,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let c = byteui::theme::color::current();
    // 顺序与用户给的示例一致:「全部」放最前当默认项。
    let candidates = [
        (FileFilter::All, "全部"),
        (FileFilter::Modified, "修改"),
        (FileFilter::Added, "新增"),
        (FileFilter::Deleted, "删除"),
        (FileFilter::Renamed, "重命名"),
    ];
    let mut bar = row![].spacing(6);
    for (filter, label) in candidates {
        let count = detail
            .files
            .iter()
            .filter(|f| filter.matches(f.status))
            .count();
        // 空分类不出 tab——点进去必然是空列表,没有意义。
        if count == 0 && filter != FileFilter::All {
            continue;
        }
        let active = filter == current;
        bar = bar.push(
            iced_widget::button(
                text(format!("{label}({count})")).size(byteui::theme::font::caption()),
            )
            .on_press(Message::SetFileFilter(filter))
            .padding([3, 8])
            .style(
                move |_t: &iced_widget::Theme, status: iced_widget::button::Status| {
                    let hovered = matches!(
                        status,
                        iced_widget::button::Status::Hovered | iced_widget::button::Status::Pressed
                    );
                    iced_widget::button::Style {
                        background: if active || hovered {
                            Some(c.card.into())
                        } else {
                            None
                        },
                        text_color: if active {
                            c.cream
                        } else if hovered {
                            c.gold
                        } else {
                            c.dim
                        },
                        border: Border {
                            color: if active {
                                c.gold
                            } else {
                                iced_widget::core::Color::TRANSPARENT
                            },
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                        ..iced_widget::button::Style::default()
                    }
                },
            ),
        );
    }
    bar.into()
}

/// 右上文件列表:选中 commit 改动的每个文件一行(状态字符 + 路径),点击
/// 发 `Message::SelectFile`,选中态同 `commit_list_view` 的金边(统一卡片样式:
/// 选中=金边、hover=金边+填充、一般态=描边)。`filter` 是上方分类 tab 选中的
/// 维度,只渲染匹配的行。
fn file_list_view<'a>(
    app: &App,
    detail: &'a Result<CommitDetail, String>,
    selected_file: Option<&'a str>,
    filter: FileFilter,
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
            // 用 `enumerate()` 的原始下标当 `HoverId::GitFile` 的 key(而不是
            // 过滤后的显示序号):key 与 `detail.files` 的下标绑定,切筛选时
            // 同一个文件始终是同一个 key,悬停高亮不会串到别的文件上。
            for (i, f) in detail.files.iter().enumerate() {
                if !filter.matches(f.status) {
                    continue;
                }
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
    // 内容列(右侧文件列表/diff)内边距对齐文件树面板(`project_pane`,8):
    // header/body/footer 的分隔线与内容容器统一按同一水平 inset 排布,避免
    // Git 面板自己另起一套 → 0 的 padding 与文件树/项目面板错位。列表列另走
    // 对话列表面板的 `conversation_list_pane`(12),见下方注释。
    let pad = theme::region::project_pane().padding;
    // 列表列(commit 列表)水平内边距对齐对话列表面板
    // (`conversation_list_pane`,12);垂直仍用 `project_pane`。空/加载态是单列
    // 铺满面板,同样按列表列的水平 inset 排布,免得 header 在"有/无提交"间
    // 左右跳一下。
    let list_pad = theme::region::conversation_list_pane().padding;
    let list_box_pad = iced_widget::core::Padding {
        top: pad.top,
        right: list_pad.right,
        bottom: pad.bottom,
        left: list_pad.left,
    };
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
                .padding(list_box_pad)
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
            .padding(list_box_pad),
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
            .padding(list_box_pad),
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
            // 文件列表上方头部:分类筛选 tab 栏 + 一条 1px 分割线。tab 栏的
            // 计数要从当前提交的文件列表算,所以只在 `detail` 落地成功时给出;
            // 加载失败时退回一句中性计数(此时 `file_list_view` 会另显示错误
            // 文案)。头部高度与 `theme::geometry::git_log_diff_header_h_px`
            // 严格同源——外部据此算 diff webview 的落点。
            let filter_bar: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
                match detail {
                    Ok(d) => file_filter_tabs(d, state.file_filter),
                    Err(_) => text("0 个修改的文件")
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().dim)
                        .into(),
                };
            let header = container(
                column![
                    filter_bar,
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
                container(file_list_view(
                    app,
                    detail,
                    state.selected_file.as_deref(),
                    state.file_filter
                ))
                .height(Length::FillPortion(top_portion)),
                crate::app::horizontal_divider_bar(
                    byteui::theme::color::current().bg,
                    byteui::theme::color::current().bg,
                    Message::RowDragStart,
                ),
                container(diff_pane_view(
                    state,
                    detail,
                    state.selected_file.as_deref()
                ))
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

    // 水平内边距下放到两列各自承担:列表列对齐对话列表面板
    // (`conversation_list_pane`,水平 12),内容列对齐文件树/预览
    // (`project_pane`,水平 8)——与对话面板"列表 12 / 内容 8"的既有分工一致
    // (用户反馈 commit 列表两侧边距太贴边)。垂直内边距仍由 body 统一施加,
    // 保证 header/footer 的上下留白不变。这样改同时让
    // `webview_geometry::git_log_diff_pane_bounds_for` 的 diff 落点更贴合真实
    // 布局:body 不再有水平内边距后,配对列宽 = 区宽 - 分隔线,与几何口径一致。
    let left_box = container(left_with_picker)
        .width(Length::FillPortion(list_portion))
        .padding(iced_widget::core::Padding {
            top: 0.0,
            right: list_pad.right,
            bottom: 0.0,
            left: list_pad.left,
        });
    let right_box = container(right)
        .width(Length::FillPortion(content_portion))
        .padding(iced_widget::core::Padding {
            top: 0.0,
            right: pad.right,
            bottom: 0.0,
            left: pad.left,
        });
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
        .padding(iced_widget::core::Padding {
            top: pad.top,
            right: 0.0,
            bottom: pad.bottom,
            left: 0.0,
        })
        .into()
}

/// 右下 diff 内容面板。渲染分三种情形(见设计文档"diff pane 路由"):
///
/// 1. **CodeMirror diff webview 已挂载**(`state.diff_webview_desired()` 为
///    `Some`,即已加载出可渲染双侧文本):这里只返回一个**空容器**占位——
///    真正内容由原生 wry 子视图绘制,webview 恒在 iced 之上,iced 再画一遍
///    只会造成重复/闪烁,所以什么都不画,只保留几何占位。
/// 2. **不可渲染**(二进制/超限):`loaded_diff` 是
///    `DiffBlobContent::NotRenderable` 时,展示只读占位文案 + `reason`,
///    webview 此时不会挂载。
///
/// 加载失败(`diff_load_error` 为 `Some`)时展示错误占位,webview 同样不
/// 会挂载。文件已选中但结果未落地(异步读 blob 的过渡帧)时给中性加载态;
/// 未选中文件则沿用既有"未选中文件"占位。
///
/// `selected_file` 对应文件的 patch 在旧实现里逐行染色(`colored_diff_lines`),
/// CodeMirror 接管后 iced 不再绘制正文;不可渲染/失败时不退回旧染色(语义
/// 不同,会给用户"能看"的错觉),而是明确给只读原因。
fn diff_pane_view<'a>(
    state: &'a State,
    detail: &'a Result<CommitDetail, String>,
    selected_file: Option<&'a str>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 情形 1:webview 会挂载 → iced 侧留空,不重复绘制。
    if state.diff_webview_desired().is_some() {
        return container(iced_widget::Space::new()).into();
    }
    // 情形 2:已加载出明确"不可渲染"结果(与当前 selection 对齐,stale
    // 结果不会落进 `loaded_diff`),给只读占位 + 原因。
    if let Some(loaded) = state.loaded_diff.as_ref()
        && let DiffBlobContent::NotRenderable { reason } = &loaded.content
    {
        return container(
            column![
                text("该文件无法在 diff 视图中渲染")
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().dim),
                text(reason.clone())
                    .size(byteui::theme::font::caption_sm())
                    .color(byteui::theme::color::current().dim),
            ]
            .spacing(4),
        )
        .padding(8)
        .into();
    }
    // 情形 2b:blob 读取失败(与当前 selection 对齐才会落地),展示原因。
    if let Some(err) = state.diff_load_error.as_ref() {
        return container(
            column![
                text("diff 内容加载失败")
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().dim),
                text(err.clone())
                    .size(byteui::theme::font::caption_sm())
                    .color(byteui::theme::color::current().dim),
            ]
            .spacing(4),
        )
        .padding(8)
        .into();
    }
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
    let Some(_entry) = detail.files.iter().find(|f| f.path == path) else {
        return container(
            text("未选中文件")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
        .padding(8)
        .into();
    };
    // 情形 3:文件已选中、但 `loaded_diff` 还没落地(异步读 blob 的过渡帧)。
    // CodeMirror webview 是唯一渲染路径,这一帧不退回旧的 iced 染色(既与
    // "CodeMirror 接管"矛盾,又会在内容到达后闪一下),给中性加载态即可。
    container(
        text("加载 diff 中…")
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim),
    )
    .padding(8)
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
    fn commit_detail_populates_blob_oids_for_added_files() {
        let (_dir, repo) = mkrepo_with_one_commit();
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).expect("应能解析临时仓库");
        let root_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, root_oid).expect("根提交相对空树应该也能算出 diff");
        for f in &detail.files {
            assert_eq!(f.old_blob, None, "根提交没有旧版本,old_blob 必须是 None");
            assert!(f.new_blob.is_some(), "新增文件必须有 new_blob");
        }
    }

    /// 在 `mkrepo_with_one_commit()` 基础上追加一个"修改 a.txt、删除 b.txt"
    /// 的第二个提交,供改动/删除文件的 blob 提取测试用。
    fn mkrepo_with_modify_and_delete_commit() -> (tempfile::TempDir, std::path::PathBuf) {
        let (dir, repo) = mkrepo_with_one_commit();
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
        std::fs::write(repo.join("a.txt"), "one\nmodified\n").unwrap();
        git(&["rm", "-q", "b.txt"]);
        git(&["add", "a.txt"]);
        git(&["commit", "-qm", "modify and delete"]);
        (dir, repo)
    }

    #[test]
    fn commit_detail_populates_blob_oids_for_modified_and_deleted_files() {
        let (_dir, repo) = mkrepo_with_modify_and_delete_commit();
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).expect("应能解析临时仓库");
        let head_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, head_oid).expect("应能算出第二个提交的 diff");

        let modified = detail
            .files
            .iter()
            .find(|f| f.path == "a.txt")
            .expect("a.txt 应该在改动文件里");
        assert_eq!(modified.status, git2::Delta::Modified);
        assert!(modified.old_blob.is_some());
        assert!(modified.new_blob.is_some());
        assert_ne!(modified.old_blob, modified.new_blob);

        let deleted = detail
            .files
            .iter()
            .find(|f| f.path == "b.txt")
            .expect("b.txt 应该在改动文件里");
        assert_eq!(deleted.status, git2::Delta::Deleted);
        assert!(deleted.old_blob.is_some());
        assert_eq!(deleted.new_blob, None, "删除文件必须是 new_blob = None");
    }

    #[test]
    fn diff_blob_content_reads_modified_file_both_sides() {
        let (_dir, repo) = mkrepo_with_modify_and_delete_commit();
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).expect("应能解析临时仓库");
        let head_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, head_oid).unwrap();
        let modified = detail.files.iter().find(|f| f.path == "a.txt").unwrap();

        let git_repo = git2::Repository::open(&repo).unwrap();
        let content = diff_blob_content(&git_repo, modified.old_blob, modified.new_blob)
            .expect("修改文件的 blob 内容应能读出");
        let DiffBlobContent::Text { old_text, new_text } = content else {
            panic!("修改文件应该判定为可渲染文本");
        };
        assert_eq!(old_text, "one\n");
        assert_eq!(new_text, "one\nmodified\n");
    }

    #[test]
    fn diff_blob_content_added_file_has_empty_old_side() {
        let (_dir, repo) = mkrepo_with_one_commit();
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).expect("应能解析临时仓库");
        let root_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, root_oid).unwrap();
        let added = detail.files.iter().find(|f| f.path == "a.txt").unwrap();

        let git_repo = git2::Repository::open(&repo).unwrap();
        let content = diff_blob_content(&git_repo, added.old_blob, added.new_blob).unwrap();
        let DiffBlobContent::Text { old_text, new_text } = content else {
            panic!("新增文件应该判定为可渲染文本");
        };
        assert_eq!(old_text, "");
        assert_eq!(new_text, "one\n");
    }

    #[test]
    fn diff_blob_content_deleted_file_has_empty_new_side() {
        let (_dir, repo) = mkrepo_with_modify_and_delete_commit();
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).expect("应能解析临时仓库");
        let head_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, head_oid).unwrap();
        let deleted = detail.files.iter().find(|f| f.path == "b.txt").unwrap();

        let git_repo = git2::Repository::open(&repo).unwrap();
        let content = diff_blob_content(&git_repo, deleted.old_blob, deleted.new_blob).unwrap();
        let DiffBlobContent::Text { old_text, new_text } = content else {
            panic!("删除文件应该判定为可渲染文本");
        };
        assert_eq!(old_text, "two\n");
        assert_eq!(new_text, "");
    }

    /// tempdir 里造一个含二进制文件(NUL 字节)的一次提交仓库。
    fn mkrepo_with_binary_commit() -> (tempfile::TempDir, std::path::PathBuf) {
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
        std::fs::write(repo.join("blob.bin"), [0x00u8, 0x01, 0x02, 0xff]).unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "binary"]);
        (dir, repo)
    }

    #[test]
    fn diff_blob_content_rejects_binary_content() {
        let (_dir, repo) = mkrepo_with_binary_commit();
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).expect("应能解析临时仓库");
        let root_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, root_oid).unwrap();
        let bin = detail.files.iter().find(|f| f.path == "blob.bin").unwrap();

        let git_repo = git2::Repository::open(&repo).unwrap();
        let content = diff_blob_content(&git_repo, bin.old_blob, bin.new_blob).unwrap();
        assert!(matches!(content, DiffBlobContent::NotRenderable { .. }));
    }

    #[test]
    fn diff_blob_content_rejects_oversized_blob() {
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
        // 512KB 上限之上一字节:MAX_DIFF_BLOB_BYTES = 512 * 1024。
        let big = "a".repeat(MAX_DIFF_BLOB_BYTES + 1);
        std::fs::write(repo.join("big.txt"), &big).unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "big"]);
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).unwrap();
        let root_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, root_oid).unwrap();
        let entry = detail.files.iter().find(|f| f.path == "big.txt").unwrap();

        let git_repo = git2::Repository::open(&repo).unwrap();
        let content = diff_blob_content(&git_repo, entry.old_blob, entry.new_blob).unwrap();
        assert!(matches!(content, DiffBlobContent::NotRenderable { .. }));
    }

    #[test]
    fn diff_blob_content_accepts_blob_exactly_at_cap() {
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
        let exact = "a".repeat(MAX_DIFF_BLOB_BYTES);
        std::fs::write(repo.join("exact.txt"), &exact).unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "exact"]);
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).unwrap();
        let root_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, root_oid).unwrap();
        let entry = detail.files.iter().find(|f| f.path == "exact.txt").unwrap();

        let git_repo = git2::Repository::open(&repo).unwrap();
        let content = diff_blob_content(&git_repo, entry.old_blob, entry.new_blob).unwrap();
        assert!(
            matches!(content, DiffBlobContent::Text { .. }),
            "恰好等于上限应当可渲染"
        );
    }

    #[test]
    fn classify_diff_bytes_matches_diff_blob_content_behavior() {
        // 提取重构不应该改变行为:同一段字节,`classify_diff_bytes` 的结果
        // 要跟通过 `diff_blob_content` 间接观察到的判定一致(正常文本/
        // 二进制/超限三种)。
        assert_eq!(classify_diff_bytes(b"hello\n"), Some("hello\n".to_string()));
        assert_eq!(classify_diff_bytes(&[0x00, 0x01, 0x02]), None);
        let big = vec![b'a'; MAX_DIFF_BLOB_BYTES + 1];
        assert_eq!(classify_diff_bytes(&big), None);
        let exact = vec![b'a'; MAX_DIFF_BLOB_BYTES];
        assert!(classify_diff_bytes(&exact).is_some());
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
                    old_blob: None,
                    new_blob: None,
                },
                DiffFileEntry {
                    path: "b.rs".to_string(),
                    status: git2::Delta::Added,
                    patch: "+y".to_string(),
                    truncated: false,
                    old_blob: None,
                    new_blob: None,
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

    /// 构造一个只关心 `path`/`status` 的 `DiffFileEntry`——分类筛选测试里
    /// 其余字段与筛选无关。
    fn file_entry(path: &str, status: git2::Delta) -> DiffFileEntry {
        DiffFileEntry {
            path: path.to_string(),
            status,
            patch: String::new(),
            truncated: false,
            old_blob: None,
            new_blob: None,
        }
    }

    /// 分桶与 `status_glyph` 的字符一一对应:`+`→Added、`-`→Deleted、
    /// `M`→Modified、`R`/`C`→Renamed;其余 delta(如 `Typechange`)归「修改」。
    #[test]
    fn file_filter_matches_buckets() {
        use git2::Delta;
        assert!(FileFilter::All.matches(Delta::Added));
        assert!(FileFilter::Added.matches(Delta::Added));
        assert!(!FileFilter::Added.matches(Delta::Modified));
        assert!(FileFilter::Deleted.matches(Delta::Deleted));
        assert!(FileFilter::Renamed.matches(Delta::Renamed));
        assert!(FileFilter::Renamed.matches(Delta::Copied));
        assert!(FileFilter::Modified.matches(Delta::Modified));
        assert!(FileFilter::Modified.matches(Delta::Typechange));
        for excluded in [Delta::Added, Delta::Deleted, Delta::Renamed, Delta::Copied] {
            assert!(!FileFilter::Modified.matches(excluded), "{excluded:?}");
        }
    }

    #[tokio::test]
    async fn set_file_filter_switches_filter() {
        let handle = tokio::runtime::Handle::current();
        let mut state = State::default();
        let result = update(
            &mut state,
            Message::SetFileFilter(FileFilter::Added),
            &handle,
            |_| {},
        );
        assert_eq!(state.file_filter, FileFilter::Added);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detail_loaded_resets_filter_when_category_absent() {
        let repo_path = PathBuf::from("/tmp/repo");
        let oid = git2::Oid::from_bytes(&[12; 20]).unwrap();
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            selected: Some(oid),
            file_filter: FileFilter::Added,
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let detail = CommitDetail {
            files: vec![file_entry("a.rs", git2::Delta::Modified)],
        };
        update(
            &mut state,
            Message::DetailLoaded(repo_path, oid, Ok(detail)),
            &handle,
            |_| {},
        );
        assert_eq!(
            state.file_filter,
            FileFilter::All,
            "新提交没有「新增」文件时应回落「全部」,否则会停在空列表上"
        );
    }

    #[tokio::test]
    async fn detail_loaded_keeps_filter_when_category_present() {
        let repo_path = PathBuf::from("/tmp/repo");
        let oid = git2::Oid::from_bytes(&[13; 20]).unwrap();
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            selected: Some(oid),
            file_filter: FileFilter::Added,
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let detail = CommitDetail {
            files: vec![
                file_entry("a.rs", git2::Delta::Modified),
                file_entry("b.rs", git2::Delta::Added),
            ],
        };
        update(
            &mut state,
            Message::DetailLoaded(repo_path, oid, Ok(detail)),
            &handle,
            |_| {},
        );
        assert_eq!(
            state.file_filter,
            FileFilter::Added,
            "新提交仍有「新增」文件时不该重置筛选"
        );
    }

    #[tokio::test]
    async fn select_file_clears_stale_diff_and_requests_fresh_load() {
        let repo_path = PathBuf::from("/tmp/repo");
        let commit_oid = git2::Oid::from_bytes(&[7; 20]).unwrap();
        let old_blob = git2::Oid::from_bytes(&[8; 20]).unwrap();
        let new_blob = git2::Oid::from_bytes(&[9; 20]).unwrap();
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            selected: Some(commit_oid),
            detail: Some(Ok(CommitDetail {
                files: vec![DiffFileEntry {
                    path: "a.txt".into(),
                    status: git2::Delta::Modified,
                    patch: "x".into(),
                    truncated: false,
                    old_blob: Some(old_blob),
                    new_blob: Some(new_blob),
                }],
            })),
            loaded_diff: Some(LoadedDiff {
                commit: commit_oid,
                path: "old-selection.txt".into(),
                content: DiffBlobContent::Text {
                    old_text: "a".into(),
                    new_text: "b".into(),
                },
            }),
            ..State::default()
        };

        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::SelectFile("a.txt".to_string()),
            &handle,
            |_| {},
        );

        assert_eq!(state.selected_file.as_deref(), Some("a.txt"));
        assert!(
            state.loaded_diff.is_none(),
            "换选中文件后必须先清空旧内容,不能让 stale 内容闪一下"
        );
    }

    #[tokio::test]
    async fn diff_content_loaded_ignored_when_selection_moved_on() {
        let commit_a = git2::Oid::from_bytes(&[11; 20]).unwrap();
        let mut state = State {
            selected: Some(commit_a),
            selected_file: Some("b.txt".to_string()), // 用户已经切到 b.txt
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        // 一条迟到的 a.txt 结果(用户点过 a.txt 但已经切走了)。
        update(
            &mut state,
            Message::DiffContentLoaded(
                commit_a,
                "a.txt".to_string(),
                Ok(DiffBlobContent::Text {
                    old_text: "x".into(),
                    new_text: "y".into(),
                }),
            ),
            &handle,
            |_| {},
        );
        assert!(
            state.loaded_diff.is_none(),
            "stale 结果(commit/path 跟当前选中对不上)必须被丢弃"
        );
    }

    #[tokio::test]
    async fn diff_content_loaded_applied_when_selection_still_matches() {
        let commit_a = git2::Oid::from_bytes(&[12; 20]).unwrap();
        let mut state = State {
            selected: Some(commit_a),
            selected_file: Some("a.txt".to_string()),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DiffContentLoaded(
                commit_a,
                "a.txt".to_string(),
                Ok(DiffBlobContent::Text {
                    old_text: "x".into(),
                    new_text: "y".into(),
                }),
            ),
            &handle,
            |_| {},
        );
        let loaded = state
            .loaded_diff
            .as_ref()
            .expect("匹配当前选择的结果应该落地");
        assert_eq!(loaded.commit, commit_a);
        assert_eq!(loaded.path, "a.txt");
        assert!(matches!(loaded.content, DiffBlobContent::Text { .. }));
    }

    fn loaded_text_state(
        commit: git2::Oid,
        path: &str,
        old_text: &str,
        new_text: &str,
        ready: bool,
    ) -> State {
        State {
            loaded_diff: Some(LoadedDiff {
                commit,
                path: path.to_string(),
                content: DiffBlobContent::Text {
                    old_text: old_text.to_string(),
                    new_text: new_text.to_string(),
                },
            }),
            diff_webview_ready: ready,
            ..State::default()
        }
    }

    #[test]
    fn pending_diff_push_requires_ready_webview() {
        let commit = git2::Oid::from_bytes(&[7; 20]).unwrap();
        let state = loaded_text_state(commit, "a.txt", "old", "new", false);
        assert!(
            state.pending_diff_push().is_none(),
            "webview 未 Ready(diff_webview_ready=false)时不推送"
        );
    }

    #[test]
    fn pending_diff_push_returns_text_once_ready() {
        let commit = git2::Oid::from_bytes(&[7; 20]).unwrap();
        let state = loaded_text_state(commit, "a.txt", "old", "new", true);
        let push = state.pending_diff_push().expect("Ready 且未送达应产出推送");
        assert_eq!(push.0, commit);
        assert_eq!(push.1, "a.txt");
        assert_eq!(push.2, "old");
        assert_eq!(push.3, "new");
    }

    #[test]
    fn pending_diff_push_suppressed_after_sent() {
        let commit = git2::Oid::from_bytes(&[7; 20]).unwrap();
        let mut state = loaded_text_state(commit, "a.txt", "old", "new", true);
        assert!(state.pending_diff_push().is_some());
        // 模拟 `take_git_log_diff_script` 下发后写回送达标记。
        state.set_diff_sent_for((commit, "a.txt".to_string()));
        assert!(
            state.pending_diff_push().is_none(),
            "同一 (commit, path) 已送达后不再重复推送"
        );
    }

    #[test]
    fn pending_diff_push_resends_when_selection_changes() {
        let commit = git2::Oid::from_bytes(&[7; 20]).unwrap();
        let mut state = loaded_text_state(commit, "a.txt", "old", "new", true);
        state.set_diff_sent_for((commit, "a.txt".to_string()));
        // 换文件(同一 commit 内):新 path 对应新内容,应再次产出推送。
        state.loaded_diff = Some(LoadedDiff {
            commit,
            path: "b.txt".to_string(),
            content: DiffBlobContent::Text {
                old_text: "x".into(),
                new_text: "z".into(),
            },
        });
        let push = state.pending_diff_push().expect("换文件后应再次推送新内容");
        assert_eq!(push.1, "b.txt");
        assert_eq!(push.3, "z");
    }

    #[test]
    fn pending_diff_push_skips_not_renderable() {
        let commit = git2::Oid::from_bytes(&[7; 20]).unwrap();
        let state = State {
            loaded_diff: Some(LoadedDiff {
                commit,
                path: "bin.dat".to_string(),
                content: DiffBlobContent::NotRenderable {
                    reason: "二进制".to_string(),
                },
            }),
            diff_webview_ready: true,
            ..State::default()
        };
        assert!(
            state.pending_diff_push().is_none(),
            "不可渲染内容不推 CodeMirror(UI 走 iced 占位)"
        );
    }

    #[test]
    fn pending_diff_push_none_without_loaded_diff() {
        let state = State {
            diff_webview_ready: true,
            ..State::default()
        };
        assert!(state.pending_diff_push().is_none());
    }

    #[test]
    fn diff_webview_desired_only_for_renderable_text() {
        let commit = git2::Oid::from_bytes(&[7; 20]).unwrap();
        let text = loaded_text_state(commit, "a.txt", "old", "new", false);
        assert_eq!(text.diff_webview_desired(), Some("a.txt"));

        let bin = State {
            loaded_diff: Some(LoadedDiff {
                commit,
                path: "bin.dat".to_string(),
                content: DiffBlobContent::NotRenderable {
                    reason: "二进制".to_string(),
                },
            }),
            ..State::default()
        };
        assert_eq!(
            bin.diff_webview_desired(),
            None,
            "不可渲染内容不挂 CodeMirror webview"
        );

        assert_eq!(State::default().diff_webview_desired(), None);
    }

    #[tokio::test]
    async fn diff_content_loaded_error_sets_load_error_and_clears_content() {
        let commit = git2::Oid::from_bytes(&[13; 20]).unwrap();
        let mut state = State {
            selected: Some(commit),
            selected_file: Some("a.txt".to_string()),
            loaded_diff: Some(LoadedDiff {
                commit,
                path: "a.txt".to_string(),
                content: DiffBlobContent::Text {
                    old_text: "stale".into(),
                    new_text: "stale".into(),
                },
            }),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DiffContentLoaded(commit, "a.txt".to_string(), Err("boom".to_string())),
            &handle,
            |_| {},
        );
        assert!(state.loaded_diff.is_none(), "失败后旧内容必须清空");
        assert_eq!(state.diff_load_error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn select_file_clears_previous_load_error() {
        let commit = git2::Oid::from_bytes(&[14; 20]).unwrap();
        let mut state = State {
            selected: Some(commit),
            selected_file: Some("a.txt".to_string()),
            detail: Some(Ok(CommitDetail {
                files: vec![DiffFileEntry {
                    path: "b.txt".into(),
                    status: git2::Delta::Modified,
                    patch: String::new(),
                    truncated: false,
                    old_blob: None,
                    new_blob: None,
                }],
            })),
            diff_load_error: Some("boom".to_string()),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::SelectFile("b.txt".to_string()),
            &handle,
            |_| {},
        );
        assert_eq!(
            state.diff_load_error, None,
            "换文件后必须先清空上一条错误,不能带着旧错误进新选择"
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
