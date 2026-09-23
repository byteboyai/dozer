//! 单个文件的历史提交对比:右键 Files 面板文件行"查看此文件历史"弹窗的
//! 数据层。跟 `git_log.rs` 的区别是那边是"整个仓库的提交图",这里是
//! "只关心一个文件路径,且要跟*当前工作目录实时内容*比较,不是跟某个
//! commit 的父提交比较"。见
//! `docs/superpowers/specs/2026-09-17-file-history-popup-design.md`。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use byteui::interaction::icons;
use iced_widget::core::{Element, Length};
use iced_widget::{button, column, container, row, scrollable, text};

/// 该文件在某一次提交里的记录——按 `build()` 的 revwalk 顺序(时间倒序)
/// 收集,只包含真正改动过这个文件的提交。
#[derive(Debug, Clone)]
pub struct FileHistoryEntry {
    pub oid: git2::Oid,
    pub short_sha: String,
    /// commit message 首行。
    pub summary: String,
    /// 收集了作者名但当前弹窗列表未渲染(见 spec「架构」数据类型定义),
    /// 保留在接口里供后续列表项展示作者,故 `#[allow(dead_code)]`。
    #[allow(dead_code)]
    pub author: Option<String>,
    /// author time,Unix 秒。
    pub time: i64,
}

#[derive(Debug, Clone)]
pub struct FileHistorySnapshot {
    /// 目标字段:当前 view 只消费 `entries`,这两个字段留作后续"快照是否
    /// 仍对应当前目标"的核对接口,故 `#[allow(dead_code)]`。
    #[allow(dead_code)]
    pub repo_path: PathBuf,
    /// 仓库相对路径,git 查询与磁盘读写都用它(`repo_path.join(file_path)`
    /// 拼出实际路径)。
    #[allow(dead_code)]
    pub file_path: PathBuf,
    pub entries: Vec<FileHistoryEntry>,
}

/// `build()` 没有显式指定 `max_count` 时的默认窗口,同
/// `git_log::DEFAULT_MAX_COMMITS` 的量级(见该常量文档)。
pub const DEFAULT_MAX_COUNT: usize = 200;

/// `diff_against_current()` patch 文本的字符数上限,超过就截断——同
/// `git_log.rs::MAX_PATCH_CHARS` 的理由(大 diff 一次性喂给 `text()` widget
/// 排版,布局开销肉眼可见),量级也保持一致。
const MAX_PATCH_CHARS: usize = 20_000;

/// 一次「查看此文件历史」的目标——右键哪个文件、属于哪个项目/仓库。
#[derive(Debug, Clone)]
pub struct FileHistoryTarget {
    /// 当前 `SnapshotLoaded` 只按 `repo_path`/`file_path` 核对目标(见 spec
    /// 「错误处理」),`project_id` 留在接口里供后续按项目核对,故
    /// `#[allow(dead_code)]`。
    #[allow(dead_code)]
    pub project_id: i64,
    pub repo_path: PathBuf,
    /// 仓库相对路径,`build`/`diff_against_current`/`rollback_to` 都吃它。
    pub file_path: PathBuf,
}

/// 当前已加载、给 CodeMirror diff webview 用的内容——`oid` 是加载时的选中
/// 版本快照,新结果落地前先核对还对不对得上"现在真正选中的",不对就丢弃
/// (同 `DiffLoaded` 的 target 核对手法,但这里额外要核对 `oid` 本身,因为
/// `DiffLoaded`/`DiffContentLoaded` 都只按 `(repo_path, file_path)` 核对
/// 目标,不看 `oid`——两条并行的加载各自要自己的 oid 匹配)。
#[derive(Debug, Clone)]
pub struct LoadedDiff {
    pub oid: git2::Oid,
    pub content: crate::extensions::git_log::DiffBlobContent,
}

/// 弹窗全部状态。整体以 `Option<State>` 挂在顶层 `App`(同
/// `project_link_menu` 的既有模式)——`None` 表示弹窗未打开。
#[derive(Default)]
pub struct State {
    target: Option<FileHistoryTarget>,
    snapshot: Option<Result<FileHistorySnapshot, String>>,
    selected: Option<git2::Oid>,
    /// 已经查过的 commit 各自的 diff 结果,按 oid 缓存,来回切换选中项不用
    /// 重复计算。
    diff_cache: HashMap<git2::Oid, Result<String, String>>,
    rollback_pending: Option<git2::Oid>,
    rollback_error: Option<String>,
    /// 当前选中版本已加载的 diff 内容(CodeMirror webview 用),与
    /// `diff_cache`(patch 文本,给 `colored_diff_lines` 用)并存。
    loaded_diff: Option<LoadedDiff>,
    /// 当前挂载的 diff webview 是否已确认 `ready`。`loaded_diff` 被清空时
    /// (换选中 / 回滚成功)连带置回 false——webview 即将因内容不可渲染或
    /// 即将重新加载而可能被摘掉/换绑,旧的 ready 状态不能带到下一份内容。
    diff_webview_ready: bool,
    /// 最近一次**确认送达** webview 的内容对应的 `oid`。跟 `loaded_diff`
    /// 的 `oid` 不一致就还需要再推一次。
    diff_sent_for: Option<git2::Oid>,
}

impl State {
    pub fn new(target: FileHistoryTarget) -> Self {
        Self {
            target: Some(target),
            ..Self::default()
        }
    }

    /// 目标只读访问器:作为 `State` 的公开接口被 plan 定义,当前 view/update
    /// 内部直接读私有字段、尚未从外部调用,故 `#[allow(dead_code)]`。
    #[allow(dead_code)]
    pub fn target(&self) -> Option<&FileHistoryTarget> {
        self.target.as_ref()
    }

    pub fn snapshot(&self) -> Option<&Result<FileHistorySnapshot, String>> {
        self.snapshot.as_ref()
    }

    pub fn selected(&self) -> Option<git2::Oid> {
        self.selected
    }

    pub fn diff_for(&self, oid: git2::Oid) -> Option<&Result<String, String>> {
        self.diff_cache.get(&oid)
    }

    pub fn loaded_diff(&self) -> Option<&LoadedDiff> {
        self.loaded_diff.as_ref()
    }

    pub fn diff_webview_ready(&self) -> bool {
        self.diff_webview_ready
    }

    pub fn diff_sent_for(&self) -> Option<git2::Oid> {
        self.diff_sent_for
    }

    pub(crate) fn set_diff_sent_for(&mut self, oid: git2::Oid) {
        self.diff_sent_for = Some(oid);
    }

    pub(crate) fn set_diff_webview_ready(&mut self, ready: bool) {
        self.diff_webview_ready = ready;
        if ready {
            self.diff_sent_for = None; // 强制下一帧重发一次当前内容。
        }
    }

    pub fn rollback_pending(&self) -> Option<git2::Oid> {
        self.rollback_pending
    }

    pub fn rollback_error(&self) -> Option<&str> {
        self.rollback_error.as_deref()
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Close,
    /// `(repo_path, file_path)` 用于核对结果落地时是不是仍是当前目标
    /// (弹窗打开期间项目被切走/关闭时,过期结果直接丢弃)。
    SnapshotLoaded(PathBuf, PathBuf, Result<FileHistorySnapshot, String>),
    SelectCommit(git2::Oid),
    /// `(repo_path, file_path)` 同 `SnapshotLoaded`——目标已切换(弹窗关了
    /// 又对另一个文件重开)时丢弃过期结果,不能只按 `oid` 判断:两个不同
    /// 文件的历史列表完全可能包含同一个 commit(比如一次全仓格式化提交),
    /// 若不核对目标,晚到达的旧文件 diff 会被错插进新文件的缓存里。
    DiffLoaded(PathBuf, PathBuf, git2::Oid, Result<String, String>),
    /// 选中版本的 blob/磁盘内容异步加载完成(CodeMirror 用,跟
    /// `DiffLoaded`——patch 文本、给 `colored_diff_lines` 用——并行、各自
    /// 独立缓存)。`(PathBuf, PathBuf)` 同 `DiffLoaded` 的目标核对手法。
    DiffContentLoaded(
        PathBuf,
        PathBuf,
        git2::Oid,
        Result<crate::extensions::git_log::DiffBlobContent, String>,
    ),
    RollbackRequest(git2::Oid),
    RollbackDone(git2::Oid, Result<(), String>),
}

/// 弹窗状态机。`state` 是 `&mut Option<State>`(不是 `&mut State`)——
/// `Message::Close` 需要能把它整个置回 `None`,同 `files::AppState.
/// context_menu` 的 `ContextMenuClose` 既有写法。
pub fn update(
    state: &mut Option<State>,
    msg: Message,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + Sync + 'static,
) {
    // 一个消息可能同时触发"patch 文本"和"CodeMirror 内容"两条异步加载,
    // 各要一份 `emit`;`emit` 本身不是 `Clone`,包一层 `Arc` 让两条
    // `spawn_blocking` 各自的 async 块都能持有一份引用计数。
    let emit = std::sync::Arc::new(emit);
    match msg {
        Message::Close => {
            *state = None;
        }
        Message::SnapshotLoaded(repo_path, file_path, result) => {
            let Some(s) = state else { return };
            let Some(target) = &s.target else { return };
            if target.repo_path != repo_path || target.file_path != file_path {
                return; // 已经不是当前目标(项目已切换/弹窗已重开),丢弃。
            }
            let first_oid = match &result {
                Ok(snapshot) => snapshot.entries.first().map(|e| e.oid),
                Err(_) => None,
            };
            s.snapshot = Some(result);
            if let Some(oid) = first_oid {
                s.selected = Some(oid);
                let target = s.target.as_ref().expect("刚核对过 target 非空");
                spawn_diff(target, oid, handle, &emit);
                spawn_diff_content(target, oid, handle, &emit);
            }
        }
        Message::SelectCommit(oid) => {
            let Some(s) = state else { return };
            s.selected = Some(oid);
            s.loaded_diff = None;
            s.diff_webview_ready = false;
            s.diff_sent_for = None;
            if !s.diff_cache.contains_key(&oid)
                && let Some(target) = &s.target
            {
                spawn_diff(target, oid, handle, &emit);
            }
            if let Some(target) = &s.target {
                spawn_diff_content(target, oid, handle, &emit);
            }
        }
        Message::DiffLoaded(repo_path, file_path, oid, result) => {
            let Some(s) = state else { return };
            let Some(target) = &s.target else { return };
            if target.repo_path != repo_path || target.file_path != file_path {
                return; // 已经不是当前目标,丢弃(见上面 `DiffLoaded` 文档)。
            }
            s.diff_cache.insert(oid, result);
        }
        Message::DiffContentLoaded(repo_path, file_path, oid, result) => {
            let Some(s) = state else { return };
            let Some(target) = &s.target else { return };
            if target.repo_path != repo_path || target.file_path != file_path {
                return; // 目标已切换,丢弃(同 DiffLoaded)。
            }
            if s.selected != Some(oid) {
                return; // 用户已经切到别的版本,这是一条迟到的结果。
            }
            s.loaded_diff = match result {
                Ok(content) => Some(LoadedDiff { oid, content }),
                Err(_) => None,
            };
        }
        Message::RollbackRequest(oid) => {
            let Some(s) = state else { return };
            let Some(target) = &s.target else { return };
            s.rollback_pending = Some(oid);
            s.rollback_error = None;
            let repo_path = target.repo_path.clone();
            let file_path = target.file_path.clone();
            handle.spawn(async move {
                let repo_path2 = repo_path.clone();
                let file_path2 = file_path.clone();
                let result =
                    tokio::task::spawn_blocking(move || rollback_to(&repo_path2, &file_path2, oid))
                        .await
                        .unwrap_or_else(|e| Err(format!("回滚任务失败: {e}")));
                emit(Message::RollbackDone(oid, result));
            });
        }
        Message::RollbackDone(_oid, result) => {
            let Some(s) = state else { return };
            s.rollback_pending = None;
            match result {
                Ok(()) => {
                    // 回滚改的是整个工作区文件的实时内容,之前缓存的所有
                    // diff(都是"某提交 vs 回滚前的工作区内容")全部失效,
                    // 不能只清掉被回滚到的这一个 oid。
                    s.diff_cache.clear();
                    s.loaded_diff = None;
                    s.diff_webview_ready = false;
                    s.diff_sent_for = None;
                    if let Some(target) = &s.target
                        && let Some(selected) = s.selected
                    {
                        spawn_diff(target, selected, handle, &emit);
                        spawn_diff_content(target, selected, handle, &emit);
                    }
                }
                Err(err) => {
                    s.rollback_error = Some(err);
                }
            }
        }
    }
}

fn spawn_diff<E>(
    target: &FileHistoryTarget,
    oid: git2::Oid,
    handle: &tokio::runtime::Handle,
    emit: &std::sync::Arc<E>,
) where
    E: Fn(Message) + Send + Sync + 'static,
{
    let repo_path = target.repo_path.clone();
    let file_path = target.file_path.clone();
    let emit = emit.clone();
    handle.spawn(async move {
        let repo_path2 = repo_path.clone();
        let file_path2 = file_path.clone();
        let result = tokio::task::spawn_blocking(move || {
            diff_against_current(&repo_path2, &file_path2, oid)
        })
        .await
        .unwrap_or_else(|e| Err(format!("diff 加载任务失败: {e}")));
        emit(Message::DiffLoaded(repo_path, file_path, oid, result));
    });
}

fn spawn_diff_content<E>(
    target: &FileHistoryTarget,
    oid: git2::Oid,
    handle: &tokio::runtime::Handle,
    emit: &std::sync::Arc<E>,
) where
    E: Fn(Message) + Send + Sync + 'static,
{
    let repo_path = target.repo_path.clone();
    let file_path = target.file_path.clone();
    let emit = emit.clone();
    handle.spawn(async move {
        let repo_path2 = repo_path.clone();
        let file_path2 = file_path.clone();
        let result = tokio::task::spawn_blocking(move || {
            let repo = git2::Repository::open(&repo_path2).map_err(|e| e.message().to_string())?;
            diff_blob_content_against_workdir(&repo, &repo_path2, &file_path2, oid)
        })
        .await
        .unwrap_or_else(|e| Err(format!("diff 内容加载任务失败: {e}")));
        emit(Message::DiffContentLoaded(
            repo_path, file_path, oid, result,
        ));
    });
}

/// 手写 revwalk:从 HEAD 开始逐提交,用
/// `repo.diff_tree_to_tree(parent_tree, tree, Some(&mut opts))` 配合
/// `DiffOptions::pathspec(file_path)` 判断这次提交是否碰过这个文件
/// (pathspec 下推给 git2 做,不用自己在结果里过滤),命中的收进结果,按
/// `max_count` 截断。没有 `git log --follow` 的 rename 跟踪。根提交(无父)
/// 按空树对比,逻辑同 `git_log.rs::commit_detail` 处理根提交的既有写法。
///
/// 注意:文件历史稀疏时(仓库有很多提交、这个文件只被改过几次)需要遍历
/// 大量提交才能凑够 `max_count` 条结果——这是 `git log -- <path>` 的固有
/// 特性,不是 bug,不额外做"扫描上限"截断(YAGNI)。
pub fn build(
    repo_path: &Path,
    file_path: &Path,
    max_count: usize,
) -> Result<FileHistorySnapshot, String> {
    let repo = git2::Repository::open(repo_path).map_err(|e| e.message().to_string())?;
    let pathspec = file_path.to_string_lossy().into_owned();

    let mut revwalk = repo.revwalk().map_err(|e| e.message().to_string())?;
    revwalk.push_head().map_err(|e| e.message().to_string())?;
    revwalk
        .set_sorting(git2::Sort::TIME)
        .map_err(|e| e.message().to_string())?;

    let mut entries = Vec::new();
    for oid in revwalk {
        if entries.len() >= max_count {
            break;
        }
        let oid = oid.map_err(|e| e.message().to_string())?;
        let commit = repo.find_commit(oid).map_err(|e| e.message().to_string())?;
        let new_tree = commit.tree().map_err(|e| e.message().to_string())?;
        let old_tree = match commit.parent(0) {
            Ok(parent) => Some(parent.tree().map_err(|e| e.message().to_string())?),
            Err(_) => None,
        };
        let mut opts = git2::DiffOptions::new();
        opts.pathspec(&pathspec);
        let diff = repo
            .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(&mut opts))
            .map_err(|e| e.message().to_string())?;
        if diff.deltas().next().is_none() {
            continue;
        }
        let full_sha = oid.to_string();
        let short_sha = full_sha.chars().take(7).collect();
        let summary = commit.summary().ok().flatten().unwrap_or("").to_string();
        let author = commit.author().name().ok().map(|n| n.to_string());
        let time = commit.time().seconds();
        entries.push(FileHistoryEntry {
            oid,
            short_sha,
            summary,
            author,
            time,
        });
    }

    Ok(FileHistorySnapshot {
        repo_path: repo_path.to_path_buf(),
        file_path: file_path.to_path_buf(),
        entries,
    })
}

/// `oid` 对应提交的树 vs *当前工作目录*的这一个文件,用
/// `repo.diff_tree_to_workdir(Some(&tree), Some(&mut opts))`(`opts` 配
/// `pathspec(file_path)`)。`diff_tree_to_workdir` 直接读磁盘上的实时内容
/// (不是索引/HEAD 里的版本,可能包含未提交改动),不需要自己
/// `std::fs::read` 再手动比较——git2 0.21 并未导出
/// `git_diff_blob_to_buffer` 这个 C API,没有"blob 对内存 buffer"直接
/// 比较的安全封装,这是选 `diff_tree_to_workdir` 而不是手动读两份内容比较
/// 的原因。两边内容相同时 `diff` 是空(0 个 delta),返回空字符串。
pub fn diff_against_current(
    repo_path: &Path,
    file_path: &Path,
    oid: git2::Oid,
) -> Result<String, String> {
    let repo = git2::Repository::open(repo_path).map_err(|e| e.message().to_string())?;
    let commit = repo.find_commit(oid).map_err(|e| e.message().to_string())?;
    let tree = commit.tree().map_err(|e| e.message().to_string())?;
    let mut opts = git2::DiffOptions::new();
    opts.pathspec(file_path.to_string_lossy().into_owned());
    let diff = repo
        .diff_tree_to_workdir(Some(&tree), Some(&mut opts))
        .map_err(|e| e.message().to_string())?;

    let mut patch = String::new();
    let mut truncated = false;
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        if truncated {
            return true;
        }
        if patch.len() >= MAX_PATCH_CHARS {
            truncated = true;
            patch.push_str("\n… diff 过长,已截断显示\n");
            return true;
        }
        let prefix = match line.origin() {
            '+' | '-' | ' ' => line.origin().to_string(),
            _ => String::new(),
        };
        patch.push_str(&prefix);
        patch.push_str(&String::from_utf8_lossy(line.content()));
        true
    })
    .map_err(|e| e.message().to_string())?;

    Ok(patch)
}

/// `oid` 对应提交树里 `file_path` 的历史内容 vs 磁盘上 `repo_path.join(
/// file_path)` 的实时内容。跟 `git_log::diff_blob_content`(两个 commit 之间)
/// 的关键差异:new 侧永远来自磁盘,不是另一个 blob;old 侧若该提交树里没有
/// 这个路径(历史记录本身是一次删除),按空字符串处理,不报错——这不是
/// 异常情况,是"文件历史"列表天然会包含的一种记录(`build()` 的 pathspec
/// 过滤只看"这次提交碰过这个路径",删除也算碰过)。
pub fn diff_blob_content_against_workdir(
    repo: &git2::Repository,
    repo_path: &Path,
    file_path: &Path,
    oid: git2::Oid,
) -> Result<crate::extensions::git_log::DiffBlobContent, String> {
    use crate::extensions::git_log::{DiffBlobContent, classify_diff_bytes};

    let commit = repo.find_commit(oid).map_err(|e| e.message().to_string())?;
    let tree = commit.tree().map_err(|e| e.message().to_string())?;
    let old_text = match tree.get_path(file_path) {
        Ok(entry) => {
            let obj = entry.to_object(repo).map_err(|e| e.message().to_string())?;
            match obj.as_blob() {
                Some(blob) => classify_diff_bytes(blob.content()),
                None => None, // 路径是目录/子模块,不是文件——判不可渲染。
            }
        }
        Err(_) => Some(String::new()), // 该提交树里没有这个路径:删除类历史记录。
    };
    let new_text = match std::fs::read(repo_path.join(file_path)) {
        Ok(bytes) => classify_diff_bytes(&bytes),
        Err(_) => None, // 磁盘文件已不存在/不可读。
    };
    match (old_text, new_text) {
        (Some(old_text), Some(new_text)) => Ok(DiffBlobContent::Text { old_text, new_text }),
        _ => Ok(DiffBlobContent::NotRenderable {
            reason: "文件不是文本、超过大小上限,或磁盘文件当前不存在,不支持 CodeMirror 渲染"
                .to_string(),
        }),
    }
}

/// 取 `oid` 对应提交树里 `file_path` 的 blob 字节,写入
/// `repo_path.join(file_path)`。不碰 git 索引,不 `git add`,是纯粹的文件
/// 系统写入——回滚后 git status 会显示这是一处未提交改动,交给用户/agent
/// 自行决定要不要提交。
pub fn rollback_to(repo_path: &Path, file_path: &Path, oid: git2::Oid) -> Result<(), String> {
    let repo = git2::Repository::open(repo_path).map_err(|e| e.message().to_string())?;
    let commit = repo.find_commit(oid).map_err(|e| e.message().to_string())?;
    let tree = commit.tree().map_err(|e| e.message().to_string())?;
    let entry = tree
        .get_path(file_path)
        .map_err(|e| e.message().to_string())?;
    let object = entry
        .to_object(&repo)
        .map_err(|e| e.message().to_string())?;
    let blob = object
        .into_blob()
        .map_err(|_| "该历史版本对应的不是一个文件".to_string())?;
    std::fs::write(repo_path.join(file_path), blob.content())
        .map_err(|e| format!("写入文件失败: {e}"))?;
    Ok(())
}

/// 取文件「上一版本」对应的 commit Oid——供文件树右键菜单「回滚」一键还原
/// 用。定义:最近一次修改该文件的提交(HEAD 版本)之前的那个版本;若该文件
/// 在整个仓库历史里只有一次提交(没有更早的版本),回落到那唯一一次提交
/// (等价于把工作区还原到最近一次提交、丢弃未提交改动);文件不在 git 跟踪内
/// (历史为空)则返回 `None`。复用 `build` 但只取前两条,避免拉满整份历史。
pub fn previous_oid(repo_path: &Path, file_path: &Path) -> Result<Option<git2::Oid>, String> {
    let snapshot = build(repo_path, file_path, 2)?;
    Ok(snapshot
        .entries
        .get(1)
        .or_else(|| snapshot.entries.get(0))
        .map(|e| e.oid))
}

/// 弹窗卡片本体(标题 + 左侧提交列表 + 右侧 diff 区),无外层居中容器——
/// 这次拆分是为了让独立 overlay 窗口(`platform/file_history_overlay.rs`)
/// 能直接复用同一份视图逻辑,只是换一个宿主(独立窗口取代 `App::view()`
/// 的 `stack!` 层)。`Length::Fill`:调用方现在总是给一块已经量好的
/// 画布(独立窗口整扇画布),不需要 `popup_view` 原本那种按
/// `window_width`/`window_height` 算 `Length::Fixed` 像素值再居中的
/// 语义(那部分逻辑挪进 `FileHistoryOverlay::open`/`reposition` 的
/// 窗口尺寸计算,不在这里)。
pub fn file_history_card(
    state: &State,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let title = row![
        icons::view(
            icons::IconKind::History,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().cream,
        ),
        text("文件历史")
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().cream),
        iced_widget::space::horizontal(),
        button(
            text("×")
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().dim)
        )
        .on_press(Message::Close)
        .style(|_t, _s| iced_widget::button::Style {
            background: None,
            text_color: byteui::theme::color::current().dim,
            ..iced_widget::button::Style::default()
        }),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let body = row![
        container(commit_list_view(state))
            .width(Length::Fixed(240.0))
            .height(Length::Fill),
        diff_area_view(state),
    ]
    .spacing(12)
    .height(Length::Fill);

    container(column![title, body].spacing(12))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(16)
        .style(crate::dialog::card_style)
        .into()
}

fn commit_list_view<'a>(
    state: &'a State,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(snapshot) = state.snapshot() else {
        return byteui::feedback::math_curve::loading_hint(
            byteui::feedback::math_curve::Curve::RoseThree,
            "加载中…",
            36.0,
        );
    };
    let entries = match snapshot {
        Ok(s) => &s.entries,
        Err(err) => {
            return container(
                text(err.clone())
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().red),
            )
            .padding(8)
            .into();
        }
    };
    if entries.is_empty() {
        return container(
            text("该文件没有提交记录")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
        .padding(8)
        .into();
    }
    let mut col = column![].spacing(4);
    for entry in entries {
        let is_selected = state.selected() == Some(entry.oid);
        let label = column![
            text(entry.summary.clone())
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().cream),
            text(format_commit_time(entry.time))
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(2);
        let row_btn = button(label)
            .on_press(Message::SelectCommit(entry.oid))
            .width(Length::Fill)
            .padding(8)
            .style(move |_t, _s| iced_widget::button::Style {
                background: if is_selected {
                    Some(byteui::theme::color::current().card.into())
                } else {
                    None
                },
                text_color: byteui::theme::color::current().body,
                ..iced_widget::button::Style::default()
            });
        col = col.push(row_btn);
    }
    scrollable(col)
        .direction(scrollable::Direction::Vertical(
            byteui::interaction::scrollbar::scrollbar(),
        ))
        .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style())
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn diff_area_view<'a>(
    state: &'a State,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(oid) = state.selected() else {
        return container(
            text("未选中版本")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
        .padding(8)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    };

    let header = row![
        text(format!("当前 vs {}", short_sha_of(state, oid)))
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().cream),
        iced_widget::space::horizontal(),
        rollback_button(state, oid),
    ]
    .spacing(8)
    .align_y(iced_widget::core::Alignment::Center);

    let content: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match state.loaded_diff() {
            Some(loaded)
                if loaded.oid == oid
                    && matches!(
                        loaded.content,
                        crate::extensions::git_log::DiffBlobContent::Text { .. }
                    ) =>
            {
                // CodeMirror 常开:留一块空区域给 Task 3 挂的 webview 合成
                // (同 git-log-diff 计划 Task 7 的手法)。
                container(iced_widget::Space::new())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
            Some(loaded) if loaded.oid == oid => {
                // 已加载但判定不可渲染(二进制/超限/磁盘文件缺失)。
                container(
                    text("(二进制文件、超出大小上限,或磁盘文件当前不存在,不支持预览)")
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().dim),
                )
                .padding(8)
                .into()
            }
            _ => match state.diff_for(oid) {
                None => byteui::feedback::math_curve::loading_hint(
                    byteui::feedback::math_curve::Curve::RoseThree,
                    "加载中…",
                    48.0,
                ),
                Some(Err(err)) => container(
                    text(err.clone())
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().red),
                )
                .padding(8)
                .into(),
                Some(Ok(patch)) if patch.is_empty() => container(
                    text("内容相同")
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().dim),
                )
                .padding(8)
                .into(),
                Some(Ok(patch)) => {
                    scrollable(crate::extensions::diff_render::colored_diff_lines(patch))
                        .direction(scrollable::Direction::Vertical(
                            byteui::interaction::scrollbar::scrollbar(),
                        ))
                        .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style())
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .into()
                }
            },
        };

    let mut col = column![header, content]
        .spacing(8)
        .width(Length::Fill)
        .height(Length::Fill);
    if let Some(err) = state.rollback_error() {
        col = col.push(
            text(format!("回滚失败: {err}"))
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().red),
        );
    }
    col.into()
}

fn rollback_button<'a>(
    state: &'a State,
    oid: git2::Oid,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let disabled = state.rollback_pending().is_some();
    button(
        text("回滚")
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().gold),
    )
    .on_press_maybe((!disabled).then_some(Message::RollbackRequest(oid)))
    .padding([4, 10])
    .style(crate::dialog::action_button_style(
        byteui::theme::color::current().gold,
    ))
    .into()
}

fn short_sha_of(state: &State, oid: git2::Oid) -> String {
    state
        .snapshot()
        .and_then(|r| r.as_ref().ok())
        .and_then(|s| s.entries.iter().find(|e| e.oid == oid))
        .map(|e| e.short_sha.clone())
        .unwrap_or_else(|| oid.to_string().chars().take(7).collect())
}

/// commit 时间戳格式化,`YYYY-MM-DD HH:MM:SS`,UTC。跟
/// `git_log.rs::format_commit_time` 同源但那边是模块私有、不便跨模块复用
/// (该函数自己的文档也是这么处理 `todo.rs` 同名函数的),这里照抄一份。
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
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(repo: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(args)
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(st.status.success(), "git {args:?}: {st:?}");
    }

    /// c1: 新建 a.txt("one\n") / c2: 新建无关的 b.txt / c3: 改 a.txt 为
    /// "two\n"。只有 c1/c3 该出现在 `build(repo, "a.txt", _)` 的结果里。
    fn mkrepo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "c1: add a.txt"]);
        std::fs::write(repo.join("b.txt"), "unrelated\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "c2: add b.txt"]);
        std::fs::write(repo.join("a.txt"), "two\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "c3: change a.txt"]);
        (dir, repo)
    }

    fn target() -> FileHistoryTarget {
        FileHistoryTarget {
            project_id: 1,
            repo_path: PathBuf::from("/tmp/dozer-file-history-test-repo"),
            file_path: PathBuf::from("a.txt"),
        }
    }

    /// `mkrepo()` 的 c1/c2/c3 之上追加 c4:删除 `a.txt`。
    fn mkrepo_with_delete_commit() -> (tempfile::TempDir, PathBuf) {
        let (dir, repo) = mkrepo();
        git(&repo, &["rm", "-q", "a.txt"]);
        git(&repo, &["commit", "-qm", "c4: delete a.txt"]);
        (dir, repo)
    }

    #[test]
    fn diff_blob_content_against_workdir_reads_historical_and_current() {
        let (_dir, repo) = mkrepo();
        let git_repo = git2::Repository::open(&repo).unwrap();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c1_oid = snapshot.entries[1].oid; // c1: a.txt == "one\n"
        // 磁盘当前内容是 c3 之后的 "two\n"。
        let content =
            diff_blob_content_against_workdir(&git_repo, &repo, Path::new("a.txt"), c1_oid)
                .expect("应能读出历史版本与磁盘当前内容");
        let crate::extensions::git_log::DiffBlobContent::Text { old_text, new_text } = content
        else {
            panic!("正常改动文件应判定为可渲染文本");
        };
        assert_eq!(old_text, "one\n");
        assert_eq!(new_text, "two\n");
    }

    #[test]
    fn diff_blob_content_against_workdir_empty_old_side_for_deletion_commit() {
        let (_dir, repo) = mkrepo_with_delete_commit();
        // 删除后又重新创建同名文件:此时"c4 那次提交"在它的树里没有这个
        // 路径(旧侧为空),但磁盘上文件在(新侧有内容)——正是"删除类历史
        // 记录"里可渲染的那一种。
        std::fs::write(repo.join("a.txt"), "reborn\n").unwrap();
        let git_repo = git2::Repository::open(&repo).unwrap();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c4_oid = snapshot.entries[0].oid; // c4: 删除 a.txt,该提交树里没有这个路径
        let content =
            diff_blob_content_against_workdir(&git_repo, &repo, Path::new("a.txt"), c4_oid)
                .expect("删除类历史记录不应报错");
        let crate::extensions::git_log::DiffBlobContent::Text { old_text, new_text } = content
        else {
            panic!("应判定为可渲染文本(旧侧为空)");
        };
        assert_eq!(old_text, "", "该提交树里没有这个路径,旧侧按空字符串处理");
        assert_eq!(new_text, "reborn\n");
    }

    #[test]
    fn diff_blob_content_against_workdir_not_renderable_when_disk_file_missing() {
        let (_dir, repo) = mkrepo();
        let git_repo = git2::Repository::open(&repo).unwrap();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c1_oid = snapshot.entries[1].oid;
        std::fs::remove_file(repo.join("a.txt")).unwrap();
        let content =
            diff_blob_content_against_workdir(&git_repo, &repo, Path::new("a.txt"), c1_oid)
                .expect("磁盘文件缺失不应报错,应判定为不可渲染");
        assert!(matches!(
            content,
            crate::extensions::git_log::DiffBlobContent::NotRenderable { .. }
        ));
    }

    fn fake_oid(byte: u8) -> git2::Oid {
        git2::Oid::from_bytes(&[byte; 20]).unwrap()
    }

    fn snapshot_with(entries: Vec<FileHistoryEntry>) -> FileHistorySnapshot {
        FileHistorySnapshot {
            repo_path: target().repo_path,
            file_path: target().file_path,
            entries,
        }
    }

    fn fake_entry(oid: git2::Oid, summary: &str) -> FileHistoryEntry {
        FileHistoryEntry {
            oid,
            short_sha: oid.to_string().chars().take(7).collect(),
            summary: summary.to_string(),
            author: None,
            time: 0,
        }
    }

    #[test]
    fn build_only_returns_commits_touching_the_file() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("a.txt"), 10).expect("build 应成功");
        assert_eq!(snapshot.entries.len(), 2, "只有 c1/c3 改过 a.txt");
        assert_eq!(
            snapshot.entries[0].summary, "c3: change a.txt",
            "时间倒序,最新的排最前"
        );
        assert_eq!(snapshot.entries[1].summary, "c1: add a.txt");
    }

    #[test]
    fn build_respects_max_count() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("a.txt"), 1).expect("build 应成功");
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].summary, "c3: change a.txt");
    }

    #[test]
    fn build_returns_empty_for_file_never_touched() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("never-existed.txt"), 10).expect("build 应成功");
        assert!(snapshot.entries.is_empty());
    }

    #[test]
    fn diff_against_current_empty_when_content_matches_historical_version() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c1_oid = snapshot.entries[1].oid; // c1: a.txt == "one\n"
        // 手动把磁盘内容改回 c1 时的样子,不提交——`diff_against_current`
        // 读的是磁盘实时内容,不是 HEAD。
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        let patch = diff_against_current(&repo, Path::new("a.txt"), c1_oid).unwrap();
        assert!(patch.is_empty(), "内容相同应返回空 patch: {patch:?}");
    }

    #[test]
    fn diff_against_current_shows_changes_when_content_differs() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c1_oid = snapshot.entries[1].oid; // c1: a.txt == "one\n"
        // 当前磁盘内容是 c3 之后的 "two\n",跟 c1 的 "one\n" 不同。
        let patch = diff_against_current(&repo, Path::new("a.txt"), c1_oid).unwrap();
        assert!(patch.contains("-one"), "应包含删除旧行: {patch:?}");
        assert!(patch.contains("+two"), "应包含新增行: {patch:?}");
    }

    #[test]
    fn rollback_to_writes_historical_bytes_to_disk() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c1_oid = snapshot.entries[1].oid; // c1: a.txt == "one\n"
        rollback_to(&repo, Path::new("a.txt"), c1_oid).expect("回滚应成功");
        let content = std::fs::read_to_string(repo.join("a.txt")).unwrap();
        assert_eq!(content, "one\n", "回滚后磁盘内容应变回 c1 时的字节");
    }

    #[test]
    fn rollback_to_then_diff_against_current_is_empty() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c1_oid = snapshot.entries[1].oid;
        rollback_to(&repo, Path::new("a.txt"), c1_oid).unwrap();
        let patch = diff_against_current(&repo, Path::new("a.txt"), c1_oid).unwrap();
        assert!(
            patch.is_empty(),
            "回滚后跟同一版本对比应是内容相同: {patch:?}"
        );
    }

    #[tokio::test]
    async fn close_clears_state() {
        let mut state = Some(State::new(target()));
        let handle = tokio::runtime::Handle::current();
        update(&mut state, Message::Close, &handle, |_| {});
        assert!(state.is_none());
    }

    #[tokio::test]
    async fn snapshot_loaded_ignores_result_for_different_repo_path() {
        let mut state = Some(State::new(target()));
        let handle = tokio::runtime::Handle::current();
        let other_repo = PathBuf::from("/tmp/other-repo");
        update(
            &mut state,
            Message::SnapshotLoaded(
                other_repo,
                target().file_path,
                Ok(snapshot_with(vec![fake_entry(fake_oid(1), "s")])),
            ),
            &handle,
            |_| panic!("过期目标的结果不该触发任何 emit"),
        );
        assert!(
            state.as_ref().unwrap().snapshot().is_none(),
            "repo_path 不匹配,结果应被丢弃"
        );
    }

    #[tokio::test]
    async fn snapshot_loaded_with_empty_entries_selects_nothing() {
        let mut state = Some(State::new(target()));
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::SnapshotLoaded(
                target().repo_path,
                target().file_path,
                Ok(snapshot_with(Vec::new())),
            ),
            &handle,
            |_| panic!("空历史列表不该触发 diff 查询"),
        );
        let s = state.unwrap();
        assert!(s.snapshot().unwrap().as_ref().unwrap().entries.is_empty());
        assert!(s.selected().is_none());
    }

    #[tokio::test]
    async fn snapshot_loaded_with_entries_selects_first() {
        let mut state = Some(State::new(target()));
        let handle = tokio::runtime::Handle::current();
        let oid1 = fake_oid(1);
        let oid2 = fake_oid(2);
        update(
            &mut state,
            Message::SnapshotLoaded(
                target().repo_path,
                target().file_path,
                Ok(snapshot_with(vec![
                    fake_entry(oid1, "newest"),
                    fake_entry(oid2, "older"),
                ])),
            ),
            &handle,
            |_| {},
        );
        assert_eq!(state.unwrap().selected(), Some(oid1));
    }

    #[tokio::test]
    async fn select_commit_with_cached_diff_does_not_respawn() {
        let mut state = Some(State::new(target()));
        let oid = fake_oid(3);
        {
            let s = state.as_mut().unwrap();
            s.diff_cache.insert(oid, Ok("cached".to_string()));
        }
        let handle = tokio::runtime::Handle::current();
        update(&mut state, Message::SelectCommit(oid), &handle, |_| {
            panic!("已缓存的 diff 不该重新触发查询")
        });
        assert_eq!(state.unwrap().selected(), Some(oid));
    }

    #[tokio::test]
    async fn diff_loaded_caches_result() {
        let mut state = Some(State::new(target()));
        let oid = fake_oid(4);
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DiffLoaded(
                target().repo_path,
                target().file_path,
                oid,
                Ok("patch text".to_string()),
            ),
            &handle,
            |_| {},
        );
        assert_eq!(
            state.unwrap().diff_for(oid),
            Some(&Ok("patch text".to_string()))
        );
    }

    #[tokio::test]
    async fn diff_loaded_ignores_result_for_different_target() {
        // 弹窗对文件 A 打开时发出的 diff 查询,晚到达时弹窗已经关了又对
        // 另一个文件 B 重开——不能把 A 的 diff 插进 B 的缓存里(即便两者
        // 恰好用了同一个 oid,比如一次全仓格式化提交)。
        let mut state = Some(State::new(target()));
        let oid = fake_oid(8);
        let other_file = PathBuf::from("b.txt");
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DiffLoaded(
                target().repo_path,
                other_file,
                oid,
                Ok("diff for a different file".to_string()),
            ),
            &handle,
            |_| {},
        );
        assert!(
            state.unwrap().diff_for(oid).is_none(),
            "file_path 不匹配,结果应被丢弃"
        );
    }

    #[tokio::test]
    async fn rollback_request_sets_pending_and_clears_previous_error() {
        let mut state = Some(State::new(target()));
        {
            let s = state.as_mut().unwrap();
            s.rollback_error = Some("上一次失败".to_string());
        }
        let oid = fake_oid(5);
        let handle = tokio::runtime::Handle::current();
        update(&mut state, Message::RollbackRequest(oid), &handle, |_| {});
        let s = state.unwrap();
        assert_eq!(s.rollback_pending(), Some(oid));
        assert!(s.rollback_error().is_none());
    }

    #[tokio::test]
    async fn rollback_done_err_sets_error_and_clears_pending() {
        let mut state = Some(State::new(target()));
        let oid = fake_oid(6);
        {
            let s = state.as_mut().unwrap();
            s.rollback_pending = Some(oid);
        }
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::RollbackDone(oid, Err("写入失败".to_string())),
            &handle,
            |_| {},
        );
        let s = state.unwrap();
        assert!(s.rollback_pending().is_none());
        assert_eq!(s.rollback_error(), Some("写入失败"));
    }

    #[tokio::test]
    async fn rollback_done_ok_clears_entire_diff_cache() {
        // 回滚改的是整个工作区文件的实时内容,不能只清掉被回滚到的这一个
        // oid——之前为其它提交缓存的 diff(都是"某提交 vs 回滚前的工作区
        // 内容")全部失效,必须整体清空。
        let mut state = Some(State::new(target()));
        let rolled_back_oid = fake_oid(7);
        let other_oid = fake_oid(8);
        {
            let s = state.as_mut().unwrap();
            s.rollback_pending = Some(rolled_back_oid);
            s.selected = Some(other_oid);
            s.diff_cache.insert(rolled_back_oid, Ok("a".to_string()));
            s.diff_cache.insert(other_oid, Ok("b".to_string()));
        }
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::RollbackDone(rolled_back_oid, Ok(())),
            &handle,
            |_| {},
        );
        let s = state.unwrap();
        assert!(s.rollback_pending().is_none());
        assert!(s.diff_for(rolled_back_oid).is_none());
        assert!(
            s.diff_for(other_oid).is_none(),
            "回滚后其它提交的旧缓存也该失效,不能只清被回滚到的那一个"
        );
    }

    #[tokio::test]
    async fn diff_content_loaded_ignored_when_target_no_longer_matches() {
        let mut state = Some(State::new(target()));
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DiffContentLoaded(
                PathBuf::from("/tmp/some-other-repo"),
                target().file_path,
                fake_oid(1),
                Ok(crate::extensions::git_log::DiffBlobContent::Text {
                    old_text: "a".into(),
                    new_text: "b".into(),
                }),
            ),
            &handle,
            |_| {},
        );
        assert!(
            state.as_ref().unwrap().loaded_diff().is_none(),
            "repo_path 对不上目标,结果应被丢弃"
        );
    }

    #[tokio::test]
    async fn diff_content_loaded_applied_when_target_matches() {
        let mut state = Some(State::new(target()));
        if let Some(s) = &mut state {
            s.selected = Some(fake_oid(1)); // 迟到结果核对:必须是当前选中项。
        }
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DiffContentLoaded(
                target().repo_path,
                target().file_path,
                fake_oid(1),
                Ok(crate::extensions::git_log::DiffBlobContent::Text {
                    old_text: "a".into(),
                    new_text: "b".into(),
                }),
            ),
            &handle,
            |_| {},
        );
        let loaded = state
            .as_ref()
            .unwrap()
            .loaded_diff()
            .expect("目标匹配的结果应该落地");
        assert_eq!(loaded.oid, fake_oid(1));
        assert!(matches!(
            loaded.content,
            crate::extensions::git_log::DiffBlobContent::Text { .. }
        ));
    }

    #[tokio::test]
    async fn diff_content_loaded_ignored_when_oid_no_longer_selected() {
        let mut state = Some(State::new(target()));
        if let Some(s) = &mut state {
            s.selected = Some(fake_oid(2)); // 用户已切到别的版本。
        }
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DiffContentLoaded(
                target().repo_path,
                target().file_path,
                fake_oid(1),
                Ok(crate::extensions::git_log::DiffBlobContent::Text {
                    old_text: "a".into(),
                    new_text: "b".into(),
                }),
            ),
            &handle,
            |_| {},
        );
        assert!(
            state.as_ref().unwrap().loaded_diff().is_none(),
            "oid 已不是当前选中项,迟到的结果应丢弃"
        );
    }

    #[tokio::test]
    async fn select_commit_clears_stale_loaded_diff() {
        let mut state = Some(State::new(target()));
        if let Some(s) = &mut state {
            s.loaded_diff = Some(LoadedDiff {
                oid: fake_oid(9),
                content: crate::extensions::git_log::DiffBlobContent::Text {
                    old_text: "old".into(),
                    new_text: "old".into(),
                },
            });
        }
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::SelectCommit(fake_oid(2)),
            &handle,
            |_| {},
        );
        assert!(
            state.as_ref().unwrap().loaded_diff().is_none(),
            "换选中提交后必须先清空旧内容"
        );
    }

    #[tokio::test]
    async fn rollback_done_success_clears_loaded_diff() {
        let mut state = Some(State::new(target()));
        if let Some(s) = &mut state {
            s.selected = Some(fake_oid(3));
            s.loaded_diff = Some(LoadedDiff {
                oid: fake_oid(3),
                content: crate::extensions::git_log::DiffBlobContent::Text {
                    old_text: "old".into(),
                    new_text: "old".into(),
                },
            });
        }
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::RollbackDone(fake_oid(3), Ok(())),
            &handle,
            |_| {},
        );
        assert!(
            state.as_ref().unwrap().loaded_diff().is_none(),
            "回滚成功后磁盘内容已变,旧的 CodeMirror 内容必须清空、等待重新加载"
        );
    }

    #[tokio::test]
    async fn set_diff_webview_ready_resets_sent_oid() {
        let mut state = Some(State::new(target()));
        let s = state.as_mut().unwrap();
        s.set_diff_sent_for(fake_oid(1));
        assert_eq!(s.diff_sent_for(), Some(fake_oid(1)));
        s.set_diff_webview_ready(true);
        assert!(s.diff_webview_ready());
        assert!(
            s.diff_sent_for().is_none(),
            "webview 确认 ready 后要强制下一帧重发一次当前内容"
        );
    }
}
