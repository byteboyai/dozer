# dozer-model:把领域数据类型从 `dozer-core::protocol` 拆成独立 crate(O7)

> 日期:2026-10-09。状态:待用户审阅。依据:O12 探针缺口 G6(`docs/dozer-v2/bytehost-H0/07-usage-probe-gaps.md`)、H8 评估 §7("O7 应在 Digger 复用面板之前先解")、`docs/superpowers/specs/2026-10-04-bytehost-extraction-roadmap.md`(O7)。

## 1. 已对齐的理解

**目标(用户已选):** 面板可复用——面板代码不应为了拿几个纯数据类型而依赖整个 `dozer-core::protocol`(3141 行,含 `Request`/`Reply` 约 200 个变体)和 `dozer-app` 里的 `conversation` 模块。
**方案(用户已选):** A,独立 crate `dozer-model`。
**范围(用户已选):** 全部 43 个纯数据类型 + `conversation.rs`。

**假设(请纠正):**
- 这一步只解"领域类型归属"。面板离开 `dozer-app` 仍需要取数接缝(`dozer_client::Client`,探针 G4/S 类边),那是 X0/H8 契约的事,本规格不碰。
- 数字(43 个类型、23 个文件、约 14 处调用点)来自设计阶段的一次测量;M0 开工时重新测量并写入计划,以实测为准。

## 2. 结构与依赖方向

```text
dozer-model   (新, crates/dozer-model, 依赖只有 serde)
   ├─ agent        AgentKind, AgentState, SessionInfo
   ├─ conversation ConversationSummary, TurnRecord, ToolCallInfo,
   │               ConversationMeta, SessionRow   ← 从 dozer-app/conversation.rs 下沉
   ├─ summary      Summary* 六个类型, SessionSummaryPayload, ConversationSummaryResult
   ├─ usage        UsagePayload
   ├─ todo         TodoInfo, TodoStoredStatus, CategoryInfo, CategoryMoveDirection
   ├─ project      ProjectInfo, MemoryInfo, MemoryDetail, MemoryHistoryEntry
   ├─ preview      Preview* 六个类型, LocateMatch, MutationOutcome
   ├─ group_chat   Group* 六个类型
   └─ misc         BookmarkInfo/Scope, CodeHealthReportInfo, ContextItemInfo, FileEditHistoryInfo

dozer-core::protocol   只剩 Request / Reply / encode_line / decode_line
                       + `pub use dozer_model::*`(兼容出口)
```

- `dozer-model` 不依赖 `dozer-core`、`bytehost-apps`、`dozer-client`;`dozer-core` 依赖 `dozer-model`。依赖严格单向。
- 线上格式零变化:`#[derive]` 与全部 `#[serde(...)]` 原样搬走,`Request`/`Reply` 的 JSON 与迁移前逐字节相同。
- 兼容出口:`dozerd`、`dozer-client`、`dozer-mcp`、`dozer-hook` 与 `dozer-app` 的非面板代码继续使用 `dozer_core::protocol::AgentKind` 等路径,不改。`dozer-hook` 的依赖闭包只多一个仅依赖 serde 的小 crate。
- inherent impl(如 `AgentKind::label/display_label`、`ConversationMeta::from_summary`)必须与类型同 crate(Rust 规则),因此随类型一起搬走。
- 名称暂定 `dozer-model`;迁独立仓库时再改成产品中立的名字。

## 3. 迁移步骤(每步一个提交,行为不变)

- **M0 特征化(搬前):** 为每个待搬类型写黄金 JSON 测试(序列化形状、`#[serde(default)]` 缺字段回落、枚举 `snake_case`),在现有位置跑绿。同时重新测量并记录基线:类型数、`extensions/**` 引用 `dozer_core::protocol` 的文件数、`crate::conversation` 调用点数、测试总数。
- **M1 建 crate + 搬类型:** 新建 `dozer-model`,机械搬类型与 impl,`protocol.rs` 改为 `pub use`。黄金测试随类型搬走,**不改一个字**必须仍通过。
- **M2 下沉 `conversation.rs`:** `ConversationMeta`/`SessionRow` 及其测试移入 `dozer-model::conversation`,更新调用点,删除 `dozer-app/src/conversation.rs`。
- **M3 面板切换导入:** `extensions/**` 把 `dozer_core::protocol::` 改成 `dozer_model::`(这些文件都不使用 `Request`/`Reply`)。
- **M4 门禁:** `scripts/audit/check_panel_boundary.py` 新增 `R-PROTOCOL`:`extensions/**` 对 `dozer_core::protocol` 的引用文件数,基线 = M3 之后的 0,只降不升。

## 4. 验收标准(均可机器验证)

1. `cargo tree -p dozer-model` 只出现 `serde` 及其 proc-macro 依赖。
2. 黄金 JSON 测试在 M0(搬前)与 M1(搬后)同样通过且未修改。
3. 全量测试数 = M0 基线 + 新增。main 上已有的 `files::delete_confirm_spec_reflects_pending_target` 失败与 `R-HOST-EXECUTORS 0 -> 1` 门禁失败照旧,不算回归,也不顺手修。
4. `R-PROTOCOL` 基线为 0。
5. 探针 G6 重新判定:Usage 面板的 C 类依赖边(用 `scripts/audit/unit_edges.py`)不再指向 `conversation::` 或 `dozer_core::protocol`,只指向 `dozer_model`。
6. `cargo clippy --all-targets` 警告数不高于基线;`cargo fmt` 干净。
7. `Request`/`Reply` 的黄金 JSON(抽样若干变体,含嵌套被搬类型)搬前搬后一致,证明 wire 未变。

## 5. 明确不做

- 不处理 `dozer_client::Client` 等服务依赖(探针 S 类边、G4)。
- 不动 `AgentKind` 的封闭枚举与品牌名(O11,Digger 接入别的 agent 时再扩)。
- 不动 `transcript.rs`(827 行,Claude 专属解析)。
- 不迁独立仓库;不改 `Request`/`Reply` 的任何变体;不改 Usage 自己的聚合类型。

## 6. 风险与对策

- **孤儿规则:** `dozerd`/`dozer-app` 内若存在对这些类型的外部 trait 实现,搬家后会编译失败;编译器会立即暴露,M1 逐个处理(预期没有)。
- **重导出遮蔽:** `pub use dozer_model::*` 可能与 `protocol` 内同名项冲突;M1 用显式列表 re-export 而非通配,冲突在编译期可见。
- **工作区 WIP:** main 上有用户未提交的 `Cargo.lock`、`crates/dozer-app/Cargo.toml`、`crates/dozer-app/packaging/macos/Info.plist`。实施在独立 worktree 进行,不提交这三个文件;worktree 构建产生的 `Cargo.lock` 变动不入提交(新增 crate 导致的 lock 变动除外,需单独核对后提交)。合并前再对一次。
- **新增 crate 的 Cargo.lock:** `dozer-model` 是 workspace 新成员,lock 必然变化;提交时只带这一处变化。

## 7. 分支与执行

- 规格分支 `spec/dozer-model`;实施另开分支,不直接在 main 上做。
- 执行计划由 writing-plans 产出,待本规格审阅通过后进行。
