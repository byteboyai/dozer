# Dozer 图标系统设计

> 状态：设计中，待用户终审。
> 需求来源：2026-07-28 对话（用户："为 dozer 引入一套图标"）。
> 上游背景：P1k（`docs/superpowers/specs/2026-07-20-dozer-shell-chrome-fidelity-design.md` Task 3）曾
> 明确决定"用现有字体的 Unicode 符号，不引外部图标资源"——这是当时的收窄范围之举，非架构硬限制；
> 本设计是对该决定的主动重开，不是推翻裁决。

## 1. 目标与现状

`dozer-app`（iced 0.14 GUI）目前全部用纯 Unicode 字符占位视觉元素：项目树展开箭头 `▸/▾`、
git/会话状态点 `●`、验收勾选 `✓/○`、警告 `⚠`，右键菜单是纯文字标签无前缀图标，顶栏搜索框/设置齿轮
（`⚙`）也是未接线的占位字符。目标：引入一套真正的图标系统，覆盖项目树文件夹/文件类型、顶栏动作、
右键菜单前缀、并评估状态指示是否要跟进（结论见 §4）。

**非目标**（本轮明确排除，详见 §5）：不做多色语言品牌图标、不重做状态指示的形状语义、不做用户可自
定义图标、不引入亮色主题变体、不做 Lucide 全量 1500+ 图标的自动化子集工具。

## 2. 技术选型

### 2.1 图标来源：Lucide（现成开源图标集）

[Lucide](https://lucide.dev)，MIT 协议，约 1500 个图标，24×24 viewBox 线性风格、`currentColor`
描边，是当前开发者工具生态里最常见的图标集选择（VS Code 插件生态、Linear 等大量同类工具在用）——
比 Phosphor 这类更装饰性的图标集更贴合 Dozer"工程师工具"的定位。备选 Tabler Icons（同为 MIT、图标
量更大）风格相近，选 Lucide 主要因为它是这个领域的事实标准，后续缺图标的概率更低。

**不做**自绘图标或多色品牌图标集（如 vscode-icons 式按语言给专属 logo）——见 §2.3 的单色约束。

### 2.2 渲染：iced 原生 `svg` 组件

`iced_widget` 0.14.2 自带 `svg` 模块（`Style { color: Option<Color> }` 支持运行时着色），只是当前
`Cargo.toml` 未开启该 feature（现只有 `["wgpu", "canvas"]`）。渲染管线是纯 Rust（`resvg`/
`tiny-skia`），不引入 Node/Python，符合 CLAUDE.md"核心不依赖 Node/Python"的裁决。

### 2.3 资产管线：编译期内嵌的精选子集

不整体 vendor 全部 Lucide 图标。只挑选本轮实际要用的 ~16 个 `.svg` 文件放进
`crates/dozer-app/assets/icons/`，通过 `include_bytes!` 在编译期内嵌进二进制（不依赖运行时文件系统
路径，`cargo run` 即用，与仓库现有资产哲学一致）。新增 `crates/dozer-app/src/icons.rs` 模块：

- `pub enum IconKind { ChevronRight, ChevronDown, Folder, FolderOpen, FileCode, FileJson, FileText,
  FileConfig, FileImage, FileGeneric, Search, Settings, FilePlus, FolderPlus, Copy, ClipboardPaste,
  Trash, Rename }`（穷举，不留开放式扩展口子——新增图标 = 加一个变体 + 一个 svg 文件，同
  `theme.rs` 精选 14 色而非任意色值的哲学一致）。
- `pub fn view(kind: IconKind, size: f32, color: Color) -> Element<'_, Message, Theme, Renderer>`
  ——统一入口，调用方原有 `.color(theme::XXX)` 的调用习惯不变，只是把 `text("▸")` 换成
  `icons::view(IconKind::ChevronRight, 14.0, theme::BODY)`。
- 每个 `IconKind` 对应一个 `include_bytes!` 常量 + `svg::Handle::from_memory(...)`。

**Lucide 是 MIT 协议**——vendor 进仓库需要一份简短署名，放
`crates/dozer-app/assets/icons/LICENSE`（内容为 Lucide 的 ISC/MIT 声明原文）。

## 3. 覆盖范围与映射

### 3.1 项目树：文件夹 / 文件类型（`workspace.rs` 约 2019/3139-3140 行的 `tree_row_glyph` 一带）

- 展开状态：`chevron-right` / `chevron-down`，替换 `▸/▾`。与文件夹图标分离（IDE 惯例：箭头管展开
  态，图标管类型），不用文件夹开合形状兼职表达展开态。
- 目录本身：`folder`（收起）/ `folder-open`（展开）。
- 文件按扩展名，走**按类别的通用图标**而非按语言的品牌 logo（见下方 caveat）：
  - 代码类（`.rs .js .ts .py .go ...`）→ `file-code`
  - `.json` → `file-json`
  - 文本/文档类（`.md .txt`）→ `file-text`
  - 配置类（`.toml .yaml .yml`）→ `file-cog`
  - 图片类（`.png .jpg .jpeg .svg .gif`）→ `file-image`
  - 其余/未知扩展名 → `file`（通用兜底）

  **权衡说明**：Lucide 是通用 UI 图标集，不是 vscode-icons/Material Icon Theme 那种"每种语言一个
  专属彩色 logo"的文件类型品牌集。品牌 logo 通常自带颜色，无法套用"调用方传一个主题色"的单色着色
  模型（§2.2 的 `Style.color`）。按类别的通用图标是与"整套图标统一走主题色着色"这个决定天然匹配的
  选择；若未来确实需要按语言品牌化，那是引入第二套图标源的更大改动，不在本轮范围。

### 3.2 顶栏动作图标（`workspace.rs:2168-2225` `top_bar`）

现状核对：搜索框目前是纯占位文字`"搜索作品、会话、产物… ⌘K"`（未接线），设置位是未接线的
`text("⚙")`。本轮只换图标，不新增点击行为：
- 搜索框前缀 `search` 图标。
- 设置位换成 `settings` 图标（替换 `⚙`）。

### 3.3 右键菜单图标（项目树右键菜单，上一会话刚落地）

每个菜单项标签前加一个前缀图标：
- 新建文件 → `file-plus`
- 新建文件夹 → `folder-plus`
- 复制 → `copy`
- 粘贴 → `clipboard-paste`
- 删除 → `trash-2`
- 重命名 → `pen-line`
- 复制绝对路径 / 复制相对路径 → 两项复用同一个 `copy` 图标（同一个"复制"动作、不同数据，不是两个
  视觉上不同的动作）

### 3.4 状态指示（git/会话/验收状态点）—— 结论：本轮不迁移

现状：`●`（git 状态点、会话活跃点）与 `✓/○`（验收勾选）都是 `text()` 直接按 `theme::XXX` 着色，
已经是"纯色驱动状态"且已经过主题色着色——SVG 化不会改变任何视觉或行为，只是换一种方式产出同样的
像素。讨论过是否顺带把状态从"纯色圆点"改成"按状态给不同形状"（提升色弱可读性），**用户明确选择
维持现状**：只做颜色区分，不引入形状语义。据此，状态指示点/勾选**保持 `text()` 实现，不纳入本轮
图标迁移**——迁移它不会带来任何差异，属于本可以省略的工作量。

## 4. 非目标（本轮明确排除）

- 按语言品牌化的多色文件类型图标（§3.1 caveat）。
- 状态指示的形状语义重设计（§3.4，用户已确认维持纯色点）。
- 用户可自定义/替换图标。
- 亮色主题变体——Dozer 目前只有 ByteBoy2077 一套主题。
- 图标动效。
- 自动化的 Lucide 子集抽取工具——手动 vendor 本轮所需的 ~16 个文件，未来加图标手动加文件+加
  `IconKind` 变体即可，工具化在图标数量真正膨胀前是过度设计。

## 5. 测试

扩展名 → `IconKind` 的映射是纯函数，走单元测试（同仓库现有 `tree_row_glyph`/`tree_row_dot` 的测试
模式）。SVG 实际渲染效果无法在 headless 环境下有意义地断言——与仓库现有 view 层"编译通过 + 真机
目测"的既定验收方式一致（上一会话的右键菜单功能采用的是同一套约定）。

## 6. 开放问题

无——本设计的所有分支点（图标来源、渲染方式、覆盖范围、状态点是否迁移）均已在对话中与用户确认。
