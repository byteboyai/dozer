//! Workspace 的 view 层:load_more_button、agent 列表与卡片、工作区内容行、
//! agent picker、review 内容 pane、preview pane 等渲染函数。

use crate::app::{App, HoverId, Message, PanelKind, tab_divider};
use crate::chrome::homespace::home_panel_head_with_actions;
use crate::chrome::tab_widget::{
    PanelTabArgs, TabOverflowEntry, TabOverflowMenuArgs, panel_tab, tab_json_tree_mode_button,
    tab_overflow_button, tab_overflow_menu, tab_render_mode_button, tab_tabular_mode_button,
    tab_window,
};
use crate::extensions::conversations;
use crate::menu_spec::{MenuSpec, MenuSpecItem};
use crate::preview::{PreviewPane, PreviewTab, TabKind};
use crate::theme;
use crate::theme::terminal_font;
use byteui::interaction::icons;
use byteui::interaction::icons::IconKind;
use dozer_core::protocol::AgentKind;
use iced_widget::core::mouse;
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Border, Element, Length, Padding};
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable, text};

use super::*;

/// 会话详情"加载更多"按钮(2026-08-27):居中的 `<summary>` 样式的图标按钮,
/// hover 金边提示,点击追加下一页回合。`after_turn_index` 用 `entries.len()`
/// 当锚点——`ReviewEntry` 不携带原始 turn index,详情页按整段摊平加载,取
/// 条数近似下一步起点即可(跳过首尾折叠带来的少量误差在可接受范围)。
pub(crate) fn load_more_button<'a>(
    conversation_id: &'a str,
    after_turn_index: i64,
) -> iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let btn = iced_widget::button(
        text("加载更多…")
            .size(byteui::theme::font::caption_sm())
            .color(byteui::theme::color::current().dim),
    )
    .on_press(Message::Conversations(
        conversations::Message::DetailLoadMore(conversation_id.to_string(), after_turn_index),
    ))
    .padding(6)
    .style(|_t, _s| iced_widget::button::Style {
        background: None,
        text_color: byteui::theme::color::current().dim,
        ..iced_widget::button::Style::default()
    });
    container(btn)
        .width(Length::Fill)
        .align_x(iced_widget::core::Alignment::Center)
        .into()
}

/// 对话面板 session 详情的首屏回合页大小(2026-08-27):列表分页
/// (`extensions::conversations::CONVERSATION_PAGE_SIZE`,20)与详情页分页
/// (这里,200)含义不同,不要混用。
pub(crate) const CONVERSATION_DETAIL_PAGE_SIZE: u32 = 200;

/// 按 `AgentKind` 把会话 tab 分组,固定顺序 Claude → Codebuddy → Opencode
/// → Codex → Goose → Aider → V8agent → Unknown(与 `conversation_agents_present`
/// 同一份顺序),只返回非空分组(没有该 agent 的会话就不出现,面板不留空
/// 分组占位)。组内保持 `tabs` 原有顺序(tab 打开顺序)。返回下标而非
/// 引用——渲染时既要下标发 `Message::SelectTab(idx)`,又要用下标回查
/// `ws.tabs[idx]` 取展示字段,直接存下标比存 `&SessionTab` 省一次生命
/// 周期纠缠。
pub(crate) fn group_tabs_by_agent(tabs: &[SessionTab]) -> Vec<(AgentKind, Vec<usize>)> {
    const ORDER: [AgentKind; 8] = [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::Codex,
        AgentKind::Goose,
        AgentKind::Aider,
        AgentKind::V8agent,
        AgentKind::Unknown,
    ];
    ORDER
        .into_iter()
        .filter_map(|kind| {
            let idxs: Vec<usize> = tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| t.agent == kind)
                .map(|(i, _)| i)
                .collect();
            (!idxs.is_empty()).then_some((kind, idxs))
        })
        .collect()
}

/// Agent 列表面板(右面板区"Agent"视图的列表侧):按 `AgentKind` 分组展示
/// 当前项目的会话,组内保留 tab 打开顺序;点击一行 = `Message::SelectTab`
/// 切焦点(同终端 tab 栏点击效果)。
pub(crate) fn agent_list_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::agent_list_pane();
    let mut content = column![
        // 套用统一 panel head:暖金 `#dcc9a3` 的 Bot 图标 + "Agent" 标题 +
        // 1px 分割线;新建 agent 的"＋"按钮放到标题同一行的右侧(见
        // `home_panel_head_with_actions` 的 `actions` 参数),不再单独占一行。
        home_panel_head_with_actions(
            IconKind::Brain,
            "Agent",
            Some(agent_picker_toggle_button(app)),
        ),
    ]
    .spacing(region.gap);

    if ws.tabs.is_empty() {
        content = content.push(lh(text("暂无会话")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim)));
    } else {
        for (agent, idxs) in group_tabs_by_agent(&ws.tabs) {
            content = content.push(
                row![
                    icons::view(
                        agent_icon(agent),
                        byteui::theme::icon_size::row(),
                        agent_dot_color(agent),
                    ),
                    lh(text(format!("{}（{}）", agent.display_label(), idxs.len()))
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().dim)),
                ]
                .align_y(iced_widget::core::alignment::Vertical::Center)
                .spacing(8),
            );
            for idx in idxs {
                content = content.push(agent_card(ws, idx));
            }
        }
    }

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

/// Agent 面板里单条会话卡片,三行:1) 图标 + agent 名(+ `(model, mode)`,
/// 只要有一项能读到就跟名字拼一起,两项都没有就只显示名字);2) "当前
/// 工作内容"(优先 Todo 任务派发出来时反查到的任务标题,拿不到就用
/// transcript 最后活动摘要兜底,都没有就省略)紧跟工作区文案(分支名+脏标),
/// 同一行不换行(卡片改版要求,work_content 在前);3) 状态点 + 状态文字。
/// 整卡可点选中该 tab(`idx == ws.active` 时金色边框高亮、无背景;hover 时
/// 显示 `CARD` 背景 + 金色边框)。Task 6 起 `TodoInfo` 自带派发记录,
/// `task_title_for_session` 不再需要 `app` 侧的元数据表(见 todo.rs)。
pub(crate) fn agent_card<'a>(
    ws: &'a Workspace,
    idx: usize,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tab = &ws.tabs[idx];
    let active = idx == ws.active;

    let model_label = tab.llm_model.as_deref().map(format_model_label);
    let mode_label = tab.permission_mode.as_deref();
    let llm_mode_value = match (&model_label, mode_label) {
        (Some(m), Some(mo)) => Some(format!("{m}, {mo}")),
        (Some(m), None) => Some(m.clone()),
        (None, Some(mo)) => Some(mo.to_string()),
        (None, None) => None,
    };
    let title = tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name);
    let title_text = match &llm_mode_value {
        Some(v) => format!("{title} ({v})"),
        None => title,
    };

    let mut lines = column![
        text(title_text)
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
    ]
    .spacing(4);

    let work_content = ws
        .todo
        .task_title_for_session(&tab.info.id)
        .map(str::to_string)
        .or_else(|| tab.last_activity.clone());

    let (branch, dirty) = match &tab.workspace_override {
        Some(w) => (w.branch.as_deref(), w.dirty),
        None => (ws.project_panel.branch(), ws.project_panel.dirty()),
    };
    lines = lines.push(work_content_and_workspace_row(
        work_content.as_deref(),
        branch,
        dirty,
    ));

    lines = lines.push(
        row![
            byteui::feedback::status::dot(dot_color(tab.agent_state, tab.alive)),
            text(agent_state_label(tab.agent_state))
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    );

    button(container(lines).padding(10))
        // 不是左侧 tab 栏本身,发 `SelectTabNoDrag` 而不是 `SelectTab`——
        // 后者会顺带武装左侧 tab 栏的拖拽状态机,导致点这张卡片后只要
        // 光标划过 tab 栏就被误判成"正在拖 tab"而错误换位(见
        // `Message::SelectTab` 文档,2026-08-17 修的真实 bug)。
        .on_press(Message::SelectTabNoDrag(idx))
        .width(Length::Fill)
        .style(byteui::interaction::cards::button_card(
            active,
            byteui::theme::color::current().card,
        ))
        .into()
}

/// "当前工作内容"(有就显示,没有就省略)+ 工作区(`@分支名` + 脏标,所有
/// agent 都显示)合并一行、不换行,work_content 排在工作区前面(卡片
/// 改版要求)。工作区不再用"工作区:"文字标签,前缀改成 `@`——跟卡片其余
/// 行的极简风格对齐。分支名规则不变:无分支(非 git 项目)显示 `—`;有
/// 未提交改动时分支名后缀 `(Uncommitted)`——跟 `extensions/files.rs`
/// 里分支切换菜单当前分支带脏标时的既有文案(`n.push_str("(Uncommitted)")`,
/// 见该文件约第 1409 行)保持同一措辞,不新造一套脏标文案。
pub(crate) fn work_content_and_workspace_row(
    work_content: Option<&str>,
    branch: Option<&str>,
    dirty: bool,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let workspace_value = match branch {
        Some(b) if dirty => format!("{b}(Uncommitted)"),
        Some(b) => b.to_string(),
        None => "—".to_string(),
    };
    let value = match work_content {
        Some(w) => format!("{w}  @{workspace_value}"),
        None => format!("@{workspace_value}"),
    };
    text(value)
        .size(byteui::theme::font::caption())
        .color(byteui::theme::color::current().dim)
        .into()
}

/// Agent 面板头部"＋"按钮:点击切换 `agent_picker_open`,弹出 agent
/// 选择菜单(`agent_picker_popup`)。样式与顶栏页签行的"＋"一致——无背景、
/// Lucide `SquarePlus` 图标、静止灰(`DIM`)、hover 平滑过渡到金(`GOLD`),
/// 由 `HoverId::AgentPickerToggle` + `MouseArea` 驱动同一套悬停动画。
pub(crate) fn agent_picker_toggle_button<'a>(
    app: &App,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    icons::icon_button_entry(
        icons::IconKind::SquarePlus,
        byteui::theme::icon_size::row(),
        false,
        false,
        app.hover_progress(HoverId::AgentPickerToggle),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::AgentPickerToggle,
        |hovered| Message::Hover(HoverId::AgentPickerToggle, hovered),
        "新建 Agent 会话",
    )
}

/// `agent_picker_popup` 的原生菜单版本,纯数据组装——九个选项与旧版完全
/// 一致(七 agent + 分隔线 + Git Shell/OS Shell),agent 图标用各自专属色
/// (`agent_dot_color`)、文字用 BODY。仅 macOS 编译,非 mac 平台继续走
/// `agent_picker_popup` 的 iced 弹层。
#[cfg(target_os = "macos")]
pub(crate) fn agent_picker_items() -> Vec<crate::chrome::native_menu::Item<Message>> {
    crate::menu_spec::to_native(agent_picker_spec())
}

/// Agent 选择器菜单内容——native(`agent_picker_items`)和 iced fallback
/// (`agent_picker_popup`)共用同一份数据，只在这里组装一次。
pub(crate) fn agent_picker_spec() -> MenuSpec<Message> {
    let agents: [(&str, PickerLaunch); 7] = [
        ("Aider", PickerLaunch::Agent(Some(AgentKind::Aider))),
        ("Claude", PickerLaunch::Agent(Some(AgentKind::Claude))),
        ("CodeBuddy", PickerLaunch::Agent(Some(AgentKind::Codebuddy))),
        ("Codex", PickerLaunch::Agent(Some(AgentKind::Codex))),
        ("Goose", PickerLaunch::Agent(Some(AgentKind::Goose))),
        ("OpenCode", PickerLaunch::Agent(Some(AgentKind::Opencode))),
        ("v8agent", PickerLaunch::Agent(Some(AgentKind::V8agent))),
    ];
    let shells: [(&str, PickerLaunch); 2] = [
        ("Git Shell", PickerLaunch::Git),
        ("OS Shell", PickerLaunch::Agent(None)),
    ];
    let mk = |label: &str, launch: PickerLaunch| {
        let (icon, icon_color) = match launch {
            PickerLaunch::Agent(Some(kind)) => (agent_icon(kind), agent_dot_color(kind)),
            PickerLaunch::Agent(None) => (
                IconKind::SquareTerminal,
                byteui::theme::color::current().body,
            ),
            PickerLaunch::Git => (IconKind::GitBranch, byteui::theme::color::current().body),
        };
        MenuSpecItem::entry_tinted(icon, icon_color, label, Message::AgentPickerSelect(launch))
    };
    let mut spec: MenuSpec<Message> = agents
        .into_iter()
        .map(|(label, launch)| mk(label, launch))
        .collect();
    spec.push(MenuSpecItem::separator());
    spec.extend(shells.into_iter().map(|(label, launch)| mk(label, launch)));
    spec
}

/// Agent 选择菜单浮层:固定挂在窗口右上角("＋"按钮下方——该按钮
/// 就在最靠右的 Agent 面板头部,近似等于窗口右上角),九个选项按标签
/// 首字母顺序排列:Aider/Claude/CodeBuddy/Codex/Goose/Git Shell/OpenCode/
/// v8agent/OS Shell(验收反馈,2026-08-21;此前是手写的固定顺序,不便
/// 找到目标 agent)。跟项目树右键菜单(`context_menu_popup`)同款按钮
/// 样式,但不需要像素坐标定位——同 `delete_confirm_popup` 一样固定
/// padding 摆位。`ws.agent_picker_open` 为假时返回空视图,调用方
/// (`App::view`)据此决定要不要把这层塞进 `stack!`。
pub(crate) fn agent_picker_popup(
    ws: &Workspace,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !ws.agent_picker_open {
        return column![].into();
    }
    // 常规 agent 按标签首字母排序。"Git Shell" / "OS Shell" 归到菜单最底部,
    // 与上方 agent 用 1px 分割线(`crate::chrome::menu::separator`)分组隔开。
    // 菜单内容组装收拢到 `agent_picker_spec()`,这里只做 iced 转换。
    let list = crate::menu_spec::to_iced(
        agent_picker_spec(),
        Length::Fixed(byteui::theme::geometry::menu_item_width()),
    );
    // 右上角固定偏移:48px 避开顶栏,16px 避开窗口右边缘。这是估算值,
    // 不是像素级对齐"＋"按钮(spec 明确"不算点击坐标")——Task 4 最后
    // 一步的人工验收里如果视觉上偏得明显,回来调这两个数字即可,不影响
    // 其余逻辑。
    container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Right)
        .align_y(iced_widget::core::alignment::Vertical::Top)
        .padding(Padding {
            top: 48.0,
            left: 0.0,
            right: 16.0,
            bottom: 0.0,
        })
        .into()
}

/// 关 Agent 面板 tab 前的确认弹窗:目标会话处于 Running/AwaitingInput 时才
/// 弹(分流见 `app::update` `Message::CloseTab`)。窗口级 overlay,复用
/// `crate::dialog::confirm` 骨架(同文件树删除/主机删除确认框)。`pending_close_tab`
/// 存的是被点 × 的下标,这里按它取出标题写进文案——下标在弹窗存活期间不会被
/// 重排(遮罩挡住 base 交互),取得到就取,取不到(极端竞态)兜底成"会话"。
pub(crate) fn agent_close_confirm_popup<'a>(
    ws: &'a Workspace,
    window_width: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(idx) = ws.pending_close_tab else {
        return column![].into();
    };
    let title = ws
        .tabs
        .get(idx)
        .map(|t| crate::workspace::hook::tab_title(t.agent, t.cwd.as_deref(), &t.info.name))
        .unwrap_or_else(|| "会话".to_string());
    crate::dialog::confirm(
        crate::dialog::ConfirmDialog {
            icon: None,
            title: format!("关闭 \"{title}\"?"),
            description: "该会话仍在运行 / 等待输入,关闭会结束此会话。".to_string(),
            cancel_label: "取消".to_string(),
            cancel_msg: Message::TermTabCloseCancel,
            confirm_label: "关闭".to_string(),
            confirm_msg: Message::TermTabCloseConfirm,
            confirm_color: byteui::theme::color::current().red,
            content_spacing: 8.0,
        },
        window_width,
    )
}

/// 审阅内容的 webview 期望清单(`preview::desired_webviews` 同款语义)。
/// 没有审阅内容 / 出错 / 空回合区间时返回空清单——`sync_webview_pool`
/// 的 `retain` 会据此销毁 webview,不需要额外的隐藏逻辑。有内容时返回
/// 唯一一条,URL 带 `rv.nonce` 当查询参数,内容变化(`Message::ReviewLoaded`
/// 落地新 entries)时 nonce 递增、URL 变化,逼 `sync_webview_pool` 重新
/// `load_url`(同 `preview.rs::PreviewTab.reload_nonce` 的手法)。`id`
/// 固定填 0,真正的池 key 由调用方(`App::preview_desired`)加
/// `CONVERSATION_REVIEW_ID_OFFSET` 决定——这个面板任意时刻只有一份内容,
/// 不需要 Files/Project 那种按 tab id 分池的能力。
pub(crate) fn review_webview_spec(review: Option<&ReviewView>) -> Vec<crate::preview::WebviewSpec> {
    let Some(rv) = review else {
        return Vec::new();
    };
    if rv.error.is_some() || rv.entries.is_empty() {
        return Vec::new();
    }
    vec![crate::preview::WebviewSpec {
        id: 0,
        url: format!("dozer://review-trace/host.html?_r={}", rv.nonce),
        visible: true,
        editor_binding: None,
    }]
}

/// 会话审阅内容面板(右面板区"对话"视图的内容侧):直接读 `ws.review`,
/// 不经过 `ws.preview` 的 tab 系统——新外壳下审阅是独立面板,不再是
/// 预览 tab 条里的一个 tab。
pub(crate) fn review_content_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::review_content_pane();
    // 内容侧标题栏:左侧名字信息面板(会话/agent 态) + 右侧"收起/展开
    // 列表列"按钮(与 Todo/Database/SSH/Agent 内容侧统一)。列表列收起后
    // 本面板拿满配对宽度,按钮仍在此处可见以便恢复。
    let header = container(
        row![home_panel_head_with_actions(
            IconKind::BotMessageSquare,
            "会话",
            Some(app.list_collapse_button(
                PanelKind::Conversations,
                app.list_collapsed(PanelKind::Conversations),
                HoverId::ConversationsListCollapse,
                "收起列表",
                "展开列表",
                Message::TogglePanelListCollapse(PanelKind::Conversations),
                move |h| { Message::Hover(HoverId::ConversationsListCollapse, h) },
            )),
        )]
        .width(Length::Fill),
    )
    .padding(theme::region::project_pane().padding);
    let mut content = column![header].spacing(region.gap);

    if ws.review.is_some() {
        let body = review_content(column![].spacing(region.gap), ws);
        content = content.push(
            Scrollable::new(body)
                .width(Length::Fill)
                .height(Length::Fill)
                .direction(scrollable::Direction::Vertical(
                    byteui::interaction::scrollbar::scrollbar(),
                ))
                .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
        );
    } else {
        content = content.push(
            container(lh(text("暂无审阅内容——点击左侧对话列表中的对话开始审阅")
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().dim)))
            .width(Length::Fill)
            .height(Length::Fill),
        );
    }

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

/// 配对视图内部"列表:内容"的 `FillPortion` 权重对。`FillPortion` 在扣掉
/// 中间固定宽的分隔线之后按权重分剩余空间,与 `pair_content_width` 同源。
pub(crate) fn split_portions(split: f32) -> (u16, u16) {
    let list = (split * 10_000.0).round() as u16;
    let content = ((1.0 - split) * 10_000.0).round() as u16;
    (list, content)
}

/// 文件树目录/文件名行的字号：与终端字号(`terminal_font`)对齐（含全局 UI
/// scale），配合下面的 `LineHeight::Relative(line_height_factor)` 让每行行高
/// 等于终端行距，目录/文件列表不再比终端稀疏。
pub(crate) fn tree_row_font_size() -> f32 {
    terminal_font::size() * byteui::theme::icon_size::scale()
}

/// 统一行高：把一段文字的行高设为终端行高
/// (`terminal_font::line_height_factor()` = 1.2)，让各面板列表/正文行的行距
/// 与文件树、终端观感一致。`size`/`color` 等仍由调用方设置，这里只补行高。
pub(crate) fn lh<'a>(
    t: iced_widget::text::Text<'a, iced_widget::Theme, iced_renderer::Renderer>,
) -> iced_widget::text::Text<'a, iced_widget::Theme, iced_renderer::Renderer> {
    t.line_height(LineHeight::Relative(terminal_font::line_height_factor()))
}

/// `PanelKind::Files` 在没有打开项目时的占位:"未打开项目"提示 + 最近项目
/// 列表(点击即打开)。这是一个项目切换器,不是文件树的一部分,`files` 模块
/// 不认识 `ws.recent_projects`/`Message::ProjectSelect` 这些核心概念,留在
/// 内核(现有 `project_pane` 的 `None` 分支的搬家版本,渲染结构原样保留)。
pub(crate) fn no_project_placeholder<'a>(
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::project_pane();
    let mut header = column![].spacing(region.gap).width(Length::Fill);
    let tree_col = column![].spacing(region.gap);
    header = header.push(
        text("未打开项目")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
    );
    for p in &ws.recent_projects {
        header = header.push(
            button(
                text(p.name.clone())
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().cream),
            )
            .on_press(Message::ProjectSelect(p.id))
            .style(|_t, _s| button::Style {
                background: None,
                text_color: byteui::theme::color::current().cream,
                ..button::Style::default()
            }),
        );
    }
    let body = container(
        column![
            header,
            Scrollable::new(tree_col)
                .width(Length::Fill)
                .height(Length::Fill)
                .direction(scrollable::Direction::Vertical(
                    byteui::interaction::scrollbar::scrollbar()
                ))
                .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
        ]
        .spacing(region.gap),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(region.padding)
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: region.background.map(Into::into),
        border: outer,
        ..container::Style::default()
    });
    container(body).width(width).height(Length::Fill).into()
}

/// 原生预览里文件内 Find 的「查询」与「替换」两个输入框各自的外框壳。
/// text_input 本体是透明无边框的(`bare`/`unframed` 或带 8px 自己 padding),
/// 真正的圆角框底/描边由这里画齐整:
/// - 底色用 `colors.bg` —— 与 `code_editor::editor_style` 画编辑器同一 `bg`,
///   让文件内搜索时敲进去的词,底色跟右侧正浏览的代码看板完全一致(需求:
///   "输入框背景色和 editor 一致")。
/// - 有内容(`active`)整框描金,否则普通 `colors.border` 边色(沿用单字段
///   chip 的"非空即高亮"约定)。
pub(crate) fn find_field_shell<'a>(
    inner: iced_widget::core::Element<
        'a,
        crate::app::Message,
        iced_widget::Theme,
        iced_renderer::Renderer,
    >,
    colors: byteui::theme::color::ColorTokens,
    vert: f32,
    horiz: f32,
    active: bool,
) -> iced_widget::core::Element<'a, crate::app::Message, iced_widget::Theme, iced_renderer::Renderer>
{
    use iced_widget::core::Length;
    iced_widget::container(inner)
        .width(Length::Fill)
        .padding([vert, horiz])
        .style({
            // 框内底色同右侧代码编辑器(`colors.bg`)——敲的词跟被找的正文
            // 底色一致,像"嵌进"编辑区的一扇窗;外层整条 Find/替换组件另有
            // 自己的 `card` 底色(见 `find_rows` 的包裹),两层色阶分得开。
            let bg = colors.bg;
            let border_color = if active { colors.gold } else { colors.border };
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(bg.into()),
                border: iced_widget::core::Border {
                    color: border_color,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..iced_widget::container::Style::default()
            }
        })
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

/// Project 面板右配对的预览 pane,复用与 Files 预览同一套渲染。独立调一个
/// 新 `PreviewPaneKind`,让项目链接打开的文件进 `ws.project_preview` 而不是
/// 冲进 Files 预览(见 Task 12 `OpenLink` 路由)。
#[derive(Debug, Clone, Copy)]
pub(crate) enum PreviewPaneKind {
    Files,
    Project,
}

pub(crate) fn preview_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    preview_pane_for(app, ws, PreviewPaneKind::Files, width, outer)
}

pub(crate) fn project_preview_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    preview_pane_for(app, ws, PreviewPaneKind::Project, width, outer)
}

/// 空白页(`TabKind::Blank`)的 Finder "Get Info" 风格信息卡:folder icon +
/// 项目名(2 倍 title 字号,奶油色),下方四行 dim label / cream value ——
/// 位置 / 大小 / 创建时间 / 修改时间。四行 label 段固定宽 + 右对齐,`:` 严格
/// 垂直对齐于 `stats_padding_left + label_colon_w` 那条竖线;头部 icon 装进
/// 同宽的槽位(`icon_slot_w`)右对齐,右缘也压在这条竖线上。`preview.blank_info`
/// 为 `None` 时四行 value 都画 `—` 占位(后台还在跑)。无项目(`ws.project`
/// 为 `None`,启动初帧 / 切项目中间)只画头部,不画 stats,避免"位置: —"
/// 这种半成品。
fn preview_blank_info_card<'a>(
    ws: &'a Workspace,
    preview: &'a PreviewPane,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let theme_colors = byteui::theme::color::current();
    // 头部名称取**项目根目录 basename**(`path.file_name()`)而不是
    // `ProjectInfo::name`(用户给项目起的"显示名",如 "Dozer AI Coder"):
    // 空白页想表达"打开的是哪个目录",与 `ws.preview_blank_info_card` 其它
    // 字段(位置/大小/创建/修改)语义自洽。无项目时回退到 `name` 占位
    // (不太可能触发——`active_is_blank` 与项目加载在 `apply_pending_blank_info`
    // 那一侧就已经过滤过了)。
    let header_label = project_root_dir_name(ws)
        .or_else(|| ws.project.as_ref().map(|p| p.name.clone()))
        .unwrap_or_default();
    // 四行 stats 的两条对齐锚线(`:` 竖线 + 行首 padding)在 header 之前就定
    // 好,因为头部 icon 要借同一条 `:` 竖线右对齐(见下面的 `icon_slot_w`)。
    // `label_colon_w=96` 容下"修改时间:"(5 CJK + 冒号 @body_font 14,实测约
    // 90px):之前用 80 时 `创建时间:`/`修改时间:` 被 iced 强行换行成
    // "创建时\n间:" / "修改时\n间:",丑且吞了 `:`,已修。
    let label_colon_w: f32 = 96.0;
    let stats_padding_left: f32 = 16.0;
    // icon 槽位宽 = stats 的行首 padding + label 列宽,槽位内右对齐 → icon
    // 右缘正好落在四行 label 的 `:` 那条竖线上。icon 比原来小一档(96 → 64),
    // 不再压过 2 倍 title 的项目名,也不会撑得整张卡头重脚轻。
    let icon_slot_w: f32 = stats_padding_left + label_colon_w;
    // 头部 icon 用 Lucide `folder-dot`(一个底角圆点暗示"当前位置/选中"
    // 的语义,比纯 folder 更贴合空白页"信息卡"语境);文件夹名字号直接
    // ×2(从 title() 16 → 32),与下方 body() 14 拉开视觉主次。
    let icon_size: f32 = 64.0;
    let header_spacing: f32 = 20.0;
    let header = row![
        container(icons::view(
            icons::IconKind::FolderDot,
            icon_size,
            theme_colors.dim
        ))
        .width(Length::Fixed(icon_slot_w))
        .align_x(iced_widget::core::alignment::Horizontal::Right),
        text(header_label)
            .size(byteui::theme::font::title() * 2)
            .color(theme_colors.cream),
    ]
    .spacing(header_spacing)
    .align_y(iced_widget::core::Alignment::Center);

    let Some(project) = ws.project.as_ref() else {
        return container(
            column![header]
                .spacing(0)
                .align_x(iced_widget::core::Alignment::Start),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into();
    };

    let info = preview.blank_info.as_ref();
    let size_value = info.map_or("—".to_string(), |i| {
        format!(
            "{} 字节,共 {} 个项目",
            with_thousands_separator(i.size_bytes),
            with_thousands_separator(i.file_count)
        )
    });
    let created_value = info
        .and_then(|i| i.created)
        .map(format_time_cn)
        .unwrap_or_else(|| "—".to_string());
    let modified_value = info
        .and_then(|i| i.modified)
        .map(format_time_cn)
        .unwrap_or_else(|| "—".to_string());

    let dim = theme_colors.dim;
    let cream = theme_colors.cream;
    let body_font = byteui::theme::font::body();
    // 不抽 label/value 闭包:`text` 返回 `Text<'a>` 含生命周期,把 `&str` 闭包
    // 返回 `Text<'b>` 会撞 iced 的 invariant 约束(报错点就是这个),就地写
    // 反而短。
    // `:` 对齐方案:每个 label 段包进 width(Fixed(label_colon_w)) + 右对齐
    // 的 container,于是该行 `:` 落在 `label_colon_w` 框右边界;stats column
    // 再用 `stats_padding_left` 把整组横向推 16px,这条竖线才落到头部 icon
    // 的右缘上——两条锚线由上面的 `icon_slot_w` 绑成同一条。
    let stats = column![
        row![
            container(text("位置:").size(body_font).color(dim))
                .width(Length::Fixed(label_colon_w))
                .align_x(iced_widget::core::alignment::Horizontal::Right),
            text(project.path.clone()).size(body_font).color(cream),
        ]
        .spacing(8),
        row![
            container(text("大小:").size(body_font).color(dim))
                .width(Length::Fixed(label_colon_w))
                .align_x(iced_widget::core::alignment::Horizontal::Right),
            text(size_value).size(body_font).color(cream),
        ]
        .spacing(8),
        row![
            container(text("创建时间:").size(body_font).color(dim))
                .width(Length::Fixed(label_colon_w))
                .align_x(iced_widget::core::alignment::Horizontal::Right),
            text(created_value).size(body_font).color(cream),
        ]
        .spacing(8),
        row![
            container(text("修改时间:").size(body_font).color(dim))
                .width(Length::Fixed(label_colon_w))
                .align_x(iced_widget::core::alignment::Horizontal::Right),
            text(modified_value).size(body_font).color(cream),
        ]
        .spacing(8),
    ]
    .spacing(8)
    .padding(Padding {
        top: 0.0,
        bottom: 0.0,
        left: stats_padding_left,
        right: 0.0,
    });

    let card = column![header, stats]
        .spacing(28)
        .align_x(iced_widget::core::Alignment::Start);

    container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

/// `n` 插入千位分隔符(中文惯例就是半角逗号,与英文一致)。`23456` →
/// `"23,456"`;`0` → `"0"`。纯展示,不损失精度。
fn with_thousands_separator(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i != 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

/// SystemTime → 中文 `"YYYY年M月D日 星期X HH:MM"` 形式(无秒)。Howard
/// Hinnant 的 `civil_from_days` 同款算法(见 `extensions/git_log.rs::1283`,
/// 那里只输出 `YYYY-MM-DD HH:MM:SS`,这里改成中文 + 加星期)。失败(平台不
/// 支持 / 1970 之前)时回退 `"未知"`。
fn format_time_cn(t: std::time::SystemTime) -> String {
    let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let total_secs = dur.as_secs();
    let days = (total_secs / 86_400) as i64;
    let secs_of_day = total_secs % 86_400;
    let (h, m, _) = (
        secs_of_day / 3600,
        (secs_of_day / 60) % 60,
        secs_of_day % 60,
    );
    let (y, mo, d) = civil_from_days(days);
    // 1970-01-01 是星期四,加天数 mod 7 拿到 0..6,offset = 周一 = 0
    let weekday = (((days + 3) % 7) + 7) % 7;
    let weekday_cn = [
        "星期一",
        "星期二",
        "星期三",
        "星期四",
        "星期五",
        "星期六",
        "星期日",
    ][weekday as usize];
    format!(
        "{y}年{mo}月{d}日 {weekday_cn} {h:02}:{m:02}",
        y = y,
        mo = mo,
        d = d,
        weekday_cn = weekday_cn,
        h = h,
        m = m,
    )
}

/// Howard Hinnant 的 `civil_from_days`:Unix epoch 起的天数 → (年, 月, 日)。
/// 范围覆盖 1970..=2100,足够文件系统时间戳用。与 `git_log.rs` 的同名函数
/// 同源(那个是模块私有,这里复制一份给空白页信息卡用)。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// 当前项目根目录的 basename(`ws.project.path` 的最后一段路径分量)。
/// 与 `ProjectInfo::name`(用户创建项目时给的"显示名"——`"Dozer AI Coder"`
/// 这种)刻意区分:空白页 / 文件 tab 标题想表达的是"打开的是哪个目录",
/// `name` 偏品牌文案的语义,不该混。拿不到文件名(尾部是 `..`/`/` 等)
/// 回退到 `path.display()` 完整字符串。无项目时返回 `None`。
pub(crate) fn project_root_dir_name(ws: &Workspace) -> Option<String> {
    let project = ws.project.as_ref()?;
    let path = std::path::Path::new(&project.path);
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .or_else(|| Some(path.display().to_string()))
}

/// 预览 tab 的渲染标题。`TabKind::Blank` 的占位 tab 返回当前项目根目录的
/// basename(见 [`project_root_dir_name`])——而不是数据层 `placeholder_tab`
/// 写死的 `"空白"` 或 `ProjectInfo::name` 这种"显示名"。切项目时标题自然跟
/// 着 `ws.project.path` 走,不需要回写 tab 字段。没项目(启动初帧 / 切项目中
/// 间帧)时回退 `tab.title`,即原 `"空白"`,保留原语义。宽度预算与实际渲染
/// 共用同一份 helper,避免选错宽度导致标题裁切。
pub(crate) fn preview_tab_display_title(ws: &Workspace, tab: &PreviewTab) -> String {
    if matches!(tab.kind, crate::preview::TabKind::Blank)
        && let Some(root) = project_root_dir_name(ws)
    {
        return root;
    }
    tab.title.clone()
}

/// 预览 pane 的共同渲染(Files 预览与 Project 面板右配对复用同一套 tab
/// 栏/原生编辑器/占位文案逻辑,只是状态取自 `ws.preview` 还是
/// `ws.project_preview`、消息与前缀路由到哪套)不同。
///
/// **空白页 tab(`TabKind::Blank`)的标题显示项目根目录 basename**:数据层
/// `placeholder_tab` 的 title 仍写死 `"空白"`(语义保留,model 不依赖
/// workspace 状态);view 在渲染时统一通过 [`preview_tab_display_title`]
/// 替换,这样 `PreviewPane::clear_all`/`adopt_project` 切换项目时,
/// 标题会自然跟着 `ws.project.path` 的 basename 走,不需要回写 tab 字段。
/// 宽度估计和实际渲染共用同一份 helper,避免选错宽度导致标题裁切。
pub(crate) fn preview_pane_for<'a>(
    app: &'a App,
    ws: &'a Workspace,
    kind: PreviewPaneKind,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // tab 栏:箭头翻页(到头变灰) + 每 tab 选择按钮 + 关闭 ×。tab 只能由项目树
    // 点击/会话恢复产生——面板本身已不再有"打开文件…"按钮或地址栏(P1 后续
    // 反馈:文件预览与浏览器彻底分离,文件只走项目树入口)。
    // P1L T5 验收返工:同 term `tab_bar`,横向 scrollable 换成索引窗口化 + clip.
    let region = theme::region::preview_pane();
    let (preview, tab_first, error) = match kind {
        PreviewPaneKind::Files => (&ws.preview, ws.preview_tab_first, &ws.preview_error),
        PreviewPaneKind::Project => (
            &ws.project_preview,
            ws.project_preview_tab_first,
            &ws.project_preview_error,
        ),
    };
    // 状态取的是一份只读引用,后续渲染把对应的消息/前缀按 `kind` 选好。
    // `move` 只捕获 `PreviewPaneKind`(Clone/Copy),多余生命周期问题一并消掉。
    let item_hover = move |idx| match kind {
        PreviewPaneKind::Files => HoverId::PreviewTabItem(idx),
        PreviewPaneKind::Project => HoverId::ProjectPreviewTabItem(idx),
    };
    let close_hover = move |idx| match kind {
        PreviewPaneKind::Files => HoverId::PreviewTabClose(idx),
        PreviewPaneKind::Project => HoverId::ProjectPreviewTabClose(idx),
    };
    let tab_group = match kind {
        PreviewPaneKind::Files => crate::app::TabGroup::Preview,
        PreviewPaneKind::Project => crate::app::TabGroup::ProjectPreview,
    };
    let select_msg = move |idx| match kind {
        PreviewPaneKind::Files => Message::PreviewSelectTab(idx),
        PreviewPaneKind::Project => Message::ProjectPreviewSelectTab(idx),
    };
    let close_msg = move |idx| match kind {
        PreviewPaneKind::Files => Message::PreviewCloseTab(idx),
        PreviewPaneKind::Project => Message::ProjectPreviewCloseTab(idx),
    };
    let toggle_msg = move |idx| match kind {
        PreviewPaneKind::Files => Message::PreviewToggleRenderMode(idx),
        PreviewPaneKind::Project => Message::ProjectPreviewToggleRenderMode(idx),
    };
    let overflow_toggle_msg = move || match kind {
        PreviewPaneKind::Files => Message::PreviewTabOverflowToggle,
        PreviewPaneKind::Project => Message::ProjectPreviewTabOverflowToggle,
    };
    let overflow_hover = move || match kind {
        PreviewPaneKind::Files => HoverId::PreviewTabOverflow,
        PreviewPaneKind::Project => HoverId::ProjectPreviewTabOverflow,
    };
    let render_mode_hover = move || match kind {
        PreviewPaneKind::Files => HoverId::PreviewRenderMode,
        PreviewPaneKind::Project => HoverId::ProjectPreviewRenderMode,
    };
    let json_tree_mode_hover = move || match kind {
        PreviewPaneKind::Files => HoverId::PreviewJsonTreeMode,
        PreviewPaneKind::Project => HoverId::ProjectPreviewJsonTreeMode,
    };
    let tabular_mode_hover = move || match kind {
        PreviewPaneKind::Files => HoverId::PreviewTabularMode,
        PreviewPaneKind::Project => HoverId::ProjectPreviewTabularMode,
    };
    // Find 条与编辑器共享同一份"按面板选消息/悬停态"手法。消息统一走带
    // `PanelKind` 的顶层 `Message::PreviewFind*`(同 `PreviewSaveActive`,一条
    // 消息两面板通吃,Files/Project 由 `PanelKind` 区分)。
    let find_panel = move || match kind {
        PreviewPaneKind::Files => PanelKind::Files,
        PreviewPaneKind::Project => PanelKind::Project,
    };

    let widths: Vec<f32> = preview
        .tabs()
        .iter()
        .map(|t| preview_tab_display_width(&preview_tab_display_title(ws, t)))
        .collect();
    let window = tab_window(
        &widths,
        4.0,
        app.preview_tab_bar_avail_px(find_panel()),
        tab_first,
    );

    let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = preview
        .tabs()
        .iter()
        .enumerate()
        .filter(|(idx, _)| (window.first..window.visible_end).contains(idx))
        .map(|(idx, tab)| {
            let active = idx == preview.active_idx();
            let title_hover_t = app.hover_progress(item_hover(idx));
            let close_hover_t = app.hover_progress(close_hover(idx));
            // 就地可写的原生 tab 有未保存改动:标题后缀 ` *`(2026-09-06)。宽度
            // 预算仍按 `tab.title`(不带星)估,最坏多一个字符略挤,不换行折叠。
            // 空白页 tab(`TabKind::Blank`)的标题走 [`preview_tab_display_title`]
            // 替换成项目根名(见该函数文档),宽度预算与渲染共用同一个 helper。
            let mut display_title = preview_tab_display_title(ws, tab);
            if tab.dirty {
                display_title.push_str(" *");
            }
            // index 0 的 `Blank` 占位 tab 不可关闭:它上面的 × 点击等同于
            // "选中空白页"(不真关),与 SSH/数据库面板 tab 条最前面那个固定
            // "空白"占位(`app.rs::ssh_tab_bar` 的 `on_close: SelectBlankTab`)
            // 完全同一套做法——数据层 `PreviewPane::close(0)` 另有兜底 no-op。
            let is_placeholder = matches!(tab.kind, crate::preview::TabKind::Blank);
            // 页签右键菜单:仅对真实文件 tab(`TabKind::File`,排除 index 0 的
            // `Blank` 占位)生效——占位 tab 没对应文件,没有可操作的右键目标。
            // 这里用循环变量里的 `tab`(PreviewTab)取路径,必须在下面
            // `let tab = panel_tab(...)` 把它遮蔽之前拿到(`last_right_click` 已由
            // main.rs 右键钳制填充,坐标不另传)。
            let tab_path = match &tab.kind {
                crate::preview::TabKind::File(p) => Some(p.clone()),
                _ => None,
            };
            let panel_kind = match kind {
                PreviewPaneKind::Project => crate::app::PanelKind::Project,
                _ => crate::app::PanelKind::Files,
            };
            let tab = panel_tab(PanelTabArgs {
                title: display_title,
                active,
                hover_t: title_hover_t,
                close_hover_t,
                prefix: None,
                suffix: None,
                on_select: select_msg(idx),
                // Blank 占位 tab 的 ×:有兄弟 tab 时是真关(用户腾位);
                // 单独存在时是 select(等价 no-op,数据层 `close` 关完会立刻
                // 自动补回一个 Blank,净效果为空,但点击 × 不会有"列表瞬空再补"
                // 的视觉跳动)。文件 tab 一律真关。
                on_close: if is_placeholder && preview.tabs().len() == 1 {
                    select_msg(idx)
                } else {
                    close_msg(idx)
                },
                show_tooltip: app.hover_tooltip_ready(item_hover(idx)),
                title_hover: move |h| Message::Hover(item_hover(idx), h),
                close_hover: move |h| Message::Hover(close_hover(idx), h),
            });
            // 拖拽换位:按住页签(选中处理已把 `app.tab_drag` 置位)后光标
            // 扫过哪个页签,这个 `on_move` 就按它发 `TabDragMove`,完成换位。
            let armed = app.dragging_group(tab_group);
            let mut area = MouseArea::new(tab).on_move(move |_| Message::TabDragMove {
                group: tab_group,
                index: idx,
            });
            if let Some(tab_path) = tab_path {
                area = area.on_right_press(Message::Files(
                    crate::extensions::files::Message::TabContextMenuOpen {
                        kind: panel_kind,
                        path: tab_path,
                    },
                ));
            }
            if armed {
                area = area.interaction(mouse::Interaction::Grabbing);
            }
            area.into()
        })
        .collect();
    // tab 列表进 clip 容器占 Fill,裁掉右侧溢出;V 按钮钉在裁剪区外、tab 组
    // 最左侧(只要 tab 组非空即显示,见 `tab_overflow_button`——无溢出也
    // 列出全部 tab 供跳转)。
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    // 预览/代码切换按钮:曾贴在每个 tab 自己身上(suffix),验收反馈挪到
    // tab 组最右侧(V 与"收起列表"之间)、只对**当前选中** tab 出一个——
    // 只有选中 tab 的内容看得见,切别的 tab 时按钮跟着换,不用每个 tab 各挂
    // 一份。仅对 `wry_toggle_eligible`(文本可编辑却默认走 wry/flyfish 渲染)
    // 的文件出现,随当前代码模式换图标(`editor.is_some()` = 代码模式 →
    // 显示 FilePlay,点它切回预览;否则显示 FileCode 切进代码)。
    let render_mode_button: Option<
        Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    > = preview.tabs().get(preview.active_idx()).and_then(|tab| {
        let eligible = match &tab.kind {
            crate::preview::TabKind::File(p) => crate::preview::wry_toggle_eligible(p),
            _ => false,
        };
        eligible.then(|| {
            tab_render_mode_button(
                tab.uses_rendered_source_editor(),
                app.hover_progress(render_mode_hover()),
                toggle_msg(preview.active_idx()),
                move |hovered| Message::Hover(render_mode_hover(), hovered),
            )
        })
    });
    // JSON/JSONL tab 的「树 / 原始文本」切换按钮:与上面 `.md`/`.html` 的
    // 「预览/代码」按钮同一处(只对当前选中 tab 出一个),仅当激活 tab 挂着
    // 已就绪的 `json_tree` 时出现。2026-09-22 从 `json_tree::view` 自己的
    // 头部行挪来——验收口径是两种双视图切换都长在 tab 栏上。图标随模式换:
    // 树视图显示 `FileCode`(点它看原始文本),原始文本显示 `ListTree`(点它
    // 回树)。原始文本态可能没有可复用的原生 `editor`(JSON 若读盘失败),
    // 此时仍给按钮——切回树视图是唯一有内容的出口。
    let json_tree_mode_button: Option<
        Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    > = preview.tabs().get(preview.active_idx()).and_then(|tab| {
        let crate::preview::JsonTreeState::Ready(view) = tab.json_tree.as_ref()? else {
            return None;
        };
        let tab_id = tab.id;
        let in_raw_text = view.view_mode == crate::json_tree::ViewMode::RawText;
        Some(tab_json_tree_mode_button(
            in_raw_text,
            app.hover_progress(json_tree_mode_hover()),
            Message::JsonTreeAction(
                find_panel(),
                tab_id,
                crate::json_tree::Action::ToggleViewMode,
            ),
            move |hovered| Message::Hover(json_tree_mode_hover(), hovered),
        ))
    });
    // CSV/TSV 的「网格 / 原文」切换:仅激活 tab 是 csv/tsv 的 Tabular 时出现。
    // 切到原文(feature 下)由 CodeMirror editor host 承载,网格仍原生。
    let tabular_mode_button: Option<
        Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    > = preview.tabs().get(preview.active_idx()).and_then(|tab| {
        let crate::preview::PreviewBackend::Tabular(tabular) = tab.backend.as_ref()? else {
            return None;
        };
        if tabular.format == crate::preview::TabularFormat::Workbook {
            return None;
        }
        let tab_id = tab.id;
        let in_text = tabular.mode == crate::preview::TabularMode::Text;
        Some(tab_tabular_mode_button(
            in_text,
            app.hover_progress(tabular_mode_hover()),
            Message::PreviewTabularTextModeToggle(find_panel(), tab_id),
            move |hovered| Message::Hover(tabular_mode_hover(), hovered),
        ))
    });
    let overflow_button = tab_overflow_button(
        preview.tabs().len(),
        app.hover_progress(overflow_hover()),
        overflow_toggle_msg(),
        move |hovered| Message::Hover(overflow_hover(), hovered),
    );
    // 预览右上角"收起/展开列表列"按钮:Files 预览收起文件树,Project 预览
    // 收起 info 列。按钮始终在此(内容侧),收起后仍可见以便恢复。图标按该
    // 面板当前所在栏(左/右)与收起态四选一,见 `IconKind::PanelLeftClose`
    // 等注释;工具文案由调用方给静态字符串。
    let collapse: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = match kind {
        PreviewPaneKind::Files => app.list_collapse_button(
            PanelKind::Files,
            app.files_tree_collapsed(),
            HoverId::FileTreeCollapse,
            "收起文件树",
            "展开文件树",
            Message::ToggleFileTreeCollapse,
            move |hovered| Message::Hover(HoverId::FileTreeCollapse, hovered),
        ),
        PreviewPaneKind::Project => app.list_collapse_button(
            PanelKind::Project,
            app.list_collapsed(PanelKind::Project),
            HoverId::ProjectListCollapse,
            "收起列表",
            "展开列表",
            Message::TogglePanelListCollapse(PanelKind::Project),
            move |hovered| Message::Hover(HoverId::ProjectListCollapse, hovered),
        ),
    };
    let mut tab_bar_row = row![]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some(btn) = overflow_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    tab_bar_row = tab_bar_row.push(clipped);
    if let Some(btn) = render_mode_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    if let Some(btn) = json_tree_mode_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    if let Some(btn) = tabular_mode_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    let tab_bar = tab_bar_row.push(collapse);

    let mut content = column![tab_bar, tab_divider()].spacing(region.gap);

    if let Some(err) = error {
        content = content.push(lh(text(format!("⚠ {err}"))
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().red)));
        // 失败时的外部打开 fallback:路径取自当前文件 tab(不隐式执行文件本身)。
        if let Some(tab) = preview.tabs().get(preview.active_idx())
            && matches!(tab.kind, crate::preview::TabKind::File(_))
        {
            let tab_id = tab.id;
            let panel = match kind {
                PreviewPaneKind::Files => PanelKind::Files,
                PreviewPaneKind::Project => PanelKind::Project,
            };
            content = content.push(
                iced_widget::button(text("在系统应用中打开").size(byteui::theme::font::body()))
                    .on_press(Message::PreviewOpenExternal(panel, tab_id)),
            );
        }
    }

    // `PreviewPane` 恒定携带第 0 项 `TabKind::Blank` 占位(见 `PreviewPane::
    // default`/`preview.rs::TabKind::Blank`),所以这里的列表正常不会空——保留
    // 这个空分支纯属防御,不再有独立的"暂无预览"文案(空态由 `Blank` 占位
    // 那个分支画 Dozer 品牌标呈现)。
    if preview.tabs().is_empty() {
        content = content.push(
            iced_widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fill),
        );
    } else {
        let active_tab = &preview.tabs()[preview.active_idx()];
        if let Some(tabular) = &active_tab.tabular
            && !active_tab.uses_editor_host()
        {
            // 表格 tab:iced 原生渲染 Tabular Viewer(虚拟化网格 + sheet 切换
            // 条),消息由 `grid::Action` 映射到 `Message::TabularAction`(带
            // `tab_id` + `PanelKind`,同 Find 条的手法)。首次打开的解析是
            // 后台线程跑的(大文件不能卡住 UI,见 `crate::tabular` 模块文档),
            // 没跑完时 `TabularState::Loading`,画统一 loading 动画占位。
            match tabular {
                crate::preview::TabularState::Ready(view) => {
                    let tab_id = active_tab.id;
                    let panel = find_panel();
                    content = content.push(
                        container(
                            view.view()
                                .map(move |act| Message::TabularAction(panel, tab_id, act)),
                        )
                        .width(Length::Fill)
                        .height(Length::Fill),
                    );
                }
                crate::preview::TabularState::Loading => {
                    // `loading_hint` 自带 `center_x/center_y(Fill)`,不需要
                    // 再包一层容器(同其余既有调用点)。
                    content = content.push(byteui::feedback::math_curve::loading_hint(
                        byteui::feedback::math_curve::Curve::RoseThree,
                        "正在打开表格…",
                        48.0,
                    ));
                }
            }
        } else if let Some(json_tree) = &active_tab.json_tree
            && !active_tab.uses_editor_host()
            && !active_tab.uses_json_editor()
        {
            // JSON/JSONL tab:双视图。Tree 模式下画树(消息映射到
            // `Message::JsonTreeAction`,带 `tab_id` + `PanelKind`);RawText
            // 模式下直接渲染该 tab 已有的原生代码编辑器(与上面 `editor` 分支
            // 逐字一致,复用同一份已加载状态,不重新解析文件)。「树 / 原始
            // 文本」切换按钮不在这里——2026-09-22 起画在 tab 栏上,与 `.md` 的
            // 「预览/代码」切换同处(见上方 `json_tree_mode_button`),所以
            // `json_tree::view()` 只剩树主体。首次加载是后台线程跑的,没跑完时
            // `JsonTreeState::Loading`,画统一 loading 占位。
            let tab_id = active_tab.id;
            let panel = find_panel();
            match json_tree {
                crate::preview::JsonTreeState::Ready(view) => {
                    if view.view_mode == crate::json_tree::ViewMode::Tree {
                        content = content.push(
                            container(
                                view.view()
                                    .map(move |act| Message::JsonTreeAction(panel, tab_id, act)),
                            )
                            .width(Length::Fill)
                            .height(Length::Fill),
                        );
                    }
                }
                crate::preview::JsonTreeState::Loading => {
                    content = content.push(byteui::feedback::math_curve::loading_hint(
                        byteui::feedback::math_curve::Curve::RoseThree,
                        "正在打开 JSON…",
                        48.0,
                    ));
                }
            }
        } else if active_tab.loading {
            // 原生编辑器候选正在后台线程异步读盘+构造(见 `App::
            // preview_open_path`/`PreviewPane::insert_loading_tab`)——这段
            // 时间不阻塞 UI 线程,画个居中 loading 动画占位(与表格/搜索/
            // 数据库等其它异步加载场景同一套 `loading_hint`),结果回来后
            // `apply_native_load` 会把这个分支换成上面 `editor` 那支。
            content = content.push(byteui::feedback::math_curve::loading_hint(
                byteui::feedback::math_curve::Curve::RoseThree,
                "正在打开文件…",
                48.0,
            ));
        } else if active_tab.kind == TabKind::Blank {
            // 空白占位 tab:居中放 Finder "Get Info" 风格的项目根简介卡
            // (folder icon + 名称头 + 位置/大小/创建/修改四行)。`blank_info`
            // 是后台异步跑的(见 `apply_pending_blank_info` 与
            // `Message::PreviewBlankInfoLoaded` handler),首帧会是 `None`,
            // 用 `—` 占位,数据回来再渲染真实值。无项目(`ws.project` 为
            // `None`)时只画头部,不画 stats——避免给空指针编出"位置: —"
            // 这种半成品。
            content = content.push(preview_blank_info_card(ws, preview));
        }
        // webview(flyfish)档 Find 条:渲染在内容区顶部,给已被 `preview_desired`
        // 下推的 webview 矩形让出固定高度的那一条。复用原生 Find 条(隐藏替换),
        // 高度钳到 `PREVIEW_FIND_BAR_HEIGHT` 与 webview 让位精确对齐——webview
        // 是原生子视图、不听 iced 绘制顺序,必须显式缩小它而不是指望层级遮挡。
        // 原生 editor 档走上面 `else if let Some(editor)` 分支里的 `true` 调用,
        // 二者互斥(find 会话要么锁原生、要么锁 webview),不会出现双条。
        if preview.find_state().is_some_and(|f| f.is_webview) {
            let bar = preview_find_bar_widget(app, preview, kind, false);
            content = content.push(
                container(bar)
                    .width(Length::Fill)
                    .height(Length::Fixed(crate::preview::PREVIEW_FIND_BAR_HEIGHT)),
            );
        }
        // 窗口化大文件整文件搜索条:窗口化 CodeMirror tab 的 ⌘F/⌘R 由 host JS
        // 拦截发 `find_request`,Rust 开这条 session,查询走
        // `large_text::stream_search`(整文件,不只搜持有窗口)。窗口化 host
        // 同样是原生 webview,条渲染时必须显式把它的矩形下推一个条高让位。
        if let Some(session) = preview.large_file_search_state() {
            let bar = preview_large_file_search_bar_widget(preview, kind, session.tab_id);
            content = content.push(
                container(bar)
                    .width(Length::Fill)
                    .height(Length::Fixed(crate::preview::PREVIEW_FIND_BAR_HEIGHT)),
            );
        }
    }

    let base: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(content.padding(region.padding))
            .width(width)
            .height(Length::Fill)
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: region.background.map(Into::into),
                border: outer,
                ..container::Style::default()
            })
            .into();
    base
}

/// 文件内 Find 条的共有渲染:`native editor` 档与 `webview(flyfish)` 档复用同一
/// 套查询框 / 「Aa」大小写开关 / n-m 计数 / 上下命中按钮,差别只在"搜索引擎"
/// 由谁执行(`PreviewPane::find_type`/`find_go` 对原生走 `CodeView` 现算、对
/// webview 挂 `pending_webview_exec` 给 `window_events` 注入 flyfish API)。
/// `show_replace` 控制是否渲染替换行与其展开圆盘箭头——原生 editor 档传
/// `true`,webview 档 flyfish 没有替换概念传 `false`(`open_find_on_active` 也已
/// 强制 `replace_open=false`,即使为 true 也不渲染)。`preview` 取只读引用读
/// `find_state()`/`find_query_focused()`,消息按 `kind` 路由到对应面板。
pub(crate) fn preview_find_bar_widget<'a>(
    app: &'a App,
    preview: &'a PreviewPane,
    kind: PreviewPaneKind,
    show_replace: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(find) = preview.find_state() else {
        return iced_widget::Space::new().into();
    };
    let panel = match kind {
        PreviewPaneKind::Files => PanelKind::Files,
        PreviewPaneKind::Project => PanelKind::Project,
    };
    let colors = byteui::theme::color::current();
    let find_font = tree_row_font_size();
    let case_toggle: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> = {
        let active = find.case_sensitive;
        button(text("Aa").size(byteui::theme::font::body()))
            .on_press(match panel {
                PanelKind::Project => Message::PreviewFindCase(PanelKind::Project, !active),
                _ => Message::PreviewFindCase(PanelKind::Files, !active),
            })
            .padding(2)
            .style(move |_t, _s| button::Style {
                background: None,
                text_color: if active { colors.cyan } else { colors.dim },
                ..button::Style::default()
            })
            .into()
    };
    let input = byteui::form::input_text::view_with_suffix_unframed_at_size(
        find_font,
        "搜索",
        &find.query,
        false,
        Some(crate::preview::find_field_id(panel)),
        !find.query.is_empty(),
        None,
        move |q: String| match panel {
            PanelKind::Project => Message::PreviewFindText(PanelKind::Project, q),
            _ => Message::PreviewFindText(PanelKind::Files, q),
        },
        case_toggle,
    );
    let count_label: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if find.count > 0 {
            text(format!("{}/{}", find.current + 1, find.count))
                .size(byteui::theme::font::body())
                .color(colors.dim)
                .into()
        } else if !find.query.is_empty() {
            text("0 个结果")
                .size(byteui::theme::font::body())
                .color(colors.red)
                .into()
        } else {
            container(iced_widget::Row::<
                Message,
                iced_widget::Theme,
                iced_renderer::Renderer,
            >::new())
            .into()
        };
    let hover = move |next: bool| match next {
        true => match panel {
            PanelKind::Project => HoverId::ProjectPreviewFindNext,
            _ => HoverId::PreviewFindNext,
        },
        false => match panel {
            PanelKind::Project => HoverId::ProjectPreviewFindPrev,
            _ => HoverId::PreviewFindPrev,
        },
    };
    let step_icon = move |next: bool| -> iced_widget::core::Element<
        'static,
        Message,
        iced_widget::Theme,
        iced_renderer::Renderer,
    > {
        let hid = hover(next);
        let btn = icons::icon_button_entry(
            if next {
                icons::IconKind::ArrowDown
            } else {
                icons::IconKind::ArrowUp
            },
            byteui::theme::icon_size::row(),
            false,
            false,
            app.hover_progress(hid),
            false,
            byteui::theme::geometry::tab_button_size(),
            true,
            match (next, panel) {
                (true, PanelKind::Project) => Message::PreviewFindGo(PanelKind::Project, true),
                (true, _) => Message::PreviewFindGo(PanelKind::Files, true),
                (false, PanelKind::Project) => Message::PreviewFindGo(PanelKind::Project, false),
                (false, _) => Message::PreviewFindGo(PanelKind::Files, false),
            },
            move |hovered| Message::Hover(hid, hovered),
            if next { "下一个" } else { "上一个" },
        );
        container(btn)
            .style(
                move |_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: None,
                    border: Border {
                        color: colors.border,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..iced_widget::container::Style::default()
                },
            )
            .into()
    };
    type EE<'x> =
        iced_widget::core::Element<'x, Message, iced_widget::Theme, iced_renderer::Renderer>;
    // 替换行展开圆盘箭头只在 show_replace(原生档)出现;webview 档 flyfish 无
    // 替换,直接给 `None`,查询行不再渲染那个箭头。
    let replace_toggle_opt: Option<EE<'_>> = if show_replace {
        let replace_open = find.replace_open;
        let replace_toggle_hid = match panel {
            PanelKind::Project => HoverId::ProjectPreviewFindReplaceToggle,
            _ => HoverId::PreviewFindReplaceToggle,
        };
        Some(icons::icon_button_entry(
            if replace_open {
                icons::IconKind::ChevronDown
            } else {
                icons::IconKind::ChevronRight
            },
            byteui::theme::icon_size::row(),
            false,
            false,
            app.hover_progress(replace_toggle_hid),
            false,
            byteui::theme::geometry::tab_button_size(),
            true,
            match panel {
                PanelKind::Project => Message::PreviewFindReplaceToggle(PanelKind::Project),
                _ => Message::PreviewFindReplaceToggle(PanelKind::Files),
            },
            move |hovered| Message::Hover(replace_toggle_hid, hovered),
            if replace_open {
                "收起替换"
            } else {
                "展开替换"
            },
        ))
    } else {
        None
    };
    const FIND_TRAILING_ZONE: f32 = 150.0;
    let query_trailing = container(
        row![count_label, step_icon(false), step_icon(true)]
            .spacing(4)
            .align_y(iced_widget::core::alignment::Alignment::Center),
    )
    .width(Length::Fixed(FIND_TRAILING_ZONE))
    .align_x(iced_widget::core::alignment::Horizontal::Right);
    let query_active = !find.query.is_empty() || preview.find_query_focused();
    let mut query_row_children: Vec<EE<'_>> = Vec::new();
    if let Some(rt) = replace_toggle_opt {
        query_row_children.push(rt);
    }
    query_row_children.push(find_field_shell(input, colors, 7.0, 10.0, query_active));
    query_row_children.push(query_trailing.into());
    let mut query_row_inner = iced_widget::Row::new()
        .spacing(4)
        .align_y(iced_widget::core::alignment::Alignment::Center);
    for c in query_row_children {
        query_row_inner = query_row_inner.push(c);
    }
    let query_row: EE<'_> = container(query_row_inner).width(Length::Fill).into();
    let mut find_rows: Vec<EE<'_>> = vec![query_row];
    // 替换行只在 show_replace(原生档)且展开时出现;webview 档恒不渲染。
    if show_replace && find.replace_open {
        let armed = find.count > 0;
        let replacement_input = byteui::form::input_text::view_at_size(
            find_font,
            "替换",
            &find.replacement,
            false,
            None,
            !find.replacement.is_empty(),
            None,
            true,
            move |s: String| match panel {
                PanelKind::Project => Message::PreviewFindReplacement(PanelKind::Project, s),
                _ => Message::PreviewFindReplacement(PanelKind::Files, s),
            },
        );
        let replace_icon_button = |kind: icons::IconKind,
                                   hid: HoverId,
                                   msg: Message,
                                   tooltip: &'static str,
                                   disabled: bool|
         -> iced_widget::core::Element<
            'static,
            Message,
            iced_widget::Theme,
            iced_renderer::Renderer,
        > {
            let btn = icons::icon_button_entry(
                kind,
                byteui::theme::icon_size::row(),
                false,
                disabled,
                app.hover_progress(hid),
                false,
                byteui::theme::geometry::tab_button_size(),
                !disabled,
                msg,
                move |hovered| Message::Hover(hid, hovered),
                tooltip,
            );
            container(btn)
                .style(
                    move |_t: &iced_widget::Theme| iced_widget::container::Style {
                        background: None,
                        border: Border {
                            color: colors.border,
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                        ..iced_widget::container::Style::default()
                    },
                )
                .into()
        };
        let replace_indent = (byteui::theme::geometry::tab_button_size() + 4.0 - 6.0).max(0.0);
        let replace_trailing = container(
            row![
                replace_icon_button(
                    icons::IconKind::Replace,
                    match panel {
                        PanelKind::Project => HoverId::ProjectPreviewFindReplaceCurrentBtn,
                        _ => HoverId::PreviewFindReplaceCurrentBtn,
                    },
                    match panel {
                        PanelKind::Project =>
                            Message::PreviewFindReplaceCurrent(PanelKind::Project),
                        _ => Message::PreviewFindReplaceCurrent(PanelKind::Files),
                    },
                    "替换当前",
                    !armed,
                ),
                replace_icon_button(
                    icons::IconKind::ReplaceAll,
                    match panel {
                        PanelKind::Project => HoverId::ProjectPreviewFindReplaceAllBtn,
                        _ => HoverId::PreviewFindReplaceAllBtn,
                    },
                    match panel {
                        PanelKind::Project => Message::PreviewFindReplaceAll(PanelKind::Project),
                        _ => Message::PreviewFindReplaceAll(PanelKind::Files),
                    },
                    "替换全部",
                    !armed,
                ),
            ]
            .spacing(6)
            .align_y(iced_widget::core::alignment::Alignment::Center),
        )
        .width(Length::Fixed(FIND_TRAILING_ZONE))
        .align_x(iced_widget::core::alignment::Horizontal::Right);
        let replace_row = container(
            row![
                iced_widget::Space::new().width(Length::Fixed(replace_indent)),
                find_field_shell(
                    replacement_input,
                    colors,
                    4.0,
                    10.0,
                    !find.replacement.is_empty()
                ),
                replace_trailing,
            ]
            .spacing(6)
            .align_y(iced_widget::core::alignment::Alignment::Center),
        )
        .width(Length::Fill)
        .into();
        find_rows.push(replace_row);
    }
    container(column(find_rows).spacing(4))
        .width(Length::Fill)
        .padding(8)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(colors.card.into()),
                border: iced_widget::core::Border {
                    color: colors.border,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}

/// 文件/项目预览 tab 栏"溢出下拉"浮层。**必须**在 `App::view` 顶层
/// `stack![base, ...]` 里拼(同 `terminal::term_tab_overflow_popup` 文档
/// 解释的理由——`anchor`/`window_size` 是全窗口坐标系,嵌在 `preview_pane_for`
/// 自己的局部布局里换算位置会跟真实点击位置对不上)。
/// 窗口化大文件整文件搜索条的渲染:查询框 + n/m 计数 + 上一条/下一条 +
/// 关闭。与 `preview_find_bar_widget` 不同,整文件流式扫描只做「找到并跳转」,
/// 没有大小写开关/替换行。`tab_id` 是 session 锁定的窗口化 CodeMirror tab。
pub(crate) fn preview_large_file_search_bar_widget<'a>(
    preview: &'a PreviewPane,
    kind: PreviewPaneKind,
    tab_id: usize,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(session) = preview.large_file_search_state() else {
        return iced_widget::Space::new().into();
    };
    let panel = match kind {
        PreviewPaneKind::Files => PanelKind::Files,
        PreviewPaneKind::Project => PanelKind::Project,
    };
    let colors = byteui::theme::color::current();
    let query_for_submit = session.query.clone();
    let count_label = text(format!(
        "{}/{}",
        if session.hits.is_empty() {
            0
        } else {
            session.current + 1
        },
        session.hits.len()
    ))
    .size(byteui::theme::font::body())
    .color(colors.dim);
    let input = byteui::form::input_text::view(
        "搜索文件内容…",
        &session.query,
        false,
        Some(crate::preview::large_file_search_field_id(panel)),
        !session.query.is_empty(),
        Some(Message::PreviewLargeFileSearchSubmit(
            panel,
            tab_id,
            query_for_submit,
        )),
        false,
        move |s: String| Message::PreviewLargeFileSearchSubmit(panel, tab_id, s),
    );
    let row_el = row![
        input,
        count_label,
        button(text("↑")).on_press(Message::PreviewLargeFileSearchGo(panel, false)),
        button(text("↓")).on_press(Message::PreviewLargeFileSearchGo(panel, true)),
        button(text("×")).on_press(Message::PreviewLargeFileSearchClose(panel)),
    ]
    .spacing(6)
    .align_y(iced_widget::core::alignment::Alignment::Center);
    container(row_el)
        .width(Length::Fill)
        .padding(8)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(colors.card.into()),
                border: iced_widget::core::Border {
                    color: colors.border,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}

pub(crate) fn preview_tab_overflow_popup<'a>(
    app: &'a App,
    ws: &'a Workspace,
    kind: PreviewPaneKind,
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let (preview, anchor) = match kind {
        PreviewPaneKind::Files => (&ws.preview, ws.preview_tab_overflow_anchor),
        PreviewPaneKind::Project => (&ws.project_preview, ws.project_preview_tab_overflow_anchor),
    };
    let anchor = anchor?;
    if preview.tabs().is_empty() {
        return None;
    }
    let select_msg = move |idx| match kind {
        PreviewPaneKind::Files => Message::PreviewSelectTab(idx),
        PreviewPaneKind::Project => Message::ProjectPreviewSelectTab(idx),
    };
    let close_msg = move |idx| match kind {
        PreviewPaneKind::Files => Message::PreviewCloseTab(idx),
        PreviewPaneKind::Project => Message::ProjectPreviewCloseTab(idx),
    };
    let overflow_dismiss_msg = match kind {
        PreviewPaneKind::Files => Message::PreviewTabOverflowDismiss,
        PreviewPaneKind::Project => Message::ProjectPreviewTabOverflowDismiss,
    };
    // 下拉列出精选组内**全部** tab(不管当前是否横向可见),便于随时
    // 跳转到某一项,而非只列"被挤出可见区"的子集。
    let entries: Vec<TabOverflowEntry<'_, Message>> = preview
        .tabs()
        .iter()
        .enumerate()
        .map(|(idx, tab)| TabOverflowEntry {
            index: idx,
            prefix: None,
            title: tab.title.clone(),
            active: idx == preview.active_idx(),
            // index 0 的 `Blank` 占位固定存在、不可关闭(同 SSH/数据库面板的
            // 固定"空白"占位),下拉里这一行不画 ×。
            closable: !matches!(tab.kind, crate::preview::TabKind::Blank),
            hover_t: app.hover_progress(HoverId::TabOverflowRow(idx)),
        })
        .collect();
    Some(tab_overflow_menu(TabOverflowMenuArgs {
        entries,
        anchor,
        window_size: app.window_size,
        on_select: select_msg,
        on_close: close_msg,
        on_dismiss: overflow_dismiss_msg,
        on_row_hover: move |idx, hovered| Message::Hover(HoverId::TabOverflowRow(idx), hovered),
    }))
}

/// 对话副行文案：`<agent> · <相对时间> · <规模>`（P1j）。
/// 相对时间文案：刚刚/N 分钟前/N 小时前/N 天前（D5，从 `conversation_sub`
/// 抽出为独立纯函数）。H0 项目卡"活跃时间"、文件卡、对话卡三处复用，
/// 不要三份重复 switch。
pub(crate) fn relative_time_text(modified_ms: u64, now_ms: u64) -> String {
    let ago = now_ms.saturating_sub(modified_ms) / 1000; // 秒
    if ago < 60 {
        "刚刚".to_string()
    } else if ago < 3600 {
        format!("{} 分钟前", ago / 60)
    } else if ago < 86400 {
        format!("{} 小时前", ago / 3600)
    } else {
        format!("{} 天前", ago / 86400)
    }
}

/// 把一段(可能是绝对路径、可能带空格/单引号)安全地包进 `sh -c` 的单引号。
/// 供 Aider launcher 命令使用——app bundle 路径 `/Applications/Dozer AI
/// Coder.app/...` 含空格,不转义会断词。
pub(crate) fn sh_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// agent 选择菜单选中的 agent → 要自动键入 PTY 的 CLI 命令。`Unknown`
/// 不该从选择菜单产生(选项是各家 agent + 纯 Shell/Git Shell,纯 Shell 走
/// `launch: None`,不经过这个函数),但函数保持穷尽 match,防止未来枚举
/// 新增变体时静默漏写。**不能**一律取 `AgentKind::label()`:
/// - Goose 的交互命令需要子命令(`goose session`,见 spec D1)。
/// - Aider 走 `dozer-hook launch aider`(launcher bridge,见 spec D1)——命令
///   里必须带 sibling `dozer-hook` 的绝对路径,所以本函数返回 `Option<String>`
///   而不是静态 `&str`,`hook_exe` 由调用方(有 `current_exe()` 的上下文)传入。
pub(crate) fn agent_launch_command(agent: AgentKind, hook_exe: &str) -> Option<String> {
    match agent {
        AgentKind::Unknown => None,
        AgentKind::Claude => Some("claude".into()),
        AgentKind::Codebuddy => Some("codebuddy".into()),
        AgentKind::Opencode => Some("opencode".into()),
        AgentKind::Codex => Some("codex".into()),
        AgentKind::Goose => Some("goose session".into()),
        AgentKind::Aider => Some(format!("{} launch aider", sh_single_quote(hook_exe))),
        AgentKind::V8agent => Some("v8agent".into()),
    }
}
