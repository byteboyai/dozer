//! Workspace 的 view 层:load_more_button、agent 列表与卡片、工作区内容行、
//! agent picker、review 内容 pane、preview pane 等渲染函数。

use crate::app::{App, HoverId, Message, PanelKind, tab_divider};
use crate::chrome::homespace::home_panel_head_with_actions;
use crate::chrome::tab_widget::{
    PanelTabArgs, TabOverflowEntry, TabOverflowMenuArgs, panel_tab, tab_overflow_button,
    tab_overflow_menu, tab_render_mode_button, tab_window,
};
use crate::extensions::conversations;
use crate::menu_spec::{MenuSpec, MenuSpecItem};
use crate::preview::TabKind;
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
/// → Codex → Kilo → V8agent → Unknown(与 `conversation_agents_present`
/// 同一份顺序),只返回非空分组(没有该 agent 的会话就不出现,面板不留空
/// 分组占位)。组内保持 `tabs` 原有顺序(tab 打开顺序)。返回下标而非
/// 引用——渲染时既要下标发 `Message::SelectTab(idx)`,又要用下标回查
/// `ws.tabs[idx]` 取展示字段,直接存下标比存 `&SessionTab` 省一次生命
/// 周期纠缠。
pub(crate) fn group_tabs_by_agent(tabs: &[SessionTab]) -> Vec<(AgentKind, Vec<usize>)> {
    const ORDER: [AgentKind; 7] = [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::Codex,
        AgentKind::Kilo,
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
                    lh(text(format!("{}（{}）", agent.label(), idxs.len()))
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

/// `agent_picker_popup` 的原生菜单版本,纯数据组装——八个选项与旧版完全
/// 一致(六 agent + 分隔线 + Git Shell/纯 Shell),agent 图标用各自专属色
/// (`agent_dot_color`)、文字用 BODY。仅 macOS 编译,非 mac 平台继续走
/// `agent_picker_popup` 的 iced 弹层。
#[cfg(target_os = "macos")]
pub(crate) fn agent_picker_items() -> Vec<crate::chrome::native_menu::Item<Message>> {
    crate::menu_spec::to_native(agent_picker_spec())
}

/// Agent 选择器菜单内容——native(`agent_picker_items`)和 iced fallback
/// (`agent_picker_popup`)共用同一份数据，只在这里组装一次。
pub(crate) fn agent_picker_spec() -> MenuSpec<Message> {
    let agents: [(&str, PickerLaunch); 6] = [
        ("Claude", PickerLaunch::Agent(Some(AgentKind::Claude))),
        ("CodeBuddy", PickerLaunch::Agent(Some(AgentKind::Codebuddy))),
        ("Codex", PickerLaunch::Agent(Some(AgentKind::Codex))),
        ("Kilo", PickerLaunch::Agent(Some(AgentKind::Kilo))),
        ("OpenCode", PickerLaunch::Agent(Some(AgentKind::Opencode))),
        ("v8agent", PickerLaunch::Agent(Some(AgentKind::V8agent))),
    ];
    let shells: [(&str, PickerLaunch); 2] = [
        ("Git Shell", PickerLaunch::Git),
        ("纯 Shell", PickerLaunch::Agent(None)),
    ];
    let mk = |label: &str, launch: PickerLaunch| {
        let (icon, icon_color) = match launch {
            PickerLaunch::Agent(Some(kind)) => (agent_icon(kind), agent_dot_color(kind)),
            PickerLaunch::Agent(None) => (IconKind::Terminal, byteui::theme::color::current().body),
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
/// 就在最靠右的 Agent 面板头部,近似等于窗口右上角),八个选项按标签
/// 首字母顺序排列:Claude/CodeBuddy/Codex/Git Shell/Kilo/OpenCode/
/// v8agent/纯 Shell(验收反馈,2026-08-21;此前是手写的固定顺序,不便
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
    // 常规 agent 按标签首字母排序。"Git Shell" / "纯 Shell" 归到菜单最底部,
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

/// 预览 pane 的共同渲染(Files 预览与 Project 面板右配对复用同一套 tab
/// 栏/原生编辑器/占位文案逻辑,只是状态取自 `ws.preview` 还是
/// `ws.project_preview`、消息与前缀路由到哪套)不同。
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
    let editor_msg = move |tab_id, ev| match kind {
        PreviewPaneKind::Files => Message::PreviewEditorEvent(tab_id, ev),
        PreviewPaneKind::Project => Message::ProjectPreviewEditorEvent(tab_id, ev),
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
        .map(|t| preview_tab_display_width(&t.title))
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
            let display_title = if tab.editor.is_some() && tab.dirty {
                format!("{} *", tab.title)
            } else {
                tab.title.clone()
            };
            // index 0 的 `Blank` 占位 tab 不可关闭:它上面的 × 点击等同于
            // "选中空白页"(不真关),与 SSH/数据库面板 tab 条最前面那个固定
            // "空白"占位(`app.rs::ssh_tab_bar` 的 `on_close: SelectBlankTab`)
            // 完全同一套做法——数据层 `PreviewPane::close(0)` 另有兜底 no-op。
            let is_placeholder = matches!(tab.kind, crate::preview::TabKind::Blank);
            let tab = panel_tab(PanelTabArgs {
                title: display_title,
                active,
                hover_t: title_hover_t,
                close_hover_t,
                prefix: None,
                suffix: None,
                on_select: select_msg(idx),
                on_close: if is_placeholder {
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
                tab.editor.is_some(),
                app.hover_progress(render_mode_hover()),
                toggle_msg(preview.active_idx()),
                move |hovered| Message::Hover(render_mode_hover(), hovered),
            )
        })
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
    let tab_bar = tab_bar_row.push(collapse);

    let mut content = column![tab_bar, tab_divider()].spacing(region.gap);

    if let Some(err) = error {
        content = content.push(lh(text(format!("⚠ {err}"))
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().red)));
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
        if let Some(editor) = &active_tab.editor {
            // 原生 tab:激活 tab 有原生 editor 时,直接在 iced 里渲染它(语法
            // 高亮/行号/ByteBoy2077 配色),put 下 content。`editor` 为 `None`
            // 的 wry 路由 tab 不 push 任何 iced 元素——那片区域由 main.rs 定位
            // 的 wry webview 子视图负责渲染,现状不变。
            let tab_id = active_tab.id;
            // 文件内 Find 条:锁着当前激活原生 tab 的会话存在时,在 tab_bar 分隔线
            // 之下、编辑器之上渲染输入框(框内右侧内嵌 Aa 大小写开关)+ n/m 计数 +
            // ↑ 上一个 / ↓ 下一命中。
            // 切走文件时 `PreviewPane` 已 cull 掉失配会话,条随之一并消失——既然
            // open/lifecycle 保证 `find` 总锁着激活原生 tab、此处又只在激活 tab 是
            // 原生时进入,读数即可,不必再校 tab 归属。
            if let Some(find) = preview.find_state() {
                let panel = find_panel();
                let colors = byteui::theme::color::current();
                // Find 条紧贴右侧编辑器,查询框/替换框正文用与编辑器相同的代码
                // 字号(`tree_row_font_size` 与 code_editor 同公式),让用户敲的
                // 词跟被找的文件正文看齐(需求:文件内查找条字号 = text editor)。
                let find_font = tree_row_font_size();
                // 「Aa」大小写开关:无独立 SVG 的字形钮(同被删的 Find × 按钮,但
                // 有真状态)。开(逐字严格)文字青 `cyan`、关(ASCII 折叠)灰 `dim`。
                // 青是 ByteBoy2077 甲方金之外的"用户动作强调色",toggle 归用户操作,
                // 不用甲方专属 gold,遵循 CLAUDE.md 裁决。
                // 内嵌在输入框同一圈边框内靠右(`input_text::view_with_suffix`,
                // 结构参照 search_box 的"共框尾控件"既有做法),不再占条上独立槽位。
                let case_toggle: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> = {
                    let active = find.case_sensitive;
                    button(text("Aa").size(byteui::theme::font::body()))
                        .on_press(match panel {
                            PanelKind::Project => {
                                Message::PreviewFindCase(PanelKind::Project, !active)
                            }
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
                // 边框高亮同 search_box 约定由调用方给:查询词非空即金框。
                // 用 unframed 透明版 text_input(框/底由外层 `find_field_shell`
                // 统一垫 editor 背景 + 描边),让 Aa 大小写钮共享同一圈内边距。
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
                // 命中计数:n 1-based;查无命中(非空 query)标红;新开 empty query
                // 不显示计数——这条进列就 pad 占用让条高稳定,避免每次刷字数跳动。
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
                            (true, PanelKind::Project) => {
                                Message::PreviewFindGo(PanelKind::Project, true)
                            }
                            (true, _) => Message::PreviewFindGo(PanelKind::Files, true),
                            (false, PanelKind::Project) => {
                                Message::PreviewFindGo(PanelKind::Project, false)
                            }
                            (false, _) => Message::PreviewFindGo(PanelKind::Files, false),
                        },
                        move |hovered| Message::Hover(hid, hovered),
                        if next { "下一个" } else { "上一个" },
                    );
                    // 同替换行图标按钮:外面补一圈固定可见的圆角边框(`icon_
                    // button_entry` 只在 `active` 态描边,这两个按钮没有持久
                    // 选中态)。
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
                // 文件内搜索的查询框与替换框各自独立、成两个带 1px 圆角边框的
                // 输入框(需求:v0.5.94 之后改回,不再合成一整块):框内底色与
                // 右侧代码编辑器同一 `colors.bg`,敲词文字底色跟被找正文看板一致
                // (比亮一点的 card 更"嵌进"编辑区);有内容时整框描金、否则普通
                // 边色。命中计数与上下箭头、替换按钮都摆在框外右侧,不占框内。
                type EE<'x> = iced_widget::core::Element<
                    'x,
                    Message,
                    iced_widget::Theme,
                    iced_renderer::Renderer,
                >;
                // 查询框前的展开/收起替换行圆盘箭头:收起态 `ChevronRight`、
                // 展开态 `ChevronDown`(同文件树/Todo 分类树展开箭头的既有语义)。
                // ⌘F 打开条时收起、⌘R 打开时展开(见 `PreviewFindOpen`/
                // `PreviewFindOpenWithReplace`),这里手动点按翻转。
                let replace_open = find.replace_open;
                let replace_toggle_hid = match panel {
                    PanelKind::Project => HoverId::ProjectPreviewFindReplaceToggle,
                    _ => HoverId::PreviewFindReplaceToggle,
                };
                let replace_toggle: EE<'_> = icons::icon_button_entry(
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
                );
                // 查询框与替换框要"长度一样、右边缘对齐"(参照 VSCode 查找条):
                // 两行各自的框后附件(计数+上下箭头 vs 替换按钮×2)天然宽度不
                // 等,若各自吃 `Length::Fill` 剩余空间,两个框会不等宽。这里给
                // 两行的"框后附件"统一钳到同一个固定宽度(取较宽的替换按钮组
                // 富余出来),框本身仍吃 `Length::Fill`——总行宽相同、附件区宽度
                // 相同,余下的 `Fill` 自然等宽,顺带右边缘也对齐。
                // 「替换当前」/「替换全部」改图标按钮后trailing 区收窄回来——
                // 决定宽度的现在是查询行那边"命中计数 + 上下箭头"这一组,不再
                // 是文字按钮组。
                const FIND_TRAILING_ZONE: f32 = 150.0;
                let query_trailing = container(
                    row![count_label, step_icon(false), step_icon(true)]
                        .spacing(4)
                        .align_y(iced_widget::core::alignment::Alignment::Center),
                )
                .width(Length::Fixed(FIND_TRAILING_ZONE))
                .align_x(iced_widget::core::alignment::Horizontal::Right);
                let mut find_rows: Vec<EE<'_>> = vec![];
                // 边框描金:查询词非空 **或** 输入框持有真实焦点——2026-09-11
                // 需求补上聚焦态,不再只靠已有内容触发(此前空 query 时点进框里
                // 光标闪烁却没有任何视觉反馈)。
                let query_active = !find.query.is_empty() || preview.find_query_focused();
                let query_row: EE<'_> = container(
                    row![
                        replace_toggle,
                        find_field_shell(input, colors, 7.0, 10.0, query_active),
                        query_trailing,
                    ]
                    .spacing(4)
                    .align_y(iced_widget::core::alignment::Alignment::Center),
                )
                .width(Length::Fill)
                .into();
                find_rows.push(query_row);

                // ---- 文件内替换行(条身之下第二行) ----
                // 替换只改锁定 buffer 并标脏、落盘仍等 ⌘S(`PreviewPane::replace_*`
                // 的语义),不做直接磁盘写。默认跳过空命中(避免误把用户缓冲区清空
                // 成替换框逗号残片)。“替换当前”会顺带到下一命中、方便一路处理,
                // “替换全部”把这一轮全部落一次。两个按钮共用一轮是否可替换的开关。
                let armed = find.count > 0;
                // 替换框同查询框独立栅格:bare=true 去底去框透明,边框/底色交给
                // 下方 `editor_field` 统一垫(editor 背景色)。
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
                        PanelKind::Project => {
                            Message::PreviewFindReplacement(PanelKind::Project, s)
                        }
                        _ => Message::PreviewFindReplacement(PanelKind::Files, s),
                    },
                );
                // 「替换当前」/「替换全部」改图标按钮(Lucide replace /
                // replace-all,2026-09-07 需求),不再是文字按钮——无命中时
                // `dim` 置灰 + `interactive=false` 不可点,同 `armed` 语义。
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
                    // `icon_button_entry` 本身只在 `active` 态描边(这两个按钮
                    // 没有持久选中态,永远描不出来)——外面再包一圈固定可见的
                    // 圆角边框,让它们看起来像独立按钮而不是裸图标。
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
                // 替换框要跟上面查询框左对齐:查询框前多了圆盘箭头
                // (`tab_button_size()` 宽 + `query_row` 的 `spacing(4)`),这里
                // 用等宽占位补上——占位宽度扣掉本行自己的 `spacing(6)`,两行加
                // 起来对 `find_field_shell` 左边缘落在同一个 x。
                let replace_indent =
                    (byteui::theme::geometry::tab_button_size() + 4.0 - 6.0).max(0.0);
                let replace_trailing = container(
                    row![
                        replace_icon_button(
                            icons::IconKind::Replace,
                            match panel {
                                PanelKind::Project => HoverId::ProjectPreviewFindReplaceCurrentBtn,
                                _ => HoverId::PreviewFindReplaceCurrentBtn,
                            },
                            match panel {
                                PanelKind::Project => {
                                    Message::PreviewFindReplaceCurrent(PanelKind::Project)
                                }
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
                                PanelKind::Project => {
                                    Message::PreviewFindReplaceAll(PanelKind::Project)
                                }
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
                if replace_open {
                    find_rows.push(replace_row);
                }
                let find_rows = container(column(find_rows).spacing(4))
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
                    );
                content = content.push(find_rows);
            }
            content = content.push(
                container(editor.view().map(move |ev| editor_msg(tab_id, ev)))
                    .width(Length::Fill)
                    .height(Length::Fill),
            );
        } else if let Some(tabular) = &active_tab.tabular {
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
        } else if active_tab.kind == TabKind::Blank {
            // 关到最后一个 tab 后自动补的空白占位:没有 wry 页面,内容区
            // 纯 iced 原生渲染,居中放 Dozer 品牌标(`IconKind::Dozer`,此前
            // 一直没有调用点,见该枚举成员的注释)。
            content = content.push(
                container(icons::view(
                    icons::IconKind::Dozer,
                    96.0,
                    byteui::theme::color::current().dim,
                ))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced_widget::core::alignment::Horizontal::Center)
                .align_y(iced_widget::core::alignment::Vertical::Center),
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

/// 文件/项目预览 tab 栏"溢出下拉"浮层。**必须**在 `App::view` 顶层
/// `stack![base, ...]` 里拼(同 `terminal::term_tab_overflow_popup` 文档
/// 解释的理由——`anchor`/`window_size` 是全窗口坐标系,嵌在 `preview_pane_for`
/// 自己的局部布局里换算位置会跟真实点击位置对不上)。
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

/// agent 选择菜单选中的 agent → 要自动键入 PTY 的 CLI 命令名。`Unknown`
/// 不该从选择菜单产生(选项只有 Claude/CodeBuddy/OpenCode/纯 Shell 四选
/// 一,纯 Shell 走 `launch: None`,不经过这个函数),但函数保持穷尽
/// match,防止未来枚举新增变体时静默漏写。已知变体的 CLI 名字与
/// `AgentKind::label()` 逐字节一致(`label()` 本身就是给这三个变体返回
/// 小写 CLI 名),这里直接复用而不重复一份映射表,避免两处拼写分叉。
pub(crate) fn agent_cli_command(agent: AgentKind) -> Option<&'static str> {
    match agent {
        AgentKind::Unknown => None,
        known => Some(known.label()),
    }
}
