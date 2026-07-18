# P1c 终端 GUI 人工验收记录

**状态:验收通过,已定稿(2026-07-18)。**
**验收人:用户(甲方);验收权归用户,实施方(agent)不代签。全部清单项经用户实机验证确认。**

## 清单逐项结果

| # | 验收项 | 结果 | 备注 |
|---|--------|------|------|
| 1 | `cargo run -p dozer-app`:窗口标题 Dozer、四栏可辨、ByteBoy2077 配色 | ✓ 2026-07-18 | |
| 2 | dozerd 未运行时启动 app → 自动拉起,无错误 | ✓ 2026-07-18 | |
| 3 | 新建 tab:`ls -G` 颜色、`echo 你好`(中文 IME)回显正确 | ✓ 2026-07-18 | 经三轮修复后复验通过(见下方修复记录) |
| 4 | `top`/`vim` 全屏程序渲染不花屏 | ✓ 2026-07-18 | top 通过;vim 启动报错经查为用户环境问题(`~/.vim/bundle` 不存在而 vimrc 引用 Vundle,任何终端下同样报错),非 Dozer 缺陷 |
| 5 | **灵魂项**:关 app → 重开 → tab 自动恢复、滚屏完整、可继续输入 | ✓ 2026-07-18 | |
| 6 | 新 tab 跑 `claude`:交互正常 | ✓ 2026-07-18 | Dozer 第一次真实驱动 agent;Ctrl+C/atuin 问题经第四轮修复 |
| 7 | 窗口拖拽缩放:reflow 不崩、cols/rows 跟随 | ✓ 2026-07-18 | |
| 8 | 体感:`cat` 大文件滚动输出不卡顿 | ✓ 2026-07-18 | canvas 逐帧重画,实测流畅 |
| 追加 | 滚轮回看历史、滚动指示条、敲键回底 | ✓ 2026-07-18 | 原清单未列;T7 反馈补做(scrollback 从未接到 UI) |
| 追加 | 拖选 + ⌘C 复制/⌘V 粘贴、拖文件转路径 | ✓ 2026-07-18 | 原清单未列;T7 第四轮反馈补做 |

## T7 反馈 → 修复记录(按轮次)

**第一轮反馈**:空格键无输出、shell 行编辑方向键异常、会话内 TERM 缺失。
→ `d80eb28` 修复(输入):空格键映射 + DECCKM 方向键模式 + 会话 TERM 环境。

**第二轮反馈**:launchd 拉起 dozerd 时中文输出乱码。
→ `5876d79` 修复(dozerd):会话 spawn 兜底 UTF-8 LANG。

**第三轮反馈**:中文输入/`ls` 中文目录名乱、无滚动条、无法回看历史(用户曾疑 atuin,排除)。
→ `290362f` 修复(渲染):headless 探针实测定位两层根因——
  1. macOS "GB18030 Bitmap" 位图字体毒化 cosmic-text CJK 回退(字形 advance=inf),启动时从进程 fontdb 剔除;
  2. CJK 回退字形 advance=1.661 格 ≠ 网格假设 2 格,rich_text 流式排版结构上保不住列对齐——term_view 重写为 canvas 逐格定位绘制。
→ `a03a37b` feat(终端):scrollback 回看——滚轮 + 指示条 + 键入回底(历史一直在 alacritty 网格里,此前未接 UI)。

**第四轮反馈**:无法复制、atuin ↑ 报"读不到光标位置"、Ctrl+C 杀不掉 claude、拖文件不转路径、vim 启动报错(top 通过)。
→ `c7b1703` 修复(终端):设备查询应答回路——TerminalModel 此前用 VoidListener 丢弃 alacritty 生成的 DSR/DA 应答,atuin/ink 类 TUI 探测超时;另 keymap 补 Ctrl+标点控制码与控制字符透传。
→ `a01ccef` feat(终端):alacritty 内建 Selection 拖选 + ⌘C 复制/⌘V 粘贴(bracketed paste 感知)+ 拖文件转 shell 转义路径;顺带修复 ⌘ 组合键裸字符漏进 PTY。
→ vim 报错定性:用户环境 `~/.vim/bundle` 不存在而 vimrc 引用 Vundle,任何终端同样报错,非 Dozer 缺陷。

## 结论

P1c 验收通过。规格 §3 需求 1(原生 agent 终端,终端基座部分)、需求 4(会话存活,GUI 级恢复)已回填达成标注。
已知边界(非缺陷,后续任务承接):OSC 7/133 shell 集成与 agent 状态感知在 P1e;预览 pane 在 P1d。
P1d 的起点状态:`feat/p1c-terminal-gui` 并入 main 后的工作区。
