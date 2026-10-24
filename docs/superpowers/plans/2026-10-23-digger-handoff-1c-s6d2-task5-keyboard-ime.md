# 交接任务 1c:S6d-2 Task 5 键盘、指针焦点、粘贴复制、拖入与输入法

> 给开发 agent 的任务说明,写于 2026-10-23。权威细节在 plan 的 Task 5,本文只是入口与约束汇总。

**目标:** 让 Digger 的 Agent 终端能接收输入:键盘字节、指针焦点、⌘C / ⌘V、拖入文件路径、中文输入法的提交与预编辑。完成后 S6d-2 才算整体完成。

**Plan(只读):**
`docs/superpowers/plans/2026-10-18-agent-s6d2-sessions.md`(Dozer 仓库)的 **Task 5**(搜索 `### Task 5: 键盘、指针焦点`)。先读文件顶部的 2026-10-22、2026-10-23 修订。

## 前置与基线

- **解除阻塞:** 输入法探针(S6d-1 Step 4)五项已由用户真机验证全部通过(2026-10-23)。候选框、预编辑、提交、Esc 取消、点外消失、连续快速输入都没有问题,不需要选退路。
- **先做完 1b:** 本任务接在 `2026-10-23-digger-handoff-1b-s6d2-review-fixes.md` 之后,同一分支 `feat/agent-sessions`、同一 worktree `../digger-s6d2`。1b 未复核通过前不要开始。基线以 1b 完成后的顶端为准,先自己重跑确认测试数。
- 若 1b 里没人填探针笔记,顺手把 `docs/superpowers/notes/2026-10-18-ime-probe.md` 五项结果改为"通过(用户真机,2026-10-23)",并去掉末行"未填写结果前,不开始……"。

## 要做的事

按 plan Task 5 的 Interfaces 与 Step 1 测试清单实现,要点:

- `agent_keys::event_bytes(ev, app_cursor)`:`Key::Character` 用 `modified_key`,具名键用 `key`,⌘ 恒返回 `None`。
- `Msg::AgentKey`、`Msg::AgentPointer`、`Msg::AgentDrop`、`Msg::AgentPaste`;订阅的 `listen_with` 只在 `status == Ignored` 时产出,`Captured` 的按键不进终端。
- `AgentPanel::on_key`:面板不可见、无焦点、无活动会话、窗口关闭中都丢弃;⌘C 仅在有选区时写剪贴板;⌘V 读剪贴板,应用开启 bracketed paste 时按它粘贴。
- `AgentPanel::on_pointer`:`focus` = 点击是否落在 `terminal_rect` 内。`AgentDrop` 写入活动会话的 `dropped_path_bytes(path)`。
- 输入法:`ImeSink` 包住终端块,`enabled = focus && 可见`;提交走 `ime_commit`;预编辑存入 `panel.preedit` 供 `PanelDisplay::ime_preedit`;失焦或切走时清除。光标矩形 = 终端块原点 + (内边距, 页签条高度) + 光标格 × 单元格尺寸。
- 目前 `focus` 恒假、`terminal_rect` 只是保留字段,`ImeSink` 未包终端块,这些都要接上。
- plan 里列的 10 条测试名逐条实现,不要改名。

## 重点风险(必须真机确认并写进报告)

1. **输入法组字期间的双重输入:** macOS 输入法工作时 iced 是否仍发 `KeyPressed`,造成拼音字母同时进终端。若出现,在 `on_key` 里用 `ImeState.active` 抑制,并在报告里写明现象与做法。
2. **焦点串扰:** 在 Project 面板输入框里打字时,终端不得收到按键;反之亦然。
3. **多项目:** 输入只进"当前聚焦项目的活动会话"。

## 约束

- 库不改。缺口停下报告,不在 Digger 里绕开。
- 键位字节表"抄值不自己推",来源写进测试注释。
- 不碰 Digger 主工作区的未提交改动、`feat/preview-pairing`。
- 隔离环境:`DIGGER_DATA_DIR`(路径别带 "digger" 以外也无所谓,1b 修好后与路径无关)与临时 `HOME`,不碰真实数据和常驻 `dozerd`。
- 终端输入字节、口令不得写进日志。
- 不合并 `main`、不打 tag、不推送、不发布。

## 交付

- 新增文件提交后 `git status --ignored` 核对。
- 完工前 `git archive HEAD` 解到独立临时目录(目录名不要带 "digger"),在其自己的 `target` 下跑 `cargo test --workspace --no-fail-fast`、`cargo clippy --workspace --all-targets`、`cargo metadata --locked --offline`。
- 报告写到 `docs/superpowers/reviews/2026-10-<日期>-s6d2-task5-report.md`:测试总数(1b 之后的基线 + 新增)、测试名守恒、每条"通过"附命令与退出码(最终代码上刚跑的)。
- plan Task 5 Step 3 的真机项(`echo hi`、方向键历史、Ctrl+C 打断 `sleep 100`、`vim` 进出、粘贴多行、⌘C 复制、拖入文件路径、中文输入一句)你启动并截图,需要人手操作的逐条写进"待用户",不要用猜测代替。
- 停在 `feat/agent-sessions`。
