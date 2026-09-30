# 静默吞错批 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让"用户发起动作后无声失败"的最伤人的五处有反馈:Keychain 密码写入/删除失败(SSH、数据库、Git 账户)、Todo 详情"处理"与"打开详情"失败、记忆详情读取失败。

**Architecture:** 复用已落地的 Toast 基础设施(`Outbox`、`Message::Toast`、每条自动写日志)。新增两个小的、可测试的构件:`toast::failure_message`(后台任务里把 `Err` 变成可经 `proxy` 发回的 Toast 消息)与 `secrets`(可注入的"钥匙串存取"抽象,让失败路径可测,且把 `NoEntry` 当成功)。记忆详情沿用该面板既有的内联 `ws_state.error`,不用 Toast。

**Tech Stack:** Rust、iced 0.14、`keyring` 4.1.6(`v1` 模式,无 mock 后端,所以自己做抽象)。

**Spec / 依据:** `docs/superpowers/specs/2026-09-30-silent-failure-audit.md`(A1、A2、A4、A5 及 `todo_detail_open` 的同类问题)。

## Global Constraints

- **分支**:独立 worktree/分支 `feat/silent-failures-batch1`,从 `main` 建,完成后审阅再合并;dispatch 子代理时**必须带完整 worktree 绝对路径前缀**。主 checkout 里目前有别的会话未提交的 `crates/dozer-app/src/extensions/project_create.rs` 改动——worktree 从 `HEAD` 建,不带它,**不要碰、不要提交**。
- **不改行为**:只补"失败反馈",成功路径行为不变;不改密码/凭据的存取语义(哪些情形写、哪些情形跳过)。
- **敏感信息**:Toast 与日志文案里**不得出现密码/口令/token 本身**,只允许出现底层错误原因(`keyring` 的错误不含秘密,但测试要断言这一点)。
- 日志来源用各面板已有的 `LOG`(`ssh`/`database`/`todo`/`settings`);Toast 经 `Outbox` 或 `Message::Toast`,自动写日志(`toast::log_toast`),**不要**再额外 `log_*!` 同一条。
- 门禁:`scripts/check-log-scope.sh` 必须仍然通过;不得引入裸 `tracing::*!`/`eprintln!`。
- 每个 Task 结束:`cargo build -p dozer-app`、`cargo test -p dozer-app`、`cargo clippy -p dozer-app --all-targets` 无新增告警、`cargo fmt` 干净。提交信息末尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。commit 前看 `git status`/`git diff --cached`。
- **已知与本批无关的测试问题**(对比基线时排除):`extensions::files::tests::delete_confirm_spec_reflects_pending_target` 在 main 上就失败;`git_log::tests::build_marks_head_branch_and_labels` 依赖仓库分支拓扑(假设 HEAD 是最新提交),在有更新提交的其它分支存在时会失败;整个 workspace 一起跑时 `dozerd` 的 3 个 `summary_pipeline::parse_*` 测试会因 `serde_json` 的 `preserve_order` 被统一而失败(单独 `-p dozerd` 通过)。

## 相对审计的一处有依据的调整

审计 A5 建议"打开记忆失败"用 Toast。改为**内联**:该面板已有 `ws_state.error`(`project/view.rs` 渲染在面板顶部),且同一组记忆操作里 `MemoryMutated(Err)` 已经写它("保存记忆失败: …")。"读取记忆失败"是同一个位置、同一类反馈,用 Toast 反而会让同一面板出现两种风格。

## 范围外

- 审计 A3(hook/mcp 安装)、A6–A8(dozerd 日志)、B 类(daemon 读请求变空数据)、C 类:留给后续批次。
- `let _ = io;` 之类死参数清理。
- 不改 `dozer-hook`/`dozer-mcp` 的 API。

## Review Focus

- **删除一个从没存过的凭据不能报警**:密钥认证的主机、没填密码的数据源本来就没有钥匙串条目,`delete_credential` 返回 `NoEntry` 必须当成功,否则每次删主机都弹一条假警告。(Task 2:`#[ignore]` 的真钥匙串测试 + 手工验收)
- **密码为空时不能碰钥匙串**:空密码不是"清除密码"。(Task 3、4:`store` 不被调用的测试)
- **Toast/日志文案里不含密码本身**。(Task 2、3、4 的断言)
- **`process_todo_now` 失败后详情仍要刷新,但只弹一条 Toast**:成功不弹;详情刷新也失败时,不能在"处理失败"之上再叠一条。(Task 6)
- **钥匙串失败不能阻断主机/数据源本身的保存**:密码没存上,但主机仍写入列表并落盘。(Task 3、4)

---

### Task 0: 建 worktree 并确认基线

- [ ] **Step 1: 建 worktree**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-silent1 -b feat/silent-failures-batch1 main
cd ../dozer-silent1
cargo build -p dozer-app 2>&1 | rg "generated|^error|Finished"
cargo test -p dozer-app 2>&1 | rg "test result|FAILED"
```

Expected:`Finished`、编译警告 5 个;测试只有 `delete_confirm_spec_reflects_pending_target`(可能再加 `build_marks_head_branch_and_labels`,见上)失败。若有其它失败,停下汇报。后续命令都在 `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-silent1` 下执行。

---

### Task 1: `toast::failure_message`(后台任务用的失败 → Toast 消息)

**Files:**
- Modify: `crates/dozer-app/src/extensions/toast.rs`

**Interfaces:**
- Consumes: 已有的 `Message::Push`、`Level`、`Scope`。
- Produces:`pub fn failure_message<T, E: std::fmt::Display>(scope: Scope, what: &str, res: &Result<T, E>) -> Option<Message>`——`Err(e)` 返回 `Some(Push { scope, level: Error, text: "{what}: {e}", key: None })`,`Ok` 返回 `None`。

- [ ] **Step 1: 写失败的测试**

在 `toast.rs` 的 `tests` 模块里(`TEST_TODO` 已声明)加:

```rust
    #[test]
    fn failure_message_is_none_on_ok() {
        assert!(failure_message(TEST_TODO, "处理任务失败", &Ok::<(), String>(())).is_none());
    }

    #[test]
    fn failure_message_builds_an_error_push_with_what_prefix() {
        let msg = failure_message(TEST_TODO, "处理任务失败", &Err::<(), _>("daemon 超时"))
            .expect("Err 应产生消息");
        let Message::Push {
            scope,
            level,
            text,
            key,
        } = msg;
        assert_eq!(scope, TEST_TODO);
        assert_eq!(level, Level::Error);
        assert_eq!(text, "处理任务失败: daemon 超时");
        assert_eq!(key, None);
    }
```

Run: `cargo test -p dozer-app failure_message 2>&1 | rg "^error" -A3 | head`
Expected:编译失败(`failure_message` 未定义)。

- [ ] **Step 2: 实现**

在 `toast.rs` 里 `log_toast` 之后加:

```rust
/// 后台任务(拿不到 `App`)里把一次失败变成可经 `proxy.send_event(Message::Toast(..))`
/// 发回主线程的 Toast 消息;`Ok` 返回 `None`(什么都不发)。文案是 `"{what}: {e}"`。
pub fn failure_message<T, E: std::fmt::Display>(
    scope: Scope,
    what: &str,
    res: &Result<T, E>,
) -> Option<Message> {
    res.as_ref().err().map(|e| Message::Push {
        scope,
        level: Level::Error,
        text: format!("{what}: {e}"),
        key: None,
    })
}
```

- [ ] **Step 3: 运行**

Run: `cargo test -p dozer-app failure_message 2>&1 | rg "^test |test result"`
Expected:2 个通过。此时 `failure_message` 还没有生产调用点,会有一条 dead_code 警告,Task 6 消除。

- [ ] **Step 4: 提交**

```bash
cargo fmt
git add crates/dozer-app/src/extensions/toast.rs
git commit -m "feat(toast): add failure_message for background tasks

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `secrets`(可注入的钥匙串存取抽象)

**Files:**
- Create: `crates/dozer-app/src/secrets.rs`
- Modify: `crates/dozer-app/src/main.rs`(加 `mod secrets;`,按字母序放 `runtime` 与 `tabular` 之间)

**Interfaces:**
- Consumes: `toast::{Level, Outbox}`、`dozer_core::log::Scope`。
- Produces:
  - `pub(crate) struct SecretRef<'a> { pub service: &'a str, pub account: &'a str }`
  - `pub(crate) trait SecretStore { fn set(&self, at: &SecretRef<'_>, secret: &str) -> Result<(), String>; fn delete(&self, at: &SecretRef<'_>) -> Result<(), String>; }`
  - `pub(crate) struct KeyringStore;`(实现 `SecretStore`;`delete` 把 `keyring::Error::NoEntry` 当成功)
  - `pub(crate) fn save(store: &dyn SecretStore, outbox: &mut Outbox, scope: Scope, at: &SecretRef<'_>, secret: &str)`:失败推 **Error** `"密码未能保存到系统钥匙串: {e}"`
  - `pub(crate) fn remove(store: &dyn SecretStore, outbox: &mut Outbox, scope: Scope, at: &SecretRef<'_>)`:失败推 **Warning** `"系统钥匙串里的凭据未能删除: {e}"`
  - `#[cfg(test)] pub(crate) mod fake { pub(crate) struct FakeStore { .. } }`(供 Task 3、4 复用)

- [ ] **Step 1: 写失败的测试(同时建文件骨架)**

创建 `crates/dozer-app/src/secrets.rs`,先只放模块文档、`use` 与测试(不写实现),并在 `main.rs` 加 `mod secrets;`:

```rust
//! 系统钥匙串写入的可注入抽象。`keyring` 4.x 没有可用的 mock 后端(全局替换存储会
//! 牵连别的测试),所以把"写/删一条凭据"抽成 `SecretStore`:生产走 `KeyringStore`,
//! 测试用 `fake::FakeStore`,失败路径因此可测。
//!
//! 此前 SSH/数据库面板对钥匙串的写入与删除都是 `let _ = entry.set_password(..)`,
//! 失败时界面显示"已保存",之后连接报一个莫名其妙的认证错误。

use crate::extensions::toast::{Level, Outbox};
use dozer_core::log::Scope;

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::cell::RefCell;

    /// 记录每次调用并返回预设结果的假钥匙串。
    pub(crate) struct FakeStore {
        pub(crate) set_result: Result<(), String>,
        pub(crate) delete_result: Result<(), String>,
        pub(crate) calls: RefCell<Vec<String>>,
    }

    impl FakeStore {
        pub(crate) fn ok() -> Self {
            Self {
                set_result: Ok(()),
                delete_result: Ok(()),
                calls: RefCell::new(Vec::new()),
            }
        }

        pub(crate) fn failing(reason: &str) -> Self {
            Self {
                set_result: Err(reason.to_string()),
                delete_result: Err(reason.to_string()),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl SecretStore for FakeStore {
        fn set(&self, at: &SecretRef<'_>, secret: &str) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("set {}/{} {}", at.service, at.account, secret));
            self.set_result.clone()
        }

        fn delete(&self, at: &SecretRef<'_>) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("delete {}/{}", at.service, at.account));
            self.delete_result.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeStore;
    use super::*;

    dozer_core::scope!(TEST_SSH, panel, "ssh");

    fn at() -> SecretRef<'static> {
        SecretRef {
            service: "dozer-ssh",
            account: "7:h1",
        }
    }

    #[test]
    fn save_ok_pushes_nothing_and_calls_the_store_once() {
        let store = FakeStore::ok();
        let mut outbox = Outbox::default();
        save(&store, &mut outbox, TEST_SSH, &at(), "pw-secret");
        assert!(outbox.take().is_empty());
        assert_eq!(*store.calls.borrow(), vec!["set dozer-ssh/7:h1 pw-secret"]);
    }

    #[test]
    fn save_failure_is_an_error_toast_that_never_contains_the_secret() {
        let store = FakeStore::failing("钥匙串已锁定");
        let mut outbox = Outbox::default();
        save(&store, &mut outbox, TEST_SSH, &at(), "pw-secret");
        let got = outbox.take();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].level, Level::Error);
        assert_eq!(got[0].scope, TEST_SSH);
        assert_eq!(got[0].text, "密码未能保存到系统钥匙串: 钥匙串已锁定");
        assert!(!got[0].text.contains("pw-secret"), "文案不得含秘密本身");
    }

    #[test]
    fn remove_ok_pushes_nothing() {
        let store = FakeStore::ok();
        let mut outbox = Outbox::default();
        remove(&store, &mut outbox, TEST_SSH, &at());
        assert!(outbox.take().is_empty());
        assert_eq!(*store.calls.borrow(), vec!["delete dozer-ssh/7:h1"]);
    }

    #[test]
    fn remove_failure_is_a_warning_not_an_error() {
        let store = FakeStore::failing("拒绝访问");
        let mut outbox = Outbox::default();
        remove(&store, &mut outbox, TEST_SSH, &at());
        let got = outbox.take();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].level, Level::Warning);
        assert_eq!(got[0].text, "系统钥匙串里的凭据未能删除: 拒绝访问");
    }

    /// 真钥匙串:删除一个不存在的条目必须当成功(密钥认证的主机从没存过密码)。
    /// 会访问系统钥匙串,默认不跑;验收时手工 `cargo test -p dozer-app -- --ignored
    /// keyring_store_delete_of_missing_entry_is_ok`。
    #[test]
    #[ignore = "touches the real system keychain"]
    fn keyring_store_delete_of_missing_entry_is_ok() {
        let account = format!("dozer-test-never-exists-{}", std::process::id());
        let at = SecretRef {
            service: "dozer-test",
            account: &account,
        };
        assert_eq!(KeyringStore.delete(&at), Ok(()));
    }
}
```

Run: `cargo test -p dozer-app secrets:: 2>&1 | rg "^error" -A3 | head`
Expected:编译失败(`SecretRef`/`SecretStore`/`save`/`remove`/`KeyringStore` 未定义)。

- [ ] **Step 2: 实现**

在 `secrets.rs` 的 `use` 与 `#[cfg(test)] pub(crate) mod fake` 之间插入:

```rust
/// 一条凭据在钥匙串里的定位。
pub(crate) struct SecretRef<'a> {
    pub service: &'a str,
    pub account: &'a str,
}

pub(crate) trait SecretStore {
    fn set(&self, at: &SecretRef<'_>, secret: &str) -> Result<(), String>;
    /// 条目本来就不存在视为成功:密钥认证的主机、没填密码的数据源从没存过密码,
    /// 删除它们不该报错(同 `git_accounts::delete_token` 的既有口径)。
    fn delete(&self, at: &SecretRef<'_>) -> Result<(), String>;
}

/// 生产实现:走系统钥匙串(`keyring`)。
pub(crate) struct KeyringStore;

impl SecretStore for KeyringStore {
    fn set(&self, at: &SecretRef<'_>, secret: &str) -> Result<(), String> {
        keyring::Entry::new(at.service, at.account)
            .and_then(|entry| entry.set_password(secret))
            .map_err(|e| e.to_string())
    }

    fn delete(&self, at: &SecretRef<'_>) -> Result<(), String> {
        match keyring::Entry::new(at.service, at.account) {
            Ok(entry) => match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(e.to_string()),
            },
            Err(e) => Err(e.to_string()),
        }
    }
}

/// 保存一条凭据;失败推 **Error** 提示(用户否则以为已保存,之后连接报认证错误)。
/// 文案只含底层错误原因,**不含秘密本身**。
pub(crate) fn save(
    store: &dyn SecretStore,
    outbox: &mut Outbox,
    scope: Scope,
    at: &SecretRef<'_>,
    secret: &str,
) {
    if let Err(e) = store.set(at, secret) {
        outbox.push(
            scope,
            Level::Error,
            format!("密码未能保存到系统钥匙串: {e}"),
        );
    }
}

/// 删除一条凭据;失败推 **Warning**(界面已显示删除,钥匙串里却还留着密钥)。
pub(crate) fn remove(
    store: &dyn SecretStore,
    outbox: &mut Outbox,
    scope: Scope,
    at: &SecretRef<'_>,
) {
    if let Err(e) = store.delete(at) {
        outbox.push(
            scope,
            Level::Warning,
            format!("系统钥匙串里的凭据未能删除: {e}"),
        );
    }
}
```

- [ ] **Step 3: 运行**

Run: `cargo test -p dozer-app secrets:: 2>&1 | rg "^test |test result"`
Expected:4 个通过,1 个 `ignored`。`KeyringStore`/`save`/`remove` 暂无生产调用点会有 dead_code 警告,Task 3–4 消除。

- [ ] **Step 4: 提交**

```bash
cargo fmt && cargo clippy -p dozer-app --all-targets 2>&1 | rg "generated"
git add crates/dozer-app/src/secrets.rs crates/dozer-app/src/main.rs
git commit -m "feat(secrets): injectable keychain store that notifies on failure

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: SSH 面板接入

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`(`keyring_entry` 附近加常量与两个函数;`DraftSave` 与 `DeleteHost` 两处改调用;测试模块加测试)

**Interfaces:**
- Consumes: Task 2 的 `secrets::{SecretRef, SecretStore, KeyringStore, save, remove}`、`ws_state.outbox`。
- Produces:`pub(crate) fn store_host_password(store: &dyn SecretStore, ws_state: &mut WorkspaceState, project_id: i64, host_id: &str, password: &str)`、`pub(crate) fn forget_host_password(store: &dyn SecretStore, ws_state: &mut WorkspaceState, project_id: i64, host_id: &str)`。

- [ ] **Step 1: 写失败的测试**

在 `ssh.rs` 的 `tests` 模块里(已有 `host()` 辅助、`persist_hosts_*` 测试)加:

```rust
    #[test]
    fn store_host_password_uses_the_ssh_service_and_project_scoped_account() {
        use crate::secrets::fake::FakeStore;
        let store = FakeStore::ok();
        let mut ws = WorkspaceState::default();
        store_host_password(&store, &mut ws, 7, "h1", "pw-secret");
        assert_eq!(*store.calls.borrow(), vec!["set dozer-ssh/7:h1 pw-secret"]);
        assert!(ws.take_outbox().is_empty());
    }

    #[test]
    fn store_host_password_failure_toasts_with_ssh_scope_and_leaks_nothing() {
        use crate::secrets::fake::FakeStore;
        let store = FakeStore::failing("钥匙串已锁定");
        let mut ws = WorkspaceState::default();
        store_host_password(&store, &mut ws, 7, "h1", "pw-secret");
        let got = ws.take_outbox();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].scope.name, "ssh");
        assert!(got[0].text.contains("钥匙串已锁定"));
        assert!(!got[0].text.contains("pw-secret"));
    }

    #[test]
    fn forget_host_password_failure_is_a_warning() {
        use crate::secrets::fake::FakeStore;
        let store = FakeStore::failing("拒绝访问");
        let mut ws = WorkspaceState::default();
        forget_host_password(&store, &mut ws, 7, "h1");
        assert_eq!(*store.calls.borrow(), vec!["delete dozer-ssh/7:h1"]);
        let got = ws.take_outbox();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].level, crate::extensions::toast::Level::Warning);
    }
```

Run: `cargo test -p dozer-app host_password 2>&1 | rg "^error" -A3 | head`
Expected:编译失败(函数未定义)。

- [ ] **Step 2: 实现**

`ssh.rs` 里把 `keyring_entry`(约 `:225`)改成用常量,并在其后加两个函数:

```rust
/// SSH 凭据在钥匙串里的 service。读(`keyring_password`)、写、删共用这一个常量,
/// 避免各写一份字面量以后漂移。
const KEYRING_SERVICE: &str = "dozer-ssh";

fn keyring_account(project_id: i64, host_id: &str) -> String {
    format!("{project_id}:{host_id}")
}

fn keyring_entry(project_id: i64, host_id: &str) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new(KEYRING_SERVICE, &keyring_account(project_id, host_id))
}

/// 把主机密码/私钥口令写进钥匙串;失败推 Toast(此前 `let _ = entry.set_password(..)`
/// 静默吞掉,界面显示已保存,之后连接报认证错误)。**调用方保证 `password` 非空**——
/// 空密码不是"清除密码",不该碰钥匙串。
pub(crate) fn store_host_password(
    store: &dyn crate::secrets::SecretStore,
    ws_state: &mut WorkspaceState,
    project_id: i64,
    host_id: &str,
    password: &str,
) {
    let account = keyring_account(project_id, host_id);
    crate::secrets::save(
        store,
        &mut ws_state.outbox,
        LOG,
        &crate::secrets::SecretRef {
            service: KEYRING_SERVICE,
            account: &account,
        },
        password,
    );
}

/// 删主机时清掉它的钥匙串条目;失败推 Warning(条目不存在视为成功,见 `SecretStore::delete`)。
pub(crate) fn forget_host_password(
    store: &dyn crate::secrets::SecretStore,
    ws_state: &mut WorkspaceState,
    project_id: i64,
    host_id: &str,
) {
    let account = keyring_account(project_id, host_id);
    crate::secrets::remove(
        store,
        &mut ws_state.outbox,
        LOG,
        &crate::secrets::SecretRef {
            service: KEYRING_SERVICE,
            account: &account,
        },
    );
}
```

把 `update` 里两处调用点改掉:

```rust
            // DraftSave 分支:原 `if !draft.password.is_empty() && let Ok(entry) = keyring_entry(..) { let _ = entry.set_password(..) }`
            if !draft.password.is_empty() {
                store_host_password(
                    &crate::secrets::KeyringStore,
                    ws_state,
                    project_id,
                    &id,
                    &draft.password,
                );
            }
            persist_hosts(ws_state, repo_path);
```

```rust
            // DeleteHost 分支:原 `if let Ok(entry) = keyring_entry(..) { let _ = entry.delete_credential(); }`
            forget_host_password(&crate::secrets::KeyringStore, ws_state, project_id, &id);
            persist_hosts(ws_state, repo_path);
```

(`keyring_entry` 仍被读密码的 `keyring_password` 使用,保留。)

- [ ] **Step 3: 运行**

Run: `cargo test -p dozer-app host_password 2>&1 | rg "^test |test result"` 与 `cargo test -p dozer-app ssh:: 2>&1 | rg "test result|FAILED"`
Expected:3 个新测试通过,`ssh` 既有测试不受影响。

- [ ] **Step 4: 提交**

```bash
cargo fmt
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "fix(ssh): report keychain write/delete failures instead of swallowing them

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 数据库面板接入

**Files:**
- Modify: `crates/dozer-app/src/extensions/database/load.rs`(`keyring_entry` 用常量)
- Modify: `crates/dozer-app/src/extensions/database/update.rs`(两处调用点 + 两个函数)
- Modify: `crates/dozer-app/src/extensions/database/mod.rs`(`persist_tests` 模块加测试)

**Interfaces:**
- Consumes: Task 2 产出;`ws_state.outbox`;数据库文件里已有的 `LOG`。
- Produces:`pub(crate) fn store_source_password(store: &dyn SecretStore, ws_state: &mut WorkspaceState, project_id: i64, source_id: &str, password: &str)`、`pub(crate) fn forget_source_password(store: &dyn SecretStore, ws_state: &mut WorkspaceState, project_id: i64, source_id: &str)`;`pub(crate) const KEYRING_SERVICE: &str = "dozer"`(在 `load.rs`)、`pub(crate) fn keyring_account(project_id: i64, source_id: &str) -> String`。

- [ ] **Step 1: 写失败的测试**

在 `database/mod.rs` 末尾的 `persist_tests` 模块里加(`use super::*;` 已有):

```rust
    #[test]
    fn store_source_password_uses_the_dozer_service_and_project_scoped_account() {
        use crate::secrets::fake::FakeStore;
        let store = FakeStore::ok();
        let mut ws = WorkspaceState::default();
        update::store_source_password(&store, &mut ws, 7, "s1", "pw-secret");
        assert_eq!(*store.calls.borrow(), vec!["set dozer/7:s1 pw-secret"]);
        assert!(ws.take_outbox().is_empty());
    }

    #[test]
    fn store_source_password_failure_toasts_with_database_scope_and_leaks_nothing() {
        use crate::secrets::fake::FakeStore;
        let store = FakeStore::failing("钥匙串已锁定");
        let mut ws = WorkspaceState::default();
        update::store_source_password(&store, &mut ws, 7, "s1", "pw-secret");
        let got = ws.take_outbox();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].scope.name, "database");
        assert!(got[0].text.contains("钥匙串已锁定"));
        assert!(!got[0].text.contains("pw-secret"));
    }

    #[test]
    fn forget_source_password_failure_is_a_warning() {
        use crate::secrets::fake::FakeStore;
        let store = FakeStore::failing("拒绝访问");
        let mut ws = WorkspaceState::default();
        update::forget_source_password(&store, &mut ws, 7, "s1");
        assert_eq!(*store.calls.borrow(), vec!["delete dozer/7:s1"]);
        let got = ws.take_outbox();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].level, crate::extensions::toast::Level::Warning);
    }
```

Run: `cargo test -p dozer-app source_password 2>&1 | rg "^error" -A3 | head`
Expected:编译失败。

- [ ] **Step 2: 实现**

`database/load.rs` 的 `keyring_entry`(约 `:316`)改成:

```rust
/// 数据库凭据在钥匙串里的 service。读、写、删共用,避免字面量漂移。
pub(crate) const KEYRING_SERVICE: &str = "dozer";

pub(crate) fn keyring_account(project_id: i64, source_id: &str) -> String {
    format!("{project_id}:{source_id}")
}

pub(crate) fn keyring_entry(
    project_id: i64,
    source_id: &str,
) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new(KEYRING_SERVICE, &keyring_account(project_id, source_id))
}
```

`database/update.rs` 里 `persist_sources` 之后加:

```rust
/// 把数据源密码写进钥匙串;失败推 Toast(此前 `let _ = entry.set_password(..)` 静默吞掉)。
/// **调用方保证 `password` 非空**(`draft_to_source` 只在有密码时返回 `Some`)。
pub(crate) fn store_source_password(
    store: &dyn crate::secrets::SecretStore,
    ws_state: &mut WorkspaceState,
    project_id: i64,
    source_id: &str,
    password: &str,
) {
    let account = keyring_account(project_id, source_id);
    crate::secrets::save(
        store,
        &mut ws_state.outbox,
        LOG,
        &crate::secrets::SecretRef {
            service: KEYRING_SERVICE,
            account: &account,
        },
        password,
    );
}

/// 删数据源时清掉它的钥匙串条目;失败推 Warning(条目不存在视为成功)。
pub(crate) fn forget_source_password(
    store: &dyn crate::secrets::SecretStore,
    ws_state: &mut WorkspaceState,
    project_id: i64,
    source_id: &str,
) {
    let account = keyring_account(project_id, source_id);
    crate::secrets::remove(
        store,
        &mut ws_state.outbox,
        LOG,
        &crate::secrets::SecretRef {
            service: KEYRING_SERVICE,
            account: &account,
        },
    );
}
```

两处调用点:

```rust
            // 草稿保存:原 `if let Some(p) = pw_to_save && let Ok(entry) = keyring_entry(..) { let _ = entry.set_password(&p); }`
            if let Some(p) = pw_to_save {
                store_source_password(&crate::secrets::KeyringStore, ws_state, project_id, &id, &p);
            }
```

```rust
            // 删除数据源:原 `if let Ok(entry) = keyring_entry(..) { let _ = entry.delete_credential(); }`
            forget_source_password(&crate::secrets::KeyringStore, ws_state, project_id, &id);
```

(`update.rs` 里另外两处 `set_password(None)`/`set_password(Some(p))` 是 `url` crate 对连接串里密码字段的设置,**不是钥匙串**,不要动。`keyring_entry` 仍被读密码的地方使用,保留。)

- [ ] **Step 3: 运行**

Run: `cargo test -p dozer-app source_password 2>&1 | rg "^test |test result"`;`cargo test -p dozer-app database:: 2>&1 | rg "test result|FAILED"`
Expected:3 个新测试通过,`database` 既有测试全过。

- [ ] **Step 4: 提交**

```bash
cargo fmt
git add crates/dozer-app/src/extensions/database
git commit -m "fix(database): report keychain write/delete failures instead of swallowing them

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Git 账户断开(设置弹窗)

**Files:**
- Modify: `crates/dozer-app/src/extensions/settings.rs`(`State` 加 `outbox` 与 `take_outbox`;`finish_disconnect`;`Disconnect` 分支;测试)
- Modify: `crates/dozer-app/src/app/update.rs`(`drain_outboxes` 加 settings)

**Interfaces:**
- Consumes: 已有的 `git_accounts::{load, save, delete_token}`、`ConnectState`、`toast::Outbox`。
- Produces:`settings::State.outbox: toast::Outbox`(`pub(crate)`)、`settings::State::take_outbox(&mut self) -> Vec<toast::Pending>`、`pub(crate) fn finish_disconnect(state: &mut State, provider: GitProvider, saved: Result<(), String>, delete_token: impl FnOnce() -> Result<(), String>)`。

行为:本地记录写成功 → 尝试删钥匙串 token(失败推 **Warning** `"账户已断开,但系统钥匙串里的 token 未能删除: {e}"`),槽位置 `NotConnected`;本地记录写失败 → **保持已连接展示**(原有语义)并推 **Error** `"断开账户失败:写入本地记录出错({e}),已保留原有连接状态"`(此前只有一条 `log_error!`,用户看到的是"点了没反应")。

- [ ] **Step 1: 写失败的测试**

在 `settings.rs` 末尾的 `#[cfg(test)] mod tests` 里加(不读磁盘:用结构体字面量造 `State`):

```rust
    fn blank_state() -> State {
        State {
            github: ConnectState::Connected {
                username: "octocat".into(),
            },
            gitlab: ConnectState::NotConnected,
            gitee: ConnectState::NotConnected,
            connect_tasks: HashMap::new(),
            advanced: AdvancedState::Idle { error: None },
            selected: SettingsTab::Theme,
            tab_hover: None,
            close_hover: false,
            outbox: Default::default(),
        }
    }

    #[test]
    fn disconnect_success_clears_the_slot_and_stays_silent() {
        let mut s = blank_state();
        finish_disconnect(&mut s, GitProvider::GitHub, Ok(()), || Ok(()));
        assert_eq!(s.github, ConnectState::NotConnected);
        assert!(s.take_outbox().is_empty());
    }

    #[test]
    fn disconnect_token_delete_failure_still_disconnects_but_warns() {
        let mut s = blank_state();
        finish_disconnect(&mut s, GitProvider::GitHub, Ok(()), || {
            Err("系统钥匙串删除失败: 拒绝访问".to_string())
        });
        assert_eq!(s.github, ConnectState::NotConnected, "本地记录已断开");
        let got = s.take_outbox();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].level, crate::extensions::toast::Level::Warning);
        assert!(got[0].text.contains("token 未能删除"), "{}", got[0].text);
        assert_eq!(got[0].scope.name, "settings");
    }

    #[test]
    fn disconnect_save_failure_keeps_connected_state_and_tells_the_user() {
        let mut s = blank_state();
        let mut token_delete_called = false;
        finish_disconnect(&mut s, GitProvider::GitHub, Err("磁盘满".to_string()), || {
            token_delete_called = true;
            Ok(())
        });
        assert!(
            matches!(s.github, ConnectState::Connected { .. }),
            "写不进本地记录就整个放弃这次断开"
        );
        assert!(!token_delete_called, "本地记录没写成,不能先把钥匙串 token 删了");
        let got = s.take_outbox();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].level, crate::extensions::toast::Level::Error);
        assert!(got[0].text.contains("磁盘满"));
    }
```

(若 `ConnectState`/`AdvancedState`/`SettingsTab` 没有 `PartialEq` 或字段名与上面不符,按 `settings.rs` 实际定义调整**测试里的构造与断言写法**,不改生产类型的派生。)

Run: `cargo test -p dozer-app disconnect_ 2>&1 | rg "^error" -A3 | head`
Expected:编译失败(`outbox` 字段/`finish_disconnect` 未定义)。

- [ ] **Step 2: 实现**

`settings.rs`:

1. `State` 结构体加字段(放在 `close_hover` 之后)与 `load` 里的初始化 `outbox: Default::default(),`:

```rust
    /// 待发提示(断开账户失败等),`App::update` 包装函数排空成 Toast。
    pub(crate) outbox: crate::extensions::toast::Outbox,
```

2. `impl State` 加:

```rust
    /// 取走待发提示(`App::drain_outboxes` 调用)。
    pub fn take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending> {
        self.outbox.take()
    }
```

3. 新函数(放在 `impl State` 之后):

```rust
/// 断开 Git 账户的收尾:`saved` 是"把账户置空写回本地非敏感记录"的结果,`delete_token`
/// 是删钥匙串 token 的动作(惰性传入,以便"本地记录没写成就不能先删 token"可测)。
///
/// 顺序沿用既有裁决:**先写本地记录,成功了才删钥匙串**——反过来会留下"钥匙串已删、
/// json 仍写已连接"这种更糟的不一致态。本地记录写失败就整个放弃这次断开,保留已连接展示,
/// 并且现在**告诉用户**(此前只写一条日志,用户看到的是点了没反应)。
pub(crate) fn finish_disconnect(
    state: &mut State,
    provider: GitProvider,
    saved: Result<(), String>,
    delete_token: impl FnOnce() -> Result<(), String>,
) {
    match saved {
        Ok(()) => {
            if let Err(e) = delete_token() {
                state.outbox.push(
                    LOG,
                    crate::extensions::toast::Level::Warning,
                    format!("账户已断开,但系统钥匙串里的 token 未能删除: {e}"),
                );
            }
            *state.slot_mut(provider) = ConnectState::NotConnected;
        }
        Err(e) => {
            state.outbox.push(
                LOG,
                crate::extensions::toast::Level::Error,
                format!("断开账户失败:写入本地记录出错({e}),已保留原有连接状态"),
            );
        }
    }
}
```

4. `Message::Disconnect(provider)` 分支改为:

```rust
        Message::Disconnect(provider) => {
            let mut accounts = git_accounts::load();
            accounts.set(*provider, None);
            let saved = git_accounts::save(&accounts).map_err(|e| e.to_string());
            finish_disconnect(state, *provider, saved, || git_accounts::delete_token(*provider));
            true
        }
```

(保留原分支上方那段"先写本地记录,成功了才删 Keychain token"的注释,挪到 `finish_disconnect` 文档里即可,不要丢。)

`app/update.rs` 的 `drain_outboxes` 里,在 `pending.extend(self.database.take_outbox());` 之后加:

```rust
        if let Some(settings) = self.settings.as_mut() {
            pending.extend(settings.take_outbox());
        }
```

- [ ] **Step 3: 运行**

Run: `cargo test -p dozer-app disconnect_ 2>&1 | rg "^test |test result"`;`cargo test -p dozer-app settings:: 2>&1 | rg "test result|FAILED"`
Expected:3 个新测试通过,`settings` 既有测试全过。

- [ ] **Step 4: 提交**

```bash
cargo fmt
git add crates/dozer-app/src/extensions/settings.rs crates/dozer-app/src/app/update.rs
git commit -m "fix(settings): tell the user when disconnecting a git account fails

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Todo 详情:"处理"与"打开详情"失败

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo/update.rs`(`LOG` 改为 `pub(crate)`)
- Modify: `crates/dozer-app/src/app/update.rs`(`todo_detail_open`、`todo_detail_process`)

**Interfaces:**
- Consumes: Task 1 的 `toast::failure_message`;`crate::extensions::todo::LOG`(经 `todo/mod.rs` 已有的 `pub(crate) use update::*;` 导出)。
- Produces:无新接口。

`failure_message` 的单测已在 Task 1;这里的接线在 `App` 的后台任务里(`App` 没有便宜的单测夹具),靠手工验收。

- [ ] **Step 1: 导出 `LOG`**

`todo/update.rs` 顶部把 `dozer_core::scope!(LOG, panel, "todo");` 改为 `dozer_core::scope!(pub(crate) LOG, panel, "todo");`。

Run: `cargo build -p dozer-app 2>&1 | rg "^error" -A6 | head`
Expected:无错误。若出现 `LOG` 在 `todo` 模块下的名字冲突(glob 重导出歧义),把 `state.rs`/`view.rs` 里若有同名声明改掉——按当前代码,只有 `update.rs` 声明它。

- [ ] **Step 2: 改 `todo_detail_open` 与 `todo_detail_process`**

`app/update.rs`:

```rust
    pub(crate) fn todo_detail_open(&mut self, idx: usize) {
        // ...(前面不变)...
        handle.spawn(async move {
            let res = client.get_todo_detail(id).await;
            match res {
                Ok((_, turns)) => {
                    let _ = proxy.send_event(Message::TodoDetailLoaded(idx, turns));
                }
                Err(e) => {
                    // 详情弹窗已经打开(空回合),读取失败时用户只会看到一片空白。
                    let _ = proxy.send_event(Message::Toast(toast::Message::Push {
                        scope: todo::LOG,
                        level: toast::Level::Error,
                        text: format!("读取任务详情失败: {e}"),
                        key: None,
                    }));
                }
            }
        });
    }
```

```rust
    pub(crate) fn todo_detail_process(&mut self) {
        // ...(前面不变)...
        handle.spawn(async move {
            // 失败此前被 `let _ =` 吞掉,随后照常刷新详情,用户看到的是"点了什么都没发生"。
            let processed = client.process_todo_now(id, reply_text.as_deref()).await;
            let failed = processed.is_err();
            if let Some(msg) = toast::failure_message(todo::LOG, "处理任务失败", &processed) {
                let _ = proxy.send_event(Message::Toast(msg));
            }
            // 无论处理成败都用服务端权威回合列表刷新;刷新本身失败时,只有在"处理成功"的
            // 情况下才再提示——处理已经失败时不在其上再叠一条。
            match client.get_todo_detail(id).await {
                Ok((_, turns)) => {
                    let _ = proxy.send_event(Message::TodoDetailLoaded(idx, turns));
                }
                Err(e) if !failed => {
                    let _ = proxy.send_event(Message::Toast(toast::Message::Push {
                        scope: todo::LOG,
                        level: toast::Level::Error,
                        text: format!("刷新任务详情失败: {e}"),
                        key: None,
                    }));
                }
                Err(_) => {}
            }
        });
    }
```

(注意保持原有 `Message::TodoDetailLoaded(idx, turns)` 的发送与函数其余部分不变;`toast`、`todo` 在 `app/update.rs` 顶部已导入。)

- [ ] **Step 3: 构建与测试**

Run: `cargo build -p dozer-app 2>&1 | rg "^error|generated" -A5`;`cargo test -p dozer-app 2>&1 | rg "test result|FAILED"`
Expected:构建通过,`failure_message` 的 dead_code 警告消失(编译警告回到 5 个);测试无新增失败。

- [ ] **Step 4: 手工验收(需要真实 daemon)**

1. 打开一个 Todo 详情、点"处理",同时让 dozerd 不可达(例如先在设置里停掉 dozerd):出现红色 Toast"处理任务失败: …",**只有这一条**。
2. dozerd 正常时点"处理":无 Toast,详情按服务端结果刷新(行为不变)。
3. 停掉 dozerd 后点开一条任务详情:出现"读取任务详情失败: …"。

- [ ] **Step 5: 提交**

```bash
cargo fmt
git add crates/dozer-app/src/extensions/todo/update.rs crates/dozer-app/src/app/update.rs
git commit -m "fix(todo): surface failures of process-now and detail loading

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 7: 记忆详情读取失败(内联)

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`(`Message` 加 `MemoryDetailLoadFailed(String)`;`memory_detail_message` 纯函数)
- Modify: `crates/dozer-app/src/extensions/project/update.rs`(两处 `if let Ok(..) = client.get_memory(..)`;新消息的处理分支;测试)

**Interfaces:**
- Consumes: `dozer_core::protocol::MemoryDetail`、`ws_state.error`(面板内联错误)。
- Produces:`Message::MemoryDetailLoadFailed(String)`;`pub(crate) fn memory_detail_message<E: std::fmt::Display>(res: Result<dozer_core::protocol::MemoryDetail, E>) -> Message`(`Ok` → `MemoryDetailLoaded`,`Err` → `MemoryDetailLoadFailed(e.to_string())`)。

- [ ] **Step 1: 写失败的测试**

在 `extensions/project.rs` 已有的 `#[cfg(test)]` 模块里(那里已有 `new_ws()`、`test_repo_path()`、`test_client()` 辅助,`update(&mut ws, msg, 1, "名字", &test_repo_path(), &test_client(), rt.handle(), |_| {})` 是既有测试调用它的方式)加:

```rust
    fn sample_memory_detail() -> dozer_core::protocol::MemoryDetail {
        dozer_core::protocol::MemoryDetail {
            id: 1,
            project_id: 1,
            title: "t".into(),
            kind: "note".into(),
            description: "d".into(),
            body: "b".into(),
            created_ms: 0,
            created_by: "user".into(),
            updated_ms: 0,
            updated_by: "user".into(),
            history: Vec::new(),
        }
    }

    #[test]
    fn memory_detail_message_maps_ok_and_err() {
        assert!(matches!(
            memory_detail_message(Ok::<_, String>(sample_memory_detail())),
            Message::MemoryDetailLoaded(_)
        ));
        match memory_detail_message(Err::<dozer_core::protocol::MemoryDetail, _>("连接被拒")) {
            Message::MemoryDetailLoadFailed(e) => assert_eq!(e, "连接被拒"),
            other => panic!("期望 MemoryDetailLoadFailed,得到 {other:?}"),
        }
    }

    #[test]
    fn memory_detail_load_failed_sets_the_inline_error() {
        let mut ws = new_ws();
        let rt = tokio::runtime::Runtime::new().unwrap();
        update(
            &mut ws,
            Message::MemoryDetailLoadFailed("连接被拒".to_string()),
            1,
            "名字",
            &test_repo_path(),
            &test_client(),
            rt.handle(),
            |_| {},
        );
        assert_eq!(ws.error.as_deref(), Some("读取记忆失败: 连接被拒"));
        assert!(ws.memory_detail.is_none(), "读取失败不应留下半截详情");
    }
```

Run: `cargo test -p dozer-app memory_detail 2>&1 | rg "^error" -A3 | head`
Expected:编译失败。

- [ ] **Step 2: 实现**

`project.rs`:`Message` 枚举里 `MemoryDetailLoaded(...)` 之后加

```rust
    /// 读取记忆详情失败(打开一条记忆,或编辑成功后刷新详情)。此前失败时静默不动,
    /// 用户点了没反应。沿用该面板既有的内联 `error` 展示("保存记忆失败"同款)。
    MemoryDetailLoadFailed(String),
```

并加纯函数:

```rust
/// 把一次 `get_memory` 的结果变成要回给 `update` 的消息。
pub(crate) fn memory_detail_message<E: std::fmt::Display>(
    res: Result<dozer_core::protocol::MemoryDetail, E>,
) -> Message {
    match res {
        Ok(detail) => Message::MemoryDetailLoaded(detail),
        Err(e) => Message::MemoryDetailLoadFailed(e.to_string()),
    }
}
```

`project/update.rs`:

```rust
        Message::MemoryDetailOpen(id) => {
            ws_state.memory_detail = None;
            ws_state.memory_edit_draft = None;
            let client = client.clone();
            handle.spawn(async move {
                emit(memory_detail_message(client.get_memory(project_id, id).await));
            });
        }
        Message::MemoryDetailLoaded(detail) => {
            ws_state.memory_detail = Some(detail);
        }
        Message::MemoryDetailLoadFailed(e) => {
            ws_state.error = Some(format!("读取记忆失败: {e}"));
        }
```

编辑成功后的刷新(原 `if let Ok(fresh) = client.get_memory(project_id, detail.id).await { emit(Message::MemoryDetailLoaded(fresh)); }`)改为:

```rust
                        emit(memory_detail_message(
                            client.get_memory(project_id, detail.id).await,
                        ));
```

(`MemoryMutated(Ok(()))` 随后照常 emit——编辑本身成功;刷新失败只在面板上多一条内联提示。)

- [ ] **Step 3: 运行**

Run: `cargo test -p dozer-app memory_detail 2>&1 | rg "^test |test result"`;`cargo test -p dozer-app project:: 2>&1 | rg "test result|FAILED"`
Expected:2 个新测试通过,`project` 既有测试全过。

- [ ] **Step 4: 手工验收**

停掉 dozerd 后,在项目面板点开一条记忆:面板顶部出现"⚠ 读取记忆失败: …";恢复后再点开正常显示详情且提示消失(下一次成功操作会清 `error`——若不清,见「已知边界」)。

- [ ] **Step 5: 提交**

```bash
cargo fmt
git add crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/extensions/project
git commit -m "fix(project): show an inline error when loading a memory detail fails

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 8: 收尾与总验证

**Files:**
- Modify: `docs/superpowers/specs/2026-09-30-silent-failure-audit.md`(标注批 1 已落地)

- [ ] **Step 1: 文档**

审计文档「建议的批次」里的批 1 后面加"**已落地**(计划 `plans/2026-09-30-silent-failures-batch1.md`)",并注明两处偏差:记忆读取失败用内联而非 Toast(理由见计划);处理 Todo 之外同时补了 `todo_detail_open` 与 Git 账户断开的本地记录写失败(此前只有日志)。

- [ ] **Step 2: 总验证**

```bash
cargo build --workspace 2>&1 | rg "generated|^error|Finished"
cargo test -p dozer-app 2>&1 | rg "test result|FAILED"
cargo test -p dozer-app -- --ignored keyring_store_delete_of_missing_entry_is_ok 2>&1 | rg "test result|FAILED"
scripts/check-log-scope.sh
cargo clippy -p dozer-app --all-targets 2>&1 | rg "generated"
cargo fmt --check && echo FMT_OK
```

Expected:构建通过、编译警告 5 个(基线);测试只有已知基线问题;`--ignored` 的真钥匙串测试通过(删除不存在的条目返回 `Ok`);门禁 ok;clippy 无新增;fmt ok。

- [ ] **Step 3: 手工验收清单(交给用户)**

- SSH/数据库面板:让钥匙串不可写(例如钥匙串被锁定且拒绝解锁)时保存带密码的主机/数据源 → 出现红色 Toast"密码未能保存到系统钥匙串: …",**主机/数据源本身仍保存成功**。
- 删除一台密钥认证的 SSH 主机(从没存过密码)→ **不出现**任何警告。
- 设置 → Git 账户 → 断开:正常时无提示;本地记录目录只读时 → Toast"断开账户失败: …"且仍显示已连接。
- Todo 详情"处理"/"打开",记忆详情:见 Task 6、7 的手工验收。

- [ ] **Step 4: 提交并交接审阅**

```bash
git add docs/superpowers/specs/2026-09-30-silent-failure-audit.md
git commit -m "docs: mark silent-failure batch 1 as landed

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

不要自行合并 `feat/silent-failures-batch1`;交给用户审阅后再合并(合并前先看主 checkout 是否有别的会话的未提交改动/并发提交,合并后重跑全量构建)。

## 实施中发现的计划勘误

- **Task 7 的 `emit(memory_detail_message(client.get_memory(..).await))` 写法会让 future 不是 `Send`**:`emit` 是 `Fn + Send`(非 `Sync`),调用表达式先借用 `emit` 再 `await` 参数,`&emit` 就横跨了 await。实施时改为先 `let res = client.get_memory(..).await;` 再 `emit(memory_detail_message(res))`。以后在 `spawn` 的异步块里给 `emit` 传带 `.await` 的参数都要先绑定。
- **Task 5 的测试辅助**:`settings.rs` 已有 `test_state(github, gitlab, gitee)`,新测试复用它(加了 `outbox` 字段初始化),没有另造 `State` 字面量。

## 已知边界

- **记忆详情的内联错误不会自动清除**:`ws_state.error` 由下一次成功的记忆操作(`MemoryMutated(Ok)`)清掉。读取失败后用户重新点开记忆成功,提示不会自己消失,直到下一次保存/删除成功。这与该面板既有"保存记忆失败"的行为一致,本批不改;若你觉得别扭,可在 `MemoryDetailLoaded` 里顺手清 `error`(一行,但要先确认不会误清"保存记忆失败"的提示)。
- **`App` 层后台任务的接线没有单测**(`todo_detail_open`/`todo_detail_process`):`App` 缺便宜的测试夹具,只测了 `failure_message` 这个纯函数,接线靠手工验收。
- **Task 3/4 只测了"给 store 的参数与失败反馈",没测 `update` 分支本身**:分支里只剩一行调用,且 `update` 需要一整套运行时参数。
