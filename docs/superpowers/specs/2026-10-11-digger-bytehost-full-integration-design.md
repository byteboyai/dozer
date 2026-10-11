# Digger 完整接入 bytehost：共享桌面宿主与应用平台

> 日期：2026-10-11。状态：规格草案，尚未实施。
> 范围：承接本次讨论的“完整共享桌面 Host + 完整应用平台接入”，分阶段交付。
> 本文位于 Dozer，作为 Dozer / bytehost / Digger 三仓库的共同实施依据；代码归属见 §4。
> 本文提出新的桌面宿主边界，不表示现有 bytehost 已提供这些 API；名称标为“拟新增”的均待实施。

## 1. 目标与完成定义

Digger 以 bytehost 作为桌面承载层和可安装应用平台：窗口、面板容器、布局、焦点、浮层、WebView 生命周期使用共享实现；Digger 只提供产品组合根、业务面板、编辑器/预览内容与服务适配。

“完全接入”同时满足：

1. Digger 正式桌面程序能独立构建、安装、启动，不需要 Dozer、dozerd 或相邻源码目录。
2. Dozer 与 Digger 使用同一套 bytehost 桌面宿主实现；Digger 不复制 Dozer 的 `App` / `Workspace` / WebView 调度器。
3. Digger 能通过注册清单组装自己的面板；单面板 + Host + fake 服务可运行，空栏和单面板布局合法。
4. 可安装应用的完整生命周期、管理界面、运行时管理、日志、隔离策略和恢复流程接通。
5. 至少一个已有面板（Usage）在两个产品中复用同一份实现，验证边界；Digger 提供适配后的真实数据来源后方可宣称该面板产品接入完成。
6. Dozer 保持现有用户行为、布局持久化兼容和应用数据兼容。

仅运行嵌入示例、仅通过 fake 测试、仅接通应用平台，均只能算阶段完成。

非目标：实现 Digger 全部写作业务；迁移 Dozer 所有面板；新建插件二进制 ABI；以 MCP 作为面板通信；更换 UI 框架；让第三方应用直接读写 Project。Windows/Linux 不属于本轮完成声明，本轮平台为 macOS 14+；其他平台明确显示未支持，不宣称同等隔离。

## 2. 依据与已核实的现状

依据：

- Digger `docs/adr/ADR-001-iced-wry-desktop-shell.md`、`ADR-011-bytehost-app-host-embedding.md`，当前均为 Proposed。
- Digger `docs/ByteBoy_Digger_Technical_Architecture_v0.1.md`。
- Dozer `docs/superpowers/specs/2026-10-04-bytehost-extraction-roadmap.md`。
- Dozer `docs/dozer-v2/bytehost-H0/07-usage-probe-gaps.md`。
- bytehost 的四个 crate、`bytehost-client/examples/embed.rs` 与 `tests/embedded_host.rs`。

2026-10-11 本地源码事实：

| 对象 | 现状 | 本规格处理 |
|---|---|---|
| bytehost | 已独立发布；本地 tag 与 Dozer 依赖均为 `v0.1.1` | 保留四 crate 能力，增量增加桌面层 |
| 四个现有 crate | `apps` / `client` / `webview` / `panel` 不依赖 iced、wry、Dozer | 原有中立性与依赖门禁不变 |
| Digger | 根目录没有生产 `Cargo.toml`；已有独立 spike 工程 | 建立正式 workspace，spike 不充当生产程序 |
| Digger UI 决策 | iced + wry + byteui | 修订 bytehost README 中过时的 Tauri 描述 |
| Dozer 服务迁移 | `extensions/**` 对 `dozer_client` 引用已清零 | 不重复迁移，继续清理宿主边界 |
| `PanelIo` | 保存 Services，但构造函数仍接收 Client；同文件包含具体适配器 | 拆开通用执行原语与 Dozer 适配器 |
| 面板注册 | 仍依赖 Dozer `PanelKind`、`RailLayout`，存在两栏非空约束 | 新宿主用稳定 ID 与产品注入布局策略 |
| WebView | Usage 等逐帧推送与 IPC 路由仍分布在 Dozer App/runtime | 用 Surface 契约统一承载 |
| Digger WebView spike | 有真实 WKWebView 策略验证 | 迁入正式集成后必须重验；不能引用 spike 成绩代替验收 |

须纠正 ADR 中的接口假设：`AppService::start` 返回 `Arc<AppService>`，启动失败可能得到不可用服务，并非 `Result::Err`；正常端口文件是 `gateway.json`。`AppHostApi::subscribe` 返回 `AppChange::{Changed, Resync, Disconnected}`，属于失效通知，不是完整业务事件流；`AppHostApi` 使用返回 future 的泛型方法，不直接做 `dyn AppHostApi`。

## 3. 架构与依赖方向

```text
Digger composition root                     Dozer composition root
  ├─ Digger 业务面板/编辑器/服务适配            ├─ Dozer 面板/Preview/Client 适配
  ├─ app-host: InProcess                     ├─ 应用平台: UDS 实现
  └────────────────┬────────────────────────┘
                   ▼
        bytehost-desktop（拟新增，iced + wry 平台实现）
                   ▼
        bytehost-core（拟新增，ID / 布局 / 生命周期 / 路由契约）

        现有应用平台：bytehost-panel → bytehost-client → bytehost-apps
        应用 WebView 策略：bytehost-webview
```

- `bytehost-core` 不依赖产品、iced、wry；`bytehost-desktop` 可以依赖 iced、wry、byteui 与 core，但不得依赖 `dozer-*` / `digger-*`。
- bytehost 仓库现有“所有 crate 不依赖 GUI”的门禁改成分组门禁：现有四 crate 保持原约束，新增 desktop 有明确 GUI 白名单；所有 crate 始终禁止产品反向依赖。
- `dozer-model`、`dozer-services` 是产品领域接缝，不能直接搬进通用 Host。Host 执行环境接受产品提供的服务上下文泛型；共享面板按需要依赖领域服务。
- Digger 的 Domain / persistence / 业务 runtime 不依赖 bytehost；桌面和 `app-host` 适配层负责接入。此处“业务 runtime”不指 desktop 使用的 Tokio 执行器。
- 面板之间调用 typed service，通过产品事件适配刷新；共享宿主只传递带身份的事件，不解释业务 payload。

## 4. 仓库与模块归属

| 仓库 | 修改范围 |
|---|---|
| bytehost | 新增 core、desktop；复用现有应用状态机；补经验证缺失的公共接口；依赖门禁、宿主 conformance、双消费者夹具、发布 tag |
| Dozer | 从 App/Workspace/平台层抽出通用实现；保留产品首页、项目语义、业务命令、终端业务与 Preview 内容；适配旧布局 |
| Digger | 建 workspace、desktop、app-host；注册产品面板；实现应用管理 UI 与产品服务；正式打包与端到端测试 |

拟新增的共享 Usage 面板先放 Dozer workspace 的独立库 `dozer-panel-usage`，不能依赖 `dozer-app`。依赖模型、服务、宿主公开 API 和内容资源，不依赖 Client。跨仓库消费采用可固定的 git revision；待 API 稳定后再按共同发布策略固定 tag。它是独立产品面板库，不进入 bytehost 的依赖树。

终端的 PTY、Agent/SSH 会话与命令执行留在产品；共享宿主只负责容器与焦点。首页内容、Project 打开/保存/权限、业务设置也留在产品。这样无需在本轮把整套终端业务搬进 bytehost。

## 5. 桌面宿主契约（拟新增）

### 5.1 稳定身份和注册

- `PanelId` 为经校验、带命名空间的稳定字符串，如 `digger.outline`、`shared.usage`、`app:<validated-id>`。应用 ID 继续由现有 `AppId` 校验。
- `ScopeId` 是产品提供的不透明作用域；Host 不理解 Project 数据。支持 application 与 workspace 两类生命周期，应用面板使用 application scope。
- `PanelInstanceId` 区分同一面板的不同实例；每次销毁重建递增实例 generation，旧任务不能写入新实例。
- 注册项提供元数据、视图/更新/激活/关闭回调和可用 Surface 描述。产品组合根决定默认布局和启用集合。
- 注册重复 ID 启动时报明确错误；缺失的历史 ID 保留在布局迁移记录中，显示可恢复占位或按产品策略隐藏，不导致整个窗口无法打开。

消息采用 `HostMessage<ProductMessage>` 形式：业务 enum 留在产品，Host 用回调分发，不要求各面板把业务 payload 序列化成 JSON 或通过任意 `Any` 下转型。只有进入 WebView 的消息走明确协议编码。

### 5.2 布局、窗口与事件

Host 持有窗口、rail、pane、拖拽尺寸、折叠、焦点、hover、浮层和主题投影。产品持有业务数据。布局序列化带 schema version，保存采用原子替换；窗口尺寸异常时钳制到可见范围。

旧 Dozer 枚举值到新 ID 的映射在 Dozer 内执行，有黄金样本测试；迁移后重启布局等价。Digger 不继承“两栏必须非空”或固定面板数限制。

通用请求包括显示/关闭面板、设置焦点、打开选择器、通知和产品命令回调。`SearchIn`、`ShowFileHistory`、Project 打开等业务命令留在产品；Host 不增加逐产品 match。折叠/hover 等宿主动作从面板业务 Message 中分离。

### 5.3 异步结果与生命周期

结果信封至少包含 scope、panel instance、generation、request key、request sequence。Host 在派发前核对实例仍存活、scope 一致、结果属于当前有效请求；同一 key 可选择 latest-only，独立 key 不相互覆盖。取消是资源优化，身份校验才是正确性保障。

Dozer 现有 request tracker 作为迁移输入，保留已验证行为；不能只改命名后丢掉乱序防护。面板关闭、项目切换、服务重启和 Surface 重建分别有对应 generation，不用一个全局计数使无关任务失效。

共享执行环境直接接收服务上下文、执行器与 emitter。Dozer 的 `ClientDaemon` / `services_for` 留在组合适配层；Digger 使用自己的服务实现与 fake。

### 5.4 Surface：统一 WebView 生命周期

Surface 的身份为 `(PanelInstanceId, SurfaceSlot, generation)`，不用 URL 或名称前缀猜所属面板。描述包含信任类别、内容来源、几何、可见性、焦点与消息处理回调。

生命周期：Absent → Creating → WaitingReady → Ready → Hidden / Destroyed；创建失败进入可重试 Failed。第三方应用未实现 ready 握手时使用导航完成状态，不要求它实现内置面板协议。

- 同一时刻每个 Surface 最多一个原生实例；所有 wry 创建、销毁、几何和焦点操作在窗口线程执行。
- 隐藏时不接受业务输入；销毁时注销路由。旧 generation 的 ready/IPC/脚本结果一律丢弃。
- 内置面板首次 ready 后发送完整快照，后续去重推送；隐藏期间保留最新待推送状态，恢复后同步；新建 WebView 必须重新发送快照。
- 几何采用统一逻辑坐标到物理坐标转换，处理 DPI、窗口缩放、零面积、分栏与裁剪。
- 原生浮层和应用 WebView 的遮挡由 Host 统一处理；in-window overlay 必须声明隐藏或缩小哪些 Surface，关闭后恢复几何与合理焦点。
- Surface 为公共机制，信任类别决定不同 builder；第三方应用 builder 不复用带编辑权限的 editor builder。

先接 Usage，再接 CodeHealth 验证不是 Usage 专用抽象；随后将 Dozer 剩余通用 WebView 承载改经此机制。编辑器内容协议、Preview 渲染和业务数据不会因此移入 Host。

## 6. Digger 应用平台完整接入

### 6.1 生命周期与存储

Digger `app-host` 独占 `AppService` 与 `InProcess`，数据根为产品独立的 `<app-data>/digger/bytehost/`；不能共享 Dozer 的注册表、端口文件或运行时可写目录。具体子目录结构由 bytehost 管理，Digger 不手写内部文件。

启动 UI 后异步启动服务，通过请求结果识别不可用状态。Gateway 出错只禁用应用功能，写作功能仍可用。应用跟随桌面进程；正常退出 await shutdown，保留 desired 状态供下次 reconcile。退出失败显示可操作错误，不静默遗留子进程；强制退出后下一次启动验证回收行为。

同一数据根只允许一个服务所有者。实施前验证现有库是否保证互斥；缺失时在共享层加锁并验证双启动，不让 Digger 和 Dozer 各写一份锁实现。

生产使用持久化端口启动；`port: 0` 只用于隔离测试。已保存端口冲突或损坏时不自动覆盖。设置提供重试和显式端口重置流程：停服务、销毁应用 Surface、说明 origin 改变对浏览器数据的影响、确认后修改并重启。若公共 API 缺失，在 bytehost 新增受测管理接口，不能由 Digger 直接修改 `gateway.json`。

### 6.2 能力矩阵

| 用户操作 | 接入方式与验收行为 |
|---|---|
| 列表、详情、刷新 | `app_list` + 共享状态机；空集合和不可用明确区分 |
| 本地目录、归档、URL 安装 | `app_plan` → Rust 审批视图 → `app_install`；来源支持遵守库约束 |
| 启动、停止、重新打开 | `app_start` / `app_stop` / `app_launch_url`；显示持久运行状态和错误 |
| 升级、回滚 | 新包走计划审批；`app_rollback`；失败后运行版本与 UI 一致 |
| 卸载、保留/删除数据 | `app_uninstall`；销毁 Surface 后协调浏览器数据清理，失败可重试 |
| 运行时探测、安装、卸载 | probe / runtime plan / install / uninstall；展示影响的应用和失败原因 |
| 日志 | 复用 LogsState，滚动跟随、刷新失败保留旧内容、关闭后丢弃迟到结果 |
| 崩溃、依赖缺失、宿主不可用 | Rust 原生持久状态页，提供重试、日志或运行时设置入口 |

使用 `bytehost-panel` 的 host/install/logs 状态机，不在 Digger 重写相同转移。适配器以泛型 `A: AppHostApi` 执行 effects，生产使用 InProcess，测试使用 fake。

订阅按真实接口处理：Changed/Resync 触发合并后的列表刷新；Disconnected 进入退避重订阅和轮询。复用共享状态机已有快速/安全轮询与退避常量，避免两个独立定时器。列表、launch URL 和日志回包都校验请求身份；隐藏/卸载后不能重新创建 Surface。

### 6.3 审批与来源策略

每次安装、升级、运行时安装均展示其实际计划并由用户确认；审批绑定来源、摘要和计划身份，重复提交防重，过期计划重新生成。Agent 与应用页面不能构造有效的人类审批事件。

本轮允许用户明确批准的本地可信 Python/Node 应用；Agent 生成与 URL 来源按 Untrusted 处理，仅支持 static_web。无法支持的来源/运行时组合明确拒绝。这里扩展 ADR-011 的 v1 范围以覆盖现有平台能力，须在采纳本规格时同步更新 ADR，不静默改变产品策略。

计划展示 LocalDir 会复制整个目录的事实、权限需求及真实 enforcement 等级；Advisory/Unsupported 不显示为强隔离。应用不获得 Project、文件服务或 Digger 编辑桥接权限。

### 6.4 应用 WebView

导航策略、数据存储 ID、nonce、IPC 白名单、数据清除队列复用 `bytehost-webview`。新 desktop 层统一实现 builder，产品只给策略配置与资源，不各自复制 wry 初始化代码。

- 不注入 Digger/Dozer 自定义协议或业务 IPC；拒绝跨 origin 导航、新窗口与下载。
- 应用 devtools 在所有构建下关闭；不声明摄像头/麦克风用途，直到有可验证的授权门禁。
- 剪贴板不沿用 spike 的默认开启：按安装授权与平台实际可执行能力决定；无法强制时显示真实 enforcement 并采用保守禁用。
- CSP 复用 Gateway 的共享策略，不由产品为个别应用临时放宽。
- launch URL 中的 token 只驻内存；不写布局、日志、错误详情或测试产物；重建时重新请求。
- app A/B 存储隔离、同一应用重启存储连续性、删除数据后的重新安装必须真实 WebView 验证。

## 7. 内置面板复用与 Digger 边界

Usage 是首个共享面板，因为已有 fake 服务和探针；CodeHealth 是 Surface 第二消费者，不要求在 Digger 产品中启用 CodeHealth。

Usage 提取应同时迁移 Rust 模块、Web 资源、构建与许可证信息，并消除对 Dozer `workspace` / `chrome` / 私有主题 helper 的引用。通用 UI helper 进入 byteui/desktop；Agent 图标等产品语义由注入的展示适配提供。

Digger Usage 数据来源定义：Digger 自己的任务/会话用量账本映射到面板服务 DTO，不连接 dozerd。真实账本尚未实现时提供明确 Unavailable 状态，并可在开发模式运行 fake 演示；阶段验收可验证空记录，但不能用恒为空的假实现宣称产品取数完成。该来源是最后完成闸门之一。

Todo / Files / Agent / Conversations 后续可沿相同接口接入；其 Digger 业务语义分别设计。本规格保证内置面板的接入机制和一例真实复用，不把 Dozer 全功能搬入 Digger。

## 8. 分阶段实施与退出条件

| 阶段 | 主要工作 | 退出条件 |
|---|---|---|
| S0 基线与契约探针 | 三仓库记录 revision、测试/门禁基线；从真实窗口验证 Surface 生命周期；确定 iced/wry/byteui 兼容版本 | Usage 与 CodeHealth 最小契约可编译；列出所有待搬通用职责；不使用历史行数充当当前基线 |
| S1 公共 core + Dozer 接缝 | ID、scope、请求身份、布局契约；拆出 Client 构造适配 | core 无产品依赖；单面板/空栏测试；Dozer 旧布局黄金测试通过 |
| S2 公共 desktop | 从 Dozer 提取窗口/布局/焦点/浮层/Surface 通用机制，Dozer 接入 | Usage、CodeHealth 跑真实窗口；通用实现只保留一份；其他面板兼容桥有清单和清除条件 |
| S3 Digger 正式壳 | 建 workspace、desktop、组合根，接 byteui 和公共 Host | 新克隆独立构建；能注册产品面板、保存布局、管理 Surface；不依赖 dozer-app |
| S4 应用服务与管理 | app-host、安装审批、运行时、事件、状态与日志 UI | §6 能力矩阵逐项有自动化与集成验证；Gateway 失败不影响主窗口 |
| S5 应用 WebView 与恢复 | 完整真实应用运行、隔离、浮层、退出、崩溃重启、数据删除 | 静态应用、py-notes、node-notes 闭环；恶意页面探针通过 |
| S6 共享面板与迁移收口 | 提取 Usage 并接两产品；Dozer 剩余通用 Host 路径切换 | 两产品同源宿主与 Usage；Digger 真实服务接通；通用机制兼容桥清零 |
| S7 发布验收 | 固定依赖版本、打包、干净环境验证、更新 ADR/README | §9 全部通过，发布产物可安装；逐项验收报告，无未列明功能缺口 |

S3 可在 S1 公共 API 稳定后开始最小壳，真实窗口验收依赖 S2；S4 可先做无头适配，GUI 完成依赖 S3；S6 面板库提取可提前准备。每阶段单独实施计划，不一次重写 Dozer 全部状态结构。

新增 API 先同仓受测，再发布 bytehost 新 tag；两产品的全部 bytehost crate 固定同一 tag。开发可以本地 patch，发布 CI 禁止依赖相邻工作区。保持四个现有 crate 的调用兼容，若确实需要破坏性调整，版本说明、两产品适配和回滚方案必须同批交付。

## 9. 验收矩阵

| 编号 | 场景 | 必须观察到的结果 |
|---|---|---|
| A1 | 独立构建 Digger/Dozer | 固定依赖可解析，无兄弟目录 patch；共享 Host 无产品依赖 |
| A2 | 一个面板 + Host + fake 服务 | 能真实显示、激活、关闭；不加载其他业务面板 |
| A3 | 切 scope/关闭重建/乱序回包 | 错 scope、旧实例、旧请求结果均不改变现有状态 |
| A4 | 旧布局迁移、单面板与空栏 | Dozer 布局等价；Digger 合法布局不受固定面板清单约束 |
| A5 | ready、隐藏、恢复、销毁 | 首次完整快照，恢复最新状态，旧 IPC 丢弃，无重复原生实例 |
| A6 | resize、DPI、分栏、原生/in-window 浮层 | 几何正确、遮挡和点击目标正确，关闭浮层后可继续输入 |
| A7 | 静态、Python、Node 应用 | 审批→安装→启动→交互→停止→重启；真实显示内容，不只检查 HTTP |
| A8 | 升级失败、回滚、卸载两种模式 | 包版本、状态、应用数据、浏览器数据与用户选项一致 |
| A9 | 运行时缺失/安装失败/运行时卸载 | 持久问题页，保留原因，可恢复；受影响应用明确可见 |
| A10 | Gateway 鉴权与导航 | 合法 cookie 200；无凭据 403；未知 Host 拒绝；跨 app/外站/新窗口被拒 |
| A11 | 伪造 IPC、合成事件、权限尝试 | 无业务桥接突破；策略与真实执行一致；token 不出现在产物 |
| A12 | app A/B 存储及重启/删除数据 | 不串数据；同 app 保持；删除后无旧数据复活 |
| A13 | 订阅断开、滞后、并发刷新 | 重同步并恢复，轮询兜底，无过期列表或 URL 覆盖 |
| A14 | 正常退出、强杀、双启动 | 正常端口释放；再启动安全处理孤儿；同根双服务拒绝；不误杀他进程 |
| A15 | Dozer 与 Digger 同时运行 | 根目录和端口独立，不串注册表与应用数据 |
| A16 | Project 移动/导出与无 Project 窗口 | 应用不进入 Project 内容；无 Project 也能管理应用 |
| A17 | Digger Usage 真实服务 | 真实任务用量映射正确；空与失败不同；不依赖 dozerd |
| A18 | macOS 发布包冷启动 | 资源齐全、应用 UI/内容可运行、退出正确；系统架构支持按实际发布构建记录 |

自动化分三层：纯状态/契约测试、进程与 Gateway 集成测试、真实 WKWebView/窗口行为测试。视觉、真实鼠标焦点、浮层与 IME 另记人工验收结果；禁止把 handler 直接调用当成真实点击验证。

Dozer 每阶段跑受影响测试及现有边界门禁，门禁只降不升；全量验收记录既有失败的测试名、原因和基线 revision，新增失败不得归入“已知问题”。bytehost 跑 fmt、clippy、全 feature 测试与分组依赖门禁；真实 Python/Node 和 WebView 用例在具备运行环境的独立任务执行，不能全部 ignore 后宣称通过。

## 10. 迁移、回滚与完成报告

每一切片保留可回滚的提交边界；不要把内部状态文件迁移和不可逆数据清理绑到启动。Dozer 新布局首次迁移保留原文件备份，新格式版本未知时拒绝覆盖。生产应用数据格式沿用 bytehost，升级遵守其兼容承诺。

回滚 UI/宿主版本不自动回滚应用包或删除用户数据。若新版本引入旧版本不可读格式，在发布前提供读旧写兼容方案或明确受控迁移；不得假定降级天然安全。

最终报告记录三仓库 revision/tag、依赖锁、支持平台、A1–A18 结果及证据路径、剩余限制和兼容桥清零结果。完成后同步更新 Digger ADR-001/011、技术架构、bytehost README 和 Dozer 提取路线图；ADR 只有在对应验证通过后才能标 Accepted。

## 11. 规格实施前需要落实的设计闸门

以下由 S0/S1 实测解决并回填本规格，不以猜测写成“已支持”：

1. iced 与 wry 窗口集成：从两仓库现有实现选择可复用入口，锁定版本与主线程所有权；以真实双消费者窗口证明。
2. Digger 真实 Usage 账本的接口和到既有 DTO 的映射；没有该业务数据源时，S6/S7 如实保留未完成状态。
3. 现有 AppService 的同根互斥、端口重置、退出错误可观测性是否够用；不足的接口在 bytehost 增补并受测。
4. 新 desktop 层对 macOS 存储清理、剪贴板与媒体权限的实际强制能力；不能用配置表代替系统行为测试。
5. 现有 Dozer UI helper 与日志/路径工具的归属清单：通用机制用标准依赖或宿主接口，产品路径/日志上下文由组合根注入，不把 dozer-core 带入 Host。

这些闸门限制的是相应阶段的实施与完成声明，不阻止先完成独立的应用平台适配与契约测试。
