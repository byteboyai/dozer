# dozer-services (X0, Usage 试点) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新建只依赖 `dozer-model` 的 crate `dozer-services`,把 Usage 面板的取数从具体 `dozer_client::Client` 换成 `UsageService` trait 接缝,并让"取数失败"与"无数据"可区分。

**Architecture:** `dozer-services` 定义 `BoxFuture`、`ServiceError`(三类)、`UsageService`、`Services`(各 trait 的 `Arc<dyn>` 束);`dozer-client` 为 `Client` 实现 `UsageService`(`roundtrip` 的错误改为带类型的 `ServiceError`,`Display` 文本不变);`PanelIo` 持有 `Services`(默认由 `Client` 适配,测试可替换);Usage 的 `spawn_refresh` 吃 `Arc<dyn UsageService>`,失败走新消息 `LoadFailed` → 面板 state 的 Toast `Outbox`。

**Tech Stack:** Rust 2024 workspace、tokio、serde;Python 3 审计脚本(`scripts/audit`)。

**Spec:** `docs/superpowers/specs/2026-10-09-dozer-services-x0-design.md`

## Global Constraints

- 在独立 worktree/分支 `feat/dozer-services`(基于 `spec/dozer-services`)实施,**不在 main 上直接做**。
- 绝不提交用户 WIP:`Info.plist`、版本号相关的 `Cargo.lock`/`crates/dozer-app/Cargo.toml` 改动。worktree 里 cargo 会给 `Cargo.lock` 里 bytegit/byteui 加 `source = "git+…"` 行——每次提交前用 `perl -ni -e 'print unless /^source = "git\+https:\/\/github.com\/byteboyai\/(bytegit|byteui)\?/' Cargo.lock` 剔除;`Cargo.lock` 只带 `dozer-services` 相关块(新 package 与依赖边)。
- `dozer-services` 的 `[dependencies]` 只有 `dozer-model`;不得依赖 `dozer-core`/`dozer-client`/`tokio`/`bytehost-*`(`tokio` 仅允许 dev-dependency)。
- 线上协议零变化:`Request`/`Reply` 不改;`crates/dozer-core/tests/model_golden.rs` 不改。
- main 上已有、**不修**:`extensions::files::tests::delete_confirm_spec_reflects_pending_target` 失败;`R-HOST-EXECUTORS 0 -> 1` 门禁失败;`usage/aggregate.rs:311` 与 `git_accounts.rs:18` 的 dead-code 警告。
- 瞬时失败提示统一走 Toast(`extensions::toast::Outbox`,经 `drain_outboxes` 排空),面板日志来源名必须是 `usage`(`dozer_core::scope!(LOG, panel, "usage")`);不得新增裸 `tracing::*!`/`eprintln!`。
- 提交消息以 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` 结尾。
- 全量测试:`cargo test --workspace --no-fail-fast > <file> 2>&1`,读尾部,不把长输出留在上下文里。

## Review Focus

- `roundtrip` 的错误类型改动不得改变任何已有错误的 `Display` 文本(`"daemon 断开"`、`"daemon 错误: …"`),否则依赖字符串的调用方与测试会静默变化 → Task 3 钉死。
- 取数失败时**不得清空已有的 rows**(失败不应抹掉上次成功的数据),且过期代数的 `LoadFailed` 必须被丢弃 → Task 5 测试。
- git 提交数是独立数据源:dozerd 失败时它仍要更新,面板仍要退出"统计中" → Task 5/6 测试。
- `Arc<dyn UsageService>` 的 future 要 `Send + 'static` 地在 `handle.spawn` 里用得起来 → Task 2 契约测试用 `tokio::spawn` 钉住。
- 夹具 `usage_probe` 的 `host_site` 行号会因 `update.rs` 增行而漂移 → Task 5 末尾必须跑 `host_site_citations_point_at_the_code_they_name` 并校正。

---

### Task 1: S0 盘点与基线

**Files:**
- Create: `scripts/audit/client_uses.py`, `scripts/audit/test_client_uses.py`, `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`

**Interfaces:**
- Produces: `client_uses.py` 产出"文件 → `Client` 方法"表(能识别换行书写的 `client\n    .method(`);ledger 里的基线数字。

- [ ] **Step 1: 记录基线(只测量)**

```bash
cd <worktree>
grep -rl 'dozer_client' crates/dozer-app/src/extensions | wc -l
cargo test --workspace --no-fail-fast > <ws>/baseline-tests.txt 2>&1; grep -E '^test result' <ws>/baseline-tests.txt | awk '{s+=$4;f+=$6} END{print "passed",s,"failed",f}'
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
```
Expected: 测试有且仅有 `delete_confirm_spec_reflects_pending_target` 一个失败;门禁仅 `R-HOST-EXECUTORS 0 -> 1`;把四个数字写进 ledger。

- [ ] **Step 2: 写失败测试**

`scripts/audit/test_client_uses.py`:
```python
import importlib.util, os, unittest
HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("cu", os.path.join(HERE, "client_uses.py"))
cu = importlib.util.module_from_spec(spec); spec.loader.exec_module(cu)

class Scan(unittest.TestCase):
    NAMES = {"get_usage_summary", "list_todos"}

    def test_same_line_and_multiline_calls_are_found(self):
        text = "let a = client.list_todos(1).await;\nlet b = client\n    .get_usage_summary(&cwd, None)\n    .await;\n"
        self.assertEqual(cu.methods_used(text, self.NAMES), {"list_todos", "get_usage_summary"})

    def test_comments_and_strings_are_ignored(self):
        text = '// client.list_todos(1)\nlet s = "client.get_usage_summary(";\n'
        self.assertEqual(cu.methods_used(text, self.NAMES), set())

    def test_unknown_names_are_ignored(self):
        self.assertEqual(cu.methods_used("x.other(1)", self.NAMES), set())

if __name__ == "__main__":
    unittest.main()
```
Run: `python3 scripts/audit/test_client_uses.py` → Expected: FAIL/ERROR(`client_uses.py` 不存在)。

- [ ] **Step 3: 实现**

`scripts/audit/client_uses.py`(复用 `edges.py` 的 `strip_comments`、`SRC`):
```python
#!/usr/bin/env python3
"""S0 盘点:各文件对 dozer_client::Client 公开方法的使用(识别换行书写的 `client\n .method(`)。
用法: client_uses.py   → 输出 markdown 表(文件 | 方法)。"""
import importlib.util, os, re, sys
HERE = os.path.dirname(os.path.abspath(__file__))
_s = importlib.util.spec_from_file_location("edges", os.path.join(HERE, "edges.py"))
edges = importlib.util.module_from_spec(_s); _s.loader.exec_module(edges)
STRING_RE = re.compile(r'"(?:\\.|[^"\\])*"')
CLIENT_LIB = os.path.join(HERE, "..", "..", "crates", "dozer-client", "src", "lib.rs")

def client_methods():
    with open(CLIENT_LIB, encoding="utf-8") as f:
        return set(re.findall(r"pub (?:async )?fn ([a-z_0-9]+)", f.read()))

def methods_used(text, names):
    text = STRING_RE.sub('""', edges.strip_comments(text))
    return {m for m in re.findall(r"\.\s*([a-z_0-9]+)\s*\(", text) if m in names}

def main():
    names = client_methods()
    print("| 文件 | 方法 |\n|---|---|")
    for dp, _, fns in sorted(os.walk(edges.SRC)):
        for fn in sorted(fns):
            if not fn.endswith(".rs"):
                continue
            p = os.path.join(dp, fn)
            with open(p, encoding="utf-8") as f:
                used = methods_used(f.read(), names) - {"clone"}
            if used:
                print(f"| `{os.path.relpath(p, edges.SRC)}` | {', '.join(sorted(used))} |")

if __name__ == "__main__":
    main()
```
Run: `python3 scripts/audit/test_client_uses.py` → Expected: PASS(3 tests)。

- [ ] **Step 4: 产出盘点文档并校对规格 §4**

`python3 scripts/audit/client_uses.py > /tmp/inv.md`;写 `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`:标题、日期、"方法由 `scripts/audit/client_uses.py` 生成"、粘贴 /tmp/inv.md,再加一节"能力分组(对照规格 §4)":把输出里的方法归入 §4 各 trait;**规格 §4 表与盘点不符之处,以盘点为准,在文档里写差异**(不改规格,不改本轮范围)。注意:`extensions/**` 之外(`app/`、`workspace/`、`runtime.rs`、`panel_host.rs`)的使用也要列在"host 侧"小节。

- [ ] **Step 5: 提交**

```bash
git add scripts/audit/client_uses.py scripts/audit/test_client_uses.py docs/dozer-v2/bytehost-H0/08-client-use-inventory.md
git commit -m "docs(services): X0 S0 面板对 Client 方法的使用盘点 + 基线

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: S1 `dozer-services` crate

**Files:**
- Create: `crates/dozer-services/Cargo.toml`, `crates/dozer-services/src/lib.rs`
- Modify: `Cargo.lock`(仅新 package 块)

**Interfaces:**
- Consumes: `dozer_model::{ConversationSummary, UsagePayload}`。
- Produces(后续任务依赖的精确签名):
  - `pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;`
  - `pub enum ServiceErrorKind { Unavailable, Rejected, Protocol }`(`Debug, Clone, Copy, PartialEq, Eq`)
  - `pub struct ServiceError { pub kind: ServiceErrorKind, pub message: String }`(`Debug, Clone, PartialEq, Eq`;`ServiceError::new(kind, message: impl Into<String>)`;`Display` = `message`;`impl std::error::Error`)
  - `pub type UsageRows = Vec<(ConversationSummary, UsagePayload)>;`
  - `pub trait UsageService: Send + Sync { fn usage_summary(&self, cwd: String, since_ts: Option<u64>) -> BoxFuture<'_, Result<UsageRows, ServiceError>>; }`
  - `#[derive(Clone)] pub struct Services { pub usage: Arc<dyn UsageService> }`

- [ ] **Step 1: 骨架 + 失败测试**

`crates/dozer-services/Cargo.toml`:
```toml
[package]
name = "dozer-services"
version = "0.1.0"
edition.workspace = true

# 面板服务 trait:只许依赖 dozer-model(spec 2026-10-09)。不要加 dozer-core / dozer-client / tokio。
[dependencies]
dozer-model = { path = "../dozer-model" }

[dev-dependencies]
tokio = { workspace = true }
```
`src/lib.rs` 先只写测试(模块 `tests`,引用尚未定义的 `ServiceError` 等,使编译失败):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Mem(Result<UsageRows, ServiceError>);
    impl UsageService for Mem {
        fn usage_summary(&self, _cwd: String, _since: Option<u64>) -> BoxFuture<'_, Result<UsageRows, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
    }

    #[test]
    fn display_is_the_message_only() {
        let e = ServiceError::new(ServiceErrorKind::Rejected, "daemon 错误: boom");
        assert_eq!(e.to_string(), "daemon 错误: boom");
        assert_eq!(e.kind, ServiceErrorKind::Rejected);
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn UsageService>` 的 async 块必须满足。
    #[test]
    fn dyn_service_future_runs_inside_tokio_spawn() {
        let svc: Arc<dyn UsageService> = Arc::new(Mem(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let out = rt.block_on(async move {
            tokio::spawn(async move { svc.usage_summary("/p".into(), None).await })
                .await
                .unwrap()
        });
        assert_eq!(out, Ok(vec![]));
    }

    #[test]
    fn failure_kinds_are_distinguishable_from_empty_success() {
        let ok: Result<UsageRows, ServiceError> = Ok(vec![]);
        let un: Result<UsageRows, ServiceError> = Err(ServiceError::new(ServiceErrorKind::Unavailable, "x"));
        assert!(ok.is_ok() && un.is_err());
        assert_ne!(ServiceErrorKind::Unavailable, ServiceErrorKind::Protocol);
    }

    #[test]
    fn services_is_cloneable_and_shares_the_service() {
        let s = Services { usage: Arc::new(Mem(Ok(vec![]))) };
        let t = s.clone();
        assert!(Arc::ptr_eq(&s.usage, &t.usage));
    }
}
```
Run: `cargo test -p dozer-services 2>&1 | tail -15` → Expected: 编译失败 `cannot find type ServiceError`(RED)。

- [ ] **Step 2: 实现**

在 `lib.rs` 测试模块之前写:
```rust
//! 面板服务 trait 接缝(X0,spec 2026-10-09)。面板只依赖这里的小 trait 和 `dozer-model`,
//! 不知道对面是 dozerd 还是内存假实现。生产实现在 `dozer-client`(为 `Client` 适配)。

use dozer_model::{ConversationSummary, UsagePayload};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// trait 方法返回的装箱 future(`async fn in trait` 不能做 trait object)。`Send` 以便在
/// `tokio::spawn`/`Handle::spawn` 里使用。
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceErrorKind {
    /// 连不上或连接中途断开。
    Unavailable,
    /// 服务端明确拒绝(dozerd 回了 `Reply::Error`)。
    Rejected,
    /// 意外应答或解码失败。
    Protocol,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceError {
    pub kind: ServiceErrorKind,
    pub message: String,
}

impl ServiceError {
    pub fn new(kind: ServiceErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ServiceError {}

pub type UsageRows = Vec<(ConversationSummary, UsagePayload)>;

/// Usage 面板取数。参数按值传,future 因此不借用调用方的局部变量。
pub trait UsageService: Send + Sync {
    fn usage_summary(
        &self,
        cwd: String,
        since_ts: Option<u64>,
    ) -> BoxFuture<'_, Result<UsageRows, ServiceError>>;
}

/// 各能力 trait 的 `Arc<dyn>` 束;host 构造,经 `PanelIo` 交给面板。其余能力 trait 随各面板迁移逐个加入。
#[derive(Clone)]
pub struct Services {
    pub usage: Arc<dyn UsageService>,
}
```
Run: `cargo test -p dozer-services` → Expected: 4 passed。

- [ ] **Step 3: 依赖验收与提交**

```bash
cargo tree -p dozer-services --edges normal      # 只有 dozer-model 与 serde 系
cargo tree -p dozer-hook --edges normal | grep -c dozer-services   # 期望 0
cargo fmt -p dozer-services && cargo clippy -p dozer-services --all-targets 2>&1 | grep -c '^warning'   # 0
perl -ni -e 'print unless /^source = "git\+https:\/\/github.com\/byteboyai\/(bytegit|byteui)\?/' Cargo.lock
git add crates/dozer-services Cargo.lock && git status --short
git commit -m "feat(services): X0 S1 新建 dozer-services(ServiceError/BoxFuture/UsageService/Services)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: S2 `Client` 实现 `UsageService`(带类型的错误)

**Files:**
- Modify: `crates/dozer-client/Cargo.toml`, `crates/dozer-client/src/lib.rs`(`roundtrip` 与新 impl)、`Cargo.lock`
- Create: `crates/dozer-app/src/usage_probe/client_adapter.rs`;Modify: `crates/dozer-app/src/usage_probe/mod.rs`(加 `mod client_adapter;`)

**Interfaces:**
- Consumes: Task 2 的 `ServiceError`、`ServiceErrorKind`、`UsageService`、`BoxFuture`、`UsageRows`。
- Produces: `impl dozer_services::UsageService for dozer_client::Client`;`pub fn dozer_client::into_service_error(e: anyhow::Error) -> ServiceError`(规则:携带 `ServiceError` 则原样;携带 `std::io::Error` → `Unavailable`;其余 → `Protocol`)。`roundtrip` 的错误 `Display` 文本不变。

- [ ] **Step 1: 写失败测试(分类单测 + FakeDozerd 适配测试)**

(a) `crates/dozer-client/src/lib.rs` 末尾(若无 `#[cfg(test)] mod tests` 则新建一个独立的 `mod service_error_tests`):
```rust
#[cfg(test)]
mod service_error_tests {
    use super::*;
    use dozer_services::{ServiceError, ServiceErrorKind};

    #[test]
    fn io_errors_are_unavailable() {
        let e = anyhow::Error::from(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused"));
        assert_eq!(into_service_error(e).kind, ServiceErrorKind::Unavailable);
    }

    #[test]
    fn typed_service_errors_pass_through_with_the_same_text() {
        let e = anyhow::Error::new(ServiceError::new(ServiceErrorKind::Rejected, "daemon 错误: boom"));
        let s = into_service_error(e);
        assert_eq!((s.kind, s.message.as_str()), (ServiceErrorKind::Rejected, "daemon 错误: boom"));
    }

    #[test]
    fn anything_else_is_protocol() {
        let s = into_service_error(anyhow::anyhow!("意外应答: Ok"));
        assert_eq!(s.kind, ServiceErrorKind::Protocol);
        assert_eq!(s.message, "意外应答: Ok");
    }
}
```
(b) `crates/dozer-app/src/usage_probe/client_adapter.rs`(先读 `fake_dozerd.rs`/`scenarios.rs` 的 `runtime()`、`repo`、`TIMEOUT` 等辅助是否 `pub(crate)`,按需复用或在本文件内就地写):
```rust
//! `Client` 作为 `UsageService` 的适配行为:用 `FakeDozerd` 讲真实协议,覆盖成功 / 断开 / 被拒。
use super::fake_dozerd::{Behavior, FakeDozerd};
use dozer_services::{ServiceErrorKind, UsageService};

fn call(behavior: Behavior) -> Result<usize, (ServiceErrorKind, String)> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let dozerd = FakeDozerd::start(&rt, behavior);
    let client = dozer_client::Client::new(dozerd.socket().to_path_buf());
    rt.block_on(client.usage_summary("/p".into(), None))
        .map(|rows| rows.len())
        .map_err(|e| (e.kind, e.message))
}

#[test]
fn rows_come_back_as_ok() {
    assert_eq!(call(Behavior::Rows(vec![])), Ok(0));
}

#[test]
fn hang_up_is_unavailable_with_the_legacy_text() {
    let (kind, msg) = call(Behavior::HangUp).unwrap_err();
    assert_eq!(kind, ServiceErrorKind::Unavailable);
    assert!(msg.contains("daemon 断开") || !msg.is_empty());
}

#[test]
fn error_reply_is_rejected_with_the_legacy_prefix() {
    let (kind, msg) = call(Behavior::ErrorReply("boom".into())).unwrap_err();
    assert_eq!(kind, ServiceErrorKind::Rejected);
    assert_eq!(msg, "daemon 错误: boom");
}

#[test]
fn nonexistent_socket_is_unavailable() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let client = dozer_client::Client::new("/tmp/dozer-x0-no-such.sock".into());
    let e = rt.block_on(client.usage_summary("/p".into(), None)).unwrap_err();
    assert_eq!(e.kind, ServiceErrorKind::Unavailable);
}
```
`mod.rs` 加 `mod client_adapter;`。`dozer-client/Cargo.toml` 与 `dozer-app/Cargo.toml` 各加 `dozer-services = { path = "../dozer-services" }`(dozer-app 的 Cargo.toml 只提交这一行,不带版本号改动)。

Run: `cargo test -p dozer-client service_error_tests 2>&1 | tail` 与 `cargo test -p dozer-app usage_probe::client_adapter 2>&1 | tail` → Expected: 编译失败(`into_service_error`/`UsageService for Client` 不存在)(RED)。

- [ ] **Step 2: 实现**

`crates/dozer-client/src/lib.rs`:
1. 文件头 `use` 加 `use dozer_services::{BoxFuture, ServiceError, ServiceErrorKind, UsageRows, UsageService};`。
2. `roundtrip` 里两处:
```rust
// 原: .ok_or_else(|| anyhow!("daemon 断开"))?;
.ok_or_else(|| anyhow::Error::new(ServiceError::new(ServiceErrorKind::Unavailable, "daemon 断开")))?;
```
```rust
// 原: bail!("daemon 错误: {message}");
return Err(anyhow::Error::new(ServiceError::new(
    ServiceErrorKind::Rejected,
    format!("daemon 错误: {message}"),
)));
```
3. 加分类函数与 impl:
```rust
/// 把 `Client` 内部的 `anyhow::Error` 归类成面板可区分的 `ServiceError`:带类型的原样返回,
/// IO 错误(连不上/读写断开)归 `Unavailable`,其余(意外应答、解码失败)归 `Protocol`。
pub fn into_service_error(e: anyhow::Error) -> ServiceError {
    match e.downcast::<ServiceError>() {
        Ok(se) => se,
        Err(e) if e.downcast_ref::<std::io::Error>().is_some() => {
            ServiceError::new(ServiceErrorKind::Unavailable, e.to_string())
        }
        Err(e) => ServiceError::new(ServiceErrorKind::Protocol, e.to_string()),
    }
}

impl UsageService for Client {
    fn usage_summary(
        &self,
        cwd: String,
        since_ts: Option<u64>,
    ) -> BoxFuture<'_, Result<UsageRows, ServiceError>> {
        Box::pin(async move {
            self.get_usage_summary(&cwd, since_ts)
                .await
                .map_err(into_service_error)
        })
    }
}
```
若 `bail!`/`anyhow!` 在改后不再使用导致 `unused import`,按编译器提示收窄 `use`。

- [ ] **Step 3: 验证**

```bash
cargo test -p dozer-client 2>&1 | grep -E 'test result|FAILED'      # 原有 + 3 个新测试全绿
cargo test -p dozer-app usage_probe::client_adapter 2>&1 | grep -E 'test result|FAILED'   # 4 passed
cargo test --workspace --no-fail-fast > <ws>/t3.txt 2>&1; grep -E '^test result' <ws>/t3.txt | awk '{s+=$4;f+=$6} END{print s,f}'
```
Expected: 比基线多 7 个通过,失败仍仅 `delete_confirm_spec_reflects_pending_target`。`hang_up_is_unavailable_with_the_legacy_text` 里的宽松断言如果 `HangUp` 实际返回的是 `daemon 断开`,把断言收紧为 `assert_eq!(msg, "daemon 断开")`;若实际是 IO 文本(读到 EOF 前连接被重置),保留宽松断言并在 ledger 记一条 `Ruling`。

- [ ] **Step 4: 提交**

```bash
cargo fmt -p dozer-client -p dozer-app
perl -ni -e 'print unless /^source = "git\+https:\/\/github.com\/byteboyai\/(bytegit|byteui)\?/' Cargo.lock
git add crates/dozer-client crates/dozer-app/src/usage_probe Cargo.lock
git add -p crates/dozer-app/Cargo.toml   # 只选 dozer-services 依赖行
git commit -m "feat(client): X0 S2 Client 实现 UsageService,roundtrip 错误带类型(Display 不变)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: S3 `PanelIo` 带 `Services` + 进程内 `FakeUsage`

**Files:**
- Modify: `crates/dozer-app/src/panel_host.rs`(`PanelIo` 字段、`new`、`Clone`、`services()`、`with_services()`)
- Create: `crates/dozer-app/src/usage_probe/fake_usage.rs`;Modify: `usage_probe/mod.rs`(`pub(crate) mod fake_usage;`)

**Interfaces:**
- Consumes: Task 2/3。
- Produces:
  - `PanelIo::services(&self) -> &Services`;`PanelIo::with_services(self, services: Services) -> Self`(`PanelIo::new` 签名不变,默认 `Services { usage: Arc::new(client.clone()) }`)。
  - `usage_probe::fake_usage::FakeUsage`:`FakeUsage::ok(rows: UsageRows) -> Self`、`FakeUsage::failing(kind: ServiceErrorKind, message: &str) -> Self`、`FakeUsage::calls(&self) -> Vec<(String, Option<u64>)>`、`FakeUsage::into_services(self) -> Services`(`Clone`,克隆共享同一份调用记录);实现 `UsageService`。

- [ ] **Step 1: 写 `FakeUsage` 与失败测试**

`fake_usage.rs`:
```rust
//! 进程内 `UsageService` 假实现:不起 UDS,返回固定结果并记录调用。
use dozer_services::{BoxFuture, ServiceError, ServiceErrorKind, Services, UsageRows, UsageService};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(crate) struct FakeUsage {
    reply: Result<UsageRows, ServiceError>,
    calls: Arc<Mutex<Vec<(String, Option<u64>)>>>,
}

impl FakeUsage {
    pub(crate) fn ok(rows: UsageRows) -> Self {
        Self { reply: Ok(rows), calls: Default::default() }
    }
    pub(crate) fn failing(kind: ServiceErrorKind, message: &str) -> Self {
        Self { reply: Err(ServiceError::new(kind, message)), calls: Default::default() }
    }
    pub(crate) fn calls(&self) -> Vec<(String, Option<u64>)> {
        self.calls.lock().unwrap().clone()
    }
    pub(crate) fn into_services(self) -> Services {
        Services { usage: Arc::new(self) }
    }
}

impl UsageService for FakeUsage {
    fn usage_summary(&self, cwd: String, since_ts: Option<u64>) -> BoxFuture<'_, Result<UsageRows, ServiceError>> {
        self.calls.lock().unwrap().push((cwd, since_ts));
        let r = self.reply.clone();
        Box::pin(async move { r })
    }
}
```
在 `panel_host.rs` 的测试模块(靠近第 396 行附近已有的 `PanelIo` 测试夹具处,读那段再就近放)加:
```rust
#[test]
fn with_services_replaces_the_default_client_adapter() {
    use crate::usage_probe::fake_usage::FakeUsage;
    let rt = tokio::runtime::Runtime::new().unwrap();
    let client = Client::new(PathBuf::from("/tmp/dozer-x0-no-such.sock"));
    let io = PanelIo::<u32>::new(client, rt.handle().clone(), |_| {});
    // 默认服务指向不存在的 socket → Unavailable
    let default_err = rt.block_on(io.services().usage.usage_summary("/p".into(), None)).unwrap_err();
    assert_eq!(default_err.kind, dozer_services::ServiceErrorKind::Unavailable);
    // 替换后走假实现
    let fake = FakeUsage::ok(vec![]);
    let io = io.with_services(fake.clone().into_services());
    assert_eq!(rt.block_on(io.services().usage.usage_summary("/p".into(), Some(3))), Ok(vec![]));
    assert_eq!(fake.calls(), vec![("/p".to_string(), Some(3))]);
    // clone 之后仍用替换后的服务
    let again = io.clone();
    rt.block_on(again.services().usage.usage_summary("/q".into(), None)).unwrap();
    assert_eq!(fake.calls().len(), 2);
}
```
Run: `cargo test -p dozer-app with_services_replaces 2>&1 | tail` → Expected: 编译失败(`services`/`with_services` 不存在)(RED)。

- [ ] **Step 2: 实现**

`PanelIo<M>` 加字段 `services: Services`;`new` 里 `services: Services { usage: Arc::new(client.clone()) }`(`client` 随后移入 `Self`,先 clone);`Clone` 手写实现里加 `services: self.services.clone()`;加:
```rust
/// host 给面板的服务束(默认由 `Client` 适配;测试可用 `with_services` 换成进程内假实现)。
pub(crate) fn services(&self) -> &Services { &self.services }

pub(crate) fn with_services(mut self, services: Services) -> Self {
    self.services = services;
    self
}
```
`use dozer_services::Services;`。**保留** `client()`、`spawn`。

- [ ] **Step 3: 验证并提交**

```bash
cargo test -p dozer-app with_services_replaces 2>&1 | grep -E 'test result|FAILED'
cargo test -p dozer-app panel_host 2>&1 | grep -E 'test result|FAILED'
cargo build --workspace 2>&1 | grep -E '^error'
cargo fmt -p dozer-app
git add crates/dozer-app/src/panel_host.rs crates/dozer-app/src/usage_probe
git commit -m "feat(host): X0 S3 PanelIo 带 Services(默认 Client 适配,可替换)+ FakeUsage

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: 全绿、无编译错误(`FakeUsage::failing`/`calls` 在 Task 5 才被用到,若出现 `dead_code` 警告,在本任务给它们加 `#[allow(dead_code)]`,Task 5 末尾去掉)。

---

### Task 5: S4 Usage 切换到 `UsageService`,失败可见

**Files:**
- Modify: `crates/dozer-app/src/extensions/usage/mod.rs`(`Message::LoadFailed`、`WorkspaceState.outbox`/`take_outbox`、`update`、`spawn_refresh`、`on_activate`、`LOG` 声明)
- Modify: `crates/dozer-app/src/app/update.rs`(`LoadFailed` 路由臂;`drain_outboxes` 加 `ws.usage.take_outbox()`)
- Modify: `crates/dozer-app/src/usage_probe/host.rs`(`Duty::DrainToastOutbox`、`deliver` 处理 `LoadFailed`、`activate_with`、`drain_toasts`;校正 `host_site` 行号与引用表)

**Interfaces:**
- Consumes: Task 4 的 `PanelIo::services()`、`FakeUsage`;Task 2 的 `UsageService`、`ServiceError`。
- Produces:
  - `usage::Message::LoadFailed(i64 /*project_id*/, u64 /*generation*/, String /*message*/, u64 /*git_commits*/, BTreeMap<i64, u64> /*git_commits_by_day*/)`。
  - `usage::spawn_refresh(project_id: i64, generation: u64, project_path: PathBuf, service: Arc<dyn UsageService>, handle: &Handle, emit: impl Fn(Message) + Send + 'static)`。
  - `WorkspaceState::take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending>`。

- [ ] **Step 1: 写失败测试(面板 `update` 层)**

先读 `usage/mod.rs` 现有 `mod tests` 里的 `rows_of(n)` 助手(约 1017 行附近)并复用。在该测试模块加:
```rust
#[test]
fn load_failed_keeps_rows_updates_git_and_queues_one_error_toast() {
    let mut ws = WorkspaceState::default();
    let g1 = ws.begin_refresh();
    update(&mut ws, Message::Loaded(1, g1, rows_of(2), 5, Default::default()));
    let g2 = ws.begin_refresh();
    update(&mut ws, Message::LoadFailed(1, g2, "daemon 断开".into(), 9, Default::default()));
    assert_eq!(ws.rows().len(), 2, "失败不得抹掉上次成功的数据");
    assert_eq!(ws.git_commits(), 9, "git 提交数是独立数据源,照常更新");
    assert!(!ws.loading(), "失败后要退出“统计中”");
    let toasts = ws.take_outbox();
    assert_eq!(toasts.len(), 1);
    assert_eq!(toasts[0].level, crate::extensions::toast::Level::Error);
    assert!(toasts[0].text.contains("daemon 断开"));
    assert_eq!(toasts[0].key.as_deref(), Some("usage-load"));
    assert!(ws.take_outbox().is_empty(), "排空后不重复");
}

#[test]
fn stale_load_failed_is_dropped_without_touching_loading() {
    let mut ws = WorkspaceState::default();
    let first = ws.begin_refresh();
    let second = ws.begin_refresh();
    update(&mut ws, Message::LoadFailed(1, first, "旧的失败".into(), 9, Default::default()));
    assert!(ws.loading(), "新的刷新还在路上");
    assert_eq!(ws.git_commits(), 0);
    assert!(ws.take_outbox().is_empty());
    update(&mut ws, Message::LoadFailed(1, second, "新的失败".into(), 1, Default::default()));
    assert!(!ws.loading());
    assert_eq!(ws.take_outbox().len(), 1);
}
```
Run: `cargo test -p dozer-app extensions::usage::tests::load_failed extensions::usage::tests::stale_load_failed 2>&1 | tail` → Expected: 编译失败(`LoadFailed`/`take_outbox` 不存在)(RED)。

- [ ] **Step 2: 面板侧实现**

`usage/mod.rs`:
1. 文件头声明日志来源(首次写日志才声明):`dozer_core::scope!(LOG, panel, "usage");`
2. `WorkspaceState` 加字段 `outbox: crate::extensions::toast::Outbox,`(派生 `Default` 仍成立)与:
```rust
pub fn take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending> {
    self.outbox.take()
}
```
3. `Message` 加(放在 `Loaded` 之后,附文档注释说明"失败时仍带 git 数据:它不依赖 dozerd"):
```rust
LoadFailed(i64, u64, String, u64, BTreeMap<i64, u64>),
```
4. `update` 加臂:
```rust
Message::LoadFailed(_, generation, message, git_commits, git_commits_by_day) => {
    if generation < ws_state.refresh_generation {
        return;
    }
    ws_state.git_commits = git_commits;
    ws_state.git_commits_by_day = git_commits_by_day;
    ws_state.loading = false;
    ws_state.outbox.push_keyed(
        LOG,
        crate::extensions::toast::Level::Error,
        format!("用量统计失败: {message}"),
        "usage-load",
    );
}
```
5. `spawn_refresh`:参数 `client: &dozer_client::Client` 改为 `service: std::sync::Arc<dyn dozer_services::UsageService>`;去掉 `let client = client.clone();`;任务体改为:
```rust
handle.spawn(async move {
    let cwd = project_path.to_string_lossy().into_owned();
    let result = service.usage_summary(cwd, None).await;
    // git 提交数(总数 + 每日分桶)是纯本地仓库遍历,与 dozerd 成败无关,两条路径都要算。
    let (git_commits, git_commits_by_day) = tokio::task::spawn_blocking(move || {
        (count_git_commits(&project_path), count_git_commits_by_day(&project_path))
    })
    .await
    .unwrap_or((0, BTreeMap::new()));
    match result {
        Ok(rows) => {
            let rows = rows
                .iter()
                .map(|(summary, payload)| {
                    (dozer_model::ConversationMeta::from_summary(summary), ConversationUsage::from(payload))
                })
                .collect::<Vec<_>>();
            emit(Message::Loaded(project_id, generation, rows, git_commits, git_commits_by_day));
        }
        Err(e) => emit(Message::LoadFailed(project_id, generation, e.message, git_commits, git_commits_by_day)),
    }
});
```
6. `on_activate`:`ctx.io.client()` 改为 `ctx.io.services().usage.clone()`。
7. 同文件里其它调用 `spawn_refresh(..)` 的测试/代码(`grep -rn 'spawn_refresh' crates/dozer-app/src`)按新签名改;测试里原来构造 `Client` 的地方改用 `FakeUsage::ok(..).into_services().usage`。

Run: `cargo test -p dozer-app extensions::usage 2>&1 | grep -E 'test result|FAILED|^error'` → Expected: 全绿(含 2 个新测试;**先**确认它们是被新增代码变绿的:Step 1 的 RED 已看过)。

- [ ] **Step 3: host 路由与排空(失败测试先行:夹具层)**

在 `usage_probe/host.rs` 加(先让夹具测试失败):`Duty::DrainToastOutbox`;`ProbeHost` 方法
```rust
/// 对应 `App::drain_outboxes`(`app/update.rs`)里 `pending.extend(ws.usage.take_outbox())`。
pub(crate) fn drain_toasts(&mut self) -> Vec<crate::extensions::toast::Pending> {
    self.duties.push(Duty::DrainToastOutbox);
    self.ws.take_outbox()
}

/// 与 `activate` 同,但服务由调用方给(进程内假实现),不经 UDS。
pub(crate) fn activate_with(
    &mut self,
    rt: &tokio::runtime::Runtime,
    services: dozer_services::Services,
    path: Option<PathBuf>,
) -> mpsc::Receiver<Message> {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let client = dozer_client::Client::new(PathBuf::from("/tmp/dozer-probe-unused.sock"));
    let io = PanelIo::new(client, rt.handle().clone(), move |m| {
        let _ = tx.lock().unwrap().send(m);
    })
    .with_services(services);
    let ctx = ActivationCtx { project_id: self.project_id, project_path: path, io };
    self.duties.push(Duty::BuildActivationCtx);
    usage::on_activate(&mut self.ws, Some(&ctx));
    rx
}
```
`deliver` 里把 `Loaded` 的两个分支改成同时覆盖 `LoadFailed`(用 `matches!`/或新增并列分支):
```rust
Message::LoadFailed(project_id, ..) if project_id != self.project_id => self.duties.push(Duty::DropOtherProject),
msg @ Message::LoadFailed(..) => { self.duties.push(Duty::RouteLoaded); usage::update(&mut self.ws, msg); }
```
把 `Duty::DrainToastOutbox` 加进 `host_site`(`"app/update.rs:<行> (drain_outboxes)"`,行号在 Step 4 填)与 `host_site_citations_point_at_the_code_they_name` 的 `table`(needle:`pending.extend(ws.usage.take_outbox())`)。

- [ ] **Step 4: 改真实 host(update.rs)并校正夹具引用**

`app/update.rs`:
1. 紧接 `Message::Usage(msg @ usage::Message::Loaded(project_id, ..)) => { … }` 之后加**独立的臂**(不改既有臂的文本,以免动漂移守卫的精确匹配):
```rust
Message::Usage(msg @ usage::Message::LoadFailed(project_id, ..)) => {
    self.with_project(project_id, move |ws, _io| {
        usage::update(&mut ws.usage, msg);
    });
}
```
2. `drain_outboxes` 里 `pending.extend(ws.agent_context.take_outbox());` 后加 `pending.extend(ws.usage.take_outbox());`。
3. 增行会让夹具 `host_site()` 里 `app/update.rs:` 之后的行号全部漂移:跑 `cargo test -p dozer-app usage_probe::host 2>&1 | grep -E 'FAILED|实际是'`,按失败信息逐个校正 `host_site()` 的行号(含新加的 `DrainToastOutbox`),直到 `host_site_citations_point_at_the_code_they_name` 与两个 `drift_guard_*` 全绿。**不改守卫逻辑,只改行号。**

- [ ] **Step 5: 探针场景:S4 翻转**

读 `scenarios.rs` 现有 S4(`s4_characterizes_g4_service_failure_looks_like_no_data`),**替换**为两个测试:
```rust
/// S4a:进程内服务失败 → 面板退出“统计中”、git 数据照常、恰有一条 Error Toast,而不是静默的“无数据”。
#[test]
fn s4a_service_failure_is_reported_not_swallowed() {
    for kind in [ServiceErrorKind::Unavailable, ServiceErrorKind::Rejected] {
        let rt = runtime();
        let repo = repo_with_one_commit();
        let fake = FakeUsage::failing(kind, "boom");
        let mut host = ProbeHost::new(PROJECT);
        host.surface.attach();
        host.webview_ready();
        let rx = host.activate_with(&rt, fake.clone().into_services(), Some(repo.path().to_path_buf()));
        host.deliver(rx.recv_timeout(TIMEOUT).unwrap());
        assert!(!host.ws.loading());
        assert!(host.ws.rows().is_empty());
        assert_eq!(host.ws.git_commits(), 1);
        let toasts = host.drain_toasts();
        assert_eq!(toasts.len(), 1, "{kind:?}");
        assert!(toasts[0].text.contains("boom"));
        assert_eq!(fake.calls().len(), 1);
    }
}

/// S4b:同一件事端到端——真实 `Client` 对 `FakeDozerd` 的 HangUp / ErrorReply 也走到 Toast。
#[test]
fn s4b_real_client_failures_surface_as_toasts() {
    for behavior in [Behavior::HangUp, Behavior::ErrorReply("boom".into())] {
        let rt = runtime();
        let repo = repo_with_one_commit();
        let dozerd = FakeDozerd::start(&rt, behavior);
        let mut host = ProbeHost::new(PROJECT);
        let rx = host.activate(&rt, &dozerd, Some(repo.path().to_path_buf()));
        host.deliver(rx.recv_timeout(TIMEOUT).unwrap());
        assert!(!host.ws.loading());
        assert_eq!(host.drain_toasts().len(), 1);
    }
}
```
(`FakeUsage`、`ServiceErrorKind` 按需 `use`。)同时检查 `s8_probe_fixture_names_no_other_panel_and_no_app_or_workspace` 与任何断言"duty 数量/集合"的测试(`grep -n 'duty_set\|Duty::' scenarios.rs`),若集合断言需要加入 `DrainToastOutbox` 则同步。去掉 Task 4 里临时加的 `#[allow(dead_code)]`。

- [ ] **Step 6: 验证并提交**

```bash
cargo test -p dozer-app usage 2>&1 | grep -E 'test result|FAILED'
cargo test --workspace --no-fail-fast > <ws>/t5.txt 2>&1; grep -E '^test result' <ws>/t5.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep FAILED <ws>/t5.txt
cargo fmt -p dozer-app && cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py
```
Expected: 失败仍仅 `delete_confirm_spec_reflects_pending_target`;clippy 警告数 ≤ 基线;门禁仅 `R-HOST-EXECUTORS 0 -> 1`;`check-log-scope.sh`(`scripts/check-log-scope.sh`)通过。
```bash
git add crates/dozer-app/src
git commit -m "feat(usage): X0 S4 Usage 取数改用 UsageService,失败走 Toast 而非静默空数据(G4)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: S6 `R-CLIENT` 门禁、验收与文档回填

**Files:**
- Modify: `scripts/audit/check_panel_boundary.py`, `scripts/audit/test_check_panel_boundary.py`, `scripts/audit/panel-boundary.baseline.json`
- Modify: `docs/dozer-v2/bytehost-H0/07-usage-probe-gaps.md`(G4 回填)、`docs/dozer-v2/bytehost-H8-evaluation.md`(§7 追一句)、`CLAUDE.md`(Workspace 布局表加 `dozer-services`)、`scripts/audit/unit_edges.py` 若 `dozer_services` 需归类

**Interfaces:**
- Produces: 规则 `R-CLIENT`:`extensions/` 下各文件对 `dozer_client` 的引用次数(注释与字符串不计);基线 = 当前实测各文件值(Usage 迁出后其 `usage/mod.rs` 为 0,不进基线),只降不升。

- [ ] **Step 1: 先写失败测试**

在 `test_check_panel_boundary.py` 的 `Compare` 类之前加(同 `ProtocolCoupling` 的风格):
```python
class ClientCoupling(unittest.TestCase):
    """R-CLIENT:面板代码对 dozer_client 的引用只许减不许增(X0:面板改吃 dozer-services 的 trait)。"""
    def scan1(self, rel, text):
        return g.scan({rel: text}).get(rel, {})

    def test_use_and_qualified_path_in_extensions_are_counted(self):
        text = "use dozer_client::Client;\nfn f(_: &dozer_client::Client) {}\n"
        self.assertEqual(self.scan1("extensions/todo.rs", text), {"R-CLIENT": 2})

    def test_comments_and_strings_are_not_counted(self):
        text = '// dozer_client::Client\nconst S: &str = "dozer_client::Client";\n'
        self.assertEqual(self.scan1("extensions/todo.rs", text), {})

    def test_other_directories_are_not_counted(self):
        self.assertEqual(self.scan1("app/update.rs", "use dozer_client::Client;\n"), {})

    def test_dozer_services_paths_are_not_counted(self):
        self.assertEqual(self.scan1("extensions/usage/mod.rs", "use dozer_services::UsageService;\n"), {})
```
Run: `python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -5` → Expected: 第一个新测试 FAIL(RED)。

- [ ] **Step 2: 实现规则**

`check_panel_boundary.py`:在 `PROTOCOL_RE` 后加
```python
# R-CLIENT(X0):面板代码对 `dozer_client` 的引用次数。面板应经 `dozer-services` 的 trait 取数;
# 只许降不许增。注释与字符串字面量不计。
CLIENT_RE = re.compile(r"\bdozer_client\b")
```
`scan()` 的 `extensions/` 分支里、`R-PROTOCOL` 之后加同形的计数(`hit["R-CLIENT"] = k`);文件头规则列表加一行。

- [ ] **Step 3: 写入基线(只写 R-CLIENT,不固化既有失败)**

```bash
python3 - <<'EOF'
import json, importlib.util, os
spec = importlib.util.spec_from_file_location("g", "scripts/audit/check_panel_boundary.py")
g = importlib.util.module_from_spec(spec); spec.loader.exec_module(g)
cur = g.scan(g.load_sources())
p = "scripts/audit/panel-boundary.baseline.json"
base = json.load(open(p, encoding="utf-8"))
for rel, hit in cur.items():
    if "R-CLIENT" in hit:
        base.setdefault(rel, {})["R-CLIENT"] = hit["R-CLIENT"]
json.dump(base, open(p, "w", encoding="utf-8"), ensure_ascii=False, indent=1, sort_keys=True)
open(p, "a").write("\n")
print({r: h["R-CLIENT"] for r, h in cur.items() if "R-CLIENT" in h})
EOF
git diff --stat scripts/audit/panel-boundary.baseline.json
python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -2
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
```
Expected: 单测全绿;基线 diff 只含新增的 `R-CLIENT` 条目,且 `extensions/usage/mod.rs` **不在其中**;门禁输出仅 `R-HOST-EXECUTORS 0 -> 1`(既有),没有 `R-CLIENT`。

- [ ] **Step 4: 验收清单(逐条跑、读输出)**

```bash
cargo tree -p dozer-services --edges normal                                  # 1: 仅 dozer-model + serde 系
python3 scripts/audit/unit_edges.py ext:usage                                # 2: S 类只剩 bytegit;无 dozer_client(若有 dozer_services 行被判 ?,在 unit_edges.py 归 A 类并补测试,先看 FAIL)
cargo test -p dozer-app usage_probe 2>&1 | grep -E 'test result|FAILED'      # 3: S4a/S4b 通过
cargo test -p dozer-core --test model_golden                                 # 5: 通过且文件未改
cargo test --workspace --no-fail-fast > <ws>/final.txt 2>&1; grep -E '^test result' <ws>/final.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep FAILED <ws>/final.txt   # 4: ≥ 基线 + 新增,仅已知失败
cargo clippy --all-targets 2>&1 | grep -c '^warning'; cargo fmt --check && echo fmt-ok     # 6
bash scripts/check-log-scope.sh && echo log-scope-ok
git status --short                                                           # 无 Info.plist、无无关 Cargo 改动
```
任一不满足即不收尾,回到对应任务修。

- [ ] **Step 5: 文档回填并提交**

- `07-usage-probe-gaps.md`:G4 状态追加"已解(X0,2026-10-09):`UsageService` 接缝 + `ServiceError` 三类;失败走 Toast;`s4_characterizes_g4_*` 已翻转为 `s4a/s4b`"。
- `bytehost-H8-evaluation.md` §7 末尾追一句"X0 试点已落地:Usage 经 `dozer-services` 取数(见 `2026-10-09-dozer-services-x0-design.md`);其余面板按纵向切片迁移"。
- `CLAUDE.md` Workspace 布局表在 `dozer-model` 行后加:`| crates/dozer-services | 面板服务 trait 接缝(仅依赖 dozer-model);面板经 PanelIo.services() 取数,不再直接依赖 dozer_client(R-CLIENT 门禁,Usage 已迁,其余面板待迁);生产实现在 dozer-client |`。
```bash
git add scripts/audit docs CLAUDE.md
git commit -m "chore(audit): X0 S6 R-CLIENT 棘轮门禁 + 文档回填

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

之后按 executing-plans 的 Final Review(整分支 fresh reviewer)与 finishing-a-development-branch;**合并进 main 属于需先询问用户的外部副作用**,合并前先核对 main 上用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist` 及他人 WIP)与本分支的重叠,用 `git stash` → 合并 → `git stash pop`。
