# AgentContext 面板迁移到服务 trait(第七个面板)

> 日期:2026-10-11。状态:待用户审阅。依据:`2026-10-09-dozer-services-x0-design.md`(接缝形状、`ServiceError`)、`2026-10-11-groupchat-services-design.md`(同为"失败本来就可见"的纯接缝迁移先例)、`docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`。

## 1. 已对齐的理解

**目标(沿用前几个面板):** 终端下方的 Agent 上下文条(`extensions/agent_context.rs`)可进程内测试/复用——取数/写数只依赖小 trait,测试注入内存实现。
**范围:** 只迁该文件里的 3 个服务调用:`list_context_items`、`add_context_item`、`remove_context_item`,以及 host 的 3 个调用点(`app/update.rs`)。
**明确不在范围:** `extensions/edit_history.rs`(修改历史弹窗,另一个独立面板,`apply_precise_edit`/`list_file_edit_history`);其它面板与 host 调用。
**不变量:** 线上协议零变化;**用户可见行为零变化**——失败本来就可见,不新增 `LoadFailed`、不改 Toast 口径;失败文本逐字不变:"加载上下文列表失败: {e}"、"已发送到终端,但未能记录到上下文列表: {e}"、"移除失败: {e}"。

## 2. 现状(读代码得到)

- 三个异步入口:
  - `request_refresh(project_id, root, client, handle, emit)`:`list_context_items` → 后台线程判定每项在磁盘上是否缺失(`root.join(entity_ref).exists()`)→ `Message::Loaded(Result<Vec<ContextEntry>, String>)`;
  - `request_add(project_id, is_dir, relative, client, handle, emit)`:`add_context_item(project_id, "dir"|"file", relative)` → `Message::Added(Result<ContextItemInfo, String>)`;
  - `update(state, project_id, root, msg, client, handle, emit)`:按 `Followup` 调 `request_refresh` 或 `remove_context_item` → `Message::Removed(Result<(), String>)`。
- 失败已可见且行为正确:`Loaded(Err)` → Error Toast 且**不替换**已有列表(已有测试 `loaded_ok_replaces_items_and_err_pushes_to_outbox` 钉住);`Added(Err)` → Warning Toast(终端粘贴已发生,只是记录失败);`Removed(Err)` → Error Toast;`Added`/`Removed` 之后一律 `Followup::Refresh`。
- 刷新只在切入项目与增删之后触发,**不轮询**,所以不需要冷却。
- host 调用点(`app/update.rs`):`agent_context_message`(把 `update` 转发给面板)、`agent_context_refresh`(切入项目时刷新)、文件树右键"添加到 Agent 上下文"(直接调 `request_add`)。
- 日志来源 `LOG`(`panel, "agent"`)已声明,`agent` 是现有的 `PanelKind` 名,不改。
- `R-CLIENT` 基线:`extensions/agent_context.rs` 为 3。

## 3. 设计

### 3.1 `ContextService`(放进 `dozer-services`,仍只依赖 `dozer-model`)

| 方法 | 返回 |
|---|---|
| `list_context_items(project_id: i64)` | `Vec<ContextItemInfo>` |
| `add_context_item(project_id: i64, entity_kind: String, entity_ref: String)` | `ContextItemInfo` |
| `remove_context_item(project_id: i64, id: i64)` | `()` |

参数按值传,返回 `BoxFuture<'_, Result<_, ServiceError>>`;`ContextItemInfo` 已在 `dozer-model`。`Services` 增加 `context: Arc<dyn ContextService>`;`Client` 实现该 trait(调用同名 inherent 方法,`map_err(into_service_error)`)。

### 3.2 面板与 host

- `request_refresh`/`request_add`/`update` 的 `client: &dozer_client::Client` 改为 `service: Arc<dyn ContextService>`(`update` 里按 `Followup` 克隆使用);`.map_err(|e| e.to_string())` → `.map_err(|e| e.message)`(与原文本相同)。`apply`、`Message`、`Followup` 不变。
- host 三处把 `let client = self.client.clone();` 换成 `let context = crate::panel_host::services_for(&self.client).context;`,调用处传 `context`。

## 4. 步骤(每步先写失败测试;每步一个提交)

- **A0 基线:** 记录测试数、clippy 警告数、`R-CLIENT` 中 `agent_context.rs`(3)、门禁输出。
- **A1 `ContextService` trait:** `dozer-services` 契约测试(内存实现、`tokio::spawn` Send 检查、读失败可与空列表区分)。
- **A2 `Client` 实现:** 回放服务端适配测试(`list` 成功、`remove` 返回 `()`、`add` 的 `ContextItemInfo` 原样带回、`HangUp → Unavailable "daemon 断开"`、`ErrorReply → Rejected "daemon 错误: boom"`)。
- **A3 `Services.context` + 假实现:** `services_for` 与 `base_services()` 补字段;进程内 `FakeContext`(预设列表、可让读/写失败、记录调用)。
- **A4 面板与 host 切换:** 面板层测试(用 `FakeContext`):
  - `request_refresh`:用临时目录当项目根,验证 `missing` 判定(文件存在为 `false`、缺失为 `true`),成功得到 `Loaded(Ok)`;服务失败得到 `Loaded(Err(text))`,文本逐字为服务消息。
  - `request_add`:`is_dir=true` 传 `"dir"`、`false` 传 `"file"`,`relative` 原样;失败得到 `Added(Err(text))`。
  - `update` 的 `Followup::RemoveItem`:调 `remove_context_item(project_id, id)`;失败得到 `Removed(Err(text))`;`Followup::Refresh` 触发 `list`。
  - 现有 `apply` 的纯状态测试保持不动。
  - 探针 `host_site` 行号如因 rustfmt 折行漂移,只改行号。
- **A5 门禁与验收:** `R-CLIENT` 基线中 `agent_context.rs` 清零;回填 `08-client-use-inventory.md`、`CLAUDE.md` 的 `dozer-services` 行(Usage、Todo、Project、GroupChat、Conversations、CodeHealth、AgentContext 已迁)。

## 5. 验收标准(机器可验证)

1. `cargo tree -p dozer-services --edges normal` 仍只有 `dozer-model` 与 serde 系。
2. `extensions/agent_context.rs` 不再含 `dozer_client` / `io.client()`;`R-CLIENT` 在该文件为 0,基线只降。
3. `missing` 判定、`add` 的 kind/路径传递、三类失败文本逐字不变,均有测试(A4)。
4. 全量测试数 ≥ 基线 + 新增;已知失败(`files::delete_confirm_spec_reflects_pending_target`、间歇的 `git_log::build_*`、dozerd 负载下的间歇失败、`R-HOST-EXECUTORS 0->1`)不算回归也不顺手修。
5. `R-HOST-PANEL-ARMS` 不上升;clippy 警告数不高于基线;`cargo fmt --check`、`scripts/check-log-scope.sh` 通过;`model_golden` 仍通过。

## 6. 明确不做

- 不迁 `edit_history`;不改任何用户可见行为(不新增 `LoadFailed`、不改 Toast 口径、不加冷却)。
- 不给 `Loaded` 补代数保护;不对齐 Usage 的失败口径;不改线上协议。

## 7. 风险与已知问题

- **host 三处替换基本一对一:** 行数应基本不变,探针 `host_site` 预计不漂移;若 rustfmt 折行导致漂移,只改行号。
- **`update` 的 `emit` 需 `Clone`:** 签名里的 `emit: impl Fn(Message) + Clone + Send + 'static` 保持不变(`Refresh` 与 `RemoveItem` 各 spawn 一次)。
- **`Cargo.lock` 噪声:** main 上已提交的锁文件缺 bytegit/byteui 的 `source` 行,干净 worktree 里跑 cargo 会补上;提交前 `git checkout Cargo.lock`(同前几次)。
- **WIP 文件:** 用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist`)在 main 工作区未提交;在独立 worktree 实施,不提交 `Info.plist` 与版本号改动。

## 8. 分支与执行

规格分支 `spec/context-services`(worktree `../dozer-context-spec`);实施另开分支。执行计划由 writing-plans 产出,待本规格审阅通过后进行;合并进 main 前需询问用户。
