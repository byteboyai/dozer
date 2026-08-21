// crates/dozer-app/src/terminal.rs
//! 终端宿主:项目自己终端页签(`TermTarget::Shared`)与 SSH 面板内嵌终端
//! (`TermTarget::SshPanel`)共用的目标标识 + 项目终端页签的宿主 view。
//!
//! `term_input`/`term_output`/`term_paste`/`term_ime_preedit` 等处理方法
//! 不在这里——它们的主体是可见性闸门 + 项目路由 + 读写 `Workspace` 字段,
//! 属于内核编排,留在 `app.rs`(同 `rail.rs` 里 `panel_select`/
//! `end_rail_drag` 留在内核的理由一致)。`ssh_terminal_pane`(SSH 面板自己
//! 的宿主 view)也不在这里,它是 `extensions::ssh` 的地盘。

use crate::app::{App, Message, PanelKind, ZoneSide, active_tab_view, tab_bar};
use crate::theme;
use crate::workspace::Workspace;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{column, container, text};

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
