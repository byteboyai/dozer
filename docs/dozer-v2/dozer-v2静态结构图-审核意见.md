# Dozer V2 静态结构图审核意见

> 审核对象：`docs/dozer-v2/dozer-v2静态结构图.md`  
> 对照材料：`docs/dozer-v2/dozer-v2架构分析.md`、当前 workspace 目录结构  
> 审核日期：2026-09-25  
> 审核性质：架构一致性与可落地性审核，不涉及代码实现

## 一、审核结论

当前版本不建议直接作为 V2 实施规格批准。

该文档适合作为架构讨论草图：覆盖了 Workspace、Plugin Runtime、Workflow Kernel、MCP Gateway、Execution Environment、Decision Service 和数据库拆分等主要方向。但目前仍存在四类会直接影响实施边界的问题：

1. Workflow Kernel 与插件私有领域模型之间存在重复事实源。
2. Plugin Runtime、Host、daemon 和 MCP Gateway 的进程职责尚未形成唯一裁决。
3. Event、Decision、Conversation、Usage 等核心数据结构不足以支撑文档声明的审计和重放语义。
4. 图中的依赖、运行时调用、模块包含关系没有被严格区分。

建议审核状态标记为：**有条件退回修改**。完成本文“准入条件”中的修改后，再进入下一轮架构评审。

---

## 二、高优先级问题

### 2.1 Todo 插件与 Workflow Kernel 形成两个 Task 事实源

静态结构图一方面把 `TASK` 定义为核心 Workflow Kernel 对象和核心数据库实体，另一方面又让 Todo 插件维护私有 `todos` 表，并由插件广播 Task 状态变化：

- 原文第 523—553 行：`dozer-plugin-todo` 负责完整 CRUD、私有 SQLite 和 Task 状态广播。
- 原文第 659—669 行：核心数据库定义 `TASK`。
- 原文第 835—847 行：插件私有数据库再次定义 `TODO_TODOS` 和状态字段。

这与架构分析 §16.2“Task 是插件协作的公共语言”以及 §18.2“Todo 面板 → Tasks”的裁决冲突。

如果两张表都可以写入任务状态，将出现以下问题：

- Task ID、状态机和权限规则重复。
- 插件数据库与核心数据库需要双写或异步同步。
- 插件离线、崩溃或卸载后，Task 状态可能不可恢复。
- Todo、Agent、Delivery 和 Decision 对“当前任务状态”的判断可能不一致。

**修改建议：**

- 核心 `Task Service/Repository` 作为 Task 唯一写入入口和事实源。
- Todo 插件只贡献 Task UI、命令、MCP tools、分类方式和视图偏好。
- Todo 插件私有数据库只保存分类、排序、过滤器等扩展数据，并通过 `task_id` 引用核心 Task。
- `todo.add`、`todo.assign`、`todo.complete` 应调用核心 Task API，不直接写入插件私有 Task 表。

### 2.2 `EVENT` 表不能支撑“append-only 权威数据源”声明

原文第 766—773 行定义的 `EVENT` 仅包含：

- `id`
- `trace_id`
- `span_kind`
- `event_type`
- `payload_json`
- `occurred_at`

但原文第 820—821 行又声明：`EVENT` 是 append-only 权威数据源，DAG 和状态视图只是投影。

当前结构不足以确定性重放 Goal、Task、Execution、Delivery、Decision 等聚合，也无法可靠处理并发写入、事件去重、Schema 演进和幂等消费。ER 图还只表达了 Execution 与 Event 的关系，无法承载非 Execution 聚合产生的事件。

需要先明确是否真正采用 Event Sourcing：

**如果采用 Event Sourcing，建议至少增加：**

- `aggregate_type`
- `aggregate_id`
- 单聚合递增的 `sequence`
- `event_schema_version`
- `correlation_id`
- `causation_id`
- `actor_type` / `actor_id`
- 全局提交顺序或事务位置
- `(aggregate_type, aggregate_id, sequence)` 唯一约束

同时需要定义快照、投影重建、事件升级、幂等消费和归档策略。

**如果不采用 Event Sourcing：**

应将表述修改为“append-only 审计事件日志”，明确 Goal、Task、Execution 等关系表才是事实源，避免同时存在两套权威状态。

### 2.3 Plugin Runtime 的所属进程和唯一控制权不清晰

原文存在三种并列描述：

- 第 100—101 行：`dozer-plugin-runtime` 同时指向 `dozerd` 和 Host。
- 第 178—182 行：Host 内存在 `supervisor_client`。
- 第 251—285 行：`dozerd` 是 Plugin Supervisor 的持有者。

这没有回答以下关键问题：

- 谁是插件进程生命周期的唯一 authority？
- 谁持久化安装、启停、升级和崩溃恢复状态？
- Host 退出后插件是否继续运行？
- 多窗口或多个 Host 实例是否共享同一个插件进程？
- Surface 消失与插件进程退出是否相互独立？

**建议裁决：**

- `dozerd` 持有插件安装状态、进程生命周期、权限、注册和崩溃恢复。
- Host 通过 daemon API 请求安装、启停、升级和重启，并负责实际 Surface 生命周期。
- Host 不直接监督插件后端进程，避免 Host 和 daemon 双重拉起或双重重启。
- `surface_binding` 可以由 daemon 保存归属元数据，但窗口、WebView 和原生组件的创建销毁属于 Host。
- 补充 daemon 的启动责任，以及多 Host 实例连接同一 Supervisor 时的仲裁规则。

### 2.4 MCP Gateway 绕过了 Supervisor 和权限控制面

原文第 110 行将 `dozer-mcp` 直接连接到插件；第 441—469 行的 Gateway 结构也只有插件客户端，没有表达其与 Plugin Supervisor、Permission Service、核心审计存储之间的关系。

在这种结构下，以下能力缺少权威来源：

- 插件 endpoint 发现和撤销。
- 插件离线及重启期间的状态判断。
- 会话、项目和用户身份验证。
- 工具调用权限裁决。
- MCP 调用与 Workflow Event 的关联。
- 统一取消、超时和审计。

**建议运行时调用关系：**

```text
Agent
  │
  ▼
dozer-mcp
  ├──► dozerd：身份、授权、工具注册表、审计、取消
  └──► plugin endpoint：执行具体 MCP tool
```

插件 endpoint 应由 Supervisor 发布和撤销。Gateway 不应自行扫描插件，也不应维护第二份插件生命周期状态。

---

## 三、中优先级问题

### 3.1 Workflow Kernel 漏掉了 Workspace 对象

架构分析 §16.2 列出的公共对象包括：

```text
Workspace
Goal
Task
Execution
Delivery
CheckResult
Artifact
Decision
Acceptance
```

静态结构图第 295—307 行和第 409—419 行只包含 Workspace 之外的八个对象，数据库中则出现了 `PROJECT`，但没有说明 Workspace、Project 与 Host UI session 的关系。

需要明确 Workspace 是：

- 独立、持久化的核心领域对象；
- Project 的 UI/session 投影；
- 还是仅存在于 Host 的临时布局状态。

如果 Workspace 不进入协议和数据库，应在文档中明确删除原因，而不是无说明地省略。

### 3.2 Platform Services 未完整映射原始架构裁决

架构分析 §4.2 和 §16.4 已列出多项平台能力，但 `dozerd` 图中没有清晰展示：

- Plugin Storage / namespaced storage
- Background Job / Progress / Cancel
- File Service
- Git Service
- Notification Service
- Artifact Store
- MCP Gateway 的 daemon 控制面

其中 Artifact 当前只有领域对象和数据库表，没有覆盖实际的内容存储、索引、权限、生命周期、清理和大对象管理。

建议补齐服务边界。尚不准备在第一阶段实现的能力可以标记为 `deferred`，但不应从目标结构中消失。

### 3.3 Contribution Registry 的类型覆盖不完整

原文第 171—176 行只明确列出 `panel_registry.rs` 和 `command_registry.rs`，但架构分析 §16.3 要求统一注册：

- Panel / Page / Tab
- Command
- Verifier
- MCP tool / resource / prompt
- Background job
- Data source
- Context provider
- Artifact renderer

建议使用统一的 `ContributionRegistry` 和稳定 `ContributionId`，再按贡献类型建立索引或适配层，避免每增加一种贡献类型就复制一套注册、撤销、权限和状态恢复逻辑。

### 3.4 Conversation 和 Usage 的数据归属不清晰

核心数据库已经定义 `EVENT` 和 `USAGE_RECORD`，插件私有数据库又定义完整的 Conversation、Turn 和 Summary 数据。

如果 Conversation Audit 和 Usage 插件成为原始数据持有者，插件卸载、禁用或数据损坏可能破坏 Execution 的核心审计链；如果核心库才是原始数据持有者，插件私库则不应再次保存一套独立事实。

**建议明确：**

- 原始 Execution Event、Tool/MCP Call、Conversation Turn 和 Usage 是平台级审计数据。
- Conversation Audit 和 Usage 插件可以持有索引、摘要、聚合统计和查询投影。
- 插件私有数据不是原始执行证据的唯一副本。
- 插件卸载后，核心 Workflow 的追溯和验收证据仍然完整。

### 3.5 Decision 数据结构不能满足自身审计要求

原文第 747—756 行的 `DECISION` 仅记录 kind、subject、status、provider、confidence 和 chosen option。

架构分析 §16.7 还要求记录：

- typed contract
- provider 的 model/version
- 输入快照引用
- 各候选概率
- 阈值
- 是否升级
- 人工是否推翻
- 最终动作和结果

当前模型无法支撑 Shadow Mode 评估、Provider 对比、校准、推翻率统计和完整决策审计。

建议至少拆分为：

- `DECISION_REQUEST`
- `DECISION_CANDIDATE`
- `DECISION_EVALUATION`
- `DECISION_RESOLUTION`

### 3.6 Artifact 的关系表达与物理约束不一致

ER 图同时画出 Execution、Delivery 和 CheckResult 与 Artifact 的关系，但 `ARTIFACT` 表使用 `owner_type + owner_id` 多态引用，数据库无法通过普通外键保证这三类关系的完整性。

需要在以下方案中作出选择：

1. 使用独立关联表，例如 `EXECUTION_ARTIFACT`、`DELIVERY_ARTIFACT`、`CHECK_RESULT_ARTIFACT`。
2. 保留多态 owner，但明确完整性由应用层和事件投影保证，并补充索引与删除策略。
3. 将 Artifact 作为独立实体，通过通用 `SUBJECT_LINK` 建立关联。

### 3.7 核心实体的约束和生命周期尚未表达

当前 ER 图更接近概念模型，而非可直接实施的物理数据库结构。例如：

- `TASK_DEPENDENCY` 没有复合主键、唯一约束和环检测策略。
- `WORKTREE.task_id` 没有表达“一项 Task 最多一个活动 Worktree”的条件约束。
- `EXECUTION_ENVIRONMENT.execution_id` 和 `AGENT_SESSION.execution_id` 没有唯一约束。
- `PLUGIN_PERMISSION` 没有表达 deny 优先级和授权来源。
- 多数状态字段没有状态转换版本或乐观锁字段。
- 没有软删除、归档、数据保留与级联规则。

建议数据库章节明确标注为“概念 ER 模型”；若要作为物理模型评审，则需补齐约束、索引、事务边界和迁移策略。

---

## 四、文档准确性与可追溯性问题

### 4.1 多处引用了当前不存在的章节

静态结构图引用了 §4.5、§4.7、§4.9、§4.11、§3.5.4 等章节，但当前 `dozer-v2架构分析.md` 中不存在对应标题。

涉及位置包括：

- 第 322 行：`§9 / §4.9`
- 第 389 行：`§4.9`
- 第 421 行：`§4.5`
- 第 608 行：`§4.7 / §16.7`
- 第 615、820 行：`§4.11`、`§3.5.4 / §4.11`

这会破坏需求到设计的追踪关系。建议使用稳定的 ADR/Decision ID，或修正为当前实际章节编号。

### 4.2 全景图箭头没有统一语义

全景图混合表达了以下三种不同关系：

- Cargo/crate 编译依赖。
- 进程间运行时通信。
- 逻辑上的“基于”或“提供给”。

例如文字写明“仓库外插件依赖 SDK”，图中却是 `sdk → plugin`；`dozer-client → host` 和 `runtime → host` 也无法确定是在表达依赖方向还是调用方向。

建议拆成三张图：

1. Workspace crate 依赖图：`dependent → dependency`。
2. 进程部署拓扑图：Host、daemon、Gateway、plugin process、sidecar。
3. 运行时调用图：请求、事件、注册、Surface 和 MCP 通道。

每张图都应提供箭头图例。

### 4.3 “现状保留”标记与当前仓库结构不一致

Micro Host 图把以下路径或文件标为现状基本保留：

- `app/message.rs`
- `app/update.rs`
- `workspace/tabs.rs`
- `workspace/panel_container.rs`
- `term/pty_view.rs`
- `chrome/cdp_driver.rs`

当前仓库中并不存在这些对应路径。文档开头虽然声明文件命名属于合理推演，但颜色图例又将其定义为“现状基本保留”，容易使实施者误判迁移工作量。

建议将节点状态至少拆成：

- 现存且路径基本一致。
- 现存能力、计划重组。
- V2 全新模块。
- 待迁移或删除的 legacy 模块。

### 4.4 迁移顺序混合了试点顺序和正式迁移顺序

静态结构图第 123—124 行描述的顺序是：

```text
Code Health → Todo → SSH → Usage → Conversations → Browser → Database
```

架构分析 §18.16 描述的正式迁移顺序是：

```text
Code Health → Usage → Conversations → Todo → Browser → Database → SSH
```

两者可以同时成立，但含义不同：

- §11.2—§11.4 是协议和运行时能力的纵向试点顺序。
- §18.16 是 Workflow 主模型稳定后的正式能力迁移顺序。

建议分别命名，避免实施计划将两条路线误认为同一个阶段序列。

### 4.5 Surface 数量表述需要统一

架构分析 §7.3 定义四种 Surface：Declarative、WebView、External、None。Host 图只绘制 A、B、C 三类文件。

`None` 不一定需要实现成独立模块，但文档应说明它代表无 UI 插件，而不是遗漏第四种 Surface。还应明确纯 MCP、纯后台任务插件不经过 Surface 生命周期。

### 4.6 `dozer-protocol` 的职责可能过宽

当前设计把以下内容同时放入一个 crate：

- Plugin Protocol envelope
- manifest schema
- MCP namespace
- 权限模型
- Workflow Kernel 领域对象
- trace/span/usage schema

这有造成协议版本、领域模型版本和观测 Schema 被绑定发布的风险。建议至少在模块和兼容策略上区分：

- wire protocol
- kernel API/domain DTO
- permission schema
- observability schema

第一阶段可以仍使用单 crate，但应避免使用一个全局版本号强制所有子协议同步升级。

---

## 五、建议的目标边界

### 5.1 核心事实源

| 数据 | 建议事实源 | 插件职责 |
|---|---|---|
| Project / Workspace identity | Platform Service | 提供 Context 展示或扩展 |
| Goal / Task | Workflow Kernel | Todo 提供 UI、命令、分类和策略 |
| Execution / Agent Session | Workflow Kernel / Session Service | Runs、Conversation 提供视图和投影 |
| Delivery / Acceptance | Workflow Kernel | Verifier、Code Health、Browser 贡献证据 |
| Event / Tool Call / Usage | Platform Audit/Event Store | Conversation、Usage 构建索引和统计投影 |
| Domain report | 插件私有存储 | 通过标准 CheckResult/Artifact 关联核心流程 |
| Plugin permission / registration | Plugin Supervisor / Permission Service | 插件仅声明需求，不自行授予 |

### 5.2 推荐进程职责

```text
dozer-host
├── Workspace Shell
├── Contribution/Surface rendering
├── 用户交互和 permission prompt
└── 通过 daemon client 查询或发起操作

dozerd
├── Workflow Kernel
├── Plugin Supervisor
├── Permission/Secret Broker
├── Event/Audit Store
├── Worktree/Execution/Verifier/Artifact Services
└── MCP Gateway 控制面

dozer-mcp
├── 面向外部 Agent 的 MCP server
├── 身份和项目上下文入口
├── 调用路由、超时和取消
└── 通过 dozerd 获取授权和插件 endpoint

plugin process
├── 领域逻辑和私有存储
├── MCP tool handler
├── Background job
└── 可选 Surface backend/assets
```

### 5.3 推荐图集拆分

下一版建议至少提供以下图：

1. 目标进程部署拓扑图。
2. Rust crate 编译依赖图。
3. Plugin 安装、启动、握手和恢复时序图。
4. MCP tool 注册与调用时序图。
5. Surface 创建、恢复和销毁时序图。
6. Workflow Kernel 概念 ER 图。
7. 核心事实源与插件投影的数据流图。

---

## 六、下一轮评审准入条件

下一版静态结构图至少应完成以下事项：

1. 明确 Task、Event、Conversation、Usage 的唯一事实源。
2. 删除 Todo 私有 Task 状态，或明确其只是核心 Task 的投影/扩展。
3. 确定 Plugin Supervisor 的唯一所属进程及多 Host 规则。
4. 补充 MCP Gateway 到 daemon 权限、注册和审计控制面的连接。
5. 补全 Plugin Storage、Job、File/Git、Notification、Artifact 等 Platform Services。
6. 补全 Contribution Registry 的全部贡献类型。
7. 明确 Workspace 与 Project 的关系。
8. 决定 EVENT 是 Event Store 还是 Audit Log，并让 Schema 与表述一致。
9. 补齐 Decision 的可审计数据模型。
10. 将 crate 依赖、进程拓扑和运行时调用拆图，并标明箭头语义。
11. 修正失效的章节引用和不准确的“现状保留”标记。
12. 分开描述纵向试点顺序与正式插件迁移顺序。
13. 将数据库章节明确标记为概念模型，或补足物理约束、索引和迁移策略。

完成以上修改后，该文档可以进入第二轮审核，并进一步评估是否可以作为 SDD/spec 的结构输入。
