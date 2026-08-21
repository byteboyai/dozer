// crates/dozer-app/src/rail.rs
//! 图标栏(Rail):11 个面板挂载的两条侧栏,支持点击切换、同栏重排、跨栏
//! 拖拽换边。类型 + 纯逻辑 + 槽位动画 + 渲染都在这个模块——不是
//! `extensions/` 那种私有 Message+State+update+view 的 extension 形态,
//! `rail_layout`/`rail_drag`/`rail_slot_anims` 三个字段仍然挂在
//! `App`/`ShellLayout` 上(多消费方共享数据,内核持有),这里只是把纯
//! Rail 逻辑物理搬出 `app.rs`。见
//! `docs/superpowers/specs/2026-08-21-rail-extraction-pilot-design.md`。

use crate::app::{PanelKind, Side};
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
