//! 浏览器面板扩展(阶段 1 扩展化重构第二个试点)。自己的 `Message`/
//! `State`/`update`/`view`,内核只认一个包装变体 `Message::Browser(..)`
//! 做转发,不知道自己被包在哪个外层类型里。收藏夹(全局/本项目两级书签)
//! 是浏览器域专属功能,状态/消息/UI 整体并入这个模块。
//!
//! `Tabs` 是浏览器专用的精简 tab/地址栏状态机,只认 URL——不复用
//! `crate::preview::PreviewPane`(文件预览专用,那边还有 `TabKind::File`/
//! `Acceptance`、`open_path`/`is_editable_extension`/`flyfish_url` 等浏览器
//! 用不到的逻辑),两者各自维护、互不知情。

use crate::preview::WebviewSpec;
use crate::workspace::{lh, preview_tab_display_width, tab_arrow_button, tab_divider, tab_window};
use crate::{chrome_style, icon_size, icons, theme, workspace_font, workspace_geometry};
use dozer_client::Client;
use dozer_core::protocol::{BookmarkInfo, BookmarkScope};
use iced_widget::core::{Border, Element, Length};
use iced_widget::{button, column, container, row, text};

/// 一个浏览器 tab。
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserTab {
    pub id: usize,
    pub url: String,
    pub title: String,
}

#[derive(Default)]
pub struct Tabs {
    tabs: Vec<BrowserTab>,
    active: usize,
    next_id: usize,
    addr_editing: bool,
    addr_buffer: String,
}

impl Tabs {
    pub fn tabs(&self) -> &[BrowserTab] {
        &self.tabs
    }

    pub fn active_idx(&self) -> usize {
        self.active
    }

    /// 取 URL 的 host 部分当标题(去掉协议头,取第一个 `/` 之前的部分)。
    /// 每次调用都新开一个 tab,不做去重——原样对齐现有
    /// `PreviewPane::open_url` 的行为(`open_path` 才有去重,`open_url`
    /// 没有),纯重构不改变这条现状。
    pub fn open_url(&mut self, url: String) -> usize {
        let title = url
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .split('/')
            .next()
            .unwrap_or(&url)
            .to_string();
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(BrowserTab { id, url, title });
        self.active = self.tabs.len() - 1;
        id
    }

    pub fn select(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active = idx;
        }
    }

    pub fn close(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if idx < self.active {
            self.active -= 1;
        }
    }

    pub fn addr_editing(&self) -> bool {
        self.addr_editing
    }

    pub fn addr_buffer(&self) -> &str {
        &self.addr_buffer
    }

    /// 进入地址栏编辑:预填当前激活 tab 的 URL(浏览器 tab 恒为网页,不像
    /// `PreviewPane::addr_begin` 还要 match `TabKind`)。
    pub fn addr_begin(&mut self) {
        self.addr_editing = true;
        self.addr_buffer = self
            .tabs
            .get(self.active)
            .map(|t| t.url.clone())
            .unwrap_or_default();
    }

    pub fn addr_text(&mut self, s: &str) {
        self.addr_buffer.push_str(s);
    }

    pub fn addr_backspace(&mut self) {
        self.addr_buffer.pop();
    }

    pub fn addr_cancel(&mut self) {
        self.addr_editing = false;
        self.addr_buffer.clear();
    }

    /// 提交解析:`Ok(Some(url))` = 有效网址(无 scheme 自动补 `http://`);
    /// `Ok(None)` = 空输入,no-op;`Err(message)` = 本地路径(以 `/` 或
    /// `~/` 开头),浏览器不支持,`message` 是"浏览器不支持打开本地文件"
    /// 这条文案。解析规则原样照抄现有 `PreviewPane::addr_submit`/
    /// `AddrTarget` 那段逻辑,只是把"返回 `AddrTarget::File` 交给调用方
    /// 判断"改成直接在这里判定并通过 `Result` 表达。
    pub fn addr_submit(&mut self) -> Result<Option<String>, String> {
        self.addr_editing = false;
        let input = std::mem::take(&mut self.addr_buffer);
        let input = input.trim();
        if input.is_empty() {
            return Ok(None);
        }
        if input.starts_with('/') || input.starts_with("~/") {
            return Err("浏览器不支持打开本地文件".to_string());
        }
        if input.contains("://") {
            return Ok(Some(input.to_string()));
        }
        Ok(Some(format!("http://{input}")))
    }

    /// 当前激活 tab 的 webview id(浏览器 tab 恒为 webview,不像
    /// `PreviewPane::active_webview_id` 还要排除 `Acceptance`)。
    pub fn active_webview_id(&self) -> Option<usize> {
        self.tabs.get(self.active).map(|t| t.id)
    }

    /// webview 期望清单:每个 tab 一个,仅激活者可见。浏览器没有
    /// `PreviewPane` 那种"验收 tab 激活时其余全隐藏"的 overlay 概念,
    /// 也没有 flyfish 文件渲染 URL,直接拿 `url` 字段。
    pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(idx, tab)| WebviewSpec {
                id: tab.id,
                url: tab.url.clone(),
                visible: idx == self.active,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_url_never_dedups_and_activates_new_tab() {
        let mut t = Tabs::default();
        let id0 = t.open_url("http://localhost:3000".into());
        let id1 = t.open_url("http://localhost:3000".into());
        assert_ne!(id0, id1, "每次都新开 tab,不去重");
        assert_eq!(t.tabs().len(), 2);
        assert_eq!(t.active_idx(), 1);
        assert_eq!(t.tabs()[0].title, "localhost:3000");
    }

    #[test]
    fn select_and_close_tabs() {
        let mut t = Tabs::default();
        t.open_url("http://a.com".into());
        t.open_url("http://b.com".into());
        t.select(0);
        assert_eq!(t.active_idx(), 0);
        t.close(0);
        assert_eq!(t.tabs().len(), 1);
        assert_eq!(t.active_idx(), 0);
        assert_eq!(t.tabs()[0].url, "http://b.com");
    }

    #[test]
    fn addr_edit_and_submit_parses_url_and_rejects_local_paths() {
        let mut t = Tabs::default();
        t.addr_begin();
        assert!(t.addr_editing());
        t.addr_text("localhost:3000/x");
        assert_eq!(t.addr_submit(), Ok(Some("http://localhost:3000/x".into())));
        assert!(!t.addr_editing());

        t.addr_begin();
        t.addr_text("https://example.com");
        assert_eq!(t.addr_submit(), Ok(Some("https://example.com".into())));

        t.addr_begin();
        t.addr_text("/tmp/x");
        assert_eq!(t.addr_submit(), Err("浏览器不支持打开本地文件".to_string()));

        t.addr_begin();
        t.addr_text("~/x");
        assert_eq!(t.addr_submit(), Err("浏览器不支持打开本地文件".to_string()));

        t.addr_begin();
        t.addr_text("abc");
        t.addr_backspace();
        t.addr_backspace();
        t.addr_backspace();
        assert_eq!(t.addr_submit(), Ok(None), "空输入不产生动作");

        t.addr_begin();
        t.addr_text("x");
        t.addr_cancel();
        assert!(!t.addr_editing());
    }

    #[test]
    fn addr_begin_prefills_current_tab_url() {
        let mut t = Tabs::default();
        t.open_url("http://a.com".into());
        t.addr_begin();
        assert_eq!(t.addr_buffer(), "http://a.com");
    }

    #[test]
    fn active_webview_id_tracks_active_tab() {
        let mut t = Tabs::default();
        assert_eq!(t.active_webview_id(), None, "无 tab 时没有 webview");
        let id0 = t.open_url("http://a.com".into());
        assert_eq!(t.active_webview_id(), Some(id0));
        let id1 = t.open_url("http://b.com".into());
        assert_eq!(t.active_webview_id(), Some(id1));
        t.select(0);
        assert_eq!(t.active_webview_id(), Some(id0));
    }

    #[test]
    fn desired_webviews_builds_urls_and_visibility() {
        let mut t = Tabs::default();
        t.open_url("http://a.com".into());
        t.open_url("http://b.com".into());
        let specs = t.desired_webviews();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].url, "http://a.com");
        assert!(!specs[0].visible, "非激活 tab 不可见");
        assert_eq!(specs[1].url, "http://b.com");
        assert!(specs[1].visible);
    }

    fn client_for_test() -> Client {
        Client::new(std::path::PathBuf::from(
            "/tmp/dozer-browser-test-nonexistent.sock",
        ))
    }

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

    #[tokio::test]
    async fn update_open_url_clears_error_and_resets_tab_first() {
        let mut state = State {
            error: Some("旧错误".to_string()),
            tab_first: 3,
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::OpenUrl("http://a.com".into()),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert!(state.error.is_none());
        assert_eq!(state.tab_first, 0);
        assert_eq!(state.tabs.tabs().len(), 1);
    }

    #[tokio::test]
    async fn update_addr_event_submit_ok_recurses_into_open_url() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::AddrClick,
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        update(
            &mut state,
            Message::AddrEvent(crate::workspace::AddrEvent::Text("http://a.com".into())),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        update(
            &mut state,
            Message::AddrEvent(crate::workspace::AddrEvent::Submit),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(
            state.tabs.tabs().len(),
            1,
            "提交应递归触发 OpenUrl 开一个 tab"
        );
        assert_eq!(state.tabs.tabs()[0].url, "http://a.com");
    }

    #[tokio::test]
    async fn update_addr_event_submit_local_path_sets_error_without_opening_tab() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::AddrClick,
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        update(
            &mut state,
            Message::AddrEvent(crate::workspace::AddrEvent::Text("/tmp/x".into())),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        update(
            &mut state,
            Message::AddrEvent(crate::workspace::AddrEvent::Submit),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.error.as_deref(), Some("浏览器不支持打开本地文件"));
        assert!(state.tabs.tabs().is_empty());
    }

    #[tokio::test]
    async fn update_tab_scroll_saturates_at_zero() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::TabScroll(false),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.tab_first, 0, "不该下溢");
        update(
            &mut state,
            Message::TabScroll(true),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.tab_first, 2);
    }

    #[tokio::test]
    async fn update_star_click_toggles_menu_and_clears_error() {
        let mut state = State {
            error: Some("旧错误".to_string()),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::StarClick,
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert!(state.star_menu_open);
        assert!(state.error.is_none());
        update(
            &mut state,
            Message::StarClick,
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert!(!state.star_menu_open);
    }

    #[tokio::test]
    async fn update_bookmarks_toggle_flips_panel() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::BookmarksToggle,
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert!(state.bookmarks_open);
    }

    #[tokio::test]
    async fn update_bookmark_add_without_active_tab_is_noop() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::BookmarkAdd(BookmarkScope::Global),
            Some(1),
            &client,
            &handle,
            |_| panic!("无 tab 时不该 emit"),
        );
        assert!(state.bookmarks.is_empty());
    }

    #[tokio::test]
    async fn update_bookmark_add_global_optimistically_inserts() {
        let mut state = State::default();
        state.tabs.open_url("http://a.com".into());
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::BookmarkAdd(BookmarkScope::Global),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.bookmarks.len(), 1);
        assert_eq!(state.bookmarks[0].scope, BookmarkScope::Global);
        assert_eq!(state.bookmarks[0].project_id, None);
        assert!(!state.star_menu_open);
    }

    #[tokio::test]
    async fn update_bookmark_add_project_scope_without_project_id_is_local_only() {
        let mut state = State::default();
        state.tabs.open_url("http://a.com".into());
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::BookmarkAdd(BookmarkScope::Project),
            None,
            &client,
            &handle,
            |_| panic!("project_id 为 None 时不该 emit 网络请求"),
        );
        assert_eq!(state.bookmarks.len(), 1, "本地乐观更新仍然发生");
    }

    #[tokio::test]
    async fn update_bookmark_remove_filters_local_cache() {
        let mut state = State {
            bookmarks: vec![bm(1, BookmarkScope::Global, None, "https://a.com")],
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::BookmarkRemove(1),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert!(state.bookmarks.is_empty());
    }

    #[tokio::test]
    async fn update_bookmarks_loaded_replaces_local_cache() {
        let mut state = State {
            bookmarks: vec![bm(1, BookmarkScope::Global, None, "https://stale.com")],
            ..State::default()
        };
        let fresh = vec![bm(2, BookmarkScope::Global, None, "https://fresh.com")];
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::BookmarksLoaded(1, fresh.clone()),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.bookmarks, fresh);
    }

    #[tokio::test]
    async fn update_bookmarks_mutated_error_sets_state_error() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::BookmarksMutated(1, Err("boom".to_string())),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.error.as_deref(), Some("boom"));
    }

    #[test]
    fn state_accessors_delegate_to_tabs() {
        let mut state = State::default();
        assert!(!state.addr_editing());
        assert_eq!(state.active_webview_id(), None);
        assert!(state.desired_webviews().is_empty());
        state.tabs.open_url("http://a.com".into());
        state.tabs.addr_begin();
        assert!(state.addr_editing());
        state.addr_cancel();
        assert!(!state.addr_editing());
        assert_eq!(state.active_webview_id(), Some(0));
        assert_eq!(state.desired_webviews().len(), 1);
    }
}

/// 乐观本地插入的占位 id:落库前不知道真实自增 id,只在"点击→下一次
/// 全量刷新落地"这一帧内部当哨兵用,不参与任何持久化或跨帧比较。
const OPTIMISTIC_BOOKMARK_ID: i64 = -1;

/// 当前 URL 在全局/本项目两边各自的收藏状态(`Some(id)` = 已收藏,
/// `id` 是"移出收藏"要传的记录 id)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BookmarkStatus {
    global: Option<i64>,
    project: Option<i64>,
}

impl BookmarkStatus {
    fn is_bookmarked(&self) -> bool {
        self.global.is_some() || self.project.is_some()
    }
}

fn bookmark_status(
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
fn optimistic_add(
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
fn optimistic_remove(bookmarks: &mut Vec<BookmarkInfo>, id: i64) {
    bookmarks.retain(|b| b.id != id);
}

/// 浏览器面板自己的消息类型——内核(`workspace.rs`)只认一个包装变体
/// `Message::Browser(extensions::browser::Message)`,这个模块本身不
/// import 顶层 `Message`。`AddrEvent` 是地址栏/验收意见框/项目树行内
/// 编辑三处共用的通用文本输入事件类型,定义在 `crate::workspace`,这里
/// 直接引用,不复制。
#[derive(Debug, Clone)]
pub enum Message {
    OpenUrl(String),
    SelectTab(usize),
    CloseTab(usize),
    AddrClick,
    AddrEvent(crate::workspace::AddrEvent),
    TabScroll(bool),
    StarClick,
    BookmarkAdd(BookmarkScope),
    BookmarkRemove(i64),
    BookmarksToggle,
    BookmarksLoaded(i64, Vec<BookmarkInfo>),
    BookmarksMutated(i64, Result<(), String>),
}

/// 浏览器面板的全部状态。挂在每个 `Workspace` 上(不像 Git Log 挂在
/// `App` 上)——项目切换靠 `Workspace` 自身生命周期天然隔离,不需要
/// 手动同步/清空逻辑。
#[derive(Default)]
pub struct State {
    tabs: Tabs,
    error: Option<String>,
    tab_first: usize,
    bookmarks: Vec<BookmarkInfo>,
    bookmarks_open: bool,
    star_menu_open: bool,
}

impl State {
    /// 地址栏是否在编辑态(内核 `App::browser_addr_editing` 键盘路由用)。
    pub fn addr_editing(&self) -> bool {
        self.tabs.addr_editing()
    }

    /// 取消地址栏编辑(内核 `App::blur_inputs` 用)。
    pub fn addr_cancel(&mut self) {
        self.tabs.addr_cancel();
    }

    /// 当前激活 tab 的 webview id(内核 `App::active_browser_webview_id`
    /// 焦点路由用)。
    pub fn active_webview_id(&self) -> Option<usize> {
        self.tabs.active_webview_id()
    }

    /// webview 期望清单(内核 `App::browser_desired` 用,供 main.rs 同步
    /// webview 池)。
    pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
        self.tabs.desired_webviews()
    }
}

/// 处理浏览器面板的全部消息。`project_id` 由内核每次调用时从
/// `ws.project.as_ref().map(|p| p.id)` 现取传入(`State` 本身不存这个,
/// 避免状态冗余/漂移)。`client` 用于收藏夹的 dozerd RPC 往返——这是跟
/// Git Log 试点 `update` 签名的主要差异(Git Log 只有本地 `git2` 阻塞
/// 调用,不需要网络)。
pub fn update(
    state: &mut State,
    msg: Message,
    project_id: Option<i64>,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::OpenUrl(url) => {
            state.error = None;
            state.tabs.open_url(url);
            state.tab_first = 0;
        }
        Message::SelectTab(idx) => state.tabs.select(idx),
        Message::CloseTab(idx) => {
            state.tabs.close(idx);
            state.tab_first = 0;
        }
        Message::AddrClick => {
            state.error = None;
            state.tabs.addr_begin();
        }
        Message::AddrEvent(ev) => match ev {
            crate::workspace::AddrEvent::Text(s) => state.tabs.addr_text(&s),
            crate::workspace::AddrEvent::Backspace => state.tabs.addr_backspace(),
            crate::workspace::AddrEvent::Cancel => state.tabs.addr_cancel(),
            crate::workspace::AddrEvent::Submit => match state.tabs.addr_submit() {
                Ok(Some(url)) => update(
                    state,
                    Message::OpenUrl(url),
                    project_id,
                    client,
                    handle,
                    emit,
                ),
                Ok(None) => {}
                Err(message) => state.error = Some(message),
            },
        },
        Message::TabScroll(right) => {
            if right {
                state.tab_first = state.tab_first.saturating_add(2);
            } else {
                state.tab_first = state.tab_first.saturating_sub(2);
            }
        }
        Message::StarClick => {
            state.error = None;
            state.star_menu_open = !state.star_menu_open;
        }
        Message::BookmarksToggle => state.bookmarks_open = !state.bookmarks_open,
        Message::BookmarkAdd(scope) => {
            state.star_menu_open = false;
            let Some(tab) = state.tabs.tabs().get(state.tabs.active_idx()) else {
                return;
            };
            let url = tab.url.clone();
            let title = tab.title.clone();
            let target_project_id = match scope {
                BookmarkScope::Global => None,
                BookmarkScope::Project => project_id,
            };
            let created_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            optimistic_add(
                &mut state.bookmarks,
                scope,
                target_project_id,
                &url,
                &title,
                created_ms,
            );
            let Some(project_id) = project_id else { return };
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .add_bookmark(scope, target_project_id, &url, &title)
                    .await
                    .map_err(|e| e.to_string());
                emit(Message::BookmarksMutated(project_id, res));
            });
        }
        Message::BookmarkRemove(id) => {
            state.star_menu_open = false;
            optimistic_remove(&mut state.bookmarks, id);
            let Some(project_id) = project_id else { return };
            let client = client.clone();
            handle.spawn(async move {
                let res = client.remove_bookmark(id).await.map_err(|e| e.to_string());
                emit(Message::BookmarksMutated(project_id, res));
            });
        }
        Message::BookmarksLoaded(_, bookmarks) => state.bookmarks = bookmarks,
        Message::BookmarksMutated(_, res) => {
            if let Err(message) = res {
                state.error = Some(message);
            }
            request_bookmarks_refresh(project_id, client, handle, emit);
        }
    }
}

/// 现有 `Workspace::spawn_bookmarks_refresh` 的搬家版本:异步拉取
/// "全局 + 当前项目"收藏夹合集。
pub fn request_bookmarks_refresh(
    project_id: Option<i64>,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let Some(project_id) = project_id else { return };
    let client = client.clone();
    handle.spawn(async move {
        let bookmarks = client
            .list_bookmarks(Some(project_id))
            .await
            .unwrap_or_default();
        emit(Message::BookmarksLoaded(project_id, bookmarks));
    });
}

/// 地址栏星标:当前 URL 在全局/本项目任一边已收藏则 GOLD 实心,否则
/// DIM;当前无 tab 时禁用(浏览器 tab 恒为网页,不需要像旧版
/// `current_browser_url` 那样再判一次 `TabKind`)。`project_id` 必须传
/// 真实值,不能传 `None` 占位——否则"只在本项目收藏、没加全局收藏"的
/// 网址会被误判成未收藏(`bookmark_status` 的 `project` 字段只在传了
/// `Some(pid)` 时才会去匹配)。
fn star_button(
    state: &State,
    project_id: Option<i64>,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let url = state
        .tabs
        .tabs()
        .get(state.tabs.active_idx())
        .map(|t| t.url.clone());
    let starred = url
        .as_ref()
        .map(|u| bookmark_status(&state.bookmarks, u, project_id).is_bookmarked())
        .unwrap_or(false);
    let color = if starred { theme::color::GOLD } else { theme::color::DIM };
    let mut btn = button(icons::view(icons::IconKind::Star, icon_size::row(), color))
        .width(Length::Fixed(workspace_geometry::tab_button_size()))
        .height(Length::Fixed(workspace_geometry::tab_button_size()))
        .padding(0)
        .style(move |_t, _s| button::Style {
            background: None,
            text_color: color,
            ..button::Style::default()
        });
    if url.is_some() {
        btn = btn.on_press(Message::StarClick);
    }
    btn.into()
}

/// tab 栏"收藏夹"下拉面板触发按钮,颜色恒定(不像星标那样带收藏状态)。
fn bookmark_menu_row(
    label: String,
    msg: Message,
) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
    button(lh(text(label)
        .size(workspace_font::body())
        .color(theme::color::CREAM)))
    .on_press(msg)
    .width(Length::Fill)
    .padding([6, 12])
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: theme::color::CREAM,
        ..button::Style::default()
    })
    .into()
}

/// 星标小菜单:未收藏显示"加入…",已收藏显示"移出…"(打勾态)。
fn star_menu_popup(
    state: &State,
    project_id: Option<i64>,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(url) = state
        .tabs
        .tabs()
        .get(state.tabs.active_idx())
        .map(|t| t.url.clone())
    else {
        return column![].into();
    };
    let status = bookmark_status(&state.bookmarks, &url, project_id);

    let mut col = column![match status.global {
        Some(id) => bookmark_menu_row("移出全局收藏".to_string(), Message::BookmarkRemove(id)),
        None => bookmark_menu_row(
            "加入全局收藏".to_string(),
            Message::BookmarkAdd(BookmarkScope::Global)
        ),
    }]
    .spacing(2);

    if project_id.is_some() {
        col = col.push(match status.project {
            Some(id) => {
                bookmark_menu_row("移出本项目收藏".to_string(), Message::BookmarkRemove(id))
            }
            None => bookmark_menu_row(
                "加入本项目收藏".to_string(),
                Message::BookmarkAdd(BookmarkScope::Project),
            ),
        });
    }

    container(col)
        .padding(6)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::CARD.into()),
            border: Border {
                color: theme::color::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// 一组收藏条目:标题(点击新开 tab)+ `×` 删除按钮,风格照抄 tab 关闭
/// 按钮。
fn bookmark_group<'a>(
    title: &'static str,
    items: &[&'a BookmarkInfo],
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![lh(text(title)
        .size(workspace_font::subtitle())
        .color(theme::color::DIM))]
    .spacing(2);
    for b in items {
        let open = button(lh(text(b.title.clone())
            .size(workspace_font::body())
            .color(theme::color::CREAM)))
        .on_press(Message::OpenUrl(b.url.clone()))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: theme::color::CREAM,
            ..button::Style::default()
        });
        let remove = button(lh(text("×").size(workspace_font::body()).color(theme::color::DIM)))
            .on_press(Message::BookmarkRemove(b.id))
            .style(|_t: &iced_widget::Theme, _s| button::Style {
                background: None,
                text_color: theme::color::DIM,
                ..button::Style::default()
            });
        col = col.push(
            row![open, remove]
                .spacing(4)
                .align_y(iced_widget::core::Alignment::Center),
        );
    }
    col.into()
}

/// 收藏夹下拉面板:分"全局收藏"/"本项目收藏"两组,都为空时显示占位文案。
fn bookmarks_panel(
    state: &State,
    project_id: Option<i64>,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let global: Vec<&BookmarkInfo> = state
        .bookmarks
        .iter()
        .filter(|b| b.scope == BookmarkScope::Global)
        .collect();
    let project: Vec<&BookmarkInfo> = state
        .bookmarks
        .iter()
        .filter(|b| b.scope == BookmarkScope::Project && b.project_id == project_id)
        .collect();

    let both_empty = global.is_empty() && project.is_empty();
    let mut col = column![].spacing(6);
    col = col.push(bookmark_group("全局收藏", &global));
    if project_id.is_some() {
        col = col.push(bookmark_group("本项目收藏", &project));
    }
    if both_empty {
        col = col.push(lh(text("暂无收藏")
            .size(workspace_font::subtitle())
            .color(theme::color::DIM)));
    }

    container(col)
        .padding(6)
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::CARD.into()),
            border: Border {
                color: theme::color::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// 浏览器面板:表头 + (可选 tab 栏)+ 地址栏(与星标/收藏夹按钮)+ (可选
/// 星标菜单/收藏夹面板)+ (可选错误文案)+ 激活 tab 的网页预览。
/// `project_id` 用于星标收藏态判定(全局/本项目两档)。
pub fn view(
    state: &State,
    project_id: Option<i64>,
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = chrome_style::browser_pane();
    let widths: Vec<f32> = state
        .tabs
        .tabs()
        .iter()
        .map(|t| preview_tab_display_width(&t.title))
        .collect();
    let (first, can_left, can_right) = tab_window(
        &widths,
        4.0,
        workspace_geometry::tab_bar_avail_px(),
        state.tab_first,
    );

    let items: Vec<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> = state
        .tabs
        .tabs()
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx >= first)
        .map(|(idx, tab)| {
            let active = idx == state.tabs.active_idx();
            let select = button(lh(text(tab.title.clone())
                .size(workspace_font::subtitle())
                .color(theme::color::CREAM)))
            .on_press(Message::SelectTab(idx))
            .style(|_t, _s| button::Style {
                background: None,
                text_color: theme::color::CREAM,
                ..button::Style::default()
            });
            let close = button(lh(text("×").size(workspace_font::body()).color(theme::color::DIM)))
                .on_press(Message::CloseTab(idx))
                .style(|_t, _s| button::Style {
                    background: None,
                    text_color: theme::color::DIM,
                    ..button::Style::default()
                });
            container(
                row![select, close]
                    .spacing(2)
                    .align_y(iced_widget::core::Alignment::Center),
            )
            .padding([2, 4])
            .style(move |_t: &iced_widget::Theme| {
                if active {
                    container::Style {
                        background: Some(theme::color::CARD.into()),
                        border: Border {
                            color: theme::color::BORDER,
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                        ..container::Style::default()
                    }
                } else {
                    container::Style::default()
                }
            })
            .into()
        })
        .collect();
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    let left_arrow = tab_arrow_button(
        icons::IconKind::ChevronLeft,
        can_left,
        Message::TabScroll(false),
    );
    let right_arrow = tab_arrow_button(
        icons::IconKind::ChevronRight,
        can_right,
        Message::TabScroll(true),
    );
    let tab_bar = row![left_arrow, right_arrow, clipped]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);

    let editing = state.addr_editing();
    let addr_text = if editing {
        format!("{}▏", state.tabs.addr_buffer())
    } else {
        "输入网址".to_string()
    };
    let addr = button(lh(text(addr_text)
        .size(workspace_font::body())
        .color(if editing { theme::color::CREAM } else { theme::color::DIM })))
    .on_press(Message::AddrClick)
    .width(Length::Fill)
    .style(move |_t, _s| button::Style {
        background: Some(theme::color::TERM_BG.into()),
        text_color: theme::color::CREAM,
        border: Border {
            color: if editing { theme::color::GOLD } else { theme::color::BORDER },
            width: 1.0,
            radius: 2.0.into(),
        },
        ..button::Style::default()
    });

    let addr_row = row![
        addr,
        star_button(state, project_id),
        bookmarks_toggle_button()
    ]
    .spacing(4)
    .align_y(iced_widget::core::Alignment::Center);

    let mut content = column![tab_bar, tab_divider(), addr_row].spacing(region.gap);

    if state.star_menu_open {
        content = content.push(star_menu_popup(state, project_id));
    }
    if state.bookmarks_open {
        content = content.push(bookmarks_panel(state, project_id));
    }

    if let Some(err) = &state.error {
        content = content.push(lh(text(format!("⚠ {err}"))
            .size(workspace_font::body())
            .color(theme::color::RED)));
    }

    if state.tabs.tabs().is_empty() {
        content = content.push(
            container(lh(text("暂无网页——在地址栏输入网址")
                .size(workspace_font::subtitle())
                .color(theme::color::DIM)))
            .width(Length::Fill)
            .height(Length::Fill),
        );
    }

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

/// tab 栏"收藏夹"下拉面板触发按钮。返回带收藏夹切换消息的按钮。
fn bookmarks_toggle_button<'a>() -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>
{
    button(icons::view(
        icons::IconKind::Bookmark,
        icon_size::row(),
        theme::color::DIM,
    ))
    .on_press(Message::BookmarksToggle)
    .width(Length::Fixed(workspace_geometry::tab_button_size()))
    .height(Length::Fixed(workspace_geometry::tab_button_size()))
    .padding(0)
    .style(|_t, _s| button::Style {
        background: None,
        text_color: theme::color::DIM,
        ..button::Style::default()
    })
    .into()
}
