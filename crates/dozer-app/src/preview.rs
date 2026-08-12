//! 预览域状态机(P1d):左二 tabs、webview 期望清单。纯数据,不碰
//! wry/iced——webview 副作用由 main.rs 对照 `desired_webviews()` 差集
//! 执行(spike 约束:句柄只活在事件分发环)。
//!
//! 地址栏/URL tab(`TabKind::Web`)、`AddrTarget` 这套逻辑已经随浏览器
//! 面板扩展化(`extensions::browser::Tabs`)搬走——文件预览面板从来没有
//! 地址栏,这里只保留文件/验收两种 tab。
use std::path::PathBuf;

use crate::theme;

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
///
/// 文本文件(`is_editable_extension`)额外挂 `&ln=1`:host.html 读到后给
/// flyfish 的 text 渲染器开 `options.text.lineNumbers`,预览里显示行号
///(图片/PDF 渲染器不认这个 option,挂了也无副作用)。
fn flyfish_url(path: &std::path::Path) -> String {
    let mut u = format!(
        "dozer://flyfish/host.html?p={}",
        encode_component(&path.to_string_lossy())
    );
    if is_editable_extension(path) {
        u.push_str("&ln=1");
    }
    u
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

    /// 当前激活 tab 若是文件(webview)则返回其 id(=webview 池的 key)。
    pub fn active_webview_id(&self) -> Option<usize> {
        self.tabs.get(self.active).map(|t| t.id)
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

    /// 拖拽换位:把 `from` 处的 tab 移到 `to` 处,并同步 `active` 下标。`from`
    /// 与 `to` 相等或越界时是 no-op。返回移动前后 `active` 是否变化(调用方
    /// 据此决定是否要重排 index-keyed 的 hover 动画键)。
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        if self.active == from {
            self.active = to;
        } else if from < self.active && to >= self.active {
            // 源在激活项左侧,且目标落到了激活项右侧/身上——激活项左移一位。
            self.active -= 1;
        } else if from > self.active && to <= self.active {
            // 源在激活项右侧,且目标落到了激活项左侧/身上——激活项右移一位。
            self.active += 1;
        }
    }

    /// webview 期望清单:每文件 tab 一个,仅激活者可见(设计 D2)。
    pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(idx, tab)| {
                let TabKind::File(path) = &tab.kind;
                let mut u = flyfish_url(path);
                if tab.reload_nonce > 0 {
                    u.push_str(&format!("&_r={}", tab.reload_nonce));
                }
                WebviewSpec {
                    id: tab.id,
                    url: u,
                    visible: idx == self.active,
                }
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

/// 编辑器 chrome 对齐到 Dozer 的 ByteBoy2077 配色——是编辑器来适配
/// Dozer,不是反过来让四栏骨架迁就编辑器的默认蓝底。背景/文本/行号栏/
/// 滚动条/当前行高亮这一层与 bg `#0a0e16` + 奶油 `#FFE5B4` + 青
/// `#47DEF0` + 边框灰 `#1c3440` 一致。金 `#F2D94E` 是"甲方动作专属",
/// 不在此处使用。语法高亮的 token 颜色由 `dozer_syntax_theme` 单独接管。
pub(crate) fn dozer_editor_style() -> iced_code_editor::theme::Style {
    use iced_widget::core::Color;
    let bg = theme::color::BG;
    let cyan = theme::color::CYAN;
    iced_code_editor::theme::Style {
        background: bg,
        text_color: theme::color::CREAM,
        gutter_background: theme::color::TERM_BG,
        gutter_border: theme::color::BORDER,
        line_number_color: theme::color::DIM,
        scrollbar_background: bg,
        scroller_color: cyan,
        current_line_highlight: Color {
            r: cyan.r,
            g: cyan.g,
            b: cyan.b,
            a: 0.10,
        },
        whitespace_color: theme::color::DIM,
    }
}

/// 构造一份 ByteBoy2077 的 syntect 语法主题,让语法高亮的 token 颜色
/// (关键字/字符串/注释/类型/函数名……)也融入 Dozer 配色。`iced-code-editor`
/// 上游把 syntect 主题硬编码成 base16-ocean.dark、无公开接口可改;我们
/// vendored 了一份打了 `set_syntax_theme` 补丁的副本(`vendor/iced-code-editor`),
/// 才能把这份主题灌进编辑器。金 `#F2D94E` 仅甲方动作专属,不用于语法着色。
pub(crate) fn dozer_syntax_theme() -> syntect::highlighting::Theme {
    use std::str::FromStr;
    use syntect::highlighting::{Color, ScopeSelectors, StyleModifier, ThemeItem};

    /// `#RRGGBB` -> syntect `Color`(alpha 固定 255)。
    fn c(hex: u32) -> Color {
        Color {
            r: ((hex >> 16) & 0xff) as u8,
            g: ((hex >> 8) & 0xff) as u8,
            b: (hex & 0xff) as u8,
            a: 255,
        }
    }
    /// 单条 scope 着色规则。
    fn scope(s: &str, hex: u32) -> ThemeItem {
        ThemeItem {
            scope: ScopeSelectors::from_str(s).expect("静态 scope 字符串必须合法"),
            style: StyleModifier {
                foreground: Some(c(hex)),
                background: None,
                font_style: None,
            },
        }
    }

    // ByteBoy2077 调色板(值与 `theme::color` 一致,这里用十六进制以便
    // 对齐 syntect 的 u8 颜色)。
    const CREAM: u32 = 0xFFE5B4;
    const BODY: u32 = 0x9AB4C4;
    const DIM: u32 = 0x6B7F8F;
    const CYAN: u32 = 0x47DEF0;
    const GREEN: u32 = 0x1AD585;
    const PURPLE: u32 = 0x9580FF;
    const RED: u32 = 0xFF6E6E;
    const ORANGE: u32 = 0xFF9B4D;
    const BLUE: u32 = 0x4D8CFF;

    syntect::highlighting::Theme {
        name: Some("ByteBoy2077".to_string()),
        author: Some("Dozer".to_string()),
        settings: syntect::highlighting::ThemeSettings {
            foreground: Some(c(CREAM)),
            background: Some(c(0x0a0e16)),
            ..Default::default()
        },
        scopes: vec![
            scope("comment", DIM),
            scope("comment.line", DIM),
            scope("comment.block", DIM),
            scope("string", GREEN),
            scope("string.quoted", GREEN),
            scope("string.regexp", ORANGE),
            scope("constant.numeric", ORANGE),
            scope("constant.language", CYAN),
            scope("constant", ORANGE),
            scope("keyword", CYAN),
            scope("keyword.control", CYAN),
            scope("keyword.operator", BODY),
            scope("keyword.other", CYAN),
            scope("storage", CYAN),
            scope("storage.type", CYAN),
            scope("storage.modifier", CYAN),
            scope("entity.name.function", BLUE),
            scope("entity.name.type", PURPLE),
            scope("entity.name.class", PURPLE),
            scope("entity.name.struct", PURPLE),
            scope("entity.name.enum", PURPLE),
            scope("entity.name.trait", PURPLE),
            scope("entity.name.namespace", BODY),
            scope("entity.name", CREAM),
            scope("entity.name.variable", CREAM),
            scope("variable", CREAM),
            scope("variable.parameter", CREAM),
            scope("variable.language", CYAN),
            scope("support.function", BLUE),
            scope("support.type", PURPLE),
            scope("support.class", PURPLE),
            scope("support.constant", ORANGE),
            scope("support.variable", CREAM),
            scope("punctuation", BODY),
            scope("punctuation.definition", BODY),
            scope("punctuation.separator", BODY),
            scope("punctuation.terminator", BODY),
            scope("meta", CREAM),
            scope("operator", BODY),
            scope("markup.inserted", GREEN),
            scope("markup.deleted", RED),
            scope("markup.changed", ORANGE),
            scope("markup.heading", CYAN),
            scope("markup.bold", CREAM),
            scope("markup.italic", CREAM),
            scope("invalid", RED),
            scope("invalid.deprecated", ORANGE),
            scope("tag", CYAN),
            scope("attribute", ORANGE),
            scope("attribute.name", ORANGE),
            scope("attribute.value", GREEN),
        ],
    }
}

/// 把文件扩展名映射到 `iced-code-editor` 的语法名(即 syntect 扩展名)。
/// 未知扩展名回退 "txt"(plain text),编辑器内部也会再兜底一次。
pub(crate) fn extension_to_syntax(path: &std::path::Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "jsx" => "jsx",
        "tsx" => "tsx",
        "go" => "go",
        "java" => "java",
        "kt" => "kotlin",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "rb" => "ruby",
        "php" => "php",
        "sh" | "bash" | "zsh" => "bash",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "markdown" => "markdown",
        "xml" => "xml",
        "sql" => "sql",
        "diff" => "diff",
        "lua" => "lua",
        "r" => "r",
        "swift" => "swift",
        "zig" => "zig",
        "dockerfile" => "dockerfile",
        "makefile" => "makefile",
        "proto" => "protobuf",
        "graphql" | "gql" => "graphql",
        "ex" | "exs" => "elixir",
        "hs" => "haskell",
        "scala" => "scala",
        _ => "txt",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn open_select_close_tabs() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.md"));
        let id1 = p.open_path(PathBuf::from("/tmp/b.md"));
        assert_eq!(p.tabs().len(), 2);
        assert_eq!(p.active_idx(), 1, "新开 tab 即激活");
        assert_ne!(id0, id1);
        assert_eq!(p.tabs()[0].title, "a.md");
        assert_eq!(p.tabs()[1].title, "b.md");
        p.select(0);
        assert_eq!(p.active_idx(), 0);
        p.close(0);
        assert_eq!(p.tabs().len(), 1);
        assert_eq!(p.active_idx(), 0);
    }

    #[test]
    fn desired_webviews_builds_urls_and_visibility() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/a b.md"));
        p.open_path(PathBuf::from("/tmp/c.md"));
        let specs = p.desired_webviews();
        assert_eq!(specs.len(), 2);
        assert_eq!(
            specs[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa%20b.md&ln=1"
        );
        assert!(!specs[0].visible, "非激活 tab 不可见");
        assert_eq!(
            specs[1].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fc.md&ln=1"
        );
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
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&ln=1&_r=1"
        );
        assert_eq!(
            specs[1].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fb.md&ln=1"
        );
        p.bump_reload(id0);
        assert_eq!(
            p.desired_webviews()[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&ln=1&_r=2"
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
        p.open_path(PathBuf::from("/tmp/b.md")); // 中间插一个,把激活挪走
        assert_eq!(p.active_idx(), 1);
        let id_again = p.open_path(PathBuf::from("/tmp/a.md"));
        assert_eq!(id_again, id0, "同文件复用同一 tab");
        assert_eq!(p.tabs().len(), 2, "不新增 tab");
        assert_eq!(p.active_idx(), 0, "切回已开的那个 tab");
    }

    #[test]
    fn encode_component_is_rfc3986_strict() {
        assert_eq!(encode_component("aZ09-._~"), "aZ09-._~");
        assert_eq!(encode_component("/a b"), "%2Fa%20b");
        assert_eq!(encode_component("你"), "%E4%BD%A0");
    }
}
