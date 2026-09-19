//! 顶栏设置齿轮弹窗:主题选择 + Git 账户连接(GitHub/GitLab/Gitee,PAT)。
//! 设计见 `docs/superpowers/specs/2026-09-18-git-account-settings-design.md`。
//! 渲染宿主是独立原生窗口 `platform::settings_overlay::SettingsOverlay`。

use crate::git_accounts::{self, GitAccountsState, GitProvider};
use byteui::theme::color::ColorScheme;
use iced_widget::core::{Alignment, Element, Length};
use iced_widget::{Space, button, column, container, row, text};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectState {
    NotConnected,
    Editing {
        token: String,
        busy: bool,
        error: Option<String>,
    },
    Connected {
        username: String,
    },
}

impl ConnectState {
    fn from_accounts(accounts: &GitAccountsState, provider: GitProvider) -> ConnectState {
        match accounts.get(provider) {
            Some(acc) => ConnectState::Connected {
                username: acc.username.clone(),
            },
            None => ConnectState::NotConnected,
        }
    }
}

/// "高级"区块(停止/重新启动 dozerd)的状态机。`Idle`/`Stopped` 各自内嵌
/// `error: Option<String>`——失败态就是"回到默认态/已停止态,附一行错误
/// 文案",不单独建一个 `Failed` 变体(spec「UI 设计」:停止失败按钮恢复
/// 默认态、重启失败保留"重新启动"按钮,两者语义上就是各自基础态的一个
/// 变体,不是第三种独立状态)。
#[derive(Debug, Clone, PartialEq)]
pub enum AdvancedState {
    Idle { error: Option<String> },
    FetchingCount,
    ConfirmingStop { session_count: u32 },
    Stopping,
    Stopped { error: Option<String> },
    RestartingDozerd,
}

pub struct State {
    pub github: ConnectState,
    pub gitlab: ConnectState,
    pub gitee: ConnectState,
    /// `OpenTokenPage` 拉起系统浏览器时置位——那会让本窗口收到一次真实
    /// `Focused(false)`,若照常触发失焦即关闭会把正在填的 PAT 表单整个
    /// 关掉(代码评审 finding:PAT-link click can auto-close Settings)。
    /// `SettingsOverlay::handle_focus` 读到就消费掉、吞掉这一次失焦。
    pub(crate) suppress_next_blur: bool,
    /// 正在进行的 `ConnectSubmit` 异步任务句柄,按 provider 存一份。
    /// `ConnectCancel` 用它真正中止任务,防止取消后 token 仍被异步写进
    /// Keychain/本地文件(代码评审 finding:Cancel doesn't stop in-flight
    /// connect task)。
    connect_tasks: HashMap<GitProvider, tokio::task::AbortHandle>,
    pub advanced: AdvancedState,
}

impl State {
    /// 打开设置弹窗时调用——从本地 `git_accounts.json` 重建三家的连接
    /// 展示态(不重新校验 token 有效性,只是回显上次连接成功记下的用户名)。
    ///
    /// `daemon_error` 是 `App.daemon_error`(整程序共享的"daemon 连不上"
    /// 状态)。每次开 Settings 都会重建一份 `State`,如果 `advanced` 一律从
    /// `Idle` 起步,用户停止 dozerd 后关窗再开就只剩"停止 dozerd"按钮,
    /// "重新启动 dozerd"这个入口永久丢失(spec 2026-09-19:"可随时重新
    /// 启动"),再点一次只会拿到一行连接报错。所以这里按 daemon 当前是否
    /// 可达推导初值——真相源是可达性,不是上次 UI 停在哪。`ensure_daemon`
    /// 幂等(daemon 其实活着时 `list()` 立刻成功返回 `Ok`),即便传进来的是
    /// 过期的 `daemon_error` 也不会误伤。
    pub fn load(daemon_error: Option<&str>) -> State {
        let accounts = git_accounts::load();
        State {
            github: ConnectState::from_accounts(&accounts, GitProvider::GitHub),
            gitlab: ConnectState::from_accounts(&accounts, GitProvider::GitLab),
            gitee: ConnectState::from_accounts(&accounts, GitProvider::Gitee),
            suppress_next_blur: false,
            connect_tasks: HashMap::new(),
            advanced: advanced_state_for_daemon(daemon_error),
        }
    }

    fn slot_mut(&mut self, provider: GitProvider) -> &mut ConnectState {
        match provider {
            GitProvider::GitHub => &mut self.github,
            GitProvider::GitLab => &mut self.gitlab,
            GitProvider::Gitee => &mut self.gitee,
        }
    }
}

/// 开窗时"高级"区块的初始态:`daemon_error` 有值 = daemon 不可达 = 该给
/// "重新启动 dozerd";否则给默认的"停止 dozerd"。抽成纯函数是为了不起
/// daemon 就能测两个分支。
fn advanced_state_for_daemon(daemon_error: Option<&str>) -> AdvancedState {
    match daemon_error {
        Some(_) => AdvancedState::Stopped { error: None },
        None => AdvancedState::Idle { error: None },
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Close,
    ThemeSelected(ColorScheme),
    ConnectClicked(GitProvider),
    TokenChanged(GitProvider, String),
    ConnectCancel(GitProvider),
    OpenTokenPage(GitProvider),
    ConnectSubmit(GitProvider),
    ConnectResult(GitProvider, Result<String, String>),
    Disconnect(GitProvider),
    AdvancedStopClicked,
    AdvancedSessionCountReady(Result<u32, String>),
    AdvancedStopCancel,
    AdvancedStopConfirm,
    AdvancedStopResult(Result<(), String>),
    AdvancedRestartClicked,
    AdvancedRestartResult(Result<(), String>),
}

/// 处理不需要 `handle`(异步)的消息,返回 `true` 表示已处理完。纯状态
/// 转换,方便单测不用真的起 tokio 任务(同 `project_create::
/// apply_field_message` 的既有拆分手法)。
fn apply_sync_message(state: &mut State, msg: &Message) -> bool {
    match msg {
        Message::ThemeSelected(scheme) => {
            byteui::theme::color::set_scheme(*scheme);
            byteui::theme::color::persist_scheme(&crate::theme::color_theme_path());
            true
        }
        Message::ConnectClicked(provider) => {
            *state.slot_mut(*provider) = ConnectState::Editing {
                token: String::new(),
                busy: false,
                error: None,
            };
            true
        }
        Message::TokenChanged(provider, token) => {
            if let ConnectState::Editing { token: t, .. } = state.slot_mut(*provider) {
                *t = token.clone();
            }
            true
        }
        Message::ConnectCancel(provider) => {
            // 真正中止掉还在跑的 validate_token/set_token/save 任务——
            // 只改 UI 状态不够,那个任务不知道自己被"取消"了,还是会把
            // token 写进 Keychain(代码评审 finding:Cancel doesn't stop
            // in-flight connect task)。
            if let Some(task) = state.connect_tasks.remove(provider) {
                task.abort();
            }
            *state.slot_mut(*provider) = ConnectState::NotConnected;
            true
        }
        Message::OpenTokenPage(provider) => {
            state.suppress_next_blur = true;
            let _ = std::process::Command::new("open")
                .arg(provider.token_creation_url())
                .spawn();
            true
        }
        Message::Disconnect(provider) => {
            // 先写本地非敏感记录,成功了才删 Keychain token——反过来做的话,
            // 本地文件写失败会留下"Keychain 已删、json 仍写着已连接"这种
            // 更糟的不一致态(代码评审 finding:Disconnect drops failed
            // save, leaves stale state)。写失败就整个放弃这次断开,保留
            // 原有已连接展示,只记日志。
            let mut accounts = git_accounts::load();
            accounts.set(*provider, None);
            match git_accounts::save(&accounts) {
                Ok(()) => {
                    let _ = git_accounts::delete_token(*provider);
                    *state.slot_mut(*provider) = ConnectState::NotConnected;
                }
                Err(e) => {
                    tracing::warn!(
                        provider = provider.as_key(),
                        error = %e,
                        "断开账户失败:写入本地记录出错,已保留 Keychain token 与已连接展示"
                    );
                }
            }
            true
        }
        Message::ConnectResult(provider, result) => {
            // 任务已经跑完,不管结果如何都清掉记着的句柄。
            state.connect_tasks.remove(provider);
            // 结果到达时若已经不是 Editing 态(用户已经 Cancel 或又开了
            // 新一轮),说明这是一份过期结果,丢弃不应用——否则会出现
            // "已经点了取消,连接状态却又被悄悄改回已连接"的情况(代码
            // 评审 finding:Cancel doesn't stop in-flight connect task)。
            if matches!(state.slot_mut(*provider), ConnectState::Editing { .. }) {
                match result {
                    Ok(username) => {
                        *state.slot_mut(*provider) = ConnectState::Connected {
                            username: username.clone(),
                        };
                    }
                    Err(e) => {
                        if let ConnectState::Editing { busy, error, .. } = state.slot_mut(*provider)
                        {
                            *busy = false;
                            *error = Some(e.clone());
                        }
                    }
                }
            }
            true
        }
        Message::AdvancedStopCancel => {
            state.advanced = AdvancedState::Idle { error: None };
            true
        }
        Message::AdvancedSessionCountReady(result) => {
            state.advanced = match result {
                Ok(n) => AdvancedState::ConfirmingStop { session_count: *n },
                Err(e) => AdvancedState::Idle {
                    error: Some(e.clone()),
                },
            };
            true
        }
        Message::AdvancedStopResult(result) => {
            state.advanced = match result {
                Ok(()) => AdvancedState::Stopped { error: None },
                Err(e) => AdvancedState::Idle {
                    error: Some(e.clone()),
                },
            };
            true
        }
        Message::AdvancedRestartResult(result) => {
            state.advanced = match result {
                Ok(()) => AdvancedState::Idle { error: None },
                Err(e) => AdvancedState::Stopped {
                    error: Some(e.clone()),
                },
            };
            true
        }
        Message::Close
        | Message::ConnectSubmit(_)
        | Message::AdvancedStopClicked
        | Message::AdvancedStopConfirm
        | Message::AdvancedRestartClicked => false,
    }
}

/// `state` 是 `&mut Option<State>`(不是 `&mut State`)——`Message::Close`
/// 需要能把它整个置回 `None`,同 `file_history::update`/`project_create::
/// update` 的既有写法。
pub fn update(
    state: &mut Option<State>,
    msg: Message,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    if let Message::Close = msg {
        *state = None;
        return;
    }
    let Some(s) = state else { return };
    if apply_sync_message(s, &msg) {
        return;
    }
    let provider = match msg {
        Message::ConnectSubmit(provider) => provider,
        Message::AdvancedStopClicked => {
            if !matches!(s.advanced, AdvancedState::Idle { .. }) {
                return;
            }
            s.advanced = AdvancedState::FetchingCount;
            let client = client.clone();
            handle.spawn(async move {
                let result = client
                    .list()
                    .await
                    .map(|sessions| count_live_sessions(&sessions))
                    .map_err(|e| e.to_string());
                emit(Message::AdvancedSessionCountReady(result));
            });
            return;
        }
        Message::AdvancedStopConfirm => {
            if !matches!(s.advanced, AdvancedState::ConfirmingStop { .. }) {
                return;
            }
            s.advanced = AdvancedState::Stopping;
            let client = client.clone();
            handle.spawn(async move {
                let result = client.shutdown_daemon().await.map_err(|e| e.to_string());
                emit(Message::AdvancedStopResult(result));
            });
            return;
        }
        Message::AdvancedRestartClicked => {
            if !matches!(s.advanced, AdvancedState::Stopped { .. }) {
                return;
            }
            s.advanced = AdvancedState::RestartingDozerd;
            let client = client.clone();
            handle.spawn(async move {
                let result = crate::runtime::ensure_daemon(&client).await;
                emit(Message::AdvancedRestartResult(result));
            });
            return;
        }
        _ => unreachable!("已在 apply_sync_message 或上面处理"),
    };
    let token = match s.slot_mut(provider) {
        ConnectState::Editing { token, busy, error } => {
            if token.trim().is_empty() {
                return;
            }
            *busy = true;
            *error = None;
            token.clone()
        }
        _ => return,
    };
    // 防御性清理:正常 UI 流程不会在上一次提交还没出结果时再提交一次
    // (按钮在 busy 态会被禁用),但如果真的发生了,先中止旧任务再开新的,
    // 不留两个任务同时跑。
    if let Some(old) = s.connect_tasks.remove(&provider) {
        old.abort();
    }
    let join_handle = handle.spawn(async move {
        let result = git_accounts::validate_token(provider, &token).await;
        let result = result.and_then(|username| {
            git_accounts::set_token(provider, &token)?;
            let mut accounts = git_accounts::load();
            accounts.set(
                provider,
                Some(git_accounts::ConnectedAccount {
                    username: username.clone(),
                }),
            );
            git_accounts::save(&accounts).map_err(|e| format!("写入本地记录失败: {e}"))?;
            Ok(username)
        });
        emit(Message::ConnectResult(provider, result));
    });
    s.connect_tasks.insert(provider, join_handle.abort_handle());
}

fn scheme_row<'a>(
    label: &'static str,
    scheme: ColorScheme,
    current: ColorScheme,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let selected = scheme == current;
    let mark: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        text(if selected { "✓" } else { " " })
            .size(byteui::theme::font::body())
            .into();
    crate::chrome::menu::item_row_fill(
        Some(mark),
        label,
        if selected {
            byteui::theme::color::current().gold
        } else {
            byteui::theme::color::current().body
        },
        Some(Message::ThemeSelected(scheme)),
    )
}

fn provider_row(
    provider: GitProvider,
    state: &ConnectState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let title = text(provider.display_name())
        .size(byteui::theme::font::body())
        .color(colors.cream);
    match state {
        ConnectState::NotConnected => {
            let status = text("未连接")
                .size(byteui::theme::font::label())
                .color(colors.dim);
            let connect_btn = button(text("连接").size(byteui::theme::font::body()))
                .on_press(Message::ConnectClicked(provider))
                .padding([6, 14]);
            row![title, status, Space::new().width(Length::Fill), connect_btn]
                .spacing(10)
                .align_y(Alignment::Center)
                .into()
        }
        ConnectState::Connected { username } => {
            let status = text(format!("已连接: {username}"))
                .size(byteui::theme::font::label())
                .color(colors.gold);
            let disconnect_btn = button(text("断开连接").size(byteui::theme::font::body()))
                .on_press(Message::Disconnect(provider))
                .padding([6, 14]);
            row![
                title,
                status,
                Space::new().width(Length::Fill),
                disconnect_btn
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into()
        }
        ConnectState::Editing { token, busy, error } => {
            let input = byteui::form::input_text::view_on_bg(
                "Personal Access Token",
                token,
                true,
                None,
                false,
                None,
                false,
                move |v| Message::TokenChanged(provider, v),
            );
            let hint = iced_widget::MouseArea::new(
                text("没有 PAT?点此生成")
                    .size(byteui::theme::font::label())
                    .color(colors.gold),
            )
            .interaction(iced_widget::core::mouse::Interaction::Pointer)
            .on_press(Message::OpenTokenPage(provider));
            let confirm_label = if *busy { "校验中…" } else { "确认" };
            let confirm = button(text(confirm_label).size(byteui::theme::font::body()));
            let confirm = if *busy {
                confirm
            } else {
                confirm.on_press(Message::ConnectSubmit(provider))
            };
            let cancel = button(text("取消").size(byteui::theme::font::body()))
                .on_press(Message::ConnectCancel(provider));
            let error_row: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
                if let Some(e) = error {
                    text(e.clone())
                        .size(byteui::theme::font::label())
                        .color(colors.red)
                        .into()
                } else {
                    Space::new().into()
                };
            column![
                title,
                input,
                row![hint, Space::new().width(Length::Fill), cancel, confirm].spacing(8),
                error_row,
            ]
            .spacing(6)
            .into()
        }
    }
}

/// 确认弹窗正文里"N 个正在运行的 agent 会话"的 N(spec 2026-09-19 §「UI
/// 设计」:插入的是"当前**存活**会话数")。`ListSessions` 返回的是 registry
/// 全量快照——daemon 侧从不摘除已退出的会话,所以直接 `len()` 会把用户早就
/// 关掉的 tab 也算进"正在运行",既虚报数字又让 N==0 那条分支几乎永远走不到。
/// 抽成纯函数是为了不起 daemon 就能测。
fn count_live_sessions(sessions: &[dozer_core::protocol::SessionInfo]) -> u32 {
    sessions.iter().filter(|s| s.alive).count() as u32
}

fn advanced_row(
    state: &AdvancedState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let hint = |s: String| text(s).size(byteui::theme::font::label()).color(colors.dim);
    let error_line =
        |e: &Option<String>| -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
            match e {
                Some(msg) => text(msg.clone())
                    .size(byteui::theme::font::label())
                    .color(colors.red)
                    .into(),
                None => Space::new().into(),
            }
        };
    match state {
        AdvancedState::Idle { error } => {
            let btn = button(text("停止 dozerd").size(byteui::theme::font::body()))
                .on_press(Message::AdvancedStopClicked)
                .padding([6, 14]);
            column![
                hint("停止后所有正在运行的 agent 会话会结束并生成总结,可随时重新启动。".into()),
                row![Space::new().width(Length::Fill), btn],
                error_line(error),
            ]
            .spacing(6)
            .into()
        }
        AdvancedState::FetchingCount => {
            let btn = button(text("检查中…").size(byteui::theme::font::body())).padding([6, 14]);
            row![
                hint("停止后所有正在运行的 agent 会话会结束并生成总结,可随时重新启动。".into()),
                Space::new().width(Length::Fill),
                btn
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        }
        AdvancedState::ConfirmingStop { session_count } => {
            let body = if *session_count == 0 {
                "当前没有正在运行的会话,dozerd 会直接停止。".to_string()
            } else {
                format!(
                    "这会结束当前 {session_count} 个正在运行的 agent 会话并生成总结(可能需要约 1 分钟)。"
                )
            };
            let cancel = button(text("取消").size(byteui::theme::font::body()))
                .on_press(Message::AdvancedStopCancel)
                .padding([6, 14]);
            let confirm = button(
                text("确认停止")
                    .size(byteui::theme::font::body())
                    .color(colors.red),
            )
            .on_press(Message::AdvancedStopConfirm)
            .padding([6, 14]);
            column![
                hint(body),
                row![Space::new().width(Length::Fill), cancel, confirm].spacing(8),
            ]
            .spacing(6)
            .into()
        }
        AdvancedState::Stopping => {
            let btn = button(
                text("停止中…(等待会话总结,最长约 1 分钟)").size(byteui::theme::font::body()),
            )
            .padding([6, 14]);
            row![Space::new().width(Length::Fill), btn].into()
        }
        AdvancedState::Stopped { error } => {
            let btn = button(text("重新启动 dozerd").size(byteui::theme::font::body()))
                .on_press(Message::AdvancedRestartClicked)
                .padding([6, 14]);
            column![
                hint("dozerd 已停止,部分功能不可用。".into()),
                row![Space::new().width(Length::Fill), btn],
                error_line(error),
            ]
            .spacing(6)
            .into()
        }
        AdvancedState::RestartingDozerd => {
            let btn = button(text("启动中…").size(byteui::theme::font::body())).padding([6, 14]);
            row![Space::new().width(Length::Fill), btn].into()
        }
    }
}

pub fn settings_card(
    state: &State,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let current = byteui::theme::color::current_scheme();
    let theme_title = text("主题")
        .size(byteui::theme::font::subtitle())
        .color(colors.cream);
    let git_title = text("Git 账户")
        .size(byteui::theme::font::subtitle())
        .color(colors.cream);
    let close = button(
        text("关闭")
            .size(byteui::theme::font::body())
            .color(colors.dim),
    )
    .on_press(Message::Close)
    .padding([6, 12])
    .style(crate::dialog::action_button_style(colors.dim));

    let advanced_title = text("高级")
        .size(byteui::theme::font::subtitle())
        .color(colors.cream);
    let content = column![
        theme_title,
        scheme_row("深色 · ByteBoy2077", ColorScheme::Dark, current),
        scheme_row("浅色 · ByteBoy2077-Light", ColorScheme::Light, current),
        git_title,
        provider_row(GitProvider::GitHub, &state.github),
        provider_row(GitProvider::GitLab, &state.gitlab),
        provider_row(GitProvider::Gitee, &state.gitee),
        advanced_title,
        advanced_row(&state.advanced),
        crate::dialog::actions(row![close]),
    ]
    .spacing(14);

    container(content)
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(crate::dialog::card_style)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试专用构造:`connect_tasks`/`suppress_next_blur` 是任务生命周期
    /// 相关的簿记字段,跟这些纯状态转换测试无关,统一给默认值,避免每个
    /// 测试都重复写。
    fn test_state(github: ConnectState, gitlab: ConnectState, gitee: ConnectState) -> State {
        State {
            github,
            gitlab,
            gitee,
            suppress_next_blur: false,
            connect_tasks: HashMap::new(),
            advanced: AdvancedState::Idle { error: None },
        }
    }

    #[test]
    fn connect_clicked_opens_editing_row() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        apply_sync_message(&mut state, &Message::ConnectClicked(GitProvider::GitHub));
        assert!(matches!(state.github, ConnectState::Editing { .. }));
        assert_eq!(state.gitlab, ConnectState::NotConnected);
    }

    #[test]
    fn token_changed_updates_editing_draft() {
        let mut state = test_state(
            ConnectState::Editing {
                token: String::new(),
                busy: false,
                error: None,
            },
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        apply_sync_message(
            &mut state,
            &Message::TokenChanged(GitProvider::GitHub, "ghp_xxx".into()),
        );
        assert_eq!(
            state.github,
            ConnectState::Editing {
                token: "ghp_xxx".into(),
                busy: false,
                error: None,
            }
        );
    }

    #[test]
    fn connect_cancel_resets_to_not_connected() {
        let mut state = test_state(
            ConnectState::Editing {
                token: "x".into(),
                busy: false,
                error: None,
            },
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        apply_sync_message(&mut state, &Message::ConnectCancel(GitProvider::GitHub));
        assert_eq!(state.github, ConnectState::NotConnected);
    }

    #[test]
    fn connect_cancel_aborts_in_flight_task_and_discards_late_result() {
        let mut state = test_state(
            ConnectState::Editing {
                token: "x".into(),
                busy: true,
                error: None,
            },
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let join = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        state
            .connect_tasks
            .insert(GitProvider::GitHub, join.abort_handle());
        apply_sync_message(&mut state, &Message::ConnectCancel(GitProvider::GitHub));
        assert_eq!(state.github, ConnectState::NotConnected);
        assert!(!state.connect_tasks.contains_key(&GitProvider::GitHub));
        // 取消之后,哪怕之前那次任务的结果晚一步才送达,也不应该把状态
        // 又悄悄改回 Connected(代码评审 finding:Cancel doesn't stop
        // in-flight connect task)。
        apply_sync_message(
            &mut state,
            &Message::ConnectResult(GitProvider::GitHub, Ok("octocat".into())),
        );
        assert_eq!(state.github, ConnectState::NotConnected);
    }

    #[test]
    fn connect_result_ok_sets_connected() {
        let mut state = test_state(
            ConnectState::Editing {
                token: "x".into(),
                busy: true,
                error: None,
            },
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        apply_sync_message(
            &mut state,
            &Message::ConnectResult(GitProvider::GitHub, Ok("octocat".into())),
        );
        assert_eq!(
            state.github,
            ConnectState::Connected {
                username: "octocat".into()
            }
        );
    }

    #[test]
    fn connect_result_err_keeps_editing_with_error() {
        let mut state = test_state(
            ConnectState::Editing {
                token: "x".into(),
                busy: true,
                error: None,
            },
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        apply_sync_message(
            &mut state,
            &Message::ConnectResult(GitProvider::GitHub, Err("令牌无效或已过期".into())),
        );
        assert_eq!(
            state.github,
            ConnectState::Editing {
                token: "x".into(),
                busy: false,
                error: Some("令牌无效或已过期".into()),
            }
        );
    }

    #[test]
    fn open_token_page_sets_suppress_next_blur() {
        let mut state = test_state(
            ConnectState::Editing {
                token: "x".into(),
                busy: false,
                error: None,
            },
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        apply_sync_message(&mut state, &Message::OpenTokenPage(GitProvider::GitHub));
        assert!(state.suppress_next_blur);
    }

    #[test]
    fn close_is_not_a_sync_message() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        assert!(!apply_sync_message(&mut state, &Message::Close));
    }

    #[test]
    fn advanced_stop_cancel_returns_to_idle() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::ConfirmingStop { session_count: 2 };
        apply_sync_message(&mut state, &Message::AdvancedStopCancel);
        assert_eq!(state.advanced, AdvancedState::Idle { error: None });
    }

    #[test]
    fn advanced_session_count_ready_ok_opens_confirm() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::FetchingCount;
        apply_sync_message(&mut state, &Message::AdvancedSessionCountReady(Ok(3)));
        assert_eq!(
            state.advanced,
            AdvancedState::ConfirmingStop { session_count: 3 }
        );
    }

    #[test]
    fn advanced_session_count_ready_err_returns_to_idle_with_error() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::FetchingCount;
        apply_sync_message(
            &mut state,
            &Message::AdvancedSessionCountReady(Err("daemon 断开".into())),
        );
        assert_eq!(
            state.advanced,
            AdvancedState::Idle {
                error: Some("daemon 断开".into())
            }
        );
    }

    #[test]
    fn advanced_stop_result_ok_marks_stopped() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::Stopping;
        apply_sync_message(&mut state, &Message::AdvancedStopResult(Ok(())));
        assert_eq!(state.advanced, AdvancedState::Stopped { error: None });
    }

    #[test]
    fn advanced_stop_result_err_returns_to_idle_with_error() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::Stopping;
        apply_sync_message(
            &mut state,
            &Message::AdvancedStopResult(Err("等待 dozerd 停止超时".into())),
        );
        assert_eq!(
            state.advanced,
            AdvancedState::Idle {
                error: Some("等待 dozerd 停止超时".into())
            }
        );
    }

    #[test]
    fn advanced_restart_result_ok_returns_to_idle() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::RestartingDozerd;
        apply_sync_message(&mut state, &Message::AdvancedRestartResult(Ok(())));
        assert_eq!(state.advanced, AdvancedState::Idle { error: None });
    }

    #[test]
    fn advanced_restart_result_err_stays_stopped_with_error() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::RestartingDozerd;
        apply_sync_message(
            &mut state,
            &Message::AdvancedRestartResult(Err("无法连接 dozerd".into())),
        );
        assert_eq!(
            state.advanced,
            AdvancedState::Stopped {
                error: Some("无法连接 dozerd".into())
            }
        );
    }

    #[test]
    fn count_live_sessions_ignores_dead_sessions_in_registry_snapshot() {
        let session = |id: &str, alive: bool| dozer_core::protocol::SessionInfo {
            id: id.to_string(),
            name: id.to_string(),
            command: "/bin/sh".to_string(),
            cwd: "/tmp".to_string(),
            alive,
            created_ms: 0,
            agent_state: dozer_core::protocol::AgentState::Idle,
            transcript_path: None,
            project_id: Some(1),
            agent: dozer_core::protocol::AgentKind::Unknown,
        };

        assert_eq!(count_live_sessions(&[]), 0);
        assert_eq!(count_live_sessions(&[session("a", true)]), 1);
        assert_eq!(count_live_sessions(&[session("a", false)]), 0);
        assert_eq!(
            count_live_sessions(&[
                session("a", true),
                session("b", false),
                session("c", true),
                session("d", false),
            ]),
            2,
            "registry 快照里的死会话不该被算成「正在运行」"
        );
    }

    #[test]
    fn advanced_async_messages_are_not_sync_messages() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        assert!(!apply_sync_message(
            &mut state,
            &Message::AdvancedStopClicked
        ));
        assert!(!apply_sync_message(
            &mut state,
            &Message::AdvancedStopConfirm
        ));
        assert!(!apply_sync_message(
            &mut state,
            &Message::AdvancedRestartClicked
        ));
    }

    #[test]
    fn advanced_state_for_daemon_offers_restart_only_when_daemon_unreachable() {
        assert_eq!(
            advanced_state_for_daemon(None),
            AdvancedState::Idle { error: None },
            "daemon 可达时应该给「停止 dozerd」"
        );
        assert_eq!(
            advanced_state_for_daemon(Some("dozerd 已停止,部分功能不可用")),
            AdvancedState::Stopped { error: None },
            "daemon 不可达时必须给「重新启动 dozerd」,否则关窗重开后入口丢失"
        );
    }
}
