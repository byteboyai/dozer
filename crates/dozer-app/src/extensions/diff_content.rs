//! diff 面板共用的"两侧文本"内容:`git_log`(提交 vs 提交)与 `file_history`
//! (提交 vs 磁盘)都产出同一个 [`DiffBlobContent`],交给 CodeMirror diff 渲染。
//!
//! 这里是两个面板之间的中立层——`file_history` 不再 import `git_log`。
//! 内容读取与"能不能当文本"的判定在 `bytegit`,这里只负责把它的分类结果折成 UI 要的
//! 二选一(可渲染 / 不可渲染)与占位文案。

use bytegit::{CommitId, Content, ContentLimits, ContentPair, Repo};
use std::path::Path;

/// 单侧内容的字节上限(old/new 各自判定),超过就判定"不可渲染"。
pub const MAX_DIFF_BLOB_BYTES: usize = 512 * 1024;

const LIMITS: ContentLimits = ContentLimits::new(MAX_DIFF_BLOB_BYTES);

/// 提交 vs 磁盘的不可渲染占位文案(多一种"磁盘文件不存在"的原因)。
const REASON_WORKDIR: &str =
    "文件不是文本、超过大小上限,或磁盘文件当前不存在,不支持 CodeMirror 渲染";

/// 要么是可渲染的双侧文本,要么给出原因(供 UI 占位文案使用)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffBlobContent {
    Text { old_text: String, new_text: String },
    NotRenderable { reason: String },
}

fn not_renderable(reason: &str) -> DiffBlobContent {
    DiffBlobContent::NotRenderable {
        reason: reason.to_string(),
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
        _ => Ok(not_renderable(REASON_WORKDIR)),
    }
}
