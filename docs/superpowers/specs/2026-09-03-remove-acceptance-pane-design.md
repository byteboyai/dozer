# 删除验收(Acceptance)面板及相关功能

**状态:已批准(brainstorming 会话,2026-09-03)**

## 背景

验收面板(`extensions::acceptance`,详见 [2026-08-08-acceptance-pane-design.md](./2026-08-08-acceptance-pane-design.md))是右侧 rail 图标可选的独立面板:目标条目复选清单(解析 `.dozer/goal.md`)、变更文件 diff 手风琴预览、通过时创建 `refs/dozer/accepted/N` git ref 并落 dozerd `acceptances` 表、打回把意见文字注回来源终端 PTY。

Todo 面板(`extensions::todo`)此后持续演进,现已具备状态流转(Pending/InProgress/Suspended/Done)、指派给 agent 会话运行(`AssignAgent`)、"详情"弹窗承载的完整对话线程(`GetTodoDetail`/`DetailReplySubmit`/`ProcessTodoNow`)、日历、分类树。用户判断:Todo 面板的对话+状态流转已经能承接"验收"这类工作,验收面板作为独立结构化流程(checklist/diff/通过-打回)不再需要存在。

brainstorming 会话中明确了删除边界:**不做任何能力迁移**——目标清单、diff 预览、git 验收 ref、"N 次验收"计数这几个具体机制直接整体删除,不往 Todo 里加任何新东西;以后验收类工作就是 Todo 详情弹窗里的对话与状态流转,不追求功能对等。同时明确:dozerd 后端(`acceptances` 表 + 协议 + client 方法)一并彻底删除,不保留哑接口;项目卡片上的"N 次验收"这行直接删除,不用其他指标替代。

## 目标 / 非目标

**目标**:

1. 删除 `crates/dozer-app/src/extensions/acceptance.rs` 整个文件,及其在 `app.rs`/`workspace.rs`/`rail.rs` 里的全部接线(`PanelKind::Acceptance`、`Message::Acceptance`、`acceptance_open`/`acceptance_reject`/`acceptance_result` 路由、rail 图标/tooltip/金点徽标、`ws.acceptance` 字段)。
2. 删除 `crates/dozer-app/src/goal.rs`(`.dozer/goal.md` 解析,唯一消费者是 acceptance.rs,验收删除后无其他消费者)。
3. 从共享文件 `delivery.rs` 中摘除验收专属部分:`accept()`、`ACCEPTED_REF_PREFIX`、`changes()`、`file_diff()`、`FileChange`、`delivery_pending()`(含对应单测)。`repo_root`/`branch`/`local_branches`/`checkout_branch`/`init_repo`/`is_dirty` 等仍被 `files.rs`/分支 UI 使用的部分原样保留。
4. 删除 `SessionTab.delivery_pending` 字段(`workspace.rs`)及其在 `app.rs` 里的计算/清空逻辑(终端轮询处、`acceptance_open` 清空处)。
5. 删除 `project.rs` 里 `project_acceptance_count` 字段与"N 次验收"这行渲染,以及触发其刷新的 `spawn_acceptance_count_refresh`(`workspace.rs`)调用点。
6. 删除 dozerd 后端:`crates/dozerd/src/acceptance.rs`(`AcceptanceStore`)整个文件、server.rs 里 `RecordAcceptance`/`GetAcceptanceCount` 两个 handler 及 `AcceptanceStore` 的构造/挂载。
7. 删除协议层:`dozer-core/src/protocol.rs` 里 `Request::RecordAcceptance`、`Request::GetAcceptanceCount`、`Reply::AcceptanceCount` 三个变体。
8. 删除 `dozer-client` 里 `record_acceptance()`/`acceptance_count()`/`RecordAcceptanceParams`。
9. 删除 `docs/user_guide/acceptance.md`。
10. 更新 `CLAUDE.md`:工作区布局表里 `dozerd` 一行去掉"验收闭环存储"措辞;关键裁决里 webview 隐藏那条备注去掉对 Acceptance 的举例引用(规则本身不变,`Database`/`Ssh`/`Agent`/`Usage` 仍是同一 match 分支的其余成员)。

**非目标**:

- 不往 Todo 面板迁移任何验收专属能力(checklist/diff 预览/通过-打回/git ref)——brainstorming 已明确确认。
- 不删除或迁移已存量用户 sqlite 文件里现存的 `acceptances` 表数据——代码不再建表/不再引用即可,老表留在盘上是无害死数据,不写 DROP TABLE 迁移(避免一次有风险的 schema 变更)。
- 不删除历史设计/计划文档(`docs/superpowers/specs/2026-08-08-acceptance-pane-design.md`、`docs/superpowers/plans/2026-08-08-acceptance-pane.md`,以及更早的 P1c–P1l 系列验收闭环 spec/plan)——作为历史记录原样保留,不会被误读为当前文档。
- 不改动 `delivery.rs` 中非验收专属的共享 git 辅助函数。
- 不改动 Todo 面板自身的任何代码或行为。

## 影响范围清单(按 crate)

### `crates/dozer-app`

- `src/extensions/acceptance.rs` — 整个文件删除。
- `src/goal.rs` — 整个文件删除;`mod goal;` 声明一并删除。
- `src/delivery.rs` — 删除 `accept`/`ACCEPTED_REF_PREFIX`/`changes`/`file_diff`/`FileChange`/`delivery_pending` 及对应单测(`delivery_pending_matrix` 等),保留其余共享辅助函数。
- `src/app.rs` — 删除 `PanelKind::Acceptance` 枚举成员及所有匹配分支(编译器会穷举报出遗漏点)、`Message::Acceptance(acceptance::Message)`、`acceptance_open`/`acceptance_reject`/`acceptance_result` 方法与调用点、终端轮询里计算 `delivery_pending` 的代码块。
- `src/workspace.rs` — 删除 `SessionTab.delivery_pending` 字段(含所有初始化点)、`ws.acceptance: acceptance::WorkspaceState` 字段、`spawn_acceptance_count_refresh` 方法与调用点。
- `src/rail.rs` — 删除验收图标/tooltip 条目、金点徽标读取逻辑、`PanelKind::Acceptance` 在 default-side 映射表里的条目。
- `src/webview_geometry.rs` — `PanelKind::Acceptance` 从共享的"无 webview 面板"match 分支里摘除该枚举成员(该行为其余四个面板继续保留)。
- `src/extensions/project.rs` — 删除 `project_acceptance_count` 字段、赋值处、"N 次验收"渲染代码、相关单测。

### `crates/dozerd`

- `src/acceptance.rs` — 整个文件删除;模块声明一并删除。
- `src/server.rs` — 删除 `RecordAcceptance`/`GetAcceptanceCount` 两个 `Request` 匹配分支、`AcceptanceStore` 的构造与挂载到 server state。

### `crates/dozer-core`

- `src/protocol.rs` — 删除 `Request::RecordAcceptance`、`Request::GetAcceptanceCount`、`Reply::AcceptanceCount` 三个变体。

### `crates/dozer-client`

- `src/lib.rs` — 删除 `record_acceptance()`、`acceptance_count()`、`RecordAcceptanceParams`。

### 文档

- `docs/user_guide/acceptance.md` — 删除。
- `CLAUDE.md` — 按上文"目标 10"更新两处措辞。
- `docs/superpowers/specs/`、`docs/superpowers/plans/` 下历史验收文档 — 不改动。

## 测试与验证计划

- 删除枚举分支/协议变体后,`cargo build`/`cargo clippy --all-targets --workspace` 会因非穷举 match 报出所有遗漏的调用点——这是本次改动最可靠的完整性检查手段,应作为实现阶段的主要验证方式(逐个消掉编译错误,而不是靠人工搜索"还有哪里引用了 acceptance")。
- `cargo fmt --all`。
- `cargo test -p dozerd -p dozer-app -p dozer-client -p dozer-core`:确认没有测试仍依赖被删的类型/端点;验收面板/`delivery_pending`/`project_acceptance_count` 相关单测随对应代码一并删除,不保留任何指向已删符号的测试。
- 手动起 GUI(`cargo run -p dozer-app`)走一遍:rail 上不再出现验收图标;原验收面板对应的项目态不再报运行时错误;项目卡片不再显示"N 次验收"这行;打开一个有 git 改动的终端会话,确认不再出现任何验收相关横幅/徽标残留。

## 未决问题

无——brainstorming 会话已就迁移范围、后端处理方式、项目卡片计数处理方式三个关键分叉逐一确认。
