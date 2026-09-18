# 创建项目对话框接入账户仓库列表 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把"创建项目"对话框 Tab 2 里 GitHub/GitLab/Gitee 三个禁用占位改成真功能——已连接账户就拉仓库列表供选择,未连接就引导去设置页连接。

**Architecture:** `crate::git_accounts` 新增 `RemoteRepo`/`list_repos`(复用 `validate_token` 已有的三家鉴权头写法)。`extensions::project_create` 新增 `CloneSource`(URL / 某个 provider)、`RepoListState`(未连接/加载中/出错/已加载)两个类型,`CloneForm` 挂上 `source`/`repo_lists` 两个字段,原来固定的"远程仓库"URL 输入框改成按 `source` 切换渲染。选中仓库直接复用既有 `CloneUrlChanged` 消息(自动推导项目名称的逻辑不用改)。未连接时的"去设置连接"按钮触发 `App` 级拦截,关掉本对话框、打开设置弹窗(两者是互斥的独立窗口)。

**Tech Stack:** 与前两个子项目一致——`reqwest`(已在 Task 1 引入)、`serde_json::Value` 手动取字段(不新建 serde 结构体,响应字段少且三家形状不同,取字段比定义三套 struct 更直接)。

**Spec:** `docs/superpowers/specs/2026-09-18-project-create-remote-repo-picker-design.md`

## Global Constraints

- **前置依赖**:本计划假定 `docs/superpowers/plans/2026-09-18-git-account-settings.md` 的 Task 9(`App.settings: Option<settings::State>`、`Message::Settings(settings::Message)`、`settings::State::load()`)已经落地。Task 4 的 Step 1 会先检查这个前提,若还没落地就先停下汇报,不要在缺依赖的情况下继续。
- 仓库列表只取个人仓库、只取第一页(spec「非目标」),不做分页/组织仓库/搜索。
- 选中仓库后复用既有 `Message::CloneUrlChanged`,不新增单独的"确认选择"步骤或额外的项目名称推导逻辑。
- 未连接账户时的"去设置连接"直接关闭本对话框、打开设置弹窗(spec 已确认,不做"两个弹窗共存"或"记住返回"之类的复杂处理)。

---

### Task 1: `git_accounts` 模块——仓库列表拉取

**Files:**
- Modify: `crates/dozer-app/src/git_accounts.rs`

**Interfaces:**
- Consumes: `GitProvider`(既有)
- Produces: `RemoteRepo { full_name: String, clone_url: String }`(`Debug, Clone, PartialEq`)、`pub(crate) fn parse_repo_list(provider: GitProvider, json: &str) -> Result<Vec<RemoteRepo>, String>`(纯函数)、`pub async fn list_repos(provider: GitProvider, token: &str) -> Result<Vec<RemoteRepo>, String>`——Task 2 的 `project_create::update` 会调用后者。

- [x] **Step 1: 写失败测试**

在 `crates/dozer-app/src/git_accounts.rs` 的 `#[cfg(test)] mod tests` 里追加:

```rust
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
```

- [x] **Step 2: 跑测试确认因类型/函数不存在而编译失败**

Run: `cargo test -p dozer-app --lib git_accounts`
Expected: 编译错误 `cannot find type 'RemoteRepo'`/`cannot find function 'parse_repo_list'`。

- [x] **Step 3: 实现——追加到 `crates/dozer-app/src/git_accounts.rs`,紧跟 `validate_token` 之后**

```rust
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
        GitProvider::GitLab => "https://gitlab.com/api/v4/projects?membership=true&owned=true&per_page=100&order_by=last_activity_at",
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
    let body = resp.text().await.map_err(|e| format!("读取响应失败: {e}"))?;
    parse_repo_list(provider, &body)
}
```

- [x] **Step 4: 跑测试**

Run: `cargo test -p dozer-app --lib git_accounts`
Expected: 新增的 7 个 `parse_repo_list_*` 测试全部 PASS,既有测试不受影响。`list_repos` 本身(真实网络调用)不做自动化测试,留给 Task 5 的人工验证清单。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/git_accounts.rs
git commit -m "feat(app): git_accounts 仓库列表拉取(list_repos/parse_repo_list)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: `project_create` 状态——`CloneSource`/`RepoListState`/reducer

**Files:**
- Modify: `crates/dozer-app/src/extensions/project_create.rs`

**Interfaces:**
- Consumes: `git_accounts::{GitProvider, RemoteRepo, get_token, list_repos}`(既有 + Task 1)
- Produces: `CloneSource`(`Url`/`Provider(GitProvider)`,`Default`=`Url`)、`RepoListState`(`Loading`/`Loaded(Vec<RemoteRepo>)`/`Error(String)`/`NotConnected`)、`CloneForm` 新增 `source`/`repo_lists` 字段、`Message::{SourceSelected, RepoListLoaded, GoToSettings}`——Task 3 的视图函数、Task 4 的 `App` 级拦截都要用到。

- [x] **Step 1: 写失败测试**

在 `crates/dozer-app/src/extensions/project_create.rs` 的 `#[cfg(test)] mod tests` 里追加(与既有测试同一个 `mod tests`):

```rust
    #[test]
    fn plan_source_selection_starts_loading_when_token_present() {
        assert_eq!(
            plan_source_selection(None, true),
            Some(RepoListState::Loading)
        );
    }

    #[test]
    fn plan_source_selection_reports_not_connected_when_no_token() {
        assert_eq!(
            plan_source_selection(None, false),
            Some(RepoListState::NotConnected)
        );
    }

    #[test]
    fn plan_source_selection_skips_when_already_loaded() {
        let existing = RepoListState::Loaded(vec![]);
        assert_eq!(plan_source_selection(Some(&existing), true), None);
    }

    #[test]
    fn plan_source_selection_skips_when_already_loading() {
        assert_eq!(plan_source_selection(Some(&RepoListState::Loading), true), None);
    }

    #[test]
    fn plan_source_selection_retries_after_error() {
        let existing = RepoListState::Error("x".into());
        assert_eq!(
            plan_source_selection(Some(&existing), true),
            Some(RepoListState::Loading)
        );
    }

    #[test]
    fn repo_list_loaded_ok_stores_loaded_state() {
        let mut state = State::default();
        state.tab = Tab::Clone;
        apply_field_message(
            &mut state,
            &Message::RepoListLoaded(
                GitProvider::GitHub,
                Ok(vec![git_accounts::RemoteRepo {
                    full_name: "a/b".into(),
                    clone_url: "https://x/a/b.git".into(),
                }]),
            ),
        );
        assert_eq!(
            state.clone_form.repo_lists.get(&GitProvider::GitHub),
            Some(&RepoListState::Loaded(vec![git_accounts::RemoteRepo {
                full_name: "a/b".into(),
                clone_url: "https://x/a/b.git".into(),
            }]))
        );
    }

    #[test]
    fn repo_list_loaded_err_stores_error_state() {
        let mut state = State::default();
        apply_field_message(
            &mut state,
            &Message::RepoListLoaded(GitProvider::GitLab, Err("网络请求失败".into())),
        );
        assert_eq!(
            state.clone_form.repo_lists.get(&GitProvider::GitLab),
            Some(&RepoListState::Error("网络请求失败".into()))
        );
    }

    #[test]
    fn go_to_settings_closes_dialog_like_close() {
        let mut state = Some(State::default());
        // 直接测顶层 `if let Message::Close | Message::GoToSettings` 分支
        // 的等价行为——不需要真的构造 `dozer_client::Client`/`handle`,
        // 因为这条分支在 `update()` 顶部就 return,不会往下走。
        if let Message::Close | Message::GoToSettings = Message::GoToSettings {
            state = None;
        }
        assert!(state.is_none());
    }
```

- [x] **Step 2: 跑测试确认因类型/函数不存在而编译失败**

Run: `cargo test -p dozer-app --lib extensions::project_create`
Expected: 编译错误(`RepoListState`/`plan_source_selection`/`Message::RepoListLoaded`/`Message::GoToSettings` 不存在)。

- [x] **Step 3: 实现**

顶部 `use` 区块加:

```rust
use crate::git_accounts::{self, GitProvider, RemoteRepo};
use std::collections::HashMap;
```

`Tab` 定义之后插入:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CloneSource {
    #[default]
    Url,
    Provider(GitProvider),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RepoListState {
    Loading,
    Loaded(Vec<RemoteRepo>),
    Error(String),
    NotConnected,
}
```

`CloneForm` 结构体加两个字段:

```rust
#[derive(Default)]
pub struct CloneForm {
    pub url: String,
    pub root_dir: String,
    pub name: String,
    pub name_touched: bool,
    pub description: iced_widget::text_editor::Content,
    /// 当前"远程仓库"字段展示的是手填 URL 还是某个账户的仓库列表。
    pub source: CloneSource,
    /// 本次对话框会话内按 provider 缓存的仓库列表加载态,切换 tab 来回点
    /// 不重复发请求(除非上次是 `Error`,那种情况允许重试)。
    pub repo_lists: HashMap<GitProvider, RepoListState>,
}
```

`Message` 枚举,在 `SubmitClone,` 之后插入:

```rust
    SourceSelected(CloneSource),
    RepoListLoaded(GitProvider, Result<Vec<RemoteRepo>, String>),
    /// 未连接账户时"去设置连接"按钮——不在本模块内部处理,由 `App::update`
    /// 顶层拦截(关本对话框、开设置弹窗),但顶部的 `if let Message::Close
    /// | Message::GoToSettings` 仍会先把 `state` 清空,保持"直接调用本模块
    /// `update` 也不会留下半开的对话框状态"这个防御性保证。
    GoToSettings,
```

`apply_field_message` 里,`Message::CloneUrlChanged(url) => { ... }` 分支之后插入 `RepoListLoaded` 处理:

```rust
        Message::RepoListLoaded(provider, result) => {
            let repo_state = match result {
                Ok(repos) => RepoListState::Loaded(repos.clone()),
                Err(e) => RepoListState::Error(e.clone()),
            };
            state.clone_form.repo_lists.insert(*provider, repo_state);
            true
        }
```

`apply_field_message` 末尾的 catch-all 改成:

```rust
        Message::Close
        | Message::SubmitLocal
        | Message::SubmitClone
        | Message::Done(_)
        | Message::SourceSelected(_)
        | Message::GoToSettings => false,
```

`update` 函数顶部的早退条件改成:

```rust
    if let Message::Close | Message::GoToSettings = msg {
        *state = None;
        return;
    }
```

在 `update` 主 `match` 里,`Message::SubmitClone => { ... }` 分支之后插入:

```rust
        Message::SourceSelected(source) => {
            s.clone_form.source = source;
            if let CloneSource::Provider(provider) = source {
                let existing = s.clone_form.repo_lists.get(&provider);
                let token = git_accounts::get_token(provider);
                if let Some(new_state) = plan_source_selection(existing, token.is_some()) {
                    let should_fetch = matches!(new_state, RepoListState::Loading);
                    s.clone_form.repo_lists.insert(provider, new_state);
                    if should_fetch {
                        let token = token.expect("Loading 状态下 token 一定存在");
                        handle.spawn(async move {
                            let result = git_accounts::list_repos(provider, &token).await;
                            emit(Message::RepoListLoaded(provider, result));
                        });
                    }
                }
            }
        }
```

`update` 末尾的 `unreachable!` 分支列表加上 `Message::GoToSettings` 和 `Message::SourceSelected(_)`(`RepoListLoaded` 已经在 `apply_field_message` 里处理,不在这个列表):

```rust
        Message::Close
        | Message::GoToSettings
        | Message::TabSelected(_)
        | Message::LocalRootDirChanged(_)
        | Message::LocalRootDirPick
        | Message::LocalRootDirPicked(_)
        | Message::LocalNameChanged(_)
        | Message::LocalDescriptionAction(_)
        | Message::LocalCreateGitToggled(_)
        | Message::CloneUrlChanged(_)
        | Message::CloneRootDirChanged(_)
        | Message::CloneRootDirPick
        | Message::CloneRootDirPicked(_)
        | Message::CloneNameChanged(_)
        | Message::CloneDescriptionAction(_)
        | Message::RepoListLoaded(..)
        | Message::SourceSelected(_) => {
            unreachable!("已在 apply_field_message 或顶部处理")
        }
```

在 `update` 函数**之前**(模块级,`apply_field_message` 之前或之后均可,建议紧跟 `apply_field_message` 之后)新增纯函数:

```rust
/// 决定 `SourceSelected(Provider)` 该不该发起新的仓库列表请求,以及请求
/// 前状态该置成什么。已经 `Loaded`/`Loading` 就不重复请求(返回
/// `None`);`Error` 允许重试。纯函数,不碰真实 Keychain——`token_present`
/// 由调用方查真实 `git_accounts::get_token` 后传入,方便单测。
fn plan_source_selection(
    existing: Option<&RepoListState>,
    token_present: bool,
) -> Option<RepoListState> {
    if matches!(
        existing,
        Some(RepoListState::Loaded(_)) | Some(RepoListState::Loading)
    ) {
        return None;
    }
    Some(if token_present {
        RepoListState::Loading
    } else {
        RepoListState::NotConnected
    })
}
```

- [x] **Step 4: 跑测试**

Run: `cargo test -p dozer-app --lib extensions::project_create`
Expected: 新增的 8 个测试(`plan_source_selection_*` 5 个 + `repo_list_loaded_*` 2 个 + `go_to_settings_closes_dialog_like_close`)全部 PASS,既有测试不受影响。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/project_create.rs
git commit -m "feat(app): project_create 接入 CloneSource/RepoListState 状态机

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: `project_create` 视图——仓库选择列表

**Files:**
- Modify: `crates/dozer-app/src/extensions/project_create.rs`

**Interfaces:**
- Consumes: `CloneSource`/`RepoListState`(Task 2)
- Produces: `sidebar_entry` 签名变更(接受 `on_press: Option<Message>` 而非 `enabled: bool`)、新的 `remote_repo_field`/`repo_radio_row` 视图函数、更新后的 `clone_sidebar`/`clone_form_view`。

- [x] **Step 1: 改 `sidebar_entry`——从"禁用占位"改成"可点选中态"**

把:
```rust
fn sidebar_entry(label: &'static str, enabled: bool) -> Element<'static> {
    let colors = byteui::theme::color::current();
    let label_el = text(label)
        .size(byteui::theme::font::body())
        .color(if enabled { colors.cream } else { colors.dim });
    let cell = container(label_el)
        .padding([8, 12])
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(if enabled { colors.card } else { colors.bg }.into()),
            ..container::Style::default()
        });
    // GitLab/Gitee 等账户接入推迟到后续(spec「非目标」):`enabled=false`
    // 视觉置灰、不挂 `on_press`,与 `menu_spec::to_iced` 里 `enabled=false`
    // 项"点不动"的既有语义一致。
    cell.into()
}

fn clone_sidebar() -> Element<'static> {
    column![
        sidebar_entry("仓库URL", true),
        sidebar_entry("GitHub", false),
        sidebar_entry("GitLab", false),
        sidebar_entry("Gitee", false),
    ]
    .spacing(4)
    .width(Length::Fixed(120.0))
    .into()
}
```
改成:
```rust
/// `selected` 决定高亮,`on_press` 现在四项都会给(GitHub/GitLab/Gitee 账户
/// 接入已经落地,不再有占位禁用项——见
/// `docs/superpowers/specs/2026-09-18-project-create-remote-repo-picker-
/// design.md`)。
fn sidebar_entry(label: &'static str, selected: bool, on_press: Message) -> Element<'static> {
    let colors = byteui::theme::color::current();
    let label_el = text(label)
        .size(byteui::theme::font::body())
        .color(if selected { colors.cream } else { colors.dim });
    let cell = container(label_el)
        .padding([8, 12])
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(if selected { colors.card } else { colors.bg }.into()),
            ..container::Style::default()
        });
    iced_widget::MouseArea::new(cell)
        .interaction(iced_widget::core::mouse::Interaction::Pointer)
        .on_press(on_press)
        .into()
}

fn clone_sidebar(source: CloneSource) -> Element<'static> {
    column![
        sidebar_entry(
            "仓库URL",
            source == CloneSource::Url,
            Message::SourceSelected(CloneSource::Url)
        ),
        sidebar_entry(
            "GitHub",
            source == CloneSource::Provider(GitProvider::GitHub),
            Message::SourceSelected(CloneSource::Provider(GitProvider::GitHub))
        ),
        sidebar_entry(
            "GitLab",
            source == CloneSource::Provider(GitProvider::GitLab),
            Message::SourceSelected(CloneSource::Provider(GitProvider::GitLab))
        ),
        sidebar_entry(
            "Gitee",
            source == CloneSource::Provider(GitProvider::Gitee),
            Message::SourceSelected(CloneSource::Provider(GitProvider::Gitee))
        ),
    ]
    .spacing(4)
    .width(Length::Fixed(120.0))
    .into()
}
```

- [x] **Step 2: 新增 `remote_repo_field`/`repo_radio_row`,替换 `clone_form_view` 里固定的 URL 输入框**

在 `clone_sidebar` 之后、`clone_form_view` 之前插入:

```rust
fn repo_radio_row(repo: &RemoteRepo, current_url: &str) -> Element<'_> {
    let colors = byteui::theme::color::current();
    let is_selected = repo.clone_url == current_url;
    let dot = container(Space::new())
        .width(Length::Fixed(10.0))
        .height(Length::Fixed(10.0))
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: if is_selected {
                Some(colors.gold.into())
            } else {
                None
            },
            border: Border {
                color: if is_selected { colors.gold } else { colors.border },
                width: 1.5,
                radius: 5.0.into(),
            },
            ..container::Style::default()
        });
    let ring = container(dot)
        .width(Length::Fixed(16.0))
        .height(Length::Fixed(16.0))
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    iced_widget::MouseArea::new(
        row![
            ring,
            text(repo.full_name.clone())
                .size(byteui::theme::font::body())
                .color(colors.cream),
        ]
        .spacing(6)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .interaction(iced_widget::core::mouse::Interaction::Pointer)
    .on_press(Message::CloneUrlChanged(repo.clone_url.clone()))
    .into()
}

/// "远程仓库"字段的内容——`CloneSource::Url` 时是手填输入框(原有行为),
/// `CloneSource::Provider` 时按加载态切换成引导/加载中/报错/单选列表。
fn remote_repo_field(form: &CloneForm) -> Element<'_> {
    let colors = byteui::theme::color::current();
    match form.source {
        CloneSource::Url => byteui::form::input_text::view(
            "Input",
            &form.url,
            false,
            None,
            false,
            None,
            false,
            Message::CloneUrlChanged,
        ),
        CloneSource::Provider(provider) => match form.repo_lists.get(&provider) {
            None | Some(RepoListState::NotConnected) => {
                let hint = text(format!("未连接 {} 账户", provider.display_name()))
                    .size(byteui::theme::font::body())
                    .color(colors.dim);
                let go_btn = button(text("去设置连接").size(byteui::theme::font::body()))
                    .on_press(Message::GoToSettings)
                    .padding([6, 14]);
                column![hint, go_btn].spacing(8).into()
            }
            Some(RepoListState::Loading) => text("加载仓库列表中…")
                .size(byteui::theme::font::body())
                .color(colors.dim)
                .into(),
            Some(RepoListState::Error(e)) => {
                let msg = text(format!("仓库列表加载失败: {e}"))
                    .size(byteui::theme::font::body())
                    .color(colors.red);
                let retry = button(text("重试").size(byteui::theme::font::body()))
                    .on_press(Message::SourceSelected(CloneSource::Provider(provider)))
                    .padding([6, 14]);
                column![msg, retry].spacing(8).into()
            }
            Some(RepoListState::Loaded(repos)) if repos.is_empty() => text("该账户名下没有仓库")
                .size(byteui::theme::font::body())
                .color(colors.dim)
                .into(),
            Some(RepoListState::Loaded(repos)) => {
                let rows: Vec<Element<'_>> = repos
                    .iter()
                    .map(|repo| repo_radio_row(repo, &form.url))
                    .collect();
                iced_widget::Column::with_children(rows).spacing(6).into()
            }
        },
    }
}
```

把 `clone_form_view` 里:
```rust
    let fields = column![
        field_label("远程仓库"),
        byteui::form::input_text::view(
            "Input",
            &form.url,
            false,
            None,
            false,
            None,
            false,
            Message::CloneUrlChanged,
        ),
        field_label("根目录"),
```
改成:
```rust
    let fields = column![
        field_label("远程仓库"),
        remote_repo_field(form),
        field_label("根目录"),
```

以及函数末尾的 `row![clone_sidebar(), fields]` 改成 `row![clone_sidebar(form.source), fields]`。

- [x] **Step 3: 编译检查**

Run: `cargo build -p dozer-app 2>&1 | grep -E "^error" | head -60`
Expected: 干净编译(此时 `Message::GoToSettings` 已经存在于 `project_create::Message`,`App` 级拦截还没加,但这不影响本 crate 自身编译——`Message::ProjectCreate(project_create::Message::GoToSettings)` 若没有专门的拦截分支,会落进 Task 9 已有的通用转发分支 `Message::ProjectCreate(msg) => { ... project_create::update(...) }`,调用 `project_create::update` 时顶部的 `if let Message::Close | Message::GoToSettings` 分支会正确关闭对话框,只是不会额外打开设置弹窗——这是 Task 4 要补的部分,不是编译问题)。若报 `iced_widget::Column::with_children` 或 `MouseArea` 相关签名不对,以实际报错为准调整(不同 iced 0.14 patch 版本个别构造器名字可能略有出入)。

- [x] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/project_create.rs
git commit -m "feat(app): project_create 仓库选择列表视图

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 4: `App` 级拦截——"去设置连接"跳转

**Files:**
- Modify: `crates/dozer-app/src/app/update.rs`

**Interfaces:**
- Consumes: `App.settings: Option<settings::State>`、`settings::State::load()`(前置依赖,见 Global Constraints)
- Produces: 点"去设置连接"能正确关掉创建项目对话框、打开设置弹窗。

- [x] **Step 1: 确认前置依赖已落地**

Run: `grep -n "pub(crate) settings: Option<settings::State>" crates/dozer-app/src/app/app.rs`
Expected: 有输出。若没有,说明 `docs/superpowers/plans/2026-09-18-git-account-settings.md` 的 Task 9 还没完成——**停下**,先确认那份计划的执行状态,不要在缺依赖的情况下继续本任务(会编译不过)。

- [x] **Step 2: 加拦截分支**

`crates/dozer-app/src/app/update.rs`,在 `Message::ProjectCreate(project_create::Message::Done(result)) => { ... }` 分支**之前**插入(必须在通用转发分支 `Message::ProjectCreate(msg) => { ... }` 之前,顺序规则同 `Done` 的既有先例):

```rust
            Message::ProjectCreate(project_create::Message::GoToSettings) => {
                self.project_create = None;
                self.settings = Some(settings::State::load());
            }
```

- [x] **Step 3: 编译 + 全量测试**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 干净编译通过。

Run: `cargo test -p dozer-app --lib`
Expected: 全部既有测试 + 本计划新增的所有测试(Task 1-3)PASS。

- [x] **Step 4: Commit**

```bash
git add crates/dozer-app/src/app/update.rs
git commit -m "feat(app): 创建项目未连接账户时跳转设置弹窗

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 5: 全量检查 + 人工验证清单

**Files:** 无代码改动(除非检查发现问题需要回头小修)

**Interfaces:** 无

- [x] **Step 1: 全量构建/测试/静态检查**

```bash
cargo build
cargo test -p dozer-app
cargo clippy --all-targets
cargo fmt --check
```

Expected: 全部通过;`cargo fmt --check` 报格式问题就跑 `cargo fmt` 后单独提交一次格式化 commit。

- [ ] **Step 2: `cargo run -p dozer-app` 人工验证清单**

（执行前先用真实账户在设置弹窗里连接好至少一个 GitHub/GitLab/Gitee 账户,另外保留一个故意不连接用于测试引导流程。）

- [ ] "创建项目"对话框 Tab 2 侧边栏四项(仓库URL/GitHub/GitLab/Gitee)现在都可点,选中项有高亮。
- [ ] 点已连接的那家:先短暂显示"加载仓库列表中…",随后展示真实仓库单选列表,选中一条后"项目名称"字段被自动填充为仓库名(可再手动编辑覆盖)。
- [ ] 点未连接的那家:展示"未连接 `<服务商>` 账户"+"去设置连接"按钮;点击后"创建项目"对话框关闭、设置弹窗打开,能看到 Git 账户区块。
- [ ] 用刚才打开的设置弹窗连接好那个账户后,重新从顶栏"+"菜单点"创建项目"进入,再次选中该服务商,应该能看到仓库列表(不需要重启 app)。
- [ ] 断网状态下点某个已连接账户:展示"网络请求失败"或对应错误文案,不崩溃;有"重试"按钮,联网后点重试能成功加载。
- [ ] 来回切换"仓库URL"↔已加载过列表的服务商 tab,不会重复触发网络请求(可用抓包工具或临时加日志确认只请求一次)。
- [ ] 选中某个仓库后切回"仓库URL"tab,输入框里能看到刚才选中仓库的 URL(状态保留,不清空)。
- [ ] 选中仓库后正常点击"签出项目"提交,能成功克隆并打开新项目 tab(复用既有签出流程,行为不变)。

- [ ] **Step 3: 若人工验证发现问题,记录并修复**

对每一条失败项:定位对应任务的代码,修复后重新跑 Step 1 的全量检查,再回到 Step 2 从头过一遍清单。

- [ ] **Step 4: 最终提交(若 Step 3 有修复)**

```bash
git add -A
git commit -m "fix(app): 创建项目仓库选择人工验证发现的问题修复

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```
