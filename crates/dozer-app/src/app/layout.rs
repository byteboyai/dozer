//! 布局/几何类型与纯函数:面板区尺寸、可拖拽分隔线、页签拖拽阈值、配对
//! 视图分割比例、面板区/终端 pane 的几何换算。Phase 2 结构重组时从
//! `app.rs` 抽出,逻辑保持原样。

use serde::{Deserialize, Serialize};

use crate::extensions::project;
use crate::rail;
use crate::theme;

use super::state::{
    MaximizedPane, PanelKind, ShellState, Side, ZoneSide, clamp_left_width, left_zone_width,
    pair_content_width, right_zone_width,
};

/// 图标栏+左右面板区的宽度/分割状态。取代 `PanelLayout`——不再有"项目栏/AI栏
/// 固定宽+预览终端共享比例"这套四栏几何，改成"左面板区总宽(可拖) + 三个
/// 配对视图各自独立记住的内部列表:内容分割比例"。右面板区总宽不持久化，
/// 恒为剩余空间(`Length::Fill`)——只有一条 LeftRight 分隔线，不需要像旧
/// 模型那样两个固定宽度各自夹一条。
///
/// `#[serde(default)]`:今后加字段时,老 `layout.json` 里缺的字段用
/// `Default` 补齐,而不是整份反序列化失败 → `unwrap_or_default()` 把用户
/// 攒下来的宽度/比例全部重置。
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
pub(crate) fn default_panel_dims() -> PanelDims {
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
pub(crate) const TAB_DRAG_CONFIRM_THRESHOLD_PX: f32 = 4.0;

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
pub(crate) fn tab_drag_past_threshold(press_pos: (f32, f32), cursor: (f32, f32)) -> bool {
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
pub(crate) const TREE_DRAG_CONFIRM_THRESHOLD_PX: f32 = 12.0;

/// `drag.press_pos` 到 `cursor` 的位移是否已越过 [`TREE_DRAG_CONFIRM_THRESHOLD_PX`]。
/// `files::Message::TreeRowPress`(见其文档)"按下即武装"是同一个已知会
/// 抖动的模式(同 `tab_drag_past_threshold` 修的那个 bug):单击落点到抬起
/// 之间的亚像素抖动偶尔会越界到邻居目录行的命中框,`TreeDragOver` 若不设
/// 这道门槛会把这次抖动当成"拖到了旁边那一行",直接判定合法性并可能提交
/// 移动——2026-09 用户实测反馈:点一下目录就报"不能把目录移到它自己或其
/// 子目录里"。`App::update` 收到 `TreeDragOver` 时先过这道阈值再转发给
/// `files::update()`,未越过就整条丢弃,`drag.target` 保持原值不变。
pub(crate) fn tree_drag_past_threshold(press_pos: (f32, f32), cursor: (f32, f32)) -> bool {
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
pub(crate) const TREE_DRAG_MIN_HOLD_DURATION: std::time::Duration =
    std::time::Duration::from_millis(300);

/// 按下到松开的 `elapsed` 是否已达到 [`TREE_DRAG_MIN_HOLD_DURATION`]——同
/// `tree_drag_past_threshold` 一起、两者都满足才判定为一次真实拖拽(见
/// `Message::TreeDragEnd` 的 `confirmed` 文档),缺一不可:纯距离挡不住
/// 快速划动的误判,纯时长又会让"按住不动很久"被误判成拖拽却没有合法
/// 目标。
pub(crate) fn tree_drag_held_long_enough(elapsed: std::time::Duration) -> bool {
    elapsed > TREE_DRAG_MIN_HOLD_DURATION
}

/// 判断某一侧的 webview 是否要因为**面板自身内部**的原生浮层被强制隐藏
/// ——`preview_desired` 里 `app_modal_open`/`tab_overflow_open` 两种"整块
/// 面板级"场景已经在调用处单独合并,这里补的是"面板本身还在,但面板内
/// 某个 `crate::menu` 弹层可能压住 webview 内容区"的场景:Files 面板的
/// 文件树右键菜单、Project 面板的链接行右键菜单、Conversations 面板的
/// agent 筛选下拉。原生 wry 子视图不听 iced 绘制顺序摆布,只能靠调用方
/// 显式把 `WebviewSpec.visible` 置 `false` 才能让浮层真正盖住它。
pub(crate) fn webview_hidden_by_panel_popup(
    kind: PanelKind,
    files_context_menu_open: bool,
    project_link_menu_open: bool,
    conversations_agent_picker_open: bool,
) -> bool {
    match kind {
        PanelKind::Files => files_context_menu_open,
        PanelKind::Project => project_link_menu_open,
        PanelKind::Conversations => conversations_agent_picker_open,
        _ => false,
    }
}
/// 配对视图内部"列表侧"与"内容侧"的宽度,按 `split`(列表侧占比)从
/// `pair_w` 分出。四个左右双栏 zone(左:文件/项目,右:Agent/对话)统一走
/// 这一份公式:`split` 恒代表列表侧占比,内容侧拿剩下的 `1 - split`。
/// 渲染/几何/预览 bounds 三侧都要共用,不许各写各的 `pair_w * split` /
/// `pair_w * (1.0 - split)`,否则一处改动、别处漂移(见 `pair_content_width`
/// 同条原则)。
pub(crate) fn pair_list_content_width(pair_w: f32, split: f32) -> (f32, f32) {
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
    use crate::app::PROJECT_PREVIEW_ID_OFFSET;

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
pub(crate) fn list_rendered_first(default_list_first: bool, mirrored: bool) -> bool {
    default_list_first != mirrored
}

/// 给定 `PanelKind`,取它在 `PanelDims` 里对应的配对分割比例字段——统一
/// 口径是"pair 内第一个 slot 的占比"(`pair_list_content_width` 的
/// `split` 参数,不区分这个 slot 语义上是"列表"还是"内容",Browser 的
/// `browser_bookmarks_split` 反着命名也是同一套算法)。
pub(crate) fn pair_split_ratio(dims: &PanelDims, kind: PanelKind) -> Option<f32> {
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
pub(crate) fn with_pair_split_ratio(dims: PanelDims, kind: PanelKind, ratio: f32) -> PanelDims {
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
            // 拖窄到小于 `project::footer_min_width`(所有可收纳面板统一走
            // 这一份估算,不各面板各算一套)时直接收起文件树列表子栏,语义同
            // `Divider::ProjectSplit` 那条分支(冻结 `files_split`、对称
            // 可逆),只是这里翻的是 `files_tree_collapsed`。
            let (list_w, _) = pair_list_content_width(pair_w, ratio);
            if list_w < project::footer_min_width() {
                PanelDims {
                    files_tree_collapsed: true,
                    ..state.dims
                }
            } else {
                PanelDims {
                    files_split: ratio,
                    files_tree_collapsed: false,
                    ..state.dims
                }
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
            // 信息面板(footer「修复项目/删除项目」两个按钮所在栏)拖窄到
            // 小于 `project::footer_min_width` 时直接收起——等同用户点了
            // 收起按钮(见 `toggle_panel_list_collapse`):冻结 `state.dims`
            // 里上一次仍够宽的 `project_split`,不让它被拖成挤爆按钮的小
            // 数值;拖回超过阈值时用当前光标位置连续算出新比例并展开,
            // 对称、可逆(拖拽是逐帧调用本函数,不是一次性判定)。这份估算是
            // 所有可收纳面板(Files/Ssh/Database/Todo/Usage/Agent/
            // Conversations)统一的最小宽度基准,不是 Project 专属——按用户
            // 要求"所有面板最小宽度都和项目面板一样,就是两个按钮的宽度"。
            let (list_w, _) = pair_list_content_width(pair_w, ratio);
            if list_w < project::footer_min_width() {
                PanelDims {
                    project_list_collapsed: true,
                    ..state.dims
                }
            } else {
                PanelDims {
                    project_split: ratio,
                    project_list_collapsed: false,
                    ..state.dims
                }
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
            // 拖窄到小于 `project::footer_min_width` 时直接收起主机列表,
            // 语义同 `Divider::ProjectSplit` 那条分支。
            let (list_w, _) = pair_list_content_width(pair_w, ratio);
            if list_w < project::footer_min_width() {
                PanelDims {
                    ssh_list_collapsed: true,
                    ..state.dims
                }
            } else {
                PanelDims {
                    ssh_split: ratio,
                    ssh_list_collapsed: false,
                    ..state.dims
                }
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
            // 拖窄到小于 `project::footer_min_width` 时直接收起 schema 树,
            // 语义同 `Divider::ProjectSplit` 那条分支。
            let (list_w, _) = pair_list_content_width(pair_w, ratio);
            if list_w < project::footer_min_width() {
                PanelDims {
                    database_list_collapsed: true,
                    ..state.dims
                }
            } else {
                PanelDims {
                    database_split: ratio,
                    database_list_collapsed: false,
                    ..state.dims
                }
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
            // 拖窄到小于 `project::footer_min_width` 时直接收起 agent 筛选栏,
            // 语义同 `Divider::ProjectSplit` 那条分支。
            let (list_w, _) = pair_list_content_width(pair_w, ratio);
            if list_w < project::footer_min_width() {
                PanelDims {
                    usage_list_collapsed: true,
                    ..state.dims
                }
            } else {
                PanelDims {
                    usage_split: ratio,
                    usage_list_collapsed: false,
                    ..state.dims
                }
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
            // 拖窄到小于 `project::footer_min_width` 时直接收起分类导航栏,
            // 语义同 `Divider::ProjectSplit` 那条分支。
            let (list_w, _) = pair_list_content_width(pair_w, ratio);
            if list_w < project::footer_min_width() {
                PanelDims {
                    todo_list_collapsed: true,
                    ..state.dims
                }
            } else {
                PanelDims {
                    todo_split: ratio,
                    todo_list_collapsed: false,
                    ..state.dims
                }
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
            // 拖窄到小于 `project::footer_min_width` 时直接收起列表侧,语义同
            // `Divider::ProjectSplit` 那条分支——`kind` 决定翻哪个
            // `*_list_collapsed` 字段。
            let (list_w, _) = pair_list_content_width(pair_w, ratio);
            match kind {
                PanelKind::Agent => {
                    if list_w < project::footer_min_width() {
                        PanelDims {
                            agent_list_collapsed: true,
                            ..state.dims
                        }
                    } else {
                        PanelDims {
                            agent_split: ratio,
                            agent_list_collapsed: false,
                            ..state.dims
                        }
                    }
                }
                PanelKind::Conversations => {
                    if list_w < project::footer_min_width() {
                        PanelDims {
                            conversations_list_collapsed: true,
                            ..state.dims
                        }
                    } else {
                        PanelDims {
                            conversations_split: ratio,
                            conversations_list_collapsed: false,
                            ..state.dims
                        }
                    }
                }
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
pub(crate) const UI_ZOOM_STEP: f32 = 1.1;
