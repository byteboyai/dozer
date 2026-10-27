//! 浏览器专用的精简 tab/地址栏状态机,只认 URL——不复用文件预览的 `PreviewPane`
//! (那边还有 `TabKind::File`/`Acceptance`、`open_path` 等浏览器用不到的逻辑),两者各自维护、互不知情。

use crate::WEB_OWNER;
use crate::adapter::BrowserWebviewSpec;

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
    fn title_for(url: &str) -> String {
        url.trim_start_matches("http://")
            .trim_start_matches("https://")
            .split('/')
            .next()
            .unwrap_or(url)
            .to_string()
    }

    /// 每次调用都新开一个 tab,不做去重——原样对齐现有
    /// `PreviewPane::open_url` 的行为(`open_path` 才有去重,`open_url`
    /// 没有),纯重构不改变这条现状。返回新开 tab 的 id。
    pub fn open_url(&mut self, url: String) -> usize {
        let title = Self::title_for(&url);
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(BrowserTab { id, url, title });
        self.active = self.tabs.len() - 1;
        id
    }

    /// 在当前激活 tab 打开 URL:复用激活 tab(更新其 url/title,不新增),
    /// 区别于 `open_url` 的"总是新开 tab"。tab 组理论上恒非空(`close` 兜底
    /// 会补 `about:blank`),但为稳妥异常情况仍新开。返回被复用的 tab 的 id。
    pub fn navigate_active(&mut self, url: String) -> usize {
        let title = Self::title_for(&url);
        if self.tabs.is_empty() {
            let id = self.next_id;
            self.next_id += 1;
            self.tabs.push(BrowserTab { id, url, title });
            return id;
        }
        let idx = self.active.min(self.tabs.len() - 1);
        self.tabs[idx].url = url;
        self.tabs[idx].title = title;
        self.active = idx;
        self.tabs[idx].id
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

    /// 网页内超链接点击后,渲染进程经 IPC 报回 webview 实际导航到的 URL
    /// (`id` 是 webview/tab id)。把对应 tab 的 `url` 原地更新为真实地址、
    /// `title` 退回 URL 标题,让地址栏/页签与页面保持一致;目标页随后由页面
    /// 标题回报(`TitleLoaded`)覆盖成 HTML `<title>`。URL 没变(同址导航/
    /// 冗余回报)时 no-op,避免造成消息风暴。
    pub fn apply_url(&mut self, id: usize, url: String) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id)
            && tab.url != url
        {
            tab.url = url.clone();
            tab.title = Self::title_for(&url);
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
    pub fn addr_submit(&mut self) -> Result<Option<String>, dozer_model::LocalizedText> {
        let input = std::mem::take(&mut self.addr_buffer);
        let input = input.trim();
        if input.is_empty() {
            return Ok(None);
        }
        if input.starts_with('/') || input.starts_with("~/") {
            return Err(dozer_model::LocalizedText::keyed(
                WEB_OWNER,
                "error.local_path",
                "Opening local files is not supported in the browser",
            ));
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
    pub fn desired_webviews(&self) -> Vec<BrowserWebviewSpec> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(idx, tab)| BrowserWebviewSpec {
                id: tab.id,
                url: tab.url.clone(),
                visible: idx == self.active,
            })
            .collect()
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
    use crate::WEB_OWNER;
    use crate::adapter::BrowserWebviewSpec;

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
        assert_eq!(
            t.addr_submit(),
            Err(dozer_model::LocalizedText::keyed(
                WEB_OWNER,
                "error.local_path",
                "Opening local files is not supported in the browser"
            ))
        );

        t.set_addr_focused(true);
        t.set_addr_buffer("~/x".to_string());
        assert_eq!(
            t.addr_submit(),
            Err(dozer_model::LocalizedText::keyed(
                WEB_OWNER,
                "error.local_path",
                "Opening local files is not supported in the browser"
            ))
        );

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

    #[test]
    fn navigate_active_reuses_active_tab_without_adding() {
        let mut t = Tabs::default();
        t.open_url("http://a.com".into());
        t.open_url("http://b.com".into());
        assert_eq!(t.tabs().len(), 2);
        t.select(0);
        let id = t.navigate_active("http://c.com".into());
        assert_eq!(t.tabs().len(), 2, "复用激活 tab,数量不变");
        assert_eq!(t.tabs()[0].url, "http://c.com");
        assert_eq!(t.tabs()[0].title, "c.com");
        assert_eq!(t.active_idx(), 0);
        assert_eq!(t.tabs()[0].id, id);
        assert_eq!(t.tabs()[1].url, "http://b.com", "无关 tab 不受影响");
    }

    #[test]
    fn apply_url_updates_matching_tab_in_place() {
        let mut t = Tabs::default();
        let id = t.open_url("http://a.com".into());
        t.open_url("http://b.com".into());
        // 目标 tab 自己的 url 被更新,title 退回 URL 标题;无关 tab 不受影响。
        t.apply_url(id, "http://nav.com".into());
        assert_eq!(t.tabs()[0].url, "http://nav.com");
        assert_eq!(t.tabs()[0].title, "nav.com");
        assert_eq!(t.tabs()[1].url, "http://b.com");
        // 同址冗余回报(如同址导航)是 no-op,不重写 title。
        t.apply_url(id, "http://nav.com".into());
        assert_eq!(t.tabs()[0].url, "http://nav.com");
        // 未知 id 是 no-op。
        t.apply_url(999, "http://x.com".into());
        assert_eq!(t.tabs().len(), 2);
    }

    /// 页签组永不为空:关掉唯一页签后补一个 `about:blank`(`Tabs::close` 本身兜底,不是 `State`)。
    #[test]
    fn closing_last_tab_leaves_one_blank_tab() {
        let mut t = Tabs::default();
        t.open_url("http://a.com".into());
        t.close(0);
        assert_eq!(t.tabs().len(), 1);
        assert_eq!(t.tabs()[0].url, "about:blank");
        assert_eq!(t.active_idx(), 0);
    }

    #[test]
    fn desired_webviews_maps_one_to_one() {
        let mut t = Tabs::default();
        let ids = [
            t.open_url("http://a.com".into()),
            t.open_url("http://b.com".into()),
            t.open_url("http://c.com".into()),
        ];
        t.select(1);
        let specs: Vec<BrowserWebviewSpec> = t.desired_webviews();
        assert_eq!(specs.len(), 3);
        for (i, s) in specs.iter().enumerate() {
            assert_eq!(s.id, ids[i], "id 与页签 id 一致");
            assert_eq!(s.visible, i == 1, "仅激活页签可见");
        }
    }
}
