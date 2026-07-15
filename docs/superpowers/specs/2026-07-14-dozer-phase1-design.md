# Dozer 一期设计（工作稿）

> 状态：需求、架构与 UI（Figma 12 帧）均已与用户逐节确认，待用户终审后转实现计划。
> 需求来源：2026-07-14/15 需求讨论（本文档为其记录）；`docs/my/dozer建议开发手册.md` 与 `docs/analysis/` 为参考。
> 术语：概念模型中作品中立的根实体"作品 Work"，在一期（软件模板）语境与 UI 中一律称"项目"。

## 1. 愿景（已验收）

**事实基础**

1. AI 使执行近乎免费，移除了旧工作流中迫使人聚焦目标、遵守流程、治理产物的天然摩擦；未经审视的意图会被立即放大成产物。
2. AI 自身在长程一致性上不可靠（跨会话失忆、架构理解流失、局部补丁倾向）。
3. 两者叠加的普遍后果：目标失焦、流程失序、产出物失治——AI 很强大，但给不出可持续演进的作品。
4. AI 拉平了执行能力，没有拉平判断能力（定义"什么是好"、发现"哪里不对"、决定"下一步"）。
5. 用户是多疑的，不会忠诚于任何单一 agent；且验收权在结构上不能属于被验收方。

**定位**：Dozer 是站在用户（甲方）一侧的、agent 中立的治理与验收层。把资深创作者的判断结构（目标锚定、验收关口、演进秩序）外化成产品，让任何希望用 AI 创作作品的人得到可持续演进的、自己满意的产出物。执行由 AI 拉平，判断由 Dozer 拉平。

**归属**：agent 是可更换的乙方承包商；目标、验收标准、验收记录、作品及其演进史属于用户，跨 agent/会话/厂商存续。

**产物的完整定义**：作品 = 演进资产集 = 终付产物 + 定义性资产（PRD、需求、设计稿、验收标准、测试用例）+ 过程性资产（任务、缺陷、验收记录、构建、数据、部署脚手架）。定义性/过程性资产是"可持续演进"的机制本身——下一轮 agent 恢复判断力的依据。

**Dozer 不是**：IDE、代码编辑器、AI 聊天应用、终端模拟器、又一个 agent。

## 2. 分期路线图（已确认）

| 期 | 主题 | 内容 | 对标 |
|----|------|------|------|
| 一期 | 单人闭环 | 原生 agent 终端 + 内建预览 + 验收闭环雏形 + 会话存活 + 对话可审阅性 | kooky 级落地 |
| 二期 | 编排与资产 | agent 编排（多路尝试/择优/派单）、过程性资产体系（自建 or MCP/GitHub 在此决断） | orca 视野 |
| 三期 | 自治与记忆 | loop engine、多 agent memory 共享（中立载体即演进资产集） | agent 平台 |
| 四期 | 协作 | 多成员验收权与资产流转 | 生态 |

## 3. 一期需求（冻结）

1. **原生 agent 终端**：多会话、标签（分屏二期）、shell 集成（OSC 7 cwd / OSC 133 命令退出）、agent 状态感知；流畅度对标 kooky——一期验收口径为"正确 + 不卡顿"，像素级打磨持续进行、不设一期关口。
2. **内建产物预览**（体验支柱，与终端并列）：Markdown、图片、PDF、代码/diff、Office 等在应用内直接看；**网页亦可预览**（预览 pane 本身即 WKWebView）——localhost 产物预览是 Web 类项目的验收现场（刚需），外部 URL（文档/GitHub）给带极简地址栏的最小网页 tab，不做标签/书签/历史等浏览器化功能；"不能让用户总跳出 Dozer，这是混乱的来源"；从对话、文件树、组件视图、验收视图处处一键可达。
3. **验收闭环雏形**（软件项目模板）：目标/验收标准 → agent 交付 → 验收（通过/打回）→ 演进史。关口松紧由 dogfooding 调；治理结构是可配置领域模板，非硬编码姿态。
4. **会话存活**：PTY 由常驻 daemon 持有，关窗/崩溃不掉会话（竞品分析结论：后补等于重写）。
5. **mac 先发、架构留门**：全 Rust 跨平台生态；无 Swift/AppKit/libghostty 专属绑定。
6. **对话可审阅性**：以人类发言为导航锚；AI 回合过程性输出默认折叠；提问/待决策/交付声明置顶；产物优先于叙述（看 diff/预览而非自述）。与验收闭环同构：作品级 vs 回合级。

**一期范围裁剪（2026-07-15 与用户逐项确认）**：
- **boy CLI 废弃，永不回归**：agent 启动归 dozerd、doctor 归项目栏状态、MLX 模型托管归 dozerd 进程管理（二期随组件视图呈现）；现有 `src/` 代码仅作 dozerd 种子迁移后删除。
- 砍掉：浅色主题与 W2 主题选择页（单主题 **ByteBoy2077**，设计稿保留）；组件视图+dbx 集成（二期）；⌘K 全局搜索/命令面板（菜单+快捷键替代，资源搜索二期）；终端分屏；H0 日历与社区板块（右栏仅"最近的文件/对话"两卡，余占位）。
- 降级：W1 首启页降级为"agent 检测 + 一键注册 hooks"，不做会话/配置导入；agent 适配器一期仅 Claude Code（codex 为 1.5 期首件，适配器接口保留）；设置 UI 仅 Agents+关于两节，其余走 config.toml；预览格式一期口径＝Markdown/图片/PDF/代码/diff/localhost 网页，Office 等长尾随 Flyfish 自带、不逐格式 QA。

## 4. 技术选型（手册 §14 核查后）

- **采纳**：tokio、serde+toml、tracing、thiserror+anyhow、notify、directories、uuid、chrono、camino、nucleo、tokio::process、portable-pty、alacritty_terminal、SQLite（rusqlite，本地单机无需异步池）。
- **修正**：`iced_dock` 不存在 → iced 内置 `pane_grid`（tab 用 iced_aw 补）；gitoxide 读路径用 gix、写路径一期驱动 git CLI。
- **重新定位**：Flyfish File Viewer（flyfish.dev，206 格式/24 管线，纯前端、离线自托管）是 JS 组件 → 预览引擎 = wry(WKWebView) + 自托管 Flyfish；配合 Preview Engine 抽象（Flyfish 为第一个实现）。
- **补充**：wry（三平台 WebView 抽象）；daemon↔客户端 IPC = UDS + 帧化 serde；`dozer-hook` 零依赖小二进制（学 KookyHook）作为对话结构化数据源；每 agent 一个 transcript 适配器（Claude Code JSONL/stream-json 先行）。
- **dbx 集成（[t8y2/dbx](https://github.com/t8y2/dbx)，Apache-2.0，Tauri 2，60+ 数据库）**：数据库资产的预览引擎，与 Flyfish（文件）构成 Preview Engine 抽象的两个实现。姿势：一期组件视图经 `@dbx-app/cli`（JSON 输出）拿连接/表/状态做原生摘要渲染，深度操作"在 DBX 中打开"（哲学同"编辑器是外部工具"）；二期经 `@dbx-app/mcp-server` 让 agent 直接读写数据库、Dozer MCP 客户端消费同一入口。不 fork 其 Rust 核心（Tauri command 后端非独立 crate）、不内嵌其 Web UI；npm 依赖归入 agent 级可选集成，不进 Dozer 核心依赖。
- **GUI 框架（已定）**：iced 0.14（2025-12 发布，1.0 前最后实验版；响应式渲染/无头测试/热重载/**IME 中文输入**；COSMIC 桌面与 Kraken Desktop 生产背书）。放弃 gpui 备选：pre-1.0、文档薄、API 随 Zed 漂移，且 WebView 嵌入在其自有平台层属无人区，而 winit+wry 路径有官方示例。
- **WebView 合成（风险已降级）**：默认走**原生子视图叠加**（WKWebView 子 NSView 叠于 wgpu 表面，wry 直接支持）。约束：webview 恒在 GPU 内容之上 → 左二必须是规则矩形 pane（成立）；**⌘K 命令面板打开时临时隐藏预览 webview**（已定，kooky 同款做法）。备选：纹理导入（wgpu-scry 系，二期跨平台再评估）；独立预览窗（最后手段）。spike 内容=验证叠加方案的焦点/滚动/缩放细节。
- **终端数据面（设计点）**：dozerd 持裸 PTY + 字节环形缓冲（滚屏）；客户端 attach 时以 alacritty_terminal 状态机回放。网格入 daemon（tmux 式）留二期。
- **IPC 帧格式**：一期 JSON Lines（可调试性优先），二期视需要换二进制帧。iced 侧启用 tokio executor feature 与 daemon 生态统一。

## 5. 运行时架构

```
dozer-app (iced GUI)              dozer-hook (被 agent hooks 调用)
     │ UDS                              │ UDS（单向写事件）
     └───────────┬──────────────────────┘
                 ▼
        dozerd —— session daemon（tokio 常驻）
        ├── PTY 池（portable-pty）+ 滚屏缓存      ← 会话存活
        ├── 项目/目标/验收/演进史（SQLite + git）  ← 验收闭环
        ├── 事件总线（tokio::broadcast）           ← timeline / 对话结构化
        ├── 进程托管（mlx_lm.server 等，二期组件视图呈现）
        └── agent 适配器（一期仅 claude；接口预留 codex/…）
```

仓库结构：本仓 workspace 化；新增 `dozer-core` / `dozerd` / `dozer-app` / `dozer-hook`。**boy CLI 废弃**（见 §3 裁剪）：现有 `src/` 的 config/process/doctor 代码作为 dozerd 种子迁移后删除，仓库重心全面转向 Dozer。

## 6. 领域模型与数据流

**实体**：项目 Project（概念模型中的作品 Work，绑 git 仓库）· 目标 Goal · 验收标准 Criteria（文字标准 + 可执行标准如 `cargo test`）· 会话 Session（含 agent 类型与 resume id）· 交付 Delivery（git diff 范围 + 文件清单 + 摘要）· 验收 Acceptance（通过/打回 + 意见 + 验收人[四期留门]）· 演进史 History（仅验收通过者推进）· 事件 Event（append-only）。

**存储（最小可逆）**：结构性记录进 daemon 本地 SQLite；文件态资产留项目仓库归 git；演进史同时落 git ref（`refs/dozer/accepted/<n>`）。不锁死二期 MCP/GitHub 方向。

**数据流**：
1. **立项**：新建项目 → 目标+验收标准（模板关口）→ 发起 agent 会话（daemon spawn PTY）。
2. **交付—验收**（核心闭环）：agent 干活 → hook/transcript 事件 → 交付声明 → daemon 算 diff → 验收视图（左：逐文件 diff/Flyfish 预览；右：验收标准清单+机器预判）→ 通过（推进演进史+git ref）或打回（意见一键注回 PTY 会话）。全程不离开 Dozer。
3. **对话结构化**：dozer-hook → daemon 事件总线 → 对话审阅视图（人类锚点/折叠/待决置顶）；终端 pane 保留裸对话为事实真相。

**关键设计判断**：验收视图 = 预览 + diff + 标准清单三合体（"预览是验收现场"的具体形态）。

## 7. 元素结构与 UI 布局（Figma 讨论产出，2026-07-15）

**两个顶级元素**：项目（甲方资产域：演进资产集/目标/验收标准/交付/验收/演进史/过程记录/模板配置）与 Agent（乙方执行域：身份安装/全局配置/用量配额/能力标签，只放与项目无关的东西）。

**会话 = 项目 × Agent 的二维表**：每个会话是一个单元格；户口在项目（对话史是项目的过程性资产，agent 可更换），可按 agent 维度分组透视。

**四栏主界面**（Figma: [Dozer Phase 1 UI](https://www.figma.com/design/NXfLQp5XQk1kF7Ohls2EbX)，参考 kooky 截图 `design/参考/`）：

| 栏 | 域 | 视图切换 |
|----|----|---------|
| 左一 项目栏 | 项目域 | 项目卡 + 单视图内容区（文件树 / git / 组件，**底部 tab 切换**——真实项目文件多，不混排）；git 段常显当前分支与脏标记（`main*`）；组件=配置文件的可视化预览（数据库、模型运行状态）；底部 doctor。顶部视图位留给任务等项目管理级视图（二期） |
| 左二 资产预览 | 项目资产的预览与验收现场 | 一切资产皆可预览成 tab：文件（Flyfish 渲染）、diff、**网页**（localhost 产物为主，外部 URL 极简地址栏）、**会话审阅**（对话史=过程性资产，结构化预览：交付卡+人类锚点+折叠 AI 回合）。状态条含"AI x 分钟前修改" |
| 左三 终端 | 会话域·执行现场 | 会话 tab（claude/codex/shell）；交付待验收横幅 + 进入验收 CTA；状态胶囊（agent 状态、Context %、dozerd 持有） |
| 左四 AI 栏 | AI 域 | Agents 简卡（名称+状态+用量，**不含会话列表**，避免与终端 tabs 重复）；Memory（项目级、跨 agent，三期）与编排（多路对照/派单，二期）为带分期角标的折叠占位行 |

跨域元素：顶部居中全局搜索（⌘K）、标题栏目标锚（当前迭代目标常驻）、右上角设置入口。无底部全局状态栏（刻意留白，等有重要信息再启用；agent 用量配额显示在 Agent 卡片内）。

**核心动线与页面清单**（从首次启动到交付的一轮循环，Figma 共 12 帧：首启 2 + 项目中心 1 + 主流程 6 + 辅助 3）：

| # | 环节 | 页面 |
|---|------|------|
| ⓪a | 首启·环境检测（一期降级版：检测 agent CLI + 一键注册 hooks；不做会话/配置导入） | W1 |
| ⓪b | 首启·选择主题（ByteBoy2077 默认选中，深空灰/浅色备选；**一期不实现**——单主题，设计稿保留） | W2 |
| ⓪c | 启动主界面·项目中心（左栏 248px：Dozer logo、项目搜索、最近活跃 5 项目——名称/活跃时间/路径/git 分支、更多项目+新增项目；右栏一期仅"最近的文件/最近的对话"两卡，日历（简单控件）与社区（easyeasyai.com）板块随后补） | H0 |
| ① | 新建项目（目录 + git 检测 + 领域模板） | S0 |
| ② | 定义目标与验收标准（关口一：无标准不派活；可执行/人工两类标准；AI 草拟入口，用户终审） | S0b |
| ③ | 派活（S0b 的 CTA"保存目标，派活给 claude"） | — |
| ④ | 执行中（四栏工作区，agent 干活） | S1 |
| ⑤ | 会话审阅（左二资产 tab：锚点时间线 + 交付卡） | S1b |
| ⑥ | 验收（diff + 标准清单 + 通过/打回） | S2 |
| ⑦ | 沉淀（目标锚变绿✓；左一 git 视图演进史 v13；左二预览"v13 验收记录"——验收记录也是资产，含 4/4 判定、查看 diff、回放会话审阅） | S3 |
| ⑧ | 下一个目标 → 回② | — |
| 辅 | 组件视图（dbx 集成）——**二期后移** | S1c |
| 辅 | 设置·Agents（各 agent CLI：路径/版本检测、启用开关、hooks 与 transcript 适配器注册状态、resume 能力；派活默认 agent） | SET1 |
| 辅 | 设置·集成（外部编辑器默认打开方式与检测；DBX：二期；Flyfish：内置离线资产与沙箱说明）。设置导航规划 8 节：通用/外观/Agents/模型(MLX)/领域模板/集成/快捷键/关于——**一期仅实现 Agents + 关于** | SET2 |

打回支线：S2 打回 → 意见注回会话 → 回 ④。

**S2 验收视图**（全屏态，自"进入验收"进入，可返回工作区）：左=变更文件列表（±行数）；中=diff 视图（并排/行内切换）；右=验收标准栏——目标 + 标准清单（**可执行标准显示机器预判**如 `cargo test 12 passed`（绿勾自动），人工标准由用户勾选（金勾），进度 2/4）+ 验收意见输入（打回时自动注回来源会话）+ 双动作：`通过·沉淀为 v13`（金 CTA，落 git ref `refs/dozer/accepted/13`）/ `打回并注回会话`（红描边）。

**S1c 组件视图**（左一切换态）：组件卡片（Postgres/Redis/mlx 等：状态点、端口、规模、动作——"在 DBX 中打开↗"/"▶ 启动"/"日志"）+ 来源配置文件（点击跳左二预览）。

**默认主题「ByteBoy2077」**：以 [Blade Runner 2049 Zed 主题](https://github.com/takk8is/blade-runner-2049-theme-for-zed)为基准、按 byteboy.ai 官网配色微调而成，命名归品牌所有。核心 token：窗口底 `#0a0e16`（官网）、面板 `#0e1620`、终端底 `#08141d`、卡片 `#12202a`、边框 `#1c3440`（官网 `#24444f` 系）；主文字 `#FFE5B4`（BR2049 奶油色）、正文 `#9AB4C4`、弱文字 `#6B7F8F`；**主强调金黄 `#F2D94E`**（官网金，用于目标/交付/验收等甲方动作）、次强调青 `#47DEF0`、运行绿 `#1AD585`（BR 终端绿系）、等待紫蓝 `#9580FF`（BR ansi blue）、品红 `#FF3DCC` 保留给通知类点缀（未用）。

## 8. 显式未决（勿擅自定死）

- 过程性资产（任务/缺陷等）自建精简存储 vs 接 MCP/GitHub —— 二期决断。
- 软件模板关口的具体位置与松紧 —— dogfooding 定。
- 多路择优、AI 辅助生成验收标准的深度 —— 后续版本（S0b 仅保留"AI 草拟、用户终审"入口）。
- H0 社区板块的内容源 —— **已定方向**：由自建的 easyeasyai.com 提供，具体 API 后续绑定；实现时先静态占位 + 预留内容拉取接口。
- H0 日历 —— **已定方向**：一期提供简单日历控件，数据仅从项目事件（验收、目标）派生；二三期考虑集成用户的 Apple 日历 / Google 日历（作为集成适配器，Apple 侧 EventKit 属平台专属能力，须隔离在适配层不进核心，遵守"架构留门"）。"例行任务"类调度是后续能力。
- W1 导入的具体范围（Claude Code 会话/hooks/CLAUDE.md 各导入到什么程度）—— 实现时按最小可用裁剪。

## 9. 一期实施顺序建议（供实现计划参考）

风险与依赖驱动：① wry×iced 合成 spike（头号风险，第一周）→ ② dozerd 会话存活内核（PTY 池 + UDS + resume）→ ③ 终端 pane（alacritty_terminal 网格）→ ④ 预览 pane（wry+Flyfish）→ ⑤ 验收闭环（S0/S0b 立项定标 → 交付横幅 → S2 验收 → S3 演进史）→ ⑥ 对话可审阅性（dozer-hook + transcript 适配器 + 审阅 tab）→ ⑦ 周边页面（W1/W2/H0/设置）与 dbx 组件视图。①—④ 是地基，⑤⑥ 是灵魂，⑦ 可并行后置。
