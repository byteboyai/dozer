//! Lucide 图标(MIT/ISC，见 `assets/icons/LICENSE`)编译期内嵌 + 统一渲染入口。
//! `IconKind` 是穷举枚举而非开放式字符串——新增图标 = 加一个变体 + 一个 svg 文件，
//! 与 `theme.rs` 精选 14 色而非任意色值同一哲学。

use iced_widget::MouseArea;
use iced_widget::button;
use iced_widget::container;
use iced_widget::core::mouse;
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::svg;
use iced_widget::{Tooltip, text, tooltip};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    /// 文件树底部分支切换按钮在弹层**展开**态下的图标(Lucide chevron-up):
    /// 收起时为 `ChevronDown`,展开后翻转,提示"点此收起弹层"。
    ChevronUp,
    Folder,
    FolderOpen,
    /// 文件树根目录头部图标(Lucide folder-open-dot:展开的文件夹 + 右上圆点,
    /// 醒目标识项目根)。
    FolderOpenDot,
    /// 项目信息面板「项目根目录」图标(Lucide folder-dot:文件夹 + 中心圆点)。
    FolderDot,
    /// 项目信息面板「Git 远程仓库」图标(Lucide folder-root:文件夹 + 中心
    /// 圆点 + 向下树根,表示仓库根)。
    FolderRoot,
    /// 文件树底部 git 分支栏图标(Lucide folder-git-2:文件夹 + git 分支/网络,
    /// 表示当前项目在 git 仓库内)。
    FolderGit2,
    /// 文件树底部"未受 git 保护"栏图标(Lucide folder-minus:文件夹 + 减号,
    /// 表示当前项目尚无 git 仓库)。
    FolderMinus,
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
    /// Git Log 面板 commit 列表"普通提交"图标(Lucide git-commit-vertical)。
    GitCommitVertical,
    /// Todo 面板 rail 图标(Lucide list-todo)。
    ListTodo,
    /// Todo 面板"进行中"分类 + EXECUTING 徽章(Lucide play)。分类图标统一
    /// 换成 `CircleSmall` 后本变体暂无调用点,资源保留(YAGNI,不新增 filled
    /// 变体),等以后需要单独强调"进行中"语义时复用。
    #[allow(dead_code)]
    Play,
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
    /// 文件树"显示/隐藏以 . 开头的文件/目录"按钮图标(Lucide eye-off:
    /// 眼睛被打上斜杠,表示点文件当前不可见)。
    EyeOff,
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
    /// Git Log 面板 commit 列表"合并提交"图标(Lucide git-merge)。
    GitMerge,
    /// 文件树搜索框的搜索按钮图标(Lucide folder-search:文件夹 + 放大镜)。
    FolderSearch,
    /// 项目信息面板小节标题前的占位图标(Lucide circle-small:小圆点)。
    CircleSmall,
    /// SSH 主机卡片"文件传输"按钮 + SFTP tab 前缀图标(Lucide
    /// folder-sync)。
    FolderSync,
    /// Todo 卡片计划日期徽章图标(Lucide calendar):点击弹出日历日期选择器。
    Calendar,
    /// Todo 新增任务框右侧提交按钮图标(Lucide circle-arrow-up)。
    CircleArrowUp,
}

impl IconKind {
    fn bytes(self) -> &'static [u8] {
        match self {
            IconKind::ChevronLeft => include_bytes!("../assets/icons/chevron-left.svg"),
            IconKind::ChevronRight => include_bytes!("../assets/icons/chevron-right.svg"),
            IconKind::ChevronDown => include_bytes!("../assets/icons/chevron-down.svg"),
            IconKind::ChevronUp => include_bytes!("../assets/icons/chevron-up.svg"),
            IconKind::Folder => include_bytes!("../assets/icons/folder.svg"),
            IconKind::FolderOpen => include_bytes!("../assets/icons/folder-open.svg"),
            IconKind::FolderOpenDot => include_bytes!("../assets/icons/folder-open-dot.svg"),
            IconKind::FolderDot => include_bytes!("../assets/icons/folder-dot.svg"),
            IconKind::FolderRoot => include_bytes!("../assets/icons/folder-root.svg"),
            IconKind::FolderGit2 => include_bytes!("../assets/icons/folder-git-2.svg"),
            IconKind::FolderMinus => include_bytes!("../assets/icons/folder-minus.svg"),
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
            IconKind::GitCommitVertical => {
                include_bytes!("../assets/icons/git-commit-vertical.svg")
            }
            IconKind::ListTodo => include_bytes!("../assets/icons/list-todo.svg"),
            IconKind::Play => include_bytes!("../assets/icons/play.svg"),
            IconKind::Briefcase => include_bytes!("../assets/icons/briefcase.svg"),
            IconKind::Star => include_bytes!("../assets/icons/star.svg"),
            IconKind::Bookmark => include_bytes!("../assets/icons/bookmark.svg"),
            IconKind::BarChart3 => include_bytes!("../assets/icons/bar-chart-3.svg"),
            IconKind::BadgeCheck => include_bytes!("../assets/icons/badge-check.svg"),
            IconKind::Database => include_bytes!("../assets/icons/database.svg"),
            IconKind::Table => include_bytes!("../assets/icons/table.svg"),
            IconKind::Eye => include_bytes!("../assets/icons/eye.svg"),
            IconKind::EyeOff => include_bytes!("../assets/icons/eye-off.svg"),
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
            IconKind::GitMerge => include_bytes!("../assets/icons/git-merge.svg"),
            IconKind::FolderSearch => include_bytes!("../assets/icons/folder-search.svg"),
            IconKind::CircleSmall => include_bytes!("../assets/icons/circle-small.svg"),
            IconKind::FolderSync => include_bytes!("../assets/icons/folder-sync.svg"),
            IconKind::Calendar => include_bytes!("../assets/icons/calendar.svg"),
            IconKind::CircleArrowUp => include_bytes!("../assets/icons/circle-arrow-up.svg"),
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

/// 图标按钮的完整接线版:图标按钮 + hover 动画 + 点击,一次性把
/// `MouseArea`(`Pointer` 光标 + `on_press`/`on_enter`/`on_exit`)接好。
/// 调用方只需算好 `hover_t`(通常是 `app.hover_progress(some_id)`,多数
/// view 函数拿不到 `&App`,这一步仍留给调用方)和一个 `on_hover` 闭包——
/// 闭包内部才知道具体 `HoverId`,`icon_button_entry` 本身不认识 `HoverId`
/// (定义在 `app.rs`,`icons.rs` 不依赖 `app.rs`)。
///
/// 视觉与两侧 icon rail 的 `app::rail_icon_button` 逐像素一致:常驻 `CARD`
/// 圆角方底,图标恒为 GOLD 或随 hover 从 DIM 平滑过渡到 GOLD,`active` 时
/// 加 1px 金框(未选中无边框),方形命中区取 `rail_button_size()`——迁移
/// rail 的 11 个按钮外观与迁移前完全一致(见实现计划 2026-08-12)。
#[allow(clippy::too_many_arguments)]
pub fn icon_button_entry<'a, M: Clone + 'a>(
    kind: IconKind,
    size: f32,
    active: bool,
    hover_t: f32,
    card: bool,
    button_size: f32,
    interactive: bool,
    on_select: M,
    on_hover: impl Fn(bool) -> M + 'a,
    tooltip: &'a str,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    // 图标颜色:选中态恒为金;未选中时 hover 平滑过渡到金(SVG 颜色构建时
    // 定死、不吃 `button::Status`,所以 hover 进度靠 `hover_t` 参数从调用方
    // 算进来)。
    let color = if active {
        crate::theme::color::GOLD
    } else {
        crate::theme::color::mix(crate::theme::color::DIM, crate::theme::color::GOLD, hover_t)
    };
    let inner = container(view(kind, size, color))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center);

    let radius = 8.0;
    let base_border = Border {
        color: Color::TRANSPARENT,
        width: 1.0,
        radius: radius.into(),
    };
    let mut btn = button(inner)
        .width(Length::Fixed(button_size))
        .height(Length::Fixed(button_size))
        .padding(0)
        .style(move |_t: &iced_widget::Theme, _status: button::Status| {
            // 圆角正方形背景常驻(`card` 为真时);金色外框只在选中态出现,
            // hover 不放金框——所以样式完全由 `active`/`card` 决定,与
            // 交互态无关。
            button::Style {
                background: if card {
                    Some(crate::theme::color::CARD.into())
                } else {
                    None
                },
                border: Border {
                    color: if active {
                        crate::theme::color::GOLD
                    } else {
                        Color::TRANSPARENT
                    },
                    ..base_border
                },
                ..button::Style::default()
            }
        });
    // `interactive` 为假时不挂 `on_press`——收藏星标按钮在没有 URL 时应该
    // 不可点(2026-08-12 全量推广时发现,`browser.rs` 的星标按钮此前用的
    // 是"条件挂载 on_press"这个手法,不能无条件套用)。
    if interactive {
        btn = btn.on_press(on_select);
    }

    let content: Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> = MouseArea::new(btn)
        .interaction(mouse::Interaction::Pointer)
        .on_enter(on_hover(true))
        .on_exit(on_hover(false))
        .into();
    with_tooltip(content, tooltip)
}

/// 提示气泡的容器样式:背景 `CARD` 实底 + `BORDER` 1px 描边圆角 6。`with_tooltip`
/// 与页签"悬停 3s 才弹标题全称"的受控 tooltip 共用同一份样式,避免两处各写。
/// 文字颜色/字号由调用方写在 bubble 内容里(`CREAM` 12px)。
pub fn tooltip_bubble_style() -> impl Fn(&iced_widget::Theme) -> container::Style {
    |_t: &iced_widget::Theme| container::Style {
        background: Some(iced_widget::core::Background::Color(
            crate::theme::color::CARD,
        )),
        border: Border {
            color: crate::theme::color::BORDER,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    }
}

/// ByteBoy2077 提示气泡:`Tooltip` 包一层,把 `content`(通常是图标按钮)用
/// 指定 `label` 描述,鼠标悬停一小段延迟后弹出。背景 `CARD` 实底 + `BORDER`
/// 1px 描边圆角 6、`CREAM` 文字 12px,三角指针对着按钮。`label` 用中文
/// (项目 UI 面向甲方,中文优先)。
pub fn with_tooltip<'a, M: Clone + 'a, R: iced_widget::core::text::Renderer + 'a>(
    content: impl Into<Element<'a, M, iced_widget::Theme, R>> + 'a,
    label: &'a str,
) -> Element<'a, M, iced_widget::Theme, R> {
    let bubble = container(text(label).size(12).color(crate::theme::color::CREAM)).padding([5, 9]);
    Tooltip::new(content, bubble, tooltip::Position::Bottom)
        .gap(3)
        .style(tooltip_bubble_style())
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
