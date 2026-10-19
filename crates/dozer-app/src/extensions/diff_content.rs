//! diff 面板共用的"两侧文本"内容:`git_log`(提交 vs 提交)与 `file_history`
//! (提交 vs 磁盘)都产出同一个 [`DiffBlobContent`],交给 CodeMirror diff 渲染。
//!
//! 这里是两个面板之间的中立层——`file_history` 不再 import `git_log`。
//! 内容读取与"能不能当文本"的判定在 `bytegit`,这里只负责把它的分类结果折成 UI 要的
//! 二选一(可渲染 / 不可渲染)与占位文案。

use bytegit::{BlobId, CommitId, Content, ContentLimits, ContentPair, Repo};
use std::path::Path;

/// 单侧内容的字节上限(old/new 各自判定),超过就判定"不可渲染"。
pub const MAX_DIFF_BLOB_BYTES: usize = 512 * 1024;

const LIMITS: ContentLimits = ContentLimits::new(MAX_DIFF_BLOB_BYTES);

/// 提交 vs 提交的不可渲染占位文案(owner/key,渲染端按 locale 解析)。
const REASON_BLOB_KEY: (&str, &str) = ("dozer.git_log", "empty.diff_unrenderable_blob");

/// 提交 vs 磁盘的不可渲染占位文案(多一种"磁盘文件不存在"的原因)。
const REASON_WORKDIR_KEY: (&str, &str) = ("dozer.host", "file_history.unrenderable_workdir");

/// 要么是可渲染的双侧文本,要么给出原因(供 UI 占位文案使用)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffBlobContent {
    Text { old_text: String, new_text: String },
    NotRenderable { reason: dozer_model::LocalizedText },
}

fn not_renderable((owner, key): (&str, &str)) -> DiffBlobContent {
    DiffBlobContent::NotRenderable {
        reason: dozer_model::LocalizedText::keyed(owner, key, ""),
    }
}

/// 向上查找项目目录所属的仓库(项目在仓库子目录里也行),失败信息可直接展示。
/// 只给不涉及"项目内路径"的查询用(如提交详情、blob 内容);带 `file_path` 的查询用 [`open_project_repo`]。
pub(crate) fn open_repo(project: &Path) -> Result<Repo, String> {
    Repo::discover(project).map_err(|e| e.message().to_string())
}

/// 项目目录所属的仓库,以及项目目录相对仓库根的前缀(项目就是仓库根时为空)。
///
/// 面板拿到的 `file_path` 是**相对项目根**的,而 bytegit 的路径一律相对**仓库根**:项目在仓库子目录里时
/// 必须先经 [`ProjectRepo::in_repo`] 换算,否则会查错文件(迁移前用只认仓库根的 `open_exact` 规避了这个问题,
/// 代价是子目录项目的历史面板直接报错;D14 统一成向上查找)。
pub(crate) struct ProjectRepo {
    pub repo: Repo,
    prefix: std::path::PathBuf,
}

impl ProjectRepo {
    /// 项目内路径 → 仓库内路径。
    pub fn in_repo(&self, project_relative: &Path) -> std::path::PathBuf {
        self.prefix.join(project_relative)
    }
}

pub(crate) fn open_project_repo(project: &Path) -> Result<ProjectRepo, String> {
    let repo = open_repo(project)?;
    // `Repo::root` 是 libgit2 解析后的真实路径(macOS 上 `/var` → `/private/var`),所以两边都先规范化再比较。
    let root = std::fs::canonicalize(repo.root()).map_err(|e| e.to_string())?;
    let project = std::fs::canonicalize(project).map_err(|e| e.to_string())?;
    let prefix = project
        .strip_prefix(&root)
        .map_err(|_| {
            // 这里拿不到当前 locale(调用方多是后台任务),按英文 fallback 出文,
            // 与同目录其它后台错误一致:key 在 host 资源里,界面侧不会再翻一次。
            crate::i18n::host_text_args(
                "file_history.project_outside_repo",
                "{project} is not inside the repository work tree",
                "",
                &[("project", &project.display().to_string())],
            )
        })?
        .to_path_buf();
    Ok(ProjectRepo { repo, prefix })
}

/// 两个 blob 之间的双侧文本。`None` 侧按新增/删除文件语义当空字符串;
/// 非 `None` 侧任一超过 [`MAX_DIFF_BLOB_BYTES`]、含二进制内容或不是合法 UTF-8 都判定
/// "不可渲染"——不做部分截断渲染。
pub fn blob_pair_content(
    repo: &Repo,
    old_blob: Option<BlobId>,
    new_blob: Option<BlobId>,
) -> Result<DiffBlobContent, String> {
    let side = |blob: Option<BlobId>| -> Result<Option<String>, String> {
        match blob {
            None => Ok(Some(String::new())),
            Some(id) => repo
                .blob_text(id, LIMITS)
                .map(Content::into_text)
                .map_err(|e| e.message().to_string()),
        }
    };
    match (side(old_blob)?, side(new_blob)?) {
        (Some(old_text), Some(new_text)) => Ok(DiffBlobContent::Text { old_text, new_text }),
        _ => Ok(not_renderable(REASON_BLOB_KEY)),
    }
}

/// `commit` 里 `path` 的历史内容 vs 磁盘上的实时内容。旧侧在该提交里不存在时按空字符串
/// (这是"文件历史"列表天然会包含的删除类记录,不是异常);新侧缺失/不可读视为不可渲染。
pub fn workdir_content(
    repo: &Repo,
    commit: CommitId,
    path: &Path,
) -> Result<DiffBlobContent, String> {
    let ContentPair { old, new } = repo
        .workdir_vs_commit(commit, path, LIMITS)
        .map_err(|e| e.message().to_string())?;
    let old_text = match old {
        None => Some(String::new()),
        Some(content) => content.into_text(),
    };
    let new_text = new.and_then(Content::into_text);
    match (old_text, new_text) {
        (Some(old_text), Some(new_text)) => Ok(DiffBlobContent::Text { old_text, new_text }),
        _ => Ok(not_renderable(REASON_WORKDIR_KEY)),
    }
}
