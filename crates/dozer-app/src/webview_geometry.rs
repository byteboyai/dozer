// crates/dozer-app/src/webview_geometry.rs
//! Files/Project/Web 三个挂了原生 wry webview 的面板共用的几何计算:
//! webview 矩形(`preview_content_bounds_for`)、文件树列表列矩形
//! (`left_files_tree_bounds_for`)、点击命中判断(`is_in_preview_column`)。
//! 跟"文件预览"业务域本身无关,是历史命名遗留——真正的文件预览/编辑器
//! 业务(`preview_open_path` 等)留在 `app.rs`,同 `rail.rs` 里
//! `panel_select`/`terminal.rs` 里 `term_input` 留在内核的理由一致。
//!
//! 依赖的配对列宽公式(`pair_content_width`/`pair_x0_and_width`/
//! `pair_columns`/`PairColumns`/`maximized_box_x_range`/
//! `maximized_box_height`)服务范围远超这三个函数,留在 `app.rs`,只放宽
//! 了可见性。见
//! `docs/superpowers/specs/2026-08-21-webview-geometry-extraction-design.md`。

use crate::app::{
    MaximizedPane, PanelKind, ShellState, Side, left_zone_width, maximized_box_height,
    maximized_box_x_range, pair_columns, pair_content_width, pair_x0_and_width, right_zone_width,
};
use crate::theme;

/// 窗口逻辑尺寸 → `side` 这一侧当前活跃 webview 面板(如果有)的内容区
/// 矩形(逻辑像素 x/y/w/h),供 main.rs 摆放 wry webview 用。`side` 这一
/// 侧收起、或不是 webview 面板(GitLog/Todo/Database/Ssh/Agent/
/// Conversations/Usage/Acceptance)时返回零尺寸矩形。
///
/// 放大态:另一侧被放大时这一侧内容被 `maximize_overlay` 的变暗遮罩整片
/// 盖住——但 wry webview 是原生子视图,不听 iced 的绘制顺序摆布,会无视
/// 遮罩径直叠在最上面,必须用零尺寸矩形把它真正藏起来(与 `collapsed`
/// 分支同一手法)。这一侧被放大时,矩形要按 `maximize_overlay` 实际渲染
/// 的更大盒子重新换算,不能再用平时的 zone 宽度公式。
///
/// 2026-08-19 Stage 4a 审阅后修订:此前隐式假设"任一时刻至多一个 webview
/// 面板活跃、只服务左栏",在 `Files` 留左栏、`Project` 挪右栏这类一步
/// 拖拽即可达到的状态下会漏掉右栏那一个——现在两侧各自独立算,`main.rs`
/// 对左右两侧各调一次。
pub fn preview_content_bounds_for(
    side: Side,
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32, f32, f32) {
    let collapsed = match side {
        Side::Left => state.left_collapsed,
        Side::Right => state.right_collapsed,
    };
    if collapsed {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let kind = match side {
        Side::Left => state.left_view,
        Side::Right => state.right_view,
    };
    let mirrored = state.layout.rail_layout.side_of(kind) != kind.default_side();
    if let Some(maximized) = state.maximized {
        let (x0, avail_w) = maximized_box_x_range(window_width);
        let y0 = byteui::theme::geometry::top_bar_height()
            + byteui::theme::geometry::maximize_overlay_padding();
        // 放大的是另一侧:这一侧内容被变暗遮罩整片盖住,原生 wry 子视图
        // 不听 iced 绘制顺序,必须用零尺寸矩形真正藏起来(同 `collapsed`
        // 分支同一手法)。
        let showing_side = match maximized {
            MaximizedPane::Left => Side::Left,
            MaximizedPane::Right => Side::Right,
        };
        if side != showing_side {
            return (0.0, 0.0, 0.0, 0.0);
        }
        // 两分支 chrome 高度不同(浏览器仍有地址栏,文件预览已去掉),必须
        // 各用各的常量——共用一个会在文件预览顶上留出一截再也画不出东西
        // 的空白(webview 摆位比实际渲染的 tab 栏低了一整个地址栏的高度)。
        let avail_h = (maximized_box_height(window_height)
            - byteui::theme::geometry::status_bar_height())
        .max(0.0);
        return match kind {
            PanelKind::Files => {
                let y = y0 + byteui::theme::geometry::preview_chrome_top_px();
                let h = (avail_h - byteui::theme::geometry::preview_chrome_top_px() - 8.0).max(0.0);
                if state.dims.files_tree_collapsed {
                    // 同非放大态分支:收起文件树后内容拿满放大盒子整宽(无列表/分隔线)。
                    let x = x0 + 8.0;
                    let w = (avail_w - 16.0).max(0.0);
                    (x, y, w, h)
                } else {
                    let pair_w = pair_content_width(avail_w);
                    let cols = pair_columns(pair_w, state.dims.files_split, mirrored);
                    let x = x0 + cols.content_x + 8.0;
                    let w = (cols.content_w - 16.0).max(0.0);
                    (x, y, w, h)
                }
            }
            // 浏览器(Web)是单栏(无配对),放大态占满整条放大盒子,side
            // 不影响它的矩形——但仍需先过上面的 `side != showing_side` 判断。
            PanelKind::Web => {
                let y = y0 + byteui::theme::geometry::browser_chrome_top_px();
                let h = (avail_h - byteui::theme::geometry::browser_chrome_top_px() - 8.0).max(0.0);
                let x = x0 + 8.0;
                let w = (avail_w - 16.0).max(0.0);
                (x, y, w, h)
            }
            // Git 提交图是原生 Canvas 绘制,不挂 webview 子视图。
            PanelKind::GitLog
            // Todo 面板同 GitLog,纯 iced 绘制,不挂 webview 子视图。
            | PanelKind::Todo => (0.0, 0.0, 0.0, 0.0),
            PanelKind::Project => {
                let y = y0 + byteui::theme::geometry::preview_chrome_top_px();
                let h = (avail_h - byteui::theme::geometry::preview_chrome_top_px() - 8.0).max(0.0);
                let pair_w = pair_content_width(avail_w);
                let cols = pair_columns(pair_w, state.dims.project_split, mirrored);
                let x = x0 + cols.content_x + 8.0;
                let w = (cols.content_w - 16.0).max(0.0);
                (x, y, w, h)
            }
            // Database/Ssh/Todo/GitLog 纯 iced 绘制,不挂 webview 子视图;
            // Agent/Usage/Acceptance 同理——任一侧放大只要显示的是这几种,
            // 都没有 webview 可摆。
            PanelKind::Database
            | PanelKind::Ssh
            | PanelKind::Agent
            | PanelKind::Usage
            | PanelKind::Acceptance => (0.0, 0.0, 0.0, 0.0),
            // 审阅内容放大态:跟非放大态同一份 `!mirrored` 理由,只是
            // x0/avail_w/avail_h 换成放大盒子的换算(同 Files/Project 放大
            // 态分支)。
            PanelKind::Conversations => {
                let y = y0 + byteui::theme::geometry::preview_chrome_top_px();
                let h = (avail_h - byteui::theme::geometry::preview_chrome_top_px() - 8.0).max(0.0);
                let pair_w = pair_content_width(avail_w);
                let cols = pair_columns(pair_w, state.dims.conversations_split, !mirrored);
                let x = x0 + cols.content_x + 8.0;
                let w = (cols.content_w - 16.0).max(0.0);
                (x, y, w, h)
            }
        };
    }
    let (zone_x0, zone_w) = pair_x0_and_width(side, window_width, state);
    // `left_zone`/`right_zone` 的上下 margin:webview 必须跟着 inset,否则
    // 会戳出外边框(原生子视图不听 iced 布局,逐像素靠这里算)。左右 margin
    // 同样要算进去,否则去掉外边框后 webview 会戳出新增的留白。
    let m = match side {
        Side::Left => theme::region::left_zone().margin,
        Side::Right => theme::region::right_zone().margin,
    };
    let y_top =
        |chrome_top: f32| -> f32 { byteui::theme::geometry::top_bar_height() + m.top + chrome_top };
    // 底部扣 footbar(`extensions::footbar::view` 的固定高度
    // = `byteui::theme::geometry::status_bar_height()`)——wry webview 不听 iced
    // 布局,若不扣会把 footbar 文字盖在底下。不留额外 8px 间隙,让
    // webview 底部紧贴 footbar 顶部(只留 zone 的下 margin)。
    let h_for = |y: f32| -> f32 {
        (window_height - y - m.bottom - byteui::theme::geometry::status_bar_height()).max(0.0)
    };
    match kind {
        PanelKind::Files => {
            let y = y_top(byteui::theme::geometry::preview_chrome_top_px());
            let h = h_for(y);
            if state.dims.files_tree_collapsed {
                // 收起文件树:配对只剩内容列(见 `app.rs::panel_body` 的
                // `files_tree_collapsed` 分支——树列表与分隔线都不渲染,预览
                // 拿满整条配对宽)。webview 必须跟着拿满,否则宽度冻结在
                // `files_split` 给树留出比例宽的量,不随收起自动扩展。注意
                // 不能用 `zone_w`(那是 `pair_content_width`,已扣过分隔线)——用
                // 原始区宽(无配对、无分隔线的场合,同 Web 单栏)才对齐 iced 的
                // `Length::Fill` 实际渲染宽度。
                let zone_raw_w = match side {
                    Side::Left => left_zone_width(window_width, state),
                    Side::Right => right_zone_width(window_width, state),
                };
                let x = zone_x0 + 8.0 + m.left;
                let w = (zone_raw_w - 16.0 - m.left - m.right).max(0.0);
                (x, y, w, h)
            } else {
                let cols = pair_columns(zone_w, state.dims.files_split, mirrored);
                let x = zone_x0 + cols.content_x + 8.0 + m.left;
                let w = (cols.content_w - 16.0 - m.left - m.right).max(0.0);
                (x, y, w, h)
            }
        }
        // 浏览器(Web):收藏夹侧栏关闭时单栏占满该侧面板区;打开时网页内容
        // 让出收藏夹侧栏的宽度。收藏夹在"内容"前面还是后面同样按
        // `mirrored` 决定(`split` 参数传 "收藏夹占比" = `1.0 -
        // browser_bookmarks_split`,因为该字段存的是内容占比,和其余 split
        // 字段"存列表侧占比"的语义相反)。
        // Web 单栏没有配对,占用**整条**左/右面板区宽——不是 `pair_x0_and_width`
        // 返回的 `zone_w`(扣过中间分隔线的配对宽),要用原始区宽。
        PanelKind::Web => {
            let y = y_top(byteui::theme::geometry::browser_chrome_top_px());
            let h = h_for(y);
            let zone_raw_w = match side {
                Side::Left => left_zone_width(window_width, state),
                Side::Right => right_zone_width(window_width, state),
            };
            let (x, w) = if state.browser_bookmarks_open {
                // `pair_columns` 内建约定是"mirrored=false 时 list 先渲染",
                // 但 `browser.rs::view` 对 Web 用的是相反约定("mirror=false
                // 时 content 先渲染",见 `if mirror { row![bookmarks, divider,
                // content] } else { row![content, divider, bookmarks] }")——
                // 这里的 `mirrored` 要取反再传给 `pair_columns`,否则
                // `cols.content_x` 会算成跟实际渲染顺序相反的那一侧,mirrored
                // 态下 webview 会摆到收藏夹底下而不是收藏夹之后。宽度这半支
                // 同时要用 `pair_content_width(zone_raw_w)`(扣过中间分隔线)
                // 而不是裸的 `zone_raw_w`——iced 的 `FillPortion` 就是在扣掉
                // 固定宽分隔线之后才按权重分剩余空间的(见 `split_portions`
                // 文档注释),否则 webview 宽度会比实际渲染的内容列宽出一条
                // 分隔线的量,右边界戳出内容列。
                let cols = pair_columns(
                    pair_content_width(zone_raw_w),
                    1.0 - state.dims.browser_bookmarks_split,
                    !mirrored,
                );
                let x = zone_x0 + cols.content_x + 8.0 + m.left;
                let w = (cols.content_w - 16.0 - m.left - m.right).max(0.0);
                (x, w)
            } else {
                let x = zone_x0 + 8.0 + m.left;
                let w = (zone_raw_w - 16.0 - m.left - m.right).max(0.0);
                (x, w)
            };
            (x, y, w, h)
        }
        PanelKind::GitLog => (0.0, 0.0, 0.0, 0.0),
        // Todo 面板同 GitLog,纯 iced 绘制,不挂 webview 子视图。
        PanelKind::Todo => (0.0, 0.0, 0.0, 0.0),
        // Project 面板右配对(项目预览)是 Files 同款预览 chrome,按
        // `project_split` 算出右配对那条 webview 矩形。
        PanelKind::Project => {
            let y = y_top(byteui::theme::geometry::preview_chrome_top_px());
            let h = h_for(y);
            let cols = pair_columns(zone_w, state.dims.project_split, mirrored);
            let x = zone_x0 + cols.content_x + 8.0 + m.left;
            let w = (cols.content_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
        // Database 面板同 Project,纯 iced 绘制,不挂 webview 子视图。
        PanelKind::Database => (0.0, 0.0, 0.0, 0.0),
        // SSH 面板同 Project,纯 iced 绘制,阶段 1 不挂 webview 子视图。
        PanelKind::Ssh => (0.0, 0.0, 0.0, 0.0),
        // Stage 4a 跨栏拖拽:该侧视图可为另一栏面板,纯 iced 绘制、该侧
        // 无 webview 可摆,装空矩形。
        PanelKind::Agent | PanelKind::Usage | PanelKind::Acceptance => (0.0, 0.0, 0.0, 0.0),
        // 审阅内容(2026-08-21 webview trace 改造):跟 Files/Project 同款
        // "配对列宽 + preview chrome 高度"算法,但 `mirrored` 要取反——
        // app.rs 的 `PanelKind::Conversations` 分支未镜像时渲染顺序是
        // `[review, divider, list]`(内容先),跟 `pair_columns` 的内建
        // 默认("mirrored=false → list 先")相反,同 Web 收藏夹那处的手法。
        PanelKind::Conversations => {
            let y = y_top(byteui::theme::geometry::preview_chrome_top_px());
            let h = h_for(y);
            let cols = pair_columns(zone_w, state.dims.conversations_split, !mirrored);
            let x = zone_x0 + cols.content_x + 8.0 + m.left;
            let w = (cols.content_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
    }
}

/// `side` 这一侧文件树的**目录列表 Scrollable** 在窗口坐标系里的矩形
/// (上/左/宽/高,逻辑像素),供 main.rs 做外部文件拖拽命中测试。返回的
/// 矩形只覆盖列表视口本身——命中测试据此把窗口 Y 换算成 `tree_scroll`
/// 偏移下的"可见行序号",再推出那行是不是目录。
///
/// 与 `preview_content_bounds_for` 同源(外层)但其目标是**配对里 list 那一
/// 栏**(树),不是 webview 的 content 列,所以横向起点用 `pair_columns`
/// 的 `list_x`(mirrored 时在 content 之后)、纵向起点换用
/// `tree_chrome_top_px`(面板头+搜索/工具栏),底部扣 `git 脚注栏` 而非
/// footbar 专用常量。
///
/// 不可命中(该侧收起 / 不是 Files / 放大的是另一侧)时返回零尺寸矩形。
/// 该侧被放大(`MaximizedPane` 对应该侧)按 `maximize_overlay` 的实际盒子
/// 换算。
pub fn left_files_tree_bounds_for(
    side: Side,
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32, f32, f32) {
    let zero = || (0.0, 0.0, 0.0, 0.0);
    let kind = match side {
        Side::Left => state.left_view,
        Side::Right => state.right_view,
    };
    let collapsed = match side {
        Side::Left => state.left_collapsed,
        Side::Right => state.right_collapsed,
    };
    // 文件树列表子栏被收起(`files_tree_collapsed`)时不再渲染,没有可拖放命中
    // 的目标,同样返回零尺寸。
    if collapsed || kind != PanelKind::Files || state.dims.files_tree_collapsed {
        return zero();
    }
    let m = match side {
        Side::Left => theme::region::left_zone().margin,
        Side::Right => theme::region::right_zone().margin,
    };
    let p = theme::region::project_pane();
    let mirrored =
        state.layout.rail_layout.side_of(PanelKind::Files) != PanelKind::Files.default_side();
    if let Some(maximized) = state.maximized {
        let showing_side = match maximized {
            MaximizedPane::Left => Side::Left,
            MaximizedPane::Right => Side::Right,
        };
        if side != showing_side {
            return zero();
        }
        let (x0, avail_w) = maximized_box_x_range(window_width);
        let y_top = byteui::theme::geometry::top_bar_height()
            + byteui::theme::geometry::maximize_overlay_padding();
        let avail_h = (maximized_box_height(window_height)
            - byteui::theme::geometry::status_bar_height())
        .max(0.0);
        let pair_w = pair_content_width(avail_w);
        let cols = pair_columns(pair_w, state.dims.files_split, mirrored);
        let x = if mirrored {
            x0 + cols.list_x + byteui::theme::geometry::divider_width() + m.left + p.padding.left
        } else {
            x0 + cols.list_x + m.left + p.padding.left
        };
        let w = (cols.list_w - p.padding.left - p.padding.right).max(0.0);
        let y = y_top + m.top + p.padding.top + theme::geometry::tree_chrome_top_px();
        let h = (avail_h
            - m.top
            - m.bottom
            - p.padding.top
            - p.padding.bottom
            - theme::geometry::tree_chrome_top_px()
            - theme::geometry::tree_chrome_bottom_px())
        .max(0.0);
        return (x, y, w, h);
    }
    // `pair_x0_and_width` 返回的第二个值已经是 `pair_content_width(...)`
    // 之后的值(扣过分隔线的配对内容宽)——直接用作 `pair_columns` 的
    // `pair_w`,不要再包一层 `pair_content_width`,否则宽度多扣一次分隔线。
    let (zone_x0, zone_w) = pair_x0_and_width(side, window_width, state);
    let cols = pair_columns(zone_w, state.dims.files_split, mirrored);
    let x = if mirrored {
        zone_x0 + cols.list_x + byteui::theme::geometry::divider_width() + m.left + p.padding.left
    } else {
        zone_x0 + cols.list_x + m.left + p.padding.left
    };
    let w = (cols.list_w - p.padding.left - p.padding.right).max(0.0);
    let y_pane = byteui::theme::geometry::top_bar_height() + m.top;
    let y = y_pane + p.padding.top + theme::geometry::tree_chrome_top_px();
    let h = ((window_height - m.bottom - byteui::theme::geometry::status_bar_height())
        - (y_pane + p.padding.top + theme::geometry::tree_chrome_top_px())
        - p.padding.bottom
        - theme::geometry::tree_chrome_bottom_px())
    .max(0.0);
    (x, y, w, h)
}

/// 逻辑 x 是否落在左侧文件预览内容区列内。焦点路由用:点击落在
/// 该列 → 键盘交给 webview;落在别处 → 交回窗口(终端)。
///
/// 放大态(Task 5):右侧被放大时左侧内容不可见,恒不落在预览列;左侧被
/// 放大时按 `maximize_overlay` 实际渲染的更大盒子重新换算横向范围。
/// 逻辑 x 是否落在某一侧的预览列内,是则返回命中的面板种类;焦点路由
/// (`main.rs`)据此决定把键盘交给哪个 webview 池(`Web` → 浏览器池,
/// `Files`/`Project` → 预览池)、以及 `active_preview_webview_id` 该查
/// `ws.preview` 还是 `ws.project_preview`。
///
/// 2026-08-19 Stage 4a 审阅后修订:此前只查 `state.left_view`,`Project`
/// 挪到右栏后点击其预览列不会被识别;现在左右两侧各自独立判断。
///
/// `Web` 分支保留原有近似(整个 zone 都算预览列,不细分收藏夹展开时的
/// 精确切分——延续现状)。
pub fn is_in_preview_column(x: f32, window_width: f32, state: &ShellState) -> Option<PanelKind> {
    for side in [Side::Left, Side::Right] {
        let collapsed = match side {
            Side::Left => state.left_collapsed,
            Side::Right => state.right_collapsed,
        };
        if collapsed {
            continue;
        }
        let kind = match side {
            Side::Left => state.left_view,
            Side::Right => state.right_view,
        };
        if let Some(maximized) = state.maximized {
            let showing_side = match maximized {
                MaximizedPane::Left => Side::Left,
                MaximizedPane::Right => Side::Right,
            };
            if side != showing_side {
                continue;
            }
            let (x0, avail_w) = maximized_box_x_range(window_width);
            let mirrored = state.layout.rail_layout.side_of(kind) != kind.default_side();
            let hit = match kind {
                PanelKind::Files => {
                    if state.dims.files_tree_collapsed {
                        // 收起文件树:内容拿满放大盒子整宽,整条都算预览列。
                        x >= x0 && x < x0 + avail_w
                    } else {
                        let cols = pair_columns(
                            pair_content_width(avail_w),
                            state.dims.files_split,
                            mirrored,
                        );
                        x >= x0 + cols.content_x && x < x0 + cols.content_x + cols.content_w
                    }
                }
                PanelKind::Web => x >= x0 && x < x0 + avail_w,
                PanelKind::Project => {
                    let cols = pair_columns(
                        pair_content_width(avail_w),
                        state.dims.project_split,
                        mirrored,
                    );
                    x >= x0 + cols.content_x && x < x0 + cols.content_x + cols.content_w
                }
                _ => false,
            };
            if hit {
                return Some(kind);
            }
            continue;
        }
        let (zone_x0, zone_w) = pair_x0_and_width(side, window_width, state);
        let mirrored = state.layout.rail_layout.side_of(kind) != kind.default_side();
        let hit = match kind {
            PanelKind::Files => {
                if state.dims.files_tree_collapsed {
                    // 收起文件树:内容拿满整条配对宽,整条都算预览列。
                    x >= zone_x0 && x < zone_x0 + zone_w
                } else {
                    let cols = pair_columns(zone_w, state.dims.files_split, mirrored);
                    x >= zone_x0 + cols.content_x && x < zone_x0 + cols.content_x + cols.content_w
                }
            }
            PanelKind::Web => x >= zone_x0 && x < zone_x0 + zone_w,
            PanelKind::Project => {
                let cols = pair_columns(zone_w, state.dims.project_split, mirrored);
                x >= zone_x0 + cols.content_x && x < zone_x0 + cols.content_x + cols.content_w
            }
            _ => false,
        };
        if hit {
            return Some(kind);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{PanelDims, ShellLayout};
    use crate::rail;

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
    fn preview_content_bounds_is_inside_left_content_column() {
        // 左面板区 640 宽,项目树占 0.35(=224),预览内容区在其右侧(过分隔线)。
        let state = test_state();
        let (x, y, w, h) = preview_content_bounds_for(Side::Left, 1440.0, 900.0, &state);
        let list_w = state.dims.left_width * state.dims.files_split;
        let col_start = byteui::theme::geometry::icon_rail_width()
            + list_w
            + byteui::theme::geometry::divider_width();
        assert!(x >= col_start && x < col_start + 16.0, "x={x}");
        assert!((380.0..=420.0).contains(&w), "w={w}");
        assert!(
            (y - (78.0 + theme::region::left_zone().margin.top)).abs() < 0.1,
            "y={y}(顶栏 40 + tab 栏 38 + left_zone 上 margin 之下,地址栏已去)"
        );
        assert!(h > 700.0 && h < 900.0 - y, "h={h}");
    }

    #[test]
    fn preview_content_bounds_conversations_review_content_is_first_when_not_mirrored() {
        // app.rs 的 PanelKind::Conversations 分支(zone 渲染,`else` 臂):
        // 未镜像时渲染顺序是 [review, divider, list]——跟 pair_columns 的
        // 内建默认("mirrored=false → list 先")相反,所以必须传 `!mirrored`
        // (同 Web 收藏夹那处的手法),否则 webview 会摆到列表底下而不是
        // 列表前面。
        let state = ShellState {
            left_view: PanelKind::Conversations,
            ..test_state()
        };
        let (x, _y, w, _h) = preview_content_bounds_for(Side::Left, 1440.0, 900.0, &state);
        let m = theme::region::left_zone().margin;
        let col_start = byteui::theme::geometry::icon_rail_width() + m.left;
        assert!(
            x >= col_start && x < col_start + 16.0,
            "非镜像态 review 内容应紧贴面板区左边界(pair 里第一个元素): x={x}"
        );
        assert!(w > 200.0, "w={w}");
    }

    #[test]
    fn preview_content_bounds_conversations_zero_when_left_collapsed() {
        let state = ShellState {
            left_view: PanelKind::Conversations,
            left_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            preview_content_bounds_for(Side::Left, 1440.0, 900.0, &state),
            (0.0, 0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn preview_content_bounds_web_view_spans_whole_left_zone() {
        // Web 视图(左栏最底部 Globe 按钮)没有配对,预览内容区从图标栏右侧起
        // 占满左面板区。
        let state = ShellState {
            left_view: PanelKind::Web,
            ..test_state()
        };
        let (x, _, w, _) = preview_content_bounds_for(Side::Left, 1440.0, 900.0, &state);
        let m = theme::region::left_zone().margin;
        let left_w = left_zone_width(1440.0, &state);
        assert_eq!(x, byteui::theme::geometry::icon_rail_width() + 8.0 + m.left);
        assert_eq!(w, left_w - 16.0 - m.left - m.right);
    }

    #[test]
    fn preview_content_bounds_files_collapsed_spans_whole_zone() {
        // 文件树收起(`files_tree_collapsed`)时,app.rs::panel_body 只渲染预览
        // 列拿满整条配对宽(无列表、无分隔线)。webview 必须跟着拿满原始区宽,
        // 而不是仍按 `files_split` 给树留比例宽 —— 这正是"收起后宽度不自动
        // 扩展"bug 的回归护栏:收起前后宽度应变宽。
        let open = ShellState {
            dims: PanelDims {
                files_tree_collapsed: false,
                ..PanelDims::default()
            },
            ..test_state()
        };
        let collapsed = ShellState {
            dims: PanelDims {
                files_tree_collapsed: true,
                ..PanelDims::default()
            },
            ..test_state()
        };
        let (_x_open, _y_open, w_open, _h_open) =
            preview_content_bounds_for(Side::Left, 1440.0, 900.0, &open);
        let (x_collapsed, _y_collapsed, w_collapsed, _h_collapsed) =
            preview_content_bounds_for(Side::Left, 1440.0, 900.0, &collapsed);
        let m = theme::region::left_zone().margin;
        let left_w = left_zone_width(1440.0, &collapsed);
        // 收起后与 Web 单栏同宽(整条区宽扣掉内边距与左右 margin),且比文件树
        // 展开时按 split 留下的内容宽更大。
        assert_eq!(
            x_collapsed,
            byteui::theme::geometry::icon_rail_width() + 8.0 + m.left
        );
        assert_eq!(w_collapsed, left_w - 16.0 - m.left - m.right);
        assert!(
            w_collapsed > w_open,
            "收起文件树后预览 webview 应比展开时更宽: w_collapsed={w_collapsed} w_open={w_open}"
        );
    }

    #[test]
    fn preview_content_bounds_web_view_shrinks_when_bookmarks_open() {
        let closed = ShellState {
            left_view: PanelKind::Web,
            browser_bookmarks_open: false,
            ..test_state()
        };
        let open = ShellState {
            left_view: PanelKind::Web,
            browser_bookmarks_open: true,
            ..test_state()
        };
        let (_, _, w_closed, _) = preview_content_bounds_for(Side::Left, 1440.0, 900.0, &closed);
        let (x_open, y_open, w_open, h_open) =
            preview_content_bounds_for(Side::Left, 1440.0, 900.0, &open);
        assert!(
            w_open < w_closed,
            "收藏夹打开时网页内容应该让出侧栏宽度: w_open={w_open} w_closed={w_closed}"
        );
        // x/y/h 不受收藏夹开关影响——网页内容起点、高度不变,只是变窄。
        let (x_closed, y_closed, _, h_closed) =
            preview_content_bounds_for(Side::Left, 1440.0, 900.0, &closed);
        assert_eq!(x_open, x_closed);
        assert_eq!(y_open, y_closed);
        assert_eq!(h_open, h_closed);
    }

    /// Stage 4b 审阅后修复:`Web` 被拖到非默认栏(镜像态)且收藏夹展开时,
    /// iced 实际渲染顺序反过来(收藏夹先、内容后,见 `browser.rs::view`
    /// 的 `if mirror { row![bookmarks, divider, content] }`)——webview 的
    /// `x` 必须跟着挪到收藏夹之后,不能再固定在 zone 左边界;宽度也必须
    /// 用 `pair_content_width` 扣过分隔线的宽度,否则和 iced 的
    /// `FillPortion` 实际布局对不上。
    #[test]
    fn preview_content_bounds_web_view_mirrored_bookmarks_content_follows_render_order() {
        let mut state = ShellState {
            browser_bookmarks_open: true,
            ..test_state()
        };
        state
            .layout
            .rail_layout
            .left
            .retain(|&k| k != PanelKind::Web);
        state.layout.rail_layout.right.push(PanelKind::Web);
        state.right_view = PanelKind::Web;

        let window_width = 1600.0;
        let (x, _, w, _) = preview_content_bounds_for(Side::Right, window_width, 900.0, &state);
        let (zone_x0, zone_w) = pair_x0_and_width(Side::Right, window_width, &state);
        let raw_zone_w = right_zone_width(window_width, &state);

        // 镜像态下 content 是 pair 里第二个元素(收藏夹先),x 应该落在
        // 收藏夹列 + 分隔线之后,不是 zone 左边界。
        assert!(
            x > zone_x0 + 8.0,
            "镜像态 content 在收藏夹之后,x 应该比 zone 左边界更靠右: x={x} zone_x0={zone_x0}"
        );
        // 宽度按扣过分隔线的 pair 宽分配,不能用裸 zone 宽——否则右边界会
        // 戳出 iced 实际渲染的内容列。content 列右边界(相对 zone_x0)按
        // 构造应恰好等于裸区宽(list_w + divider + content_w = raw_zone_w),
        // 即 webview 右边界不应超出这一侧面板区的实际右边界。
        assert!(
            zone_w < raw_zone_w,
            "pair_x0_and_width 返回的应是扣过分隔线的宽度,测试前提不成立"
        );
        assert!(
            x + w <= zone_x0 + raw_zone_w + 0.5,
            "webview 右边界不应超出这一侧面板区的实际右边界: x={x} w={w} zone_x0={zone_x0} raw_zone_w={raw_zone_w}"
        );
    }

    #[test]
    fn preview_content_bounds_zero_when_left_collapsed() {
        let state = ShellState {
            left_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            preview_content_bounds_for(Side::Left, 1440.0, 900.0, &state),
            (0.0, 0.0, 0.0, 0.0)
        );
    }

    /// Fix round 1 Critical:右侧被放大时,左侧 webview 必须归零——它是原生
    /// wry 子视图,不听 iced 的绘制顺序摆布,不归零会无视变暗遮罩径直叠在
    /// 最上面。
    #[test]
    fn preview_content_bounds_zero_when_right_maximized() {
        let state = ShellState {
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        assert_eq!(
            preview_content_bounds_for(Side::Left, 1440.0, 900.0, &state),
            (0.0, 0.0, 0.0, 0.0)
        );
    }

    /// Fix round 1 Critical:左侧被放大(Files 配对)时,webview 矩形必须
    /// 按 `maximize_overlay` 实际渲染的更大盒子换算,不能再用平时的
    /// `left_zone_width`(640)。用具体数字核对,不只看"落在范围内"：
    /// x0=byteui::theme::geometry::icon_rail_width()(44)+byteui::theme::geometry::maximize_overlay_padding()(40)=84,
    /// avail_w=1440-2*44-2*40=1272,pair_w=1272-8=1264,
    /// list_w=1264*0.35=442.4,x=84+442.4+8+8=542.4,w=1264*0.65-16=805.6;
    /// y0=byteui::theme::geometry::top_bar_height()(40)+40=80,y=80+38(byteui::theme::geometry::preview_chrome_top_px(),地址栏已去)=118,
    /// avail_h=900-40-80=780 - status_bar_height()(26,扣 footbar)=754,
    /// h=754-38-8=708。
    #[test]
    fn preview_content_bounds_left_maximized_files_matches_overlay_geometry() {
        let state = ShellState {
            maximized: Some(MaximizedPane::Left),
            ..test_state()
        };
        let (x, y, w, h) = preview_content_bounds_for(Side::Left, 1440.0, 900.0, &state);
        assert!((x - 542.4).abs() < 0.1, "x={x}");
        assert!((y - 118.0).abs() < 0.1, "y={y}");
        assert!((w - 805.6).abs() < 0.1, "w={w}");
        assert!((h - 708.0).abs() < 0.1, "h={h}");
        // 明显区别于平时(非放大)的几何——不能巧合碰上同一个值。
        let normal = preview_content_bounds_for(Side::Left, 1440.0, 900.0, &test_state());
        assert_ne!((x, y, w, h), normal, "放大态几何必须和平时不同");
    }

    /// Fix round 1 Critical:左侧被放大(Web 视图,无配对)时同样要按放大盒子
    /// 换算。x=x0+8=92,w=avail_w-16=1256。
    #[test]
    fn preview_content_bounds_left_maximized_web_spans_whole_overlay_box() {
        let state = ShellState {
            left_view: PanelKind::Web,
            maximized: Some(MaximizedPane::Left),
            ..test_state()
        };
        let (x, _, w, _) = preview_content_bounds_for(Side::Left, 1440.0, 900.0, &state);
        assert!((x - 92.0).abs() < 0.1, "x={x}");
        assert!((w - 1256.0).abs() < 0.1, "w={w}");
    }

    /// Fix round 1 Critical:焦点路由与 webview 摆位必须用同一份放大态
    /// 几何——右侧放大时左侧列恒不可点中;左侧放大时命中范围要按放大盒子
    /// 的横向范围([534.4, 1356))判定,不是平时的 [280, 688)。
    #[test]
    fn preview_column_hit_test_respects_maximized_state() {
        let right_max = ShellState {
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        assert!(is_in_preview_column(500.0, 1440.0, &right_max).is_none());

        let left_max = ShellState {
            maximized: Some(MaximizedPane::Left),
            ..test_state()
        };
        assert!(
            is_in_preview_column(500.0, 1440.0, &left_max).is_none(),
            "500 在平时的预览列内,但放大盒子的列起点在 534.4 之后"
        );
        assert_eq!(
            is_in_preview_column(600.0, 1440.0, &left_max),
            Some(PanelKind::Files)
        );
        assert!(is_in_preview_column(1300.0, 1440.0, &left_max).is_some());
        assert!(is_in_preview_column(1400.0, 1440.0, &left_max).is_none());
    }

    /// Stage 4a 跨栏拖拽:右栏面板被拖到左栏后成了 `left_view`。它们纯 iced
    /// 绘制、左区没有 webview 可摆,`preview_content_bounds` 必须返回空矩形
    /// 而不是命中 `_ => unreachable!`(GUI 拖拽核对抓到的崩溃)。
    ///
    /// `Conversations` 已从这里除名:2026-08-21 webview trace 改造后它跟
    /// Files/Project 一样挂 wry webview(即使被拖到左栏也返回真实 content
    /// 矩形,见 `preview_content_bounds_conversations_*` 两个测试)。
    #[test]
    fn preview_content_bounds_bare_for_right_panel_on_left() {
        for kind in [PanelKind::Agent, PanelKind::Usage, PanelKind::Acceptance] {
            let state = ShellState {
                left_view: kind,
                ..test_state()
            };
            assert_eq!(
                preview_content_bounds_for(Side::Left, 1440.0, 900.0, &state),
                (0.0, 0.0, 0.0, 0.0),
                "左视图是右栏面板 {kind:?} 时应返回空矩形"
            );
            let left_max = ShellState {
                left_view: kind,
                maximized: Some(MaximizedPane::Left),
                ..test_state()
            };
            assert_eq!(
                preview_content_bounds_for(Side::Left, 1440.0, 900.0, &left_max),
                (0.0, 0.0, 0.0, 0.0),
                "放大态下左视图是右栏面板 {kind:?} 时同样应返回空矩形"
            );
        }
    }

    /// Stage 4a 跨栏拖拽:右栏面板当左视图时永不落在预览列(`is_in_preview_column`
    /// 不得命中 `_ => unreachable!`)。
    #[test]
    fn is_in_preview_column_false_for_right_panel_on_left() {
        for kind in [
            PanelKind::Agent,
            PanelKind::Conversations,
            PanelKind::Usage,
            PanelKind::Acceptance,
        ] {
            let state = ShellState {
                left_view: kind,
                ..test_state()
            };
            assert!(
                is_in_preview_column(500.0, 1440.0, &state).is_none(),
                "左视图是右栏面板 {kind:?} 时永不落在预览列"
            );
        }
    }

    #[test]
    fn preview_content_bounds_never_negative() {
        let state = test_state();
        let (_, _, w, h) = preview_content_bounds_for(Side::Left, 100.0, 50.0, &state);
        assert!(w >= 0.0 && h >= 0.0);
    }

    #[test]
    fn preview_column_hit_test() {
        // 窗口宽 1440:左图标栏 44 + 左面板区 640(项目树 0.35=224 + 分隔线 8)。
        // 预览内容列 = [273.2, 684)。
        let state = test_state();
        assert!(
            is_in_preview_column(100.0, 1440.0, &state).is_none(),
            "落在项目树列"
        );
        assert_eq!(
            is_in_preview_column(273.2, 1440.0, &state),
            Some(PanelKind::Files),
            "预览列左边界(过配对分隔线)"
        );
        assert_eq!(
            is_in_preview_column(500.0, 1440.0, &state),
            Some(PanelKind::Files),
            "预览列内"
        );
        assert!(
            is_in_preview_column(700.0, 1440.0, &state).is_none(),
            "已进右面板区"
        );
        assert!(
            is_in_preview_column(1200.0, 1440.0, &state).is_none(),
            "右面板区内"
        );
    }

    #[test]
    fn preview_column_hit_test_left_collapsed_is_never_hit() {
        let state = ShellState {
            left_collapsed: true,
            ..test_state()
        };
        assert!(is_in_preview_column(300.0, 1440.0, &state).is_none());
    }

    /// Stage 4b:Project 面板拖到右栏(镜像态)后,`is_in_preview_column`
    /// 必须在右栏命中 `PanelKind::Project`——修正老实现只看 `left_view`、
    /// 右栏 Project 预览 webview 点击永远进不了预览池(拿不到键盘焦点)的
    /// 跨栏 bug。镜像态 content 渲染在配对最左(`content_x == 0`),命中区是
    /// `[x0, x0 + content_w)`。
    #[test]
    fn is_in_preview_column_returns_project_when_project_on_right() {
        let state = ShellState {
            layout: ShellLayout {
                rail_layout: rail::RailLayout {
                    // Project 从默认左栏挪到右栏(11 个面板不重不漏),
                    // 保持 `side_of` 不变式——Project 只出现在右栏。
                    left: vec![
                        PanelKind::Todo,
                        PanelKind::Files,
                        PanelKind::GitLog,
                        PanelKind::Database,
                        PanelKind::Ssh,
                        PanelKind::Web,
                    ],
                    right: vec![
                        PanelKind::Project,
                        PanelKind::Agent,
                        PanelKind::Conversations,
                        PanelKind::Usage,
                        PanelKind::Acceptance,
                    ],
                },
                ..ShellLayout::default()
            },
            right_view: PanelKind::Project,
            ..test_state()
        };
        let (x0, w) = pair_x0_and_width(Side::Right, 1440.0, &state);
        let cols = pair_columns(pair_content_width(w), state.dims.project_split, true);
        let inside = x0 + 10.0;
        assert!(
            inside < x0 + cols.content_w,
            "测试点必须落在右栏镜像 Project 的预览列内(inside={inside},x0={x0},content_w={})",
            cols.content_w
        );
        assert_eq!(
            is_in_preview_column(inside, 1440.0, &state),
            Some(PanelKind::Project)
        );
        let list_side = x0 + cols.content_w + 10.0;
        assert!(
            is_in_preview_column(list_side, 1440.0, &state).is_none(),
            "列表侧(内容右侧)不算预览列"
        );
    }

    /// 文件树收起 + Files 左放大:webview 拿满放大盒子整宽(无列表/分隔线),
    /// 与 Web 放大态同款。
    #[test]
    fn preview_content_bounds_files_maximized_collapsed_spans_whole_box() {
        let state = ShellState {
            dims: PanelDims {
                files_tree_collapsed: true,
                ..PanelDims::default()
            },
            maximized: Some(MaximizedPane::Left),
            ..test_state()
        };
        let (x, _y, w, h) = preview_content_bounds_for(Side::Left, 1440.0, 900.0, &state);
        let (x0, avail_w) = maximized_box_x_range(1440.0);
        assert_eq!(x, x0 + 8.0);
        assert_eq!(w, (avail_w - 16.0).max(0.0));
        assert!(h > 100.0, "放大态应有高度: h={h}");
    }

    /// 文件树收起:`is_in_preview_column` 把整条配对宽都算预览列(否则点击
    /// 原来列表列的区域拿不到预览/webview 焦点)。
    #[test]
    fn is_in_preview_column_files_collapsed_covers_whole_zone() {
        let state = ShellState {
            dims: PanelDims {
                files_tree_collapsed: true,
                ..PanelDims::default()
            },
            ..test_state()
        };
        // left_zone 内、原列表列所占的 x(在 split 比例内)现在也应命中 Files 预览列。
        let x_in_former_list = byteui::theme::geometry::icon_rail_width() + 40.0;
        assert_eq!(
            is_in_preview_column(x_in_former_list, 1440.0, &state),
            Some(PanelKind::Files)
        );
        // 超出左区宽的部分不算。
        let far_right =
            byteui::theme::geometry::icon_rail_width() + left_zone_width(1440.0, &state) + 100.0;
        assert!(is_in_preview_column(far_right, 1440.0, &state).is_none());
    }
}
