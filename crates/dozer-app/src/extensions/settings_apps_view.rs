//! 设置「应用」页的 iced 视图:只画 [`InstallFlowState`] 与 `bytehost_panel::install` 里的
//! 纯展示函数,不含任何状态迁移逻辑(状态机在 `bytehost_panel::install`)。

use bytehost_apps::plan::InstallPlan;
use bytehost_apps::proto::{AppSummary, RuntimeInstallPlan, RuntimeProbe};
use bytehost_apps::registry::UninstallMode;
use bytehost_apps::state::ObservedState;
use bytehost_panel::install::{
    Flow, InstallFlowState as State, Load, Message, PickTarget, SourceChoice,
    managed_runtime_label, managed_runtime_of, observed_label, plan_lines, plan_view,
    probe_is_installing, progress_text, rollback_note_label, runtime_line,
};
use bytehost_panel::logs::LogsView;

use iced_widget::core::{Alignment, Element, Length};
use iced_widget::{Space, button, column, container, row, scrollable, text};

type El<'a> = Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>;

fn action_button<'a>(
    label: impl text::IntoFragment<'a>,
    msg: Message,
    color: iced_widget::core::Color,
) -> El<'a> {
    button(text(label).size(byteui::theme::font::body()))
        .padding([4, 12])
        .on_press(msg)
        .style(byteui::feedback::dialog::action_button_style(color))
        .into()
}

fn heading<'a>(label: &'static str) -> El<'a> {
    text(label)
        .size(byteui::theme::font::subtitle())
        .color(byteui::theme::color::current().cream)
        .into()
}

fn dim<'a>(
    s: impl text::IntoFragment<'a>,
) -> iced_widget::Text<'a, iced_widget::Theme, iced_renderer::Renderer> {
    text(s)
        .size(byteui::theme::font::label())
        .color(byteui::theme::color::current().dim)
}

pub fn view(state: &State) -> El<'_> {
    let mut body = column![heading("应用")].spacing(12).width(Length::Fill);

    // 运行时探测 + 受管运行时的安装/卸载。
    body = body.push(dim("运行时(受管版本随 dozerd 安装在本机,优先于系统版本)"));
    let probes_view: El<'_> = match &state.probes {
        Load::Loading => dim("检测中…").into(),
        Load::Failed(why) => dim(format!("探测失败:{why}")).into(),
        Load::Loaded(probes) => {
            let mut col = column![].spacing(4);
            for probe in probes {
                col = col.push(runtime_row_view(state, probe));
            }
            col.into()
        }
    };
    body = body.push(probes_view);

    // 已安装应用。
    body = body.push(heading("已安装"));
    let apps_view: El<'_> = match &state.apps {
        Load::Loading => dim("读取中…").into(),
        Load::Failed(why) => dim(format!("读取失败:{why}")).into(),
        Load::Loaded(apps) if apps.is_empty() => dim("还没有安装任何应用").into(),
        Load::Loaded(apps) => {
            let mut col = column![].spacing(6);
            for app in apps {
                col = col.push(app_row(state, app));
                // 展开着日志的应用:在该行下方画查看器。
                if state.is_log_open(app.id.as_str()) {
                    col = col.push(log_viewer(state.logs_view(app.id.as_str()), &app.id));
                }
            }
            col.into()
        }
    };
    body = body.push(apps_view);

    // 安装/卸载流程。
    body = body.push(flow_view(state));

    scrollable(body)
        .direction(scrollable::Direction::Vertical(
            byteui::interaction::scrollbar::scrollbar(),
        ))
        .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style())
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// 一行受管运行时:名称、状态、受管版本、进度或安装/卸载按钮。
fn runtime_row_view<'a>(state: &'a State, probe: &'a RuntimeProbe) -> El<'a> {
    let colors = byteui::theme::color::current();
    let (name, status, _) = runtime_line(probe);
    let mut r = row![
        text(name)
            .size(byteui::theme::font::body())
            .color(colors.cream),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    if probe_is_installing(probe) {
        let progress = probe.job.as_ref().map(progress_text).unwrap_or_default();
        r = r.push(dim(progress));
        r = r.push(Space::new().width(Length::Fill));
        return r.into();
    }

    r = r.push(dim(status));
    r = r.push(Space::new().width(Length::Fill));

    let runtime = managed_runtime_of(&probe.runtime);
    if !probe.managed.is_empty() {
        for v in &probe.managed {
            let Some(rt) = runtime else {
                r = r.push(dim(v.clone()));
                continue;
            };
            if state.runtime_uninstalling(rt, v) {
                r = r.push(dim(format!("{v}(处理中…)")));
                continue;
            }
            r = r.push(dim(v.clone()));
            r = r.push(action_button(
                "卸载",
                Message::RuntimeUninstallClicked(rt, v.clone()),
                colors.red,
            ));
        }
    } else if probe.installable
        && let Some(rt) = runtime
    {
        r = r.push(action_button(
            "安装…",
            Message::RuntimeInstallClicked(rt),
            colors.gold,
        ));
    }
    r.into()
}

fn app_row<'a>(state: &'a State, app: &'a AppSummary) -> El<'a> {
    let colors = byteui::theme::color::current();
    let id = app.id.as_str().to_owned();
    let mut r = row![
        text(app.name.as_str())
            .size(byteui::theme::font::body())
            .color(colors.cream),
        dim(format!(
            "版本 {} · {}",
            app.version,
            observed_label(&app.observed)
        )),
        Space::new().width(Length::Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    match state.is_acting(&id) {
        Some(_) => r = r.push(dim("处理中…")),
        None => {
            if matches!(
                app.observed,
                ObservedState::Running | ObservedState::Starting
            ) {
                r = r.push(action_button(
                    "停止",
                    Message::StopClicked(id.clone()),
                    colors.dim,
                ));
            }
            // 有上一版才给回滚入口(A6g)。
            if let Some(to) = app.previous_version {
                r = r.push(action_button(
                    format!("回滚到 {}", to),
                    Message::RollbackClicked(id.clone()),
                    colors.dim,
                ));
            }
            // 运行中与失败都能看日志;展开着时按钮变「收起日志」。
            let (log_label, log_msg) = if state.is_log_open(&id) {
                ("收起日志", Message::HideLogs)
            } else {
                ("日志", Message::ShowLogs(id.clone()))
            };
            r = r.push(action_button(log_label, log_msg, colors.dim));
            if !app.observed.is_transient() {
                r = r.push(action_button(
                    "卸载",
                    Message::UninstallClicked(id.clone()),
                    colors.red,
                ));
            }
        }
    }

    let first: El<'a> = r.into();
    let mut col = column![first].spacing(6).width(Length::Fill);
    // 该应用的回滚确认开着:行内二次确认(不用 Toast)。
    if let Flow::ConfirmRollback {
        id: pending, to, ..
    } = &state.flow
        && pending == &id
    {
        col = col.push(
            column![
                text(format!(
                    "回滚到 {to}:应用数据不会一起回滚,旧版本可能无法读取新版本写过的数据。确定回滚?"
                ))
                .size(byteui::theme::font::body())
                .color(colors.cream),
                row![
                    action_button("取消", Message::RollbackCancelled, colors.dim),
                    action_button(
                        "确定回滚",
                        Message::RollbackConfirmed(id.clone()),
                        colors.gold,
                    ),
                ]
                .spacing(10),
            ]
            .spacing(6)
            .width(Length::Fill),
        );
    }
    // 回滚过(自动/手动)的持久说明,留在行下方。
    if let Some(note) = &app.rollback_note {
        col = col.push(dim(rollback_note_label(note)));
    }
    col.into()
}

/// 一个应用的日志查看器(系统默认字体、可滚动;`truncated` 顶部提示;刷新失败标一行)。
/// 滚动区带稳定 `Id`,贴底跟随时由 `App` 每帧钉到底部;`on_scroll` 回报是否贴底。
fn log_viewer(view: LogsView, app_id: &bytehost_apps::id::AppId) -> El<'static> {
    let colors = byteui::theme::color::current();
    let mut col = column![].spacing(4).width(Length::Fill);
    match view {
        LogsView::Hidden => {}
        LogsView::Loading => col = col.push(dim("读取日志…")),
        LogsView::Failed(reason) => {
            col = col.push(
                text(reason)
                    .size(byteui::theme::font::body())
                    .color(colors.red),
            )
        }
        LogsView::Loaded {
            text: log_text,
            truncated,
            stale,
        } => {
            if truncated {
                col = col.push(dim("仅显示末尾若干行"));
            }
            if stale {
                col = col.push(dim("刷新失败,显示的是上一次的内容"));
            }
            let scrolled_id = app_id.to_string();
            let lines = iced_widget::text(log_text)
                .size(byteui::theme::font::body())
                .shaping(iced_widget::core::text::Shaping::Advanced)
                .width(Length::Fill);
            col = col.push(
                container(
                    scrollable(lines)
                        .id(crate::extensions::app_logs::scroll_id(app_id.as_str()))
                        .direction(scrollable::Direction::Vertical(
                            byteui::interaction::scrollbar::scrollbar(),
                        ))
                        .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style())
                        .width(Length::Fill)
                        .on_scroll(move |viewport| {
                            Message::LogsScrolled(scrolled_id.clone(), at_bottom(&viewport))
                        }),
                )
                .height(Length::Fixed(200.0))
                .width(Length::Fill)
                .padding(6),
            );
        }
    }
    container(col).width(Length::Fill).padding(6).into()
}

/// 视口是否贴底(留 4px 容差)。
fn at_bottom(viewport: &scrollable::Viewport) -> bool {
    let abs = viewport.absolute_offset();
    let bottom = abs.y + viewport.bounds().height;
    bottom >= viewport.content_bounds().height - 4.0
}

/// 来源单选行:三颗互斥按钮;当前选中那颗用金色。
fn source_choice_row(current: SourceChoice) -> El<'static> {
    let colors = byteui::theme::color::current();
    let mk = |label: &'static str, choice: SourceChoice| -> El<'static> {
        let selected = current == choice;
        action_button(
            label,
            Message::SourceChoiceChanged(choice),
            if selected { colors.gold } else { colors.dim },
        )
    };
    row![
        mk("本机目录", SourceChoice::Dir),
        mk("本机压缩包", SourceChoice::Archive),
        mk("网络 URL", SourceChoice::Url),
    ]
    .spacing(8)
    .into()
}

/// 空闲时的安装表单:来源切换 + (视来源)各色输入框与主按钮。
fn install_form(state: &State) -> El<'_> {
    let colors = byteui::theme::color::current();
    let mut col = column![source_choice_row(state.source_choice)]
        .spacing(8)
        .width(Length::Fill);
    match state.source_choice {
        SourceChoice::Dir => {
            col = col.push(action_button(
                "安装应用…",
                Message::InstallClicked,
                colors.gold,
            ));
        }
        SourceChoice::Archive => {
            col = col.push(action_button(
                "安装应用…",
                Message::PickArchive,
                colors.gold,
            ));
        }
        SourceChoice::Url => {
            col = col.push(dim("https 压缩包地址(只允许静态应用)"));
            // 输入框沿用 `byteui::form::input_text::view`(真 `text_input`,同分类树改名框)。
            col = col.push(byteui::form::input_text::view(
                "https://example.com/app.zip",
                state.url_input.as_str(),
                false,
                None,
                false,
                None,
                false,
                Message::UrlChanged,
            ));
            col = col.push(dim(
                "期望 sha256(可选;填了就钉死,不填则审批卡展示实际算出的哈希)",
            ));
            col = col.push(byteui::form::input_text::view(
                "留空 = 不钉死",
                state.sha_input.as_str(),
                false,
                None,
                false,
                None,
                false,
                Message::Sha256Changed,
            ));
            if let Some(err) = &state.url_error {
                col = col.push(
                    text(err.as_str())
                        .size(byteui::theme::font::label())
                        .color(colors.red),
                );
            }
            col = col.push(action_button(
                "获取计划",
                Message::UrlPlanClicked,
                colors.gold,
            ));
        }
    }
    container(col).width(Length::Fill).into()
}

fn flow_view(state: &State) -> El<'_> {
    let colors = byteui::theme::color::current();
    let flow = &state.flow;
    match flow {
        Flow::Idle => install_form(state),
        Flow::Picking { target } => match target {
            PickTarget::Dir => {
                dim("请在弹出的对话框里选择应用目录(目录里要有 manifest.toml)…").into()
            }
            PickTarget::Archive => {
                dim("请在弹出的对话框里选择压缩包(.zip / .tar.gz / .tgz)…").into()
            }
        },
        Flow::Planning { .. } => dim("正在检查应用…").into(),
        Flow::Installing { .. } => dim("正在安装…").into(),
        Flow::Failed { message } => column![
            text(message.as_str())
                .size(byteui::theme::font::body())
                .color(colors.red),
            action_button("知道了", Message::FlowDismissed, colors.dim),
        ]
        .spacing(8)
        .into(),
        Flow::ConfirmUninstall { name, .. } => column![
            text(format!("卸载应用「{name}」?"))
                .size(byteui::theme::font::body())
                .color(colors.cream),
            dim("「保留数据」只删程序,重装后数据还在;「连数据一起删」不可恢复。"),
            row![
                action_button("取消", Message::FlowDismissed, colors.dim),
                action_button(
                    "保留数据",
                    Message::UninstallConfirmed(UninstallMode::Program),
                    colors.gold
                ),
                action_button(
                    "连数据一起删",
                    Message::UninstallConfirmed(UninstallMode::ProgramAndData),
                    colors.red
                ),
            ]
            .spacing(8),
        ]
        .spacing(8)
        .into(),
        // 回滚确认画在对应应用行下方(行内),这里不出内容。
        Flow::ConfirmRollback { .. } => Space::new().height(Length::Fixed(0.0)).into(),
        Flow::Reviewing { plan, .. } => {
            let running = state.observed_running(plan.app_id.as_str());
            review_view(plan, running)
        }
        Flow::RuntimePlanning { runtime } => dim(format!(
            "正在准备 {} 运行时的安装计划…",
            managed_runtime_label(*runtime)
        ))
        .into(),
        Flow::RuntimeReviewing { plan } => runtime_review_view(plan),
        Flow::ConfirmRuntimeUninstall { runtime, version } => column![
            text(format!(
                "卸载受管运行时 {} {version}?",
                managed_runtime_label(*runtime)
            ))
            .size(byteui::theme::font::body())
            .color(colors.cream),
            dim("依赖它的应用下次启动会因运行时缺失而失败;重装可恢复。"),
            row![
                action_button("取消", Message::FlowDismissed, colors.dim),
                action_button("卸载", Message::RuntimeUninstallConfirmed, colors.red),
            ]
            .spacing(8),
        ]
        .spacing(8)
        .into(),
    }
}

/// 运行时安装审批卡:逐行展示 `plan_lines`(来源 URL、固定版本、完整 SHA-256、目标目录)。
fn runtime_review_view(plan: &RuntimeInstallPlan) -> El<'_> {
    let colors = byteui::theme::color::current();
    let mut col = column![
        text(format!(
            "安装 {} 运行时",
            managed_runtime_label(plan.runtime)
        ))
        .size(byteui::theme::font::body())
        .color(colors.cream),
    ]
    .spacing(6);
    for (k, v) in plan_lines(plan) {
        col = col.push(
            row![
                dim(k),
                text(v)
                    .size(byteui::theme::font::label())
                    .color(colors.cream)
            ]
            .spacing(10),
        );
    }
    col = col.push(dim("将要做的事:"));
    for line in &plan.will_do {
        col = col.push(
            text(line.clone())
                .size(byteui::theme::font::label())
                .color(colors.cream),
        );
    }
    col = col.push(
        row![
            action_button("取消", Message::FlowDismissed, colors.dim),
            action_button("下载并安装", Message::RuntimeApproveClicked, colors.gold),
        ]
        .spacing(8),
    );
    container(col).padding(10).width(Length::Fill).into()
}

/// 审批卡:把 `plan_view` 的每一行如实画出来;升级(申请比已授予更多)的权限用金色标 `↑`。
fn review_view(plan: &InstallPlan, running: bool) -> El<'_> {
    let colors = byteui::theme::color::current();
    let view = plan_view(plan, running);
    let mut col = column![
        text(format!("安装 {}", view.title))
            .size(byteui::theme::font::body())
            .color(colors.cream),
    ]
    .spacing(6);
    for notice in view.notices.clone() {
        col = col.push(
            text(notice)
                .size(byteui::theme::font::label())
                .color(colors.gold),
        );
    }
    for (k, v) in view.facts {
        col = col.push(
            row![
                dim(k),
                text(v)
                    .size(byteui::theme::font::label())
                    .color(colors.cream)
            ]
            .spacing(10),
        );
    }
    // 来源披露(A6h):事实行原样画;来自网络时先给醒目的"不可信"标注。
    col = col.push(dim("来源:"));
    if view.source.from_network {
        col = col.push(
            text("来自网络,不可信")
                .size(byteui::theme::font::label())
                .color(colors.gold),
        );
        col = col.push(dim("只允许静态应用,运行在严格 CSP 下"));
    }
    for (k, v) in view.source.rows {
        col = col.push(
            row![
                dim(k),
                text(v)
                    .size(byteui::theme::font::label())
                    .color(colors.cream)
            ]
            .spacing(10),
        );
    }
    for warning in view.source.warnings {
        col = col.push(
            text(warning)
                .size(byteui::theme::font::label())
                .color(colors.gold),
        );
    }
    col = col.push(dim("安装与运行时会执行:"));
    for line in view.will_run {
        col = col.push(
            text(line)
                .size(byteui::theme::font::label())
                .color(colors.cream),
        );
    }
    col = col.push(dim("申请的权限:"));
    if view.permissions.is_empty() {
        col = col.push(
            text("不申请任何权限")
                .size(byteui::theme::font::label())
                .color(colors.cream),
        );
    }
    for p in view.permissions {
        col = col.push(
            row![
                text(if p.escalation { "↑" } else { " " })
                    .size(byteui::theme::font::label())
                    .color(colors.gold),
                text(p.label)
                    .size(byteui::theme::font::label())
                    .color(colors.cream),
                text(p.change)
                    .size(byteui::theme::font::label())
                    .color(colors.cream),
                Space::new().width(Length::Fill),
                dim(p.enforcement),
            ]
            .spacing(10),
        );
    }
    col = col.push(
        row![
            action_button("取消", Message::FlowDismissed, colors.dim),
            action_button("批准并安装", Message::ApproveClicked, colors.gold),
        ]
        .spacing(8),
    );
    container(col).padding(10).width(Length::Fill).into()
}
