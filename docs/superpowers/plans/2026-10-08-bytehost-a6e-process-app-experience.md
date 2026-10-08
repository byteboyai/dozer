# bytehost A6e:进程型应用的使用体验与端到端验收 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 A6a–d 做出来的进程型应用从"后端能跑"推到"用户能用、出问题时看得懂":① 清单里的 `node`/`python` 版本要求真的被校验;② 运行时缺失/版本不符时,应用自己的面板里由 host 画一张带「去设置安装」「重试」的提示页(规格 §6.3);③ 应用崩溃/起不来时能看到日志末尾;④ 运行中进程卡死(活着但不应答)会被周期性健康检查发现并按重启策略处理;⑤ 用两个真实的示例应用(Python、Node,都含 SSE 与 WebSocket 与持久化数据)把 A6 全系列第一次真正端到端跑通,并留下验收报告。

**Architecture:** 后端:`runtime::version`(版本号与要求的纯解析/比较)+ `VersionProbe`(试跑 `--version`)接进 `start_process_locked`;`AppManager` 在内存里记一份"当前问题"(`AppIssue`)随 `AppSummary` 回给 GUI;`AppRequest::Logs` 读日志末尾(有界、清洗控制字符);监管线程的盯守循环加周期探测。前端:`app_host` 纯状态机多两个视图态(运行时问题页、日志展开),`view.rs` 画出来,「去设置安装」直达设置→应用页。验收:`crates/dozerd/tests/process_apps_live.rs`(`#[ignore]`,真 python3/node、真 gateway)+ 手工 GUI 清单。

**Tech Stack:** Rust;示例应用只用 Python/Node 标准库(无依赖,不需要 lockfile);不新增 crate。

**Spec:** `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md` §4.4、§6.2、§6.3、验收 8;前置计划 `…-a6c-process-apps-wiring.md`、`…-a6d-runtime-install.md`。

## Global Constraints

- **版本要求语法固定且严格**:逗号分隔的比较子,每个 = 运算符(`>=`、`>`、`<=`、`<`、`=`)+ `X[.Y[.Z]]`,如 `>=3.12, <4`;**没有运算符的裸版本号、空串、未知运算符一律在清单校验时拒绝**(不猜)。缺省的 minor/patch 在 `>=`/`>`/`<=`/`<` 里按 0 处理;`=3.12` 是前缀匹配(任意 3.12.x)。
- **版本检查只对能确定解释器的情况做**:Node 应用(`node`/`npm`/`npx` 开头)检查解析到的 `node`;Python 应用 `python`/`python3` 开头检查解析到的那个;`uv` 开头的 Python 应用(`uv run …` 由 uv 选 Python)**不检查**,在安装计划的 `will_run` 里如实写"版本要求不适用于 uv 管理的解释器"。试跑 `--version` 超时 3s,输出不信任(只按固定格式解析数字),解析不出就按"不满足"处理并在原因里写明原始输出的第一行(截断 80 字符)。
- **问题页是 host 的页面,不是应用 origin 的页面**(规格 §6.3):由 iced 画在应用面板里(webview 此时不显示);**持久状态不进 Toast**(CLAUDE.md "瞬时消息统一走 Toast"的反面:"当前处于某状态"留在原位);动作(去设置、重试)经 `app_host` 消息回到 `AppService`/设置窗口。
- **日志读取有界且已清洗**:只读该应用 `logs/app.log`(不够再补 `app.log.1`);最多 500 行、256 KiB(取末尾);按有损 UTF-8 解码;**剥掉 ANSI 转义序列与除 `\n`、`\t` 外的控制字符**;`max_lines` 由服务端夹到 `1..=500`;应用 id 经 `AppId` 校验,不接受路径。日志里可能有应用打印的敏感内容——它只在本机、只给 GUI 看,**不写 dozerd 日志、不广播事件**。
- **日志展示用系统默认字体**(CLAUDE.md:只有 code editor 与 pty 终端用 JetBrains Mono)。
- **健康检查不能误杀**:只在 `Running` 之后周期探测;**连续** N 次(默认 3)失败才算卡死;探测单次超时 2s、间隔 15s(`Launch` 字段,测试里可调小);任一次成功清零计数;卡死按"进程退出"同样走 `RestartTracker`(退避、放弃)。
- **线上协议只往后加**:`AppSummary.issue` 是 `#[serde(default, skip_serializing_if = "Option::is_none")]`;`RuntimeReason::Unsatisfied` 追加在枚举末尾;新请求/应答变体各有 serde 往返测试,旧形状 JSON 仍能解析。
- **问题状态不持久化**:`AppIssue` 只存内存(dozerd 重启后由 `reconcile` 重新走一遍 `start` 得出),不写 `state.json`。
- 沿用 A6 的全部约束:监管线程回写状态持锁 + 核对取消标志 + 尝试拿锁;静态应用行为不变;`scripts/check-bytehost-apps-deps.sh`、`scripts/check-log-scope.sh` 通过;`cargo clippy -p bytehost-apps -p dozerd -p dozer-client -p dozer-app --all-targets` 无新警告。

## Review Focus

- **版本比较的边界**:`24.21.0` vs `>=24`、`<25`、`=24.21`、`3.9` vs `>=3.12`、`3.13.16` vs `<4`、`10.0` vs `>=9`(字符串比较会错成 `10 < 9`)——表驱动测试钉住(Task 1)。
- **畸形要求**:`""`、`"3.12"`、`">=3..1"`、`">= "`、`">=3.12,"`、`"~=3.12"`、超长数字(`>=99999999999999999999`)——清单校验给出可读错误,不 panic(Task 1)。
- **畸形 `--version` 输出**:空、乱码、只有 `Python`、带 `rc1`/`+local` 后缀(`3.14.0rc1`、`3.13.1+`)——解析取前三段数字,其余忽略;完全解析不出 → "不满足"(Task 1、2)。
- **日志里的转义序列/超长单行/非 UTF-8/二进制**:不得破坏界面(剥离转义与控制字符;单行超 4 KiB 截断并标 `…`)(Task 3)。
- **日志不存在/应用没装/应用 id 非法**:分别返回空文本、`NotFound`、`Rejected`,不 panic(Task 3)。
- **健康检查的误杀**:慢但最终应答的应用(单次 1.5s)不被判卡死;连续失败后恢复(第 2 次起又成功)计数清零;停止/取消期间不再探测(Task 4)。
- **问题页状态陈旧**:装好运行时后点「重试」→ 问题清除、应用启动;应用被 `stop`/卸载 → 问题清除(Task 2、5)。
- **"去设置安装"直达**:设置窗口已开时切到应用页并刷新探测;未开时打开(Task 5)。

## 文件结构

- 新增 `crates/bytehost-apps/src/runtime_version.rs` — 版本号/要求(纯函数,默认 feature 可用)
- 新增 `crates/bytehost-apps/src/logs.rs` — 日志末尾读取与清洗(仅 `server` feature)
- 新增 `crates/dozerd/tests/process_apps_live.rs`、`scripts/bytehost/samples/{py-notes,node-notes}/…`
- 新增 `docs/superpowers/specs/2026-10-08-bytehost-a6e-acceptance-report.md`
- 修改 `bytehost-apps`:`manifest.rs`(校验要求语法)、`plan.rs`(`will_run`)、`proto.rs`、`event.rs`、`manager.rs`、`supervisor.rs`、`runtime/mod.rs`(`VersionProbe`)、`lib.rs`
- 修改 `dozer-client`(`app_logs`)、`dozerd/src/app_service.rs`、`dozer-app` 的 `extensions/app_host.rs`、`app/{view,update,message}.rs`、`extensions/settings.rs`
- 修改 `CLAUDE.md`、规格 §7 表(A6e 一行)、A5 验收报告里过时的测试名引用

---

### Task 1: 版本号与版本要求(纯函数)+ 清单校验

**Files:** Create `crates/bytehost-apps/src/runtime_version.rs`(放在 crate 顶层而不是 `runtime/`:`manifest.rs` 的校验在默认 feature 下,而 `runtime/` 属 `server` feature);Modify `lib.rs`(`pub mod runtime_version;`)、`manifest.rs`(校验)、`plan.rs`(`will_run`)。

**Interfaces:**
- Produces:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
  pub struct Triple(pub u64, pub u64, pub u64);
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct VersionReq { /* Vec<(Op, partial)> */ }
  impl VersionReq {
      pub fn parse(text: &str) -> Result<Self, String>;     // 错误文案给人看,含出错的片段
      pub fn matches(&self, v: Triple) -> bool;
  }
  impl std::fmt::Display for VersionReq {}                   // 规范化回显:`>=3.12, <4`
  /// 从 `node --version`("v24.21.0")、`python3 --version`("Python 3.13.16")等输出里取前三段数字;其余后缀忽略。
  pub fn parse_version_output(output: &str) -> Option<Triple>;
  ```

- [ ] **Step 1: 写失败测试**(`runtime_version.rs` 的 `mod tests`)

```rust
fn t(a: u64, b: u64, c: u64) -> Triple { Triple(a, b, c) }
fn req(s: &str) -> VersionReq { VersionReq::parse(s).unwrap() }

#[test]
fn comparison_is_numeric_not_textual() {
    assert!(req(">=9").matches(t(10, 0, 0)), "10 不能因字符串比较小于 9");
    assert!(req(">=3.12").matches(t(3, 13, 16)));
    assert!(!req(">=3.12").matches(t(3, 9, 0)));
    assert!(req("<4").matches(t(3, 13, 16)));
    assert!(!req("<4").matches(t(4, 0, 0)));
    assert!(req(">=24, <25").matches(t(24, 21, 0)));
    assert!(!req(">=24, <25").matches(t(25, 0, 0)));
    assert!(req(">3.12").matches(t(3, 12, 1)));
    assert!(!req(">3.12").matches(t(3, 12, 0)));
    assert!(req("<=3.12").matches(t(3, 12, 0)));
}

#[test]
fn an_equals_requirement_is_a_prefix_match_on_the_parts_given() {
    assert!(req("=24.21").matches(t(24, 21, 0)));
    assert!(req("=24.21").matches(t(24, 21, 7)));
    assert!(!req("=24.21").matches(t(24, 22, 0)));
    assert!(req("=24").matches(t(24, 5, 5)));
    assert!(req("=24.21.0").matches(t(24, 21, 0)));
    assert!(!req("=24.21.0").matches(t(24, 21, 1)));
}

#[test]
fn malformed_requirements_are_refused_with_the_offending_piece() {
    for bad in ["", "  ", "3.12", ">=", ">= ", ">=3..1", ">=3.12,", ",>=3", "~=3.12", "^3", ">=a.b", ">=1.2.3.4", ">=99999999999999999999", ">=3.12 <4"] {
        let e = VersionReq::parse(bad).unwrap_err();
        assert!(!e.is_empty(), "{bad:?}");
    }
}

#[test]
fn requirements_display_in_a_normalized_form_that_parses_back() {
    let r = req("  >=3.12 ,<4  ");
    assert_eq!(r.to_string(), ">=3.12, <4");
    assert_eq!(VersionReq::parse(&r.to_string()).unwrap(), r);
}

#[test]
fn version_output_takes_the_first_three_numeric_parts_and_ignores_suffixes() {
    assert_eq!(parse_version_output("v24.21.0\n"), Some(t(24, 21, 0)));
    assert_eq!(parse_version_output("Python 3.13.16"), Some(t(3, 13, 16)));
    assert_eq!(parse_version_output("Python 3.14.0rc1"), Some(t(3, 14, 0)));
    assert_eq!(parse_version_output("Python 3.13.1+"), Some(t(3, 13, 1)));
    assert_eq!(parse_version_output("v22.1"), Some(t(22, 1, 0)));
    for bad in ["", "Python", "garbage \u{1b}[31m", "v.1.2"] {
        assert_eq!(parse_version_output(bad), None, "{bad:?}");
    }
}
```
  `manifest.rs` 测试:`node = ">=22"` / `python = ">=3.12, <4"` 合法;`python = "3.12"` 校验失败且错误指出 `runtime.python`;`node = ""` 失败。`plan.rs` 测试:声明了要求时 `will_run` 含一行 `需要 node >=22`;`uv` 开头的 Python 应用(命令 `["uv","run","a.py"]`)声明要求时,`will_run` 含"版本要求不适用于 uv 管理的解释器"。
- [ ] **Step 2: 确认失败 → Step 3: 实现**(解析:按 `,` 切;每段 trim;匹配最长的运算符前缀 `>=`/`<=`/`>`/`<`/`=`;版本部分按 `.` 切 1–3 段,每段 `u64::from_str`(溢出即错);`parse_version_output` 扫描第一个"数字开头的 token",取前三段数字(遇到非数字截止),至少 1 段才算)。`Manifest::validate` 对 `Node{node: Some(r)}`/`Python{python: Some(r)}` 调 `VersionReq::parse`,错误进 `problems`(格式 `runtime.node: <原因>`)。`will_run` 追加上述两种文案。
- [ ] **Step 4: 通过** — `cargo test -p bytehost-apps runtime_version manifest plan`(默认 feature 与 `--all-features` 各跑一遍,确认默认 feature 依赖不变:`scripts/check-bytehost-apps-deps.sh`)。
- [ ] **Step 5: Commit** — `feat(bytehost-apps): runtime version requirements — strict grammar, numeric comparison, manifest validation (A6e task 1)`。

---

### Task 2: 启动时校验版本 + `AppIssue` 随列表回给 GUI

**Files:** Modify `proto.rs`、`event.rs`、`manager.rs`(含 `Core`)、`runtime/mod.rs`(`VersionProbe`)。

**Interfaces:**
- Consumes:Task 1 的 `VersionReq`/`Triple`/`parse_version_output`;A6c 的 `start_process_locked`/`fail_runtime_missing`/`Resolved`;`runtime::{CommandRunner, SystemRunner}`。
- Produces:
  ```rust
  // proto.rs
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(tag = "issue", rename_all = "snake_case")]
  pub enum AppIssue {
      RuntimeMissing { runtime: String },                              // "node" / "python3" / "uv" …(解释器名)
      RuntimeVersion { runtime: String, required: String, found: String },
  }
  // AppSummary 新增: #[serde(default, skip_serializing_if = "Option::is_none")] pub issue: Option<AppIssue>,
  // event.rs: RuntimeReason 末尾追加 Unsatisfied { runtime: String, required: String, found: String }
  // runtime/mod.rs
  pub trait VersionProbe: Send + Sync { fn version_output(&self, program: &std::path::Path) -> Option<String>; }
  pub struct SystemVersionProbe;   // SystemRunner{timeout: 3s}.run(program, ["--version"]);成功取 stdout,为空取 stderr(老 python 打到 stderr)
  // manager.rs Core 新增字段: version_probe: Arc<dyn VersionProbe>, issues: Mutex<HashMap<AppId, AppIssue>>
  // 构造: with_resolver_and_policy 多一个 version_probe 参数;pub AppManager::new/with_resolver 默认 SystemVersionProbe;
  //       #[cfg(test)] with_parts_for_test(root, host_version, gateway, resolver, policy, version_probe)
  ```
  行为:
  - `start_process_locked` 在解析出 `resolved` 之后、组 `Launch` 之前:若清单声明了要求且该应用属于"能检查"的情况(见 Global Constraints),取 `version_probe.version_output(<被检查解释器的绝对路径>)`,`parse_version_output`;不满足 → `observed = Failed{reason: "需要 node >=22,当前 20.1.0", retryable: true}`、`emit(RuntimeUnavailable{ reason: Unsatisfied{..} })`、记 `AppIssue::RuntimeVersion`、返回新的 `ManagerError::RuntimeVersion { runtime, required, found }`(线上类别 `Unavailable`)。解析不出 → `found = "无法识别: <输出首行,≤80 字符>"`,同样走失败。
  - Node 应用即使 `argv[0]` 是 `npm`/`npx`,也用 `resolver.resolve("node")` 取 `node` 来检查;解析不到 `node` → 走原有 `RuntimeUnavailable("node")`。
  - `fail_runtime_missing` 同时记 `AppIssue::RuntimeMissing`。
  - 清除时机(在锁内):`start_process_locked` 开头清掉旧问题;`stop_locked`、`uninstall` 清掉;监管线程 `ready()` 成功清掉(防御)。
  - `list()` 把 `issues` 填进 `AppSummary.issue`(只在 `observed` 是 `Failed` 时带出,避免陈旧问题挂在健康应用上)。

- [ ] **Step 1: 写失败测试**
  1. `proto.rs`:`AppIssue` 两个变体与带 `issue` 的 `AppSummary` 的 serde 往返;**旧 JSON(无 `issue`)仍解析为 `issue: None`**;`RuntimeReason::Unsatisfied` 往返。
  2. `manager.rs`(用 `with_parts_for_test` + 假 `VersionProbe`(按程序路径返回预设输出)+ 假 `SystemResolver::with_dirs`):
     - `a_node_app_whose_node_is_too_old_fails_to_start_with_a_clear_reason_and_an_issue`:要求 `>=22`、假输出 `v20.1.0` → `Err(RuntimeVersion{..})`;`list()` 里 `observed = Failed{retryable:true}` 且 `issue == Some(RuntimeVersion{runtime:"node", required:">=22", found:"20.1.0"})`;事件含 `RuntimeUnavailable{Unsatisfied}`;**没有起任何进程/线程**(用假解析器指向会写标记文件的脚本,断言文件不存在)。
     - `an_unparseable_version_output_counts_as_unsatisfied_and_is_truncated`(输出 200 个字符的乱码 → `found` 以 `无法识别:` 开头且 ≤ 90 字符)。
     - `an_npm_command_is_checked_against_node_not_npm`:`command = ["npm","start"]`,假探针只对 `node` 路径给输出。
     - `a_uv_python_app_skips_the_version_check`:`command = ["uv","run","a.py"]` 且声明 `python = ">=99"`,假探针记录调用次数 = 0,应用正常进入 `Starting`。
     - `a_satisfied_requirement_starts_normally`(`v24.21.0` vs `>=22, <25`)。
     - `a_missing_runtime_records_a_runtime_missing_issue_and_stop_clears_it`:`list()` 带 `RuntimeMissing`;`stop` 之后 `issue == None`。
     - `an_issue_is_only_reported_while_the_app_is_failed`:手工把 `issues` 塞一条、应用 `observed = Running` → `list()` 的 `issue == None`。
     - `retrying_after_the_runtime_appears_clears_the_issue`:第一次解析器无 python → 失败;换成有 python 的解析器(`RuntimeResolver` 用 `Mutex<Vec<PathBuf>>` 的可变假实现)→ `start` 成功,`issue == None`。
- [ ] **Step 2: 确认失败 → Step 3: 实现 → Step 4: 通过**(`cargo test -p bytehost-apps --all-features`,e2e 里已有的 python 测试不得变红)。
- [ ] **Step 5: Commit** — `feat(bytehost-apps): enforce manifest runtime versions at start; report app issues on the list (A6e task 2)`。

---

### Task 3: 日志末尾读取(`AppRequest::Logs`)

**Files:** Create `crates/bytehost-apps/src/logs.rs`;Modify `lib.rs`(`#[cfg(feature = "server")] pub mod logs;`)、`proto.rs`、`manager.rs`(`logs(&self, id, max_lines)`)、`dozer-client`、`dozerd/src/app_service.rs`。

**Interfaces:**
- Produces:
  ```rust
  // logs.rs
  pub const MAX_LINES: usize = 500;
  pub const MAX_BYTES: usize = 256 * 1024;
  pub const MAX_LINE_CHARS: usize = 4096;
  pub struct Tail { pub text: String, pub truncated: bool }
  /// 读 `<logs_dir>/app.log`,不够再前补 `app.log.1`;取末尾 `max_lines`(夹到 1..=MAX_LINES)与 MAX_BYTES。
  pub fn read_tail(logs_dir: &Path, max_lines: usize) -> io::Result<Tail>;
  /// 剥 ANSI 转义(CSI/OSC)与除 `\n`、`\t` 外的控制字符;单行超 MAX_LINE_CHARS 截断并以 `…` 结尾。
  pub fn sanitize(raw: &str) -> String;
  // proto.rs
  AppRequest::Logs { id: AppId, max_lines: u32 }   AppReply::Logs { text: String, truncated: bool }
  // manager: pub(crate) fn logs(&self, id: &AppId, max_lines: u32) -> Result<(String, bool), ManagerError>   // 未安装 → NotInstalled
  // dozer-client: pub async fn app_logs(&self, id: AppId, max_lines: u32) -> Result<(String, bool)>
  ```

- [ ] **Step 1: 写失败测试**(`logs.rs`):
  1. `the_tail_returns_only_the_last_lines_and_marks_truncation`(1000 行取 50 → 恰好后 50 行、`truncated`)。
  2. `a_short_current_log_is_topped_up_from_the_rotated_one`(`app.log` 3 行、`app.log.1` 100 行,请求 10 行 → 7 行来自 `.1` 的末尾 + 3 行,顺序正确)。
  3. `a_missing_log_is_empty_not_an_error`、`max_lines_is_clamped`(0 → 1;100000 → 500)。
  4. `byte_cap_wins_over_line_cap`(500 行每行 2 KiB → 总字节 ≤ 256 KiB,且首行不是被腰斩的半行以外的乱码——允许首行被截,但整体合法 UTF-8)。
  5. `sanitize_strips_ansi_osc_and_control_characters_but_keeps_text`:`"\u{1b}[31mred\u{1b}[0m"`→`red`;`"\u{1b}]0;title\u{7}x"`→`x`;`"a\u{0}b\u{8}c\r\n"`→`abc\n`(`\r` 也剥);`\t` 保留;中文保留。
  6. `an_overlong_line_is_cut_and_marked`、`invalid_utf8_is_replaced_not_fatal`(`b"ok\xff\xfe"`)。
  7. `manager.rs`:`logs` 对未安装应用 `NotInstalled`;对已装但从未跑过的应用返回空文本;`dozerd` 层:`failures_are_classified…` 里加 `Logs` 对非法 id 的 `Rejected`(`AppId::new` 失败映射)。
  8. serde 往返:`Logs` 请求/应答。
- [ ] **Step 2: 确认失败 → Step 3: 实现**(读尾部用"从文件末尾向前按块读到够行数或字节数"的做法,**不要整文件读入**——日志可达 4 × 1 MiB 也一样要按块;`sanitize` 手写状态机,不引正则依赖)。
- [ ] **Step 4: 通过 → Step 5: Commit** — `feat(bytehost-apps): bounded, sanitized log tail over the wire (A6e task 3)`。

---

### Task 4: 运行中周期健康检查

**Files:** Modify `crates/bytehost-apps/src/supervisor.rs`(`Launch` 加字段、盯守循环)、`manager.rs`(默认值)。

**Interfaces:**
- Produces:`Launch` 新增 `pub monitor_interval: Duration`、`pub monitor_timeout: Duration`、`pub monitor_failures: u32`;常量 `DEFAULT_MONITOR_INTERVAL = 15s`、`DEFAULT_MONITOR_TIMEOUT = 2s`、`DEFAULT_MONITOR_FAILURES = 3`。盯守循环(`Healthy` 之后)除 200ms 的退出/取消检查外,每过 `monitor_interval` 调一次 `probe_http(port, health_path, monitor_timeout)`;`Unhealthy` 则 `consecutive += 1` 否则清零;`consecutive >= monitor_failures` → `running.stop(grace)`,`reason = "运行中健康检查连续失败 N 次"`,走既有 `RestartTracker`。`monitor_failures == 0` 表示关闭检查。

- [ ] **Step 1: 写失败测试**(`supervisor.rs` 的 `mod tests`,python3 缺失则 `return`;监视参数调小:interval 150ms、timeout 300ms、failures 3):
  1. `a_process_that_stops_answering_is_killed_and_restarted`:`server.py` 前 3 次请求(含启动期探测)正常应答,之后 `time.sleep(3600)`(单线程服务器 → 后续探测超时)。期望序列含 `ready`,随后 `down … 运行中健康检查连续失败`(`Some(restart)`),再 `Starting`、`ready`(新进程重新应答);旧进程已被收掉(脚本把 `os.getpid()` 写文件,测试断言旧 pid 不存在)。
  2. `a_slow_but_answering_app_is_not_killed`:每次应答 sleep 100ms(< timeout 300ms),观察 1.5s,序列里没有 `down`。
  3. `a_single_failed_probe_does_not_count_after_a_success`:应答模式"成功,失败,成功,失败…"交替(失败 = 本次 sleep 超过 timeout 再恢复;用一个计数器按请求序号决定),观察 2s 无 `down`(连续失败从不达到 3)。
  4. `disabling_the_monitor_keeps_the_old_behaviour`(`monitor_failures = 0`:卡死的应用不被杀,观察 1s)。
  5. `cancelling_during_monitoring_stops_without_further_probes`(`cancel_on = "ready"` 的既有用例仍通过,且取消后 `run` 在 `grace+1s` 内返回)。
  所有既有 `Launch` 构造(supervisor 测试辅助、`manager.rs`)补新字段;`manager.rs` 用默认常量。
- [ ] **Step 2: 确认失败 → Step 3: 实现 → Step 4: 通过**(连跑 3 遍:`for i in 1 2 3; do cargo test -p bytehost-apps --all-features supervisor:: || break; done`)。
- [ ] **Step 5: Commit** — `feat(bytehost-apps): periodic health monitoring — consecutive failures restart a hung app (A6e task 4)`。

---

### Task 5: GUI——运行时问题页、日志展开、「去设置安装」

**Files:** Modify `crates/dozer-app/src/extensions/app_host.rs`(纯状态机)、`crates/dozer-app/src/app/{view.rs,update.rs,message.rs}`、`crates/dozer-app/src/extensions/settings.rs`(`State::load_on(tab)` 或 `SettingsTab` 预选)、`crates/dozer-client`(已有 `app_logs`)。

**Interfaces:**
- Consumes:Task 2 的 `AppSummary.issue`/`AppIssue`;Task 3 的 `app_logs`。
- Produces(`app_host.rs`):
  ```rust
  // PanelView 新增
  RuntimeIssue { issue: AppIssue },                       // 取代对应场景下的 Crashed(reason)
  // Crashed 保持 Crashed(String),但 view_model 额外暴露:
  pub fn logs_view(&self, slot: AppSlot) -> LogsView;     // Hidden | Loading | Loaded{text, truncated} | Failed(String)
  // Message 新增
  ShowLogs(AppSlot), HideLogs(AppSlot), LogsLoaded(AppSlot, Result<(String, bool), Failure>), OpenRuntimeSettings
  // Effect 新增
  FetchLogs(AppSlot, u32 /*max_lines: 200*/), OpenSettingsApps
  ```
  行为:
  - `view_model`:`row.observed` 是 `Failed` 且 `row.issue.is_some()` → `PanelView::RuntimeIssue`(优先于 `Crashed`);其余不变。`Busy/Start/Stop` 的 `acting` 优先级不变。
  - `ShowLogs(slot)` → 该 slot 的日志状态置 `Loading`,`Effect::FetchLogs(slot, 200)`;`LogsLoaded` 写入;`HideLogs` 回 `Hidden`;**应用状态离开 `Failed`(被重启/运行)或 `ListLoaded` 里该应用的 `observed` 变化时,日志状态复位为 `Hidden`**(防陈旧)。同一 slot 在途时重复 `ShowLogs` 不重复发请求。
  - `OpenRuntimeSettings` → `Effect::OpenSettingsApps`;`App` 执行:`self.settings = Some(settings::State::load_with_tab(daemon_unavailable, SettingsTab::Apps))` 并触发 `settings_apps::Message::Opened`(已有 `opened_apps` 逻辑,见 `settings.rs:366`),若设置窗口已开则只切 tab + 刷新。
- 视图(`view.rs::app_panel_pane`):
  - `RuntimeIssue`:标题"需要 {运行时显示名}"(`RuntimeMissing{runtime}` 把解释器名映射成 Node.js/Python/uv);详情一行(缺失:"尚未安装,可在 设置 → 应用 里一键安装";版本:"要求 {required},当前 {found}");按钮「去设置安装」(金色主按钮)与「重试」(`M::Start(slot)`)。
  - `Crashed`:原有「重新启动」旁加「查看日志」/「收起日志」;展开时下方一个可滚动文本区(系统默认字体,`Shaping::Advanced`,保留换行),`truncated` 时顶行灰字"仅显示最后 N 行";`Loading` 显示"读取日志…";`Failed` 显示原因。
  - 不使用 Toast;不新增 `MouseArea`+`on_enter/on_exit` 手写接线(用既有 `button` + `action_button_style`)。

- [ ] **Step 1: 写失败测试**(沿用 `app_host.rs` 现有 fixture `loaded(...)` 风格):
  1. `a_failed_app_with_a_runtime_issue_shows_the_issue_page_not_the_crash_page`;无 issue 的 Failed 仍是 `Crashed`;issue 存在但应用是 `Running` → `Running`(陈旧问题不显示)。
  2. `showing_logs_fetches_once_and_stores_the_result`、`logs_failure_is_kept_in_place_not_toasted`。
  3. `logs_are_hidden_again_when_the_app_leaves_the_failed_state`(`ListLoaded` 里变 `Running`/`Starting`)。
  4. `open_runtime_settings_emits_exactly_one_effect`。
  5. `retry_from_the_issue_page_starts_the_app`(`Start(slot)` 的既有路径,断言 issue 页下的 `acting` 变 `Busy("启动中…")`)。
  6. `view.rs` 没有 App 夹具可测的部分,写一个纯函数 `issue_texts(&AppIssue) -> (String /*标题*/, String /*详情*/)` 并表驱动测试三种输入(缺 node、缺 python3、版本不符)。
- [ ] **Step 2: 确认失败 → Step 3: 实现 → Step 4: 通过**(`cargo test -p dozer-app app_host settings`;`cargo clippy -p dozer-app --all-targets` 无新警告)。
- [ ] **Step 5: 手动验收**(需要 GUI;结果写进提交信息):
  1. 装一个声明 `python = ">=99"` 的 python 应用 → 启动 → 面板出现"需要 Python / 要求 >=99,当前 3.x"页 → 「去设置安装」直接落在设置→应用页。
  2. 临时把 `PATH` 里的 node 藏起来(受管 node 也没装)启动 node 应用 → "需要 Node.js / 尚未安装" → 设置里安装 Node → 回面板「重试」→ 应用起来。
  3. 让应用启动即崩(`exit 1`)→ 放弃重启后 Crashed 页 → 「查看日志」能看到应用的输出末尾 → 「收起日志」。
- [ ] **Step 6: Commit** — `feat(dozer-app): runtime-issue page, log viewer, jump to settings (A6e task 5)`。

---

### Task 6: 真实示例应用与端到端验收

**Files:** Create `scripts/bytehost/samples/py-notes/{manifest.toml,server.py}`、`scripts/bytehost/samples/node-notes/{manifest.toml,server.js}`、`crates/dozerd/tests/process_apps_live.rs`、`docs/superpowers/specs/2026-10-08-bytehost-a6e-acceptance-report.md`。

**两个示例的行为(完全相同,各用自己运行时的标准库实现,无第三方依赖,无 lockfile):**
- 从环境变量(`manifest.toml` 里 `port_env = "PORT"`)取端口,绑 `127.0.0.1`;`BYTEHOST_DATA_DIR/counter.txt` 里存访问计数。
- `GET /` → HTML 页(显示计数与当前运行时版本,含一个 `EventSource('/events')` 与 `WebSocket(ws://…/ws)` 的小脚本;页面本身不依赖外部资源);`POST /hit` → 计数 +1 并返回 JSON(用来测写方法同源检查与持久化);`GET /events` → SSE,每 200ms 发一条 `data: tick N`,共 5 条后保持连接空闲(测流式);`GET /ws` → 手写 RFC 6455 握手,收到文本帧原样回显(测升级与隧道);`GET /pid` → 返回进程号(测重启);`GET /hang` → 此后不再应答任何请求(`time.sleep(3600)` / 死循环式 `Atomics.wait`),用来触发 Task 4;`GET /crash` → `os._exit(3)` / `process.exit(3)`。
- 清单 `[health] path = "/"`,`python = ">=3.9"` / `node = ">=18"`。

**`process_apps_live.rs`**(`#[ignore]`,需要 `python3` 与 `node`——缺一就在该子测试里 `panic!("缺少 python3")` 而不是静默跳过,因为它只在人工要求时运行;用真 `SystemResolver`、真 gateway、真监管线程,`AppService::start_with` + 临时根目录):对两个示例各跑下面这条流程,全部断言:
  1. `Plan` → `Install` → `Start` → 轮询到 `Running`(≤ 20s)。
  2. 带会话 Cookie 经 gateway 请求 `/`:200 且正文含运行时版本;**不带 Cookie 请求 → 403 且应用侧的访问计数不变**。
  3. SSE:读 `/events` 在 1.5s 内收到 ≥ 3 条 `tick`(流式,不是等整个响应结束)。
  4. WebSocket:升级成功,发 `hello` 收到 `hello`;跨源 `Origin` 的升级被 403。
  5. `POST /hit` 带同源 `Origin` → 计数 1;带 `Origin: http://evil.example` → 403;计数不变。
  6. `GET /pid` 记下 pid → 请求 `/crash` → 监管线程重启 → 轮询到新的 pid(≠ 旧)且重新 `Running`(≤ 20s,退避默认 1s)。
  7. 计数仍是 1(`BYTEHOST_DATA_DIR` 持久化,跨进程重启)。
  8. `GET /hang` → 之后最多 `15s × 3 + 10s` 内观察到 pid 变化(健康检查杀掉并重启);测试里用 `AppService` 暴露的 `#[cfg(test)]` 注入把监视间隔调成 300ms,把总等待压到 ≤ 10s(给 `AppService::start_with_policy_for_test` 加监视参数;同时 `RestartPolicy` 调小退避)。
  9. `Stop` → 端口不再应答;`Uninstall(ProgramAndData)` 后目录消失。
  10. 日志:`Logs` 请求返回的文本含示例启动时打印的一行 `listening on`(两个示例启动时都打印这行)。
  11. 版本要求:再装一个声明 `python = ">=99"` 的副本(只改清单)→ `Start` 返回 `Unavailable` 类别失败,`List` 里 `issue == RuntimeVersion{..}`。

**验收报告**(`specs/2026-10-08-bytehost-a6e-acceptance-report.md`,仿 A5 的结构):§1 自动化结果表(上述 11 条逐条给命令与结果,**由实际运行填写**);§2 手工 GUI 清单(**未执行的保持未勾选,并在文件头写明"执行会话无法操作 Dozer 窗口,不得据此标完成"**):
  - [ ] 设置→应用→安装应用…→选 `py-notes`:审批卡显示运行方式 `python`、命令行、"版本要求 >=3.9"、全部权限强制等级为"仅声明,不强制";批准并安装 → 图标栏出现 → 启动 → 面板里页面出现、计数、SSE 滚动、WebSocket 回显。
  - [ ] 同上对 `node-notes`。
  - [ ] 点页面上的「crash」→ 面板短暂变"启动中…"后自动恢复;点「hang」→ 约 45 秒后自动恢复(默认 15s×3)。
  - [ ] 退出整个 Dozer → 重开 → 应用仍在运行(应用跟随 dozerd,GUI 退出不影响)→ 停 dozerd 后端口关闭 → 重启 dozerd 后自动恢复,计数没丢。
  - [ ] 运行时缺失/版本不符的提示页(Task 5 的三步手动验收)。
  - [ ] 把 `server.py` 改成启动即崩的版本重装 → Crashed 页 + 「查看日志」可见输出。
  - [ ] 卸载"保留数据"再装:计数还在;"含数据":计数归零。
  - [ ] 受管运行时:设置里装 Node(A6d 手工项)后,`node-notes` 用的是受管版本(页面显示的 node 版本 = 受管版本)。
  §3 发现的缺陷与已知局限:**执行时如实记录**(例如发现的 bug、修复提交;没有就写"无新增缺陷"——不得预先填写)。

- [ ] **Step 1: 写示例应用与清单**;各自用 `python3 -m py_compile` / `node --check` 做语法检查。
- [ ] **Step 2: 写 `process_apps_live.rs`**(先全部写好再运行);`cargo test -p dozerd --test process_apps_live -- --ignored --nocapture`。**任何失败都要追到根因**:是示例的错、测试的错,还是 A6 的真缺陷——真缺陷在本任务里修(最小修复 + 回归测试),并记入报告 §3。
- [ ] **Step 3: 连跑 3 遍**确认稳定;把结果(各步耗时)填进报告 §1。
- [ ] **Step 4: Commit** — `test(dozerd): real python and node apps end to end through gateway, SSE, WebSocket, restart, hang detection (A6e task 6)`;报告单独一个 commit。

---

### Task 7: 文档收尾

**Files:** Modify `CLAUDE.md`、规格 §7 表、`docs/superpowers/specs/2026-10-05-bytehost-a5-acceptance-report.md`。

- [ ] **Step 1:** `CLAUDE.md` 的 `bytehost-apps` 一行补:A6e 落地——版本要求(`runtime_version`,严格语法,`uv` 管理的解释器不检查)、`AppIssue` 经 `AppSummary.issue` 回给 GUI(仅内存、仅 `Failed` 时带出)、`AppRequest::Logs`(有界、已清洗、不写 dozerd 日志)、运行中周期健康检查(连续失败才重启,默认 15s×3,`monitor_failures = 0` 关闭)、运行时问题页与日志展开在 `extensions/app_host.rs`(持久状态不进 Toast)。
- [ ] **Step 2:** 规格 §7 表加 A6e 一行并链接本计划与验收报告;A5 验收报告 §2 第 8 行里的 `manager::only_static_web_is_supported_in_phase_one` 改为 `manager::containers_are_still_unsupported`(A6c 改过名)。
- [ ] **Step 3: 全量**:`cargo test -p bytehost-apps -p dozerd -p dozer-client -p dozer-app`(已知与本系列无关的失败——`files::tests::delete_confirm_spec_reflects_pending_target`、`memory::tests::list_orders_by_updated_ms_desc`——如仍存在,逐个注明而不要顺手改)、`cargo clippy --all-targets`、两个门禁脚本。
- [ ] **Step 4: Commit** — `docs(bytehost): A6e landed`。

---

## 已知局限(写在这里,不是缺陷)

- **`uv run` 的 Python 版本不校验**:解释器由 uv 在运行时挑选,宿主拿不到。
- **版本检查是启动时一次**:解释器之后被卸载/升级,只会在下次启动时发现。
- **健康检查只看 HTTP 状态行**(与启动期同判据):应用返回 5xx 算失败;"能应答但业务坏了"不在范围。
- **日志是只读快照**:点一次读一次,没有实时 tail;大于 256 KiB 的部分取末尾。
- **问题页只覆盖"运行时缺失/版本不符"**;依赖安装失败(`npm ci` 出错)仍显示为 `Crashed`+日志(原因里带 `安装依赖失败`)。
- **容器运行时仍未实现**;规格验收 8 的"容器不可用提示页"继续用"安装时明确拒绝"代替。
- **没有沙箱**;进程型应用出站网络强制等级仍为 `Advisory`。

## Self-Review(写计划时已核对)

- **范围覆盖**:用户选的五项——版本要求(Task 1–2)、运行时缺失提示页(Task 2 的 `AppIssue` + Task 5)、日志查看(Task 3 + Task 5)、真实应用端到端验收(Task 6)、周期健康检查(Task 4)——各有任务;规格 §6.3"提示页由 host 提供、不由应用 origin 提供、动作回到 host"落在 Task 5 与 Global Constraints。
- **占位符**:无;Task 1/3 给出表驱动测试的具体输入;Task 6 的报告 §1/§3 明确要求**执行时由实际运行填写**,不预填。
- **类型一致**:`AppIssue`/`RuntimeReason::Unsatisfied`/`VersionReq`/`Triple`/`VersionProbe`/`Launch.monitor_*`/`PanelView::RuntimeIssue`/`LogsView`/`Effect::FetchLogs`/`Effect::OpenSettingsApps` 在定义它们的任务里给出签名,后续任务按同名使用;`runtime_version` 放在 crate 顶层(默认 feature 可用)而不是 `runtime/`,因为 `manifest.rs` 的校验在默认 feature 下。
