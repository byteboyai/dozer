# byteui 剩余未尽事项

整理日期：2026-10-16（同日已处理 README 示例、CHANGELOG `0.4.2`、本地旧 tag 三项，见 §2；这些文档提交在 byteui 本地提交 `6748902`，未推送时远程仍是旧内容）。byteui 当前 `v0.4.4`（仓库 `byteboyai/byteui`，`main` 与远程同步，无本地未推送提交）。dozer、digger、bytehost（`bytehost-chrome`）均依赖 `v0.4.4`。本地验证：`cargo test` 85 个测试通过；`clippy` 无 byteui 自身告警（只有依赖 `block` 的 future-incompat 提示）。

权威来源：`docs/superpowers/specs/2026-10-01-byteui-standalone-repo-design.md`（§8 未决项）、`2026-08-18-byteui-library-design.md`、`2026-08-19-byteui-runtime-theme-tokens-design.md`。拆库本身已完成；以下是拆库之后仍悬着的事。

## 1. 最大的一项：外壳区域样式 `theme::region` 两份并存

- **现状：** `v0.4.3` 给 byteui 新增了 `theme::region`（顶栏、左右图标栏、左右面板区、放大浮层、底栏等 7 个外壳区域，257 行），供 `bytehost-chrome` 共享窗口骨架使用。dozer 自己仍保留一份 `crates/dozer-app/src/theme/region.rs`（602 行，15 个区域，数据源是 `assets/theme/workspace.json`）。两处数值靠 digger 仓库的 `scripts/check-shell-regions.sh`（byteui 的 `examples/dump_shell_regions.rs` 导出 JSON，与 dozer 的 `workspace.json` 逐项比对）人工比对，不是自动 CI。
- **与原设计冲突：** 拆库 spec §3 明确写“`region`、`terminal_font`、`homespace_*` 留在各应用，进库会带上 dozer 的业务约定”。`v0.4.3` 实际把 region 的一部分搬进了库，spec 没有同步修订。byteui 里的版本只覆盖 7 个外壳区域，dozer 的 15 个区域里另外 8 个（7 个面板内容区 + 整体外框）仍只在 dozer。
- **风险：** 改 dozer 的 `workspace.json` 忘了同步 byteui，digger 与 dozer 的外壳就悄悄不一致；一致性检查脚本不在任何 CI 里。
- **要做的事（先定方向再动手）：**
  1. 裁决：dozer 迁移到 byteui 的 `region`（合并为一份），还是接受两份并存、把一致性检查接入 CI。
  2. 若合并：dozer 的 `theme/region.rs` 与 `workspace.json` 里这 7 个区域改读 byteui；剩余 8 个面板区域留在 dozer；`check-shell-regions.sh` 随之退役。
  3. 无论哪种，修订拆库 spec §3 的“留在应用”清单，使之与现状一致。
- **触发条件：** 下一次要改外壳区域数值，或 digger 的窗口骨架开始真正落地之前。

## 2. 版本与仓库卫生

| 项 | 现状 | 建议 |
|---|---|---|
| ~~本地残留的旧 tag~~ | **已处理（2026-10-16）**：本地删除了 `v0.5.151`/`v0.5.152`/`v0.5.153`/`v0.5.157`/`v0.5.158`（原指向 `35a494c`、`35a494c`、`dfc2185`、`6841bfc`、`3147ac6`，均为 `v0.4.4` 的祖先；远程本来就没有）。现在只剩 `v0.1.0`–`v0.4.4` | — |
| ~~远程遗留分支~~ | **无需处理（2026-10-16 核对）**：远程 `origin` 上只有 `main`（`git ls-remote --heads origin` 为证）。之前看到的 `feat/badge-tag-button` 等五个是本地过期的远程跟踪引用，`git fetch --prune` 已清掉；远程分支早已删除，并非未合并的遗留 | — |
| ~~README 示例版本~~ | **已处理**：示例改为 `tag = "vX.Y.Z"` | — |
| ~~CHANGELOG 缺项~~ | **已处理**：补了 `0.4.2` 一节 | — |
| CI | 只有 macOS 一个 job：`fmt --check`、`clippy --all-targets -D warnings`（含 examples）、`test`。没有与 dozer/digger/bytehost 的联合构建，发版前不知道会不会破坏消费方 | 见 §4 |

## 3. 组件与主题层面还没做完的

- **间距与圆角常量没进 geometry token。** README 明写“内边距、圆角等间距目前是组件内部常量（尚未进 geometry token）；某个项目需要调节时再把对应常量提成 token”。`feedback::dialog` 的 PANEL 底、金色描边（1.5）、圆角（8）也是组件内常量。触发条件：digger 或另一产品需要不同的间距/圆角。
- **没有 widget 渲染快照测试。** 测试只覆盖纯逻辑（设计时的有意取舍，workspace 里没有先例）。组件外观回归靠 `cargo run --example gallery` 目测。
- **dozer 内手写的悬停接线仍多。** 项目规则要求 icon 按钮/tab 类 UI 优先用 `icons::icon_button_entry`/`tabs::tab_core`，不要手写 `MouseArea` + `on_enter`/`on_exit`。目前 `on_enter` 仍出现在 8 个文件：`extensions/{project_create,ssh,settings,git_log}.rs`、`extensions/files/{state,view}.rs`、`chrome/{tab_widget,topbar}.rs`。其中有的属于“形状/交互明显不同”的合理例外（需要在 plan 里说明），有的可能可迁。需逐处盘点，不是一次性改动。
- **dozer-app 里仍有只有 dozer 用的“通用”控件**，是否应进库按“至少两个项目需要”判断。digger 的 `desktop` crate 目前只在 `main.rs`、`shell.rs`、`projects`、`apps/*`、`adapters/usage.rs`、`product.rs` 里引用 byteui；没有证据显示 digger 手写了与 dozer 重复的组件，所以暂无明确的“待搬迁清单”。digger 的设计稿要求出现新组件（统计卡片、前后对比卡片）时，属于 digger 专属，按 spec 非目标留在 digger。

## 4. 跨仓库流程缺口

- **iced 版本一致性没有自动检查。** README 要求 byteui、dozer、digger、bytehost 的 iced 版本相同，升级时一起升，但没有 CI 校验。digger 侧有 `scripts/check-host-deps.sh` 与共享依赖守卫（`docs: shared-deps guard`，见 `docs/v2` 依赖矩阵），byteui 自己的 CI 没有接入。
- **升级 byteui 要 dozer、digger 同日一起升**（digger `Cargo.toml` 注释里的 ADR），目前靠人工。`bytehost-chrome` 也固定 byteui `v0.4.4`，是第三个要同步的消费方。
- **本地联调配置不统一。** dozer 的 `.cargo/config.toml` patch 指向 `../byteui`；新建 worktree 时没有这份配置（该文件被 gitignore），需要每次手动建。

## 5. 明确延后的事（有触发条件）

| 项 | 内容 | 触发条件 |
|---|---|---|
| finmeter 接入与 `byteui-tokens` 拆分 | finmeter 倾向 GPUI，byteui 是 iced 组件库，无法直接复用；需要把 token（颜色、字号、几何）拆成不依赖 iced 的 `byteui-tokens` crate | finmeter 开始写代码时单独立项（拆库 spec §8） |
| `region`、`terminal_font`、`homespace_*` 迁移 | 2026-08-19 字体/几何迁移设计里“没有迁移计划”；其中 `region` 的一部分已在 §1 实际进库 | 随 §1 的裁决一并重新评估 `terminal_font` 与 `homespace_*` |
| digger 技术架构文档更正 | `digger/docs/ByteBoy_Digger_Technical_Architecture_v0.1.md` 仍写 Tauri 2 + React，与“digger 与 dozer 同技术栈”不符 | digger 侧另行更新，不属于 byteui |

## 6. 已确认不做

- JSON schema 驱动 UI（amis 那种）；完整 amis Table（排序/虚拟滚动/列定义）；跨框架组件规格。
- Digger 专属组件（统计卡片、前后对比卡片）进库。
