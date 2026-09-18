# Git 账户设置设计

## 背景与动机

`docs/superpowers/specs/2026-09-18-new-project-creation-design.md`(新建项目对话框)把 GitHub/GitLab/Gitee 账户接入明确列为非目标,线框图里对应的三个侧边栏入口只做了禁用占位。用户后续确认这块仍要做,但补充了一个关键前提:**创建项目对话框假定账户已经连接好,没连接就引导用户去设置页完成**——即账户的连接/鉴权本身不应该塞进创建项目这个已经很拥挤的表单里,而是拆成两个先后关系明确的子项目:

1. **本文档(第一个子项目)**:账户设置页面——在哪连接、怎么连接、连接状态怎么存。
2. **后续子项目(不在本文档范围)**:创建项目对话框的 GitHub/GitLab/Gitee tab 读取本文档存下的连接状态,已连接则调用对应 API 列仓库供选择、未连接则引导跳转到本文档的设置页;选中仓库后复用已有的 URL 签出提交路径(`docs/superpowers/specs/2026-09-18-new-project-creation-design.md`「Tab 2」的 `clone_repo`),不重新发明克隆逻辑。

设计过程中额外发现一处现状缺陷:`crates/dozer-app/src/settings.rs` 目前是顶栏齿轮图标弹出的设置弹窗,只有"主题"一项,渲染方式是 `app/view.rs:45-50` 里的普通 iced `stack!` 叠层——**没有接入独立窗口机制,会被预览 webview 天然遮住**(wry webview 恒在 GPU 内容之上,CLAUDE.md 关键裁决已经点名这类"需要盖住 webview 的原生浮层"必须显式处理)。因为这次要往这个弹窗里塞真正有实用价值的新功能(Git 账户连接),而不是继续放着这个 bug,顺手把整个设置弹窗迁到 `docs/superpowers/specs/2026-09-18-overlay-window-shared-abstraction-design.md` 抽出的独立窗口共享机制(`OverlayGpu`/`open_child_window`),与 `SearchOverlay`/`FileHistoryOverlay` 同构。

## 目标 / 非目标

**目标:**

1. 设置弹窗(顶栏齿轮图标,`Message::SettingsOpen`)新增"Git 账户"区块:GitHub/GitLab/Gitee 各一行,展示连接状态(未连接 / 已连接:`<用户名>`)+ 对应操作按钮(连接 / 断开连接)。
2. 点"连接"展开一个内联 PAT(Personal Access Token)输入框 + 一行"如何生成 PAT"的可点击提示,点击后用系统默认浏览器打开对应官方 token 创建页。
3. 提交 PAT 后异步调用该服务商的 whoami 接口校验有效性,成功后把 token 存入 macOS Keychain(`keyring` crate,新命名空间),把返回的用户名等非敏感展示信息写入本地小文件;校验失败在设置弹窗内联展示服务商返回的错误。
4. "断开连接"删除 Keychain 条目与本地记录,状态回到"未连接"。
5. 整个设置弹窗从现有的 `app/view.rs` 内联 `stack!` 叠层迁移到独立原生窗口(`SettingsOverlay`),修掉"被预览 webview 遮住"这个现状缺陷,主题选择这一项功能不变。

**非目标:**

- GitHub/GitLab/Gitee 的 OAuth 授权流程(设备码登录等)——本次固定用 PAT 粘贴。
- 自建/企业自托管实例(自定义 API base URL)——本次固定官方 SaaS 域名(`api.github.com`/`gitlab.com`/`gitee.com`)。
- 单个服务商下的多账户管理——每家最多一个已连接账户,再次连接直接覆盖旧 token。
- 创建项目对话框如何消费这里存的连接状态、如何拉取/展示仓库列表——这是明确的后续子项目,不在本文档范围,本文档只保证"连接状态可查、token 可取"这个接口稳定。
- 除 Git 账户外,设置弹窗其它可能扩展的设置项(如快捷键、通知)——本次只加 Git 账户这一块。

## 数据模型

**Keychain(敏感数据,token 本身)**:沿用 `crates/dozer-app/src/extensions/ssh.rs` 的既有用法(`keyring::Entry::new("dozer-ssh", "{project_id}:{host_id}")`),新命名空间 `"dozer-git"`,key 直接用服务商标识(全局账户,不挂靠项目,不需要 `project_id` 前缀):

```rust
fn keyring_entry(provider: GitProvider) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new("dozer-git", provider.as_key())
}
```

`GitProvider` 是 `{ GitHub, GitLab, Gitee }` 三元枚举,`as_key()` 返回 `"github"`/`"gitlab"`/`"gitee"`。

**本地小文件(非敏感展示信息)**:`dozer_core::paths::config_dir().join("git_accounts.json")`,结构对照 `open_projects.json`(同属"UI 侧非权威展示状态,不进 dozerd"的既有分工):

```json
{
  "github": { "username": "octocat" },
  "gitlab": null,
  "gitee": null
}
```

`username` 是连接成功时 whoami 接口返回的账户名,仅用于"已连接:xxx"这行文字展示;判断"是否已连接"看这个文件里对应 key 是否为非 null,**不**用它反推 Keychain 里是否真的还有 token(两者理论上应该总是一致,写入/删除时同一次操作里先后完成,不做额外的一致性校验——如果用户手动清过 Keychain 却没走"断开连接"按钮,属地已知的、可接受的边缘情况,下次调用 API 时会因为 401 而在使用侧报错,不在本文档处理范围)。

## UI 设计

### 设置弹窗结构

在 `settings::settings_modal` 现有的"主题"区块下方新增"Git 账户"区块,每个服务商一行:

- 未连接:服务商名 + 灰色"未连接" + "连接"按钮。
- 已连接:服务商名 + 金色"已连接:`<username>`" + "断开连接"按钮。
- 点"连接"后,该行原地展开成:PAT 输入框(`byteui::form::input_text`,`secure=true`,遮罩显示)+ "确认"/"取消"两个小按钮 + 一行"没有 PAT?点此生成"的可点击文字(`MouseArea` + 下划线样式,同现有"打开外部链接"类文案的观感)。
- 提交中禁用输入框和按钮,展示"校验中…";失败在这一行下方展示服务商返回的错误文案(如"401 Unauthorized"翻译成"令牌无效或已过期",避免直接甩英文错误码)。

### 渲染宿主

`SettingsOverlay`(新建 `crates/dozer-app/src/platform/settings_overlay.rs`),结构与 `FileHistoryOverlay` 同构:持有 `window: Arc<Window>`、`gpu: OverlayGpu`,`open`/`redraw`/`handle_input`/`reposition`/`window_id` 五个方法。因为有 PAT 文本输入,需要 `SearchOverlay` 那一套 IME(`window.set_ime_allowed(true)`)+ 原生右键菜单挂靠(`native_menu::install_content_view`,`Drop` 时还原)机制,不能只照抄 `FileHistoryOverlay` 的无输入版本。

失焦行为:**接入失焦即关闭**(`FocusTracker`)——与"创建项目"对话框不同,设置弹窗没有嵌套的 rfd 原生面板会触发意外失焦,失焦关闭是合理且与 `SearchOverlay` 一致的默认行为;唯一需要注意的是 PAT 输入框展开状态下,用户点击"没有 PAT?点此生成"会拉起系统浏览器——这本身不会让本窗口失焦(系统浏览器是另一个独立进程的窗口,不是本窗口的子面板),不需要像"创建项目"那样特殊处理。

互斥:`OverlayKind` 新增一个 `Settings` 变体,加入 `close_other_overlays` 的判断,与 `Search`/`FileHistory`/(若已合入的)`ProjectCreate` 保持互斥——具体加在哪个位置以合入时 `OverlayKind` 实际已有的变体为准,原则是"新开一个必须关掉其余所有已开的独立窗口"这条既有规则覆盖到 `Settings`。

`app/view.rs:45-50` 里那段判断 `self.settings_modal_open` 走 `stack!` 叠加 `settings::settings_modal` 的分支整段删除——设置弹窗不再是主窗口内容树的一部分。

## 依赖与外部调用

**新增 `reqwest`**(`crates/dozer-app/Cargo.toml`,features `["json"]`,复用默认的 tokio 异步运行时,不用额外配置 executor)——这是本次唯一的新增第三方依赖,仓库目前没有任何 HTTP 客户端。

**Whoami 校验接口**(用于"连接"时校验 token 有效并取用户名,不做更多字段读取):

| 服务商 | 接口 | 鉴权头 |
|---|---|---|
| GitHub | `GET https://api.github.com/user` | `Authorization: Bearer <token>` |
| GitLab | `GET https://gitlab.com/api/v4/user` | `PRIVATE-TOKEN: <token>` |
| Gitee | `GET https://gitee.com/api/v5/user?access_token=<token>` | 无(token 走 query 参数,Gitee API v5 的既定用法) |

三家分别取响应 JSON 里的用户名字段(GitHub/GitLab 是 `login`,Gitee 是 `login`)作为展示用 `username`,不缓存其它字段。非 2xx 响应一律视为"令牌无效或已过期",不区分 401/403/404 的具体文案差异(YAGNI,真出现需要区分的场景再细化)。

**打开 PAT 生成页**:复用 `crates/dozer-app/src/extensions/files/update.rs:88` 已有的 `std::process::Command::new("open")` 唤起系统默认浏览器的写法,三家各自的固定链接:

- GitHub: `https://github.com/settings/tokens/new`
- GitLab: `https://gitlab.com/-/user_settings/personal_access_tokens`
- Gitee: `https://gitee.com/profile/personal_access_tokens/new`

## 错误处理

| 场景 | 处理 |
|---|---|
| PAT 为空 | "连接"按钮保持禁用,不发请求 |
| whoami 请求网络失败(超时/DNS 等) | 内联展示"网络请求失败,请检查网络后重试",允许重试 |
| whoami 返回非 2xx | 内联展示"令牌无效或已过期",允许重新输入 |
| Keychain 写入/删除失败(`keyring::Error`) | 内联展示"系统钥匙串操作失败: `<error>`",不假装连接/断开成功 |
| 本地小文件写入失败 | 同上归类为一次连接/断开失败,回滚成"未连接"态展示,不留半成品状态(即宁可用户重试,也不留"Keychain 有 token 但本地文件没记录"或反过来的不一致态——具体做法是**先调 whoami、再写 Keychain、最后写本地文件**,任一步失败都不再往下执行且清理已完成的前置步骤) |

## 测试策略

- `GitProvider::as_key()`、`keyring_entry` 的命名空间拼接、本地文件的序列化/反序列化(`git_accounts.json` 读写),以及"用户名字段提取"这几个纯函数/纯逻辑用真实 tempdir 补单测,风格同 `project_meta.rs`/`delivery.rs` 现有测试。
- whoami 请求本身(真实网络调用)不做自动化测试,人工验证清单覆盖三家分别用真实 PAT 连接成功、用错误 token 连接失败展示报错、断开连接后状态回到未连接、重启 app 后已连接状态能从本地文件正确恢复(不需要重新输入 token)。
- 独立窗口生命周期(开关/与其它三类弹窗互斥/失焦关闭/resize 跟随)没有自动化测试手段,人工 `cargo run` 验证清单覆盖,项目同类既有验收方式(`SearchOverlay`/`FileHistoryOverlay`)。
- "打开 PAT 生成页"三个链接每个人工点一次确认能跳转到对应服务商的正确页面。
