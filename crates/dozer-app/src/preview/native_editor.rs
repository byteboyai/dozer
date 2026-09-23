//! 文件预览路由用到的扩展名/文件名判定与语法 token 映射。老 iced editor 的
//! 读盘/分档/构造代码已随 Phase D 退役删除;这里只保留路由与语言注册需要
//! 的纯函数。

/// `.gitignore` 这类点开头、`Path::extension()` 认不出扩展名的文件沿用旧
/// 单独特判;`LICENSE`/`Makefile` 等其它**无扩展名**文件由 `filename_code_rule`
/// 注册表(T5)认领。`md/html/svg` 虽命中语法分支返回 `true`,却由
/// `prefers_rendered_preview` 挡住默认预览,默认走渲染。
pub fn is_editable_extension(path: &std::path::Path) -> bool {
    if path.file_name().and_then(|n| n.to_str()) == Some(".gitignore") {
        return true;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // 主判据:高亮器认得出语法 → 代码类 → 原生文本/编辑器预览。
    if extension_to_syntax(path) != "txt" {
        return true;
    }
    // 兜底:没有语法映射但确实是纯文档/配置文本的扩展名。
    matches!(
        ext.as_str(),
        "txt" | "log" | "conf" | "cfg" | "ini" | "csv" | "tsv"
    )
}

/// 默认预览要不要走 flyfish 渲染而不是文本:`.md`/`.markdown`/`.html`/`.htm`
/// 以及 `.svg`(T5:SVG 默认图像渲染,另提供 CodeMirror XML 源码模式)。这只
/// 决定**默认预览**走渲染效果;用户切到源码后仍是可写文本(CodeMirror)。
pub(crate) fn prefers_rendered_preview(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "md" | "markdown" | "html" | "htm" | "svg"
    )
}

/// 文件名注册表(T5):优先于扩展名 fallback 的、按**文件名**认领的文本类型。
/// 返回 `(规则名, language token)`——规则名进 `RouteReason` 供诊断,语言 token
/// 交给 CodeMirror。仅覆盖计划点名的少量稳定文件名,不做"所有 dotfile 都是
/// 文本"的宽泛规则;调用方仍须用内容画像(含 NUL/二进制)做安全检查。
pub(crate) fn filename_code_rule(path: &std::path::Path) -> Option<(&'static str, &'static str)> {
    let name = path.file_name()?.to_str()?;
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "dockerfile" => return Some(("dockerfile", "dockerfile")),
        "makefile" | "gnumakefile" => return Some(("makefile", "makefile")),
        ".gitignore" | ".dockerignore" => return Some(("ignore", "txt")),
        // `.env` 及明确允许的变体(`.env.local`/`.env.development`…)。
        _ if lower == ".env" || lower.starts_with(".env.") => {
            return Some(("env", "txt"));
        }
        _ => {}
    }
    if lower.starts_with("license") || lower.starts_with("notice") {
        return Some(("license", "txt"));
    }
    None
}

/// JSON 家族扩展名(严格 json 与 json5/jsonc/jsonl/ndjson)。路由据此归入
/// `PreviewKind::Json`;其中严格 `.json` 给 Tree/Text 双视图,其余只给文本。
pub(crate) fn is_json_family_extension(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "json" | "json5" | "jsonc" | "jsonl" | "ndjson"
    )
}

/// tab 上"预览/代码"切换按钮该不该出现:只对"文本可编辑、但默认走渲染"的
/// 文件出现(目前即 `.md`/`.markdown`/`.html`/`.htm`)。
pub fn wry_toggle_eligible(path: &std::path::Path) -> bool {
    is_editable_extension(path) && prefers_rendered_preview(path)
}

/// 把文件扩展名映射到语言 token(前端 CodeMirror / JSON host 用来加载语言)。
/// 未知扩展名回退 "txt"(plain text)。前端 `web/editor/src/languages.ts` 与
/// 这份映射**一一对应**,改动需同步。
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
        // JSON Lines / NDJSON:每行是 JSON 文本,文本视图按 json 高亮更贴近。
        "jsonl" | "ndjson" => "json",
        "jsonc" | "json5" => "jsonc",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "markdown" => "markdown",
        "xml" => "xml",
        // SVG 是 XML 家族;默认走图像渲染,源码模式用 XML 高亮。
        "svg" => "xml",
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
