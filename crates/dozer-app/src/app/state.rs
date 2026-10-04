//! 外壳态的类型:面板标识、顶栏按钮、悬停动画状态机、页面/放大/聚焦态、
//! 项目页签槽、几何只读快照 `ShellState` 及其宽度访问器。Phase 2 结构重组
//! 时从 `app.rs` 抽出,类型与语义保持原样。

use serde::{Deserialize, Serialize};

use dozer_core::protocol::ProjectInfo;
use iced_widget::core::Color;

use crate::chrome::rail;
use crate::panel_host::HoverSlot;
use crate::workspace::Workspace;

use super::layout::{PanelDims, ShellLayout};

/// 工作区 12 个面板的统一标识——workspace 图标栏拖拽换栏功能
/// (见 `2026-08-19-rail-panel-drag-relocation-design.md`)的面板类型。
/// 由原左栏(7)+ 右栏(3)两个枚举合并而来,variant 名字逐一沿用,
/// 不改名。Stage 1(这次)只做了类型统一 + 数据模型,渲染/交互仍各自
/// 按 `left_view`/`right_view` 字段走(Stage 2/4 才遍历 `RailLayout`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelKind {
    Files,
    GitLog,
    Todo,
    Project,
    Database,
    Ssh,
    Web,
    Agent,
    GroupChat,
    Conversations,
    Usage,
    CodeHealth,
}

impl PanelKind {
    /// 面板默认挂在哪条栏——`RailLayout::default()`、以及镜像判断
    /// ("是否偏离了默认栏")共用这一份真相,不要在两处各写一份可能
    /// 不同步的列表。
    pub fn default_side(self) -> Side {
        match self {
            Self::Files
            | Self::GitLog
            | Self::Todo
            | Self::Project
            | Self::Database
            | Self::Ssh
            | Self::Web => Side::Left,
            Self::Agent
            | Self::GroupChat
            | Self::Conversations
            | Self::Usage
            | Self::CodeHealth => Side::Right,
        }
    }
}

/// 顶栏右侧按钮(添加项目 / 设置)的标识,用于追踪 hover 态(图标颜色在
/// hover 时需变金,而 SVG 颜色在构建时就定死、不随 `button::Status` 变化,
/// 所以得在 App 里记一个 hovered 目标,改色时按它重算)。语义同
/// `RailButton`,只是作用域在顶栏而非图标栏。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TopbarButton {
    AddProject,
    Settings,
}

/// 标识一个可弹右键菜单的 iced 原生输入框。右键时 `TextInputMenuOpen(target)`
/// 用它做两件事:
///
/// - `id`: 把焦点程序化移到该输入(`interface.operate` 的 `focusable::focus`),
///   这样菜单的复制/粘贴经 main.rs 合成回 ⌘/Ctrl+`c`/`v` 键盘事件后能作用于
///   这个输入(iced 的 `text_input` 只有左键会聚焦,右键不会,必须显式补)。
/// - `secure`: 密码框。复制/剪切在密码框里被 iced 原生禁用(快捷键同),右键
///   菜单里也应置灰这几项;粘贴/全选仍可用。
///
/// 各输入点用自己稳定的 `field_id()`(没有的已补)构造一个目标。
#[derive(Debug, Clone)]
pub struct TextInputTarget {
    pub id: iced_widget::core::widget::Id,
    pub secure: bool,
}

/// 所有需要"悬停平滑过渡动画"的按钮的统一标识。把顶栏右侧按钮
/// (`TopbarButton`)、图标栏按钮(`RailButton`)收进同一个
/// 枚举,这样它们能共用一套 `hover_anims` 状态机与同一条自驱 redraw 定时
/// 唤醒(见 `App::set_hover`/`advance_hover_anims`/`hover_progress`),不必
/// 每个按钮各写一套进度字段。顶栏 Home 品牌页签的标题文字 hover 时也走这个
/// 动画表(从 DIM 平滑过渡到 GOLD),与项目页签一致;选中态恒为 GOLD(见
/// `dozer_home_tab`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HoverId {
    Topbar(TopbarButton),
    Rail(rail::RailButton),
    /// 某个项目页签的关闭按钮(`×`),按项目 id 区分——同一时刻可能有多个
    /// 页签,不能像 `TopbarButton`/`RailButton` 那样用一个全局标识共用。
    ProjectTabClose(i64),
    /// 某个项目页签的标题文字,按项目 id 区分(同 `ProjectTabClose` 的考量):
    /// 未选中态 hover 时标题从 DIM 平滑过渡到 GOLD(见 `project_tab_item`)。
    ProjectTabItem(i64),
    /// 终端面板某个会话 tab 的标题文字,按会话序号区分——未选中态 hover 时
    /// 标题从 DIM 平滑过渡到 GOLD(见 `panel_tab`)。
    TermTabItem(usize),
    /// 终端面板某个会话 tab 的关闭按钮(×),按会话序号区分——hover 时颜色从
    /// DIM 平滑过渡到 GOLD(见 `panel_tab`)。
    TermTabClose(usize),
    /// 顶栏 Dozer Home 品牌页签的标题文字(图标 + "Dozer"):未选中态 hover 时
    /// 从 DIM 平滑过渡到 GOLD,选中态恒为 GOLD——与 `ProjectTabItem` 同一手法
    /// (见 `dozer_home_tab`)。
    HomeTab,
    /// 终端面板 tab 栏"溢出下拉"入口(`ChevronDown`):静止 DIM,hover
    /// 平滑过渡到 GOLD,处理方式同 `HoverId::list_collapse(PanelKind::Files)`(见 `terminal::tab_bar`)。
    TermTabOverflow,
    /// 任一 tab 组"溢出下拉"菜单里**某一行**的悬停(按该行在菜单里的
    /// `TabOverflowEntry::index` 区分)。五处下拉(终端/文件预览/项目预览/
    /// SSH/Database)互斥展开,同一时刻只会有其中一个菜单可见,因此不同组
    /// 复用同一套行下标键不会冲突(见 `tab_widget::tab_overflow_menu`)。
    TabOverflowRow(usize),
    /// 首页项目列表搜索框内的提交按钮(`Search`):静止 DIM,hover 平滑
    /// 过渡到 GOLD,处理方式同 `TodoAddSubmit`(见
    /// `homespace::home_project_list_view`)。
    HomeProjectSearchSubmit,
    /// 首页项目列表"更多..."翻页图标按钮(`Ellipsis`):静止 DIM,hover
    /// 平滑过渡到 GOLD,处理方式同 `HomeProjectSearchSubmit`。
    HomeProjectMore,
    /// 某个面板里的一个可悬停元素,按"面板 + 槽位"作用域取键——**宿主枚举里不再点名任何面板的按钮**
    /// (槽位词汇见 [`HoverSlot`],通用、不含面板名)。用下面的 `HoverId::tab_item(..)` 等构造函数造。
    Panel(PanelKind, HoverSlot),
}

impl HoverId {
    /// 面板列表列的折叠/展开按钮。
    pub(crate) fn list_collapse(panel: PanelKind) -> Self {
        Self::Panel(panel, HoverSlot::ListCollapse)
    }
    /// 面板搜索框旁的提交按钮。
    pub(crate) fn search_submit(panel: PanelKind) -> Self {
        Self::Panel(panel, HoverSlot::SearchSubmit)
    }
    /// 列表末尾的"更多…"翻页按钮。
    pub(crate) fn more(panel: PanelKind) -> Self {
        Self::Panel(panel, HoverSlot::More)
    }
    /// 页签标题(`key` 是页签下标或稳定 id)。
    pub(crate) fn tab_item(panel: PanelKind, key: u64) -> Self {
        Self::Panel(panel, HoverSlot::TabItem(key))
    }
    /// 页签关闭按钮(`key` 同 `tab_item`)。
    pub(crate) fn tab_close(panel: PanelKind, key: u64) -> Self {
        Self::Panel(panel, HoverSlot::TabClose(key))
    }
    /// 页签溢出菜单按钮。
    pub(crate) fn tab_overflow(panel: PanelKind) -> Self {
        Self::Panel(panel, HoverSlot::TabOverflow)
    }
    /// 列表行/卡片(`key` 是行下标或稳定 id)。
    pub(crate) fn row(panel: PanelKind, key: u64) -> Self {
        Self::Panel(panel, HoverSlot::Row(key))
    }
    /// 一组互斥选项/筛选里的一项。
    pub(crate) fn choice(panel: PanelKind, key: u64) -> Self {
        Self::Panel(panel, HoverSlot::Choice(key))
    }
    /// 面板内一次性的具名按钮(`name` 在同一面板内唯一)。
    pub(crate) fn named(panel: PanelKind, name: &'static str) -> Self {
        Self::Panel(panel, HoverSlot::Named(name))
    }
}

/// 一个可平滑过渡的 hover 动画状态机。iced 0.14 无内置动画 API,这套自驱
/// redraw(与光标闪烁同款范式)把图标/背景颜色在 idle↔hover 之间做 ease-out
/// 插值,而非硬切。`progress` 朝 `target`(0 或 1)指数逼近,约 80ms 收敛。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct HoverAnim {
    /// 当前帧插值系数(0..=1)。
    pub(crate) progress: f32,
    /// 动画目标(0=未悬停,1=悬停)。
    pub(crate) target: f32,
}

impl HoverAnim {
    /// 设置悬停目标(`true`=进入,`false`=离开);动画由 `advance` 循环逼近。
    pub(crate) fn set(&mut self, hovered: bool) {
        self.target = if hovered { 1.0 } else { 0.0 };
    }
    /// 朝目标逼近一拍(每拍残余 50%),足够接近则 snap 到目标避免无限抖动。
    pub(crate) fn advance(&mut self) {
        let next = self.progress + (self.target - self.progress) * 0.5;
        self.progress = if (next - self.target).abs() < 0.01 {
            self.target
        } else {
            next
        };
    }
    /// 动画是否仍在进行中(进度未到目标)。
    pub(crate) fn active(&self) -> bool {
        (self.progress - self.target).abs() > 0.001
    }
    /// 当前插值系数,给视图层做颜色插值。
    pub(crate) fn t(&self) -> f32 {
        self.progress
    }
}

/// 页签标题 tooltip 的悬停触发延迟:进入页签并持续悬停满 2s 才弹出标题全称,
/// 避免短暂停留就弹气泡打扰。计时起点记在 `App::hover_tooltip_starts`,由
/// `Message::Hover` 进入/离开驱动;main.rs 的自驱 redraw 负责在满 2s 那一刻
/// 重绘出气泡(见 `next_tooltip_wake`)。
pub(crate) const HOVER_TOOLTIP_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
/// 顶层级页面：工作区(默认,左右面板区+页签) / 首页落地页(点顶栏 Dozer 进入)。
/// 默认 `Workspace`——程序启动照常进工作区,Home 是用户主动点击 Dozer 才进。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppPage {
    #[default]
    Workspace,
    Home,
}

/// 当前放大态：放大的是左面板区的内容子面板，还是右面板区的。`None` = 未放大。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MaximizedPane {
    Left,
    Right,
}

/// 当前"聚焦"在哪一侧面板区:左键点击落点决定(见 [`zone_at_x`]),用于
/// `left_zone`/`right_zone` 外边框的高亮态——点哪侧,哪侧的边框就变亮
/// (GOLD),不区分具体点中区内哪个 pane(项目树/预览/终端/Agent 列表等）。
/// 点在图标栏/分隔线上不改变当前态(`zone_at_x` 返回 `None`)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoneSide {
    Left,
    Right,
}

/// 单个项目页签的加载状态：懒加载用。`Stub` 只有页签渲染需要的最小信息
/// （启动恢复时,还没被聚焦过的页签停在这一态）,`Loaded` 是完整的
/// `Workspace`（P2a 多项目并行）。
pub enum WorkspaceSlot {
    Stub {
        info: ProjectInfo,
        /// 该项目在**启动恢复那一刻**的后台活动指示点颜色(见
        /// [`stub_activity`],内核与 `Loaded` 页签共用的 [`project_dot`])。
        /// `None` = 那时没有存活会话,不画指示点。存成算好的颜色而不是裸
        /// `AgentState`,是因为"死会话"(纯 shell/git shell/Unknown agent)
        /// 那个灰点不是任何一个 `AgentState` 变体能表达的。
        ///
        /// 这份快照之后不会再更新——`Stub` 没有任何会话事件流,真正的实时
        /// 指示点要等它被促成成 `Loaded`。写成"restart 时已知"而不是空白,
        /// 是因为重启后**所有**后台页签都是 `Stub`,指示点全空等于整个功能
        /// 在最常见的场景下不工作(最终审查 Required Fix #5)。
        activity: Option<Color>,
    },
    Loaded(Box<Workspace>),
}

/// 图标栏方向:左栏或右栏。用作 `RailLayout` 的访问器参数,以及后续拖拽
/// (Stage 4)的方向来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}
/// 主界面当前几何状态的只读快照(main.rs 拖拽追踪/离屏几何计算用途,
/// `Copy` 类型直接按值传递)。取代旧 `PanelLayout` 单独传递的做法——
/// 新几何公式(webview bounds/焦点路由/IME 光标)都依赖"当前是哪个视图、
/// 是否收起"，不能只靠宽高数字算，所以把这些也打包进来。
#[derive(Debug, Clone, PartialEq)]
pub struct ShellState {
    pub layout: ShellLayout,
    /// 当前活跃项目面板区尺寸(左宽 + 四个 split)。每项目一份,由
    /// `App::adopt_panel_layout` 切项目时灌入,`apply_column_drag` 拖拽后写回。
    pub dims: PanelDims,
    pub left_view: PanelKind,
    pub left_collapsed: bool,
    pub right_view: PanelKind,
    pub right_collapsed: bool,
    /// 浏览器收藏夹侧栏是否展开——`preview_content_bounds` 的
    /// `PanelKind::Web` 分支据此决定网页 webview 要不要让出侧栏宽度。
    pub browser_bookmarks_open: bool,
    /// 当前放大态。`preview_content_bounds`/`is_in_preview_column`/
    /// `terminal_pane_pixel_size` 靠这个字段才能感知"这块内容其实被放大
    /// 遮罩盖住了/放大到了整个 maximize 区域"——没有它,离屏 webview 摆位、
    /// 焦点路由、终端 PTY 网格都会对着放大前的旧几何算,和 `maximize_overlay`
    /// 实际渲染的画面对不上。
    pub maximized: Option<MaximizedPane>,
}

/// 两条图标栏与那条恒在的 `LeftRight` 分隔线之外，留给左右两个面板区的
/// 总宽。`view()` 无条件渲染 `divider_bar(Divider::LeftRight)`(收起某侧也
/// 保留拖拽手柄)，所以这 8px 恒扣，不看收起态——早先版本只在两侧都可见时
/// 扣，导致收起一侧后几何比实际渲染宽 8px 且原点左偏 8px。
pub(crate) fn zones_width(window_width: f32) -> f32 {
    (window_width
        - 2.0 * byteui::theme::geometry::icon_rail_width()
        - byteui::theme::geometry::divider_width())
    .max(0.0)
}

/// 把持久化的 `left_width` 夹进"当前窗口宽度下合法"的区间:下限
/// `byteui::theme::geometry::min_zone_width()`,上限"给右面板区也留够 `byteui::theme::geometry::min_zone_width()`"。窗口窄到
/// 上界低于下界时用 `.max(byteui::theme::geometry::min_zone_width())` 把上界垫平,`clamp` 恒不 panic。
///
/// 这是**唯一**一处 left_width 的夹取:渲染侧(`left_panel_area` 经
/// `Workspace::effective_left_width`)、几何侧(`left_zone_width` → webview
/// bounds/命中测试/终端网格)、拖拽侧(`apply_column_drag`)全部走这里。
/// 各写一份夹取(或只在拖拽时夹一次)就会出现:窗口被拖窄后持久化宽仍按
/// 640 渲染,`Length::Fixed(640)` 在 flex 第一趟就把可用空间吃光,唯一
/// `Fill` 的右面板区拿到 0 宽——整个右半边(终端/Agent 列表/对话/审阅)
/// 凭空消失(Fix round 2 Critical #1)。夹取只发生在渲染/几何时刻,不回写
/// `ShellLayout`,窗口再拉宽时用户原来偏好的宽度自动复原。
pub(crate) fn clamp_left_width(window_width: f32, left_width: f32) -> f32 {
    let upper = (zones_width(window_width) - byteui::theme::geometry::min_zone_width())
        .max(byteui::theme::geometry::min_zone_width());
    left_width.clamp(byteui::theme::geometry::min_zone_width(), upper)
}

/// 左面板区当前实际宽度(逻辑像素)：收起时 0；对侧收起时独占 `zones_width`
/// (与 `left_panel_area` 此时渲染成 `Length::Fill` 对应)；否则用持久化宽
/// 按当前窗口宽夹取后的**有效**宽(见 `clamp_left_width`)。
pub(crate) fn left_zone_width(window_width: f32, state: &ShellState) -> f32 {
    if state.left_collapsed {
        0.0
    } else if state.right_collapsed {
        zones_width(window_width)
    } else {
        clamp_left_width(window_width, state.dims.left_width)
    }
}

/// 右面板区当前实际宽度(逻辑像素)；收起时为 0；否则是 `zones_width` 里
/// 左面板区没占走的剩余空间——右面板区不像左面板区那样有独立持久化宽度，
/// 恒为 Fill。
pub(crate) fn right_zone_width(window_width: f32, state: &ShellState) -> f32 {
    if state.right_collapsed {
        return 0.0;
    }
    (zones_width(window_width) - left_zone_width(window_width, state)).max(0.0)
}

/// 配对视图内部可按 split 比例分配的宽度 = 区宽减去中间那条分隔线。
/// 两侧配对都用 `FillPortion` 渲染内部分割，而 `FillPortion` 是在扣掉固定
/// 宽的分隔线之后才按比例分剩余空间的，所以比例的分母必须是这个值，不是
/// 区宽本身。
pub(crate) fn pair_content_width(zone_width: f32) -> f32 {
    (zone_width - byteui::theme::geometry::divider_width()).max(0.0)
}

#[cfg(test)]
mod log_name_tests {
    use super::PanelKind;

    /// 每个 `PanelKind` 对应的日志面板名(单一真相)。源码里每个
    /// `scope!(_, panel, "<名>")` 的名字都必须出自这张表——由下面的源码扫描测试强制。
    /// 穷举 `match` 保证新增变体时编译报错,提醒同步这张表。
    fn expected_log_name(kind: PanelKind) -> &'static str {
        match kind {
            PanelKind::Files => "files",
            PanelKind::GitLog => "git_log",
            PanelKind::Todo => "todo",
            PanelKind::Project => "project",
            PanelKind::Database => "database",
            PanelKind::Ssh => "ssh",
            PanelKind::Web => "web",
            PanelKind::Agent => "agent",
            PanelKind::GroupChat => "group_chat",
            PanelKind::Conversations => "conversations",
            PanelKind::Usage => "usage",
            PanelKind::CodeHealth => "code_health",
        }
    }

    const ALL_KINDS: [PanelKind; 12] = [
        PanelKind::Files,
        PanelKind::GitLog,
        PanelKind::Todo,
        PanelKind::Project,
        PanelKind::Database,
        PanelKind::Ssh,
        PanelKind::Web,
        PanelKind::Agent,
        PanelKind::GroupChat,
        PanelKind::Conversations,
        PanelKind::Usage,
        PanelKind::CodeHealth,
    ];

    #[test]
    fn log_names_are_unique_and_valid_scope_names() {
        let mut seen = std::collections::HashSet::new();
        for k in ALL_KINDS {
            let n = expected_log_name(k);
            assert!(dozer_core::log::is_valid_scope_name(n), "{n}");
            assert!(seen.insert(n), "重复的面板日志名: {n}");
        }
    }

    /// 源码里每个 panel 来源的名字都必须是某个 `PanelKind` 的日志名;弹窗类 extension
    /// 与非面板代码必须用 module 来源,不能冒充面板。
    #[test]
    fn every_panel_scope_in_source_uses_a_known_panel_name() {
        let known: std::collections::HashSet<&str> =
            ALL_KINDS.iter().map(|k| expected_log_name(*k)).collect();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root];
        let mut found = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&p).unwrap();
                for line in text.lines() {
                    let t = line.trim_start();
                    if t.starts_with("//") {
                        continue;
                    }
                    let Some(rest) = t.strip_prefix("dozer_core::scope!(") else {
                        continue;
                    };
                    let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
                    if parts.len() >= 3 && parts[1] == "panel" {
                        let name = parts[2].trim_end_matches(");").trim_matches('"');
                        assert!(
                            known.contains(name),
                            "{}: 未知面板日志名 {name:?}(必须是 PanelKind 对应的名字)",
                            p.display()
                        );
                        found.push(name.to_string());
                    }
                }
            }
        }
        // 至少已经声明的面板来源都被扫到,证明扫描本身有效而不是空转。
        for expected in [
            "files",
            "todo",
            "ssh",
            "database",
            "agent",
            "project",
            "conversations",
        ] {
            assert!(
                found.iter().any(|n| n == expected),
                "源码扫描没找到 panel 来源 {expected:?},实际找到 {found:?}"
            );
        }
    }
}
