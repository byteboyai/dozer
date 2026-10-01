# byteui 独立仓库化设计

日期：2026-10-01
状态：设计已逐段确认，待审阅

## 1. 背景与目标

`byteui` 目前是 `dozer/crates/byteui`（约 4300 行，iced 0.14，73 次提交历史），dozer-app 通过
`path = "../byteui"` 引用。`dozer-silent1` 里还有一份内容相同的独立副本。

byteboy 下的 **dozer** 与 **digger** 使用相同技术栈（Rust + iced 0.14），Digger 的 Phase 1 桌面 UI
设计稿（Figma "ByteBoy Digger — Phase 1 Desktop UI"）与 dozer 的整体框架一致：顶栏项目页签、左侧图标
rail、左侧树面板、中央工作区、右侧面板；配色语义一致（金 = 甲方动作、青 = 选中/聚焦、绿 = 健康/完成、
红 = 冲突）。Digger 设计稿里的 token 取值不够精确，但大框架一致。

**目标**：dozer 与 digger 共用同一个 byteui，改一处，两个项目一起得到。

**成功标准**
- 新项目加一个 git 依赖就能用 byteui；换主题只靠 `set_theme()`，不复制、不改 byteui 源码。
- dozer-app 的调用点不改（函数签名不变）。
- byteui 仓库自己有 CI 与组件展示示例。

**非目标**
- finmeter（倾向 GPUI）的适配，以及为它拆分 `byteui-tokens`：等 finmeter 开始写代码再做，那时边界才清楚。
- 跨框架组件规格、JSON schema 驱动 UI。
- Digger 专属组件（统计卡片、前后对比卡片）。

> 注：Digger 的 `docs/ByteBoy_Digger_Technical_Architecture_v0.1.md` 仍写着 Tauri 2 + React，
> 与"digger 与 dozer 同技术栈"不符，应在 digger 侧另行更新，不在本设计范围内。

## 2. 仓库形状

- 新建仓库 `byteboyai/byteui`，用 `git filter-repo` 把 `crates/byteui` 的提交历史带过去。
- 仓库内只含这一个 crate，布局保持现状：`theme/ interaction/ layout/ form/ feedback/ data/` 与
  `assets/icons`。
- 语义化版本，打 tag（`v0.1.0` 起）。消费方使用
  `byteui = { git = "https://github.com/byteboyai/byteui", tag = "vX.Y.Z" }`，**用 tag 不用 branch**。
- iced 版本跟随 dozer（0.14）。所有消费方必须使用同一个 iced 版本，否则 `Element` 类型不兼容；写进
  README 与 CI，升级 iced 时 dozer、digger、byteui 一起升。
- 本地联调：在消费方的 `.cargo/config.toml` 或根 `Cargo.toml` 里加
  `[patch."https://github.com/byteboyai/byteui"] byteui = { path = "../byteui" }`，不提交即可联调。
- 拆出后 dozer 删除 `crates/byteui`；`dozer-silent1` 的副本同样处理（或确认其是否仍需保留）。

## 3. 什么进库，什么留在应用

**进 byteui**
- `theme` 的 color / font / geometry / icon_size：默认值为 ByteBoy2077，可在运行时用 `set_theme()` 整体替换。
- `interaction`（icon 按钮、tab、卡片、滚动条、右键菜单）、`layout`、`form`、`feedback`
  （toast、进度、状态点）、`data`，以及图标 SVG。

**留在各应用**
- `region`、`terminal_font`、`homespace_*`，以及依赖它们的 `tree_row_h` /
  `tree_chrome_top_px` / `tree_chrome_bottom_px`：这是应用自己的面板划分，进库会带上 dozer 的业务约定。
- 应用自己的 `theme::init()`：启动时读自己的 JSON 并调用 `set_theme()`。digger 初期直接用默认值，后续按需微调。
- `icon_size` 缩放配置路径由应用传入（现状如此），digger 传自己的路径。

**拆出前要改的三处**
0. `icon_size` 读取的环境变量 `DOZER_ICON_SCALE` 是 Dozer 专属名字。库里改为优先读
   `BYTEUI_ICON_SCALE`，同时兼容旧名 `DOZER_ICON_SCALE`（已有用户的设置不失效）；非数字、0、负数视为
   未设置。（评审实现计划时补记。）
1. 测试 `byteboy2077_matches_dozer_app_baseline` 引用了 dozer-app 的取值，拆库后不能保留。改成 byteui
   内部的锁值快照；dozer-app 另加一条测试，说明自己的 JSON 与默认值的差异是有意为之。
2. 注释中提到 dozer-app 文件路径的地方（`search_box.rs`、`toast.rs`、`theme/color.rs`、`theme/font.rs`
   等）改成通用说法。

## 4. 新增组件与准入规则

**先补三样**
- **Badge**（角标：数字、红点）与 **Tag**（带文字的标签，如"可审查""冲突""仅本地"）：分成两个组件，
  与 amis 保持一致。这推翻了 `2026-08-18-byteui-library` 计划中"不做 Badge/Tag"的决定，原因是现在
  Digger 设计稿明确需要，已有第二个使用者。
- **Button**：金色主按钮、次要描边按钮、禁用态，支持带 icon。

**准入规则（写进 README）**
- 至少两个项目需要，或设计稿中已明确出现，才进库；只有一个项目用到的留在该应用里。
- 命名沿用 amis 组件名。
- 组件无状态：`fn(参数) -> Element`，颜色、字号、间距一律读 token，不写死数值。
- 每个组件有最小示例或测试，覆盖其各种状态。

**版本节奏**
- 加组件为 minor；改已有签名为 breaking，即使在 0.x 也要在 CHANGELOG 里写清楚，并在 dozer 与 digger
  同时升级。

## 5. 迁移步骤

每一步可独立验证、可回退。

1. 在 dozer 仓库内先改 byteui 的注释与基线测试（§3 两处），跑 `cargo test -p byteui`。
2. 用 `git filter-repo` 导出 `crates/byteui` 历史到新仓库，推到 `byteboyai/byteui`，打 `v0.1.0`
   （内容与当前完全一致）。
3. dozer 独立分支：把 `path = "../byteui"` 改为指向 `v0.1.0` 的 git 依赖，删除 `crates/byteui`，
   全 workspace 跑 build / test / clippy。dozer-app 调用点不改。
4. `dozer-silent1` 同样处理，或确认是否保留。
5. 在 byteui 仓库补 Badge、Tag、Button，发 `v0.2.0`。
6. digger 建 Cargo 项目时直接依赖 `v0.2.0`。

## 6. 测试与 CI

- GitHub Actions：`cargo fmt --check`、`cargo clippy`、`cargo test`。
- 保留锁值测试，防止默认配色被误改。
- 新增 `examples/gallery`：在一个窗口里列出所有组件的各种状态；改组件后在此目测，也是日后与 Figma 对照的地方。

## 7. 风险与对策

| 风险 | 对策 |
|------|------|
| dozer 工作目录有未提交改动，且常有其他会话或 WorkBuddy 在 main 上并发提交 | 迁移在独立分支（worktree）里做；开工前 `git status`，有无关改动先 `git stash push -u` 保留；合并前再核对一次 |
| 两个仓库来回切换，开发体验变差 | 用 `[patch]` 本地联调，并把用法写进 dozer 的 CLAUDE.md，避免有人去改 `~/.cargo/git` 缓存 |
| iced 版本不一致导致类型不兼容 | byteui 的 README 与 CI 写明；升级 iced 时三个仓库一起升 |
| dozer 历史 plan/spec 大量引用 `crates/byteui/...` 路径 | 历史文档不改，仅在 CLAUDE.md 加一句"byteui 已迁到独立仓库" |
| 消费方用 branch 依赖导致被动变更 | 规定一律用 tag |

## 8. 未决项

- 仓库名为 `byteui`、历史随迁：已确认。
- `dozer-silent1`：已确认它是 dozer 的一个 git worktree（分支 `feat/silent-failures-batch1`，领先 main
  8 个提交、未推远程），不是独立副本，**必须保留**，不在本次处理范围；它不改动 byteui，拆分合并后
  `git rebase main` 即可。
- finmeter 的接入与 `byteui-tokens` 拆分：待 finmeter 开始实现时单独立项。
