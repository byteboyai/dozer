# bytehost:面板钩子(Effect)与面板注册制设计

> 状态：**草案，待用户审阅**。日期：2026-10-04。
> 依据：`2026-10-03-bytehost-boundary-design.md`（E1/E2/E3、§8 未决项）、`docs/dozer-v2/bytehost-H0/`（审计）、H1–H3 的成果（`PanelHost`、`HoverId::Panel`、`PanelDims` 访问器已在 `main`）。
> 本文定的是 H0 汇总里**剩下的大块**（候选 3、5 与 `PanelKind` 注册制 B1/B4/B5）的机制设计与迁移顺序。**不是实施计划**：H4 的实施计划等本文评审后再写。

## 1. 已定裁决（2026-10-04，用户）

1. **O5：默认栏位与默认顺序由产品组合根（composition root）决定。** 注册信息里只放面板自身属性，不放"默认在哪一栏"。Dozer 与 Digger 各自的组合根给出面板清单、默认栏位与顺序；"恰 N 个面板"的布局校验也由组合根给出的清单驱动。
2. **面板钩子取"返回 Effect 的纯函数"：** 面板对 host 的接入形如 `fn(&mut PanelState, Event) -> Vec<Effect>`；host 执行 Effect，面板不直接碰 `App`/`Workspace`。

## 2. 现状事实（2026-10-04，`main` = `f6db32af`）

| 事实 | 数据 | 来源 |
|---|---|---|
| host 在 `App::update` 里**对某个面板有专属处理体**的消息臂 | **97 个**（Files 15、Ssh 15、Database 13、Conversations 9、Project 9、Browser 6、GitLog 6、CodeHealth 5、Usage 4、…），分布在 6475 行的 `app/update.rs` | `grep` 统计 `Message::<面板>(` 臂 |
| `platform/window_events.rs` | 4596 行，内联 Files/Todo/Browser/Project/ProjectCreate/Search/GroupChat 的事件处理 | E3-010/011/038 |
| 面板自带的"异步任务"约定 | 38 个 `spawn_*(…, client: &Client, handle: &Handle, emit: impl Fn(M) + Send + 'static)` 形态的函数，分布在 20 个文件 | `grep "emit: impl Fn("` |
| 面板自带的 Effect/Followup 枚举 | `group_chat::Effect`（3 个变体）、`agent_context::Followup`、`edit_history::Followup`；`toast::Outbox`（面板把待发提示推进自己的 state，host 统一排空） | `grep` |
| host 里执行面板 Effect 的代码 | `App::run_group_chat_effects`（`app/update.rs`,约 35 行）只是把 `group_chat::Effect` 映射成 `gc::spawn_*` 调用 | `app/update.rs:4711` |

两个结论：

- **"host 点名面板"的真正体量不在 `PanelKind` 的 973 行引用，而在这 97 个消息臂 + `window_events.rs`**——它们是 host 替面板做编排（跨面板消息、系统对话框、异步结果回投、切入刷新）。
- **代码库已经有两套半成形的机制**：toast 的 `Outbox`（面板写、host 排空）与"面板自己的 `spawn_*(client, handle, emit)`"。设计应沿用它们，不另起炉灶。

## 3. 设计

### 3.1 `PanelIo<M>`：host 给面板的"执行原语"

面板异步任务现在都要三样东西：`Client`（dozerd 连接）、`Handle`（tokio runtime）、`emit`（把结果包成 `Message::<面板>(m)` 投回事件循环）。把它们收成一个值，由 host 构造、传给面板：

```rust
pub(crate) struct PanelIo<M> { /* client, handle, wrap: Arc<dyn Fn(M) + Send + Sync> */ }
impl<M: Send + 'static> PanelIo<M> {
    pub fn client(&self) -> &Client;
    pub fn emit(&self, m: M);                 // 同步投回一条面板消息
    pub fn spawn<F, Fut>(&self, task: F)  // 任务拿到 host 的 Client 与一份 PanelIo 副本,想发几条消息(含零条)自己定
    where F: FnOnce(Client, PanelIo<M>) -> Fut + Send + 'static, Fut: Future<Output = ()> + Send + 'static;
}
```

(`spawn_blocking` 暂时没人用,按需再加——H4 实现取舍。)`PanelIo` 与 H1 的 `PanelHost`（只读视图契约）是一对：`PanelHost` 管"读 host 状态来画 view"，`PanelIo` 管"让 host 替我跑东西"。仍然**不为未来的第二个宿主实现预先抽象 trait**：`PanelIo` 是具体结构体，测试里用一个"录制 emit"的构造函数即可。

### 3.2 Effect 的两层（对用户裁决的一处**细化**，需评审确认）

用户选的是"返回 Effect 的纯函数、host 提供统一执行器"。读代码后我把它拆成两层，理由见 §5：

- **面板专属 Effect（类型化，留在面板模块里）：** 面板的 `update` 返回自己的 `enum Effect`（如 `group_chat::Effect::FetchMessages{..}`），保持纯函数、可断言。**执行器是面板模块里的 `run_effect(effect, project_id, &PanelIo<Message>)`**，host 不再认识这些变体（删掉 `run_group_chat_effects` 这类代码）。
- **host 通用 Effect（词汇固定、host 自己执行）：** 面板需要 host 才有的能力时用 `HostEffect`——`ShowPanel(PanelId)`、`PickDirectory{ start, on_done }`/`PickFile`（系统对话框，现在 `window_events.rs` 为面板代拦 `rfd`）、`OpenExternal(path)`、`Emit(宿主消息)`（跨面板消息，如 Files 的 `OpenSearch` 变成 `search::Message::SearchOpen`）、`Toast(…)`（现有 `Outbox` 并入）。词汇小而稳，按真实需求增加。

(H5 实现取舍:需求经面板 state 里的 `HostOutbox` 返回,而不是 `update` 的返回值——沿用 `toast::Outbox` 的现有模式,各面板 `update` 的签名不变;本刀词汇只落地 `PickDirectory` 与 `Command(PanelCommand)`,`ShowPanel` 等有使用者时再加。)

### 3.3 切入钩子

`fire_panel_switch_in`（9 个面板各一段"切入时刷新"）变成面板的 `on_activate(&mut PanelState, ActivationCtx) -> Vec<Effect>`；`ActivationCtx` 只带只读的 `project_id`/项目路径。host 对所有面板调用同一个入口，不再 `match`。

### 3.4 注册制与类型

- `PanelKind`（封闭枚举）→ `PanelId`（轻量 id 类型，序列化成**与现在枚举名相同的字符串**，保持 `panel_layouts.json` 兼容）。
- `PanelDescriptor { id, title(i18n key + fallback), icon, needs_preview_column, hooks }` 由**组合根**注册；host 通过 registry 遍历，不 `match`。
- 默认栏位/顺序 = 组合根给的 `DefaultLayout`；`RailLayout` 的"恰 12 个不重复"校验改成"恰等于组合根清单"；`migrate_legacy_rail` 这类按枚举写死的迁移要改成按清单。
- 每面板展开的状态（`Workspace` 的 14 个面板字段、`Message` 的 20 个包装变体、`PanelDims` 的字段）**第一阶段不动**：注册制先只替换"类型传递/元数据/遍历"（H0 `01-panelkind.md` B1–B3），状态与消息信封的动态化是最后一步（§4 H8），收益与风险都最大，等前面验证再定。

## 4. 迁移顺序（每一步单独可合并、行为不变）

| 步 | 内容 | 依赖 | 体量（估） |
|---|---|---|---|
| **H4** | 引入 `PanelIo<M>`；把 `group_chat` 的 Effect 执行器与 `Command` 的异步分支搬进 `group_chat`（删 `run_group_chat_effects` 与 `group_chat_command` 的 spawn 分支）；`PanelIo` 带测试构造函数，给 `group_chat` 补 effect 单测 | 无 | 约 −60 行 host、+80 行（`PanelIo` + 测试） **已完成(H4,`bytehost-h4`):`778df4de`** |
| **H5** | `HostEffect` 最小词汇（`ShowPanel`、`Emit`、`PickDirectory`）+ 执行器；迁 `window_events.rs` 的 rfd 对话框拦截（E3-011）与 `Files::OpenSearch`/`FileHistoryOpen` 这类跨面板消息臂 | H4 | 约 −300 行 host **部分完成(H5a,`bytehost-h5`):`ee7a2803`**——3 个选目录对话框(project_create 两处 + Files 的移动到目录)与 Files 的搜索/查看历史两条跨面板臂;`PanelCommand` 目前只有 `SearchIn`/`ShowFileHistory`;余下见 E3-010/E3-011 |
| **H6** | 切入钩子：`on_activate` 取代 `fire_panel_switch_in`（9 个臂） | H4、H5 | 约 −70 行 host |
| **H7** | `PanelId` 类型 + `PanelDescriptor`/registry + 组合根给默认栏位（B1/B2 的剩余部分，O5 已定）；布局校验按清单；serde 兼容测试 | H6 | 大（`PanelKind` 约 970 行引用里的类型传递部分，机械） |
| **H8** | 状态与消息信封动态化（`Workspace` 面板字段、`Message` 包装）——**仅在 H7 后评估是否做** | H7 | 很大，单独立项 |

`window_events.rs` 的"面板事件声明化"（H0 候选 3）拆进 H5（对话框、拖拽落点里调用面板的部分）与 H8（真正的事件钩子需要 registry 才能做完）。

## 5. 对"统一 Effect 执行器"裁决的细化与理由（请评审）

用户裁决的原文是"host 提供统一的 Effect 执行器（spawn 异步任务、弹系统对话框、发消息）"。如果把"spawn 异步任务"也做成 host 通用 Effect（例如 `Effect::Daemon(Box<dyn FnOnce(Client) -> BoxFuture<M>>)`），有三个问题：

1. **可测试性变差：** 闭包不可比较，`group_chat` 现在的 `Effect::FetchMessages{group_id}` 是可断言的纯数据；变成闭包后单测只能执行它。
2. **与现状不一致：** 38 个现成的 `spawn_*(client, handle, emit)` 已经是"面板自己拥有异步逻辑、host 只给原语"的形态，改成闭包是大规模重写而非收口。
3. **类型擦除压力：** host 通用 Effect 的结果消息要回投给**某个面板的 `Message`**，要么按面板泛型（`Effect<M>`，执行器仍不认识面板），要么 `Box<dyn Any>`（H8 才需要）。

所以本文取：**纯函数 + 类型化面板 Effect + 面板侧执行器（用 `PanelIo`）**，host 通用 Effect 只收"只有 host 能做的事"（对话框、显示面板、跨面板消息、toast）。**效果与用户裁决的意图一致**——面板钩子是纯函数、host 不再认识面板的具体动作——差别只在"异步任务的执行器放哪"。如果你更想要字面意义的"统一执行器"（全部走 `HostEffect`），告诉我，H4 的设计会改成 `Effect<M>` + 闭包形态，并接受上面三点代价。

## 6. 验收标准

1. 每个迁移步骤：全量测试通过数 = 基线 + 新增测试数；clippy 诊断与基线逐文件一致；门禁（`scripts/audit/check_panel_boundary.py`）只降不升。
2. 新增门禁规则（H4 起）：`app/update.rs` 里"对某个面板有专属处理体的消息臂"的条数只许减不许增（基线 97）；`run_<面板>_effects` 这类 host 里的面板专属执行器清单。
3. H7 的兼容测试：旧版落盘的 `panel_layouts.json`（含 12 个面板名）能被新 `PanelId` 反序列化，布局不丢。
4. 面板代码里不得出现 `App`/`Workspace`（已有门禁），H4 起不得出现 `Message::<别的面板>`（跨面板消息只能经 `HostEffect::Emit`）。

## 7. 未决项

| # | 问题 | 倾向 |
|---|---|---|
| P1 | `HostEffect::Emit` 携带的是 host 的 `Message` 还是一个受限的"跨面板命令"类型？前者简单但让面板依赖 host 总消息，后者要多一层 | 受限命令类型（`PanelCommand`），否则 E2 形同虚设；H5 设计时定 **已定(2026-10-04,用户):受限的 `PanelCommand` 类型**,不让面板依赖 host 总消息;H5 设计时定其词汇 |
| P2 | Effect 的执行顺序与失败语义（一条失败是否影响后续）；现状各面板各自隐式约定 | 顺序执行、互不影响；失败经 `Toast` 报告 |
| P3 | `PanelIo::spawn` 的取消（面板被关/项目被关后任务结果如何丢弃） | 沿用现状（结果带 `project_id`，host 在投递时按项目路由，项目已关则丢） |
| P4 | H8（状态与消息信封动态化）是否值得做 | H7 完成后用 H0 的度量重新评估 |
| P5 | `PanelDescriptor` 里的 `hooks` 是 trait object 还是函数指针表 | 函数指针表（零成本、与"不预设 trait"一致）；H7 设计时定 |
