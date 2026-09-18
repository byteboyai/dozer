# Git 账户设置 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给顶栏齿轮"设置"弹窗加一个"Git 账户"区块,支持用 PAT(Personal Access Token)连接/断开 GitHub/GitLab/Gitee 三家账户,同时把整个设置弹窗从普通 iced 叠层迁到独立原生窗口(修掉"被预览 webview 遮住"的现状缺陷)。

**Architecture:** 新增 `crate::git_accounts` 模块(crate 根,与 `open_projects.rs` 同级)承载"账户"这个领域概念——`GitProvider` 三元枚举、本地非敏感元数据(`git_accounts.json`,同 `open_projects.json` 的读写方式)、Keychain 里的 token 存取(同 `extensions::ssh.rs` 的 `keyring` 用法)、以及调用各家 whoami 接口校验 token 的 HTTP 逻辑(新引入 `reqwest` 依赖)。现有 `crates/dozer-app/src/settings.rs`(crate 根,只有主题选择)整体挪进 `extensions::settings`,升级成完整的 `State`/`Message`/`update` 扩展模块(结构对照 `extensions::file_history`/`extensions::project_create`),渲染宿主是新的独立窗口 `platform::settings_overlay::SettingsOverlay`——技术上是 `FileHistoryOverlay`(`FocusTracker` 失焦即关闭)与 `ProjectCreateOverlay`(IME + 原生右键菜单挂靠,PAT 输入框需要)两者的混合体。

**Tech Stack:** iced 0.14、winit 0.30、`keyring = "4.1.6"`(已有依赖)、新增 `reqwest`(异步 HTTP 客户端,`rustls-tls` 保持与 `sqlx`/`mongodb`/`russh-sftp` 一致不引入 OpenSSL)、`byteui::form::input_text`(PAT 输入框复用 `secure=true`)。

**Spec:** `docs/superpowers/specs/2026-09-18-git-account-settings-design.md`

## Global Constraints

- 仅官方 SaaS 域名(`api.github.com`/`gitlab.com`/`gitee.com`),不支持自定义 base URL(spec「非目标」)。
- 每家最多一个已连接账户,再次连接直接覆盖旧 token(spec「非目标」)。
- 鉴权方式固定 PAT 粘贴,不做 OAuth 设备码流程(spec「目标」2)。
- token 只进 Keychain,不进任何本地明文文件;本地文件只存用户名等非敏感展示信息(spec「数据模型」)。
- 设置弹窗迁到独立原生窗口后接入失焦即关闭(`FocusTracker`),因为本表单没有嵌套的 rfd 原生面板会触发意外失焦——这与"创建项目"对话框的"不接失焦关闭"刻意不同,不要混淆两者(spec「渲染宿主」)。
- 不做自建/企业自托管实例支持、不做多账户、不做 OAuth——这些都是明确非目标,发现工作量超出这三条时先停下确认,不要自行扩大范围。

---

### Task 1: 新增 `reqwest` 依赖

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`

**Interfaces:**
- Consumes: 无
- Produces: `reqwest::Client`——Task 3 的 `validate_token` 会用到。

- [ ] **Step 1: 加依赖**

`crates/dozer-app/Cargo.toml` 的 `[dependencies]` 段,在 `keyring = "4.1.6"` 之后插入(按现有依赖块无严格字母序,跟在语义相关的行后面即可):

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
```

- [ ] **Step 2: 编译确认依赖解析成功**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 干净编译通过(此时还没有任何代码使用 `reqwest`,只验证依赖本身能解析、不产生版本冲突)。`Cargo.lock` 会被更新,属于预期改动。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/Cargo.toml Cargo.lock
git commit -m "feat(app): 新增 reqwest 依赖(Git 账户 whoami 校验用)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: `git_accounts` 模块——账户元数据与 Keychain 存取

**Files:**
- Create: `crates/dozer-app/src/git_accounts.rs`
- Modify: `crates/dozer-app/src/main.rs`(注册模块)

**Interfaces:**
- Consumes: `dozer_core::paths::config_dir()`(既有)、`keyring::Entry`(既有依赖)
- Produces: `GitProvider`(`GitHub`/`GitLab`/`Gitee`,含 `as_key`/`display_name`/`whoami_url`/`token_creation_url`)、`ConnectedAccount { username: String }`、`GitAccountsState { github, gitlab, gitee: Option<ConnectedAccount> }`(含 `get`/`set`)、`load()`/`save()`、`get_token`/`set_token`/`delete_token`——Task 3(HTTP 校验)、Task 4/5(`extensions::settings` 的 reducer)都要用这些。

- [ ] **Step 1: 写失败测试**

新建 `crates/dozer-app/src/git_accounts.rs`:

```rust
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
```

- [ ] **Step 2: 注册模块**

`crates/dozer-app/src/main.rs`,在 `mod open_projects;`(约 15 行)附近按字母序插入:

```rust
mod git_accounts;
```

- [ ] **Step 3: 跑测试**

Run: `cargo test -p dozer-app --lib git_accounts`
Expected: `git_provider_keys_and_urls_are_distinct`/`load_from_missing_file_returns_default`/`save_then_load_round_trips`/`load_from_corrupt_file_returns_default`/`get_and_set_route_to_matching_field` 全部 PASS。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/git_accounts.rs crates/dozer-app/src/main.rs
git commit -m "feat(app): git_accounts 模块——账户元数据与 Keychain 存取

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: `git_accounts` 模块——whoami 校验

**Files:**
- Modify: `crates/dozer-app/src/git_accounts.rs`(追加)

**Interfaces:**
- Consumes: `reqwest::Client`(Task 1)
- Produces: `extract_username(json: &str) -> Result<String, String>`(纯函数)、`async fn validate_token(provider: GitProvider, token: &str) -> Result<String, String>`——Task 5 的 `ConnectSubmit` 处理会调用后者。

- [ ] **Step 1: 写失败测试(纯函数部分)**

在 `crates/dozer-app/src/git_accounts.rs` 的 `#[cfg(test)] mod tests` 里追加:

```rust
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
```

- [ ] **Step 2: 跑测试确认因函数不存在而编译失败**

Run: `cargo test -p dozer-app --lib git_accounts`
Expected: 编译错误 `cannot find function 'extract_username'`。

- [ ] **Step 3: 实现——追加到 `crates/dozer-app/src/git_accounts.rs`,紧跟 `delete_token` 之后**

```rust
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
    let body = resp.text().await.map_err(|e| format!("读取响应失败: {e}"))?;
    extract_username(&body)
}
```

- [ ] **Step 4: 跑测试**

Run: `cargo test -p dozer-app --lib git_accounts`
Expected: 新增的 3 个 `extract_username_*` 测试 PASS,Task 2 的既有测试仍 PASS。`validate_token` 本身(真实网络调用)不做自动化测试,留给 Task 10 的人工验证清单。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/git_accounts.rs
git commit -m "feat(app): git_accounts whoami 校验(extract_username/validate_token)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 4: `settings.rs` 迁入 `extensions::settings`——State/Message/非提交类 reducer

**Files:**
- Create: `crates/dozer-app/src/extensions/settings.rs`
- Delete: `crates/dozer-app/src/settings.rs`
- Modify: `crates/dozer-app/src/main.rs`(删掉旧模块声明)
- Modify: `crates/dozer-app/src/extensions.rs`(注册新模块)

**Interfaces:**
- Consumes: `git_accounts::{GitProvider, GitAccountsState, ConnectedAccount}`(Task 2)、`byteui::theme::color::{ColorScheme, set_scheme, persist_scheme, current_scheme}`(既有)
- Produces: `settings::ConnectState`(`NotConnected`/`Editing`/`Connected`)、`settings::State`(`github`/`gitlab`/`gitee: ConnectState`,含 `load()`)、`settings::Message` 枚举、`settings::update(state: &mut Option<State>, msg: Message, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`——Task 5 补提交类分支,Task 6 的视图函数读 `State`,Task 9 的 `App` 持有 `Option<State>` 并调用这个 `update`。

- [ ] **Step 1: 写失败测试**

新建 `crates/dozer-app/src/extensions/settings.rs`:

```rust
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
```

- [ ] **Step 2: 删旧文件、改模块声明**

删除 `crates/dozer-app/src/settings.rs`。

`crates/dozer-app/src/main.rs`:删掉 `mod settings;` 这一行。

`crates/dozer-app/src/extensions.rs`,按字母序在 `pub mod search;` 之前插入:

```rust
pub mod settings;
```

- [ ] **Step 3: 跑测试确认因残留引用而编译失败**

Run: `cargo build -p dozer-app 2>&1 | grep -E "^error" | head -60`
Expected: 报 `crate::settings` 找不到(`app/view.rs`/`app/message.rs`/`app/update.rs`/`chrome/topbar.rs` 里还引用着旧路径)——这些在 Task 9 才会全部改完,这一步先确认新模块自身(`extensions::settings`)没有语法/类型错误,残留的旧引用错误是预期中的、留给 Task 9 处理。

Run: `cargo test -p dozer-app --lib extensions::settings 2>&1 | tail -60`
Expected: 若整个 crate 因为 Task 9 还没做而无法编译,这一步的测试也跑不起来——**这是预期的循环依赖**(同 2026-09-18-new-project-creation 计划里 Task 7/8 之间的先例),继续往下做 Task 5/6/7/8/9,最后一并编译验证;若你的执行环境要求每个任务独立可编译,则把 Task 9 的"删除旧引用、改 `Message`/`App` 字段"这部分提前并入本任务的 Step 2(即 Task 4 直接把 Task 9 的“断开旧引用”那部分也做了),两种做法都可以,以实际执行时的编译反馈为准。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/settings.rs crates/dozer-app/src/main.rs crates/dozer-app/src/extensions.rs
git rm crates/dozer-app/src/settings.rs
git commit -m "feat(app): settings 迁入 extensions,补 Git 账户 State/Message 骨架

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 5: `extensions::settings`——`ConnectSubmit` 异步提交逻辑

**Files:**
- Modify: `crates/dozer-app/src/extensions/settings.rs`(补 `update` 的 `ConnectSubmit` 分支)

**Interfaces:**
- Consumes: `git_accounts::validate_token`(Task 3)、`git_accounts::{set_token, load, save}`(Task 2)
- Produces: 完整的 `update()`。

- [ ] **Step 1: 补 `update` 函数体,替换 Task 4 里的占位 `let _ = (handle, emit, msg);`**

```rust
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
    handle.spawn(async move {
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
}
```

- [ ] **Step 2: 编译检查**

Run: `cargo build -p dozer-app 2>&1 | grep -E "^error" | head -60`
Expected: 仍会报 Task 9 才处理的旧引用错误(`crate::settings` 找不到);确认没有新增的、与本任务改动直接相关的错误(比如 `git_accounts::set_token(provider, &token)?` 在 `.and_then` 闭包里的 `?` 用法——闭包返回类型是 `Result<String, String>`,`set_token` 返回 `Result<(), String>`,错误类型一致,`?` 可以正常提前返回)。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/src/extensions/settings.rs
git commit -m "feat(app): settings ConnectSubmit 异步提交逻辑

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 6: `extensions::settings`——视图

**Files:**
- Modify: `crates/dozer-app/src/extensions/settings.rs`(追加视图函数)

**Interfaces:**
- Consumes: `byteui::form::input_text::view_on_bg`、`crate::dialog::{card_style, actions, action_button_style}`、`crate::chrome::menu::item_row_fill`(既有,原 `scheme_row` 已用)
- Produces: `settings::settings_card(state: &State) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>`——Task 7 的 `SettingsOverlay::redraw`/`handle_input` 会 `.map(Message::Settings)` 这个函数的返回值。

- [ ] **Step 1: 写主题行 + provider 行视图函数**

在 `crates/dozer-app/src/extensions/settings.rs` 顶部 `use` 区块补充:

```rust
use iced_widget::core::{Alignment, Element, Length};
use iced_widget::{Space, button, column, container, row, text};
```

在文件末尾(`#[cfg(test)]` 之前)追加:

```rust
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

fn provider_row(provider: GitProvider, state: &ConnectState) -> Element<'_, Message> {
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
            row![title, status, Space::new().width(Length::Fill), disconnect_btn]
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
            let error_row: Element<'_, Message> = if let Some(e) = error {
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

pub fn settings_card(state: &State) -> Element<'_, Message> {
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
```

（旧 `crates/dozer-app/src/settings.rs` 里的 `settings_modal` 函数已被这个 `settings_card` 取代,不再需要外层"全窗居中容器 + `width(crate::dialog::width(window_width))`"那层——独立窗口本身已经是量好的画布,同 `file_history_card`/`project_create_card` 当初从"在更大画布里收缩/居中适配"改成"填满调用方给的画布"的既有调整。）

- [ ] **Step 2: 编译检查**

Run: `cargo build -p dozer-app 2>&1 | grep -E "^error" | grep -v "extensions::settings\|crate::settings" | head -40`
Expected: 除了 Task 9 才处理的 `crate::settings` 残留引用外,`extensions/settings.rs` 自身不再新增编译错误。若报字段/函数签名不对(比如 `byteui::theme::font::subtitle()`/`item_row_fill` 的具体参数),以 `cargo build` 报错为准调整,不要凭空猜新名字。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/src/extensions/settings.rs
git commit -m "feat(app): settings_card 视图(主题 + Git 账户)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 7: `SettingsOverlay` 独立窗口宿主

**Files:**
- Create: `crates/dozer-app/src/platform/settings_overlay.rs`
- Modify: `crates/dozer-app/src/platform/mod.rs`

**Interfaces:**
- Consumes: `OverlayGpu::{open, reconfigure}`(`platform::overlay_gpu`,既有)、`FocusTracker`(`platform::overlay_focus`,既有)、`open_child_window`/`centered_overlay_bounds`(`platform::overlay_window`,既有)、`settings::{State, Message, settings_card}`(Task 4-6)
- Produces: `SettingsOverlay::{open, redraw, handle_input, handle_focus, reposition, window_id, request_redraw}`,`sync_action(open: bool, overlay_present: bool) -> SyncAction`——Task 8 的 `window_events.rs` 会调用这些。**接入 `FocusTracker`**(失焦即关闭,与"创建项目"对话框刻意不同)。

- [ ] **Step 1: 注册模块**

`crates/dozer-app/src/platform/mod.rs`,按字母序在 `pub mod window;` 之前插入:

```rust
pub mod settings_overlay;
```

- [ ] **Step 2: 实现——新建 `crates/dozer-app/src/platform/settings_overlay.rs`**

```rust
//! 设置弹窗(主题 + Git 账户连接)的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-18-git-account-settings-design.md`。
//! 结构上是 `FileHistoryOverlay`(`FocusTracker` 失焦即关闭)与
//! `ProjectCreateOverlay`(IME + 原生右键菜单挂靠,PAT 输入框需要)两者的
//! 混合:接入失焦关闭是因为本表单没有嵌套的 rfd 原生面板会触发意外
//! 失焦(跟"创建项目"对话框的场景不同),但仍需要 IME(账户用户名可能是
//! 中文相关字符)和原生右键菜单(PAT 输入框的粘贴)。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::conversion;
use iced_winit::core::{Event, mouse};
use iced_winit::runtime::user_interface::UserInterface;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::settings;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

/// 卡片逻辑尺寸——固定值,不随主窗口宽高缩放:设置表单内容量有限,不需要
/// 像 file_history/project_create 那样按主窗口比例伸缩。
fn card_logical_size() -> LogicalSize<f32> {
    LogicalSize::new(480.0, 560.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncAction {
    Open,
    Close,
    Noop,
}

pub(crate) fn sync_action(open: bool, overlay_present: bool) -> SyncAction {
    match (open, overlay_present) {
        (true, false) => SyncAction::Open,
        (false, true) => SyncAction::Close,
        (true, true) | (false, false) => SyncAction::Noop,
    }
}

pub(crate) struct SettingsOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    main_window: Arc<Window>,
}

impl SettingsOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        _main_window_size: LogicalSize<f32>,
        el: &ActiveEventLoop,
    ) -> SettingsOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size();
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );
        let window = open_child_window(main_window, pos, size, "settings", el);
        window.set_ime_allowed(true);
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);

        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);

        SettingsOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            main_window: main_window.clone(),
        }
    }

    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus.handle_focus(focused)
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
        _window_width: f32,
        _window_height: f32,
    ) {
        let card_logical = card_logical_size();
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(state) = app.settings.as_ref() else {
            return;
        };
        let mut interface = UserInterface::build(
            settings::settings_card(state).map(Message::Settings),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let _ = interface.update(
            &[],
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.gpu.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            self.cursor,
        );
        self.gpu.cache = interface.into_cache();

        let Ok(frame) = self.gpu.surface.get_current_texture() else {
            self.window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu
            .renderer
            .present(None, frame.texture.format(), &view, &self.gpu.viewport);
        frame.present();
    }

    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        if let WindowEvent::KeyboardInput {
            event: key_event,
            is_synthetic: false,
            ..
        } = event
            && key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            return vec![Message::Settings(settings::Message::Close)];
        }
        if let WindowEvent::ModifiersChanged(new_modifiers) = event {
            self.modifiers = new_modifiers.state();
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor = mouse::Cursor::Available(conversion::cursor_position(
                *position,
                self.gpu.viewport.scale_factor(),
            ));
        }
        let Some(iced_event) = conversion::window_event(
            event.clone(),
            self.gpu.viewport.scale_factor(),
            self.modifiers,
        ) else {
            return Vec::new();
        };
        let events: [Event; 1] = [iced_event];
        let Some(state) = app.settings.as_ref() else {
            return Vec::new();
        };
        let mut interface = UserInterface::build(
            settings::settings_card(state).map(Message::Settings),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut messages = Vec::new();
        let _ = interface.update(
            &events,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut messages,
        );
        self.gpu.cache = interface.into_cache();
        self.window.request_redraw();
        messages
    }
}

impl Drop for SettingsOverlay {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&self.main_window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_action_opens_when_open_and_no_overlay() {
        assert_eq!(sync_action(true, false), SyncAction::Open);
    }

    #[test]
    fn sync_action_closes_when_closed_but_overlay_present() {
        assert_eq!(sync_action(false, true), SyncAction::Close);
    }

    #[test]
    fn sync_action_noop_when_states_already_match() {
        assert_eq!(sync_action(true, true), SyncAction::Noop);
        assert_eq!(sync_action(false, false), SyncAction::Noop);
    }
}
```

- [ ] **Step 3: 编译检查**

Run: `cargo build -p dozer-app 2>&1 | grep -E "^error" | grep -v "crate::settings\b" | head -60`
Expected: 报 `App` 没有 `settings` 字段(Task 9 才加)、`Message::Settings` 变体不存在(同上)——这些是预期的;确认本文件自身没有其它编译错误。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/platform/settings_overlay.rs crates/dozer-app/src/platform/mod.rs
git commit -m "feat(app): SettingsOverlay 独立窗口宿主

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 8: `Runner`/`window_events.rs` 独立窗口生命周期接线

**Files:**
- Modify: `crates/dozer-app/src/platform/window_events.rs`

**Interfaces:**
- Consumes: `SettingsOverlay::{open, redraw, handle_input, handle_focus, reposition, window_id, request_redraw}`(Task 7)、`app.settings.is_some()`(Task 9)
- Produces: 设置弹窗能开合、跟随主窗口移动/resize、失焦/Esc/关闭按钮均可关闭、与 search/file_history/project_create 互斥。

- [ ] **Step 1: `use` 声明 + `Ready` 结构体加字段**

顶部 `use crate::platform::project_create_overlay;` 旁边加:

```rust
use crate::platform::settings_overlay;
```

`Ready` 枚举变体,在 `project_create_overlay: Option<project_create_overlay::ProjectCreateOverlay>,` 之后追加:

```rust
        /// 同 `search_overlay`/`file_history_overlay`/`project_create_
        /// overlay`,设置弹窗的独立窗口宿主。生命周期由
        /// `sync_settings_overlay` 按 `app.settings.is_some()` 单向驱动
        /// 开/关,与其余三类互斥。
        settings_overlay: Option<settings_overlay::SettingsOverlay>,
```

`resumed()` 里 `Ready` 构造处(`project_create_overlay: None,` 之后)追加:

```rust
                settings_overlay: None,
```

- [ ] **Step 2: `OverlayKind` 加变体**

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum OverlayKind {
    Search,
    FileHistory,
    ProjectCreate,
    Settings,
}
```

- [ ] **Step 3: `close_other_overlays` 加分支**

```rust
    fn close_other_overlays(&mut self, keep: OverlayKind) {
        let Self::Ready {
            search_overlay,
            file_history_overlay,
            project_create_overlay,
            settings_overlay,
            ..
        } = self
        else {
            return;
        };
        if keep != OverlayKind::Search {
            *search_overlay = None;
        }
        if keep != OverlayKind::FileHistory {
            *file_history_overlay = None;
        }
        if keep != OverlayKind::ProjectCreate {
            *project_create_overlay = None;
        }
        if keep != OverlayKind::Settings {
            *settings_overlay = None;
        }
    }
```

- [ ] **Step 4: 新增 `sync_settings_overlay`,克隆自 `sync_file_history_overlay`(带 `Focused` 场景的那一版,不是 `sync_project_create_overlay`)**

```rust
    /// 同 `sync_file_history_overlay`,按 `app.settings.is_some()` 开/关
    /// settings overlay 窗口。
    fn sync_settings_overlay(&mut self, el: &winit::event_loop::ActiveEventLoop) {
        let action = {
            let Self::Ready {
                app,
                settings_overlay,
                ..
            } = self
            else {
                return;
            };
            settings_overlay::sync_action(app.settings.is_some(), settings_overlay.is_some())
        };
        match action {
            settings_overlay::SyncAction::Open => {
                self.close_other_overlays(OverlayKind::Settings);
                let Self::Ready {
                    window,
                    instance,
                    adapter,
                    device,
                    queue,
                    app,
                    settings_overlay,
                    ..
                } = self
                else {
                    return;
                };
                let main_window_size =
                    winit::dpi::LogicalSize::new(app.window_size.0, app.window_size.1);
                *settings_overlay = Some(settings_overlay::SettingsOverlay::open(
                    window,
                    adapter,
                    device,
                    queue,
                    instance,
                    main_window_size,
                    el,
                ));
            }
            settings_overlay::SyncAction::Close => {
                let Self::Ready {
                    settings_overlay, ..
                } = self
                else {
                    return;
                };
                *settings_overlay = None;
            }
            settings_overlay::SyncAction::Noop => {}
        }
        let Self::Ready {
            settings_overlay, ..
        } = self
        else {
            return;
        };
        if let Some(overlay) = settings_overlay {
            overlay.request_redraw();
        }
    }
```

- [ ] **Step 5: `window_event` 新增按 `WindowId` 分发的分支(模板是 file_history 那版,带 `Focused` 处理)**

紧跟 file-history overlay 分支之后插入:

```rust
        if let Self::Ready {
            app,
            settings_overlay,
            ..
        } = self
            && let Some(overlay) = settings_overlay
            && window_id == overlay.window_id()
        {
            if matches!(event, WindowEvent::RedrawRequested) {
                overlay.redraw(app);
            } else if matches!(event, WindowEvent::CloseRequested) {
                self.dispatch(Message::Settings(extensions::settings::Message::Close));
            } else if let WindowEvent::Focused(focused) = event {
                if overlay.handle_focus(focused) {
                    self.dispatch(Message::Settings(extensions::settings::Message::Close));
                }
            } else {
                for message in overlay.handle_input(app, &event) {
                    self.dispatch(message);
                }
            }
            self.sync_settings_overlay(event_loop);
            return;
        }
```

- [ ] **Step 6: `Resized`/`CloseRequested` 处理加对应调用**

`Resized` 分支(`project_create_overlay` 的 `reposition` 调用之后)追加:

```rust
                    if let Some(overlay) = settings_overlay {
                        overlay.reposition(
                            device,
                            window
                                .outer_position()
                                .unwrap_or(winit::dpi::PhysicalPosition::new(0, 0)),
                            new_size,
                            window.scale_factor(),
                            app.window_size.0,
                            app.window_size.1,
                        );
                    }
```

`CloseRequested` 分支(`*project_create_overlay = None;` 之后)追加:

```rust
                    *settings_overlay = None; // 图干净,Drop 本身就会释放。
```

这两处所在的 `let Self::Ready { .. } = self else { return; };` 大解构字段列表都要把 `settings_overlay` 加进去。

- [ ] **Step 7: 两处"每帧结尾同步调用"追加**

`user_event` 结尾和 `window_event` 结尾(`self.sync_project_create_overlay(event_loop);` 之后)都追加:

```rust
        self.sync_settings_overlay(event_loop);
```

- [ ] **Step 8: 编译 + 测试**

Run: `cargo build -p dozer-app 2>&1 | grep -E "^error" | grep -v "crate::settings\b\|App.*settings\|Message::Settings" | head -60`
Expected: 除了 Task 9 才处理的 `App::settings`/`Message::Settings` 缺失外,`window_events.rs` 自身不再新增编译错误。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/platform/window_events.rs
git commit -m "feat(app): settings 独立窗口接入 Runner 生命周期

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 9: `App`/`Message` 级接线 + 清理旧引用

**Files:**
- Modify: `crates/dozer-app/src/app/app.rs`
- Modify: `crates/dozer-app/src/app/message.rs`
- Modify: `crates/dozer-app/src/app/update.rs`
- Modify: `crates/dozer-app/src/app/view.rs`

**Interfaces:**
- Consumes: `settings::{State, Message, update}`(Task 4-5)
- Produces: `App.settings: Option<settings::State>`;`Message::Settings(settings::Message)`;`Message::SettingsOpen` 保留但改指向新字段;整个 crate 恢复可编译状态。

- [ ] **Step 1: `App` 结构体——替换旧字段**

`crates/dozer-app/src/app/app.rs`:删掉 `pub(crate) settings_modal_open: bool,`(约 337 行)及其构造初始化 `settings_modal_open: false,`(约 745 行),在 `project_create: Option<project_create::State>,` 之后插入:

```rust
    /// 设置弹窗状态(主题 + Git 账户)——见 `extensions::settings::State`。
    /// `None` 表示当前没开。
    pub(crate) settings: Option<settings::State>,
```

构造处 `project_create: None,` 之后插入:

```rust
            settings: None,
```

顶部 `use crate::extensions::project_create;` 附近加:

```rust
use crate::extensions::settings;
```

- [ ] **Step 2: `Message` 枚举——替换旧变体**

`crates/dozer-app/src/app/message.rs`:模块级 `use crate::extensions::{...}` 列表按字母序加 `settings`:

```rust
use crate::extensions::{
    browser, conversations, database, file_history, files, footbar, git_log, project,
    project_create, search, settings, ssh, todo, usage,
};
```

删掉 `SettingsClose`/`SettingsThemeSelected` 两个变体定义,在 `SettingsOpen,` 之后插入:

```rust
    /// 设置弹窗内部消息,转发给 `extensions::settings::update`。
    Settings(settings::Message),
```

`SettingsOpen` 自身的文档注释更新为(反映现在开的是独立窗口,不是内嵌叠层):

```rust
    /// 顶栏设置齿轮:打开设置弹窗(独立原生窗口,主题 + Git 账户)。
    SettingsOpen,
```

- [ ] **Step 3: `app/update.rs`——改 `SettingsOpen` 处理,删旧分支,加新分发**

把:
```rust
            Message::SettingsOpen => {
                self.settings_modal_open = true;
            }
            Message::SettingsClose => {
                self.settings_modal_open = false;
            }
            Message::SettingsThemeSelected(scheme) => {
                byteui::theme::color::set_scheme(scheme);
                byteui::theme::color::persist_scheme(&crate::theme::color_theme_path());
            }
```
改成:
```rust
            Message::SettingsOpen => {
                self.settings = Some(settings::State::load());
            }
            Message::Settings(msg) => {
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Settings(m));
                };
                settings::update(&mut self.settings, msg, &handle, emit);
            }
```

- [ ] **Step 4: `app/view.rs`——删掉旧的内嵌叠层分支**

把:
```rust
    pub fn view(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let content = self.view_inner();
        // 设置弹窗是 App 级浮层(见 `settings_modal_open` 字段文档),必须能
        // 盖在首页/空工作区/项目工作区三种 `view_inner` 分支之上——所以放在
        // 最外层统一叠加,而不是塞进 `view_inner` 内部某个分支。
        if self.settings_modal_open {
            stack![
                content,
                crate::dialog::scrim(Message::SettingsClose),
                settings::settings_modal(self.window_size.0)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            content
        }
    }
```
改成:
```rust
    pub fn view(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
        // 设置弹窗已迁到独立原生窗口(`platform::settings_overlay`),不再
        // 是主窗口内容树的一部分,同 search/file_history/project_create。
        self.view_inner()
    }
```

删掉本文件顶部现在已死的 `use crate::settings;`(若删除后 `stack!`/`Length` 等导入因这是唯一用途而变成未使用,一并清理;`view_inner()` 内部若还用得到 `Length`/`stack!` 则保留)。

- [ ] **Step 5: 全量编译**

Run: `cargo build -p dozer-app 2>&1 | tail -100`
Expected: 干净编译通过——此时 Task 4/5/6/7/8 里所有"预期中的残留引用错误"都应该消失。若还有报错,大概率是某处遗漏的 `Message::SettingsClose`/`Message::SettingsThemeSelected`/`self.settings_modal_open`/`crate::settings::` 引用,按报错定位补齐。

- [ ] **Step 6: 全量测试**

Run: `cargo test -p dozer-app --lib`
Expected: 全部既有测试 + 本计划新增的所有测试(Task 2-8)PASS,零失败。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/app/app.rs crates/dozer-app/src/app/message.rs crates/dozer-app/src/app/update.rs crates/dozer-app/src/app/view.rs
git commit -m "feat(app): App/Message 接入 settings 独立窗口,删旧内嵌叠层

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 10: 全量检查 + 人工验证清单

**Files:** 无代码改动(除非检查发现问题需要回头小修)

**Interfaces:** 无

- [ ] **Step 1: 全量构建/测试/静态检查**

```bash
cargo build
cargo test -p dozer-app
cargo clippy --all-targets
cargo fmt --check
```

Expected: 全部通过;`cargo fmt --check` 报格式问题就跑 `cargo fmt` 后单独提交一次格式化 commit。

- [ ] **Step 2: `cargo run -p dozer-app` 人工验证清单**

- [ ] 顶栏齿轮"设置"按钮打开的是一扇独立窗口(不再是叠在主窗口内容之上的半透明遮罩),标题栏区域正常。
- [ ] 主题切换(深色/浅色)功能与迁移前完全一致,选中即生效并跨重启记住。
- [ ] GitHub/GitLab/Gitee 三行初始均显示"未连接"。
- [ ] 点某一家的"连接",展开 PAT 输入框 + "没有 PAT?点此生成"提示;点提示能跳转到该服务商正确的 token 创建页(用系统默认浏览器打开)。
- [ ] 粘贴一个真实有效的 PAT 并点"确认",按钮态变"校验中…",成功后该行变成"已连接: `<真实用户名>`"。
- [ ] 粘贴一个无效/过期的 token,内联展示"令牌无效或已过期",表单不关闭,可修改后重试。
- [ ] 断网状态下尝试连接,内联展示"网络请求失败"文案,不崩溃。
- [ ] 点"断开连接",该行回到"未连接";重新打开设置弹窗确认状态没有残留。
- [ ] 重启 app 后重新打开设置弹窗,之前成功连接过的账户仍显示"已连接: `<用户名>`"(不需要重新输入 token)。
- [ ] 在某个项目 tab 内打开预览 webview(比如预览一个 markdown 文件),此时打开设置弹窗——弹窗完整可见、不被 webview 遮住(这是本次要修的现状 bug)。
- [ ] 设置弹窗打开时若 search/file_history/"创建项目"任一个已经开着,会被自动关闭;反之亦然。
- [ ] 设置弹窗点击窗口外部区域(切到主窗口或其它 app)会自动关闭(失焦即关闭,与"创建项目"对话框的行为刻意不同)。
- [ ] Esc 键与"关闭"按钮均可关闭设置弹窗。
- [ ] PAT 输入框支持右键粘贴(原生右键菜单),且遮罩显示(看不到明文 token)。

- [ ] **Step 3: 若人工验证发现问题,记录并修复**

对每一条失败项:定位对应任务的代码,修复后重新跑 Step 1 的全量检查,再回到 Step 2 从头过一遍清单。

- [ ] **Step 4: 最终提交(若 Step 3 有修复)**

```bash
git add -A
git commit -m "fix(app): Git 账户设置人工验证发现的问题修复

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```
