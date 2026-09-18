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
}

impl State {
    /// 打开设置弹窗时调用——从本地 `git_accounts.json` 重建三家的连接
    /// 展示态(不重新校验 token 有效性,只是回显上次连接成功记下的用户名)。
    pub fn load() -> State {
        let accounts = git_accounts::load();
        State {
            github: ConnectState::from_accounts(&accounts, GitProvider::GitHub),
            gitlab: ConnectState::from_accounts(&accounts, GitProvider::GitLab),
            gitee: ConnectState::from_accounts(&accounts, GitProvider::Gitee),
            suppress_next_blur: false,
            connect_tasks: HashMap::new(),
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
        Message::Close | Message::ConnectSubmit(_) => false,
    }
}

/// `state` 是 `&mut Option<State>`(不是 `&mut State`)——`Message::Close`
/// 需要能把它整个置回 `None`,同 `file_history::update`/`project_create::
/// update` 的既有写法。
pub fn update(
    state: &mut Option<State>,
    msg: Message,
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
    let Message::ConnectSubmit(provider) = msg else {
        unreachable!("已在 apply_sync_message 或顶部处理");
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

    let content = column![
        theme_title,
        scheme_row("深色 · ByteBoy2077", ColorScheme::Dark, current),
        scheme_row("浅色 · ByteBoy2077-Light", ColorScheme::Light, current),
        git_title,
        provider_row(GitProvider::GitHub, &state.github),
        provider_row(GitProvider::GitLab, &state.gitlab),
        provider_row(GitProvider::Gitee, &state.gitee),
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
}
