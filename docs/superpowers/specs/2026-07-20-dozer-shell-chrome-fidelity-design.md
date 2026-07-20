# Dozer Shell Chrome 还原度设计（P1k）

> 状态：设计中，待用户终审。
> 需求来源：2026-07-20 对话（用户："把 UI 的还原度做出来，这样就基本可用了"）。
> 权威参照：Figma "Dozer Phase 1 UI" S1 主帧（node `2:2`，fileKey `NXfLQp5XQk1kF7Ohls2EbX`）。
> 上游规格：`docs/superpowers/specs/2026-07-14-dozer-phase1-design.md` §7（元素结构与 UI 布局）。

## 1. 目标与还原度标杆（已确认）

把 `dozer-app` 的外观从"功能可用但粗糙"提到"整体气质接近 Figma、协调可用"。

**标杆 = 神似 · 高影响优先**（用户确认）：追求整体气质与协调，不逐像素抠。间距/字号取近似值，
优先补最影响"完成感"的 chrome（顶栏、状态栏、图标、卡片）。**非目标**：像素级对齐、
动效、尚无数据支撑的真实功能（搜索检索、设置面板内容、Context% 计量）——这些以视觉占位立骨架，
接线各自排后续迭代。

## 2. 现状与差距

`view()` 现为 `row![project, preview, terminal, ai]`——四栏功能俱在，但缺全部"外壳 chrome"，
每个 pane 都是 `column!` 的 `text`/`button` + 基础 `container` 样式。差距是**均匀的"chrome + 打磨"，
不是结构性重写**。14 个主题色（`theme.rs`）与四栏 tab 栏已就位，骨架正确。

| 区域 | 现状 | Figma 目标 |
|------|------|-----------|
| 顶栏 | **完全缺失** | 窗口标题行 `Dozer` + ⌘K 搜索框 + 金色"● 目标：…"胶囊 + 设置齿轮 |
| 状态栏 | **缺失** | 项目栏底"● 环境正常 · dozerd 运行中" + [文件\|git\|组件] tab；终端栏底"● 思考中 · Context · resume ✓ · dozerd 持有" |
| 文件树 | `▸/▾` 文本字形 + git 变色 | 文件夹/文件图标 + 彩色状态点 |
| 卡片/pill | 平铺文本按钮 | 项目卡圆角 + "N 次验收"、agent 用量条、激活 tab 的 pill 态、goal/横幅圆角 |

## 3. 架构与分解

一个连贯子系统（**外壳 chrome**），落为**一个 spec、4 个可独立交付/验收的 task**，按视觉影响排序。
采用**逐区域增量（方案 A）**：每步 diff 小、可单独 dogfood 验收、隔离干净。（否决方案 B 一次性像素
重建 view()——风险大难审；否决方案 C 先抽 widget 库——在只有一处用处时前置抽象，YAGNI。）

`view()` 由 `row![四栏]` 改为 `column![topbar, row![四栏], (状态栏并入各栏底部)]`——topbar 是唯一
顶层结构变化；状态栏并入各自 pane 的 `column!` 末尾，不改 `row!` 骨架。

### Task 1 — 顶栏（最高可见度，先立框）

固定高 ~44px 的 `container`，横跨窗口顶部，底色 `BG`，`row!` 三段：
- **左**：`Dozer` 标题（`CREAM`，size 15）。macOS 交通灯属系统窗口装饰，**不自绘**。
- **中**：⌘K 搜索框——圆角 `CARD` 底、`BORDER` 边、占位符文本"搜索作品、会话、产物… ⌘K"（`DIM`）。
  **纯视觉占位**（无检索逻辑），点击可先无响应或聚焦；实搜索排后续。
- **右**：金色目标胶囊"● 目标：{goal.title}" + 设置齿轮（`DIM`，占位无面板）。
  胶囊：`GOLD` 圆点 + `CREAM` 字、`CARD` 底、圆角 6、`GOLD` 细边。goal 读已有
  `goal::parse_goal(.dozer/goal.md)`（P1f 已解析，终端交付横幅已在用 `Goal.title`）。
  无 goal / 无项目时胶囊隐藏。标题过长则截断加省略号（纯函数 `goal_capsule_text`）。

### Task 2 — 状态栏（项目栏底 + 终端栏底）

各 pane `column!` 末尾追加一条固定高（~26px）状态条，`PANEL`/`TERM_BG` 略深底 + `BORDER` 上边线：
- **项目栏底**：左"● 环境正常 · dozerd 运行中"（daemon 连通=`GREEN` 点，`daemon_error` 时=`RED`
  "dozerd 未连接"）；右侧 [文件\|git {branch}\|组件] 三段——**"文件"当前态高亮，git 段复用已有
  branch/dirty，"组件"占位**。
- **终端栏底**：当前激活 tab 的 agent 态——"● {状态中文} · resume {✓/—} · dozerd 持有 · 断连可恢复"。
  状态点复用已有 `dot_color(agent_state, alive)`；状态中文由纯函数 `agent_state_label(AgentState)`
  给出（Running→"运行中"、AwaitingInput→"待输入"、TurnEnded→"回合毕"、Idle→"空闲"）。
  Context% 无真实数据 → 省略该段（不占位假数字）。

### Task 3 — 文件树图标 + 状态点

替换 `▸/▾` 文本字形：目录用展开/收拢三角图标 + 文件夹字形，文件按扩展名给通用文件字形
（用现有字体的 Unicode 符号，不引外部图标资源——`mac 先发但不引专属能力` 裁决）。git 状态从
"文字变色 + 后缀标记"改为**行尾彩色圆点**（改=`GOLD`、新=`GREEN`、删=`RED`，目录 rollup 复用
已有 `delivery::dir_status`）。图标/点映射为纯函数 `tree_row_glyph` / `tree_row_dot`，可单测。

### Task 4 — 卡片 / pill 打磨

- **项目卡**：项目名 + 路径 + "N 次验收"副行，包进圆角 `CARD` 容器（现为平铺文本）。
  验收数经 dozerd 已有 `AcceptanceStore::count()` 暴露——需**协议加一字段 + client 一方法 +
  workspace 启动/验收后载入**（小而定义清晰的接线）；未取到时省略该副行，不显 0。
- **激活 tab pill**：四栏 tab 栏的当前 tab 加 `CARD` 底 + 圆角高亮态（现仅文字色区分）。
- **agent 卡 / 交付横幅**：agent 简卡与金色交付横幅补圆角与内边距，贴近 Figma；agent 用量条
  （已用 %）**有真实数据才画，否则不画**（P1e 无用量遥测，故本期多半不画）。

## 4. 数据与错误处理

- 全部新增视觉**降级为空态而非崩溃**：无 goal→无胶囊；无项目→顶栏中/右段照常（搜索框在、胶囊隐）；
  daemon 断→状态栏红字"dozerd 未连接"；验收数取不到→无副行。
- 视觉占位元素（搜索、齿轮、组件 tab）**明确无副作用**：不误触发、不显假数据。
- 主题色一律取自 `theme.rs`，不新增硬编码色值（防漂移锚测试已覆盖 BG/CREAM）。

## 5. 测试策略

iced `view()` 难做单元断言，故**把可判定逻辑抽成纯函数并单测**，视觉本身走人工 dogfood 验收：
- `goal_capsule_text(title, max_len) -> String`（截断/省略号）。
- `agent_state_label(AgentState) -> &str`（四态中文）。
- `env_status_text(daemon_ok) -> (&str, Color)`（环境/未连接）。
- `tree_row_glyph(is_dir, expanded, name)` / `tree_row_dot(status) -> Option<Color>`。
- 验收数：`AcceptanceStore::count()` 已有 roundtrip 测试；新增协议字段编解码测试（沿用 protocol.rs
  既有 `old_*_decodes_none` 前向兼容模式）。
- 每 task 收尾 `cargo clippy --all-targets && cargo fmt` 干净、全量 `cargo test` 绿。
- 每 task 末做一次真机 dogfood 目测比对 Figma S1，记录到验收档。

## 6. 显式非目标（本期不做）

搜索检索实现、设置面板、Context% 计量、agent 用量遥测、动效/过渡、⌘K 打开搜索态（现仅
"⌘K 隐藏预览"裁决，未实现）、像素级对齐。均以视觉占位或省略立骨架，接线各自排后续。
