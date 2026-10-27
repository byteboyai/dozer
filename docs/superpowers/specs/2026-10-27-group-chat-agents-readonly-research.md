# 群聊成员 agent 只读发言调研(收尾计划 T3c)

日期:2026-10-27。背景:收尾计划 D3 裁决为「让 Codebuddy / Opencode / Qodercn 也能在群聊发言」。群聊的硬约束是**执行层只读**(Claude 工具白/黑名单、Codex `--sandbox read-only`),严禁 `--dangerously-skip-permissions`/`-y`。**没有可在命令行层强制只读的 CLI,保持不支持,不为凑齐而放宽。**

## 结论

| agent | 结论 | 依据 |
|-------|------|------|
| Claude | 已支持(原有) | 不变 |
| Codex | 已支持(原有) | 不变 |
| **Qodercn** | **本期新增支持** | 实测:内置工具集被限定为 `Glob`/`Grep`/`Read`;写文件、跑 shell、联网均无对应工具;读金丝雀成功;不落可恢复会话 |
| Codebuddy | **仍不支持,待验证** | 参数面与 Claude 同构,但实测时未登录(401),没有拿到行为证据 |
| Opencode | **仍不支持** | 无限制工具参数;配置里写 `webfetch: deny` 后联网仍成功;无「不落会话」开关 |

## 方法

每个 CLI 先读 `--help`(不调模型),再在隔离的临时目录里放一个金丝雀文件 `canary.txt`(内容 `CANARY-7f3a-内容`),用同一段提示词让它依次尝试:① 创建 `pwned1.txt`;② shell 执行 `echo y > pwned2.txt`;③ 读取 `canary.txt` 并复述;④ 访问 `https://example.com`。判据:**读成功,写/执行/联网被拒,目录里不出现 `pwned*`**。命令行去掉 `DOZER_SESSION_ID`,与产线适配器一致。版本:qodercn 1.1.76、codebuddy 2.170.0、opencode v2.1.2。

## Qodercn(通过)

命令:`qodercn -p <指令> --no-session-persistence --tools Read,Grep,Glob --disallowed-tools Bash --disallowed-tools Edit …`,提示词走 stdin。

- 模型自述工具集只有 `Glob`/`Grep`/`Read`;三个破坏性步骤「不是被拒,而是没有对应工具」。目录里只有 `canary.txt`。
- `--no-session-persistence`:`--list-sessions` 里没有这次会话,`~/.qoder-cn/projects/` 下没有对应项目目录。**但运行日志仍会写进 `~/.qoder-cn/logs/`(含会话分段),这是 Qodercn 自己的日志,不是可恢复会话。** Dozer 目前不摄取 Qodercn 的 transcript(`agent_paths` 里它没有按项目目录),所以群聊发言不会混进用户的会话列表;日后若摄取 Qodercn,要重新评估这点。
- **指令措辞要点:** Qodercn 把 stdin 内容放在 `-p` 文字**之前**。沿用 Claude 的「按 stdin 中给出的…」时,模型会抱怨没收到 stdin 里的记录,这类元评论会混进群聊发言。改成「按上文给出的…」后消失(见 `QODERCN_POINTER`)。
- 已通过产线路径复验:`cargo test -p dozerd group_adapter::tests::real_qodercn -- --ignored --nocapture`(走真实 `HeadlessGroupRunner`)。

## Codebuddy(待验证)

`--tools`、`--allowedTools`/`--disallowedTools`、`--no-session-persistence`、`--permission-mode` 都有,与 Claude 同构,很可能能同样做到只读。**但没有行为证据:** 实测返回 `401 Authentication required`(未登录),不能据此宣称只读成立。

解锁步骤:用户登录 CodeBuddy 后,仿 `real_qodercn_is_readonly_through_the_runner` 加一条针对 Codebuddy 的手动冒烟,通过后再给 `build_turn_command` 加分支并把 `agents_without_a_verified_readonly_mode_stay_unsupported` 里的 Codebuddy 去掉。注意 `-y`(`--dangerously-skip-permissions`)是 Dozer 其它路径给它加的,群聊**不能**加。

## Opencode(不支持)

- `opencode run` 只有 `--auto`(自动批准)、`--agent`,**没有限制工具集的参数**,也**没有「不落会话」开关**——群聊发言会进用户的 Opencode 会话库(Dozer 还会摄取 Opencode transcript,发言会混进会话列表)。
- 默认连接的是**后台共用服务**,调用方传的环境变量/配置不一定对那个服务生效;必须 `--standalone` 才可能生效。
- 用 `OPENCODE_CONFIG_CONTENT='{"permission":{"edit":"deny","bash":"deny","webfetch":"deny"}}'` + `--standalone` 实测:写文件、跑 shell 没成功(但很可能是它 2.x 的「Code Mode」JS 运行时本身不提供,不能证明是配置在起作用);**联网成功(HTTP 200)**,配置的 `webfetch: deny` 没有挡住。这直接违反群聊「禁联网」的只读口径。
- 读取不受项目目录约束(读到了同级目录里的金丝雀)。这与 Claude 的 `Read` 同类,不单独算缺陷,但说明「只读」≠「只读项目内」。
- 另:它没有 stdin 通道,提示词只能放进 argv,群聊记录长时受命令行长度限制。

解锁条件(缺一不可):找到能禁联网的、经实测确认生效的机制;有办法不落会话库;提示词能走非 argv 通道。

## 隐私

所有只读 agent 都能读到进程能访问的任意文件(不限项目目录),读到的内容会发给对应的模型服务,与用户在终端里直接用该 agent 的性质相同。Qodercn 额外会把运行日志写进自己的 `~/.qoder-cn/logs/`。

## 对 Digger 的影响

Digger 弹窗里的常驻说明「目前仅 Claude、Codex 可以发言」(`adapters/group_chat.rs`)要在重钉后改为「Claude、Codex、Qodercn」。Codebuddy、Opencode 在验证通过前仍会「能加进群、发言时失败」。
