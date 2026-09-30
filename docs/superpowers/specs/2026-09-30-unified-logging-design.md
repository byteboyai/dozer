# 统一日志服务设计

状态:草案(2026-09-30),待评审。不改变一期范围;不涉及规格 §8 的显式未决项。2026-09-30 已按评审意见定稿全部原「未决项」(见文末「已决事项」):纳入 hook/mcp、纯文本格式、只按天数保留不设大小上限、直接删除遗留 `[DIAG]`/`DEBUG` 日志、`Agent` 与 `agent_context` 合并来源。

## 实施结果与偏差(2026-09-30,已按 `plans/2026-09-30-logging-and-toast-migration.md` 落地)

与本文下面的设计相比,实施中有这些确认过的偏差,**以此节为准**:

1. **clippy `disallowed_macros` 门禁不可用**(spike B):它会把 `log_warn!` 等包装宏展开出的 `tracing::warn!` 在**每个调用点**报成违规(同 crate、跨 crate 都是),函数上加 `#[allow(clippy::disallowed_macros)]` 也压不住。所以**不建 `clippy.toml`**,门禁只有 `scripts/check-log-scope.sh`(`grep` 扫 `dozerd`/`dozer-app`,带 `// cli-output` 的行放行)。§5 的第 1 条作废。
2. **`target: $scope.target` 可行**(spike A):`const` 项的字段可以作 `tracing` 的 `target:`,`RUST_LOG=dozer::panel::files=debug` 的按面板过滤有单测。
3. **Toast 日志不能用调用方来源当 target**:`tracing` 的 `target:` 必须是常量,而 `push_toast` 的 `Scope` 是运行时形参。Toast 日志固定用 `module::toast` 作 target,调用方来源放进字段 `scope`(值为其 target 字符串)。所以 Toast 日志不能按面板过滤,但仍写明面板名;**没有** `toast = true` 字段(target 已经标明是 Toast 日志)。写日志的唯一函数是 `toast::log_toast`,`App::emit_toast` 与 `Message::Toast` 分支各调用一次(不是"一个咽喉点函数")。
4. **hook/mcp 范围收窄**:`dozer-hook`/`dozer-mcp` 的 `eprintln!` 绝大多数是 `install`/`uninstall`/`launch` 给命令行用户看的结果,不是日志,保持不动。运行期诊断只有 hook 的 4 处(已改 `plain_warn!`);`dozer-mcp` 没有。门禁脚本与日志约束只覆盖 `dozerd`/`dozer-app`。
5. **`Outbox`**(spec 未预见):`toast::Outbox` 让 extension 的 `update` 在拿不到 `App` 时排队提示,由 `App::update` 包装函数统一排空。
6. **遗留调试日志删除时发现一处安全问题**:`DEBUG term input fallback fired` 会把每次终端击键的字节以 `warn` 级别打出来。GUI 日志落盘后这会把口令等输入写进磁盘,所以是删除而不是降级。
7. **审计更正**:`tree_error` 不是"加载失败",全部写入点都是文件操作反馈;已拆成移动对话框内联的 `move_error` 与 Toast(见 `error-feedback-audit.md`)。
8. **外部打开失败**:审计假设 `Command::spawn()` 失败即打开失败,这是错的——`open -a 不存在的App` 能正常 spawn,`open` 随后才以非零退出码报错。改为后台等退出状态(`external_apps::run_open_command`)。


## 背景与动机

现状(审计于 2026-09-30):

- 代码里日志统一用 `tracing`,但**只有 `dozerd` 有日志服务**:`crates/dozerd/src/main.rs::init_logging()` 用 `tracing_subscriber` + `tracing_appender`,按天写到 `dozer_core::paths::logs_dir()/dozerd.log`,同时输出 stdout,`RUST_LOG` 控制级别、默认 `info`。这份初始化逻辑是 dozerd 私有的。
- **`dozer-app`(GUI)只有 `tracing_subscriber::fmt::init()`(`main.rs`)**:输出到 stderr、不落盘。用 `.app`/Finder 启动时日志基本丢失。GUI 里有 96 处 `tracing::`/`eprintln!` 调用,其中约 80 处是 `warn!/error!`,用户和开发者事后都找不到。
- `dozer-hook`、`dozer-mcp` 用裸 `eprintln!`(共约 50 处),无 tracing 依赖;它们由 agent 拉起,stderr 多半被吞。
- **日志里没有"这条日志来自哪个面板"。** 现有调用大多只有一句中文文案,少数(`app/update.rs` 里的 3 处)手写了 `panel = ?panel` 字段,写法不统一。想排查"Todo 面板的写操作失败"只能靠搜文案。
- 配套问题:GUI 里混着大量 `[DIAG]`/`DEBUG` 调试输出,全部用 `warn!` 级别(`window.rs`、`window_events.rs`),既污染真实告警,又在热路径上有开销。

本设计的直接触发点:用户要求"由 core 提供整体的 log 服务,每个面板可以使用这个服务,每个面板在日志中需要写明自己的面板名称"。它也是 `2026-09-30-error-feedback-audit.md` 里"保持日志即可"这一批结论的前提——那批结论假定日志真的能被找到。

## 目标 / 非目标

**目标**

1. `dozer-core` 提供统一的日志初始化与写日志入口;`dozerd`、`dozer-app` 共用同一份实现(格式、滚动、保留、过滤规则一致)。
2. **每条日志都带来源名。** 面板日志的来源名就是面板名(`files`、`todo`、`git_log`…),写在日志行里,可按面板过滤。
3. **写来源是编译期强制的**,不靠自觉:面板代码不能写出没有来源的日志。
4. GUI 日志落盘,并有保留策略(不会无限增长)。
5. `dozer-hook`、`dozer-mcp` 也接入同一套来源与文件约定,但**不依赖 `tracing`**(见「设计 §4b」)。

**非目标**

- 不做日志查看器/GUI 内的日志面板(可作为后续)。
- 不做远程上报/遥测。
- 不引入新的日志框架;`dozerd`/`dozer-app` 继续用 `tracing` 作为底层。
- 不做 JSON 输出,不做单文件大小上限(已决,见「已决事项」)。
- 不改变 Toast 对用户的展示行为(时长、堆叠、去重不变);但 Toast 的入口签名会多一个 `Scope` 参数并自动写日志,见「与 Toast 的关系」。

## 设计

### 1. 模块与依赖

新增 `crates/dozer-core/src/log.rs`,`dozer-core` 加 **cargo feature `logging`(默认关)**:

```toml
[features]
logging = ["dep:tracing", "dep:tracing-subscriber", "dep:tracing-appender"]
```

- `dozerd`、`dozer-app` 依赖 `dozer-core` 时开 `logging`。
- `dozer-hook`、`dozer-client`、`dozer-mcp` 不开,**不会被拉进 tracing**(hook 是"零依赖小二进制",这条约束不变)。
- `log.rs` 分两层:**始终编译**的部分(`Scope`、`scope!`、`Component`、下面「§4b」的纯 std 追加写函数、保留清理函数)不依赖任何第三方 crate;只有 `tracing` 后端(`init`、`log_*!` 宏)在 `logging` feature 后面。这样 hook/mcp 也能用同一套 `Scope` 与命名规则。
- `paths::logs_dir()` 已在 core,沿用。

### 2. 来源(Scope)

```rust
/// 日志来源。`target` 是 `tracing` 的 target 字符串,格式固定为
/// `dozer::<kind>::<name>`,面板日志即 `dozer::panel::<面板名>`。
#[derive(Debug, Clone, Copy)]
pub struct Scope {
    pub kind: &'static str,   // "panel" | "module"
    pub name: &'static str,
    pub target: &'static str, // concat!("dozer::", kind, "::", name)
}
```

声明宏(在模块顶部声明一个常量,一个模块一份):

```rust
// crates/dozer-app/src/extensions/todo/mod.rs
dozer_core::scope!(LOG, panel, "todo");
```

展开为 `const LOG: Scope = Scope { kind: "panel", name: "todo", target: "dozer::panel::todo" };`。`kind` 只有两种:

- `panel`:对应 `PanelKind` 的 11 个面板。
- `module`:非面板的来源(外壳、平台层、预览、runtime、dozerd 的各模块)。

来源名字符集限定为 `[a-z0-9_]+`,由 `scope!` 在编译期用 `const` 断言检查。

**为什么用 `target` 而不是自定义字段**:

1. `tracing` 默认的 fmt 输出本来就打印 target,所以日志行天然写明面板名,**不需要自定义 formatter**:
   `2026-09-30T09:12:03Z  WARN dozer::panel::todo: Todo 写操作失败: ...`
2. `RUST_LOG` 的标准语法直接支持按来源过滤:`RUST_LOG=info,dozer::panel::files=debug`。不需要自造过滤语言。
3. 输出格式以后换成 JSON 时,target 字段自然保留。

### 3. 写日志的入口

`dozer-core` 导出五个宏,第一个参数必须是 `Scope`:

```rust
dozer_core::log_warn!(LOG, "Todo 写操作失败: {e}");
dozer_core::log_error!(LOG, error = %e, "保存失败");
dozer_core::log_info!(LOG, project_id, "项目已打开");
```

展开为 `tracing::warn!(target: LOG.target, ...)`,其余参数原样透传(所以 `tracing` 的结构化字段语法都可用)。宏内部通过 `$crate::log::__tracing` 引用 tracing,调用方不需要自己依赖 `tracing`。

**共享代码里的运行时面板名**:预览(`preview/*`)这类被 Files 与 Project 两个面板共用的代码,来源用它自己的模块来源(`module::preview`),运行时面板作为普通字段带上:`log_warn!(LOG, panel = ?panel, tab_id, %error, "表格首次加载失败")`。规则:**来源标"代码属于谁",字段标"这次在替谁工作"。** 这也统一了现在 `app/update.rs` 里 3 处手写 `panel = ?panel` 的写法。

### 4. 初始化

```rust
pub enum Component { App, Daemon, Hook, Mcp }

/// 仅接受 `App`/`Daemon`(走 tracing);`Hook`/`Mcp` 用 §4b 的纯 std 写入。
pub fn init(component: Component) -> Guard;
pub fn init_at(component: Component, dir: &Path, stderr: bool) -> Guard; // 测试与自定义目录用
```

- `Guard` 持有 `tracing-appender` 的 `WorkerGuard`,调用方在 `main` 里绑住直到进程退出(与现有 dozerd 的约定一致)。
- 文件:`logs_dir()/dozer-app.log.<日期>` 与 `logs_dir()/dozerd.log.<日期>`,**两个进程各写各的文件**,避免两个进程同写一个文件的竞争;来源 target 在同一份文件里区分模块。按天滚动。
- 同时输出 stderr(`cargo run`/终端启动时可见),与现在 GUI 的行为兼容。
- 过滤:`RUST_LOG` 优先,缺省 `info`(与 dozerd 现状一致)。
- **保留策略**:初始化时删除 `logs_dir()` 下所有组件前缀(`dozer-app.log.`/`dozerd.log.`/`dozer-hook.log.`/`dozer-mcp.log.`)中修改时间早于 14 天的文件(保留天数为常量 `RETENTION_DAYS = 14`)。**只按天数,不设单文件或总大小上限**(已决)。这是对 dozerd 现状("不清理旧文件")的有意改变:GUI 与 hook/mcp 也开始落盘后,总量增长,不清理不再合适。清理由 `dozerd`/`dozer-app` 的 `init` 负责,hook/mcp 是短命进程,每次调用都扫目录不划算,它们的旧文件由前两者顺带清理。删除失败只忽略,不影响启动。
- 日志目录创建失败时降级为只输出 stderr,并记一条警告(沿用 dozerd 现有做法)。
- 启动时写一条 info 横幅:组件名、版本、pid,便于在滚动文件里定位一次运行的起点。
- 安装 panic hook:panic 信息(含位置)写一条 error 日志后再调用原 hook。GUI 目前崩溃时没有任何落盘记录,这条补上。

### 4b. `dozer-hook` / `dozer-mcp`:纯 std 追加写

hook 由 agent 每个事件拉起一次(短命、并发),mcp 的 stdout 是协议通道。两者都不能引入 `tracing`,也不能往 stdout 写。做法:`dozer-core::log` 始终编译的部分提供

```rust
pub enum PlainLevel { Error, Warn, Info }

/// 追加一行到 `logs_dir()/<组件>.log.<UTC日期>`。失败一律静默忽略(日志不能
/// 让 hook/mcp 本身失败)。行格式与 tracing 后端一致:
/// `2026-09-30T09:12:03.123Z  WARN dozer::module::hook: 消息`
pub fn plain_write(component: Component, scope: &Scope, level: PlainLevel, msg: &str);
```

并导出 `plain_error!/plain_warn!/plain_info!(SCOPE, "格式串", 参数...)` 三个宏(仅 `format!` 语义,无结构化字段)。实现要点:

- 用 `OpenOptions::append(true).create(true)` 打开,单行一次 `write_all`(`O_APPEND` 下小于 `PIPE_BUF` 的单次写是原子的,多个 hook 进程并发写同一文件不会交错)。
- 时间戳只用 `std::time::SystemTime`:自己把 Unix 秒换算成 UTC 日历日期与时间(一个几十行的纯函数,可单测),**不为此给 `dozer-core` 引入 `chrono`/`time`**。日期同时用作文件名后缀,与 `tracing_appender::rolling::daily` 的命名(`<前缀>.log.YYYY-MM-DD`)一致,所以保留清理函数一份逻辑通吃四种文件。
- 不写 stderr:hook 的 stderr 可能被 agent 当成 hook 输出展示,写进去会污染 agent 界面。这是对现状(`eprintln!` 直接输出)的有意改变——它们原本大概率被 agent 吞掉,落盘后反而可查。
- **区分"诊断日志"与"CLI 用户输出"**:`dozer-mcp` 的 `install`/`uninstall`/用法提示(`eprintln!("用法: dozer-mcp ...")`、`读 {path} 失败` 等)是**给命令行用户看的结果,不是日志**,必须保留为 `eprintln!`。迁移只替换运行期诊断(hook 的转发/落盘失败提示,mcp `serve` 路径里的运行期错误)。这些 CLI 输出所在的模块(`dozer-mcp` 的 `main.rs`/`install.rs` 里的用户输出函数)用 `#[allow(clippy::disallowed_macros)]` 局部豁免,并在注释里写明"CLI 用户输出,非日志"。

来源:`hook` 与 `mcp` 各一个 `module` 来源(`dozer::module::hook`、`dozer::module::mcp`)。hook 内部按 agent 适配器再分子来源不必要,消息文案里已含 agent 名。

### 5. 强制"必须有来源"

两层保险,不依赖代码审阅:

1. ~~**clippy `disallowed-macros`**~~ **(作废,见上面「实施结果与偏差」第 1 条:包装宏的每个调用点都会被误报。)**
2. **兜底脚本**:`scripts/check-log-scope.sh` 用 `rg` 在 `crates/{dozerd,dozer-app,dozer-hook,dozer-mcp}` 里搜裸 `tracing::(error|warn|info|debug|trace)!` 与 `eprintln!`(带 `// cli-output` 标记的行豁免),命中即失败。用于 clippy 对宏展开判断有漏洞时兜底(见「风险」)。

### 6. 来源命名表

面板(`kind = panel`)——与 `PanelKind` 一一对应,`PanelKind` 新增 `pub fn log_name(self) -> &'static str` 作为单一真相,测试断言它与表一致:

| `PanelKind` | 来源名 | 主要代码位置 |
|---|---|---|
| `Files` | `files` | `extensions/files/*` |
| `GitLog` | `git_log` | `extensions/git_log.rs` |
| `Todo` | `todo` | `extensions/todo/*` |
| `Project` | `project` | `extensions/project/*`、`extensions/project.rs` |
| `Database` | `database` | `extensions/database/*` |
| `Ssh` | `ssh` | `extensions/ssh/*`、`extensions/ssh.rs` |
| `Web` | `web` | `extensions/browser.rs` |
| `Agent` | `agent` | `extensions/agent_context.rs`(Agent 面板下方的上下文条)及 agent 面板视图 |
| `Conversations` | `conversations` | `extensions/conversations.rs` |
| `Usage` | `usage` | `extensions/usage/*` |
| `CodeHealth` | `code_health` | `extensions/codehealth/*` |

非面板(`kind = module`),`dozer-app`:

| 来源名 | 代码 |
|---|---|
| `shell` | `app/*`、`workspace/*`(内核、布局、项目页签) |
| `platform` | `platform/*`、`chrome/native_menu.rs`(overlay、窗口、原生菜单/拖拽钩子) |
| `preview` | `preview/*`、`tabular/*`(文件预览与 webview 宿主) |
| `runtime` | `runtime.rs`(启动、daemon 拉起、webview IPC 分发) |
| `term` | `term/*` |
| `search`、`file_history`、`edit_history`、`project_create`、`settings`、`footbar`、`toast` | 同名 extension(弹窗类,不在 `PanelKind` 里) |

`dozer-hook` → `hook`,`dozer-mcp` → `mcp`(均为 `module`,走 §4b 纯 std 写入,不经 `tracing`)。

`agent_context` **合并进 `Agent` 面板的来源 `agent`**(已决):它是 Agent 面板下方的一条,不单独设来源。

非面板(`kind = module`),`dozerd`:按源文件模块命名,如 `server`、`session`、`registry`、`ide_bridge`、`summary`(`summary_*`)、`todo`(daemon 侧)、`memory`。具体划分在 dozerd 迁移时按模块落定,规则同上:一个模块一个来源常量。注意 dozerd 的 `todo` 与 GUI 的面板 `todo` 是不同进程的不同日志文件,`target` 的 `kind` 不同(`module` vs `panel`),不会混淆。

### 7. 级别规范

迁移时统一按下表重定级,不只是机械替换:

| 级别 | 用途 | 例 |
|---|---|---|
| `error` | 用户发起的操作失败、数据未落盘 | 保存/写盘失败、打开项目失败 |
| `warn` | 已自动降级、功能仍可用 | git_watch 降级为手动刷新、可恢复的重试 |
| `info` | 生命周期与关键状态迁移 | 启动横幅、项目打开/关闭、daemon 拉起 |
| `debug` | 诊断 | 新增的排查输出 |
| `trace` | 逐事件级 | 鼠标/键盘事件流 |

**遗留的 `[DIAG]`/`DEBUG` 调试日志直接删除**(已决),不降级保留。审计时定位到的有:`platform/window.rs` 的 `[DIAG] mouseDownCanMoveWindow called…`、`platform/window_events.rs` 的 `[DIAG] TOPBAR_CONTROL_HOVERED changed…`、`[DIAG] left MouseInput state=…`、`DEBUG term input fallback fired`(约 4 处,均在鼠标/输入热路径上)。迁移时以 `rg -n "\[DIAG\]|DEBUG " crates` 再扫一遍确认没有遗漏,删除时保留其外层的逻辑不变(这几条是纯输出语句,不含副作用,已读代码确认前不要假设——删之前逐条看一眼上下文)。

## 与 Toast 的关系

两者互补:**Toast 面向用户,日志面向排查**。Toast 8 秒后消失,不留痕迹,而且固定几何会裁剪长文本。**已决:每条 Toast 自动写一条日志**,不靠各调用点自觉——这样 Toast 上"打开项目失败"消失后,日志里仍能查到完整原因与来源面板。

**接口变化**(相对 `2026-09-30-unified-toast-design.md`/计划里的现状 `push_toast(level, text)`):所有入口多一个 `Scope` 参数,放在第一位,与日志宏的约定一致:

```rust
App::push_toast(scope: Scope, level: toast::Level, text: impl AsRef<str>)
App::push_toast_keyed(scope: Scope, level: toast::Level, text: impl AsRef<str>, key: &str)
toast::Message::Push { scope: Scope, level: Level, text: String, key: Option<String> }
```

`Scope` 是 `Copy + Debug`,放进 `Message` 不影响其 `Debug, Clone` 派生。没有旧签名的兼容垫片:只有少数调用点(Toast 第一份计划里的打开项目失败、删除项目未完全成功、`agent_context` 的取走点),改签名后让编译器逐个报错即可,比留两套接口更不易漏。第二份 Toast 迁移计划里引入的 `Outbox` 条目同样带 `Scope`(用各 extension 自己模块的 `LOG` 常量)。

**写日志的位置——单一咽喉点**:`ToastCenter` 保持纯逻辑、不写日志(单测不依赖 tracing)。日志由 `App` 里唯一的一个私有函数 `App::emit_toast(scope, level, text, key)` 写,`push_toast`/`push_toast_keyed`/`Message::Toast` 的处理分支都走它,内部先写日志、再调 `toast::update`。这样无论 Toast 从哪条路径进来,都必然写日志,也只写一次。

**日志内容与级别**:

- 级别映射:`Info`/`Success` → `info`,`Warning` → `warn`,`Error` → `error`。
- 写**原始文本**(未经 `normalize` 折叠空白、未被固定卡片高度裁剪的完整文本),这正是日志相对 Toast 多出来的价值。
- ~~带结构化字段 `toast = true`~~ **(未采用)**:Toast 日志的 target 固定为 `dozer::module::toast`,调用方来源在字段 `scope` 里,见「实施结果与偏差」第 3 条。
- 同 `key` 的重复推送**每次都写日志**:Toast 去重是为了不刷屏,日志是历史记录,重复发生本身就是信息(例如重试连续失败了几次)。
- 文本归一化后为空的推送(`ToastCenter::push` 会忽略)**不写日志**——`emit_toast` 在写日志前用同一个判空规则,避免空日志行。

测试:把"级别映射 + 写日志"抽成一个不依赖 `App` 的纯函数 `log_toast(scope, level, text)`(`App` 没有便宜的单测夹具),用测试 subscriber 捕获输出,断言四个级别各自的日志级别、`toast=true` 字段、target 为传入来源、以及多行文本被原样保留。

## 迁移

按存量与风险分批,每批独立提交、可单独合并:

1. **批 0:`dozer-core::log` + 单测 + 兜底脚本**(含 `clippy.toml`,但先不启用,避免存量报错)。
1b. **批 0b:Toast 接入日志**(见「与 Toast 的关系」):`push_toast`/`push_toast_keyed`/`toast::Message::Push` 加 `Scope` 参数,新增 `App::emit_toast` 与 `log_toast`,改掉现有几处调用点。**前置条件**:批 0 已合并(需要 `Scope` 与日志宏),且 `2026-09-30-unified-toast.md`(Toast 第一份计划)已合并——那份计划正在开发,使用的是无 `Scope` 的旧签名,本批不应在它合并之前动它的文件。本批改动小(几处调用点 + 一个函数),应在 Toast 第二份迁移计划**之前**落地,这样第二份计划从一开始就用带 `Scope` 的签名,不用返工。
2. **批 1:`dozerd`**(42 处)。dozerd 已经有日志服务,这批把它的 `init_logging` 换成 `dozer_core::log::init(Component::Daemon)`,并把已有调用改成带来源的宏。行为差异:新增保留策略、来源写入 target。
3. **批 2:`dozer-app` 的初始化 + 外壳/平台/runtime**:`main.rs` 换成 `init(Component::App)`;`shell`/`platform`/`runtime`/`preview`/`term` 五个 module 来源;**删除** `[DIAG]`/`DEBUG` 遗留输出。
4. **批 3:面板与弹窗 extension**:按 11 个面板与 7 个弹窗类 extension 逐个加 `scope!` 并迁移调用。这一批与 Toast 第二份迁移计划改动的文件大量重叠(`files`、`todo`、`ssh`、`database`),**两份计划的执行顺序要协调**,避免同一文件被两个分支同时改。
5. **批 4:`dozer-hook`/`dozer-mcp`**:落地 §4b 的纯 std 写入,把运行期诊断的 `eprintln!` 换成 `plain_*!`;`dozer-mcp` 的 CLI 用户输出保留并局部豁免。
6. **批 5:启用 `clippy.toml` 与兜底脚本**,确认全绿后作为门禁。放最后,是因为四个 crate 都迁完才不会有存量报错。

## 测试

单测(`dozer-core`,均在 `logging` feature 下):

- `scope!` 生成的 `target` 恰为 `dozer::panel::<名>`/`dozer::module::<名>`;含非法字符的名字编译失败(用 `compile_fail` 文档测试或 `trybuild` 之外更简单的 `const` 断言 + 一个运行时校验函数的单测覆盖字符集规则)。
- 日志行写明来源:用测试 writer 装一个 subscriber,`log_warn!(LOG, ...)` 后断言输出包含 `dozer::panel::todo`。
- 按来源过滤:filter 为 `info,dozer::panel::files=debug` 时,`files` 的 `debug` 输出、`todo` 的 `debug` 不输出。
- `init_at` 写到临时目录、文件名带组件前缀、内容含横幅与来源。
- 保留策略:预置一个 15 天前和一个 1 天前的同前缀文件,`init_at` 后前者被删、后者保留;非同前缀文件不被误删;四种组件前缀(`dozer-app`/`dozerd`/`dozer-hook`/`dozer-mcp`)都被清理。
- `plain_write`(不需要 `logging` feature 也能编译):写入的行包含来源 target 与级别;连续写两次是追加而不是覆盖;日志目录不存在时自动创建;目录不可写时静默返回不 panic。
- UTC 时间换算纯函数:用几个已知时间戳断言(如 `0` → `1970-01-01T00:00:00`、闰年 `2024-02-29`、跨年 `2025-12-31T23:59:59` → 下一秒 `2026-01-01T00:00:00`),这是自己实现日期算术,必须覆盖闰年与月/年边界。
- 并发追加:起若干线程同时对同一文件 `plain_write`,断言行数等于总写入数且每行完整(无交错)。
- 日志目录不可创建时不 panic,降级到仅 stderr。

`dozer-app`:

- `PanelKind::log_name()` 覆盖全部变体、互不重复、符合字符集(用一个手写穷举 `match` 保证新增变体时编译报错)。

手工:

- 从 Finder/`open` 启动 `.app`,确认 `logs_dir()` 下出现 `dozer-app.log.<日期>`,并含面板来源;`RUST_LOG=info,dozer::panel::todo=debug` 启动后只有 todo 的 debug 出现。
- 触发一次 panic(测试用),确认日志里有 panic 记录。

## 风险与待验证

1. **clippy `disallowed_macros` 对"宏内部展开出的被禁用宏"是否误报。** `log_warn!` 展开成 `tracing::warn!`,若 clippy 把展开体当调用点会全部误报。这是批 0 的 spike:先在最小样例上验证;若误报且无法用 `#[allow]` 在展开点局部豁免,则放弃 clippy 这一层,只保留兜底脚本作为门禁。
2. **`tracing` 的 `target:` 是否接受 `const` 结构体字段访问。** `tracing` 把 target 写进 `static` callsite 元数据,要求是常量表达式。`LOG.target`(`const` 项的字段)按 Rust 规则应该可以,但需要在批 0 编译验证;若不行,退路是让 `scope!` 同时生成一个独立的 `const LOG_TARGET: &str`,宏改用它。
3. **每模块一个 `LOG` 常量会重名**:同一 crate 多个模块各自 `const LOG` 没问题(模块作用域隔离),但 `use super::*` 会在子模块里遮蔽/冲突,需要在迁移时留意。
4. **热路径日志量**:迁移不应给每帧/每事件路径新增日志;`debug`/`trace` 级别的宏在被过滤时开销很小,但格式化参数仍可能被求值——`tracing` 对此是惰性的,无需担心,但迁移时不要把昂贵计算写进日志参数。
5. **与并发会话的冲突**:批 3 触及几乎所有 extension 文件,而主 checkout 长期有并发提交。必须在独立分支/worktree 上做,合并前 rebase 并重跑全量构建(本仓库历史上多次出现合并"无冲突"但编译失败)。

## 已决事项(2026-09-30 评审)

1. **`dozer-hook`/`dozer-mcp` 纳入**:走 §4b 的纯 std 追加写,不依赖 `tracing`,不写 stderr/stdout;CLI 用户输出(如 `dozer-mcp install` 的结果、用法提示)不是日志,保留 `eprintln!`。
2. **日志格式**:纯文本,沿用 `tracing` 默认 fmt,不做 JSON。
3. **保留策略**:只按天数(14 天),**不设**单文件或总大小上限。
4. **遗留 `[DIAG]`/`DEBUG` 调试日志**:直接删除,不降级保留。
5. **`Agent` 面板与 `agent_context`**:合并为同一个来源 `agent`。
6. **`push_toast` 自动写日志**:所有 Toast 入口加 `Scope` 参数,由 `App::emit_toast` 单一咽喉点写日志(原始文本、`toast=true` 字段、同 key 重复推送每次都写),不靠调用点自觉。落地为迁移批 0b。

## 未决项(不擅自定死)

1. **批 3(面板迁移)与 Toast 第二份迁移计划的执行顺序**:两者改动的文件大量重叠,谁先谁后、是否合并成一条分支,开工前需要协调。批 0b 已经为此定了一个前置约束(Toast 第二份计划之前落地),但批 3 与 Toast 第二份计划之间的先后仍待定。
