// crates/dozer-app/src/app.rs
//! 整个程序只有一份的外壳态:daemon 客户端、窗口尺寸、图标栏/面板区的
//! 收起-拖宽-放大状态、并行打开的 N 个项目页签。`App::view()`/`update()`
//! 是 iced 应用的顶层入口,`Message` 是它的消息协议。
//!
//! 与 `Workspace`(单个项目的全部状态,见 `crate::workspace`)的拆分边界:
//! `impl Workspace` 的方法零处依赖 `App`(`ShellIo` 就是为此存在的解耦
//! 值类型),因此依赖方向是单向的——本文件 `use crate::workspace::
//! Workspace;`,`workspace.rs` 只反向借用本文件的 `Message` 一个类型
//! (`Workspace` 的异步方法要构造 `Message` 变体经 `EventLoopProxy`
//! 回传)。拆分细节见
//! `docs/superpowers/specs/2026-08-08-app-workspace-file-split-design.md`.

use crate::conversation::SessionRow;
use crate::extensions::browser;
use crate::extensions::database;
use crate::extensions::files;
use crate::extensions::footbar;
use crate::extensions::git_log;
use crate::extensions::project;
use crate::extensions::search;
use crate::extensions::ssh;
use crate::extensions::todo;
use crate::extensions::usage;
use crate::git_watch;
use crate::homespace::{self, HomeRecentConversation, HomeRecentFile, load_home_recents};
use crate::layout;
use crate::open_projects;
use crate::panel_layouts;
use crate::preview::WebviewSpec;
use crate::rail;
use crate::tab_widget;
use crate::term_view;
use crate::terminal;
use crate::theme;
use crate::topbar;
use crate::transcript::ReviewEntry;
use crate::webview_geometry;
use crate::workspace::{
    CONVERSATION_DETAIL_PAGE_SIZE, PickerLaunch, RestorePayload, ReviewSource, ReviewView, ShellIo,
    SshOut, TabBackend, Workspace, agent_list_pane, agent_picker_popup, conversation_list_pane,
    dot_color, edit_discard_confirm_popup, edit_modal, exited_marker, fetch_project_restore,
    no_project_placeholder, preview_pane, preview_tab_display_width, project_preview_pane,
    relative_time_text, review_content_pane, review_should_refresh_on_turn,
    spawn_disk_usage_refresh, spawn_project_git_refresh, split_portions, tab_display_width,
    tab_title,
};
use byteui::interaction::icons;
use dozer_client::Client;
use dozer_core::protocol::{AgentKind, AgentState, BookmarkInfo, ProjectInfo, SessionInfo};
use iced_widget::core::border::Radius;
use iced_widget::core::font::Weight;
use iced_widget::core::mouse;
use iced_widget::core::{Border, Color, Element, Font, Length, Padding};
use iced_widget::tooltip::{Position, Tooltip};
use iced_widget::{MouseArea, button, column, container, row, stack, text};
use iced_winit::winit::event_loop::EventLoopProxy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

/// 终端初始网格尺寸（列 x 行）。真实尺寸由窗口创建后的第一次
/// `Message::PaneResized` 立刻纠正（见 `main.rs` 的 `resumed()`）；这里只是
/// "窗口还没量出真实像素前"的兜底默认值。
pub(crate) const DEFAULT_COLS: u16 = 80;

pub(crate) const DEFAULT_ROWS: u16 = 24;

/// `dozer://review-trace/data.json` 的响应体形状——`review_trace.html` 按
/// 这个结构消费(`entries`/`agent_label`/`summary_title`/`summary_text`/
/// `summary_time` 顶层字段)。总结区(标题/全文/时间)只在本会话详情里非空,
/// 活会话 `None`。
#[derive(serde::Serialize)]
struct ReviewSnapshot<'a> {
    entries: &'a [ReviewEntry],
    /// `AgentKind::label()`(如 `"claude"`)——AI 气泡的头像名字标签用它
    /// 替代写死的"AI"。
    agent_label: &'static str,
    /// session 详情顶部总结区标题/全文/相对时间(2026-08-27 起标题/全文;
    /// 2026-08-28 加时间;活会话审阅为 `None`)。
    summary_title: Option<String>,
    summary_text: Option<String>,
    summary_time: Option<String>,
}

/// `Message::ReviewLoaded` 落地新一页回合时,决定是替换还是追加进已有
/// `entries`。`append == true`("加载更多")必须真的累加,不能让调用方
/// 在这一步完成之前就拿刚到手的这一页去建 webview 快照——那样会把之前
/// 已经展示的内容整个换掉,只剩最新这一页(2026-08-27 修正的真实 bug,
/// 抽成纯函数方便 headless 单测覆盖这条语义,不依赖 App 级测试夹具)。
fn merge_review_entries(existing: &mut Vec<ReviewEntry>, new: Vec<ReviewEntry>, append: bool) {
    if append {
        existing.extend(new);
    } else {
        *existing = new;
    }
}

/// 工作区 10 个面板的统一标识——workspace 图标栏拖拽换栏功能
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
    Conversations,
    Usage,
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
            Self::Agent | Self::Conversations | Self::Usage => Side::Right,
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
    /// 预览面板某个文件 tab 的标题文字,按 tab 序号区分(同 `TermTabItem`)。
    PreviewTabItem(usize),
    /// 预览面板某个文件 tab 的关闭按钮(×),按 tab 序号区分(同 `TermTabClose`)。
    PreviewTabClose(usize),
    /// Project 面板右配对预览某个文件 tab 的标题文字,按 tab 序号区分。
    ProjectPreviewTabItem(usize),
    /// Project 面板右配对预览某个文件 tab 的关闭按钮(×),按 tab 序号区分。
    ProjectPreviewTabClose(usize),
    /// SSH 面板自己 tab 条上某个 tab 的标题文字,按 host_id 的哈希区分
    /// (SSH tab 没有稳定的数字序号——按身份是 `(host_id, SshTabKind)`,
    /// `HoverId` 整体 `derive(Copy)`,`String` 不是 `Copy`,不能直接塞
    /// `host_id.clone()`,用哈希值退化成 `u64`,不要求无碰撞,只要求
    /// "实践中够用"。
    SshTabItem(u64),
    /// SSH 面板自己 tab 条上某个 tab 的关闭按钮(×),同上按 host_id 哈希区分。
    SshTabClose(u64),
    /// 顶栏 Dozer Home 品牌页签的标题文字(图标 + "Dozer"):未选中态 hover 时
    /// 从 DIM 平滑过渡到 GOLD,选中态恒为 GOLD——与 `ProjectTabItem` 同一手法
    /// (见 `dozer_home_tab`)。
    HomeTab,
    /// Agent 面板头部"＋"按钮:无背景的 `SquarePlus` 图标,未选中态静止 DIM,
    /// hover 平滑过渡到 GOLD(见 `agent_picker_toggle_button`)。
    AgentPickerToggle,
    /// 文件树搜索提交按钮(`FolderSearch`):静止 DIM,hover 过渡到 GOLD
    /// (见 `extensions/files.rs` 的搜索按钮)。
    FilesSearchSubmit,
    /// 文件树"显示隐藏文件"切换按钮(`Eye`/`EyeOff`):静止 DIM,hover 过渡
    /// 到 GOLD;已开启(隐藏文件可见)恒金(见 `extensions/files.rs`)。
    FilesDotfiles,
    /// 文件树底栏 git 分支切换按钮(`ChevronDown`):静止 DIM,hover 过渡到
    /// GOLD(见 `extensions/files.rs` 的 `git_footer_bar`)。
    FilesBranchSwitch,
    /// 文件预览右上角"收起/展开文件树"按钮(`panel-left/right-close/open`):
    /// 静止 DIM,hover 平滑过渡到 GOLD(见 `preview_pane_for`)。
    FileTreeCollapse,
    /// Project 面板列表列"收起/展开"按钮(`panel-left/right-close/open`):
    /// 处理方式同 `FileTreeCollapse`(见 `preview_pane_for`)。
    ProjectListCollapse,
    /// 预览文件 Find 条的上一个/下一个命中(⌃/⌄)与关闭(×)图标按钮,悬停
    /// DIM→GOLD(处理同 `FilesBranchSwitch`)。Files / Project 预览各一套。
    PreviewFindPrev,
    PreviewFindNext,
    /// Project 面板预览 Find 条同上定向的独立 hover 态(Files / Project 各有
    /// 一套;× 关闭按钮是文字 glyph,不参与悬停着色)。
    ProjectPreviewFindPrev,
    ProjectPreviewFindNext,
    /// 查询框前的展开/收起替换行圆盘箭头(Files / Project 各一套)。
    PreviewFindReplaceToggle,
    ProjectPreviewFindReplaceToggle,
    /// 替换行「替换当前」/「替换全部」图标按钮(Files / Project 各一套)。
    PreviewFindReplaceCurrentBtn,
    PreviewFindReplaceAllBtn,
    ProjectPreviewFindReplaceCurrentBtn,
    ProjectPreviewFindReplaceAllBtn,
    /// Todo 面板列表列"收起/展开"按钮:处理方式同 `FileTreeCollapse`
    /// (见 `extensions::todo::view`)。
    TodoListCollapse,
    /// Todo 面板分类树的某一行(展开箭头 + 行本身共用一个悬停态,按分类
    /// id 区分,同一时刻可能有多行渲染,不能用全局标识共用)。
    TodoCategoryRow(i64),
    /// Database 面板列表列"收起/展开"按钮:处理方式同 `FileTreeCollapse`
    /// (见 `extensions::database::content_pane`)。
    DatabaseListCollapse,
    /// SSH 面板列表列"收起/展开"按钮:处理方式同 `FileTreeCollapse`
    /// (见 `ssh_tab_bar`)。
    SshListCollapse,
    /// Agent 面板列表列"收起/展开"按钮:处理方式同 `FileTreeCollapse`
    /// (见 `terminal::tab_bar`)。
    AgentListCollapse,
    /// Conversations 面板列表列"收起/展开"按钮:处理方式同 `FileTreeCollapse`
    /// (见 `review_content_pane`)。
    ConversationsListCollapse,
    /// 用量面板 agent 筛选栏"收起/展开"按钮:处理方式同 `FileTreeCollapse`
    /// (见 `extensions::usage::content_pane`)。
    UsageListCollapse,
    /// 数据库内容窗格 tab 栏:某个 tab 本体的悬停(按索引区分,同
    /// `PreviewTabItem`)。
    DatabaseTabItem(usize),
    /// 数据库内容窗格 tab 栏:某个 tab 关闭按钮 `×` 的悬停。
    DatabaseTabClose(usize),
    /// Todo 面板单个任务卡(按下标区分):hover 时填充 `CARD` 背景 + 金色描边
    /// (见 `extensions::todo::todo_card`,统一卡片样式)。
    TodoCard(usize),
    /// Todo 面板底部"新增任务"输入框内的提交按钮(`CircleArrowUp`):静止
    /// DIM,hover 平滑过渡到 GOLD(见 `extensions::todo::todo_footer_bar`)。
    TodoAddSubmit,
    /// Todo 面板搜索框内的提交按钮(`Search`):静止 DIM,hover 平滑过渡到
    /// GOLD,处理方式同 `TodoAddSubmit`(见 `extensions::todo::todo_search_bar`)。
    TodoSearchSubmit,
    /// 首页项目列表搜索框内的提交按钮(`Search`):静止 DIM,hover 平滑
    /// 过渡到 GOLD,处理方式同 `TodoAddSubmit`(见
    /// `homespace::home_project_list_view`)。
    HomeProjectSearchSubmit,
    /// 首页项目列表"更多..."翻页图标按钮(`Ellipsis`):静止 DIM,hover
    /// 平滑过渡到 GOLD,处理方式同 `HomeProjectSearchSubmit`。
    HomeProjectMore,
    /// Git Log 面板 commit 列表末尾"更多"翻页图标按钮,处理方式同
    /// `HomeProjectMore`(见 `extensions::git_log::commit_list_view`)。
    CommitListMore,
    /// 对话列表面板(会话列表)扁平列表末尾的"更多..."翻页图标按钮,处理
    /// 方式同 `CommitListMore`(见 `conversation_list_pane`)。
    ConversationListMore,
    /// 会话列表搜索框内的提交按钮(`Search`):静止 DIM,hover 平滑过渡到
    /// GOLD,处理方式同 `HomeProjectSearchSubmit`(见 `conversation_list_pane`)。
    ConversationSearchSubmit,
    /// Git Log 面板改动文件列表某行(按下标区分):hover 时填充 `CARD` 背景 +
    /// 金色描边(见 `extensions::git_log::file_list_view`,统一卡片样式)。
    GitFile(usize),
    /// Git Log 面板 commit 搜索框内的提交按钮(`Search`):静止 DIM,hover
    /// 平滑过渡到 GOLD,处理方式同 `ConversationSearchSubmit`(见
    /// `extensions::git_log::commit_search_box`)。
    GitLogSearchSubmit,
    /// 主机面板单个主机卡(按 host_id 哈希区分,`HoverId` 整体 `Copy` 不能
    /// 塞 `String`,同 `SshTabItem` 的精度取舍):hover 时填充 `CARD` 背景 +
    /// 金色描边(见 `extensions::ssh::host_card`,统一卡片样式)。
    HostCard(u64),
}

/// 一个可平滑过渡的 hover 动画状态机。iced 0.14 无内置动画 API,这套自驱
/// redraw(与光标闪烁同款范式)把图标/背景颜色在 idle↔hover 之间做 ease-out
/// 插值,而非硬切。`progress` 朝 `target`(0 或 1)指数逼近,约 80ms 收敛。
#[derive(Debug, Clone, Copy, Default)]
struct HoverAnim {
    /// 当前帧插值系数(0..=1)。
    progress: f32,
    /// 动画目标(0=未悬停,1=悬停)。
    target: f32,
}

impl HoverAnim {
    /// 设置悬停目标(`true`=进入,`false`=离开);动画由 `advance` 循环逼近。
    fn set(&mut self, hovered: bool) {
        self.target = if hovered { 1.0 } else { 0.0 };
    }
    /// 朝目标逼近一拍(每拍残余 50%),足够接近则 snap 到目标避免无限抖动。
    fn advance(&mut self) {
        let next = self.progress + (self.target - self.progress) * 0.5;
        self.progress = if (next - self.target).abs() < 0.01 {
            self.target
        } else {
            next
        };
    }
    /// 动画是否仍在进行中(进度未到目标)。
    fn active(&self) -> bool {
        (self.progress - self.target).abs() > 0.001
    }
    /// 当前插值系数,给视图层做颜色插值。
    fn t(&self) -> f32 {
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

/// 图标栏+左右面板区的宽度/分割状态。取代 `PanelLayout`——不再有"项目栏/AI栏
/// 固定宽+预览终端共享比例"这套四栏几何，改成"左面板区总宽(可拖) + 三个
/// 配对视图各自独立记住的内部列表:内容分割比例"。右面板区总宽不持久化，
/// 恒为剩余空间(`Length::Fill`)——只有一条 LeftRight 分隔线，不需要像旧
/// 模型那样两个固定宽度各自夹一条。
///
/// `#[serde(default)]`:今后加字段时,老 `layout.json` 里缺的字段用
/// `Default` 补齐,而不是整份反序列化失败 → `unwrap_or_default()` 把用户
/// 攒下来的宽度/比例全部重置。
/// 图标栏方向:左栏或右栏。用作 `RailLayout` 的访问器参数,以及后续拖拽
/// (Stage 4)的方向来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellLayout {
    /// 上次退出时的窗口逻辑尺寸(宽,高)。`main.rs` 建窗时读它决定初始
    /// `with_inner_size`,取代写死的 `byteui::theme::geometry::initial_window_size()`；`App::
    /// persist_window_size_on_exit` 在 `WindowEvent::CloseRequested` 时
    /// 写回。跟其余字段一样走 `#[serde(default)]`,老 `layout.json` 缺这
    /// 两个字段时退化成 `byteui::theme::geometry::initial_window_size()`,不影响其余已存的偏好。
    ///
    /// 左右面板区的宽度/分割比例(`left_width` 与四个 split)已迁进每项目
    /// `PanelLayout`(见 `PanelDims`/`panel_layouts.json`),`ShellLayout`
    /// 不再持有——它们是 per-project 偏好,切项目要各自换,放这里会全局
    /// 共享(见切换项目 bug)。只有窗口尺寸是全局的,留在这里。
    pub window_width: f32,
    pub window_height: f32,
    /// 每条图标栏当前有哪些面板、栏内顺序(拖拽换栏的唯一真相源,Stage 4
    /// 才真正读写;这个 Stage 只定义 + 持久化,值永远是 `default()`)。
    #[serde(default)]
    pub rail_layout: rail::RailLayout,
}

impl Default for ShellLayout {
    fn default() -> Self {
        Self {
            window_width: byteui::theme::geometry::initial_window_size().0,
            window_height: byteui::theme::geometry::initial_window_size().1,
            rail_layout: rail::RailLayout::default(),
        }
    }
}

/// 左右面板区各维度尺寸(每项目一份)。原来是 `ShellLayout` 的字段(全局共享),
/// 迁进 `PanelDims` 后挂在每项目 `PanelLayout` 上,按项目 id 记到
/// `panel_layouts.json`。`left_width` 是左面板区宽度;四个 split 是各配对视图
/// 内部的分割比例。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelDims {
    pub left_width: f32,
    /// 文件列表配对:项目树占左面板区宽度的比例，文件预览拿剩下的。
    pub files_split: f32,
    /// 文件树列表子栏是否被收起(文件预览面板右上角按钮切换)。收起时文件树
    /// 列表不渲染(预览拿满整个配对宽度),但 `files_split` 比例保留,展开时按
    /// 原比例恢复。`#[serde(default)]` 对老 `panel_layouts.json` 缺该字段时补
    /// `false`(默认展开)。
    pub files_tree_collapsed: bool,
    /// Project 面板列表列是否被收起(内容侧的展开/收起按钮切换)。语义同
    /// `files_tree_collapsed`——列表不渲染、内容拿满配对宽度,split 比例保留。
    pub project_list_collapsed: bool,
    /// Todo 面板列表列是否被收起,语义同 `project_list_collapsed`。
    pub todo_list_collapsed: bool,
    /// Database 面板列表列是否被收起,语义同 `project_list_collapsed`。
    pub database_list_collapsed: bool,
    /// SSH 面板列表列是否被收起,语义同 `project_list_collapsed`。
    pub ssh_list_collapsed: bool,
    /// Agent 面板列表列是否被收起,语义同 `project_list_collapsed`。
    pub agent_list_collapsed: bool,
    /// Conversations 面板列表列是否被收起,语义同 `project_list_collapsed`。
    pub conversations_list_collapsed: bool,
    /// 用量面板 agent 筛选栏是否被收起,语义同 `project_list_collapsed`。
    pub usage_list_collapsed: bool,
    /// Project 面板配对:信息面板占左面板区宽度的比例，项目预览(右配对)拿剩下的。
    pub project_split: f32,
    /// SSH 面板"主机列表 | 内嵌终端"两栏的分屏比例,镜像 `project_split`。
    pub ssh_split: f32,
    /// Todo 面板配对:分类导航占左面板区宽度的比例，列表/MARKDOWN 内容
    /// (右配对)拿剩下的。
    pub todo_split: f32,
    /// Git Log 面板配对:commit 列表占左面板区宽度的比例,右侧(文件列表+diff)
    /// 拿剩下的。
    pub git_log_split: f32,
    /// Git Log 面板右侧配对:文件列表占右侧区域高度的比例,diff 内容拿剩下的。
    pub git_log_file_diff_split: f32,
    /// Agent配对:Agent列表占右面板区宽度的比例，终端拿剩下的。
    pub agent_split: f32,
    /// 对话配对:对话列表占右面板区宽度的比例，对话审阅拿剩下的。
    pub conversations_split: f32,
    /// 浏览器面板配对:网页内容占左面板区宽度的比例,收藏夹侧栏(右)拿剩下
    /// 的。与其余 split 字段语义相反(内容占比而非列表占比)——浏览器是
    /// "内容在左、收藏夹侧栏在右"的唯一左面板区配对,详见 spec 第 1 节命名
    /// 理由。
    pub browser_bookmarks_split: f32,
    /// 数据库面板配对:schema 树占左面板区宽度的比例,右侧内容窗格(表/
    /// 集合/查询 tab)拿剩下的。
    pub database_split: f32,
    /// 用量面板配对:agent 筛选栏占右面板区宽度的比例,统计内容(左)拿
    /// 剩下的。语义同 `agent_split`(默认"内容在前、列表在后",见
    /// `Divider::UsageSplit`)。
    pub usage_split: f32,
}

/// 每项目尺寸的默认值(数值来源统一从这取,迁走的 `ShellLayout::default()`
/// 就是这组)。作为 `PanelDims::default()`。
/// 四个 split 用同一个 `default_split_ratio()`:所有左右双栏 zone(文件/项目/
/// Agent/对话)的初始宽度分配统一。
fn default_panel_dims() -> PanelDims {
    PanelDims {
        left_width: 640.0,
        files_split: byteui::theme::geometry::default_split_ratio(),
        files_tree_collapsed: false,
        project_list_collapsed: false,
        todo_list_collapsed: false,
        database_list_collapsed: false,
        ssh_list_collapsed: false,
        agent_list_collapsed: false,
        conversations_list_collapsed: false,
        usage_list_collapsed: false,
        project_split: byteui::theme::geometry::default_split_ratio(),
        ssh_split: byteui::theme::geometry::default_split_ratio(),
        todo_split: byteui::theme::geometry::default_split_ratio(),
        git_log_split: byteui::theme::geometry::default_split_ratio(),
        git_log_file_diff_split: byteui::theme::geometry::default_split_ratio(),
        agent_split: byteui::theme::geometry::default_split_ratio(),
        conversations_split: byteui::theme::geometry::default_split_ratio(),
        browser_bookmarks_split: byteui::theme::geometry::default_split_ratio(),
        database_split: byteui::theme::geometry::default_split_ratio(),
        usage_split: byteui::theme::geometry::default_split_ratio(),
    }
}

impl Default for PanelDims {
    fn default() -> Self {
        default_panel_dims()
    }
}

/// 每个项目各自记住的面板布局:左右面板区当前显示的配对视图、以及左右
/// 面板区是否收起。原来这几项是全局 `ShellLayout` 的字段,所有项目共享同一
/// 份,导致"在项目 A 改完面板,切到项目 B 时 B 被 A 的面板状态盖掉"(切换
/// 项目 bug)。改成按项目 id 记到 `panel_layouts.json`(见 `panel_layouts`
/// 模块),切换项目只换当前这份、绝不碰别的项目那份。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PanelLayout {
    pub left_view: PanelKind,
    pub right_view: PanelKind,
    pub left_collapsed: bool,
    pub right_collapsed: bool,
    /// 本项目的面板区尺寸(左宽 + 四个 split)。`#[serde(default)]` 对老
    /// `panel_layouts.json` 缺尺寸字段时补 `PanelDims::default()`;是否用
    /// 全局旧值/12% 回填见 `panel_layouts::load_from`。
    pub dims: PanelDims,
}

impl Default for PanelLayout {
    fn default() -> Self {
        Self {
            left_view: PanelKind::Files,
            right_view: PanelKind::Agent,
            left_collapsed: false,
            right_collapsed: false,
            dims: PanelDims::default(),
        }
    }
}

/// 把从磁盘读回来的 `ShellLayout` 夹进合法范围(`layout::load_from` 调用)。
/// 迁走面板尺寸后只剩窗口尺寸:夹下限(`byteui::theme::geometry::min_window_width()`/
/// `byteui::theme::geometry::min_window_height()`,建窗时还有 `with_min_inner_size`
/// 兜底),非法值(非有限数、缺字段的 0.0)退化成
/// `byteui::theme::geometry::initial_window_size()`。
///
/// `rail_layout` 走 `sanitize_rail_layout`:任何坏数据(任一栏为空、两侧合计
/// 不是恰 11 个不重复面板)回落 `RailLayout::default()`。
pub fn sanitize_shell_layout(l: ShellLayout) -> ShellLayout {
    ShellLayout {
        window_width: if l.window_width.is_finite() && l.window_width > 0.0 {
            l.window_width
                .max(byteui::theme::geometry::min_window_width())
        } else {
            byteui::theme::geometry::initial_window_size().0
        },
        window_height: if l.window_height.is_finite() && l.window_height > 0.0 {
            l.window_height
                .max(byteui::theme::geometry::min_window_height())
        } else {
            byteui::theme::geometry::initial_window_size().1
        },
        rail_layout: rail::sanitize_rail_layout(l.rail_layout),
    }
}

/// 把从磁盘读回来(或首次默认算出)的面板区尺寸夹进合法范围(`panel_layouts::
/// load_from` 与每项目 adopt 调用)。四个 split 用与拖拽同一对上下界:比例恰为
/// 0.0/1.0 时 `split_portions` 会给出 `FillPortion(0)`,那一块在 flex 里拿不到
/// 任何宽度、整块消失;`left_width` 只保下限(上限依赖窗口宽,由渲染/几何时刻
/// 的 `clamp_left_width` 负责,不在这里写死)。
pub fn sanitize_panel_dims(d: PanelDims) -> PanelDims {
    let clamp_split = |v: f32| {
        if v.is_finite() {
            v.clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            )
        } else {
            PanelDims::default().files_split
        }
    };
    PanelDims {
        left_width: if d.left_width.is_finite() {
            d.left_width.max(byteui::theme::geometry::min_zone_width())
        } else {
            PanelDims::default().left_width
        },
        files_split: clamp_split(d.files_split),
        files_tree_collapsed: d.files_tree_collapsed,
        project_list_collapsed: d.project_list_collapsed,
        todo_list_collapsed: d.todo_list_collapsed,
        database_list_collapsed: d.database_list_collapsed,
        ssh_list_collapsed: d.ssh_list_collapsed,
        agent_list_collapsed: d.agent_list_collapsed,
        conversations_list_collapsed: d.conversations_list_collapsed,
        usage_list_collapsed: d.usage_list_collapsed,
        project_split: clamp_split(d.project_split),
        ssh_split: clamp_split(d.ssh_split),
        todo_split: clamp_split(d.todo_split),
        git_log_split: clamp_split(d.git_log_split),
        git_log_file_diff_split: clamp_split(d.git_log_file_diff_split),
        agent_split: clamp_split(d.agent_split),
        conversations_split: clamp_split(d.conversations_split),
        browser_bookmarks_split: clamp_split(d.browser_bookmarks_split),
        database_split: clamp_split(d.database_split),
        usage_split: clamp_split(d.usage_split),
    }
}

/// 新外壳的三条可拖拽分隔线：左右面板区之间、左侧配对视图内部、右侧配对
/// 视图内部。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Divider {
    LeftRight,
    LeftPairSplit,
    /// Project 面板内部的配对分隔线:左边信息面板、右边项目预览。
    ProjectSplit,
    /// SSH 面板内部的分隔线:左边主机列表、右边内嵌终端。
    SshSplit,
    /// Todo 面板内部的配对分隔线:左边分类导航、右边列表/MARKDOWN 内容。
    TodoSplit,
    /// Git Log 面板内部左右分隔线:左边 commit 列表,右边文件列表+diff。
    GitLogSplit,
    /// 浏览器面板内部分割线:左边网页内容,右边收藏夹侧栏。与其余左面板区
    /// 分割线不同的是配对顺序反了(内容在左、列表在右),所以
    /// `apply_column_drag` 这条分支直接写 `ratio`(拖拽点左侧占比 = 内容占
    /// 比),不需要像 `RightPairSplit` 那样取反。
    BrowserBookmarksSplit,
    /// 数据库面板内部的分隔线:左边 schema 树,右边表/集合/查询内容窗格。
    DatabaseSplit,
    /// 用量面板内部的分隔线:左边统计内容,右边 agent 筛选栏。跟
    /// `RightPairSplit` 一样"默认内容在前、列表在后",但用量面板不参与
    /// Agent/Conversations 的互斥右栏轮换,所以单独开一个 divider 而不是
    /// 塞进 `RightPairSplit` 的 `kind` 分支。
    UsageSplit,
    RightPairSplit,
}

/// 纵向(上下)可拖拽分割线——目前只有 Git Log 面板右侧"文件列表 | diff
/// 内容"这一条,单独开一个枚举而不是塞进 `Divider`(横向语义不同,`Divider`
/// 现有变体全部是左右分割,`apply_column_drag`/`Message::ColumnDrag` 的
/// 几何计算全部基于 `logical_x`,混进去会让那个函数的语义变得模糊)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RowDivider {
    GitLogFileDiffSplit,
    /// Todo 面板底部"新增任务框"的顶边框拖拽手柄:向上拉放大输入框高度,
    /// 高度换算出来的像素值写回 `WorkspaceState::add_input_height`(不是
    /// `PanelDims`——它是每项目的工作树状态,不是全局布局)。基线 = 框底
    /// = 左面板区底 = 顶栏之下、footbar 之上的整段,即
    /// `window_height - footbar_height`;高度 = 基线 - 光标 y。
    TodoAddGrow,
}

/// 参与拖拽换位的四种 tab 组：顶栏项目页签、终端会话页签、预览页签、浏览器
/// 页签。`main.rs` 在拖拽中只知道"当前在拖哪个组"，据此转发光标位置；真正
/// 的换位发生在 `App::update`（持有各组的状态与纯函数算宽度的能力）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabGroup {
    Project,
    Terminal,
    Preview,
    /// Project 面板右配对的预览 tab——与 `Preview`(Files 预览)是两套独立
    /// 状态,拖拽换位不能混用,得单独一个组区分。
    ProjectPreview,
    Browser,
}

/// 正在进行的页签拖拽换位。`source` 记拖起时该组里的源下标，换位过程中源
/// 下标会随 `Vec` 移动而更新（移动后源跑到新位置，续拖以新位置为准）。
/// `press_pos` 记按下瞬间的光标位置(同 `RailDrag::press_pos` 手法)，
/// `tab_drag_move` 据此过滤"按下即松开途中的亚像素抖动"，见其文档。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabDrag {
    pub group: TabGroup,
    pub source: usize,
    pub press_pos: (f32, f32),
}

/// 页签拖拽确认阈值(同 rail 图标栏 `RAIL_DRAG_VISUAL_THRESHOLD_PX`)：按下
/// 瞬间到当前光标的位移必须越过这个半径才算"确认是一次拖拽换位"，见
/// `tab_drag_past_threshold` 用法处的文档。
const TAB_DRAG_CONFIRM_THRESHOLD_PX: f32 = 4.0;

/// `drag.press_pos` 到 `cursor` 的位移是否已越过 [`TAB_DRAG_CONFIRM_THRESHOLD_PX`]。
/// 页签(4px 间距)比 rail 图标栏排得更紧——`select_tab`/`preview_select_tab`
/// 等"按下即武装拖拽"的调用点(见 `Message::SelectTab` 文档)本身没问题，
/// 但 `tab_drag_move` 此前对**任何** `on_move`（哪怕只挪了半个像素）都直接
/// 执行换位 + `rekey_hover_range`：触控板等高灵敏输入下，单击落点到抬起
/// 之间的亚像素抖动偶尔会越界到邻居页签的命中框，触发一次肉眼不可见的
/// "拖拽"，把正被按住那个页签的 hover 光效错挪到邻居页签上——观感上就是
/// 两个页签同时像被选中(2026-09-04 用户反馈截图：点击切换 tab 后出现两个
/// 高亮页签，且无拖拽意图、偶发)。`tab_drag_move` 现在先过这道阈值，真正
/// 的拖拽(持续位移必然越界)不受影响，普通点击的抖动不再触发换位。
fn tab_drag_past_threshold(press_pos: (f32, f32), cursor: (f32, f32)) -> bool {
    let dx = cursor.0 - press_pos.0;
    let dy = cursor.1 - press_pos.1;
    dx * dx + dy * dy > TAB_DRAG_CONFIRM_THRESHOLD_PX * TAB_DRAG_CONFIRM_THRESHOLD_PX
}

/// 文件树内拖拽确认阈值。最初照抄 `TAB_DRAG_CONFIRM_THRESHOLD_PX` 的 4px,
/// 但 tab/rail 是紧凑排列的小控件,4px 越界就意味着真的碰到邻居;树行是
/// 整行高的目标,trackpad 上一次认真点击(尤其是刻意放慢、想点准的那种)
/// 本身就可能带着好几像素的手指位移,4px 太容易被"只是想点一下"的正常
/// 点击越过——2026-09 用户实测反馈(第二轮):做了 Pending/Dragging 两阶段
/// 拆分后仍然"点一下就进入拖拽态",且连带累及点击本身(左侧 `>` 图标点了
/// 展开不了目录,因为松开时 `confirmed` 被误判成 true,走的是"拖拽失败"
/// 分支而不是"这其实是单击"分支,见 `Message::TreeDragEnd` 文档)。调大到
/// 12px,量级对齐"确实移到另一行附近"而不是"点按的手指晃了几像素"。
const TREE_DRAG_CONFIRM_THRESHOLD_PX: f32 = 12.0;

/// `drag.press_pos` 到 `cursor` 的位移是否已越过 [`TREE_DRAG_CONFIRM_THRESHOLD_PX`]。
/// `files::Message::TreeRowPress`(见其文档)"按下即武装"是同一个已知会
/// 抖动的模式(同 `tab_drag_past_threshold` 修的那个 bug):单击落点到抬起
/// 之间的亚像素抖动偶尔会越界到邻居目录行的命中框,`TreeDragOver` 若不设
/// 这道门槛会把这次抖动当成"拖到了旁边那一行",直接判定合法性并可能提交
/// 移动——2026-09 用户实测反馈:点一下目录就报"不能把目录移到它自己或其
/// 子目录里"。`App::update` 收到 `TreeDragOver` 时先过这道阈值再转发给
/// `files::update()`,未越过就整条丢弃,`drag.target` 保持原值不变。
fn tree_drag_past_threshold(press_pos: (f32, f32), cursor: (f32, f32)) -> bool {
    let dx = cursor.0 - press_pos.0;
    let dy = cursor.1 - press_pos.1;
    dx * dx + dy * dy > TREE_DRAG_CONFIRM_THRESHOLD_PX * TREE_DRAG_CONFIRM_THRESHOLD_PX
}

/// 树内拖拽最短按住时长。2026-09 用户实测反馈(带诊断日志实锤):在
/// trackpad 上快速点两下目录,第二下按下到松开之间光标真的划出了 ~32px
/// (远超 [`TREE_DRAG_CONFIRM_THRESHOLD_PX`])——纯距离阈值挡不住"快速
/// 划动"这种真实位移但并非有意拖拽的手势。真正想把文件拖到某个目录、
/// 看着落点高亮再松手,耗时天然比一次快速点按长得多,加一道时长门槛作为
/// 距离阈值之外的第二重确认。200ms 在第二轮反馈里仍偏紧——刻意放慢、想
/// 点准的一次单击也可能超过 200ms,调到 300ms 留更多余量,真实拖拽(移动
/// 到目标目录、看着高亮再松手)耗时远不止于此。
const TREE_DRAG_MIN_HOLD_DURATION: std::time::Duration = std::time::Duration::from_millis(300);

/// 按下到松开的 `elapsed` 是否已达到 [`TREE_DRAG_MIN_HOLD_DURATION`]——同
/// `tree_drag_past_threshold` 一起、两者都满足才判定为一次真实拖拽(见
/// `Message::TreeDragEnd` 的 `confirmed` 文档),缺一不可:纯距离挡不住
/// 快速划动的误判,纯时长又会让"按住不动很久"被误判成拖拽却没有合法
/// 目标。
fn tree_drag_held_long_enough(elapsed: std::time::Duration) -> bool {
    elapsed > TREE_DRAG_MIN_HOLD_DURATION
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

/// 配对视图内部"列表侧"与"内容侧"的宽度,按 `split`(列表侧占比)从
/// `pair_w` 分出。四个左右双栏 zone(左:文件/项目,右:Agent/对话)统一走
/// 这一份公式:`split` 恒代表列表侧占比,内容侧拿剩下的 `1 - split`。
/// 渲染/几何/预览 bounds 三侧都要共用,不许各写各的 `pair_w * split` /
/// `pair_w * (1.0 - split)`,否则一处改动、别处漂移(见 `pair_content_width`
/// 同条原则)。
fn pair_list_content_width(pair_w: f32, split: f32) -> (f32, f32) {
    (pair_w * split, pair_w * (1.0 - split))
}

/// 配对视图内 list/content 两列相对**所在 zone/盒子内容区左边界**的
/// 横向偏移与宽度(不含 zone/盒子自己的 x0——调用方自己加)。`mirrored`
/// = false 时 list 在前(x=0)、content 在后(x=list_w+divider_width);
/// `mirrored` = true 时反过来。`preview_content_bounds_for`/
/// `left_files_tree_bounds_for`/`is_in_preview_column` 三个函数(webview
/// 矩形、文件树命中、焦点路由)都靠这一份算,不许各写各的偏移公式。
pub(crate) struct PairColumns {
    pub(crate) list_x: f32,
    pub(crate) list_w: f32,
    pub(crate) content_x: f32,
    pub(crate) content_w: f32,
}

pub(crate) fn pair_columns(pair_w: f32, split: f32, mirrored: bool) -> PairColumns {
    let (list_w, content_w) = pair_list_content_width(pair_w, split);
    let divider = byteui::theme::geometry::divider_width();
    if mirrored {
        PairColumns {
            content_x: 0.0,
            content_w,
            list_x: content_w + divider,
            list_w,
        }
    } else {
        PairColumns {
            list_x: 0.0,
            list_w,
            content_x: list_w + divider,
            content_w,
        }
    }
}

#[cfg(test)]
mod pair_columns_tests {
    use super::*;

    #[test]
    fn not_mirrored_puts_list_first() {
        let c = pair_columns(600.0, 0.4, false);
        assert_eq!(c.list_x, 0.0);
        assert!(c.content_x > c.list_x + c.list_w);
    }

    #[test]
    fn mirrored_puts_content_first() {
        let c = pair_columns(600.0, 0.4, true);
        assert_eq!(c.content_x, 0.0);
        assert!(c.list_x > c.content_x + c.content_w);
    }

    #[test]
    fn list_and_content_widths_sum_to_pair_width_regardless_of_mirror() {
        let pair_w = 600.0;
        let a = pair_columns(pair_w, 0.4, false);
        let b = pair_columns(pair_w, 0.4, true);
        assert!((a.list_w + a.content_w - pair_w).abs() < 0.01);
        assert_eq!(a.list_w, b.list_w);
        assert_eq!(a.content_w, b.content_w);
    }
}

/// `Files`/`Project` 两个 webview 面板在左右两侧同时活跃时,id 空间必须
/// 靠 `PROJECT_PREVIEW_ID_OFFSET` 隔离(否则 `ws.preview` 与
/// `ws.project_preview` 各自 `next_id` 从 0 起数,同进一个池会撞)。Task 3
/// 引入的核心不变量,拆开单测锁住。
#[cfg(test)]
mod preview_desired_concurrent_tests {
    use super::*;

    #[test]
    #[allow(clippy::eq_op, clippy::assertions_on_constants, clippy::identity_op)]
    fn project_id_offset_keeps_ids_disjoint_from_files() {
        assert!(PROJECT_PREVIEW_ID_OFFSET > 0);
        let project_id = 0 + PROJECT_PREVIEW_ID_OFFSET;
        assert_ne!(project_id, 0usize);
        assert!(
            PROJECT_PREVIEW_ID_OFFSET > 100_000,
            "off量级应远超真实 tab 数,才不会反向撞回 ws.preview 的 id"
        );
    }
}

/// 给定面板当前所在栏(不是默认栏,是"当前"——`RailLayout` 实时查),
/// 算出这条分割线要用哪个 zone 的横向基准(x0)与可分配宽度。左栏基准是
/// `icon_rail_width()`(从窗口左沿量),右栏基准是"窗口宽 - 右图标栏宽 -
/// 右区宽"(从窗口左沿量到右区左边界,同现有 `RightPairSplit` 分支已经
/// 在用的 `right_x0` 算法,这里把它提出来给两侧共用)。
pub(crate) fn pair_x0_and_width(side: Side, window_width: f32, state: &ShellState) -> (f32, f32) {
    match side {
        Side::Left => (
            byteui::theme::geometry::icon_rail_width(),
            pair_content_width(left_zone_width(window_width, state)),
        ),
        Side::Right => {
            let right_w = right_zone_width(window_width, state);
            (
                window_width - byteui::theme::geometry::icon_rail_width() - right_w,
                pair_content_width(right_w),
            )
        }
    }
}

/// 给定面板默认(未镜像)态下"列表侧是否渲染在前(pair 内第一个元素,
/// 几何上更靠左)"与当前是否处于镜像态,算出"列表侧现在是否渲染在前"。
/// `apply_column_drag` 算出的 `ratio` 恒是"pair 内第一个元素的宽度占比"
/// (鼠标左侧的宽度 / pair 总宽)——只有列表侧现在确实渲染在前时,
/// `ratio` 才能直接当"列表侧占比"写回 split 字段;渲染在后时要写
/// `1.0 - ratio`。
fn list_rendered_first(default_list_first: bool, mirrored: bool) -> bool {
    default_list_first != mirrored
}

/// 给定 `PanelKind`,取它在 `PanelDims` 里对应的配对分割比例字段——统一
/// 口径是"pair 内第一个 slot 的占比"(`pair_list_content_width` 的
/// `split` 参数,不区分这个 slot 语义上是"列表"还是"内容",Browser 的
/// `browser_bookmarks_split` 反着命名也是同一套算法)。
fn pair_split_ratio(dims: &PanelDims, kind: PanelKind) -> Option<f32> {
    match kind {
        PanelKind::Files => Some(dims.files_split),
        PanelKind::Project => Some(dims.project_split),
        PanelKind::Ssh => Some(dims.ssh_split),
        PanelKind::Database => Some(dims.database_split),
        PanelKind::Todo => Some(dims.todo_split),
        PanelKind::GitLog => Some(dims.git_log_split),
        PanelKind::Web => Some(dims.browser_bookmarks_split),
        PanelKind::Agent => Some(dims.agent_split),
        PanelKind::Conversations => Some(dims.conversations_split),
        PanelKind::Usage => Some(dims.usage_split),
    }
}

/// `pair_split_ratio` 的写入侧。
fn with_pair_split_ratio(dims: PanelDims, kind: PanelKind, ratio: f32) -> PanelDims {
    match kind {
        PanelKind::Files => PanelDims {
            files_split: ratio,
            ..dims
        },
        PanelKind::Project => PanelDims {
            project_split: ratio,
            ..dims
        },
        PanelKind::Ssh => PanelDims {
            ssh_split: ratio,
            ..dims
        },
        PanelKind::Database => PanelDims {
            database_split: ratio,
            ..dims
        },
        PanelKind::Todo => PanelDims {
            todo_split: ratio,
            ..dims
        },
        PanelKind::GitLog => PanelDims {
            git_log_split: ratio,
            ..dims
        },
        PanelKind::Web => PanelDims {
            browser_bookmarks_split: ratio,
            ..dims
        },
        PanelKind::Agent => PanelDims {
            agent_split: ratio,
            ..dims
        },
        PanelKind::Conversations => PanelDims {
            conversations_split: ratio,
            ..dims
        },
        PanelKind::Usage => PanelDims {
            usage_split: ratio,
            ..dims
        },
    }
}

/// 拖拽某条分隔线到窗口逻辑 x 坐标 `logical_x` 后的新 `ShellLayout`。
/// `LeftPairSplit`/`RightPairSplit` 写哪个 split 字段取决于当前那一侧的
/// 视图选择(比如右侧当前是"对话"就写 `conversations_split`，不是
/// `agent_split`)——这条信息 `ShellLayout` 自己没有，靠 `ShellState` 带过来。
pub(crate) fn apply_column_drag(
    state: ShellState,
    divider: Divider,
    window_width: f32,
    logical_x: f32,
) -> PanelDims {
    match divider {
        Divider::LeftRight => {
            let new_left = clamp_left_width(
                window_width,
                logical_x - byteui::theme::geometry::icon_rail_width(),
            );
            let mut new_dims = PanelDims {
                left_width: new_left,
                ..state.dims
            };
            // 拖外层 zone 分隔线时,连带补偿左右两侧当前活跃的配对面板的
            // 分割比例,让"拖内层分隔线定下来的那个像素宽度"保持不变——
            // 否则比如文件树这类列表面板会跟着 zone 宽度重新按比例缩放,
            // 用户明明只是在调整左右两个 zone 的分界,没碰过文件树自己的
            // 分隔线(2026-08-29 用户反馈)。用新 `left_width` 探测两侧新
            // pair 宽,按"旧像素宽 / 新 pair 宽"反解新比例,clamp 到合法
            // 区间兜底(pair 宽被压得极窄时不会算出离谱的比例)。
            let new_probe = ShellState {
                dims: new_dims,
                ..state.clone()
            };
            for (kind, side) in [
                (
                    state.left_view,
                    state.layout.rail_layout.side_of(state.left_view),
                ),
                (
                    state.right_view,
                    state.layout.rail_layout.side_of(state.right_view),
                ),
            ] {
                let Some(old_ratio) = pair_split_ratio(&state.dims, kind) else {
                    continue;
                };
                let (_, old_pair_w) = pair_x0_and_width(side, window_width, &state);
                let (_, new_pair_w) = pair_x0_and_width(side, window_width, &new_probe);
                if old_pair_w <= 0.0 || new_pair_w <= 0.0 {
                    continue;
                }
                let fixed_px = old_pair_w * old_ratio;
                let new_ratio = (fixed_px / new_pair_w).clamp(
                    byteui::theme::geometry::min_split_ratio(),
                    byteui::theme::geometry::max_split_ratio(),
                );
                new_dims = with_pair_split_ratio(new_dims, kind, new_ratio);
            }
            new_dims
        }
        Divider::LeftPairSplit => {
            let side = state.layout.rail_layout.side_of(PanelKind::Files);
            let (x0, pair_w) = pair_x0_and_width(side, window_width, &state);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let raw_ratio = ((logical_x - x0) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            let mirrored = side != PanelKind::Files.default_side();
            let ratio = if list_rendered_first(true, mirrored) {
                raw_ratio
            } else {
                1.0 - raw_ratio
            };
            PanelDims {
                files_split: ratio,
                ..state.dims
            }
        }
        Divider::ProjectSplit => {
            let side = state.layout.rail_layout.side_of(PanelKind::Project);
            let (x0, pair_w) = pair_x0_and_width(side, window_width, &state);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let raw_ratio = ((logical_x - x0) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            let mirrored = side != PanelKind::Project.default_side();
            let ratio = if list_rendered_first(true, mirrored) {
                raw_ratio
            } else {
                1.0 - raw_ratio
            };
            PanelDims {
                project_split: ratio,
                ..state.dims
            }
        }
        Divider::SshSplit => {
            let side = state.layout.rail_layout.side_of(PanelKind::Ssh);
            let (x0, pair_w) = pair_x0_and_width(side, window_width, &state);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let raw_ratio = ((logical_x - x0) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            let mirrored = side != PanelKind::Ssh.default_side();
            let ratio = if list_rendered_first(true, mirrored) {
                raw_ratio
            } else {
                1.0 - raw_ratio
            };
            PanelDims {
                ssh_split: ratio,
                ..state.dims
            }
        }
        Divider::DatabaseSplit => {
            let side = state.layout.rail_layout.side_of(PanelKind::Database);
            let (x0, pair_w) = pair_x0_and_width(side, window_width, &state);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let raw_ratio = ((logical_x - x0) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            let mirrored = side != PanelKind::Database.default_side();
            let ratio = if list_rendered_first(true, mirrored) {
                raw_ratio
            } else {
                1.0 - raw_ratio
            };
            PanelDims {
                database_split: ratio,
                ..state.dims
            }
        }
        Divider::UsageSplit => {
            let side = state.layout.rail_layout.side_of(PanelKind::Usage);
            let (x0, pair_w) = pair_x0_and_width(side, window_width, &state);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let raw_ratio = ((logical_x - x0) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            let mirrored = side != PanelKind::Usage.default_side();
            // 用量面板默认"内容在前、agent 筛选栏在后"(同 Agent/Conversations
            // 的 `RightPairSplit`,`default_list_first = false`),翻转方向
            // 跟 Database/Ssh 等"列表在前"的面板相反。
            let ratio = if list_rendered_first(false, mirrored) {
                raw_ratio
            } else {
                1.0 - raw_ratio
            };
            PanelDims {
                usage_split: ratio,
                ..state.dims
            }
        }
        Divider::TodoSplit => {
            let side = state.layout.rail_layout.side_of(PanelKind::Todo);
            let (x0, pair_w) = pair_x0_and_width(side, window_width, &state);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let raw_ratio = ((logical_x - x0) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            let mirrored = side != PanelKind::Todo.default_side();
            let ratio = if list_rendered_first(true, mirrored) {
                raw_ratio
            } else {
                1.0 - raw_ratio
            };
            PanelDims {
                todo_split: ratio,
                ..state.dims
            }
        }
        Divider::GitLogSplit => {
            let side = state.layout.rail_layout.side_of(PanelKind::GitLog);
            let (x0, pair_w) = pair_x0_and_width(side, window_width, &state);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let raw_ratio = ((logical_x - x0) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            let mirrored = side != PanelKind::GitLog.default_side();
            let ratio = if list_rendered_first(true, mirrored) {
                raw_ratio
            } else {
                1.0 - raw_ratio
            };
            PanelDims {
                git_log_split: ratio,
                ..state.dims
            }
        }
        Divider::BrowserBookmarksSplit => {
            let side = state.layout.rail_layout.side_of(PanelKind::Web);
            let (x0, pair_w) = pair_x0_and_width(side, window_width, &state);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let raw_ratio = ((logical_x - x0) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            let mirrored = side != PanelKind::Web.default_side();
            // browser_bookmarks_split 存的是"内容占比"(Web 唯一反着命名
            // 的字段),content 默认渲染在前,所以这里 `list_rendered_first`
            // 的 `default_list_first` 参数传 `false`(不是 `true`)——
            // "list" 这个泛化概念在 Web 这里对应收藏夹侧栏,不是内容。
            // 翻转方向和 Files/Project 相反,写反会收藏夹拖拽方向错乱。
            let ratio = if list_rendered_first(false, mirrored) {
                1.0 - raw_ratio
            } else {
                raw_ratio
            };
            PanelDims {
                browser_bookmarks_split: ratio,
                ..state.dims
            }
        }
        Divider::RightPairSplit => {
            let kind = state.right_view; // Agent 或 Conversations
            let side = state.layout.rail_layout.side_of(kind);
            let (x0, pair_w) = pair_x0_and_width(side, window_width, &state);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let raw_ratio = ((logical_x - x0) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            let mirrored = side != kind.default_side();
            // Agent/Conversations 默认"内容在前"(default_list_first = false),
            // 和 Task 5 三个面板相反。默认栏(`mirrored = false`)下
            // `list_rendered_first(false, false) = false`,走 `1.0 - raw_ratio`
            // 这条分支,和改造前的固定行为逐字节一致(防回归)。
            let ratio = if list_rendered_first(false, mirrored) {
                raw_ratio
            } else {
                1.0 - raw_ratio
            };
            match kind {
                PanelKind::Agent => PanelDims {
                    agent_split: ratio,
                    ..state.dims
                },
                PanelKind::Conversations => PanelDims {
                    conversations_split: ratio,
                    ..state.dims
                },
                // 用量统计是单栏（不分割）,没有自己的 split 权重。
                PanelKind::Usage => state.dims,
                _ => unreachable!(
                    "RightPairSplit 只会在 state.right_view 是 Agent/Conversations/\
                     Usage 之一时出现——Stage 1 遗留的兜底,这里维持"
                ),
            }
        }
    }
}

/// `apply_column_drag` 的纵向镜像:按 `logical_y`/`window_height` 算比例。
/// "可用高度"用近似估算(粗略减去顶栏/footbar 这类固定装饰高度)——Git Log
/// 面板内部标题/worktree 条的精确高度不在这里计算,内核不关心面板内部布局
/// 细节,只提供窗口级的粗略换算;像素级对齐精度不足时人工验收阶段允许
/// 后续单独调整这个估算值。
pub(crate) fn apply_row_drag(
    state: ShellState,
    divider: RowDivider,
    window_height: f32,
    logical_y: f32,
) -> PanelDims {
    match divider {
        RowDivider::GitLogFileDiffSplit => {
            let usable_height =
                (window_height - byteui::theme::geometry::status_bar_height() * 2.0).max(1.0);
            let ratio = (logical_y / usable_height).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            PanelDims {
                git_log_file_diff_split: ratio,
                ..state.dims
            }
        }
        // `TodoAddGrow` 的高度换算不走这套 `PanelDims`(它落在 `ws.todo`),
        // 由 `update` 的 `RowDrag` 分支单独处理。`apply_row_drag` 只会被
        // `GitLogFileDiffSplit` 调用,这里给个兜底。
        RowDivider::TodoAddGrow => state.dims,
    }
}

/// 放大态金色描边盒子在窗口坐标系里的横向范围 (x0, 可用宽度)。放大左侧
/// 还是右侧都是同一个盒子(`maximize_overlay` 的 dim_bg 铺满两条图标栏
/// 之间,`bordered` 再铺满其内边距之内),所以这一份公式两侧共用:三层留白
/// 累加 = 图标栏宽 + `maximize_overlay` 里 dim_bg 的内边距——`bordered`
/// 容器本身无内边距、宽度铺满,所以到这里为止。
/// `preview_content_bounds_for`/`is_in_preview_column`/`terminal_pane_pixel_size`
/// 都靠它换算放大态几何,不能各写各的字面量,否则和 `maximize_overlay`
/// 实际渲染的画面对不上。
pub(crate) fn maximized_box_x_range(window_width: f32) -> (f32, f32) {
    let x0 = byteui::theme::geometry::icon_rail_width()
        + byteui::theme::geometry::maximize_overlay_padding();
    let avail_w = (window_width
        - 2.0 * byteui::theme::geometry::icon_rail_width()
        - 2.0 * byteui::theme::geometry::maximize_overlay_padding())
    .max(0.0);
    (x0, avail_w)
}

/// 放大态金色描边盒子的纵向可用高度(逻辑像素)。`maximize_overlay` 顶部
/// 垫了一条 `byteui::theme::geometry::top_bar_height()` 高的 Space 把遮罩钉在顶栏之下,盒子上下各留
/// `byteui::theme::geometry::maximize_overlay_padding()`;遮罩铺到窗口底边(状态栏也被盖住),所以这里
/// **不**扣 `byteui::theme::geometry::status_bar_height()`——与 `preview_content_bounds` 放大分支同源。
pub(crate) fn maximized_box_height(window_height: f32) -> f32 {
    (window_height
        - byteui::theme::geometry::top_bar_height()
        - 2.0 * byteui::theme::geometry::maximize_overlay_padding())
    .max(0.0)
}

/// 逻辑 x 落在哪一侧面板区(整区,不分区内具体是哪个 pane)。左键点击
/// 落点决定当前"聚焦"哪一侧,驱动 `left_zone`/`right_zone` 外边框的高亮态
/// (见 [`ZoneSide`])。落在图标栏本身(两侧各 `byteui::theme::geometry::icon_rail_width()` 宽)或
/// 某侧收起而点在了"不存在的那一侧"时不算数,返回 `None`(调用方应保持
/// 点击前的聚焦态不变,而不是清空)。
///
/// 放大态:整个内容区就是放大的那一侧,不用再按横坐标细分——
/// `maximize_overlay` 渲染时两条图标栏原样露在外面,和非放大态同一
/// 横向范围,所以图标栏判定不用跟着改。
pub fn zone_at_x(x: f32, window_width: f32, state: &ShellState) -> Option<ZoneSide> {
    if x < byteui::theme::geometry::icon_rail_width()
        || x > window_width - byteui::theme::geometry::icon_rail_width()
    {
        return None;
    }
    if let Some(which) = state.maximized {
        return Some(match which {
            MaximizedPane::Left => ZoneSide::Left,
            MaximizedPane::Right => ZoneSide::Right,
        });
    }
    if state.left_collapsed {
        return if state.right_collapsed {
            None
        } else {
            Some(ZoneSide::Right)
        };
    }
    if state.right_collapsed {
        return Some(ZoneSide::Left);
    }
    let boundary =
        byteui::theme::geometry::icon_rail_width() + left_zone_width(window_width, state);
    Some(if x < boundary {
        ZoneSide::Left
    } else {
        ZoneSide::Right
    })
}

/// 终端 pane 此刻是否真的呈现在用户眼前。键盘输入(`Message::TermInput`)
/// 必须以此为闸门:右侧收起、右视图切到对话、或左侧被放大(右半被变暗遮罩
/// 整片盖住)时,敲下的回车/方向键会静默提交给一个看不见的 agent 会话——这条
/// 在旧四栏布局里不存在(终端恒在屏上),是新外壳带出来的新风险
/// (Fix round 2 #3)。放大的正是右侧时终端**是**可见的(只是更大),算可见。
pub(crate) fn terminal_visible(state: &ShellState) -> bool {
    state.right_view == PanelKind::Agent
        && !state.right_collapsed
        && state.maximized != Some(MaximizedPane::Left)
}

/// SSH 面板内嵌终端此刻是否真的呈现在用户眼前——镜像 `terminal_visible`,
/// 判定对象换成左侧:SSH 面板必须是当前左视图,且没有被"右侧放大"盖住
/// (角色与 `terminal_visible` 的 `MaximizedPane::Left` 判断对调:终端在
/// 右、被左侧放大遮住;SSH 面板在左、被右侧放大遮住)。
pub(crate) fn ssh_terminal_visible(state: &ShellState) -> bool {
    state.left_view == PanelKind::Ssh && state.maximized != Some(MaximizedPane::Right)
}

/// 换算终端 PTY 网格时用的假想外壳状态:强制"右侧展开 + 显示 Agent 配对"。
///
/// 终端此刻可能不可见(右侧收起 / 右视图是对话),但它的 PTY 网格仍应按
/// "被显示时占多大"来定——否则上次退出前停在对话视图的会话,重开 app 后会
/// 一直卡在 `DEFAULT_COLS`×`DEFAULT_ROWS`(80×24),直到用户偶然拖一下窗口
/// 才纠正(Fix round 2 #6)。用字段覆盖表达这个假想,复用同一套宽度公式,
/// 不另写一份几何。
pub(crate) fn terminal_grid_state(state: ShellState) -> ShellState {
    // `maximized == Some(Right)` 只有在真实 `right_view` 本来就是 `Agent`
    // 时才代表"终端被放大"——对话视图下点"放大"是放大审阅 pane
    // (`review_content_pane` 自己的放大按钮),不是终端。不做这个过滤会让
    // "对话视图下放大审阅"被这里误判成"终端被放大",按放大格算出一个终端
    // 实际不可见、也不是那个尺寸的网格,给所有存活 PTY 发一次错的 SIGWINCH
    // (Fix round 3,scoped re-review 发现)。
    let maximized = state
        .maximized
        .filter(|m| *m != MaximizedPane::Right || state.right_view == PanelKind::Agent);
    ShellState {
        right_collapsed: false,
        right_view: PanelKind::Agent,
        maximized,
        ..state
    }
}

/// 窗口整体逻辑像素尺寸 → 终端 pane 的可用像素尺寸。终端只在右侧视图是
/// `Agent` 且未收起时可见；否则返回零尺寸(调用方在这种情况下本就不会真的
/// 用这个尺寸去 resize 一个不可见的终端，返回零是安全兜底；要按"若显示则
/// 多大"换算 PTY 网格的场合见 `Workspace::sync_terminal_grid`)。
///
/// 放大态(Fix round 2 #6):右侧被放大时终端所在的整个右面板区被
/// `maximize_overlay` 渲染成金色描边盒子那么大,网格必须跟着变大,否则
/// "放大终端"只放大了外框、字符网格还是放大前那么小(放大等于白放)。
/// 左侧被放大时右面板区在遮罩之下、几何不变,沿用常规分支。
pub fn terminal_pane_pixel_size(
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32) {
    if state.right_collapsed || state.right_view != PanelKind::Agent {
        return (0.0, 0.0);
    }
    if state.maximized == Some(MaximizedPane::Right) {
        let (_x0, avail_w) = maximized_box_x_range(window_width);
        let (_list_w, content_w) =
            pair_list_content_width(pair_content_width(avail_w), state.dims.agent_split);
        let pane_width = (content_w - byteui::theme::geometry::chrome_width_px()).max(0.0);
        let pane_height = (maximized_box_height(window_height)
            - byteui::theme::geometry::chrome_height_px())
        .max(0.0);
        return (pane_width, pane_height);
    }
    let right_w = right_zone_width(window_width, state);
    let (_list_w, content_w) =
        pair_list_content_width(pair_content_width(right_w), state.dims.agent_split);
    let pane_width = (content_w - byteui::theme::geometry::chrome_width_px()).max(0.0);
    // `right_zone` 上下 margin:终端是 iced 布局(自动 inset),但其 PTY 网格
    // 尺寸靠这里算,必须同步扣掉上下 margin,否则字符网格比实际渲染区高。
    //
    // 不扣 `status_bar_height()`:终端 pane 自带的底栏(`terminal_status_bar`)
    // 已经按要求去掉(`terminal.rs::terminal_pane` 不再往 column 里塞状态栏
    // 元素),这里之前仍在扣这块高度是遗留的死重——真实渲染区比这个公式
    // 算出来的整整多一条状态栏那么高,PTY 网格因此比实际可见区少了几行，
    // 造成终端 pane 底部有一截真实存在、但 PTY 不知道的空白，v8agent 自己
    // 画的状态栏一旦跨越这条边界（内容一多、发生了 resize 之后）就会跟真实
    // 渲染错位。SSH 终端那边的姊妹函数 `ssh_terminal_pane_pixel_size` 从来
    // 没有这一条减法，这条本该在状态栏移除时一起删掉。
    let m = theme::region::right_zone().margin;
    let pane_height = (window_height
        - byteui::theme::geometry::top_bar_height()
        - byteui::theme::geometry::chrome_height_px()
        - m.top
        - m.bottom)
        .max(0.0);
    (pane_width, pane_height)
}

/// 窗口整体逻辑像素尺寸 → SSH 面板内嵌终端 pane 的可用像素尺寸。
///
/// 与 `terminal_pane_pixel_size`(右侧共享终端)不同,SSH 终端挂在**左面板区**:
/// 左栏`主机列表 | 内嵌终端`配对里,终端拿 `pair_content_width(left_w) *
/// (1 - ssh_split)`(镜像 `left_panel_area` 里 `ssh::view` 的 `FillPortion`
/// 布局与 `Divider::SshSplit` 拖拽,几何只此一份真源)。它的网格必须按这块
/// pane 的实际宽度换算,否则 SSH 终端会沿用共享终端的列数,窗口/分隔条一
/// 拖动就跟不上、字符折行错乱。
///
/// 左面板区收起时返回零尺寸(此时 SSH 终端不可见,调用方不 resize)。
pub fn ssh_terminal_pane_pixel_size(
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32) {
    let left_w = left_zone_width(window_width, state);
    if left_w <= 0.0 {
        return (0.0, 0.0);
    }
    let pair_w = pair_content_width(left_w);
    let (_list_w, content_w) = pair_list_content_width(pair_w, state.dims.ssh_split);
    let pane_width = (content_w - byteui::theme::geometry::chrome_width_px()).max(0.0);
    // `chrome_height_px()`(tab 栏 + padding + spacing 的估算)与
    // `terminal_pane_pixel_size` 同源;两边现在都不扣 `status_bar_height()`
    // ——共享终端自带的 `terminal_status_bar` 早已按要求去掉，两个函数的
    // 高度公式形状一致，不是"SSH 特例更简单"。
    let m = theme::region::left_zone().margin;
    let pane_height = (window_height
        - byteui::theme::geometry::top_bar_height()
        - byteui::theme::geometry::chrome_height_px()
        - m.top
        - m.bottom)
        .max(0.0);
    (pane_width, pane_height)
}

/// 一批**异步结果**消息共同的首个字段:它们归属哪个项目。
///
/// 为什么必须显式带上、不能"投给当时聚焦的那个项目"(P2a Task 7 fix round 1
/// 的 Critical):`SessionTab::tab_id` 来自每个 `Workspace` 自己的
/// `next_tab_id`,**每个项目都从 0 起编**。多项目并行之后"项目 A 和项目 B
/// 各有活着的会话"是常态而不是边角情况,两边的第一个 tab 都是 id 0。若按
/// `App::with_focused_project`(投给当前聚焦项目)路由,后台项目 A 的 agent 每吐一次输出,
/// `TermOutput(0, ..)` 就会被喂进前台项目 B 的 tab 0 里——这不是竞态窗口,
/// 是只要两个项目都有会话就持续发生。`SessionExited` 会把错误的 tab 标成
/// 已死,`TabAttached` 会让 B 认领本属于 A 的会话。
///
/// 所以凡是"发起时就已知归属项目、结果晚些才回来"的消息,一律带上
/// `project_id`,由 [`App::with_project`] 直接投递到对应槽位;投递不到
/// (项目已被关掉/还没促成)就静默丢弃。反过来,由用户点击当前界面直接
/// 触发的消息(`TermInput`/`PreviewSelectTab` …)仍然走
/// `with_focused_project`——它们的语义本来就是"作用于用户此刻看着的那个项目"。
pub type ProjectId = i64;

/// Ctrl + / Ctrl - 每次触发的相对缩放步近因子（1.1 ≈ 每按一次放大 10%）。
const UI_ZOOM_STEP: f32 = 1.1;

#[derive(Debug, Clone)]
pub enum Message {
    /// 终端聚焦时的键盘/IME 输入字节（已经过 `keymap` 翻译）。直接写给
    /// 当前激活 tab 对应的 daemon 会话（`client.write`）——不再本地
    /// echo，回显完全走 PTY 真实回路（daemon → attach 流 → `TermOutput`）。
    TermInput(terminal::TermTarget, Vec<u8>),
    /// IME 组字预览(未提交):`None` 表示组字结束/取消,清空预览。不发字节
    /// 给 PTY——只是渲染层叠加,`term_view` 画在光标位置(见其 `draw`)。
    TermImePreedit(terminal::TermTarget, Option<String>),
    /// attach 事件流转发来的输出字节，`usize` 是 tab 的稳定 id
    /// （`SessionTab::tab_id`，不是 vec 位置——关闭 tab 会移动位置，
    /// 但 id 不变，事件流路由必须认 id）。首字段的项目归属见 [`ProjectId`]
    /// ——`tab_id` 只在单个项目内唯一,跨项目会撞。
    TermOutput(ProjectId, usize, Vec<u8>),
    /// 对应 tab 的会话已退出（PTY 子进程退出或 daemon 断连）。
    SessionExited(ProjectId, usize),
    /// attach 流转发来的 agent 状态变更（tab_id, 状态, 该会话最新 transcript 路径）。
    AgentStateChanged(ProjectId, usize, AgentKind, AgentState, Option<String>),
    /// hook 事件驱动的 Agent 卡片元信息刷新结果(tab_id, LLM 型号,
    /// permission mode,transcript 最后活动摘要(兜底"当前工作内容",见
    /// `workspace::agent_card`),工作区分支/脏标覆盖)。`None` 字段表示
    /// 这次没有新值,落地时不覆盖已有值(见
    /// `workspace::apply_agent_card_refresh`)。
    AgentCardRefreshed(
        ProjectId,
        usize,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<crate::workspace::WorkspaceGitInfo>,
    ),
    /// 会话审阅:解析完成（来源, 追加标记, 条目 / 错误文案）。`append`
    /// 为 `true` 时新条目应追加进现有 `entries`(详情"加载更多"),`false`
    /// 时整段替换(首次打开/活会话刷新)。
    ReviewLoaded(
        ProjectId,
        ReviewSource,
        bool,
        Result<Vec<ReviewEntry>, String>,
    ),
    /// 对话面板会话列表刷新结果:当前项目全部 session(联查总结),按最后
    /// 活跃时间倒序(2026-08-27，取代按回合拍平的列表；见
    /// `Workspace::spawn_conversations_refresh`)。
    ConversationSessionsRefreshed(ProjectId, Result<Vec<SessionRow>, String>),
    /// `GetTodoDetail` 异步结果:`usize` 是打开弹窗时记录的卡片下标(用来
    /// 校验弹窗还开着同一个任务,不是用 id 找——`items()` 下标和渲染时
    /// 用的下标必须一致,同 `todo::Message` 全线用下标寻址任务的既有约定)。
    TodoDetailLoaded(usize, Vec<dozer_core::protocol::TurnRecord>),
    /// Usage 面板的全部消息,内核只转发不解读——见 `extensions::usage::Message`。
    Usage(usage::Message),
    /// 点击对话面板会话列表里的某一行——打开该 session 的详情审阅
    /// (整段回合列表 + 总结展示区)。`agent` 随行内数据一并带上。
    ConversationSessionOpen(String, AgentKind),
    /// "加载更多"追加当前 session 详情的下一页(`after_turn_index`)。
    ConversationDetailLoadMore(String, i64),
    /// 一次"删除项目"执行完成。`Vec<String>` 是文件系统步骤各自独立的
    /// 失败原因(空 = 全部成功);dozerd 侧两步(登记/agent 历史)任一失败
    /// 时这里只会收到那一条错误。项目对应的 tab 在发起删除时已经关掉,
    /// 这个消息到达时已经没有面板可以展示状态,统一走 `self.daemon_error`
    /// (同 `project_tab_opened` 失败路径的既有做法)。
    ProjectDeleteDone(Vec<String>),
    /// 对话列表面板(会话列表)点"更多..."翻页图标按钮——只把当前已缓存的
    /// `conversation_turn_groups` 往下多展开一页(`CONVERSATION_PAGE_SIZE` 条),
    /// 不问 daemon 要新数据(同 `git_log::Message::CommitListMore`)。
    ConversationListMore,
    /// 会话列表搜索框草稿变化(iced `text_input::on_input`)。
    ConversationSearchInput(String),
    /// 回车 / 点搜索按钮:把草稿落成生效的 `conversation_search` 过滤词,
    /// 同时把翻页重置回第 1 页(过滤后结果变少,停在旧页码没有意义)。
    ConversationSearchSubmit,
    /// 会话列表底部 agent 筛选下拉某一项点击:`None` = 全部。同样把翻页
    /// 重置回第 1 页,并顺带收起下拉(同 `files::Message::BranchSwitch` 选
    /// 完即收起下拉的既有语义)。
    ConversationAgentFilterSelect(Option<AgentKind>),
    /// 展开/收起会话列表底部的 agent 筛选下拉(样式对齐文件树面板的分支
    /// 切换下拉)。
    ConversationAgentPickerOpen,
    ConversationAgentPickerClose,
    /// 切换当前显示的 tab（这里的 `usize` 是 vec 位置——用户点击的是
    /// "屏幕上第几个 tab"，跟稳定 id 是两回事）。**仅限左侧终端 tab 栏本身
    /// 的按钮**发这条消息——`select_tab()` 顺带把 `tab_drag` 武装成"这一
    /// 页签正被按住",随后光标划过 tab 栏任意条目就会触发换位
    /// (`tab_drag_move`)。任何不是 tab 栏本身、但也想"选中某个 tab"的地方
    /// (比如 Agent 面板右侧卡片列表)必须发 `SelectTabNoDrag`,否则会在
    /// 无关点击后意外武装拖拽状态机,松手前只要划过 tab 栏就会错误换位
    /// (2026-08-17 修的一个真实 bug)。
    SelectTab(usize),
    /// 语义同 `SelectTab`(选中 + 路由键盘焦点),但**不武装拖拽状态机**。
    /// 给"不是 tab 栏本身、但也要切换 tab"的调用方用(目前只有 Agent 面板
    /// 右侧卡片列表 `agent_card`)。
    SelectTabNoDrag(usize),
    /// 关闭 tab = 结束会话：中断转发任务并 kill daemon 侧会话（P1e 验收
    /// 反馈裁决：重开 app 只恢复"关 app 时还开着"的 tab，已关的不还魂）。
    /// "会话存活"保的是关 app/崩溃不掉会话——退 app 才是 detach。
    CloseTab(usize),
    /// Agent 面板"＋"按钮:开/关 agent 选择菜单。
    AgentPickerToggle,
    /// agent 选择菜单:点击菜单外/Esc,关闭不建会话。
    AgentPickerClose,
    /// agent 选择菜单:选中一项(`Agent(None)` = 纯 Shell,`Agent(Some(a))`
    /// = 新建会话后自动键入该 agent 的 CLI 名字,`Git` = 项目根开 git shell)。
    AgentPickerSelect(PickerLaunch),
    /// 顶栏"＋新增项目"按钮:开/关最近项目选择菜单。
    ProjectAddMenuToggle,
    /// 最近项目选择菜单:点击菜单外/Esc,关闭不做任何事。
    ProjectAddMenuClose,
    /// 新建会话完成 attach（tab_id、`SessionInfo`、初始快照）。
    /// 启动时的恢复走同步的 `bootstrap`，不需要过一次消息循环。
    TabAttached(ProjectId, usize, SessionInfo, Vec<u8>),
    /// Todo 面板的全部消息(派发到已有/新建会话除外——那两条内核直接
    /// 拦截处理,见 `update()` 对应分支),内核只转发不解读——见
    /// `extensions::todo::Message`。
    Todo(todo::Message),
    /// 数据库面板的全部消息。`TestConnectionResult` 特化分支内核直接拦截
    /// 处理(带 `project_id`,不能按当前聚焦项目路由),其余走通配分发。
    Database(database::Message),
    /// 文件树右键"搜索"弹窗的全部消息,内核只转发不解读——见
    /// `extensions::search::Message`。`SearchResults`(带 `project_id`)按
    /// 项目路由,其余(弹窗常驻 UI 交互)投给当前聚焦项目。
    Search(search::Message),
    /// 终端 pane 像素尺寸变化换算出的新网格尺寸；对所有 tab 生效
    /// （包括当前不可见的），保证切换 tab 时尺寸已经是最新的。
    /// 共享终端 pane 与 SSH 面板内嵌终端并行重算:两个 pane 几何不同,
    /// 必须带着各自的网格一起下发,否则 SSH 终端会沿用共享终端的列数。
    PaneResized {
        cols: u16,
        rows: u16,
        ssh_cols: u16,
        ssh_rows: u16,
    },
    /// 按下某条分隔线,记录"正在拖哪条"(main.rs 后续 CursorMoved 靠这个
    /// 状态决定要不要继续转发拖拽)。构造方为 `divider_bar` 的 `on_press`。
    ColumnDragStart(Divider),
    /// 拖拽中:当前窗口逻辑宽 + 光标逻辑 x(main.rs 换算好传入,`update()`
    /// 统一算+夹取,不与 main.rs 分摊裁剪逻辑)。构造方为 main.rs 的
    /// `CursorMoved` 续传。
    ColumnDrag {
        window_width: f32,
        logical_x: f32,
    },
    /// 松开左键,结束拖拽并触发写盘。构造方为 main.rs 的
    /// `MouseInput{Released}` 分支。
    ColumnDragEnd,
    /// 按下某条纵向(上下)分隔线,记录"正在拖哪条"。构造方为
    /// `horizontal_divider_bar` 的 `on_press`。
    RowDragStart(RowDivider),
    /// 纵向拖拽中:当前窗口逻辑高 + 光标逻辑 y(main.rs 换算好传入)。
    RowDrag {
        window_height: f32,
        logical_y: f32,
    },
    /// 松开左键,结束纵向拖拽并触发写盘。
    RowDragEnd,
    /// 拖拽中,光标进入了 `group` 组的第 `index` 个 tab 上空——拖起的源项
    /// 应移动到这个目标位(换位)。构造方为该组每个 tab 顶层的
    /// `MouseArea::on_move`(仅在 `tab_drag` 命中本组时挂载)。按住页签＝
    /// 准备拖的来源,由各选中处理(`SelectTab`/`PreviewSelectTab`/
    /// `ProjectTabSwitch` 及浏览器 `SelectTab`)在按住瞬间把 `tab_drag` 置位。
    TabDragMove {
        group: TabGroup,
        index: usize,
    },
    /// 松开左键,结束页签拖拽。构造方为 main.rs 的 `MouseInput{Released}`
    /// 分支;项目页签组顺带把新顺序写盘。
    TabDragEnd,
    /// 图标栏面板拖拽,光标进入了 `side` 栏第 `index` 个位置——同栏内是
    /// 重排,跨栏是记录悬停目标。构造方为该栏每个按钮顶层的
    /// `MouseArea::on_move`(仅在 `rail_drag` 命中时挂载)。按住图标＝准备
    /// 拖的来源由 `panel_select` 在按住瞬间武装(`self.rail_drag` 置位)。
    RailDragMove {
        side: Side,
        index: usize,
    },
    /// 松开左键,结束图标栏面板拖拽。构造方为 main.rs 的
    /// `MouseInput{Released}` 分支;跨栏移动此时才提交并写盘。
    RailDragEnd,
    /// Todo 面板拖拽排序结束:松开左键,把新顺序写盘。构造方为 main.rs 的
    /// `MouseInput{Released}` 分支,同 `TabDragEnd`(页签拖拽)那套。拖拽中
    /// 的 `DragMove` 由卡片外层 `MouseArea::on_move` 直接发 `Todo::DragMove`
    /// (走 `Message::Todo` 通道),不需要顶层变体——这里只收尾。
    TodoDragEnd,
    /// 图标栏点击选中某个面板——不区分左右栏,`panel_select` 内部按
    /// `RailLayout::side_of` 查它当前挂在哪条栏。
    PanelSelect(PanelKind),
    /// 文件预览面板右上角"收起/展开文件树"按钮:翻转
    /// `dims.files_tree_collapsed`。只影响文件树列表子栏的显隐,不触碰
    /// `files_split` 比例,展开时按原比例恢复。
    ToggleFileTreeCollapse,
    /// 某两栏面板的列表列收起/展开按钮:翻转该面板对应的 `dims.*_list_collapsed`。
    /// 与 `ToggleFileTreeCollapse` 同一套语义——只改列表子栏显隐、不触碰
    /// split 比例,展开时按原宽度恢复。`PanelKind` 只能是六个两栏面板之一
    /// (Project/Todo/Database/Ssh/Agent/Conversations),不是它们则忽略。
    TogglePanelListCollapse(PanelKind),
    /// 任意 iced 原生输入框(`text_input`/`text_editor`)的右键菜单:在某输入
    /// 框上右键触发(由 byteui 的 `context_menu::wrap` 接线)。携带被右键的
    /// 输入目标,用于本次右键时把焦点移到该输入,让菜单的复制/粘贴作用于
    /// 它。定位坐标复用 `files.last_right_click`(main.rs 右键时已写入)。
    TextInputMenuOpen(TextInputTarget),
    /// 输入框右键菜单关闭(点遮罩 / 按 Esc)。
    TextInputMenuClose,
    /// 输入框右键菜单的动作项:剪切/复制/粘贴/全选。`App::update` 只负责把
    /// 菜单关掉;真正把动作作用到聚焦输入框的是 main.rs——把这些消息合成回
    /// 对应的 ⌘/Ctrl+`x`/`c`/`v`/`a` 键盘事件,喂给下一帧 `interface.update`,
    /// 复用 iced 原生的剪贴板/光标插入逻辑(见 `dispatch`/`editor`)。密码框
    /// 会禁用剪切/复制(同原生快捷键)。
    TextInputMenuCut,
    TextInputMenuCopy,
    TextInputMenuPaste,
    TextInputMenuSelectAll,
    /// 图标栏按钮 hover 进入/离开:进入带 `Some(id)`,离开带 `None`,
    /// 任意按钮的 hover 进入/离开:带按钮标识 `HoverId` 与 `true`/`false`,
    /// 驱动该按钮图标/背景/边框颜色的平滑过渡动画(见 `App::set_hover`/
    /// `advance_hover_anims`/`hover_progress`)。取代原 `RailHover`/
    /// `TopbarHover`/`HomeHover` 三个专为各自按钮写的变体。
    Hover(HoverId, bool),
    /// 点击放大态背后的变暗遮罩:退出放大。
    MaximizeClose,
    /// 双击顶栏空白处(去掉原生标题栏后,原生"双击标题栏缩放窗口"手势
    /// 只在系统认为仍是"标题栏"的那一小条区域生效;顶栏其余空白靠这条
    /// 消息手动补上同样的行为)。真正调用 `window.set_maximized(...)`
    /// 的是 main.rs——`App` 不持有 `winit::window::Window` 句柄,这里只
    /// 记一个待处理标记,由 `take_pending_zoom_toggle` 供 main.rs 轮询。
    TopBarDoubleClick,
    /// 点击顶栏 Dozer(带 home 图标)按钮:进入首页落地页(`AppPage::Home`)。
    /// 打开/切换项目会自动退回 `Workspace`(见 `ProjectTabOpened`/
    /// `ProjectTabSwitch`/`ProjectSelect`)。
    TopBarHome,
    /// 什么也不做。专门给"就地吃掉事件、不让它冒泡到父级"的 `MouseArea`
    /// 用(`MouseArea::on_press`/`on_scroll` 一旦有消息就会
    /// `shell.capture_event()`)。目前唯一用处:放大态浮层里罩在放大内容
    /// 之上的那层——不吃掉的话,点在审阅正文/卡片空白等"自己不消费点击"
    /// 的地方会穿到外层 dim 遮罩的 `MaximizeClose`,一点正文就退出放大;
    /// 滚轮同理会穿到底层那块看不见的终端 canvas 上,把它的历史滚走
    /// (Fix round 2 #4)。
    Noop,
    /// daemon 不可用（启动连接失败，或某次会话操作失败）的错误文案，
    /// 终端区以 RED 文案展示。
    DaemonError(String),
    /// 终端滚轮：视口向历史方向（正数）/活动区方向（负数）滚动的行数。
    /// 只作用于当前激活 tab（滚轮事件来自它的 canvas）。
    TermScroll(terminal::TermTarget, i32),
    /// 终端 tab 栏溢出下拉开关：点 V 按钮切换；打开时把 `App::last_cursor`
    /// 记进 `Workspace::term_tab_overflow_anchor` 作为悬浮定位锚点。
    TermTabOverflowToggle,
    /// 终端 tab 栏溢出下拉：点击外部区域关闭。
    TermTabOverflowDismiss,
    /// 文件预览 tab 栏溢出下拉开关,语义同 `TermTabOverflowToggle`。
    PreviewTabOverflowToggle,
    /// 文件预览 tab 栏溢出下拉:点击外部关闭。
    PreviewTabOverflowDismiss,
    /// 终端左键按下：在视口格 `(col, row)` 起新选区（`right` = 按点在
    /// 格子右半）。
    TermSelStart {
        target: terminal::TermTarget,
        col: usize,
        row: usize,
        right: bool,
    },
    /// 终端拖拽：选区末端更新到视口格 `(col, row)`。
    TermSelUpdate {
        target: terminal::TermTarget,
        col: usize,
        row: usize,
        right: bool,
    },
    /// ⌘V 粘贴剪贴板文本：按会话的 bracketed paste 模式决定是否包裹
    /// `ESC[200~`/`ESC[201~` 后写入 daemon。
    TermPaste(terminal::TermTarget, String),
    /// 预览:打开本地文件为新 tab(路径已由入口侧确认存在,来自项目树点击/
    /// 会话恢复;预览面板本身已不再有"打开文件…"按钮或地址栏)。
    PreviewOpenPath(PathBuf),
    /// 预览:切换 tab(vec 位置).
    PreviewSelectTab(usize),
    /// 预览:关闭 tab(vec 位置).
    PreviewCloseTab(usize),
    /// 预览:点 tab chip 上的"编辑"按钮,携带 tab 下标(渲染时发出,和
    /// `PreviewSelectTab`/`PreviewCloseTab` 同一约定)。
    PreviewEditOpen(usize),
    /// 预览 tab 右键菜单里的"刷新":从文件系统重新读盘并重新渲染该 tab
    /// (下标即 vec 位置,与 `PreviewCloseTab` 同约定)。
    PreviewReload(usize),
    /// 预览编辑弹层:官方 `text_editor` 的 `Action`。由 `main.rs` 的 dispatch
    /// 直接转发给 `App::preview_edit_event`(剪贴板由 iced 运行时自己处理,
    /// 不需要像 vendored `iced-code-editor` 那样手动拆 `Task` 桥接)。
    EditorEvent(iced_widget::text_editor::Action),
    /// 预览编辑弹层:撤销(⌘Z / Ctrl+Z)。同样由 `main.rs` 命中组合键后直接
    /// 转发给 `App` 下的 `workspace`(见 `preview_edit_undo`)。
    EditorUndo,
    /// 预览编辑弹层:重做(⌘⇧Z / Ctrl+⇧Z)。
    EditorRedo,
    /// 原生预览 tab 的 `text_editor::Action`,`usize` 是 `PreviewTab.id`。
    /// 与 `EditorEvent`(编辑弹层专用)是两条独立路径,互不路由串台——见
    /// `preview_tab_editor_event` 的文档。
    PreviewEditorEvent(usize, iced_widget::text_editor::Action),
    /// 预览编辑弹层:"保存"按钮 / ⌘S。
    PreviewEditSave,
    /// 原生预览就地可写后的 ⌘S:把 `kind` 指向面板(`Files`/`Project`)当前激活
    /// 原生 tab 的改动保存到磁盘(仅脏的原生 tab 动作;见
    /// `Workspace::preview_pane_save_active`)。与 `PreviewEditSave`(弹层专用)
    /// 是不同路径。携带 `PanelKind`(可由 main.rs `FocusIntent::Preview` 直接
    /// 转发,不必频繁 preview↔panel 双枚举映射)。
    PreviewSaveActive(PanelKind),
    /// 原生预览 tab 就地敲 Tab(裸 Tab、非 ⌘/⌃/⌥ 组合):iced 官方
    /// `text_editor` 默认 Binding 对 Tab 完全不产生动作,必须在这里作为一条
    /// 编辑动作手工插 `\t`(走 `CodeView::perform` 的原生 Content 光标,选区
    /// 替换/撤销都能对)。`kind` 指向当前真正聚焦的原生编辑器所在面板,同
    /// `PreviewSaveActive` 一跳区分、不做 preview↔panel 双枚举映射。构造/调用
    /// 处是 main.rs 原生预览闸门;先拦下,没命中的键才放行给 iced。
    PreviewTabInsertTab(PanelKind),
    /// 原生预览打开 File-Find(⌘F)。`kind` 指向 `Files` 或 `Project` 面板——
    /// File-Find 对两个面板的原生编辑 tab 语义相同(见
    /// `Workspace::preview_find_open`,⌘F 已开时是重聚焦的 no-op)。跟
    /// `PreviewSaveActive` 一样用 `PanelKind` 一跳区分面板,不强做 preview↔panel
    /// 双枚举映射。
    PreviewFindOpen(PanelKind),
    /// 同 `PreviewFindOpen`(⌘R),但替换行默认展开——查询框前的圆盘箭头也
    /// 展示这个展开态,`PreviewFindReplaceToggle` 再手动翻转。
    PreviewFindOpenWithReplace(PanelKind),
    /// 查询框前的圆盘箭头点击:手动翻转替换行展开/收起,不受 ⌘F/⌘R 影响。
    PreviewFindReplaceToggle(PanelKind),
    /// File-Find 关闭(输入框 × / Esc / 切走文件)。`kind` 语义同
    /// `PreviewFindOpen`。
    PreviewFindClose(PanelKind),
    /// File-Find 输入框每键的 query 落定:同步到面板并让面板当场重算、跳首个命中。
    PreviewFindText(PanelKind, String),
    /// File-Find 下一条 / 上一条。
    PreviewFindGo(PanelKind, bool),
    /// File-Find 大小写敏感开关(`true`=逐字严格、`false`=ASCII 大小写折叠)——
    /// 点条上「Aa」切换钮落定的方向。只翻当轮会话的语义,不改全局默认。
    PreviewFindCase(PanelKind, bool),
    /// File-Find 条的「替换为」输入框每键落定(只写 `FindState::replacement`
    /// 草稿,不触发任何替换;真正动作在点「替…」按钮时发生)。
    PreviewFindReplacement(PanelKind, String),
    /// File-Find 条「替换当前」:把本轮 `current` 指着的那一处清掉换成替换框
    /// 文本。照 Enter/⌘S 外的普通打字语义,只改**原生 buffer 并标脏**等待用户
    /// ⌘S 落盘——替换不隐式写盘([CLAUDE.md 裁决]预览优先渲染/不可逆动作留给
    /// 显式保存)。
    PreviewFindReplaceCurrent(PanelKind),
    /// File-Find 条「替换全部」:与 `PreviewFindReplaceCurrent` 同一 buffer-only
    /// 语义,只是把这轮每一处命中一次性全改、同样标脏等 ⌘S。
    PreviewFindReplaceAll(PanelKind),
    /// 预览编辑弹层:×按钮 / 点遮罩——脏改动会先转成二次确认,不直接关。
    PreviewEditCloseRequest,
    /// 预览编辑弹层二次确认:"放弃改动"。
    PreviewEditConfirmDiscard,
    /// 预览编辑弹层二次确认:"取消"(回到编辑态)。
    PreviewEditConfirmCancel,
    /// 预览 tab 右键菜单:在 `preview_pane` 某 tab 上右键打开,携带 tab 下标
    /// 与该文件是否可编辑(仅文本类文件,决定菜单"编辑"项是否出现)。定位
    /// 坐标复用 `files.last_right_click`(main.rs 右键时写入)。
    PreviewTabContextMenu {
        idx: usize,
        editable: bool,
    },
    /// 预览 tab 右键菜单关闭(点遮罩 / 按 Esc)。
    PreviewTabContextMenuClose,
    /// Project 面板右配对预览:打开本地文件为新 tab,语义同 `PreviewOpenPath`。
    ProjectPreviewOpenPath(PathBuf),
    /// Project 面板右配对预览:切换 tab(vec 位置)。
    ProjectPreviewSelectTab(usize),
    /// Project 面板右配对预览:关闭 tab(vec 位置)。
    ProjectPreviewCloseTab(usize),
    /// Project 面板右配对预览 tab 栏溢出下拉开关,语义同上
    /// (`PreviewTabOverflowToggle`)。
    ProjectPreviewTabOverflowToggle,
    /// Project 面板右配对预览 tab 栏溢出下拉:点击外部关闭。
    ProjectPreviewTabOverflowDismiss,
    /// Project 面板右配对预览的原生 `text_editor::Action`，语义同
    /// `PreviewEditorEvent`。
    ProjectPreviewEditorEvent(usize, iced_widget::text_editor::Action),
    /// Project 面板右配对预览:右键菜单里的"编辑"项,语义同 `PreviewEditOpen`。
    ProjectPreviewEditOpen(usize),
    /// Project 面板右配对预览 tab 右键菜单里的"刷新",语义同 `PreviewReload`。
    ProjectPreviewReload(usize),
    /// Project 面板右配对预览 tab 右键菜单,语义同 `PreviewTabContextMenu`。
    ProjectPreviewTabContextMenu {
        idx: usize,
        editable: bool,
    },
    /// Project 面板右配对预览 tab 右键菜单关闭。
    ProjectPreviewTabContextMenuClose,
    /// 浏览器面板的全部消息,内核只转发不解读——见
    /// `extensions::browser::Message`。
    Browser(browser::Message),
    /// 浏览器 webview 渲染进程报回页面 HTML 标题(id = webview/tab id,
    /// title = `document.title`)。浏览器面板有首页全局(`home_browser`)与
    /// 工作区(`ws.browser`)两套、且同时只有一套活跃,按 `is_home()` 路由;
    /// main.rs 的 IPC 分支不知道自己在哪套里,故用独立顶层消息,不开新
    /// `browser::Message::TitleLoaded` 包装。
    BrowserTitle(usize, String),
    /// 浏览器 webview 渲染进程报回网页内超链接/`window.open` 自行导航后的
    /// 真实地址(id = webview/tab id,url = webview 加载到的 URL)。路由口径
    /// 同 `BrowserTitle`:首页全局/工作区两套浏览器按 `is_home()` 分派,而不
    /// 走 `Message::Browser(..)`(那会只落到聚焦工作区)。
    BrowserNavigated(usize, String),
    /// 网页内 `target="_blank"`/`window.open` 请求新窗口。wry 的
    /// `new_window_req` 分支不知道目标归属哪套浏览器,同 `BrowserTitle` 按
    /// `is_home()` 路由,在其内 `open_url`(总开新 tab,不复用激活 tab)。
    BrowserNewWindow(String),
    /// 项目:切换到最近项目。
    ProjectSelect(i64),
    /// 项目:"打开项目…"→ rfd 文件夹选择(main.rs 执行),选中后回送
    /// `ProjectTabOpen`。顶栏"＋"与项目栏那颗"打开项目…"按钮**共用**这
    /// 一条入口——两者都是"我要打开一个项目",都必须落成**新增页签**。
    ///
    /// 此前项目栏那颗按钮走的是另一条 `ProjectPickFolder`→`ProjectOpened`
    /// 的**就地改写**路径(`close_all_tabs_for_switch` + `adopt_project`),
    /// 会把当前聚焦项目的终端会话全杀掉——直接违反设计文档 §2"只有用户
    /// 显式关闭一个页签,才结束该项目下的会话"。整条就地改写路径连同
    /// `retarget_active_slot` 已随之删除,不留第二条会杀会话的打开入口。
    ProjectTabPickFolder,
    /// 单颗"＋"按钮:main.rs 弹 rfd 模态(根目录在项目根),选完后按
    /// `is_dir()` 判 `LinkKind` 回送 `project::Message::LinkAdd`。
    ProjectLinkPick(project::links::LinkTarget),
    /// Project 面板链接行右键菜单关闭(点遮罩 / 按 Esc)。
    ProjectLinkContextMenuClose,
    /// Todo 分类树节点右键菜单关闭(点遮罩 / 按 Esc)。
    CategoryContextMenuClose,
    /// 分类选择器("移动到..." / 任务挂分类)浮层关闭(点遮罩 / 按 Esc)。
    CategoryPickerClose,
    /// 选择器里点了某一项:`None` = "未分类"(仅 `Todo` target 下有效,
    /// `Category` target 选"未分类"表示挪到顶层)。
    CategoryPickerSelect(Option<i64>),
    /// 数据库面板数据源树 header 行右键菜单关闭(点遮罩 / 按 Esc)。
    DatabaseSourceContextMenuClose,
    /// 项目页签:把某路径作为**新页签**打开(不动任何已存在页签的内容)。
    ProjectTabOpen(PathBuf),
    /// 项目页签:`ProjectTabOpen` 异步完成(daemon upsert 结果 + 最近列表)。
    /// `None` = 这次打开失败,只报错、不改任何页签状态。
    ProjectTabOpened(Option<ProjectInfo>, Vec<ProjectInfo>),
    /// H0 项目中心:`Message::TopBarHome` 发起的异步刷新完成(最近改动的文件、
    /// 最近的对话两份列表;D4)。
    HomeRecentsLoaded(Vec<HomeRecentFile>, Vec<HomeRecentConversation>),
    /// 首页左图标栏:切换 `HomeLeftView`(项目列表/Recents)。首页没有
    /// collapse 概念,恒有一个 pane 显示,不像工作区 `PanelSelect` 那样
    /// 需要处理"点已选中图标收起面板区"的分支。
    HomeLeftIconSelect(homespace::HomeLeftView),
    /// 首页"项目列表" pane:点"更多..."再展开 5 个项目。纯面板内状态变更,
    /// 无 IO;只有还有更多项目时才渲染那颗按钮(见
    /// `homespace::home_project_list_view`)。
    HomeMoreProjects,
    /// 首页项目列表搜索框草稿变化(iced `text_input::on_input`)。
    HomeProjectSearchInput(String),
    /// 回车 / 点搜索按钮:把草稿落成生效的 `home_project_search` 过滤词,
    /// 同时把翻页重置回第 1 页(过滤后结果变少,停在旧页码没有意义)。
    HomeProjectSearchSubmit,
    /// 首页右图标栏:切换 `HomeRightView`(目前只有 Browser)。
    HomeRightIconSelect(homespace::HomeRightView),
    /// 首页全局浏览器面板的全部消息,内核只转发不解读——见
    /// `extensions::browser::Message`。路由到 `app.home_browser`,
    /// `project_id` 恒传 `None`;与工作区 `Message::Browser` 路由到
    /// `ws.browser` 是两条独立路径,互不影响。
    HomeBrowser(browser::Message),
    /// 项目页签:点已存在的页签 → 前台化该项目。只改"当前是哪个页签",
    /// 不结束任何会话、不改写任何 `Workspace` 的内容。
    ProjectTabSwitch(i64),
    /// 项目页签:点页签的 × → 关闭该页签,并结束该项目下所有会话。
    ProjectTabClose(i64),
    /// 项目页签:`App::ensure_loaded` 的异步促成完成——素材已取回,由
    /// `update` 在 UI 线程上装配成 `Workspace`,替换掉那份"加载中"占位
    /// (载荷是一次性信封,见 [`RestorePayload`])。
    ProjectSlotLoaded(i64, RestorePayload),
    /// 项目:`git_watch` 监听到工作区/`.git` 引用变化,该重新跑一次 git 刷新
    /// 了(D4)。`Relevance` 决定这次触发要不要顺带做 Plan 2 的 Git Log 快照
    /// 重建。这条消息同时喂给 Files(刷新 git_statuses)、Project(刷新
    /// branch/dirty)和 Git Log(条件触发快照重建)三个独立扩展,
    /// 内核继续拦截、分别转发,不包进任何一个 extension 的 `Message`。
    ProjectFsChanged(ProjectId, git_watch::FsChanges),
    /// Git Log 面板的全部消息,内核只转发不解读——见
    /// `extensions::git_log::Message`。
    GitLog(git_log::Message),
    /// Files 面板的全部消息,内核只转发不解读——见 `extensions::files::Message`。
    Files(files::Message),
    /// Project 信息面板的全部消息,内核只转发不解读——见
    /// `extensions::project::Message`。
    Project(project::Message),
    /// SSH 主机面板的全部消息,内核只转发不解读——见
    /// `extensions::ssh::Message`。
    Ssh(ssh::Message),
    /// Footbar 系统信息条的消息,内核只转发不解读——见
    /// `extensions::footbar::Message`。App 级状态(不挂 `Workspace`),
    /// 路由比其它 extension 简单:不带 `project_id`,不需要
    /// `with_project`/`with_focused_project`,直接 `footbar::update`。
    Footbar(footbar::Message),
    /// UI 整体放大(Ctrl +)：放大/还原的全局 scale 乘一个步近因子,下一帧
    /// 按新 scale 重排全部图标/字号/间距/骨架。
    ZoomIn,
    /// UI 整体缩小(Ctrl -)。
    ZoomOut,
    /// UI 缩放还原(Ctrl+1)：回到启动基准 scale。
    ZoomReset,
    /// 预览/浏览器 webview 收到鼠标点击(JS mousedown → IPC → EventLoopProxy),
    /// 通知 main.rs 调 `view.focus()` 让 WKWebView 成为 first responder。
    /// winit 收不到子 webview 上的 `MouseInput`,这条消息是唯一焦点信号源。
    /// 不区分 Preview/Browser:`left_view` 互斥,`dispatch` 按 `shell_state`
    /// 判断归谁。
    WebViewFocused,
    /// 子 webview 上的鼠标松开(winit 收不到,JS 经 IPC 发来)。目的是结束
    /// 页签拖拽:若用户把 tab 从 iced 表层一路拖进 webview 并在这里松开,
    /// winit 的根本 `MouseInput{Released}` 收不到,`TabDragEnd` 就永不触发,
    /// 拖拽状态会残留、变成"松开还能继续拖"。这条消息统一兜底清掉。
    WebViewMouseUp,
}

/// 顶层容器:main.rs 持有的就是这个(取代此前直接持有单个 `Workspace`)。
/// 外壳字段是整个程序只有一份的窗口态,`projects` 承载并行打开的项目
/// 页签——每个 `Workspace` 是完全独立、同时存活的一套项目态(P2a)。
/// 文件预览 tab 的右键菜单浮层状态:定位坐标(屏幕空间,复用 `files`
/// 右键落点)+ 目标 tab 下标 + 该文件是否可编辑(仅文本类文件可编辑,
/// 决定菜单里"编辑"项是否出现)。见 `preview_pane` / 顶层 `view`。
struct PreviewTabMenu {
    x: f32,
    y: f32,
    idx: usize,
    editable: bool,
}

/// Project 面板「项目文档 / Agent 记忆」虚拟链接行的右键菜单浮层状态:
/// 定位坐标(屏幕空间,复用 `files` 右键落点)+ 目标区 + 下标。删除动作回
/// `project::Message::LinkRemove`。见 `project::links_section`。
struct ProjectLinkMenu {
    x: f32,
    y: f32,
    target: project::links::LinkTarget,
    index: usize,
}

/// Todo 分类树节点右键菜单浮层状态,镜像 `ProjectLinkMenu`。`id` 为
/// `None` 表示右键的是"全部"/"未分类"伪节点(菜单只含"新建分类"新建
/// 顶层分类)。
struct CategoryContextMenu {
    x: f32,
    y: f32,
    /// 被右键的节点:`Some` 为真实分类 id,`None` 为全部/未分类伪节点。
    id: Option<i64>,
}

/// 分类选择器要挂靠的目标:给任务挂分类、给分类节点 reparent。二者共用
/// "点树选一个节点";挂任务 → `set_todo_category`,reparent → `reparent_category`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CategoryPickerTarget {
    /// 给某个任务挂分类(任务 id)。
    Todo(i64),
    /// 给某个分类节点 reparent(分类 id)。
    Category(i64),
}

/// 分类选择器浮层状态:定位坐标 + 目标。渲染内容复用
/// `category_tree_nav` 同一份树数据(只读展示,不接展开/右键,选中即
/// 关闭并提交)。
struct CategoryPicker {
    x: f32,
    y: f32,
    target: CategoryPickerTarget,
}

/// 输入框右键菜单浮层状态:定位坐标(屏幕空间,复用 `files.last_right_click`)
/// 与被右键的输入目标。二者都由 `TextInputMenuOpen(target)` 写入,关闭或
/// 执行某个编辑动作后清空。渲染见 `app.view` 顶层 `text_input_menu_popup`。
struct TextInputMenu {
    x: f32,
    y: f32,
    target: TextInputTarget,
}

/// 数据库面板数据源树 header 行的右键菜单浮层状态:定位坐标(复用
/// `files.last_right_click`)+ 目标数据源 id。测试连接/编辑/删除/刷新
/// 四个动作见 `database_source_context_menu_popup`。
struct DatabaseSourceMenu {
    x: f32,
    y: f32,
    source_id: String,
}

pub struct App {
    client: Client,
    handle: Handle,
    proxy: EventLoopProxy<Message>,
    /// 关 tab/丢弃过期促成结果时发往 daemon 的 kill/总结请求句柄——退出前
    /// `wait_for_pending_exit_tasks` 要等它们跑完,不然请求可能因为 tokio
    /// runtime 随进程退出被中途丢弃,daemon 侧会话仍是 `alive`,下次启动
    /// 又被恢复出来。见 `ShellIo::track_exit_critical`。
    pending_exit_tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    /// 当前终端网格尺寸,随 `PaneResized` 更新;新建 tab 时也用这份
    /// 尺寸,保证新会话从一开始就跟 pane 实际大小匹配。
    cols: u16,
    rows: u16,
    /// SSH 面板内嵌终端的网格尺寸,与 `cols/rows`(共享终端)分开记——两个
    /// pane 几何不同,`PaneResized` 各带一份,无法互相替代。
    ssh_cols: u16,
    ssh_rows: u16,
    /// 当前终端 IME 组字预览(未提交,`Ime::Preedit` 驱动):不进 PTY,只在
    /// `term_view::draw` 里叠一层带下划线的预览文字。`Ime::Commit`/组字
    /// 取消(空 preedit)时清空。只对 `App::keyboard_term_target()` 当前
    /// 指向的那个终端 pane 生效(见两处 `term_view::view` 调用处按
    /// `focused` 决定是否传入)。
    term_ime_preedit: Option<String>,
    /// daemon 连接失败,或某次会话操作失败时的错误文案。整个程序共享
    /// 一份:daemon 连不连得上不是某个项目自己的状态。
    pub(crate) daemon_error: Option<String>,
    /// 上次真正执行 Todo 面板磁盘轮询(`poll_todo_if_visible`)的时刻:
    /// 按 `TODO_POLL_INTERVAL` 自限速,未到点的调用直接 no-op。现在靠
    /// mtime 检查已经安全(没变化就早退,见该方法文档),这里补上限速是为了
    /// 让"周期性函数自己对被更快唤醒节奏带跑免疫"这条约定对周期性关注点
    /// (悬停动画/Todo 轮询)都显式成立,不留一个"靠巧合安全"的例外。
    last_todo_poll_at: std::time::Instant,
    /// 全局窗口尺寸;启动时 `layout::load()` 读盘作起始值,退出前写盘。
    /// 只存窗口尺寸——左右面板区的宽度/分割比例(**每个项目各自**的偏好)
    /// 已迁进每项目 `dims`(见 `panel_layouts`),不放在这里。
    pub(crate) shell_layout: ShellLayout,
    /// 当前活跃项目的面板区尺寸(左宽 + 四个 split)活值。切项目前经
    /// `current_panel_layout` 回填进 `PanelLayout.dims` stash,切过去由
    /// `adopt_panel_layout` 灌回来。拖拽(`ColumnDragEnd`)写回这份。
    dims: PanelDims,
    /// 每个项目各自的面板布局(左右视图选择 + 收起态),按项目 id 索引;
    /// 启动时从 `panel_layouts::load()` 读回,切换/改面板时写回。当前正
    /// 显示的项目的布局由 `left_view`/`right_view`/`left_collapsed`/
    /// `right_collapsed` 这几份"活值"承载,切换项目前先 `stash` 回这里、
    /// 切过去再 `adopt` 出来,别的项目那份绝不被当前项目盖掉。
    panel_layouts: HashMap<i64, PanelLayout>,
    /// 左面板区当前显示的配对视图(左图标栏点击切换)。
    pub(crate) left_view: PanelKind,
    /// 右面板区当前显示的配对视图(右图标栏点击切换)。
    pub(crate) right_view: PanelKind,
    /// 左面板区是否折叠(再点一次当前已激活的图标即收起)。
    pub(crate) left_collapsed: bool,
    /// 右面板区是否折叠,语义同 `left_collapsed`。
    pub(crate) right_collapsed: bool,
    /// 当前放大的内容子面板(`None`=未放大)。
    maximized: Option<MaximizedPane>,
    /// 左键点击落点决定的当前"聚焦"面板区,驱动 `left_zone`/`right_zone`
    /// 外边框的高亮态(见 `set_active_zone`/`zone_at_x`)。启动默认
    /// `Some(Right)`——终端处默认焦点区,终端在右面板区。
    pub(crate) active_zone: Option<ZoneSide>,
    /// 所有按钮的悬停动画状态机(顶栏 Home / 顶栏右侧 / 图标栏),key 为
    /// `HoverId`。iced 0.14 无内置动画 API,这套自驱 redraw(与光标闪烁同款)
    /// 把图标/背景颜色在 idle↔hover 间 ease-out 过渡。进度由 main.rs 的定时
    /// 唤醒经 `advance_hover_anims` 指数逼近各自 `target`(见 `HoverAnim`)。
    hover_anims: std::collections::HashMap<HoverId, HoverAnim>,
    /// 页签标题 tooltip 的悬停计时起点:key 复用 `HoverId`(与 `hover_anims`
    /// 同源),进入页签记 `Instant::now()`,离开即清除。悬停满
    /// `HOVER_TOOLTIP_DELAY` 后视图层据此弹出标题全称 tooltip(见
    /// `hover_tooltip_ready`)。浏览器面板的页签走自己那套 hover 状态机,
    /// 计时另存于 `browser::State::tooltip_starts`,本表只覆盖顶栏页签与
    /// 终端/预览/SSH 面板页签。
    hover_tooltip_starts: std::collections::HashMap<HoverId, std::time::Instant>,
    /// 图标栏拖拽换位/换栏时,每个面板按钮的动画槽位状态机——同栏重排让
    /// 让位的相邻按钮平滑滑动到新槽位,而不是瞬间跳变。key 为
    /// `PanelKind`,与 `hover_anims` 同款自驱 redraw 节奏(`advance_hover_anims`
    /// 顺带推进,见 `RailSlotAnim`)。跨栏移动的目标侧与来源侧是两条完全
    /// 不同的物理列,不追求跨列平滑滑动,该面板在新一侧直接按新槽位
    /// snap(`RailSlotAnim::retarget` 检测到侧变化即重置,不生成动画)。
    rail_slot_anims: std::collections::HashMap<PanelKind, rail::RailSlotAnim>,
    /// 双击顶栏空白处待处理标记,见 `Message::TopBarDoubleClick`/
    /// `take_pending_zoom_toggle`。`App` 不持有 `winit::window::Window`
    /// 句柄,真正切换最大化态由 main.rs 轮询这个标记后调用。
    pending_zoom_toggle: bool,
    /// 顶栏"＋新增项目"按钮的最近项目选择菜单是否打开(见
    /// `Message::ProjectAddMenuToggle`/`project_add_menu_popup`)。挂在
    /// `App` 而不是某个 `Workspace` 上——这个按钮本身就在顶栏、不属于任何
    /// 单个项目,与 `ws.agent_picker_open`(Agent 面板"＋",项目内状态)是
    /// 两个不同归属层级的同类开关。
    pub(crate) project_add_menu_open: bool,
    /// 打开菜单那一刻的光标逻辑坐标,菜单弹出锚点——"＋"按钮的 x 随已开
    /// 页签数量浮动(`project_tabs_row` 页签 `Shrink` 宽、"＋"紧跟最后一片
    /// 页签之后),没有固定 padding 能蒙对,改用 `todo::set_calendar_anchor`
    /// /`set_dispatch_anchor` 同款"记下点击时的 `App::last_cursor`"手法。
    pub(crate) project_add_menu_anchor: (f32, f32),
    /// 全局 UI 缩放(⌘/Ctrl +/-)改变后,预览/浏览器 webview 的
    /// `WebView::zoom` 也要同步——但 `App` 不持有 webview 句柄,只能
    /// 置这个标记,由 main.rs 轮询 `take_pending_preview_zoom` 后逐个
    /// 应用。新 webview 在 `sync_webview_pool` 建出来时直接按当前 scale
    /// 初始化,所以本标记只管"已存在 webview 的缩放变更"这一增量。
    pending_preview_zoom: bool,
    /// 当前窗口逻辑尺寸(宽,高)。由 main.rs 建窗口/`WindowEvent::Resized`
    /// 时经 `set_window_size` 写入。
    pub(crate) window_size: (f32, f32),
    /// 最近一次 `CursorMoved` 的光标逻辑坐标(宽,高),由 main.rs 每帧更新。
    /// Todo 日历浮层用它当弹出锚点(点日历按钮时光标就在按钮上,等价于"按钮
    /// 旁边"),镜像 `files.last_right_click` 的坐标复用套路。
    pub(crate) last_cursor: (f32, f32),
    /// 正在拖拽的分隔线;`None` 表示未在拖拽。
    dragging: Option<Divider>,
    /// 正在拖拽的纵向(上下)分隔线;`None` 表示未在拖拽。
    dragging_row: Option<RowDivider>,
    /// 正在拖拽的页签(换位);`None` 表示未在拖拽页签。与 `dragging` 分隔线
    /// 互斥(一次左键拖拽只能是一件事)。
    tab_drag: Option<TabDrag>,
    /// 正在进行的图标栏面板拖拽(同栏重排/跨栏移动);`None` 表示未在拖拽。
    /// 与 `tab_drag`/`dragging` 互斥(一次左键拖拽只能是一件事)。
    pub(crate) rail_drag: Option<rail::RailDrag>,
    /// Files 面板右键菜单浮层状态——见 `extensions::files::AppState`。
    files: files::AppState,
    /// 文件预览 tab 右键菜单浮层状态(屏幕空间单例,不随项目切换各自保留);
    /// 定位坐标复用 `files.last_right_click`(main.rs 右键时已写入)。
    preview_tab_menu: Option<PreviewTabMenu>,
    /// Project 面板右配对预览 tab 的右键菜单浮层状态,语义同
    /// `preview_tab_menu`,定位坐标同样复用 `files.last_right_click`。
    project_preview_tab_menu: Option<PreviewTabMenu>,
    /// Project 面板「项目文档 / Agent 记忆」链接行的右键菜单浮层状态,坐标
    /// 同样复用 `files.last_right_click`。
    project_link_menu: Option<ProjectLinkMenu>,
    /// Todo 分类树节点右键菜单浮层状态,坐标复用 `files.last_right_click`。
    category_context_menu: Option<CategoryContextMenu>,
    /// 分类选择器("移动到..." / 任务挂分类)浮层状态:定位坐标 + 目标。
    category_picker: Option<CategoryPicker>,
    /// 通用输入框右键菜单浮层状态(屏幕空间单例)。`TextInputMenuOpen` 时
    /// 写入、`TextInputMenuClose`/动作后清空。同一时刻最多挂一个。
    text_input_menu: Option<TextInputMenu>,
    /// 数据库面板数据源树 header 行的右键菜单浮层状态,坐标同样复用
    /// `files.last_right_click`。
    database_source_menu: Option<DatabaseSourceMenu>,
    /// 待处理的"输入框右键菜单要作用的输入"焦点:载入 `TextInputMenuOpen`
    /// 携带的 `TextInputTarget.id`,由 main.rs 的 `apply_pending_focus` 在
    /// 本帧后移至该输入,供菜单的复制/粘贴作用到被右键的输入。
    pending_text_input_focus: Option<iced_widget::core::widget::Id>,

    /// 并行打开的项目页签:project id → 该项目的完整/占位状态。
    pub(crate) projects: HashMap<i64, WorkspaceSlot>,
    /// 页签顺序(`projects` 是 HashMap,顺序另存;Task 6 的页签栏按它渲染)。
    pub(crate) project_order: Vec<i64>,
    /// 当前聚焦的项目页签(`None`=一个项目都没打开)。
    pub(crate) active_project_id: Option<i64>,
    /// 当前顶层页面(工作区 / 首页)。默认 `Workspace`;点顶栏 Dozer 切到
    /// `Home`,打开/切换项目切回 `Workspace`。
    pub(crate) current_page: AppPage,
    /// H0 项目中心侧栏用的"最近项目"列表(D2)。与 `Workspace.recent_projects`
    /// 语义相同但字段独立——避免为了 H0 牵连项目栏"未打开项目"兜底列表那条
    /// 无关路径。`App::bootstrap()`/`Message::ProjectTabOpened` 处理函数负责
    /// 让它跟 daemon 的 `list_projects()` 结果保持同步。
    pub(crate) recent_projects: Vec<ProjectInfo>,
    /// H0"最近的文件"卡数据(D4);`Message::TopBarHome` 时异步刷新。
    pub(crate) home_recent_files: Vec<HomeRecentFile>,
    /// H0"最近的对话"卡数据(D4);语义同上。
    pub(crate) home_recent_conversations: Vec<HomeRecentConversation>,
    /// 是否已经收到过至少一次 `HomeRecentsLoaded`——区分"还在加载"与"加载完
    /// 但结果为空"，两张卡据此决定画"加载中…"还是空状态文案(spec §4)。
    pub(crate) home_recents_loaded: bool,
    /// 首页左栏当前显示哪个 pane(项目列表/Recents)。不持久化,每次
    /// `Message::TopBarHome` 进首页都重置为默认值——见
    /// `homespace::HomeLeftView`。
    pub(crate) home_left_view: homespace::HomeLeftView,
    /// 首页"项目列表" pane 已经展开的项目页数(点一次"更多..." +1)。首屏
    /// 只显示前 `PAGE_SIZE`(5)个,翻页后显示 `pages * 5` 个。不持久化,每次
    /// `Message::TopBarHome` 进首页重置回 1——与 `home_left_view` 同套
    /// "进首页即重置"语义;别的 pane(Recents)不读它。
    pub(crate) home_project_pages: usize,
    /// 首页"项目列表"搜索框已提交生效的过滤词(空串 = 不过滤)。不持久化,
    /// 与 `home_project_pages` 同套"进首页即重置"语义。
    pub(crate) home_project_search: String,
    /// 搜索框编辑态草稿——同 `todo::search_draft`,打字期间只改草稿,
    /// 回车/点搜索按钮才落成 `home_project_search`。
    pub(crate) home_project_search_draft: String,
    /// 是否持有 iced 真实焦点。**不是**应用层手动置位的镜像——每帧渲染
    /// 循环里 `CaptureHomeSearchFocus` 问一遍 iced 真相后立刻写进这里
    /// (`set_home_project_search_focused`)。
    pub(crate) home_project_search_focused: bool,
    /// 首页右栏当前显示哪个 pane(目前只有 Browser)。语义同上。
    pub(crate) home_right_view: homespace::HomeRightView,
    /// 首页全局浏览器面板状态,不挂在任何 `Workspace` 上;`view`/`update`
    /// 调用时 `project_id` 恒传 `None`(全局收藏夹作用域)。
    pub(crate) home_browser: browser::State,
    /// Git Log 面板状态——自己的 `Message`/`update`/`view`,见
    /// `extensions::git_log`。`App` 级共享、不按项目分(现状,纯重构不改,
    /// 见 `sync_git_log_to_active_project`)。
    git_log: git_log::State,
    /// 数据库面板 App 级状态(哪些驱动类型在"新增数据源"下拉里可选,
    /// 启动时读盘)——见 `extensions::database::AppState`。
    database: database::AppState,
    /// Footbar 系统信息条 App 级状态——跨所有项目页签共享(见
    /// `extensions::footbar::AppState`)。`spawn_sampler` 在 `new_shell`
    /// 阶段启动一个长生命周期 tokio 任务,每 1s/300s 采样一次发回
    /// `Message::Footbar(Message::Sampled)`,UI 即刻刷新。
    footbar: footbar::AppState,
}

/// 前台化一个**已经存在**的项目页签:只改"当前是哪个页签",一个槽位的内容
/// 都不碰。返回 false = 没有这个页签(调用方原地放弃)。
///
/// 这是**唯一**一条"换到另一个项目"的路。此前还有一条
/// `retarget_active_slot` 的**就地改写**路(`ProjectOpened`:把当前槽位的内容
/// 整体改写成另一个项目,先 `close_all_tabs_for_switch` 关光终端、再
/// `adopt_project`),它与设计文档 §2"只有用户显式关闭一个页签,才结束该项目
/// 下的会话"直接冲突,已随最终审查一并删除。**不要**把它请回来:打开项目一律
/// 是"新增/前台化一个页签",绝不是"把手上这个换掉"。
pub(crate) fn focus_project_tab(
    projects: &HashMap<i64, WorkspaceSlot>,
    active_project_id: &mut Option<i64>,
    id: i64,
) -> bool {
    if !projects.contains_key(&id) {
        return false;
    }
    *active_project_id = Some(id);
    true
}

/// 关掉某个页签后,焦点该落到谁身上:原位置的右邻优先,没有右邻取左邻,
/// 一个都不剩则 `None`(退回"未打开任何项目"的空外壳)。
///
/// 传入的是**删除之前**的顺序表——右邻/左邻要按被关页签的原位置算,删完
/// 再算就分不清"右邻"了。
pub(crate) fn next_active_after_close(project_order: &[i64], closed: i64) -> Option<i64> {
    let idx = project_order.iter().position(|&i| i == closed)?;
    project_order
        .get(idx + 1)
        .or_else(|| idx.checked_sub(1).and_then(|prev| project_order.get(prev)))
        .copied()
}

/// 从槽位表里摘掉一个项目页签,返回被摘掉的槽位交给调用方善后(结束会话)。
/// 只在关的正好是当前页签时才动 `active_project_id`(落到
/// [`next_active_after_close`] 给的邻居),关后台页签不打扰前台。
pub(crate) fn take_project_tab(
    projects: &mut HashMap<i64, WorkspaceSlot>,
    project_order: &mut Vec<i64>,
    active_project_id: &mut Option<i64>,
    id: i64,
) -> Option<WorkspaceSlot> {
    let slot = projects.remove(&id)?;
    if *active_project_id == Some(id) {
        *active_project_id = next_active_after_close(project_order, id);
    }
    project_order.retain(|pid| *pid != id);
    Some(slot)
}

/// 按 `project_id` 取一份**已加载**的 `Workspace`,与"此刻聚焦的是哪个项目"
/// 完全无关——这就是 [`App::with_project`] 的全部路由逻辑。
///
/// 拆成自由函数是为了能 headless 单测(`App` 要 daemon 连接 + winit
/// `EventLoopProxy` 才构造得出来),与本文件其余纯逻辑一致。这条路由是多项目
/// 并行下最要紧的一条不变式,理由见 [`ProjectId`]。
pub(crate) fn loaded_workspace_mut(
    projects: &mut HashMap<i64, WorkspaceSlot>,
    project_id: ProjectId,
) -> Option<&mut Workspace> {
    match projects.get_mut(&project_id)? {
        WorkspaceSlot::Loaded(ws) => Some(ws),
        // `Stub` 从没促成过,不可能有指向它的在飞会话结果(促成期的"加载中"
        // 占位是 `Loaded`)。
        WorkspaceSlot::Stub { .. } => None,
    }
}

/// 启动恢复的纯逻辑:把盘上记的"上次开着哪些页签"(`open_projects.json`)与
/// daemon 现在还认识的项目列表对一遍,给出该恢复成页签的项目顺序表 + 该聚焦
/// 哪一个。
///
/// 规则(设计文档 §5):
/// - daemon 已经不认识的 id 直接跳过——项目可能在上次退出后被删了,给它开个
///   点不动的空页签只会碍事;
/// - 盘上出现重复 id(理论上不该有,但文件是用户可编辑的普通 JSON)去重,
///   否则 `project_order` 会带出两个指向同一个槽位的页签;
/// - 一个都没恢复出来(首次启动/文件缺失/项目全被删)回落到 `known` 的第一个
///   ——`list_projects()` 按 `last_active_ms` 倒序,第一个就是最近用过的那个,
///   保持"打开 app 就能干活"的既有行为;
/// - 聚焦项:记着的那个若已不在恢复出的页签集合里,回落到第一个页签。
///
/// 抽成自由函数是为了能 headless 单测(`App` 要有 daemon 连接 + winit
/// `EventLoopProxy` 才构造得出来),与本文件其余纯逻辑的处理一致。
pub(crate) fn restore_open_tabs(
    known: &[ProjectInfo],
    state: &open_projects::OpenProjectsState,
) -> (Vec<i64>, Option<i64>) {
    let mut order: Vec<i64> = Vec::new();
    for id in &state.project_ids {
        if !known.iter().any(|p| p.id == *id) {
            continue;
        }
        if order.contains(id) {
            continue;
        }
        order.push(*id);
    }
    if order.is_empty()
        && let Some(p) = known.first()
    {
        order.push(p.id);
    }
    let active = state
        .active_project_id
        .filter(|id| order.contains(id))
        .or_else(|| order.first().copied());
    (order, active)
}

/// 项目信息面板切入时的 README 保障(纯函数,方便单测):保证项目根目录有
/// 一份可读的 `README.md`。已有就**原样保留不重写**(返回其路径);没有则
/// 用项目名当一级标题、`.dozer/description.md` 的描述(`load_description`,
/// 无描述则省略)生成一份再返回路径。
///
/// 只有 README 最终存在且可读时才返回 `Some(readme 路径)`;创建失败(目录
/// 不可写等)返回 `None`——绝不拿一个空文件或半截文件去占预览,也绝不让面
/// 板切入失败。
///
/// 描述固定读磁盘权威来源,不用 `WorkspaceState.description` 缓存字段——
/// 那可能滞后于磁盘,而 README 一旦生成就固化,必须用写入时的真实描述。
fn ensure_project_readme(repo: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let readme = repo.join("README.md");
    if !readme.exists() {
        let description = crate::project_meta::load_description(repo);
        let mut content = format!("# {}\n\n", name);
        if let Some(desc) = description {
            content.push_str(&desc);
            content.push('\n');
        }
        std::fs::write(&readme, content).ok()?;
    }
    // 已存在 → 跳过写入原样返回;刚生成 → 写入成功才走到这。最终都以"文件
    // 确实可读"为准——存在但读不了(如权限)就回 `None`,不拿去打开。
    if std::fs::read_to_string(&readme).is_ok() {
        Some(readme)
    } else {
        None
    }
}

/// `ws.preview`(Files)与 `ws.project_preview`(Project)是两个独立
/// `PreviewPane`,各自 `next_id` 从 0 起数——两者的 webview 一旦同时
/// 进同一个 `webviews` 池(`Files` 在左栏、`Project` 在右栏同时活跃时
/// 就会发生),原始 id 会撞(两边都可能是 0/1/2...)。给 `Project` 那
/// 一侧的 id 统一加这个偏移,`ws.preview` 侧不动——量级远超真实 tab
/// 数(几十个封顶),不会反向撞回 `ws.preview` 的 id 区间。main.rs 里
/// 任何按 id 反查 `ws.project_preview` webview(`active_preview_webview_id`
/// 的 Project 分支)都要用同一个偏移量加/减,两处不同步会导致查错池。
pub(crate) const PROJECT_PREVIEW_ID_OFFSET: usize = 1_000_000;

/// `Conversations` 面板的审阅 webview 只有唯一一份内容,不需要 Files/
/// Project 那种按 tab id 分池——固定用这一个 id(经 `review_webview_spec`
/// 的 `id: 0` 加这个偏移得到),与另两个偏移空间(`0` 起、`PROJECT_
/// PREVIEW_ID_OFFSET` 起)互不相撞。
pub(crate) const CONVERSATION_REVIEW_ID_OFFSET: usize = 2_000_000;

/// `wait_for_pending_exit_tasks` 允许在飞的关 tab 收尾请求跑完的总预算。
/// 本地 UDS 往返通常亚毫秒级,留 2 秒是给 daemon 偶尔卡顿的余量,而不是
/// 期望真正用满——超时后放弃等待,不能让退出被一个卡死的 daemon 拖住。
const EXIT_TASK_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

/// `App::wait_for_pending_exit_tasks` 的核心逻辑,拆成独立函数以便不依赖
/// `ShellIo`/`EventLoopProxy`(单测环境构造不出真实 winit 事件循环)直接
/// 测试:一批 spawn 任务在预算内全部跑完就正常返回,超预算就放弃等待
/// 并打日志,但**不会**无限期挂住调用方。
async fn join_pending_exit_tasks(
    tasks: Vec<tokio::task::JoinHandle<()>>,
    budget: std::time::Duration,
) {
    let joined = futures::future::join_all(tasks);
    if tokio::time::timeout(budget, joined).await.is_err() {
        tracing::warn!("退出前等待关 tab 的收尾请求超时,放弃等待");
    }
}

impl App {
    /// 启动序列成功路径:建好外壳态,再把上次退出时开着的**整份**项目页签
    /// 集合恢复出来。
    ///
    /// 恢复策略是"页签全恢复、内容懒加载"(设计文档 §2/§5):
    /// `open_projects.json` 里记着的每个项目都落一个 `Stub` 槽位——这一步不发
    /// 起任何网络请求,只是把页签栏画全,所以开着十个项目也不会拖慢启动;
    /// 只有上次聚焦的那一个立刻 `Workspace::bootstrap` 出完整状态(会话重挂/
    /// 文件树/git/对话),其余留在 `Stub`,用户点开时才由
    /// [`App::ensure_loaded`] 促成。
    ///
    /// 聚焦的那个这里**直接 `await`**、不走 `ensure_loaded` 的异步路径:窗口
    /// 还没建出来,此刻多等一个 UDS 往返不占用任何 UI 线程,换来的是第一帧
    /// 就是完整界面,而不是先闪一下空的"加载中"再补内容。
    ///
    /// 除了 `list_projects()`,这里还多做一次 `list()`(全量会话快照):`Stub`
    /// 页签靠它算出各自的后台活动指示点(见 [`stub_activity`])。一次协议往返
    /// 换"重启后后台页签的状态点不是空白",而且**不促成**任何 `Workspace`,
    /// 懒加载策略原样保留。
    pub async fn bootstrap(client: Client, handle: Handle, proxy: EventLoopProxy<Message>) -> Self {
        let mut app = Self::new_shell(client, handle, proxy, None);
        let io = app.shell_io();
        // daemon 不再记"活跃项目"(P2a Task 1-3 删掉了这个概念),开着哪些
        // 项目改由 GUI 侧的 open_projects.json 记(Task 4/6 写,这里读回)。
        let known = io.client.list_projects().await.unwrap_or_default();
        app.recent_projects = known.clone();
        let sessions = io.client.list().await.unwrap_or_default();
        let saved = open_projects::load();
        let (order, active) = restore_open_tabs(&known, &saved);
        for id in &order {
            let Some(info) = known.iter().find(|p| p.id == *id) else {
                continue; // restore_open_tabs 已过滤,这里只是让类型收敛
            };
            app.projects.insert(
                *id,
                WorkspaceSlot::Stub {
                    info: info.clone(),
                    activity: stub_activity(&sessions, *id),
                },
            );
        }
        let pruned = order != saved.project_ids || active != saved.active_project_id;
        app.project_order = order;
        app.active_project_id = active;
        // 启动恢复出的活跃项目,把它的面板布局换上来(没存过就退化成默认)。
        if let Some(id) = active {
            app.adopt_panel_layout(id);
        }
        if let Some(id) = active
            && let Some(WorkspaceSlot::Stub { info, .. }) = app.projects.get(&id)
        {
            let ws = Workspace::bootstrap(&io, info.clone()).await;
            app.projects.insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
        }
        // `restore_open_tabs` 会剔掉 daemon 已经不认识的 id、去重、必要时回落
        // 到最近项目——这些都是对盘上那份记录的修正,不写回去的话每次启动都要
        // 重算一遍,而且下次崩在写盘之前时盘上还是那份脏数据。
        if pruned {
            app.persist_open_projects();
        }
        app
    }

    /// daemon 连接失败（自动拉起 + 重试后仍不可用）时的降级构造：不做任何
    /// 会话/项目恢复，只记下错误文案，交给 `view()` 画 RED 文案。
    ///
    /// 这是旧 `Workspace::with_daemon_error` 的正确归宿——"daemon 连不上"
    /// 是整个程序共享的状态（`daemon_error` 现在长在 `App` 上），从来就不是
    /// 某一个项目自己的状态。
    pub fn with_daemon_error(
        client: Client,
        handle: Handle,
        proxy: EventLoopProxy<Message>,
        message: String,
    ) -> Self {
        Self::new_shell(client, handle, proxy, Some(message))
    }

    /// 两个构造函数共用的"只有外壳、一个项目都没打开"的起点。
    fn new_shell(
        client: Client,
        handle: Handle,
        proxy: EventLoopProxy<Message>,
        daemon_error: Option<String>,
    ) -> Self {
        let shell_layout = layout::load();
        let panel_layouts = panel_layouts::load();
        let shell = Self {
            client,
            handle,
            proxy,
            pending_exit_tasks: Arc::new(Mutex::new(Vec::new())),
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            ssh_cols: DEFAULT_COLS,
            ssh_rows: DEFAULT_ROWS,
            term_ime_preedit: None,
            daemon_error,
            last_todo_poll_at: std::time::Instant::now(),
            left_view: PanelLayout::default().left_view,
            right_view: PanelLayout::default().right_view,
            left_collapsed: PanelLayout::default().left_collapsed,
            right_collapsed: PanelLayout::default().right_collapsed,
            shell_layout,
            dims: PanelDims::default(),
            panel_layouts,
            maximized: None,
            active_zone: Some(ZoneSide::Right),
            hover_anims: std::collections::HashMap::new(),
            hover_tooltip_starts: std::collections::HashMap::new(),
            rail_slot_anims: std::collections::HashMap::new(),
            pending_zoom_toggle: false,
            project_add_menu_open: false,
            project_add_menu_anchor: (0.0, 0.0),
            pending_preview_zoom: false,
            window_size: byteui::theme::geometry::initial_window_size(),
            last_cursor: (0.0, 0.0),
            dragging: None,
            dragging_row: None,
            tab_drag: None,
            rail_drag: None,
            files: files::AppState::default(),
            preview_tab_menu: None,
            project_preview_tab_menu: None,
            project_link_menu: None,
            category_context_menu: None,
            category_picker: None,
            text_input_menu: None,
            database_source_menu: None,
            pending_text_input_focus: None,
            projects: HashMap::new(),
            project_order: Vec::new(),
            active_project_id: None,
            current_page: AppPage::Workspace,
            recent_projects: Vec::new(),
            home_recent_files: Vec::new(),
            home_recent_conversations: Vec::new(),
            home_recents_loaded: false,
            home_left_view: homespace::HomeLeftView::default(),
            home_project_pages: 1,
            home_project_search: String::new(),
            home_project_search_draft: String::new(),
            home_project_search_focused: false,
            home_right_view: homespace::HomeRightView::default(),
            home_browser: browser::State::with_initial_url("https://byteboy.ai"),
            git_log: git_log::State::default(),
            database: database::AppState::load(),
            footbar: footbar::AppState::default(),
        };
        // 启动 footbar 采样任务(fire-and-forget):runtime drop 时任务自然取消。
        let io = shell.shell_io();
        footbar::spawn_sampler(&io);
        shell
    }

    /// 外壳侧共享句柄的快照,交给项目态方法发起异步 IO(见 [`ShellIo`])。
    fn shell_io(&self) -> ShellIo {
        ShellIo {
            client: self.client.clone(),
            handle: self.handle.clone(),
            proxy: self.proxy.clone(),
            cols: self.cols,
            rows: self.rows,
            pending_exit_tasks: self.pending_exit_tasks.clone(),
        }
    }

    /// 按 `active_project_id` 取当前项目的 `Workspace` 只读引用；`Stub`
    /// 态和"没有任何页签"都返回 `None`（只读场景不促成加载）。
    pub fn active_workspace(&self) -> Option<&Workspace> {
        let id = self.active_project_id?;
        match self.projects.get(&id)? {
            WorkspaceSlot::Loaded(ws) => Some(ws),
            WorkspaceSlot::Stub { .. } => None,
        }
    }

    /// 外部 OS 文件拖拽命中测试：窗口坐标 (x, y) 在**当前**左侧是否落在
    /// 文件树某一行上，返回该行的落点结果(见 `files::DropHit` 文档:命中
    /// 文件行时 `target` 会退到其父目录)。main.rs 在 winit 原生事件层的
    /// `CursorMoved`/`DroppedFile` 上调用它——只要不在文件树上就返回
    /// `None`（此时拖入按现状落给终端）。
    ///
    /// 不做任何像素布局复制：文件树列的矩形由 [`left_files_tree_bounds_for`]
    /// 按 `preview_content_bounds_for` 同源的谱系换算，可见行集合从当前项目
    /// `WorkspaceState` 现取现算，二者与渲染侧 `files::view` 同源。
    pub fn files_drop_target(
        &self,
        window_w: f32,
        window_h: f32,
        x: f32,
        y: f32,
    ) -> Option<files::DropHit> {
        let side = self
            .shell_state()
            .layout
            .rail_layout
            .side_of(PanelKind::Files);
        let kind = match side {
            Side::Left => self.left_view,
            Side::Right => self.right_view,
        };
        let collapsed = match side {
            Side::Left => self.left_collapsed,
            Side::Right => self.right_collapsed,
        };
        if collapsed || kind != PanelKind::Files {
            return None;
        }
        let ws = self.active_workspace()?;
        let bounds = webview_geometry::left_files_tree_bounds_for(
            side,
            window_w,
            window_h,
            &self.shell_state(),
        );
        let rows = ws.files.visible_tree_rows();
        files::tree_drop_target(x, y, bounds, ws.files.tree_scroll(), &rows)
    }

    /// 拖拽(外部 OS 拖入/内部树拖拽共用)悬停命中一个仍处于折叠态的目录时
    /// 调用:不立即展开,武装 `files::DRAG_HOVER_EXPAND_DELAY` 计时——真正
    /// 展开推迟到 `advance_drag_hover_expand` 满时才做,见 `WorkspaceState::
    /// drag_expand_pending` 文档(立即展开会让拖着划过沿途目录疯狂跳动
    /// 布局,2026-09 用户实测反馈)。已展开(或查不到该行,当已展开处理)
    /// 直接清空计时。
    pub fn arm_drag_hover_expand(&mut self, dir: &std::path::Path) {
        let Some(ws) = self.active_workspace() else {
            return;
        };
        let already_expanded = ws
            .files
            .visible_tree_rows()
            .iter()
            .find(|r| r.path == dir)
            .is_none_or(|r| r.expanded);
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        ws.files
            .arm_drag_expand(dir.to_path_buf(), already_expanded);
    }

    /// 悬停离开文件树、拖拽收尾/取消时调用:清空展开计时,不留残留状态。
    pub fn clear_drag_hover_expand(&mut self) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.files.clear_drag_expand();
        }
    }

    /// `about_to_wait`/`ResumeTimeReached` 每次唤醒都调:当前项目悬停中的
    /// 目录若已计满 `DRAG_HOVER_EXPAND_DELAY`,发 `Message::Toggle` 真正
    /// 展开并清空计时;未满或没有悬停中的目录都是 no-op。
    pub fn advance_drag_hover_expand(&mut self) {
        let Some(dir) = self
            .active_workspace()
            .and_then(|ws| ws.files.drag_expand_ready())
        else {
            return;
        };
        self.update(Message::Files(files::Message::Toggle(dir)));
        if let Some(ws) = self.active_workspace_mut() {
            ws.files.clear_drag_expand();
        }
    }

    /// 距当前项目的展开计时满 1s 的剩余时间,`about_to_wait` 据此排精确
    /// 唤醒(同 `next_tooltip_wake` 手法)。没有悬停中的目录时返回 `None`。
    pub fn next_drag_hover_expand_wake(&self) -> Option<std::time::Duration> {
        self.active_workspace()
            .and_then(|ws| ws.files.next_drag_expand_wake())
    }

    /// 同上，可变引用版本；如果对应槽位是 `Stub`，就地促成 `Loaded`
    /// （拉取该项目的完整状态）后再返回引用。
    pub fn active_workspace_mut(&mut self) -> Option<&mut Workspace> {
        let id = self.active_project_id?;
        self.ensure_loaded(id);
        match self.projects.get_mut(&id)? {
            WorkspaceSlot::Loaded(ws) => Some(ws),
            WorkspaceSlot::Stub { .. } => None,
        }
    }

    /// `Stub` → `Loaded` 的促成:两步走。
    ///
    /// 1. **同步**把槽位换成 [`Workspace::loading_for_project`] 的"加载中"占位
    ///    ——用户点开一个 `Stub` 页签的那一刻画面就该有反应(项目名/文件树根
    ///    立刻出来),不能等异步任务跑完才有任何视觉变化。这一步之所以必须
    ///    是**带 `project` 的**占位而不是空壳,见 `loading_for_project` 的
    ///    文档:异步窗口期里这个 `Workspace` 是可达的当前项目,`project` 为
    ///    `None` 会让 `spawn_new_tab` 的 `expect` 重新变成可达路径。
    /// 2. `handle.spawn` 一个任务跑促成的 IO 段
    ///    [`fetch_project_restore`](重挂该项目的存活会话 + 最近项目列表),
    ///    完成后经 `EventLoopProxy` 回送 `Message::ProjectSlotLoaded`;装配段
    ///    (`Workspace::from_restore`)在 `update` 里、UI 线程上跑,把占位换成
    ///    真正的结果。两段之所以不能合成一段,见 [`ProjectRestore`]。
    ///
    /// 已经是 `Loaded`(含正在促成的占位)或 id 根本不在槽位表里时都是 no-op
    /// ——尤其"占位已在"这条保证了同一个页签连点几下不会重复发起促成。
    fn ensure_loaded(&mut self, id: i64) {
        let Some(WorkspaceSlot::Stub { info, .. }) = self.projects.get(&id) else {
            return;
        };
        let info = info.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        self.projects.insert(
            id,
            WorkspaceSlot::Loaded(Box::new(Workspace::loading_for_project(info.clone()))),
        );
        self.handle.spawn(async move {
            let restore = fetch_project_restore(&client, info).await;
            let _ = proxy.send_event(Message::ProjectSlotLoaded(id, RestorePayload::new(restore)));
        });
    }

    /// 当前聚焦的项目 id(main.rs 的 webview 池按它分家,见
    /// `sync_previews`)。`None` = 一个项目页签都没开。
    pub fn active_project_id(&self) -> Option<i64> {
        self.active_project_id
    }

    /// 把"现在开着哪些项目页签、什么顺序、哪个在前台"写盘(Task 4 的
    /// `open_projects`)。页签集合每次变动都调一次——写的是一个几十字节的
    /// JSON,和 `preview_state`/`layout` 走同一套 `handle.spawn` 异步落盘,
    /// 不阻塞 UI 线程。
    ///
    /// 读回来的一侧目前只有 `App::bootstrap` 用了 `active_project_id`(启动
    /// 恢复上次聚焦的那个项目);把 `project_ids` 整份恢复成多页签是 Task 7
    /// 的活儿,那时这里已经有正确的数据可读。
    fn persist_open_projects(&self) {
        let state = open_projects::OpenProjectsState {
            project_ids: self.project_order.clone(),
            active_project_id: self.active_project_id,
        };
        self.handle.spawn(async move {
            if let Err(e) = open_projects::save(&state) {
                tracing::warn!("项目页签集合写盘失败: {e}");
            }
        });
    }

    /// `update()` 里"这条消息只动项目态"的统一入口:先取一份外壳句柄快照
    /// （[`ShellIo`]），再拿当前项目的 `Workspace`，一并交给闭包。没有任何
    /// 项目打开时整条消息丢弃——项目态消息没有归属就无处可落。
    ///
    /// 闭包里的 `return` 就是"提前结束这条消息的处理"，与搬家前写在
    /// `match` 分支里的 `return` 语义一致。
    /// **只用于用户点击当前界面直接触发的消息**——"作用于我此刻看着的那个
    /// 项目"正是它们的语义。异步结果消息一律**不能**走这里,必须用
    /// [`App::with_project`] 按消息自带的 `project_id` 投递,理由见
    /// [`ProjectId`]。
    ///
    /// 名字从早先的 `with_ws` 改成现在这个,是因为它与 [`App::with_project`]
    /// 的安全性质**相反**(一个投给"此刻聚焦的",一个投给"指定 id 的"),而
    /// 更短、看起来更像默认选项的那个偏偏是用错了会串项目的那个。名字必须
    /// 一眼看出差别,不能靠读文档才知道选哪个。
    fn with_focused_project(&mut self, f: impl FnOnce(&mut Workspace, &ShellIo)) {
        let io = self.shell_io();
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        f(ws, &io);
    }

    /// `update()` 里"这条异步结果属于**某个指定项目**"的统一入口:按
    /// `project_id` 直接投递到对应槽位,与"此刻聚焦的是谁"完全无关。
    ///
    /// 这是多项目并行的核心路由不变式(见 [`ProjectId`]):后台项目的会话
    /// 输出/退出/交付检测结果绝不能落到前台项目身上。
    ///
    /// 投不到时静默丢弃,不报错:
    /// - 槽位不存在 = 用户在结果回来之前把这个页签关掉了,他已经不关心了;
    /// - 槽位是 `Stub` = 这个项目还没促成过,不可能有任何在飞的会话结果指向
    ///   它(促成期的"加载中"占位是 `Loaded`,不落进这一支)。
    fn with_project(&mut self, project_id: ProjectId, f: impl FnOnce(&mut Workspace, &ShellIo)) {
        let io = self.shell_io();
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        f(ws, &io);
    }

    /// 当前是否"正看着"某个项目的 Todo 面板——轮询是否要继续排下一拍
    /// 唤醒的判断条件（`main.rs::about_to_wait`），跟
    /// `any_hover_anim_active` 同一层级。
    pub fn todo_panel_visible(&self) -> bool {
        self.left_view == PanelKind::Todo && self.active_workspace().is_some()
    }

    /// `main.rs` 定时唤醒调用：只在 `todo_panel_visible()` 时真的拉取一次
    /// `dozerd` 侧的任务列表(经 `request_todos_refresh` 异步 `Loaded` 落回
    /// `ws.todo.items`),兼顾响应与省电。按 `last_todo_poll_at` 自限速到
    /// `TODO_POLL_INTERVAL`——悬停动画等更快节奏把唤醒带密时不会跟着高频
    /// 重复发请求(2026-08-12 解耦重构)。
    pub fn poll_todo_if_visible(&mut self) {
        if !self.todo_panel_visible() {
            return;
        }
        let now = std::time::Instant::now();
        if now.duration_since(self.last_todo_poll_at) < crate::TODO_POLL_INTERVAL {
            return;
        }
        self.last_todo_poll_at = now;
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m: todo::Message| {
            let _ = proxy.send_event(Message::Todo(m));
        };
        let emit_todos = emit.clone();
        todo::request_todos_refresh(project_id, &client, &handle, emit_todos);
        todo::request_categories_refresh(project_id, &client, &handle, emit);
    }

    /// 设置某按钮的悬停目标（`true`=进入,`false`=离开）；动画由
    /// `advance_hover_anims` 循环把它指数逼近（见 `HoverAnim`）。
    pub fn set_hover(&mut self, id: HoverId, hovered: bool) {
        self.hover_anims.entry(id).or_default().set(hovered);
        // 标题 tooltip 计时:进入即记起点,离开即清(计时满 2s 由视图层
        // `hover_tooltip_ready` 判断,本函数只负责起止)。
        if hovered {
            self.hover_tooltip_starts
                .insert(id, std::time::Instant::now());
        } else {
            self.hover_tooltip_starts.remove(&id);
        }
    }

    /// 推进所有按钮的悬停动画一拍（约 60fps 一拍，由 main.rs 的定时唤醒
    /// 驱动；与光标闪烁同款自驱 redraw 范式）。每拍残余 50%（逼近系数
    /// 0.5），约 80ms 内收敛到目标，视觉上是干脆的 ease-out。
    pub fn advance_hover_anims(&mut self) {
        for a in self.hover_anims.values_mut() {
            a.advance();
        }
        // 浏览器面板有独立 hover 进度机(无法复用全局 `hover_anims`),这里
        // 一并推进:首页 `home_browser` 与每个已加载工作区的浏览器。
        self.home_browser.advance_hover_anims();
        for slot in self.projects.values_mut() {
            if let WorkspaceSlot::Loaded(ws) = slot {
                ws.browser.advance_hover_anims();
            }
        }
        self.advance_rail_slot_anims();
    }

    /// 是否还有按钮的悬停动画在进行中（任一进度未到目标）。
    /// main.rs 据此决定是否继续排下一拍定时唤醒。
    pub fn any_hover_anim_active(&self) -> bool {
        self.hover_anims.values().any(HoverAnim::active)
            || self.home_browser.any_hover_active()
            || self
                .projects
                .values()
                .any(|s| matches!(s, WorkspaceSlot::Loaded(ws) if ws.browser.any_hover_active()))
            || self.any_rail_slot_anim_active()
    }

    /// 推进图标栏按钮的槽位动画一拍——两侧各自按 `rail_layout` 当前顺序
    /// 现算每个面板的目标槽位号,`RailSlotAnim::retarget` 朝它逼近。折进
    /// `advance_hover_anims` 同一个调用点,复用同一套 60fps 自驱 redraw
    /// 节奏,不需要 main.rs 另开一条唤醒源。
    fn advance_rail_slot_anims(&mut self) {
        rail::advance_slot_anims(&self.shell_layout.rail_layout, &mut self.rail_slot_anims);
    }

    /// 是否还有图标栏按钮的槽位动画在进行中。
    fn any_rail_slot_anim_active(&self) -> bool {
        rail::any_slot_anim_active(&self.shell_layout.rail_layout, &self.rail_slot_anims)
    }

    /// `kind` 在 `side` 栏当前应渲染的动画槽位号(浮点,逼近中的
    /// `rail_layout` 下标)。渲染层据此算按钮的 y 偏移,取代直接用
    /// `rail_layout` 下标瞬间跳变。没有动画记录(刚出现在这一侧,还没被
    /// `advance_rail_slot_anims` 追上)或记录的 `side` 跟当前不符(刚跨栏
    /// 落地那一帧)时,直接返回目标槽位号本身,不插值。
    pub(crate) fn rail_slot_position(&self, side: Side, kind: PanelKind, target_idx: usize) -> f32 {
        rail::slot_position(&self.rail_slot_anims, side, kind, target_idx)
    }

    /// 某按钮当前悬停动画进度(0..=1)，给视图层做颜色插值。
    pub fn hover_progress(&self, id: HoverId) -> f32 {
        self.hover_anims.get(&id).map(HoverAnim::t).unwrap_or(0.0)
    }

    /// 某元素当前是否处于 hover **目标态**(0/1,不做平滑插值)。卡片填充/描边
    /// 这类二元视觉用这个:与 `button` 卡的原生 `button::Status::Hovered` 同
    /// 语义(瞬时切换),不像 `hover_progress` 那样带 ease-out 淡入淡出——图标
    /// 颜色过渡需要平滑,卡片背景/边框切换需要干脆,避免 hover 离开后边框还
    /// 拖着淡出一段(观感像"动画停了一下")。
    pub fn hover_target(&self, id: HoverId) -> bool {
        self.hover_anims
            .get(&id)
            .map(|a| a.target > 0.5)
            .unwrap_or(false)
    }

    /// 某页签悬停是否已持续满 `HOVER_TOOLTIP_DELAY`:满则应在视图层弹出标题
    /// 全称 tooltip(`controlled_tooltip` 据此驱动 `Tooltip::show`)。
    pub fn hover_tooltip_ready(&self, id: HoverId) -> bool {
        self.hover_tooltip_starts
            .get(&id)
            .is_some_and(|start| start.elapsed() >= HOVER_TOOLTIP_DELAY)
    }

    /// 距下一个 tooltip 计时满 2s 的最短剩余时间:main.rs 据此排下次唤醒,做到
    /// "恰好满 2s 才重绘",不空转也不延迟。浏览器面板页签的计时一并纳入。
    pub fn next_tooltip_wake(&self) -> Option<std::time::Duration> {
        let mut next = self
            .hover_tooltip_starts
            .values()
            .filter_map(|start| HOVER_TOOLTIP_DELAY.checked_sub(start.elapsed()))
            .min();
        let browser_next = self
            .home_browser
            .next_tooltip_wake()
            .into_iter()
            .chain(self.projects.values().filter_map(|s| match s {
                WorkspaceSlot::Loaded(ws) => ws.browser.next_tooltip_wake(),
                _ => None,
            }))
            .min();
        next = match (next, browser_next) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        next
    }

    /// 拖拽换位:把当前拖起的源项(`self.tab_drag.source`)移到 `group` 组的
    /// `to` 处。源与目标同址/越界/不在拖拽中/未越过 [`tab_drag_past_threshold`]
    /// 均 no-op(阈值过滤见其文档——挡的是单击途中的抖动,不是真实拖拽)。
    /// 换位后把 `self.tab_drag.source` 更新成新位置(续拖以新位置为准),并
    /// 顺带修正受影响的 index-keyed hover 键/激活项。
    fn tab_drag_move(&mut self, group: TabGroup, to: usize) {
        let Some(drag) = self.tab_drag else {
            return;
        };
        if !tab_drag_past_threshold(drag.press_pos, self.last_cursor) {
            return;
        }
        if drag.group != group {
            return;
        }
        let from = drag.source;
        match group {
            TabGroup::Project => {
                if from == to || from >= self.project_order.len() || to >= self.project_order.len()
                {
                    return;
                }
                let id = self.project_order.remove(from);
                self.project_order.insert(to, id);
                self.tab_drag = Some(TabDrag {
                    group,
                    source: to,
                    press_pos: drag.press_pos,
                });
                // 项目页签 hover 存的是 id-keyed 键(`HoverId::ProjectTabItem`/
                // `ProjectTabClose`),值本身不会因换位错配到别的项目——但换位
                // 让页签在**树里的位置**跟着挪，`MouseArea` 自己那份按位置续存
                // 的悬停内部状态（`is_hovered` 等）跟这份 id-keyed 记录对不上
                // 了：原来悬停中的那个 id，它的 `MouseArea` 实例已经换到别的
                // 树位置，不会再收到 `on_exit`，`hover_anims` 里的值就砸在原地
                // 出不来，页签松手后一直亮着金色胶囊（换位越频繁越容易撞上）。
                // 没有更细粒度的续存机制（`MouseArea` 不支持 `.id()`），换位
                // 时索性把两类项目页签 hover 全部清零最省事——真实悬停哪个,
                // 下一帧鼠标移动会立刻重新点亮,观感上无感知。
                self.hover_anims.retain(|k, _| {
                    !matches!(k, HoverId::ProjectTabItem(_) | HoverId::ProjectTabClose(_))
                });
            }
            TabGroup::Terminal => {
                if from == to {
                    return;
                }
                {
                    let Some(ws) = self.active_workspace_mut() else {
                        return;
                    };
                    if to >= ws.tabs.len() {
                        return;
                    }
                    ws.reorder_term_tab(from, to);
                }
                self.tab_drag = Some(TabDrag {
                    group,
                    source: to,
                    press_pos: drag.press_pos,
                });
                self.rekey_hover_range(HoverId::TermTabItem, HoverId::TermTabClose, from, to);
            }
            TabGroup::Preview => {
                if from == to {
                    return;
                }
                {
                    let Some(ws) = self.active_workspace_mut() else {
                        return;
                    };
                    if to >= ws.preview.tabs().len() {
                        return;
                    }
                    ws.preview.reorder(from, to);
                }
                self.tab_drag = Some(TabDrag {
                    group,
                    source: to,
                    press_pos: drag.press_pos,
                });
                self.rekey_hover_range(HoverId::PreviewTabItem, HoverId::PreviewTabClose, from, to);
            }
            TabGroup::ProjectPreview => {
                if from == to {
                    return;
                }
                {
                    let Some(ws) = self.active_workspace_mut() else {
                        return;
                    };
                    if to >= ws.project_preview.tabs().len() {
                        return;
                    }
                    ws.project_preview.reorder(from, to);
                }
                self.tab_drag = Some(TabDrag {
                    group,
                    source: to,
                    press_pos: drag.press_pos,
                });
                self.rekey_hover_range(
                    HoverId::ProjectPreviewTabItem,
                    HoverId::ProjectPreviewTabClose,
                    from,
                    to,
                );
            }
            TabGroup::Browser => {
                if from == to {
                    return;
                }
                {
                    let Some(ws) = self.active_workspace_mut() else {
                        return;
                    };
                    if to >= ws.browser.tab_count() {
                        return;
                    }
                    ws.browser.reorder_tab(from, to);
                }
                self.tab_drag = Some(TabDrag {
                    group,
                    source: to,
                    press_pos: drag.press_pos,
                });
            }
        }
    }

    /// 给 `from..=to` 区间的 index-keyed hover 键整体顺移一位,使其跟上拖拽换位
    /// 后的条目位置:`Item(i)` 与 `Close(i)` 两种键都必须跟着槽位移。`item_f`/
    /// `close_f` 是构造 `HoverId` 的两个构造器(终端/预览各自的 `Item`/`Close`
    /// 变体)。键在拖拽期间通常无动画在跑(拖走即离开),把它们重排到正确槽位
    /// 即可,不追求平滑。
    fn rekey_hover_range(
        &mut self,
        item_f: fn(usize) -> HoverId,
        close_f: fn(usize) -> HoverId,
        from: usize,
        to: usize,
    ) {
        let lo = from.min(to);
        let hi = from.max(to);
        // 取旧槽上每个键的当前动画值,再按换位后的新槽写回。
        let mut remap = Vec::new();
        for i in lo..=hi {
            for f in [item_f, close_f] {
                if let Some(h) = self.hover_anims.remove(&f(i)) {
                    let new_i = if i == from {
                        to
                    } else if from < to {
                        // 向右拖:中间 from+1..=to 全左移一位。
                        i - 1
                    } else {
                        // 向左拖:中间 to..from 全右移一位。
                        i + 1
                    };
                    remap.push((f(new_i), h));
                }
            }
        }
        for (id, h) in remap {
            self.hover_anims.insert(id, h);
        }
    }

    /// 当前激活 tab 的选区文本（⌘C 复制用）。
    pub fn active_selection_text(&self) -> Option<String> {
        self.active_workspace()?.active_selection_text()
    }

    /// 浏览器地址栏是否持有 iced 真实焦点(main.rs 据此路由键盘)。首页
    /// (`is_home()`)展示的是 `home_browser`(全局浏览器,`App` 级独立
    /// 实例,不属于任何 `Workspace`),跟工作区里打开的 `ws.browser` 是
    /// 两个不同的地址栏——两者共用同一个 `addr_field_id()`(不会同时渲染,
    /// 不会冲突),但读写目标必须按当前在首页还是工作区分流,否则首页地址栏
    /// 的焦点态永远查不到(`active_workspace()` 在首页上恒为 `None`)。
    pub fn browser_addr_focused(&self) -> bool {
        if self.is_home() {
            return self.home_browser.addr_focused();
        }
        self.active_workspace()
            .is_some_and(|ws| ws.browser_addr_focused())
    }

    /// 每帧渲染循环调用:把 `extensions::browser::CaptureAddrFocus` 问到
    /// 的真实焦点态写进正确的浏览器实例(首页 `home_browser` 或当前工作区
    /// `ws.browser`,见 `browser_addr_focused` 的说明)。
    pub fn set_browser_addr_focused(&mut self, focused: bool) {
        if self.is_home() {
            self.home_browser.set_addr_focused(focused);
            return;
        }
        if let Some(ws) = self.active_workspace_mut() {
            ws.browser.set_addr_focused(focused);
        }
    }

    /// 取走"地址栏需全选"的一次性标记(消费即复位),路由同
    /// `set_browser_addr_focused`(首页 `home_browser` / 当前工作区
    /// `ws.browser` 二选一)。
    pub fn take_addr_select_all_pending(&mut self) -> bool {
        if self.is_home() {
            return self.home_browser.take_addr_select_all_pending();
        }
        self.active_workspace_mut()
            .is_some_and(|ws| ws.browser.take_addr_select_all_pending())
    }

    /// 项目树行内编辑框是否持有 iced 真实焦点(main.rs 键盘路由用)。为真时
    /// 按键放行给标准 iced 事件管线,交真正的 text_input 自己处理。
    pub fn tree_edit_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.tree_edit_focused())
    }

    /// 每帧渲染循环调用:把 `extensions::files::CaptureTreeEditFocus` 问到的
    /// 真实焦点态写进当前工作区(`main.rs` 键盘路由随后读
    /// `tree_edit_focused` 消费)。焦点从真变假(失焦)时在边缘处落盘——
    /// 项目树重命名/新建是"点别处就该保存"的一次性行内编辑,直接调
    /// `ws.files.submit_tree_edit` 走与回车提交(`Message::EditSubmit`)同一
    /// 份逻辑(空名字/未改动会自行退出编辑态,不产生文件/重命名)。
    pub fn set_tree_edit_focused(&mut self, focused: bool) {
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let was_focused = ws.tree_edit_focused();
        if was_focused && !focused && ws.files.tree_edit_is_some() {
            let emit = move |m: files::Message| {
                let _ = proxy.send_event(Message::Files(m));
            };
            ws.files.submit_tree_edit(project_id, &handle, emit);
        }
        ws.files.set_tree_edit_focused(focused);
    }

    /// 文件树搜索框是否持有 iced 真实焦点(main.rs 键盘路由用)。为真时按键
    /// 放行给标准 iced 事件管线,交真正的 text_input 自己处理。
    pub fn files_search_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.files_search_focused())
    }

    /// SSH 连接表单是否打开(main.rs 键盘路由用)。
    pub fn ssh_form_open(&self) -> bool {
        self.active_workspace().is_some_and(|ws| ws.ssh_form_open())
    }

    /// Database 连接表单是否打开(main.rs 键盘路由用)。
    pub fn database_form_open(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.database_form_open())
    }

    /// 拖拽移动确认框是否打开(main.rs 键盘路由用)。同 SSH/Database 表单
    /// 的粗粒度口径——框里只有两个字段,整体放行不细分哪个字段真正持有
    /// 焦点,见 `WorkspaceState::pending_move_is_some` 文档。
    pub fn files_move_confirm_open(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.files.pending_move_is_some())
    }

    /// 每帧渲染循环调用:把 `extensions::files::CaptureSearchFocus` 问到
    /// 的真实焦点态写进当前工作区(`main.rs` 键盘路由随后读
    /// `files_search_focused` 消费)。
    pub fn set_files_search_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.files.set_search_focused(focused);
        }
    }

    /// 右键文件树"搜索"弹窗是否打开(main.rs 键盘路由 + App view 浮层用)。
    pub fn search_popup_open(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.search_popup_open())
    }

    /// 右键文件树"搜索"弹窗查询框是否持有 iced 真实焦点(main.rs 原生放行
    /// 闸门用)。
    pub fn query_focused(&self) -> bool {
        self.active_workspace().is_some_and(|ws| ws.query_focused())
    }

    /// 每帧渲染循环读走 `CaptureQueryFocus` 查到的真实焦点态后写进当前
    /// 工作区。
    pub fn set_query_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.search.set_query_focused(focused);
        }
    }

    /// 项目信息面板名称编辑框是否持有 iced 真实焦点(main.rs 原生放行闸门用)。
    pub fn project_name_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.name_edit_focused())
    }

    /// 每帧渲染循环调用:把 `CaptureNameEditFocus` 问到的真实焦点态写进
    /// 当前工作区,并在"焦点从真变假"的那一刻做落盘判断(同 `App::
    /// set_todo_content_focused` 的既有手法,`submit_name_edit` 走这条
    /// 而不是重新手写一份 `rename_project` 调用)。
    pub fn set_project_name_focused(&mut self, focused: bool) {
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let was_focused = ws.name_edit_focused();
        if was_focused
            && !focused
            && let Some(project) = ws.project.clone()
        {
            let emit = move |m: project::Message| {
                let _ = proxy.send_event(Message::Project(m));
            };
            project::submit_name_edit(
                &mut ws.project_panel,
                project.id,
                &project.name,
                client,
                &handle,
                emit,
            );
        }
        ws.project_panel.set_name_edit_focused_flag(focused);
    }

    /// Todo 面板搜索框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn todo_search_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.search_focused())
    }

    /// 每帧渲染循环调用:把 `extensions::todo::CaptureTodoSearchFocus` 问到
    /// 的真实焦点态写进当前工作区的 Todo(`main.rs` 键盘路由随后读
    /// `todo_search_focused` 消费)。
    pub fn set_todo_search_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.todo.set_search_focused(focused);
        }
    }

    /// 会话列表搜索框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn conversation_search_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.conversation_search_focused)
    }

    /// 每帧渲染循环调用:把 `workspace::CaptureConversationSearchFocus` 问到
    /// 的真实焦点态写进当前工作区(`main.rs` 键盘路由随后读
    /// `conversation_search_focused` 消费)。
    pub fn set_conversation_search_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.conversation_search_focused = focused;
        }
    }

    /// Git Log 面板 commit 搜索框是否持有 iced 真实焦点(main.rs 键盘路由
    /// 用)。`git_log::State` 不按项目分(同 `home_project_search_focused`
    /// 直接挂在 `App` 上,不用走 `active_workspace` 那套间接)。
    pub fn git_log_search_focused(&self) -> bool {
        self.git_log.search_focused()
    }

    /// 每帧渲染循环调用:把 `extensions::git_log::CaptureSearchFocus` 问到
    /// 的真实焦点态写进 `git_log::State`(`main.rs` 键盘路由随后读
    /// `git_log_search_focused` 消费)。
    pub fn set_git_log_search_focused(&mut self, focused: bool) {
        self.git_log.set_search_focused(focused);
    }

    /// 首页项目搜索框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn home_project_search_focused(&self) -> bool {
        self.home_project_search_focused
    }

    /// 每帧渲染循环调用:把 `CaptureHomeSearchFocus` 问到的真实焦点态
    /// 写进来。
    pub fn set_home_project_search_focused(&mut self, focused: bool) {
        self.home_project_search_focused = focused;
    }

    /// 搜索框草稿落成为生效的 `home_project_search` 过滤词,并把翻页
    /// 重置回第 1 页(同 `todo::WorkspaceState::commit_search`)。
    fn commit_home_project_search(&mut self) {
        self.home_project_search = self.home_project_search_draft.clone();
        self.home_project_pages = 1;
    }

    /// Todo 面板新增任务框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn todo_add_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.add_focused())
    }

    /// 每帧渲染循环调用:把 `extensions::todo::CaptureAddFocus` 问到的真实
    /// 焦点态写进当前工作区的 Todo(`main.rs` 键盘路由随后读
    /// `todo_add_focused` 消费)。
    pub fn set_todo_add_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.todo.set_add_focused(focused);
        }
    }

    /// Todo 任务内容编辑框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn todo_content_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.content_edit_focused())
    }

    /// 每帧渲染循环调用:把 `CaptureContentEditFocus` 问到的真实焦点态
    /// 写进当前工作区的 Todo,并在"焦点从真变假"的那一刻做落盘判断
    /// (同 `Workspace::blur_inputs` 原先的"有项目就 commit、没项目就
    /// cancel"逻辑,只是触发时机从"点击别处"改成"真实焦点丢失")。
    pub fn set_todo_content_focused(&mut self, focused: bool) {
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let was_focused = ws.todo.content_edit_focused();
        // 失焦回退:取走待提交的草稿改动(`commit_content_edit` 返回
        // `Some((id, new_text))` 表示草稿确有改动,且会消费 `editing_content`);
        // 无改动/空草稿返回 `None`,到此随 `editing_content` 一并丢弃。
        let pending_commit = if was_focused && !focused {
            ws.todo.commit_content_edit()
        } else {
            None
        };
        ws.todo.set_content_edit_focused_flag(focused);
        // 先把 `ws` 的借用放掉,再经 self 的 client/handle/proxy 发起异步提交
        // (否则 `active_workspace_mut` 对 `self` 的可变借用会挡住 `self.client`)。
        if let Some((id, new_text)) = pending_commit {
            let client = self.client.clone();
            let handle = self.handle.clone();
            let proxy = self.proxy.clone();
            handle.spawn(async move {
                let res = client
                    .edit_todo_text(id, &new_text)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                let _ = proxy.send_event(Message::Todo(todo::Message::Mutated(res)));
            });
        }
    }

    /// 分类改名框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn category_rename_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.category_rename_focused())
    }

    /// 分类改名框真实焦点态每帧写回;失焦边缘(`was_focused && !focused`)
    /// 触发一次提交(镜像 `set_todo_content_focused`,只是落盘方法换成
    /// `rename_category`,成功/失败都触发 `CategoryMutated` 刷新)。
    pub fn set_category_rename_focused(&mut self, focused: bool) {
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let was_focused = ws.todo.category_rename_focused();
        let pending_commit = if was_focused && !focused {
            ws.todo.commit_category_rename_for_blur()
        } else {
            None
        };
        ws.todo.set_category_rename_focused_flag(focused);
        if let Some((id, new_name)) = pending_commit {
            let client = self.client.clone();
            let handle = self.handle.clone();
            let proxy = self.proxy.clone();
            handle.spawn(async move {
                let res = client
                    .rename_category(id, &new_name)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                let _ = proxy.send_event(Message::Todo(todo::Message::CategoryMutated(res)));
            });
        }
    }

    /// 详情弹窗回复框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn detail_reply_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.detail_reply_focused())
    }

    /// 每帧渲染循环调用:把 `CaptureDetailReplyFocus` 问到的真实焦点态
    /// 写进当前工作区的 Todo(`main.rs` 键盘路由随后读 `detail_reply_focused`
    /// 消费)。这个输入没有"失焦提交"的语义(提交靠点"处理"按钮,不是
    /// 失焦/回车),所以不需要 `category_rename_focused`/`todo_content_focused`
    /// 那种"失焦边缘触发落盘"的逻辑,直接写回标记位即可。
    pub fn set_detail_reply_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.todo.set_detail_reply_focused_flag(focused);
        }
    }

    /// 当前左栏显示哪个面板(main.rs 每帧 `interface.operate` 捕获 Todo 自绘
    /// 输入字段 bounds 时用来判断是否要遍历,避免无谓开销)。
    pub fn left_view(&self) -> PanelKind {
        self.left_view
    }

    /// 是否正在拖拽 Todo 任务排序(main.rs 鼠标释放路由 + about_to_wait
    /// 持续重绘用;同 `dragging_tab` 那套)。
    pub fn todo_dragging(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.drag_active())
    }

    /// 距新增闪光自动清除的剩余时间:main.rs 据此排下次唤醒,恰好到点重绘
    /// 一次清除高亮(同 `next_tooltip_wake` 的定时范式)。
    pub fn next_todo_flash_wake(&self) -> Option<std::time::Duration> {
        self.active_workspace()?.todo.next_flash_wake()
    }

    /// 推进新增闪光倒计时(每帧 `new_events` 调用):到点且用户未手动改选则
    /// 自动清除选中高亮。
    pub fn advance_todo_flash(&mut self) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.todo.advance_flash();
        }
    }

    /// 取走"Todo 列表滚回顶部"的一次性滚动位(main.rs 渲染循环消费)。
    pub fn take_todo_scroll_to_top(&mut self) -> bool {
        self.active_workspace_mut()
            .is_some_and(|ws| ws.todo.take_scroll_to_top())
    }

    /// 当前项目根路径(供 main.rs 算相对路径用;未打开项目时 None)。
    pub fn active_project_path(&self) -> Option<PathBuf> {
        self.active_workspace()?.active_project_path()
    }

    /// 当前激活 tab 是否处于 application cursor mode（DECCKM）。
    pub fn active_app_cursor_mode(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.active_app_cursor_mode())
    }

    /// `kind` 是 `is_in_preview_column` 命中的面板(`Files` 或 `Project`),
    /// 据此查 `ws.preview`(Files)还是 `ws.project_preview`(Project)——后者
    /// 的 id 已加 `PROJECT_PREVIEW_ID_OFFSET`(见 workspace.rs 同名方法)。
    /// main.rs 焦点路由取句柄用。
    pub fn active_preview_webview_id(&self, kind: PanelKind) -> Option<usize> {
        self.active_workspace()?.active_preview_webview_id(kind)
    }

    /// 当前哪个预览面板有活跃 webview(`Files`/`Project`/`None`),
    /// 语义见 workspace.rs 同名方法。`WebViewFocused` 这种不携带面板
    /// 信息的信号需要反推池身份时用。
    pub fn active_preview_panel_kind(&self) -> Option<PanelKind> {
        self.active_workspace()?.active_preview_panel_kind()
    }

    /// 当前激活浏览器 tab 的 webview id,语义同 `active_preview_webview_id`。
    pub fn active_browser_webview_id(&self) -> Option<usize> {
        if self.current_page == AppPage::Home {
            return self.home_browser.active_webview_id();
        }
        self.active_workspace()?.active_browser_webview_id()
    }

    /// 当前是否在首页(`AppPage::Home`)——内核(main.rs 的 webview 池同步)
    /// 据此判断浏览器 webview 该用右面板区边界还是工作区左面板预览边界。
    pub(crate) fn is_home(&self) -> bool {
        self.current_page == AppPage::Home
    }

    /// 协议闭包共享的文件白名单句柄(当前项目的那一份)。没有项目打开时
    /// `preview_desired`/`browser_desired` 也必然为空、不会有 webview 去查
    /// 这个白名单,给一个空的即可。
    pub fn allowed_files(&self) -> Arc<Mutex<HashSet<PathBuf>>> {
        match self.active_workspace() {
            Some(ws) => ws.allowed_files(),
            None => Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// 协议闭包共享的审阅内容快照句柄(当前项目的那一份)——同
    /// `allowed_files` 的手法,webview 创建时按聚焦项目捕获,天然做到
    /// per-project 隔离,不会跨项目串数据。
    pub fn review_snapshot(&self) -> Arc<Mutex<Option<String>>> {
        match self.active_workspace() {
            Some(ws) => ws.review_snapshot(),
            None => Arc::new(Mutex::new(None)),
        }
    }

    /// 点击输入框外时退出所有自绘输入的编辑态(验收反馈:失焦回正常态)。
    /// 项目名称编辑的"失焦保存"已搬进 `set_project_name_focused` 的边缘触发
    /// (与回车提交共用 `extensions::project::submit_name_edit`),这里只交
    /// 给 `Workspace::blur_inputs` 清其它编辑态。
    ///
    /// `keep_native_preview_editor` 为 `true` 时保留原生预览编辑器的焦点(见
    /// `Workspace::blur_inputs`/`active_tab_is_native` 的说明):这次左键按下
    /// 若落在原生 `text_editor` 上,编辑器自己那帧会 self-focus 出光标,不能再
    /// 顺带 `blur`——2026-09-06 "点代码预览无法获得光标" 修复。
    pub fn blur_inputs(&mut self, keep_native_preview_editor: bool) {
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        ws.blur_inputs(keep_native_preview_editor);
    }

    /// 当前指定 `PanelKind`(Files/Project)预览列是否"本身是原生可编辑预览"
    /// (激活 tab 是 `CodeView`)。main.rs 在左键按下路由到该列时据此决定
    /// `blur_inputs` 要不要保留预览编辑器(-->)。
    pub fn preview_active_tab_is_native(&self, kind: PanelKind) -> bool {
        let Some(ws) = self.active_workspace() else {
            return false;
        };
        match kind {
            PanelKind::Files => ws.preview.active_tab_is_native(),
            PanelKind::Project => ws.project_preview.active_tab_is_native(),
            _ => false,
        }
    }

    /// 键盘焦点被消息(非鼠标点击)拨离预览列时调用——`main.rs` 里切终端
    /// tab/新会话落成/选中 agent 都走这条路径,不经过 `WindowEvent::
    /// MouseInput` 那次 `blur_inputs()`,原生预览编辑器不会自己让出焦点
    /// （见 `Workspace::blur_preview_editors` 的说明），得单独补一次。
    pub fn blur_preview_editors(&mut self) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.blur_preview_editors();
        }
    }

    /// 把几何状态(宽度/分割比例/窗口尺寸,**不含**左右视图选择与收起态——
    /// 那些按项目分,见 `panel_layouts`)写盘。图标切换/收起要立即持久化几何,
    /// 不能只靠 `ColumnDragEnd` 顺带存(用户可能从没拖过分隔线)。
    fn spawn_shell_layout_save(&mut self) {
        let layout = self.shell_layout.clone();
        self.handle.spawn(async move {
            if let Err(e) = layout::save(&layout) {
                tracing::warn!("外壳布局写盘失败: {e}");
            }
        });
    }

    /// 当前正显示项目的面板布局(活值)打包成 `PanelLayout`。
    fn current_panel_layout(&self) -> PanelLayout {
        PanelLayout {
            left_view: self.left_view,
            right_view: self.right_view,
            left_collapsed: self.left_collapsed,
            right_collapsed: self.right_collapsed,
            dims: self.dims,
        }
    }

    /// 把当前活值存回"当前活跃项目"在 `panel_layouts` 里的那份,并异步写盘。
    /// 切换项目**之前**调:此时 `active_project_id` 还指着老项目,于是老项目
    /// 的面板状态被原样记下,绝不会被接下来要切过去的新项目盖掉。
    fn stash_active_panel_layout(&mut self) {
        if let Some(id) = self.active_project_id {
            self.panel_layouts.insert(id, self.current_panel_layout());
            self.spawn_panel_layouts_save();
        }
    }

    /// 把 `id` 项目自己存的面板布局取出来灌进活值,让界面切到它的样子。
    /// `id` 还没存过(layout.json 升级前/第一次开)时退化成 `PanelLayout::
    /// default()`,跟旧行为一致。
    fn adopt_panel_layout(&mut self, id: i64) {
        let pl = self.panel_layouts.get(&id).copied().unwrap_or_default();
        self.left_view = pl.left_view;
        self.right_view = pl.right_view;
        // 磁盘数据可能来自 `end_rail_drag` 修复前写入的坏状态:两侧
        // active view 撞成同一个 `kind`,渲染时同一面板画两遍,复现过
        // wgpu StagingBelt "still mapped" panic(2026-08-20 崩溃排查)。
        // 落盘数据不可信,载入时兜底一次。
        if self.right_view == self.left_view {
            self.right_view = self
                .shell_layout
                .rail_layout
                .side(Side::Right)
                .iter()
                .find(|k| **k != self.left_view)
                .copied()
                .unwrap_or(self.right_view);
        }
        self.left_collapsed = pl.left_collapsed;
        self.right_collapsed = pl.right_collapsed;
        self.dims = pl.dims;
    }

    /// 把整份 `panel_layouts`(所有项目的面板布局)异步写盘。
    fn spawn_panel_layouts_save(&mut self) {
        let map = self.panel_layouts.clone();
        self.handle.spawn(async move {
            if let Err(e) = panel_layouts::save(&map) {
                tracing::warn!("面板布局写盘失败: {e}");
            }
        });
    }

    /// 外壳几何状态(视图选择/收起态/分隔线位置)变化后的统一收尾:持久化 +
    /// 按新几何重算终端网格。任何改变"终端 pane 实际拿到多少像素"的
    /// handler 都该走这里,不要只调其中一半——只存不重算,终端网格会停在
    /// 上一次窗口 resize 时的尺寸(Fix round 2 #6)。
    fn on_shell_layout_changed(&mut self) {
        // 视图选择/收起态变了:先记进当前活跃项目自己的那份(别的项目不动),
        // 再存几何、重算网格。
        self.stash_active_panel_layout();
        self.spawn_shell_layout_save();
        self.sync_terminal_grid();
    }

    /// main.rs 建窗/`WindowEvent::Resized` 时告知当前窗口逻辑尺寸:记下来
    /// (`view()`/几何公式都要用),并按新尺寸重算终端网格。
    pub fn set_window_size(&mut self, width: f32, height: f32) {
        self.window_size = (width, height);
        self.sync_terminal_grid();
    }

    /// main.rs 左键按下时告知点击落点(逻辑坐标):按 `zone_at_x` 换算落在
    /// 哪一侧面板区,命中就更新 `active_zone`(驱动 `left_zone`/
    /// `right_zone` 外边框高亮)。落在图标栏/两侧都收起等 `None` 场景保持
    /// 原有聚焦态不变——点导航图标不该清空"上一次在哪侧干活"的高亮。
    pub fn set_active_zone(&mut self, x: f32, window_width: f32) {
        if let Some(zone) = zone_at_x(x, window_width, &self.shell_state()) {
            self.active_zone = Some(zone);
        }
    }

    /// main.rs 键盘/粘贴路由用的目标终端(`Shared`/`SshPanel`)。委托给
    /// 纯函数 `keyboard_term_target`(单测用),这里只补上 `App` 私有字段的
    /// 读取。
    pub(crate) fn keyboard_term_target(&self) -> terminal::TermTarget {
        terminal::keyboard_term_target(self.left_view, self.active_zone)
    }

    /// 当前终端 IME 组字预览文本(`term_view` 渲染 + `ime_cursor_area` 算
    /// 候选窗位置共用同一份状态)。
    pub(crate) fn term_ime_preedit(&self) -> Option<&str> {
        self.term_ime_preedit.as_deref()
    }

    /// 建窗时用的初始窗口尺寸偏好:优先用上次退出前持久化的
    /// `shell_layout.window_width/height`(已经过 `sanitize_shell_layout`
    /// 夹取),`layout.json` 不存在/读不到时 `layout::load()` 本身已经退化
    /// 成 `ShellLayout::default()`,即 `byteui::theme::geometry::initial_window_size()`,这里不用再
    /// 单独处理"没存过"的分支。
    pub fn window_size_pref(&self) -> (f32, f32) {
        (
            self.shell_layout.window_width,
            self.shell_layout.window_height,
        )
    }

    /// 退出前把当前窗口尺寸并入 `shell_layout` 落盘(main.rs 在
    /// `WindowEvent::CloseRequested` 时调用,`event_loop.exit()` 之前)。
    /// 不走 `spawn_shell_layout_save` 的异步落盘——进程马上退出,spawn 的
    /// tokio 任务不保证能在进程终止前跑完;这里退化成同步写,反正只在
    /// 退出这一刻触发一次,不占渲染帧预算。
    pub fn persist_window_size_on_exit(&mut self) {
        self.shell_layout.window_width = self.window_size.0;
        self.shell_layout.window_height = self.window_size.1;
        if let Err(e) = layout::save(&self.shell_layout) {
            tracing::warn!("退出前窗口尺寸写盘失败: {e}");
        }
    }

    /// 退出前等"关 tab 时发往 daemon 的 kill/总结请求"真正跑完(main.rs 在
    /// `WindowEvent::CloseRequested` 时调用,`event_loop.exit()` 之前)。
    ///
    /// 这些请求走 `io.handle.spawn` fire-and-forget,只需一次本地 UDS
    /// 往返(通常亚毫秒级)。但如果用户关 tab 后紧接着退出,`event_loop.exit()`
    /// 后 `run_app` 返回、tokio `Runtime` 被 drop——drop 不保证在飞的
    /// spawn 任务跑完,kill 请求可能根本没发出去,daemon 侧会话仍是
    /// `alive`,下次启动就被恢复策略(`s.alive && s.project_id == ...`)
    /// 当成"还开着"重新挂回来(用户报告的验收反馈:显式关闭的 tab 不该
    /// 在重启后还魂)。
    ///
    /// 这里是继启动序列 `runtime.block_on(build_app(..))` 之后,UI 线程
    /// 上第二处、也是唯一一处允许 `block_on` 的地方——同样的理由:窗口
    /// 马上要关,不存在"占用正在渲染的 UI 线程"的问题。有限超时防止
    /// daemon 卡死/socket 挂起时把退出拖住。
    pub fn wait_for_pending_exit_tasks(&self) {
        let tasks: Vec<_> = match self.pending_exit_tasks.lock() {
            Ok(mut pending) => std::mem::take(&mut *pending),
            Err(_) => return,
        };
        if tasks.is_empty() {
            return;
        }
        self.handle
            .block_on(join_pending_exit_tasks(tasks, EXIT_TASK_BUDGET));
    }

    /// 按当前窗口尺寸+外壳状态重算终端网格并同步给所有 tab / daemon。
    ///
    /// 用于换算的是"右侧展开且显示 Agent 配对"这个假想状态,而不是当前
    /// 真实状态:终端此刻可能不可见(右侧收起 / 右视图是对话),但它的 PTY
    /// 网格仍应按"被显示时占多大"来定——否则上次退出前停在对话视图的会话,
    /// 重开 app 后会一直卡在 `DEFAULT_COLS`×`DEFAULT_ROWS`(80×24),直到用户
    /// 偶然拖一下窗口才纠正(Fix round 2 #6)。假想状态用 `ShellState`
    /// 的字段覆盖表达,不另写一套宽度公式,避免两份几何漂移。
    /// 按当前窗口尺寸+外壳状态重算终端网格并同步给所有 tab / daemon。
    ///
    /// 用于换算共享终端的是"右侧展开且显示 Agent 配对"这个假想状态,而不是
    /// 当前真实状态:终端此刻可能不可见(右侧收起 / 右视图是对话),但它的
    /// PTY 网格仍应按"被显示时占多大"来定——否则上次退出前停在对话视图的
    /// 会话,重开 app 后会一直卡在 `DEFAULT_COLS`×`DEFAULT_ROWS`(80×24),
    /// 直到用户偶然拖一下窗口才纠正(Fix round 2 #6)。假想状态用
    /// `ShellState` 的字段覆盖表达,不另写一套宽度公式,避免两份几何漂移。
    fn sync_terminal_grid(&mut self) {
        let (w, h) = self.window_size;
        let shown = terminal_grid_state(self.shell_state());
        let (pane_w, pane_h) = terminal_pane_pixel_size(w, h, &shown);
        let (cols, rows) = crate::term_view::grid_size(pane_w, pane_h);
        // SSH 面板内嵌终端挂在左面板区,几何与共享终端完全不同,由
        // `ssh_terminal_pane_pixel_size` 按左栏 `主机列表|终端` 配对换算一份
        // 独立网格——否则 SSH 终端永远套用共享终端的列数,窗口/分隔条一动
        // 宽度就跟不上宿主面板(见该函数注释)。
        let (ssh_pane_w, ssh_pane_h) = ssh_terminal_pane_pixel_size(w, h, &self.shell_state());
        let (ssh_cols, ssh_rows) = crate::term_view::grid_size(ssh_pane_w, ssh_pane_h);
        // 共享终端与 SSH 终端各自只在当前可见时才有可测量的 pane。某个 pane
        // 此刻不可换算(右侧收起 / 左面板区收起)时,它的终端可能仍挂在后台
        // (SSH tab 跨左视图常驻),这时沿用上一次跟踪的网格、发一个等值尺寸
        // 给 `PaneResized`——`pane_resized` 内部去重,套用后网格保持正确。
        let shared_ok = cols > 0 && rows > 0;
        let ssh_ok = ssh_cols > 0 && ssh_rows > 0;
        let cols = if shared_ok { cols } else { self.cols as usize };
        let rows = if shared_ok { rows } else { self.rows as usize };
        let ssh_cols = if ssh_ok {
            ssh_cols
        } else {
            self.ssh_cols as usize
        };
        let ssh_rows = if ssh_ok {
            ssh_rows
        } else {
            self.ssh_rows as usize
        };
        self.update(Message::PaneResized {
            cols: cols as u16,
            rows: rows as u16,
            ssh_cols: ssh_cols as u16,
            ssh_rows: ssh_rows as u16,
        });
    }

    /// 左面板区当前**有效**宽度:每项目持久化宽按当前窗口宽夹取(见
    /// `clamp_left_width`)。渲染侧(`left_panel_area`)必须用这个值,而不是
    /// 直接读 `self.dims.left_width`——几何侧(`left_zone_width`)走的是
    /// 同一个 `clamp_left_width`,两边只有共用同一份夹取才不会漂移。
    fn effective_left_width(&self) -> f32 {
        clamp_left_width(self.window_size.0, self.dims.left_width)
    }

    /// 终端 pane 此刻是否真的呈现在用户眼前(判定见自由函数
    /// [`terminal_visible`];逻辑只此一份,便于单测直接喂 `ShellState`)。
    fn terminal_visible(&self) -> bool {
        terminal_visible(&self.shell_state())
    }

    /// 镜像 `terminal_visible` 的方法包装:SSH 面板内嵌终端是否可见。
    fn ssh_terminal_visible(&self) -> bool {
        ssh_terminal_visible(&self.shell_state())
    }

    /// 当前外壳几何状态快照(main.rs 拖拽追踪/离屏几何计算用;`Clone`
    /// 类型,按值返回快照)。
    pub fn shell_state(&self) -> ShellState {
        ShellState {
            layout: self.shell_layout.clone(),
            dims: self.dims,
            left_view: self.left_view,
            left_collapsed: self.left_collapsed,
            right_view: self.right_view,
            right_collapsed: self.right_collapsed,
            browser_bookmarks_open: self
                .active_workspace()
                .map(|ws| ws.browser.bookmarks_open())
                .unwrap_or(false),
            maximized: self.maximized,
        }
    }

    /// 当前正在拖拽的分隔线(main.rs 拖拽追踪用,调用方为
    /// `on_window_event` 的 `CursorMoved`/`MouseInput{Released}` 分支)。
    pub fn dragging_divider(&self) -> Option<Divider> {
        self.dragging
    }

    /// 当前正在拖拽的纵向分隔线(main.rs 拖拽追踪用,调用方同
    /// `dragging_divider`)。
    pub fn dragging_row(&self) -> Option<RowDivider> {
        self.dragging_row
    }

    /// 当前正在拖拽的页签(main.rs 拖拽追踪用,调用方同 `dragging_divider`)。
    pub fn dragging_tab(&self) -> Option<TabDrag> {
        self.tab_drag
    }

    /// 当前是否正按住 `group` 组的页签(渲染侧据此把光标改成"抓取"把手)。
    pub fn dragging_group(&self, group: TabGroup) -> bool {
        self.tab_drag.is_some_and(|d| d.group == group)
    }

    /// 图标栏按钮拖拽悬停到 `side` 栏的第 `to` 个位置。同栏内是重排
    /// (立即生效,`Vec::remove`+`insert`);跨栏只记悬停目标,交给
    /// `end_rail_drag` 统一提交——避免每帧 `CursorMoved` 都触发一次
    /// `Vec` 搬移和后续的布局存盘。
    fn rail_drag_move(&mut self, side: Side, to: usize) {
        let Some(mut drag) = self.rail_drag else {
            return;
        };
        rail::rail_drag_move_into(&mut self.shell_layout.rail_layout, &mut drag, side, to);
        self.rail_drag = Some(drag);
    }

    /// 结束图标栏拖拽:松手即"锁定"——
    ///
    /// - 纯同栏重排(悬停目标期间 `rail_drag_move` 已把 `rail_layout` 实时
    ///   改到位,这里只有清空拖拽态;`source_index != origin_index` 说明真
    ///   发生了重排,据此把新顺序落盘)。
    /// - 跨栏悬停过另一栏:提交跨栏移动并落盘。
    /// - 两者皆无(按住后原地松开):只清拖拽态,不动布局、不落盘。
    ///
    /// 无论如何拖拽态都在此终止(`take()`),松手后不会再被任何残留的
    /// `RailDragMove` 驱动。
    fn end_rail_drag(&mut self) {
        let Some(drag) = self.rail_drag.take() else {
            return;
        };
        let Some((target_side, target_index)) = drag.pending_cross_side else {
            // 纯同栏重排路径:若真重排过(源下标偏离起始值),把新顺序落盘。
            if drag.source_index != drag.origin_index {
                self.on_shell_layout_changed();
            }
            return;
        };
        let Some(kind) = rail::rail_cross_apply(
            &mut self.shell_layout.rail_layout,
            drag.source_side,
            drag.source_index,
            target_side,
            target_index,
        ) else {
            // 源栏只剩这一个面板,搬走会清空——挡住,状态已经在上面
            // `take()` 时清空,这里直接返回即可,`RailLayout` 未改动。
            return;
        };
        // 面板搬走后,若源栏原本正显示的就是它,那个 active view 就悬空了
        // ——不补救的话会跟目标栏同时显示同一个 `kind`,两侧渲染出重复的
        // 面板实例(重复的 widget id/图片纹理请求),曾在拖回来回几次后
        // 稳定复现 wgpu `StagingBelt` "still mapped" panic(见
        // 2026-08-20 崩溃排查)。源栏移除后必然还剩至少一个面板(`rail_
        // cross_apply` 不允许栏清空),落到它现在的第一个面板上。
        let source_view = match drag.source_side {
            Side::Left => &mut self.left_view,
            Side::Right => &mut self.right_view,
        };
        if *source_view == kind {
            *source_view = *self
                .shell_layout
                .rail_layout
                .side(drag.source_side)
                .first()
                .expect("rail_cross_apply 保证源栏搬空前至少剩一个面板");
        }

        // 被移动面板成为目标栏新 active,跟随"移动后在按钮所在一侧打开
        // 面板"的要求。跨栏移动结束后统一走 `on_shell_layout_changed`
        // (存盘 + 重算网格)。
        match target_side {
            Side::Left => {
                self.left_view = kind;
                self.left_collapsed = false;
            }
            Side::Right => {
                self.right_view = kind;
                self.right_collapsed = false;
            }
        }
        self.on_shell_layout_changed();
    }

    /// 当前是否正按住某个图标栏按钮(渲染侧据此把光标改成"抓取"把手,
    /// 同 `dragging_group` 对 `TabDrag` 的用法)。
    pub fn dragging_rail(&self) -> bool {
        self.rail_drag.is_some()
    }

    /// 当前是否正在拖拽文件树内的某一行(main.rs 全局左键松开靠它判断该不
    /// 该发 `TreeDragEnd` 收尾——同 `dragging_rail`/`dragging_tab` 的用法)。
    pub(crate) fn dragging_tree_item(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.files.is_dragging_tree_item())
    }

    /// 树内拖拽是否已确认(`Dragging` 阶段,见 `TreeDragPhase` 文档)——
    /// 供顶层 `view()` 判断要不要叠加 `files::tree_drag_ghost` 幽灵胶囊
    /// (同 `rail_drag_confirmed()` 驱动 `rail_drag_ghost` 的用法)。
    pub(crate) fn tree_drag_confirmed(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.files.tree_drag_confirmed())
    }

    /// 每次 `CursorMoved`(main.rs 调用)都判断一次:当前树内拖拽若还在
    /// `Pending`、且光标位移越过 [`tree_drag_past_threshold`]、按住时长
    /// 越过 [`tree_drag_held_long_enough`],就推进到 `Dragging`——这个
    /// 转换只发生一次(`confirm_tree_drag` 对已是 `Dragging` 的状态是
    /// no-op),之后 `TreeDragEnd` 的 `confirmed` 直接读这个结果,不必在
    /// 松开时重新算(见 `TreeDragPhase`/`Message::TreeDragRelease` 文档)。
    /// 返回是否真的发生了这次转换,供调用方决定要不要 `request_redraw`
    /// (转换会让行开始挂 `on_move`/显示抓取光标/幽灵胶囊,不重绘看不出来)。
    /// `left_mouse_down` 是硬性前提(见 `main.rs` 里 `left_mouse_down` 字段
    /// 文档):不管存好的按下坐标/时间戳算出来是否越过阈值,左键这一刻没
    /// 有物理按住就绝不确认——顺手把任何残留的 `tree_drag` 自愈清空(正常
    /// 路径下 `TreeDragRelease` 早该清过一次;还留着只可能是那次收尾因为
    /// 某种原因没触发,不是一次合法的、仍在进行的拖拽)。
    pub(crate) fn maybe_confirm_tree_drag(&mut self, left_mouse_down: bool) -> bool {
        if !left_mouse_down {
            if let Some(ws) = self.active_workspace_mut() {
                ws.files.cancel_tree_drag();
            }
            return false;
        }
        let should_confirm = self.active_workspace().is_some_and(|ws| {
            ws.files.tree_drag_is_pending()
                && ws
                    .files
                    .tree_drag_press_pos()
                    .is_some_and(|p| tree_drag_past_threshold(p, self.last_cursor))
                && ws
                    .files
                    .tree_drag_armed_at()
                    .is_some_and(|t| tree_drag_held_long_enough(t.elapsed()))
        });
        if should_confirm && let Some(ws) = self.active_workspace_mut() {
            ws.files.confirm_tree_drag();
        }
        should_confirm
    }

    /// 当前正被拖拽的面板种类(`None` = 未在拖拽)——视图层(`icon_rail`
    /// 源图标变淡 / `rail_drag_ghost` 幽灵图标取图标)据此判断"这是不是
    /// 我"。薄包装 `dragged_panel_kind` 自由函数(同 `rail_drag_move`
    /// 包装 `rail_drag_move_into` 的既有手法),不重复实现逻辑。
    pub fn dragged_panel_kind(&self) -> Option<PanelKind> {
        rail::dragged_panel_kind(&self.shell_layout.rail_layout, self.rail_drag)
    }

    /// `dragging_rail()` 为真(已按下武装)且光标已相对按下点位移超过
    /// [`RAIL_DRAG_VISUAL_THRESHOLD_PX`]——只有这时才算"确认是一次拖拽,
    /// 不是单击",视图层(`icon_rail` 源图标变淡/抓手光标、`rail_drag_
    /// ghost` 幽灵图标)一律看这个而不是 `dragging_rail()`,避免快速单击
    /// 也闪一下拖拽视觉(见 `RailDrag::press_pos` 文档)。`RailDragEnd` 的
    /// 收尾逻辑(`main.rs`/`end_rail_drag`)不受影响,继续按 `dragging_rail()`
    /// 判断,因为松手清理拖拽态这件事无论有没有越过阈值都要做。
    pub fn rail_drag_confirmed(&self) -> bool {
        self.rail_drag
            .is_some_and(|d| rail::rail_drag_past_threshold(d, self.last_cursor))
    }

    /// 结束页签拖拽:清掉拖拽态,若是项目页签组还把新顺序写盘。松开左键的
    /// 两条路径都会走到这里——winit 的 `MouseInput{Released}`(`TabDragEnd`)
    /// 与子 webview 上 JS 上报的 `mouseup`(`WebViewMouseUp`)——保证拖拽态
    /// 在任何情况下都不会残留。
    fn end_tab_drag(&mut self) {
        if let Some(drag) = self.tab_drag.take()
            && drag.group == TabGroup::Project
        {
            // 项目页签顺序变了——写盘(同打开项目那条持久化路径)。
            self.persist_open_projects();
        }
    }

    /// 项目树右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn context_menu_open(&self) -> bool {
        self.files.context_menu_is_some()
            || self.preview_tab_menu.is_some()
            || self.project_preview_tab_menu.is_some()
            || self.project_link_menu.is_some()
            || self.text_input_menu.is_some()
            || self.database_source_menu.is_some()
    }

    /// 输入框右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn text_input_menu_open(&self) -> bool {
        self.text_input_menu.is_some()
    }

    /// main.rs 读取"本帧若产生右上角输入框右键菜单动作,要作用到的输入
    /// 焦点",清空后返回。`TextInputMenuOpen` 时写入,供复制/粘贴作用于
    /// 被右键的输入。
    pub fn take_pending_text_input_focus(&mut self) -> Option<iced_widget::core::widget::Id> {
        self.pending_text_input_focus.take()
    }

    /// main.rs 读取"输入框右键菜单当前要作用的输入 id"。菜单展开时
    /// `TextInputMenuOpen` 已写入 `target`,main.rs 在合成复制/粘贴键盘事件前
    /// 用它把焦点再补一次到被右键的输入(保证作用于它而不是别的)。
    pub fn text_input_menu_target_id(&self) -> Option<iced_widget::core::widget::Id> {
        self.text_input_menu.as_ref().map(|m| m.target.id.clone())
    }

    /// 预览 tab 右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn preview_tab_context_menu_open(&self) -> bool {
        self.preview_tab_menu.is_some()
    }

    /// Project 面板右配对预览 tab 右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn project_preview_tab_context_menu_open(&self) -> bool {
        self.project_preview_tab_menu.is_some()
    }

    /// Project 面板链接行右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn project_link_context_menu_open(&self) -> bool {
        self.project_link_menu.is_some()
    }

    /// 数据库面板数据源树 header 行右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn database_source_context_menu_open(&self) -> bool {
        self.database_source_menu.is_some()
    }

    /// 打开数据库面板数据源树 header 行的右键菜单(测试连接/编辑/删除/
    /// 刷新)。与文件树右键菜单互斥(坐标复用 `files.last_right_click`)。
    fn database_source_context_menu(&mut self, source_id: String) {
        let (x, y) = self.files.last_right_click();
        self.files.close_context_menu();
        self.database_source_menu = Some(DatabaseSourceMenu { x, y, source_id });
    }

    /// 打开 Project 面板「项目文档 / Agent 记忆」链接行的删除右键菜单。与
    /// 文件树右键菜单互斥(坐标复用 `files.last_right_click`)。
    fn project_link_context_menu(&mut self, target: project::links::LinkTarget, index: usize) {
        let (x, y) = self.files.last_right_click();
        self.files.close_context_menu();
        self.project_link_menu = Some(ProjectLinkMenu {
            x,
            y,
            target,
            index,
        });
        // 右击即选中该行:从对应链接列表取下标项路径,标记到
        // `project_panel.selected_link`(参考文件树 `ContextMenuOpen` 同时选中)。
        if let Some(path) = self
            .active_workspace()
            .and_then(|ws| ws.project_panel.link_path_at(target, index))
        {
            self.with_focused_project(|ws, _io| {
                ws.project_panel.set_selected_link(path);
            });
        }
    }

    /// 打开分类树节点的右键菜单。坐标复用 `files.last_right_click()`
    /// (同 `project_link_context_menu` 的既有接线方式)。
    fn todo_category_context_menu(&mut self, id: Option<i64>) {
        let (x, y) = self.files.last_right_click();
        self.files.close_context_menu();
        self.category_context_menu = Some(CategoryContextMenu { x, y, id });
    }

    /// 分类选择器是否打开(main.rs Esc 键路由用)。
    pub fn category_picker_open(&self) -> bool {
        self.category_picker.is_some()
    }

    /// Todo 分类树节点右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn category_context_menu_open(&self) -> bool {
        self.category_context_menu.is_some()
    }

    /// 打开分类选择器浮层,挂到给定目标(任务挂分类 / 分类 reparent)。
    fn todo_category_picker_open(&mut self, target: CategoryPickerTarget) {
        let (x, y) = self.last_cursor;
        self.category_picker = Some(CategoryPicker { x, y, target });
    }

    /// Agent 选择菜单是否打开(main.rs Esc 键路由用)。
    pub fn agent_picker_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.agent_picker_open)
            .unwrap_or(false)
    }

    /// 顶栏新增项目菜单是否打开(main.rs Esc 键路由用)。
    pub fn project_add_menu_open(&self) -> bool {
        self.project_add_menu_open
    }

    /// Todo 派发选择层是否打开(给 main.rs 的 Esc 关闭用)。
    pub fn todo_dispatch_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.todo.dispatch_popup_open())
            .unwrap_or(false)
    }

    /// Todo 日历日期选择器是否打开(给 main.rs 的 Esc 关闭用,同
    /// `todo_dispatch_open` 的既有模式)。
    pub fn todo_calendar_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.todo.calendar_popup_open())
            .unwrap_or(false)
    }

    /// Todo 状态下拉选择层是否打开(给 main.rs 的 Esc 关闭用,同
    /// `todo_dispatch_open` 的既有模式)。
    pub fn todo_status_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.todo.status_popup_open())
            .unwrap_or(false)
    }

    /// Todo **搜索框状态筛选**浮层是否打开(给 main.rs 的 Esc 关闭用,同
    /// `todo_dispatch_open` 的既有模式)。
    pub fn todo_status_filter_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.todo.status_filter_popup_open())
            .unwrap_or(false)
    }

    /// 预览编辑弹层是否打开(main.rs 键盘路由用)。打开期间键盘必须走
    /// 弹层的文本编辑器,不能落进终端 PTY——弹层挂在左侧预览面板,不影响
    /// `terminal_visible()` 的判断条件(右侧展开与否),不加这道闸门的话,
    /// 默认布局(右侧终端可见)下编辑弹层里敲的每个字符,包括回车,都会
    /// 同时写进背后那个终端/agent 会话。
    pub fn edit_session_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.edit_session.is_some())
            .unwrap_or(false)
    }

    /// 当前激活预览 tab 是否走原生渲染。main.rs 键盘路由用,同 `edit_session_open`
    /// 那道闸门的道理——但原生预览不是模态弹层,还要求键盘焦点确实在预览列
    /// (`current_focus == FocusIntent::Preview`),否则用户正在打字给终端时,
    /// 只因为预览列背景里开着一个原生 tab 就会把按键错误地拦下来。
    pub fn active_preview_tab_has_native_editor(&self, kind: PanelKind) -> bool {
        self.active_workspace()
            .map(|ws| ws.active_preview_tab_has_native_editor(kind))
            .unwrap_or(false)
    }

    /// `kind` 预览面板的 Find 条当前是否显示。main.rs Esc/⌘ 键盘路由据此决定在
    /// 原生预览闸门里先吃哪些键(Esc 关条/⌘G 步进只在有条时可行动)。
    pub fn preview_find_bar_open(&self, kind: PanelKind) -> bool {
        self.active_workspace()
            .map(|ws| ws.preview_find_bar_open(kind))
            .unwrap_or(false)
    }

    /// 转发 `text_editor::Action` 到当前聚焦项目的编辑弹层。官方 `text_editor`
    /// 的剪贴板读写由 iced 运行时经 `Widget::update` 拿到的 `Clipboard` 直接
    /// 处理,不再需要像 vendored `iced-code-editor` 那样手动拆 `Task` 桥接
    /// (那套桥接已随依赖一起删除,见 main.rs 历史)。
    pub fn preview_edit_event(&mut self, action: iced_widget::text_editor::Action) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.preview_edit_event(action);
        }
    }

    /// 转发到聚焦项目里某个原生预览 tab 的 editor,语义同 `preview_edit_event`
    /// 但按 `tab_id` 定位而不是"当前编辑弹层"。
    pub fn preview_tab_editor_event(
        &mut self,
        tab_id: usize,
        action: iced_widget::text_editor::Action,
    ) {
        let io = self.shell_io();
        if let Some(ws) = self.active_workspace_mut() {
            ws.preview_tab_editor_event(tab_id, action);
            ws.spawn_preview_context_push(&io);
        }
    }

    /// Project 面板右配对预览 tab 的 `text_editor::Action` 转发,语义同
    /// `preview_tab_editor_event`,作用于 `ws.project_preview`。
    pub fn project_preview_tab_editor_event(
        &mut self,
        tab_id: usize,
        action: iced_widget::text_editor::Action,
    ) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.project_preview_tab_editor_event(tab_id, action);
        }
    }

    /// 取走"双击顶栏空白处"待处理标记(取走即清零)。main.rs 在派发完
    /// 消息后轮询这个方法,命中就调用 `window.set_maximized(!window.
    /// is_maximized())`——`App` 自己不持有 `Window` 句柄,做不到这一步。
    pub fn take_pending_zoom_toggle(&mut self) -> bool {
        std::mem::take(&mut self.pending_zoom_toggle)
    }

    /// 全局 UI 缩放改了之后,main.rs 轮询这个标记把新 scale 应用到所有
    /// 已存在的预览/浏览器 webview(新的 webview 由 `sync_webview_pool`
    /// 在创建时按当前 scale 初始化,不依赖本标记)。
    pub fn take_pending_preview_zoom(&mut self) -> bool {
        std::mem::take(&mut self.pending_preview_zoom)
    }

    /// 终端光标的窗口逻辑坐标 `(x, y_底, 行高)`,给 main.rs 在没有任何原生
    /// `text_input` 要 IME 时(`InputMethod::Disabled`,即终端聚焦——终端
    /// 不是 iced 控件,没有这套机制)设候选窗位置用。原生 `text_input`
    /// 聚焦要 IME 的情况(地址栏/意见框/搜索框/……)现在统一由 main.rs 直接
    /// 读 `UserInterface::update()` 返回的 `InputMethod::Enabled { cursor,
    /// .. }` 处理,不再靠这里手写特判(2026-08-21 修复:`input_method` 字段
    /// 此前被丢弃,原生控件的候选窗一律钉在这份终端光标算法给出的位置,
    /// 组字预览文字也压根没地方画出来)。单元格尺寸由 pane 像素 ÷ 网格
    /// 推出,不依赖字号常量。
    pub fn ime_cursor_area(&self, window_w: f32, window_h: f32) -> (f32, f32, f32) {
        let state = self.shell_state();
        let (pane_w, pane_h) = terminal_pane_pixel_size(window_w, window_h, &state);
        let cell_w = pane_w / self.cols.max(1) as f32;
        let line_h = pane_h / self.rows.max(1) as f32;
        let right_w = right_zone_width(window_w, &state);
        // `PanelKind::Agent` 面板未镜像时,实际渲染顺序是 `[terminal,
        // divider, list]`(内容在前,见 `app.rs` 的 `PanelKind::Agent`
        // 分支 `else` 臂)——跟 `pair_columns` 的内建默认("mirrored=false
        // → list 先")相反,必须传 `!mirrored`,否则算出来的内容列起点会
        // 多算上一整个 list 宽度(2026-08-22 实测调试日志确认:候选窗
        // x 坐标比实际光标多偏了正好一个 `list_w` 的量,同 `webview_
        // geometry.rs` 的 `Conversations`/`Web` 两处同款手法)。
        let mirrored =
            state.layout.rail_layout.side_of(PanelKind::Agent) != PanelKind::Agent.default_side();
        let cols = pair_columns(
            pair_content_width(right_w),
            state.dims.agent_split,
            !mirrored,
        );
        let m = theme::region::right_zone().margin;
        let x0 = window_w - byteui::theme::geometry::icon_rail_width() - right_w
            + cols.content_x
            + 8.0
            + m.left;
        // 终端网格上方 chrome:顶栏 44 + 上 padding 8 + tab 栏 30 + spacing 4(header 已去,P1L #4)
        let y0 = byteui::theme::geometry::top_bar_height() + 8.0 + 30.0 + 4.0;
        let (col, row) = self
            .active_workspace()
            .and_then(|ws| ws.tabs.get(ws.active))
            .map(|t| {
                // 光标不可信(全屏重绘型 TUI 未发 `?25h` 显示光标,实测
                // CodeBuddy CLI——见 `term_view.rs` 画方块光标那处同款
                // `cursor_visible()` 判断的文档)或正在回看历史
                // (`display_offset() > 0`)时,`model.cursor()` 只是一堆
                // 重绘期间移动/清行序列扫过后留下的陈旧坐标,跟视觉上
                // 光标实际所在毫无关系——不同 CLI 的终端更新习惯不同,
                // 表现就是"候选窗偏移量因 agent 而异"(2026-08-21 用户
                // 实测反馈)。退回"面板底部一行、列 0"这个粗略但不离谱的
                // 默认位置,好过让候选窗跳到跟视觉毫不相关的地方。
                if t.model.cursor_visible() && t.model.display_offset() == 0 {
                    t.model.cursor()
                } else {
                    (0usize, self.rows.max(1) as usize - 1)
                }
            })
            .unwrap_or((0, 0));
        // 组字预览期间 PTY 收不到字节,`model.cursor()` 原地不动——候选窗
        // 要跟着预览文字的末尾走(与 `term_view` 画预览的落点算法一致),
        // 否则用户敲得越多,候选窗越是钉在组字开始前的旧光标位置不跟手。
        let preedit_cols: usize = self
            .term_ime_preedit
            .as_deref()
            .map(|s| {
                s.chars()
                    .map(|c| {
                        unicode_width::UnicodeWidthChar::width(c)
                            .unwrap_or(1)
                            .max(1)
                    })
                    .sum()
            })
            .unwrap_or(0);
        let x = x0 + (col + preedit_cols) as f32 * cell_w;
        let y = y0 + (row as f32 + 1.0) * line_h; // 光标格底部,候选窗落其下方
        (x, y, line_h)
    }

    /// 当前应存在的"文件/项目预览"webview 清单(main.rs 差集同步用),
    /// 每条自带按其所在侧算好的矩形。左右两侧各自独立判断——`Files` 在
    /// 左栏、`Project` 在右栏可以同时非空(见 spec"webview 面板的镜像
    /// bounds(2026-08-19 Stage 4a 审阅后修订)"一节)。不在文件视图时
    /// 该侧整体不产出;进首页时两侧都不产出(原因见旧版注释:首页时
    /// 预览区根本不在屏上)。
    pub fn preview_desired(
        &self,
        window_width: f32,
        window_height: f32,
    ) -> Vec<(WebviewSpec, (f32, f32, f32, f32))> {
        if self.current_page == AppPage::Home {
            return Vec::new();
        }
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        // `search_modal` 跟 `edit_modal` 同款满窗 SCRIM+卡片形制(见
        // `extensions/search.rs::search_modal` 注释),同样要在打开时隐藏
        // webview,否则 webview 会盖住遮罩和弹窗卡片。
        let app_modal_open = ws.edit_session.is_some() || ws.search.is_open();
        let mut out = Vec::new();
        for side in [Side::Left, Side::Right] {
            let kind = match side {
                Side::Left => self.left_view,
                Side::Right => self.right_view,
            };
            let (specs, id_offset): (Vec<WebviewSpec>, usize) = match kind {
                PanelKind::Files => (ws.preview.desired_webviews(), 0),
                PanelKind::Project => (
                    ws.project_preview.desired_webviews(),
                    PROJECT_PREVIEW_ID_OFFSET,
                ),
                PanelKind::Conversations => (
                    crate::workspace::review_webview_spec(ws.review.as_ref()),
                    CONVERSATION_REVIEW_ID_OFFSET,
                ),
                _ => continue,
            };
            let bounds = webview_geometry::preview_content_bounds_for(
                side,
                window_width,
                window_height,
                &self.shell_state(),
            );
            out.extend(specs.into_iter().map(|mut s| {
                s.id += id_offset;
                // 编辑弹层或搜索弹窗开着时,应用级模态盖住了预览区,原生
                // wry 子视图不听 iced 绘制顺序摆布,必须显式 visible=false
                // 才能真正藏起来。
                if app_modal_open {
                    s.visible = false;
                }
                (s, bounds)
            }));
        }
        out
    }

    /// 浏览器域的 webview 清单,语义同 `preview_desired`,查独立的
    /// `Workspace::browser`。首页时矩形留空(main.rs 用 `home_browser_bounds`
    /// 单独覆盖,见调用处),工作区内按 `Web` 当前所在侧现算矩形。
    pub fn browser_desired(
        &self,
        window_width: f32,
        window_height: f32,
    ) -> Vec<(WebviewSpec, (f32, f32, f32, f32))> {
        // 首页右栏恒为全局浏览器(`home_browser`),与 `left_view` 无关——
        // 进首页就让它成为浏览器 webview 池的唯一来源,否则默认 URL 的 tab
        // 建了却永远等不到 webview(见 `sync_webview_pool`)。
        if self.current_page == AppPage::Home {
            return self
                .home_browser
                .desired_webviews()
                .into_iter()
                .map(|s| (s, (0.0, 0.0, 0.0, 0.0)))
                .collect();
        }
        let side = if self.left_view == PanelKind::Web {
            Side::Left
        } else if self.right_view == PanelKind::Web {
            Side::Right
        } else {
            return Vec::new();
        };
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        let bounds = webview_geometry::preview_content_bounds_for(
            side,
            window_width,
            window_height,
            &self.shell_state(),
        );
        ws.browser
            .desired_webviews()
            .into_iter()
            .map(|s| (s, bounds))
            .collect()
    }

    /// 保证 `git_log` 状态跟得上"现在应该看哪个项目"——`git_log: State`
    /// 是 `App` 级字段,不是每个项目各自一份(不像 `Workspace.files`),
    /// 所以面板打开时(`PanelSelect`)和切项目页签时(`ProjectTabSwitch`)
    /// 都得调这个方法对齐一次,否则 Git Log 面板开着的状态下切页签,提交图
    /// 会停在上一个项目不动,而同一面板里的 worktree 速览条(`ws.files
    /// .worktrees()` 是按项目取的)却已经跳到新项目——两者对不上。缓存已经是当前项目的
    /// 路径就不动(避免每次切页签都重算一遍),路径不一致就重建,没有项目
    /// 就清空。只在 `left_view == PanelKind::GitLog` 时调用才有意义。
    fn sync_git_log_to_active_project(&mut self) {
        let path = self
            .active_workspace()
            .and_then(|ws| ws.active_project_path());
        match path {
            Some(p) if self.git_log.cache_repo_path() != Some(p.as_path()) => {
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::GitLog(m));
                };
                git_log::request_refresh(
                    &mut self.git_log,
                    p,
                    git_log::DEFAULT_MAX_COMMITS,
                    &handle,
                    emit,
                );
            }
            None => {
                self.git_log = git_log::State::default();
            }
            _ => {}
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::TermInput(target, bytes) => self.term_input(target, bytes),
            Message::TermImePreedit(target, text) => {
                if target == self.keyboard_term_target() {
                    self.term_ime_preedit = text;
                }
            }
            Message::TermOutput(project_id, tab_id, bytes) => {
                self.term_output(project_id, tab_id, bytes)
            }
            Message::SessionExited(project_id, tab_id) => {
                self.with_project(project_id, |ws, _io| {
                    if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                        tab.alive = false;
                        // 本地标记行，非会话真实输出；应答无处可写，丢弃。
                        let _ = tab.model.feed(&exited_marker());
                    }
                });
            }
            Message::AgentStateChanged(project_id, tab_id, agent, state, transcript_path) => {
                self.agent_state_changed(project_id, tab_id, agent, state, transcript_path)
            }
            Message::AgentCardRefreshed(
                project_id,
                tab_id,
                llm_model,
                mode,
                activity,
                workspace,
            ) => {
                self.with_project(project_id, |ws, _io| {
                    crate::workspace::apply_agent_card_refresh(
                        &mut ws.tabs,
                        tab_id,
                        llm_model,
                        mode,
                        activity,
                        workspace,
                    );
                });
            }
            Message::ReviewLoaded(project_id, source, append, result) => {
                self.with_project(project_id, |ws, _io| {
                    if result.is_ok() {
                        ws.review_nonce = ws.review_nonce.wrapping_add(1);
                    }
                    let nonce = ws.review_nonce;
                    if let Some(rv) = &mut ws.review
                        && rv.source == source
                    {
                        match result {
                            Ok(entries) => {
                                // 快照写入必须放在 `rv.source == source` 判断
                                // 通过之后——这是它跟旧实现(main.rs 里的裸
                                // `static`,过期/乱序结果也会无条件覆盖)的
                                // 关键区别,过期加载结果到这里已经被
                                // 上面的守卫挡在外面,不会再污染快照。
                                merge_review_entries(&mut rv.entries, entries, append);
                                let snapshot = ReviewSnapshot {
                                    entries: &rv.entries,
                                    agent_label: rv.agent.label(),
                                    summary_title: rv.summary_title.clone(),
                                    summary_text: rv.summary_text.clone(),
                                    summary_time: rv.summary_time.clone(),
                                };
                                let json = serde_json::to_string(&snapshot).unwrap_or_default();
                                *ws.review_snapshot.lock().expect("review snapshot 锁") =
                                    Some(json);
                                rv.nonce = nonce;
                                rv.error = None;
                            }
                            Err(e) => rv.error = Some(e),
                        }
                    }
                });
            }
            Message::ConversationSessionsRefreshed(project_id, result) => {
                self.with_project(project_id, move |ws, _io| match result {
                    Ok(rows) => ws.conversation_sessions = Some(rows),
                    Err(e) => {
                        tracing::warn!(error = %e, "对话会话列表查询失败");
                        ws.conversation_sessions = Some(Vec::new());
                    }
                });
            }
            Message::ProjectDeleteDone(errors) => {
                if !errors.is_empty() {
                    self.daemon_error = Some(format!("删除项目未完全成功: {}", errors.join("; ")));
                }
            }
            Message::Usage(msg @ usage::Message::Loaded(project_id, ..)) => {
                self.with_project(project_id, move |ws, _io| {
                    usage::update(&mut ws.usage, msg);
                });
            }
            Message::Usage(usage::Message::ToggleListCollapse) => {
                self.toggle_panel_list_collapse(PanelKind::Usage);
            }
            Message::Usage(usage::Message::Hover(id, h)) => self.set_hover(id, h),
            Message::Usage(msg) => {
                self.with_focused_project(|ws, _io| {
                    usage::update(&mut ws.usage, msg);
                });
            }
            Message::ConversationSessionOpen(conversation_id, agent) => {
                self.conversation_session_open(conversation_id, agent);
            }
            Message::ConversationDetailLoadMore(conversation_id, after_turn_index) => {
                self.with_focused_project(|ws, io| {
                    ws.spawn_review_load_conversation(
                        io,
                        conversation_id,
                        after_turn_index,
                        CONVERSATION_DETAIL_PAGE_SIZE,
                        true,
                    );
                });
            }
            Message::ConversationListMore => {
                self.with_focused_project(|ws, _io| {
                    ws.conversation_pages += 1;
                });
            }
            Message::ConversationSearchInput(s) => {
                self.with_focused_project(|ws, _io| {
                    ws.conversation_search_draft = s;
                });
            }
            Message::ConversationSearchSubmit => {
                self.with_focused_project(|ws, _io| {
                    ws.conversation_search = ws.conversation_search_draft.clone();
                    ws.conversation_pages = 0;
                });
            }
            Message::ConversationAgentFilterSelect(agent) => {
                self.with_focused_project(|ws, _io| {
                    ws.conversation_agent_filter = agent;
                    ws.conversation_pages = 0;
                    ws.conversation_agent_picker_open = false;
                });
            }
            Message::ConversationAgentPickerOpen => {
                self.with_focused_project(|ws, _io| {
                    ws.conversation_agent_picker_open = true;
                });
            }
            Message::ConversationAgentPickerClose => {
                self.with_focused_project(|ws, _io| {
                    ws.conversation_agent_picker_open = false;
                });
            }

            Message::SelectTab(idx) => self.select_tab(idx),
            Message::SelectTabNoDrag(idx) => self.select_tab_no_drag(idx),
            Message::CloseTab(idx) => {
                self.with_focused_project(|ws, io| {
                    ws.close_tab(io, idx);
                    ws.ensure_project_terminal(io);
                });
            }
            Message::AgentPickerToggle => {
                self.with_focused_project(|ws, _io| {
                    ws.agent_picker_open = !ws.agent_picker_open;
                });
            }
            Message::AgentPickerClose => {
                self.with_focused_project(|ws, _io| {
                    ws.agent_picker_open = false;
                });
            }
            Message::AgentPickerSelect(agent) => {
                self.with_focused_project(|ws, io| {
                    ws.agent_picker_open = false;
                    ws.spawn_new_tab(io, agent, None);
                });
            }
            Message::ProjectAddMenuToggle => {
                self.project_add_menu_open = !self.project_add_menu_open;
                if self.project_add_menu_open {
                    // 点"＋"时的光标逻辑坐标,作为菜单弹出锚点——同
                    // `todo::set_calendar_anchor`/`set_dispatch_anchor` 手法。
                    self.project_add_menu_anchor = self.last_cursor;
                }
            }
            Message::ProjectAddMenuClose => {
                self.project_add_menu_open = false;
            }
            Message::Todo(todo::Message::AssignAgent(idx, agent)) => {
                self.todo_assign_agent(idx, agent)
            }
            Message::Todo(todo::Message::DetailOpen(idx)) => self.todo_detail_open(idx),
            Message::Todo(todo::Message::DetailReplySubmit) => self.todo_detail_process(),
            // 数据库连接测试的异步结果带显式 `project_id`——用户可能在等待
            // 期间切走了项目页签,必须按自带 id 路由,不能用当前聚焦项目
            // (同 `TabAttached`/`ProjectSlotLoaded` 那批异步消息的约定,见设计
            // 文档"结果经 `emit` 回传"一节)。特化分支必须排在通配
            // `Message::Database(msg)` **之前**,否则永远匹配不到。
            Message::Database(database::Message::TestConnectionResult(
                project_id,
                source_id,
                result,
            )) => self.database_test_connection_result(project_id, source_id, result),
            // schema 树两个异步结果同 `TestConnectionResult` 口径:自带 project_id,
            // 按自带 id 路由,不能用当前聚焦项目。特化分支必须排在通配
            // `Message::Database(msg)` 之前,否则永远匹配不到。
            Message::Database(database::Message::TablesLoaded(project_id, source_id, result)) => {
                self.database_tables_loaded(project_id, source_id, result)
            }
            Message::Database(database::Message::ColumnsLoaded {
                project_id,
                source_id,
                schema,
                table,
                result,
            }) => self.database_columns_loaded(project_id, source_id, schema, table, result),
            Message::Database(database::Message::BrowseResult(
                project_id,
                tab_id,
                run_seq,
                result,
            )) => self.database_browse_result(project_id, tab_id, run_seq, result),
            Message::Database(database::Message::QueryResult(
                project_id,
                tab_id,
                run_seq,
                result,
            )) => self.database_query_result(project_id, tab_id, run_seq, result),
            Message::Database(database::Message::SourceContextMenu(source_id)) => {
                self.database_source_context_menu(source_id);
            }
            Message::Database(database::Message::TabHover(target, idx, hovered)) => {
                // 内容窗格 tab 本体/关闭按钮的悬停,转发成 `HoverId`(同
                // `ToolbarHover` 的口径)。
                let id = match target {
                    database::DatabaseTabHoverTarget::Title => HoverId::DatabaseTabItem(idx),
                    database::DatabaseTabHoverTarget::Close => HoverId::DatabaseTabClose(idx),
                };
                self.set_hover(id, hovered);
            }
            Message::Database(database::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Database(database::Message::ToggleListCollapse) => {
                self.toggle_panel_list_collapse(PanelKind::Database);
            }
            Message::Database(database::Message::TabOverflowToggle) => {
                let last_cursor = self.last_cursor;
                self.with_focused_project(|ws, _io| {
                    ws.database.content_mut().toggle_tab_overflow(last_cursor);
                });
            }
            Message::Database(database::Message::TabOverflowDismiss) => {
                self.with_focused_project(|ws, _io| {
                    ws.database.content_mut().dismiss_tab_overflow();
                });
            }
            Message::Database(database::Message::Hover(id, h)) => self.set_hover(id, h),
            Message::Database(msg) => self.database_message(msg),
            Message::Todo(msg) => match msg {
                todo::Message::Hover(id, h) => self.set_hover(id, h),
                todo::Message::TextInputMenuOpen(target) => {
                    self.update(Message::TextInputMenuOpen(target));
                }
                todo::Message::ToggleListCollapse => {
                    self.toggle_panel_list_collapse(PanelKind::Todo);
                }
                todo::Message::CategoryContextMenuOpen(id) => {
                    self.todo_category_context_menu(id);
                }
                // 以下几种分类动作都是从右键菜单里点出来的:先关掉菜单本
                // 身(浮层 if-else 链里 `category_context_menu` 分支排在
                // `category_picker` 之前,不关会导致"移动到..."开了选择器
                // 却永远被菜单盖住),镜像 `files.rs::RenameStart` 落盘动作
                // 时 `app_state.context_menu = None;` 的既有口径。
                todo::Message::CategoryReparentPickerOpen(id) => {
                    self.category_context_menu = None;
                    self.todo_category_picker_open(CategoryPickerTarget::Category(id));
                }
                todo::Message::CategoryNewChild(_)
                | todo::Message::CategoryNewSibling(_)
                | todo::Message::CategoryDelete(_)
                | todo::Message::CategoryRenameStart(_)
                | todo::Message::CategoryMoveSibling(_, _) => {
                    self.category_context_menu = None;
                    self.todo_message(msg);
                }
                todo::Message::CategoryPickerOpenForTodo(todo_id) => {
                    self.todo_category_picker_open(CategoryPickerTarget::Todo(todo_id));
                }
                other => self.todo_message(other),
            },
            Message::TodoDetailLoaded(idx, turns) => {
                self.with_focused_project(move |ws, _io| {
                    if ws.todo.detail_open_idx() == Some(idx) {
                        ws.todo.replace_detail_turns(turns);
                    }
                });
            }
            // 文件树右键"搜索"弹窗:`SearchResults` 带 `project_id`,异步结果
            // 按所属项目路由(用户可能已切走);其余交互投当前聚焦项目。
            Message::Search(search::Message::SearchResults(project_id, result)) => {
                self.search_results(project_id, result)
            }
            Message::Search(search::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Search(msg) => {
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                self.with_focused_project(move |ws, _io| {
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Search(m));
                    };
                    search::update(&mut ws.search, msg, project_id, &handle, emit);
                });
            }
            Message::TabAttached(project_id, tab_id, info, snapshot) => {
                self.with_project(project_id, move |ws, io| {
                    ws.on_tab_attached(io.cols, io.rows, tab_id, info, snapshot)
                });
            }
            Message::PaneResized {
                cols,
                rows,
                ssh_cols,
                ssh_rows,
            } => self.pane_resized(cols, rows, ssh_cols, ssh_rows),
            Message::ColumnDragStart(divider) => {
                self.dragging = Some(divider);
            }
            Message::ColumnDrag {
                window_width,
                logical_x,
            } => {
                if let Some(divider) = self.dragging {
                    let state = self.shell_state();
                    self.dims = apply_column_drag(state, divider, window_width, logical_x);
                }
            }
            Message::ColumnDragEnd => {
                self.dragging = None;
                self.on_shell_layout_changed();
            }
            Message::RowDragStart(divider) => {
                self.dragging_row = Some(divider);
            }
            Message::RowDrag {
                window_height,
                logical_y,
            } => {
                if let Some(divider) = self.dragging_row {
                    match divider {
                        RowDivider::GitLogFileDiffSplit => {
                            let state = self.shell_state();
                            self.dims = apply_row_drag(state, divider, window_height, logical_y);
                        }
                        // 新增任务框高度:基线 = 框底 = 左面板区底 =
                        // `window_height - footbar_height`(顶栏在 `base`
                        // 之上,不参与);高度 = 基线 - 光标 y,向上拉变高。
                        // 上限再夹一道,避免列表区被压没(留约 140px)。
                        RowDivider::TodoAddGrow => {
                            let baseline =
                                window_height - byteui::theme::geometry::footbar_height();
                            let max_h =
                                (baseline - byteui::theme::geometry::top_bar_height() - 140.0)
                                    .max(todo::ADD_INPUT_MIN_HEIGHT);
                            let h = (baseline - logical_y).clamp(todo::ADD_INPUT_MIN_HEIGHT, max_h);
                            if let Some(ws) = self.active_workspace_mut() {
                                ws.todo.set_add_input_height(h);
                            }
                        }
                    }
                }
            }
            Message::RowDragEnd => {
                self.dragging_row = None;
                self.on_shell_layout_changed();
            }
            Message::TabDragMove { group, index } => {
                self.tab_drag_move(group, index);
            }
            Message::TabDragEnd => {
                self.end_tab_drag();
            }
            Message::RailDragMove { side, index } => {
                self.rail_drag_move(side, index);
            }
            Message::RailDragEnd => {
                self.end_rail_drag();
            }
            Message::TodoDragEnd => {
                self.todo_message(todo::Message::DragEnd);
            }
            Message::PanelSelect(v) => self.panel_select(v),
            Message::ToggleFileTreeCollapse => self.toggle_files_tree_collapse(),
            Message::TogglePanelListCollapse(kind) => self.toggle_panel_list_collapse(kind),
            Message::TextInputMenuOpen(target) => {
                // 与其它右键菜单互斥——关掉别的,只留本菜单(同时避免互相顶)。
                self.files.close_context_menu();
                self.preview_tab_menu = None;
                self.project_preview_tab_menu = None;
                self.project_link_menu = None;
                let (x, y) = self.files.last_right_click();
                self.text_input_menu = Some(TextInputMenu {
                    x,
                    y,
                    target: target.clone(),
                });
                // 右键不聚焦 iced 输入框(只有左键会),菜单的复制/粘贴需要通过
                // `interface.operate` 把焦点移到目标输入,否则合成回的 ⌘+c/v
                // 事件作用不到它。记录待聚焦 id,本帧后由 `apply_pending_focus`
                // 应用(main.rs)。
                self.pending_text_input_focus = Some(target.id);
            }
            Message::TextInputMenuClose => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuCut => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuCopy => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuPaste => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuSelectAll => {
                self.text_input_menu = None;
            }
            Message::Hover(id, h) => {
                self.set_hover(id, h);
            }
            Message::MaximizeClose => {
                self.maximized = None;
                self.sync_terminal_grid();
            }
            Message::TopBarDoubleClick => {
                self.pending_zoom_toggle = true;
            }
            Message::TopBarHome => self.top_bar_home(),
            Message::HomeRecentsLoaded(files, convs) => {
                self.home_recent_files = files;
                self.home_recent_conversations = convs;
                self.home_recents_loaded = true;
            }
            Message::HomeLeftIconSelect(v) => {
                self.home_left_view = v;
            }
            Message::HomeMoreProjects => self.home_project_pages += 1,
            Message::HomeProjectSearchInput(s) => self.home_project_search_draft = s,
            Message::HomeProjectSearchSubmit => self.commit_home_project_search(),
            Message::HomeRightIconSelect(v) => {
                self.home_right_view = v;
            }
            Message::HomeBrowser(msg) => {
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::HomeBrowser(m));
                };
                browser::update(&mut self.home_browser, msg, None, &client, &handle, emit);
            }
            Message::BrowserTitle(id, title) => {
                let msg = browser::Message::TitleLoaded(id, title);
                if self.is_home() {
                    // 首页全局浏览器:webview 属于 `home_browser`,直接落地。
                    browser::update(
                        &mut self.home_browser,
                        msg,
                        None,
                        &self.client,
                        &self.handle,
                        |_| {},
                    );
                } else {
                    // 工作区浏览器:webview id 落在当前聚焦工作区的 `ws.browser`。
                    self.with_focused_project(|ws, io| {
                        browser::update(
                            &mut ws.browser,
                            msg,
                            ws.project.as_ref().map(|p| p.id),
                            &io.client,
                            &io.handle,
                            |_| {},
                        );
                    });
                }
            }
            Message::BrowserNavigated(id, url) => {
                let msg = browser::Message::Loaded(id, url);
                if self.is_home() {
                    browser::update(
                        &mut self.home_browser,
                        msg,
                        None,
                        &self.client,
                        &self.handle,
                        |_| {},
                    );
                } else {
                    self.with_focused_project(|ws, io| {
                        browser::update(
                            &mut ws.browser,
                            msg,
                            ws.project.as_ref().map(|p| p.id),
                            &io.client,
                            &io.handle,
                            |_| {},
                        );
                    });
                }
            }
            Message::BrowserNewWindow(url) => {
                let msg = browser::Message::OpenUrl(url);
                if self.is_home() {
                    browser::update(
                        &mut self.home_browser,
                        msg,
                        None,
                        &self.client,
                        &self.handle,
                        |_| {},
                    );
                } else {
                    self.with_focused_project(|ws, io| {
                        browser::update(
                            &mut ws.browser,
                            msg,
                            ws.project.as_ref().map(|p| p.id),
                            &io.client,
                            &io.handle,
                            |_| {},
                        );
                    });
                }
            }
            Message::Noop => {}
            Message::DaemonError(message) => self.daemon_error = Some(message),
            Message::TermScroll(target, delta) => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        terminal::TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        terminal::TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.scroll_display(delta);
                    }
                });
            }
            Message::TermTabOverflowToggle => {
                let last_cursor = self.last_cursor;
                self.with_focused_project(|ws, _io| {
                    ws.term_tab_overflow_anchor = if ws.term_tab_overflow_anchor.is_some() {
                        None
                    } else {
                        Some(last_cursor)
                    };
                });
            }
            Message::TermTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.term_tab_overflow_anchor = None;
                });
            }
            Message::PreviewTabOverflowToggle => {
                let last_cursor = self.last_cursor;
                self.with_focused_project(|ws, _io| {
                    ws.preview_tab_overflow_anchor = if ws.preview_tab_overflow_anchor.is_some() {
                        None
                    } else {
                        Some(last_cursor)
                    };
                });
            }
            Message::PreviewTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.preview_tab_overflow_anchor = None;
                });
            }
            Message::TermSelStart {
                target,
                col,
                row,
                right,
            } => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        terminal::TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        terminal::TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.selection_start(col, row, right);
                    }
                });
            }
            Message::TermSelUpdate {
                target,
                col,
                row,
                right,
            } => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        terminal::TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        terminal::TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.selection_update(col, row, right);
                    }
                });
            }
            Message::TermPaste(target, text) => self.term_paste(target, text),
            Message::PreviewOpenPath(path) => self.preview_open_path(path),
            Message::PreviewSelectTab(idx) => self.preview_select_tab(idx),
            Message::PreviewCloseTab(idx) => {
                self.preview_tab_menu = None;
                self.with_focused_project(|ws, io| {
                    ws.preview.close(idx);
                    // 关 tab 后位置全变，旧 first 可能越界——归零防御（P1L T5）。
                    ws.preview_tab_first = 0;
                    ws.spawn_preview_state_save(io);
                    ws.spawn_preview_context_push(io);
                });
            }
            Message::PreviewEditOpen(idx) => {
                self.preview_tab_menu = None;
                self.with_focused_project(move |ws, _io| ws.preview_edit_open(idx));
            }
            Message::PreviewReload(idx) => {
                self.preview_tab_menu = None;
                self.with_focused_project(move |ws, _io| ws.preview_reload(idx));
            }
            Message::EditorEvent(_action) => {
                // main.rs 的 dispatch 直接调 `App::preview_edit_event`,不经过
                // 这里的 `App::update`——到达此处说明未走 dispatch 拦截,忽略。
            }
            Message::EditorUndo => {
                // 键盘(⌘Z)经 `app.update` 进来时走这里真正撤销;main.rs 另有
                // `App::preview_edit_undo` 直呼口(绕过 `update`),两路都只操作
                // 聚焦项目编辑弹层的 editor。
                self.with_focused_project(|ws, _io| ws.preview_edit_undo());
            }
            Message::EditorRedo => {
                // 同 `EditorUndo`(⌘⇧Z)。
                self.with_focused_project(|ws, _io| ws.preview_edit_redo());
            }
            Message::PreviewEditorEvent(_tab_id, _action) => {
                // 同 `EditorEvent`,main.rs 直接调 `App::preview_tab_editor_event`。
            }
            Message::PreviewEditSave => {
                self.with_focused_project(|ws, _io| ws.preview_edit_save());
            }
            Message::PreviewSaveActive(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_pane_save_active(kind));
            }
            Message::PreviewTabInsertTab(kind) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_pane_active_editor_event(
                        kind,
                        iced_widget::text_editor::Action::Edit(
                            iced_widget::text_editor::Edit::Insert('\t'),
                        ),
                    );
                });
            }
            Message::PreviewFindOpen(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_open(kind));
            }
            Message::PreviewFindOpenWithReplace(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_open_with_replace(kind));
            }
            Message::PreviewFindReplaceToggle(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_toggle_replace(kind));
            }
            Message::PreviewFindClose(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_close(kind));
            }
            Message::PreviewFindText(kind, query) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_type(kind, query));
            }
            Message::PreviewFindGo(kind, next) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_go(kind, next));
            }
            Message::PreviewFindCase(kind, sensitive) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_case(kind, sensitive));
            }
            Message::PreviewFindReplacement(kind, repl) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_find_set_replacement(kind, repl);
                });
            }
            Message::PreviewFindReplaceCurrent(kind) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_find_replace_current(kind);
                });
            }
            Message::PreviewFindReplaceAll(kind) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_find_replace_all(kind);
                });
            }
            Message::PreviewEditCloseRequest => {
                self.with_focused_project(|ws, _io| ws.preview_edit_close_request());
            }
            Message::PreviewEditConfirmDiscard => {
                self.with_focused_project(|ws, _io| ws.preview_edit_confirm_discard());
            }
            Message::PreviewEditConfirmCancel => {
                self.with_focused_project(|ws, _io| ws.preview_edit_confirm_cancel());
            }
            Message::PreviewTabContextMenu { idx, editable } => {
                let (x, y) = self.files.last_right_click();
                // 与文件树右键菜单互斥——避免两者同时挂着,关掉一个把另一个
                // 意外顶出来。
                self.files.close_context_menu();
                self.preview_tab_menu = Some(PreviewTabMenu {
                    x,
                    y,
                    idx,
                    editable,
                });
            }
            Message::PreviewTabContextMenuClose => {
                self.preview_tab_menu = None;
            }
            Message::ProjectPreviewOpenPath(path) => self.project_preview_open_path(path),
            Message::ProjectPreviewSelectTab(idx) => self.project_preview_select_tab(idx),
            Message::ProjectPreviewCloseTab(idx) => {
                self.project_preview_tab_menu = None;
                self.with_focused_project(|ws, _io| {
                    ws.project_preview.close(idx);
                    ws.project_preview_tab_first = 0;
                });
            }
            Message::ProjectPreviewTabOverflowToggle => {
                let last_cursor = self.last_cursor;
                self.with_focused_project(|ws, _io| {
                    ws.project_preview_tab_overflow_anchor =
                        if ws.project_preview_tab_overflow_anchor.is_some() {
                            None
                        } else {
                            Some(last_cursor)
                        };
                });
            }
            Message::ProjectPreviewTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.project_preview_tab_overflow_anchor = None;
                });
            }
            Message::ProjectPreviewEditorEvent(_tab_id, _event) => {
                // 与 `PreviewEditorEvent` 同口径:到达 `App::update` 说明未走
                // main.rs 的 Task 桥接器,直接忽略。
            }
            Message::ProjectPreviewEditOpen(idx) => {
                self.project_preview_tab_menu = None;
                self.with_focused_project(move |ws, _io| ws.project_preview_edit_open(idx));
            }
            Message::ProjectPreviewReload(idx) => {
                self.project_preview_tab_menu = None;
                self.with_focused_project(move |ws, _io| ws.project_preview_reload(idx));
            }
            Message::ProjectPreviewTabContextMenu { idx, editable } => {
                let (x, y) = self.files.last_right_click();
                self.project_preview_tab_menu = Some(PreviewTabMenu {
                    x,
                    y,
                    idx,
                    editable,
                });
            }
            Message::ProjectPreviewTabContextMenuClose => {
                self.project_preview_tab_menu = None;
            }
            Message::Browser(browser::Message::BookmarksLoaded(pid, bookmarks)) => {
                self.browser_bookmarks_loaded(pid, bookmarks)
            }
            Message::Browser(browser::Message::BookmarksMutated(pid, res)) => {
                self.browser_bookmarks_mutated(pid, res)
            }
            Message::Browser(browser::Message::DragHover(idx)) => {
                // 浏览器 tab 脱的换位:光标扫过 `idx` 页签 → 走共同换位逻辑。
                self.tab_drag_move(TabGroup::Browser, idx);
            }
            Message::Browser(browser::Message::ColumnDragStart) => {
                self.update(Message::ColumnDragStart(Divider::BrowserBookmarksSplit));
            }
            Message::Browser(browser::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Browser(msg) => self.browser_message(msg),
            Message::ProjectSelect(id) => self.project_select(id),
            Message::ProjectTabPickFolder => {
                // 副作用在 main.rs(rfd 文件夹选择);从新增项目菜单触发时顺带
                // 关掉菜单,同 `project_select` 的处理口径。
                self.project_add_menu_open = false;
            }
            // rfd 弹窗在 main.rs 里同步处理,选中后转成 project::Message::LinkAdd
            // 再回送到这里;这条顶层消息本身不需要 App::update 处理任何东西。
            Message::ProjectLinkPick(_) => {}
            Message::ProjectLinkContextMenuClose => {
                self.project_link_menu = None;
            }
            Message::CategoryContextMenuClose => {
                self.category_context_menu = None;
            }
            Message::CategoryPickerClose => {
                self.category_picker = None;
            }
            Message::CategoryPickerSelect(chosen) => {
                let Some(picker) = self.category_picker.take() else {
                    return;
                };
                match picker.target {
                    CategoryPickerTarget::Todo(todo_id) => {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        handle.spawn(async move {
                            let res = client
                                .set_todo_category(todo_id, chosen)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string());
                            let _ = proxy
                                .send_event(Message::Todo(todo::Message::CategoryMutated(res)));
                        });
                    }
                    CategoryPickerTarget::Category(category_id) => {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        handle.spawn(async move {
                            let res = client
                                .reparent_category(category_id, chosen)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string());
                            let _ = proxy
                                .send_event(Message::Todo(todo::Message::CategoryMutated(res)));
                        });
                    }
                }
            }
            Message::DatabaseSourceContextMenuClose => {
                self.database_source_menu = None;
            }
            Message::ProjectTabOpen(path) => {
                let client = self.client.clone();
                let proxy = self.proxy.clone();
                let path_s = path.to_string_lossy().into_owned();
                self.handle.spawn(async move {
                    let opened = client.open_project(&path_s).await.ok().flatten();
                    let recent = client.list_projects().await.unwrap_or_default();
                    let _ = proxy.send_event(Message::ProjectTabOpened(opened, recent));
                });
            }
            Message::ProjectTabOpened(project, recent) => self.project_tab_opened(project, recent),
            Message::ProjectTabSwitch(id) => self.project_tab_switch(id),
            Message::ProjectTabClose(id) => self.project_tab_close(id),
            Message::ProjectSlotLoaded(id, payload) => self.project_slot_loaded(id, payload),
            Message::ProjectFsChanged(project_id, changes) => {
                self.project_fs_changed(project_id, changes)
            }
            Message::GitLog(git_log::Message::ColumnDragStart) => {
                // Git Log 三栏布局里左右分割线开始拖拽——扩展发不了 app 级
                // 拖拽消息,由内核代发。
                self.update(Message::ColumnDragStart(Divider::GitLogSplit));
            }
            Message::GitLog(git_log::Message::RowDragStart) => {
                self.update(Message::RowDragStart(RowDivider::GitLogFileDiffSplit));
            }
            Message::GitLog(git_log::Message::BranchPickerOpen) => {
                // 先把"展开"这个状态位落地(纯状态机部分仍走 update,不跳过),
                // 首次展开且还没缓存过分支列表时,顺带异步查一次本地分支。
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::GitLog(m));
                };
                let needs_fetch = self.git_log.branches_is_empty();
                git_log::update(
                    &mut self.git_log,
                    git_log::Message::BranchPickerOpen,
                    &handle,
                    emit.clone(),
                );
                if needs_fetch
                    && let Some(repo_path) = self
                        .active_workspace()
                        .and_then(|ws| ws.active_project_path())
                {
                    self.handle.spawn(async move {
                        let repo_path2 = repo_path.clone();
                        let (branches, dirty) = tokio::task::spawn_blocking(move || {
                            let branches =
                                crate::delivery::local_branches(&repo_path2).unwrap_or_default();
                            let dirty = crate::delivery::is_dirty(&repo_path2);
                            (branches, dirty)
                        })
                        .await
                        .unwrap_or_default();
                        emit(git_log::Message::BranchesLoaded(repo_path, branches, dirty));
                    });
                }
            }
            Message::GitLog(git_log::Message::BranchSwitch(name)) => {
                let Some(repo_path) = self
                    .active_workspace()
                    .and_then(|ws| ws.active_project_path())
                else {
                    return;
                };
                self.git_log.set_branch_switch_pending(true);
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let repo_path2 = repo_path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        crate::delivery::checkout_branch(&repo_path2, &name)
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let _ = proxy
                        .send_event(Message::GitLog(git_log::Message::BranchSwitchDone(result)));
                });
            }
            Message::GitLog(git_log::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::GitLog(msg) => {
                if let git_log::Message::Hover(id, h) = msg {
                    self.set_hover(id, h);
                    return;
                }
                // 分支切换成功后(checkout 改了 HEAD/工作区),commit 列表要重拉。
                let is_branch_switch_success =
                    matches!(&msg, git_log::Message::BranchSwitchDone(Ok(())));
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::GitLog(m));
                };
                if let Some(next) = git_log::update(&mut self.git_log, msg, &handle, emit.clone()) {
                    self.update(Message::GitLog(next));
                }
                if is_branch_switch_success
                    && let Some(repo_path) = self
                        .active_workspace()
                        .and_then(|ws| ws.active_project_path())
                {
                    let max_count = self.git_log.cache_max_count();
                    git_log::request_refresh(
                        &mut self.git_log,
                        repo_path,
                        max_count,
                        &handle,
                        emit,
                    );
                }
            }
            Message::Files(files::Message::CopyPath(path, kind)) => {
                let _ = (path, kind); // main.rs 拦截处理写剪贴板,这里维持现状空分支
            }
            Message::Files(files::Message::OpenSearch(path, is_dir)) => {
                // 右键菜单"搜索":跨 `files::Message` 边界,由内核把它映射成
                // `search::Message::SearchOpen`。先关右键菜单(否则搜索弹窗
                // dismiss 一关,旧菜单又冒回来),作用域由 `is_dir` 决定——目录
                // 按目录递归搜,文件只搜单文件。
                self.files.close_context_menu();
                let scope = if is_dir {
                    search::Scope::Dir(path)
                } else {
                    search::Scope::File(path)
                };
                self.update(Message::Search(search::Message::SearchOpen(scope)));
            }
            Message::Files(
                msg @ (files::Message::StatusesRefreshed(project_id, ..)
                | files::Message::PasteDone(project_id, ..)
                | files::Message::OpDone { project_id, .. }
                | files::Message::GitInfoLoaded(project_id, ..)
                | files::Message::BranchSwitchDone(project_id, ..)
                | files::Message::GitInitDone(project_id, ..)
                | files::Message::FileDropDone(project_id, ..)),
            ) => self.files_project_message(project_id, msg),

            Message::Files(files::Message::ToolbarHover(target, hovered)) => {
                // 文件树工具行 icon 按钮的 hover:本面板不挂 App 的 hover 动画
                // 表,把进入/离开转发成 `HoverId` 由内核统一驱动动画进度。
                let id = match target {
                    files::FilesToolbarTarget::SearchSubmit => HoverId::FilesSearchSubmit,
                    files::FilesToolbarTarget::Dotfiles => HoverId::FilesDotfiles,
                    files::FilesToolbarTarget::BranchSwitch => HoverId::FilesBranchSwitch,
                };
                self.set_hover(id, hovered);
            }
            Message::Files(files::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            // 树内行被按下:只武装拖拽(供 main.rs 全局松开左键时收尾),
            // **不**立即执行任何点击语义(既不展开/折叠目录,也不打开文件)。
            // 两者都推迟到真正松开、且确认这其实只是一次单击(未越过拖拽
            // 确认阈值)时才在 `TreeDragEnd` 里补做——展开目录会让下方行
            // 布局位移,若按下就立即展开,静止不动的光标可能被 iced 判定成
            // 树内行被按下:只武装 `Pending`(见 `TreeDragPhase` 文档)——
            // `Pending` 期间完全没有任何反应,不挂 `on_move`、不展开目录、
            // 不打开文件。真正推进到 `Dragging`(越过距离+时长两道阈值)由
            // `maybe_confirm_tree_drag` 在每次 `CursorMoved` 时判断(main.rs
            // 调用),不在这里做。
            Message::Files(files::Message::TreeRowPress { path, is_dir }) => {
                let press_pos = self.last_cursor;
                if let Some(ws) = self.active_workspace_mut() {
                    ws.files
                        .arm_tree_drag(path, is_dir, press_pos, std::time::Instant::now());
                }
            }
            // 树内拖拽松开左键:main.rs 发这条。`confirmed` 就是"这场拖拽有
            // 没有走到 `Dragging` 阶段"——由 `maybe_confirm_tree_drag` 在
            // 越过阈值那一刻就已经推进过一次,这里直接读结果,不重新算
            // 距离/时长(2026-09 用户实测反馈带诊断日志实锤过纯距离阈值挡
            // 不住 trackpad 快速点按的真实位移,才改成阈值判断只在
            // `Pending → Dragging` 转换时做一次、结果落进状态机里的这个
            // 设计,见 `TreeDragPhase` 文档)。
            Message::Files(files::Message::TreeDragRelease) => {
                let confirmed = self
                    .active_workspace()
                    .is_some_and(|ws| ws.files.tree_drag_confirmed());
                self.update(Message::Files(files::Message::TreeDragEnd(confirmed)));
            }
            // 树行双击:目录复用 `Message::Toggle` 那条本地消息直接切换展开
            // 态(同点箭头效果),文件跨到 `PreviewOpenPath`——该面板本身
            // 不认识这条消息,见 `files::Message::TreeRowDoubleClick` 文档。
            Message::Files(files::Message::TreeRowDoubleClick { path, is_dir }) => {
                if is_dir {
                    self.update(Message::Files(files::Message::Toggle(path)));
                } else {
                    self.update(Message::PreviewOpenPath(path));
                }
            }
            Message::Files(msg) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Files(m));
                };
                let app_files = &mut self.files;
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                files::update(&mut ws.files, app_files, msg, project_id, &handle, emit);
            }
            Message::Project(
                msg @ (project::Message::GitRefreshed(project_id, ..)
                | project::Message::NameRenamed(project_id, ..)
                | project::Message::DiskUsageLoaded(project_id, ..)
                | project::Message::ScaffoldStepStarted(project_id, ..)
                | project::Message::ScaffoldStepFinished(project_id, ..)
                | project::Message::TranscriptBackfillStarted(project_id, ..)
                | project::Message::TranscriptBackfillFinished(project_id, ..)
                | project::Message::SummaryBackfillProgress(project_id, ..)),
            ) => {
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let current_name = project.name.clone();
                let repo_path = std::path::PathBuf::from(&project.path);
                // `NameRenamed(Ok(updated))` 要把顶栏项目页签等读的 `ws.project`
                // 缓存一并更新——这是这个面板第一次出现需要内核介入(而不是纯
                // 委托给 `project::update`)的消息。
                if let project::Message::NameRenamed(_, Ok(updated)) = &msg {
                    ws.project = Some(updated.clone());
                }
                // 补总结进度追到 Done 时,弹窗外面缓存的 `conversation_sessions`
                // (`spawn_conversations_refresh` 唯一写入点)不会自动感知
                // `session_summaries` 表的新增行——不重新拉一次,对话列表会一直
                // 显示"未总结"直到用户重开项目 tab 或触发别的回合结束刷新。
                let refresh_conversations = matches!(
                    &msg,
                    project::Message::SummaryBackfillProgress(_, completed, total)
                        if completed >= total
                );
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Project(m));
                };
                project::update(
                    &mut ws.project_panel,
                    msg,
                    project_id,
                    &current_name,
                    &repo_path,
                    &client,
                    &handle,
                    emit,
                );
                if refresh_conversations {
                    self.with_project(project_id, |ws, io| {
                        ws.spawn_conversations_refresh(io);
                    });
                }
            }
            Message::Project(project::Message::OpenLink(path)) => {
                // 项目链接打开的文件进 Project 面板右配对的预览(`ws.project_preview`),
                // 不冲进 Files 预览——两条预览各自独立,互相不打扰。
                self.update(Message::ProjectPreviewOpenPath(path));
            }
            Message::Project(project::Message::Pick(target)) => {
                // 文件/目录选择器依赖 macOS 主线程原生能力(rfd/NSOpenPanel 模态,
                // 见 main.rs `pick_file_or_dir`),必须由 main.rs 的 winit 事件循环
                // 里 `dispatch` 拦截同步执行。这里只用代理把这条消息回灌回事件循环
                // ——不能 `self.update(Message::ProjectLinkPick(..))` 直调:那是同步
                // 递归,只会命中 `App::update` 里那格 no-op,绝不会触发文件选择弹窗。
                let _ = self.proxy.send_event(Message::ProjectLinkPick(target));
            }
            Message::Project(project::Message::LinkContextMenu { target, index }) => {
                self.project_link_context_menu(target, index);
            }
            Message::Project(project::Message::LinkRemove { target, index }) => {
                // 删除来自行内右键菜单:落 `LinkRemove` 时把菜单浮层一并收起,
                // 然后委托 `project::update` 真正执行删除(含越界校验与保存失败
                // 回滚,见 `project.rs` 的 `Message::LinkRemove`)。不能
                // `self.update(同一条 LinkRemove)` 直调——那会命中本分支自身
                // 再次匹配 `LinkRemove`,无限递归爆栈。
                self.project_link_menu = None;
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let current_name = project.name.clone();
                let repo_path = std::path::PathBuf::from(&project.path);
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Project(m));
                };
                project::update(
                    &mut ws.project_panel,
                    project::Message::LinkRemove { target, index },
                    project_id,
                    &current_name,
                    &repo_path,
                    &client,
                    &handle,
                    emit,
                );
            }
            Message::Project(project::Message::DeleteProjectConfirm) => {
                self.project_delete_confirm();
            }
            Message::Project(project::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Project(msg) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let current_name = project.name.clone();
                let repo_path = std::path::PathBuf::from(&project.path);
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Project(m));
                };
                project::update(
                    &mut ws.project_panel,
                    msg,
                    project_id,
                    &current_name,
                    &repo_path,
                    &client,
                    &handle,
                    emit,
                );
            }
            Message::Ssh(ssh::Message::TestConnectionResult(project_id, host_id, result)) => {
                self.ssh_test_connection_result(project_id, host_id, result)
            }
            Message::Ssh(ssh::Message::UnknownKeyDetected(
                project_id,
                host_id,
                fingerprint,
                key_bytes,
            )) => self.ssh_unknown_key_detected(project_id, host_id, fingerprint, key_bytes),
            Message::Ssh(ssh::Message::KeyChanged(project_id, host_id, fingerprint)) => {
                self.ssh_key_changed(project_id, host_id, fingerprint)
            }
            // 点"终端"按钮:与既有 `TestConnection`/其它同步交互消息不同,
            // 这个消息不走 `ssh::update`(它要新建一个 tab,需要 `&mut
            // Workspace` 整体,`ssh::update` 只拿得到 `&mut ws.ssh`)——
            // 拦截在通配 `Message::Ssh(msg)` 之前,直接调 `Workspace::
            // spawn_ssh_tab`。
            Message::Ssh(ssh::Message::OpenSshTab(host_id, ssh::SshTabKind::Terminal)) => {
                // 新 SSH 终端从一开始就用 SSH 面板自己的网格(不是共享终端的
                // 列数)——在闭包里再借 `self` 会与 `with_focused_project` 的
                // `&mut self` 冲突,先取到局变量。
                let ssh_cols = self.ssh_cols;
                let ssh_rows = self.ssh_rows;
                self.with_focused_project(|ws, io| {
                    // 已经开着这台主机的终端 tab 就直接切过去,不重新握手
                    // 连一遍(阶段 3 SFTP 决定"每个 tab 独立新建连接",但
                    // 终端 tab 本来就是"一台主机一条常驻连接",重复点
                    // "终端"图标应该是切换焦点而不是叠加新连接)。
                    let already_open = ws
                        .ssh_tabs
                        .iter()
                        .any(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()));
                    if already_open {
                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Terminal);
                    } else {
                        ws.ssh.record_reopen_after_trust(host_id.clone());
                        ws.spawn_ssh_tab(io, host_id, ssh_cols, ssh_rows);
                    }
                });
            }
            // Sftp 阶段 3:真实打开一个 SFTP tab(独立连接 + 命令通道)。
            Message::Ssh(ssh::Message::OpenSshTab(host_id, ssh::SshTabKind::Sftp)) => {
                self.with_focused_project(|ws, io| {
                    if ws.sftp_tabs.contains_key(&host_id) {
                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Sftp);
                    } else {
                        ws.spawn_sftp_tab(io, host_id.clone());
                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Sftp);
                    }
                });
            }
            Message::Ssh(ssh::Message::CloseSshTab(host_id, kind)) => {
                self.with_focused_project(|ws, io| match kind {
                    ssh::SshTabKind::Terminal => ws.close_ssh_tab(io, &host_id, kind),
                    ssh::SshTabKind::Sftp => {
                        ws.sftp_tabs.remove(&host_id);
                        if ws.ssh_active.as_ref().map(|(h, k)| (h.as_str(), *k))
                            == Some((host_id.as_str(), ssh::SshTabKind::Sftp))
                        {
                            ws.ssh_active = None; // 简化处理:关掉 SFTP tab 后不自动
                            // 切到其它 tab,和终端 tab 关闭后的
                            // "切到剩下第一个"逻辑不强行统一,
                            // 因为 ssh_tabs/sftp_tabs 是两个不同
                            // 集合,统一切换逻辑收益不大,YAGNI。
                        }
                    }
                });
            }
            Message::Ssh(ssh::Message::SelectSshTab(host_id, kind)) => {
                self.with_focused_project(|ws, _io| {
                    ws.select_ssh_tab(host_id, kind);
                    let mut widths: Vec<f32> = vec![tab_display_width("空白")];
                    widths.extend(ws.ssh_tabs.iter().map(|t| {
                        tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name))
                    }));
                    widths.extend(ws.sftp_tabs.keys().map(|host_id| {
                        tab_display_width(
                            &ws.ssh
                                .hosts()
                                .iter()
                                .find(|h| &h.id == host_id)
                                .map(|h| h.name.clone())
                                .unwrap_or_else(|| host_id.clone()),
                        )
                    }));
                    let target = match &ws.ssh_active {
                        None => 0,
                        Some((active_host, ssh::SshTabKind::Terminal)) => ws
                            .ssh_tabs
                            .iter()
                            .position(|t| {
                                t.info.id.strip_prefix("ssh:") == Some(active_host.as_str())
                            })
                            .map(|i| i + 1)
                            .unwrap_or(0),
                        Some((active_host, ssh::SshTabKind::Sftp)) => ws
                            .sftp_tabs
                            .keys()
                            .position(|h| h == active_host)
                            .map(|i| i + 1 + ws.ssh_tabs.len())
                            .unwrap_or(0),
                    };
                    ws.ssh_tab_first = tab_widget::tab_window_reveal(
                        &widths,
                        4.0,
                        byteui::theme::geometry::tab_bar_avail_px(),
                        ws.ssh_tab_first,
                        target,
                    );
                    ws.ssh_tab_overflow_anchor = None;
                });
            }
            // 点固定的"空白"占位 tab:它不对应 `ssh_tabs`/`sftp_tabs` 里
            // 任何一条记录,选中态就是 `ssh_active == None`。
            Message::Ssh(ssh::Message::SelectBlankTab) => {
                self.with_focused_project(|ws, _io| {
                    ws.ssh_active = None;
                    let mut widths: Vec<f32> = vec![tab_display_width("空白")];
                    widths.extend(ws.ssh_tabs.iter().map(|t| {
                        tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name))
                    }));
                    widths.extend(ws.sftp_tabs.keys().map(|host_id| {
                        tab_display_width(
                            &ws.ssh
                                .hosts()
                                .iter()
                                .find(|h| &h.id == host_id)
                                .map(|h| h.name.clone())
                                .unwrap_or_else(|| host_id.clone()),
                        )
                    }));
                    ws.ssh_tab_first = tab_widget::tab_window_reveal(
                        &widths,
                        4.0,
                        byteui::theme::geometry::tab_bar_avail_px(),
                        ws.ssh_tab_first,
                        0,
                    );
                    ws.ssh_tab_overflow_anchor = None;
                });
            }
            Message::Ssh(ssh::Message::TabOverflowToggle) => {
                let last_cursor = self.last_cursor;
                self.with_focused_project(|ws, _io| {
                    ws.ssh_tab_overflow_anchor = if ws.ssh_tab_overflow_anchor.is_some() {
                        None
                    } else {
                        Some(last_cursor)
                    };
                });
            }
            Message::Ssh(ssh::Message::TabOverflowDismiss) => {
                self.with_focused_project(|ws, _io| {
                    ws.ssh_tab_overflow_anchor = None;
                });
            }
            // SFTP tab 内部交互:按 host_id 路由到 `sftp::route`,真正的
            // 处理逻辑在那边(sftp::Message 有 7+ 个变体,内容又都操作
            // `ws.sftp_tabs`,摊平会让这里的大 match 更难读)。
            Message::Ssh(ssh::Message::Sftp(msg)) => {
                self.with_focused_project(|ws, io| {
                    ssh::sftp::route(ws, io, msg);
                });
            }
            // 终端连接失败:先做内核层面的清理(pending/ssh_out_pending
            // 两处暂存——这次连接没能走到 `TabAttached`,不清理会一直占着
            // 这两个 map 的位置),再转给 `ssh::update` 落卡片状态(同
            // `TestConnectionResult` 的路由口径,带显式 project_id,套用
            // 一模一样的 `with_project` 外壳)。
            Message::Ssh(ssh::Message::TerminalConnectFailed(project_id, host_id, tab_id, err)) => {
                self.ssh_terminal_connect_failed(project_id, host_id, tab_id, err)
            }
            Message::Ssh(ssh::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Ssh(msg) => {
                if let ssh::Message::Hover(id, h) = msg {
                    self.set_hover(id, h);
                    return;
                }
                self.with_focused_project(|ws, io| {
                    let Some(project) = ws.project.as_ref() else {
                        return;
                    };
                    let project_id = project.id;
                    let repo_path = PathBuf::from(&project.path);
                    let handle = io.handle.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Ssh(m));
                    };
                    ssh::update(&mut ws.ssh, msg, project_id, &repo_path, &handle, emit);
                });
            }
            Message::Footbar(msg) => {
                // App 级 + 纯展示,不带 project_id,不需要
                // `with_project`/`with_focused_project`,直接更新。
                footbar::update(&mut self.footbar, msg);
            }
            Message::ZoomIn => {
                byteui::theme::icon_size::zoom_by(UI_ZOOM_STEP);
                byteui::theme::icon_size::persist_scale(&crate::theme::ui_scale_path());
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            Message::ZoomOut => {
                byteui::theme::icon_size::zoom_by(1.0 / UI_ZOOM_STEP);
                byteui::theme::icon_size::persist_scale(&crate::theme::ui_scale_path());
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            Message::ZoomReset => {
                byteui::theme::icon_size::reset_scale(&crate::theme::ui_scale_path());
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            // WebViewFocused 只在 main.rs 的 dispatch 里设 pending_focus,
            // App::update 无需处理。
            Message::WebViewFocused => {}
            // 鼠标在子 webview 上松开(见 `WebViewMouseUp` 文档):一并结束页签
            // 拖拽,避免"松开还能继续拖"。
            Message::WebViewMouseUp => self.end_tab_drag(),
        }
    }

    fn project_select(&mut self, id: i64) {
        // 从新增项目菜单点选时顺带关掉菜单(菜单本来就该在选中后消失);
        // 从其它入口(首页最近项目卡片)调用时这里恒为 false,no-op。
        self.project_add_menu_open = false;
        // 切项目不再通知 daemon:"活跃项目"是 GUI 侧的概念了(P2a
        // Task 1-3 删掉了 SetActiveProject)。
        //
        // 这个项目已经开着页签(`Loaded` 或还没促成的 `Stub`)时,点最近
        // 项目卡片就只是"切到那个页签",走与点页签完全相同的非破坏性
        // 路径——绝不能杀掉任何已有页签的会话(设计文档 §2)。
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            self.adopt_panel_layout(id);
            self.maximized = None;
            self.current_page = AppPage::Workspace;
            self.ensure_loaded(id);
            // 清放大态改变了终端 pane 的像素尺寸,网格必须跟着重算:
            // `terminal_grid_state` 把 `maximized` 算进去,不重算的话
            // PTY 会一直停在放大时的 cols/rows,直到某个无关的几何事件
            // 偶然触发一次重算(最终审查 Required Fix #2)。
            self.sync_terminal_grid();
            self.persist_open_projects();
            return;
        }
        // 还没开着:作为**新页签**打开(与顶栏"＋"同一条 `ProjectTabOpened`
        // 落地路径),而不是把当前页签的内容换掉——多页签下"点一张最近
        // 项目卡片"的直觉是"再开一个",不是"把手上这个换掉"。
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let recent = client.list_projects().await.unwrap_or_default();
            let opened = recent.iter().find(|p| p.id == id).cloned();
            let _ = proxy.send_event(Message::ProjectTabOpened(opened, recent));
        });
    }

    fn project_tab_opened(&mut self, project: Option<ProjectInfo>, recent: Vec<ProjectInfo>) {
        self.recent_projects = recent.clone();
        // `None` = 这次打开失败(daemon 不通/回 `Reply::Error`)。硬性
        // 要求:失败绝不能落进任何 `Workspace`,否则会留下"有界面、没
        // 归属项目"的破状态,用户一点 tab 栏的"＋"就 panic
        // (`spawn_new_tab` 的 expect)。失败文案挂到 App 级的
        // `daemon_error` 上——它不依赖任何 `Workspace` 存在,一个项目
        // 都没打开时空态视图也画得出来(Required Fix #1)。
        let Some(project) = project else {
            tracing::warn!("打开项目页签失败,页签集合保持不变");
            self.daemon_error = Some("打开项目失败,请确认 dozerd 正常后重试".to_string());
            self.with_focused_project(move |ws, _io| {
                ws.recent_projects = recent;
            });
            return;
        };
        self.daemon_error = None;
        // 放大态是外壳态,换页签后留着只会挡住新页签的界面。
        self.maximized = None;
        let id = project.id;
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            self.adopt_panel_layout(id);
            // 这个项目已经开着页签了:只前台化,绝不改写它的内容——
            // 那会把这个页签既有的终端全关掉、文件树对话列表全清空重来。
            self.current_page = AppPage::Workspace;
            self.ensure_loaded(id);
            self.with_focused_project(move |ws, _io| {
                ws.recent_projects = recent;
            });
            self.sync_terminal_grid(); // 清放大态后重算网格,理由见 `ProjectSelect`
            self.persist_open_projects();
            return;
        }
        let io = self.shell_io();
        let repo_path = PathBuf::from(&project.path);
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.recent_projects = recent;
        ws.adopt_project(&io, project);
        self.projects
            .insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
        self.project_order.push(id);
        self.active_project_id = Some(id);
        // 换成新项目的面板布局(它自己没存过就退化成默认)。
        self.adopt_panel_layout(id);
        self.current_page = AppPage::Workspace;
        self.sync_terminal_grid(); // 同上
        self.persist_open_projects();
        // 新开的项目 tab(区别于"已开着、只是前台化"那条 `focus_project_tab`
        // 早退分支):静默跑一次 ensure(README/.dozer/git/agent 历史),不
        // 展示结果(见 project_scaffold 设计"打开即 ensure"一节)。
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Project(m));
        };
        project::spawn_scaffold_run(repo_path, client, &handle, emit);
    }

    fn project_tab_switch(&mut self, id: i64) {
        // 切页签只有两件事:改 `active_project_id`、必要时促成 `Stub`。
        // 没有任何内容改写,因此后台项目的终端/预览/审阅原样留着,切
        // 回来还是刚才那副样子。
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if !focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            return;
        }
        self.adopt_panel_layout(id);
        self.maximized = None;
        self.current_page = AppPage::Workspace;
        self.ensure_loaded(id);
        // Git Log 面板已经开着的话,提交图缓存是 `App` 级的、不随项目
        // 页签走(见 `sync_git_log_to_active_project` 文档),不补这一
        // 下切页签会让提交图停在上一个项目,跟同一面板里已经按新项目
        // 刷新的 worktree 速览条对不上。
        if self.left_view == PanelKind::GitLog {
            self.sync_git_log_to_active_project();
        }
        // 清放大态后必须重算终端网格。`PaneResized` 那条分支只在**窗口
        // 几何变化**时触发,清 `maximized` 不会自己走到那里;而
        // `terminal_grid_state` 把 `maximized` 算进公式,不重算的话
        // "在项目 A 放大终端 → 切到 B"会让 A 的 PTY 停在放大时的
        // cols/rows(最终审查 Required Fix #2)。重算是幂等的:算出来
        // 与当前 `cols/rows` 相同时 `PaneResized` 的去重会原地返回。
        self.sync_terminal_grid();
        self.persist_open_projects();
        // 按下项目页签＝选中＋准备被拖走(同终端/预览页签)。
        if let Some(idx) = self.project_order.iter().position(|p| *p == id) {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Project,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    fn project_tab_close(&mut self, id: i64) {
        // 关掉当前页签前先把它的面板布局原样存下(焦点还在它身上,
        // `stash` 会记进 `id` 那份),以后重开还能恢复。
        self.stash_active_panel_layout();
        let io = self.shell_io();
        let Some(slot) = take_project_tab(
            &mut self.projects,
            &mut self.project_order,
            &mut self.active_project_id,
            id,
        ) else {
            return;
        };
        if let WorkspaceSlot::Loaded(mut ws) = slot {
            // 关页签 = 结束该项目下所有会话(abort 转发任务 + kill
            // daemon 侧会话)。不 kill 的话会话会继续在 daemon 上跑,
            // 还会被下次 bootstrap 恢复出来。
            ws.close_all_tabs_for_switch(&io);
        }
        self.maximized = None;
        // 焦点被 `take_project_tab` 挪到了邻居页签上,而那个邻居可能还
        // 是个懒加载 `Stub`——`view()` 走的是只读的 `active_workspace()`,
        // 它**不促成** `Stub`,于是界面会画成"未打开任何项目",尽管顶栏
        // 那个页签明明高亮着。必须在这里显式促成(最终审查 Required
        // Fix #3)。
        if let Some(next) = self.active_project_id {
            // 焦点被挪到了邻居页签,把它的面板布局换上来。
            self.adopt_panel_layout(next);
            self.ensure_loaded(next);
        }
        self.sync_terminal_grid(); // 清放大态后重算网格,理由同 `ProjectTabSwitch`
        self.persist_open_projects();
    }

    /// "删除项目"确认弹窗的"删除"按钮触发,由 `Message::Project(project::
    /// Message::DeleteProjectConfirm)` 拦截调用(见该分支注释)。这个操作
    /// 一定作用在当前聚焦的项目上——删除按钮本来就在那个项目自己的面板
    /// 里,不存在"删除一个没打开的项目"这回事。
    fn project_delete_confirm(&mut self) {
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(scope) = ws.project_panel.delete_pending.take() else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = PathBuf::from(&project.path);
        // 关 tab 必须在发起删除请求之前——删除一旦成功,这个项目在
        // dozerd/磁盘上都可能已经不存在了,`Workspace` 不该继续留着。
        self.project_tab_close(project_id);
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let on_done = move |errors: Vec<String>| {
            let _ = proxy.send_event(Message::ProjectDeleteDone(errors));
        };
        project::delete::spawn_delete_project(
            project_id, repo_path, scope, client, &handle, on_done,
        );
    }

    fn project_slot_loaded(&mut self, id: i64, payload: RestorePayload) {
        let Some(restore) = payload.take() else {
            return; // 信封已被取走(理论上不会发生),没有素材可落地
        };
        // 只在槽位仍是那份"加载中"占位时落地。两种落空情形:
        // - 页签在促成完成前被用户关掉了(槽位已不存在);
        // - 槽位已经被别的路径换成了真正的内容(比如
        //   `ProjectTabOpened` 的 `adopt_project`)。
        // 两种情形下这份素材都没人要了,但它已经 attach 上了该项目在
        // daemon 上的存活会话——直接 drop 只是断开事件流,daemon 侧
        // 会话仍在跑,会变成"没有任何页签持有、却还占着 PTY"的野会话。
        // 所以按关页签的语义结束掉它们(`ProjectTabClose` 同款处理)。
        // 落地的同时把占位那份 `allowed_files` 句柄接过来:main.rs 的
        // webview 池只在 `active_project_id` **变化**时才清空,它看不见
        // "同一个项目换了一份 `Workspace` 对象"。促成窗口期里用户点开
        // 的文件预览已经建出一个 id 0 的 webview,其 `dozer://` 协议
        // 闭包捕获的是**占位那一个** `Arc`;新 `Workspace` 若另起一个
        // `Arc`,`restore_preview_state` 重开的 id 0 会被
        // `sync_webview_pool` 认成"这个 id 已经有 webview 了"而只调
        // `load_url`,于是文件请求走的还是旧 `Arc` 的白名单 → 对不上
        // → 空白预览。这与 Required Fix #3 是同一个失效模式,只是触发
        // 点从"切项目"变成"促成换对象"。共用同一个 `Arc` 即可,而且
        // 不损失已经建好的 webview(比清空池更省一次导航)。
        let inherited = match self.projects.get(&id) {
            Some(WorkspaceSlot::Loaded(cur)) if cur.loading => Some(cur.allowed_files()),
            _ => None,
        };
        let landed = inherited.is_some();
        if !landed {
            let client = self.client.clone();
            let ids: Vec<String> = restore
                .sessions
                .iter()
                .map(|(info, _, _)| info.id.clone())
                .collect();
            let task = self.handle.spawn(async move {
                for sid in ids {
                    if let Err(e) = client.kill(&sid).await {
                        tracing::warn!("丢弃过期促成结果时结束会话失败: {e}");
                    }
                }
            });
            if let Ok(mut pending) = self.pending_exit_tasks.lock() {
                pending.push(task);
            }
            return;
        }
        let io = self.shell_io();
        let mut ws = Workspace::from_restore(&io, *restore, inherited);
        // 重挂出来的会话,终端模型是按 `DEFAULT_COLS`×`DEFAULT_ROWS`
        // 建的,得按当前窗口几何纠正一次。这里**不能**指望
        // `sync_terminal_grid`:它算出来的网格与 `self.cols/rows` 相同
        // 时 `PaneResized` 会原地返回(去重),于是这份新装配的
        // `Workspace` 会一直停在 80×24。直接对它自己 resize 一次——
        // 共享与 SSH 两个 pane 各按自己跟踪的网格分别纠正。
        ws.resize_all(&io, io.cols, io.rows, self.ssh_cols, self.ssh_rows);
        self.projects
            .insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
    }

    fn project_fs_changed(&mut self, project_id: ProjectId, changes: git_watch::FsChanges) {
        // 工作区类变更:文件树 + 打开的 webview 预览即时跟进。`notify` 递上
        // 的是具体变更路径 `changes.paths`,文件树按"受影响即相关"整棵从盘重
        // 读已缓存目录(`reload_tree_from_disk`,只重读已展开/缓存过的层,开销
        // 小),预览则只重载路径命中的 webview tab。
        if changes.relevance == Some(git_watch::Relevance::Workdir)
            || changes.relevance == Some(git_watch::Relevance::GitRefs)
        {
            self.with_project(project_id, |ws, _io| {
                ws.files.reload_tree_from_disk();
                ws.preview.reload_webviews_for(&changes.paths);
                ws.project_preview.reload_webviews_for(&changes.paths);
            });
        }
        self.with_project(project_id, |ws, io| {
            let Some(project) = &ws.project else { return };
            let repo_path = PathBuf::from(&project.path);
            spawn_project_git_refresh(project_id, repo_path.clone(), io);
            spawn_disk_usage_refresh(project_id, repo_path, io);
        });
        // 只有 `.git` 引用类变化(分支切换/外部提交/其他 worktree
        // 提交)才值得重建 Git Log 快照——纯工作区文件编辑不影响
        // 提交历史,重算是纯浪费。`git_log_cache` 是 `App` 级、不是
        // 按项目分的(见 `sync_git_log_to_active_project`),所以这里
        // 必须先核实这条事件本来就是"当前聚焦项目"发出的
        // (`project_id == self.active_project_id`)——否则后台项目
        // 的引用变化会拿"缓存路径恰好等于前台项目路径"这个巧合当
        // 通行证,把前台正打开的详情/选中态平白清掉,而其实什么都
        // 没变。项目 id 匹配之外再核一次路径,双保险防状态漂移。
        if changes.relevance == Some(git_watch::Relevance::GitRefs)
            && self.active_project_id == Some(project_id)
            && let Some(repo_path) = self.git_log.cache_repo_path().map(|p| p.to_path_buf())
            && self
                .active_workspace()
                .and_then(|ws| ws.active_project_path())
                .as_deref()
                == Some(repo_path.as_path())
        {
            // 引用变化只是要"内容不变、重新拉一遍",窗口大小维持原样——
            // 用 `cache_max_count()`(读当前缓存的 max_count),不是"加载
            // 更多"专用、会 `+LOAD_MORE_STEP` 的 `next_load_more_count()`。
            let max = self.git_log.cache_max_count();
            let handle = self.handle.clone();
            let proxy = self.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::GitLog(m));
            };
            git_log::request_refresh(&mut self.git_log, repo_path, max, &handle, emit);
        }
    }

    fn database_test_connection_result(
        &mut self,
        project_id: i64,
        source_id: String,
        result: Result<(), String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::TestConnectionResult(project_id, source_id, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn database_browse_result(
        &mut self,
        project_id: i64,
        tab_id: usize,
        run_seq: u64,
        result: Result<database::BrowsePage, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::BrowseResult(project_id, tab_id, run_seq, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn database_query_result(
        &mut self,
        project_id: i64,
        tab_id: usize,
        run_seq: u64,
        result: Result<database::QueryOutcome, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::QueryResult(project_id, tab_id, run_seq, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn database_tables_loaded(
        &mut self,
        project_id: i64,
        source_id: String,
        result: Result<Vec<database::TableRef>, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::TablesLoaded(project_id, source_id, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn database_columns_loaded(
        &mut self,
        project_id: i64,
        source_id: String,
        schema: Option<String>,
        table: String,
        result: Result<Vec<database::ColumnInfo>, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::ColumnsLoaded {
                project_id,
                source_id,
                schema,
                table,
                result,
            },
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn database_message(&mut self, msg: database::Message) {
        // 四个动作都可能来自数据源树 header 行的右键菜单——菜单本体没有
        // "点了就自动收起"的行为(popup 盖在 dismiss 遮罩之上,点菜单项本身
        // 吃不到遮罩的点击),落地时顺手收掉,不来自菜单时该字段本就是
        // `None`,无副作用。
        if matches!(
            msg,
            database::Message::TestConnection(_)
                | database::Message::EditSourceStart(_)
                | database::Message::DeleteSource(_)
                | database::Message::SchemaRefresh(_)
        ) {
            self.database_source_menu = None;
        }
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            msg,
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn ssh_test_connection_result(
        &mut self,
        project_id: i64,
        host_id: String,
        result: Result<(), String>,
    ) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::TestConnectionResult(project_id, host_id, result),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    fn ssh_unknown_key_detected(
        &mut self,
        project_id: i64,
        host_id: String,
        fingerprint: String,
        key_bytes: Vec<u8>,
    ) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::UnknownKeyDetected(project_id, host_id, fingerprint, key_bytes),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    fn ssh_key_changed(&mut self, project_id: i64, host_id: String, fingerprint: String) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::KeyChanged(project_id, host_id, fingerprint),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    fn ssh_terminal_connect_failed(
        &mut self,
        project_id: i64,
        host_id: String,
        tab_id: usize,
        err: String,
    ) {
        self.with_project(project_id, move |ws, io| {
            ws.pending.remove(&tab_id);
            ws.ssh_out_pending.remove(&tab_id);
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::TerminalConnectFailed(project_id, host_id, tab_id, err),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    /// 指派任务给某个 agent 种类,纯记录,不触发任何执行。异步确认经
    /// `Mutated` 刷新列表——与其它写操作同一条乐观更新链路。
    fn todo_assign_agent(&mut self, idx: usize, agent: dozer_core::protocol::AgentKind) {
        let Some(id) = self
            .active_workspace()
            .and_then(|ws| ws.todo.items().get(idx))
            .map(|item| item.id)
        else {
            return;
        };
        self.with_focused_project(|ws, _io| {
            ws.todo.close_dispatch_popup();
        });
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            let res = client
                .assign_todo_agent(id, agent)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = proxy.send_event(Message::Todo(todo::Message::Mutated(res)));
        });
    }

    /// 打开任务详情弹窗:先本地记下 `idx`(弹窗定位/后续"处理"要用),再
    /// 异步拉 `GetTodoDetail`。RPC 结果经专门的 `Message::TodoDetailLoaded`
    /// 落地——`todo::Message::Mutated` 那条通用刷新链路只刷 `items`/
    /// `categories`,不携带回合数据,不能复用。
    fn todo_detail_open(&mut self, idx: usize) {
        let Some(id) = self
            .active_workspace()
            .and_then(|ws| ws.todo.items().get(idx))
            .map(|item| item.id)
        else {
            return;
        };
        self.with_focused_project(|ws, _io| {
            ws.todo.open_detail(idx, Vec::new());
        });
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            if let Ok((_, turns)) = client.get_todo_detail(id).await {
                let _ = proxy.send_event(Message::TodoDetailLoaded(idx, turns));
            }
        });
    }

    /// 详情弹窗"处理"按钮:乐观插入已经在 `todo::update`(`DetailReplySubmit`
    /// 分支)做过,这里只管发 `ProcessTodoNow` RPC 并在结果回来后用服务端
    /// 权威回合列表刷新。耗时可能到 10 分钟,走 `handle.spawn` 不阻塞 UI。
    fn todo_detail_process(&mut self) {
        let Some((idx, id, reply_text)) = self.active_workspace().and_then(|ws| {
            let idx = ws.todo.detail_open_idx()?;
            let id = ws.todo.items().get(idx)?.id;
            Some((idx, id, ws.todo.last_reply_text()))
        }) else {
            return;
        };
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            let _ = client.process_todo_now(id, reply_text.as_deref()).await;
            if let Ok((_, turns)) = client.get_todo_detail(id).await {
                let _ = proxy.send_event(Message::TodoDetailLoaded(idx, turns));
            }
        });
    }

    fn todo_message(&mut self, msg: todo::Message) {
        // 新增任务框高度拖拽:只在 app 层接管,置 `dragging_row`,后续
        // `CursorMoved` → `RowDrag` 由 `update` 统一换算高度写回
        // `ws.todo`(见 `RowDrag` 的 `TodoAddGrow` 分支)。这条不到
        // `todo::update`(那里有 no-op arm 保持 match 穷尽)。
        if let todo::Message::AddResizeStart = msg {
            self.dragging_row = Some(RowDivider::TodoAddGrow);
            return;
        }
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let last_cursor = self.last_cursor;
        // 异步结果/写确认透过 `proxy` 重发回主循环,回调里会借用 `self`
        // 的 client/handle ——闭包捕获是 move 出来的副本,行得通(同
        // `Message::Search` 分支的既有手法)。
        let emit = move |m: todo::Message| {
            let _ = proxy.send_event(Message::Todo(m));
        };
        self.with_focused_project(move |ws, _io| {
            // 点日历按钮时的光标逻辑坐标,作为窗口级 overlay 的弹出锚点——
            // 先记下再交给 `todo::update` 展开(它只管 `calendar_open`/`calendar_view`)。
            if matches!(msg, todo::Message::CalendarOpen(_)) {
                ws.todo.set_calendar_anchor(last_cursor);
            }
            // 点"指派"按钮时的光标逻辑坐标,作为派发选择层 overlay 的弹出锚点。
            if matches!(msg, todo::Message::DispatchOpen(_)) {
                ws.todo.set_dispatch_anchor(last_cursor);
            }
            // 点卡片左下"状态"按钮的光标逻辑坐标,作为状态下拉选择层 overlay
            // 的弹出锚点。`StatusOpen` 自身交给 `todo::update` 展开(它只改
            // `status_open`)。
            if matches!(msg, todo::Message::StatusOpen(_)) {
                ws.todo.set_status_anchor(last_cursor);
            }
            // 点搜索框左前"状态"segment 按钮时的光标逻辑坐标,作为搜索框状态
            // 筛选浮层 overlay 的弹出锚点。`StatusFilterOpen` 自身交给
            // `todo::update` 展开(它只改 `status_filter_open`)。
            if matches!(msg, todo::Message::StatusFilterOpen) {
                ws.todo.set_status_filter_anchor(last_cursor);
            }
            todo::update(&mut ws.todo, msg, project_id, &client, &handle, emit);
        });
    }

    fn browser_bookmarks_loaded(&mut self, pid: Option<i64>, bookmarks: Vec<BookmarkInfo>) {
        if pid.is_none() {
            // 首页全局浏览器(或无项目工作区)的收藏列表刷新。
            let handle = self.handle.clone();
            let client = self.client.clone();
            let proxy = self.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::HomeBrowser(m));
            };
            browser::update(
                &mut self.home_browser,
                browser::Message::BookmarksLoaded(None, bookmarks),
                None,
                &client,
                &handle,
                emit,
            );
            return;
        }
        self.with_project(pid.unwrap(), move |ws, io| {
            let pid = pid.unwrap();
            let handle = io.handle.clone();
            let client = io.client.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(
                &mut ws.browser,
                browser::Message::BookmarksLoaded(Some(pid), bookmarks),
                Some(pid),
                &client,
                &handle,
                emit,
            );
        });
    }

    fn browser_bookmarks_mutated(&mut self, pid: Option<i64>, res: Result<(), String>) {
        if pid.is_none() {
            // 首页全局浏览器(或无项目工作区)的添加/删除结果回调。
            let handle = self.handle.clone();
            let client = self.client.clone();
            let proxy = self.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::HomeBrowser(m));
            };
            browser::update(
                &mut self.home_browser,
                browser::Message::BookmarksMutated(None, res),
                None,
                &client,
                &handle,
                emit,
            );
            return;
        }
        self.with_project(pid.unwrap(), move |ws, io| {
            let pid = pid.unwrap();
            let handle = io.handle.clone();
            let client = io.client.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(
                &mut ws.browser,
                browser::Message::BookmarksMutated(Some(pid), res),
                Some(pid),
                &client,
                &handle,
                emit,
            );
        });
    }

    fn browser_message(&mut self, msg: browser::Message) {
        // 按下浏览器页签＝选中＋准备被拖走(`SelectTab` 在
        // `browser::update` 里真正选中为 `active`,这里按它记下拖起源)。
        let was_select = matches!(msg, browser::Message::SelectTab(_));
        self.with_focused_project(|ws, io| {
            let project_id = ws.project.as_ref().map(|p| p.id);
            let client = io.client.clone();
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(&mut ws.browser, msg, project_id, &client, &handle, emit);
        });
        if was_select
            && let Some(ws) = self.active_workspace()
            && ws.browser.active_tab_idx() < ws.browser.tab_count()
        {
            let active = ws.browser.active_tab_idx();
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Browser,
                source: active,
                press_pos: self.last_cursor,
            });
        }
    }

    fn term_input(&mut self, target: terminal::TermTarget, bytes: Vec<u8>) {
        // 终端不在屏上时丢弃按键(不报错、不写 PTY):否则用户在读
        // 对话审阅时敲的回车/方向键会静默提交给隐藏在后面的 agent
        // 会话(Fix round 2 #3)。
        let visible = match target {
            terminal::TermTarget::Shared => self.terminal_visible(),
            terminal::TermTarget::SshPanel => self.ssh_terminal_visible(), // Task 12 新增
        };
        if !visible {
            return;
        }
        self.with_focused_project(|ws, io| {
            match target {
                terminal::TermTarget::Shared => {
                    // 键入即回底 + 清选区：正在回看历史时一敲键盘，视口跳回
                    // 实时输出（常规终端语义），再把字节写给 daemon。
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.scroll_to_bottom();
                        tab.model.selection_clear();
                    }
                    ws.send_input(io, bytes);
                }
                terminal::TermTarget::SshPanel => {
                    if let Some(tab) = ws.ssh_active_tab_mut() {
                        tab.model.scroll_to_bottom();
                        tab.model.selection_clear();
                    }
                    ws.ssh_send_input(io, bytes);
                }
            }
        });
    }

    fn term_output(&mut self, project_id: ProjectId, tab_id: usize, bytes: Vec<u8>) {
        self.with_project(project_id, |ws, io| {
            let Some(tab) = ws.tab_by_id_mut(tab_id) else {
                return;
            };
            // 实时输出可能含设备查询（DSR/DA 等），应答必须写回 PTY
            // ——atuin/claude 等 TUI 依赖它（此前丢弃导致探测超时）。
            tab.ingest_osc(&bytes);
            let responses = tab.model.feed(&bytes);
            if responses.is_empty() || !tab.alive {
                return;
            }
            match &tab.backend {
                TabBackend::Daemon => {
                    let client = io.client.clone();
                    let id = tab.info.id.clone();
                    io.handle.spawn(async move {
                        if let Err(e) = client.write(&id, &responses).await {
                            tracing::warn!("回写终端查询应答失败: {e}");
                        }
                    });
                }
                TabBackend::Ssh { out } => {
                    let _ = out.send(SshOut::Data(responses));
                }
            }
        });
    }

    fn term_paste(&mut self, target: terminal::TermTarget, text: String) {
        // 同 TermInput 的可见性闸门(Fix round 3):⌘V 粘贴走同一条
        // PTY 写入路径,粘贴内容若含换行还会在看不见的会话里直接
        // 执行,比单个按键更危险,必须同样拦截。
        let visible = match target {
            terminal::TermTarget::Shared => self.terminal_visible(),
            terminal::TermTarget::SshPanel => self.ssh_terminal_visible(),
        };
        if !visible {
            return;
        }
        self.with_focused_project(move |ws, io| {
            let bracketed = match target {
                terminal::TermTarget::Shared => {
                    ws.tabs.get(ws.active).map(|t| t.model.bracketed_paste())
                }
                terminal::TermTarget::SshPanel => ws
                    .ssh_tabs
                    .iter()
                    .find(|t| {
                        ws.ssh_active.as_ref().is_some_and(|(h, _)| {
                            t.info.id.strip_prefix("ssh:") == Some(h.as_str())
                        })
                    })
                    .map(|t| t.model.bracketed_paste()),
            };
            let Some(bracketed) = bracketed else {
                return;
            };
            match target {
                terminal::TermTarget::Shared => {
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.scroll_to_bottom();
                    }
                }
                terminal::TermTarget::SshPanel => {
                    if let Some(tab) = ws.ssh_active_tab_mut() {
                        tab.model.scroll_to_bottom();
                    }
                }
            }
            let bytes = if bracketed {
                let mut b = b"\x1b[200~".to_vec();
                b.extend_from_slice(text.as_bytes());
                b.extend_from_slice(b"\x1b[201~");
                b
            } else {
                text.into_bytes()
            };
            match target {
                terminal::TermTarget::Shared => ws.send_input(io, bytes),
                terminal::TermTarget::SshPanel => ws.ssh_send_input(io, bytes),
            }
        });
    }

    fn select_tab(&mut self, idx: usize) {
        // 按下页签＝选中＋准备被拖走:选中仍是唯一的语义,但顺带记下
        // "这一页签正被按住",随后鼠标划过其它页签时 `on_move` 触发
        // `TabDragMove` 完成换位;松开时 main.rs `TabDragEnd` 收尾。
        // 只应该被 tab 栏本身的按钮调用——见 `Message::SelectTab` 文档。
        self.select_tab_no_drag(idx);
        if let Some(ws) = self.active_workspace()
            && idx < ws.tabs.len()
        {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Terminal,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    /// `select_tab` 去掉"武装拖拽状态机"那部分,给非 tab 栏的调用方
    /// (Agent 面板右侧卡片列表)用——见 `Message::SelectTabNoDrag` 文档。
    fn select_tab_no_drag(&mut self, idx: usize) {
        self.with_focused_project(|ws, _io| {
            if idx < ws.tabs.len() {
                ws.active = idx;
                // 溢出下拉里选中一个被挤出可见区的 tab:选中它之后自动把
                // 主条滚入可见窗口(已可见则不受影响,见 `tab_window_reveal`),
                // 并收起下拉——避免"选中了却看不见在哪"。
                let widths: Vec<f32> = ws
                    .tabs
                    .iter()
                    .map(|t| tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name)))
                    .collect();
                ws.term_tab_first = tab_widget::tab_window_reveal(
                    &widths,
                    4.0,
                    byteui::theme::geometry::tab_bar_avail_px(),
                    ws.term_tab_first,
                    idx,
                );
                ws.term_tab_overflow_anchor = None;
            }
        });
        // `term_ime_preedit` 是 `App` 上唯一一份、不按 tab 分的组字预览态
        // (见该字段文档),只在真正 `Ime::Commit`/组字取消时才清空——切
        // tab 不会清。`TermTarget::Shared` 这道"该不该显示"闸门只判断
        // "键盘现在归不归共享终端条",不区分具体哪个 tab,于是新切过去的
        // tab 会直接"继承"上一个 tab 还没提交完的组字预览文字/候选词
        // (2026-08-21 用户实测反馈:切 tab 后串台,选完字对方也不会真的
        // 收到那些字——因为提交字节确实是发给切换后的新 tab 的 PTY,只是
        // 预览视觉是借来的)。切 tab 时无条件清掉(即使 idx 越界导致上面
        // 没真的切,清掉一份陈旧组字预览也没有副作用),避免这份陈旧状态
        // 被新激活的 tab 误当成自己的组字预览渲染出来。
        self.term_ime_preedit = None;
    }

    fn pane_resized(&mut self, cols: u16, rows: u16, ssh_cols: u16, ssh_rows: u16) {
        if cols == 0 || rows == 0 {
            return;
        }
        // 共享与 SSH 两个网格各自带独立去重:任一真变了都要往 dev 文件里
        // propagate,不能因为共享网格没动就跳掉 SSH 网格的同步。
        let shared_changed = (cols, rows) != (self.cols, self.rows);
        let ssh_changed = (ssh_cols, ssh_rows) != (self.ssh_cols, self.ssh_rows);
        if !shared_changed && !ssh_changed {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.ssh_cols = ssh_cols;
        self.ssh_rows = ssh_rows;
        let io = self.shell_io();
        // 终端网格是窗口级的:并行打开的每个项目各有一套终端 tab,但它们
        // 共用同一批 pane。只改当前项目的话,切回后台项目会看到一个停在
        // 旧网格、和 pane 对不上的画面,直到用户偶然再拖一次窗口才纠正——
        // 所以这里对所有已加载项目一起改(`Stub` 还没有任何 tab,促成时
        // 自然按当时的 `io.cols/rows`)。共享/SSH 各自带独立网格下发。
        for slot in self.projects.values_mut() {
            if let WorkspaceSlot::Loaded(ws) = slot {
                ws.resize_all(&io, cols, rows, ssh_cols, ssh_rows);
            }
        }
    }

    fn panel_select(&mut self, kind: PanelKind) {
        let side = self.shell_layout.rail_layout.side_of(kind);
        // 武装拖拽态:按住图标＝准备拖(同 `TabDrag` 的"按下即武装"手法)。
        // 同栏重排 / 跨栏移动都是靠渲染层挂在图标上的 `on_move` 驱动
        // (`Message::RailDragMove`),`MouseMotion` 期间逐帧上报；这里只记下
        // "从哪栏的哪个位置开始拖"。`RailLayout` 的不变式(sanitize 已保证
        // 10 个面板不重不漏分到两栏)确保 `kind` 一定能在 `side_of` 返回的
        // 那一栏里被 `position` 找到。
        let source_index = self
            .shell_layout
            .rail_layout
            .side(side)
            .iter()
            .position(|&k| k == kind)
            .expect("kind 应该在 side_of 返回的那一侧里,sanitize 已保证不变式");
        self.rail_drag = Some(rail::RailDrag {
            source_side: side,
            source_index,
            origin_index: source_index,
            pending_cross_side: None,
            press_pos: self.last_cursor,
        });
        // 点当前已激活的图标:退回未选中并收起对应面板区;但若对侧面板区
        // 也已收起,当前侧就是最后一个还开着的 zone,不能关(两侧对称)。
        let switched = match side {
            Side::Left => {
                if self.left_view == kind {
                    if !self.right_collapsed {
                        self.left_collapsed = !self.left_collapsed;
                    }
                    false
                } else {
                    self.left_view = kind;
                    self.left_collapsed = false;
                    true
                }
            }
            Side::Right => {
                if self.right_view == kind {
                    if !self.left_collapsed {
                        self.right_collapsed = !self.right_collapsed;
                    }
                    false
                } else {
                    self.right_view = kind;
                    self.right_collapsed = false;
                    true
                }
            }
        };
        // 面板专属的"切入时动作"。原左栏处理器把 GitLog/Todo/
        // Database/Project/Ssh 的触发放在 if/else 之后的无条件
        // `if self.left_view == PanelKind::X` 里——收起/展开当前激活的特殊
        // 面板也会跑一遍;原右栏处理器把 Usage 放在 else(真正
        // 切换)分支里——只有切换时才触发。为保持逐像素零差异,左侧面板恒
        // 触发、右侧面板仅在真正切换时触发(默认布局下它们恰好按这个分侧;
        // Stage 4 拖拽换栏后这里再按 `rail_layout.side_of` 重新对齐各面板
        // 的触发语义)。
        let fire = match side {
            Side::Left => true,
            Side::Right => switched,
        };
        if fire {
            match kind {
                PanelKind::GitLog => self.sync_git_log_to_active_project(),
                PanelKind::Todo => {
                    if let Some(project_id) = self.active_project_id {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        let emit = move |m: todo::Message| {
                            let _ = proxy.send_event(Message::Todo(m));
                        };
                        let emit_todos = emit.clone();
                        todo::request_todos_refresh(project_id, &client, &handle, emit_todos);
                        todo::request_categories_refresh(project_id, &client, &handle, emit);
                    }
                }
                PanelKind::Database => self.with_focused_project(|ws, _io| {
                    if let Some(project) = ws.project.as_ref() {
                        database::reload_from_disk(
                            &mut ws.database,
                            std::path::Path::new(&project.path),
                        );
                    }
                }),
                PanelKind::Project => self.ensure_project_readme_and_reveal(),
                PanelKind::Ssh => self.with_focused_project(|ws, _io| {
                    if let Some(project) = ws.project.as_ref() {
                        ssh::reload_from_disk(&mut ws.ssh, std::path::Path::new(&project.path));
                    }
                }),
                PanelKind::Usage => self.with_focused_project(|ws, io| {
                    ws.usage.set_loading(true);
                    ws.spawn_usage_refresh(io);
                }),
                // 会话列表原本只在项目打开时和回合结束时刷新,切进这个面板时
                // 没有任何补救手段——离开一段时间再切回来看到的还是上次的
                // 快照。补一次切入即刷新,同 `Usage` 面板的既有口径。
                PanelKind::Conversations => self.with_focused_project(|ws, io| {
                    ws.spawn_conversations_refresh(io);
                }),
                PanelKind::Files | PanelKind::Web | PanelKind::Agent => {}
            }
        }
        // 图标栏点击一律退出放大态。放大态浮层不拦图标栏上的点击
        // (遮罩两侧垫的是无交互 Space,点击穿到下层图标按钮),所以
        // "放大左侧 → 点文件夹图标收起左侧"是可达的:不清 `maximized`
        // 就会留下一个空的金色描边浮层,只能点变暗区才能脱身
        // (Fix round 2 #2)。切换本侧显示什么内容时,放大态本也不该
        // 存活,无条件清最简单也最不容易出意外。
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    /// 文件预览右上角按钮:翻转文件树列表子栏的展开/收起。只改一个布尔
    /// (`dims.files_tree_collapsed`),不动 `files_split` 比例(展开时按原比例
    /// 恢复)。收起态下文件树列表不渲染、预览拿满整个配对宽度。与
    /// `panel_select` 一样退出放大态并落盘/重算网格。
    fn toggle_files_tree_collapse(&mut self) {
        self.dims.files_tree_collapsed = !self.dims.files_tree_collapsed;
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    /// 该面板当前是否偏离了默认栏——8 个有内部两栏布局的面板据此决定
    /// 渲染顺序要不要反转。这个 Stage 结束时 `RailLayout` 只可能是
    /// `default()`,所以这个函数在正常运行时恒返回 `false`;它的分支
    /// 靠单元测试直接构造非默认 `RailLayout` 来触发验证,不依赖 GUI
    /// 能不能拖拽出这个状态(Stage 4 才有拖拽)。
    pub(crate) fn panel_mirrored(&self, kind: PanelKind) -> bool {
        rail::panel_mirrored_in(&self.shell_layout.rail_layout, kind)
    }

    /// 文件树列表子栏当前是否被收起(文件预览右上角按钮切换)。暴露只读
    /// 的 `dims.files_tree_collapsed` 给 workspace 层渲染收起按钮时用,
    /// `dims` 字段本身保持模块私有。
    pub(crate) fn files_tree_collapsed(&self) -> bool {
        self.dims.files_tree_collapsed
    }

    /// 七个两栏面板(Project/Todo/Database/Ssh/Agent/Conversations/Usage)的
    /// 列表列当前是否被收起。语义同 `files_tree_collapsed`:列表不渲染、内容
    /// 拿满配对宽度,split 比例保留(展开时按原宽度恢复)。
    pub(crate) fn list_collapsed(&self, kind: PanelKind) -> bool {
        match kind {
            PanelKind::Project => self.dims.project_list_collapsed,
            PanelKind::Todo => self.dims.todo_list_collapsed,
            PanelKind::Database => self.dims.database_list_collapsed,
            PanelKind::Ssh => self.dims.ssh_list_collapsed,
            PanelKind::Agent => self.dims.agent_list_collapsed,
            PanelKind::Conversations => self.dims.conversations_list_collapsed,
            PanelKind::Usage => self.dims.usage_list_collapsed,
            _ => false,
        }
    }

    /// 翻转某两栏面板列表列的展开/收起(`Message::TogglePanelListCollapse` 的
    /// 处理)。只改对应布尔、不动 split 比例,并像 `panel_select` 一样退出
    /// 放大态 + 落盘/重算网格。非两栏面板(`Files` 走独立的
    /// `files_tree_collapsed`,其余单/两栏面板无此能力)直接忽略。
    fn toggle_panel_list_collapse(&mut self, kind: PanelKind) {
        let flag = match kind {
            PanelKind::Project => &mut self.dims.project_list_collapsed,
            PanelKind::Todo => &mut self.dims.todo_list_collapsed,
            PanelKind::Database => &mut self.dims.database_list_collapsed,
            PanelKind::Ssh => &mut self.dims.ssh_list_collapsed,
            PanelKind::Agent => &mut self.dims.agent_list_collapsed,
            PanelKind::Conversations => &mut self.dims.conversations_list_collapsed,
            PanelKind::Usage => &mut self.dims.usage_list_collapsed,
            _ => return,
        };
        *flag = !*flag;
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    /// 内容侧"收起/展开列表列"按钮:用户点击某面板内容区的按钮翻转其列表列
    /// 显隐。语义完全对齐文件预览的 `FileTreeCollapse` 按钮(见 `preview_pane_for`),
    /// 只是图标按该面板当前所在栏(左/右)与收起态四选一、tooltip 由调用方
    /// 给静态文案。泛型 `M` 兼容顶层 `Message` 与各扩展模块的本地 `Message`。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        kind: PanelKind,
        collapsed: bool,
        hover_id: HoverId,
        tooltip_collapse: &'a str,
        tooltip_expand: &'a str,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
        let side = self.shell_layout.rail_layout.side_of(kind);
        let (icon, tooltip) = match (side, collapsed) {
            (Side::Left, false) => (icons::IconKind::PanelLeftClose, tooltip_collapse),
            (Side::Left, true) => (icons::IconKind::PanelLeftOpen, tooltip_expand),
            (Side::Right, false) => (icons::IconKind::PanelRightClose, tooltip_collapse),
            (Side::Right, true) => (icons::IconKind::PanelRightOpen, tooltip_expand),
        };
        icons::icon_button_entry(
            icon,
            byteui::theme::icon_size::row(),
            false,
            false,
            self.hover_progress(hover_id),
            false,
            byteui::theme::icon_size::row() + 6.0,
            true,
            on_select,
            on_hover,
            tooltip,
        )
    }

    fn top_bar_home(&mut self) {
        self.current_page = AppPage::Home;
        self.home_recents_loaded = false;
        self.home_left_view = homespace::HomeLeftView::default();
        self.home_project_pages = 1;
        self.home_project_search.clear();
        self.home_project_search_draft.clear();
        self.home_project_search_focused = false;
        self.home_right_view = homespace::HomeRightView::default();
        let projects: Vec<ProjectInfo> = self.recent_projects.iter().take(5).cloned().collect();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let (files, convs) = load_home_recents(&client, &projects).await;
            let _ = proxy.send_event(Message::HomeRecentsLoaded(files, convs));
        });
    }

    fn preview_open_path(&mut self, path: PathBuf) {
        self.with_focused_project(move |ws, io| {
            if !path.is_file() {
                ws.preview_error = Some(format!("文件不存在或不可读: {}", path.display()));
                return;
            }
            ws.preview_error = None;
            ws.files.set_tree_selected(path.clone());
            ws.allowed_files
                .lock()
                .expect("allowed_files 锁")
                .insert(path.clone());
            ws.preview.open_path(path);
            // 新 tab 落在末尾，滚回最左让它可见（P1L T5）。
            ws.preview_tab_first = 0;
            ws.spawn_preview_state_save(io);
            ws.spawn_preview_context_push(io);
        });
    }

    fn preview_select_tab(&mut self, idx: usize) {
        let arming = self
            .active_workspace()
            .map(|ws| idx < ws.preview.tabs().len())
            .unwrap_or(false);
        self.with_focused_project(|ws, io| {
            ws.preview.select(idx);
            if idx < ws.preview.tabs().len() {
                let widths: Vec<f32> = ws
                    .preview
                    .tabs()
                    .iter()
                    .map(|t| preview_tab_display_width(&t.title))
                    .collect();
                ws.preview_tab_first = tab_widget::tab_window_reveal(
                    &widths,
                    4.0,
                    byteui::theme::geometry::tab_bar_avail_px(),
                    ws.preview_tab_first,
                    idx,
                );
                ws.preview_tab_overflow_anchor = None;
            }
            ws.spawn_preview_state_save(io);
            ws.spawn_preview_context_push(io);
        });
        if arming {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Preview,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    /// Project 面板右配对预览打开文件:写入 `ws.project_preview`(独立的
    /// `PreviewPane`),完全不碰 Files 预览的 `ws.preview`/`ws.files`。
    /// tab 是项目链接点开产生的会话期状态,不持久化、也不向 daemon 推上下文,
    /// 避免与 Files 预览那份持久化 `preview_state` 互相覆盖。
    fn project_preview_open_path(&mut self, path: PathBuf) {
        self.with_focused_project(|ws, _io| {
            if !path.is_file() {
                ws.project_preview_error = Some(format!("文件不存在或不可读: {}", path.display()));
                return;
            }
            ws.project_preview_error = None;
            ws.project_panel.set_selected_link(path.clone());
            ws.allowed_files
                .lock()
                .expect("allowed_files 锁")
                .insert(path.clone());
            ws.project_preview.open_path(path);
            // 新 tab 落在末尾,滚回最左让它可见(同 Files 预览)。
            ws.project_preview_tab_first = 0;
        });
    }

    /// 项目信息面板切入时调用:确保项目根目录有一份 `README.md`(没有就按
    /// 项目名 + 描述生成,已有则原样保留),然后**一律**在右侧配套预览窗打
    /// 开这份 README(首次切进来就让它展示项目文档)。
    ///
    /// 打开/生成依赖同一份"可读"保障——README 创建失败或不可读时静默返回,
    /// 绝不拿一个空文件去占预览,也绝不让面板切入失败。
    fn ensure_project_readme_and_reveal(&mut self) {
        let Some(project) = self.active_workspace().and_then(|ws| ws.project.clone()) else {
            return;
        };
        let Some(readme) =
            ensure_project_readme(&std::path::PathBuf::from(&project.path), &project.name)
        else {
            return;
        };
        self.project_preview_open_path(readme);
    }

    fn project_preview_select_tab(&mut self, idx: usize) {
        let arming = self
            .active_workspace()
            .map(|ws| idx < ws.project_preview.tabs().len())
            .unwrap_or(false);
        self.with_focused_project(|ws, _io| {
            ws.project_preview.select(idx);
            if idx < ws.project_preview.tabs().len() {
                let widths: Vec<f32> = ws
                    .project_preview
                    .tabs()
                    .iter()
                    .map(|t| preview_tab_display_width(&t.title))
                    .collect();
                ws.project_preview_tab_first = tab_widget::tab_window_reveal(
                    &widths,
                    4.0,
                    byteui::theme::geometry::tab_bar_avail_px(),
                    ws.project_preview_tab_first,
                    idx,
                );
                ws.project_preview_tab_overflow_anchor = None;
            }
        });
        if arming {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::ProjectPreview,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    fn agent_state_changed(
        &mut self,
        project_id: ProjectId,
        tab_id: usize,
        agent: AgentKind,
        state: AgentState,
        transcript_path: Option<String>,
    ) {
        self.with_project(project_id, |ws, io| {
            // 当前项目路径先取出（下面要 &mut 借 tab，冲突）；重锚:项目优先。
            let active_repo = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
            // 卡片刷新要传的数据在这里先摘出来（`Option` 同时充当"tab 是否
            // 存在"的哨兵）：`tab`（来自 `ws.tab_by_id_mut`）借的是整个
            // `ws`，下面 TurnEnded 分支还要再用 `tab`，中间插一句
            // `ws.spawn_agent_card_refresh`（借 `&ws`）会跟这个 `&mut ws`
            // 借用重叠、过不了借用检查；摘成局部变量、挪到这个 `if let`
            // 块结束、`tab` 借用已经释放之后再调用，规避这个冲突,语义不变
            // (仍然是"tab 存在就必调用一次,不进 TurnEnded 条件分支")。
            let mut card_refresh_args: Option<(Option<String>, PathBuf)> = None;
            if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                tab.agent_state = state;
                tab.agent = agent;
                if let Some(tp) = transcript_path {
                    tab.transcript_path = Some(tp);
                }
                tracing::info!(tab_id, ?state, "agent 状态变更");
                card_refresh_args = Some((tab.transcript_path.clone(), tab.effective_cwd()));
            }
            if let Some((transcript_path, cwd)) = card_refresh_args {
                ws.spawn_agent_card_refresh(
                    io,
                    tab_id,
                    agent,
                    transcript_path,
                    cwd,
                    active_repo.clone(),
                );
            }
            // 回合结束后刷新会话列表(transcript 增长/新增；P1j)与项目 git/
            // 磁盘占用状态(文件树装饰随之更新；P1h)。这两项刷新原先分别挂在
            // 会话列表自己的耦合链路、以及已删除的验收检测异步回调
            // (`delivery_checked`)上——验收闭环删除后,后者连带的刷新触发点
            // 也没了,这里改成回合结束就无条件触发,不再依赖任何验收检测结果
            // (前半"会话列表"这条 P1j 当年就已经这样修过一次,这次是把后半
            // "git/磁盘占用"也补齐同样的处理)。
            if state == AgentState::TurnEnded {
                ws.spawn_conversations_refresh(io);
                if let Some(project) = &ws.project {
                    let repo_path = PathBuf::from(&project.path);
                    spawn_project_git_refresh(project_id, repo_path.clone(), io);
                    spawn_disk_usage_refresh(project_id, repo_path, io);
                }
            }
            // 审阅 tab 若开着且属本会话,回合结束重解析 transcript（P1i）。
            if state == AgentState::TurnEnded
                && let Some(rv) = &ws.review
                && review_should_refresh_on_turn(&rv.source, tab_id)
                && let Some((path, tab_agent)) = ws
                    .tabs
                    .iter()
                    .find(|t| t.tab_id == tab_id)
                    .and_then(|t| t.transcript_path.clone().map(|p| (p, t.agent)))
            {
                ws.spawn_review_load(
                    io,
                    ReviewSource::Session(tab_id),
                    path,
                    tab_agent,
                    -1,
                    10_000,
                );
            }
        });
    }

    /// 点击对话面板扁平列表里的某一行——只把审阅面板加载到这一个回合
    /// 点对话面板会话列表某一行 → 打开该 session 的详情审阅:整段回合
    /// 列表 + 总结展示区(标题/全文)。`agent` 由调用方随行内数据一并传入。
    /// 总结数据直接取自已加载的 `SessionRow`,不为此单独发请求(2026-08-27)。
    fn conversation_session_open(&mut self, conversation_id: String, agent: AgentKind) {
        self.with_focused_project(move |ws, io| {
            let Some(row) = ws
                .conversation_sessions
                .as_ref()
                .and_then(|rows| rows.iter().find(|r| r.conversation_id == conversation_id))
            else {
                // 列表刷新与点击之间的竞态(极小概率):这一行已经不在当前
                // 列表里了,直接不打开详情,不 panic、不报错弹窗。
                return;
            };
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let source = ReviewSource::Conversation(conversation_id.clone());
            ws.review = Some(ReviewView {
                source: source.clone(),
                entries: Vec::new(),
                error: None,
                agent,
                nonce: 0,
                summary_title: Some(row.display_title.clone()),
                summary_text: row.summary.clone(),
                summary_time: Some(relative_time_text(row.last_ts, now_ms)),
            });
            ws.spawn_review_load_conversation(
                io,
                conversation_id,
                -1,
                CONVERSATION_DETAIL_PAGE_SIZE,
                false,
            );
        });
    }

    fn search_results(
        &mut self,
        project_id: i64,
        result: Result<Vec<(String, Vec<search::SearchHit>)>, String>,
    ) {
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        self.with_project(project_id, move |ws, _io| {
            let emit = move |m| {
                let _ = proxy.send_event(Message::Search(m));
            };
            search::update(
                &mut ws.search,
                search::Message::SearchResults(project_id, result),
                project_id,
                &handle,
                emit,
            );
        });
    }

    fn files_project_message(&mut self, project_id: i64, msg: files::Message) {
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Files(m));
        };
        let app_files = &mut self.files;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        files::update(&mut ws.files, app_files, msg, project_id, &handle, emit);
    }

    /// 文件预览 tab 右键菜单浮层:含"刷新"(恒有)、"编辑"(仅可编辑文本文件)
    /// 与"关闭"三项。定位坐标由 `PreviewTabContextMenu` 打开时记录,风格与
    /// 文件树右键菜单一致(`files::context_menu_popup`/`menu_item`)。
    fn preview_tab_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.preview_tab_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            Vec::new();
        // “刷新”:从文件系统重新读盘并重新渲染当前预览的文件——右键菜单
        // 里最常用的一档,排最前。
        items.push(crate::menu::item::<Message>(
            Some(icons::IconKind::RefreshCw),
            "刷新",
            Message::PreviewReload(menu.idx),
        ));
        // 仅可编辑文本文件显示"编辑"(见 `is_editable_extension`)。
        if menu.editable {
            items.push(crate::menu::item::<Message>(
                Some(icons::IconKind::Rename),
                "编辑",
                Message::PreviewEditOpen(menu.idx),
            ));
        }
        items.push(crate::menu::item::<Message>(
            None,
            "关闭",
            Message::PreviewCloseTab(menu.idx),
        ));

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::menu::shell(
                items,
                Length::Fixed(byteui::theme::geometry::menu_item_width()),
            );
        // 文件预览的 webview 只铺在 tab 栏**下方**的内容区(这就是 tab 栏本身
        // 始终以 iced 显示、不被 webview 盖住的原因)。右键菜单若向下弹会压到
        // webview、被原生子视图挡住;故改为**向上弹**——以光标为底边、向上展开,
        // 整片落在 tab 栏上方的 iced 区域,既不被 webview 遮、也不用在菜单期间
        // 藏掉预览内容(那个方案会让预览整片消失,体验更差)。
        let window_h = self.window_size.1;
        let bottom = (window_h - menu.y).max(0.0);
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(iced_widget::core::alignment::Vertical::Bottom)
            .padding(Padding {
                top: 0.0,
                left: menu.x,
                right: 0.0,
                bottom,
            })
            .into()
    }

    /// Project 面板右配对预览 tab 右键菜单浮层,语义同
    /// `preview_tab_context_menu_popup`。定位坐标同样复用 `files.last_right_click`
    /// (main.rs 任意右键都会先写入),"编辑"项落 `ProjectPreviewEditOpen`(编辑
    /// project 预览的 tab)、"关闭"落 `ProjectPreviewCloseTab`。
    fn project_preview_tab_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.project_preview_tab_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            Vec::new();
        items.push(crate::menu::item::<Message>(
            Some(icons::IconKind::RefreshCw),
            "刷新",
            Message::ProjectPreviewReload(menu.idx),
        ));
        if menu.editable {
            items.push(crate::menu::item::<Message>(
                Some(icons::IconKind::Rename),
                "编辑",
                Message::ProjectPreviewEditOpen(menu.idx),
            ));
        }
        items.push(crate::menu::item::<Message>(
            None,
            "关闭",
            Message::ProjectPreviewCloseTab(menu.idx),
        ));

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::menu::shell(
                items,
                Length::Fixed(byteui::theme::geometry::menu_item_width()),
            );
        let window_h = self.window_size.1;
        let bottom = (window_h - menu.y).max(0.0);
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(iced_widget::core::alignment::Vertical::Bottom)
            .padding(Padding {
                top: 0.0,
                left: menu.x,
                right: 0.0,
                bottom,
            })
            .into()
    }

    /// Project 面板链接行右键菜单浮层:当前只含"删除"。定位坐标复用
    /// `files.last_right_click`(main.rs 任意右键都会先写入),"删除"回
    /// `project::Message::LinkRemove`。
    fn project_link_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.project_link_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            vec![crate::menu::item::<Message>(
                Some(icons::IconKind::Trash),
                "删除",
                Message::Project(project::Message::LinkRemove {
                    target: menu.target,
                    index: menu.index,
                }),
            )];

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::menu::shell(items, Length::Shrink);
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: menu.y,
                left: menu.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    /// Todo 分类树节点右键菜单浮层:新建子/同级分类、重命名、删除、上移/
    /// 下移、移动到...。定位坐标复用 `files.last_right_click`。
    fn category_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.category_context_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        // 右键的是"全部"/"未分类"伪节点:菜单只含"新建分类"(新建顶层
        // 分类,不挂在任何真实名字下——那两个只是视图桶)。真实节点才给
        // 完整节点操作集。
        let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            match menu.id {
                None => vec![crate::menu::item::<Message>(
                    Some(icons::IconKind::SquarePlus),
                    "新建分类",
                    Message::Todo(todo::Message::CategoryNewSibling(None)),
                )],
                Some(id) => {
                    // 被右键节点的 parent_id,给"新建同级分类"用(同级 =
                    // 挂在同一个 parent_id 下)。取不到就退化为顶层。
                    let sibling_parent_id = self.active_workspace().and_then(|ws| {
                        ws.todo
                            .categories()
                            .iter()
                            .find(|c| c.id == id)
                            .and_then(|c| c.parent_id)
                    });
                    vec![
                        crate::menu::item::<Message>(
                            Some(icons::IconKind::SquarePlus),
                            "新建子分类",
                            Message::Todo(todo::Message::CategoryNewChild(id)),
                        ),
                        crate::menu::item::<Message>(
                            Some(icons::IconKind::SquarePlus),
                            "新建同级分类",
                            Message::Todo(todo::Message::CategoryNewSibling(sibling_parent_id)),
                        ),
                        crate::menu::item::<Message>(
                            Some(icons::IconKind::ChevronUp),
                            "上移",
                            Message::Todo(todo::Message::CategoryMoveSibling(
                                id,
                                dozer_core::protocol::CategoryMoveDirection::Up,
                            )),
                        ),
                        crate::menu::item::<Message>(
                            Some(icons::IconKind::ChevronDown),
                            "下移",
                            Message::Todo(todo::Message::CategoryMoveSibling(
                                id,
                                dozer_core::protocol::CategoryMoveDirection::Down,
                            )),
                        ),
                        crate::menu::item::<Message>(
                            Some(icons::IconKind::FolderOpen),
                            "移动到...",
                            Message::Todo(todo::Message::CategoryReparentPickerOpen(id)),
                        ),
                        crate::menu::item::<Message>(
                            Some(icons::IconKind::Rename),
                            "重命名",
                            Message::Todo(todo::Message::CategoryRenameStart(id)),
                        ),
                        crate::menu::item::<Message>(
                            Some(icons::IconKind::Trash),
                            "删除",
                            Message::Todo(todo::Message::CategoryDelete(id)),
                        ),
                    ]
                }
            };

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::menu::shell(items, Length::Shrink);
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: menu.y,
                left: menu.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    /// 分类选择器浮层:列出当前项目的全部分类节点(全展开按 depth 缩进
    /// 平铺),点"未分类"或某个节点即把 `category_picker` 目标落盘并关闭
    /// (Category → reparent,Todo → set_todo_category)。浮层只服务这两类
    /// "把一个节点挂到某个分类"的动作 —— 搜索框的分类速滤已移除(改为
    /// 按状态过滤),不再有 `Filter` 目标。
    fn category_picker_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let Some(picker) = &self.category_picker else {
            return column![].into();
        };
        let categories = self
            .active_workspace()
            .map(|ws| ws.todo.categories().to_vec())
            .unwrap_or_default();
        let mut list = column![].spacing(2);

        // "未分类"钉在树顶(`None` = 未分类)。
        list = list.push(
            button(text("未分类").size(byteui::theme::font::body()))
                .on_press(Message::CategoryPickerSelect(None))
                .width(Length::Fill)
                .padding([6, 10]),
        );
        // 复用一份没有展开态(全展开)的拍平——选择器只做单次选择,不需要
        // 折叠交互,直接把整棵树按 depth 缩进平铺出来最简单。
        let all_expanded: std::collections::HashSet<i64> =
            categories.iter().map(|c| c.id).collect();
        for row in todo::visible_category_rows(&categories, &all_expanded) {
            let id = row.id;
            let click = Message::CategoryPickerSelect(Some(id));
            list = list.push(
                button(
                    text(format!("{}{}", "  ".repeat(row.depth), row.name))
                        .size(byteui::theme::font::body()),
                )
                .on_press(click)
                .width(Length::Fill)
                .padding([6, 10]),
            );
        }
        let list =
            container(list.width(Length::Fixed(220.0))).style(move |_t: &iced_widget::Theme| {
                container::Style {
                    background: Some(byteui::theme::color::current().card.into()),
                    border: Border {
                        color: byteui::theme::color::current().border,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..container::Style::default()
                }
            });
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: picker.y,
                left: picker.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    /// 任务详情弹窗:原生 iced 渲染(不复用会话面板的 webview trace——
    /// 那套渲染实际内容在 `dozer://review-trace/host.html` 里,任务详情
    /// 只需要看人类/agent 往来文本,不需要工具调用折叠/trace 可视化,
    /// 塞进一个跟随光标定位、随时开合的原生弹窗里没有必要也不合适)。
    fn todo_detail_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let Some(ws) = self.active_workspace() else {
            return column![].into();
        };
        let Some(idx) = ws.todo.detail_open_idx() else {
            return column![].into();
        };
        let Some(item) = ws.todo.items().get(idx) else {
            return column![].into();
        };

        let header = column![
            text(item.text.clone()).size(byteui::theme::font::subtitle()),
            text(
                item.assigned_agent
                    .map(|a| format!("指派给:{}", a.label()))
                    .unwrap_or_else(|| "未指派".to_string())
            )
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
        ]
        .spacing(4);

        let mut turns_col = column![].spacing(8);
        for turn in ws.todo.detail_turns() {
            let label = if turn.role == "human" {
                "你".to_string()
            } else {
                item.assigned_agent
                    .map(|a| a.label().to_string())
                    .unwrap_or_else(|| "AI".to_string())
            };
            turns_col = turns_col.push(
                column![
                    text(label)
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().gold),
                    text(turn.content.clone())
                        .size(byteui::theme::font::body())
                        .width(Length::Fill),
                ]
                .spacing(2),
            );
        }
        let turns_scroll = iced_widget::Scrollable::new(turns_col)
            .width(Length::Fill)
            .height(Length::Fixed(320.0))
            .direction(iced_widget::scrollable::Direction::Vertical(
                byteui::interaction::scrollbar::scrollbar(),
            ))
            .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style());

        let reply_box = container(byteui::form::input_text::view(
            "回复...",
            ws.todo.detail_reply_draft(),
            false,
            Some(todo::detail_reply_field_id()),
            false,
            None,
            false,
            |s| Message::Todo(todo::Message::DetailReplyInput(s)),
        ))
        .width(Length::Fill);
        let submit_label = if ws.todo.detail_processing() {
            "处理中…"
        } else {
            "处理"
        };
        let submit = button(text(submit_label))
            .on_press_maybe(
                (!ws.todo.detail_processing())
                    .then_some(Message::Todo(todo::Message::DetailReplySubmit)),
            )
            .padding([6, 12]);

        let card = column![header, turns_scroll, row![reply_box, submit].spacing(8)]
            .spacing(12)
            .padding(16)
            .width(Length::Fixed(480.0));
        let card = container(card).style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().card.into()),
            border: Border {
                color: byteui::theme::color::current().border,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        });

        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced_widget::core::alignment::Horizontal::Center)
            .align_y(iced_widget::core::alignment::Vertical::Center)
            .into()
    }

    /// 数据库面板数据源树 header 行右键菜单浮层:测试连接/编辑/删除/刷新。
    /// 定位坐标复用 `files.last_right_click`。"刷新"只有该数据源当前已
    /// 展开(有 schema 树数据)才可点,未展开时置灰——展开动作本身走左键
    /// 点 header,不进这个菜单。
    fn database_source_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.database_source_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let source_id = menu.source_id.clone();
        let expanded = self
            .active_workspace()
            .is_some_and(|ws| ws.database.is_expanded(&source_id));
        let dim = byteui::theme::color::current().dim;
        let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = vec![
            crate::menu::item::<Message>(
                Some(icons::IconKind::RefreshCw),
                "测试连接",
                Message::Database(database::Message::TestConnection(source_id.clone())),
            ),
            crate::menu::item::<Message>(
                Some(icons::IconKind::Settings),
                "编辑",
                Message::Database(database::Message::EditSourceStart(source_id.clone())),
            ),
            crate::menu::item::<Message>(
                Some(icons::IconKind::Trash),
                "删除",
                Message::Database(database::Message::DeleteSource(source_id.clone())),
            ),
        ];
        items.push(if expanded {
            crate::menu::item::<Message>(
                Some(icons::IconKind::RotateCw),
                "刷新",
                Message::Database(database::Message::SchemaRefresh(source_id.clone())),
            )
        } else {
            crate::menu::item_locked(Some(icons::IconKind::RotateCw), "刷新", dim)
        });

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::menu::shell(
                items,
                Length::Fixed(byteui::theme::geometry::menu_item_width()),
            );
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: menu.y,
                left: menu.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    /// 输入框右键菜单浮层:固定四项(剪切/复制/粘贴/全选),定位坐标复用
    /// `files.last_right_click`(main.rs 任意右键都会先写入,`TextInputMenuOpen`
    /// 已用它填好 `x/y`)。动作消息回 main.rs——由它合成回 ⌘/Ctrl+`x`/`c`/`v`/`a`
    /// 键盘事件作用到被右键的输入。密码框(`secure`)禁用剪切/复制(置灰)。
    fn text_input_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.text_input_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let dim = byteui::theme::color::current().dim;
        let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            Vec::new();
        if menu.target.secure {
            // 密码框:复制/剪切同原生快捷键一样被禁用,置灰不可点。
            items.push(crate::menu::item_locked(
                Some(icons::IconKind::Scissors),
                "剪切",
                dim,
            ));
            items.push(crate::menu::item_locked(
                Some(icons::IconKind::Copy),
                "复制",
                dim,
            ));
        } else {
            items.push(crate::menu::item(
                Some(icons::IconKind::Scissors),
                "剪切",
                Message::TextInputMenuCut,
            ));
            items.push(crate::menu::item(
                Some(icons::IconKind::Copy),
                "复制",
                Message::TextInputMenuCopy,
            ));
        }
        items.push(crate::menu::item(
            Some(icons::IconKind::ClipboardPaste),
            "粘贴",
            Message::TextInputMenuPaste,
        ));
        items.push(crate::menu::item(
            Some(icons::IconKind::SelectAll),
            "全选",
            Message::TextInputMenuSelectAll,
        ));

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::menu::shell(
                items,
                Length::Fixed(byteui::theme::geometry::menu_item_width()),
            );
        // 常规右键菜单就地向下/向上弹即可,这里输入框多用在面板内容区,直接
        // 以光标为左上锚弹出(必要时可在下方再夹窗口高度,留待需要时加)。
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(iced_widget::core::alignment::Vertical::Top)
            .padding(Padding {
                top: menu.y,
                left: menu.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    pub fn view(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
        // 顶栏先画:它是外壳的一部分(项目页签行 + "＋"就在上面),一个项目都
        // 没打开时更要画得出来——否则用户没有任何入口去打开第一个项目。
        let top = topbar::top_bar(self);
        // 首页落地页:点顶栏 Dozer 进入,独立于工作区(即使没开任何项目也画得
        // 出来)。打开/切换项目会自动退回工作区(见各 `ProjectTab*` 处理器)。
        if self.current_page == AppPage::Home {
            return column![top, homespace::home_page(self, &self.footbar)].into();
        }
        // 一个项目页签都没有(或当前页签还停在 `Stub` 没促成)时的占位正文。
        let Some(ws) = self.active_workspace() else {
            // `daemon_error` 必须在这里也画:它平时挂在 `terminal_pane`/
            // `project_status_bar` 上,而那两处都在"有 `Workspace` 才走到"的
            // 分支里。偏偏 daemon 连不上时(`App::with_daemon_error`)一个项目
            // 都恢复不出来,恰恰只会走到这条空态分支——错误文案于是在最需要它
            // 的时候恰好隐身,用户只看到"点 ＋ 打开一个",点了又静默失败
            // (`ProjectTabOpened(None, ..)` 只是再写一遍 `daemon_error`)。
            // 配色沿用 `terminal_pane` 那条同源文案的 RED(最终审查
            // Required Fix #1)。
            let mut hint_col = column![
                text("未打开任何项目——点顶栏的 ＋ 打开一个")
                    .size(byteui::theme::font::subtitle())
                    .color(byteui::theme::color::current().dim)
            ]
            .spacing(8);
            if let Some(err) = &self.daemon_error {
                hint_col = hint_col.push(
                    text(format!("⚠ {err}"))
                        .size(byteui::theme::font::body())
                        .color(byteui::theme::color::current().red),
                );
            }
            let hint = container(hint_col.padding(16))
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_t: &iced_widget::Theme| container::Style {
                    background: Some(theme::region::background().into()),
                    ..container::Style::default()
                });
            return column![top, hint].into();
        };
        // 用 `row!`(经 `Row::push`/`enclose`)构造:只要子元素里有一个声明了
        // `Length::Fill`/`FillPortion`(如某侧收起时的 `left_panel_area`),
        // 这条 row 自身的宽度就会被自动升级成 `Fill`,从而在 flex 布局里正确
        // 撑满窗口。换成 `Row::from_vec`(其文档明确说明不会检视子元素)或
        // 手动 `.width(Length::Shrink)` 会让 flex 第三阶段(fill 分配)不再
        // 执行,右图标栏就会缩到窗口中间——不要在不理解这个前提的情况下改写。
        let body = row![
            rail::icon_rail(self, Side::Left),
            column![
                row![
                    left_panel_area(self, ws, false),
                    divider_bar(
                        Divider::LeftRight,
                        byteui::theme::color::current().bg,
                        byteui::theme::color::current().bg,
                        Message::ColumnDragStart(Divider::LeftRight),
                    ),
                    right_panel_area(self, ws, false),
                ]
                .height(Length::Fill),
                footbar::view(&self.footbar).map(Message::Footbar),
            ]
            .width(Length::Fill),
            rail::icon_rail(self, Side::Right),
        ];
        let base = column![top, body];

        let popped = if ws.search_popup_open() {
            // 文件树右键"搜索"弹窗:窗口级浮层。遮罩"点点即关"由
            // `search_modal` 内部自己处理(整窗 `SCRIM` 做成可点击目标,卡片
            // 是兄弟元素盖在上面),这里只需把弹窗叠在 `base` 之上。
            let project_root = ws.project.as_ref().map(|p| std::path::Path::new(&p.path));
            stack![
                base,
                search::search_modal(&ws.search, project_root).map(Message::Search)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.edit_session.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::PreviewEditCloseRequest);
            let confirm_discard = ws.edit_session.as_ref().is_some_and(|s| s.confirm_discard);
            if confirm_discard {
                stack![base, dismiss, edit_modal(ws), edit_discard_confirm_popup()]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            } else {
                stack![base, dismiss, edit_modal(ws)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
        } else if ws.files.tree_delete_confirm_is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Files(files::Message::DeleteCancel));
            stack![
                base,
                dismiss,
                files::delete_confirm_popup(&ws.files).map(Message::Files)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.files.pending_move_is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Files(files::Message::MoveCancel));
            stack![
                base,
                dismiss,
                files::move_confirm_popup(&ws.files).map(Message::Files)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if self.files.context_menu_is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Files(files::Message::ContextMenuClose));
            stack![
                base,
                dismiss,
                files::context_menu_popup(&self.files, &ws.files).map(Message::Files)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.files.branch_picker_is_open() {
            // 分支切换弹层:窗口级浮层。下层铺一块透明 `MouseArea` 承接
            // "点弹层外的任何地方收起"(与右键菜单同款 dismiss 约定),弹层
            // 本体(`branch_picker_popup`)只占 git 底栏上方一隅。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Files(files::Message::BranchPickerClose));
            stack![
                base,
                dismiss,
                files::branch_picker_popup(&ws.files).map(Message::Files)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if self.preview_tab_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::PreviewTabContextMenuClose);
            stack![base, dismiss, self.preview_tab_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.project_preview_tab_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectPreviewTabContextMenuClose);
            stack![base, dismiss, self.project_preview_tab_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.project_link_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectLinkContextMenuClose);
            stack![base, dismiss, self.project_link_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.category_context_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::CategoryContextMenuClose);
            stack![base, dismiss, self.category_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.category_picker.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::CategoryPickerClose);
            stack![base, dismiss, self.category_picker_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.text_input_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::TextInputMenuClose);
            stack![base, dismiss, self.text_input_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.database_source_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::DatabaseSourceContextMenuClose);
            stack![base, dismiss, self.database_source_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if ws.agent_picker_open {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::AgentPickerClose);
            stack![base, dismiss, agent_picker_popup(ws)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.project_add_menu_open {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectAddMenuClose);
            stack![base, dismiss, topbar::project_add_menu_popup(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if ws.todo.status_popup_open() {
            // 状态下拉选择层:窗口级 overlay。点弹层外任意处经 dismiss 收起
            // (与右键菜单/分支切换同款约定),弹层本体定位到点击"状态"按钮时
            // 的光标锚点。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Todo(todo::Message::StatusClose));
            match todo::todo_status_overlay(ws, self.window_size) {
                Some(popup) => stack![base, dismiss, popup.map(Message::Todo)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.todo.calendar_popup_open() {
            // 日历浮层:窗口级 overlay。点弹层外任意处经 dismiss 收起(与右键
            // 菜单/分支切换同款约定),弹层本体定位到点击按钮时的光标锚点。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Todo(todo::Message::CalendarClose));
            match todo::todo_calendar_overlay(ws, self.window_size) {
                Some(popup) => stack![base, dismiss, popup.map(Message::Todo)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.todo.dispatch_popup_open() {
            // 派发选择层:窗口级 overlay。点弹层外任意处经 dismiss 收起(与
            // 右键菜单/分支切换同款约定),弹层本体列出可指派的 agent(带图标),
            // 定位到点击"指派"按钮时的光标锚点。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Todo(todo::Message::DispatchClose));
            match todo::todo_dispatch_overlay(ws, self.window_size) {
                Some(popup) => stack![base, dismiss, popup.map(Message::Todo)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.todo.detail_popup_open() {
            // 任务详情弹窗:窗口级 overlay,原生渲染(不走 wry webview)。
            // 点弹层外任意处经 dismiss 收起,与其它 Todo 浮层同款约定。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Todo(todo::Message::DetailClose));
            stack![base, dismiss, self.todo_detail_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if ws.todo.status_filter_popup_open() {
            // 搜索框左前"状态"筛选浮层:窗口级 overlay。点弹层外任意处经
            // dismiss 收起(与右键菜单/分支切换同款约定),弹层本体的每一项
            // (全部/待办/进行中/搁置/已完成)emit `StatusFilterPick`,选中
            // 浮层即收。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Todo(todo::Message::StatusFilterClose));
            match todo::todo_status_filter_overlay(ws, self.window_size) {
                Some(popup) => stack![base, dismiss, popup.map(Message::Todo)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else {
            // 始终用 `Stack` 作根,与上面两个分支(删确认弹窗 / 右键菜单)保持一致:
            // 右键菜单开关会把根 widget 类型在 `Column`(`base.into()`)与 `Stack`
            // 之间切换,而 iced 的 `Tree::diff` 在根 tag 变化时(见
            // `iced_core::widget::tree::Tree::diff`)会整体重建整棵树、丢掉所有
            // 嵌套状态——文件树 scrollable 的滚动偏移就在其中,于是右键后滚动条
            // 跳回顶部。统一成 `Stack` 后根 tag 恒定,`base` 子树被 reconcile 原地
            // 保留,滚动位置不再丢失。
            stack![base].into()
        };

        let with_maximize: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            if let Some(which) = self.maximized {
                stack![popped, maximize_overlay(self, ws, which)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            } else {
                popped
            };

        if self.rail_drag_confirmed() {
            stack![with_maximize, rail::rail_drag_ghost(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.tree_drag_confirmed() {
            stack![with_maximize, files::tree_drag_ghost(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            with_maximize
        }
    }
}

/// 左二预览 pane:表头 + tab 栏 + 地址栏;内容区本体是 wry webview
/// 子视图(不在 iced 树里),这里只留占位背景——无 tab 时显示提示文案。
/// 左一项目栏：项目卡（名称 + git 分支/脏 + 路径）+ 文件树；无项目时"打开项目…" + 最近。
/// 右一 AI 栏（P1j）：视图切换 [对话|Agents] + 对话列表（当前行金框高亮）。
/// 顶栏：左 Dozer 标题、中 并行项目页签行 + "＋"、右 金色目标胶囊 + 设置齿轮。
///
/// 参数从 `&Workspace` 改成 `&App`：页签行要读的是**外壳级**的
/// `projects`/`project_order`/`active_project_id`（哪些项目开着、什么顺序、
/// 谁在前台），单个 `Workspace` 里没有这份信息。目标胶囊仍只讲当前项目，
/// 从 `app.active_workspace()` 取——没有项目在前台时它自然不画。
///
/// 原先中间的 ⌘K 搜索框是视觉占位（没有任何交互接线），让位给页签行；
/// 搜索入口日后回来时应另找位置，不要再把页签挤掉。
/// 顶栏内容行直接吃满 `top_bar_height()` 并 `align_y(Center)` 垂直居中——
/// 高度由 `workspace.json` 的 `geometry.top_bar_height` 单一来源驱动。
/// (`MACOS_TRAFFIC_LIGHT_BAND_HEIGHT` 的 28px 顶对齐约定已废弃:用户要
/// 求顶栏用自身高度居中内容,不再贴 macOS 交通灯基准。)
///
/// Figma 设计稿(Dozer Phase 1 UI,node-id=87:31)里顶栏标题/页签/加号
/// 文字标的都是 Inter Medium——应用没绑定 Inter,用系统默认字体的
/// Medium 档位贴近这个字重意图,不引入新字体文件。
pub(crate) fn top_bar_font() -> Font {
    Font {
        weight: Weight::Medium,
        ..Font::default()
    }
}

/// 上面那个的纯逻辑内核(可单测:构造 `Workspace` 需要 daemon + EventLoop,
/// headless 测试里造不出来,与本文件既有约定一致)。`agent == Unknown` 的
/// 会话(纯 shell/git shell/hook 还没上报过——`SessionInfo::agent` 文档:
/// "首个 hook 事件到达前恒 Unknown")不参与正常优先级竞争,它们的
/// `AgentState` 只是从未被真实 hook 改写过的默认值,不代表真实"空闲"——
/// 混进竞争会让纯 shell 页签显示成跟真实 agent 完成一轮工作同款的
/// cyan"空闲"点,分不清"agent 真空下来了"和"这压根不是 agent 会话"。
/// 若项目里**还有**真实 agent 存活,优先级/颜色照旧只看那些;若存活会话
/// **全是** Unknown,显示"死会话"灰点(不是"没有点"——用户仍要看得出这个
/// 项目有存活会话,只是状态不可知)。
pub(crate) fn project_dot(alive: &[(AgentState, AgentKind)]) -> Option<Color> {
    let real_states: Vec<AgentState> = alive
        .iter()
        .filter(|(_, agent)| *agent != AgentKind::Unknown)
        .map(|(state, _)| *state)
        .collect();
    if let Some(state) = winning_agent_state(&real_states) {
        return Some(agent_state_dot(state));
    }
    if alive.iter().any(|(_, agent)| *agent == AgentKind::Unknown) {
        return Some(byteui::theme::color::current().dim);
    }
    None
}

/// 一组存活会话状态里"最值得关注"的那个(2026-08-17 用户重新定案的优先级):
/// AwaitingInput(agent 在等你)> Running(还在跑)> TurnEnded(该你出手了)
/// > Idle > 无存活会话(`None`,不画点)。
///
/// 与 [`agent_state_dot`] 分家是为了让 `Stub` 页签也能用:启动恢复时那些还没
/// 促成的页签手上只有 daemon 的 `SessionInfo` 列表,没有 `Workspace`,但"哪个
/// 状态优先"这条规则必须与 `Loaded` 页签**完全一致**,不能各写一份
/// (最终审查 Required Fix #5)。
fn winning_agent_state(alive_states: &[AgentState]) -> Option<AgentState> {
    [
        AgentState::AwaitingInput,
        AgentState::Running,
        AgentState::TurnEnded,
        AgentState::Idle,
    ]
    .into_iter()
    .find(|candidate| alive_states.contains(candidate))
}

/// 胜出状态 → 颜色。不另造一套表,直接问既有 `dot_color`——页签点与 tab
/// 点讲的是同一种语言,两份颜色表迟早会漂。
fn agent_state_dot(state: AgentState) -> Color {
    dot_color(state, true)
}

/// 启动恢复时给每个 `Stub` 页签算后台活动状态:从 daemon 一次性吐出的全量
/// 会话列表里,挑出属于该项目的**存活**会话,套用与 `Loaded` 页签相同的优先级。
///
/// 为什么非要有这一步:重启后除了上次聚焦的那一个,**所有**页签都是 `Stub`,
/// 而 `Stub` 手上没有会话列表 → 指示点恒为空。也就是说"切走了还想知道另一个
/// 项目有没有在动"这个整套功能存在的核心理由(设计文档 §6),在最常见的
/// "刚打开 app"场景下完全不工作。这里只额外花一次 `list()` 往返、不促成任何
/// `Workspace`,懒加载照旧(最终审查 Required Fix #5)。
fn stub_activity(sessions: &[SessionInfo], project_id: i64) -> Option<Color> {
    let alive: Vec<(AgentState, AgentKind)> = sessions
        .iter()
        .filter(|s| s.alive && s.project_id == Some(project_id))
        .map(|s| (s.agent_state, s.agent))
        .collect();
    project_dot(&alive)
}

/// 面板区里某块 pane 在外框圆角处要收圆的外角:`Left`/`Right` 配对视图里
/// 左 pane 收左侧、右 pane 收右侧;`All` 是 Web 单 pane 收全部四角;`None`
/// 不收(放大态下 pane 直接撑满放大盒子,外框由金色浮层负责,方角才对)。
#[derive(Clone, Copy)]
pub(crate) enum PaneCorner {
    None,
    Left,
    Right,
    All,
}

/// 把 `left_zone`/`right_zone` 的圆角背景"透"到内部 pane 上:iced 的
/// `Container::clip(true)` 只把子元素裁成**矩形**,裁不出圆角,所以 pane
/// 自己的方角会戳出 zone 的圆角 CARD 背景,在四角形成小尖角。让 pane 的外
/// 圆角跟随 zone 圆角(半径减掉 zone 内边距),方角就被收进圆角里,只在外
/// 侧那一边收(`corner` 决定),配对的内部接缝仍是方角(本来就藏在 zone 内)。
pub(crate) fn zone_pane_border(zone: theme::region::RegionStyle, corner: PaneCorner) -> Border {
    let r = zone.border.map(|b| b.radius.top_left).unwrap_or(0.0);
    let r = (r - zone.padding.top).max(0.0);
    let radius = match corner {
        PaneCorner::None => Radius::from(0.0),
        PaneCorner::All => Radius::from(r),
        PaneCorner::Left => Radius {
            top_left: r,
            bottom_left: r,
            ..Radius::from(0.0)
        },
        PaneCorner::Right => Radius {
            top_right: r,
            bottom_right: r,
            ..Radius::from(0.0)
        },
    };
    Border {
        color: Color::TRANSPARENT,
        width: 0.0,
        radius,
    }
}

/// 渲染"某一种面板"的内容——`left_panel_area`/`right_panel_area` 共用。
///
/// Stage 4a 给图标栏面板加了跨栏拖拽后,`left_view` 可以是原先挂右栏的
/// `Agent`/`Conversations`/`Usage`/`Acceptance`,`right_view` 也可以是原先
/// 挂左栏的 `Files`/`GitLog`/`Todo`/`Project`/`Database`/`Ssh`/`Web`——
/// 之前两侧各自 `match` 里那行 `_ => unreachable!("Stage 1 ... 面板还固定
/// 在各自原侧")` 已经不成立,再碰到跨栏后的对侧面板会在渲染期直接 abort
/// (GUI 拖拽核对抓到的崩溃)。
///
/// 所以把"渲染一个面板"抽到这里做穷尽 `match`。每个分支只依赖
/// `zone`(外框主题与内部分割线配色)和 `lc/rc/ac`(pane 圆角朝向),由两侧
/// 各自传入自己那一侧的主题——除此之外同一面板在左/右栏渲染完全一致
/// (内部分割线用的 `Divider` variant 是面板固有属性,和挂哪条栏无关;
/// `panel_mirrored(kind)` 已经按当前实际所在栏算出是否镜像、自行翻转
/// `row!` 顺序)。各面板分割线的几何(`apply_column_drag`)已经由
/// Task 5/6 做成 side+镜像感知,这里只需正确渲染,无需再按左/右分支。
fn panel_body<'a>(
    app: &'a App,
    ws: &'a Workspace,
    kind: PanelKind,
    zone: theme::region::RegionStyle,
    lc: PaneCorner,
    rc: PaneCorner,
    ac: PaneCorner,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    match kind {
        PanelKind::Files => {
            let preview = preview_pane(app, ws, Length::Fill, zone_pane_border(zone, rc));
            if app.dims.files_tree_collapsed {
                // 收起文件树:整个配对宽度都交给预览,项目树列表与分隔线都不
                // 渲染。`files_split` 比例保留,展开时按原比例恢复。
                return preview;
            }
            let (list_portion, content_portion) = split_portions(app.dims.files_split);
            let list_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
                if ws.project.is_some() {
                    files::view(
                        &ws.files,
                        Length::FillPortion(list_portion),
                        zone_pane_border(zone, lc),
                        app.hover_progress(HoverId::FilesSearchSubmit),
                        app.hover_progress(HoverId::FilesDotfiles),
                        app.hover_progress(HoverId::FilesBranchSwitch),
                    )
                    .map(Message::Files)
                } else {
                    no_project_placeholder(
                        ws,
                        Length::FillPortion(list_portion),
                        zone_pane_border(zone, lc),
                    )
                };
            let preview = preview_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            );
            let list_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let preview_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Files) {
                row![
                    preview,
                    divider_bar(
                        Divider::LeftPairSplit,
                        preview_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::LeftPairSplit),
                    ),
                    list_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    list_pane,
                    divider_bar(
                        Divider::LeftPairSplit,
                        list_bg,
                        preview_bg,
                        Message::ColumnDragStart(Divider::LeftPairSplit),
                    ),
                    preview,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::GitLog => git_log::view(
            app,
            &app.git_log,
            app.dims.git_log_split,
            app.dims.git_log_file_diff_split,
            app.panel_mirrored(PanelKind::GitLog),
        )
        .map(Message::GitLog),
        PanelKind::Todo => {
            let collapsed = app.list_collapsed(PanelKind::Todo);
            // 列表列收起:内容拿满整个配对宽度,侧栏不渲染。
            if collapsed {
                return todo::view(
                    app,
                    &ws.todo,
                    ws,
                    Length::Fixed(0.0),
                    Border::default(),
                    Length::Fill,
                    zone_pane_border(zone, rc),
                )
                .1
                .map(Message::Todo);
            }
            let (list_portion, content_portion) = split_portions(app.dims.todo_split);
            let (sidebar_pane, content_pane) = todo::view(
                app,
                &ws.todo,
                ws,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, lc),
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            );
            let sidebar = sidebar_pane.map(Message::Todo);
            let content = content_pane.map(Message::Todo);
            let sidebar_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let content_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Todo) {
                row![
                    content,
                    divider_bar(
                        Divider::TodoSplit,
                        content_bg,
                        sidebar_bg,
                        Message::ColumnDragStart(Divider::TodoSplit),
                    ),
                    sidebar,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    sidebar,
                    divider_bar(
                        Divider::TodoSplit,
                        sidebar_bg,
                        content_bg,
                        Message::ColumnDragStart(Divider::TodoSplit),
                    ),
                    content,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Project => {
            if app.list_collapsed(PanelKind::Project) {
                return project_preview_pane(app, ws, Length::Fill, zone_pane_border(zone, rc));
            }
            let (list_portion, content_portion) = split_portions(app.dims.project_split);
            let info_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
                project::view(
                    &ws.project_panel,
                    ws.project.as_ref(),
                    Length::FillPortion(list_portion),
                    zone_pane_border(zone, lc),
                )
                .map(Message::Project);
            let preview = project_preview_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            );
            let info_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let preview_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Project) {
                row![
                    preview,
                    divider_bar(
                        Divider::ProjectSplit,
                        preview_bg,
                        info_bg,
                        Message::ColumnDragStart(Divider::ProjectSplit),
                    ),
                    info_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    info_pane,
                    divider_bar(
                        Divider::ProjectSplit,
                        info_bg,
                        preview_bg,
                        Message::ColumnDragStart(Divider::ProjectSplit),
                    ),
                    preview,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Database => {
            // 数据库面板需要项目已打开才能读写 `.dozer/database.json`。
            if ws.project.is_none() {
                return column![].into();
            }
            if app.list_collapsed(PanelKind::Database) {
                return database::content_pane(
                    app,
                    &ws.database,
                    Length::Fill,
                    zone_pane_border(zone, rc),
                )
                .map(Message::Database);
            }
            let (list_portion, content_portion) = split_portions(app.dims.database_split);
            let list_pane = database::view(
                &app.database,
                &ws.database,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, lc),
            )
            .map(Message::Database);
            let content_pane = database::content_pane(
                app,
                &ws.database,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            )
            .map(Message::Database);
            let list_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let content_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Database) {
                row![
                    content_pane,
                    divider_bar(
                        Divider::DatabaseSplit,
                        content_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::DatabaseSplit),
                    ),
                    list_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    list_pane,
                    divider_bar(
                        Divider::DatabaseSplit,
                        list_bg,
                        content_bg,
                        Message::ColumnDragStart(Divider::DatabaseSplit),
                    ),
                    content_pane,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Ssh => {
            // 同 Files/Database 面板:`ws.project.is_none()` 是 Stub→Loaded
            // 促成期间的占位态。
            if ws.project.is_none() {
                return column![].into();
            }
            if app.list_collapsed(PanelKind::Ssh) {
                return ssh_terminal_pane(app, ws, Length::Fill, zone_pane_border(zone, rc));
            }
            let (list_portion, content_portion) = split_portions(app.dims.ssh_split);
            let list_pane = ssh::view(
                app,
                &ws.ssh,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, lc),
            )
            .map(Message::Ssh);
            let terminal = ssh_terminal_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            );
            let list_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let terminal_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Ssh) {
                row![
                    terminal,
                    divider_bar(
                        Divider::SshSplit,
                        terminal_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::SshSplit),
                    ),
                    list_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    list_pane,
                    divider_bar(
                        Divider::SshSplit,
                        list_bg,
                        terminal_bg,
                        Message::ColumnDragStart(Divider::SshSplit),
                    ),
                    terminal,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Web => browser::view(
            &ws.browser,
            ws.project.as_ref().map(|p| p.id),
            app.dims.browser_bookmarks_split,
            Length::Fill,
            zone_pane_border(zone, ac),
            app.panel_mirrored(PanelKind::Web),
        )
        .map(Message::Browser),
        PanelKind::Agent => {
            if app.list_collapsed(PanelKind::Agent) {
                return terminal::terminal_pane(app, ws, Length::Fill, zone_pane_border(zone, lc));
            }
            let (list_portion, content_portion) = split_portions(app.dims.agent_split);
            let terminal = terminal::terminal_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, lc),
            );
            let list = agent_list_pane(
                app,
                ws,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, rc),
            );
            let terminal_bg = theme::region::terminal_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let list_bg = theme::region::agent_list_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Agent) {
                row![
                    list,
                    divider_bar(
                        Divider::RightPairSplit,
                        list_bg,
                        terminal_bg,
                        Message::ColumnDragStart(Divider::RightPairSplit),
                    ),
                    terminal,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    terminal,
                    divider_bar(
                        Divider::RightPairSplit,
                        terminal_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::RightPairSplit),
                    ),
                    list,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Conversations => {
            if app.list_collapsed(PanelKind::Conversations) {
                return review_content_pane(app, ws, Length::Fill, zone_pane_border(zone, lc));
            }
            let (list_portion, content_portion) = split_portions(app.dims.conversations_split);
            let review = review_content_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, lc),
            );
            let list = conversation_list_pane(
                app,
                ws,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, rc),
            );
            let review_bg = theme::region::review_content_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let list_bg = theme::region::conversation_list_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Conversations) {
                row![
                    list,
                    divider_bar(
                        Divider::RightPairSplit,
                        list_bg,
                        review_bg,
                        Message::ColumnDragStart(Divider::RightPairSplit),
                    ),
                    review,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    review,
                    divider_bar(
                        Divider::RightPairSplit,
                        review_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::RightPairSplit),
                    ),
                    list,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Usage => {
            // 加载中/还没数据时没有 agent 筛选栏可拼(同改造前
            // `sidebar: Option<..>` 为 `None` 时的行为),内容侧独占全宽。
            if !ws.usage.has_agent_filter() {
                return usage::content_pane(
                    app,
                    &ws.usage,
                    Length::Fill,
                    zone_pane_border(zone, ac),
                )
                .map(Message::Usage);
            }
            if app.list_collapsed(PanelKind::Usage) {
                return usage::content_pane(
                    app,
                    &ws.usage,
                    Length::Fill,
                    zone_pane_border(zone, lc),
                )
                .map(Message::Usage);
            }
            let (list_portion, content_portion) = split_portions(app.dims.usage_split);
            let content_pane = usage::content_pane(
                app,
                &ws.usage,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, lc),
            )
            .map(Message::Usage);
            let list_pane = usage::list_pane(
                &ws.usage,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, rc),
            )
            .map(Message::Usage);
            let content_bg = byteui::theme::color::current().panel;
            let list_bg = byteui::theme::color::current().bg;
            if app.panel_mirrored(PanelKind::Usage) {
                row![
                    list_pane,
                    divider_bar(
                        Divider::UsageSplit,
                        list_bg,
                        content_bg,
                        Message::ColumnDragStart(Divider::UsageSplit),
                    ),
                    content_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    content_pane,
                    divider_bar(
                        Divider::UsageSplit,
                        content_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::UsageSplit),
                    ),
                    list_pane,
                ]
                .width(Length::Fill)
                .into()
            }
        }
    }
}

/// 左面板区:按当前左视图组合"项目树+文件预览"配对或单个 Web 预览面板;
/// 收起时渲染成空元素(不占宽度)。
///
/// 宽度语义与 `left_zone_width` 严格对应:对侧收起时本区 `Fill` 独占
/// `zones_width`(否则整行会缩到"两条图标栏+一条分隔线"那么宽,右图标栏
/// 跑到窗口中间去);两侧都收起时由本区出一个 `Fill` 空白把窗口撑满;
/// 常规态用 `Workspace::effective_left_width()`——**不是**直接读持久化的
/// `dims.left_width`。持久化宽可能大过当前窗口容得下的范围(用户
/// 在大窗口拖宽后把窗口缩小),那样这条 `Length::Fixed` 会在 flex 第一趟
/// 把可用空间吃光,唯一 `Fill` 的右面板区拿到 0 宽(Fix round 2 Critical #1)。
///
/// `maximized`(放大态用):为 `true` 时强制 `Length::Fill`,不看持久化宽/
/// 对侧收起态——`maximize_overlay` 需要这块区域真正撑满整个放大盒子,而不是
/// 停在平时拖拽出来的 `left_width` 那么宽。放大态下 `left_collapsed` 仍可能
/// 为真(旧注释断言"恒为 false"是错的:放大浮层不拦图标栏点击,先放大再点
/// 图标收起本侧是可达路径),此时上面那条收起分支返回空元素;`maximized`
/// 会被 `PanelSelect` 无条件清掉,所以这个组合不会
/// 停留超过一帧(Fix round 2 #2)。
/// 非放大态下,左1(项目树/Web)+左2(预览)两栏被视觉框成一个整体,套
/// `theme::region::left_zone()` 的外框(四向 margin 做悬浮留白,无描边)。
/// 放大态跳过——`maximize_overlay` 已经用金色边框把同一块内容整体框起来,
/// 再套一层外框会在金框内侧多出一圈视觉噪音。
fn left_panel_area<'a>(
    app: &'a App,
    ws: &'a Workspace,
    maximized: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if app.left_collapsed {
        return if app.right_collapsed {
            iced_widget::space::horizontal().into()
        } else {
            column![].into()
        };
    }
    let total = if maximized || app.right_collapsed {
        Length::Fill
    } else {
        Length::Fixed(app.effective_left_width())
    };
    let zone = theme::region::left_zone();
    let (lc, rc, ac) = if maximized {
        (PaneCorner::None, PaneCorner::None, PaneCorner::None)
    } else {
        (PaneCorner::Left, PaneCorner::Right, PaneCorner::All)
    };
    let inner = panel_body(app, ws, app.left_view, zone, lc, rc, ac);
    if maximized {
        return inner;
    }
    let region = zone;
    // 其它 worktree 切换条已从 Git Log 面板移除(用户需求),这里不再包任何
    // 额外层,直接透传面板本体。
    let zone_body: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> = inner;
    // 聚焦态外框:本 zone 拿到焦点(= `active_zone`)时描 GOLD 边,否则沿用
    // region 的默认(无描边)外框。半径保持与默认外框一致。
    let left_focused = app.active_zone == Some(ZoneSide::Left);
    let base_border = region.border.unwrap_or_default();
    let zone_box = container(zone_body)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(region.padding)
        .clip(true)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: if left_focused {
                Border {
                    color: byteui::theme::color::current().gold,
                    width: 2.0,
                    radius: base_border.radius,
                }
            } else {
                base_border
            },
            ..container::Style::default()
        });
    // 四向 margin:把整块外边框从顶栏/窗口底/图标栏/对侧分隔条各推开一段,
    // 做出悬浮留白。左右 margin 来自 `left_zone` 配置(默认左 8、右 0)。
    let m = region.margin;
    container(zone_box)
        .width(total)
        .height(Length::Fill)
        .padding(Padding {
            top: m.top,
            right: m.right,
            bottom: m.bottom,
            left: m.left,
        })
        .into()
}

/// 右面板区:按当前右视图组合"Agent 列表+终端"或"对话列表+对话审阅"配对;
/// 收起时渲染成空元素。总宽恒为剩余空间(`Fill`),不像左面板区那样有持久化
/// 的固定像素宽——所以内部分割只能用 `FillPortion` 表达,不能预先算像素。
///
/// 两块 pane 的宽度直接由它们自己的外层容器声明成 `FillPortion`,不再套一层
/// 包装容器:`Limits::width(Fixed(w))` 会把子元素的 min/max 都钉成 `w`,父级
/// 的 `FillPortion` 只约束包装容器本身、传不进子元素,曾导致这四块 pane 全部
/// 以 0 宽布局(右半边整片空白)。
///
/// 非放大态下,右1(Agent 列表/对话列表)+右2(终端/审阅)两栏被视觉框成
/// 一个整体,套 `theme::region::right_zone()` 的外框(四向 margin 做悬浮留白,
/// 无描边),`maximized` 时跳过(理由同 `left_panel_area`)。
fn right_panel_area<'a>(
    app: &'a App,
    ws: &'a Workspace,
    maximized: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if app.right_collapsed {
        return column![].into();
    }
    // 两个配对都是"内容侧渲染在左、列表侧渲染在右"——终端在左/Agent 列表
    // 在右,审阅在左/对话列表在右。`agent_split`/`conversations_split` 仍是
    // "列表侧(Agent 列表/对话列表)占右面板区宽度的比例"这个原有语义不变
    // (`terminal_pane_pixel_size` 等既有几何公式全靠它,不能跟着挪);只是
    // `content_portion`(∝ 1-split)现在给左边那块、`list_portion`(∝ split)
    // 给右边那块,单纯是 `row!` 里两个 pane 的先后顺序换了。`apply_column_drag`
    // 的 `RightPairSplit` 分支要相应把算出来的 ratio 取反再写回,否则拖拽
    // 方向感会反过来(见该函数注释)。
    let zone = theme::region::right_zone();
    let (lc, rc, ac) = if maximized {
        (PaneCorner::None, PaneCorner::None, PaneCorner::None)
    } else {
        (PaneCorner::Left, PaneCorner::Right, PaneCorner::All)
    };
    let inner = panel_body(app, ws, app.right_view, zone, lc, rc, ac);
    if maximized {
        return inner;
    }
    let region = zone;
    // 聚焦态外框:本 zone 拿到焦点(= `active_zone`)时描 GOLD 边,否则沿用
    // region 的默认(无描边)外框。半径保持与默认外框一致。
    let right_focused = app.active_zone == Some(ZoneSide::Right);
    let base_border = region.border.unwrap_or_default();
    let zone_box = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(region.padding)
        .clip(true)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: if right_focused {
                Border {
                    color: byteui::theme::color::current().gold,
                    width: 2.0,
                    radius: base_border.radius,
                }
            } else {
                base_border
            },
            ..container::Style::default()
        });
    // 四向 margin:同 `left_panel_area`,左右 margin 来自 `right_zone` 配置
    // (默认左 0、右 8)。
    let m = region.margin;
    container(zone_box)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: m.top,
            right: m.right,
            bottom: m.bottom,
            left: m.left,
        })
        .into()
}

/// 放大态浮层:两条图标栏之间的整个内容区变暗+背景虚化，放大的那一侧
/// 内容(左/右面板区，含其内部列表:内容子分隔线，原样渲染，只是占满整个
/// 中间区域)金色描边突出。点变暗区域(放大内容之外的部分)退出放大。
fn maximize_overlay<'a>(
    app: &'a App,
    ws: &'a Workspace,
    which: MaximizedPane,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let inner = match which {
        MaximizedPane::Left => left_panel_area(app, ws, true),
        MaximizedPane::Right => right_panel_area(app, ws, true),
    };
    // `bordered` 显式给 `Length::Fill`(不留给默认 `Length::Shrink`)——
    // iced 0.14 的 `Limits` 有个"compression"传染机制:一个 `Shrink` 容器
    // 包一个 `Fill`/`FillPortion` 子元素时,子元素的 Fill 不会展开到可用
    // 空间,而是退化成"贴着内容收缩"(`Limits::resolve` 对 Fill 的展开分支
    // 要求 `!compression`,`Shrink` 会把 compression 设 true 并一路往下传,
    // 直到遇到一个显式 `Length::Fixed` 才重置)。这里如果不显式给 Fill,
    // `inner`(`left_panel_area`/`right_panel_area` 内部大量 FillPortion
    // 组成)会整体收缩成远小于放大盒子的intrinsic 尺寸,金色描边就会贴着
    // 一小块内容而不是撑满两条图标栏之间的放大区域。
    let overlay_style = theme::region::maximize_overlay();
    let bordered = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            border: overlay_style.border,
            ..container::Style::default()
        });
    // 放大内容自己再罩一层"吃掉点击/滚轮"的 MouseArea,拦住它们冒泡到外层
    // dim 遮罩的 `MaximizeClose`(Fix round 2 #4)。iced 的事件是子先父后:
    // 内容里真正可交互的控件(按钮、终端 canvas)会先自己 capture,压根到不了
    // 这一层;到得了这一层的正是"不消费点击的地方"——审阅正文、卡片下方空白、
    // 列表空处——此前它们会一路穿到 dim 遮罩上,点一下正文就把放大退掉,而
    // 放大审阅恰恰是这个功能存在的理由。滚轮同理:内部 scrollable 真滚动了
    // 会自己 capture,滚不动时旧行为是穿到下层(基础层那块被遮住的终端
    // canvas)去滚终端历史,这里一并吃掉。
    let content_guard = MouseArea::new(bordered)
        .on_press(Message::Noop)
        .on_scroll(|_delta| Message::Noop);
    let dim_bg = MouseArea::new(
        container(content_guard)
            .padding(overlay_style.scrim_padding)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(overlay_style.scrim_background.into()),
                ..container::Style::default()
            }),
    )
    .on_press(Message::MaximizeClose);

    // 顶部垫一条透明的 `byteui::theme::geometry::top_bar_height()` 高 Space,把变暗遮罩钉在顶栏
    // 之下——`base = column![top, body]` 里顶栏和内容区就是这么分的,
    // 这里镜像同一结构,让变暗区域精确对齐 `body` 的渲染范围,不覆盖顶栏
    // (Important:此前没有这条 Space,遮罩会盖住整个窗口高度,连顶栏的
    // 项目 tab 等控件都会被染黑)。
    column![
        iced_widget::space::Space::new()
            .height(Length::Fixed(byteui::theme::geometry::top_bar_height())),
        row![
            iced_widget::space::Space::new()
                .width(Length::Fixed(byteui::theme::geometry::icon_rail_width())),
            dim_bg,
            iced_widget::space::Space::new()
                .width(Length::Fixed(byteui::theme::geometry::icon_rail_width())),
        ],
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// 分隔线:命中区 `byteui::theme::geometry::divider_width()` 宽、`Length::Fill` 高,
/// 中间一条 2px BORDER 竖线。悬停变 resize 光标走 `MouseArea::interaction` →
/// iced 既有的 `mouse_interaction` → `window.set_cursor` 管线(main.rs:808-816
/// 已有),不必另起一套光标代码。`on_press` 只发起拖拽状态,不指望 `MouseArea`
/// 的 `on_move`/`on_release`——它们要求光标不离开这条窄带才触发,快速拖拽会
/// 在光标移出后"断掉";持续追踪交给 `main.rs` 原始事件层。
///
/// 配对视图(左1左2 / 右1右2)内部:`left_bg`/`right_bg` 是分隔线两侧紧贴的
/// pane 底色。命中区左右两半(各 `(divider_width-2)/2`)分别填上这两色,只留
/// 中间 2px BORDER 竖线——否则 8px 命中区是透明的,会露出 zone 的 CARD 底色,
/// 在两块 pane 之间顶出一条浅色"沟",看起来像多了 padding/margin。填色后两块
/// pane 视觉贴合、只剩一条分割线,命中区宽度(拖拽手感)不变。
///
/// `Divider::LeftRight` 不画那条 2px 竖线、也不填色——它两侧各自套了
/// `theme::region::left_zone()`/`right_zone()` 的整体外框,这条 8px 缝是故意
/// 空出来给两侧 zone 圆角边框各自收边的,不能填成某侧 pane 色。
pub(crate) fn divider_bar<'a, M: Clone + 'a>(
    divider: Divider,
    left_bg: Color,
    right_bg: Color,
    on_drag: M,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let show_line = !matches!(divider, Divider::LeftRight);
    let body: Element<'_, M, iced_widget::Theme, iced_renderer::Renderer> = if !show_line {
        iced_widget::Space::new()
            .width(Length::Fixed(byteui::theme::geometry::divider_width()))
            .height(Length::Fill)
            .into()
    } else {
        let line_w = 2.0_f32;
        let side_w = (byteui::theme::geometry::divider_width() - line_w) / 2.0;
        let left_side = container(iced_widget::Space::new())
            .width(Length::Fixed(side_w))
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(left_bg.into()),
                ..container::Style::default()
            });
        let right_side = container(iced_widget::Space::new())
            .width(Length::Fixed(side_w))
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(right_bg.into()),
                ..container::Style::default()
            });
        let line = container(iced_widget::Space::new())
            .width(Length::Fixed(line_w))
            .height(Length::Fill)
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().border.into()),
                ..container::Style::default()
            });
        row![left_side, line, right_side]
            .width(Length::Fixed(byteui::theme::geometry::divider_width()))
            .height(Length::Fill)
            .into()
    };
    MouseArea::new(body)
        .interaction(mouse::Interaction::ResizingColumn)
        .on_press(on_drag)
        .into()
}

/// `divider_bar` 的纵向(上下)镜像:一条水平分割线,`row!`→`column!`、
/// `width`↔`height` 互换,鼠标样式 `ResizingRow`(对应横向的
/// `ResizingColumn`)。目前只有 Git Log 面板右侧"文件列表 | diff 内容"这条
/// 纵向拖拽线用它。粗细复用 `byteui::theme::geometry::divider_width()`,与横向一致。
pub(crate) fn horizontal_divider_bar<'a, M: Clone + 'a>(
    top_bg: Color,
    bottom_bg: Color,
    on_drag: M,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let line_h = 2.0_f32;
    let side_h = (byteui::theme::geometry::divider_width() - line_h) / 2.0;
    let top_side = container(iced_widget::Space::new())
        .height(Length::Fixed(side_h))
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(top_bg.into()),
            ..container::Style::default()
        });
    let bottom_side = container(iced_widget::Space::new())
        .height(Length::Fixed(side_h))
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(bottom_bg.into()),
            ..container::Style::default()
        });
    let line = container(iced_widget::Space::new())
        .height(Length::Fixed(line_h))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..container::Style::default()
        });
    let col = column![top_side, line, bottom_side]
        .height(Length::Fixed(byteui::theme::geometry::divider_width()))
        .width(Length::Fill);
    MouseArea::new(col)
        .interaction(mouse::Interaction::ResizingRow)
        .on_press(on_drag)
        .into()
}

/// tab 栏下方的 1px 分割线。
pub(crate) fn tab_divider<'a, M: 'a>() -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>
{
    byteui::layout::divider::horizontal()
}

/// 受控 tooltip:iced 0.14 的 `Tooltip` 没有"延迟显示"开关(它一悬停就弹),
/// 所以这里不靠 `Tooltip` 自带的 hover 检测,而是**仅在 `show` 为真时才把
/// `content` 包进 `Tooltip`**——调用方按"悬停满 2s"算好 `show`(见
/// `App::hover_tooltip_ready` / `browser::State::hover_tooltip_ready`),满 2s
/// 那一刻视图层才挂载 `Tooltip`,气泡随即弹出;离开即 `show` 为假,直接返回
/// 裸 `content`,气泡消失。`position` 由调用方按页签位置定(顶栏页签用
/// `Bottom`、底部面板页签用 `Top`,免得气泡出屏)。`label` 收 `String`(拥有
/// 所有权),使气泡 `Element` 寿命不受调用方局部借用牵制,`Tooltip` 才能正常
/// 把它当 overlay 渲染。
pub(crate) fn controlled_tooltip<'a, M, R>(
    content: Element<'a, M, iced_widget::Theme, R>,
    label: String,
    position: Position,
    show: bool,
) -> Element<'a, M, iced_widget::Theme, R>
where
    M: Clone + 'a,
    R: iced_widget::core::text::Renderer + 'a,
{
    // 未悬停满 2s:不包 tooltip,直接返回裸内容,避免一悬停就弹气泡打扰。
    if !show {
        return content;
    }
    let bubble = container(
        text(label)
            .size(12)
            .color(byteui::theme::color::current().cream),
    )
    .padding([5, 9]);
    Tooltip::new(content, bubble, position)
        .gap(4)
        .style(icons::tooltip_bubble_style())
        .into()
}

/// `host_id` → `HoverId::SshTab{Item,Close}` 用的哈希键(`HoverId` 整体
/// `derive(Copy)`,`String` 不是 `Copy`,退化成 `u64`,不要求无碰撞——
/// 碰撞在同一台主机的 tab hover 高亮场景下不构成实际风险)。
pub(crate) fn ssh_tab_hover_key(host_id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    host_id.hash(&mut hasher);
    hasher.finish()
}

/// SSH 面板自己的 tab 条:固定一个"空白"占位 tab 打头,后面遍历
/// `ws.ssh_tabs`/`ws.sftp_tabs`,每个渲染一个可关闭 tab。直接复用
/// `panel_tab`(右侧共享终端条 `tab_item` 用的同一个函数)而不是自己拼
/// 容器样式,视觉/hover 动画与全应用其它 tab 完全一致——不需要
/// `tabs::tab_core` 手动接线。前缀图标固定用 `IconKind::Terminal`(阶段
/// 4 只有这一种;阶段 3 加 `Sftp` 变体后按 tab 的种类换图标,`SessionTab`
/// 本身不带 `SshTabKind` 字段,种类信息只在 `ws.ssh_active` 里——阶段 4
/// 全部 `ssh_tabs` 里的 tab 都是 `Terminal` 种类,这里暂时不需要按 tab
/// 查种类,阶段 3 扩展这个函数时才需要处理"同一个 host_id 可能对应两个
/// 不同种类的 tab,要分别渲染两个 tab 条目"这件事)。
///
/// 翻页箭头 + 窗口化裁剪(P1L T5 那套 `tab_window` 索引窗口)镜像
/// `workspace.rs::preview_pane_for`:先把全部 tab 元素连同估算宽度收进
/// `entries`,再用 `tab_window` 算出可视窗口起点 `first`,只渲染
/// `entries[first..]`,左右箭头到头置灰。原先没有这套窗口化,tab 一多
/// 就会被右侧"收起列表"按钮的 `clip` 直接裁没、连滚动入口都没有。
fn ssh_tab_bar<'a>(
    app: &'a App,
    ws: &'a Workspace,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut entries: Vec<(
        f32,
        Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    )> = Vec::new();
    // "空白"占位 tab:不对应 `ssh_tabs`/`sftp_tabs` 里任何一条记录,选中
    // 态即 `ssh_active == None`(未开任何主机 tab,或关到最后一个后的
    // 默认落点)。跟文件预览面板 `preview.rs::TabKind::Blank` 是同一个
    // 产品概念,但这边没有对应的轻量 tab 数据可插进 `ssh_tabs`,所以只在
    // 这里画一个固定条目,内容侧靠 `ssh_active == None` 分支渲染
    // `ssh_empty_state()`,不需要真的建一个 tab 结构体。用 `""` 当 hover
    // key(真实 host_id 是 UUID,不会是空串,不会撞)。
    let blank_key = ssh_tab_hover_key("");
    let blank_active = ws.ssh_active.is_none();
    entries.push((
        tab_display_width("空白"),
        tab_widget::panel_tab(tab_widget::PanelTabArgs {
            title: "空白".to_string(),
            active: blank_active,
            hover_t: app.hover_progress(HoverId::SshTabItem(blank_key)),
            close_hover_t: app.hover_progress(HoverId::SshTabClose(blank_key)),
            prefix: None,
            suffix: None,
            on_select: Message::Ssh(ssh::Message::SelectBlankTab),
            on_close: Message::Ssh(ssh::Message::SelectBlankTab),
            show_tooltip: app.hover_tooltip_ready(HoverId::SshTabItem(blank_key)),
            title_hover: move |h| Message::Hover(HoverId::SshTabItem(blank_key), h),
            close_hover: move |h| Message::Hover(HoverId::SshTabClose(blank_key), h),
        }),
    ));
    for tab in &ws.ssh_tabs {
        let host_id = tab
            .info
            .id
            .strip_prefix("ssh:")
            .unwrap_or(&tab.info.id)
            .to_string();
        let is_active = ws
            .ssh_active
            .as_ref()
            .is_some_and(|(h, k)| h == &host_id && *k == ssh::SshTabKind::Terminal);
        let key = ssh_tab_hover_key(&host_id);
        let title_hover_t = app.hover_progress(HoverId::SshTabItem(key));
        let close_hover_t = app.hover_progress(HoverId::SshTabClose(key));
        let icon = icons::view(
            icons::IconKind::Terminal,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().dim,
        );
        let select_id = host_id.clone();
        let close_id = host_id.clone();
        let title_hover_id = host_id.clone();
        let close_hover_id = host_id;
        let title = tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name);
        entries.push((
            tab_display_width(&title),
            tab_widget::panel_tab(tab_widget::PanelTabArgs {
                title,
                active: is_active,
                hover_t: title_hover_t,
                close_hover_t,
                prefix: Some(icon),
                suffix: None,
                on_select: Message::Ssh(ssh::Message::SelectSshTab(
                    select_id,
                    ssh::SshTabKind::Terminal,
                )),
                on_close: Message::Ssh(ssh::Message::CloseSshTab(
                    close_id,
                    ssh::SshTabKind::Terminal,
                )),
                show_tooltip: app.hover_tooltip_ready(HoverId::SshTabItem(key)),
                title_hover: move |h| {
                    Message::Hover(HoverId::SshTabItem(ssh_tab_hover_key(&title_hover_id)), h)
                },
                close_hover: move |h| {
                    Message::Hover(HoverId::SshTabClose(ssh_tab_hover_key(&close_hover_id)), h)
                },
            }),
        ));
    }
    // SFTP tab(阶段 3):`sftp_tabs` 按 host_id 去重,渲染形状跟终端 tab
    // 一致(复用 `panel_tab`/`tab_core`),只是图标用 FolderSync、标题用主机名。
    for (host_id, state) in &ws.sftp_tabs {
        let is_active = ws.ssh_active.as_ref() == Some(&(host_id.clone(), ssh::SshTabKind::Sftp));
        let label = ws
            .ssh
            .hosts()
            .iter()
            .find(|h| &h.id == host_id)
            .map(|h| h.name.clone())
            .unwrap_or_else(|| host_id.clone());
        let key = ssh_tab_hover_key(host_id);
        let title_hover_t = app.hover_progress(HoverId::SshTabItem(key));
        let close_hover_t = app.hover_progress(HoverId::SshTabClose(key));
        let icon = icons::view(
            icons::IconKind::FolderSync,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().dim,
        );
        let select_id = host_id.clone();
        let close_id = host_id.clone();
        let title_hover_id = host_id.clone();
        let close_hover_id = host_id.clone();
        entries.push((
            tab_display_width(&label),
            tab_widget::panel_tab(tab_widget::PanelTabArgs {
                title: label,
                active: is_active,
                hover_t: title_hover_t,
                close_hover_t,
                prefix: Some(icon),
                suffix: None,
                on_select: Message::Ssh(ssh::Message::SelectSshTab(
                    select_id,
                    ssh::SshTabKind::Sftp,
                )),
                on_close: Message::Ssh(ssh::Message::CloseSshTab(close_id, ssh::SshTabKind::Sftp)),
                show_tooltip: app.hover_tooltip_ready(HoverId::SshTabItem(key)),
                title_hover: move |h| {
                    Message::Hover(HoverId::SshTabItem(ssh_tab_hover_key(&title_hover_id)), h)
                },
                close_hover: move |h| {
                    Message::Hover(HoverId::SshTabClose(ssh_tab_hover_key(&close_hover_id)), h)
                },
            }),
        ));
        let _ = state;
    }
    let widths: Vec<f32> = entries.iter().map(|(w, _)| *w).collect();
    let window = tab_widget::tab_window(
        &widths,
        4.0,
        byteui::theme::geometry::tab_bar_avail_px(),
        ws.ssh_tab_first,
    );
    let items: Vec<_> = entries
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| (window.first..window.visible_end).contains(idx))
        .map(|(_, (_, el))| el)
        .collect();
    // tab 条本身占 Fill、裁掉右侧溢出,让"收起列表"钉在裁剪区外的最右侧
    // (镜像 `workspace.rs::preview_pane_for` 的 `clipped`/`collapse` 布局
    // ——之前 `bar` 整体是 `Shrink`,收起按钮只是跟在最后一个 tab 后面,
    // tab 少时会贴在中间而不是面板右边缘,验收反馈要求钉死在右侧)。
    let clipped = container(row(items).spacing(4))
        .width(Length::Fill)
        .clip(true);
    let hidden_count = window.hidden_before().len() + window.hidden_after(widths.len()).len();
    let overflow_button = tab_widget::tab_overflow_button(
        hidden_count,
        Message::Ssh(ssh::Message::TabOverflowToggle),
    );
    // 内容侧"收起/展开列表列"按钮(收起左列主机列表后仍在此可见以便恢复)。
    let collapse = app.list_collapse_button(
        PanelKind::Ssh,
        app.list_collapsed(PanelKind::Ssh),
        HoverId::SshListCollapse,
        "收起列表",
        "展开列表",
        Message::TogglePanelListCollapse(PanelKind::Ssh),
        move |hovered| Message::Hover(HoverId::SshListCollapse, hovered),
    );
    let mut tab_bar_row = row![clipped]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some(btn) = overflow_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    let tab_bar = tab_bar_row.push(collapse);

    let base: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = tab_bar.into();
    if let Some(anchor) = ws.ssh_tab_overflow_anchor {
        if window.has_overflow(widths.len()) {
            let mut entries: Vec<tab_widget::TabOverflowEntry<'_, Message>> = Vec::new();
            let hidden: std::collections::HashSet<usize> = window
                .hidden_before()
                .chain(window.hidden_after(widths.len()))
                .collect();
            if hidden.contains(&0) {
                entries.push(tab_widget::TabOverflowEntry {
                    index: 0,
                    prefix: None,
                    title: "空白".to_string(),
                    active: ws.ssh_active.is_none(),
                    closable: false,
                });
            }
            for (i, tab) in ws.ssh_tabs.iter().enumerate() {
                let idx = i + 1;
                if !hidden.contains(&idx) {
                    continue;
                }
                let host_id = tab
                    .info
                    .id
                    .strip_prefix("ssh:")
                    .unwrap_or(&tab.info.id)
                    .to_string();
                entries.push(tab_widget::TabOverflowEntry {
                    index: idx,
                    prefix: Some(icons::view(
                        icons::IconKind::Terminal,
                        byteui::theme::icon_size::row(),
                        byteui::theme::color::current().dim,
                    )),
                    title: tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
                    active: ws
                        .ssh_active
                        .as_ref()
                        .is_some_and(|(h, k)| h == &host_id && *k == ssh::SshTabKind::Terminal),
                    closable: true,
                });
            }
            for (i, (host_id, _)) in ws.sftp_tabs.iter().enumerate() {
                let idx = i + 1 + ws.ssh_tabs.len();
                if !hidden.contains(&idx) {
                    continue;
                }
                let label = ws
                    .ssh
                    .hosts()
                    .iter()
                    .find(|h| &h.id == host_id)
                    .map(|h| h.name.clone())
                    .unwrap_or_else(|| host_id.clone());
                entries.push(tab_widget::TabOverflowEntry {
                    index: idx,
                    prefix: Some(icons::view(
                        icons::IconKind::FolderSync,
                        byteui::theme::icon_size::row(),
                        byteui::theme::color::current().dim,
                    )),
                    title: label,
                    active: ws.ssh_active.as_ref()
                        == Some(&(host_id.clone(), ssh::SshTabKind::Sftp)),
                    closable: true,
                });
            }
            let menu = tab_widget::tab_overflow_menu(tab_widget::TabOverflowMenuArgs {
                entries,
                anchor,
                window_size: app.window_size,
                on_select: |idx| ssh_tab_overflow_select_message(ws, idx),
                on_close: |idx| ssh_tab_overflow_close_message(ws, idx),
                on_dismiss: Message::Ssh(ssh::Message::TabOverflowDismiss),
            });
            return iced_widget::stack![base, menu].into();
        }
    }
    base
}

/// 把溢出下拉的扁平下标翻回 SSH 的 `(host_id, kind)`：下标 0 = 空白占位
/// tab；`1..=ssh_tabs.len()` 是终端段；再往后是 SFTP 段。越界兜底回空白态。
fn ssh_tab_overflow_select_message(ws: &Workspace, idx: usize) -> Message {
    if idx == 0 {
        return Message::Ssh(ssh::Message::SelectBlankTab);
    }
    let terminal_count = ws.ssh_tabs.len();
    if idx <= terminal_count {
        let tab = &ws.ssh_tabs[idx - 1];
        let host_id = tab
            .info
            .id
            .strip_prefix("ssh:")
            .unwrap_or(&tab.info.id)
            .to_string();
        return Message::Ssh(ssh::Message::SelectSshTab(
            host_id,
            ssh::SshTabKind::Terminal,
        ));
    }
    match ws.sftp_tabs.keys().nth(idx - 1 - terminal_count) {
        Some(host_id) => Message::Ssh(ssh::Message::SelectSshTab(
            host_id.clone(),
            ssh::SshTabKind::Sftp,
        )),
        None => Message::Ssh(ssh::Message::SelectBlankTab),
    }
}

/// `ssh_tab_overflow_select_message` 的关闭版。空白占位 `closable: false`
/// 保证下拉里它没有 x，走到这只能是兜底，发顶层 no-op。
fn ssh_tab_overflow_close_message(ws: &Workspace, idx: usize) -> Message {
    if idx == 0 {
        return Message::Noop;
    }
    let terminal_count = ws.ssh_tabs.len();
    if idx <= terminal_count {
        let tab = &ws.ssh_tabs[idx - 1];
        let host_id = tab
            .info
            .id
            .strip_prefix("ssh:")
            .unwrap_or(&tab.info.id)
            .to_string();
        return Message::Ssh(ssh::Message::CloseSshTab(
            host_id,
            ssh::SshTabKind::Terminal,
        ));
    }
    match ws.sftp_tabs.keys().nth(idx - 1 - terminal_count) {
        Some(host_id) => Message::Ssh(ssh::Message::CloseSshTab(
            host_id.clone(),
            ssh::SshTabKind::Sftp,
        )),
        None => Message::Noop,
    }
}

/// SSH 面板内嵌终端区:tab 条 + 终端画布(或空态)。镜像 `preview_pane`/
/// `project_preview_pane` 的既有模式——渲染函数不属于 `extensions::ssh`
/// 模块,因为它要用顶层 `Message` 直接操作 `ws.ssh_tabs`。
fn ssh_terminal_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::terminal_pane();
    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match ws.ssh_active.as_ref() {
            Some((host_id, ssh::SshTabKind::Terminal)) => {
                let tab = ws
                    .ssh_tabs
                    .iter()
                    .find(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()));
                match tab {
                    Some(tab) => {
                        let focused =
                            terminal::keyboard_term_target(app.left_view, app.active_zone)
                                == terminal::TermTarget::SshPanel;
                        term_view::view(
                            &tab.model,
                            focused,
                            terminal::TermTarget::SshPanel,
                            focused.then(|| app.term_ime_preedit()).flatten(),
                            None,
                        )
                    }
                    None => ssh_empty_state(),
                }
            }
            Some((host_id, ssh::SshTabKind::Sftp)) => match ws.sftp_tabs.get(host_id) {
                Some(state) => {
                    ssh::sftp::sftp_pane_view(state).map(|m| Message::Ssh(ssh::Message::Sftp(m)))
                }
                None => ssh_empty_state(),
            },
            None => ssh_empty_state(),
        };
    container(
        column![ssh_tab_bar(app, ws), tab_divider(), body]
            .spacing(region.gap)
            .height(Length::Fill),
    )
    .width(width)
    .padding(region.padding)
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: region.background.map(Into::into),
        border: outer,
        ..container::Style::default()
    })
    .into()
}

/// "空白" tab(`ssh_active == None`)选中时的内容:跟文件预览面板
/// `preview.rs::TabKind::Blank` 是同一套视觉语言——居中放 Dozer 品牌标 +
/// 引导文案。SSH 这边每个真实 tab 都对应一条 PTY/SFTP 连接
/// (`SessionTab`/`SftpTabState`),没有轻量数据能塞进 `ws.ssh_tabs` 去
/// 表示"空白",所以"空白" tab 只在 `ssh_tab_bar()` 里画一个固定条目,
/// 内容侧靠 `ssh_active == None` 这个分支渲染,不是真的建一个 tab 结构体
/// (对照 preview 那边"tab 数据里有一个 `TabKind::Blank` 变体"的做法)。
fn ssh_empty_state<'a>() -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(
        column![
            icons::view(
                icons::IconKind::Dozer,
                72.0,
                byteui::theme::color::current().dim,
            ),
            text("点主机卡片的终端/文件传输图标开始")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(14)
        .align_x(iced_widget::core::alignment::Horizontal::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(iced_widget::core::alignment::Horizontal::Center)
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 光标没动(或只在阈值内小幅抖动)不算越过阈值——普通单击场景,同
    /// `rail::rail_drag_past_threshold` 的对应用例。这是 2026-09-04
    /// 用户反馈"agent tab 偶尔两个同时看起来被选中"的根因防回归测试:
    /// 单击 tab 时按下瞬间到抬起前的亚像素抖动不该被当成一次拖拽换位。
    #[test]
    fn tab_drag_past_threshold_false_when_cursor_has_not_moved() {
        assert!(!tab_drag_past_threshold((100.0, 100.0), (100.0, 100.0)));
        assert!(!tab_drag_past_threshold((100.0, 100.0), (101.0, 100.0)));
    }

    /// 恰好等于阈值(平方比较是 `>` 不是 `>=`)不算越过,严格大于才算。
    #[test]
    fn tab_drag_past_threshold_false_when_exactly_at_threshold() {
        assert!(!tab_drag_past_threshold(
            (0.0, 0.0),
            (TAB_DRAG_CONFIRM_THRESHOLD_PX, 0.0)
        ));
    }

    /// 光标越过阈值(任意方向,这里用纯 x 位移)判定为真的拖拽,真实拖拽
    /// 不受这道阈值影响。
    #[test]
    fn tab_drag_past_threshold_true_once_cursor_moves_past_it() {
        assert!(tab_drag_past_threshold(
            (0.0, 0.0),
            (TAB_DRAG_CONFIRM_THRESHOLD_PX + 1.0, 0.0)
        ));
    }

    /// `tree_drag_past_threshold` 同款三条用例(同 `tab_drag_past_threshold`
    /// 的对应测试)——2026-09 用户实测反馈"点一下目录就报'不能把目录移到
    /// 它自己或其子目录里'"的根因防回归测试:单击时按下瞬间到抬起前的
    /// 亚像素抖动不该被当成一次真实拖拽。
    #[test]
    fn tree_drag_past_threshold_false_when_cursor_has_not_moved() {
        assert!(!tree_drag_past_threshold((100.0, 100.0), (100.0, 100.0)));
        assert!(!tree_drag_past_threshold((100.0, 100.0), (101.0, 100.0)));
    }

    #[test]
    fn tree_drag_past_threshold_false_when_exactly_at_threshold() {
        assert!(!tree_drag_past_threshold(
            (0.0, 0.0),
            (TREE_DRAG_CONFIRM_THRESHOLD_PX, 0.0)
        ));
    }

    #[test]
    fn tree_drag_past_threshold_true_once_cursor_moves_past_it() {
        assert!(tree_drag_past_threshold(
            (0.0, 0.0),
            (TREE_DRAG_CONFIRM_THRESHOLD_PX + 1.0, 0.0)
        ));
    }

    /// 2026-09 用户实测反馈(带日志实锤):快速点两下目录,第二下按下到
    /// 松开之间光标真的位移了 ~32px(trackpad 一次快速点按/移开的正常
    /// 抖动量级,超过了 [`TREE_DRAG_CONFIRM_THRESHOLD_PX`]),被判定为一次
    /// "确认的拖拽"并真的把目录移走了——纯距离阈值挡不住这类快速划动。
    /// 加一道"按住时长"门槛:`tree_drag_held_long_enough` 要求按下到松开
    /// 之间至少过了 [`TREE_DRAG_MIN_HOLD_DURATION`],配合距离阈值(两者都要
    /// 满足)才判定为真实拖拽——真正拖拽文件到目标目录、看着高亮再松手,
    /// 耗时远比一次快速点按长。
    #[test]
    fn tree_drag_held_long_enough_false_for_a_quick_flick() {
        assert!(!tree_drag_held_long_enough(
            std::time::Duration::from_millis(30)
        ));
    }

    #[test]
    fn tree_drag_held_long_enough_false_exactly_at_threshold() {
        assert!(!tree_drag_held_long_enough(TREE_DRAG_MIN_HOLD_DURATION));
    }

    #[test]
    fn tree_drag_held_long_enough_true_once_held_past_threshold() {
        assert!(tree_drag_held_long_enough(
            TREE_DRAG_MIN_HOLD_DURATION + std::time::Duration::from_millis(1)
        ));
    }

    /// 复现验收反馈的核心机制:关 tab 后立刻退出,`event_loop.exit()` 不该
    /// 让在飞的 kill/总结 spawn 任务半路被丢弃——`join_pending_exit_tasks`
    /// 必须真的等它们跑完(而不是像修复前那样直接 drop `JoinHandle` 不管)。
    #[test]
    fn join_pending_exit_tasks_waits_for_spawned_tasks_to_finish() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ran_clone = ran.clone();
        let task = rt.spawn(async move {
            // 模拟一次真实的本地 UDS 往返耗时。
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            ran_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        rt.block_on(join_pending_exit_tasks(
            vec![task],
            std::time::Duration::from_secs(2),
        ));
        assert!(
            ran.load(std::sync::atomic::Ordering::SeqCst),
            "预算充足时必须等 spawn 任务真正跑完,而不是提前放弃"
        );
    }

    /// 超预算的任务(daemon 卡死等极端情况)不能把退出流程无限期挂住——
    /// 放弃等待即可,不要求任务本身被中止。
    #[test]
    fn join_pending_exit_tasks_gives_up_after_budget_without_hanging() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let task = rt.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        });
        let start = std::time::Instant::now();
        rt.block_on(join_pending_exit_tasks(
            vec![task],
            std::time::Duration::from_millis(50),
        ));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "超预算必须尽快放弃等待,不能拖住退出流程"
        );
    }

    /// 没有任何在飞任务时(没关过 tab,或都已跑完)应该是零成本的直接返回。
    #[test]
    fn join_pending_exit_tasks_empty_list_returns_immediately() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let start = std::time::Instant::now();
        rt.block_on(join_pending_exit_tasks(
            Vec::new(),
            std::time::Duration::from_secs(2),
        ));
        assert!(start.elapsed() < std::time::Duration::from_millis(100));
    }

    #[test]
    fn merge_review_entries_append_true_accumulates_instead_of_replacing() {
        let mut existing = vec![ReviewEntry::Human {
            text: "第一页第一条".into(),
        }];
        let new = vec![ReviewEntry::Human {
            text: "第二页第一条".into(),
        }];
        merge_review_entries(&mut existing, new, true);
        assert_eq!(existing.len(), 2, "加载更多应该追加,不该丢掉已有内容");
        assert_eq!(
            existing[0],
            ReviewEntry::Human {
                text: "第一页第一条".into()
            }
        );
        assert_eq!(
            existing[1],
            ReviewEntry::Human {
                text: "第二页第一条".into()
            }
        );
    }

    #[test]
    fn merge_review_entries_append_false_replaces() {
        let mut existing = vec![ReviewEntry::Human {
            text: "旧内容".into(),
        }];
        let new = vec![ReviewEntry::Human {
            text: "首次加载的新内容".into(),
        }];
        merge_review_entries(&mut existing, new, false);
        assert_eq!(existing.len(), 1);
        assert_eq!(
            existing[0],
            ReviewEntry::Human {
                text: "首次加载的新内容".into()
            }
        );
    }

    fn loaded_slot(marker: &str) -> WorkspaceSlot {
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.files.set_tree_error(Some(marker.to_string()));
        WorkspaceSlot::Loaded(Box::new(ws))
    }

    fn slot_marker(slot: Option<&WorkspaceSlot>) -> Option<String> {
        match slot {
            Some(WorkspaceSlot::Loaded(ws)) => ws.files.tree_error().map(String::from),
            _ => None,
        }
    }

    fn project_info(id: i64) -> ProjectInfo {
        ProjectInfo {
            id,
            path: format!("/tmp/p{id}"),
            name: format!("p{id}"),
            last_active_ms: 0,
            created_ms: 0,
            updated_ms: 0,
        }
    }

    fn stub_slot(id: i64) -> WorkspaceSlot {
        WorkspaceSlot::Stub {
            info: project_info(id),
            activity: None,
        }
    }

    /// 切页签的核心不变式:只改"当前是哪个页签",两个槽位的内容一个字节都
    /// 不动。设计文档 §2 的"切换永不销毁状态,只有关闭才销毁"整条就落在这
    /// 一个函数上——它若开始改写槽位内容,用户点一下别的页签就丢了工作现场。
    #[test]
    fn focus_project_tab_never_touches_slot_contents() {
        let mut projects = HashMap::new();
        projects.insert(1, loaded_slot("A"));
        projects.insert(2, loaded_slot("B"));
        let order = vec![1, 2];
        let mut active = Some(1);

        assert!(focus_project_tab(&projects, &mut active, 2));

        assert_eq!(active, Some(2));
        assert_eq!(slot_marker(projects.get(&1)).as_deref(), Some("A"));
        assert_eq!(slot_marker(projects.get(&2)).as_deref(), Some("B"));
        assert_eq!(order, vec![1, 2], "切页签不重排页签顺序");
        assert_eq!(projects.len(), 2, "切页签不新增/不删除槽位");
    }

    /// 点一个不存在的页签(脏顺序表/竞态)时原地放弃,不把 `active` 指到一个
    /// 没有槽位的 id 上——那会让 `active_workspace()` 恒为 `None`,界面空白。
    #[test]
    fn focus_project_tab_rejects_unknown_id() {
        let mut projects = HashMap::new();
        projects.insert(1, loaded_slot("A"));
        let mut active = Some(1);

        assert!(!focus_project_tab(&projects, &mut active, 42));
        assert_eq!(active, Some(1));
    }

    /// 关页签后焦点落到右邻;没有右邻取左邻;关光了就回"没有任何项目"。
    #[test]
    fn next_active_after_close_prefers_right_then_left() {
        assert_eq!(next_active_after_close(&[1, 2, 3], 2), Some(3), "右邻优先");
        assert_eq!(
            next_active_after_close(&[1, 2, 3], 3),
            Some(2),
            "末页签取左邻"
        );
        assert_eq!(next_active_after_close(&[1, 2, 3], 1), Some(2));
        assert_eq!(next_active_after_close(&[1], 1), None, "关光了没有下一个");
        assert_eq!(next_active_after_close(&[1, 2], 9), None, "不在表里");
    }

    /// 多项目并行最要紧的一条路由不变式:异步结果按**消息自带的**
    /// `project_id` 落地,与"此刻聚焦的是谁"完全无关。
    ///
    /// 场景就是 review 指出的那个持续性 bug:项目 A 的 agent 在后台跑,用户
    /// 正看着项目 B。A 的 `TermOutput(A, 0, ..)` 到达时,若按"投给当前聚焦的
    /// 项目"路由,`tab_by_id_mut(0)` 会命中 **B 的 tab 0**(每个项目的
    /// `next_tab_id` 都从 0 起编,两边的第一个 tab 必然同号),A 的输出就被
    /// 喂进了 B 的终端。这里断言两个方向都投对了人。
    #[test]
    fn async_results_route_by_project_id_not_by_focus() {
        let mut projects = HashMap::new();
        projects.insert(1, loaded_slot("A"));
        projects.insert(2, loaded_slot("B"));
        // "当前聚焦 B"——路由不该看这个值,这里只是把场景写全。
        let active = Some(2);

        // A 的异步结果(A 在后台)必须落到 A 身上。
        let ws = loaded_workspace_mut(&mut projects, 1).expect("A 已加载");
        assert_eq!(
            ws.files.tree_error(),
            Some("A"),
            "后台项目的结果不能落到前台项目"
        );
        // 反向同理:B 的结果落到 B。
        let ws = loaded_workspace_mut(&mut projects, 2).expect("B 已加载");
        assert_eq!(ws.files.tree_error(), Some("B"));
        assert_eq!(active, Some(2), "路由全程没有读过 active_project_id");
    }

    /// 投不到时静默丢弃,不 panic、不误投给别人:页签在结果回来前被关掉
    /// (槽位不存在),或目标还是没促成过的 `Stub`(不可能有在飞的会话结果)。
    #[test]
    fn async_results_are_dropped_when_target_is_gone_or_unpromoted() {
        let mut projects = HashMap::new();
        projects.insert(1, loaded_slot("A"));
        projects.insert(9, stub_slot(9));

        assert!(
            loaded_workspace_mut(&mut projects, 42).is_none(),
            "页签已关 → 丢弃"
        );
        assert!(
            loaded_workspace_mut(&mut projects, 9).is_none(),
            "Stub 没促成过,不可能有指向它的会话结果"
        );
        // 丢弃的那两条没有波及仍在的槽位。
        assert_eq!(
            loaded_workspace_mut(&mut projects, 1)
                .and_then(|w| w.files.tree_error().map(String::from)),
            Some("A".to_string())
        );
    }

    fn known(ids: &[i64]) -> Vec<ProjectInfo> {
        ids.iter()
            .map(|id| ProjectInfo {
                id: *id,
                path: format!("/tmp/p{id}"),
                name: format!("p{id}"),
                last_active_ms: 0,
                created_ms: 0,
                updated_ms: 0,
            })
            .collect()
    }

    fn state(ids: &[i64], active: Option<i64>) -> open_projects::OpenProjectsState {
        open_projects::OpenProjectsState {
            project_ids: ids.to_vec(),
            active_project_id: active,
        }
    }

    /// 启动恢复的主干:盘上记着的**整份**页签集合都恢复出来(不是只恢复
    /// 聚焦的那一个),顺序按盘上的顺序,聚焦项就是上次退出前聚焦的那个。
    #[test]
    fn restore_open_tabs_restores_whole_tab_set_in_order() {
        let (order, active) = restore_open_tabs(&known(&[1, 2, 3]), &state(&[3, 1, 2], Some(1)));
        assert_eq!(
            order,
            vec![3, 1, 2],
            "页签顺序按盘上记的,不按 daemon 的排序"
        );
        assert_eq!(active, Some(1));
    }

    /// daemon 已经不认识的 id(项目在上次退出后被删了)直接跳过:给它开一个
    /// 点不动的空页签只会碍事。
    #[test]
    fn restore_open_tabs_skips_ids_daemon_no_longer_knows() {
        let (order, active) = restore_open_tabs(&known(&[1, 3]), &state(&[1, 2, 3], Some(3)));
        assert_eq!(order, vec![1, 3]);
        assert_eq!(active, Some(3));
    }

    /// 记着的聚焦项自己就是被删掉的那个 → 回落到第一个页签,而不是留一个
    /// 指向空槽位的 `active_project_id`(那会让界面恒空白)。
    #[test]
    fn restore_open_tabs_falls_back_when_active_is_stale() {
        let (order, active) = restore_open_tabs(&known(&[1, 3]), &state(&[1, 3], Some(2)));
        assert_eq!(order, vec![1, 3]);
        assert_eq!(active, Some(1));
    }

    /// 首次启动(没有 open_projects.json)/上次开着的项目全被删:回落到
    /// `list_projects()` 的第一个——它按 last_active_ms 倒序,就是最近用过的
    /// 那个,保持"打开 app 就能干活"的既有行为。
    #[test]
    fn restore_open_tabs_falls_back_to_most_recent_project() {
        let (order, active) = restore_open_tabs(&known(&[7, 8]), &state(&[], None));
        assert_eq!(order, vec![7]);
        assert_eq!(active, Some(7));

        let (order, active) = restore_open_tabs(&known(&[7, 8]), &state(&[99], Some(99)));
        assert_eq!(order, vec![7], "记着的项目全没了也要回落,不能留空页签栏");
        assert_eq!(active, Some(7));
    }

    /// daemon 上一个项目都没有(全新机器):什么都恢复不出来,`App` 停在
    /// "未打开任何项目"的空外壳,而不是造一个指向不存在项目的页签。
    #[test]
    fn restore_open_tabs_yields_nothing_when_daemon_has_no_projects() {
        let (order, active) = restore_open_tabs(&[], &state(&[1, 2], Some(1)));
        assert!(order.is_empty());
        assert_eq!(active, None);
    }

    /// `open_projects.json` 是用户可编辑的普通 JSON,重复 id 要去重——否则
    /// `project_order` 会带出两个指向同一个槽位的页签(点其中一个,两个一起
    /// 高亮)。
    #[test]
    fn restore_open_tabs_dedups_repeated_ids() {
        let (order, active) = restore_open_tabs(&known(&[1, 2]), &state(&[1, 2, 1], Some(2)));
        assert_eq!(order, vec![1, 2]);
        assert_eq!(active, Some(2));
    }

    /// 关**后台**页签不打扰前台:被关的槽位摘掉、顺序表去掉它,
    /// `active_project_id` 原样不动。
    #[test]
    fn take_project_tab_keeps_active_when_closing_background_tab() {
        let mut projects = HashMap::new();
        projects.insert(1, loaded_slot("A"));
        projects.insert(2, loaded_slot("B"));
        let mut order = vec![1, 2];
        let mut active = Some(1);

        let taken = take_project_tab(&mut projects, &mut order, &mut active, 2);

        assert_eq!(
            slot_marker(taken.as_ref()).as_deref(),
            Some("B"),
            "摘出来的正是那个槽位"
        );
        assert_eq!(active, Some(1), "关后台页签不该改前台");
        assert_eq!(order, vec![1]);
        assert_eq!(slot_marker(projects.get(&1)).as_deref(), Some("A"));
    }

    /// 关的正好是前台页签时焦点顺延到邻居;关光最后一个则回到"未打开任何
    /// 项目"的空外壳(`active_project_id = None`)。
    #[test]
    fn take_project_tab_moves_active_to_neighbor_then_none() {
        let mut projects = HashMap::new();
        projects.insert(1, loaded_slot("A"));
        projects.insert(2, loaded_slot("B"));
        let mut order = vec![1, 2];
        let mut active = Some(1);

        take_project_tab(&mut projects, &mut order, &mut active, 1);
        assert_eq!(active, Some(2));
        assert_eq!(order, vec![2]);

        take_project_tab(&mut projects, &mut order, &mut active, 2);
        assert_eq!(active, None);
        assert!(order.is_empty());
        assert!(projects.is_empty());
    }

    /// 关一个不存在的页签是彻底的空操作(不改 active、不改顺序表)。
    #[test]
    fn take_project_tab_unknown_id_is_noop() {
        let mut projects = HashMap::new();
        projects.insert(1, loaded_slot("A"));
        let mut order = vec![1];
        let mut active = Some(1);

        assert!(take_project_tab(&mut projects, &mut order, &mut active, 42).is_none());
        assert_eq!(active, Some(1));
        assert_eq!(order, vec![1]);
    }

    /// 页签指示点的优先级(2026-08-17 重新定案):红(AwaitingInput,agent 在
    /// 等你)> 绿(Running,还在跑)> 金(TurnEnded,该你出手了)> 青(Idle)
    /// > 不画点。各状态固定配色,不再有闪烁区分。
    ///
    /// 用 `AgentKind::Claude` 代表真实 agent,与下面
    /// `project_dot_unknown_agent_*` 系列(Unknown agent 的死会话灰点)分开测。
    #[test]
    fn project_dot_color_priority() {
        use dozer_core::protocol::AgentKind::Claude;
        use dozer_core::protocol::AgentState::*;

        assert_eq!(project_dot(&[]), None, "无存活会话不画点");
        assert_eq!(
            project_dot(&[(Idle, Claude)]),
            Some(byteui::theme::color::current().cyan)
        );
        assert_eq!(
            project_dot(&[(Idle, Claude), (TurnEnded, Claude)]),
            Some(byteui::theme::color::current().gold),
            "回合结束优先于空闲"
        );
        assert_eq!(
            project_dot(&[(Idle, Claude), (TurnEnded, Claude), (Running, Claude)]),
            Some(byteui::theme::color::current().green),
            "还在跑优先于回合结束/空闲"
        );
        assert_eq!(
            project_dot(&[
                (Idle, Claude),
                (TurnEnded, Claude),
                (Running, Claude),
                (AwaitingInput, Claude)
            ]),
            Some(byteui::theme::color::current().red),
            "agent 在等你优先级最高"
        );
        // 顺序无关:优先级看的是状态集合,不是 tab 的先后。
        assert_eq!(
            project_dot(&[(TurnEnded, Claude), (Idle, Claude)]),
            project_dot(&[(Idle, Claude), (TurnEnded, Claude)])
        );
    }

    /// Unknown agent(纯 shell/git shell/hook 还没上报过)不该显示成跟真实
    /// agent 完成一轮工作同款的 cyan"空闲"——应该显示灰色"死会话"点。
    #[test]
    fn project_dot_unknown_agent_shows_dead_session_gray_not_idle_cyan() {
        use dozer_core::protocol::AgentKind::Unknown;
        use dozer_core::protocol::AgentState::Idle;

        assert_eq!(
            project_dot(&[(Idle, Unknown)]),
            Some(byteui::theme::color::current().dim),
            "只有纯 shell/git shell 存活时应显示死会话灰点,不是空闲青点"
        );
    }

    /// 项目里同时有真实 agent 和纯 shell 存活时,真实 agent 的状态照旧
    /// 优先决定颜色——Unknown 会话不参与竞争,也不会把真实状态"拉低"。
    #[test]
    fn project_dot_real_agent_wins_over_unknown_when_both_alive() {
        use dozer_core::protocol::AgentKind::{Claude, Unknown};
        use dozer_core::protocol::AgentState::{Idle, Running};

        assert_eq!(
            project_dot(&[(Idle, Unknown), (Running, Claude)]),
            Some(byteui::theme::color::current().green),
            "真实 agent 在跑,应该显示绿点,不受纯 shell 的 Idle 干扰"
        );
    }

    /// 测试基准态:默认布局、左=文件列表、右=Agent、两侧都展开。
    fn test_state() -> ShellState {
        ShellState {
            layout: ShellLayout::default(),
            dims: PanelDims::default(),
            left_view: PanelKind::Files,
            left_collapsed: false,
            right_view: PanelKind::Agent,
            right_collapsed: false,
            browser_bookmarks_open: false,
            maximized: None,
        }
    }

    #[test]
    fn terminal_pane_height_excludes_top_bar_and_chrome() {
        // 共享终端自带的 `terminal_status_bar` 已经去掉，高度公式不再扣
        // `status_bar_height()`——只扣顶栏 + chrome + right_zone 上下
        // margin，跟 SSH 终端那边的公式形状一致。
        let state = test_state();
        let (_, h_with) = terminal_pane_pixel_size(1440.0, 900.0, &state);
        let only_chrome = 900.0 - byteui::theme::geometry::chrome_height_px();
        let m = theme::region::right_zone().margin;
        assert!(
            (only_chrome - h_with - (byteui::theme::geometry::top_bar_height() + m.top + m.bottom))
                .abs()
                < 0.01,
            "终端 pane 高度必须再扣顶栏+right_zone 上下 margin"
        );
    }

    #[test]
    fn terminal_pane_zero_when_terminal_not_visible() {
        // 终端只在右侧=Agent 且未收起时可见,否则零尺寸(不 resize 不可见终端)。
        let collapsed = ShellState {
            right_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            terminal_pane_pixel_size(1440.0, 900.0, &collapsed),
            (0.0, 0.0)
        );
        let conversations = ShellState {
            right_view: PanelKind::Conversations,
            ..test_state()
        };
        assert_eq!(
            terminal_pane_pixel_size(1440.0, 900.0, &conversations),
            (0.0, 0.0)
        );
    }

    #[test]
    fn ssh_terminal_pane_matches_left_content_panel_width() {
        // SSH 终端挂在左面板区 `主机列表|终端` 配对的内容侧:宽 = 左区内容
        // 宽 × (1 - ssh_split),再扣左右 chrome。必须跟渲染侧 `left_panel_area`
        // 的 `row![list(FillPortion), divider, content(FillPortion)]` 布局对得上,
        // 否则终端列数套的是共享终端的,拉分隔条就跟不上宿主面板。
        let state = test_state();
        let (w, _h) = ssh_terminal_pane_pixel_size(1440.0, 900.0, &state);
        let left_w = left_zone_width(1440.0, &state);
        let pair_w = pair_content_width(left_w);
        let (_list_w, content_w) = pair_list_content_width(pair_w, state.dims.ssh_split);
        let expected = content_w - byteui::theme::geometry::chrome_width_px();
        assert!(
            (w - expected).abs() < 0.01,
            "SSH 终端 pane 宽必须等于左栏内容侧宽减 chrome: w={w}, expected={expected}"
        );
        assert!(
            w > 0.0 && w < left_w,
            "SSH 终端应占左区一部分宽度、且小于整块左区: w={w}, left_w={left_w}"
        );
        // 隔板越往终端一侧拖(ssh_split 越大),终端越窄——网格必须跟着变。
        let mut tall = state;
        tall.dims.ssh_split = 0.8;
        let (w_tall, _) = ssh_terminal_pane_pixel_size(1440.0, 900.0, &tall);
        assert!(
            w_tall < w,
            "ssh_split 增大后 SSH 终端 pane 应变窄: w_tall={w_tall}, w={w}"
        );
    }

    #[test]
    fn ssh_terminal_pane_height_excludes_top_bar_and_chrome_without_status_bar() {
        // SSH 终端没有 `terminal_status_bar`,高度只扣顶栏 + chrome + 左区
        // 上下 margin(不像共享终端那样再额外扣状态栏高)。
        let state = test_state();
        let (_, h) = ssh_terminal_pane_pixel_size(1440.0, 900.0, &state);
        let m = theme::region::left_zone().margin;
        let expected = 900.0
            - byteui::theme::geometry::top_bar_height()
            - byteui::theme::geometry::chrome_height_px()
            - m.top
            - m.bottom;
        assert!(
            (h - expected).abs() < 0.01,
            "SSH 终端 pane 高度:{h}, expected:{expected}"
        );
    }

    #[test]
    fn ssh_terminal_pane_zero_when_left_collapsed() {
        let collapsed = ShellState {
            left_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            ssh_terminal_pane_pixel_size(1440.0, 900.0, &collapsed),
            (0.0, 0.0),
            "左面板区收起时 SSH 终端不可见,应返回零尺寸"
        );
    }

    /// Fix round 2 Critical #1:窗口被缩到比持久化 `left_width` 还窄时,
    /// 左面板区的**有效**宽必须重新夹取,否则右面板区一寸不剩。
    ///
    /// 具体数字(窗口逻辑宽 720——1440pt 屏上把窗口贴半屏就是这个宽度):
    /// `zones_width(720)` = 720 - 2*44(图标栏) - 8(LeftRight 分隔线) = 624;
    /// 上界 = max(624 - 320(byteui::theme::geometry::min_zone_width()), 320) = 320;
    /// 默认 `left_width`=640 夹取后 = 320,右面板区 = 624 - 320 = 304(>0)。
    ///
    /// 修复前的 flex 追账(iced_core flex.rs `resolve` 第一趟按顺序给
    /// 非流体子元素分配、`available` 递减):available=720 →左图标栏 Fixed(44)
    /// →676 →左面板区 `Length::Fixed(640)` 全额吃下 →36 →分隔线 Fixed(8)
    /// →28 →右图标栏 Fixed(44) 被 `Limits::resolve` 夹到 28 →0,
    /// `remaining`=0,唯一 `Fill` 的右面板区第三趟拿到 0 宽:终端/Agent 列表/
    /// 对话/审阅整片消失,右图标栏还被压成半宽。
    #[test]
    fn left_zone_width_reclamped_when_window_narrower_than_persisted_width() {
        let state = test_state();
        assert_eq!(state.dims.left_width, 640.0, "前提:默认持久化宽 640");

        assert_eq!(clamp_left_width(720.0, 640.0), 320.0);
        assert_eq!(left_zone_width(720.0, &state), 320.0);
        let right = right_zone_width(720.0, &state);
        assert!(
            (right - 304.0).abs() < 0.01,
            "右面板区必须仍有宽度: {right}"
        );
        assert!(right >= 1.0, "右半边不能塌成 0 宽");

        // 宽窗口下不受影响(不能为了修窄窗把正常情形也改坏)。
        assert_eq!(clamp_left_width(1440.0, 640.0), 640.0);
        assert_eq!(left_zone_width(1440.0, &state), 640.0);
        assert_ne!(
            left_zone_width(720.0, &state),
            left_zone_width(1440.0, &state),
            "窄窗与宽窗必须得出不同的有效宽"
        );

        // 夹取是**渲染/几何时刻**的临时行为:不回写持久化值,窗口再拉宽
        // 时用户原来偏好的 640 自动复原。
        assert_eq!(state.dims.left_width, 640.0, "夹取不得改写各项目持久化宽");
    }

    /// 极窄窗口(比 `byteui::theme::geometry::min_window_width()` 还窄,例如外部强制 resize)下也不 panic,
    /// 且左区宽不会超过 `zones_width` 本身。
    #[test]
    fn clamp_left_width_survives_absurdly_narrow_window() {
        assert_eq!(
            clamp_left_width(200.0, 640.0),
            byteui::theme::geometry::min_zone_width()
        );
        assert_eq!(
            clamp_left_width(0.0, 640.0),
            byteui::theme::geometry::min_zone_width()
        );
        // 最小窗口宽恰好能让两侧都拿到 byteui::theme::geometry::min_zone_width()。
        assert_eq!(
            clamp_left_width(byteui::theme::geometry::min_window_width(), 640.0),
            byteui::theme::geometry::min_zone_width()
        );
        assert!(
            (zones_width(byteui::theme::geometry::min_window_width())
                - 2.0 * byteui::theme::geometry::min_zone_width())
            .abs()
                < 0.01
        );
    }

    /// Fix round 2 #3:终端可见性判定。旧四栏布局里终端恒在屏上,新外壳有三
    /// 条路径能把它藏起来,藏着时不能再把按键写进 PTY。
    #[test]
    fn terminal_visible_only_when_agent_pair_on_screen() {
        assert!(
            terminal_visible(&test_state()),
            "基准态:右侧 Agent 配对可见"
        );
        assert!(
            !terminal_visible(&ShellState {
                right_view: PanelKind::Conversations,
                ..test_state()
            }),
            "右视图切到对话:终端不在屏上"
        );
        assert!(
            !terminal_visible(&ShellState {
                right_collapsed: true,
                ..test_state()
            }),
            "右侧收起:终端不在屏上"
        );
        assert!(
            !terminal_visible(&ShellState {
                maximized: Some(MaximizedPane::Left),
                ..test_state()
            }),
            "左侧放大:右半边被变暗遮罩整片盖住"
        );
        assert!(
            terminal_visible(&ShellState {
                maximized: Some(MaximizedPane::Right),
                ..test_state()
            }),
            "放大的正是终端那一侧:终端更大更可见,算可见"
        );
    }

    /// Fix round 3(scoped re-review 发现):对话视图下放大的是审阅 pane
    /// 自己的放大按钮(同样发 `MaximizedPane::Right`),不是终端——
    /// `terminal_grid_state` 如果不过滤这种情况,会把"审阅被放大"误判成
    /// "终端被放大",按放大格给一个实际不可见、也不是那个尺寸的终端算网格,
    /// 给存活 PTY 发一次错的 SIGWINCH。
    #[test]
    fn terminal_grid_state_ignores_maximized_review_not_terminal() {
        let review_maximized = ShellState {
            right_view: PanelKind::Conversations,
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        let grid_state = terminal_grid_state(review_maximized);
        assert_eq!(
            grid_state.maximized, None,
            "对话视图下的 MaximizedPane::Right 指的是审阅 pane,换算终端网格时不该当成终端被放大"
        );

        let terminal_maximized = ShellState {
            right_view: PanelKind::Agent,
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        assert_eq!(
            terminal_grid_state(terminal_maximized).maximized,
            Some(MaximizedPane::Right),
            "右视图本来就是 Agent 时,MaximizedPane::Right 才真的是终端被放大,要保留"
        );
    }

    /// Fix round 2 #6a:放大终端时 PTY 网格必须按 `maximize_overlay` 实际
    /// 渲染的金色描边盒子重算,不能停在放大前的尺寸(否则只放大了外框)。
    ///
    /// 具体数字(1440x900,`agent_split`=默认统一 split 0.35):
    /// avail_w = 1440 - 2*44 - 2*40 = 1272,pair_w = 1272 - 8 = 1264,
    /// 终端占 1-0.35 → 1264*0.65 = 821.6,减 `byteui::theme::geometry::chrome_width_px()`(16) = 805.6;
    /// 盒子高 = 900 - 40(顶栏) - 2*40 = 780,再减
    /// `byteui::theme::geometry::chrome_height_px()`(50) = 730(不扣状态栏高——
    /// 终端 pane 自带的 `terminal_status_bar` 已经去掉)。
    /// 对照平时:zones_width = 1440-2*44-8=1344,right_w = 1344 - 640 = 704,pair = 696,
    /// 696*0.65 = 452.4,减 16 = 436.4;高 = 900 - 40 - 50 - right_zone 上下 margin = 804。
    /// 换成网格(CELL_WIDTH=8.4,LINE_HEIGHT_PX=16.8 即 14*1.2):放大后 95x43,平时 51x47。
    #[test]
    fn terminal_pane_pixel_size_right_maximized_matches_overlay_box() {
        let maxed = ShellState {
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        let (w, h) = terminal_pane_pixel_size(1440.0, 900.0, &maxed);
        assert!((w - 805.6).abs() < 0.1, "w={w}");
        assert!((h - 730.0).abs() < 0.1, "h={h}");

        let normal = terminal_pane_pixel_size(1440.0, 900.0, &test_state());
        assert!((normal.0 - 436.4).abs() < 0.1, "平时 w={}", normal.0);
        assert!((normal.1 - 804.0).abs() < 0.1, "平时 h={}", normal.1);
        assert_ne!((w, h), normal, "放大态几何必须和平时不同");
        assert!(w > normal.0, "放大后终端必须真的更宽(网格跟着变宽)");

        assert_eq!(crate::term_view::grid_size(w, h), (95, 43));
        assert_eq!(crate::term_view::grid_size(normal.0, normal.1), (51, 47));

        // 左侧放大不改变右面板区几何(右半只是被遮罩盖住)。
        let left_maxed = ShellState {
            maximized: Some(MaximizedPane::Left),
            ..test_state()
        };
        assert_eq!(terminal_pane_pixel_size(1440.0, 900.0, &left_maxed), normal);
    }

    /// Fix round 2 #6b:终端当前不可见时,网格换算走"若显示则多大"的假想
    /// 状态。否则上次退出时右侧停在对话视图 → 启动时 `terminal_pane_pixel_size`
    /// 返回 (0,0) → main.rs 的 `cols>0 && rows>0` 守卫跳过 → 恢复出来的会话
    /// 一直卡在 80x24(`DEFAULT_COLS`/`DEFAULT_ROWS`),哪怕屏幕很大。
    #[test]
    fn terminal_grid_state_sizes_hidden_terminal_as_if_shown() {
        let shown = terminal_pane_pixel_size(1440.0, 900.0, &test_state());

        for hidden in [
            ShellState {
                right_view: PanelKind::Conversations,
                ..test_state()
            },
            ShellState {
                right_collapsed: true,
                ..test_state()
            },
        ] {
            // 真实状态下仍是零尺寸(不 resize 一个不可见的终端)。
            assert_eq!(terminal_pane_pixel_size(1440.0, 900.0, &hidden), (0.0, 0.0));
            // 网格换算用的假想状态下,尺寸与"右侧展开显示 Agent"完全一致。
            let for_grid = terminal_grid_state(hidden);
            assert_eq!(terminal_pane_pixel_size(1440.0, 900.0, &for_grid), shown);
        }

        // 具体网格:1440x900 下应是 51x47(已扣 right_zone 上下 margin),不是兜底的 80x24。
        let (cols, rows) = crate::term_view::grid_size(shown.0, shown.1);
        assert_eq!((cols, rows), (51, 47));
        assert_ne!(
            (cols as u16, rows as u16),
            (DEFAULT_COLS, DEFAULT_ROWS),
            "启动时必须算出真实网格,不能停在 80x24 兜底值"
        );

        // 假想状态只覆盖右侧收起/右视图,不篡改放大态与左侧状态。
        let left_max = ShellState {
            maximized: Some(MaximizedPane::Left),
            left_collapsed: true,
            right_collapsed: true,
            ..test_state()
        };
        let g = terminal_grid_state(left_max);
        assert_eq!(g.maximized, Some(MaximizedPane::Left));
        assert!(g.left_collapsed);
        assert!(!g.right_collapsed);
    }

    /// 可选项:磁盘上的面板尺寸值不一定出自本程序(手改 panel_layouts.json/
    /// 别的版本)。split 恰为 0.0/1.0 时 `split_portions` 会给出 `FillPortion(0)`,
    /// 那一块在 flex 里拿不到任何宽度、整块消失,所以读盘时先夹一遍。
    #[test]
    fn sanitize_panel_dims_clamps_foreign_values() {
        let poisoned = PanelDims {
            left_width: 10.0,
            files_split: 0.0,
            agent_split: 1.0,
            conversations_split: f32::NAN,
            ..PanelDims::default()
        };
        let s = sanitize_panel_dims(poisoned);
        assert_eq!(s.left_width, byteui::theme::geometry::min_zone_width());
        assert_eq!(s.files_split, byteui::theme::geometry::min_split_ratio());
        assert_eq!(s.agent_split, byteui::theme::geometry::max_split_ratio());
        assert_eq!(s.conversations_split, PanelDims::default().files_split);
        // 夹过之后 FillPortion 两侧都非 0(那一块不会凭空消失)。
        for split in [
            s.files_split,
            s.project_split,
            s.agent_split,
            s.conversations_split,
        ] {
            let (list, content) = split_portions(split);
            assert!(list > 0 && content > 0, "split={split}");
        }
        // 合法值原样保留。
        let sane = PanelDims {
            left_width: 500.0,
            files_split: 0.35,
            ..PanelDims::default()
        };
        assert_eq!(sanitize_panel_dims(sane), sane);
    }

    #[test]
    fn apply_column_drag_updates_git_log_split_ratio() {
        let state = test_state();
        let window_width = 1600.0;
        let result = apply_column_drag(state, Divider::GitLogSplit, window_width, 300.0);
        assert!(result.git_log_split >= byteui::theme::geometry::min_split_ratio());
        assert!(result.git_log_split <= byteui::theme::geometry::max_split_ratio());
    }

    #[test]
    fn apply_column_drag_updates_browser_bookmarks_split_ratio() {
        let state = test_state();
        let window_width = 1600.0;
        // 拖拽点在左面板区靠右侧,网页内容(拖拽点左侧)占比应偏大。
        let result = apply_column_drag(state, Divider::BrowserBookmarksSplit, window_width, 500.0);
        assert!(result.browser_bookmarks_split >= byteui::theme::geometry::min_split_ratio());
        assert!(result.browser_bookmarks_split <= byteui::theme::geometry::max_split_ratio());
    }

    #[test]
    fn apply_column_drag_browser_bookmarks_split_direction_matches_content_side() {
        // 方向性回归:拖拽点越靠右,网页内容(左侧)占比应该越大——
        // browser_bookmarks_split 存的是内容占比,不是收藏夹占比。
        let state = test_state();
        let window_width = 1600.0;
        let near = apply_column_drag(
            state.clone(),
            Divider::BrowserBookmarksSplit,
            window_width,
            100.0,
        );
        let far = apply_column_drag(state, Divider::BrowserBookmarksSplit, window_width, 500.0);
        assert!(
            far.browser_bookmarks_split > near.browser_bookmarks_split,
            "near={} far={}",
            near.browser_bookmarks_split,
            far.browser_bookmarks_split
        );
    }

    #[test]
    fn sanitize_panel_dims_clamps_git_log_split() {
        let dims = PanelDims {
            git_log_split: 5.0,
            ..PanelDims::default()
        };
        let sanitized = sanitize_panel_dims(dims);
        assert!(sanitized.git_log_split <= byteui::theme::geometry::max_split_ratio());

        let dims = PanelDims {
            git_log_split: f32::NAN,
            ..PanelDims::default()
        };
        let sanitized = sanitize_panel_dims(dims);
        assert_eq!(sanitized.git_log_split, PanelDims::default().files_split);
    }

    #[test]
    fn default_panel_dims_includes_browser_bookmarks_split() {
        let dims = PanelDims::default();
        assert_eq!(
            dims.browser_bookmarks_split,
            byteui::theme::geometry::default_split_ratio()
        );
    }

    #[test]
    fn sanitize_panel_dims_clamps_browser_bookmarks_split() {
        let dims = PanelDims {
            browser_bookmarks_split: 5.0,
            ..PanelDims::default()
        };
        let sanitized = sanitize_panel_dims(dims);
        assert!(sanitized.browser_bookmarks_split <= byteui::theme::geometry::max_split_ratio());

        let dims = PanelDims {
            browser_bookmarks_split: f32::NAN,
            ..PanelDims::default()
        };
        let sanitized = sanitize_panel_dims(dims);
        assert_eq!(
            sanitized.browser_bookmarks_split,
            PanelDims::default().files_split
        );
    }

    #[test]
    fn apply_row_drag_clamps_ratio_within_valid_range() {
        let state = test_state();
        let window_height = 1000.0;

        // 光标在窗口中间——应该落在合法比例区间内。
        let mid = apply_row_drag(
            state.clone(),
            RowDivider::GitLogFileDiffSplit,
            window_height,
            500.0,
        );
        assert!(mid.git_log_file_diff_split >= byteui::theme::geometry::min_split_ratio());
        assert!(mid.git_log_file_diff_split <= byteui::theme::geometry::max_split_ratio());

        // 光标远超窗口顶部/底部——应该被 clamp,不产生非法比例。
        let top = apply_row_drag(
            state.clone(),
            RowDivider::GitLogFileDiffSplit,
            window_height,
            -500.0,
        );
        assert_eq!(
            top.git_log_file_diff_split,
            byteui::theme::geometry::min_split_ratio()
        );
        let bottom = apply_row_drag(
            state,
            RowDivider::GitLogFileDiffSplit,
            window_height,
            5000.0,
        );
        assert_eq!(
            bottom.git_log_file_diff_split,
            byteui::theme::geometry::max_split_ratio()
        );
    }

    #[test]
    fn sanitize_panel_dims_clamps_git_log_file_diff_split() {
        let dims = PanelDims {
            git_log_file_diff_split: -1.0,
            ..PanelDims::default()
        };
        let sanitized = sanitize_panel_dims(dims);
        assert!(sanitized.git_log_file_diff_split >= byteui::theme::geometry::min_split_ratio());
    }

    /// `window_width`/`window_height` 的夹取单独测:老 `layout.json` 缺这两
    /// 个字段时 serde 补 0.0(不是 `f32::NAN`,判断要用 `> 0.0` 而不能只查
    /// `is_finite`),负数/NAN 同样要落回 `byteui::theme::geometry::initial_window_size()`;合法但过小
    /// 的值只夹下限,不整个重置。
    #[test]
    fn sanitize_shell_layout_clamps_window_size() {
        let missing_fields = ShellLayout {
            window_width: 0.0,
            window_height: 0.0,
            ..ShellLayout::default()
        };
        let s = sanitize_shell_layout(missing_fields);
        assert_eq!(
            s.window_width,
            byteui::theme::geometry::initial_window_size().0
        );
        assert_eq!(
            s.window_height,
            byteui::theme::geometry::initial_window_size().1
        );

        let poisoned = ShellLayout {
            window_width: -100.0,
            window_height: f32::NAN,
            ..ShellLayout::default()
        };
        let s = sanitize_shell_layout(poisoned);
        assert_eq!(
            s.window_width,
            byteui::theme::geometry::initial_window_size().0
        );
        assert_eq!(
            s.window_height,
            byteui::theme::geometry::initial_window_size().1
        );

        let too_small = ShellLayout {
            window_width: 10.0,
            window_height: 10.0,
            ..ShellLayout::default()
        };
        let s = sanitize_shell_layout(too_small);
        assert_eq!(s.window_width, byteui::theme::geometry::min_window_width());
        assert_eq!(
            s.window_height,
            byteui::theme::geometry::min_window_height()
        );

        let legit = ShellLayout {
            window_width: 1800.0,
            window_height: 1100.0,
            ..ShellLayout::default()
        };
        assert_eq!(sanitize_shell_layout(legit.clone()), legit);
    }

    #[test]
    fn zone_at_x_splits_left_and_right_at_zone_boundary() {
        // 窗口宽 1440:左图标栏 44 + 左面板区 640 → 边界在 x=684。
        let state = test_state();
        assert_eq!(
            zone_at_x(100.0, 1440.0, &state),
            Some(ZoneSide::Left),
            "左面板区内"
        );
        assert_eq!(
            zone_at_x(683.9, 1440.0, &state),
            Some(ZoneSide::Left),
            "边界前一发"
        );
        assert_eq!(
            zone_at_x(684.0, 1440.0, &state),
            Some(ZoneSide::Right),
            "边界本身归右面板区"
        );
        assert_eq!(
            zone_at_x(1200.0, 1440.0, &state),
            Some(ZoneSide::Right),
            "右面板区内"
        );
    }

    #[test]
    fn zone_at_x_icon_rail_returns_none() {
        let state = test_state();
        assert_eq!(zone_at_x(20.0, 1440.0, &state), None, "左图标栏本身");
        assert_eq!(zone_at_x(1400.0, 1440.0, &state), None, "右图标栏本身");
    }

    #[test]
    fn zone_at_x_collapsed_side_routes_to_the_other() {
        let left_collapsed = ShellState {
            left_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            zone_at_x(300.0, 1440.0, &left_collapsed),
            Some(ZoneSide::Right),
            "左侧收起,内容区恒归右侧"
        );

        let right_collapsed = ShellState {
            right_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            zone_at_x(1200.0, 1440.0, &right_collapsed),
            Some(ZoneSide::Left),
            "右侧收起,内容区恒归左侧"
        );

        let both_collapsed = ShellState {
            left_collapsed: true,
            right_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            zone_at_x(700.0, 1440.0, &both_collapsed),
            None,
            "两侧都收起,没有哪一侧可点"
        );
    }

    #[test]
    fn zone_at_x_maximized_ignores_x_within_content_range() {
        let left_max = ShellState {
            maximized: Some(MaximizedPane::Left),
            ..test_state()
        };
        // 放大态下整个内容区都算放大的那一侧,不再按横坐标细分。
        assert_eq!(zone_at_x(100.0, 1440.0, &left_max), Some(ZoneSide::Left));
        assert_eq!(zone_at_x(1200.0, 1440.0, &left_max), Some(ZoneSide::Left));

        let right_max = ShellState {
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        assert_eq!(zone_at_x(100.0, 1440.0, &right_max), Some(ZoneSide::Right));
    }

    #[test]
    fn clamp_left_width_within_bounds() {
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftRight, 1440.0, 500.0);
        assert_eq!(
            l.left_width,
            500.0 - byteui::theme::geometry::icon_rail_width()
        );
    }

    #[test]
    fn clamp_left_width_to_minimum() {
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftRight, 1440.0, 10.0);
        assert_eq!(l.left_width, byteui::theme::geometry::min_zone_width());
    }

    #[test]
    fn clamp_left_width_to_maximum_keeps_right_zone_alive() {
        // 拖到最右也要给右面板区留 byteui::theme::geometry::min_zone_width()。
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftRight, 1440.0, 1430.0);
        let expected = 1440.0
            - 2.0 * byteui::theme::geometry::icon_rail_width()
            - byteui::theme::geometry::divider_width()
            - byteui::theme::geometry::min_zone_width();
        assert_eq!(l.left_width, expected);
    }

    #[test]
    fn clamp_left_width_when_window_too_narrow_does_not_panic() {
        // 窗口窄到上界低于下界时,`.max(下限)` 把上界垫平,恒不 panic。
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftRight, 700.0, 650.0);
        assert_eq!(l.left_width, byteui::theme::geometry::min_zone_width());
    }

    #[test]
    fn left_right_drag_keeps_list_pane_pixel_width_fixed() {
        // 默认 `test_state()`:left_view = Files,files_split 默认 0.35。
        // 拖外层 zone 分隔线把 left_width 从默认 640 拉到 800,文件树的
        // 绝对像素宽应该保持不变(只有比例跟着 zone 变宽而回调)。
        let state = test_state();
        let window_width = 1440.0;
        let old_pair_w = pair_content_width(clamp_left_width(window_width, state.dims.left_width));
        let old_list_px = old_pair_w * state.dims.files_split;

        let logical_x = 800.0 + byteui::theme::geometry::icon_rail_width();
        let new = apply_column_drag(state.clone(), Divider::LeftRight, window_width, logical_x);
        assert!(
            (new.left_width - 800.0).abs() < 0.01,
            "拖拽目标本身要生效: {}",
            new.left_width
        );

        let new_pair_w = pair_content_width(clamp_left_width(window_width, new.left_width));
        let new_list_px = new_pair_w * new.files_split;
        assert!(
            (new_list_px - old_list_px).abs() < 0.5,
            "文件树列表像素宽应保持不变: old={old_list_px}, new={new_list_px}"
        );
        // 反解出的新比例必须比默认值小——zone 变宽了,同样的像素宽占比更低。
        assert!(new.files_split < state.dims.files_split);
    }

    #[test]
    fn left_right_drag_also_compensates_right_zone_active_panel() {
        // right_view 默认是 Agent(agent_split)。左边变宽会挤压右边 zone,
        // 右侧配对面板(这里是 Agent 列表)同样不该被连带缩放。
        let state = test_state();
        let window_width = 1440.0;
        let old_right_zone_w = right_zone_width(window_width, &state);
        let old_pair_w = pair_content_width(old_right_zone_w);
        let old_list_px = old_pair_w * state.dims.agent_split;

        let logical_x = 800.0 + byteui::theme::geometry::icon_rail_width();
        let new = apply_column_drag(state.clone(), Divider::LeftRight, window_width, logical_x);
        let new_probe = ShellState { dims: new, ..state };
        let new_right_zone_w = right_zone_width(window_width, &new_probe);
        let new_pair_w = pair_content_width(new_right_zone_w);
        let new_list_px = new_pair_w * new_probe.dims.agent_split;
        assert!(
            (new_list_px - old_list_px).abs() < 0.5,
            "Agent 列表像素宽应保持不变: old={old_list_px}, new={new_list_px}"
        );
    }

    #[test]
    fn clamp_files_split_within_bounds() {
        let state = test_state();
        // 左面板区 640 宽,配对内容宽 = 640-8=632,拖到其中点(316)→ 0.5。
        let l = apply_column_drag(
            state,
            Divider::LeftPairSplit,
            1440.0,
            byteui::theme::geometry::icon_rail_width() + 316.0,
        );
        assert!((l.files_split - 0.5).abs() < 0.001, "{}", l.files_split);
    }

    #[test]
    fn clamp_files_split_to_range() {
        let state = test_state();
        let l = apply_column_drag(state.clone(), Divider::LeftPairSplit, 1440.0, 10.0);
        assert_eq!(l.files_split, byteui::theme::geometry::min_split_ratio());
        let l = apply_column_drag(state, Divider::LeftPairSplit, 1440.0, 5000.0);
        assert_eq!(l.files_split, byteui::theme::geometry::max_split_ratio());
    }

    #[test]
    fn split_drag_skips_update_on_zero_zone_width() {
        // 该侧已收起 → 区宽 0,除零防御:原样返回不 panic。
        let left_gone = ShellState {
            left_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            apply_column_drag(left_gone.clone(), Divider::LeftPairSplit, 1440.0, 500.0),
            left_gone.dims
        );
        let right_gone = ShellState {
            right_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            apply_column_drag(right_gone.clone(), Divider::RightPairSplit, 1440.0, 1000.0),
            right_gone.dims
        );
    }

    #[test]
    fn right_pair_split_writes_field_of_current_right_view() {
        // 右面板区宽 = 1440 - 2*44 - 640 - 8 = 704,左边缘 x = 1440-44-704 = 692;
        // 配对内容宽 = 704-8=696,其中点 348 处拖动(692+348=1040)→ 0.5。
        let agent = test_state();
        let l = apply_column_drag(
            agent.clone(),
            Divider::RightPairSplit,
            1440.0,
            696.0 + 344.0,
        );
        assert!((l.agent_split - 0.5).abs() < 0.001, "{}", l.agent_split);
        assert_eq!(
            l.conversations_split, agent.dims.conversations_split,
            "不该串写另一配对的比例"
        );

        let conversations = ShellState {
            right_view: PanelKind::Conversations,
            ..test_state()
        };
        let l = apply_column_drag(
            conversations.clone(),
            Divider::RightPairSplit,
            1440.0,
            696.0 + 344.0,
        );
        assert!(
            (l.conversations_split - 0.5).abs() < 0.001,
            "{}",
            l.conversations_split
        );
        assert_eq!(l.agent_split, conversations.dims.agent_split);
    }

    /// 中点(0.5)拖到哪都是 0.5,取反前后数值一样,不能证明真的取反了。
    /// 这里挑一个偏离中点的落点单独验证方向:配对渲染顺序是"终端/审阅在
    /// 左、Agent 列表/对话列表在右"(见 `right_panel_area`),拖拽点落在
    /// 离左边缘 1/4 处 → 左边(终端/审阅)只分到 25% 宽 → 右边(列表侧,
    /// `agent_split`/`conversations_split` 存的量)理应分到 75%,而不是 25%。
    #[test]
    fn right_pair_split_ratio_is_inverted_for_swapped_visual_order() {
        // 同上一个测试:右面板区左边缘 x=692,配对内容宽=696。落点在左边缘
        // 往右 174(=696/4)处,即左侧(终端)拿到 1/4 宽。
        let agent = test_state();
        let l = apply_column_drag(agent, Divider::RightPairSplit, 1440.0, 692.0 + 174.0);
        assert!(
            (l.agent_split - 0.75).abs() < 0.001,
            "左侧(终端)占 1/4 时,右侧(Agent 列表)该占 3/4,实得 {}",
            l.agent_split
        );

        let conversations = ShellState {
            right_view: PanelKind::Conversations,
            ..test_state()
        };
        let l = apply_column_drag(
            conversations,
            Divider::RightPairSplit,
            1440.0,
            692.0 + 174.0,
        );
        assert!(
            (l.conversations_split - 0.75).abs() < 0.001,
            "左侧(审阅)占 1/4 时,右侧(对话列表)该占 3/4,实得 {}",
            l.conversations_split
        );
    }

    // ---- README 自动生成 ---- //

    /// 没有 README、没写描述:只生成一个"标题+空行"的最小文档,带项目名。
    #[test]
    fn readme_created_from_name_without_description() {
        let dir = tempfile::tempdir().unwrap();
        let created = ensure_project_readme(dir.path(), "我的项目").unwrap();
        assert_eq!(created, dir.path().join("README.md"));
        let body = std::fs::read_to_string(&created).unwrap();
        assert!(body.starts_with("# 我的项目\n\n"), "实际: {body:?}");
    }

    /// 带有 `.dozer/description.md`:一级标题用项目名,正文接描述。
    #[test]
    fn readme_embeds_description_from_dozer_dir() {
        let dir = tempfile::tempdir().unwrap();
        crate::project_meta::write_description(dir.path(), "这是一段中文描述").unwrap();
        let created = ensure_project_readme(dir.path(), "Demo").unwrap();
        let body = std::fs::read_to_string(&created).unwrap();
        assert_eq!(body, "# Demo\n\n这是一段中文描述\n");
    }

    /// 已存在 README:直接返回它的路径(送到预览窗去展示),绝不重写、绝不
    /// 覆盖用户已有内容。
    #[test]
    fn readme_exists_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("README.md");
        std::fs::write(&existing, "用户手写的内容\n").unwrap();
        assert_eq!(
            ensure_project_readme(dir.path(), "D"),
            Some(existing.clone())
        );
        assert_eq!(
            std::fs::read_to_string(&existing).unwrap(),
            "用户手写的内容\n"
        );
    }

    /// 目标是文件而非目录时的拒绝语义,等价于 repo 根不可写/不可用。
    #[test]
    fn readme_missing_on_unwritable_root_returns_none() {
        let file_as_repo = tempfile::tempdir().unwrap();
        let repo_path = file_as_repo.path().join("not_a_dir");
        std::fs::write(&repo_path, "我是文件").unwrap();
        assert_eq!(ensure_project_readme(&repo_path, "D"), None);
    }

    /// `apply_column_drag` 里 Ssh/Todo/GitLog 三个左栏默认面板的 side+镜像
    /// 感知改造测试。默认栏(左)下方向应与改造前固定行为逐字节一致(防回归
    /// 锚);挪到右栏后方向要反转(镜像态下"列表在后")。用 near/far 方向性
    /// 比较,不手算精确数值(同既有
    /// `apply_column_drag_browser_bookmarks_split_direction_matches_content_side`)。
    mod apply_column_drag_ssh_todo_gitlog_mirror_tests {
        use super::*;

        fn right_x0_inside(window_width: f32, state: &ShellState) -> f32 {
            window_width
                - byteui::theme::geometry::icon_rail_width()
                - right_zone_width(window_width, state)
        }

        fn relocate_to_right(state: &mut ShellState, kind: PanelKind) {
            state.layout.rail_layout.left.retain(|&k| k != kind);
            state.layout.rail_layout.right.push(kind);
        }

        #[test]
        fn ssh_split_direction_on_default_side_matches_pre_migration_behavior() {
            let state = test_state();
            let window_width = 1600.0;
            let near = apply_column_drag(state.clone(), Divider::SshSplit, window_width, 300.0);
            let far = apply_column_drag(state, Divider::SshSplit, window_width, 500.0);
            assert!(
                far.ssh_split > near.ssh_split,
                "near={} far={}",
                near.ssh_split,
                far.ssh_split
            );
        }

        #[test]
        fn ssh_split_direction_flips_when_relocated_to_right_side() {
            let mut state = test_state();
            relocate_to_right(&mut state, PanelKind::Ssh);
            let window_width = 1600.0;
            let x0 = right_x0_inside(window_width, &state);
            let near = apply_column_drag(state.clone(), Divider::SshSplit, window_width, x0 + 50.0);
            let far = apply_column_drag(state, Divider::SshSplit, window_width, x0 + 250.0);
            assert!(
                far.ssh_split < near.ssh_split,
                "镜像态下方向应反转:near={} far={}",
                near.ssh_split,
                far.ssh_split
            );
        }

        #[test]
        fn todo_split_direction_on_default_side_matches_pre_migration_behavior() {
            let state = test_state();
            let window_width = 1600.0;
            let near = apply_column_drag(state.clone(), Divider::TodoSplit, window_width, 300.0);
            let far = apply_column_drag(state, Divider::TodoSplit, window_width, 500.0);
            assert!(
                far.todo_split > near.todo_split,
                "near={} far={}",
                near.todo_split,
                far.todo_split
            );
        }

        #[test]
        fn todo_split_direction_flips_when_relocated_to_right_side() {
            let mut state = test_state();
            relocate_to_right(&mut state, PanelKind::Todo);
            let window_width = 1600.0;
            let x0 = right_x0_inside(window_width, &state);
            let near =
                apply_column_drag(state.clone(), Divider::TodoSplit, window_width, x0 + 50.0);
            let far = apply_column_drag(state, Divider::TodoSplit, window_width, x0 + 250.0);
            assert!(
                far.todo_split < near.todo_split,
                "镜像态下方向应反转:near={} far={}",
                near.todo_split,
                far.todo_split
            );
        }

        #[test]
        fn git_log_split_direction_on_default_side_matches_pre_migration_behavior() {
            let state = test_state();
            let window_width = 1600.0;
            let near = apply_column_drag(state.clone(), Divider::GitLogSplit, window_width, 300.0);
            let far = apply_column_drag(state, Divider::GitLogSplit, window_width, 500.0);
            assert!(
                far.git_log_split > near.git_log_split,
                "near={} far={}",
                near.git_log_split,
                far.git_log_split
            );
        }

        #[test]
        fn git_log_split_direction_flips_when_relocated_to_right_side() {
            let mut state = test_state();
            relocate_to_right(&mut state, PanelKind::GitLog);
            let window_width = 1600.0;
            let x0 = right_x0_inside(window_width, &state);
            let near =
                apply_column_drag(state.clone(), Divider::GitLogSplit, window_width, x0 + 50.0);
            let far = apply_column_drag(state, Divider::GitLogSplit, window_width, x0 + 250.0);
            assert!(
                far.git_log_split < near.git_log_split,
                "镜像态下方向应反转:near={} far={}",
                near.git_log_split,
                far.git_log_split
            );
        }
    }

    /// `apply_column_drag` 里 `LeftPairSplit`(Files)/`ProjectSplit`/
    /// `BrowserBookmarksSplit` 三个本 Stage 改过 base 的分支的 side+镜像
    /// 感知测试(此前硬编码 `left_zone_width` + `icon_rail_width()`,Project
    /// /Web 挪到右栏后拖拽方向直接错乱)。Files/Project 默认"列表在前",
    /// Web 默认"内容在前"且 `browser_bookmarks_split` 存内容占比——方向翻转
    /// 的判定彼此相反,分开写清楚。
    mod apply_column_drag_files_project_web_mirror_tests {
        use super::*;

        fn right_x0_inside(window_width: f32, state: &ShellState) -> f32 {
            window_width
                - byteui::theme::geometry::icon_rail_width()
                - right_zone_width(window_width, state)
        }

        fn relocate_to_right(state: &mut ShellState, kind: PanelKind) {
            state.layout.rail_layout.left.retain(|&k| k != kind);
            state.layout.rail_layout.right.push(kind);
        }

        #[test]
        fn files_split_direction_on_default_side_matches_pre_migration_behavior() {
            let state = test_state();
            let window_width = 1600.0;
            let near =
                apply_column_drag(state.clone(), Divider::LeftPairSplit, window_width, 300.0);
            let far = apply_column_drag(state, Divider::LeftPairSplit, window_width, 500.0);
            assert!(
                far.files_split > near.files_split,
                "near={} far={}",
                near.files_split,
                far.files_split
            );
        }

        #[test]
        fn files_split_direction_flips_when_relocated_to_right_side() {
            let mut state = test_state();
            relocate_to_right(&mut state, PanelKind::Files);
            let window_width = 1600.0;
            let x0 = right_x0_inside(window_width, &state);
            let near = apply_column_drag(
                state.clone(),
                Divider::LeftPairSplit,
                window_width,
                x0 + 50.0,
            );
            let far = apply_column_drag(state, Divider::LeftPairSplit, window_width, x0 + 250.0);
            assert!(
                far.files_split < near.files_split,
                "镜像态下方向应反转:near={} far={}",
                near.files_split,
                far.files_split
            );
        }

        #[test]
        fn project_split_direction_on_default_side_matches_pre_migration_behavior() {
            let state = test_state();
            let window_width = 1600.0;
            let near = apply_column_drag(state.clone(), Divider::ProjectSplit, window_width, 300.0);
            let far = apply_column_drag(state, Divider::ProjectSplit, window_width, 500.0);
            assert!(
                far.project_split > near.project_split,
                "near={} far={}",
                near.project_split,
                far.project_split
            );
        }

        #[test]
        fn project_split_direction_flips_when_relocated_to_right_side() {
            let mut state = test_state();
            relocate_to_right(&mut state, PanelKind::Project);
            let window_width = 1600.0;
            let x0 = right_x0_inside(window_width, &state);
            let near = apply_column_drag(
                state.clone(),
                Divider::ProjectSplit,
                window_width,
                x0 + 50.0,
            );
            let far = apply_column_drag(state, Divider::ProjectSplit, window_width, x0 + 250.0);
            assert!(
                far.project_split < near.project_split,
                "镜像态下方向应反转:near={} far={}",
                near.project_split,
                far.project_split
            );
        }

        #[test]
        fn browser_bookmarks_split_direction_on_default_side_matches_pre_migration_behavior() {
            // Web 默认"内容在前"且字段存内容占比:默认栏(左)下拖拽点越靠右,
            // 内容占比越大(与既有 `browser_bookmarks_split_direction_matches_content_side`
            // 一致的方向锚)。
            let state = test_state();
            let window_width = 1600.0;
            let near = apply_column_drag(
                state.clone(),
                Divider::BrowserBookmarksSplit,
                window_width,
                100.0,
            );
            let far = apply_column_drag(state, Divider::BrowserBookmarksSplit, window_width, 500.0);
            assert!(
                far.browser_bookmarks_split > near.browser_bookmarks_split,
                "near={} far={}",
                near.browser_bookmarks_split,
                far.browser_bookmarks_split
            );
        }

        #[test]
        fn browser_bookmarks_split_direction_flips_when_relocated_to_right_side() {
            let mut state = test_state();
            relocate_to_right(&mut state, PanelKind::Web);
            let window_width = 1600.0;
            let x0 = right_x0_inside(window_width, &state);
            let near = apply_column_drag(
                state.clone(),
                Divider::BrowserBookmarksSplit,
                window_width,
                x0 + 50.0,
            );
            let far = apply_column_drag(
                state,
                Divider::BrowserBookmarksSplit,
                window_width,
                x0 + 250.0,
            );
            assert!(
                far.browser_bookmarks_split < near.browser_bookmarks_split,
                "镜像态下方向应反转:near={} far={}",
                near.browser_bookmarks_split,
                far.browser_bookmarks_split
            );
        }
    }

    /// `apply_column_drag` 里 `RightPairSplit`(Agent/Conversations)的
    /// side+镜像感知改造测试。默认在右栏、默认"内容在前"——默认栏下拖拽点
    /// 越靠右,内容占比越大、列表占比越*小*;挪到左栏后镜像成"列表在前",
    /// 方向反转。
    mod apply_column_drag_right_pair_mirror_tests {
        use super::*;

        #[test]
        fn agent_split_direction_on_default_side_matches_pre_migration_behavior() {
            let state = test_state(); // right_view 已经是 Agent
            let window_width = 1600.0;
            let right_x0 = window_width
                - byteui::theme::geometry::icon_rail_width()
                - right_zone_width(window_width, &state);
            let near = apply_column_drag(
                state.clone(),
                Divider::RightPairSplit,
                window_width,
                right_x0 + 50.0,
            );
            let far = apply_column_drag(
                state,
                Divider::RightPairSplit,
                window_width,
                right_x0 + 250.0,
            );
            assert!(
                far.agent_split < near.agent_split,
                "near={} far={}",
                near.agent_split,
                far.agent_split
            );
        }

        #[test]
        fn agent_split_direction_flips_when_relocated_to_left_side() {
            let mut state = test_state();
            state
                .layout
                .rail_layout
                .right
                .retain(|&k| k != PanelKind::Agent);
            state.layout.rail_layout.left.push(PanelKind::Agent);
            let window_width = 1600.0;
            let near = apply_column_drag(
                state.clone(),
                Divider::RightPairSplit,
                window_width,
                byteui::theme::geometry::icon_rail_width() + 50.0,
            );
            let far = apply_column_drag(
                state,
                Divider::RightPairSplit,
                window_width,
                byteui::theme::geometry::icon_rail_width() + 250.0,
            );
            assert!(
                far.agent_split > near.agent_split,
                "镜像态下方向应反转:near={} far={}",
                near.agent_split,
                far.agent_split
            );
        }

        #[test]
        fn conversations_split_direction_on_default_side_matches_pre_migration_behavior() {
            let mut state = test_state();
            state.right_view = PanelKind::Conversations;
            let window_width = 1600.0;
            let right_x0 = window_width
                - byteui::theme::geometry::icon_rail_width()
                - right_zone_width(window_width, &state);
            let near = apply_column_drag(
                state.clone(),
                Divider::RightPairSplit,
                window_width,
                right_x0 + 50.0,
            );
            let far = apply_column_drag(
                state,
                Divider::RightPairSplit,
                window_width,
                right_x0 + 250.0,
            );
            assert!(
                far.conversations_split < near.conversations_split,
                "near={} far={}",
                near.conversations_split,
                far.conversations_split
            );
        }

        #[test]
        fn conversations_split_direction_flips_when_relocated_to_left_side() {
            let mut state = test_state();
            state.right_view = PanelKind::Conversations;
            state
                .layout
                .rail_layout
                .right
                .retain(|&k| k != PanelKind::Conversations);
            state.layout.rail_layout.left.push(PanelKind::Conversations);
            let window_width = 1600.0;
            let near = apply_column_drag(
                state.clone(),
                Divider::RightPairSplit,
                window_width,
                byteui::theme::geometry::icon_rail_width() + 50.0,
            );
            let far = apply_column_drag(
                state,
                Divider::RightPairSplit,
                window_width,
                byteui::theme::geometry::icon_rail_width() + 250.0,
            );
            assert!(
                far.conversations_split > near.conversations_split,
                "镜像态下方向应反转:near={} far={}",
                near.conversations_split,
                far.conversations_split
            );
        }
    }
}
