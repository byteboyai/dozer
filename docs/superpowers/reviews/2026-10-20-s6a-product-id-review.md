# S6a Part A(产品身份与 `dozerd` 库化)开发结果审核

日期:2026-10-20。

审核对象:[2026-10-18-agent-s6a-daemon-identity.md](../plans/2026-10-18-agent-s6a-daemon-identity.md) 的 **Part A(Task 1–6)**,Dozer 分支 `feat/product-id`(worktree `../dozer-product-id`),HEAD `e742298d`,基于当前 `main`(`b9373b02`),共 6 个提交,43 个文件、+2156/−534。

## 结论

**第二轮复审(2026-10-21,修复提交 `8c556d83`):R1–R4 全部通过;唯一阻塞合并的 N1(`Cargo.lock` 未提交)已由审核人在 `72a3544c` 修复并验证,代码侧已无阻塞项。** 下一步是用户目测验收。详见文末"第二轮复审"。

**第一轮结论(2026-10-20):Part A 暂不通过合并。R1、R2、R3 三项 P1 与 R4 修复后再复审。** 用户已于 2026-10-20 裁决:R2 采用建议的兼容策略,R3 选方案 A。 其余实现质量好:产品身份、会话环境变量、Claude hook 并排共存、`dozerd::run` 库化、进程级隔离测试都与计划一致,测试扎实。

**S6a 整体未完成。** Part C(Task 7–11:`diggerd`、`digger.db`、迁移、拉起、打包)在 Digger 仓库没有任何分支、worktree 或文件(无 `feat/diggerd`、无 `digger-s6a`、无 `diggerd` / `digger-hook` / `digger-paths`)。Task 6 的 Step 3(用户目测验收)与 Step 4(合并、切 tag、推送)也未做。实现者没有擅自打 tag,这点正确。

**验证记录(审核人在 `feat/product-id` worktree 实跑,输出与退出码分别落盘):**

| 检查 | 结果 |
| --- | --- |
| `cargo test --workspace` | 退出码 0;3356 通过 / 0 失败 / 4 ignored;分支未新增任何 `#[ignore]` |
| `cargo clippy --workspace --all-targets` | 退出码 0;39 条警告,**无一在本分支改动的文件里** |
| `cargo fmt --check` | 退出码 1,**`main` 上同样为 1**;失败的 9 个文件与本分支改动文件无交集,属既有问题 |
| `scripts/check-log-scope.sh` | 退出码 1,**`main` 上同样为 1**;命中 `workspace/preview_panel.rs:233`,本分支未改该文件,属既有问题 |
| Task 6 自检 | `"DOZER_SESSION_ID"` / `"dozerd.sock"` / `"dozer.db"` 字面量只剩 `ProductId::dozer()` 定义与测试夹具;`dozerd/src/main.rs` 恰好 40 行(达标,在上限) |

**已读代码:** `paths.rs`、`install.rs`、`opencode_install.rs` / `goose_install.rs` / `aider_launcher.rs` 的 diff、`protocol.rs` 握手部分、`dozer-client` 握手、`daemon.rs` 的 `run` 与关停段(对照旧 `main.rs`)、`cli.rs::run_main`、`product_isolation.rs` 用例清单。**未细读:** `session.rs`、`headless_agent.rs`、`group_adapter.rs`、`server.rs` 的参数化改动,以及 `dozer-mcp` 的小改动;未做人工目测验收。

## R1 · P1:Digger 的 opencode 插件 import 路径写死 `dozer-lib`

**位置:** `crates/dozer-hook/src/opencode_install.rs` 第 34–35 行(`TRANSLATE_IMPORT_SRC` / `TRANSLATE_IMPORT_INSTALLED`)、第 76 行(`lib_dir_name = format!("{}-lib", product.name)`)、第 90 行(`.replace(TRANSLATE_IMPORT_SRC, TRANSLATE_IMPORT_INSTALLED)`)。

**问题:** 库目录名已按产品推导,翻译文件写到 `<product>-lib/dozer-translate.ts`;但写进插件文本的 import 路径是常量 `"./dozer-lib/dozer-translate"`,不随产品变化。Digger 身份下,插件文件是 `digger.ts`,翻译文件在 `digger-lib/`,而插件去 import `./dozer-lib/...`:

- Dozer 没装时,import 失败,opencode 加载不了这个插件,Digger 里 opencode 的状态点永不更新——正中 M1 成功标准(Digger 里 opencode 要能干活)。
- Dozer 装着时,Digger 的插件悄悄依赖 Dozer 的库目录;Dozer 一卸载(会删 `dozer-lib`),Digger 的插件随之失效,违反"两产品互不串线"。

该结论来自代码阅读,未运行复现。

**为什么测试没抓到:** 该文件的 `run_at_with_exe` 直接读进程级 `product()`,没有 `install.rs` 里那种 `_for(product)` 变体,所以整组测试只覆盖 Dozer 身份(断言 `dozer.ts`、`dozer-lib`)。

**修改要求:**
1. 安装后的 import 路径由 `lib_dir_name` 动态生成(例如 `format!("\"./{lib_dir_name}/dozer-translate\"")`),不再用写死的常量;源码树里的 `TRANSLATE_IMPORT_SRC` 保持不变。
2. 给该文件加产品参数化的内核(如 `run_at_with_exe_for(dir, install, exe, product)`),原 `run_at_with_exe` 作为取 `product()` 的包装,签名不变。`goose_install.rs` 若同样依赖进程级 `product()`,一并加,保持两处一致。

**验收:**
- 新增测试,用非 Dozer 身份(沿用 `install.rs` 的 `digger_product` 夹具)安装到临时目录:插件文件为 `digger.ts`,其 import 串精确等于 `"./digger-lib/dozer-translate"`,且该目标文件真实存在;文本里不再含 `./dozer-lib`。
- 同一临时目录里先后以 Dozer、Digger 身份各装一次,两套文件(`dozer.ts` + `dozer-lib/`、`digger.ts` + `digger-lib/`)并存且互不引用;只卸载其中一个,另一个的插件及其库目录完整无损。
- 原有 Dozer 身份的全部 opencode 测试原样通过、期望不改。

## R2 · P1:客户端版本握手未接入,计划要求的测试缺失

**位置:** `crates/dozer-client/src/lib.rs` 的 `Client::check_version`(第 94 行)、`hello_once` 中 `product: _`(第 119 行);`crates/dozer-client/tests/version_handshake.rs`。

**问题:** 计划 Task 4 的 Interfaces 要求"`Client` 在首个业务请求前调用一次(结果缓存)"。实际 `check_version` 在**生产代码里没有任何调用点**,`grep` 只命中测试;握手只有守护进程端(`server.rs`)和 hook 的 `forward.rs` 接了线。因此版本不一致时 GUI 与 MCP 仍会走到"莫名其妙的反序列化失败",而这正是该任务要消除的现象。提交说明没有提及这个偏差。

另外:
- 计划列出的测试 `client_refuses_requests_after_a_protocol_mismatch_with_a_readable_error` **没有写**,实现里只有 3 个握手测试(`matching_versions_pass_and_the_check_is_cached`、`protocol_mismatch_yields_a_readable_error`、`old_daemon_that_only_replies_error_is_reported_as_too_old`),都只直接调 `check_version`,没有任何一个验证"握手失败后业务请求被拒"。
- `Reply::Hello.product` 客户端不校验(`product: _`),连到另一个产品的守护进程不会被发现。

**兼容策略(总设计 Q3,用户 2026-10-20 已确认):** 把握手直接接入后,任何**早于本改动、仍在运行的旧 `dozerd`**(不认识 `Hello`,回 `Reply::Error`)会被判为 `DaemonTooOld`;若一律拒绝,新 GUI 将无法使用它,用户需要先杀掉 `dozerd` 才能连上,这会丢掉所有存活会话。2026-10-20 审核时的实际环境正是如此:`/Applications` 里的 `dozerd` 自 Oct 16 起常驻,早于本改动。

**已确认的策略:**
- `ProtocolMismatch`(版本号明确不等)拒绝;`Reply::Hello.product` 与当前产品名不符(连到了别的产品的守护进程)拒绝。
- **同一产品**的旧守护进程(只回 `Reply::Error`、不认识 `Hello`,即 `DaemonTooOld`)**放行**,仅在日志里记一条 warn,不阻断、不向用户弹错。
- 放行只针对"不认识 `Hello`"这一种情形;能正常应答 `Hello` 的守护进程,版本不一致就是不一致,不放行。

**修改要求:**
1. `Client` 在首个业务请求前自动调用 `check_version`(结果缓存),握手失败时业务请求返回对应 `ClientError`,不再继续发请求。
2. 校验 `Reply::Hello.product` 与当前 `paths::product().name` 一致。
3. 补上计划列出的缺失测试。

**验收:**
- `client_refuses_requests_after_a_protocol_mismatch_with_a_readable_error`:用假守护进程回不一致的版本,断言先调业务方法(如 `list`)得到可读错误(文案含两个版本号),且假守护进程**没有收到**任何业务请求。
- 产品名不符的测试:假守护进程回 `product: "digger"`,Dozer 身份的客户端拒绝并给出可读错误。
- 同产品旧守护进程(假守护进程只回 `Reply::Error`):`check_version` 返回 `Ok`,业务请求正常发出并得到应答,日志里有一条 warn;并在 `Client` 的文档注释里写明这条策略。
- 能应答 `Hello` 但版本不一致的守护进程仍被拒绝(`protocol_mismatch_yields_a_readable_error` 期望不变)。
- 现有测试 `old_daemon_that_only_replies_error_is_reported_as_too_old` 与新策略冲突(它断言旧守护进程被报"过旧"):该测试要改成断言放行,并在测试注释里写明改动原因是用户 2026-10-20 的兼容裁决。`DaemonTooOld` 这个错误种类若不再有产生路径,一并删除,不留死代码。

## R3 · P1:`~/.dozer/agents/*` 日志目录两产品共用(用户已选方案 A:按产品参数化)

**位置:** `crates/dozer-core/src/agent_paths.rs` 第 72、94、115 行(`opencode_project_dir_in` / `goose_project_dir_in` / `aider_project_dir_in` 里的 `.dozer/agents/<agent>`)和第 162–164 行(`projects_root_in` 的同样三条);`crates/dozer-hook/src/opencode.rs` 第 38、81 行(`home.join(".dozer")`)。本分支未改动这些文件。

**问题:** opencode、goose、aider 的 hook journal 写在 `~/.dozer/agents/<agent>/projects/...`,根目录写死 `.dozer`,不随 `ProductId` 变化。总设计 §5 的反目标写明"Digger 的 `diggerd` 读写了 Dozer 的目录、socket 或数据库"算失败。Digger 会话里的 `digger-hook` 会把 journal 写进 Dozer 的目录,`diggerd` 再从里面摄取。会话文件按 session uuid 命名,实际不会内容串线,但违反既定隔离原则。这是**计划 Task 2 的遗漏**(它只列了环境变量与 Agent 配置的产品化),不是实现者偏离。

**用户裁决(2026-10-20):方案 A。** 这三家的根目录按产品推导:Dozer 仍是 `~/.dozer/agents/<agent>`(字节不变),Digger 为 `~/.digger/agents/<agent>`。

**范围界定(避免过度改动):**
- **改:** 只有 opencode、goose、aider 三家。它们的 journal 是 Dozer 自己写的,所以根目录属于产品。
- **不改:** Claude、codebuddy、codex、v8agent 读的是各 agent 自己的目录(如 `~/.claude/projects`),天然被所有产品共享,不属于任何一个产品的目录。两个产品同时安装时,各自的守护进程都会扫到这些 agent 自有目录,这是 agent 数据的固有性质,不在本项修复内。
- **不改:** 项目目录下的 `.dozer/code-health.toml`(`dozer-codehealth`)是项目级配置文件,不是产品数据目录。
- **不改:** `DOZER_AIDER_*`、`DOZER_ORIG_ZDOTDIR` 等环境变量,计划 Task 2 已说明它们指向各守护进程自己状态目录里的文件。

**修改要求:**
1. 在 `agent_paths.rs` 里把产品根目录名收敛成一处来源(如 `fn product_dir_name() -> String`,取 `paths::product().name`,前面加 `.`),三家的 `*_project_dir_in`、`projects_root_in`、`project_session_dir_in` 都从它派生,文件内不再出现字面量 `".dozer/agents/..."`。带 `home` 参数的 `_in` 变体仍按进程当前产品取根目录名(与同文件其它自由函数的做法一致),签名不变。
2. `dozer-hook/src/opencode.rs` 的两处 `.join(".dozer")` 改为同一来源,hook 写 journal 与 `dozerd` 摄取读 journal 必须读到同一个路径。
3. 不得出现"hook 写一处、摄取读另一处"的分叉:两侧只能通过 `agent_paths` 取路径。

**验收:**
- **黄金测试(Dozer 不变):** 默认产品下,三家的 `*_project_dir_in(home, cwd)` 与 `projects_root_in` 的结果与改造前字符串逐字相同(例如保留现有 `ends_with("/.dozer/agents/opencode/projects/-a-b-anrong_finagent")` 这类断言且通过)。
- **Digger 身份:** 新增测试,在 Digger 身份下三家路径都落在 `~/.digger/agents/<agent>` 之下,且与 Dozer 的任何路径互不包含。由于 `install` 是进程级,这类测试要么走接受 `ProductId` 参数的内核函数,要么放在独立测试进程里,与 `install_same_product.rs` 同样的做法。
- **读写一致:** 新增测试,以非 Dozer 身份让 hook 侧的 journal 写入函数写一条记录,再用摄取侧的扫描函数在同一 `home` 下能读到;并断言 Dozer 的 `~/.dozer/agents` 下没有新增任何文件。
- 原有 `agent_paths` 与 `transcripts::scan` 的测试期望不改。
- `grep -rn '"\.dozer/agents\|".dozer"' crates --include='*.rs'` 在非测试代码里不再命中(`dozer-codehealth` 的项目级 `.dozer/code-health.toml` 除外)。

**计划同步:** 该项作为新增任务补进 S6a(见计划末尾"复审追加"),放在 Task 2 之后、Task 3 之前不强求,但必须在 Part A 合并前完成。

## R4 · P2:`DaemonFeatures` 的非法组合会 panic

**位置:** `crates/dozerd/src/daemon.rs` 第 19 行(`DaemonFeatures`,字段均为 `pub`)、第 171–183 行。

**问题:** `task_poller == true` 且 `summaries == false` 时,启动走到 `.expect("summaries off 时不应有 task_poller")` 直接 panic。`DaemonFeatures` 的字段全是公开的,调用方(含将来的 `diggerd`)可以合法地构造出这个组合。

**修改要求:** 在 `run` 开头(任何有副作用的启动工作之前,且在 `paths::install` 之后或之前均可,但须在 `ensure_single_instance` 之前)校验组合,非法组合返回带可读文案的 `Err`,不 panic;或者让 `task_poller` 在无 `summary_service` 时自动不启动并记 warn。二选一,在提交说明里写明。

**验收:** 新增测试:`task_poller: true, summaries: false` 的配置调用 `run` 得到 `Err`(或明确的降级行为),不 panic;`Default` 与 `minimal()` 的行为不变。

## R5 · 已关闭(用户裁决:保持现状):hook 条目识别过宽

**位置:** `crates/dozer-hook/src/install.rs` 的 `entry_is_product`;测试 `a_foreign_entry_with_a_similar_name_keeps_current_substring_behaviour`(第 292 行)。

**说明:** 识别沿用子串匹配,命令里含 `dozer-hook` 即认作 Dozer 条目,所以别人的 `/opt/my-dozer-hook-wrapper` 会被 Dozer 的重装顶掉。实现者按计划"保持现状、写成测试钉住、交用户裁决"处理,做法正确。**用户 2026-10-20 裁决:保持现状,不收紧。** 该测试继续作为现状特征化测试保留;若将来改成路径末段精确匹配,须连同该测试的期望一起改。无需实现者动作。

## 修复范围汇总

| 编号 | 级别 | 状态 | 责任 |
| --- | --- | --- | --- |
| R1 | P1 | 需修复 | 实现者 |
| R2 | P1 | 需修复(兼容策略已由用户确认) | 实现者 |
| R3 | P1 | 需修复(用户已选方案 A;计划遗漏项) | 实现者 |
| R4 | P2 | 需修复 | 实现者 |
| R5 | — | 已关闭:保持现状(用户裁决) | 无 |

**复审与剩余门禁:**
1. R1、R2、R3、R4 修复并补测试后,审核人复审,复跑 `cargo test --workspace`,并对 R1 做用非 Dozer 身份的安装实测、对 R3 做 hook 写入与摄取读取的路径一致性实测。
2. Task 6 Step 3:用户在 Dozer 上目测验收(新建 Claude 会话、状态点随 hook 更新、重启后会话仍在、`~/.claude/settings.json` 的 hook 条目与改造前一致)。**未验收不继续。**
3. Task 6 Step 4:合并、切 `libs-*` tag、推送,须向用户确认后执行。
4. 以上完成之后才开始 Part C(Task 7–11);Part C 目前一行未动。
5. `fmt` 与日志门禁在 `main` 上本就不通过,与 S6a 无关,不计入本次修复范围,建议另行清理。

---

## 第二轮复审(2026-10-21)

审核对象:`feat/product-id` 新增的单个修复提交 `8c556d83`(11 个文件,+840/−112),基于上一轮的 `e742298d`。

**验证记录(审核人实跑):**

| 检查 | 结果 |
| --- | --- |
| `cargo test --workspace` | 退出码 0;**3366 通过**(第一轮 3356,净增 10)/ 0 失败 / 4 ignored(未变) |
| `cargo clippy --workspace --all-targets` | 退出码 0;39 条警告(与第一轮相同),无一在改动的文件里 |
| 干净快照 `git archive HEAD` 后 `cargo metadata --locked --offline` | **退出码 101**:`cannot update the lock file ... because --locked was passed`;而上一轮的 `e742298d` 同样检查退出码 0 → 这是修复提交引入的 |

**R1 通过。** `opencode_install` 与 `goose_install` 都新增了接受 `ProductId` 的 `run_at_with_exe_for`,安装后的 import 串由 `lib_dir_name` 动态生成(`"./{lib_dir_name}/dozer-translate"`),写死的常量已移到测试侧只用来钉 Dozer 身份的产物。新测试覆盖:Digger 身份下插件为 `digger.ts`、import 精确为 `"./digger-lib/dozer-translate"`、目标文件存在、文本里没有 `./dozer-lib`、不写任何 Dozer 名下的文件;两产品并存后互不引用;卸载 Digger 后 Dozer 的插件逐字不变。goose 同样有并存与单独卸载测试。

**R2 通过。** 握手接入了 `roundtrip`、`attach` 和应用传输层三个业务入口,第 193 行的直连是握手自身;`dozer-hook` 走自己的 `forward.rs`;其它 crate 没有绕过 `Client` 直连 socket 的生产代码。策略与裁决一致:版本不符、产品不符拒绝;不认识 `Hello` 的同产品旧守护进程放行并记一条 warn;`DaemonTooOld` 已删除并换成 `ProductMismatch`。测试记录了假守护进程实际收到的请求类型:版本不符时断言只收到 `hello`(业务请求没有外发);旧守护进程放行时严格是 `hello` 然后 `list_sessions`,且放行结果被缓存;产品名不符与"能应答 `Hello` 但版本不一致"都仍被拒绝。计划要求的 `client_refuses_requests_after_a_protocol_mismatch_with_a_readable_error` 已补上。

**R3 通过。** `agent_paths.rs` 收敛成 `product_agents_root_in` 一处来源,三家的 `*_project_dir_in`、`projects_root_in`、`project_session_dir_in` 都从它派生;Claude、codebuddy、v8agent 仍走各自 agent 的目录,有测试钉住不随产品变。`dozer-hook/src/opencode.rs` 的写入转调 `agent_paths::opencode_project_dir_in`,hook 与摄取共用同一路径来源。我核对了两点:原来 `opencode.rs` 自带的 `project_key` 与 `agent_paths::project_key` 实现完全相同(都是 `replace('/', "-")`),所以 Dozer 的 hook 写入位置没有变化;非测试代码里已无 `.dozer/agents` 字面量(剩下的命中是注释、测试和项目级的 `.dozer/*.json|toml`,均在范围外)。新增 `agent_paths_product_identity.rs`(Digger 落点、与 Dozer 互不包含、非 journal 家不变)与 `journal_product_isolation.rs`(读写一致、Dozer 目录零写入、home 顶层只出现 `.digger`)。

**R4 通过。** 新增 `validate_features`,在单实例检查等任何有副作用的启动工作之前调用,非法组合返回带 `task_poller` 与 `summaries` 字样的可读错误;`Default` 与 `minimal()` 都合法。

### N1 · P2:修复提交新增了依赖,但 `Cargo.lock` 没有提交(已关闭:`72a3544c`)

**处置(2026-10-21):** 经用户同意由审核人直接修复:确认工作区只有 `Cargo.lock` 的那一行改动(`+ "serde_json",`)后,以追加提交 `72a3544c` 单独提交它(未改写实现者的提交,未推送)。在干净快照 `git archive HEAD` 上 `cargo metadata --locked --offline` 退出码为 0,工作区干净。下面保留原始发现供追溯。

**位置:** `crates/dozer-client/Cargo.toml` 在 `8c556d83` 里新增 `serde_json.workspace = true`;工作区里 `Cargo.lock` 有未提交的对应改动(`dozer-client` 的依赖列表多一行 `"serde_json"`)。

**问题:** 已提交的状态与它自己的 lock 文件不一致。任何干净克隆、CI、以及按 `--locked` 构建的人都会失败;Digger 经 git 依赖编译 Dozer 的库(S6a Task 6 Step 4 的要求)时也会受影响。上面的快照实验已确认:上一轮提交通过,本提交不通过。

**修改要求:** 把 `Cargo.lock` 里这一行改动并入提交(用 `git commit --amend` 或追加一个只含 `Cargo.lock` 的提交均可)。注意只提交这一处,`Cargo.lock` 里不得夹带别的无关改动。

**验收:** 在干净快照上 `cargo metadata --locked --offline` 退出码为 0;`git status` 里 `Cargo.lock` 不再是 modified。

### 次要观察(不阻塞,不要求修)

- **R3 的"hook 写入"并未真正经过 hook 的写入函数:** `journal_product_isolation.rs` 里是用 `agent_paths` 的函数自己拼路径再写文件,没有调用 `dozer-hook` 的 `append_transcript_line_in`(`dozerd` 的测试不依赖 `dozer-hook`)。由于 hook 现在直接转调同一个 `agent_paths` 函数,风险很小;`dozer-hook/src/opencode.rs` 自身的测试仍只覆盖 Dozer 身份的布局。若 Part C 之后想补,可以在 `dozer-hook` 侧加一条非 Dozer 身份的路径测试。
- **R4 只单测了 `validate_features`,没有测 `run` 本身返回 `Err`:** 因为校验在 `run` 里先于任何副作用调用,从代码看行为是对的,只是没有端到端断言。
- **R2 的 warn 日志没有被断言:** 放行判定由 `classify_hello_reply` 单测钉住,warn 只是 `tracing::warn!` 一行,没有测试断言它被输出。

### 剩余门禁

1. ~~修好 N1 并提交~~ 已完成(`72a3544c`)。
2. **Task 6 Step 3:用户在 Dozer 上目测验收**(新建 Claude 会话、状态点随 hook 更新、重启 Dozer 后会话仍在、`~/.claude/settings.json` 的 hook 条目与改造前一致)。这一步现在也要多看一件事:**用户机器上常驻的旧 `dozerd` 在新 App 启动后仍能连上**(R2 放行策略的真实场景)。
3. **Task 6 Step 4:合并、切 `libs-*` tag、推送,须向用户确认后执行。**
4. 以上完成后才开始 Part C(Task 7–11)。

---

## 追加(2026-10-21,审核 S6b 时发现):N2 · P1:`product_daemon.rs` 被 `.gitignore` 吞掉,从未提交(已关闭:`b7763bb8`)

**处置(2026-10-21,经用户同意由审核人直接修复):** 在 `feat/product-id` 上追加提交 `b7763bb8`:提交 `crates/dozerd/examples/product_daemon.rs`(43 行,内容核对过就是测试要的辅助进程),并在 `.gitignore` 的 `examples/` 后加例外 `!crates/dozerd/examples/`。核对过例外没有扩散:`git status --ignored` 不再列出 `crates/dozerd/examples/`,其它 `examples/` 目录(如 `crates/dozer-app/examples/`)仍被忽略。**干净快照验证:** `git archive HEAD` 解到临时目录后 `cargo test -p dozerd --no-fail-fast` 退出码 0,**508 通过 / 0 失败 / 1 ignored**,`product_isolation` 的 4 个测试都实际运行并通过。随后 S6b 的 `feat/panel-term` 已变基到新顶端 `b7763bb8`(无冲突,变基前后的差异恰好是这两个文件)。下面保留原始发现供追溯。

**位置:** `.gitignore` 第 8 行 `examples/`;被吞掉的文件是 `crates/dozerd/examples/product_daemon.rs`;依赖它的是 `crates/dozerd/tests/product_isolation.rs`(第 17–38 行按 `target/**/examples/product_daemon` 找示例二进制)。

**问题:** S6a 的进程级隔离测试(计划 Task 5 的硬门禁)需要这个示例二进制,但文件被 `.gitignore` 的 `examples/` 忽略,**从来没有进过任何提交**:`git show 72a3544c:crates/dozerd/examples/product_daemon.rs` 报 `does not exist`。它只存在于实现者 S6a worktree 的磁盘上(`git status --ignored` 显示 `!! crates/dozerd/examples/`),所以只有在那个目录里测试才会通过。任何干净克隆、CI、以及 S6b 的 worktree 里,`product_isolation` 的 4 个测试都会失败(`找不到 product_daemon 示例二进制`)。S6b 的实现者在基线上撞到了这 4 个失败,但在报告里只写成"文件在该提交上不存在、与 S6b 无关",没有指出根因是被忽略。

**我上一轮为什么没发现(审核人的疏漏):** 第一、二轮我是在实现者的 S6a worktree 里直接 `cargo test --workspace`,那个目录里被忽略的文件恰好在磁盘上,所以测试全绿(3356、3366)。我只对 `Cargo.lock` 做了干净快照验证,没有对测试也做。这两轮表里"3356 / 3366 通过"在干净克隆上无法复现,**以实现者与 S6b 的数字为准:干净克隆上 `product_isolation` 的 4 个测试失败**。以后凡是"提交后的状态是否自洽"的结论,测试也必须在干净快照上跑。

**修改要求:**
1. 把 `crates/dozerd/examples/product_daemon.rs` 提交进 `feat/product-id`:`git add -f crates/dozerd/examples/product_daemon.rs`(仓库里已有先例:`crates/dozer-client/examples/*.rs` 也是强制添加的)。同时在 `.gitignore` 的 `examples/` 下面加例外 `!crates/dozerd/examples/`,避免下一个人再踩;改 `.gitignore` 要在提交说明里写明,并确认它不会让别的被忽略文件意外进入仓库(`git status` 对比前后)。
2. 提交前看一眼文件内容(1.5 KB)确认它就是测试需要的那个,不夹带别的。

**验收:**
- 在**干净快照**(`git archive HEAD` 解到临时目录)里跑 `cargo test -p dozerd --test product_isolation`,4 个测试全部通过(退出码 0)。
- `git ls-files crates/dozerd/examples` 有 `product_daemon.rs`;`git status --ignored` 不再列出 `!! crates/dozerd/examples/`。

**叠分支的后果:** `feat/panel-term`(S6b)当前基于 `72a3544c`。S6a 追加这个修复提交后,要把 S6b 分支变基到新的 `feat/product-id` 顶端(`git rebase --onto <新顶端> 72a3544c feat/panel-term`)。S6c-1 还没开始,此时变基没有下游成本;变基后在干净快照上重跑 S6b 的测试,`product_isolation` 应转绿。
