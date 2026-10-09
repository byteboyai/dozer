# dozer-services:面板服务 trait 接缝(X0,Usage 试点)

> 日期:2026-10-09。状态:待用户审阅。依据:路线图 `2026-10-04-bytehost-extraction-roadmap.md` 的 X0 / T5、探针缺口 G4(`docs/dozer-v2/bytehost-H0/07-usage-probe-gaps.md`)、O7 已解(`2026-10-09-dozer-model-crate-design.md`)。

## 1. 已对齐的理解

**目标(用户已选):** 面板可进程内测试/复用——面板取数只依赖小 trait,测试和将来的 Digger 可以注入内存实现,不再需要起假 UDS。
**范围(用户已选):** 规格盘点全部面板用到的服务能力并给出 trait 分组;**只落地 Usage 试点**,其余面板之后按纵向切片逐个迁。
**设计选择(用户已确认):** 按能力拆小 trait + `Arc<dyn>` 服务束(方案 A);trait 放在新 crate `dozer-services`;方法返回手写 `BoxFuture`,不引入 `async-trait`。

**假设(请纠正):**
- 路线图 X0 原意是"只定义接口与适配层,不搬代码"。本规格在此基础上加一个试点落地,用来验证接缝可行。
- H 系列曾裁定 `PanelIo` 是具体结构体、"不为未来的第二个宿主预先抽象"。现在第二个消费者(探针夹具、Digger)已经存在,本规格有意修订该裁定,但保持 `PanelIo<M>` 的泛型形状不变。

## 2. 结构与依赖方向

```text
dozer-model            (已有, 仅 serde)
dozer-services (新)    依赖 dozer-model;不依赖 dozer-core / dozer-client / tokio
   ├─ BoxFuture<'a, T>
   ├─ ServiceError { kind: Unavailable | Rejected | Protocol, message }
   └─ UsageService::usage_summary(...)            ← 试点
dozer-client           为 Client 实现 UsageService(薄适配)
dozer-app
   ├─ PanelIo 增加 services: Services;`client()` 保留
   └─ usage_probe 新增进程内 FakeUsage
```

- 依赖严格单向:`dozer-services → dozer-model`;`dozer-client → dozer-services`。面板将来只依赖 `dozer-model` + `dozer-services`。
- `Services` 是各 trait 的 `Arc<dyn …>` 的集合,试点期只含 `usage: Arc<dyn UsageService>`。
- 共存期:`PanelIo::client()` 保留,未迁移的面板不受影响,每迁一个删一个调用点。

## 3. 错误模型(解决 G4)

`ServiceError { kind, message }`,`kind`:
- `Unavailable`:连不上或连接中途断开(对应 `HangUp` / UDS 连接错误);
- `Rejected`:dozerd 回了 `Reply::Error`;
- `Protocol`:回了意外应答或解码失败。

Usage 面板据此把"没有数据"(成功但为空)与"取数失败"分开:失败时发一条明确的失败消息,面板显示失败状态,**不再**把失败当成空列表。这是**用户可见的行为变化**(此前取数失败显示为空数据),属于 G4 要修的缺陷;失败的具体展示(面板内状态或 Toast)在计划里定,审阅本规格时请确认接受这一变化。

## 4. 能力清单(本轮只实现 Usage)

| trait | 覆盖的 `Client` 方法(面板侧) | 现用面板 | 本轮 |
|---|---|---|---|
| `UsageService` | `get_usage_summary` | Usage | **实现** |
| `TodoService` | `list_todos`、`list_categories`、`delete_category` 等 | Todo | 仅登记 |
| `GroupChatService` | `create/delete/cancel_group`、`post/retry_group_message`、`push_group_message_to_todo`、`remove_group_member` | GroupChat | 仅登记 |
| `ProjectService` | `list_projects`、`remove_project`、`list/get_memory`、`*_summary_batch`、`backfill/delete_project_transcripts` | Project、ProjectCreate | 仅登记 |
| `BookmarkService` / `ContextService` | `list/remove_bookmark`、`list_context_items` | Browser、AgentContext | 仅登记 |
| `AppHostApi` | 12 个 `app_*` 方法 | 设置「应用」页 | 已有 trait,登记同属服务束 |
| 不抽 trait | `database`、`shutdown_daemon`、终端流(`subscribe`/`attach`/`write`/`resize`) | Database、设置、终端 | 记录理由 |

- 终端流与 `database` 是流式或会话级接口,形状与请求/应答式取数不同,硬套同一套 trait 会失真,故不抽。
- 此表来自一次粗略 grep,**漏掉了方法名换行书写的调用**(如 Usage 的 `get_usage_summary`)。S0 用编译器重新盘点,以盘点结果为准,盘点后允许修订本表(只改分组,不改本轮范围)。

## 5. 迁移步骤(每步一个提交)

- **S0 盘点:** 产出"面板 → 方法 → 能力"表,落 `docs/dozer-v2/`,重新确认 §4;同时记录基线(测试数、clippy 警告数、`extensions/**` 引用 `dozer_client` 的文件数)。
- **S1 建 `dozer-services`:** 先写 `ServiceError` 与 `UsageService` 契约测试(内存实现),再建 crate。依赖仅 `dozer-model`。
- **S2 `Client` 实现 `UsageService`:** 先用 `FakeDozerd` 写失败测试,覆盖正常返回、`HangUp` → `Unavailable`、`ErrorReply` → `Rejected`;再实现。
- **S3 `PanelIo` 带 `Services`:** 在 host 创建 `PanelIo` 的地方注入 `Client` 适配出的服务束;保留 `client()`。
- **S4 Usage 切换:** `spawn_refresh` 改吃 `UsageService`;失败不再变空列表。先写"取数失败不显示为无数据"的测试并看它失败,再改。
- **S5 探针升级:** `usage_probe` 改用进程内 `FakeUsage`;钉住 G4 的特征化测试 `s4_characterizes_g4_*` 改写为断言新行为;`FakeDozerd` 保留给 S2 的适配测试。
- **S6 门禁:** `check_panel_boundary.py` 新增 `R-CLIENT`:`extensions/**` 对 `dozer_client` 的引用文件数,基线为实测值,只降不升。

## 6. 验收标准(均可机器验证)

1. `cargo tree -p dozer-services` 只出现 `dozer-model` 与 serde 系。
2. Usage 面板源码不再出现 `dozer_client`;`scripts/audit/unit_edges.py ext:usage` 的 S 类边只剩 `bytegit`。
3. Usage 的取数失败与"无数据"可区分,有测试。
4. 全量测试数 ≥ 基线 + 新增;main 上已有的 `files::delete_confirm_spec_reflects_pending_target` 失败与 `R-HOST-EXECUTORS` 门禁失败不算回归,也不顺手修。
5. 线上协议零变化:`Request`/`Reply` 不动,`dozer-core/tests/model_golden.rs` 仍通过。
6. `R-CLIENT` 基线 = 实测值,门禁通过;`cargo clippy --all-targets` 警告数不高于基线;`cargo fmt --check` 干净。

## 7. 明确不做

- 不迁其余面板;不抽终端流与 `database`。
- 不处理 Usage 的 `bytegit::Repo`(本地仓库读取,不是 dozerd 服务,属另一条 S 类边)。
- 不删 `PanelIo::client()`,不改 `PanelHost`/`PanelIo<M>` 泛型形状。
- 不动 `AgentKind` 封闭枚举(O11);不改 wire;不迁独立仓库。

## 8. 风险与对策

- **失败语义是产品行为变化**(见 §3):规格评审时确认;计划里写明展示方式。
- **`BoxFuture` 生命周期:** `handle.spawn` 要求 `'static`,故 trait 方法参数按值传(`String`、`Option<…>`),适配层 clone;S1 的契约测试用 `tokio::spawn` 调一次以尽早暴露。
- **trait 对象与 `Send`:** `Arc<dyn UsageService + Send + Sync>`,返回的 future 要 `Send`;写进别名。
- **`dozer-hook`/`dozer-mcp` 依赖闭包:** 它们不依赖 `dozer-client`,新 crate 不进它们的闭包;S1 用 `cargo tree -p dozer-hook` 确认。
- **WIP 文件与 `Cargo.lock` 噪声:** 在独立 worktree 实施,不提交 `Info.plist`、版本号相关改动;`Cargo.lock` 只带 `dozer-services` 相关块,剔除 bytegit/byteui 的 `source =` 行(同 O7)。

## 9. 分支与执行

规格分支 `spec/dozer-services`;实施另开分支,不直接在 main 上做。执行计划由 writing-plans 产出,待本规格审阅通过后进行。
