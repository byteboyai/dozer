//! Workspace 的 view 层:load_more_button、agent 列表与卡片、工作区内容行、
//! agent picker、review 内容 pane、preview pane 等渲染函数。

use crate::app::{App, HoverId, Message, PanelKind};
use crate::chrome::homespace::home_panel_head_with_actions;
use crate::extensions::conversations;
use crate::menu_spec::{MenuSpec, MenuSpecItem};
use crate::preview::ViewerPanel;
use crate::theme;
use crate::theme::terminal_font;
use crate::ui_text::text;
use byteui::interaction::icons;
use byteui::interaction::icons::IconKind;
use dozer_core::protocol::AgentKind;
use dozer_panel_preview::ViewCtx;
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Border, Element, Length, Padding};
use iced_widget::{Scrollable, button, column, container, row, scrollable};

use super::*;

/// 会话详情"加载更多"按钮(2026-08-27):居中的 `<summary>` 样式的图标按钮,
/// hover 金边提示,点击追加下一页回合。`after_turn_index` 用 `entries.len()`
/// 当锚点——`ReviewEntry` 不携带原始 turn index,详情页按整段摊平加载,取
/// 条数近似下一步起点即可(跳过首尾折叠带来的少量误差在可接受范围)。
pub(crate) fn load_more_button<'a>(
    conversation_id: &'a str,
    after_turn_index: i64,
    locale: &str,
) -> iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let btn = iced_widget::button(
        text(crate::i18n::host_text(
            "conversations.load_more",
            "Load more…",
            locale,
        ))
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
/// → Codex → Goose → Aider → V8agent → Qodercn → Unknown(与 `conversation_agents_present`
/// 同一份顺序),只返回非空分组(没有该 agent 的会话就不出现,面板不留空
/// 分组占位)。组内保持 `tabs` 原有顺序(tab 打开顺序)。返回下标而非
/// 引用——渲染时既要下标发 `Message::SelectTab(idx)`,又要用下标回查
/// `ws.tabs[idx]` 取展示字段,直接存下标比存 `&SessionTab` 省一次生命
/// 周期纠缠。
pub(crate) fn group_tabs_by_agent(tabs: &[SessionTab]) -> Vec<(AgentKind, Vec<usize>)> {
    const ORDER: [AgentKind; 9] = [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::Codex,
        AgentKind::Goose,
        AgentKind::Aider,
        AgentKind::V8agent,
        AgentKind::Qodercn,
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
    let locale = app.locale.active_locale();
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
        content = content.push(lh(text(crate::i18n::host_text(
            "agent.no_conversations",
            "No conversations",
            locale,
        ))
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
                content = content.push(agent_card(ws, idx, locale));
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
    locale: &str,
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
            text(agent_state_label(tab.agent_state, locale))
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
        // 工作内容可能是无空格长串(如粘贴的临时文件路径),默认按词换行会外溢。
        .wrapping(iced_widget::core::text::Wrapping::WordOrGlyph)
        .into()
}

/// Agent 面板头部"＋"按钮:点击切换 `agent_picker_open`,弹出 agent
/// 选择菜单(`agent_picker_popup`)。样式与顶栏页签行的"＋"一致——无背景、
/// Lucide `SquarePlus` 图标、静止灰(`DIM`)、hover 平滑过渡到金(`GOLD`),
/// 由 `HoverId::named(PanelKind::Agent, "picker_toggle")` + `MouseArea` 驱动同一套悬停动画。
pub(crate) fn agent_picker_toggle_button<'a>(
    app: &App,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    icons::icon_button_entry(
        icons::IconKind::SquarePlus,
        byteui::theme::icon_size::row(),
        false,
        false,
        app.hover_progress(HoverId::named(PanelKind::Agent, "picker_toggle")),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::AgentPickerToggle,
        |hovered| Message::Hover(HoverId::named(PanelKind::Agent, "picker_toggle"), hovered),
        crate::i18n::host_text(
            "agent.new_session",
            "New agent session",
            app.locale.active_locale(),
        ),
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

/// 分支切换原生菜单条目(文件树 git 底栏 / git log 面板两处版本选择共用)
/// ——样式对齐 tab 组下拉菜单(`tab_overflow_items`):当前分支 CREAM 文字,
/// 标题前置固定图标列里的 `>`(`ChevronRight`,CREAM),其余行 BODY 且不画
/// 箭头;工作区有未提交改动(`dirty`)时除当前分支外全部 DIM 置灰禁用
/// (dirty 下切分支会被 git 拒绝,提前锁定),当前分支追加 "(Uncommitted)"
/// 提示——文案沿用 `files.rs::branch_picker_popup` 的既有口径。仅 macOS 编译。
#[cfg(target_os = "macos")]
pub(crate) fn branch_picker_items<Msg: Clone>(
    current: Option<&str>,
    branches: &[String],
    dirty: bool,
    on_switch: impl Fn(String) -> Msg,
) -> Vec<crate::chrome::native_menu::Item<Msg>> {
    let colors = byteui::theme::color::current();
    branches
        .iter()
        .map(|name| {
            let is_current = Some(name.as_str()) == current;
            let mut label = name.clone();
            if is_current && dirty {
                label.push_str("(Uncommitted)");
            }
            let (color, icon, icon_color) = if is_current {
                (
                    colors.cream,
                    Some(IconKind::ChevronRight),
                    Some(colors.cream),
                )
            } else if dirty {
                // 置灰行必须显式给 DIM——`MenuItemView` 只按调用方给的颜色
                // 画字,`enabled:false` 只负责不接 hover/点击,不会自动变灰。
                (colors.dim, None, None)
            } else {
                (colors.body, None, None)
            };
            crate::chrome::native_menu::Item::Entry {
                icon,
                icon_color,
                label,
                color,
                enabled: is_current || !dirty,
                msg: on_switch(name.clone()),
            }
        })
        .collect()
}

/// Agent 选择器菜单内容——native(`agent_picker_items`)和 iced fallback
/// (`agent_picker_popup`)共用同一份数据，只在这里组装一次。
pub(crate) fn agent_picker_spec() -> MenuSpec<Message> {
    // 由 `AgentKind::known()` 派生:新增 agent 自动出现在菜单里;按展示名不分大小写排序。
    let mut agents: Vec<(&'static str, PickerLaunch)> = AgentKind::known()
        .map(|k| (k.display_label(), PickerLaunch::Agent(Some(k))))
        .collect();
    agents.sort_by_key(|(label, _)| label.to_lowercase());
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
/// `byteui::feedback::dialog::confirm` 骨架(同文件树删除/主机删除确认框)。
/// `pending_close_tab` 存的是被点 × 那个会话的稳定 id(不是下标)——弹窗
/// 迁到独立原生窗口后主窗口仍可交互,tab 列表在弹窗存活期间可能被重排/
/// 增删,按 id 现查才能保证标题始终对应真正会被关掉的那个会话;查不到
/// (会话已经通过别的路径关掉)兜底成"会话"。
/// 关 Agent tab 确认框的内容描述——宿主(`platform::confirm_overlay`)取这一
/// 份渲染。`None` 表示当前没有待确认的关闭。
pub(crate) fn agent_close_confirm_spec(
    ws: &Workspace,
    locale: &str,
) -> Option<byteui::feedback::dialog::ConfirmDialog<Message>> {
    let id = ws.pending_close_tab.as_ref()?;
    let agent_label = ws
        .tabs
        .iter()
        .find(|t| &t.info.id == id)
        .map(|t| t.agent.label().to_string())
        .unwrap_or_else(|| "agent".to_string());
    Some(byteui::feedback::dialog::ConfirmDialog {
        icon: Some(IconKind::CircleAlert),
        title: crate::i18n::host_text_args(
            "agent.close_tab_title",
            "Close the \"{agent}\" Agent?",
            locale,
            &[("agent", agent_label.as_str())],
        ),
        description: crate::i18n::host_text_args(
            "agent.close_tab_description",
            "The \"{agent}\" Agent is currently active. Closing it ends the session and clears its context.",
            locale,
            &[("agent", agent_label.as_str())],
        ),
        cancel_label: crate::i18n::host_text("common.cancel", "Cancel", locale),
        cancel_msg: Message::TermTabCloseCancel,
        confirm_label: crate::i18n::host_text("common.close", "Close", locale),
        confirm_msg: Message::TermTabCloseConfirm,
        close_msg: Some(Message::TermTabCloseCancel),
        confirm_color: byteui::theme::color::current().red,
        content_spacing: 8.0,
    })
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
pub(crate) fn review_webview_spec(
    review: Option<&ReviewView>,
    locale: &str,
) -> Vec<crate::preview::WebviewSpec> {
    let Some(rv) = review else {
        return Vec::new();
    };
    if rv.error.is_some() || rv.entries.is_empty() {
        return Vec::new();
    }
    let loaded = rv.loaded_nonce == Some(rv.nonce);
    vec![crate::preview::WebviewSpec {
        id: 0,
        url: format!(
            "dozer://review-trace/host.html?_r={}&locale={}",
            rv.nonce, locale
        ),
        visible: loaded,
        editor_binding: None,
        reserve: None,
        loading_generation: None,
        surface: None,
        // review-trace 是同款「未就绪先创建」的 Rendered 宿主(Preact 渲染),
        // 未就绪时离屏停放,避免 hidden 下 rAF 被挂起拖慢/卡住首帧。
        park_offscreen: !loaded,
        display_i18n: None,
    }]
}

/// `review_content_pane` 该渲染哪一态(空/loading/内容三态)。抽成纯函数
/// 供 headless 单测——视图函数本身只管把结果画出来。
#[derive(Debug, PartialEq)]
pub(crate) enum ReviewPaneState {
    Empty,
    Loading,
    Ready,
}

pub(crate) fn review_pane_state(rv: Option<&ReviewView>) -> ReviewPaneState {
    let Some(rv) = rv else {
        return ReviewPaneState::Empty;
    };
    // error / 空 entries 两种情况下 `review_webview_spec` 会销毁 webview
    // (永远等不到 `document_loaded`),`loaded_nonce` 永远是 None——必须归入
    // Ready 让 `review_content` 显示"⚠ 错误" / "暂无对话"文案,否则会永久
    // 卡在"加载中…"占位。这跟"根本没有数据"、"正在加载"是两回事。
    if rv.error.is_some() || rv.entries.is_empty() || rv.loaded_nonce == Some(rv.nonce) {
        ReviewPaneState::Ready
    } else {
        ReviewPaneState::Loading
    }
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
    let locale = app.locale.active_locale();
    // 内容侧标题栏:左侧名字信息面板(会话/agent 态) + 右侧"生成总结"按钮
    // (会话详情手动生成/重试,spec 2026-09-26 第 8 节)+ "收起/展开列表列"
    // 按钮(与 Todo/Database/SSH/Agent 内容侧统一)。
    let collapse_button = app.list_collapse_button(
        PanelKind::Conversations,
        app.list_collapsed(PanelKind::Conversations),
        HoverId::list_collapse(PanelKind::Conversations),
        crate::i18n::host_text("panel.collapse_list", "Collapse list", locale),
        crate::i18n::host_text("panel.expand_list", "Expand list", locale),
        Message::TogglePanelListCollapse(PanelKind::Conversations),
        move |h| Message::Hover(HoverId::list_collapse(PanelKind::Conversations), h),
    );
    let actions: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match ws.review.as_ref().map(|rv| (&rv.source, rv.agent)) {
            Some((ReviewSource::Conversation(cid), agent)) => {
                let generating = ws.conversations.summary_generating(cid);
                let generate = button(
                    text(if generating {
                        crate::i18n::host_text("conversations.generating", "Generating…", locale)
                    } else {
                        crate::i18n::host_text(
                            "conversations.generate_summary",
                            "Generate Summary",
                            locale,
                        )
                    })
                    .size(byteui::theme::font::label())
                    .color(byteui::theme::color::current().cream),
                )
                .padding([4, 8]);
                let generate = if generating {
                    generate
                } else {
                    generate.on_press(Message::Conversations(
                        conversations::Message::SummaryGenerate(cid.clone(), agent),
                    ))
                };
                row![generate, collapse_button].spacing(6).into()
            }
            _ => collapse_button,
        };
    let header = container(
        row![home_panel_head_with_actions(
            IconKind::BotMessageSquare,
            crate::i18n::host_text("conversations.pane_title", "Conversations", locale),
            Some(actions),
        )]
        .width(Length::Fill),
    )
    .padding(theme::region::project_pane().padding);
    let mut content = column![header].spacing(region.gap);

    if let Some(error) = ws.review.as_ref().and_then(|rv| match &rv.source {
        ReviewSource::Conversation(cid) => ws.conversations.summary_error(cid),
        ReviewSource::Session(_) => None,
    }) {
        content = content.push(
            text(crate::i18n::host_render(error, locale))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().red),
        );
    }

    match review_pane_state(ws.review.as_ref()) {
        ReviewPaneState::Empty => {
            content = content.push(
                container(lh(text(crate::i18n::host_text(
                    "conversations.no_review_content",
                    "No review content—click a conversation in the list on the left to start reviewing",
                    locale,
                ))
                    .size(byteui::theme::font::subtitle())
                    .color(byteui::theme::color::current().dim)))
                .width(Length::Fill)
                .height(Length::Fill),
            );
        }
        ReviewPaneState::Loading => {
            // 与 Empty 同结构(独占内容区、不渲染 Scrollable),只是文案换成
            // "加载中",让 webview 隐藏期间这块区域有意义的提示而不是空的可
            // 滚动区域(同用量面板"统计中…"分支的写法,视觉基调统一)。
            content = content.push(
                container(
                    iced_widget::column![
                        byteui::feedback::math_curve::view_animated::<Message>(
                            byteui::feedback::math_curve::Curve::RoseThree,
                            64.0,
                        ),
                        iced_widget::text(crate::i18n::host_text(
                            "common.loading",
                            "Loading…",
                            locale,
                        ))
                        .size(byteui::theme::font::body())
                        .color(byteui::theme::color::current().dim),
                    ]
                    .spacing(12)
                    .align_x(iced_widget::core::alignment::Horizontal::Center),
                )
                .center_x(Length::Fill)
                .center_y(Length::Fill),
            );
        }
        ReviewPaneState::Ready => {
            let body = review_content(column![].spacing(region.gap), ws, locale);
            content = content.push(
                Scrollable::new(body)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .direction(scrollable::Direction::Vertical(
                        byteui::interaction::scrollbar::scrollbar(),
                    ))
                    .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
            );
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
    locale: &str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::project_pane();
    let mut header = column![].spacing(region.gap).width(Length::Fill);
    let tree_col = column![].spacing(region.gap);
    header = header.push(
        text(crate::i18n::host_text(
            "files.no_project",
            "No project open",
            locale,
        ))
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

/// Project 面板右配对的预览 pane,复用与 Files 预览同一套渲染。独立调一个
/// 新 `PreviewPaneKind`,让项目链接打开的文件进 `ws.project_preview` 而不是
/// 冲进 Files 预览(见 Task 12 `OpenLink` 路由)。
#[derive(Debug, Clone, Copy)]
pub(crate) enum PreviewPaneKind {
    Files,
    Project,
}

/// 当前项目根目录的 basename(`ws.project.path` 的最后一段路径分量)。生产代码
/// 走 [`project_root_dir_name_of`](纯函数,在 `preview_models`);这个按 `Workspace`
/// 取的薄封装只给既有测试用,保持它们原样。
#[cfg(test)]
pub(crate) fn project_root_dir_name(ws: &Workspace) -> Option<String> {
    dozer_panel_preview::view_model::project_root_dir_name_of(ws.project.as_ref())
}

/// 预览 tab 的渲染标题(测试用薄封装;生产代码走库的 `view_model::preview_tab_title`)。
#[cfg(test)]
pub(crate) fn preview_tab_display_title(
    ws: &Workspace,
    tab: &crate::preview::PreviewTab,
    locale: &str,
) -> String {
    dozer_panel_preview::view_model::preview_tab_title(ws.project.as_ref(), tab, locale)
}

/// Files 预览 pane:状态取自 `ws.preview`,渲染全部在 `dozer-panel-preview`。
pub(crate) fn preview_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    dozer_panel_preview::view::pane(
        &ws.preview,
        ws.preview_tab_first,
        ws.preview_error.as_ref(),
        ViewerPanel::Files,
        app,
        &ViewCtx {
            project: ws.project.as_ref(),
        },
        width,
        outer,
    )
}

/// Project 面板右配对的预览 pane(独立的 `ws.project_preview`)。
pub(crate) fn project_preview_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    dozer_panel_preview::view::pane(
        &ws.project_preview,
        ws.project_preview_tab_first,
        ws.project_preview_error.as_ref(),
        ViewerPanel::Project,
        app,
        &ViewCtx {
            project: ws.project.as_ref(),
        },
        width,
        outer,
    )
}

/// 文件/项目预览 tab 栏「溢出下拉」浮层。**必须**在 `App::view` 顶层 `stack![base, ...]` 里拼
/// (锚点 / 窗口尺寸是全窗口坐标系)。
pub(crate) fn preview_tab_overflow_popup<'a>(
    app: &'a App,
    ws: &'a Workspace,
    kind: PreviewPaneKind,
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let (panel, pane, anchor) = match kind {
        PreviewPaneKind::Files => (
            ViewerPanel::Files,
            &ws.preview,
            ws.preview_tab_overflow_anchor,
        ),
        PreviewPaneKind::Project => (
            ViewerPanel::Project,
            &ws.project_preview,
            ws.project_preview_tab_overflow_anchor,
        ),
    };
    dozer_panel_preview::view::overflow_menu(app, panel, pane, anchor)
}

/// 对话副行文案：`<agent> · <相对时间> · <规模>`（P1j）。
/// 相对时间文案：刚刚/N 分钟前/N 小时前/N 天前（D5，从 `conversation_sub`
/// 抽出为独立纯函数）。H0 项目卡"活跃时间"、文件卡、对话卡三处复用，
/// 不要三份重复 switch。
pub(crate) fn relative_time_text(modified_ms: u64, now_ms: u64, locale: &str) -> String {
    let ago = now_ms.saturating_sub(modified_ms) / 1000; // 秒
    if ago < 60 {
        crate::i18n::host_text("time.just_now", "Just now", locale)
    } else if ago < 3600 {
        crate::i18n::host_text_args(
            "time.minutes_ago",
            "{n} minutes ago",
            locale,
            &[("n", &(ago / 60).to_string())],
        )
    } else if ago < 86400 {
        crate::i18n::host_text_args(
            "time.hours_ago",
            "{n} hours ago",
            locale,
            &[("n", &(ago / 3600).to_string())],
        )
    } else {
        crate::i18n::host_text_args(
            "time.days_ago",
            "{n} days ago",
            locale,
            &[("n", &(ago / 86400).to_string())],
        )
    }
}

pub(crate) use dozer_panel_agent::launch::{agent_launch_command, sh_single_quote};

#[cfg(test)]
mod tests {
    use super::*;

    mod review_pane_state_tests {
        use super::*;
        use crate::transcript::ReviewEntry;
        use dozer_core::protocol::AgentKind;

        fn rv(nonce: u64, loaded_nonce: Option<u64>) -> ReviewView {
            ReviewView {
                source: ReviewSource::Conversation("c1".into()),
                entries: vec![ReviewEntry::Human { text: "hi".into() }],
                error: None,
                agent: AgentKind::Claude,
                nonce,
                summary_title: None,
                summary_text: None,
                summary_time: None,
                loaded_nonce,
            }
        }

        #[test]
        fn none_is_empty() {
            assert_eq!(review_pane_state(None), ReviewPaneState::Empty);
        }

        #[test]
        fn unloaded_nonce_is_loading() {
            assert_eq!(
                review_pane_state(Some(&rv(3, None))),
                ReviewPaneState::Loading
            );
            assert_eq!(
                review_pane_state(Some(&rv(3, Some(2)))),
                ReviewPaneState::Loading
            );
        }

        #[test]
        fn matching_nonce_is_ready() {
            assert_eq!(
                review_pane_state(Some(&rv(3, Some(3)))),
                ReviewPaneState::Ready
            );
        }

        /// 首次加载失败:`review_webview_spec` 会销毁 webview(永远等不到
        /// `document_loaded`),`loaded_nonce` 停在 None——必须走 Ready 让
        /// `review_content` 显示"⚠ 错误"文案,不能永久卡在"加载中…"。
        #[test]
        fn error_is_ready_even_without_loaded_nonce() {
            let mut r = rv(3, None);
            r.error = Some("transcript 解析失败".into());
            assert_eq!(review_pane_state(Some(&r)), ReviewPaneState::Ready);
        }

        /// 首次加载成功但会话为空:同样销毁 webview、无事件回传,必须走
        /// Ready 显示"暂无对话",不能卡在"加载中…"。
        #[test]
        fn empty_entries_is_ready_even_without_loaded_nonce() {
            let mut r = rv(3, None);
            r.entries = Vec::new();
            assert_eq!(review_pane_state(Some(&r)), ReviewPaneState::Ready);
        }
    }
}
