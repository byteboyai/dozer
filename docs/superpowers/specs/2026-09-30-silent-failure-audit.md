# 静默吞错审计:用户发起动作后无声失败的点

状态:审计结论(2026-09-30),只读盘点,未改任何源码。承接 `2026-09-30-error-feedback-audit.md`(那份盘点的是"只写日志"的失败;这份盘点"连日志都没有"的失败)。

行号是审计时刻的快照(`main` 在 `9979806b`),迁移时以符号/上下文为准。

范围:`dozer-app`、`dozerd`(hook/mcp 只看 GUI 调用它们的位置)。

## 结论摘要

- **大头是无害的**:约 60% 的 `let _ =` 是 `proxy.send_event`(事件循环已关闭时才会失败)、终端喂数据、状态机迁移、webview 操作、临时文件清理和测试代码,保持静默是对的。
- **真正需要处理的约 15 处**,集中在五类,按对用户的伤害排序:
  1. **Keychain 写入失败被吞**——用户以为密码存好了,之后连接报一个莫名其妙的认证失败。
  2. **hook/mcp 安装的返回码被丢**——Agent 面板的名字/状态永远停在 `Unknown`/`Idle`,这正是 2026-08 那次线上事故的同类问题,而且这次连 stderr 都不进日志。
  3. **daemon 读请求失败被悄悄换成空数据**(14 处 `unwrap_or_default()`)——Todo 列表/记忆/书签"凭空清空",看起来像数据丢了。
  4. **用户点了按钮、请求失败、界面毫无变化**(处理 Todo、打开记忆)。
  5. **dozerd 后台任务的错误没有任何日志**(`task_poller`、`summary_service` 状态回写)。
- **一个相关的结构性缺口**:`daemon_unavailable` 徽标只覆盖"启动连不上"和"用户在设置里停掉"两种情形,**运行中 dozerd 崩了不会点亮它**。第 3 类问题在这种情形下最伤人——面板都是空的,顶栏却看不出 daemon 已经没了。

## 盘点方法与覆盖范围(诚实的边界)

| 写法 | 数量(app / dozerd) | 我怎么审的 |
|---|---|---|
| `let _ = <可失败调用>` | 313 / 34 | 排除已知无害目标(`send_event`、`feed`、`try_transition`、`view.*`、`request_redraw`、`interface.update`)后剩 131 处,**逐条读过** |
| 语句级 `.ok();` | 102 合计 | 全部列出:除 4 处赋值外都是测试里的临时文件清理,**无问题** |
| `Err(_) =>` | 29 / 4 | 33 处全部读过:多数已转成用户可见错误(各种超时文案),个别见下 |
| `.await` 后接 `.ok()`/`unwrap_or_default()` | 14 | 全部读过(daemon 读请求被换成默认值),见第 3 类 |
| `if let Ok(..)` 无 else | 18 / 7 | 全部读过,挑出 2 处(`get_memory`) |
| **未逐条审**:`.ok()` 转 `Option`(约 119)、非 client 调用的 `unwrap_or_default()`(约 108)、`let Ok(..) = .. else { return }`(55) | | 抽查为"文件不存在→取默认""窗口句柄取不到→返回"这类惯用写法,**没有逐条确认**;若要彻底,需要再做一轮 |

`let _ =` 里还有约 25 处是 `let _ = io;`、`let _ = kind;` 这种**消除未使用变量警告**的写法,不是吞错,但说明有死参数,见「顺带发现」。

## A. 建议处理(用户发起动作后无声失败)

| # | 位置 | 现状 | 影响 | 建议 |
|---|---|---|---|---|
| A1 | `extensions/ssh.rs:593`、`extensions/database/update.rs:100`:`let _ = entry.set_password(..)` | Keychain 写入失败被吞,界面显示"已保存" | 之后连接失败,报认证错误,用户不知道是密码根本没存进去 | **Toast Error**"密码未能保存到系统钥匙串:{e}" + 日志(走 outbox) |
| A2 | `extensions/ssh.rs:610`、`extensions/database/update.rs:162`:`let _ = entry.delete_credential()`;`extensions/settings.rs:210`:`let _ = git_accounts::delete_token(..)` | 删除凭据失败被吞 | 界面显示已删除/已断开,钥匙串里仍留着密钥(安全相关) | **Toast Warning**"钥匙串里的凭据未能删除",日志 |
| A3 | `workspace/hook.rs:133/141/148/171`、`workspace/state.rs:1708-1709`:`ensure_hook_installed`/`ensure_mcp_installed` 的 `i32` 返回码被 `let _ =` 丢掉 | 安装失败(settings.json 解析失败拒绝写入、目录不可写…)时,原函数只 `eprintln!` 到**进程 stderr**(GUI 现在虽然落盘日志,但 `eprintln!` 不进日志文件) | Agent 面板 name/status 永远 `Unknown`/`Idle`,与 2026-08 事故同类;文档还写着"失败只 warn",**代码里没有 warn** | 检查返回码:非 0 写**日志 warn**(带 agent 名),并对首次失败推**keyed Toast Warning**"无法为 {agent} 安装 hook,Agent 状态可能无法识别"(同 agent 同会话只提示一次) |
| A4 | `app/update.rs:4694`:`let _ = client.process_todo_now(..)` | 用户在 Todo 详情里点"立即处理",请求失败被吞;随后照常刷新详情,看起来"什么都没发生" | 用户会重复点 | **Toast Error**"处理任务失败:{e}"(此处在后台任务里,经 `proxy` 发 `Message::Toast`) |
| A5 | `extensions/project/update.rs:289`、`:356`:`if let Ok(detail) = client.get_memory(..)` | 打开/刷新一条记忆失败时静默不动 | 点了没反应 | **Toast Error**"读取记忆失败:{e}" |
| A6 | `dozerd/src/task_poller.rs:90`:`let _ = process_task(..).await` | 后台轮询处理 Todo 任务,失败**没有任何日志** | 一直失败也无人知晓,每个轮询周期重试 | **日志 error**(dozerd 有日志服务),带 todo id |
| A7 | `dozerd/src/summary_service.rs:351/416/432/484`:`let _ = self.jobs.update_job_status(..)` / `recount_batch(..)` | 在**错误路径里**回写"失败"状态,回写本身失败又被吞 | 任务状态卡在 `Running`,批次计数不对,无迹可查 | **日志 error**(二次失败必须留痕) |
| A8 | `dozerd/src/headless_agent.rs:447/534`:`let _ = stdin.write_all(..).await` | 向无头 agent 的 stdin 写入失败被吞 | 总结/处理请求发出但 agent 没收到,表现为超时 | **日志 warn** |

## B. daemon 读请求失败被换成"空数据"

`client.xxx().await.unwrap_or_default()` 共 14 处。daemon 不通时,面板显示的是"没有数据"而不是"读不到":

| 位置 | 读什么 | 失败时用户看到 |
|---|---|---|
| `extensions/todo/state.rs:996`、`:1010`、`extensions/todo/update.rs:107` | Todo 列表/分类 | 列表清空,像任务丢了 |
| `extensions/project/update.rs:280`、`:424` | 记忆列表 | 记忆区域清空 |
| `extensions/browser.rs:1483` | 书签 | 收藏夹清空 |
| `app/app.rs:703`、`:705` | 启动时已知项目/存活会话 | 启动恢复不出项目/会话(此时 `daemon_unavailable` 通常已置位) |
| `app/update.rs:2883`、`:3980`、`extensions/project_create.rs:1088`、`:1119`、`workspace/hook.rs:437` | 最近项目 | "最近项目"列表清空 |

这一类**不适合 Toast**(它是"当前处于某状态",不是一次性事件)。两个方向,需要你定:

1. **最小做法:先写日志 warn**(带来源与错误),不改界面。至少让排查有据可查。
2. **面板内联的"读取失败,点击重试"状态**:Todo/记忆/书签各自加一个 `load_error`,替代空列表。更符合"状态留在原位"的判据,但要动 3 个面板的视图。

**关联的结构性问题**:`daemon_unavailable` 徽标在 dozerd **运行中崩溃**时不会亮。若在这里加一个轻量的存活判据(例如"任一 daemon 请求以连接错误失败时置位,下一次任一请求成功时清除"),这一整类问题就有了统一的可见信号,比给每个面板各做一个 `load_error` 便宜得多。这需要单独设计(误报、抖动、什么算连接错误),不在本审计内定。

## C. 低优先级 / 建议只加日志

| 位置 | 现状 | 建议 |
|---|---|---|
| `extensions/project/update.rs:404-408`(`spawn_scaffold_run`) | 静默 scaffold:四个同步步骤的 `Vec<(label, ScaffoldStepResult)>` 结果和 `backfill_project_transcripts` 的错误都被丢弃。文档明确写"不产出任何 UI 可见结果" | 静默是有意的,但**失败的步骤至少写 warn 日志** |
| `extensions/project/links.rs:137`:`let _ = save(repo, &state)` | 首次发现文档后落盘失败 | 日志 warn(下次打开会重新发现,损失很小) |
| `extensions/settings.rs:195`:`let _ = Command::new("open").arg(url).spawn()` | 打开 token 创建页失败被吞;和外部打开同一个坑(spawn 成功不代表打开成功) | 复用 `external_apps::run_open_command`,失败 Toast——但价值低 |
| `extensions/file_history.rs:274`:`Err(_) => loaded_diff = None` | diff 加载失败后面板空白且无原因 | 面板内显示"无法加载此版本"(状态,非 Toast) |
| `extensions/ssh/sftp.rs:257/282/309/323/339`、`workspace/state.rs:797/836/2222`:`let _ = tx.send(..)` / `out.send(..)` | 通道对端(连接 worker)已退出时,上传/下载/输入被丢弃 | 值得**验证**:连接断开时界面是否已经显示"已断开"。若已显示,则保持;若没有,这是 B 类的近亲 |
| `preview/startup.rs:89/97`、`runtime.rs:98/106`:写启动状态标记 | 尽力而为的诊断文件 | 保持 |

## D. 确认无需处理(保持静默是对的)

- `proxy.send_event(..)`(约 130 处):只有事件循环已关闭(退出中)才会失败。
- `t.feed(..)`(29 处)、`tab.backend_state.try_transition(..)`(18 处):终端喂数据与状态机,失败即"该 tab 已不存在"。
- `view.evaluate_script/zoom/set_bounds/focus/load_url`(约 20 处):webview 已被销毁。
- 临时文件与快照清理:`remove_file(&tmp)`(`text_save.rs`、`recovery.rs`、`startup.rs`)、`ide_bridge` 的锁文件清理、会话结束时的 `s.kill()`。
- 测试里的 `remove_file(..).ok()`(约 98 处)。
- `Err(_) =>` 里已经转成用户可见文案的超时分支(`database/*`、`ssh.rs`、`workspace/state.rs:1859/2016` 的"连接超时"):不是吞错,是分类。

## 顺带发现(与本审计无关但值得记)

- **约 25 处 `let _ = io;` / `let _ = kind;` / `let _ = shown;`**(如 `workspace/state.rs:816`、`:1188`、`extensions/files/update.rs:143`、`extensions/todo/view.rs:675`):用赋值消除"未使用参数"警告。它们指向**死参数**,可以用 `_` 前缀参数名或删参数收掉,是纯清理,不影响行为。
- `ensure_mcp_installed` 的文档注释声称"失败只 warn",与代码(丢弃返回值)不符,修 A3 时一并改正。
- `ensure_hook_installed` 里被丢的 `i32` 语义是"退出码"(`0` 成功),原函数把失败原因 `eprintln!` 出来——这些 `eprintln!` 在被 GUI 进程内直接调用时不会进入日志文件。**这是 `dozer-hook` 的 CLI 用法和 GUI 的库式调用共用同一个函数造成的**:CLI 需要 stderr,GUI 需要返回值/日志。更干净的修法是让安装函数返回 `Result<(), String>`(带原因),CLI 入口自己打印;这会动 `dozer-hook`/`dozer-mcp` 的 API,需要单独评审。

## 建议的批次

依赖:Toast 的 `Outbox` 与日志来源机制(均已落地)。

1. **批 1(伤害最大,改动集中)**:A1、A2(Keychain 读写)、A4(处理 Todo)、A5(打开记忆)。都是 outbox 或 `proxy` 一行接入 + 单测(Keychain 部分需要把 `entry` 操作抽成可注入的小函数才能测失败路径)。
   **已落地(2026-09-30,`plans/2026-09-30-silent-failures-batch1.md`)**:Keychain 写入/删除(SSH、数据库经新的可注入 `secrets::SecretStore`;Git 账户断开)、处理 Todo 与打开详情、记忆详情读取失败。与本审计的差异:① 记忆读取失败用**内联** `ws_state.error` 而非 Toast(同组"保存记忆失败"已用它);② 额外补了 `todo_detail_open` 与"Git 账户断开时本地记录写失败"(此前只有日志,界面表现为点了没反应);③ SSH 保存密码实际有**两个**吞错点(`keyring_entry` 失败被 `&& let Ok(..)` 跳过,`set_password` 失败被 `let _ =` 吞),都已覆盖。
2. **批 2(dozerd 日志)**:A6、A7、A8。只加日志,零 UI 风险。
3. **批 3(hook/mcp 安装)**:A3。先只做"GUI 侧检查返回码、写日志 + keyed Toast";让安装函数返回带原因的 `Result` 的 API 改造留后。
4. **批 4(读请求)**:B 类,**先做最小方案(日志 warn)**;面板内联重试状态与 daemon 存活判据各自单独立项,等你定方向。
5. **批 5(低优先级)**:C 类。

## 未决项(不擅自定死)

1. **B 类走哪个方向**:只写日志,还是加面板内联的"读取失败"状态,还是做 daemon 存活判据统一解决。这是本审计里最大的设计选择,我倾向"先日志 + 单独评估存活判据",因为它同时解决所有面板且不动视图。
2. **A3 是否要对用户弹 Toast**:hook 安装发生在会话启动时,不是用户显式动作,弹 Toast 可能是噪音;但它一旦失败后果严重(Agent 状态全断)。我倾向弹一次 keyed Warning。
3. **`dozer-hook`/`dozer-mcp` 的安装函数是否改成返回 `Result`**(见「顺带发现」),涉及两个 crate 的公开 API。
4. **`let _ = io;` 这类死参数清理**要不要顺手做(纯清理,不影响行为)。
5. **没有逐条审的三类**(`.ok()` 转 `Option`、非 client 的 `unwrap_or_default()`、`let Ok(..) = .. else { return }`,合计约 280 处)是否值得再做一轮。抽查是惯用写法,但没有逐条确认。
