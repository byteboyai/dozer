//! 收藏夹纯逻辑:当前 URL 的收藏状态与乐观本地更新。

use dozer_model::{BookmarkInfo, BookmarkScope};

/// 乐观本地插入的占位 id:落库前不知道真实自增 id,只在"点击→下一次
/// 全量刷新落地"这一帧内部当哨兵用,不参与任何持久化或跨帧比较。
pub(crate) const OPTIMISTIC_BOOKMARK_ID: i64 = -1;

/// 当前 URL 在全局/本项目两边各自的收藏状态(`Some(id)` = 已收藏,
/// `id` 是"移出收藏"要传的记录 id)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookmarkStatus {
    pub global: Option<i64>,
    pub project: Option<i64>,
}

impl BookmarkStatus {
    pub fn is_bookmarked(&self) -> bool {
        self.global.is_some() || self.project.is_some()
    }
}

pub fn bookmark_status(
    bookmarks: &[BookmarkInfo],
    url: &str,
    project_id: Option<i64>,
) -> BookmarkStatus {
    let global = bookmarks
        .iter()
        .find(|b| b.scope == BookmarkScope::Global && b.url == url)
        .map(|b| b.id);
    let project = project_id.and_then(|pid| {
        bookmarks
            .iter()
            .find(|b| {
                b.scope == BookmarkScope::Project && b.project_id == Some(pid) && b.url == url
            })
            .map(|b| b.id)
    });
    BookmarkStatus { global, project }
}

/// 已存在(同 scope+project_id+url)则 no-op,幂等,与 dozerd 侧
/// `INSERT OR IGNORE` 语义一致。
pub(crate) fn optimistic_add(
    bookmarks: &mut Vec<BookmarkInfo>,
    scope: BookmarkScope,
    project_id: Option<i64>,
    url: &str,
    title: &str,
    created_ms: u64,
) {
    let exists = bookmarks
        .iter()
        .any(|b| b.scope == scope && b.project_id == project_id && b.url == url);
    if exists {
        return;
    }
    bookmarks.push(BookmarkInfo {
        id: OPTIMISTIC_BOOKMARK_ID,
        scope,
        project_id,
        url: url.to_string(),
        title: title.to_string(),
        created_ms,
    });
}

/// 按 id 过滤;未知 id 是 no-op,同 dozerd 侧 `remove` 语义。
pub(crate) fn optimistic_remove(bookmarks: &mut Vec<BookmarkInfo>, id: i64) {
    bookmarks.retain(|b| b.id != id);
}
