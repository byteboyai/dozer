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

use crate::app::{App, Message, PanelKind, TabGroup, ZoneSide};
use crate::workspace::Workspace;
use iced_widget::MouseArea;
use iced_widget::core::mouse;
use iced_widget::core::{Border, Element, Length};

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
    let pid = ws.project.as_ref().map(|p| p.id).unwrap_or_default();
    let display = crate::host_adapters::agent_panel::DozerDisplay::new(app, pid);
    let locale = app.locale.active_locale();
    let floating = crate::extensions::agent_context::expanded_panel(&ws.agent_context, locale)
        .map(|panel| panel.map(move |m| Message::AgentContext(pid, m)));
    let footer = Some(
        crate::extensions::agent_context::strip(&ws.agent_context, locale)
            .map(move |m| Message::AgentContext(pid, m)),
    );
    dozer_panel_agent::view::terminal::terminal_pane(
        &ws.agent,
        &display,
        dozer_panel_agent::display::TerminalSlots { floating, footer },
        width,
        outer,
    )
}

/// 终端 tab 栏"溢出下拉"浮层。**必须**在 `App::view` 顶层 `stack![base, ...]`
/// 里拼(同 `topbar::project_add_menu_popup`/`todo::todo_calendar_overlay`
/// 的既有套路)——`anchor`(`App::last_cursor` 快照)与 `app.window_size` 都是
/// 全窗口坐标系,若像先前那样嵌在 `tab_bar` 自己的局部布局里返回
/// `stack![base, menu]`,`positioned` 的 `Length::Fill` 只会撑满这个面板自己
/// 分到的那格(远小于整窗、且左上角并非窗口原点),换算出来的位置会跟真实
/// 点击位置对不上(验收反馈"菜单错位了")。
pub(crate) fn term_tab_overflow_popup<'a>(
    app: &'a App,
    ws: &'a Workspace,
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let pid = ws.project.as_ref().map(|p| p.id).unwrap_or_default();
    let display = crate::host_adapters::agent_panel::DozerDisplay::new(app, pid);
    dozer_panel_agent::view::terminal::overflow_popup(&ws.agent, &display)
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
    on_move: Message,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let _ = (group, index);
    let area = MouseArea::new(content).on_move(move |_| on_move.clone());
    if armed {
        let area = area.interaction(mouse::Interaction::Grabbing);
        return area.into();
    }
    area.into()
}
