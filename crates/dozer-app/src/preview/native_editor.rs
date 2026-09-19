//! 可编辑原生编辑器:读盘构造 CodeView、可编辑扩展名判定、wry 切换资格、
//! syntect 语法主题与扩展名映射。

/// 低于此值:全功能编辑(含 undo/保存)。固定值,不随机器内存缩放——这一档
/// 的瓶颈是 undo 栈本身的设计(`code_editor::EDIT_HISTORY_LIMIT` 份整文件
/// `String` 快照),不是单次读取的内存代价。见
/// `docs/superpowers/specs/2026-09-19-large-file-editor-performance-design.md`
/// "分档策略与阈值"。
pub(crate) const EDIT_MODE_MAX_BYTES: u64 = 20 * 1024 * 1024;

/// 按机器可用内存动态算"只读·整读"档上限(纯函数,供 [`full_load_max_bytes`]
/// 与单测复用):总内存 10% ÷ 3(读取+校验临时拷贝+常驻拷贝的峰值安全边际),
/// 钳到 [256MB, 4GB]。
pub(crate) fn full_load_max_bytes_for(total_ram_bytes: u64) -> u64 {
    ((total_ram_bytes as f64 * 0.10 / 3.0) as u64).clamp(256 * 1024 * 1024, 4 * 1024 * 1024 * 1024)
}

/// 查询系统总内存并套 [`full_load_max_bytes_for`]。查询失败(极端环境)时
/// 退化为 512MB 默认值,不 panic、不阻塞打开流程。
pub(crate) fn full_load_max_bytes() -> u64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let total = sys.total_memory();
    if total == 0 {
        return 512 * 1024 * 1024;
    }
    full_load_max_bytes_for(total)
}

/// 三档分类结果。
pub(crate) enum SizeTier {
    /// < `EDIT_MODE_MAX_BYTES`:全功能编辑。
    Edit,
    /// [`EDIT_MODE_MAX_BYTES`, full_load_max):只读,整文件读入内存。
    FullLoadReadOnly,
    /// >= full_load_max:只读,首屏只读入 full_load_max 字节,可"加载更多"。
    ChunkedReadOnly,
}

/// 按文件大小(`len`)与本次打开时算好的整读上限(`full_load_max`,调用方
/// 传入而非在这里现查,避免每次分类都重复查一次系统内存)分类。
pub(crate) fn classify_size(len: u64, full_load_max: u64) -> SizeTier {
    if len < EDIT_MODE_MAX_BYTES {
        SizeTier::Edit
    } else if len < full_load_max {
        SizeTier::FullLoadReadOnly
    } else {
        SizeTier::ChunkedReadOnly
    }
}

/// 在 `bytes` 里找不超过 `max_len` 的最大合法 UTF-8 前缀长度——分块读取截断
/// 点若落在多字节字符中间,回退到该字符起点之前,避免产生非法 UTF-8 或
/// 半个字符。`max_len` 超过 `bytes.len()` 时钳到 `bytes.len()`。
pub(crate) fn utf8_safe_prefix_len(bytes: &[u8], max_len: usize) -> usize {
    let max_len = max_len.min(bytes.len());
    match std::str::from_utf8(&bytes[..max_len]) {
        Ok(_) => max_len,
        Err(e) => e.valid_up_to(),
    }
}

/// 读盘 + 三档分类的纯数据结果(不含 `CodeView`)。`#[derive(Debug, Clone)]`
/// ——专为跨 `Message`/线程边界传递设计(`CodeView` 没有实现 `Clone`,而
/// `Message` enum 整体 `#[derive(Debug, Clone)]`,见 Task 3):异步读盘任务
/// 只做 I/O 与分类,`CodeView::new`(全局 `font_system` 锁,纯 CPU 计算,
/// 已验证是 `Send`、不要求在"拥有窗口/事件循环"的线程上做,见 Task 1"对
/// Task 3 的影响")留给 Task 3 的后台任务在同一个 `spawn_blocking` 里现场
/// 构造。
#[derive(Debug, Clone)]
pub(crate) struct NativeFileData {
    pub text: String,
    pub syntax_token: String,
    pub read_only: bool,
    pub loaded_bytes: u64,
    pub total_bytes: u64,
    pub truncated: bool,
}

/// 读盘并按三档策略产出 [`NativeFileData`]:
/// - `SizeTier::Edit`(< 20MB):全量读入,可写(`read_only=false`),语义同
///   2026-09-06 起的"原生预览默认可编辑"。
/// - `SizeTier::FullLoadReadOnly`:全量读入,但 `read_only=true`。
/// - `SizeTier::ChunkedReadOnly`:只读入前 `full_load_max` 字节(按合法 UTF-8
///   边界截断),`read_only=true`,`truncated=true`。
///
/// 内容不是合法 UTF-8 时降级用 lossy 转换(不当错误);其余读取失败(不存在/
/// 权限不够等)原样透传 `std::io::Error`。
pub(crate) fn read_native_file_data(path: &std::path::Path) -> std::io::Result<NativeFileData> {
    let total_bytes = std::fs::metadata(path)?.len();
    let full_load_max = full_load_max_bytes();
    let tier = classify_size(total_bytes, full_load_max);

    let (raw, loaded_bytes, truncated) = match tier {
        SizeTier::Edit | SizeTier::FullLoadReadOnly => {
            let bytes = std::fs::read(path)?;
            let len = bytes.len() as u64;
            (bytes, len, false)
        }
        SizeTier::ChunkedReadOnly => {
            use std::io::Read;
            let mut file = std::fs::File::open(path)?;
            let cap = full_load_max as usize;
            let mut buf = vec![0u8; cap];
            let mut read_total = 0usize;
            while read_total < cap {
                let n = file.read(&mut buf[read_total..])?;
                if n == 0 {
                    break;
                }
                read_total += n;
            }
            buf.truncate(read_total);
            let safe_len = utf8_safe_prefix_len(&buf, buf.len());
            buf.truncate(safe_len);
            let len = buf.len() as u64;
            (buf, len, true)
        }
    };

    let text = String::from_utf8(raw)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    let read_only = !matches!(tier, SizeTier::Edit);
    Ok(NativeFileData {
        text,
        syntax_token: extension_to_syntax(path),
        read_only,
        loaded_bytes,
        total_bytes,
        truncated,
    })
}

/// 读盘 + 构造好的 `CodeView` 打包在一起。两个来源:`push_tab`/
/// `bump_reload`/`enter_code_mode` 的**同步**路径(已知不大的场景:测试
/// fixture、已打开文件保存后刷新、预览↔代码切换只对 `wry_toggle_eligible`
/// 的 md/html 生效)直接拿 [`read_and_build_native_editor`] 的返回值;
/// `App::preview_open_path`/`project_preview_open_path` 的**异步**路径
/// (Task 3,用户从文件树打开任意大小文件走这条)在 `spawn_blocking` 里构造
/// 好之后,包一层 [`NativeEditorLoadHandle`] 跨 `Message` 边界传回——见该
/// 类型文档"为什么不能直接放进 `Message`"。
pub(crate) struct NativeEditorLoad {
    pub view: crate::code_editor::CodeView,
    pub loaded_bytes: u64,
    pub total_bytes: u64,
    pub truncated: bool,
}

/// 读盘并构造 `CodeView`(阻塞调用方线程——同步路径直接在调用方线程做;
/// 异步路径在 `spawn_blocking` 的后台线程里做,见上方 [`NativeEditorLoad`]
/// 文档)。内容不是合法 UTF-8 时降级用 lossy 转换;其余读取失败原样透传
/// `std::io::Error`。
///
/// 打开即程序化聚焦(键盘事件无需先点击一次即可直达编辑器)这件事挪到
/// `push_tab`/`PreviewPane::apply_native_load` 里置一次性 `pending_focus`
/// 位——官方 `text_editor` 的焦点是真实 iced 焦点树的一部分,不能像
/// vendored 版本那样在构造时直接 `request_focus()` 拿到。
pub(crate) fn read_and_build_native_editor(
    path: &std::path::Path,
) -> std::io::Result<NativeEditorLoad> {
    let data = read_native_file_data(path)?;
    let view = crate::code_editor::CodeView::new(&data.text, data.syntax_token, data.read_only);
    Ok(NativeEditorLoad {
        view,
        loaded_bytes: data.loaded_bytes,
        total_bytes: data.total_bytes,
        truncated: data.truncated,
    })
}

/// 跨 `Message` 边界传递一次性构造好的 [`NativeEditorLoad`]。`CodeView`
/// 没有实现 `Clone`(撤销栈/`Content` 都不必要求 `Clone`),而 `Message`
/// 整体 `#[derive(Debug, Clone)]`,所有变体的字段都要满足这两个 trait——
/// 见 Task 1"对 Task 3 的影响":`CodeView`/`Content` 已验证是 `Send`,不要求
/// 在"拥有窗口/事件循环"的线程上构造,所以 `spawn_blocking` 的后台任务可以
/// 直接把 `CodeView::new` 一起做了,不必只传纯数据回主线程现场构造(那样
/// 反而会把真正耗时的 shaping 挪回 UI 线程,违背异步化的本意)。
/// `Arc<Mutex<Option<_>>>` 本身廉价 `Clone`(只是引用计数 + 一次判空锁),
/// 接收端 [`NativeEditorLoadHandle::take`] 精确取出一次;`Debug` 手写为占位
/// (不下探锁内容,同 `PreviewTab` 对不可 `Debug` 字段的既有处理方式)。
#[derive(Clone)]
pub(crate) struct NativeEditorLoadHandle(
    std::sync::Arc<std::sync::Mutex<Option<NativeEditorLoad>>>,
);

impl std::fmt::Debug for NativeEditorLoadHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeEditorLoadHandle")
            .finish_non_exhaustive()
    }
}

impl NativeEditorLoadHandle {
    pub(crate) fn new(load: NativeEditorLoad) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(Some(load))))
    }

    /// 取出内部的 `NativeEditorLoad`;正常只应被调用一次(`apply_native_load`
    /// 收到结果时取走),第二次调用返回 `None`。锁中毒(持锁线程 panic)时
    /// 同样按 `None` 处理——不让这里的 panic 传播炸掉 `update()`。
    pub(crate) fn take(&self) -> Option<NativeEditorLoad> {
        self.0.lock().ok().and_then(|mut guard| guard.take())
    }
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

/// "加载更多"续读:从字节偏移 `start` 起读最多 `max_extra` 字节(按合法
/// UTF-8 边界截断),返回 `(续读到的文本, 新的已加载字节数, 是否仍被截断)`。
/// `truncated` 语义:`start + 实际读到的字节数 < 文件总大小` 则仍为真。
pub(crate) fn read_more_bytes(
    path: &std::path::Path,
    start: u64,
    max_extra: u64,
) -> std::io::Result<(String, u64, bool)> {
    use std::io::{Read, Seek, SeekFrom};
    let total_bytes = std::fs::metadata(path)?.len();
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let cap = max_extra as usize;
    let mut buf = vec![0u8; cap];
    let mut read_total = 0usize;
    while read_total < cap {
        let n = file.read(&mut buf[read_total..])?;
        if n == 0 {
            break;
        }
        read_total += n;
    }
    buf.truncate(read_total);
    let safe_len = utf8_safe_prefix_len(&buf, buf.len());
    buf.truncate(safe_len);
    let new_loaded = start + buf.len() as u64;
    let text = String::from_utf8(buf)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    let truncated = new_loaded < total_bytes;
    Ok((text, new_loaded, truncated))
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

#[cfg(test)]
mod size_tier_tests {
    use super::*;

    #[test]
    fn full_load_max_clamps_to_floor_on_small_machines() {
        // 4GB 机器:4×0.10/3 ≈ 137MB,应钳到 256MB 下限。
        let max = full_load_max_bytes_for(4 * 1024 * 1024 * 1024);
        assert_eq!(max, 256 * 1024 * 1024);
    }

    #[test]
    fn full_load_max_clamps_to_ceiling_on_huge_machines() {
        // 128GB 机器:128×0.10/3 ≈ 4.27GB,应钳到 4GB 上限。
        let max = full_load_max_bytes_for(128 * 1024 * 1024 * 1024);
        assert_eq!(max, 4 * 1024 * 1024 * 1024);
    }

    #[test]
    fn full_load_max_scales_between_clamps() {
        // 32GB 机器:32×0.10/3 ≈ 1.0667GB,应落在钳位区间内、非两端。
        let max = full_load_max_bytes_for(32 * 1024 * 1024 * 1024);
        assert!(max > 256 * 1024 * 1024 && max < 4 * 1024 * 1024 * 1024);
        // 32×1024³×0.10/3 = 1_145_324_612.26…,`as u64` 截断取整。
        assert_eq!(max, 1_145_324_612);
    }

    #[test]
    fn classify_size_boundaries() {
        let full_max = 1_000_000_000u64;
        assert!(matches!(classify_size(0, full_max), SizeTier::Edit));
        assert!(matches!(
            classify_size(EDIT_MODE_MAX_BYTES - 1, full_max),
            SizeTier::Edit
        ));
        assert!(matches!(
            classify_size(EDIT_MODE_MAX_BYTES, full_max),
            SizeTier::FullLoadReadOnly
        ));
        assert!(matches!(
            classify_size(full_max - 1, full_max),
            SizeTier::FullLoadReadOnly
        ));
        assert!(matches!(
            classify_size(full_max, full_max),
            SizeTier::ChunkedReadOnly
        ));
    }

    #[test]
    fn utf8_safe_prefix_len_trims_incomplete_multibyte_tail() {
        // "中" 是 3 字节 UTF-8(E4 B8 AD)。截在第 1、2 字节处都应回退到
        // 该字符起点之前;截在第 3 字节(字符完整)处应保留整个字符。
        let text = "ab中cd";
        let bytes = text.as_bytes();
        assert_eq!(utf8_safe_prefix_len(bytes, 2), 2);
        assert_eq!(utf8_safe_prefix_len(bytes, 3), 2);
        assert_eq!(utf8_safe_prefix_len(bytes, 4), 2);
        assert_eq!(utf8_safe_prefix_len(bytes, 5), 5);
        assert_eq!(utf8_safe_prefix_len(bytes, 100), bytes.len());
    }

    #[test]
    fn read_native_file_data_edit_tier_is_writable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let data = read_native_file_data(&path).unwrap();
        assert!(!data.read_only);
        assert!(!data.truncated);
        assert_eq!(data.total_bytes, data.loaded_bytes);
        assert_eq!(data.text, "fn main() {}\n");
        assert_eq!(data.syntax_token, "rust");
    }

    #[test]
    fn read_more_bytes_continues_from_offset_and_respects_utf8_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        // "中" 在字节 8..11(前 8 字节是 "aaaaaaaa"),故意把续读窗口卡在
        // 这个多字节字符中间,验证不产生半个字符。
        std::fs::write(&path, "aaaaaaaa中bbbbbbbb").unwrap();
        let (first, loaded1, truncated1) = read_more_bytes(&path, 0, 9).unwrap();
        assert_eq!(first, "aaaaaaaa");
        assert_eq!(loaded1, 8);
        assert!(truncated1);

        let (second, loaded2, truncated2) = read_more_bytes(&path, loaded1, 100).unwrap();
        assert_eq!(second, "中bbbbbbbb");
        assert!(!truncated2);
        assert_eq!(loaded2, std::fs::metadata(&path).unwrap().len());
    }
}
