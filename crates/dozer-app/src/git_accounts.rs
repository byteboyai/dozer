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
    pub const ALL: [GitProvider; 3] =
        [GitProvider::GitHub, GitProvider::GitLab, GitProvider::Gitee];

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
    keyring_entry(provider)
        .ok()
        .and_then(|e| e.get_password().ok())
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

/// 三家 whoami 接口(GitHub `/user`、GitLab `/api/v4/user`、Gitee
/// `/api/v5/user`)返回的用户名字段都叫 `login`,不需要按 provider 分支。
pub(crate) fn extract_username(json: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("解析响应失败: {e}"))?;
    value
        .get("login")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "响应中缺少 login 字段".to_string())
}

/// 调用服务商 whoami 接口校验 `token` 有效,成功返回用户名。三家鉴权方式
/// 不同:GitHub 用 `Authorization: Bearer`,GitLab 用 `PRIVATE-TOKEN` 头,
/// Gitee 用 `access_token` query 参数(Gitee API v5 的既定用法)。不做
/// 401/403/404 的文案区分(YAGNI),非 2xx 一律归为"令牌无效或已过期"。
pub async fn validate_token(provider: GitProvider, token: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let mut req = client
        .get(provider.whoami_url())
        .header("User-Agent", "dozer");
    req = match provider {
        GitProvider::GitHub => req.header("Authorization", format!("Bearer {token}")),
        GitProvider::GitLab => req.header("PRIVATE-TOKEN", token),
        GitProvider::Gitee => req.query(&[("access_token", token)]),
    };
    let resp = req.send().await.map_err(|e| format!("网络请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err("令牌无效或已过期".to_string());
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;
    extract_username(&body)
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteRepo {
    pub full_name: String,
    pub clone_url: String,
}

fn list_repos_url(provider: GitProvider) -> &'static str {
    match provider {
        GitProvider::GitHub => {
            "https://api.github.com/user/repos?per_page=100&sort=updated&affiliation=owner"
        }
        GitProvider::GitLab => {
            "https://gitlab.com/api/v4/projects?membership=true&owned=true&per_page=100&order_by=last_activity_at"
        }
        GitProvider::Gitee => "https://gitee.com/api/v5/user/repos?per_page=100&sort=updated",
    }
}

/// 三家仓库列表接口返回数组,字段名不同(设计文档已注明:实现前建议用
/// 真实账户核实一次响应形状,这里的字段名如与实际不符以真实响应为准
/// 调整)。单个条目缺期望字段直接跳过,不让整个列表因为一条脏数据报错。
pub(crate) fn parse_repo_list(
    provider: GitProvider,
    json: &str,
) -> Result<Vec<RemoteRepo>, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("解析响应失败: {e}"))?;
    let items = value
        .as_array()
        .ok_or_else(|| "响应格式不是数组".to_string())?;
    let (name_field, url_field) = match provider {
        GitProvider::GitHub => ("full_name", "clone_url"),
        GitProvider::GitLab => ("path_with_namespace", "http_url_to_repo"),
        GitProvider::Gitee => ("full_name", "html_url"),
    };
    let repos = items
        .iter()
        .filter_map(|item| {
            let full_name = item.get(name_field)?.as_str()?.to_string();
            let clone_url = item.get(url_field)?.as_str()?.to_string();
            Some(RemoteRepo {
                full_name,
                clone_url,
            })
        })
        .collect();
    Ok(repos)
}

/// 拉取 `provider` 账户名下的个人仓库(不含组织/团队,只取第一页,见
/// `docs/superpowers/specs/2026-09-18-project-create-remote-repo-picker-
/// design.md`「非目标」)。鉴权头写法同 `validate_token`。
pub async fn list_repos(provider: GitProvider, token: &str) -> Result<Vec<RemoteRepo>, String> {
    let client = reqwest::Client::new();
    let mut req = client
        .get(list_repos_url(provider))
        .header("User-Agent", "dozer");
    req = match provider {
        GitProvider::GitHub => req.header("Authorization", format!("Bearer {token}")),
        GitProvider::GitLab => req.header("PRIVATE-TOKEN", token),
        GitProvider::Gitee => req.query(&[("access_token", token)]),
    };
    let resp = req.send().await.map_err(|e| format!("网络请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err("仓库列表加载失败: 令牌可能已失效,请重新连接账户".to_string());
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;
    parse_repo_list(provider, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_repo_list_github_shape() {
        let json = r#"[
            {"full_name":"abc/python_project","clone_url":"https://github.com/abc/python_project.git"},
            {"full_name":"abc/rust_project","clone_url":"https://github.com/abc/rust_project.git"}
        ]"#;
        let repos = parse_repo_list(GitProvider::GitHub, json).unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].full_name, "abc/python_project");
        assert_eq!(
            repos[0].clone_url,
            "https://github.com/abc/python_project.git"
        );
    }

    #[test]
    fn parse_repo_list_gitlab_shape() {
        let json = r#"[
            {"path_with_namespace":"abc/site","http_url_to_repo":"https://gitlab.com/abc/site.git"}
        ]"#;
        let repos = parse_repo_list(GitProvider::GitLab, json).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].full_name, "abc/site");
        assert_eq!(repos[0].clone_url, "https://gitlab.com/abc/site.git");
    }

    #[test]
    fn parse_repo_list_gitee_shape() {
        let json = r#"[
            {"full_name":"abc/demo","html_url":"https://gitee.com/abc/demo"}
        ]"#;
        let repos = parse_repo_list(GitProvider::Gitee, json).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].clone_url, "https://gitee.com/abc/demo");
    }

    #[test]
    fn parse_repo_list_empty_array_is_empty_vec() {
        assert_eq!(parse_repo_list(GitProvider::GitHub, "[]").unwrap(), vec![]);
    }

    #[test]
    fn parse_repo_list_skips_items_missing_expected_fields() {
        let json = r#"[{"full_name":"a/b"}, {"full_name":"c/d","clone_url":"https://x/c/d.git"}]"#;
        let repos = parse_repo_list(GitProvider::GitHub, json).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].full_name, "c/d");
    }

    #[test]
    fn parse_repo_list_rejects_non_array_json() {
        assert!(parse_repo_list(GitProvider::GitHub, r#"{"login":"x"}"#).is_err());
    }

    #[test]
    fn parse_repo_list_rejects_invalid_json() {
        assert!(parse_repo_list(GitProvider::GitHub, "not json").is_err());
    }

    #[test]
    fn extract_username_reads_login_field() {
        assert_eq!(
            extract_username(r#"{"login":"octocat","id":1}"#).unwrap(),
            "octocat"
        );
    }

    #[test]
    fn extract_username_rejects_missing_login() {
        assert!(extract_username(r#"{"id":1}"#).is_err());
    }

    #[test]
    fn extract_username_rejects_invalid_json() {
        assert!(extract_username("not json").is_err());
    }

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
