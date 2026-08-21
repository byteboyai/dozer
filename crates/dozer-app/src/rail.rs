// crates/dozer-app/src/rail.rs
//! 图标栏(Rail):11 个面板挂载的两条侧栏,支持点击切换、同栏重排、跨栏
//! 拖拽换边。类型 + 纯逻辑 + 槽位动画 + 渲染都在这个模块——不是
//! `extensions/` 那种私有 Message+State+update+view 的 extension 形态,
//! `rail_layout`/`rail_drag`/`rail_slot_anims` 三个字段仍然挂在
//! `App`/`ShellLayout` 上(多消费方共享数据,内核持有),这里只是把纯
//! Rail 逻辑物理搬出 `app.rs`。见
//! `docs/superpowers/specs/2026-08-21-rail-extraction-pilot-design.md`。

use crate::app::{App, HoverId, Message, PanelKind, Side};
use crate::theme;
use byteui::interaction::icons;
use iced_widget::core::mouse;
use iced_widget::core::{Border, Color, Element, Length, Padding};
use iced_widget::{MouseArea, button, column, container, stack};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 四个图标栏按钮的标识,用于追踪 hover 态(图标颜色在 hover 时需变金,
/// 而 SVG 颜色在构建时就定死、不随 `button::Status` 变化,所以得在 App
/// 里记一个 hovered 目标,改色时按它重算)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RailButton {
    Panel(PanelKind),
    /// 首页左栏"项目列表" pane 图标。
    HomeProjectList,
    /// 首页左栏"Recents" pane 图标。
    HomeRecents,
    /// 首页右栏"浏览器" pane 图标。
    HomeBrowser,
}

/// 正在进行的图标栏面板拖拽(同栏重排 / 跨栏移动)。语义、生命周期管理
/// 手法照抄 `TabDrag`,但不复用它——`TabDrag`/`TabGroup` 是"同组内换位",
/// 图标栏这次还要支持"跨栏移动",合并进同一个类型会让校验逻辑变复杂。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RailDrag {
    pub source_side: Side,
    pub source_index: usize,
    /// 拖拽开始时 `source_index` 的原始值,不在拖拽期间随同栏重排更新。
    /// `end_rail_drag` 用它判断纯同栏重排是否真的发生过(优先级重排会
    /// 推高 `source_index`,未发生则保持原值),决定要不要把新顺序落盘。
    pub origin_index: usize,
    /// 悬停到另一栏时记录目标位置;`RailDragEnd` 才真正提交搬移,悬停
    /// 期间不搬、不落盘。悬停回源栏(或还没悬停到任何另一栏位置)时是
    /// `None`。
    pub pending_cross_side: Option<(Side, usize)>,
    /// 按下瞬间的 `App::last_cursor`,拖拽期间不更新——`rail_drag_confirmed`
    /// 用它和当前光标算位移,判断这是不是"真的在拖"(见其定义)。之所以
    /// 不能靠 `dragging_rail()`(`rail_drag.is_some()`)本身:那个从按下
    /// 瞬间就为真(见 `rail_drag_surface` 文档解释的"按下即武装"原因),
    /// 快速单击(按下几乎立刻松开、光标几乎不动)也会先武装再清空,若视觉
    /// (幽灵图标/源图标变淡/抓手光标)直接跟 `dragging_rail()` 走,会在
    /// 单击时闪一下"进入拖拽"的效果。
    pub press_pos: (f32, f32),
}

/// 每个面板当前挂在哪条图标栏、栏内什么顺序——图标栏拖拽换栏(Stage 4)
/// 的唯一真相源。这个 Stage 只负责定义类型 + 持久化,渲染/交互还没有
/// 任何地方读它(那是 Stage 2/4 的范围),所以此刻它的值必然等于
/// `default()`,不会有别的取值出现。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RailLayout {
    pub left: Vec<PanelKind>,
    pub right: Vec<PanelKind>,
}

impl RailLayout {
    /// 返回某一侧图标栏当前挂载的面板列表(渲染顺序)。
    pub fn side(&self, side: Side) -> &Vec<PanelKind> {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    #[allow(dead_code)]
    pub fn side_mut(&mut self, side: Side) -> &mut Vec<PanelKind> {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    /// 给定面板,反查它当前挂在哪条栏。`RailLayout` 的不变式(见
    /// `sanitize_rail_layout`)保证 11 个面板不重不漏分布在两条栏,
    /// 所以这里的 `expect` 不会在合法状态下触发——`RailLayout` 一旦
    /// 通不过消毒就已经在 `layout::load_from` 里回落 `default()` 了,
    /// 不会带着"某个面板哪条栏都不在"的坏数据流到这里。
    pub fn side_of(&self, kind: PanelKind) -> Side {
        if self.left.contains(&kind) {
            Side::Left
        } else if self.right.contains(&kind) {
            Side::Right
        } else {
            unreachable!(
                "RailLayout 不变式被破坏:{kind:?} 不在任何一条栏——\
                 sanitize_rail_layout 应该已经挡掉这种坏数据"
            )
        }
    }
}

/// 面板当前是否偏离了默认栏(`side_of(kind) != default_side()`)。抽成
/// 自由函数单纯是为了让单元测试不必构造一个完整 `App`(它需要 `Client`/
/// 事件循环钩子)——逻辑本身和 `App::panel_mirrored` 完全一致,后者只是
/// 把自己的 `rail_layout` 喂进来。
pub(crate) fn panel_mirrored_in(rail: &RailLayout, kind: PanelKind) -> bool {
    rail.side_of(kind) != kind.default_side()
}

impl Default for RailLayout {
    fn default() -> Self {
        Self {
            left: vec![
                PanelKind::Project,
                PanelKind::Todo,
                PanelKind::Files,
                PanelKind::GitLog,
                PanelKind::Database,
                PanelKind::Ssh,
                PanelKind::Web,
            ],
            right: vec![
                PanelKind::Agent,
                PanelKind::Conversations,
                PanelKind::Usage,
                PanelKind::Acceptance,
            ],
        }
    }
}

/// `RailLayout` 的消毒:任一栏为空,或两侧合计不是恰 11 个不重复的
/// `PanelKind`(手改/版本不一致导致的坏数据),整个回落 `default()`。
/// 不做部分修复——缺一个面板就补在默认栏这种中间态比"直接用默认值"
/// 更难排查。
pub(crate) fn sanitize_rail_layout(rail: RailLayout) -> RailLayout {
    if rail.left.is_empty() || rail.right.is_empty() {
        return RailLayout::default();
    }
    let mut all: Vec<_> = rail.left.iter().chain(rail.right.iter()).collect();
    all.sort_by_key(|k| format!("{k:?}"));
    all.dedup();
    if all.len() != 11 || rail.left.len() + rail.right.len() != 11 {
        return RailLayout::default();
    }
    rail
}

/// 图标栏拖拽"移动到 `side` 栏第 `to` 位"的纯逻辑核心:不依赖 `App` 的
/// 其他字段,抽成自由函数以便单元测试直接构造 `RailLayout` 验证(同 Stage 3
/// `panel_mirrored_in` 的做法的理由——`App` 需要 `Client`/事件循环钩子,
/// 构造成本高)。
///
/// 同栏(*`drag.source_side == side`*):立即重排(`Vec::remove`+`insert`),
/// 并把 `drag.source_index` 更新为新的源位置、清掉任何跨栏悬停残留。跨栏:
/// 只记 `drag.pending_cross_side`,具体搬移留给 `rail_cross_apply`/`App::
/// `end_rail_drag` 统一提交——避免每帧 `CursorMoved` 都触发一次 `Vec` 搬移
/// 和后续的布局存盘。
pub(crate) fn rail_drag_move_into(
    rail: &mut RailLayout,
    drag: &mut RailDrag,
    side: Side,
    to: usize,
) {
    if drag.source_side == side {
        // 光标回到源栏:不再悬停另一栏,先取消可能残留的跨栏悬停目标,
        // 再做同栏内重排。语义上"悬停回源栏"就撤销了"将要跨栏"的意图。
        drag.pending_cross_side = None;
        let panels = rail.side_mut(side);
        let from = drag.source_index;
        if from == to || from >= panels.len() || to >= panels.len() {
            return;
        }
        let kind = panels.remove(from);
        panels.insert(to, kind);
        drag.source_index = to;
    } else {
        drag.pending_cross_side = Some((side, to));
    }
}

/// 跨栏移动的“真正落地”纯逻辑:把 `source_side` 第 `source_index` 个面板
/// 搬到 `target_side` 第 `target_index` 位,返回被移动的面板;源栏只剩这一个
/// 时(搬走会清空,不支持“栏清空”,见 spec 非目标)或源下标越界时返回
/// `None`、`rail` 不被改动。目标下标越界时 clamp 到末尾。
pub(crate) fn rail_cross_apply(
    rail: &mut RailLayout,
    source_side: Side,
    source_index: usize,
    target_side: Side,
    target_index: usize,
) -> Option<PanelKind> {
    let source_panels = rail.side(source_side);
    if source_side == target_side || source_panels.len() <= 1 || source_index >= source_panels.len()
    {
        return None;
    }
    let kind = rail.side_mut(source_side).remove(source_index);
    let target_index = target_index.min(rail.side(target_side).len());
    rail.side_mut(target_side).insert(target_index, kind);
    Some(kind)
}

/// 当前正被图标栏拖拽的面板种类(`None` = 未在拖拽)。纯查询,不修改
/// `rail`/`drag`——拖拽中同栏重排会实时更新 `drag.source_index`(见
/// `rail_drag_move_into`),所以这里查到的永远是"此刻鼠标下真正拖着的
/// 那个图标",不是拖拽开始时的原始位置。下标越界(理论不会发生,防御性)
/// 时返回 `None`,不 panic。
pub(crate) fn dragged_panel_kind(rail: &RailLayout, drag: Option<RailDrag>) -> Option<PanelKind> {
    let drag = drag?;
    rail.side(drag.source_side).get(drag.source_index).copied()
}

/// 图标栏拖拽从"按下武装"到"视觉判定为一次真的拖拽"所需的最小位移
/// (像素,窗口逻辑坐标系,同 `App::last_cursor`)。低于这个距离只是武装
/// 态,不展示幽灵图标/源图标变淡/抓手光标——见 `RailDrag::press_pos` 字段
/// 文档解释的"快速单击也会先武装"问题。数值对齐常见桌面 OS 的点击/拖拽
/// 判定阈值。
const RAIL_DRAG_VISUAL_THRESHOLD_PX: f32 = 4.0;

/// `drag` 从武装(按下)到 `cursor`(当前 `App::last_cursor`)是否已经
/// 越过 [`RAIL_DRAG_VISUAL_THRESHOLD_PX`]——越过才算"确认是一次拖拽,不是
/// 单击",视图层据此决定要不要展示拖拽视觉。纯查询,不修改 `drag`。
pub(crate) fn rail_drag_past_threshold(drag: RailDrag, cursor: (f32, f32)) -> bool {
    let dx = cursor.0 - drag.press_pos.0;
    let dy = cursor.1 - drag.press_pos.1;
    dx * dx + dy * dy > RAIL_DRAG_VISUAL_THRESHOLD_PX * RAIL_DRAG_VISUAL_THRESHOLD_PX
}

/// 图标栏按钮的动画槽位状态机:`current` 是本帧渲染用的浮点槽位号(在
/// `rail_layout` 里的下标,逼近 `target` 中),`side` 记录上一次逼近所在的
/// 栏——同栏内 `target` 变化(重排让位)时正常指数逼近,平滑滑动;`side`
/// 本身变化(跨栏移动)时说明这是两条完全不同的物理列,`current`/`target`
/// 数值上的"接近"没有几何意义,`retarget` 直接 snap 到新 `target`,不生成
/// 滑动动画。逼近手法与 `HoverAnim` 同源,只是目标值域从"0..=1 悬停进度"
/// 换成"任意非负槽位号"。
#[derive(Debug, Clone, Copy)]
pub(crate) struct RailSlotAnim {
    current: f32,
    side: Side,
}

impl RailSlotAnim {
    /// 把这个按钮的目标槽位设成 `(side, target)`,必要时朝它逼近一拍。
    /// `side` 与上次不同(跨栏移动)时直接 snap,不留一帧"跨列插值"的
    /// 视觉噪音。
    pub(crate) fn retarget(&mut self, side: Side, target: f32) {
        if self.side != side {
            self.side = side;
            self.current = target;
            return;
        }
        let next = self.current + (target - self.current) * 0.5;
        self.current = if (next - target).abs() < 0.02 {
            target
        } else {
            next
        };
    }
    /// 动画是否仍在进行中(某按钮的槽位还没收敛到目标)。调用方需要先把
    /// `target` 通过 `retarget` 写入才能得到有意义的结果——这个方法只读
    /// 当前状态,不推进。
    pub(crate) fn active(&self, target: f32) -> bool {
        (self.current - target).abs() > 0.005
    }
}

/// 推进图标栏按钮的槽位动画一拍——两侧各自按 `rail_layout` 当前顺序
/// 现算每个面板的目标槽位号,`RailSlotAnim::retarget` 朝它逼近。
pub(crate) fn advance_slot_anims(rail: &RailLayout, anims: &mut HashMap<PanelKind, RailSlotAnim>) {
    for side in [Side::Left, Side::Right] {
        for (idx, &kind) in rail.side(side).iter().enumerate() {
            let target = idx as f32;
            anims
                .entry(kind)
                .or_insert(RailSlotAnim {
                    current: target,
                    side,
                })
                .retarget(side, target);
        }
    }
}

/// 是否还有图标栏按钮的槽位动画在进行中。
pub(crate) fn any_slot_anim_active(
    rail: &RailLayout,
    anims: &HashMap<PanelKind, RailSlotAnim>,
) -> bool {
    [Side::Left, Side::Right].into_iter().any(|side| {
        rail.side(side).iter().enumerate().any(|(idx, &kind)| {
            anims
                .get(&kind)
                .is_some_and(|a| a.side == side && a.active(idx as f32))
        })
    })
}

/// `kind` 在 `side` 栏当前应渲染的动画槽位号(浮点,逼近中的
/// `rail_layout` 下标)。渲染层据此算按钮的 y 偏移,取代直接用
/// `rail_layout` 下标瞬间跳变。
pub(crate) fn slot_position(
    anims: &HashMap<PanelKind, RailSlotAnim>,
    side: Side,
    kind: PanelKind,
    target_idx: usize,
) -> f32 {
    let target = target_idx as f32;
    anims
        .get(&kind)
        .filter(|a| a.side == side)
        .map(|a| a.current)
        .unwrap_or(target)
}

/// 单个图标栏按钮：圆角正方形背景常驻,hover 图标变金(无金框),选中图标
/// 变金且带金色外框。
pub(crate) fn rail_icon_button<'a>(
    icon: icons::IconKind,
    active: bool,
    hover_t: f32,
    msg: Message,
    tooltip: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 图标颜色:选中态恒为金;未选中时 hover 平滑过渡到金(见 `HoverId`/
    // `App::hover_progress`——与光标闪烁同款自驱 redraw 动画)。SVG 颜色
    // 构建时定死、不吃 `button::Status`,所以 hover 进度靠 `hover_t` 参数从
    // App 算进来。
    let color = if active {
        byteui::theme::color::current().gold
    } else {
        byteui::theme::color::mix(
            byteui::theme::color::current().dim,
            byteui::theme::color::current().gold,
            hover_t,
        )
    };
    let inner = container(icons::view(icon, byteui::theme::icon_size::rail(), color))
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

    let content: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = button(inner)
        .on_press(msg)
        .width(Length::Fixed(byteui::theme::geometry::rail_button_size()))
        .height(Length::Fixed(byteui::theme::geometry::rail_button_size()))
        .padding(0)
        .style(move |_t: &iced_widget::Theme, _status: button::Status| {
            // 圆角正方形背景常驻(`CARD`);金色外框只在选中态出现,hover
            // 不放金框——所以样式完全由 `active` 决定,与交互态无关。
            button::Style {
                background: Some(byteui::theme::color::current().card.into()),
                border: Border {
                    color: if active {
                        byteui::theme::color::current().gold
                    } else {
                        Color::TRANSPARENT
                    },
                    ..base_border
                },
                ..button::Style::default()
            }
        })
        .into();
    icons::with_tooltip(content, tooltip)
}

/// 图标栏:按 `app.shell_layout.rail_layout.side(side)` 的顺序遍历渲染。
/// 左右两条栏共用这一份实现——差异(区域样式、选中态取哪个
/// `*_view`/`*_collapsed` 字段判断)通过 `side` 参数分派。
pub(crate) fn icon_rail(
    app: &App,
    side: Side,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = match side {
        Side::Left => theme::region::left_icon_rail(),
        Side::Right => theme::region::right_icon_rail(),
    };
    // 视觉"选中"= 该视图激活 **且**对应面板区展开。点已选中的图标会收起
    // 面板区,此时图标要退回未选中态,所以 `active` 得带上 `!collapsed`
    // ——语义同拆分前的两条原图标栏函数。
    let (active_kind, open) = match side {
        Side::Left => (app.left_view, !app.left_collapsed),
        Side::Right => (app.right_view, !app.right_collapsed),
    };
    // 每个按钮各占一个绝对定位的 `stack!` 图层,纵向偏移按
    // `App::rail_slot_position` 算出的动画槽位号换算像素——取代原先的
    // `column!`(严格按 `rail_layout` 下标顺序摆、换位瞬间跳变),让同栏
    // 拖拽重排时让位的相邻按钮能平滑滑动到新槽位,而不是硬切。
    let button_size = byteui::theme::geometry::rail_button_size();
    let step = button_size + region.gap;
    let mut layers = Vec::new();
    for (idx, &kind) in app.shell_layout.rail_layout.side(side).iter().enumerate() {
        let (icon, tooltip) = panel_meta(kind);
        // `interactive: false`——按下选中不走这里内层的
        // `iced_widget::button::on_press`(松手才触发,时机不对,见
        // `rail_drag_surface` 的注释),改由外层 `rail_drag_surface` 的
        // `MouseArea::on_press` 接管,`on_select` 参数这里只是占位不会被
        // 内部真正接线,原样传 `Message::PanelSelect(kind)` 保持调用方
        // 语义一致。视觉(选中金框/hover 渐变)不受 `interactive` 影响,
        // 只有交互接线这一步被跳过。
        let base: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            icons::icon_button_entry(
                icon,
                byteui::theme::icon_size::rail(),
                kind == active_kind && open,
                app.rail_drag_confirmed() && Some(kind) == app.dragged_panel_kind(),
                app.hover_progress(HoverId::Rail(RailButton::Panel(kind))),
                true,
                button_size,
                false,
                Message::PanelSelect(kind),
                move |hovered| Message::Hover(HoverId::Rail(RailButton::Panel(kind)), hovered),
                tooltip,
            );
        let entry = match panel_badge(app, kind) {
            Some(badge) => stack![base, badge].into(),
            None => base,
        };
        let y = region.padding.top + app.rail_slot_position(side, kind, idx) * step;
        let positioned = container(rail_drag_surface(
            entry,
            side,
            idx,
            app.rail_drag_confirmed(),
            Message::PanelSelect(kind),
        ))
        .padding(Padding {
            top: y,
            left: region.padding.left,
            right: region.padding.right,
            bottom: 0.0,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Left)
        .align_y(iced_widget::core::alignment::Vertical::Top);
        layers.push(positioned.into());
    }

    // 拖拽悬停(同栏重排 / 跨栏悬停)时,在预计插入点画一条金色插入线,
    // 两种情况互斥(`pending_cross_side` 只在悬停到*另一*栏时才 `Some`),
    // 同一帧同一侧最多画一条:
    // - 跨栏悬停到本栏:插入点是 `pending_cross_side.1`——`rail_cross_
    //   apply` 落地时是 `insert`(把已有项推后一位),不是跟目标位的按钮
    //   互换,所以高亮画成"卡在两个按钮之间的线",不描边某个已存在按钮
    //   (那样会误导成"要跟它换位")。这个下标恒是目标栏某个已有按钮自己
    //   上报的下标(见 `rail_drag_surface` 的 `on_move` 只挂在真实按钮
    //   上),不会是 `len()`(悬停不到"最后一个之后"这个位置——现有交互
    //   面就是如此,不是这次新引入的限制)。
    // - 同栏内拖拽重排:插入点是 `drag.source_index`——同栏分支的
    //   `rail_drag_move_into` 已经把 `RailLayout`/`source_index` 实时改到
    //   目标位(不像跨栏要等 `RailDragEnd` 才落地),所以这里不是"预告",
    //   是"跟当前已生效的顺序对齐"的同一条线,视觉语言与跨栏悬停统一。
    // 两种情况都只在越过点击/拖拽视觉阈值(`rail_drag_confirmed`)后才
    // 画,理由同幽灵图标/源图标变淡——避免快速单击也闪一下插入线。
    let insertion_idx = app
        .rail_drag
        .filter(|_| app.rail_drag_confirmed())
        .and_then(|d| match d.pending_cross_side {
            Some((cross_side, idx)) => (cross_side == side).then_some(idx),
            None => (d.source_side == side).then_some(d.source_index),
        });
    if let Some(idx) = insertion_idx {
        let bar_h = 3.0;
        let y = (region.padding.top + idx as f32 * step - region.gap / 2.0 - bar_h / 2.0).max(0.0);
        let marker = container(iced_widget::Space::new())
            .width(Length::Fixed(button_size))
            .height(Length::Fixed(bar_h))
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().gold.into()),
                ..container::Style::default()
            });
        let positioned = container(marker)
            .padding(Padding {
                top: y,
                left: region.padding.left,
                right: region.padding.right,
                bottom: 0.0,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced_widget::core::alignment::Horizontal::Left)
            .align_y(iced_widget::core::alignment::Vertical::Top);
        layers.push(positioned.into());
    }

    let content = iced_widget::Stack::with_children(layers)
        .width(Length::Fill)
        .height(Length::Fill);
    container(content)
        .width(Length::Fixed(byteui::theme::geometry::icon_rail_width()))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}

/// 面板 → (图标, 图标栏 tooltip 文案)。11 个 `PanelKind` variant 逐一
/// 对应,顺序与 `PanelKind` 定义顺序一致,不代表渲染顺序(渲染顺序看
/// `RailLayout`)。
fn panel_meta(kind: PanelKind) -> (icons::IconKind, &'static str) {
    match kind {
        PanelKind::Files => (icons::IconKind::FolderTree, "文件"),
        PanelKind::GitLog => (icons::IconKind::GitGraph, "Git 提交"),
        PanelKind::Todo => (icons::IconKind::ListTodo, "待办"),
        PanelKind::Project => (icons::IconKind::Briefcase, "项目"),
        PanelKind::Database => (icons::IconKind::Database, "数据库"),
        PanelKind::Ssh => (icons::IconKind::Server, "SSH 主机"),
        PanelKind::Web => (icons::IconKind::Globe, "浏览器"),
        PanelKind::Agent => (icons::IconKind::Brain, "代理"),
        PanelKind::Conversations => (icons::IconKind::BotMessageSquare, "对话"),
        PanelKind::Usage => (icons::IconKind::BarChart3, "用量"),
        PanelKind::Acceptance => (icons::IconKind::BadgeCheck, "验收"),
    }
}

/// 面板专属的按钮徽标装饰(目前只有验收面板有:当前激活 tab 有待处理
/// 交付时,右上角叠一个金色小圆点)。其余 10 个面板返回 `None`。
fn panel_badge(
    app: &App,
    kind: PanelKind,
) -> Option<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    if kind != PanelKind::Acceptance {
        return None;
    }
    let pending = app
        .active_workspace()
        .and_then(|ws| ws.tabs.get(ws.active))
        .map(|t| t.delivery_pending)
        .unwrap_or(false);
    if !pending {
        return None;
    }
    Some(
        container(iced_widget::Space::new())
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0))
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().gold.into()),
                border: Border {
                    radius: 4.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into(),
    )
}
/// 给一个图标栏按钮包上"拖拽换栏/换位"的感应层,手法同 `tab_core::select`
/// (`MouseArea::on_press`)——**这一层现在是按钮唯一的选中/拖拽入口**,
/// 调用方必须给内层 `icon_button_entry` 传 `interactive: false`(见本函数
/// 内部注释解释为什么不能像 `tab_drag_surface` 那样"内容自己接
/// on_press、外层只补 on_move")。`on_move`:光标移动到这个按钮上时,若
/// 正在拖拽(`App::dragging_rail()`,按下即为真,与下面的 `armed` 无关),
/// 上报 `RailDragMove { side, index }`。`rail_drag_move` 只在 `rail_drag`
/// 命中时才做同栏重排 / 记跨栏悬停,所以没在拖拽时这条 `on_move` 是无害的
/// no-op;这条判断刻意继续用 `dragging_rail()` 而不是 `armed`,因为同栏
/// 重排要求光标移到另一个按钮上(天然已经远超阈值),没有"快速单击误判"
/// 这层顾虑,不需要等阈值。`armed`(调用方传 `app.rail_drag_confirmed()`,
/// 已越过 `RAIL_DRAG_VISUAL_THRESHOLD_PX` 位移阈值,不是单纯的
/// `dragging_rail()`——避免快速单击也闪一下抓手光标)为真时把光标切成
/// "抓取"手型,给出"确实按住在拖"的视觉反馈,而不是悄无声息就换了位。
fn rail_drag_surface(
    content: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>,
    side: Side,
    index: usize,
    armed: bool,
    on_select: Message,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // `on_press` 挂在这一层(而不是靠内层 `icon_button_entry` 自带的
    // `iced_widget::button::on_press`)是这个函数存在的**核心原因**,不是
    // 随手选的写法:`iced_widget::button` 的 `on_press` 实际在
    // `ButtonReleased` 且松手时光标仍在按钮范围内才触发("点击"语义,
    // 允许按下后拖出范围松手来取消)——`tab_core::select` 用
    // `MouseArea::on_press`(`ButtonPressed` 即触发,`mousedown 即选中+
    // 备拖` 见其模块文档)才是这里真正要的语义:必须在**按下瞬间**就把
    // `rail_drag` 武装好,才能让紧随其后的 `RailDragMove`(拖拽期间的
    // `CursorMoved`)有意义。若继续走内层 `button::on_press`,武装动作会
    // 推迟到松手那一刻才发生,而 `main.rs` 的
    // `WindowEvent::MouseInput{Released}` 收尾检查(`RailDragEnd`)在这次
    // 事件分发里跑在它前面,看到的还是"未武装",什么也不清——`rail_drag`
    // 会一直悬空到下次点击,期间任何鼠标移动(不按键)都会被误判成
    // 拖拽换位。调用方必须给内层 `icon_button_entry` 传 `interactive:
    // false`,不接 `button::on_press`,否则内层 `button` 会先一步捕获
    // `ButtonPressed`,这一层的 `on_press` 永远收不到事件(iced 的
    // widget `update()` 先递归子级、子级 `capture_event()` 后父级直接
    // 提前返回)。
    let area = MouseArea::new(content)
        .on_press(on_select)
        .on_move(move |_| Message::RailDragMove { side, index });
    if armed {
        let area = area.interaction(mouse::Interaction::Grabbing);
        return area.into();
    }
    area.into()
}
/// 图标栏拖拽期间跟随光标的幽灵图标——视觉语言对齐 OS 拖文件夹:一个
/// 圆角方卡(同 `icon_button_entry` 选中态的 CARD 底 + 金框),里面是被
/// 拖面板的图标,整体以光标为中心悬浮。没有任何交互(不接 `MouseArea`/
/// `on_press`),纯展示——不会挡住底下 `rail_drag_surface` 的 `on_move`/
/// `on_press`(iced 里非交互 widget 天然不参与命中测试,同本文件
/// `stack![base, badge]` 徽标叠在按钮上不挡点击的既有先例)。
///
/// 定位手法同 `project_add_menu_popup`:整窗 `Length::Fill` 容器 + 用
/// `padding` 把内容推到目标坐标,这次坐标是每帧都在变的 `App::last_cursor`
/// 而不是开菜单那一刻的定格快照,所以幽灵图标才会真的"跟手"——
/// `last_cursor` 本来就在每次 `CursorMoved` 里更新,main.rs 也已经在每次
/// `CursorMoved` 后无条件 `window.request_redraw()`(见其注释"悬停也要
/// 请求重绘"),这两点凑在一起,`rail_drag_ghost` 不需要任何额外的重绘
/// 触发就能逐帧跟手。
///
/// 拖拽未在进行、拖拽已武装但还没越过 [`RAIL_DRAG_VISUAL_THRESHOLD_PX`]
/// 位移阈值(快速单击,见 `App::rail_drag_confirmed`),或(理论不会发生
/// 的防御性分支)拖拽中但下标越界拿不到面板种类时,返回空占位——不画
/// 任何东西。调用方(`App::view`)已经用 `rail_drag_confirmed()` 做了同样
/// 的外层判断,这里的检查是防御性的第二道,不是唯一把关处。
pub(crate) fn rail_drag_ghost(
    app: &App,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !app.rail_drag_confirmed() {
        return column![].into();
    }
    let Some(kind) = app.dragged_panel_kind() else {
        return column![].into();
    };
    let (icon, _tooltip) = panel_meta(kind);
    let size = byteui::theme::geometry::rail_button_size();
    let colors = byteui::theme::color::current();

    let ghost = container(icons::view(
        icon,
        byteui::theme::icon_size::rail(),
        colors.gold,
    ))
    .width(Length::Fixed(size))
    .height(Length::Fixed(size))
    .align_x(iced_widget::core::alignment::Horizontal::Center)
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: Some(colors.card.into()),
        border: Border {
            color: colors.gold,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    });

    // 幽灵图标以光标为中心(减半个按钮边长做偏移),并钳制在窗口范围内
    // ——防止贴着窗口边缘拖拽时图标一半画到窗口外。
    let (cx, cy) = app.last_cursor;
    let (window_w, window_h) = app.window_size;
    let x = (cx - size / 2.0).clamp(0.0, (window_w - size).max(0.0));
    let y = (cy - size / 2.0).clamp(0.0, (window_h - size).max(0.0));

    container(ghost)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: y,
            left: x,
            right: 0.0,
            bottom: 0.0,
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PanelKind::default_side()` 与 `RailLayout::default()` 的分组一致——
    /// 两处共用同一份真相,防止一处改了另一处漂移。
    #[test]
    fn default_side_matches_rail_layout_default() {
        let rail = RailLayout::default();
        for &kind in rail.left.iter() {
            assert_eq!(kind.default_side(), Side::Left, "{kind:?}");
        }
        for &kind in rail.right.iter() {
            assert_eq!(kind.default_side(), Side::Right, "{kind:?}");
        }
    }

    /// 默认布局下每个面板都不判为镜像。
    #[test]
    fn panel_mirrored_false_when_rail_layout_is_default() {
        let rail = RailLayout::default();
        for kind in [
            PanelKind::Files,
            PanelKind::GitLog,
            PanelKind::Todo,
            PanelKind::Project,
            PanelKind::Ssh,
            PanelKind::Web,
            PanelKind::Agent,
            PanelKind::Conversations,
        ] {
            assert!(
                !panel_mirrored_in(&rail, kind),
                "{kind:?} 不应该在默认布局下判定为镜像"
            );
        }
    }

    /// 手动把 `Files` 挪到右侧栏后,仅它判为镜像,其它面板不受影响。
    #[test]
    fn panel_mirrored_true_when_manually_relocated() {
        let mut rail = RailLayout::default();
        rail.left.retain(|&k| k != PanelKind::Files);
        rail.right.push(PanelKind::Files);
        assert!(panel_mirrored_in(&rail, PanelKind::Files));
        assert!(
            !panel_mirrored_in(&rail, PanelKind::Todo),
            "没挪的面板不受影响"
        );
    }

    /// `RailLayout::default()` 把 11 个面板不重不漏分到左右两栏,
    /// 与现状 7/4 分组逐一对应(防漂移锚)。
    #[test]
    fn rail_layout_default_covers_all_panels_without_duplicates() {
        let rail = RailLayout::default();
        assert_eq!(rail.left.len(), 7);
        assert_eq!(rail.right.len(), 4);
        let mut all: Vec<_> = rail.left.iter().chain(rail.right.iter()).collect();
        all.sort_by_key(|k| format!("{k:?}"));
        all.dedup();
        assert_eq!(all.len(), 11, "11 个面板不重不漏分到左右两栏");
    }

    #[test]
    fn rail_layout_side_accessors_map_correctly() {
        let rail = RailLayout::default();
        assert_eq!(rail.side(Side::Left), &rail.left);
        assert_eq!(rail.side(Side::Right), &rail.right);
    }

    #[test]
    fn side_of_finds_every_default_panel() {
        let rail = RailLayout::default();
        assert_eq!(rail.side_of(PanelKind::Files), Side::Left);
        assert_eq!(rail.side_of(PanelKind::Web), Side::Left);
        assert_eq!(rail.side_of(PanelKind::Agent), Side::Right);
        assert_eq!(rail.side_of(PanelKind::Acceptance), Side::Right);
    }

    /// `sanitize_rail_layout` 对坏数据回落默认:任一栏为空、面板重复、
    /// 面板数不是 11——任一情形都不做部分修复。
    #[test]
    fn sanitize_rail_layout_falls_back_to_default_on_bad_data() {
        // 左侧为空。
        let empty_left = RailLayout {
            left: vec![],
            right: vec![PanelKind::Agent],
        };
        assert_eq!(sanitize_rail_layout(empty_left), RailLayout::default());

        // 面板数不是 11。
        let too_few = RailLayout {
            left: vec![PanelKind::Files],
            right: vec![PanelKind::Agent],
        };
        assert_eq!(sanitize_rail_layout(too_few), RailLayout::default());

        // 面板重复(缺一个面板 + 重复另一个,合计仍 11 但去重后不足)。
        let dup = RailLayout {
            left: vec![PanelKind::Files; 7],
            right: vec![PanelKind::Agent; 4],
        };
        assert_eq!(sanitize_rail_layout(dup), RailLayout::default());

        // 合法数据原样保留。
        let legit = RailLayout::default();
        assert_eq!(sanitize_rail_layout(legit.clone()), legit);
    }

    /// `RailDrag` 拖拽逻辑的纯核心测试。`App` 没有 `Default` 实现、也无可
    /// 复用的测试构造 helper(构造成本高),所以 `rail_drag_move`/`end_rail_drag`
    /// 的纯逻辑被抽成 `rail_drag_move_into`/`rail_cross_apply` 两个自由函数,
    /// 这里直接构造 `RailLayout` 验证(同 `panel_mirrored_in` 的理由)。
    mod rail_drag_tests {
        use super::*;

        /// 同栏内把第一个图标拖到第三个位置:该面板移动、其余相对顺序不变,
        /// 且不产生跨栏悬停残留。
        #[test]
        fn same_side_reorder_moves_kind() {
            let mut rail = RailLayout::default();
            let mut drag = RailDrag {
                source_side: Side::Left,
                source_index: 0,
                origin_index: 0,
                pending_cross_side: None,
                press_pos: (0.0, 0.0),
            };
            let original_left = rail.left.clone();
            rail_drag_move_into(&mut rail, &mut drag, Side::Left, 2);
            assert_eq!(rail.left[2], original_left[0], "源项应落到目标位");
            assert_eq!(
                rail.left[0], original_left[1],
                "源项前面整体右移一位落到首位"
            );
            assert_eq!(rail.left[1], original_left[2], "源项之后续到第二位");
            assert_eq!(rail.left[3], original_left[3], "目标位之后顺序不变");
            assert_eq!(drag.source_index, 2, "重排后源下标应更新到新位置");
            assert_eq!(
                drag.origin_index, 0,
                "起始位应锁定不变,供 end_rail_drag 判断重排是否发生"
            );
            assert_eq!(drag.pending_cross_side, None, "同栏重排不设跨栏悬停");
        }

        /// 同栏重排放到同一个位置(或越界/no-op)不应移动任何面板。
        #[test]
        fn same_side_reorder_to_same_index_is_noop() {
            let mut rail = RailLayout::default();
            let original = rail.clone();
            let mut drag = RailDrag {
                source_side: Side::Left,
                source_index: 0,
                origin_index: 0,
                pending_cross_side: None,
                press_pos: (0.0, 0.0),
            };
            rail_drag_move_into(&mut rail, &mut drag, Side::Left, 0);
            assert_eq!(rail, original, "拖到同一位置是 no-op,RailLayout 不变");
        }

        /// 跨栏移动:被移动面板搬到目标栏,并从源栏消失(`rail_cross_apply`
        /// 返回被移动的面板,App 侧据此把目标栏设为它 active)。
        #[test]
        fn cross_side_move_relocates_panel() {
            let mut rail = RailLayout::default();
            let kind = rail.left[0];
            let applied = rail_cross_apply(&mut rail, Side::Left, 0, Side::Right, 0);
            assert_eq!(applied, Some(kind));
            assert!(!rail.left.contains(&kind), "源栏应不再含被移动面板");
            assert!(rail.right.contains(&kind), "目标栏应含被移动面板");
        }

        /// 源栏只剩 1 个面板时禁止搬走(不支持"栏清空"),`RailLayout` 不变。
        #[test]
        fn cross_side_move_is_noop_when_source_side_would_become_empty() {
            let mut rail = RailLayout::default();
            while rail.right.len() > 1 {
                let kind = rail.right.remove(0);
                rail.left.push(kind);
            }
            let before = rail.clone();
            let applied = rail_cross_apply(&mut rail, Side::Right, 0, Side::Left, 0);
            assert_eq!(applied, None, "源栏只剩 1 个时应返回 None,不搬走");
            assert_eq!(rail, before, "搬移被挡下,RailLayout 不变");
            assert_eq!(rail.right.len(), 1, "右栏保留最后 1 个");
        }

        /// 源下标越界(传入了过期的拖拽源下标)应安全 no-op。
        #[test]
        fn cross_side_move_with_stale_source_index_is_noop() {
            let mut rail = RailLayout::default();
            let before = rail.clone();
            let applied = rail_cross_apply(&mut rail, Side::Left, 999, Side::Right, 0);
            assert_eq!(applied, None);
            assert_eq!(rail, before);
        }

        /// 跨栏悬停后,把光标移回源栏(同一位置,index 没变)应取消这条
        /// 悬停——`pending_cross_side` 回到 `None`,`end_rail_drag` 看见
        /// `None` 时不做搬移。
        #[test]
        fn hovering_back_to_source_side_cancels_the_pending_cross_move() {
            let mut rail = RailLayout::default();
            let before = rail.clone();
            let mut drag = RailDrag {
                source_side: Side::Left,
                source_index: 0,
                origin_index: 0,
                pending_cross_side: None,
                press_pos: (0.0, 0.0),
            };
            rail_drag_move_into(&mut rail, &mut drag, Side::Right, 0); // 悬停到对侧
            assert_eq!(drag.pending_cross_side, Some((Side::Right, 0)));
            rail_drag_move_into(&mut rail, &mut drag, Side::Left, 0); // 移回源栏,index 未变
            assert_eq!(drag.pending_cross_side, None, "移回源栏取消跨栏悬停");
            assert_eq!(rail, before, "整个过程没有搬移,RailLayout 不变");
        }

        #[test]
        fn dragged_panel_kind_none_when_not_dragging() {
            let rail = RailLayout::default();
            assert_eq!(dragged_panel_kind(&rail, None), None);
        }

        #[test]
        fn dragged_panel_kind_reads_source_slot() {
            let rail = RailLayout::default();
            let kind = rail.left[0];
            let drag = RailDrag {
                source_side: Side::Left,
                source_index: 0,
                origin_index: 0,
                pending_cross_side: None,
                press_pos: (0.0, 0.0),
            };
            assert_eq!(dragged_panel_kind(&rail, Some(drag)), Some(kind));
        }

        #[test]
        fn dragged_panel_kind_none_when_index_out_of_bounds() {
            let rail = RailLayout::default();
            let drag = RailDrag {
                source_side: Side::Left,
                source_index: 999,
                origin_index: 999,
                pending_cross_side: None,
                press_pos: (0.0, 0.0),
            };
            assert_eq!(dragged_panel_kind(&rail, Some(drag)), None);
        }

        fn drag_at(press_pos: (f32, f32)) -> RailDrag {
            RailDrag {
                source_side: Side::Left,
                source_index: 0,
                origin_index: 0,
                pending_cross_side: None,
                press_pos,
            }
        }

        /// 光标没动(或只在阈值内小幅抖动)不算越过阈值——快速单击场景。
        #[test]
        fn past_threshold_false_when_cursor_has_not_moved() {
            let drag = drag_at((100.0, 100.0));
            assert!(!rail_drag_past_threshold(drag, (100.0, 100.0)));
            assert!(!rail_drag_past_threshold(drag, (101.0, 100.0)));
        }

        /// 恰好等于阈值(平方比较是 `>` 不是 `>=`)不算越过,严格大于才算。
        #[test]
        fn past_threshold_false_when_exactly_at_threshold() {
            let drag = drag_at((0.0, 0.0));
            assert!(!rail_drag_past_threshold(
                drag,
                (RAIL_DRAG_VISUAL_THRESHOLD_PX, 0.0)
            ));
        }

        /// 光标越过阈值(任意方向,这里用纯 x 位移)判定为真的拖拽。
        #[test]
        fn past_threshold_true_once_cursor_moves_past_it() {
            let drag = drag_at((0.0, 0.0));
            assert!(rail_drag_past_threshold(
                drag,
                (RAIL_DRAG_VISUAL_THRESHOLD_PX + 1.0, 0.0)
            ));
        }
    }

    /// 图标栏拖拽换位/换栏动画状态机(`RailSlotAnim`)的独立测试——不需要
    /// 构造 `App`(本文件里其余需要真实交互状态的测试都靠自由函数直接测,
    /// `App::new` 依赖 tokio handle/事件代理,构造成本高,这里同样绕开)。
    mod rail_slot_anim_tests {
        use super::*;

        #[test]
        fn retarget_same_side_eases_toward_target_without_snapping_immediately() {
            let mut a = RailSlotAnim {
                current: 0.0,
                side: Side::Left,
            };
            a.retarget(Side::Left, 3.0);
            assert!(
                a.current > 0.0 && a.current < 3.0,
                "第一拍应该只逼近一部分,不是瞬间跳到目标: current={}",
                a.current
            );
            assert!(a.active(3.0), "还没收敛,应算作动画进行中");
        }

        #[test]
        fn retarget_same_side_converges_and_snaps_after_enough_ticks() {
            let mut a = RailSlotAnim {
                current: 0.0,
                side: Side::Left,
            };
            for _ in 0..50 {
                a.retarget(Side::Left, 3.0);
            }
            assert_eq!(a.current, 3.0, "足够多拍之后应该 snap 到目标,不留残余误差");
            assert!(!a.active(3.0), "已收敛,不应再算作动画进行中");
        }

        #[test]
        fn retarget_side_change_snaps_immediately_no_interpolation() {
            // 模拟"面板从左栏第 2 位跨栏落到右栏第 0 位":`current=2.0` 是
            // 左栏坐标系下的槽位号,对右栏这条完全不同的物理列没有几何
            // 意义,不能继续朝新 `target` 插值(会产生一帧"从左栏槽位2滑到
            // 右栏槽位0"的错乱动画),必须直接 snap。
            let mut a = RailSlotAnim {
                current: 2.0,
                side: Side::Left,
            };
            a.retarget(Side::Right, 0.0);
            assert_eq!(a.current, 0.0, "跨栏应直接 snap 到新目标,不插值");
            assert_eq!(a.side, Side::Right, "记录的 side 应更新为新栏");
            assert!(!a.active(0.0), "snap 后应立即视为已收敛");
        }

        #[test]
        fn retarget_target_unchanged_stays_converged() {
            let mut a = RailSlotAnim {
                current: 2.0,
                side: Side::Left,
            };
            a.retarget(Side::Left, 2.0);
            assert_eq!(a.current, 2.0);
            assert!(!a.active(2.0), "目标未变时不应产生动画");
        }
    }
}
