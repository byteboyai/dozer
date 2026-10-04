# H0-03:host 对面板的硬编码与 E3 登记清单初版(Q13)

> 基线提交:`f245321`(bytehost-h0 分支,代码与 `main` 的 `4845fb85` 一致)。
> 机械数据:`data/host-to-ext.md`,生成命令 `python3 scripts/audit/report.py host-to-ext`(host 层 = `app/`、`workspace/`、`chrome/`、`platform/`、`preview/`;统计对 `crate::extensions::<面板>` 的引用,含测试)。
> 回答:要求文档 Q13(host 自身对 `git_log` 的直接引用是否也要经 Registry)。产出 E3 登记清单初版:`E3-registry.tsv`。

## 1. 分类口径

- **A 编排:** host 持有面板的 state/message 并转发(`Message::Todo(todo::Message)`、`Workspace.todo: todo::WorkspaceState`)。这是"面板接入 host"的正常形态,迁移后由 registry 的通用接口承担,**不进 E3**(但会在 §3 计数)。
- **B 特判:** host 对这个面板有别的面板没有的特殊逻辑。进 E3,类别 `panel-special-case`。
- **C 越界:** host 直接读写面板内部字段或调用面板内部函数;或面板直接读 host 内部状态。同样进 E3(`panel-special-case`),在"对象"列写明。
- **例外:`toast`** 是 host 基础设施(规格 §3.1),`workspace/state.rs`×8、`platform/toast_overlay.rs`×2 对它的引用不算耦合,不进 E3。

## 2. host×面板引用分类表(引用次数 ≥3 的行)

| host 文件 | 面板 | 次数 | 分类 | 依据 | E3 |
|---|---|---|---|---|---|
| `platform/window_events.rs` | files | 12 | **B** | `:305-723` 窗口事件里直接构造 `files::Message::{FileDragHover, RightClickAt, TreeDragRelease, FileDrop, ContextMenuClose}`:文件拖放与右键是 Files 面板的行为,窗口层却认识它的消息 | E3-010 |
| `app/update.rs` | group_chat | 11 | **B+C** | `:1787-1794` 调 `group_chat::update` 并处理 effects;`:4723-4794` `group_chat_command`/`group_chat_effects` 把面板的 `Command`/`Effect` 在 host 里执行 | E3-014 |
| `app/app.rs` | agent_context | 9 | A+测试 | 多为 `mod tests`(`:4647+`)里对 `strip_reserved_height` 的断言;生产引用见 `app/layout.rs:1208,1231` | E3-015 |
| `workspace/state.rs` | toast | 8 | 例外 | host 基础设施 | — |
| `app/app.rs` | group_chat | 7 | A+B | `Message::GroupChat`、字段 `group_chat_panel_visible`(`:1634`)按面板名判断可见 | E3-008 |
| `app/app.rs` | todo | 7 | A+B | `todo_webview: todo::WebviewPushState`(App 字段)、`todo` 面板的 webview 期望几何(`:3515`) | E3-002 |
| `platform/window_events.rs` | browser | 6 | **B** | `:2141-2142`、`:2295-2300` 拦截 `Message::Browser(OpenUrl/SelectTab/Nav)` | E3-011 |
| `platform/window_events.rs` | project | 6 | **B** | `:2258-2263` 构造 `project::Message::LinkAdd{target,path,kind}`,用 `project::links::LinkKind` | E3-011 |
| `app/app.rs` | codehealth / usage | 4 / 4 | A+B | `Message` 包装 + `:3456-3487` 各一段 webview 几何 | E3-002 |
| `app/update.rs` | agent_context | 4 | A | `:3266 request_add`、`:5478-5544` 刷新与应用;面板自有函数被 host 编排 | — |
| `app/update.rs` | edit_history | 4 | A | edit_history 是弹窗类 extension(`Option<State>`),host 开关 | — |
| `platform/window_events.rs` | project_create | 4 | **B** | `:2201-2216` host 做 rfd 目录选择再回投 `LocalRootDirPicked/CloneRootDirPicked` | E3-011 |
| `platform/window_events.rs` | todo | 4 | **B** | `:565-577` `todo::Message::CategoryDragRelease(app.last_cursor)` | E3-010 |
| `workspace/view.rs` | project_create | 4 | A | 工作区 view 渲染创建项目入口 | — |

## 3. A 类的量:host 持有多少面板状态

- `App`(`app/app.rs`)直接持有的面板/弹窗类状态字段 **15 个**(评审后订正,原误写约 8 个):`files: files::AppState`、`database: database::AppState`、`git_log`、`home_browser`、`project_link_menu`、四个 `*_webview` 推送状态(`usage_webview`、`codehealth_webview`、`todo_webview`、`group_chat_webview`)、`footbar`、`toast`,以及 `Option<…>` 形式的 `file_history`、`edit_history`、`project_create`、`settings`。
- `Workspace`(`workspace/state.rs`)的 `pub struct Workspace` 里按项目持有的面板状态 **14 个**:`browser`、`agent_context`、`conversations`、`usage`、`codehealth`、`project_panel`、`files`、`todo`、`group_chat`、`database`、`ssh`(+`ssh_active`、`sftp_tabs`)、`search`。
- `Message`(`app/message.rs`)里面板消息直接包装变体 **20 个**(评审后订正,原误写 16 个;另有 5 个面板 webview 事件/外壳消息变体 `UsageContentWebviewEvent`、`CodeHealthContentWebviewEvent`、`TodoContentWebviewEvent`、`GroupChatContentWebviewEvent`、`GroupChatShell`):`ProjectCreate/HomeBrowser/FileHistory/EditHistory/AgentContext(i64, …)` 加上`Usage/CodeHealth/Conversations/Todo/GroupChat/Database/Search/Browser/GitLog/AgentContext/Files/Project/Ssh/Footbar/Toast/Settings` 等。

这三处是 host 对面板最直接的**编译期耦合面**:每新增一个面板,要同时改 `Workspace` 字段、`Message` 变体、`App::update` 的转发臂。迁移后它们应变成按注册 id 索引的 state 容器与统一的消息信封(`Message::Panel(panel_id, Box<dyn Any>)` 一类,**具体形态不在 H0 设计**)。

## 4. Q13 专节:host 对 `git_log` 的直接引用

`app/update.rs:2846-2975` 与 `app/state.rs:11,261`:

| 位置 | 内容 | 分类 | 经 registry 需要什么 |
|---|---|---|---|
| `app/update.rs:2846-2858` | 拦截 `GitLog(ColumnDragStart / RowDragStart / BranchPickerOpen)` 做拖拽/弹出 | B | 面板返回 Effect(`BeginDrag`、`OpenBranchPicker`),host 执行 |
| `app/update.rs:2864-2871`、`:2910-2917` | **host 调 `git_log::load_branch_picker_data(&repo_path)` 并用 `git_log::update(...BranchesLoaded...)` 回灌**(macOS 同步路径、非 macOS 走 `spawn_blocking`) | **C** | 分支数据加载是面板自己的事;面板返回"需要分支列表"Effect,由 host 提供的 spawn_blocking 执行器运行 |
| `app/update.rs:2922-2941` | `BranchSwitch` → host 调 bytegit 切换并回投 `BranchSwitchDone(result)` | C | 同上(写操作 Effect) |
| `app/update.rs:2960-2975` | `GitLog(msg)` 通用转发之后,**host 判断 `BranchSwitchDone(Ok)` 并调用 `git_log::request_refresh`** | **C** | "切换成功后重拉列表"是面板自己的状态机,不该由 host 判断 |
| `app/state.rs:11,261` | `HoverId::GitFileFilter(FileFilter)` 引用 `git_log::FileFilter` | B | 随 `HoverId` 命名空间化(E3-016) |

**结论:** Q13 的答案是**是,必须经 registry**——而且比"调用 `git_log::update`"更深:host 在分支选择器里替面板做了数据加载、写操作和切换后刷新,这是 C 类越界。这几处进 E3-012、E3-013。迁移到 Effect 机制后,`git_log` 对 host 的依赖只剩"提交 Effect"。

## 5. 对 E3 清单的口径说明

- E3 初版 **24 条:17 条 `panel-special-case`、7 条 `parked-service`**;Task 6 回填 `parked-service` 的移除条件并追加 14 条 `overlay`,共 **38 条**(见 `05-platform-overlays.md`)。
- 一条登记 = 一处**可独立移除**的单元(一个函数、一段 match、一个类型)。同一个函数里对 N 个面板的特判算一条,"对象"列列出 N 个面板。
- `parked-service` 的"移除条件"目前写的是**触发条件**(什么情况下必须有定论),候选最终去向由 `04-shared-modules.md`(Task 6)裁决后回填。
- **门禁形式留 H1:** H0 只做 `App`/`Workspace` 引用的棘轮(Task 8);"清单外无新增特判"的门禁需要先有稳定的特判识别规则(如 `PanelKind::X` 出现在 `extensions/` 之外),在 H1 设计。
