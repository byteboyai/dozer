//! 浏览器收藏夹:纯逻辑辅助,不碰 iced/网络。
//!
//! `Workspace::bookmarks` 是"全局 + 当前项目"合集的本地缓存,由
//! `ListBookmarks` 落地时整份替换(见 `workspace.rs`
//! `spawn_bookmarks_refresh`)。这里只提供两类纯函数:
//! - `bookmark_status`:给定当前 URL,判定它在全局/本项目两边各自是否
//!   已收藏(星标图标颜色、小菜单"加入"/"移出"文案据此渲染)。
//! - `optimistic_add`/`optimistic_remove`:用户点击的当帧本地立即改
//!   `Workspace::bookmarks`,不等 dozerd 往返——星标/面板立即反馈,真实
//!   持久化结果由随后一次 `ListBookmarks` 全量刷新纠正(不做显式回滚,
//!   见设计文档"数据流与状态机"一节)。

use dozer_core::protocol::{BookmarkInfo, BookmarkScope};

/// 乐观本地插入的占位 id:落库前不知道真实自增 id,只在"点击→下一次
/// 全量刷新落地"这一帧内部当哨兵用,不参与任何持久化或跨帧比较。
pub const OPTIMISTIC_BOOKMARK_ID: i64 = -1;

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
pub fn optimistic_add(
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
pub fn optimistic_remove(bookmarks: &mut Vec<BookmarkInfo>, id: i64) {
    bookmarks.retain(|b| b.id != id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bm(id: i64, scope: BookmarkScope, project_id: Option<i64>, url: &str) -> BookmarkInfo {
        BookmarkInfo {
            id,
            scope,
            project_id,
            url: url.into(),
            title: url.into(),
            created_ms: 0,
        }
    }

    #[test]
    fn bookmark_status_detects_global_and_project_independently() {
        let list = vec![
            bm(1, BookmarkScope::Global, None, "https://a.com"),
            bm(2, BookmarkScope::Project, Some(7), "https://a.com"),
        ];
        let status = bookmark_status(&list, "https://a.com", Some(7));
        assert_eq!(status.global, Some(1));
        assert_eq!(status.project, Some(2));
        assert!(status.is_bookmarked());
    }

    #[test]
    fn bookmark_status_project_none_when_no_project_open() {
        let list = vec![bm(1, BookmarkScope::Global, None, "https://a.com")];
        let status = bookmark_status(&list, "https://a.com", None);
        assert_eq!(status.global, Some(1));
        assert_eq!(status.project, None);
    }

    #[test]
    fn bookmark_status_project_scoped_to_current_project_only() {
        let list = vec![bm(1, BookmarkScope::Project, Some(7), "https://a.com")];
        let status = bookmark_status(&list, "https://a.com", Some(8));
        assert_eq!(status.project, None, "不该看到别的项目的收藏");
    }

    #[test]
    fn is_bookmarked_false_when_neither_side_has_it() {
        let list: Vec<BookmarkInfo> = vec![];
        assert!(!bookmark_status(&list, "https://a.com", Some(1)).is_bookmarked());
    }

    #[test]
    fn optimistic_add_appends_new_entry() {
        let mut list = vec![];
        optimistic_add(
            &mut list,
            BookmarkScope::Global,
            None,
            "https://a.com",
            "A",
            100,
        );
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, OPTIMISTIC_BOOKMARK_ID);
        assert_eq!(list[0].title, "A");
    }

    #[test]
    fn optimistic_add_is_idempotent_for_same_scope_and_url() {
        let mut list = vec![];
        optimistic_add(
            &mut list,
            BookmarkScope::Global,
            None,
            "https://a.com",
            "A",
            100,
        );
        optimistic_add(
            &mut list,
            BookmarkScope::Global,
            None,
            "https://a.com",
            "改名",
            200,
        );
        assert_eq!(list.len(), 1, "已存在则不重复插入");
        assert_eq!(list[0].title, "A");
    }

    #[test]
    fn optimistic_add_allows_same_url_in_different_scope() {
        let mut list = vec![];
        optimistic_add(
            &mut list,
            BookmarkScope::Global,
            None,
            "https://a.com",
            "A",
            100,
        );
        optimistic_add(
            &mut list,
            BookmarkScope::Project,
            Some(1),
            "https://a.com",
            "A",
            100,
        );
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn optimistic_remove_filters_by_id() {
        let mut list = vec![
            bm(1, BookmarkScope::Global, None, "https://a.com"),
            bm(2, BookmarkScope::Global, None, "https://b.com"),
        ];
        optimistic_remove(&mut list, 1);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, 2);
    }

    #[test]
    fn optimistic_remove_unknown_id_is_noop() {
        let mut list = vec![bm(1, BookmarkScope::Global, None, "https://a.com")];
        optimistic_remove(&mut list, 999);
        assert_eq!(list.len(), 1);
    }
}
