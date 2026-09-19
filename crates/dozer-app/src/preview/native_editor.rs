//! 可编辑原生编辑器:读盘构造 CodeView、可编辑扩展名判定、wry 切换资格、
//! syntect 语法主题与扩展名映射。

/// 原生可写编辑器能吃得下的文件大小上限(字节)。超过则退回只读 flyfish
/// 预览——`Editor::with_text` 会对整份文档做一次 `Shaping::Advanced`
/// 预整形(cosmic-text `set_text` → `shape_until_scroll`,buffer 尚无尺寸时
/// 会整形**全部行**),大文件(尤其含 CJK,实测 ~14ms/KB,267KB ≈ 3.5s、7.5MB
/// ≈ 98s)会阻塞 UI 线程数秒到数十秒,表现为「打开即卡死/未响应」。只读
/// 预览由 webview 独立进程渲染,不占 UI 线程,与「预览优先于编辑」的裁决一致。
pub(crate) const MAX_NATIVE_EDITOR_BYTES: u64 = 256 * 1024;

/// 文件是否超过原生编辑器的可载入上限(只看 `fs::metadata` 的 `len`,
/// 不读内容;拿不到元数据按「未超限」处理,交给后续真正的读盘去报错)。
pub(crate) fn exceeds_native_editor_limit(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.len() > MAX_NATIVE_EDITOR_BYTES)
        .unwrap_or(false)
}

/// 读盘并按白名单扩展名构造一个**可写** `CodeView`(2026-09-06 起原生文本预览
/// 不再只读:用户可直接拖选/复制/就地编辑,配合 `Workspace` 侧的脏标记与
/// `preview_pane_save` ⌘S 落盘——见 `docs/superpowers/plans/2026-09-06-*.md`）。
/// 内容不是合法 UTF-8 时降级用 lossy 转换(不当错误);其余读取失败(不存在/权限
/// 不够等)原样透传 `std::io::Error`,调用方(`push_tab`/`bump_reload`)按现有
/// "打开失败"路径处理,不在这里新增错误类型。
///
/// 超过 [`MAX_NATIVE_EDITOR_BYTES`] 的文件直接返回 Err(在真正读盘前就拦下),
/// 让 `push_tab` 的 `.ok()` 落到 `None` → 该 tab 走 wry 只读 flyfish 预览,
/// 避免把 UI 线程卡死在整文档预整形上。
///
/// 打开即程序化聚焦(键盘事件无需先点击一次即可直达编辑器)这件事挪到
/// `push_tab` 里置一次性 `pending_focus` 位——官方 `text_editor` 的焦点是
/// 真实 iced 焦点树的一部分,不能像 vendored 版本那样在构造时直接
/// `request_focus()` 拿到。
pub(crate) fn read_and_build_native_editor(
    path: &std::path::Path,
) -> std::io::Result<crate::code_editor::CodeView> {
    if exceeds_native_editor_limit(path) {
        return Err(std::io::Error::other(
            "file exceeds native editor size limit",
        ));
    }
    let text = std::fs::read_to_string(path).or_else(|e| {
        // 白名单扩展名但内容不是合法 UTF-8:降级用 lossy 转换,不当错误处理
        // (多数文本查看器的通行做法,见设计文档"错误处理"一节)。
        if e.kind() == std::io::ErrorKind::InvalidData {
            std::fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        } else {
            Err(e)
        }
    })?;
    Ok(crate::code_editor::CodeView::new(
        &text,
        extension_to_syntax(path),
        false,
    ))
}

/// "编辑/原生 code editor 预览"的适用范围。判定规则单一来源 = 语法能力:
/// `extension_to_syntax` 能给出具体语法(syntact 语法高亮)的源码类扩展名,
/// 一律收敛进原生 text editor 预览(不再落到 flyfish 只当纯文本/兜底);
/// 再加上少数"没有语法、但纯文本、进原生一样能看"的兜底扩展名。这样
/// `is_editable_extension` 与高亮器认识的语言集保持一致,不会出现"文件能高亮
/// 却一开始就进不了编辑器"的脱节(2026-09-05 用户:js/json 等代码类的文件
/// 都应由 text editor 预览)。
///
/// `.gitignore` 这类点开头、`Path::extension()` 认不出扩展名的文件沿用旧
/// 单独特判;`LICENSE`/`Makefile` 等其它**无扩展名**文件刻意不进(既有约定,
/// 见 `is_editable_extension_rejects_unknown_and_binary_like`)。`md/html` 虽
/// 命中语法分支返回 `true`,却由 `prefers_rendered_preview` 挡住默认预览(见
/// 其文档),默认走渲染、tab 上出现预览/代码切换按钮(`wry_toggle_eligible`,
/// 见 [`wry_toggle_eligible`])可一键切回可写原生编辑器;图片/PDF/二进制扩展名
/// 高亮器不认识、又不在纯文本兜底集,照旧交给 flyfish。
pub fn is_editable_extension(path: &std::path::Path) -> bool {
    if path.file_name().and_then(|n| n.to_str()) == Some(".gitignore") {
        return true;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // 主判据:高亮器认得出语法 → 代码类 → 原生文本/编辑器预览。顺带把
    // .md/.html 等渲染型也归为"可编辑"(右键能编辑),跟前几版行为一致。
    if extension_to_syntax(path) != "txt" {
        return true;
    }
    // 兜底:没有语法映射但确实是纯文档/配置文本的扩展名,原生预览优于 flyfish。
    matches!(
        ext.as_str(),
        "txt" | "log" | "conf" | "cfg" | "ini" | "csv" | "tsv"
    )
}

/// 默认预览要不要走 flyfish 渲染而不是原生代码编辑器:目前只有
/// .md/.markdown——flyfish 内置的 markdown 渲染器能出标题/粗体/列表/代码块
/// 排版效果(GitHub 风格 `.markdown-body`),原生编辑器只能给纯文本+语法
/// 高亮,看不出排版。这只决定**默认预览**走渲染效果;用户切到可写原生 tab
/// 后仍是 `.txt` 一类的就地编辑器,不依赖这个默认走不走的判定。
pub(crate) fn prefers_rendered_preview(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "md" | "markdown" | "html" | "htm"
    )
}

/// tab 上"预览/代码"切换按钮该不该出现的判定:只对"文本可编辑、但默认走
/// wry/flyfish 渲染"的文件出现(目前即 `.md`/`.markdown`/`.html`/`.htm`)。
/// 图片/PDF/压缩包等真二进制文件(`is_editable_extension` 为假)不出现;
/// 本来就默认原生可编辑的其它文本文件(`!prefers_rendered_preview`,如
/// `.rs`/`.py`/`.json`)也不出现——它们从来不是 wry 打开的,永远原生。
pub fn wry_toggle_eligible(path: &std::path::Path) -> bool {
    is_editable_extension(path) && prefers_rendered_preview(path)
}

/// 构造一份 ByteBoy2077 的 syntect 语法主题,让语法高亮的 token 颜色
/// (关键字/字符串/注释/类型/函数名……)融入 Dozer 配色。喂给
/// `code_editor::highlighter::Highlighter`(自实现的 `text::Highlighter`,
/// 不用 `iced_highlighter` 自带的 5 个内置主题——那是个封闭枚举,没有
/// "传入任意 syntect Theme" 的公开口子)。
///
/// 编辑器 chrome(背景/文本/选区色)对齐 ByteBoy2077 配色的逻辑挪到
/// `code_editor::editor_style`——官方 `text_editor::Style` 字段比这份
/// syntect 主题简单得多,没有 gutter/滚动条/右键菜单的概念(那些 UI 官方
/// widget 本来就不画,gutter 是 `code_editor` 自建的 canvas)。
///
/// 配色唯一真相源是终端 16 色面板(`term_model::ANSI16`)与终端默认前景
/// (`term_model::default_fg_rgb`)——编辑器里展示的语法色因此与终端里
/// 同级角色(字符串/关键字/注释/类型/函数……)观感一致,不会出现"编辑
/// 器一套饱和霓虹、终端一套灰调"的割裂。金 `#F2D94E`(ANSI Yellow)是
/// 甲方动作专属,不用于语法着色——需要"奶油黄"角色时用 BrightYellow。
///
/// `scheme` 显式传入而非读全局 `current_scheme()`:浅/深两份主题由调用方
/// (见 `code_editor::highlighter` 的两个 `LazyLock`)各自按方案预计算并
/// 缓存,缓存时机与当时的全局方案无关,因此换主题不会错色,也不会因为
/// "第一次打开编辑器时恰好是深色"就把浅色主题永久冻成深色。终端色板与
/// 默认前景本身早已支持双方案(`term_model::ansi16_color_for`/
/// `default_fg_rgb_for`),这里只是把"取哪个方案的表"变成入参。
pub(crate) fn dozer_syntax_theme(
    scheme: byteui::theme::color::ColorScheme,
) -> syntect::highlighting::Theme {
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
    // 用闭包而非 fn:要捕获本次构造的 `scheme`(fn item 不能捕获环境)。
    // `scheme` 下终端 16 色面板第 `idx` 项(下标见 `term_model::ANSI16`)。
    let ansi = |idx: usize| {
        crate::term::term_model::ansi16_color_for(scheme, idx).expect("ANSI16 静态色表必须完整")
    };
    // `scheme` 下终端默认前景。
    let body = || crate::term::term_model::default_fg_rgb_for(scheme);
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

    // 背景留空:编辑器不画自己的底色(见 `code_editor::editor_style` 的
    // 透明背景),面板底色透上来,语法主题只负责文字前景色。`to_format`
    // 本来就只暴露前景/字重,这里填什么都进不了渲染,留 `None` 最诚实——
    // "背景归面板管,编辑器不持有背景"。
    syntect::highlighting::Theme {
        name: Some(format!("ByteBoy2077-{scheme:?}")),
        author: Some("Dozer".to_string()),
        settings: syntect::highlighting::ThemeSettings {
            foreground: Some(c(ansi(CREAM))),
            background: None,
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
        // JSON5 是 JSON 超集(允许注释、尾逗号、不加引号的键),交给 vendored
        // 的 `JSONC` 语法高亮(见 `code_editor::highlighter` 的 EXTRA_SYNTAXES_DIR),
        // 这样 `//` / `/* */` 注释与尾逗号都能正确着色;`.jsonc` 同理。
        // 严格 `.json` 仍走 bundle 自带的 `JSON` 语法(不认注释,符合 JSON 规范)。
        "json" => "json",
        "jsonc" | "json5" => "jsonc",
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
