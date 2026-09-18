//! 顶栏设置齿轮弹窗:主题选择 + Git 账户连接(GitHub/GitLab/Gitee,PAT)。
//! 设计见 `docs/superpowers/specs/2026-09-18-git-account-settings-design.md`。
//! 渲染宿主是独立原生窗口 `platform::settings_overlay::SettingsOverlay`。

use crate::git_accounts::{self, GitAccountsState, GitProvider};
use byteui::theme::color::ColorScheme;

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
            *state.slot_mut(*provider) = ConnectState::NotConnected;
            true
        }
        Message::OpenTokenPage(provider) => {
            let _ = std::process::Command::new("open")
                .arg(provider.token_creation_url())
                .spawn();
            true
        }
        Message::Disconnect(provider) => {
            let _ = git_accounts::delete_token(*provider);
            let mut accounts = git_accounts::load();
            accounts.set(*provider, None);
            let _ = git_accounts::save(&accounts);
            *state.slot_mut(*provider) = ConnectState::NotConnected;
            true
        }
        Message::ConnectResult(provider, result) => {
            match result {
                Ok(username) => {
                    *state.slot_mut(*provider) = ConnectState::Connected {
                        username: username.clone(),
                    };
                }
                Err(e) => {
                    if let ConnectState::Editing { busy, error, .. } = state.slot_mut(*provider) {
                        *busy = false;
                        *error = Some(e.clone());
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
/// update` 的既有写法。`ConnectSubmit` 在 Task 5 补上。
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
    // 走到这里的只剩 ConnectSubmit,Task 5 实现。
    let _ = (handle, emit, msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_clicked_opens_editing_row() {
        let mut state = State {
            github: ConnectState::NotConnected,
            gitlab: ConnectState::NotConnected,
            gitee: ConnectState::NotConnected,
        };
        apply_sync_message(&mut state, &Message::ConnectClicked(GitProvider::GitHub));
        assert!(matches!(state.github, ConnectState::Editing { .. }));
        assert_eq!(state.gitlab, ConnectState::NotConnected);
    }

    #[test]
    fn token_changed_updates_editing_draft() {
        let mut state = State {
            github: ConnectState::Editing {
                token: String::new(),
                busy: false,
                error: None,
            },
            gitlab: ConnectState::NotConnected,
            gitee: ConnectState::NotConnected,
        };
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
        let mut state = State {
            github: ConnectState::Editing {
                token: "x".into(),
                busy: false,
                error: None,
            },
            gitlab: ConnectState::NotConnected,
            gitee: ConnectState::NotConnected,
        };
        apply_sync_message(&mut state, &Message::ConnectCancel(GitProvider::GitHub));
        assert_eq!(state.github, ConnectState::NotConnected);
    }

    #[test]
    fn connect_result_ok_sets_connected() {
        let mut state = State {
            github: ConnectState::Editing {
                token: "x".into(),
                busy: true,
                error: None,
            },
            gitlab: ConnectState::NotConnected,
            gitee: ConnectState::NotConnected,
        };
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
        let mut state = State {
            github: ConnectState::Editing {
                token: "x".into(),
                busy: true,
                error: None,
            },
            gitlab: ConnectState::NotConnected,
            gitee: ConnectState::NotConnected,
        };
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
    fn close_is_not_a_sync_message() {
        let mut state = State {
            github: ConnectState::NotConnected,
            gitlab: ConnectState::NotConnected,
            gitee: ConnectState::NotConnected,
        };
        assert!(!apply_sync_message(&mut state, &Message::Close));
    }
}
