//! 文件树拖拽状态机与落点命中:TreeDragPhase/TreeDrag/DropHit/tree_drop_target/
//! is_valid_move_target/tree_drag_ghost/tree_state_color。

use crate::delivery;
use crate::project::TreeRow;
use byteui::interaction::icons;
use iced_widget::core::{Border, Element, Length, Padding};
use iced_widget::{column, container, row, text};
use std::path::PathBuf;

/// 树内拖拽两阶段状态机——2026-09 用户实测反馈"点一下就进入拖拽态"的
/// 根因是此前"武装(按下)"和"正在拖拽"是同一件事:按下瞬间
/// `tree_drag.is_some()` 就为真,行的 `on_move`/悬停高亮立刻开始生效,
/// 视觉上和真拖拽没有区别。拆成两阶段后,`Pending` 期间**完全不产生任何
/// 反应**(不挂 `on_move`、不显示光标/幽灵图标、不高亮),只有真正越过
/// 距离(见 `App::tree_drag_past_threshold`)和按住时长(见
/// `App::tree_drag_held_long_enough`)两道阈值、由 `App::maybe_confirm_
/// tree_drag`(main.rs 每次 `CursorMoved` 都调一次)推进到 `Dragging` 后,
/// 才开始有任何视觉/交互反应——这个转换只发生一次(见其调用点),之后
/// `confirmed` 就是"是否处于 `Dragging`"这一件事,不需要在松开时重新算
/// 距离/时长(旧版在 `TreeDragEnd` 里重算过一次,和武装时的判定可能不一致,
/// 是这版重构顺手拿掉的一个不必要的重复判据)。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TreeDragPhase {
    /// 按下但还没越过确认阈值——纯粹的死区,任何交互都不该对它有反应。
    Pending,
    /// 已确认是一次真实拖拽:`target` 是当前悬停命中的合法落点目录,悬停到
    /// 非法落点(自身/自身子树)或没悬停到任何目录行时为 `None`——
    /// `TreeDragEnd` 只在 `Some` 时提交移动。
    Dragging { target: Option<PathBuf> },
}

/// 树内拖拽(文件/文件夹在项目树内移动目录)进行中的状态,见
/// `TreeDragPhase` 文档。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TreeDrag {
    pub(crate) source: PathBuf,
    pub(crate) source_is_dir: bool,
    pub(crate) phase: TreeDragPhase,
    /// 按下瞬间的 `App::last_cursor`(窗口逻辑坐标),拖拽期间不更新——
    /// 内核用它和当前光标算位移,判断是否已越过 `Pending → Dragging` 的
    /// 确认阈值(同 `TabDrag::press_pos`/`RailDrag::press_pos` 的既有用法)。
    pub(crate) press_pos: (f32, f32),
    /// 按下瞬间的墙钟时间,拖拽期间不更新——内核用它算按住时长,和
    /// `press_pos` 的位移阈值一起(两者都要满足)判断是否该推进到
    /// `Dragging`。纯距离阈值挡不住 trackpad 快速点按产生的真实位移
    /// (2026-09 用户实测反馈,带诊断日志实锤:一次快速点按真的划出了
    /// ~32px,被误判成确认拖拽)。
    pub(crate) armed_at: std::time::Instant,
}

/// 外部 OS 拖拽命中一行后的结果:`highlight` 是光标字面命中的行路径
/// (文件或目录都可,`view()` 据此渲染金框高亮——2026-09 用户实测反馈
/// "悬浮到文件上也该有高亮"),`target` 是真正的落点目录:命中目录本身即
/// 为 `target`;命中文件则退到其父目录(同 Finder"拖到某个文件上=拖进它
/// 所在文件夹"的既有语义)。命中目录行时两者相同。
#[derive(Debug, Clone, PartialEq)]
pub struct DropHit {
    pub highlight: PathBuf,
    pub target: PathBuf,
}

/// 窗口坐标 (x, y) → 命中行的落点结果。外部 OS 文件拖拽的命中测试：
/// main.rs 在原生事件层拿不到 iced 布局，只能靠 `left_files_tree_bounds`
/// 算出的树视口矩形 + `tree_scroll` 偏移 + 行高/行间距，把窗口 Y 换算成
/// 可见行序号。
///
/// 命中视图外返回 `None`；命中文件行退到其父目录当 `target`(见 `DropHit`
/// 文档),父目录必然存在(树内路径不可能是文件系统根)。行 i 的屏幕上沿
/// = `bounds.y - scroll + i * (row_h + region.gap)`，行高与间距必须和渲染
/// 侧同源（`tree_row_h()`、`project_pane().gap`）。行间死区（间距）落在
/// 任一相邻行之间时按最近行吸住。
pub fn tree_drop_target(
    x: f32,
    y: f32,
    bounds: (f32, f32, f32, f32),
    scroll: f32,
    rows: &[TreeRow],
) -> Option<DropHit> {
    let (bx, by, bw, bh) = bounds;
    if bw <= 0.0 || bh <= 0.0 || !(bx..bx + bw).contains(&x) || !(by..by + bh).contains(&y) {
        return None;
    }
    let row_h = crate::theme::geometry::tree_row_h();
    let gap = crate::theme::region::project_pane().gap;
    let pitch = row_h + gap;
    // 内容坐标(未滚动)下的命中 Y。
    let content_y = (y - by) + scroll;
    // 命中行序号(含行间死区吸附)。
    let idx = (content_y / pitch).floor() as isize;
    let within_row = content_y - idx as f32 * pitch <= row_h;
    let idx = if within_row { idx } else { idx + 1 };
    if idx >= rows.len() as isize || idx < 0 {
        return None;
    }
    let row = &rows[idx as usize];
    if row.is_dir {
        Some(DropHit {
            highlight: row.path.clone(),
            target: row.path.clone(),
        })
    } else {
        row.path.parent().map(|p| DropHit {
            highlight: row.path.clone(),
            target: p.to_path_buf(),
        })
    }
}

/// 树内拖拽合法落点校验:只拒绝"移到自己"、"目录移进自己子树"(会产生
/// 环/孤儿)。`starts_with` 走路径分量比较(不是字符串前缀),`/proj/ab`
/// 不会被误判成 `/proj/a` 的子路径。
///
/// 拖到"当前所在的父目录"(true no-op)**不**在这里拒绝——2026-09 用户
/// 实测反馈"无法拖到父目录":这条曾经的"帮用户挡掉无意义操作"的好心
/// 拒绝,实际效果只是悬停时不高亮、用户以为拖拽坏了。真无意义时
/// `crate::project::move_item` 自己会安全兜底(目标路径与源相同:文件走
/// "已存在同名项"报错,目录走"不能移到它自己"报错,`std::fs::rename`
/// 都不会被调用),不需要在这一层提前拦。
pub(crate) fn is_valid_move_target(
    source: &std::path::Path,
    source_is_dir: bool,
    target: &std::path::Path,
) -> bool {
    if target == source {
        return false;
    }
    if source_is_dir && target.starts_with(source) {
        return false;
    }
    true
}

/// 树内拖拽已确认(`Dragging`)时跟随光标的幽灵胶囊(图标 + 文件名),
/// `rail::rail_drag_ghost` 手法照抄——用绝对定位的 `Padding` 把胶囊钉在
/// 光标旁边;`Pending` 或没有拖拽时画空。2026-09 用户实测反馈"拖动时
/// 没有指针和文件图标":光标本身换成抓取图标由 `view()` 里各行的
/// `.interaction(Grabbing)` 负责(iced 逐帧 `mouse_interaction` →
/// `window.set_cursor` 既有管线),这个函数只管跟着光标的"正在拖什么"
/// 提示胶囊。挂在 `App` 顶层 view 的 `stack!` 里(同 `rail_drag_ghost`),
/// 不是 `files::view()` 的一部分,故吃 `&App` 不是 `&WorkspaceState`。
pub(crate) fn tree_drag_ghost(
    app: &crate::app::App,
) -> Element<'_, crate::app::Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(ws) = app.active_workspace() else {
        return column![].into();
    };
    let Some((source, is_dir)) = ws.files.tree_drag_ghost_source() else {
        return column![].into();
    };
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| source.display().to_string());
    let icon_kind = if is_dir {
        icons::IconKind::Folder
    } else {
        icons::icon_for_file(&name)
    };
    let colors = byteui::theme::color::current();
    let chip = container(
        row![
            icons::view(icon_kind, byteui::theme::icon_size::row(), colors.gold),
            text(name)
                .size(byteui::theme::font::body())
                .color(colors.cream),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .padding([4, 8])
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: Some(colors.card.into()),
        border: Border {
            color: colors.gold,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    });

    // 胶囊宽度随文件名变化、渲染前量不出来,不像 `rail_drag_ghost` 能拿
    // 固定按钮边长居中——改成钉在光标右下方一个小偏移处(同真实 OS 拖拽
    // 缩略图的惯例:贴着光标而不是压在正下方,免得挡住落点判断的视线)。
    let (cx, cy) = app.last_cursor;
    let (window_w, window_h) = app.window_size;
    let x = (cx + 12.0).clamp(0.0, window_w);
    let y = (cy + 16.0).clamp(0.0, window_h);

    container(chip)
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

/// 文件树名称颜色编码 git 状态,取代早前 D2 的行尾色点。按
/// `delivery::TreeState` 档位取色:未加入版本 → 红 `RED`;加入版本未提交的
/// 新文件 → 绿 `GREEN`;修改/删除未提交 → 青 `CYAN`;被忽略 → 弱灰
/// `IGNORED`。无改动(状态为 `None`)由调用方给灰色 `BODY`。
pub(crate) fn tree_state_color(state: delivery::TreeState) -> iced_widget::core::Color {
    match state {
        delivery::TreeState::Untracked => byteui::theme::color::current().red,
        delivery::TreeState::StagedNew => byteui::theme::color::current().green,
        delivery::TreeState::Modified => byteui::theme::color::current().cyan,
        delivery::TreeState::Unchanged => byteui::theme::color::current().body,
        delivery::TreeState::Ignored => byteui::theme::color::current().ignored,
    }
}
