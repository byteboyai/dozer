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
        self.tabs.push(BrowserTab {
            id,
            url,
            title,
        });
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
        assert_eq!(
            t.addr_submit(),
            Err("浏览器不支持打开本地文件".to_string())
        );

        t.addr_begin();
        t.addr_text("~/x");
        assert_eq!(
            t.addr_submit(),
            Err("浏览器不支持打开本地文件".to_string())
        );

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
}
