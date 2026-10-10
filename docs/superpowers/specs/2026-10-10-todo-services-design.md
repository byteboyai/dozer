# Todo 面板迁移到服务 trait(X0 之后第二个面板)

> 日期:2026-10-10。状态:待用户审阅。依据:`2026-10-09-dozer-services-x0-design.md`(接缝形状、`ServiceError`)、`docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`(Todo 的方法清单)。

## 1. 已对齐的理解

**目标(沿用 X0):** 面板可进程内测试/复用——Todo 面板取数、写数只依赖小 trait,测试注入内存实现,不再需要假 UDS。
**范围(用户已选):** 只迁 `extensions/todo/**` 里的读写:2 个读 + 13 个写。host 侧的 Todo 调用(`get_todo_detail`、`assign_todo_agent`、`process_todo_now`、`todo_detail_process`,在 `app/update.rs`)牵涉终端会话与派发,**不动**。
**不变量:** 线上协议零变化;写操作失败今天已可见(`Mutated(Err)`),保持原样。

**用户已确认的两个口径:**
1. 读失败分档:保留已有列表;`Unavailable` 静默(顶栏徽标已表达"dozerd 不可用"),`Rejected`/`Protocol` 才弹带 key 的 Error Toast。
2. 写操作的 trait 方法返回 `()`(面板本来就丢弃 `TodoInfo`/`CategoryInfo`,成功后靠重新拉列表)。

## 2. 现状(读代码得到)

- 读:`todo/state.rs` 的 `request_todos_refresh`、`request_categories_refresh` 用 `unwrap_or_default()` 吞错——失败显示为"空列表",并且**会抹掉已有列表**(`Loaded(vec![])` 整体替换 `items`)。这是 Usage G4 的同类缺陷。
- 轮询:`App::poll_todo_if_visible`(`app/app.rs`)定时调用同两个函数,所以 dozerd 掉线时会反复失败。
- 写:`todo/update.rs` 约 13 处 `client.<method>(..)`,结果已映射为 `Result<(), String>` 经 `Message::Mutated`/`CategoryMutated` 回传。
- 两个 host 入口都从 `App` 的 `self.client` 拿 `Client`(`app/update.rs` 的 `todo::update(.., &client, ..)`、`app/app.rs` 的轮询);`on_activate` 用 `ctx.io.client()`。
- `Message::Loaded(Vec<TodoInfo>)` 没有 project_id 和代数(与 Usage 不同)。本规格**不**补——属另一条问题,见 §7。

## 3. 设计

### 3.1 `TodoService`(放进 `dozer-services`)

参数按值传(满足 `handle.spawn` 的 `'static`),返回 `BoxFuture<'_, Result<T, ServiceError>>`。方法对应 `Client` 现有签名:

| 方法 | 返回 |
|---|---|
| `list_todos(project_id)` | `Vec<TodoInfo>` |
| `list_categories(project_id)` | `Vec<CategoryInfo>` |
| `add_todo(project_id, text)` | `()` |
| `toggle_todo(id, done)` | `()` |
| `edit_todo_text(id, text)` | `()` |
| `reorder_todo(id, after_id)` | `()` |
| `set_todo_plan_date(id, plan_date)` | `()` |
| `set_todo_status(id, status: TodoStoredStatus)` | `()` |
| `set_todo_category(id, category_id)` | `()` |
| `add_category(project_id, parent_id: Option<i64>, name)` | `()` |
| `rename_category(id, name)` | `()` |
| `delete_category(id)` | `()` |
| `reparent_category(id, new_parent_id: Option<i64>)` | `()` |
| `move_category_sibling(id, direction: CategoryMoveDirection)` | `()` |

`TodoInfo`/`CategoryInfo`/`CategoryMoveDirection`/`TodoStoredStatus` 均已在 `dozer-model`;`dozer-services` 仍只依赖 `dozer-model`。`Services` 增加 `todo: Arc<dyn TodoService>`。`Client` 实现该 trait:调用现有方法并 `.map(|_| ())`、`map_err(into_service_error)`。错误文本即 `ServiceError.message`,与原来 `e.to_string()` 逐字相同(X0 已钉住 `roundtrip` 的 Display)。

### 3.2 面板与 host

- `todo::update` / `request_todos_refresh` / `request_categories_refresh` / `on_activate` 的 `client: &Client` 改为 `&Services`(或其 `todo` 字段的 `Arc<dyn TodoService>`)。
- `App` 构造时持有一份 `Services`(由 `self.client` 适配),供 `app/update.rs` 与轮询使用;`PanelIo::services()` 已存在,`on_activate` 直接用。
- 写操作的错误映射改为 `e.message`(文本不变)。

### 3.3 读失败(G4 的 Todo 版)

新增 `Message::LoadFailed(ServiceErrorKind, String)`(读任务与分类读共用;失败时发它,**不发** `Loaded(vec![])`):
- 面板 `update`:**不动** `items`/`categories`;
- `kind == Unavailable`:静默(只写一条 debug 级面板日志,来源 `todo`);
- `Rejected`/`Protocol`:`outbox.push_keyed(LOG, Error, "任务加载失败: {message}", "todo-load")`。
- `WorkspaceState` 增加 `outbox: toast::Outbox` 与 `take_outbox`;`App::drain_outboxes` 加一行排空。`LoadFailed` 的路由臂并入已有 `Message::Todo(..)` 处理,**不新增**面板专属臂(避免 `R-HOST-PANEL-ARMS` 上升,同 X0 的教训)。

> `ServiceErrorKind` 要能放进面板 `Message`(`Debug + Clone`):已满足(`Debug, Clone, Copy`)。

## 4. 步骤(每步一个提交,每步先写失败测试)

- **T0 基线:** 记录测试数、clippy 警告数、`R-CLIENT` 中 `todo/state.rs`(3)与 `todo/update.rs`(1)的当前值、门禁输出。
- **T1 `TodoService` + `Services.todo`:** `dozer-services` 契约测试(内存实现、`tokio::spawn` Send 检查、`Services` 可克隆);更新 X0 已有的构造 `Services { usage }` 的地方(`PanelIo::new` 默认、`FakeUsage::into_services`、`host.rs` 里的构造)。
- **T2 `Client` 实现 `TodoService`:** `FakeDozerd` 需要能回放 Todo 请求——先看它现在只支持 Usage;计划阶段决定是扩展 `FakeDozerd` 还是对适配用更小的 UDS 夹具。至少覆盖:`list_todos` 成功、`HangUp → Unavailable`、`ErrorReply → Rejected`,以及一个写操作返回 `()`。
- **T3 读路径切换 + 读失败可见:** `FakeTodo` 进程内假实现;面板层测试:读失败保留旧列表、`Unavailable` 不弹 Toast、`Rejected` 弹且只弹一条、过期无关(无代数)。轮询入口也走 `Services`。
- **T4 写路径切换:** `update` 的 13 处写改为 `services.todo`;测试:各类写失败时 `Mutated(Err(text))` 的文本与原来一致。
- **T5 门禁与验收:** `R-CLIENT` 基线中 `todo/*` 降到 0(用 `--update` 仅在下降时重写,并核对 diff 只含 todo 条目);回填 `08-client-use-inventory.md`、`CLAUDE.md` 的 `dozer-services` 行(Usage、Todo 已迁)。

## 5. 验收标准(机器可验证)

1. `cargo tree -p dozer-services --edges normal` 仍只有 `dozer-model` 与 serde 系。
2. `extensions/todo/**` 不再含 `dozer_client` / `io.client()`;`R-CLIENT` 在 `todo/*` 为 0,基线只降。
3. 读失败不清空列表;`Unavailable` 不弹 Toast、`Rejected`/`Protocol` 恰弹一条;均有测试。
4. 写操作失败的 `Mutated` 文本逐字不变;有测试。
5. 全量测试数 ≥ 基线 + 新增;已知失败(`files::delete_confirm_spec_reflects_pending_target`、`git_log::build_*` 间歇、`R-HOST-EXECUTORS 0->1`)不算回归也不顺手修。
6. `R-HOST-PANEL-ARMS` 不上升;clippy 警告数不高于基线;`cargo fmt --check`、`scripts/check-log-scope.sh` 通过;`model_golden` 仍通过。

## 6. 明确不做

- 不迁 host 侧的 Todo 调用(详情、派发、`process_todo_now`、终端相关)。
- 不改 Usage 的失败口径(Usage 现在对所有失败弹 Toast;是否同步成 Todo 的分档口径,Todo 完成后单独定)。
- 不给 `Loaded` 补 project_id/代数。
- 不改线上协议,不动 `PanelIo::client()`,不迁其它面板。

## 7. 风险与已知问题

- **`update` 签名变动面广:** `todo/**` 约 3700 行、`app/update.rs` 与 `app/app.rs` 各有入口。分读/写两个提交,每步全量测试。
- **探针行号漂移:** `app/update.rs` 若增删行,`usage_probe/host.rs` 的 `host_site` 引用会漂,按 X0 做法只校正行号。
- **`FakeDozerd` 只讲 Usage:** T2 的适配测试需要它(或替代夹具)支持 Todo 请求;计划阶段先读夹具再定。
- **`Loaded` 无项目/代数保护(现存):** 慢的旧结果可能覆盖新结果,与 Usage 修过的 G3b 同类。**本规格不修**,但会在 T5 的文档里登记为后续项。
- **WIP 文件:** 用户 WIP(含 `Cargo.lock`、`crates/dozer-app/Cargo.toml`、`files/*` 等)在 main 工作区未提交;在独立 worktree 实施,不提交 `Info.plist` 与版本号改动,`Cargo.lock` 剔除 bytegit/byteui 的 `source =` 行。

## 8. 分支与执行

规格分支 `spec/todo-services`(worktree `../dozer-todo-spec`);实施另开分支。执行计划由 writing-plans 产出,待本规格审阅通过后进行;合并进 main 前需询问用户。
