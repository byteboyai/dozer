# P1e agent 集成人工验收记录

**状态:验收通过,已定稿(2026-07-18)。**
**验收人:用户(甲方);验收权归用户,实施方(agent)不代签。全部清单项经用户实机验证确认。**
**前置回归:cargo test 121 通过、clippy 零警告、fmt 干净;app 冒烟 8 秒无 panic。**

## 清单逐项结果

| # | 验收项 | 结果 | 备注 |
|---|--------|------|------|
| 1 | 新 tab `cd` 几层 → tab 标题跟随目录名;`false` → 红字 `exit 1`,下条命令后消失 | ✓ 2026-07-18 | 首测未过,定性为环境:旧 daemon 在服役(见修复记录第一轮) |
| 2 | `dozer-hook install` → settings.json 现 7 事件条目且原配置无损;重复执行不重复 | ✓ 2026-07-18 | 需用 target 完整路径执行(未装进 PATH) |
| 3 | tab 里跑 `claude` 派活 → 胶囊绿"运行中"→ 紫"待输入"→ 金"回合毕" | ✓ 2026-07-18 | |
| 4 | 关 app 重开 → 胶囊状态恢复 | ✓ 2026-07-18 | 顺带催生关 tab 语义修复(第三轮) |
| 5 | 停掉 dozerd 后在 Dozer 外跑 claude → 无报错无卡顿(hook 静默) | ✓ 2026-07-18 | |
| 6 | `DOZER_SHELL_INTEGRATION=0` 起 dozerd → tab 标题不跟随 cwd;去掉恢复 | ✓ 2026-07-18 | |
| 7 | `dozer-hook uninstall` → dozer 条目干净移除,他人配置无损 | ✓ 2026-07-18 | |

## 反馈 → 修复记录(按轮次)

**第一轮反馈**:tab 标题不跟随 cd。
→ 定性为验收环境问题,非代码缺陷:在服役的 dozerd(PID 27439)是当日 11:09 启动的 P1d 时代旧进程,占着 socket 使 app 不拉新二进制,旧 daemon 起的 zsh 无 ZDOTDIR 注入。端到端复核新二进制(新 daemon + 交互式 zsh + cd)OSC 7 三连发正常。处置:`pkill dozerd` 后重启 app。
→ **记录已知边界**:daemon 与 GUI 版本偏斜时功能静默缺席、无提示——版本握手留 P1f 考虑。

**第二轮反馈**:starship 报 `/usr/bin/swift` 超时告警。
→ 定性为 starship 自身行为(进入 Swift 项目目录探测版本超 500ms 默认超时),与 Dozer 无关。顺带实测排除嫌疑:即使 starship 等 precmd 钩子先注册,zsh 会为每个 precmd 保留原始退出码,`133;D;1` 采集不被污染。

**第三轮反馈**:关 app 重开不应恢复"重开前已关闭"的 tab,只应恢复关 app 时还开着的。
→ `bf8bdac` 修复(终端):关 tab 从"纯 detach"改为"结束会话"(kill daemon 侧会话)。语义裁决:"会话存活"保关 app/崩溃(退 app 仍是纯 detach),显式点 × 是明确的结束动作,与主流终端一致;误关由 `claude --resume` 兜底。可逆:若 P1f 会话资产化要求"关而不杀",改 daemon 侧 closed 标记。

## 结论

P1e 验收通过。规格 §3 需求 1(原生 agent 终端,shell 集成 + agent 状态感知部分)已回填达成标注。
已知边界(非缺陷,后续任务承接):daemon/GUI 版本握手(P1f 考虑);状态胶囊的 Context % 随 transcript 适配器来(P1f/g);bash/fish 发射端未注入(zsh-only,留门);W1 一键注册 hooks GUI 属周边页面阶段(复用 `dozer-hook install` 逻辑)。
P1f(验收闭环)的起点状态:`feat/p1e-agent-integration` 并入 main 后的工作区;hook 事件通路(`Request::HookEvent.data` 原样透传)已为交付声明留好挂点。
