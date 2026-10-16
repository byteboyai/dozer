//! 树内拖拽的确认阈值(文件树与 Todo 分类树共用):距离阈值与最短按住时长,两者都满足才算一次真实拖拽。
//! 逐字搬自 `dozer-app/src/app/layout.rs`(只改可见性)。

/// 文件树内拖拽确认阈值。最初照抄页签拖拽阈值(`byteui::interaction::tab_strip::TAB_DRAG_CONFIRM_THRESHOLD_PX`)的 4px,
/// 但 tab/rail 是紧凑排列的小控件,4px 越界就意味着真的碰到邻居;树行是
/// 整行高的目标,trackpad 上一次认真点击(尤其是刻意放慢、想点准的那种)
/// 本身就可能带着好几像素的手指位移,4px 太容易被"只是想点一下"的正常
/// 点击越过——2026-09 用户实测反馈(第二轮):做了 Pending/Dragging 两阶段
/// 拆分后仍然"点一下就进入拖拽态",且连带累及点击本身(左侧 `>` 图标点了
/// 展开不了目录,因为松开时 `confirmed` 被误判成 true,走的是"拖拽失败"
/// 分支而不是"这其实是单击"分支,见 `Message::TreeDragEnd` 文档)。调大到
/// 12px,量级对齐"确实移到另一行附近"而不是"点按的手指晃了几像素"。
pub const TREE_DRAG_CONFIRM_THRESHOLD_PX: f32 = 12.0;

/// `drag.press_pos` 到 `cursor` 的位移是否已越过 [`TREE_DRAG_CONFIRM_THRESHOLD_PX`]。
/// `files::Message::TreeRowPress`(见其文档)"按下即武装"是同一个已知会
/// 抖动的模式(同 `tab_drag_past_threshold` 修的那个 bug):单击落点到抬起
/// 之间的亚像素抖动偶尔会越界到邻居目录行的命中框,`TreeDragOver` 若不设
/// 这道门槛会把这次抖动当成"拖到了旁边那一行",直接判定合法性并可能提交
/// 移动——2026-09 用户实测反馈:点一下目录就报"不能把目录移到它自己或其
/// 子目录里"。`App::update` 收到 `TreeDragOver` 时先过这道阈值再转发给
/// `files::update()`,未越过就整条丢弃,`drag.target` 保持原值不变。
pub fn tree_drag_past_threshold(press_pos: (f32, f32), cursor: (f32, f32)) -> bool {
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
pub const TREE_DRAG_MIN_HOLD_DURATION: std::time::Duration = std::time::Duration::from_millis(300);

/// 按下到松开的 `elapsed` 是否已达到 [`TREE_DRAG_MIN_HOLD_DURATION`]——同
/// `tree_drag_past_threshold` 一起、两者都满足才判定为一次真实拖拽(见
/// `Message::TreeDragEnd` 的 `confirmed` 文档),缺一不可:纯距离挡不住
/// 快速划动的误判,纯时长又会让"按住不动很久"被误判成拖拽却没有合法
/// 目标。
pub fn tree_drag_held_long_enough(elapsed: std::time::Duration) -> bool {
    elapsed > TREE_DRAG_MIN_HOLD_DURATION
}
