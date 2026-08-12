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
pub struct PreviewTab {
    pub id: usize,
    pub kind: TabKind,
    pub title: String,
    /// 保存编辑后 `+1`,驱动 `desired_webviews()` 换 URL 逼 `sync_webview_pool`
    /// 重新 `load_url`(同 URL 不会重载,flyfish 的 WKWebView 会一直显示
    /// 保存前的旧内容)。
    pub reload_nonce: u64,
    /// 仅白名单扩展名(`is_editable_extension`)的文件 tab 有值。非空即代表这个
    /// tab 走原生渲染路径,`desired_webviews()` 据此把它从 wry 期望清单里排除。
    /// `CodeEditor` 没有实现 `Clone`/`PartialEq`,这也是 `PreviewTab` 摘掉这两个
    /// derive 的原因(见下方手写的 `Debug`)。
    pub editor: Option<iced_code_editor::CodeEditor>,
}

impl std::fmt::Debug for PreviewTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewTab")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("title", &self.title)
            .field("reload_nonce", &self.reload_nonce)
            .field("editor", &self.editor.is_some())
            .finish()
    }
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

/// 读盘并按白名单扩展名构造一个只读 `CodeEditor`。内容不是合法 UTF-8 时降级
/// 用 lossy 转换(不当错误);其余读取失败(不存在/权限不够等)原样透传
/// `std::io::Error`,调用方(`push_tab`/`bump_reload`)按现有"打开失败"路径
/// 处理,不在这里新增错误类型。
fn read_and_build_native_editor(
    path: &std::path::Path,
) -> std::io::Result<iced_code_editor::CodeEditor> {
    let text = std::fs::read_to_string(path).or_else(|e| {
        // 白名单扩展名但内容不是合法 UTF-8:降级用 lossy 转换,不当错误处理
        // (多数文本查看器的通行做法,见设计文档"错误处理"一节)。
        if e.kind() == std::io::ErrorKind::InvalidData {
            std::fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        } else {
            Err(e)
        }
    })?;
    let mut editor =
        iced_code_editor::CodeEditor::new(&text, &extension_to_syntax(path)).with_read_only(true);
    editor.set_theme(dozer_editor_style());
    editor.set_syntax_theme(dozer_syntax_theme());
    editor.set_font(crate::fonts::code_font());
    editor.set_font_size(theme::font::body() as f32, false);
    // 打开即夺焦点(同 `preview_edit_open` 的编辑弹层),键盘事件无需先点击
    // 一次即可直达编辑器——否则新开的原生预览 tab 得先点一下才能用方向键
    // 滚动/移动光标。
    editor.request_focus();
    let _ = editor.update(&iced_code_editor::Message::CanvasFocusGained);
    Ok(editor)
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
        let editor = match &kind {
            TabKind::File(path) if is_editable_extension(path) => {
                read_and_build_native_editor(path).ok()
            }
            _ => None,
        };
        self.tabs.push(PreviewTab {
            id,
            kind,
            title,
            reload_nonce: 0,
            editor,
        });
        self.active = self.tabs.len() - 1;
        id
    }

    /// 当前激活 tab 若是文件(webview)则返回其 id(=webview 池的 key)。
    /// 原生渲染 tab(有 `editor`)返回 `None`——它不进 webview 池。
    pub fn active_webview_id(&self) -> Option<usize> {
        self.tabs
            .get(self.active)
            .filter(|t| t.editor.is_none())
            .map(|t| t.id)
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
            .filter(|(_, tab)| tab.editor.is_none())
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

    /// 按 tab id 取该 tab 的原生 editor 可变引用。tab 不存在或该 tab 走 wry
    /// 路径(没有 editor)都返回 `None`。main.rs 的 `Message::PreviewEditorEvent`
    /// 桥接器用它把 `iced_code_editor::Message` 转发给正确的 tab。
    pub fn editor_mut(&mut self, tab_id: usize) -> Option<&mut iced_code_editor::CodeEditor> {
        self.tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.editor.as_mut())
    }

    /// 编辑保存后调用:按 `PreviewTab.id` 找到对应 tab,推进 reload。原生
    /// (有 `editor`)tab 直接读盘重建编辑器实例(`bump_reload` 路径),wry
    /// tab 走 `reload_nonce` 计数(驱动 `desired_webviews()` 换 URL)。未知
    /// id 是 no-op(tab 可能已被关闭)。
    pub fn bump_reload(&mut self, tab_id: usize) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        if tab.editor.is_some() {
            // 原生 tab:只读态没有光标/undo 历史值得跨重建保留,直接读盘换新
            // 实例比"原地更新缓冲区"更简单可靠。读取失败保留旧 editor 不动
            // (比闪成空白/丢内容更安全的降级)。
            let TabKind::File(path) = &tab.kind;
            if let Ok(fresh) = read_and_build_native_editor(path) {
                tab.editor = Some(fresh);
            }
        } else {
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
        scrollbar: iced_code_editor::theme::ScrollbarStyle {
            // 与中央 `scrollbar.rs` 同一套几何/配色:轨道透明无描边、thumb 用
            // `TAB_ACTIVE_BORDER`(#dcc9a3)胶囊,半径 = thumb宽/2。hover 时
            // 朝奶油 `#FFE5B4` 提亮一档,便于在编辑器内看清可拖拽。
            rail_width: theme::geometry::scrollbar_width(),
            thumb_width: theme::geometry::scrollbar_thumb_width(),
            thumb_radius: theme::geometry::scrollbar_thumb_width() / 2.0,
            thumb_color: theme::color::TAB_ACTIVE_BORDER,
            thumb_hover_color: theme::color::mix(
                theme::color::TAB_ACTIVE_BORDER,
                theme::color::CREAM,
                0.35,
            ),
            thumb_border: iced_widget::core::Border::default(),
            track_background: None,
            track_radius: 0.0,
            track_border: iced_widget::core::Border::default(),
        },
        current_line_highlight: Color {
            r: cyan.r,
            g: cyan.g,
            b: cyan.b,
            a: 0.10,
        },
        whitespace_color: theme::color::DIM,
        context_menu: iced_code_editor::theme::ContextMenuStyle {
            // 与文件树/分支切换右键菜单同一套 `context_menu` 区域令牌:
            // bg `#0a0e16` 实底 + `#1c3440` 1px 描边圆角 10 + 内边距 6、列距 2。
            background: bg,
            border_color: theme::color::BORDER,
            border_width: 1.0,
            border_radius: 10.0,
            // Dozer 的右键菜单不投影(region 无 shadow),这里清掉编辑器默认阴影。
            shadow: iced_widget::core::Shadow::default(),
            padding: 6.0,
            gap: 2.0,
            // 固定宽:比文件树菜单项(160)略宽,给"操作名 + 快捷键"两列都留足
            // 空间,不裁剪 ⇧⌘Z 这类长快捷键。取 `context_menu_width`(180)。
            menu_width: theme::geometry::context_menu_width(),
            item_radius: 4.0,
            item_hover_background: theme::color::TAB_HOVER,
            item_text_color: theme::color::CREAM,
            item_disabled_text_color: theme::color::DIM,
            item_padding_h: theme::geometry::menu_pad_h(),
            item_padding_v: theme::geometry::menu_pad_v(),
            item_gap: theme::geometry::menu_gap(),
            separator_color: theme::color::BORDER,
        },
    }
}

/// 给编辑器灌一套与终端同源的排版指标(字号、行高),再乘上全局 scale(
/// `icon_size::scale`)做布局。见 `dozer_editor_style` 上方注释。
///
/// 字距不做处理:编辑器与终端都走 cosmic-text 默认字距,天然一致,显式
/// 加宽反而会引入第二套数据源。
pub(crate) fn dozer_editor_font_metrics(editor: &mut iced_code_editor::CodeEditor) {
    let scale = theme::icon_size::scale();
    let font_size = theme::terminal_font::size() * scale;
    editor.set_font_size(font_size, false);
    let line_height = font_size * theme::terminal_font::line_height_factor();
    editor.set_line_height(line_height);

    // 编辑器的静态布局像素(行号区宽 / 折叠列宽 / 字形顶部内边距)取
    // `ice-code-editor::theme` 的公开默认(基线锚),再乘全局 scale——
    // 这样 Ctrl ± 时编辑器与终端/图标一起等比放大,而非冻结在启动时刻。
    // scale=1 时不漂移。
    editor.set_layout_metrics(
        iced_code_editor::theme::DEFAULT_GUTTER_WIDTH * scale,
        iced_code_editor::theme::DEFAULT_FOLD_MARGIN_WIDTH * scale,
        iced_code_editor::theme::DEFAULT_TOP_PADDING * scale,
    );
}

/// 构造一份 ByteBoy2077 的 syntect 语法主题,让语法高亮的 token 颜色
/// (关键字/字符串/注释/类型/函数名……)融入 Dozer 配色。`iced-code-editor`
/// 上游把 syntect 主题硬编码成 base16-ocean.dark、无公开接口可改;我们
/// vendored 了一份打了 `set_syntax_theme` 补丁的副本(`vendor/iced-code-editor`),
/// 才能把这份主题灌进编辑器。
///
/// 配色唯一真相源是终端 16 色面板(`term_model::ANSI16`)与终端默认前景
/// (`term_model::default_fg_rgb`)——编辑器里展示的语法色因此与终端里
/// 同级角色(字符串/关键字/注释/类型/函数……)观感一致,不会出现"编辑
/// 器一套饱和霓虹、终端一套灰调"的割裂。金 `#F2D94E`(ANSI Yellow)是
/// 甲方动作专属,不用于语法着色——需要"奶油黄"角色时用 BrightYellow。
pub(crate) fn dozer_syntax_theme() -> syntect::highlighting::Theme {
    use std::str::FromStr;
    use syntect::highlighting::{Color, ScopeSelectors, StyleModifier, ThemeItem};

    /// `(r,g,b)` -> syntect `Color`(alpha 固定 255)。
    fn c(rgb: (u8, u8, u8)) -> Color {
        Color {
            r: rgb.0,
            g: rgb.1,
            b: rgb.2,
            a: 255,
        }
    }
    /// 终端 16 色面板第 `idx` 项(下标见 `term_model::ANSI16`)。
    fn ansi(idx: usize) -> (u8, u8, u8) {
        crate::term_model::ansi16_color(idx).expect("ANSI16 静态色表必须完整")
    }
    /// 终端默认前景。
    fn body() -> (u8, u8, u8) {
        crate::term_model::default_fg_rgb()
    }
    /// 单条 scope 着色规则。
    fn scope(s: &str, rgb: (u8, u8, u8)) -> ThemeItem {
        ThemeItem {
            scope: ScopeSelectors::from_str(s).expect("静态 scope 字符串必须合法"),
            style: StyleModifier {
                foreground: Some(c(rgb)),
                background: None,
                font_style: None,
            },
        }
    }

    // 从终端色板取的语法角色(下标即 ANSI16 下标):
    //   1 Red         2 Green       4 Blue(类型/类)    6 Cyan(关键字)
    //   7 White(奶油) 8 BrightBlack(注解/屏弱)            9 BrightRed(删除)
    //  11 BrightYellow(橙/数值/属性)                       12 BrightBlue(函数)
    //  14 BrightCyan / 2 Green(插入)
    const COMMENT: usize = 8; // BrightBlack #6B7F8F
    const CREAM: usize = 7; //  White #FFE5B4
    const GREEN: usize = 2; //  Green #1AD585
    const CYAN: usize = 6; //  Cyan   #47DEF0
    const PURPLE: usize = 4; // Blue   #9580FF
    const RED: usize = 1; //  Red    #FF6E6E
    const ORANGE: usize = 11; // BrightYellow #FFF3B0(非甲方金)
    const FUNCTION: usize = 12; // BrightBlue   #B5A5FF

    syntect::highlighting::Theme {
        name: Some("ByteBoy2077".to_string()),
        author: Some("Dozer".to_string()),
        settings: syntect::highlighting::ThemeSettings {
            foreground: Some(c(ansi(CREAM))),
            background: Some(c((0x0a, 0x0e, 0x16))),
            ..Default::default()
        },
        scopes: vec![
            scope("comment", ansi(COMMENT)),
            scope("comment.line", ansi(COMMENT)),
            scope("comment.block", ansi(COMMENT)),
            scope("string", ansi(GREEN)),
            scope("string.quoted", ansi(GREEN)),
            scope("string.regexp", ansi(ORANGE)),
            scope("constant.numeric", ansi(ORANGE)),
            scope("constant.language", ansi(CYAN)),
            scope("constant", ansi(ORANGE)),
            scope("keyword", ansi(CYAN)),
            scope("keyword.control", ansi(CYAN)),
            scope("keyword.operator", body()),
            scope("keyword.other", ansi(CYAN)),
            scope("storage", ansi(CYAN)),
            scope("storage.type", ansi(CYAN)),
            scope("storage.modifier", ansi(CYAN)),
            scope("entity.name.function", ansi(FUNCTION)),
            scope("entity.name.type", ansi(PURPLE)),
            scope("entity.name.class", ansi(PURPLE)),
            scope("entity.name.struct", ansi(PURPLE)),
            scope("entity.name.enum", ansi(PURPLE)),
            scope("entity.name.trait", ansi(PURPLE)),
            scope("entity.name.namespace", body()),
            scope("entity.name", ansi(CREAM)),
            scope("entity.name.variable", ansi(CREAM)),
            scope("variable", ansi(CREAM)),
            scope("variable.parameter", ansi(CREAM)),
            scope("variable.language", ansi(CYAN)),
            scope("support.function", ansi(FUNCTION)),
            scope("support.type", ansi(PURPLE)),
            scope("support.class", ansi(PURPLE)),
            scope("support.constant", ansi(ORANGE)),
            scope("support.variable", ansi(CREAM)),
            scope("punctuation", body()),
            scope("punctuation.definition", body()),
            scope("punctuation.separator", body()),
            scope("punctuation.terminator", body()),
            scope("meta", ansi(CREAM)),
            scope("operator", body()),
            scope("markup.inserted", ansi(GREEN)),
            scope("markup.deleted", ansi(RED)),
            scope("markup.changed", ansi(ORANGE)),
            scope("markup.heading", ansi(CYAN)),
            scope("markup.bold", ansi(CREAM)),
            scope("markup.italic", ansi(CREAM)),
            scope("invalid", ansi(RED)),
            scope("invalid.deprecated", ansi(ORANGE)),
            scope("tag", ansi(CYAN)),
            scope("attribute", ansi(ORANGE)),
            scope("attribute.name", ansi(ORANGE)),
            scope("attribute.value", ansi(GREEN)),
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
    fn bump_reload_rebuilds_native_editor_without_bumping_nonce() {
        let path =
            std::env::temp_dir().join(format!("preview_reload_test_{}.rs", std::process::id()));
        std::fs::write(&path, "fn one() {}").unwrap();

        let mut p = PreviewPane::default();
        let id = p.open_path(path.clone());
        assert!(p.tabs()[0].editor.is_some());
        let nonce_before = p.tabs()[0].reload_nonce;

        std::fs::write(&path, "fn two() {}").unwrap();
        p.bump_reload(id);

        assert_eq!(
            p.tabs()[0].reload_nonce,
            nonce_before,
            "原生 tab 的 reload 不该走 reload_nonce 计数(那是 wry URL 换参专用信号)"
        );
        assert!(
            p.tabs()[0].editor.is_some(),
            "reload 后原生 tab 应仍持有(重建后的)editor"
        );
        assert_eq!(
            p.tabs()[0].editor.as_ref().unwrap().content(),
            "fn two() {}",
            "原生 tab reload 应读入磁盘上的新内容"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn open_path_builds_native_editor_for_whitelisted_extension_only() {
        let dir = std::env::temp_dir();
        let rs_path = dir.join(format!("preview_native_test_{}.rs", std::process::id()));
        let png_path = dir.join(format!("preview_native_test_{}.png", std::process::id()));
        std::fs::write(&rs_path, "fn main() {}").unwrap();
        std::fs::write(&png_path, [0u8; 4]).unwrap();

        let mut p = PreviewPane::default();
        p.open_path(rs_path.clone());
        p.open_path(png_path.clone());

        assert!(p.tabs()[0].editor.is_some(), ".rs 扩展名应构造原生 editor");
        assert!(
            p.tabs()[1].editor.is_none(),
            ".png 扩展名不应构造原生 editor,继续走 wry"
        );

        let specs = p.desired_webviews();
        assert_eq!(
            specs.len(),
            1,
            "原生 tab 不应出现在 wry 期望清单里,只剩 .png 那个"
        );
        assert_eq!(
            specs[0].url,
            format!(
                "dozer://flyfish/host.html?p={}",
                encode_component(&png_path.to_string_lossy())
            ),
            "剩下的唯一一条 wry 期望清单条目应该是 .png 那个,URL 编码规则同 flyfish_url"
        );

        std::fs::remove_file(&rs_path).ok();
        std::fs::remove_file(&png_path).ok();
    }

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

    /// 防漂移锚:原生预览编辑器的右键菜单必须和 Dozer 文件树/分支切换右键
    /// 菜单同一套 ByteBoy2077 视觉——`#0a0e16` 实底、`#1c3440` 1px 圆角 10
    /// 描边、无投影、`TAB_HOVER` hover 底、`CREAM` 文字、`DIM` 禁用,内边距/
    /// 列距取自 `context_menu` region 令牌。某天有人手滑改掉会在这里炸。
    #[test]
    fn dozer_editor_context_menu_matches_byteboy_style() {
        let m = dozer_editor_style().context_menu;
        assert_eq!(m.background, theme::color::BG);
        assert_eq!(m.border_color, theme::color::BORDER);
        assert_eq!(m.border_width, 1.0);
        assert_eq!(m.border_radius, 10.0);
        assert_eq!(m.shadow, iced_widget::core::Shadow::default());
        assert_eq!(m.padding, 6.0);
        assert_eq!(m.gap, 2.0);
        assert_eq!(m.menu_width, theme::geometry::context_menu_width());
        assert_eq!(m.item_radius, 4.0);
        assert_eq!(m.item_hover_background, theme::color::TAB_HOVER);
        assert_eq!(m.item_text_color, theme::color::CREAM);
        assert_eq!(m.item_disabled_text_color, theme::color::DIM);
        assert_eq!(m.item_padding_h, theme::geometry::menu_pad_h());
        assert_eq!(m.item_padding_v, theme::geometry::menu_pad_v());
        assert_eq!(m.item_gap, theme::geometry::menu_gap());
        assert_eq!(m.separator_color, theme::color::BORDER);
    }

    /// 防漂移锚:编辑器排版指标必须和终端同源——字号 `terminal_font::size()`
    /// 乘全局 scale、行高再乘 `terminal_font::line_height_factor()`,静态布局
    /// 像素(行号区/折叠列/字形内边距)取 vendored 编辑器公开默认再乘 scale。
    #[test]
    fn dozer_editor_font_metrics_match_terminal() {
        let scale = theme::icon_size::scale();
        let mut editor = iced_code_editor::CodeEditor::new("abc", "rs");
        dozer_editor_font_metrics(&mut editor);

        assert_eq!(
            editor.font_size(),
            theme::terminal_font::size() * scale,
            "编辑器字号应与终端同源(terminal_font)"
        );
        assert_eq!(
            editor.line_height(),
            theme::terminal_font::size() * scale * theme::terminal_font::line_height_factor(),
            "编辑器行高应与终端同源(terminal_font::line_height_factor)"
        );
    }

    /// 防漂移锚:编辑器语法高亮的 token 颜色必须锚定到终端 16 色面板与终端
    /// 默认前景,而不是一套独立的十六进制魔数。字符串=Green、关键字=Cyan、
    /// 注释=BrightBlack、类型=Blue、函数=BrightBlue、默认前/后景=终端本色。
    fn syntax_token(theme: &syntect::highlighting::Theme, scope: &str) -> Option<(u8, u8, u8)> {
        use std::str::FromStr;
        let sel =
            syntect::highlighting::ScopeSelectors::from_str(scope).expect("测试 scope 必须合法");
        theme
            .scopes
            .iter()
            .find(|item| item.scope == sel)
            .and_then(|item| item.style.foreground)
            .map(|c| (c.r, c.g, c.b))
    }

    #[test]
    fn dozer_syntax_theme_anchored_to_terminal_palette() {
        let t = dozer_syntax_theme();
        assert_eq!(
            syntax_token(&t, "string").expect("未命中 string"),
            crate::term_model::ansi16_color(2).unwrap(),
            "字符串应锚定终端 Green"
        );
        assert_eq!(
            syntax_token(&t, "keyword").expect("未命中 keyword"),
            crate::term_model::ansi16_color(6).unwrap(),
            "关键字应锚定终端 Cyan"
        );
        assert_eq!(
            syntax_token(&t, "comment").expect("未命中 comment"),
            crate::term_model::ansi16_color(8).unwrap(),
            "注释应锚定终端 BrightBlack"
        );
        assert_eq!(
            syntax_token(&t, "entity.name.type").expect("未命中类型"),
            crate::term_model::ansi16_color(4).unwrap(),
            "类型应锚定终端 Blue"
        );
        assert_eq!(
            syntax_token(&t, "entity.name.function").expect("未命中函数"),
            crate::term_model::ansi16_color(12).unwrap(),
            "函数应锚定终端 BrightBlue"
        );
        assert_eq!(
            syntax_token(&t, "operator").expect("未命中 operator"),
            crate::term_model::default_fg_rgb(),
            "运算符应锚定终端默认前景"
        );
        assert!(
            syntax_token(&t, "string.regexp")
                .map(|(_, g, _)| g)
                .expect("未命中 regexp")
                != 0xd9,
            "regexp 不应使用甲方金 #F2D94E"
        );
    }

    /// 防漂移锚:编辑器滚动条的几何/配色必须与中央 `scrollbar.rs` 的规范一致。
    #[test]
    fn dozer_editor_scrollbar_matches_byteboy_style() {
        let s = dozer_editor_style().scrollbar;
        assert_eq!(s.rail_width, theme::geometry::scrollbar_width());
        assert_eq!(s.thumb_width, theme::geometry::scrollbar_thumb_width());
        assert_eq!(
            s.thumb_radius,
            theme::geometry::scrollbar_thumb_width() / 2.0
        );
        assert_eq!(s.thumb_color, theme::color::TAB_ACTIVE_BORDER);
        assert_eq!(s.track_background, None);
        assert_eq!(
            s.track_border,
            iced_widget::core::Border::default(),
            "轨道应无描边"
        );
        assert_eq!(
            s.thumb_border,
            iced_widget::core::Border::default(),
            "thumb 应无描边"
        );
    }
}
