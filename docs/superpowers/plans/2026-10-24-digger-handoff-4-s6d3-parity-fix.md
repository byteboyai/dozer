# 交接任务 4:S6d-3 页签外观对照验收与修差异

> 给开发 agent 的任务说明,写于 2026-10-24。S6d-3 的代码已完成并合入 Digger `main`;本任务是 plan 的 Task 4 Step 2–3:与 Dozer 逐状态对照,把差异修掉。发布与压力回归不在本任务内。

**目标:** 让 Digger 的 Agent 页签、溢出菜单、悬停与提示的外观与 Dozer 一致,差异清单里不再有"待修"项。

**Plan(只读):**
`docs/superpowers/plans/2026-10-18-agent-s6d3-chrome-and-release.md`(Dozer 仓库)的 **Task 4**。先读顶部修订。

## 基线

- Digger 仓库 `/Users/chrischiang/Projects/CoralProjects/byteboy/digger`,基线 `main` 的 `52df4b9`(开工先核对)。
- 新建 worktree `../digger-s6d3-parity`,分支 `feat/agent-parity-fix`,从 `52df4b9` 切出。
- **不要动主工作区。** 它有别人未提交的改动(版本号 0.1.5、S6 文档删除、`.qoder/`)。
- 依赖已钉 `libs-2026-10-23b`,`Cargo.lock` 已提交,不需要本地 patch。
- 测试基线 536 个通过,0 失败。clippy 只允许 `block v0.1.6` 那 1 条警告。
- 已知偶发失败 `apps::tests::closing_during_startup_waits_and_releases_the_root` 是旧问题,已决定先放过,遇到不要处理,报告里注明即可。

## 先读哪里

1. `crates/desktop/src/adapters/agent_tabs.rs`:页签四件与度量常量(第 24–36 行)。
2. Dozer 的对照对象:`/Users/chrischiang/Projects/CoralProjects/byteboy/dozer/crates/dozer-app/src/chrome/tab_widget.rs`。
3. `crates/desktop/src/adapters/agent_display.rs`:接进库的 `PanelDisplay`。
4. `crates/desktop/examples/agent_gallery.rs`:对照画廊。
5. `docs/superpowers/notes/2026-10-18-agent-parity.md`:11 个状态的摆法与待填差异表。

## 要做的事

1. **Digger 一侧截图。** 对 11 个状态各跑一次:
   `cargo run -p digger-desktop --example agent_gallery -- --state <name>`
   状态:`empty`、`one-shell`、`grouped`、`active-card`、`many-tabs`、`narrow`、`overflow-open`、`picker-open`、`confirm-open`、`notice`、`daemon-unavailable`。
2. **Dozer 一侧截图。** 在 Dozer 里摆出同一状态并截图。**只读使用 Dozer,不要改它的任何代码或数据。**
   - 用 Dozer 的发布包或 `cargo run -p dozer-app`,数据目录和 `HOME` 用隔离的临时目录,不碰真实数据和常驻 `dozerd`。
   - 操作桌面界面优先用 `agent-desktop`;不要默认走 Dozer 自带的 MCP。
   - 某个状态在 Dozer 里摆不出来,在表里标"未对照"并写原因,不要编造。
3. **逐状态填差异表。** 写进 `agent-parity.md`,每项写:度量或颜色、Dozer 值、Digger 值、是否属于已接受差异。
4. **修差异。** 只改 Digger 一侧的 `pane_style` 与 `agent_tabs.rs` 常量。**不改库。** 修一项重拍一次。

## 已接受的差异(不要当缺陷修)

1. 悬停是阶跃(进入即满、离开即空),Dozer 是渐变。
2. 延迟提示精度 1 秒,Dozer 逐帧。
3. 覆盖层状态用 `picker` 与 `overflow` 两个互斥字段,而不是单个枚举。

## 约束

- 库不改。差异若出在库上,只报告,不在 Digger 里绕开。
- 不碰 `feat/preview-pairing`。
- 所有回归用隔离的 `DIGGER_DATA_DIR` 和临时 `HOME`,路径不要依赖含 "digger" 的目录名。
- 不合并 `main`、不打 tag、不推送、不发布。
- **不在本任务内:** 压力回归、守护进程死亡回归、同机并存、中文输入、Dozer 与 Digger 的发布。这些需要用户真机,由用户另做。

## 交付

- 差异表填完,"待修"项清零(或在报告里逐条说明为什么留下)。
- 截图存在 `docs/superpowers/notes/parity-shots/`,文件名 `<state>-dozer.png`、`<state>-digger.png`。
- 完工前 `git archive HEAD` 解到独立临时目录(目录名不带 "digger"),在其自己的 `target` 下跑 `cargo test --workspace --no-fail-fast`、`cargo clippy --workspace --all-targets`、`cargo metadata --locked --offline`。
- 报告写到 `docs/superpowers/reviews/2026-10-<日期>-s6d3-parity-report.md`:测试总数(基线 536 + 新增)、每个修过的差异的前后值,每条"通过"附命令与退出码。
- 停在 `feat/agent-parity-fix`。
