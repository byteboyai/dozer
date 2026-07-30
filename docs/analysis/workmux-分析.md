# Workmux 代码级分析

> 分析对象:https://github.com/raine/workmux (本地副本 `/Users/chrischiang/AI/workmux`)
> 分析日期:2026-07-28 · 版本 v0.1.229 · Rust 7.6 万行 / MIT · 1947 star

## 一句话定位

Workmux 是 **驾驭现有终端复用器(tmux/Zellij/WezTerm/kitty)的 CLI + TUI 编排层**,自己完全不做 PTY 持有和终端渲染——`git worktree` 建隔离目录,tmux 起窗口,agent CLI(Claude/Codex/Gemini/…)在窗口里跑,workmux 只负责"建窗口、装 hook、读状态、画 dashboard"。这是三个分析对象里(kooky/orca/workmux)**唯一不自研终端引擎**的一个:kooky 嵌 libghostty,orca 自研 PTY daemon,workmux 直接把 tmux 当黑盒来遥控。

后果是双向的:终端渲染、resize、粘贴、焦点这类"坑"完全外包给了 tmux,7.6 万行代码几乎全花在编排逻辑上(比 kooky 21K 行大 3.6 倍,但同样不碰 VT 解析);代价是**能力天花板锁死在 tmux 暴露的接口上**——`capture-pane` 只能拿到已渲染的文本快照,没有结构化事件流,所以 agent 状态感知必须完全依赖 hook + 标题这类带外信道,不能像 kooky 那样在字节流里做语义解析。

## 与 kooky/orca 的根本区别:借来的终端 vs 自研的终端

| | kooky | orca | workmux |
|---|---|---|---|
| 终端引擎 | 嵌入 libghostty(GPU 渲染) | 自研 PTY daemon(node-pty + xterm/headless) | **不自研,遥控 tmux/Zellij/WezTerm/kitty** |
| 组织单位 | tab | git worktree | git worktree(同构) |
| 会话存活 | 不存活,`--resume` | daemon 持有 PTY,app 重启无感 | **tmux 本身就是 daemon**,workmux 进程退出无所谓 |
| 读取代理输出 | VT 解析出的语义事件 | 无头终端模拟(结构化) | `capture-pane` 拿渲染后纯文本(降级到"读屏") |

"会话存活"这个 kooky/orca 都要专门解决的架构难题,workmux 是**白拿**的——tmux server 本身天然常驻、天然扛得住前台进程崩溃。这是"借用宿主"路线最大的免费红利,也是它 7.6 万行里完全没有 PTY/daemon-crash-recovery 类代码的原因。

## 工程结构(按子系统 LOC)

| 子系统 | 行数 | 职责 |
|---|---|---|
| `command/sidebar/` | 11559 | 常驻 daemon 轮询 tmux + 广播快照给"侧边栏"TUI 客户端(ratatui) |
| `command/dashboard/` | 10169 | 独立的全屏 TUI:agent 列表、diff 预览、worktree 管理 |
| `sandbox/` | 8919 | 容器(Docker/Podman/Apple Container)+ Lima VM 隔离,RPC 桥,网络代理 |
| `multiplexer/` | 7864 | tmux/WezTerm/Zellij/kitty 四后端的 trait 抽象 |
| `workflow/` | 6675 | create/merge/remove/rename/resurrect 等高层业务流程编排 |
| `config.rs` | 5570 | 单文件 YAML 配置 schema(agent 定义、pane 布局、hook、sandbox 规则) |
| `agent_setup/` | 3167 | 8 个 agent(Claude/Codex/Gemini/Copilot/Antigravity/OpenCode/pi/omp)的 hook 安装器 |
| `git/` | 2370 | worktree 生命周期、状态查询、merge/rebase |

`command/` 下还有 `add`/`merge`/`rebase`/`sync_files`/`reap_agents` 等三十多个子命令文件,是典型"每个 CLI 子命令一个文件"的 clap 项目结构,没有 kooky/orca 那种"厚 lib + 薄壳"的分层——**workmux 本身就是唯一的可执行文件**,没有拆出独立小 hook 二进制(hook 命令是 `workmux set-window-status`,复用同一个二进制,靠 fork 的进程启动开销换来了架构简单)。

## 核心机制一:Multiplexer trait 抽象(`multiplexer/mod.rs`)

单个 ~30 方法的 `Multiplexer` trait,四个实现(tmux/WezTerm/Zellij/kitty)。设计上大量方法给了默认实现(返回 `Err`/no-op/空集合),新增后端只需实现窗口分屏、发送按键这几个原语——`setup_panes()` 的完整编排逻辑(命令解析、agent 占位符替换、handshake 同步、sandbox 包裹、resume 参数注入)写在 trait 默认方法里,四个后端共享,不需要各自重复。

后端探测顺序体现了一个值得记住的细节:**内层复用器优先于外层**——`$TMUX` 先于 `$WEZTERM_PANE` 先于 `$ZELLIJ_*` 先于 `$KITTY_WINDOW_ID`,因为 tmux 常被嵌套在其他终端里运行,env var 是运行时最新写入的那个赢,顺序错了会认错后端。9 个单测直接覆盖了全排列组合,而不是靠人肉走查——这种"纯函数化 + 穷举分支测试"的模式在全项目重复出现(`resolve_backend` 与实际 `detect_backend` 分离,前者可测,后者才碰真实环境变量)。

`create_handshake()` 抽象出了"pane 起了但 shell 还没就绪"这个所有后端共同的竞态:先 spawn 一个内含 handshake 脚本的 shell,脚本执行到位后通过 unix pipe 通知,workmux 收到通知才发送真正的启动命令——比 kooky 的方案更规整(kooky 靠 sleep/轮询应付类似问题未见文档提及,workmux 是显式同步原语)。

## 核心机制二:Agent 状态感知(hook 安装 + 落盘 + 实时核对)

三段式,和 kooky 的"unix socket + hook 小工具"同构但工程细节更硬核:

### 1. Hook 安装:JSON 树合并,不是覆盖写

`agent_setup/hooks.rs` 是这块的核心。8 家 agent CLI 的 hook 配置全是"JSON 里一个 `hooks` 键,按事件名分组,组内是 command 数组"这同一种形状(Claude/Codex/Gemini 三家共享这个格式,Copilot/Antigravity/OpenCode/pi/omp 各自适配)。安装逻辑不是"写文件",是**语义化合并**:

- `merge_hook_groups`:按 `serde_json::Value` 相等性去重后 push 进已有事件数组,不存在的事件直接插入整个数组——用户自己配的其他 hook(如 `afplay` 提示音)原样保留
- `remove_workmux_hooks`:按 command 字符串包含 `workmux set-window-status` 精确摘除,同一 group 里混有用户 hook 时只删 workmux 那一条,不删整个 group
- `remove_empty_hooks_wrapper`:摘完后空对象/空数组要连壳一起清掉,否则配置文件里留一堆 `{}`

**幂等性作为一等公民**:每个函数都有"再调一次应返回 false(无变化)"的测试。这比 kooky 文档里描述的"写一份 hooks 配置"更接近真实生产系统该有的样子——用户的 settings.json 是共享可写资源,agent 自己的其他插件/hook 随时可能并存,合并/摘除必须无损。

### 2. 状态落盘:一 pane 一文件,而不是一个大数据库

`state/store.rs` + `state/types.rs`:`$XDG_STATE_HOME/workmux/agents/{backend}__{instance}__{pane_id}.json`,一个 agent 一个文件,`PaneKey`(backend + instance + pane_id)做复合主键防止多 tmux server/多后端撞车。文件名里的 `/`、`:`、`%` 会被 percent-encode(tmux socket 路径本身带 `/`)。写入统一走 `write_atomic`(临时文件 + rename),读取遇到损坏 JSON 直接删除重来而不是报错阻塞——这两条和 kooky"运行时字段一律 Optional、从磁盘读的值不可信"的裁决完全一致,是两个独立项目收敛到的同一条经验。

### 3. 状态核对:拉取式 reconciliation,不是定时器扫描

kooky 用"60 秒没等到 Post 的调用标记为 stalled"的**后台定时器**做 orphan 检测;workmux 走的是**拉取式**路径——`load_reconciled_agents()` 在 dashboard/sidebar 每次刷新时,一次性批量查询 tmux 所有 pane 的实时信息(`get_all_live_pane_info`,单条 tmux 命令),逐个比对存盘状态:

- pane 完全查不到 → 判定关闭,删状态文件(除非检测到 tmux server 重启过,此时保留以支持 `resurrect` 复活)
- `pid` 对不上存盘的 `pane_pid` → pane ID 被复用(旧 pane 关了,tmux 把 ID 分给了新进程),判定失效
- `current_command` 变了(如 `node` 变成 `zsh`)→ agent 进程退出,判定失效

三层判定共用一个"是否跨过 server 生命周期"(`boot_id` 比对)的前置分支——server 重启导致的 PID/command 突变要保留(等用户 resurrect),真实退出导致的突变要清理,这是全模块里最容易踩坑但被显式测试覆盖到的分支。**没有后台线程,没有轮询定时器,核对只在真正需要展示状态时才做一次批量查询**——比定时器扫描更省资源,代价是"刚发生的退出"要等下次 UI 刷新才能被发现,workmux 用 sidebar daemon 的轮询(见下)填上这个延迟。

## 核心机制三:Sandbox(三个项目里独一份,工程含量最高)

kooky/orca 都没有的子系统:agent 可以跑在**容器(Docker/Podman/Apple Container)或 Lima VM**里,与宿主机的 SSH key、AWS 凭证、GPG key 完全隔离,这样"YOLO 模式"(免权限确认自动执行)才敢开给 agent。

- **双后端**:容器是进程级/VM 级隔离 + 每会话新建即弃,自带 6 个 agent 的预置 Dockerfile(`docker/Dockerfile.{base,claude,codex,gemini,opencode,pi,omp}`,`include_str!` 编译进二进制);Lima 是持久化 VM,内建 Nix/Devbox 工具链支持
- **RPC 桥**(`sandbox/rpc.rs`,1778 行):guest 里的 workmux 二进制通过 TCP 连回 host 端 RPC server,JSON-lines 协议,支持 `SetStatus`/`SetTitle`/`SpawnAgent`/`Merge`/`Exec` 等请求——**沙盒内的 agent 调用 `workmux merge` 这类命令时,实际执行发生在 host 侧**,guest 只是转发请求,这样容器里不需要装 git/gh 等宿主工具链
- **网络代理**(`sandbox/network_proxy.rs`):HTTP CONNECT 代理 + 域名白名单,host 侧做 DNS 解析并拒绝解析到内网 IP 的域名(防止 allowlist 绕过内网访问),配合容器内 iptables 默认拒绝出站、只放行代理端口——**双保险**:代理挡应用层,iptables 挡"agent 直接无视代理环境变量"这种绕过
- **鉴权**:RPC token 和代理 token 都走 `constant_time_eq`(逐字节异或,不提前 return)做比较,防时序攻击——一个 10 行的工具函数,但说明这条 host↔guest 通道被当作真实的信任边界在设计,不是"能跑就行"

这套东西直接对应 Dozer"用户侧验收层"的核心诉求之一:**agent 可以自动执行,但不能拿到不该拿的东西**。workmux 证明了这条边界可以不侵入 agent CLI 本身(不需要 fork Claude Code)、靠外部容器/VM + 代理转发就能建立起来。

## 数据模型速览

```
$XDG_STATE_HOME/workmux/
├── settings.json              # 全局 dashboard 偏好(排序/过滤/侧栏宽度…)
├── agents/
│   └── {backend}__{instance}__{pane_id}.json   # 一 pane 一状态文件
├── containers/{worktree_handle}/{container_name}   # 容器归属标记(空文件,内容是 runtime 名)
└── runtime/{backend}__{instance}.json         # sidebar daemon 产出的临时信号(如"疑似卡死"pane 集合)
```

`containers/` 目录值得一提:标记文件本身没有语义内容,只是"这个 worktree 名下曾起过这个容器"的存在性记录,配合 `list_containers` 在 `workmux remove` 时找到该清理的容器——**用文件系统当轻量级关系表**,和 `agents/` 目录同一套哲学,不引入 sqlite。

## 测试文化:1358 Rust 单测 + 174 Python 端到端

Rust 侧 1358 个 `#[test]`,集中在纯函数(JSON 合并、backend 探测、PaneKey 编解码这类)——可以摆脱真实 tmux 环境快速跑。但 tmux/git worktree 的交互终究没法在 Rust 单测里高保真模拟,workmux 的解法是**独立的 Python 测试套件**(`tests/`,174 个 `test_*` 函数,`pytest` + 真实 tmux/git 子进程),覆盖 `add`/`merge`/`rebase`/`sandbox`/多 agent hook 安装等端到端场景。两层分工清晰:Rust 测纯逻辑,Python 测"真的起一个 tmux server 会不会翻车"。这是一个可复用的测试策略经验:**语言实现和验收测试的语言不必一致**,选对每层最省心的工具。

## 对 Dozer 的启示

Dozer 与 kooky 同构(GUI + 自持 PTY 池 + hook socket),与 workmux 在"是否自研终端"这条轴上正好站在对面——所以 workmux 的 multiplexer 抽象层本身不直接可搬(Dozer 不遥控外部 tmux,`dozerd` 自己就是那个"tmux"),但其余三块高度可复用:

| workmux 子系统 | 对应 Dozer 位置 | 借鉴点 |
|---|---|---|
| `agent_setup/hooks.rs` 的 JSON 树合并/摘除 | `dozer-hook` 的安装逻辑 | 幂等合并 + 精确摘除 + 空壳清理,而不是覆盖写整个 settings 文件;用户自己的其他 hook 必须原样保留 |
| `state/store.rs` 一 pane 一文件 + 批量 reconciliation | `dozerd` 的会话存活/验收闭环存储 | 拉取式核对(dashboard 刷新时批量查一次)比后台定时器更省资源;`boot_id` 判定"服务重启 vs 真实退出"这条分支值得直接照搬到 dozerd 的 PTY 池崩溃恢复逻辑 |
| `sandbox/`(容器 + Lima + RPC 桥 + CONNECT 代理) | 若 Dozer 的"验收层"未来要做权限边界/网络限制 | 证明了隔离可以不侵入 agent CLI,靠外部容器/VM + host↔guest RPC 转发达成;`constant_time_eq` 这类细节说明这条通道要按信任边界设计,不是内部 IPC 随便传 |
| Rust 单测 + Python 端到端双层测试 | Dozer 自己的测试策略 | `cargo test` 测 dozer-core 纯逻辑,真实拉起 `dozerd`/PTY 的场景交给独立脚本层(Python 或 shell),不要硬塞进 `cargo test` |
| `config.rs` 单文件 5570 行 | Dozer 的配置 schema 设计 | 反面教材:功能全塞进一个大文件会让"新增一个 agent = 改一处"退化成"新增一个 agent = 改十几处分散在同一文件里"。Dozer 若走 TOML `[agents]` 配置,应从一开始按 agent/子系统拆文件,不要等到 5000+ 行才重构 |

最值得单独展开的一点:**workmux 没有"会话存活"问题是因为它把这个问题甩给了 tmux**。Dozer 选择自己持有 PTY(`dozerd` 的 PTY 池),就是主动放弃了这个免费红利,换来的是不依赖用户装 tmux、跨前端(GUI/未来 TUI/hook)统一会话模型的自由度——这个取舍在 [[dozer-vision]] 里已经定过,workmux 的存在只是从反面印证了"自持 PTY"确实是条更重的路,重量换来的是控制权。