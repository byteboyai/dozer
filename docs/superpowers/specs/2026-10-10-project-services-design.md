# Project 面板迁移到服务 trait(第三个面板)

> 日期:2026-10-10。状态:待用户审阅。依据:`2026-10-09-dozer-services-x0-design.md`(接缝形状、`ServiceError`)、`2026-10-10-todo-services-design.md`(读失败分档口径)、`docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`。

## 1. 已对齐的理解

**目标(沿用 X0/Todo):** 面板可进程内测试/复用——Project 面板取数、写数只依赖小 trait,测试注入内存实现。
**范围(用户已选):** 只迁 `extensions/project.rs` 与 `extensions/project/**`(`update.rs`、`delete.rs`)里的 10 个服务方法,拆成两个 trait:
- `MemoryService`(共享记忆 CRUD,4 个):`list_memories`、`get_memory`、`write_memory`、`delete_memory`;
- `ProjectService`(项目维护,6 个):`rename_project`、`remove_project`、`delete_project_transcripts`、`backfill_project_transcripts`、`submit_summary_batch`、`get_summary_batch`。

**明确不在范围:** `extensions/project_create.rs` 的 `open_project`/`list_projects`(创建/克隆对话框,与 host 打开项目逻辑相连);host 侧其它调用。
**不变量:** 线上协议零变化;写操作失败文本逐字不变(原 `e.to_string()` 等于 `ServiceError.message`)。
**用户已确认的失败口径:** `list_memories` 读失败沿用 Todo 的分档(保留旧列表;`Unavailable` 静默;其余弹带 key 的 Error Toast,相同文本 60 秒内只提示一次);`spawn_scaffold_run` 里被 `let _ =` 吞掉的补录失败改写一条 warn 日志,不弹窗(保持"静默打开项目")。

## 2. 现状(读代码得到)

- 记忆:`request_memories_refresh` 与 `MemoryMutated` 分支用 `list_memories(..).unwrap_or_default()`,失败显示为"空列表"并整体替换 `MemoriesLoaded`;写失败已走 `Err(String)` → `ws_state.error`(面板内字段,本规格不改)。
- 维护:`spawn_scaffold_run`(打开项目静默 ensure)里 `let _ = client.backfill_project_transcripts(..)`;`spawn_repair_run`(用户点"修复项目")做脚手架步骤 → 补录 → 提交总结批次 → 轮询批次进度,已有 `SummaryBackfill{Progress,Failed}` 消息;`submit_name_edit`、`spawn_delete_project` 各一到两个调用。
- 这些入口都以 `dozer_client::Client` 按值或按引用传入;host 调用点:`app/update.rs`(`project::update` 共 3 处、`spawn_scaffold_run` 1 处、`spawn_delete_project` 1 处)与 `app/app.rs`(失焦提交 `submit_name_edit`)。
- Project 面板 `WorkspaceState` 目前**没有** `outbox`;日志来源 `LOG`(`panel, "project"`)已在 `project/update.rs` 声明。
- 返回值类型都已在 `dozer-model`:`MemoryInfo`、`MemoryDetail`、`ProjectInfo`、`SummaryBatchInfo`、`AgentKind`。

## 3. 设计

### 3.1 trait(放进 `dozer-services`,仍只依赖 `dozer-model`)

参数按值传,返回 `BoxFuture<'_, Result<T, ServiceError>>`。

`MemoryService`:

| 方法 | 返回 |
|---|---|
| `list_memories(project_id: i64)` | `Vec<MemoryInfo>` |
| `get_memory(project_id: i64, id: i64)` | `MemoryDetail` |
| `write_memory(project_id: i64, title: String, kind: String, description: String, body: String, actor: String)` | `MemoryDetail` |
| `delete_memory(project_id: i64, id: i64, actor: String)` | `()` |

`ProjectService`:

| 方法 | 返回 |
|---|---|
| `rename_project(id: i64, name: String)` | `Option<ProjectInfo>` |
| `remove_project(id: i64)` | `()` |
| `delete_project_transcripts(cwd: String)` | `u32` |
| `backfill_project_transcripts(cwd: String)` | `u32` |
| `submit_summary_batch(cwd: String, provider: Option<AgentKind>, model: Option<String>)` | `(i64, u32)` |
| `get_summary_batch(batch_id: i64)` | `Option<SummaryBatchInfo>` |

返回值保持 `Client` 原样,因为面板确实使用它们(补录条数、批次 id/进度、`MemoryDetail` 回填详情、`Option<ProjectInfo>` 判"项目不存在")。`Services` 增加 `memory: Arc<dyn MemoryService>`、`project: Arc<dyn ProjectService>`;`Client` 实现两者(调用同名 inherent 方法,`map_err(into_service_error)`),错误文本由 `roundtrip` 已有的 `ServiceError` 保证不变。

### 3.2 面板与 host

- `project::update` 的 `client: &Client` 改为 `memory: &Arc<dyn MemoryService>` 与 `project: &Arc<dyn ProjectService>`(或收成一个借用 `&Services`;具体形态在计划阶段按调用点最少改动定)。
- `spawn_scaffold_run`、`spawn_repair_run`、`submit_name_edit` 改吃 `Arc<dyn ProjectService>`;`spawn_delete_project` 改吃 `Arc<dyn ProjectService>`;`request_memories_refresh`/`on_activate` 改吃 `Arc<dyn MemoryService>`(`on_activate` 用 `ctx.io.services()`)。
- host 的 5 个调用点用 `services_for(&self.client)`(Todo 已引入的 `panel_host::services_for`)。

### 3.3 读失败与 Toast

- 新增 `Message::MemoriesLoadFailed(ServiceErrorKind, String)`:不改 `memories` 列表;`Unavailable` 静默(debug 日志);其余 `outbox.push_keyed(LOG, Error, "记忆加载失败: {message}", "project-memories-load")`,并用 `last_load_failure: Option<(String, Instant)>` 做 60 秒同文本去重(同 Todo 的 `LOAD_FAILURE_TOAST_COOLDOWN`)。
- `WorkspaceState` 增加 `outbox: toast::Outbox` 与 `take_outbox`;`App::drain_outboxes` 加一行排空。路由沿用现有 `Message::Project(..)` 通用路径,**不新增**面板专属臂(`R-HOST-PANEL-ARMS` 不上升)。
- `MemoryDetailLoadFailed(String)` 已存在,保持不变。

### 3.4 补录失败

`spawn_scaffold_run`:`let _ = client.backfill_project_transcripts(..)` 改为 `if let Err(e) = ... { log_warn!(LOG, error = %e, "补录转录失败") }`,仍无条件 emit `ScaffoldDone`。

## 4. 步骤(每步先写失败测试;每步一个提交,Task 4 例外见下)

- **P0 基线:** 记录测试数、clippy 警告数、`R-CLIENT` 中 `project.rs`、`project/update.rs`、`project/delete.rs` 的当前值、门禁输出。
- **P1 两个 trait:** `dozer-services` 契约测试(内存实现、`tokio::spawn` Send 检查、读失败可与空列表区分)。
- **P2 `Client` 实现两个 trait:** 回放服务端适配测试(成功、`HangUp → Unavailable "daemon 断开"`、`ErrorReply → Rejected "daemon 错误: boom"`、返回非 `()` 的写操作取到值)。
- **P3 `Services.project`/`memory` + 假实现:** 更新 `services_for`;进程内 `FakeMemory`/`FakeProject`(记录调用、可预设成功值或失败)。
- **P4 面板与 host 切换:** `update` 签名是原子改动,读/写/维护一次切换(同 Todo 的 Ruling)。面板层测试:`list_memories` 读失败保留旧列表、`Unavailable` 静默、`Rejected` 弹且 60 秒内只弹一次;写失败文本不变;`rename_project` 返回 `None` → "项目不存在";`spawn_repair_run` 假实现跑通成功、批次不存在、`get_summary_batch` 失败三条路径;`spawn_scaffold_run` 补录失败仍 emit `ScaffoldDone`;`spawn_delete_project` 取消登记失败时不跑后续步骤。
- **P5 门禁与验收:** `R-CLIENT` 基线中 `project*` 条目清零(只下调,diff 只含这些条目);回填 `08-client-use-inventory.md`、`CLAUDE.md` 的 `dozer-services` 行(Usage、Todo、Project 已迁)。

## 5. 验收标准(机器可验证)

1. `cargo tree -p dozer-services --edges normal` 仍只有 `dozer-model` 与 serde 系。
2. `extensions/project.rs` 与 `extensions/project/**` 不再含 `dozer_client` / `io.client()`;`R-CLIENT` 在这些文件为 0,基线只降。
3. `list_memories` 读失败不清空列表;`Unavailable` 不弹 Toast;`Rejected`/`Protocol` 恰弹一条且 60 秒内不重复;均有测试。
4. 写操作与维护操作失败文本逐字不变;有测试。
5. 全量测试数 ≥ 基线 + 新增;已知失败(`files::delete_confirm_spec_reflects_pending_target`、间歇的 `git_log::build_*`、`R-HOST-EXECUTORS 0->1`)不算回归也不顺手修。
6. `R-HOST-PANEL-ARMS` 不上升;clippy 警告数不高于基线;`cargo fmt --check`、`scripts/check-log-scope.sh` 通过;`model_golden` 仍通过。

## 6. 明确不做

- 不迁 `project_create.rs`(`open_project`、`list_projects`)与 host 侧其它调用。
- 不改 `ws_state.error`(记忆写失败的面板内字段)与 `MemoryDetailLoadFailed`。
- 不给 `MemoriesLoaded`/`MemoryDetailLoaded` 补项目/代数保护。
- 不对齐 Usage 的失败口径。
- 不改线上协议,不动 `PanelIo::client()`,不迁其它面板。

## 7. 风险与已知问题

- **`spawn_repair_run` 是长流程**(脚手架 → 补录 → 提交批次 → 轮询):切换后行为必须逐条等价。用假实现覆盖成功/批次不存在/查询失败,`SummaryBackfill*` 消息语义不变。
- **`update` 签名变动面广:** `app/update.rs` 3 处 + 其它 3 个 host 入口。P4 一个提交内完成,靠测试与全量测试兜底。
- **探针行号漂移:** `app/update.rs` 增删行会让 `usage_probe/host.rs` 的 `host_site` 引用漂移,按前两次的做法只校正行号。
- **`project.rs` 的测试用 `dozer_client::Client::new(..)`(约第 567、1243 行)作占位**:切换后改为 `FakeProject`/`FakeMemory`,否则 `R-CLIENT` 清不到 0。
- **WIP 文件:** 用户 WIP 在 main 工作区未提交;在独立 worktree 实施,不提交 `Info.plist` 与版本号改动,`Cargo.lock` 剔除 bytegit/byteui 的 `source =` 行。

## 8. 分支与执行

规格分支 `spec/project-services`(worktree `../dozer-project-spec`);实施另开分支。执行计划由 writing-plans 产出,待本规格审阅通过后进行;合并进 main 前需询问用户。
