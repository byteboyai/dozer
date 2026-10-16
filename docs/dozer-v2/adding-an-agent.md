# 新增一个 agent 的清单

> 配套 `docs/dozer-v2/agent-pillar-audit.md`(Q15 审计)。审计量出:接入 Qoder CLI CN 的
> L1 改了 **16 个非测试/含测试文件、分布 6 个 crate**;V8agent 是 22 个。本清单把"改哪些
> 地方、哪些编译器会提醒、哪些必须靠测试兜住"写死,接新 agent 时逐条对照。
>
> 现状(2026-10-16 起):`AgentKind::ALL` / `known()` / `from_label()` 是**唯一真相**
> (`crates/dozer-model/src/agent.rs`)。静态事实能收进表的都收;仍要手写的地方下面标注。

## 0. 先决定接入层级

不是每个 agent 都做全栈。Qoder CN 只做到 **L1**(终端启动 + hook 状态检测 + MCP 注册 +
展示),没有 transcript 解析、没有 headless 总结。层级决定下面要做哪些节:

- **L0 仅展示/归属**:`AgentKind` 变体 + `label()` + `display_label()`。
- **L1 终端可用**:L0 + 启动命令 + hook 安装/事件映射 + MCP 挂载(如适用)。
- **L2 历史/用量**:L1 + transcript 目录与解析 + headless 总结 provider。

**先把层级写进本清单对应行的注释,再动手**;否则容易顺手漏掉 L2 的活。

## 1. 编译器会抓到的(漏了编不过)

给 `AgentKind` 加一个变体后,`cargo check --workspace --all-targets` 会报
`error[E0004] non-exhaustive patterns`。以下 `match` **故意不写通配 `_`**,新变体必须在
这里表态(2026-10-16 实测点位):

| 位置 | 函数 | 语义 |
|------|------|------|
| `dozer-model/src/agent.rs` | `label()` | 持久化/命令行名(写进 SQLite,见 §3) |
| `dozer-model/src/agent.rs` | `display_label()` | 用户可见品牌名 |
| `dozer-model/src/agent.rs` | `ALL` 数组 + 测试里的 `variant_index` | 表本身;漏加到 `ALL` 会编译失败 |
| `dozer-core/src/agent_paths.rs` | `projects_root_in()` | `<home>/<root>/projects` 根 |
| `dozer-core/src/agent_paths.rs` | `project_session_dir_in()` | 某项目在该 agent 下的会话目录 |
| `dozerd/src/headless_agent.rs` | `bare_program_name()` | headless CLI 裸命令名 |
| `dozerd/src/headless_agent.rs` | 两处 `build_*` 命令构造 | headless 调用参数 |
| `dozerd/src/summary_provider.rs` | `provider_isolation()` | provider 只读能力表 |
| `dozerd/src/transcripts/parse.rs` | 两处 `match agent` | transcript 解析分流 |
| `dozer-app/src/workspace/hook.rs` | `hook_install_target()` | hook 安装目标 |
| `dozer-app/src/workspace/hook.rs` | `agent_dot_color()` | 对话列表圆点颜色 |
| `dozer-app/src/workspace/hook.rs` | `agent_icon()` | 菜单图标 |
| `dozer-app/src/workspace/view.rs` | `agent_launch_command()` | 终端启动命令(Aider 走 launcher bridge) |

> `agent_picker_spec()`(菜单项)与 `transcripts/scan.rs`、`transcripts/mod.rs` 的目录枚举
> **已经是表驱动**(Part 1 起从 `AgentKind::known()` / `agent_paths` 派生),新 agent 只要
> §1 的 `label`/`display_label`/`agent_paths` 填好就自动出现,**不用单独改**。

## 2. 编译器抓不到的(必须靠测试/人工)

这类地方写了通配 `_ => Unknown` 或字符串字面量,加变体时编译器**一声不吭**,是真正的坑:

1. **`_ => Unknown` 的字符串映射**(写成 `label()` 的值,读回却变 `Unknown`——写得进读不回,
   无编译错、无运行时报错)。已知点位:
   - `dozerd/src/session_summary.rs`: `agent_from_str`(已改为 `AgentKind::from_label`,保留
     `every_agent_*round_trip*` 测试兜底)。
   - `dozerd/src/summary_jobs.rs`: 同上。
   - `dozerd/src/group_store.rs`: `agent_to_str` / `agent_from_str`(群聊成员)。
   - `dozer-hook/src/main.rs`: `parse_agent`(hook 事件里的 agent 名)。
   - `dozerd/src/server.rs:323`: hook 请求的兜底 `(Unknown, None)`。
   **接新 agent(尤其进群聊/进 hook 事件)时逐个确认是否要在这些映射里表态**——若该 agent
   会出现在这条链路上却漏了,它会被悄悄降级成 `Unknown`。`from_label()` 只在
   `session_summary`/`summary_jobs` 两处替换了手写分支;其余仍是手写,改动时对照。

2. **前端字符串映射**(Rust 侧类型改了,TS/JS 侧不同步,不会编译失败):
   - `dozer-panel-usage/web/usage-content/src/types.ts`: `AgentLabel` 联合类型。
   - `dozer-app/web/review-trace/src/types.ts` + `agentIcons.ts`: agent 名联合类型与
     `AGENT_STYLE` 表(有 `agentIcons.test.ts` 钉住"八家 + shell 全覆盖",新增要一起改)。
   - 这两个前端各有 `npm run build` 产物(`assets/.../*.js`),**改完前端源码必须重新 build 并
     一起提交**(见 CLAUDE.md 群聊同款要求)。

3. **`CREATABLE_AGENTS`**(`dozer-app/src/extensions/group_chat/mod.rs`):新建群弹窗可勾选的
   agent。是否支持群聊是**产品决策**,不是自动跟随;若该 agent 不进群聊,这里不列,但要确认
   `dozerd::group_store::agent_from_str` 不会因此把它误判。加进这里要同步后端 `agent_to_str`
   与 app 侧 `protocol::route_event` 的 `AddMember` 校验。

4. **byteui 语义色已用尽**:8 家 agent 已把 `byteui` 的语义色用完(`ignored` 与 `dim` 数值
   相同)。新 agent 只能借一个现有色(会有"像错误色"之类歧义),根治要给 byteui 加 agent
   专用色并**发新 tag**。

## 3. 持久化名一旦发出就不能改

`label()` 的值写进 SQLite、进 UDS 线上 JSON、被老进程反序列化。**新增变体随便起名,已有变体
的名字永不能改**(改了老 dozerd 读新名字会反序列化失败)。`dozer-core/src/protocol.rs` 有
`assert_eq!(AgentKind::X.label(), "...")` 钉住已发出的名字。`AgentKind::Unknown` 的 `label()`
是 `"shell"`,但 `transcripts/mod.rs` 历史上把它序列化成 `"unknown"`(有单独测试),不要统一。

## 4. 新 agent 的 L1 实操顺序(以 Qoder CN 为样本)

1. `dozer-model`:加 `AgentKind` 变体,填 `label()`/`display_label()`,加进 `ALL` 与测试
   `variant_index`(编译失败会提醒)。
2. `dozer-core/agent_paths.rs`:填 `projects_root_in` / `project_session_dir_in`(不做历史则
   两处都 `return None` 并写明原因)。
3. `dozer-app/workspace`:填 `agent_launch_command` / `agent_icon` / `agent_dot_color` /
   `hook_install_target`。
4. `dozer-hook`:若有 settings 文件,加 `install.rs` 的安装分支 + `main.rs::parse_agent`。
5. `dozer-mcp`:若支持 MCP,加 `install.rs` 的配置路径分支。
6. `dozerd`:确认 §2.1 的字符串映射是否需要表态;`default_agent_config.rs` 若要能选它,加进
   测试的 `(name, AgentKind)` 表。
7. 前端:§2.2 的联合类型与图标表(`review-trace` 有测试强制),重新 `npm run build`。
8. 跑门禁:见 §5。

## 5. 接完必须过的门禁

```bash
cargo fmt --all --check
cargo test --workspace --no-fail-fast
cargo clippy --all-targets          # 基线 33 warning,不应新增
python3 scripts/audit/unit_edges.py # 面板边界
bash scripts/check-log-scope.sh     # 日志来源
```

新 agent 若改了面板日志来源,来源名必须与 `PanelKind` 对应名一致(`app/state.rs` 有测试扫源码)。

## 6. 验证编译可达面(可选)

想确认"新变体到底触及多少非通配 match",可临时在 `AgentKind` 加一个 `Probe` 变体,按编译器
报错逐处补 `Probe` 分支,收集完再删。这是 audit §7 量"16 个文件"用的方法——比 grep
`AgentKind::` 可靠,因为 `_ =>` 兜底不会出现在 grep 结果里。
