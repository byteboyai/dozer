# EditHistory 面板迁移到服务 trait(第八个面板)

> 日期:2026-10-11。状态:待用户审阅。依据:`2026-10-09-dozer-services-x0-design.md`(接缝形状、`ServiceError`)、`2026-10-11-context-services-design.md`(同为"失败本来就可见"的纯接缝迁移先例)、`docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`。

## 1. 已对齐的理解

**目标(沿用前几个面板):** Agent 修改历史弹窗(`extensions/edit_history.rs`)可进程内测试/复用——取数/写数只依赖小 trait,测试注入内存实现。
**范围:** 只迁该文件里的 2 个服务调用:`list_file_edit_history`(`request_load`)与 `apply_precise_edit`(撤销流程),以及 host 的 2 个调用点(`app/update.rs` 的 `open_edit_history`、`edit_history_message` 的 `other` 分支)。
**明确不在范围:** `browser`、`settings`、`project_create`;其它 host 调用(`dozer-mcp` 等用 `Client` 的 inherent 方法,不受影响)。
**不变量:** 线上协议零变化;**用户可见行为零变化**——失败本来就可见,不新增 `LoadFailed`、不改展示口径。撤销时传给服务的内容逐字不变:`expected_text = 该条 new_text`、`new_text = 该条 old_text`、坐标取该条修改后区间(起点 `start_*`、结束 `new_end_*`)、`summary = "撤销:{summary}"`、`actor = "dozer"`、`session_id = "dozer-gui"`。

## 2. 现状(读代码得到)

- 两个异步入口:
  - `request_load(project_id, filter, client, handle, emit)`:`list_file_edit_history(project_id, filter.as_deref(), DEFAULT_LIMIT)`(`DEFAULT_LIMIT = 200`)→ `Message::Loaded { project_id, filter, result: Result<Vec<FileEditHistoryInfo>, String> }`;
  - `update(state: &mut Option<State>, msg, client, handle, emit)`:按 `Followup` 调 `request_load`(`Reload`)或撤销(`Revert(entry)`,调 `apply_precise_edit` 后发 `Message::RevertDone { id, result: Result<MutationOutcome, String> }`)。
- 失败已可见:加载结果进 `State.entries: Option<Result<Vec<_>, String>>`(`None` = 加载中,`Some(Err)` 在弹窗里显示);撤销结果进 `RevertStatus::{Pending, Done, Failed(原因)}` 并内联显示(`MutationOutcome` 的非 `Applied` 变体经 `revert_failure` 转成原因);迟到的消息按 `project_id`/`filter` 比对后丢弃。状态是 `Option<State>`,**没有 outbox**,也不轮询,不需要冷却。
- `apply_precise_edit` 的 inherent 签名有 **11 个参数**(连续 4 个 `u32` 坐标、多个 `&str`),顺序传错编译器发现不了。
- 日志:本文件未声明 `LOG`;迁移不新增日志。
- `R-CLIENT` 基线:`extensions/edit_history.rs` 为 2(`request_load` 与 `update` 的 `&dozer_client::Client` 参数类型)。

## 3. 设计

### 3.1 `EditHistoryService`(放进 `dozer-services`,仍只依赖 `dozer-model`)

| 方法 | 返回 |
|---|---|
| `list_file_edit_history(project_id: i64, path_filter: Option<String>, limit: u32)` | `Vec<FileEditHistoryInfo>` |
| `apply_precise_edit(edit: PreciseEdit)` | `MutationOutcome` |

参数按值传,返回 `BoxFuture<'_, Result<_, ServiceError>>`。`FileEditHistoryInfo`、`MutationOutcome` 已在 `dozer-model`。

**`PreciseEdit` 参数结构体**(仓库约定:参数 ≥7 且多个同类型参数相邻时用具名字段结构体,CLAUDE.md "关键裁决"):放在 `dozer-services`(它是调用参数而不是线上形状,所以不进 `dozer-model`),字段 `project_id: i64`、`path: String`、`start_line: u32`、`start_col: u32`、`end_line: u32`、`end_col: u32`、`expected_text: String`、`new_text: String`、`summary: String`、`actor: String`、`session_id: String`;派生 `Debug, Clone, PartialEq`。`Client` 的 `impl` 解构它,调用原来的 11 参数 inherent 方法(inherent 方法不改,`dozer-mcp` 等不受影响)。

`Services` 增加 `edit_history: Arc<dyn EditHistoryService>`;`Client` 实现该 trait(调用同名 inherent 方法,`map_err(into_service_error)`),错误文本由 `roundtrip` 已有的 `ServiceError` 保证不变。

### 3.2 面板与 host

- `request_load`/`update` 的 `client: &dozer_client::Client` 改为 `service: Arc<dyn EditHistoryService>`;`.map_err(|e| e.to_string())` / `.map_err(|err| err.to_string())` → `.map_err(|e| e.message)`(与原文本相同)。`apply`、`Message`、`Followup`、`RevertStatus` 不变。
- 撤销:`Followup::Revert(e)` 里构造 `PreciseEdit { project_id: e.project_id, path: e.target_path, start_line: e.start_line, start_col: e.start_col, end_line: e.new_end_line, end_col: e.new_end_col, expected_text: e.new_text, new_text: e.old_text, summary: format!("撤销:{}", e.summary), actor: "dozer".into(), session_id: "dozer-gui".into() }`(与现在 11 个位置参数一一对应)。
- host 两处把 `let client = self.client.clone();` 换成 `let edit_history = crate::panel_host::services_for(&self.client).edit_history;`,调用处传 `edit_history`。

## 4. 步骤(每步先写失败测试;每步一个提交)

- **E0 基线:** 记录测试数、clippy 警告数、`R-CLIENT` 中 `edit_history.rs`(2)、门禁输出。
- **E1 `EditHistoryService` + `PreciseEdit`:** `dozer-services` 契约测试(内存实现、`tokio::spawn` Send 检查、读失败可与空列表区分、`PreciseEdit` 可克隆比较)。
- **E2 `Client` 实现:** 回放服务端适配测试。除了常规的成功/`HangUp → Unavailable "daemon 断开"`/`ErrorReply → Rejected "daemon 错误: boom"` 之外,**需要一个能捕获请求行的回放服务端**(现有 `serve_once` 读一行后丢弃),用它断言 `apply_precise_edit` 发出的 `Request::ApplyPreciseEdit` 的 11 个字段与 `PreciseEdit` 一一对应(防止解构时字段错位)。
- **E3 `Services.edit_history` + 假实现:** `services_for` 与 `base_services()` 补字段;进程内 `FakeEditHistory`(预设列表与撤销结果、可让读/写失败、记录收到的列表参数与 `PreciseEdit`)。
- **E4 面板与 host 切换:** 面板层测试(用 `FakeEditHistory`):
  - `request_load`:`filter` 与 `DEFAULT_LIMIT`(200)原样传给服务;成功得到 `Loaded { result: Ok }`;服务失败得到 `Loaded { result: Err(服务消息) }`。
  - 撤销:`update(Some(state), Message::Revert(id), …)` 后服务收到的 `PreciseEdit` 与 §1 的规则逐字一致;成功得到 `RevertDone { result: Ok(outcome) }`;失败得到 `RevertDone { result: Err(服务消息) }`。
  - `RevertDone(Ok(Applied…))` 之后 `Followup::Reload` 触发一次 `list`。
  - 现有 `apply` 纯状态测试保持不动。
- **E5 门禁与验收:** `R-CLIENT` 基线中 `edit_history.rs` 清零;回填 `08-client-use-inventory.md`、`CLAUDE.md` 的 `dozer-services` 行(Usage、Todo、Project、GroupChat、Conversations、CodeHealth、AgentContext、EditHistory 已迁)。

## 5. 验收标准(机器可验证)

1. `cargo tree -p dozer-services --edges normal` 仍只有 `dozer-model` 与 serde 系。
2. `extensions/edit_history.rs` 不再含 `dozer_client` / `io.client()`;`R-CLIENT` 在该文件为 0,基线只降。
3. 加载参数传递、撤销的 `PreciseEdit` 逐字规则、失败文本,以及 `Client` 适配层 11 个字段不错位,均有测试(E2/E4)。
4. 全量测试数 ≥ 基线 + 新增;已知失败(`files::delete_confirm_spec_reflects_pending_target`、间歇的 `git_log::build_*`、`assets::tests::serves_vendored_asset_with_mime` 与 dozerd 在负载下的间歇失败、`R-HOST-EXECUTORS 0->1`)不算回归也不顺手修。
5. `R-HOST-PANEL-ARMS` 不上升;clippy 警告数不高于基线;`cargo fmt --check`、`scripts/check-log-scope.sh` 通过;`model_golden` 仍通过。

## 6. 明确不做

- 不改任何用户可见行为(不新增 `LoadFailed`、不改弹窗展示、不加冷却、不加 outbox)。
- 不迁 `browser`/`settings`/`project_create`;不改 `apply_precise_edit` 的 inherent 签名;不改线上协议。

## 7. 风险与已知问题

- **解构错位:** `PreciseEdit` → 11 个位置参数的映射是本次最容易出错的地方(连续 `u32`、多个 `String`);E2 的"捕获请求行"测试专门防它。
- **host 两处替换基本一对一:** 行数应基本不变,探针 `host_site` 预计不漂移;若 rustfmt 折行导致漂移,只改行号。
- **`Cargo.lock` 噪声:** main 上已提交的锁文件缺 bytegit/byteui 的 `source` 行,干净 worktree 里跑 cargo 会补上;提交前 `git checkout Cargo.lock`(同前几次)。
- **WIP 文件:** 用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist`)在 main 工作区未提交;在独立 worktree 实施,不提交 `Info.plist` 与版本号改动。

## 8. 分支与执行

规格分支 `spec/edit-history-services`(worktree `../dozer-edithistory-spec`);实施另开分支。执行计划由 writing-plans 产出,待本规格审阅通过后进行;合并进 main 前需询问用户。
