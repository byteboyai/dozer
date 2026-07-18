//! 预览域状态机(P1d):左二 tabs、地址栏编辑态、webview 期望清单。
//! 纯数据,不碰 wry/iced——webview 副作用由 main.rs 对照
//! `desired_webviews()` 差集执行(spike 约束:句柄只活在事件分发环)。
use std::path::PathBuf;

/// 一个预览 tab。`TabKind::Diff` 变体留给 P1f(验收闭环)补。
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewTab {
    pub id: usize,
    pub kind: TabKind,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TabKind {
    File(PathBuf),
    Web { url: String },
}

/// 地址栏提交的解析结果:绝对路径 → 文件预览;其余按 URL 处理
/// (无 scheme 自动补 `http://`,localhost 场景免敲协议头)。
#[derive(Debug, Clone, PartialEq)]
pub enum AddrTarget {
    File(PathBuf),
    Url(String),
}

/// main.rs 同步 webview 的期望清单项。
#[derive(Debug, Clone, PartialEq)]
pub struct WebviewSpec {
    pub id: usize,
    pub url: String,
    pub visible: bool,
}

/// RFC3986 严格百分号编码:unreserved(字母/数字/`-._~`)之外全部 %XX。
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Default)]
pub struct PreviewPane {
    tabs: Vec<PreviewTab>,
    active: usize,
    next_id: usize,
    addr_editing: bool,
    addr_buffer: String,
}

impl PreviewPane {
    pub fn tabs(&self) -> &[PreviewTab] {
        &self.tabs
    }

    pub fn active_idx(&self) -> usize {
        self.active
    }

    pub fn open_path(&mut self, path: PathBuf) -> usize {
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        self.push_tab(TabKind::File(path), title)
    }

    pub fn open_url(&mut self, url: String) -> usize {
        let title = url
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .split('/')
            .next()
            .unwrap_or(&url)
            .to_string();
        self.push_tab(TabKind::Web { url: url.clone() }, title)
    }

    fn push_tab(&mut self, kind: TabKind, title: String) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(PreviewTab { id, kind, title });
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

    /// 进入地址栏编辑:预填当前激活网页 tab 的 URL(文件 tab 不预填)。
    pub fn addr_begin(&mut self) {
        self.addr_editing = true;
        self.addr_buffer = match self.tabs.get(self.active).map(|t| &t.kind) {
            Some(TabKind::Web { url }) => url.clone(),
            _ => String::new(),
        };
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

    pub fn addr_submit(&mut self) -> Option<AddrTarget> {
        self.addr_editing = false;
        let input = std::mem::take(&mut self.addr_buffer);
        let input = input.trim();
        if input.is_empty() {
            return None;
        }
        if input.starts_with('/') {
            return Some(AddrTarget::File(PathBuf::from(input)));
        }
        if let Some(rest) = input.strip_prefix("~/") {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            return Some(AddrTarget::File(PathBuf::from(home).join(rest)));
        }
        if input.contains("://") {
            return Some(AddrTarget::Url(input.to_string()));
        }
        Some(AddrTarget::Url(format!("http://{input}")))
    }

    /// webview 期望清单:每 tab 一个,仅激活者可见(设计 D2)。
    pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(idx, tab)| WebviewSpec {
                id: tab.id,
                url: match &tab.kind {
                    TabKind::File(path) => format!(
                        "dozer://flyfish/host.html?p={}",
                        encode_component(&path.to_string_lossy())
                    ),
                    TabKind::Web { url } => url.clone(),
                },
                visible: idx == self.active,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn open_select_close_tabs() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.md"));
        let id1 = p.open_url("http://localhost:3000".into());
        assert_eq!(p.tabs().len(), 2);
        assert_eq!(p.active_idx(), 1, "新开 tab 即激活");
        assert_ne!(id0, id1);
        assert_eq!(p.tabs()[0].title, "a.md");
        assert_eq!(p.tabs()[1].title, "localhost:3000");
        p.select(0);
        assert_eq!(p.active_idx(), 0);
        p.close(0);
        assert_eq!(p.tabs().len(), 1);
        assert_eq!(p.active_idx(), 0);
    }

    #[test]
    fn addr_edit_and_submit_parses_path_vs_url() {
        let mut p = PreviewPane::default();
        p.addr_begin();
        assert!(p.addr_editing());
        for c in "/tmp/设计 稿.pdf".chars() {
            p.addr_text(&c.to_string());
        }
        assert_eq!(
            p.addr_submit(),
            Some(AddrTarget::File(PathBuf::from("/tmp/设计 稿.pdf")))
        );
        assert!(!p.addr_editing());

        p.addr_begin();
        p.addr_text("localhost:3000/x");
        assert_eq!(
            p.addr_submit(),
            Some(AddrTarget::Url("http://localhost:3000/x".into()))
        );

        p.addr_begin();
        p.addr_text("https://example.com");
        assert_eq!(
            p.addr_submit(),
            Some(AddrTarget::Url("https://example.com".into()))
        );

        p.addr_begin();
        p.addr_text("abc");
        p.addr_backspace();
        p.addr_backspace();
        p.addr_backspace();
        assert_eq!(p.addr_submit(), None, "空输入不产生动作");

        p.addr_begin();
        p.addr_text("x");
        p.addr_cancel();
        assert!(!p.addr_editing());
    }

    #[test]
    fn desired_webviews_builds_urls_and_visibility() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/a b.md"));
        p.open_url("http://localhost:3000".into());
        let specs = p.desired_webviews();
        assert_eq!(specs.len(), 2);
        assert_eq!(
            specs[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa%20b.md"
        );
        assert!(!specs[0].visible, "非激活 tab 不可见");
        assert_eq!(specs[1].url, "http://localhost:3000");
        assert!(specs[1].visible);
    }

    #[test]
    fn encode_component_is_rfc3986_strict() {
        assert_eq!(encode_component("aZ09-._~"), "aZ09-._~");
        assert_eq!(encode_component("/a b"), "%2Fa%20b");
        assert_eq!(encode_component("你"), "%E4%BD%A0");
    }
}
