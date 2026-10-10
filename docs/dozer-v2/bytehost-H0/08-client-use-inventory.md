# 08 — 面板对 `Client` 方法的使用盘点(X0 S0)

> 日期:2026-10-09。表由 `scripts/audit/client_uses.py` 生成(按 `dozer-client/src/lib.rs` 的 `pub fn` 名匹配 `.method(`,识别换行书写,忽略注释与字符串)。
> 注意:`list`/`write`/`resize`/`attach`/`kill` 等通用名会与 `Vec`/`io`/终端对象误撞,属噪声;以 `Client` 专有名为准。

## 基线

- `extensions/**` 引用 `dozer_client` 的文件数:14
- 门禁:仅 `R-HOST-EXECUTORS 0 -> 1`(main 既有失败)
- 测试/clippy 基线见 ledger。

## 使用表

| 文件 | 方法 |
|---|---|
| `panel_host.rs` | list_groups |
| `runtime.rs` | list |
| `app/app.rs` | list, list_projects |
| `app/update.rs` | assign_todo_agent, get_summary_job, get_todo_detail, kill, list_conversations_with_summaries, list_projects, open_project, process_todo_now, report_preview_command_outcome, submit_summary_job, write |
| `assets/fonts.rs` | write |
| `chrome/homespace.rs` | list_conversations |
| `extensions/agent_context.rs` | add_context_item, list_context_items, remove_context_item |
| `extensions/conversations.rs` | list_conversations_with_summaries |
| `extensions/edit_history.rs` | apply_precise_edit, list_file_edit_history |
| `extensions/project.rs` | list |
| `extensions/settings.rs` | list, shutdown_daemon |
| `extensions/codehealth/aggregate.rs` | get_code_health_report, list_code_health_reports, save_code_health_report |
| `extensions/group_chat/mod.rs` | add_group_member, cancel_group, create_group, delete_group, list_group_messages, list_groups, post_group_message, push_group_message_to_todo, remove_group_member, retry_group_message, update_group_member |
| `extensions/project/delete.rs` | delete_project_transcripts, remove_project |
| `extensions/project/update.rs` | backfill_project_transcripts, delete_memory, get_memory, get_summary_batch, list, list_memories, rename_project, submit_summary_batch, write_memory |
| `extensions/project/view.rs` | list |
| `extensions/ssh/sftp.rs` | write |
| `extensions/todo/state.rs` | list_categories, list_todos |
| `extensions/todo/update.rs` | add_category, add_todo, delete_category, edit_todo_text, list_todos, move_category_sibling, rename_category, reorder_todo, reparent_category, set_todo_category, set_todo_plan_date, set_todo_status, toggle_todo |
| `extensions/usage/mod.rs` | get_usage_summary |
| `platform/window_events.rs` | write |
| `preview/text_save.rs` | write |
| `tabular/mod.rs` | resize |
| `term/term_model.rs` | resize |
| `usage_probe/fake_dozerd.rs` | get_usage_summary |
| `usage_probe/host.rs` | attach |
| `usage_probe/scenarios.rs` | attach |
| `usage_probe/surface.rs` | attach |
| `workspace/hook.rs` | attach, list, list_projects |
| `workspace/state.rs` | attach, close_with_summary, create_for_agent, get_conversation_turns, kill, resize, take_pending_preview_commands, update_preview_context, write |

## host 侧(`extensions/**` 之外)

`panel_host.rs`、`runtime.rs`、`app/app.rs`、`app/update.rs`、`chrome/homespace.rs`、`workspace/hook.rs`、`workspace/state.rs`:调用 `list_projects`、`open_project`、`list_conversations*`、`submit/get_summary_job`、`assign_todo_agent`、`process_todo_now`、`report_preview_command_outcome`、终端流(`attach`/`write`/`resize`/`kill`)等。这些属 host 职责,本轮不抽。

## 能力分组(对照规格 §4)

| trait(规格 §4) | 盘点结果 | 差异 |
|---|---|---|
| `UsageService` | `usage/mod.rs`:`get_usage_summary` | 一致(本轮实现) |
| `TodoService` | `todo/state.rs`、`todo/update.rs`:`list/add/delete/rename/reparent/move_category_*`、`*_todo*`、`set_todo_*`、`reorder_todo` | 规格列的是子集,实际远多于 `list_todos/list_categories/delete_category` |
| `GroupChatService` | `group_chat/mod.rs`:另含 `add_group_member`、`update_group_member`、`list_groups`、`list_group_messages`、`post_group_message` | 规格漏列这几个 |
| `ProjectService` | `project/*`:另含 `rename_project`、`list_memories`、`write_memory`、`delete_memory`、`submit_summary_batch` | 规格漏列 |
| `BookmarkService`/`ContextService` | `agent_context.rs`:`*_context_item`;未见 `extensions/` 内的 bookmark 调用(浏览器收藏夹另有路径) | bookmark 在本盘点范围内未出现 |
| (规格未列) | `edit_history.rs`:`apply_precise_edit`、`list_file_edit_history`;`codehealth/aggregate.rs`:`*_code_health_report*`;`conversations.rs`:`list_conversations_with_summaries`;`settings.rs`:`shutdown_daemon` | 需新增 `EditHistoryService`、`CodeHealthService`、`ConversationService` 分组;`shutdown_daemon` 按规格不抽 |
| 不抽 | `database`、终端流 | 一致 |

以盘点为准;分组修订只作登记,不改本轮范围(仅 Usage)。

## Todo 迁移(2026-10-10)

`extensions/todo/**` 已经 `TodoService`(`dozer-services`)取数与写入;`R-CLIENT` 基线中 `todo/*` 已清零。读失败改为 `LoadFailed`:保留旧列表,`Unavailable` 静默,`Rejected`/`Protocol` 弹 `todo-load` Toast。`add_category` 返回 `CategoryInfo`(面板要用新分类 id 进入重命名),其余写操作返回 `()`。规格 `docs/superpowers/specs/2026-10-10-todo-services-design.md`。

**后续项(未修):** `todo::Message::Loaded` / `CategoriesLoaded` 没有 project_id 与代数保护(同 Usage 已修的 G3b),慢的旧结果可能覆盖新结果。

## Project 迁移(2026-10-10)

`extensions/project.rs` 与 `extensions/project/**` 已经 `MemoryService`/`ProjectService`(`dozer-services`)取数与写入;`R-CLIENT` 基线中 `project*` 已清零。记忆读失败改为 `MemoriesLoadFailed`:保留旧列表,`Unavailable` 静默,其余弹 `project-memories-load` Toast(同文本 60 秒内一次);`spawn_scaffold_run` 的补录失败改写 warn 日志。规格 `docs/superpowers/specs/2026-10-10-project-services-design.md`。

**后续项(未修):**
- `MemoriesLoaded` / `MemoryDetailLoaded` 没有项目/代数保护(同 Usage 已修的 G3b)。
- 读失败冷却逻辑在 Todo 与 Project 各有一份(`LOAD_FAILURE_TOAST_COOLDOWN`),可抽到 `toast::Outbox`。
- `extensions/project_create.rs` 仍直接用 `Client`(`open_project`/`list_projects`)。

## GroupChat 迁移(2026-10-11)

`extensions/group_chat/**` 已经 `GroupChatService`(`dozer-services`)取数与写入;`R-CLIENT` 基线中 `group_chat/mod.rs` 已清零。用户可见行为零变化(群聊的读写失败本来就可见,不新增 `LoadFailed`);`PanelIo::spawn` 的闭包改拿 `Services`。规格 `docs/superpowers/specs/2026-10-11-groupchat-services-design.md`。

**后续项(未修):**
- `GroupsLoaded` 没有项目/代数保护(`Polled` 靠 `selected` 比对)。
- 剩余直接使用 `Client` 的面板:`conversations`、`codehealth`、`agent_context`、`browser`、`edit_history`、`settings`、`project_create`。

## Conversations 迁移(2026-10-11)

`extensions/conversations.rs` 已经 `ConversationService`(`dozer-services`)取数;`R-CLIENT` 基线中 `conversations.rs` 已清零。刷新失败改为分档:已有列表保留(此前会整体清空)、首次失败仍显示空列表(不卡在"加载中")、`Unavailable` 静默、其余弹 `conversations-load` Toast(同文本 60 秒内一次)。host 的 `conversation_summary_generate` 里那一次列表刷新调用随 `SessionsRefreshed` 的结果类型一起改用服务。规格 `docs/superpowers/specs/2026-10-11-conversation-services-design.md`。

**后续项(未修):**
- `SessionsRefreshed` 没有代数保护(慢的旧结果可能覆盖新结果)。
- 读失败冷却逻辑在 Todo / Project / Conversations 各有一份(`LOAD_FAILURE_TOAST_COOLDOWN`),可抽到 `toast::Outbox`。
- host 侧其余会话调用(打开、翻页、总结任务、homespace、hook)仍直接用 `Client`。
- 剩余直接用 `Client` 的面板:`codehealth`、`agent_context`、`browser`、`edit_history`、`settings`、`project_create`。

## CodeHealth 迁移(2026-10-11)

`extensions/codehealth/**` 已经 `CodeHealthService`(`dozer-services`)取数与保存;`R-CLIENT` 基线中 `codehealth/aggregate.rs` 已清零。缓存读取失败改为 `LoadFailed`(携带无报告的 panel):已有报告保留、首次失败采用携带的 panel、`Unavailable` 静默、其余弹 `codehealth-load` Toast(同文本 60 秒内一次)。历史读取失败改为写 warn 日志(对比降级为无基线,扫描照常);保存失败的内联提示文本不变。面板日志来源名为 `code_health`。`PanelIo::client()` 随最后一个使用者迁走而删除(`PanelIo::new` 签名不变)。规格 `docs/superpowers/specs/2026-10-11-codehealth-services-design.md`。

**后续项(未修):**
- `Loaded` / `Scanned` 没有代数保护(慢的旧结果可能覆盖新结果)。
- 读失败冷却逻辑在 Todo / Project / Conversations / CodeHealth 各有一份(`LOAD_FAILURE_TOAST_COOLDOWN`),可抽到 `toast::Outbox`。
- 剩余直接使用 `Client` 的面板:`agent_context`、`browser`、`edit_history`、`settings`、`project_create`。
