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
    FolderTree,
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
    Brain,
    MessageSquare,
    /// 会话列表面板(右面板区"对话"视图列表侧)的图标(Lucide
    /// bot-message-square)。
    BotMessageSquare,
    /// 内容 pane 放大/还原(`MaximizedPane`)的触发图标。触发按钮已在
    /// `a0d324e`(2026-08-06)被主动移除,状态机/overlay 仍保留,是否重新
    /// 接一个入口留给后续产品决策——见
    /// `docs/superpowers/plans/2026-08-07-dozer-milestone-summary-and-plan-audit.md`。
    #[allow(dead_code)]
    Maximize,
    /// 顶栏 `dozer_home_tab` 品牌页签标题图标(Lucide house)。
    Home,
    RefreshCw,
    SquarePlus,
    GitBranch,
    /// Todo 面板 rail 图标(Lucide list-todo)。
    ListTodo,
    /// 项目信息面板 rail 图标(Lucide briefcase)。
    Briefcase,
    /// 浏览器地址栏"加入/移出收藏"星标(Lucide star)。已收藏态靠调用方
    /// 传 GOLD 而非切换到另一份实心图标——`icons::view` 只管描边色,单一
    /// 资源足够表达"已收藏/未收藏"两态(YAGNI,不新增 filled 变体)。
    Star,
    /// tab 栏"收藏夹"下拉面板触发图标(Lucide bookmark)。
    Bookmark,
    /// Agent 用量统计面板的图标(Lucide bar-chart-3)。
    BarChart3,
    /// 验收面板 rail 图标(Lucide badge-check)。
    BadgeCheck,
    /// 数据库面板 rail 图标(Lucide database)。
    Database,
    /// schema 树表节点图标(Lucide table)。
    Table,
    /// schema 树视图节点图标(Lucide eye)。
    Eye,
    /// 终端/Shell(纯 Shell 启动项),Lucide。
    Terminal,
    /// SSH 主机面板 rail 图标(Lucide server)。
    Server,
    /// 首页左栏"项目列表" pane rail 图标(Lucide layout-list)。
    LayoutList,
    /// 首页左栏"Recents" pane rail 图标(Lucide history)。
    History,
    /// Dozer 品牌标(dozer-logo-main.jpeg → potrace 矢量化)。顶栏
    /// `dozer_home_tab` 品牌页签改用 Lucide house(`IconKind::Home`)后暂无
    /// 调用点——资源本身保留(重新矢量化成本不低),供以后需要展示这枚
    /// 定制矢量标时复用。
    #[allow(dead_code)]
    Dozer,
    /// Agent 品牌标(着色用 `currentColor`,由调用方按 agent 指定主题色)。
    /// 来源:Claude/CodeBuddy 取自 Simple Icons,OpenCode 取自其官网 favicon 并
    /// 归一化到 24×24。均为品牌标识,非 Lucide;仅供 Agent 身份识别。
    Claude,
    Codebuddy,
    Opencode,
    /// footbar CPU 段前缀图标(Lucide square-activity:圆角方框 + 折线,表活跃度)。
    SquareActivity,
    /// footbar Proxy 段前分隔图标(Lucide square-radical:方括号根号,代代理/路由)。
    SquareRadical,
    /// 顶栏/footbar 品牌前置图标(Lucide square-terminal:方角框 + 终端提示符),
    /// 用于 footbar 右侧 "Dozer AI Coder" 名称前作品牌标记。
    SquareTerminal,
    /// git-log 面板 rail 图标(Lucide git-graph:节点 + 连线的图形化历史)。
    GitGraph,
    /// 文件树搜索框的搜索按钮图标(Lucide folder-search:文件夹 + 放大镜)。
    FolderSearch,
}

impl IconKind {
    fn bytes(self) -> &'static [u8] {
        match self {
            IconKind::ChevronLeft => include_bytes!("../assets/icons/chevron-left.svg"),
            IconKind::ChevronRight => include_bytes!("../assets/icons/chevron-right.svg"),
            IconKind::ChevronDown => include_bytes!("../assets/icons/chevron-down.svg"),
            IconKind::Folder => include_bytes!("../assets/icons/folder.svg"),
            IconKind::FolderOpen => include_bytes!("../assets/icons/folder-open.svg"),
            IconKind::FolderTree => include_bytes!("../assets/icons/folder-tree.svg"),
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
            IconKind::Brain => include_bytes!("../assets/icons/brain.svg"),
            IconKind::MessageSquare => include_bytes!("../assets/icons/message-square.svg"),
            IconKind::BotMessageSquare => include_bytes!("../assets/icons/bot-message-square.svg"),
            IconKind::Maximize => include_bytes!("../assets/icons/maximize-2.svg"),
            IconKind::Home => include_bytes!("../assets/icons/home.svg"),
            IconKind::RefreshCw => include_bytes!("../assets/icons/refresh-cw.svg"),
            IconKind::SquarePlus => include_bytes!("../assets/icons/square-plus.svg"),
            IconKind::GitBranch => include_bytes!("../assets/icons/git-branch.svg"),
            IconKind::ListTodo => include_bytes!("../assets/icons/list-todo.svg"),
            IconKind::Briefcase => include_bytes!("../assets/icons/briefcase.svg"),
            IconKind::Star => include_bytes!("../assets/icons/star.svg"),
            IconKind::Bookmark => include_bytes!("../assets/icons/bookmark.svg"),
            IconKind::BarChart3 => include_bytes!("../assets/icons/bar-chart-3.svg"),
            IconKind::BadgeCheck => include_bytes!("../assets/icons/badge-check.svg"),
            IconKind::Database => include_bytes!("../assets/icons/database.svg"),
            IconKind::Table => include_bytes!("../assets/icons/table.svg"),
            IconKind::Eye => include_bytes!("../assets/icons/eye.svg"),
            IconKind::Terminal => include_bytes!("../assets/icons/terminal.svg"),
            IconKind::Server => include_bytes!("../assets/icons/server.svg"),
            IconKind::LayoutList => include_bytes!("../assets/icons/layout-list.svg"),
            IconKind::History => include_bytes!("../assets/icons/history.svg"),
            IconKind::Dozer => include_bytes!("../assets/icons/dozer-logo.svg"),
            IconKind::Claude => include_bytes!("../assets/icons/claude.svg"),
            IconKind::Codebuddy => include_bytes!("../assets/icons/codebuddy.svg"),
            IconKind::Opencode => include_bytes!("../assets/icons/opencode.svg"),
            IconKind::SquareActivity => include_bytes!("../assets/icons/square-activity.svg"),
            IconKind::SquareRadical => include_bytes!("../assets/icons/square-radical.svg"),
            IconKind::SquareTerminal => include_bytes!("../assets/icons/square-terminal.svg"),
            IconKind::GitGraph => include_bytes!("../assets/icons/git-graph.svg"),
            IconKind::FolderSearch => include_bytes!("../assets/icons/folder-search.svg"),
        }
    }
}

/// 统一图标渲染入口:调用方传主题色，用法与 `text().color(...)` 一致。
/// 泛型于 `Message`(图标本身不接受点击，任何 `Message` 类型都能塞进去)。
pub fn view<'a, Message: 'a>(
    kind: IconKind,
    size: f32,
    color: Color,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
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
