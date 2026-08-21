// crates/dozer-app/src/terminal.rs
//! 终端宿主:项目自己终端页签(`TermTarget::Shared`)与 SSH 面板内嵌终端
//! (`TermTarget::SshPanel`)共用的目标标识 + 项目终端页签的宿主 view +
//! 终端页签栏本身(`tab_bar`/`active_tab_view`/`tab_drag_surface`/
//! `tab_item`)。这四个函数原本留在 `app.rs`,但摸底发现它们的唯一真实
//! 调用方就是本文件的 `terminal_pane`——不是跨面板共享的 chrome,是这次
//! 搬迁时才发现的范围误判,一并修正过来。`tab_arrow_button`/`tab_window`
//! 起初也被误判成终端专属,后来发现 `workspace.rs` 的预览页签栏也在用,
//! 改归 `tab_widget.rs`(与同样跨模块共享的 `panel_tab` 放一起)。
//!
//! `term_input`/`term_output`/`term_paste`/`term_ime_preedit` 等处理方法
//! 不在这里——它们的主体是可见性闸门 + 项目路由 + 读写 `Workspace` 字段,
//! 属于内核编排,留在 `app.rs`(同 `rail.rs` 里 `panel_select`/
//! `end_rail_drag` 留在内核的理由一致)。`ssh_terminal_pane`(SSH 面板自己
//! 的宿主 view)也不在这里,它是 `extensions::ssh` 的地盘;`tab_divider`
//! (`ssh_terminal_pane`/`workspace.rs` 预览页签栏也在用)同样留在
//! `app.rs`。

use crate::app::{App, HoverId, Message, PanelKind, TabGroup, ZoneSide, tab_divider};
use crate::tab_widget;
use crate::term_view;
use crate::theme;
use crate::workspace::{SessionTab, Workspace, dot_color, tab_display_width, tab_title};
use byteui::interaction::icons;
use iced_widget::core::mouse;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{MouseArea, column, container, row, text};

/// 终端相关消息(键盘/滚轮/选区/粘贴)该写去右侧共享终端条还是 SSH 面板
/// 自己的内嵌终端——两者可能同时在屏幕上,裸消息本身不带这个信息,靠
/// canvas 渲染时(`term_view::view`)烘焙进它构造的每条消息里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermTarget {
    Shared,
    SshPanel,
}

/// 键盘/粘贴事件此刻该写给右侧共享终端条还是 SSH 面板自己的内嵌终端。
/// 复用既有 `active_zone`(点击左右面板区任意位置就会更新,已经在驱动
/// `left_zone`/`right_zone` 的高亮边框,见 `App::set_active_zone`)——
/// SSH 面板在左侧且左侧是当前聚焦区时走 SSH 面板,否则走现状的共享
/// 终端条(不需要新增专门的终端焦点状态)。
pub(crate) fn keyboard_term_target(
    left_view: PanelKind,
    active_zone: Option<ZoneSide>,
) -> TermTarget {
    if left_view == PanelKind::Ssh && active_zone == Some(ZoneSide::Left) {
        TermTarget::SshPanel
    } else {
        TermTarget::Shared
    }
}

/// 终端栏：表头 + tab 栏 + （可能的错误文案）+ 当前激活 tab 的终端网格。
pub(crate) fn terminal_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::terminal_pane();
    let mut content = column![tab_bar(app, ws)].spacing(region.gap);

    if let Some(err) = &app.daemon_error {
        content = content.push(
            text(format!("⚠ {err}"))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().red),
        );
    }

    // OSC 133;D 的最近命令非零退出码提示（下一条命令开始时消失）。
    if let Some(code) = ws.tabs.get(ws.active).and_then(|t| t.last_exit)
        && code != 0
    {
        content = content.push(
            text(format!("exit {code}"))
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().red),
        );
    }

    // P1j 收敛：终端"审阅"按钮移除，会话审阅入口统一到右一对话列表。

    content = content.push(active_tab_view(app, ws));

    let body = container(content.spacing(region.gap).padding(region.padding))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        });

    // 底部状态栏(agent 态/resume/dozerd 持有说明)已按要求去掉——`body`
    // 自己的 `style` 已经用 `outer` 收了圆角(含底角),不需要额外元素
    // 补底角,直接就是这块 pane 的全部内容。
    container(body).width(width).height(Length::Fill).into()
}

/// tab 栏：两侧箭头翻页(到头变灰) + 每会话一个按钮(状态点 + 名称 + 关闭
/// ×)。新建会话走 Agent 面板"＋"(纯 Shell 也在其菜单里),终端 tab 栏
/// 不再放独立"＋"。P1L T5 验收返工：横向 scrollable(底部滚动条)
/// 换成索引窗口化 + `clip`——`on_scroll` 只认滚轮/拖拽，程序化滚动在本
/// app 自建循环里够不到，箭头翻页必须走状态驱动的窗口渲染。
pub(crate) fn tab_bar<'a>(
    app: &'a App,
    ws: &'a Workspace,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let widths: Vec<f32> = ws
        .tabs
        .iter()
        .map(|t| tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name)))
        .collect();
    let (first, can_left, can_right) = tab_widget::tab_window(
        &widths,
        4.0,
        byteui::theme::geometry::tab_bar_avail_px(),
        ws.term_tab_first,
    );

    let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = ws
        .tabs
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx >= first)
        .map(|(idx, tab)| {
            let title_hover_t = app.hover_progress(HoverId::TermTabItem(idx));
            let close_hover_t = app.hover_progress(HoverId::TermTabClose(idx));
            let armed = app.dragging_group(TabGroup::Terminal);
            tab_drag_surface(
                tab_item(
                    idx,
                    tab,
                    idx == ws.active,
                    title_hover_t,
                    close_hover_t,
                    app.hover_tooltip_ready(HoverId::TermTabItem(idx)),
                ),
                TabGroup::Terminal,
                idx,
                armed,
            )
        })
        .collect();

    // tab 列表进 clip 容器占 Fill,裁掉右侧溢出;左右箭头钉在裁剪区外.
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    let left_arrow = tab_widget::tab_arrow_button(
        icons::IconKind::ChevronLeft,
        can_left,
        Message::TermTabScroll(false),
    );
    let right_arrow = tab_widget::tab_arrow_button(
        icons::IconKind::ChevronRight,
        can_right,
        Message::TermTabScroll(true),
    );

    let tab_row = row![left_arrow, right_arrow, clipped]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);

    column![tab_row, tab_divider()].spacing(4).into()
}

/// 单个 tab：状态点（颜色见 `dot_color`）+ 名称的选中按钮，紧跟一个关闭
/// 按钮（点击 = detach，见 `Message::CloseTab` 的文档）。状态不再用文字
/// 胶囊表达，全部收敛到点点的颜色（goal.md）：各状态各自固定配色，常亮，
/// 不再有闪烁动画。
fn tab_item(
    idx: usize,
    tab: &SessionTab,
    active: bool,
    title_hover_t: f32,
    close_hover_t: f32,
    show_tooltip: bool,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let color = dot_color(tab.agent_state, tab.alive);
    // 状态点作 `panel_tab` 的 prefix。
    let dot = byteui::feedback::status::dot(color);

    tab_widget::panel_tab(
        tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
        active,
        title_hover_t,
        close_hover_t,
        Some(dot),
        None,
        Message::SelectTab(idx),
        Message::CloseTab(idx),
        show_tooltip,
        move |h| Message::Hover(HoverId::TermTabItem(idx), h),
        move |h| Message::Hover(HoverId::TermTabClose(idx), h),
    )
}

/// 给一块 tab 内容包上"拖拽换位"的感应层:内容本身仍是原来的交互(点标题
/// 选中/点 × 关闭全在内部),外层只补一个 `on_move`——因为子按钮只会吞掉
/// **按下**事件,光标在页签上移动的 `CursorMoved` 不会被吞,`on_move` 照常
/// 触发,据此发出 `TabDragMove`。真正"按住页签＝准备拖"由各选中处理
/// (`SelectTab`/`PreviewSelectTab`/`ProjectTabSwitch`)在按住瞬间把
/// `tab_drag` 置位,这里的 `on_move` 只认"当前拖的是本组"的时刻(见
/// `App::tab_drag_move` 的组校验),松开由 main.rs 发 `TabDragEnd`。这样
/// 点击选中与拖拽换位互不干扰,也和 `ColumnDrag` 同一套原始事件后端。
pub(crate) fn tab_drag_surface(
    content: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>,
    group: TabGroup,
    index: usize,
    armed: bool,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let area = MouseArea::new(content).on_move(move |_| Message::TabDragMove { group, index });
    if armed {
        let area = area.interaction(mouse::Interaction::Grabbing);
        return area.into();
    }
    area.into()
}

pub(crate) fn active_tab_view<'a>(
    app: &'a App,
    ws: &'a Workspace,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    match ws.tabs.get(ws.active) {
        Some(tab) => {
            let focused =
                keyboard_term_target(app.left_view, app.active_zone) == TermTarget::Shared;
            term_view::view(
                &tab.model,
                focused,
                TermTarget::Shared,
                focused.then(|| app.term_ime_preedit()).flatten(),
            )
        }
        None => container(
            text("暂无会话——到 Agent 面板点「＋」")
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().dim),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into(),
    }
}
