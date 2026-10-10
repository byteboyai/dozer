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
