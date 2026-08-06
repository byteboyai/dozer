//! 预览域状态机(P1d):左二 tabs、地址栏编辑态、webview 期望清单。
//! 纯数据,不碰 wry/iced——webview 副作用由 main.rs 对照
//! `desired_webviews()` 差集执行(spike 约束:句柄只活在事件分发环)。
use std::path::PathBuf;

/// 一个预览 tab。
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewTab {
    pub id: usize,
    pub kind: TabKind,
    pub title: String,
    /// 保存编辑后 `+1`,驱动 `desired_webviews()` 换 URL 逼 `sync_webview_pool`
    /// 重新 `load_url`(同 URL 不会重载,flyfish 的 WKWebView 会一直显示
    /// 保存前的旧内容)。
    pub reload_nonce: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TabKind {
    File(PathBuf),
    Web {
        url: String,
    },
    /// 验收 tab（P1f）:不产 webview,内容由 iced 直绘。
    Acceptance,
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

/// `TabKind::File` → flyfish 渲染 URL 的唯一决策点。目前只有这一条渲染
/// 路径;把它从内联拼接抽成具名函数,是为将来"某些扩展名不走 flyfish、
/// 走 Acceptance 式 iced 原生 pane"的分叉预留一个函数级插入点——不引入
/// trait/注册表,YAGNI。
fn flyfish_url(path: &std::path::Path) -> String {
    format!(
        "dozer://flyfish/host.html?p={}",
        encode_component(&path.to_string_lossy())
    )
}

/// "编辑"按钮的显示范围:纯扩展名白名单,不做内容嗅探(YAGNI,见设计文档
/// "范围外")。`.gitignore` 这类点开头、`Path::extension()` 认不出扩展名
/// 的文件单独特判文件名。
pub fn is_editable_extension(path: &std::path::Path) -> bool {
    if path.file_name().and_then(|n| n.to_str()) == Some(".gitignore") {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "rs" | "toml"
            | "md"
            | "txt"
            | "json"
            | "yaml"
            | "yml"
            | "sh"
            | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "html"
            | "css"
            | "xml"
            | "log"
            | "conf"
    )
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
        // 同一文件已开则切过去,不重复开 tab（验收反馈）。
        if let Some((idx, tab)) = self
            .tabs
            .iter()
            .enumerate()
            .find(|(_, t)| t.kind == TabKind::File(path.clone()))
        {
            let id = tab.id;
            self.active = idx;
            return id;
        }
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
        self.tabs.push(PreviewTab {
            id,
            kind,
            title,
            reload_nonce: 0,
        });
        self.active = self.tabs.len() - 1;
        id
    }

    /// 打开验收 tab:已存在则激活复用（全局至多一个）。
    pub fn open_acceptance(&mut self) -> usize {
        if let Some((idx, tab)) = self
            .tabs
            .iter()
            .enumerate()
            .find(|(_, t)| t.kind == TabKind::Acceptance)
        {
            let id = tab.id;
            self.active = idx;
            return id;
        }
        self.push_tab(TabKind::Acceptance, "验收".to_string())
    }

    /// 当前激活 tab 是否验收 tab。
    pub fn acceptance_active(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|t| t.kind == TabKind::Acceptance)
    }

    /// 当前激活 tab 若是 webview(文件/网页)则返回其 id(=webview 池的 key)。
    pub fn active_webview_id(&self) -> Option<usize> {
        self.tabs.get(self.active).and_then(|t| match t.kind {
            TabKind::File(_) | TabKind::Web { .. } => Some(t.id),
            TabKind::Acceptance => None,
        })
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

    /// webview 期望清单:每文件/网页 tab 一个,仅激活者可见(设计 D2)；
    /// 验收 tab 不产 webview,且它激活时其余 webview 全隐藏(iced 直绘 pane)。
    pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
        // 验收 tab 是 iced 直绘的覆盖层,它激活时其余 webview 全隐藏。
        let overlay_active = self.acceptance_active();
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(idx, tab)| {
                let url = match &tab.kind {
                    TabKind::File(path) => {
                        let mut u = flyfish_url(path);
                        if tab.reload_nonce > 0 {
                            u.push_str(&format!("&_r={}", tab.reload_nonce));
                        }
                        u
                    }
                    TabKind::Web { url } => url.clone(),
                    TabKind::Acceptance => return None,
                };
                Some(WebviewSpec {
                    id: tab.id,
                    url,
                    visible: idx == self.active && !overlay_active,
                })
            })
            .collect()
    }

    /// 编辑保存后调用:按 `PreviewTab.id` 找到对应 tab,推进它的 reload
    /// 计数器。未知 id 是 no-op(tab 可能已被关闭)。
    pub fn bump_reload(&mut self, tab_id: usize) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.reload_nonce += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

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
    fn bump_reload_appends_query_param_and_only_affects_target_tab() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.md"));
        let _id1 = p.open_path(PathBuf::from("/tmp/b.md"));
        p.bump_reload(id0);
        let specs = p.desired_webviews();
        assert_eq!(
            specs[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&_r=1"
        );
        assert_eq!(specs[1].url, "dozer://flyfish/host.html?p=%2Ftmp%2Fb.md");
        p.bump_reload(id0);
        assert_eq!(
            p.desired_webviews()[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&_r=2"
        );
        // 未知 id 是 no-op,不 panic。
        p.bump_reload(9999);
    }

    #[test]
    fn is_editable_extension_covers_common_text_types() {
        assert!(is_editable_extension(Path::new("main.rs")));
        assert!(is_editable_extension(Path::new("Cargo.toml")));
        assert!(is_editable_extension(Path::new("README.md")));
        assert!(
            is_editable_extension(Path::new("notes.TXT")),
            "大小写不敏感"
        );
        assert!(is_editable_extension(Path::new("package.json")));
        assert!(is_editable_extension(Path::new("ci.yaml")));
        assert!(is_editable_extension(Path::new("ci.yml")));
        assert!(is_editable_extension(Path::new("run.sh")));
        assert!(is_editable_extension(Path::new("app.py")));
        assert!(is_editable_extension(Path::new("index.js")));
        assert!(is_editable_extension(Path::new("index.ts")));
        assert!(is_editable_extension(Path::new("index.tsx")));
        assert!(is_editable_extension(Path::new("index.jsx")));
        assert!(is_editable_extension(Path::new("page.html")));
        assert!(is_editable_extension(Path::new("style.css")));
        assert!(is_editable_extension(Path::new("data.xml")));
        assert!(is_editable_extension(Path::new("out.log")));
        assert!(is_editable_extension(Path::new("nginx.conf")));
        assert!(
            is_editable_extension(Path::new(".gitignore")),
            "点开头的无扩展名文件要特判"
        );
    }

    #[test]
    fn is_editable_extension_rejects_unknown_and_binary_like() {
        assert!(!is_editable_extension(Path::new("logo.png")));
        assert!(!is_editable_extension(Path::new("archive.zip")));
        assert!(
            !is_editable_extension(Path::new("LICENSE")),
            "无扩展名不在白名单里"
        );
        assert!(!is_editable_extension(Path::new("Makefile")));
    }

    #[test]
    fn reopening_same_file_reuses_tab() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.md"));
        p.open_url("http://localhost:3000".into()); // 中间插一个,把激活挪走
        assert_eq!(p.active_idx(), 1);
        let id_again = p.open_path(PathBuf::from("/tmp/a.md"));
        assert_eq!(id_again, id0, "同文件复用同一 tab");
        assert_eq!(p.tabs().len(), 2, "不新增 tab");
        assert_eq!(p.active_idx(), 0, "切回已开的那个 tab");
    }

    #[test]
    fn acceptance_tab_produces_no_webview_and_hides_others() {
        let mut p = PreviewPane::default();
        p.open_url("http://localhost:3000".into());
        let acc_id = p.open_acceptance();
        let specs = p.desired_webviews();
        assert_eq!(specs.len(), 1, "验收 tab 不产 webview");
        assert!(!specs[0].visible, "验收 tab 激活时其余全隐藏");
        // 重复打开复用同一 tab
        assert_eq!(p.open_acceptance(), acc_id);
        assert_eq!(p.tabs().len(), 2);
        // 切回网页 tab → webview 复显
        p.select(0);
        assert!(p.desired_webviews()[0].visible);
    }

    #[test]
    fn encode_component_is_rfc3986_strict() {
        assert_eq!(encode_component("aZ09-._~"), "aZ09-._~");
        assert_eq!(encode_component("/a b"), "%2Fa%20b");
        assert_eq!(encode_component("你"), "%E4%BD%A0");
    }
}
