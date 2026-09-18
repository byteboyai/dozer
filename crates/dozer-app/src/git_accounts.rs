//! Git 账户连接的本地元数据(非敏感:用户名/连接状态)。Token 本身存在
//! macOS Keychain(`keyring` crate,命名空间 `"dozer-git"`),不落地任何
//! 明文文件——本文件的 JSON 只存显示用的用户名。设计见
//! `docs/superpowers/specs/2026-09-18-git-account-settings-design.md`。

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GitProvider {
    GitHub,
    GitLab,
    Gitee,
}

impl GitProvider {
    pub const ALL: [GitProvider; 3] = [GitProvider::GitHub, GitProvider::GitLab, GitProvider::Gitee];

    pub fn as_key(self) -> &'static str {
        match self {
            GitProvider::GitHub => "github",
            GitProvider::GitLab => "gitlab",
            GitProvider::Gitee => "gitee",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            GitProvider::GitHub => "GitHub",
            GitProvider::GitLab => "GitLab",
            GitProvider::Gitee => "Gitee",
        }
    }

    pub fn whoami_url(self) -> &'static str {
        match self {
            GitProvider::GitHub => "https://api.github.com/user",
            GitProvider::GitLab => "https://gitlab.com/api/v4/user",
            GitProvider::Gitee => "https://gitee.com/api/v5/user",
        }
    }

    pub fn token_creation_url(self) -> &'static str {
        match self {
            GitProvider::GitHub => "https://github.com/settings/tokens/new",
            GitProvider::GitLab => "https://gitlab.com/-/user_settings/personal_access_tokens",
            GitProvider::Gitee => "https://gitee.com/profile/personal_access_tokens/new",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectedAccount {
    pub username: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GitAccountsState {
    pub github: Option<ConnectedAccount>,
    pub gitlab: Option<ConnectedAccount>,
    pub gitee: Option<ConnectedAccount>,
}

impl GitAccountsState {
    pub fn get(&self, provider: GitProvider) -> Option<&ConnectedAccount> {
        match provider {
            GitProvider::GitHub => self.github.as_ref(),
            GitProvider::GitLab => self.gitlab.as_ref(),
            GitProvider::Gitee => self.gitee.as_ref(),
        }
    }

    pub fn set(&mut self, provider: GitProvider, account: Option<ConnectedAccount>) {
        match provider {
            GitProvider::GitHub => self.github = account,
            GitProvider::GitLab => self.gitlab = account,
            GitProvider::Gitee => self.gitee = account,
        }
    }
}

fn file_path() -> PathBuf {
    dozer_core::paths::config_dir().join("git_accounts.json")
}

pub fn load() -> GitAccountsState {
    load_from(&file_path())
}

pub fn save(state: &GitAccountsState) -> io::Result<()> {
    save_to(&file_path(), state)
}

fn load_from(path: &Path) -> GitAccountsState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_to(path: &Path, state: &GitAccountsState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state).expect("GitAccountsState 总能序列化");
    std::fs::write(path, json)
}

fn keyring_entry(provider: GitProvider) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new("dozer-git", provider.as_key())
}

/// 读不到按未连接处理,不 panic(同 `extensions::ssh::keyring_password`
/// 的既有口径)。
pub fn get_token(provider: GitProvider) -> Option<String> {
    keyring_entry(provider).ok().and_then(|e| e.get_password().ok())
}

pub fn set_token(provider: GitProvider, token: &str) -> Result<(), String> {
    keyring_entry(provider)
        .and_then(|e| e.set_password(token))
        .map_err(|e| format!("系统钥匙串写入失败: {e}"))
}

/// 条目本来就不存在(`keyring::Error::NoEntry`)视为成功——"断开一个从没
/// 真正连上的账户"不应该报错。
pub fn delete_token(provider: GitProvider) -> Result<(), String> {
    match keyring_entry(provider) {
        Ok(entry) => match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("系统钥匙串删除失败: {e}")),
        },
        Err(e) => Err(format!("系统钥匙串删除失败: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_provider_keys_and_urls_are_distinct() {
        for p in GitProvider::ALL {
            assert!(!p.as_key().is_empty());
            assert!(p.whoami_url().starts_with("https://"));
            assert!(p.token_creation_url().starts_with("https://"));
        }
        assert_ne!(GitProvider::GitHub.as_key(), GitProvider::GitLab.as_key());
        assert_ne!(GitProvider::GitLab.as_key(), GitProvider::Gitee.as_key());
    }

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        assert_eq!(load_from(&path), GitAccountsState::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("git_accounts.json");
        let mut state = GitAccountsState::default();
        state.set(
            GitProvider::GitHub,
            Some(ConnectedAccount {
                username: "octocat".to_string(),
            }),
        );
        save_to(&path, &state).unwrap();
        assert_eq!(load_from(&path), state);
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load_from(&path), GitAccountsState::default());
    }

    #[test]
    fn get_and_set_route_to_matching_field() {
        let mut state = GitAccountsState::default();
        assert_eq!(state.get(GitProvider::Gitee), None);
        state.set(
            GitProvider::Gitee,
            Some(ConnectedAccount {
                username: "abc".to_string(),
            }),
        );
        assert_eq!(state.get(GitProvider::Gitee).unwrap().username, "abc");
        assert_eq!(state.get(GitProvider::GitHub), None);
    }
}
