# 创建项目对话框接入账户仓库列表设计

## 背景与动机

`docs/superpowers/specs/2026-09-18-new-project-creation-design.md`(新建项目对话框)把 GitHub/GitLab/Gitee 账户接入列为非目标,`extensions::project_create::clone_sidebar` 里这三项目前是禁用置灰的占位(`sidebar_entry(label, false)`,不挂 `on_press`)。`docs/superpowers/specs/2026-09-18-git-account-settings-design.md`(第一个子项目)补上了"账户连接"这一半——用户能在设置弹窗里用 PAT 连接三家账户,token 存 Keychain、用户名存本地 `git_accounts.json`。

本文档是第二个子项目:把 `project_create` 对话框里这三个占位项接上真功能——已连接就调用对应 API 列出用户仓库供选择,未连接就引导去设置页完成连接。选中仓库后复用已有的 `CloneUrlChanged` 消息(会自动推导项目名称)把 URL 填进去,后续提交路径(`SubmitClone`→`delivery::clone_repo`)完全不变。

**前置依赖**:本文档描述的功能依赖 `docs/superpowers/specs/2026-09-18-git-account-settings-design.md` 那份计划里的 `crate::git_accounts` 模块(`GitProvider`/`get_token`)和 `extensions::settings`/`App.settings`/`Message::SettingsOpen`/`Message::Settings` 已经落地——本文档新增的 `Message::ProjectCreate(project_create::Message::GoToSettings)` 拦截分支需要调用 `settings::State::load()`。若这份依赖还没实现完,本文档描述的功能无法编译通过,写实现计划时要在 Task 列表里显式检查这个前提。

## 目标 / 非目标

**目标:**

1. `project_create` 对话框 Tab 2「签出Git远程仓库的项目」的侧边栏 GitHub/GitLab/Gitee 三项从禁用占位改成可点击。
2. 点某一项:
   - 若该账户**已连接**(`git_accounts::get_token(provider).is_some()`):异步调用该服务商仓库列表接口,加载中显示"加载中…",成功后把"远程仓库"字段原来的 URL 输入框换成一个单选列表(仓库全名,如 `abc/python_project`),选中即把该仓库的 clone URL 写入 `clone_form.url`(复用既有 `CloneUrlChanged`,自动推导项目名称的逻辑不变)。
   - 若该账户**未连接**:展示"未连接 `<服务商>` 账户"+"去设置连接"按钮,点击后关闭"创建项目"对话框、打开设置弹窗(两者是互斥的独立窗口,关一个开一个是唯一自洽的处理方式——用户连接完账户后需要自己重新从"+"菜单点"创建项目"进来,可接受,因为这一步对话框本来就还没填什么内容)。
3. 仓库列表只取**用户自己名下的仓库**(不含所属组织/团队),只取**第一页**(每页拉大到 100 条缓解大多数用户的够用问题,不做翻页/滚动加载)。
4. 加载失败(网络错误/token 失效等)展示错误文案 +"重试"按钮,不影响其它字段。

**非目标:**

- 组织/团队仓库。
- 完整分页/滚动加载。
- 仓库列表内的搜索/过滤。
- 自建/企业自托管实例(沿用第一个子项目"仅官方 SaaS"的既有限定)。
- 多账户/账户切换(沿用"每家单账户"的既有限定)。
- 仓库列表的排序自定义(用接口默认排序,通常是最近活跃优先)。

## 数据模型

`crate::git_accounts` 新增(在第一个子项目已有的 `GitProvider`/`ConnectedAccount`/`GitAccountsState`/`validate_token` 基础上追加):

```rust
pub struct RemoteRepo {
    pub full_name: String,   // "abc/python_project"
    pub clone_url: String,   // https clone URL
}

pub async fn list_repos(provider: GitProvider, token: &str) -> Result<Vec<RemoteRepo>, String>
```

各家列表接口与字段映射(**设计阶段最佳猜测,实现前建议用真实账户 `curl` 核实一次响应形状,字段名如有出入以实际响应为准调整,不强行遵照本表**):

| 服务商 | 接口 | 鉴权头 | 全名字段 | Clone URL 字段 |
|---|---|---|---|---|
| GitHub | `GET https://api.github.com/user/repos?per_page=100&sort=updated&affiliation=owner` | `Authorization: Bearer <token>` | `full_name` | `clone_url` |
| GitLab | `GET https://gitlab.com/api/v4/projects?membership=true&owned=true&per_page=100&order_by=last_activity_at` | `PRIVATE-TOKEN: <token>` | `path_with_namespace` | `http_url_to_repo` |
| Gitee | `GET https://gitee.com/api/v5/user/repos?per_page=100&sort=updated` | `access_token` query 参数 | `full_name` | `html_url`(若响应另有专门的 clone/https 字段,优先用那个) |

## UI 设计

### 侧边栏

`clone_sidebar()` 里 GitHub/GitLab/Gitee 三个 `sidebar_entry(..., false)` 改成 `true`,并挂 `on_press(Message::SourceSelected(CloneSource::Provider(provider)))`;"仓库URL"项同样挂 `on_press(Message::SourceSelected(CloneSource::Url))`。当前选中项高亮(背景色区分,同现有 `enabled` 视觉但语义改成"是否选中")。

### 右侧内容(替换原来固定的"远程仓库" URL 输入框那一块)

- `CloneSource::Url`:保持现状,一个文本输入框(`byteui::form::input_text`)。
- `CloneSource::Provider(provider)`:
  - 未连接:一行文字"未连接 `<服务商>` 账户" + "去设置连接"按钮。
  - 加载中:一行文字"加载仓库列表中…"。
  - 加载失败:错误文案(内联,同其它字段的错误展示风格)+"重试"按钮(重新触发同一次 `SourceSelected`)。
  - 加载成功且列表非空:单选列表,每行一个仓库全名,选中态用 `extensions/project/view.rs::project_delete_confirm_popup` 现成的"彩色圆点 + `MouseArea`"idiom(不新增 `iced_widget::radio` 依赖,同仓库既有约定)。
  - 加载成功但列表为空:一行文字"该账户名下没有仓库"。

根目录/项目名称/项目描述/"签出项目"按钮这几块保持原位不变。

## 消息与状态

`extensions::project_create` 新增:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneSource {
    Url,
    Provider(GitProvider),
}

#[derive(Debug, Clone)]
pub enum RepoListState {
    Loading,
    Loaded(Vec<git_accounts::RemoteRepo>),
    Error(String),
    NotConnected,
}
```

`CloneForm` 新增两个字段:

```rust
pub source: CloneSource,                                    // 默认 CloneSource::Url
pub repo_lists: std::collections::HashMap<GitProvider, RepoListState>, // 本次对话框会话内的按需缓存
```

`Message` 新增:

```rust
SourceSelected(CloneSource),
RepoListLoaded(GitProvider, Result<Vec<git_accounts::RemoteRepo>, String>),
GoToSettings,
```

`SourceSelected(CloneSource::Provider(provider))` 的处理:若 `repo_lists` 里该 provider 已经是 `Loaded`/`Loading` 就不重复请求(会话内缓存,切换 tab 来回点不重复打 API);否则查 `git_accounts::get_token(provider)`——`None` 直接置 `RepoListState::NotConnected`,`Some(token)` 置 `RepoListState::Loading` 并异步 spawn 调 `git_accounts::list_repos`。

`GoToSettings` **不在 `project_create::update` 内部处理**,由 `App::update` 在顶层拦截(同 `Message::ProjectCreate(project_create::Message::Done(result))` 已有的拦截先例,必须写在泛化转发分支之前):

```rust
Message::ProjectCreate(project_create::Message::GoToSettings) => {
    self.project_create = None;
    self.settings = Some(settings::State::load());
}
```

## 错误处理

| 场景 | 处理 |
|---|---|
| 未连接账户时点该服务商 | 展示"去设置连接"引导,不发请求 |
| 已连接但 token 失效(接口返回非 2xx) | `RepoListState::Error`,展示"仓库列表加载失败: 令牌可能已失效,请重新连接账户"+ "重试" 按钮;不自动跳设置页(避免用户还没看清报错就被强制跳走) |
| 网络请求失败 | `RepoListState::Error("网络请求失败")`,同上给"重试" |
| 该账户名下没有仓库 | `RepoListState::Loaded(vec![])`,展示"该账户名下没有仓库",不算错误 |
| 选中仓库后又切换回"仓库URL"或另一个服务商 | `clone_form.url` 保留上次选中值(不清空),用户可以在"仓库URL"tab 里看到并继续编辑;`repo_lists` 缓存不清 |

## 测试策略

- `git_accounts::list_repos` 内部"把响应 JSON 数组解析成 `Vec<RemoteRepo>`"这部分拆成纯函数(三个服务商各一个,或按字段名传参数复用一个通用解析器),用真实响应形状的 JSON 字符串字面量做单测,覆盖"正常列表"/"空列表"/"缺少期望字段"三种输入。
- `list_repos` 本身(真实网络调用)不做自动化测试。
- `SourceSelected` 的"已连接判定 + 触发/复用缓存"逻辑因为要读真实 Keychain,不做自动化测试(同第一个子项目里 keyring 相关函数一贯不测的口径),人工验证清单覆盖:已连接账户选中后能看到仓库列表;未连接账户选中后能看到"去设置连接"引导且点击后正确关闭本对话框、打开设置弹窗定位到 Git 账户区块;网络异常/token 失效时的报错与重试;选中仓库后"项目名称"字段被正确自动填充且可再手动编辑;来回切换仓库URL/GitHub/GitLab/Gitee 不重复发请求(会话缓存生效,可用抓包或加日志人工确认)。
