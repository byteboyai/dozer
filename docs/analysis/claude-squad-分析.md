# Claude Squad 代码级分析

> 分析对象:https://github.com/smtg-ai/claude-squad(本地副本 `/Users/chrischiang/AI/claude-squad`)
> 分析日期:2026-07-28 · 最后一次 push 2026-06-17(HEAD `5a604f7`,距分析日约 6 周,活跃度明显低于同系列其他项目)
> Go 9,198 行 / 47 文件 / 9 个测试文件 / 33 个单测 · AGPL-3.0 · 8,195 star(fork 593)

## 一句话定位

Claude Squad(二进制名 `cs`)是**驾驭已有 agent CLI**(README 宣称 Claude Code/Codex/Gemini/Aider/OpenCode/Amp,但代码实际只对 Claude/Aider/Gemini 三家做了专属文本识别,其余均是"随便一个 shell 命令")的 **Go + Bubbletea TUI + tmux + git worktree** 编排器——和已有的 **workmux 是同一条路线的另一次独立实现**:都不自研终端引擎,都靠 `git worktree` 做"一 agent 一工作区"隔离,都用 tmux 的 detach/attach 机制白拿"会话存活不依赖自己进程"这个免费红利。

**结论先给**:这不是一次"另一种解法"的验证,而是**同一标准件的第五次独立验证,但工程严谨度和范围都明显更薄**——没有 hook 安装(纯文本模式匹配代替结构化状态感知)、没有会话核对/清理机制、没有 sandbox、单文件状态存储、33 个测试(workmux 是 1,532 个)。值得写进文档的不是"claude-squad 引入了什么新机制",而是"claude-squad 用一种更轻/更脆弱的方式解决了 workmux 已经用更硬核方式解决过的同一批问题",以及一两个 workmux 分析里没有明确提到的小而具体的 UI 差异点。

## 工程结构

```
main.go                入口:解析 --daemon/--reset/--autoyes 等 flag
app/        1,722 行   Bubbletea 主循环(home model)+ 帮助屏
cmd/           32 行   os/exec 包装(可测试的 Executor 接口)
config/       654 行   ~/.claude-squad/config.json(默认程序/AutoYes/轮询间隔/profile)+ state.json(单文件持久化)
daemon/       196 行   --daemon 子进程:轮询所有已存实例做 AutoYes,PID 文件管理生命周期
keys/         134 行   全局按键表(vi 风格 hjkl + 专属键)
log/           76 行   日志
session/      800 行   Instance(状态机)+ Storage(序列化)
session/git   965 行   GitWorktree:创建/清理/commit/push(gh CLI)/dirty 检查
session/tmux  764 行   TmuxSession:唯一的多路复用后端,自带一个 PTY
ui/         2,583 行   list(侧栏)/preview(只读预览)/terminal(内嵌 shell 面板)/diff(diff 视图)/tabbed_window
ui/overlay  1,079 行   confirmationOverlay/branchPicker/profilePicker/textInput
web/        —          纯 Next.js 官网(marketing),与运行时无关
```

对比 workmux 的 178 文件、七万六千行、四个多路复用后端(tmux/Zellij/WezTerm/kitty)+ 独立 sandbox 子系统,claude-squad 是一个**单文件可执行、单一 tmux 后端、无隔离层**的精简实现——体量差 8 倍,不是因为它做得更聚焦,而是因为它砍掉了 workmux 里"hook 结构化合并""三层会话核对""容器/VM 沙盒"这三块工程量最大的子系统,全部换成了更简单(也更脆弱)的替代方案,下面逐条对照。

## 核心机制一:会话托管——遥控 tmux,但自己也认领了一段 PTY

和 workmux 的判定一致:**detach 的 tmux session 才是真正的执行边界**——`TmuxSession.Start()` 执行 `tmux new-session -d -s <name> -c <workdir> <program>`,agent 进程活在 tmux server 里,`cs` 进程退出、崩溃都不影响 agent 存活,这条和 workmux 完全同构,不重复展开。

真正值得记录的分歧点是**"谁来渲染/转发终端字节流"**:

- workmux 完全不持有 PTY,用户是靠自己已经在跑的 tmux/Zellij/WezTerm 客户端去 attach,workmux 进程只管建窗口、装 hook、读状态。
- claude-squad **在自己的进程内持有一个 PTY**(`session/tmux/pty.go`,`github.com/creack/pty`),但这个 PTY 包的命令不是 agent 本身,而是 `tmux attach-session -t <name>`——即 claude-squad 把"attach 一个已存在的 tmux 会话"这个动作,内嵌进了自己的单一 Bubbletea 进程里,而不是让用户切换到另一个真实终端窗口。`TmuxSession.Attach()` 起两个 goroutine:一个 `io.Copy(os.Stdout, ptmx)` 把 tmux 客户端的渲染输出原样转发到当前终端,另一个读 `os.Stdin` 转发按键给 tmux(并用一个 50ms 窗口"吞掉"attach 瞬间终端自身吐出的控制序列,`Ctrl+Q` 硬编码为退出键)。

后果:claude-squad 的"全屏进入某个 agent 会话"体验是无缝的(不用 `Ctrl+B d` 再切窗口),但代价是这段 PTY 转发逻辑(50ms 窗口猜测控制序列、`panic` 式的 Detach 失败处理"没法恢复,不如让用户重开程序")比 workmux 的"完全甩给用户自己的终端"更脆弱——`Detach()` 里明确写着"如果关闭失败,恐慌好过弄坏用户的终端 pane",说明作者自己也认为这段状态机没有把所有分支想清楚。

**对 Dozer 的意义**:这不是一条可以直接抄的机制,而是一个反例参照——claude-squad 证明了"在单进程里内嵌一段 attach-passthrough PTY 来模拟无缝全屏体验"这条路是可行的,但引入了一类 workmux 完全不用面对的新故障域(控制序列吞吐时序、attach/detach 状态机的边界条件、异常退出要不要 panic)。Dozer 的 `dozerd` 本身就是终端引擎的所有者,不需要"内嵌一个转发层去 attach 外部 tmux",这条分歧点对 Dozer 没有直接借鉴价值,只是确认了"自己造终端 vs 转发别人的终端"这两条路线之间还存在"转发但内嵌"这样一种中间态,而这种中间态工程上并不比两端更省心。

## 核心机制二:Agent 状态感知——纯文本模式匹配,不装 hook

这是和 workmux 差异最大、也最值得展开的一点。workmux 的状态感知靠**给 8 家 agent CLI 装结构化 hook**(JSON 树语义合并/摘除,幂等,详见 workmux-分析.md),状态从 agent 自己上报。claude-squad **完全没有 hook 安装机制**,`session/tmux/tmux.go` 里状态感知的全部实现是:

```go
if t.program == ProgramClaude {
    hasPrompt = strings.Contains(content, "No, and tell Claude what to do differently")
} else if strings.HasPrefix(t.program, ProgramAider) {
    hasPrompt = strings.Contains(content, "(Y)es/(N)o/(D)on't ask again")
} else if strings.HasPrefix(t.program, ProgramGemini) {
    hasPrompt = strings.Contains(content, "Yes, allow once")
}
```

`HasUpdated()` 每次调用 `tmux capture-pane -p -e -J` 抓取当前渲染文本,对内容做 SHA-256 哈希对比判断"是否变化",再对固定字符串做子串匹配判断"是否出现了确认提示",匹配上就调用 `TapEnter()`(往 PTY 写 `0x0D`)模拟自动确认——这就是 claude-squad 的"yolo/autoyes 模式"全部实现。**没有 hook,没有结构化事件,纯粹是对 agent CLI 渲染出的 UI 文案做字符串包含判断**,只覆盖 Claude/Aider/Gemini 三家,README 宣传的 Codex/OpenCode/Amp 支持在代码层面只是"任意一个可执行命令字符串",没有专属的确认提示识别或 hook 安装,只能靠用户自己在这些 CLI 里配置免确认参数。

这条比 workmux 脆弱在几个具体位置:agent CLI 的 UI 文案换一个版本(比如 Claude Code 把提示语从"No, and tell Claude..."改了措辞)就会让 autoyes 静默失效而不报错;不装 hook 意味着完全没有"回合开始/结束""工具调用"这类语义边界,只有"pane 内容变了"这个粗粒度信号;而且 `TapEnter()` 是无差别地对匹配到的确认提示按回车,如果某次提示实际上是危险操作确认(比如误伤性的 `rm -rf` 二次确认),claude-squad 的 autoyes 模式会无差别地帮用户按下"是"——workmux/Dozer 目前都没有做这类"危险命令强制打断自动模式"的机制,但至少没有像 claude-squad 这样把"看到确认提示就自动点头"当作产品默认可选项来卖(README 的"yolo / auto-accept mode"卖点)。

**对 Dozer 的意义**:这是一次有价值的负面确认,而不是借鉴点——claude-squad 独立验证了"不装 hook、纯靠屏幕文案匹配"这条路径**可以工作但天然脆弱**(耦合 agent CLI 的 UI 文案、无语义边界、无法区分提示类型)。Dozer 已经选择了 hook + 结构化事件这条更重但更稳的路线(`dozer-hook` + `dozerd::agent_state_for`),claude-squad 的实现从反面印证了这个选择的必要性:如果 Dozer 未来要支持一个没有 hook 能力的 agent CLI,退回到"capture 输出 + 字符串匹配"是可行的兜底方案,但应该被当作**明确降级、而非常规路径**来对待,并且"自动确认"这类功能不应该在没有语义分级的情况下无差别启用。

## 核心机制三:Worktree 与 Pause/Resume 生命周期

和 workmux 结论一致的部分一句话带过:一 agent 一 worktree(`git worktree add -b <branch> <path> <base-commit>`),`Cleanup()` 走 `worktree remove -f` + `branch -D` + `worktree prune`,`PushChanges` 通过 `gh repo sync` 把变更同步到远程分支——这套"worktree 生命周期"的具体 git 命令序列和 workmux 的 `workflow/` 没有本质区别。

值得单独记录的是 claude-squad 特有的 **Pause/Resume** 状态:`Instance.Pause()` 是一个**用户显式触发**的动作(不是自动 reap),行为是:检查 worktree 是否 dirty → dirty 就本地 commit(不 push)→ `DetachSafely()` 断开 attach 的 PTY → `git worktree remove` 删掉工作区目录但保留分支 → 把分支名复制到剪贴板 → 状态置为 `Paused`。`Resume()` 反向操作:检查目标分支当前有没有被 checkout 到别处(避免冲突)→ 重新 `worktree add` 用同一分支 → 如果 tmux session 还在就 `Restore()`(仅重新接上 PTY),不在就整个重新 `Start()`。

这个"Pause 保留分支、丢弃 worktree 目录和磁盘占用,Resume 时按需重建"的模式,workmux 的 `resurrect` 命令覆盖的是相近但更窄的场景(tmux server 重启后恢复 pane 关联),claude-squad 这里把它做成了用户主动触发的、明确对外暴露的一等公民操作,并且专门处理了"worktree 目录/`.git` 文件缺失"(orphaned)这种边缘状态——`IsValidWorktree()` 检测到 orphaned 就跳过 dirty 检查和 `git worktree remove`(这两个操作在 orphaned 状态下都会报错),直接清理残留目录和 git 元数据。这条边界处理是 claude-squad 代码里少数几处体现出"认真考虑过失败模式"的地方。

**对 Dozer 的意义**:Pause/Resume 这个显式的"归还磁盘空间但保留身份(分支名)"生命周期状态,是一个 workmux 分析里没有明确对应物的具体产品设计点——如果 Dozer 未来要支持"agent 会话长期挂起、不占用磁盘/内存,但随时可以按原状态恢复"这类场景(比如用户开了几十个任务但只有几个在跑),claude-squad 这个"保留分支、丢弃 worktree、恢复时重建"的三段式操作序列是一个可以直接参考的小颗粒度设计,比 workmux 的"resurrect"覆盖场景更完整(显式用户操作而非仅限崩溃恢复)。

## 简短带过:与 workmux 结论一致、不重复展开的部分

- **状态落盘容错哲学**:`config/state.go` 的 `LoadState()` 遇到 JSON 解析失败直接返回 `DefaultState()`(相当于清空实例列表)——"磁盘值不可信"这条哲学方向和 workmux/kooky 一致,但**没有 workmux 的"一 pane 一文件 + 原子写(temp+rename)"**,而是把所有实例序列化进单个 `instances.json` 字段整体重写(`os.WriteFile`,非原子),一次写入中途失败或磁盘满,理论上可能损坏整个实例列表而不只是一条记录。方向一致,工程细节明显更粗糙。
- **会话核对/清理**:**基本不存在**。没有 workmux 的 `boot_id`/PID/command 三层判定,只有零散的 `DoesSessionExist()` 检查(访问某个 pane 时才顺带查一次,查不到就地删缓存重建,`ui/terminal.go:100-144`),没有统一的"启动时批量核对所有已存实例,清理确实已消失的"流程——`FromInstanceData()` 在非 Paused 状态下直接调用 `Start(false)` 尝试 `Restore()` attach 到存盘记录的 tmux 名,如果该会话已经不存在,`Restore()` 返回 error,整条 `LoadInstances()` 直接失败退出,没有"跳过这一条、继续加载其余实例"的降级路径。这是明显弱于 workmux 拉取式 reconciliation 的地方。
- **daemon**:`--daemon` 子进程只做一件事——轮询所有存盘实例,`HasUpdated()` 命中确认提示就 `TapEnter()`,退出前把实例列表存盘。生命周期靠一个 PID 文件(`~/.claude-squad/daemon.pid`)管理,`StopDaemon()` 直接按 PID kill,没有 workmux runtime 信号那样的结构化产出,也没有校验"这个 PID 现在是否真的还是当初启动的那个 daemon 进程"(理论上存在 PID 复用杀错进程的窗口,虽然概率低)。
- **Sandbox/隔离**:全仓库检索 `sandbox`/`docker`/`container`/`lima`/`seccomp` 关键字**零命中**。和 workmux 的容器/VM/RPC 桥/CONNECT 代理形成鲜明对比——claude-squad 完全没有这层,agent 在用户自己的账号权限下直接跑,YOLO 模式没有任何隔离边界兜底。这与 workmux 分析结论一致的部分是"没有 sandbox 也能是个可用产品",但反过来看也印证了 workmux 这块投入的稀缺性:六个同类项目里目前仍然只有 workmux 一家做了这件事。

## 测试文化:33 个测试,六个项目里(连同 workmux)密度最低的编排层项目

9 个 `_test.go` 文件,`grep -c "^func Test"` 共 33 个,集中在 `worktree_ops_test.go`(孤儿 worktree 清理场景)、`tmux_test.go`(后端探测/命令构造)、`config_test.go`、`app_test.go`(状态机)、`ui/list_test.go`/`preview_test.go`/`terminal_test.go`。没有端到端测试套件(workmux 有独立的 174 个 Python pytest 覆盖真实 tmux/git 交互),纯 Go 单测里也没见到系统性的"穷举分支"风格(workmux 的 `resolve_backend` 全排列测试那种)。9,198 行代码配 33 个测试,测试密度(测试数/千行 ≈ 3.6)远低于 workmux 的 ≈ 17.9(1,358 Rust 测试 / 75,922 行,不算 Python 端到端）,也低于 kooky 的 ≈ 23.9。**这一点上 claude-squad 没有提供任何新信息,只是又一次印证"工程严谨度和 star 数/受欢迎程度不成正比"——8,195 star 排在六个分析对象之外单独看也是最高的,但测试投入是最低的。**

## 对 Dozer 的启示汇总

大部分机制是 workmux 已验证标准件的更薄实现,新增借鉴点确实有限——这个结论本身就是发现:claude-squad 是第五个独立收敛到"TUI + tmux + git worktree"这套编排范式的项目,进一步提高了这套范式作为"编排层标准配置"的置信度,但它自己在工程细节上没有给 Dozer 带来 workmux 尚未覆盖的正面新知识。

| claude-squad 机制 | 与 workmux 的关系 | 对 Dozer 的意义 |
|---|---|---|
| tmux detach session 做执行边界 | 结论一致,不重复展开 | 再次印证"借用宿主换会话存活"是可行范式,但 Dozer 已选自持 PTY 路线,不适用 |
| 自己持有 PTY 去 attach 外部 tmux(内嵌全屏 passthrough) | workmux 完全不持有 PTY,用户自己的终端 attach | 负面参照:证明"转发但内嵌"是可行的中间态,但引入了新的故障域(控制序列吞吐时序、panic 式异常处理),Dozer 自持终端引擎不需要走这条路 |
| 纯文本模式匹配识别确认提示,无 hook 安装 | workmux 是结构化 hook 语义合并 | 负面确认:印证 hook 路线的必要性——屏幕文案匹配脆弱(耦合 UI 文案版本、无语义边界、无法按危险等级区分),只应作为无 hook 能力时的明确降级方案,不应是默认路径 |
| Pause/Resume(保留分支、丢弃 worktree、按需重建)+ orphaned worktree 边界处理 | workmux 的 `resurrect` 场景更窄(仅崩溃恢复) | 少数真正差异化的小颗粒度设计:若 Dozer 未来要支持"长期挂起会话、按需恢复"场景,这个三段式操作序列和 orphaned 检测逻辑可直接参考 |
| 单文件 `instances.json`、非原子写、加载失败即整体放弃 | workmux 是一 pane 一文件 + 原子写 + 拉取式 reconciliation | 反面教材,不建议参考——再次确认 workmux 那套设计是更值得抄的版本,不需要额外从 claude-squad 学 |
| 无 sandbox | 与 workmux 结论一致(有 vs 没有的对比本身即结论) | 无新增借鉴点,workmux 仍是六个项目里唯一给出隔离参考实现的 |
| 33 测试 / 9,198 行,测试密度六个项目里最低 | 与"workmux 测试密度最高"互为印证 | 无新增借鉴点,只是进一步佐证"star 数不代表工程严谨度",维持 Dozer 已有的测试纪律判断 |

**未能本地核实的部分**:Codex/OpenCode/Amp 在 claude-squad 里的实际运行体验(代码层面只是任意程序字符串,是否有用户自己配置的外部脚本弥补确认提示识别,未追踪到);`gh repo sync` 依赖的具体失败模式(网络/权限报错的用户可见程度)未做实测,只读了源码路径。
