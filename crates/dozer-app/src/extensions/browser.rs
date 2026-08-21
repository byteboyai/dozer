//! 浏览器面板扩展(阶段 1 扩展化重构第二个试点)。自己的 `Message`/
//! `State`/`update`/`view`,内核只认一个包装变体 `Message::Browser(..)`
//! 做转发,不知道自己被包在哪个外层类型里。收藏夹(全局/本项目两级书签)
//! 是浏览器域专属功能,状态/消息/UI 整体并入这个模块。
//!
//! `Tabs` 是浏览器专用的精简 tab/地址栏状态机,只认 URL——不复用
//! `crate::preview::PreviewPane`(文件预览专用,那边还有 `TabKind::File`/
//! `Acceptance`、`open_path`/`is_editable_extension`/`flyfish_url` 等浏览器
//! 用不到的逻辑),两者各自维护、互不知情。

use crate::app::{panel_tab, tab_divider};
use crate::preview::WebviewSpec;
use crate::theme;
use crate::workspace::{lh, split_portions};
use byteui::interaction::icons;
use dozer_client::Client;
use dozer_core::protocol::{BookmarkInfo, BookmarkScope};
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Border, Element, Length, Rectangle};
use iced_widget::{MouseArea, button, column, container, row, text};
use std::collections::HashMap;

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
    /// 地址栏是否持有 iced 内部真实焦点。**不是**应用层手动置位的镜像——
    /// 每帧渲染循环里 `CaptureAddrFocus` 问一遍 iced 真相后立刻写进这里
    /// (`set_addr_focused`),`main.rs` 键盘路由读它决定要不要把事件放行
    /// 给标准 iced 管线。
    addr_focused: bool,
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

    /// 按 webview/tab id 更新 tab 标题——页面加载完成后覆写 `open_url`
    /// 初建的 URL 标题。空标题(`about:blank` 等未设 title 的页面)保持
    /// 原 URL 标题不变,未知 id 是 no-op。
    pub fn apply_title(&mut self, id: usize, title: String) {
        if title.trim().is_empty() {
            return;
        }
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            tab.title = title;
        }
    }

    pub fn close(&mut self, idx: usize) {
        if self.tabs.is_empty() || idx >= self.tabs.len() {
            return;
        }
        let closing_last = self.tabs.len() == 1;
        self.tabs.remove(idx);
        // 页签组永不为空:关掉最后一个 tab 时自动补一个 `about:blank`,
        // 与初始状态一致(见 `State::default`)。
        if closing_last {
            self.open_url("about:blank".to_string());
            return;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if idx < self.active {
            self.active -= 1;
        }
    }

    /// 拖拽换位:把 `from` 处的 tab 移到 `to`,并同步 `active`。越界/同址即
    /// no-op。
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        if self.active == from {
            self.active = to;
        } else if from < self.active && to >= self.active {
            self.active -= 1;
        } else if from > self.active && to <= self.active {
            self.active += 1;
        }
    }

    /// 地址栏是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn addr_focused(&self) -> bool {
        self.addr_focused
    }

    pub fn addr_buffer(&self) -> &str {
        &self.addr_buffer
    }

    /// `iced_widget::text_input::on_input` 每次给全量当前字符串。
    pub fn set_addr_buffer(&mut self, s: String) {
        self.addr_buffer = s;
    }

    /// 每帧渲染循环读走 `CaptureAddrFocus` 查到的真实焦点态后写进来。焦点
    /// 从假变真(刚获得焦点)时预填当前激活 tab 的网址——浏览器 tab 恒为
    /// 网页,不像 `PreviewPane::addr_begin` 还要 match `TabKind`。空标签页
    /// (`about:blank`)没有可编辑的网址,预填会把 `about:blank` 带进输入框、
    /// 再被后续键入拼成 `about:blankhttp://x.com` 这类垃圾——遇到空标签就
    /// 当空输入处理,让用户直接打新地址。焦点从真变假(刚失去焦点)时清空
    /// 草稿——不聚焦的地址栏由 `view()` 直接回显当前激活 tab 的完整网址
    /// (不再只显示占位符"输入网址")。
    pub fn set_addr_focused(&mut self, focused: bool) {
        if !self.addr_focused && focused {
            self.addr_buffer = self
                .tabs
                .get(self.active)
                .map(|t| {
                    if t.url.is_empty() || t.url == "about:blank" {
                        String::new()
                    } else {
                        t.url.clone()
                    }
                })
                .unwrap_or_default();
        } else if self.addr_focused && !focused {
            self.addr_buffer.clear();
        }
        self.addr_focused = focused;
    }

    /// 提交解析:`Ok(Some(url))` = 有效网址(裸域名自动补 `https://`);
    /// `Ok(None)` = 空输入,no-op;`Err(message)` = 本地路径(以 `/` 或
    /// `~/` 开头),浏览器不支持,`message` 是"浏览器不支持打开本地文件"
    /// 这条文案。带 `://` 的完整 URL(如 `https://x.com`)和单冒号 scheme
    /// (如 `about:blank`/`data:`/`mailto:`)都原样保留,只有既无 `://` 也
    /// 无 scheme 的裸输入(域名/IP/`host:port`)才补 `https://`。**不**自动
    /// 失焦(同 Files 搜索框 Stage 2 的决定,Enter 提交后光标仍留在输入框
    /// 里,是记录在案的小行为变化,不是遗漏)。
    pub fn addr_submit(&mut self) -> Result<Option<String>, String> {
        let input = std::mem::take(&mut self.addr_buffer);
        let input = input.trim();
        if input.is_empty() {
            return Ok(None);
        }
        if input.starts_with('/') || input.starts_with("~/") {
            return Err("浏览器不支持打开本地文件".to_string());
        }
        if input.contains("://") || has_explicit_scheme(input) {
            return Ok(Some(input.to_string()));
        }
        Ok(Some(format!("https://{input}")))
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

/// 地址栏稳定的 iced widget id:`view()` 里 `.id()` 挂给真正的
/// `text_input`,`CaptureAddrFocus` 每帧靠它在 widget 树里认出这一个(同
/// `extensions::files::search_field_id` 的既有手法)。
pub fn addr_field_id() -> Id {
    Id::new("browser-addr-box")
}

static ADDR_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走(非消费)地址栏上一帧是否持有 iced 内部真实焦点。`main.rs` 渲染
/// 循环每帧跑完 `CaptureAddrFocus` 后立刻调用本函数,把结果塞进当前
/// `Workspace`(`State::set_addr_focused`)——`static` 只是临时桥接(同
/// `extensions::files::take_search_focused` 的既有手法)。
pub fn take_addr_focused() -> bool {
    *ADDR_FOCUSED.lock().unwrap()
}

/// 每帧 `interface.operate()` 跑一遍,把 `addr_field_id()` 命中的
/// `text_input` 当前是否持有 iced 焦点写进 `ADDR_FOCUSED`。`traverse`
/// **必须**调用传入的 `operate` 闭包才会继续递归子节点——地址栏嵌在
/// `row!`/`container!` 里,空 `traverse` 会导致 `Row`/`Column` 的
/// `operate()` 直接跳过子节点,`focusable()` 永远不会被触达(见
/// `extensions::files::CaptureSearchFocus` 修复过的同款 Critical bug,
/// commit `b8281cd`,这次从一开始就不能再犯)。
pub struct CaptureAddrFocus;
impl Operation<()> for CaptureAddrFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&addr_field_id()) {
            *ADDR_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

/// 判定 `input` 是否已带 URI scheme(单冒号形式,如 `about:blank`/`data:`/
/// `mailto:`)。scheme 名必须是 `[a-zA-Z]` 开头、由 `[a-zA-Z0-9+.-]` 组成;
/// `localhost:3000`/`baidu.com:8080` 这类 `host:port` 不判为 scheme(冒号
/// 后面紧跟的是纯数字端口)。带 `://` 的完整 URL 由调用方 `contains("://")`
/// 单独判定,不经过这里。
fn has_explicit_scheme(input: &str) -> bool {
    let Some(colon) = input.find(':') else {
        return false;
    };
    let scheme = &input[..colon];
    if scheme.is_empty()
        || !scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
    {
        return false;
    }
    if !scheme
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '.' || c == '-')
    {
        return false;
    }
    // 冒号后内容非空、且第一个字符不是数字才认作 scheme:`about:blank` ->
    // "blank";`localhost:3000` -> "3000" 是端口(可能再跟 `/path`),
    // 回落成"裸输入补 https://"。
    let rest = &input[colon + 1..];
    !rest.is_empty() && !rest.chars().next().is_some_and(|c| c.is_ascii_digit())
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
    fn addr_focus_and_submit_parses_url_and_rejects_local_paths() {
        let mut t = Tabs::default();
        t.set_addr_focused(true);
        assert!(t.addr_focused());
        t.set_addr_buffer("localhost:3000/x".to_string());
        assert_eq!(t.addr_submit(), Ok(Some("https://localhost:3000/x".into())));

        t.set_addr_focused(true);
        t.set_addr_buffer("baidu.com".to_string());
        assert_eq!(t.addr_submit(), Ok(Some("https://baidu.com".into())));

        t.set_addr_focused(true);
        t.set_addr_buffer("https://example.com".to_string());
        assert_eq!(t.addr_submit(), Ok(Some("https://example.com".into())));

        t.set_addr_focused(true);
        t.set_addr_buffer("/tmp/x".to_string());
        assert_eq!(t.addr_submit(), Err("浏览器不支持打开本地文件".to_string()));

        t.set_addr_focused(true);
        t.set_addr_buffer("~/x".to_string());
        assert_eq!(t.addr_submit(), Err("浏览器不支持打开本地文件".to_string()));

        t.set_addr_focused(true);
        t.set_addr_buffer("".to_string());
        assert_eq!(t.addr_submit(), Ok(None), "空输入不产生动作");

        // 失焦清空草稿,不影响下一次聚焦时重新预填。
        t.set_addr_focused(true);
        t.set_addr_buffer("x".to_string());
        t.set_addr_focused(false);
        assert_eq!(t.addr_buffer(), "");
        assert!(!t.addr_focused());
    }

    #[test]
    fn addr_submit_keeps_explicit_schemes_and_prepends_https() {
        let mut t = Tabs::default();

        // 单冒号特殊 scheme 原样保留,不补 https://。
        for scheme_url in ["about:blank", "data:text/html,hi", "mailto:a@b.com"] {
            t.set_addr_focused(true);
            t.set_addr_buffer(scheme_url.to_string());
            assert_eq!(t.addr_submit(), Ok(Some(scheme_url.into())));
        }

        // host:port 不是 scheme,应补 https://。
        t.set_addr_focused(true);
        t.set_addr_buffer("example.com:8080".to_string());
        assert_eq!(t.addr_submit(), Ok(Some("https://example.com:8080".into())));
    }

    #[test]
    fn set_addr_focused_true_prefills_current_tab_url() {
        let mut t = Tabs::default();
        t.open_url("http://a.com".into());
        t.set_addr_focused(true);
        assert_eq!(t.addr_buffer(), "http://a.com");
    }

    #[test]
    fn set_addr_focused_true_on_blank_tab_prefills_empty() {
        let mut t = Tabs::default();
        // 默认带一个 about:blank 标签页。
        t.set_addr_focused(true);
        assert_eq!(t.addr_buffer(), "", "about:blank 不预填,避免拼出垃圾");
    }

    #[test]
    fn addr_field_id_is_stable_across_calls() {
        assert_eq!(addr_field_id(), addr_field_id());
    }

    #[test]
    fn apply_title_replaces_url_title_with_html_title_once_loaded() {
        let mut t = Tabs::default();
        // `open_url` 初建的标题是 URL 的 host。
        let id = t.open_url("https://example.com/foo".into());
        assert_eq!(t.tabs()[0].title, "example.com");
        // 页面加载完成后,HTML `<title>` 覆盖 URL 标题。
        t.apply_title(id, "Example · Official Site".into());
        assert_eq!(t.tabs()[0].title, "Example · Official Site");
        // 空标题(`about:blank` 等未设 `<title>` 的页面)保持 URL 标题。
        t.apply_title(id, "   ".into());
        assert_eq!(t.tabs()[0].title, "Example · Official Site");
        // 未知 id 是 no-op。
        t.apply_title(999, "Ghost".into());
        assert_eq!(t.tabs()[0].title, "Example · Official Site");
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
    async fn update_open_url_clears_error() {
        let mut state = State {
            error: Some("旧错误".to_string()),
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
        assert_eq!(state.tabs.tabs().len(), 2);
    }

    #[tokio::test]
    async fn update_title_loaded_applies_html_title() {
        let mut state = State {
            tabs: Tabs::default(),
            error: None,
            bookmarks: Vec::new(),
            bookmarks_open: false,
            star_menu_open: false,
            hover: Default::default(),
            tooltip_starts: Default::default(),
            addr_select_all_pending: false,
        };
        let id = state.tabs.open_url("https://example.com".into());
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::TitleLoaded(id, "Example Home".into()),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.tabs.tabs()[0].title, "Example Home");
        // 空标题不覆盖 URL 标题。
        update(
            &mut state,
            Message::TitleLoaded(id, "".into()),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.tabs.tabs()[0].title, "Example Home");
    }

    #[tokio::test]
    async fn update_addr_submit_ok_recurses_into_open_url() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::AddrInput("http://a.com".to_string()),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        update(
            &mut state,
            Message::AddrSubmit,
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(
            state.tabs.tabs().len(),
            2,
            "默认带一个 about:blank(下标 0),提交应再开一个真实 tab(下标 1)"
        );
        assert_eq!(state.tabs.tabs()[1].url, "http://a.com");
    }

    #[tokio::test]
    async fn update_addr_submit_local_path_sets_error_without_opening_tab() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::AddrInput("/tmp/x".to_string()),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        update(
            &mut state,
            Message::AddrSubmit,
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.error.as_deref(), Some("浏览器不支持打开本地文件"));
        assert_eq!(
            state.tabs.tabs().len(),
            1,
            "仍是默认的 about:blank,未新增 tab"
        );
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
        // `State::default()` 现在自带一个 about:blank 标签页,不再是空标签;
        // 要测"无激活 tab 时收藏是 no-op"的守卫分支,显式构造一个空标签状态。
        let mut state = State {
            tabs: Tabs::default(),
            error: None,
            bookmarks: Vec::new(),
            bookmarks_open: false,
            star_menu_open: false,
            hover: HashMap::new(),
            tooltip_starts: HashMap::new(),
            addr_select_all_pending: false,
        };
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
        assert!(!state.addr_focused());
        // 默认带一个 about:blank 标签页,它就是激活 tab
        assert_eq!(state.active_webview_id(), Some(0));
        assert_eq!(state.desired_webviews().len(), 1);
        state.tabs.open_url("http://a.com".into());
        state.set_addr_focused(true);
        assert!(state.addr_focused());
        state.set_addr_focused(false);
        assert!(!state.addr_focused());
        // 新开的 a.com 成为激活 tab,空标签仍在列表里
        assert_eq!(state.active_webview_id(), Some(1));
        assert_eq!(state.desired_webviews().len(), 2);
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

/// 浏览器导航动作(后退/前进/刷新)。实际导航由 main.rs 的 `dispatch`
/// 拦截 `Message::Nav` 后对激活 webview 的 `wry::WebView` 句柄执行(句柄
/// 在 main.rs 的 `browser_webviews` 池里,浏览器 `State` 摸不到);这里
/// 只是纯语义枚举,不带任何状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavAction {
    Back,
    Forward,
    Refresh,
}

/// 浏览器面板自己的消息类型——内核(`workspace.rs`)只认一个包装变体
/// `Message::Browser(extensions::browser::Message)`,这个模块本身不
/// import 顶层 `Message`。`AddrEvent` 是验收意见框/项目树行内编辑两处
/// 共用的通用文本输入事件类型,定义在 `crate::workspace`,这里直接引用,
/// 不复制。
#[derive(Debug, Clone)]
pub enum Message {
    OpenUrl(String),
    /// 页面加载完成后,渲染进程经 IPC 报回 HTML `document.title`(`id` 是
    /// webview/tab id,`title` 是页面标题)——覆盖 `open_url` 初建的 URL 标题。
    /// `about:blank` 等空标题页面不覆盖(见 `Tabs::apply_title` 的空标题守卫)。
    TitleLoaded(usize, String),
    SelectTab(usize),
    CloseTab(usize),
    /// 后退/前进/刷新按钮。`browser::update` 里是 no-op,真正的 webview
    /// 导航由 main.rs `dispatch` 拦截执行(见 `NavAction`)。
    Nav(NavAction),
    /// 拖拽换位:光标扫过页签 `idx` 时由 tab 的 `MouseArea::on_move` 发出,
    /// `App::update` 翻译成 `TabDragMove`(浏览器组在这里完成换位)。
    DragHover(usize),
    /// 地址栏草稿变化(iced `text_input::on_input`,每次按键给全量当前
    /// 字符串)。
    AddrInput(String),
    AddrSubmit,
    StarClick,
    BookmarkAdd(BookmarkScope),
    BookmarkRemove(i64),
    BookmarksToggle,
    BookmarksLoaded(i64, Vec<BookmarkInfo>),
    BookmarksMutated(i64, Result<(), String>),
    /// tab 标题/关闭按钮的 hover 进入/离开(idx = tab 序号, bool = 是否关闭
    /// 按钮, 最后 bool = 进入/离开)。浏览器面板有独立 `State`,无法复用顶栏
    /// 全局 `App::hover_progress`,自己维护一套进度机(见 `State::hover`)。
    Hover(usize, bool, bool),
    /// 收藏夹侧栏分割线开始拖:扩展发不了 app 级拖拽消息,由内核代发,见
    /// `App::update` 里 `Message::Browser(Message::ColumnDragStart)` 分支
    /// (同 `extensions::git_log::Message::ColumnDragStart` 的处理方式)。
    ColumnDragStart,
}

/// 浏览器面板的全部状态。挂在每个 `Workspace` 上(不像 Git Log 挂在
/// `App` 上)——项目切换靠 `Workspace` 自身生命周期天然隔离,不需要
/// 手动同步/清空逻辑。
/// 单个 tab 标题/关闭按钮的 hover 动画状态机,与顶栏 `HoverAnim` 同款
/// (每拍残余 50%,约 80ms 收敛,snap 0.01):浏览器面板独立 `State`,自带
/// 进度机,借全局 HOVER_ANIM 定时 redraw 推进(见 `State::advance_hover_anims`)。
#[derive(Debug, Clone, Copy, Default)]
struct TabHover {
    progress: f32,
    target: f32,
}

impl TabHover {
    fn set(&mut self, hovered: bool) {
        self.target = if hovered { 1.0 } else { 0.0 };
    }
    fn advance(&mut self) {
        let next = self.progress + (self.target - self.progress) * 0.5;
        self.progress = if (next - self.target).abs() < 0.01 {
            self.target
        } else {
            next
        };
    }
    fn active(&self) -> bool {
        (self.progress - self.target).abs() > 0.001
    }
}

pub struct State {
    tabs: Tabs,
    error: Option<String>,
    bookmarks: Vec<BookmarkInfo>,
    bookmarks_open: bool,
    star_menu_open: bool,
    /// tab 标题/关闭按钮的 hover 进度,键 `(tab 序号, 是否关闭按钮)`。
    hover: HashMap<(usize, bool), TabHover>,
    /// 页签标题 tooltip 的悬停计时起点:键为 tab 序号(关闭按钮不计,只认
    /// 页签整体悬停)。进入页签记 `Instant::now()`,离开即清除;悬停满 2s
    /// 后视图层据此弹标题全称 tooltip(见 `hover_tooltip_ready`)。与顶栏/
    /// 终端页签的计时分开存(`App::hover_tooltip_starts`),因为浏览器面板
    /// 走自己这套 hover 状态机(`hover` 而非全局 `HoverId`)。
    tooltip_starts: HashMap<usize, std::time::Instant>,
    /// 地址栏刚获得焦点时置位,请求内核在下一帧对地址栏做一次"全选"(
    /// `main.rs` 的 draw 循环用 `interface.operate` 跑 `text_input::
    /// select_all`)。一次性位,消费即复位(见 `take_addr_select_all_pending`)。
    addr_select_all_pending: bool,
}

impl Default for State {
    /// 浏览器面板默认状态:开一个 `about:blank` 标签页,而不是空标签栏——
    /// 面板首次呈现就有一块可看的内容,地址栏也提示"输入网址"开始浏览。
    fn default() -> Self {
        Self::with_initial_url("about:blank")
    }
}

impl State {
    /// 构造一个带初始 tab 的浏览器状态——用于首页全局浏览器默认打开某个
    /// 站点(见 `app::App::home_browser` 初始化)。默认(`State::default`)
    /// 本就带一个 `about:blank` 标签页,这里把它换成给定的真实 URL,保持
    /// "单一初始标签页"语义,不带多余空标签。
    pub fn with_initial_url(url: &str) -> Self {
        let mut s = State {
            tabs: Tabs::default(),
            error: None,
            bookmarks: Vec::new(),
            bookmarks_open: false,
            star_menu_open: false,
            hover: HashMap::new(),
            tooltip_starts: HashMap::new(),
            addr_select_all_pending: false,
        };
        s.tabs.open_url(url.to_string());
        s
    }

    /// 地址栏是否持有 iced 真实焦点(内核 `App::browser_addr_focused` 键盘
    /// 路由用)。
    pub fn addr_focused(&self) -> bool {
        self.tabs.addr_focused()
    }

    /// 收藏夹侧栏当前是否展开——`app.rs::App::shell_state()` 读这个填
    /// `ShellState::browser_bookmarks_open`,几何计算据此决定网页 webview
    /// 是否要让出侧栏宽度。
    pub fn bookmarks_open(&self) -> bool {
        self.bookmarks_open
    }

    /// 每帧渲染循环调用:把 `CaptureAddrFocus` 问到的真实焦点态写进来;
    /// 焦点从假变真时顺带清掉上一次提交失败留下的错误提示(同旧版
    /// `AddrClick` 里的 `state.error = None`)。
    pub fn set_addr_focused(&mut self, focused: bool) {
        if !self.tabs.addr_focused() && focused {
            self.error = None;
            // 焦点刚获得:请求内核下一帧对地址栏做一次全选(见
            // `take_addr_select_all_pending`),让用户单击即选中整条网址。
            self.addr_select_all_pending = true;
        }
        self.tabs.set_addr_focused(focused);
    }

    /// 取走"地址栏需全选"的一次性标记(消费即复位),内核 draw 循环据此
    /// 跑 `text_input::select_all`(`main.rs`)。
    pub fn take_addr_select_all_pending(&mut self) -> bool {
        std::mem::take(&mut self.addr_select_all_pending)
    }

    /// 当前激活 tab 的 webview id(内核 `App::active_browser_webview_id`
    /// 焦点路由用)。
    pub fn active_webview_id(&self) -> Option<usize> {
        self.tabs.active_webview_id()
    }

    /// 页签总数(内核 `App` 拖拽换位的越界保护用)。
    pub fn tab_count(&self) -> usize {
        self.tabs.tabs().len()
    }

    /// 当前激活页签下标(内核 `App` 拖拽换位的源记录用)。
    pub fn active_tab_idx(&self) -> usize {
        self.tabs.active_idx()
    }

    /// webview 期望清单(内核 `App::browser_desired` 用,供 main.rs 同步
    /// webview 池)。
    pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
        self.tabs.desired_webviews()
    }

    /// 推进所有 tab 的 hover 动画一拍(每拍残余 50%,约 80ms 收敛),与顶栏
    /// `App::advance_hover_anims` 同款。由内核在全局 HOVER_ANIM 定时里调用,
    /// 浏览器面板靠它复用顶栏那套自驱 redraw(见 `any_hover_active`)。
    pub fn advance_hover_anims(&mut self) {
        for h in self.hover.values_mut() {
            h.advance();
        }
    }

    /// 是否还有 tab hover 动画在进行中(进度未到目标)——内核
    /// `App::any_hover_anim_active` 据此决定是否继续排下一拍定时唤醒。
    pub fn any_hover_active(&self) -> bool {
        self.hover.values().any(TabHover::active)
    }

    /// 维护某页签的 tooltip 悬停计时:进入记起点、离开清除(满 2s 由
    /// `hover_tooltip_ready` 判断)。与 `hover` 动画进度同源触发,但计时是
    /// 独立的一份(见 `tooltip_starts` 字段注释)。
    pub(crate) fn set_tab_tooltip(&mut self, idx: usize, hovered: bool) {
        if hovered {
            self.tooltip_starts.insert(idx, std::time::Instant::now());
        } else {
            self.tooltip_starts.remove(&idx);
        }
    }

    /// 某页签悬停是否已持续满 `HOVER_TOOLTIP_DELAY`(2s):满则视图层弹标题
    /// 全称 tooltip(见 `panel_tab` 的 `show_tooltip` 参数)。
    pub(crate) fn hover_tooltip_ready(&self, idx: usize) -> bool {
        self.tooltip_starts
            .get(&idx)
            .is_some_and(|start| start.elapsed() >= crate::app::HOVER_TOOLTIP_DELAY)
    }

    /// 距下一个 tooltip 计时满 2s 的最短剩余时间——内核 `App::next_tooltip_wake`
    /// 据此排下次唤醒,做到"恰好满 2s 才重绘"。
    pub(crate) fn next_tooltip_wake(&self) -> Option<std::time::Duration> {
        self.tooltip_starts
            .values()
            .filter_map(|start| crate::app::HOVER_TOOLTIP_DELAY.checked_sub(start.elapsed()))
            .min()
    }

    /// 取某 tab 标题(idx, is_close=false)或关闭按钮(idx, is_close=true)的
    /// 当前 hover 进度(0..=1),给 `view` 做颜色插值。
    pub(crate) fn hover_progress(&self, idx: usize, is_close: bool) -> f32 {
        self.hover
            .get(&(idx, is_close))
            .map(|h| h.progress)
            .unwrap_or(0.0)
    }

    /// 星标按钮(当前 tab 收藏/取消收藏)的 hover 进度。用哨兵键
    /// `(STAR_HOVER_KEY, false)` 区分于真实 tab(真实 tab 序号不可能
    /// 等于 `usize::MAX`)。
    pub(crate) fn star_hover(&self) -> f32 {
        self.hover
            .get(&(STAR_HOVER_KEY, false))
            .map(|h| h.progress)
            .unwrap_or(0.0)
    }

    /// 拖拽换位:把 `from` 处的 tab 移到 `to`,并重排 index-keyed 的 hover 键
    /// (键 `(idx, is_close)`)。哨兵键 `STAR_HOVER_KEY` 不受影响。
    pub fn reorder_tab(&mut self, from: usize, to: usize) {
        self.tabs.reorder(from, to);
        // 源与目标之间所有条目顺移一位,把它们的 hover 键跟着挪。
        let lo = from.min(to);
        let hi = from.max(to);
        if lo != hi {
            let mut remap = Vec::new();
            for i in lo..=hi {
                for is_close in [false, true] {
                    if let Some(h) = self.hover.remove(&(i, is_close)) {
                        // 换位后:与 from 同侧锚定更直观——若 i==from 移到的
                        // 是新位 to;否则若从右往左(hi<原次序)各 i 左移,反
                        // 之右移。这里直接让"所在槽位的 hover"跟着槽位走最省心:
                        let new_i = if i == from {
                            to
                        } else if from < to {
                            i - 1
                        } else {
                            i + 1
                        };
                        remap.push((new_i, is_close, h));
                    }
                }
            }
            for (i, is_close, h) in remap {
                self.hover.insert((i, is_close), h);
            }
        }
    }

    /// 收藏夹下拉按钮的 hover 进度,哨兵键 `(STAR_HOVER_KEY, true)`。
    pub(crate) fn bookmark_hover(&self) -> f32 {
        self.hover
            .get(&(STAR_HOVER_KEY, true))
            .map(|h| h.progress)
            .unwrap_or(0.0)
    }

    /// 后退/前进/刷新三颗导航按钮的 hover 进度,各自独立哨兵键
    /// `(NAV_*_KEY, false)`,与真实 tab 序号和星标/收藏夹键都不冲突。
    pub(crate) fn nav_hover(&self, action: NavAction) -> f32 {
        let key = match action {
            NavAction::Back => NAV_BACK_KEY,
            NavAction::Forward => NAV_FORWARD_KEY,
            NavAction::Refresh => NAV_REFRESH_KEY,
        };
        self.hover
            .get(&(key, false))
            .map(|h| h.progress)
            .unwrap_or(0.0)
    }

    /// tab 栏末尾"新标签页"(+)按钮的 hover 进度,哨兵键
    /// `(NEW_TAB_KEY, false)`,远离真实 tab 序号空间。
    pub(crate) fn new_tab_hover(&self) -> f32 {
        self.hover
            .get(&(NEW_TAB_KEY, false))
            .map(|h| h.progress)
            .unwrap_or(0.0)
    }
}

/// 浏览器面板"星标/收藏夹"两个工具栏按钮的 hover 哨兵键——真实 tab 序号
/// 从 0 递增,不可能等于 `usize::MAX`,用它作键不与 tab 冲突。
const STAR_HOVER_KEY: usize = usize::MAX;
/// 后退/前进/刷新三颗导航按钮的 hover 哨兵键,依次紧挨 `STAR_HOVER_KEY`
/// 往下排,同样远离真实 tab 序号空间。
const NAV_BACK_KEY: usize = usize::MAX - 1;
const NAV_FORWARD_KEY: usize = usize::MAX - 2;
const NAV_REFRESH_KEY: usize = usize::MAX - 3;
/// tab 栏末尾"新标签页"(+)按钮的 hover 哨兵键,继续沿用上面的哨兵键空间。
const NEW_TAB_KEY: usize = usize::MAX - 4;

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
        }
        Message::TitleLoaded(id, title) => state.tabs.apply_title(id, title),
        Message::SelectTab(idx) => state.tabs.select(idx),
        Message::DragHover(_) => {} // 拖拽换位在 `App::update` 翻译后处理,不落到这里
        // 导航按钮:真正的 webview 历史导航/刷新由 main.rs `dispatch` 拦截
        // `Message::Browser(Nav(..))` 执行,这里不碰状态(no-op)。
        Message::Nav(_) => {}
        Message::CloseTab(idx) => {
            state.tabs.close(idx);
        }
        Message::Hover(idx, is_close, hovered) => {
            state.hover.entry((idx, is_close)).or_default().set(hovered);
            // 标题 tooltip 计时:进入即记起点、离开即清(满 2s 由
            // `hover_tooltip_ready` 判断,与关闭按钮的 hover 无关,只认页签
            // 整体悬停)。
            state.set_tab_tooltip(idx, hovered);
        }
        Message::AddrInput(s) => state.tabs.set_addr_buffer(s),
        Message::AddrSubmit => match state.tabs.addr_submit() {
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
        Message::StarClick => {
            state.error = None;
            let Some(url) = state
                .tabs
                .tabs()
                .get(state.tabs.active_idx())
                .map(|t| t.url.clone())
            else {
                return;
            };
            let status = bookmark_status(&state.bookmarks, &url, project_id);
            if !status.is_bookmarked() {
                // 未收藏 → 原样切换"加入收藏"菜单的开合(与旧行为一致)。
                state.star_menu_open = !state.star_menu_open;
                return;
            }
            // 已收藏(选中态)→直接点击即全部移出收藏,不再弹菜单。复用
            // `BookmarkRemove` 的处理(乐观移除 + dozerd RPC),逐 scope 派发。
            state.star_menu_open = false;
            for id in status.global.into_iter().chain(status.project) {
                emit(Message::BookmarkRemove(id));
            }
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
        Message::ColumnDragStart => {
            debug_assert!(
                false,
                "ColumnDragStart 由内核在 Message::Browser 分支里直接处理\
                 (转成 app 级拖拽消息),不会转发到这里"
            );
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

/// 地址栏后退/前进/刷新导航按钮。复用统一 icon 按钮规范
/// (`icon_button_entry`),静止 DIM、hover 平滑过渡到 GOLD,与同行星标/
/// 收藏夹按钮观感一致。点击发 `Message::Nav(action)`,真正的 webview
/// 历史导航/刷新由 main.rs `dispatch` 拦截执行。
fn nav_button(
    state: &State,
    action: NavAction,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (kind, tooltip, key) = match action {
        NavAction::Back => (icons::IconKind::CircleArrowLeft, "后退", NAV_BACK_KEY),
        NavAction::Forward => (icons::IconKind::CircleArrowRight, "前进", NAV_FORWARD_KEY),
        NavAction::Refresh => (icons::IconKind::RotateCw, "刷新", NAV_REFRESH_KEY),
    };
    icons::icon_button_entry(
        kind,
        byteui::theme::icon_size::row(),
        false,
        false,
        state.nav_hover(action),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::Nav(action),
        move |hovered| Message::Hover(key, false, hovered),
        tooltip,
    )
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
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let url = state
        .tabs
        .tabs()
        .get(state.tabs.active_idx())
        .map(|t| t.url.clone());
    let starred = url
        .as_ref()
        .map(|u| bookmark_status(&state.bookmarks, u, project_id).is_bookmarked())
        .unwrap_or(false);
    // 统一 icon 按钮规范:未收藏静止 DIM、hover 过渡到 GOLD;已收藏恒金
    // (active=true)。hover 动画走浏览器自己的 `State` 进度机(哨兵键)。
    icons::icon_button_entry(
        icons::IconKind::Star,
        byteui::theme::icon_size::row(),
        starred,
        false,
        state.star_hover(),
        false,
        byteui::theme::geometry::tab_button_size(),
        url.is_some(),
        Message::StarClick,
        |hovered| Message::Hover(STAR_HOVER_KEY, false, hovered),
        "收藏",
    )
}

/// 地址栏星标按钮:加入/移出收藏夹(弹出菜单)。图标用 `Star`,已收藏恒金。
/// 样式统一走 `crate::menu::item_row_fill`(整行撑满所属面板宽)。
fn bookmark_menu_row(
    label: String,
    msg: Message,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    crate::menu::item_row_fill(
        None,
        label,
        byteui::theme::color::current().cream,
        Some(msg),
    )
}

/// 星标小菜单:未收藏显示"加入…",已收藏显示"移出…"(打勾态)。
fn star_menu_popup(
    state: &State,
    project_id: Option<i64>,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(url) = state
        .tabs
        .tabs()
        .get(state.tabs.active_idx())
        .map(|t| t.url.clone())
    else {
        return column![].into();
    };
    let status = bookmark_status(&state.bookmarks, &url, project_id);

    let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        vec![match status.global {
            Some(id) => bookmark_menu_row("移出全局收藏".to_string(), Message::BookmarkRemove(id)),
            None => bookmark_menu_row(
                "加入全局收藏".to_string(),
                Message::BookmarkAdd(BookmarkScope::Global),
            ),
        }];

    if project_id.is_some() {
        items.push(match status.project {
            Some(id) => {
                bookmark_menu_row("移出本项目收藏".to_string(), Message::BookmarkRemove(id))
            }
            None => bookmark_menu_row(
                "加入本项目收藏".to_string(),
                Message::BookmarkAdd(BookmarkScope::Project),
            ),
        });
    }

    crate::menu::shell(items, Length::Fill)
}

/// 一组收藏条目:文件夹图标 + 标题(点击新开 tab)+ `×` 删除按钮,风格照抄
/// tab 关闭按钮。条目在文件夹标题下缩进一级,呼应收藏夹"文件夹树"视觉。
fn bookmark_group<'a>(
    title: &'static str,
    items: &[&'a BookmarkInfo],
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let header = row![
        icons::view(
            icons::IconKind::Folder,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().dim
        ),
        lh(text(title)
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().dim)),
    ]
    .spacing(4)
    .align_y(iced_widget::core::Alignment::Center);
    let mut col = column![header].spacing(2);
    for b in items {
        let open = button(lh(text(b.title.clone())
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream)))
        .on_press(Message::OpenUrl(b.url.clone()))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: byteui::theme::color::current().cream,
            ..button::Style::default()
        });
        let remove = button(lh(text("×")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim)))
        .on_press(Message::BookmarkRemove(b.id))
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: byteui::theme::color::current().dim,
            ..button::Style::default()
        });
        col = col.push(
            row![
                iced_widget::Space::new().width(Length::Fixed(16.0)),
                row![open, remove]
                    .spacing(4)
                    .align_y(iced_widget::core::Alignment::Center),
            ]
            .width(Length::Fill),
        );
    }
    col.into()
}

/// 收藏夹侧栏:分"全局收藏"/"本项目收藏"两组文件夹分组,都为空时显示占位
/// 文案。`width` 由调用方按配对布局里侧栏那一份宽度的 flex 权重传入,侧栏
/// 背景用列表侧一致的 `byteui::theme::color::current().bg`(结构性常驻侧栏,不再是盖在下方的
/// 浮层卡片)。
fn bookmarks_panel(
    state: &State,
    project_id: Option<i64>,
    width: Length,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
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
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().dim)));
    }

    container(col)
        .padding(6)
        .width(width)
        .height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().bg.into()),
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
    bookmarks_split: f32,
    width: Length,
    outer: Border,
    mirror: bool,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::browser_pane();

    let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = state
        .tabs
        .tabs()
        .iter()
        .enumerate()
        .map(|(idx, tab)| {
            let active = idx == state.tabs.active_idx();
            let title_hover_t = state.hover_progress(idx, false);
            let close_hover_t = state.hover_progress(idx, true);
            let tab = panel_tab(
                tab.title.clone(),
                active,
                title_hover_t,
                close_hover_t,
                None,
                None,
                Message::SelectTab(idx),
                Message::CloseTab(idx),
                state.hover_tooltip_ready(idx),
                move |h| Message::Hover(idx, false, h),
                move |h| Message::Hover(idx, true, h),
            );
            // 拖拽换位:按住页签(App 侧把 `tab_drag` 置位)后光标扫过哪个
            // 页签,这个 `on_move` 发出 `DragHover(idx)`,再在 `App::update`
            // 翻译成 `TabDragMove`(组校验在那里做),完成换位。
            MouseArea::new(tab)
                .on_move(move |_| Message::DragHover(idx))
                .into()
        })
        .collect();
    let tabs_row = row(items).spacing(4);
    // 页签自身区域可横向裁切(溢出部分 clip),但末尾的"新标签页"(+)按钮
    // 不做裁切,始终钉在 tab 栏右端可点——避免页签一多就被挤出可视区。
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    let new_tab_btn = icons::icon_button_entry(
        icons::IconKind::SquarePlus,
        byteui::theme::icon_size::row(),
        false,
        false,
        state.new_tab_hover(),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::OpenUrl("about:blank".to_string()),
        |hovered| Message::Hover(NEW_TAB_KEY, false, hovered),
        "新标签页",
    );
    let tab_bar = row![clipped, new_tab_btn]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);

    let editing = state.addr_focused();
    // 未聚焦时地址栏回显当前激活 tab 的完整网址(不再只显示"输入网址"
    // 占位符)——聚焦时仍用 `addr_buffer`(焦点刚获得时已预填当前网址)。
    let active_url = state
        .tabs
        .tabs()
        .get(state.tabs.active_idx())
        .map(|t| t.url.as_str())
        .unwrap_or("");
    let addr_value: &str = if editing {
        state.tabs.addr_buffer()
    } else {
        active_url
    };
    let addr_input = byteui::form::input_text::view(
        "输入网址",
        addr_value,
        false,
        Some(addr_field_id()),
        false,
        Some(Message::AddrSubmit),
        true,
        Message::AddrInput,
    );

    // 地址栏本体:单个带边框的容器,把"网址文字 + 星标(收藏)按钮"一起包进
    // 边框内(复用 todo 新增输入框 / `crate::search_box` 的布局模式)。不再
    // 需要外层 `MouseArea`/`AddrClick`——`text_input` 是真控件,点击命中范围内
    // 就由 iced 标准鼠标管线自己处理聚焦,不需要应用层代理点击(唯一影响:
    // 点击胶囊的 4px padding 空白处不再能进编辑态,只有点在输入框自身范围
    // 内才行,判定为可接受的小回归,见本计划 Global Constraints)。星标按钮
    // 是内层 widget,自己截获点击(弹加入/移出收藏夹菜单)。框高由按钮的
    // 方形尺寸撑起,文字垂直居中,视觉上按钮嵌在地址栏右侧。
    let content_h = byteui::theme::geometry::tab_button_size();
    let addr_box = container(
        row![
            container(addr_input)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Center)
                .align_x(iced_widget::core::alignment::Horizontal::Left),
            container(star_button(state, project_id))
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Center),
        ]
        .width(Length::Fill)
        .height(Length::Fixed(content_h))
        .align_y(iced_widget::core::Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(content_h + 8.0))
    .padding([4, 8])
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: Some(byteui::theme::color::current().term_bg.into()),
        border: Border {
            color: if editing {
                byteui::theme::color::current().gold
            } else {
                byteui::theme::color::current().border
            },
            width: 1.0,
            radius: 2.0.into(),
        },
        ..container::Style::default()
    });

    // 后退/前进/刷新三颗导航按钮紧凑成组(组内间距 0,比下方整体 4 更紧),
    // 再与地址栏/收藏等拉开到 4,突出"导航簇"的视觉聚合。
    let nav_buttons = row![
        nav_button(state, NavAction::Back),
        nav_button(state, NavAction::Forward),
        nav_button(state, NavAction::Refresh),
    ]
    .spacing(0)
    .align_y(iced_widget::core::Alignment::Center);

    let addr_row = row![nav_buttons, addr_box, bookmarks_toggle_button(state),]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);

    let mut content = column![tab_bar, tab_divider(), addr_row].spacing(region.gap);

    if state.star_menu_open {
        content = content.push(star_menu_popup(state, project_id));
    }
    if let Some(err) = &state.error {
        content = content.push(lh(text(format!("⚠ {err}"))
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().red)));
    }

    let body: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if state.tabs.tabs().is_empty() {
            container(lh(text("暂无网页——在地址栏输入网址")
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().dim)))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            // 真实网页由 wry webview 叠加渲染,这里只需要一块透明占位
            // (不能有不透明背景,否则会盖住 webview)。
            iced_widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

    content = content.push(if state.bookmarks_open {
        let bg = region
            .background
            .unwrap_or(byteui::theme::color::current().bg);
        let (list_portion, content_portion) = split_portions(1.0 - bookmarks_split);
        let content_box = container(body).width(Length::FillPortion(content_portion));
        let bookmarks_box = bookmarks_panel(state, project_id, Length::FillPortion(list_portion));
        let divider = crate::app::divider_bar(
            crate::app::Divider::BrowserBookmarksSplit,
            bg,
            bg,
            Message::ColumnDragStart,
        );
        if mirror {
            row![bookmarks_box, divider, content_box]
        } else {
            row![content_box, divider, bookmarks_box]
        }
        .height(Length::Fill)
        .into()
    } else {
        body
    });

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

/// 地址栏"收藏夹"展开按钮:切换收藏夹侧栏。图标用 `FolderBookmark`,颜色
/// 恒定(不像星标那样带收藏状态)。走统一 icon 按钮规范(DIM→GOLD hover,
/// 无选中态),hover 动画走浏览器自己的 `State` 进度机(哨兵键)。
fn bookmarks_toggle_button(
    state: &State,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    icons::icon_button_entry(
        icons::IconKind::FolderBookmark,
        byteui::theme::icon_size::row(),
        false,
        false,
        state.bookmark_hover(),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::BookmarksToggle,
        |hovered| Message::Hover(STAR_HOVER_KEY, true, hovered),
        "收藏夹",
    )
}
