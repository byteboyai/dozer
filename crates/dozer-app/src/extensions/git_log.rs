// crates/dozer-app/src/git_log.rs
//! spike(2026-08-06): 验证第三方 Rust 库 `gleisbau`(git-graph 的布局引擎,
//! 拆出来的独立 crate)能否喂出可在 iced Canvas 里原生画出的提交图数据,
//! 探路"要不要在 Dozer 里做一个类似 VS Code Git Graph 的面板"。
//!
//! 只验证数据链路是否走得通,不追求 curve/fork 的像素级还原:每条 track
//! 画一根直线,commit 是线上的一个圆点,父子关系用直线连接(不是贝塞尔)。
//! 验证通过、决定转正时,再补动画/交互/性能优化。
use crate::theme;
use iced_widget::canvas::{self, Canvas};
use iced_widget::core::alignment;
use iced_widget::core::{Color, Element, Font, Length, Padding, Pixels, Point, Rectangle, Vector};
use iced_widget::{column, container, row, scrollable, text};
use std::path::{Path, PathBuf};

/// 首次打开面板拉多少个 commit——够看出分叉/合并的形状,又不至于让
/// revwalk + 分支归属分析在大仓库上明显卡顿。"加载更多"每次在当前基础上
/// 加这么多再整份重算(gleisbau 的 API 是"从头按 max_count 走一遍
/// revwalk",没有增量/游标接口,重算是唯一选项——见 build() 文档)。
pub const DEFAULT_MAX_COMMITS: usize = 200;
pub const LOAD_MORE_STEP: usize = 200;

const ROW_HEIGHT: f32 = 22.0;
const COL_WIDTH: f32 = 14.0;
const DOT_RADIUS: f32 = 3.5;
const LEFT_MARGIN: f32 = 12.0;
const TEXT_GAP: f32 = 12.0;
const LINE_WIDTH: f32 = 1.6;
/// 选中提交详情子面板的宽度(px)。面板本身是 `Length::Fill` 高度、固定在
/// canvas 右侧,宽度固定以免挤压提交图。要放得下每个文件的 unified diff
/// 文本(等宽字体,常见改动行 60-80 列),比只放文件列表时的宽度宽一截。
const DETAIL_WIDTH: f32 = 460.0;

/// 与主题色轮换配色的 track 调色板——不用 gleisbau 自带的 CSS 颜色名,
/// 省掉一个颜色名解析器,顺便让图和 ByteBoy2077 主题保持一致。
const TRACK_COLORS: [Color; 5] = [
    theme::color::CYAN,
    theme::color::GREEN,
    theme::color::GOLD,
    theme::color::PURPLE,
    theme::color::RED,
];

fn track_color(color_idx: usize) -> Color {
    TRACK_COLORS[color_idx % TRACK_COLORS.len()]
}

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
    column: usize,
    color_idx: usize,
    short_sha: String,
    summary: String,
    /// 父 commit 的 (row, column, color_idx),用于画连线;可能落在
    /// `max_count` 截断范围之外——那种父 commit 不出现在 `rows` 里,
    /// 此处已被过滤掉。
    parents: Vec<(usize, usize, usize)>,
    /// 指向这个 commit 的分支/tag(可能为空)。
    refs: Vec<RefLabel>,
    /// 这个 commit 的完整 40 位 oid,选中详情(Task 2)用——`short_sha` 只
    /// 够显示,不够拿去 `git2::Repository::find_commit`。
    oid: git2::Oid,
}

/// 派生 `Debug + Clone`,理由同 [`CommitRow`]。
#[derive(Debug, Clone)]
pub struct GitLogSnapshot {
    repo_path: PathBuf,
    rows: Vec<CommitRow>,
    max_column: usize,
    /// 当前 HEAD 所在的本地分支名(detached HEAD 时为 `None`)——图上给这
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

    let mut max_column = 0usize;
    let rows = graph
        .tracks
        .commits
        .iter()
        .map(|commit| {
            let b_idx = commit
                .branch_trace
                .ok_or_else(|| "commit 缺少 branch_trace".to_string())?;
            let column = graph
                .layout
                .track_visual(b_idx)
                .and_then(|v| v.column)
                .unwrap_or(0);
            max_column = max_column.max(column);
            let color_idx = b_idx.index();
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
            let parents = commit
                .parents
                .iter()
                .filter_map(|poid| {
                    let p_idx = *graph.tracks.indices.get(poid)?;
                    let p_commit = graph.tracks.commits.get(p_idx)?;
                    let p_b_idx = p_commit.branch_trace?;
                    let p_column = graph
                        .layout
                        .track_visual(p_b_idx)
                        .and_then(|v| v.column)
                        .unwrap_or(0);
                    Some((p_idx, p_column, p_b_idx.index()))
                })
                .collect();
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
            Ok(CommitRow {
                column,
                color_idx,
                short_sha,
                summary,
                parents,
                refs,
                oid: commit.oid,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(GitLogSnapshot {
        repo_path: repo_path.to_path_buf(),
        rows,
        max_column,
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
    DetailLoaded(PathBuf, git2::Oid, Result<CommitDetail, String>),
    SnapshotLoaded(PathBuf, usize, Result<GitLogSnapshot, String>),
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
    /// 最近一次派发的 `build` 请求 (repo_path, max_count)——落地时核对
    /// 还对不对得上"现在真正需要的",不对就丢弃。
    pending: Option<(PathBuf, usize)>,
    /// "加载更多"发起前记下的选中提交,新快照落地后据此还原选中态。
    restore_after_load: Option<git2::Oid>,
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
        Message::SelectCommit(oid) => {
            state.selected = Some(oid);
            state.detail = None;
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
        Message::DetailLoaded(repo_path, oid, result) => {
            let still_current = state.cache.as_ref().map(|c| c.repo_path())
                == Some(repo_path.as_path())
                && state.selected == Some(oid);
            if still_current {
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
    }
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

struct GitLogCanvas<'a> {
    snapshot: &'a GitLogSnapshot,
    selected: Option<git2::Oid>,
    head_branch: Option<&'a str>,
}

fn row_center(row: usize, column: usize) -> Point {
    Point::new(
        LEFT_MARGIN + column as f32 * COL_WIDTH,
        ROW_HEIGHT * 0.5 + row as f32 * ROW_HEIGHT,
    )
}

impl canvas::Program<Message, iced_widget::Theme, iced_renderer::Renderer> for GitLogCanvas<'_> {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &iced_widget::core::Event,
        bounds: Rectangle,
        cursor: iced_widget::core::mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let iced_widget::core::Event::Mouse(iced_widget::core::mouse::Event::ButtonPressed(
            iced_widget::core::mouse::Button::Left,
        )) = event
        else {
            return None;
        };
        let pos = cursor.position_in(bounds)?;
        if pos.x < 0.0 || pos.y < 0.0 {
            return None;
        }
        let row_idx = (pos.y / ROW_HEIGHT) as usize;
        let row = self.snapshot.rows.get(row_idx)?;
        Some(canvas::Action::publish(Message::SelectCommit(row.oid)).and_capture())
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_renderer::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: iced_widget::core::mouse::Cursor,
    ) -> Vec<canvas::Geometry<iced_renderer::Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let text_x = LEFT_MARGIN + (self.snapshot.max_column + 1) as f32 * COL_WIDTH + TEXT_GAP;

        // 先画连线,commit 圆点和文字盖在上面。
        for (row_idx, commit) in self.snapshot.rows.iter().enumerate() {
            let from = row_center(row_idx, commit.column);
            for &(p_row, p_col, p_color_idx) in &commit.parents {
                let to = row_center(p_row, p_col);
                let path = canvas::Path::line(from, to);
                frame.stroke(
                    &path,
                    canvas::Stroke::default()
                        .with_color(track_color(p_color_idx))
                        .with_width(LINE_WIDTH),
                );
            }
        }

        for (row_idx, commit) in self.snapshot.rows.iter().enumerate() {
            let center = row_center(row_idx, commit.column);
            let color = track_color(commit.color_idx);
            if self.selected == Some(commit.oid) {
                frame.stroke(
                    &canvas::Path::circle(center, DOT_RADIUS + 2.5),
                    canvas::Stroke::default()
                        .with_color(theme::color::GOLD)
                        .with_width(1.5),
                );
            }
            frame.fill(&canvas::Path::circle(center, DOT_RADIUS), color);

            let refs_prefix = ref_labels_text(&commit.refs, self.head_branch);

            frame.with_save(|frame| {
                frame.translate(Vector::new(
                    text_x,
                    row_idx as f32 * ROW_HEIGHT + ROW_HEIGHT * 0.5,
                ));
                let content = if refs_prefix.is_empty() {
                    format!("{}  {}", commit.short_sha, commit.summary)
                } else {
                    format!("{}  {}  {}", commit.short_sha, refs_prefix, commit.summary)
                };
                frame.fill_text(canvas::Text {
                    content,
                    position: Point::ORIGIN,
                    color: theme::color::CREAM,
                    size: Pixels(theme::font::body() as f32),
                    align_y: alignment::Vertical::Center,
                    font: Font::MONOSPACE,
                    ..canvas::Text::default()
                });
            });
        }

        vec![frame.into_geometry()]
    }
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

/// 渲染整块提交图面板:有数据画 Canvas,出错画错误文案,两者皆无(比如
/// 尚未打开项目)画空状态提示。纯函数——不碰 `App`/`Workspace` 内部状态,
/// 调用方(`workspace.rs`)负责取数据、决定何时重建缓存、维护选中态。
pub fn view(state: &State) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let error = state.error.as_deref();
    if let Some(err) = error {
        return container(text(format!("git log 读取失败: {err}")).color(theme::color::RED))
            .padding(16)
            .into();
    }
    let loading = state.pending.is_some();
    let Some(snapshot) = state.cache.as_ref() else {
        let text_content = if loading {
            "加载中…"
        } else {
            "未打开项目"
        };
        return container(text(text_content).color(theme::color::DIM))
            .padding(16)
            .into();
    };
    let selected = state.selected;
    let detail = state.detail.as_ref();
    if snapshot.rows.is_empty() {
        return container(text("没有可显示的提交").color(theme::color::DIM))
            .padding(16)
            .into();
    }
    let height = ROW_HEIGHT * snapshot.rows.len() as f32;
    let head_branch = snapshot.head_branch.as_deref();
    let canvas: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        Canvas::new(GitLogCanvas {
            snapshot,
            selected,
            head_branch,
        })
        .width(Length::Fill)
        .height(Length::Fixed(height))
        .into();
    let mut header = row![
        text(snapshot.repo_path.display().to_string())
            .size(theme::font::caption())
            .color(theme::color::DIM)
    ]
    .padding([4, 8]);
    // 已经有旧快照在画的时候(引用变化重建/加载更多)又发起了新一轮异步
    // 加载——旧图先留着不闪空,但得给个文案说明"正在换新",不然用户会
    // 疑惑点了"加载更多"怎么行数没变。
    if loading {
        header = header.push(
            text("刷新中…")
                .size(theme::font::caption())
                .color(theme::color::DIM),
        );
    }
    let load_more = iced_widget::button(
        text("加载更多提交 (+200)")
            .size(theme::font::caption())
            .color(theme::color::CREAM),
    )
    .on_press_maybe((!loading).then_some(Message::LoadMore))
    .padding([4, 12]);
    let graph_body = column![canvas, load_more].padding(Padding {
        top: 0.0,
        right: 0.0,
        bottom: 8.0,
        left: 0.0,
    });
    let graph = scrollable(graph_body)
        .width(Length::Fill)
        .height(Length::Fill);
    if let Some(detail_res) = detail {
        let detail_panel = detail_view(snapshot, selected, detail_res);
        column![header, row![graph, detail_panel],]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    } else {
        column![header, graph]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

/// 选中提交右侧的详情子面板:文件列表(状态色点 + 路径)+ 聚焦文件的
/// unified diff。不内聚滚动,交给外层 `column` 撑;文件列表自滚动。
/// 纯函数:选中态、详情结果都由上层 `view` 传进来。
fn detail_view<'a>(
    _snapshot: &GitLogSnapshot,
    _selected: Option<git2::Oid>,
    result: &'a Result<CommitDetail, String>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let body: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> = match result {
        Err(err) => container(text(format!("详情加载失败: {err}")).color(theme::color::RED))
            .padding(8)
            .into(),
        Ok(detail) if detail.files.is_empty() => {
            container(text("无文件改动").color(theme::color::DIM))
                .padding(8)
                .into()
        }
        Ok(detail) => {
            let list = detail.files.iter().fold(column![].spacing(10), |acc, f| {
                let color = match f.status {
                    git2::Delta::Added => theme::color::GREEN,
                    git2::Delta::Deleted => theme::color::RED,
                    // 修改/重命名/复制等其余状态是纯分类展示,不是甲方动作,
                    // 不能借用 `theme::color::GOLD`(CLAUDE.md 硬性裁决)。
                    _ => theme::color::CYAN,
                };
                let header = row![
                    text(status_glyph(f.status)).color(color).width(18),
                    text(&f.path)
                        .size(theme::font::caption())
                        .color(theme::color::CREAM),
                ]
                .spacing(4)
                .padding([2, 8]);
                let acc = acc.push(header);
                if f.patch.is_empty() {
                    acc
                } else {
                    acc.push(
                        text(f.patch.clone())
                            .size(theme::font::caption())
                            .color(theme::color::BODY)
                            .font(Font::MONOSPACE),
                    )
                }
            });
            scrollable(list)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
    };
    container(body)
        .width(Length::Fixed(DETAIL_WIDTH))
        .height(Length::Fill)
        .padding(8)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(crate::theme::region::background().into()),
            ..container::Style::default()
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// spike 验证核心问题:`gleisbau` 能否对 Dozer 自己这个真实、有分叉/合并
    /// 历史的仓库跑出合理的布局数据。跑 `cargo test -p dozer-app git_log::tests
    /// -- --nocapture` 看打印的前 20 行,人工核对 column/parents 是否符合直觉
    /// (主线一列到底,feature 分支另开列,merge commit 有多个 parent 边)。
    #[test]
    fn build_against_real_repo() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crates/dozer-app 应有两层上级目录到仓库根");
        let snapshot =
            build(repo_root, DEFAULT_MAX_COMMITS).expect("gleisbau 应能解析 dozer 自己的仓库");

        assert!(!snapshot.rows.is_empty(), "真实仓库应至少有一个 commit");
        assert!(
            snapshot.max_column < 50,
            "正常仓库的分支列数不该失控般大: {}",
            snapshot.max_column
        );

        for (row_idx, row) in snapshot.rows.iter().take(20).enumerate() {
            println!(
                "row={row_idx} col={} color={} sha={} parents={:?} summary={:?}",
                row.column, row.color_idx, row.short_sha, row.parents, row.summary
            );
        }

        // merge commit(有 ≥2 个 parent)理应至少出现一次——Dozer 仓库历史里
        // 确实有过 merge(如 2429d15),不是纯线性历史;若这条断了,说明布局
        // 丢了合并边,数据链路没走通。
        assert!(
            snapshot.rows.iter().any(|r| r.parents.len() >= 2),
            "200 个 commit 窗口内应能看到至少一个 merge"
        );
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
            assert_eq!(a.column, b.column);
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

    fn snapshot_at(repo_path: &Path, max_count: usize) -> GitLogSnapshot {
        GitLogSnapshot {
            repo_path: repo_path.to_path_buf(),
            rows: Vec::new(),
            max_column: 0,
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
