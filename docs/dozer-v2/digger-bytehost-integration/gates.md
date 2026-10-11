# Task 03：AppService 缺口 + Digger 数据源调查（回填规格 §11 闸门）

> 计划 Task 03 产出。输入：`B/crates/bytehost-apps/src/{service.rs,port.rs}`、`B/crates/bytehost-client/src/in_process.rs`、`D/docs/ByteBoy_Digger_{Logical_Domain_Model_v0.3,Technical_Architecture_v0.1}.md`、`D/docs/adr/ADR-011-bytehost-app-host-embedding.md`、`Z/docs/dozer-v2/bytehost-H0/{07-usage-probe-gaps,08-client-use-inventory}.md`。
> 规格 §11 有 5 条闸门（`Z/docs/superpowers/specs/2026-10-11-digger-bytehost-full-integration-design.md:238-248`）。本任务回填闸门 2、3；闸门 1 由 Task 02 回填（见 `api-decisions.md`）；闸门 4、5 由后续 Task（S2/S3）回填。

## 闸门 1（iced×wry 窗口集成）— 已回填（Task 02）

见 `api-decisions.md` §8。锁定 `iced 0.14` / `wry 0.55.1`；`iced::application` 单循环 + `window::run` 建 wry 子视图；主线程 thread_local 所有权；探针 `B/spike/desktop-contract/` 以真实窗口与双产品类型编译证明。**结论：成立。**

## 闸门 2（Digger 真实 Usage 账本的接口与到既有 DTO 的映射）

**调查结论：Digger 侧不存在真实 Usage 账本，仅有领域层占位与设计意图。属"尚未实现"，S6/S7 须如实保留未完成状态。**

证据：

| 来源 | 内容 | 是否生产实现 |
|---|---|---|
| `D/docs/ByteBoy_Digger_Logical_Domain_Model_v0.3.md:1661-1668` | "Token Usage" 是 **Execution Trace（Observability）** 的一条，**不是** Core Project History；"Observability is infrastructure; History is domain projection." | 领域模型，无实现 |
| `D/docs/ByteBoy_Digger_Logical_Domain_Model_v0.3.md:1688` | Agent Runtime 的 reasoning/tools/sessions 属 Runtime/Observability，**不得**直接持久化 Creator Domain State | 边界声明，无实现 |
| `D/docs/ByteBoy_Digger_Technical_Architecture_v0.1.md:676` | Rust **持久化** "token/cost budget"（AgentSession 生命周期的一部分） | 架构意图，无实现 |
| `D/docs/ByteBoy_Digger_Technical_Architecture_v0.1.md:698` | ModelResponse 含 `usage`（由 TS gateway 归一化） | 架构意图，无实现 |
| `D/docs/ByteBoy_Digger_Technical_Architecture_v0.1.md:781` | Runtime "model/tool usage, budget and failures" 属可观测性 | 架构意图，无实现 |
| `D/spike/*`（block-identity/editor-host/md-mirror/…） | 无任何 usage/token/账本代码（`rg -i "usage|token"` 仅命中无关 tokenizer/设计 token 文本） | 无实现 |
| `D/` 无生产根 `Cargo.toml` | 尚无正式 workspace，只有 `spike/*` 独立工程 | 无生产实现 |

**缺口定位（供 Task 20 用）：**
- **取数契约**：Digger 需要把自己的"任务/会话用量账本"映射到面板服务 DTO（规格 `:182`）。该账本**归 Rust 持久化**（`Technical_Architecture_v0.1.md:670-677`），sidecar 只保留 transient。
- **到 DTO 的映射**：Dozer 侧 Usage 面板的既有 DTO 是 `UsagePayload`（`Z/crates/dozer-model`，见 `07-usage-probe-gaps.md:45` G6 已迁入）。Digger 需产出同形 payload，**不连接 dozerd**（规格 `:182`）。
- **Unavailable 状态**：真实账本未实现时，服务必须返回明确的 Unavailable，而非恒空假实现；开发模式可跑 fake 演示；阶段验收可验证空记录，**但不能用恒空假实现宣称取数完成**（规格 `:182`）。该来源是**最后完成闸门之一**。
- **实现顺序建议**：如计划主路径，Task 20（Usage 纵向切片）在宿主迁移完成后落地；本闸门不阻塞 S0/S1 的契约与应用平台适配。

**结论：闸门 2 成立为"缺失且已定位"，非"已解决"。** S6/S7 不得声称 Digger Usage 取数完成，除非 Task 20 落地真实账本。

## 闸门 3（AppService 同根互斥、端口重置、退出错误可观测性是否够用）

**调查结论：现有 AppService 在多数点上够用；"同根互斥/端口占用可观测"够用（带类别错误），但"退出错误可观测性"不足——`shutdown()` 返回 `()`，无法表达失败。需在 bytehost 增补受测接口。**

证据（`B/crates/bytehost-apps/src/service.rs`、`port.rs`）：

| 子问题 | 现状 | 是否够用 |
|---|---|---|
| 启动失败可观测 | `AppService::start` **永不失败**，失败进入 `State::Unavailable(reason)`；此后 `handle` 一律回 `AppFailure{kind: Unavailable}`（`service.rs:31,40-42,202-204`） | **够用**（调用方靠请求结果识别，非 `Result::Err`） |
| 同根互斥 / 端口占用 | 端口持久化在 `<root>/gateway.json`（`port.rs:32`），首次从 `20000..=32767` 随机选、**绑定成功后**才写（`port.rs:13-14,58-61`）；**绝不静默重选**——占用报 `GatewayError::PortInUse`（`port.rs:3-6`） | **够用**（冲突可观测、不静默换端口） |
| 端口文件损坏 | 损坏/越界是错误，不静默新选（`port.rs:109` 钉死测试 `corrupt_or_out_of_range_settings_are_errors_not_a_silent_new_port`）；空文件会成永久 `InvalidData`，故原子改名落盘（`port.rs:58`） | **够用**（但设置页需显式"重置端口"流程，规格 `:138`） |
| 端口重置流程 | bytehost **当前无**公共重置接口；规格要求"停服务、销毁 Surface、说明 origin 变化影响、确认后改并重启"，且"不能由 Digger 直接改 `gateway.json`"（`:138`） | **不足**，需在 bytehost 增补受测管理接口 |
| 生命周期：随桌面进程、保留 desired | `shutdown()` 幂等：撤站点、观察态落 `Stopped`、停 gateway，**保留 `desired`**；下次 `start` 时 `reconcile` 按 `desired=Running` 重新拉起（`service.rs:6-7,178-179,352`） | **够用** |
| 退出错误可观测性 | `pub async fn shutdown(&self)` 返回 **`()`**（`service.rs:352`），**无法表达失败**；任务注释"退出失败显示可操作错误，不静默遗留子进程"（规格 `:134`） | **不足**，需增补返回结果或事件 |
| 强制退出后回收 | `reconcile` 会在下次启动回收上次遗留（`service.rs:1176-1182` 钉死测试：同根重建时 `desired=Running` 仍被拉起） | **够用** |

**需在 bytehost 增补（受测）的接口：**
1. **退出可观测**：让 `shutdown` 能报告失败（返回 `Result` 或错误事件），使 Digger 能"显示可操作错误，不静默遗留子进程"（规格 `:134`）。
2. **端口重置管理接口**：一个受测的公共方法，执行"停服务 → 清/换端口 → 重启"的既定流程，避免 Digger 直接操作 `gateway.json`（规格 `:138`）。
   - 二者都属"在 bytehost 新增受测管理接口"，不改既有 wire 形状（`AppRequest`/`AppReply` 只追加）。

**结论：闸门 3 成立；3 个子项中 4 项够用，2 项（退出可观测、端口重置接口）需在 bytehost 增补受测接口。** 该增补不阻塞 S0/S1 的独立应用平台适配与契约测试（规格 `:248`）。

## 闸门 4、5 — 未回填（后续 Task）

- 闸门 4（desktop 层对 macOS 存储清理、剪贴板、媒体权限的实际强制能力）：属 S2/S3（desktop 层搬移后），需系统行为测试，不能用配置表代替。**本任务未做。**
- 闸门 5（Dozer UI helper / 日志 / 路径工具归属清单）：属 ownership 任务的深化，见 `ownership.md`；`07-usage-probe-gaps.md` §5 已给出 B 类 helper 清单（`agent_icon`/`agent_dot_color`/`chrome::homespace`/`HoverId`/`PanelKind` 等）。**待后续 Task 归位。**

## 对实施的影响

- S0/S1（Task 01–03）不阻塞：闸门 1 已解，闸门 3 的增补是 bytehost 内受测接口，闸门 2 的缺失只影响 S6/S7 的完成声明。
- Task 04+（desktop 层搬移）可开始；Task 20（Usage 纵向切片）须在 Digger 真实账本落地后，否则如实标 Unavailable。
- bytehost 的退出可观测与端口重置接口，可与搬移并行增补。
