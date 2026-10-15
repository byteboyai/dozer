//! 设置「应用」页的 iced 视图:只画 [`InstallFlowState`] 与 `bytehost_panel::install` 里的
//! 纯展示函数,不含任何状态迁移逻辑(状态机在 `bytehost_panel::install`)。

use bytehost_apps::plan::InstallPlan;
use bytehost_apps::proto::{AppSummary, DisplayTitleScope, RuntimeInstallPlan, RuntimeProbe};
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

fn heading<'a>(label: &str) -> El<'a> {
    text(label.to_string())
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

pub fn view<'a>(state: &'a State, locale: &str) -> El<'a> {
    let mut body = column![heading(&crate::i18n::host_text(
        "settings.apps.title",
        "Apps",
        locale
    ))]
    .spacing(12)
    .width(Length::Fill);

    // 运行时探测 + 受管运行时的安装/卸载。
    body = body.push(dim(crate::i18n::host_text(
        "settings.apps.runtime_note",
        "Runtimes (managed versions ship with dozerd and take priority over system versions)",
        locale,
    )));
    let probes_view: El<'_> = match &state.probes {
        Load::Loading => dim(crate::i18n::host_text(
            "settings.apps.detecting",
            "Detecting…",
            locale,
        ))
        .into(),
        Load::Failed(why) => dim(crate::i18n::host_text_args(
            "settings.apps.probe_failed",
            "Probe failed: {why}",
            locale,
            &[("why", why)],
        ))
        .into(),
        Load::Loaded(probes) => {
            let mut col = column![].spacing(4);
            for probe in probes {
                col = col.push(runtime_row_view(state, probe, locale));
            }
            col.into()
        }
    };
    body = body.push(probes_view);

    // 已安装应用。
    body = body.push(heading(&crate::i18n::host_text(
        "settings.apps.installed_heading",
        "Installed",
        locale,
    )));
    let apps_view: El<'_> = match &state.apps {
        Load::Loading => dim(crate::i18n::host_text(
            "settings.apps.reading",
            "Reading…",
            locale,
        ))
        .into(),
        Load::Failed(why) => dim(crate::i18n::host_text_args(
            "settings.apps.read_failed",
            "Read failed: {why}",
            locale,
            &[("why", why)],
        ))
        .into(),
        Load::Loaded(apps) if apps.is_empty() => dim(crate::i18n::host_text(
            "settings.apps.no_apps",
            "No apps installed yet",
            locale,
        ))
        .into(),
        Load::Loaded(apps) => {
            let mut col = column![].spacing(6);
            for app in apps {
                col = col.push(app_row(state, app, locale));
                // 展开着日志的应用:在该行下方画查看器。
                if state.is_log_open(app.id.as_str()) {
                    col = col.push(log_viewer(
                        state.logs_view(app.id.as_str()),
                        &app.id,
                        locale,
                    ));
                }
            }
            col.into()
        }
    };
    body = body.push(apps_view);

    // 安装/卸载流程。
    body = body.push(flow_view(state, locale));

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
fn runtime_row_view<'a>(state: &'a State, probe: &'a RuntimeProbe, locale: &str) -> El<'a> {
    let colors = byteui::theme::color::current();
    let (name, status, _) = runtime_line(probe);
    let mut r = row![
        text(name.to_string())
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
                r = r.push(dim(crate::i18n::host_text_args(
                    "settings.apps.version_processing",
                    "{v} (processing…)",
                    locale,
                    &[("v", v)],
                )));
                continue;
            }
            r = r.push(dim(v.clone()));
            r = r.push(action_button(
                crate::i18n::host_text("settings.apps.uninstall", "Uninstall", locale),
                Message::RuntimeUninstallClicked(rt, v.clone()),
                colors.red,
            ));
        }
    } else if probe.installable
        && let Some(rt) = runtime
    {
        r = r.push(action_button(
            crate::i18n::host_text("settings.apps.install_ellipsis", "Install…", locale),
            Message::RuntimeInstallClicked(rt),
            colors.gold,
        ));
    }
    r.into()
}

fn app_row<'a>(state: &'a State, app: &'a AppSummary, locale: &str) -> El<'a> {
    let colors = byteui::theme::color::current();
    let id = app.id.as_str().to_owned();
    let version_line = crate::i18n::host_text_args(
        "settings.apps.version_line",
        "Version {version} · {observed}",
        locale,
        &[
            ("version", &app.version.to_string()),
            ("observed", &observed_label(&app.observed)),
        ],
    );
    // T6:外壳标题按当前 UI 语言解析;无 i18n 投影或解析落空时回落语言中立的 `app.name`。
    let title = app
        .display_i18n
        .as_ref()
        .and_then(|d| d.resolve_title(&DisplayTitleScope::Presentation, locale))
        .unwrap_or(app.name.as_str());
    let mut r = row![
        text(title)
            .size(byteui::theme::font::body())
            .color(colors.cream),
        dim(version_line),
        Space::new().width(Length::Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    match state.is_acting(&id) {
        Some(_) => {
            r = r.push(dim(crate::i18n::host_text(
                "settings.apps.processing",
                "Processing…",
                locale,
            )))
        }
        None => {
            if matches!(
                app.observed,
                ObservedState::Running | ObservedState::Starting
            ) {
                r = r.push(action_button(
                    crate::i18n::host_text("settings.apps.stop", "Stop", locale),
                    Message::StopClicked(id.clone()),
                    colors.dim,
                ));
            }
            // 有上一版才给回滚入口(A6g)。
            if let Some(to) = app.previous_version {
                r = r.push(action_button(
                    crate::i18n::host_text_args(
                        "settings.apps.rollback_to",
                        "Roll back to {to}",
                        locale,
                        &[("to", &to.to_string())],
                    ),
                    Message::RollbackClicked(id.clone()),
                    colors.dim,
                ));
            }
            // 运行中与失败都能看日志;展开着时按钮变「收起日志」。
            let (log_label, log_msg) = if state.is_log_open(&id) {
                (
                    crate::i18n::host_text("settings.apps.hide_logs", "Hide logs", locale),
                    Message::HideLogs,
                )
            } else {
                (
                    crate::i18n::host_text("settings.apps.logs", "Logs", locale),
                    Message::ShowLogs(id.clone()),
                )
            };
            r = r.push(action_button(log_label, log_msg, colors.dim));
            if !app.observed.is_transient() {
                r = r.push(action_button(
                    crate::i18n::host_text("settings.apps.uninstall", "Uninstall", locale),
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
                text(crate::i18n::host_text_args(
                    "settings.apps.rollback_confirm",
                    "Roll back to {to}: app data is not rolled back with it, and the old version may not read data written by the new version. Roll back?",
                    locale,
                    &[("to", &to.to_string())],
                ))
                .size(byteui::theme::font::body())
                .color(colors.cream),
                row![
                    action_button(
                        crate::i18n::host_text("common.cancel", "Cancel", locale),
                        Message::RollbackCancelled,
                        colors.dim
                    ),
                    action_button(
                        crate::i18n::host_text("settings.apps.confirm_rollback", "Confirm rollback", locale),
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
fn log_viewer(view: LogsView, app_id: &bytehost_apps::id::AppId, locale: &str) -> El<'static> {
    let colors = byteui::theme::color::current();
    let mut col = column![].spacing(4).width(Length::Fill);
    match view {
        LogsView::Hidden => {}
        LogsView::Loading => {
            col = col.push(dim(crate::i18n::host_text(
                "settings.apps.reading_logs",
                "Reading logs…",
                locale,
            )))
        }
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
                col = col.push(dim(crate::i18n::host_text(
                    "settings.apps.logs_truncated",
                    "Only the last lines are shown",
                    locale,
                )));
            }
            if stale {
                col = col.push(dim(crate::i18n::host_text(
                    "settings.apps.logs_stale",
                    "Refresh failed; showing the previous content",
                    locale,
                )));
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
fn source_choice_row(current: SourceChoice, locale: &str) -> El<'static> {
    let colors = byteui::theme::color::current();
    let mk = |label: String, choice: SourceChoice| -> El<'static> {
        let selected = current == choice;
        action_button(
            label,
            Message::SourceChoiceChanged(choice),
            if selected { colors.gold } else { colors.dim },
        )
    };
    row![
        mk(
            crate::i18n::host_text("settings.apps.source_dir", "Local directory", locale),
            SourceChoice::Dir
        ),
        mk(
            crate::i18n::host_text("settings.apps.source_archive", "Local archive", locale),
            SourceChoice::Archive
        ),
        mk(
            crate::i18n::host_text("settings.apps.source_url", "Network URL", locale),
            SourceChoice::Url
        ),
    ]
    .spacing(8)
    .into()
}

/// 空闲时的安装表单:来源切换 + (视来源)各色输入框与主按钮。
fn install_form<'a>(state: &'a State, locale: &str) -> El<'a> {
    let colors = byteui::theme::color::current();
    let mut col = column![source_choice_row(state.source_choice, locale)]
        .spacing(8)
        .width(Length::Fill);
    match state.source_choice {
        SourceChoice::Dir => {
            col = col.push(action_button(
                crate::i18n::host_text(
                    "settings.apps.install_app_ellipsis",
                    "Install app…",
                    locale,
                ),
                Message::InstallClicked,
                colors.gold,
            ));
        }
        SourceChoice::Archive => {
            col = col.push(action_button(
                crate::i18n::host_text(
                    "settings.apps.install_app_ellipsis",
                    "Install app…",
                    locale,
                ),
                Message::PickArchive,
                colors.gold,
            ));
        }
        SourceChoice::Url => {
            col = col.push(dim(crate::i18n::host_text(
                "settings.apps.url_hint",
                "https archive URL (static apps only)",
                locale,
            )));
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
            col = col.push(dim(crate::i18n::host_text(
                "settings.apps.sha_hint",
                "Expected sha256 (optional; if set it is pinned, otherwise the approval card shows the actual computed hash)",
                locale,
            )));
            let sha_placeholder = crate::i18n::host_text(
                "settings.apps.sha_placeholder",
                "Leave empty = not pinned",
                locale,
            );
            col = col.push(byteui::form::input_text::view(
                &sha_placeholder,
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
                crate::i18n::host_text("settings.apps.fetch_plan", "Fetch plan", locale),
                Message::UrlPlanClicked,
                colors.gold,
            ));
        }
    }
    container(col).width(Length::Fill).into()
}

fn flow_view<'a>(state: &'a State, locale: &str) -> El<'a> {
    let colors = byteui::theme::color::current();
    let flow = &state.flow;
    match flow {
        Flow::Idle => install_form(state, locale),
        Flow::Picking { target } => match target {
            PickTarget::Dir => dim(crate::i18n::host_text(
                "settings.apps.pick_dir_hint",
                "Choose the app directory in the dialog (it must contain manifest.toml)…",
                locale,
            ))
            .into(),
            PickTarget::Archive => dim(crate::i18n::host_text(
                "settings.apps.pick_archive_hint",
                "Choose the archive in the dialog (.zip / .tar.gz / .tgz)…",
                locale,
            ))
            .into(),
        },
        Flow::Planning { .. } => dim(crate::i18n::host_text(
            "settings.apps.checking_app",
            "Checking app…",
            locale,
        ))
        .into(),
        Flow::Installing { .. } => dim(crate::i18n::host_text(
            "settings.apps.installing",
            "Installing…",
            locale,
        ))
        .into(),
        Flow::Failed { message } => column![
            text(message.as_str())
                .size(byteui::theme::font::body())
                .color(colors.red),
            action_button(
                crate::i18n::host_text("settings.apps.got_it", "Got it", locale),
                Message::FlowDismissed,
                colors.dim
            ),
        ]
        .spacing(8)
        .into(),
        Flow::ConfirmUninstall { name, .. } => column![
            text(crate::i18n::host_text_args(
                "settings.apps.uninstall_title",
                "Uninstall app \"{name}\"?",
                locale,
                &[("name", name.as_str())],
            ))
            .size(byteui::theme::font::body())
            .color(colors.cream),
            dim(crate::i18n::host_text(
                "settings.apps.uninstall_body",
                "\"Keep data\" removes only the program, so data remains after reinstall; \"Delete data too\" cannot be undone.",
                locale,
            )),
            row![
                action_button(
                    crate::i18n::host_text("common.cancel", "Cancel", locale),
                    Message::FlowDismissed,
                    colors.dim
                ),
                action_button(
                    crate::i18n::host_text("settings.apps.keep_data", "Keep data", locale),
                    Message::UninstallConfirmed(UninstallMode::Program),
                    colors.gold
                ),
                action_button(
                    crate::i18n::host_text(
                        "settings.apps.delete_data_too",
                        "Delete data too",
                        locale
                    ),
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
            review_view(plan, running, locale)
        }
        Flow::RuntimePlanning { runtime } => dim(crate::i18n::host_text_args(
            "settings.apps.runtime_planning",
            "Preparing the install plan for the {runtime} runtime…",
            locale,
            &[("runtime", managed_runtime_label(*runtime))],
        ))
        .into(),
        Flow::RuntimeReviewing { plan } => runtime_review_view(plan, locale),
        Flow::ConfirmRuntimeUninstall { runtime, version } => column![
            text(crate::i18n::host_text_args(
                "settings.apps.runtime_uninstall_title",
                "Uninstall managed runtime {runtime} {version}?",
                locale,
                &[
                    ("runtime", managed_runtime_label(*runtime)),
                    ("version", version.as_str()),
                ],
            ))
            .size(byteui::theme::font::body())
            .color(colors.cream),
            dim(crate::i18n::host_text(
                "settings.apps.runtime_uninstall_body",
                "Apps depending on it will fail to start next time because the runtime is missing; reinstalling can restore it.",
                locale,
            )),
            row![
                action_button(
                    crate::i18n::host_text("common.cancel", "Cancel", locale),
                    Message::FlowDismissed,
                    colors.dim
                ),
                action_button(
                    crate::i18n::host_text("settings.apps.uninstall", "Uninstall", locale),
                    Message::RuntimeUninstallConfirmed,
                    colors.red
                ),
            ]
            .spacing(8),
        ]
        .spacing(8)
        .into(),
    }
}

/// 运行时安装审批卡:逐行展示 `plan_lines`(来源 URL、固定版本、完整 SHA-256、目标目录)。
fn runtime_review_view<'a>(plan: &'a RuntimeInstallPlan, locale: &str) -> El<'a> {
    let colors = byteui::theme::color::current();
    let mut col = column![
        text(crate::i18n::host_text_args(
            "settings.apps.install_runtime_title",
            "Install the {runtime} runtime",
            locale,
            &[("runtime", managed_runtime_label(plan.runtime))],
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
    col = col.push(dim(crate::i18n::host_text(
        "settings.apps.will_do",
        "What will happen:",
        locale,
    )));
    for line in &plan.will_do {
        col = col.push(
            text(line.clone())
                .size(byteui::theme::font::label())
                .color(colors.cream),
        );
    }
    col = col.push(
        row![
            action_button(
                crate::i18n::host_text("common.cancel", "Cancel", locale),
                Message::FlowDismissed,
                colors.dim
            ),
            action_button(
                crate::i18n::host_text(
                    "settings.apps.download_and_install",
                    "Download and install",
                    locale
                ),
                Message::RuntimeApproveClicked,
                colors.gold
            ),
        ]
        .spacing(8),
    );
    container(col).padding(10).width(Length::Fill).into()
}

/// 审批卡:把 `plan_view` 的每一行如实画出来;升级(申请比已授予更多)的权限用金色标 `↑`。
fn review_view<'a>(plan: &'a InstallPlan, running: bool, locale: &str) -> El<'a> {
    let colors = byteui::theme::color::current();
    let view = plan_view(plan, running);
    let mut col = column![
        text(crate::i18n::host_text_args(
            "settings.apps.install_title",
            "Install {name}",
            locale,
            &[("name", view.title.as_str())],
        ))
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
    col = col.push(dim(crate::i18n::host_text(
        "settings.apps.source_heading",
        "Source:",
        locale,
    )));
    if view.source.from_network {
        col = col.push(
            text(crate::i18n::host_text(
                "settings.apps.from_network_untrusted",
                "From the network, untrusted",
                locale,
            ))
            .size(byteui::theme::font::label())
            .color(colors.gold),
        );
        col = col.push(dim(crate::i18n::host_text(
            "settings.apps.static_only_csp",
            "Static apps only, running under a strict CSP",
            locale,
        )));
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
    col = col.push(dim(crate::i18n::host_text(
        "settings.apps.will_run",
        "Installing and running will execute:",
        locale,
    )));
    for line in view.will_run {
        col = col.push(
            text(line)
                .size(byteui::theme::font::label())
                .color(colors.cream),
        );
    }
    col = col.push(dim(crate::i18n::host_text(
        "settings.apps.permissions_heading",
        "Requested permissions:",
        locale,
    )));
    if view.permissions.is_empty() {
        col = col.push(
            text(crate::i18n::host_text(
                "settings.apps.no_permissions",
                "No permissions requested",
                locale,
            ))
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
            action_button(
                crate::i18n::host_text("common.cancel", "Cancel", locale),
                Message::FlowDismissed,
                colors.dim,
            ),
            action_button(
                crate::i18n::host_text(
                    "settings.apps.approve_and_install",
                    "Approve and install",
                    locale
                ),
                Message::ApproveClicked,
                colors.gold,
            ),
        ]
        .spacing(8),
    );
    container(col).padding(10).width(Length::Fill).into()
}
