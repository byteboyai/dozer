//! Workspace 的 view 层:load_more_button、agent 列表与卡片、工作区内容行、
//! agent picker、review 内容 pane、preview pane 等渲染函数。

use crate::app::{App, Message};
use crate::menu_spec::{MenuSpec, MenuSpecItem};
use crate::preview::ViewerPanel;
use crate::theme;
use crate::theme::terminal_font;
use crate::ui_text::text;
use byteui::interaction::icons::IconKind;
use dozer_panel_preview::ViewCtx;
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Border, Element, Length, Padding};
use iced_widget::{Scrollable, button, column, container, scrollable};

use super::*;

pub(crate) use dozer_panel_agent::card::CardCtx;

/// 会话详情"加载更多"按钮(2026-08-27):居中的 `<summary>` 样式的图标按钮,
/// hover 金边提示,点击追加下一页回合。`after_turn_index` 用 `entries.len()`
/// 当锚点——`ReviewEntry` 不携带原始 turn index,详情页按整段摊平加载,取
/// 条数近似下一步起点即可(跳过首尾折叠带来的少量误差在可接受范围)。
/// 对话面板 session 详情的首屏回合页大小(2026-08-27):列表分页
/// (`extensions::conversations::CONVERSATION_PAGE_SIZE`,20)与详情页分页
/// (这里,200)含义不同,不要混用。

/// Agent 列表面板(右面板区"Agent"视图的列表侧):按 `AgentKind` 分组展示
/// 当前项目的会话,组内保留 tab 打开顺序;点击一行 = `Message::SelectTab`
/// 切焦点(同终端 tab 栏点击效果)。
pub(crate) fn agent_list_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let project_id = ws.project.as_ref().map(|p| p.id).unwrap_or_default();
    let display = crate::host_adapters::agent_panel::DozerDisplay::new(app, project_id);
    let ctx = CardCtx {
        titles: ws,
        project_branch: ws.project_panel.branch(),
        project_dirty: ws.project_panel.dirty(),
    };
    dozer_panel_agent::view::list::agent_list(&ws.agent, &display, &ctx, width, outer)
}

/// "当前工作内容"(有就显示,没有就省略)+ 工作区(`@分支名` + 脏标,所有
/// agent 都显示)合并一行、不换行,work_content 排在工作区前面(卡片
/// 改版要求)。工作区不再用"工作区:"文字标签,前缀改成 `@`——跟卡片其余
/// 行的极简风格对齐。分支名规则不变:无分支(非 git 项目)显示 `—`;有
/// 未提交改动时分支名后缀 `(Uncommitted)`——跟 `extensions/files.rs`
/// 里分支切换菜单当前分支带脏标时的既有文案(`n.push_str("(Uncommitted)")`,
/// 见该文件约第 1409 行)保持同一措辞,不新造一套脏标文案。
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
    dozer_panel_agent::picker::picker_entries()
        .into_iter()
        .map(|entry| match entry {
            dozer_panel_agent::picker::PickerEntry::Separator => MenuSpecItem::separator(),
            dozer_panel_agent::picker::PickerEntry::Launch {
                launch,
                label,
                icon,
                icon_color,
            } => MenuSpecItem::entry_tinted(
                icon,
                icon_color,
                label,
                Message::AgentPickerSelect(launch),
            ),
        })
        .collect()
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
    let project_id = ws.project.as_ref().map(|p| p.id).unwrap_or_default();
    dozer_panel_agent::picker::close_confirm_spec(&ws.agent, locale, move |m| match m {
        dozer_panel_agent::update::Message::CloseConfirm => Message::TermTabCloseConfirm,
        dozer_panel_agent::update::Message::CloseCancel => Message::TermTabCloseCancel,
        other => Message::Agent(project_id, other),
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
    dozer_panel_conversations::snapshot::review_webview_spec(review, locale, "dozer")
        .into_iter()
        .map(|spec| crate::preview::WebviewSpec {
            id: 0,
            url: spec.url,
            visible: spec.visible,
            park_offscreen: spec.park_offscreen,
            editor_binding: None,
            reserve: None,
            loading_generation: None,
            surface: None,
            display_i18n: None,
        })
        .collect()
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
    dozer_panel_conversations::view::review_pane(
        &ws.conversations,
        app,
        dozer_panel_conversations::adapter::ConversationsCapabilities::all(),
        width,
        outer,
    )
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
