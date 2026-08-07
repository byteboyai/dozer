//! Lucide 图标(MIT/ISC，见 `assets/icons/LICENSE`)编译期内嵌 + 统一渲染入口。
//! `IconKind` 是穷举枚举而非开放式字符串——新增图标 = 加一个变体 + 一个 svg 文件，
//! 与 `theme.rs` 精选 14 色而非任意色值同一哲学。

use iced_widget::core::{Color, Element, Length};
use iced_widget::svg;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    Folder,
    FolderOpen,
    FileCode,
    FileJson,
    FileText,
    FileConfig,
    FileImage,
    FileGeneric,
    Search,
    Settings,
    FilePlus,
    FolderPlus,
    Copy,
    ClipboardPaste,
    Trash,
    Rename,
    Globe,
    Bot,
    MessageSquare,
    /// 内容 pane 放大/还原(`MaximizedPane`)的触发图标。触发按钮已在
    /// `a0d324e`(2026-08-06)被主动移除,状态机/overlay 仍保留,是否重新
    /// 接一个入口留给后续产品决策——见
    /// `docs/superpowers/plans/2026-08-07-dozer-milestone-summary-and-plan-audit.md`。
    #[allow(dead_code)]
    Maximize,
    Home,
    RefreshCw,
    SquarePlus,
    GitBranch,
    ListChecks,
    /// Agent 用量统计面板的图标(Lucide bar-chart-3)。
    BarChart3,
    /// 终端/Shell(纯 Shell 启动项),Lucide。
    Terminal,
    /// Dozer 品牌标(dozer-logo-main.jpeg → potrace 矢量化),
    /// 只用于顶栏"Dozer"页签,与其他 Lucide 图标同款通过 `currentColor` 着色。
    Dozer,
    /// Agent 品牌标(着色用 `currentColor`,由调用方按 agent 指定主题色)。
    /// 来源:Claude/CodeBuddy 取自 Simple Icons,OpenCode 取自其官网 favicon 并
    /// 归一化到 24×24。均为品牌标识,非 Lucide;仅供 Agent 身份识别。
    Claude,
    Codebuddy,
    Opencode,
}

impl IconKind {
    fn bytes(self) -> &'static [u8] {
        match self {
            IconKind::ChevronLeft => include_bytes!("../assets/icons/chevron-left.svg"),
            IconKind::ChevronRight => include_bytes!("../assets/icons/chevron-right.svg"),
            IconKind::ChevronDown => include_bytes!("../assets/icons/chevron-down.svg"),
            IconKind::Folder => include_bytes!("../assets/icons/folder.svg"),
            IconKind::FolderOpen => include_bytes!("../assets/icons/folder-open.svg"),
            IconKind::FileCode => include_bytes!("../assets/icons/file-code.svg"),
            IconKind::FileJson => include_bytes!("../assets/icons/braces.svg"),
            IconKind::FileText => include_bytes!("../assets/icons/file-text.svg"),
            IconKind::FileConfig => include_bytes!("../assets/icons/file-cog.svg"),
            IconKind::FileImage => include_bytes!("../assets/icons/file-image.svg"),
            IconKind::FileGeneric => include_bytes!("../assets/icons/file.svg"),
            IconKind::Search => include_bytes!("../assets/icons/search.svg"),
            IconKind::Settings => include_bytes!("../assets/icons/settings.svg"),
            IconKind::FilePlus => include_bytes!("../assets/icons/file-plus.svg"),
            IconKind::FolderPlus => include_bytes!("../assets/icons/folder-plus.svg"),
            IconKind::Copy => include_bytes!("../assets/icons/copy.svg"),
            IconKind::ClipboardPaste => include_bytes!("../assets/icons/clipboard-paste.svg"),
            IconKind::Trash => include_bytes!("../assets/icons/trash-2.svg"),
            IconKind::Rename => include_bytes!("../assets/icons/pen-line.svg"),
            IconKind::Globe => include_bytes!("../assets/icons/globe.svg"),
            IconKind::Bot => include_bytes!("../assets/icons/bot.svg"),
            IconKind::MessageSquare => include_bytes!("../assets/icons/message-square.svg"),
            IconKind::Maximize => include_bytes!("../assets/icons/maximize-2.svg"),
            IconKind::Home => include_bytes!("../assets/icons/home.svg"),
            IconKind::RefreshCw => include_bytes!("../assets/icons/refresh-cw.svg"),
            IconKind::SquarePlus => include_bytes!("../assets/icons/square-plus.svg"),
            IconKind::GitBranch => include_bytes!("../assets/icons/git-branch.svg"),
            IconKind::ListChecks => include_bytes!("../assets/icons/list-checks.svg"),
            IconKind::BarChart3 => include_bytes!("../assets/icons/bar-chart-3.svg"),
            IconKind::Terminal => include_bytes!("../assets/icons/terminal.svg"),
            IconKind::Dozer => include_bytes!("../assets/icons/dozer-logo.svg"),
            IconKind::Claude => include_bytes!("../assets/icons/claude.svg"),
            IconKind::Codebuddy => include_bytes!("../assets/icons/codebuddy.svg"),
            IconKind::Opencode => include_bytes!("../assets/icons/opencode.svg"),
        }
    }
}

/// 统一图标渲染入口:调用方传主题色，用法与 `text().color(...)` 一致。
/// 泛型于 `Message`(图标本身不接受点击，任何 `Message` 类型都能塞进去)。
pub fn view<'a, Message: 'a>(
    kind: IconKind,
    size: f32,
    color: Color,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    // 调用方传入的 `size` 应为 `icon_size` 的 token（已含全局 scale），
    // 本函数是纯渲染入口，不再二次乘 scale。
    svg(svg::Handle::from_memory(kind.bytes()))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_theme: &iced_widget::Theme, _status| svg::Style { color: Some(color) })
        .into()
}

/// 文件名 → 图标类别(按扩展名，类别式而非按语言品牌，见设计文档 §3.1 caveat；
/// 大小写不敏感)。
pub fn icon_for_file(name: &str) -> IconKind {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" | "js" | "jsx" | "ts" | "tsx" | "py" | "go" | "java" | "c" | "cpp" | "h" | "hpp"
        | "rb" | "swift" | "kt" | "sh" => IconKind::FileCode,
        "json" => IconKind::FileJson,
        "md" | "txt" => IconKind::FileText,
        "toml" | "yaml" | "yml" => IconKind::FileConfig,
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" => IconKind::FileImage,
        _ => IconKind::FileGeneric,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_for_file_maps_code_extensions() {
        assert_eq!(icon_for_file("main.rs"), IconKind::FileCode);
        assert_eq!(icon_for_file("app.TS"), IconKind::FileCode); // 大小写不敏感
        assert_eq!(icon_for_file("script.py"), IconKind::FileCode);
    }

    #[test]
    fn icon_for_file_maps_json() {
        assert_eq!(icon_for_file("package.json"), IconKind::FileJson);
    }

    #[test]
    fn icon_for_file_maps_text() {
        assert_eq!(icon_for_file("README.md"), IconKind::FileText);
        assert_eq!(icon_for_file("notes.txt"), IconKind::FileText);
    }

    #[test]
    fn icon_for_file_maps_config() {
        assert_eq!(icon_for_file("Cargo.toml"), IconKind::FileConfig);
        assert_eq!(icon_for_file("ci.yaml"), IconKind::FileConfig);
        assert_eq!(icon_for_file("ci.yml"), IconKind::FileConfig);
    }

    #[test]
    fn icon_for_file_maps_image() {
        assert_eq!(icon_for_file("logo.png"), IconKind::FileImage);
        assert_eq!(icon_for_file("icon.svg"), IconKind::FileImage);
    }

    #[test]
    fn icon_for_file_falls_back_to_generic_for_unknown_extension() {
        assert_eq!(icon_for_file("LICENSE"), IconKind::FileGeneric);
        assert_eq!(icon_for_file("Makefile"), IconKind::FileGeneric);
        assert_eq!(icon_for_file("data.xyz"), IconKind::FileGeneric);
    }
}
