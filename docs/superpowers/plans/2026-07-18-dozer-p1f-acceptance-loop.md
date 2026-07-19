# Dozer P1f 验收闭环纵向薄片 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 跑通最简"定标(goal.md) → 交付横幅(TurnEnded+有变更) → 验收 tab(标准勾选+文件清单+意见+通过/打回) → 沉淀(git ref + SQLite 记录)"闭环。

**Architecture:** git 操作全走 git CLI、全在 GUI 侧（tokio 任务里跑，结果经 EventLoopProxy 回 UI 线程）；验收 tab 挂进左二预览域（`TabKind::Acceptance`，不产 webview——激活时全体 webview 隐藏，iced 直绘内容区）；验收记录经既有 UDS 协议新增 `RecordAcceptance` 落 dozerd 侧 SQLite（rusqlite bundled）；意见输入复用地址栏的自绘输入 + main.rs 键盘路由模式。

**Tech Stack:** Rust workspace、git CLI（std::process::Command）、rusqlite(bundled)、tokio、iced 0.14、tempfile（测试建真 git 仓库）。

**Spec:** `docs/superpowers/specs/2026-07-18-dozer-p1f-acceptance-loop-design.md`

## Global Constraints

- 金 `theme::GOLD` 专属甲方动作（横幅、通过按钮、金勾）；打回按钮红描边 `theme::RED`；错误红字域内显示。
- dozerd 不碰 git（D4）；GUI 线程不跑 git CLI（spawn 到 tokio，结果经 proxy 消息回来）。
- 薄片不设硬关口：goal.md 缺失只提示不拦截（规格 §8 未决项）。
- 工作区脏时"通过"必须被阻止（D6：沉淀物必须是提交）。
- `acceptances` 表含 `acceptor` 字段，一期恒 `"user"`（四期留门）。
- 测试全部 headless；GUI 行为归 Task 6 人工验收。
- commit 信息中文、结尾 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`；每 Task 收尾 `cargo clippy --all-targets && cargo fmt` 零警告。

---

### Task 1: goal.rs——.dozer/goal.md 解析器

**Files:**
- Create: `crates/dozer-app/src/goal.rs`
- Modify: `crates/dozer-app/src/main.rs`（`mod osc;` 后加 `mod goal;`）

**Interfaces:**
- Produces:
  - `Goal { pub title: String, pub criteria: Vec<String> }`（Debug/Clone/PartialEq）
  - `parse_goal(md: &str) -> Option<Goal>`（全空白 → None）
  - `goal_path(repo: &std::path::Path) -> std::path::PathBuf`（`repo/.dozer/goal.md`）

- [x] **Step 1: 写失败测试**（`goal.rs` 尾部）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_title_and_criteria() {
        let md = "# 让预览支持 PDF\n\n说明文字。\n\n- [ ] cargo test 全绿\n- [x] 打开 PDF 不白屏\n- 普通列表项不算标准\n";
        let g = parse_goal(md).unwrap();
        assert_eq!(g.title, "让预览支持 PDF");
        assert_eq!(g.criteria, vec!["cargo test 全绿", "打开 PDF 不白屏"]);
    }

    #[test]
    fn title_without_hash_and_no_criteria() {
        let g = parse_goal("裸标题目标\n正文").unwrap();
        assert_eq!(g.title, "裸标题目标");
        assert!(g.criteria.is_empty());
    }

    #[test]
    fn blank_input_is_none() {
        assert!(parse_goal("").is_none());
        assert!(parse_goal("  \n\n \n").is_none());
    }

    #[test]
    fn goal_path_is_dot_dozer() {
        assert_eq!(
            goal_path(std::path::Path::new("/repo")),
            std::path::PathBuf::from("/repo/.dozer/goal.md")
        );
    }
}
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app goal`
Expected: 编译错误（模块/函数不存在）。

- [x] **Step 3: 最小实现**

```rust
//! `.dozer/goal.md` 定标文件解析（spec P1f D2）：
//! 第一个非空行=目标（去掉行首 `#` 与空白）；`- [ ]`/`- [x]` 列表项=验收标准。

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Goal {
    pub title: String,
    pub criteria: Vec<String>,
}

pub fn goal_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("goal.md")
}

pub fn parse_goal(md: &str) -> Option<Goal> {
    let mut title: Option<String> = None;
    let mut criteria = Vec::new();
    for line in md.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("- [ ]")
            .or_else(|| trimmed.strip_prefix("- [x]"))
        {
            criteria.push(rest.trim().to_string());
            continue;
        }
        if title.is_none() {
            title = Some(trimmed.trim_start_matches('#').trim().to_string());
        }
    }
    title.map(|title| Goal { title, criteria })
}
```

（main.rs 的 `mod osc;` 后加一行 `mod goal;`。）

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app goal`
Expected: 4 测试通过。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app
git commit -m "feat(验收): goal.md 定标解析器——标题 + checkbox 标准清单

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: delivery.rs——git CLI 封装（变更检测 / 沉淀 ref）

**Files:**
- Create: `crates/dozer-app/src/delivery.rs`
- Modify: `crates/dozer-app/src/main.rs`（`mod goal;` 后加 `mod delivery;`）
- Modify: `crates/dozer-app/Cargo.toml`（dev-dependencies 加 `tempfile = "3"`）

**Interfaces:**
- Produces:
  - `FileChange { pub path: String, pub added: Option<u32>, pub removed: Option<u32> }`（None=二进制或未跟踪）
  - `repo_root(dir: &Path) -> Option<PathBuf>`
  - `is_dirty(repo: &Path) -> bool`
  - `head_commit(repo: &Path) -> Option<String>`
  - `last_accepted(repo: &Path) -> Option<(u32, String)>`（最大号 ref 及其 commit）
  - `changes(repo: &Path) -> Vec<FileChange>`（基线=最大 accepted ref，无则 HEAD；含未跟踪文件）
  - `accept(repo: &Path) -> anyhow::Result<u32>`（脏→Err；写 `refs/dozer/accepted/<n>`）
  - `delivery_pending(dirty: bool, head: Option<&str>, accepted: Option<&str>, last_turn_head: Option<&str>) -> bool`（纯函数，spec D3 精确定义）

- [x] **Step 1: 写失败测试**（`delivery.rs` 尾部；用真 git 仓库）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// tempdir 里造一个带一次提交的真 git 仓库
    fn mkrepo() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let st = Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(st.status.success(), "git {args:?}: {st:?}");
        };
        git(&["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "c1"]);
        (dir, repo)
    }

    #[test]
    fn repo_root_and_head_and_dirty() {
        let (_d, repo) = mkrepo();
        let sub = repo.join("sub");
        std::fs::create_dir(&sub).unwrap();
        assert_eq!(
            repo_root(&sub).unwrap().canonicalize().unwrap(),
            repo.canonicalize().unwrap()
        );
        assert!(repo_root(std::path::Path::new("/")).is_none());
        assert!(head_commit(&repo).is_some());
        assert!(!is_dirty(&repo));
        std::fs::write(repo.join("a.txt"), "two\n").unwrap();
        assert!(is_dirty(&repo));
    }

    #[test]
    fn accept_writes_sequential_refs_and_refuses_dirty() {
        let (_d, repo) = mkrepo();
        assert!(last_accepted(&repo).is_none());
        assert_eq!(accept(&repo).unwrap(), 1);
        let (n, commit) = last_accepted(&repo).unwrap();
        assert_eq!(n, 1);
        assert_eq!(commit, head_commit(&repo).unwrap());
        // 再提交一轮 → v2
        std::fs::write(repo.join("b.txt"), "x\n").unwrap();
        let git = |args: &[&str]| {
            Command::new("git").args(args).current_dir(&repo).output().unwrap()
        };
        git(&["add", "."]);
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "c2"]);
        assert_eq!(accept(&repo).unwrap(), 2);
        // 脏工作区拒绝
        std::fs::write(repo.join("a.txt"), "dirty\n").unwrap();
        assert!(accept(&repo).is_err());
    }

    #[test]
    fn changes_lists_modified_and_untracked_against_baseline() {
        let (_d, repo) = mkrepo();
        accept(&repo).unwrap(); // 基线 v1
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap(); // 修改
        std::fs::write(repo.join("new.txt"), "n\n").unwrap(); // 未跟踪
        let ch = changes(&repo);
        let a = ch.iter().find(|c| c.path == "a.txt").expect("a.txt");
        assert_eq!(a.added, Some(1));
        assert_eq!(a.removed, Some(0));
        let n = ch.iter().find(|c| c.path == "new.txt").expect("new.txt");
        assert_eq!(n.added, None, "未跟踪无行数");
    }

    #[test]
    fn delivery_pending_matrix() {
        // 脏 → 恒 pending
        assert!(delivery_pending(true, Some("h"), None, Some("h")));
        // 有沉淀 ref：HEAD 偏离才 pending
        assert!(delivery_pending(false, Some("h2"), Some("h1"), None));
        assert!(!delivery_pending(false, Some("h1"), Some("h1"), None));
        // 无 ref：与上回合 HEAD 比
        assert!(delivery_pending(false, Some("h2"), None, Some("h1")));
        assert!(!delivery_pending(false, Some("h1"), None, Some("h1")));
    }
}
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app delivery`
Expected: 编译错误（模块不存在）。

- [x] **Step 3: 最小实现**

```rust
//! 交付检测与沉淀：git CLI 薄封装（spec P1f D3/D4/D6）。
//! 全部同步阻塞——调用方负责放进 tokio 任务，不许在 UI 线程直呼。

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const ACCEPTED_REF_PREFIX: &str = "refs/dozer/accepted/";

#[derive(Debug, Clone, PartialEq)]
pub struct FileChange {
    pub path: String,
    /// None = 二进制或未跟踪（numstat 给不出行数）
    pub added: Option<u32>,
    pub removed: Option<u32>,
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).current_dir(repo).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let out = git(dir, &["rev-parse", "--show-toplevel"])?;
    let line = out.lines().next()?.trim();
    (!line.is_empty()).then(|| PathBuf::from(line))
}

pub fn is_dirty(repo: &Path) -> bool {
    git(repo, &["status", "--porcelain"]).is_some_and(|s| !s.trim().is_empty())
}

pub fn head_commit(repo: &Path) -> Option<String> {
    let out = git(repo, &["rev-parse", "HEAD"])?;
    let line = out.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

/// 现存最大号 accepted ref：`(n, commit)`。
pub fn last_accepted(repo: &Path) -> Option<(u32, String)> {
    let out = git(
        repo,
        &["for-each-ref", "--format=%(refname) %(objectname)", ACCEPTED_REF_PREFIX],
    )?;
    out.lines()
        .filter_map(|l| {
            let (name, commit) = l.split_once(' ')?;
            let n: u32 = name.strip_prefix(ACCEPTED_REF_PREFIX)?.parse().ok()?;
            Some((n, commit.to_string()))
        })
        .max_by_key(|(n, _)| *n)
}

/// 变更清单：基线=最大 accepted ref（无则 HEAD）对工作区 numstat + 未跟踪文件。
pub fn changes(repo: &Path) -> Vec<FileChange> {
    let mut out = Vec::new();
    let base = last_accepted(repo)
        .map(|(n, _)| format!("{ACCEPTED_REF_PREFIX}{n}"))
        .or_else(|| head_commit(repo).map(|_| "HEAD".to_string()));
    if let Some(base) = base
        && let Some(numstat) = git(repo, &["diff", "--numstat", &base])
    {
        for line in numstat.lines() {
            let mut parts = line.split('\t');
            let (Some(a), Some(r), Some(p)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            out.push(FileChange {
                path: p.to_string(),
                added: a.parse().ok(),
                removed: r.parse().ok(),
            });
        }
    }
    if let Some(status) = git(repo, &["status", "--porcelain"]) {
        for line in status.lines() {
            if let Some(p) = line.strip_prefix("?? ") {
                out.push(FileChange {
                    path: p.trim().to_string(),
                    added: None,
                    removed: None,
                });
            }
        }
    }
    out
}

/// 通过·沉淀：脏工作区拒绝（沉淀物必须是提交，spec D6）。
pub fn accept(repo: &Path) -> Result<u32> {
    if is_dirty(repo) {
        bail!("有未提交变更，先让 agent 提交再沉淀");
    }
    let head = head_commit(repo).context("仓库没有任何提交")?;
    let n = last_accepted(repo).map(|(n, _)| n + 1).unwrap_or(1);
    let refname = format!("{ACCEPTED_REF_PREFIX}{n}");
    let ok = Command::new("git")
        .args(["update-ref", &refname, &head])
        .current_dir(repo)
        .status()
        .context("git update-ref")?
        .success();
    if !ok {
        bail!("写沉淀 ref 失败: {refname}");
    }
    Ok(n)
}

/// spec D3 的"有变更"精确定义（纯函数，方便矩阵测试）。
pub fn delivery_pending(
    dirty: bool,
    head: Option<&str>,
    accepted: Option<&str>,
    last_turn_head: Option<&str>,
) -> bool {
    if dirty {
        return true;
    }
    match accepted {
        Some(a) => head != Some(a),
        None => head != last_turn_head,
    }
}
```

（main.rs 加 `mod delivery;`；`Cargo.toml` dev-dependencies 加 `tempfile = "3"`。）

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app delivery`
Expected: 4 测试通过。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app
git commit -m "feat(验收): delivery.rs git CLI 封装——变更检测/numstat/沉淀 ref/pending 判定

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: RecordAcceptance 协议 + dozerd SQLite 存储

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`
- Create: `crates/dozerd/src/acceptance.rs`
- Modify: `crates/dozerd/src/lib.rs`（加 `pub mod acceptance;`）
- Modify: `crates/dozerd/src/server.rs`（`serve` 增 store 参数 + 分支）
- Modify: `crates/dozerd/src/main.rs`（构造 store）
- Modify: `crates/dozerd/Cargo.toml`（`rusqlite = { version = "0.32", features = ["bundled"] }`）
- Modify: `crates/dozerd/tests/session_survival.rs`、`crates/dozerd/tests/hook_events.rs`（serve 调用点补参数）

**Interfaces:**
- Produces:
  - `Request::RecordAcceptance { repo: String, goal: String, criteria_checked: Vec<String>, verdict: String, comment: String, ref_name: String, ts_ms: u64 }` → `Reply::Ok`
  - `AcceptanceStore::open(path: &Path) -> anyhow::Result<AcceptanceStore>`
  - `AcceptanceStore::record(&self, r: &AcceptanceRecord) -> anyhow::Result<()>`
  - `AcceptanceStore::count(&self) -> anyhow::Result<u64>`（测试与后续演进史用）
  - `AcceptanceRecord { repo, goal, criteria_checked: Vec<String>, verdict, comment, ref_name, acceptor, ts_ms }`（String/u64 字段）
  - `dozerd::server::serve(socket, registry, store: Arc<AcceptanceStore>)`

- [x] **Step 1: 写失败测试**

`protocol.rs` tests 追加：

```rust
    #[test]
    fn record_acceptance_roundtrips() {
        let req = Request::RecordAcceptance {
            repo: "/r".into(),
            goal: "目标".into(),
            criteria_checked: vec!["测试全绿".into()],
            verdict: "accepted".into(),
            comment: "".into(),
            ref_name: "refs/dozer/accepted/1".into(),
            ts_ms: 9,
        };
        let back: Request = decode_line(encode_line(&req).trim()).unwrap();
        assert_eq!(back, req);
    }
```

`acceptance.rs` tests：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_record_count_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = AcceptanceStore::open(&dir.path().join("t.db")).unwrap();
        assert_eq!(store.count().unwrap(), 0);
        store
            .record(&AcceptanceRecord {
                repo: "/r".into(),
                goal: "目标".into(),
                criteria_checked: vec!["a".into(), "b".into()],
                verdict: "accepted".into(),
                comment: "好".into(),
                ref_name: "refs/dozer/accepted/1".into(),
                acceptor: "user".into(),
                ts_ms: 42,
            })
            .unwrap();
        assert_eq!(store.count().unwrap(), 1);
        // 重开库仍在（持久化）
        drop(store);
        let store = AcceptanceStore::open(&dir.path().join("t.db")).unwrap();
        assert_eq!(store.count().unwrap(), 1);
    }
}
```

`hook_events.rs` 追加端到端（并修两处 serve 调用点编译）：

```rust
#[tokio::test]
async fn record_acceptance_persists() {
    let sock = std::env::temp_dir().join(format!("dozerd-acc-{}.sock", uuid::Uuid::new_v4()));
    let db = std::env::temp_dir().join(format!("dozerd-acc-{}.db", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(&db).unwrap());
    tokio::spawn({
        let (sock, registry, store) = (sock.clone(), registry.clone(), store.clone());
        async move { dozerd::server::serve(&sock, registry, store).await }
    });
    for _ in 0..100 {
        if sock.exists() { break; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    match send_req(&sock, &Request::RecordAcceptance {
        repo: "/r".into(), goal: "g".into(), criteria_checked: vec![],
        verdict: "accepted".into(), comment: "".into(),
        ref_name: "refs/dozer/accepted/1".into(), ts_ms: 1,
    }).await {
        Reply::Ok => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(store.count().unwrap(), 1);
}
```

（dozerd `[dev-dependencies]` 加 `tempfile = "3"`。）

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-core && cargo test -p dozerd`
Expected: 编译错误（消息变体、acceptance 模块、serve 签名）。

- [x] **Step 3: 最小实现**

`protocol.rs` `Request` 追加：

```rust
    /// 验收通过的结构性记录（spec P1f D5）；acceptor 由 daemon 侧补 "user"。
    RecordAcceptance {
        repo: String,
        goal: String,
        criteria_checked: Vec<String>,
        verdict: String,
        comment: String,
        ref_name: String,
        ts_ms: u64,
    },
```

`crates/dozerd/src/acceptance.rs`：

```rust
//! 验收记录存储：rusqlite 单表（spec P1f D5）。
//! Mutex<Connection>——写入频度极低（人工验收动作），无需连接池。

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub struct AcceptanceRecord {
    pub repo: String,
    pub goal: String,
    pub criteria_checked: Vec<String>,
    pub verdict: String,
    pub comment: String,
    pub ref_name: String,
    pub acceptor: String,
    pub ts_ms: u64,
}

pub struct AcceptanceStore {
    conn: Mutex<Connection>,
}

impl AcceptanceStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS acceptances (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo TEXT NOT NULL,
                goal TEXT NOT NULL,
                criteria_checked TEXT NOT NULL, -- JSON array
                verdict TEXT NOT NULL,
                comment TEXT NOT NULL,
                ref_name TEXT NOT NULL,
                acceptor TEXT NOT NULL,
                ts_ms INTEGER NOT NULL
            );",
        )
        .context("建表")?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn record(&self, r: &AcceptanceRecord) -> Result<()> {
        let criteria = serde_json::to_string(&r.criteria_checked)?;
        self.conn.lock().expect("db lock").execute(
            "INSERT INTO acceptances
             (repo, goal, criteria_checked, verdict, comment, ref_name, acceptor, ts_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![r.repo, r.goal, criteria, r.verdict, r.comment, r.ref_name, r.acceptor, r.ts_ms],
        )?;
        Ok(())
    }

    pub fn count(&self) -> Result<u64> {
        let n: u64 = self
            .conn
            .lock()
            .expect("db lock")
            .query_row("SELECT COUNT(*) FROM acceptances", [], |row| row.get(0))?;
        Ok(n)
    }
}
```

`server.rs`：`serve` 签名与转发：

```rust
pub async fn serve(
    socket: &Path,
    registry: Arc<SessionRegistry>,
    store: Arc<crate::acceptance::AcceptanceStore>,
) -> Result<()> {
```

（accept 循环里 `let store = store.clone();` 随 registry 一起 move 进 `handle_conn(stream, registry, store)`；`handle_conn` 同样加参数。）请求分支（`HookEvent` 之后）：

```rust
                        Request::RecordAcceptance {
                            repo, goal, criteria_checked, verdict, comment, ref_name, ts_ms,
                        } => {
                            let rec = crate::acceptance::AcceptanceRecord {
                                repo, goal, criteria_checked, verdict, comment, ref_name,
                                acceptor: "user".into(), // 一期单人;四期多成员在此扩展
                                ts_ms,
                            };
                            match store.record(&rec) {
                                Ok(()) => Reply::Ok,
                                Err(e) => Reply::Error { message: format!("验收记录落库失败: {e}") },
                            }
                        }
```

`dozerd/src/main.rs`：registry 构造后加：

```rust
    let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
```

（`serve(&socket, registry)` 改 `serve(&socket, registry, store)`；`session_survival.rs` 与 `hook_events.rs` 原有 serve 调用点补 store 参数——各测试用 tempdir 库文件。）

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-core && cargo test -p dozerd`
Expected: 全绿（含新增 3 测试）。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-core crates/dozerd Cargo.lock
git commit -m "feat(验收): RecordAcceptance 协议 + dozerd SQLite 验收记录存储(acceptor 四期留门)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: 交付横幅——TurnEnded 触发检测 + 终端栏 UI

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`（`record_acceptance` 方法，为 Task 5 备）
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: Task 2 `delivery::{repo_root, is_dirty, head_commit, last_accepted, delivery_pending}`；P1e `Message::AgentStateChanged`/`AgentState::TurnEnded`。
- Produces:
  - `Client::record_acceptance(&self, repo,goal,criteria_checked,verdict,comment,ref_name,ts_ms) -> Result<()>`
  - `Message::DeliveryChecked(usize, bool)`（tab_id, pending）
  - `Message::AcceptanceOpen(usize)`（tab_id；Task 5 实现打开动作，本任务先只灭横幅占位）
  - `SessionTab.delivery_pending: bool`、`SessionTab.last_turn_head: Option<String>`
  - 终端栏金横幅（active tab pending 时显示）

- [x] **Step 1: 写失败测试**（`workspace.rs` tests 追加；横幅判定逻辑已在 delivery_pending 覆盖，这里测 update 分支的状态落地——Workspace 无法 headless 构造，退而测纯函数 `banner_text`）

```rust
    #[test]
    fn banner_text_for_pending() {
        assert_eq!(banner_text(true), Some("交付待验收"));
        assert_eq!(banner_text(false), None);
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app banner`
Expected: 编译错误（`banner_text` 未定义）。

- [x] **Step 3: 实现**

`dozer-client/src/lib.rs` `impl Client` 追加：

```rust
    #[allow(clippy::too_many_arguments)]
    pub async fn record_acceptance(
        &self,
        repo: &str,
        goal: &str,
        criteria_checked: &[String],
        verdict: &str,
        comment: &str,
        ref_name: &str,
        ts_ms: u64,
    ) -> Result<()> {
        match self
            .roundtrip(&Request::RecordAcceptance {
                repo: repo.into(),
                goal: goal.into(),
                criteria_checked: criteria_checked.to_vec(),
                verdict: verdict.into(),
                comment: comment.into(),
                ref_name: ref_name.into(),
                ts_ms,
            })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

`workspace.rs`：

1. `use crate::delivery;`
2. `Message` 追加：

```rust
    /// TurnEnded 触发的交付检测结果（tab_id, 是否有待验收交付）。
    DeliveryChecked(usize, bool),
    /// 点击横幅"进入验收"（tab_id 为来源会话）。
    AcceptanceOpen(usize),
```

3. `SessionTab` 加字段（构造处初始化 `delivery_pending: false, last_turn_head: None,`）：

```rust
    /// TurnEnded 检测出的"交付待验收"标记（spec P1f D3）。
    pub delivery_pending: bool,
    /// 上一次 TurnEnded 时的 HEAD（无沉淀 ref 时的比对基线）。
    pub last_turn_head: Option<String>,
```

4. `Message::AgentStateChanged` 分支扩展为：

```rust
            Message::AgentStateChanged(tab_id, state) => {
                if let Some(tab) = self.tab_by_id_mut(tab_id) {
                    tab.agent_state = state;
                    if state == AgentState::TurnEnded {
                        // git 检测不许在 UI 线程跑：丢 tokio,结果经 proxy 回来
                        let cwd = PathBuf::from(tab.info.cwd.clone());
                        let last_turn = tab.last_turn_head.clone();
                        let proxy = self.proxy.clone();
                        self.handle.spawn(async move {
                            let pending = tokio::task::spawn_blocking(move || {
                                let Some(repo) = delivery::repo_root(&cwd) else {
                                    return None; // 非 git 仓库:不参与闭环
                                };
                                let dirty = delivery::is_dirty(&repo);
                                let head = delivery::head_commit(&repo);
                                let accepted = delivery::last_accepted(&repo).map(|(_, c)| c);
                                Some(delivery::delivery_pending(
                                    dirty,
                                    head.as_deref(),
                                    accepted.as_deref(),
                                    last_turn.as_deref(),
                                ))
                            })
                            .await
                            .ok()
                            .flatten();
                            if let Some(pending) = pending {
                                let _ = proxy.send_event(Message::DeliveryChecked(tab_id, pending));
                            }
                        });
                    }
                }
            }
```

5. `update` 追加分支：

```rust
            Message::DeliveryChecked(tab_id, pending) => {
                if let Some(tab) = self.tab_by_id_mut(tab_id) {
                    tab.delivery_pending = pending;
                    // 记录本回合 HEAD 供下回合比对（同步读一次可容忍:仅 rev-parse）
                    let cwd = PathBuf::from(tab.info.cwd.clone());
                    if let Some(repo) = delivery::repo_root(&cwd) {
                        tab.last_turn_head = delivery::head_commit(&repo);
                    }
                }
            }
            Message::AcceptanceOpen(tab_id) => {
                // Task 5 接真实打开;本任务先灭横幅占位,保证按钮可点不 panic
                if let Some(tab) = self.tab_by_id_mut(tab_id) {
                    tab.delivery_pending = false;
                }
            }
```

6. 文件级纯函数 + `terminal_pane` 接线（`exit` 提示之后）：

```rust
/// 交付横幅文案：pending 才有（金色,甲方动作）。
fn banner_text(pending: bool) -> Option<&'static str> {
    pending.then_some("交付待验收")
}
```

```rust
    // 交付横幅（spec P1f D3）:金字金框,CTA 进入验收
    if let Some(tab) = ws.tabs.get(ws.active)
        && let Some(text_str) = banner_text(tab.delivery_pending)
    {
        let banner = row![
            text(text_str).size(12).color(theme::GOLD),
            button(text("进入验收").size(12).color(theme::GOLD))
                .on_press(Message::AcceptanceOpen(tab.tab_id))
                .style(|_t, _s| button::Style {
                    background: Some(theme::CARD.into()),
                    text_color: theme::GOLD,
                    border: Border { color: theme::GOLD, width: 1.0, radius: 2.0.into() },
                    ..button::Style::default()
                }),
        ]
        .spacing(8);
        content = content.push(banner);
    }
```

**注**：`DeliveryChecked` 里的同步 `repo_root`/`head_commit` 是两次本地 `git rev-parse`（毫秒级），在 UI 线程可容忍；若实测卡顿改为把 head 一并随消息带回。

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app && cargo test -p dozer-client`
Expected: 全绿。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app crates/dozer-client
git commit -m "feat(验收): 交付横幅——TurnEnded 异步 git 检测 + 终端栏金色 CTA

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: 验收 tab——预览域挂载 + 勾选/意见/通过/打回

**Files:**
- Modify: `crates/dozer-app/src/preview.rs`（`TabKind::Acceptance` + webview 过滤）
- Modify: `crates/dozer-app/src/workspace.rs`（AcceptanceView 状态 + 消息 + 渲染 + 动作）
- Modify: `crates/dozer-app/src/main.rs`（键盘路由扩展到意见输入）

**Interfaces:**
- Consumes: Task 1 `goal::{parse_goal, goal_path, Goal}`；Task 2 `delivery::{repo_root, changes, accept, FileChange, ACCEPTED_REF_PREFIX}`；Task 4 `Client::record_acceptance`、`Message::AcceptanceOpen`。
- Produces:
  - `TabKind::Acceptance`（无 webview）；`PreviewPane::open_acceptance() -> usize`（复用即有则激活）
  - `Workspace::acceptance_comment_editing(&self) -> bool`（main.rs 键盘路由用）
  - `Message::{AcceptanceLoaded(...), AcceptanceToggle(usize), AcceptanceCommentClick, AcceptanceCommentEvent(AddrEvent), AcceptanceAccept, AcceptanceReject, AcceptanceDone(Result<u32, String>)}`

- [x] **Step 1: 写失败测试**

`preview.rs` tests 追加：

```rust
    #[test]
    fn acceptance_tab_produces_no_webview_and_hides_others() {
        let mut p = PreviewPane::default();
        p.open_url("http://localhost:3000".into());
        let acc_id = p.open_acceptance();
        let specs = p.desired_webviews();
        assert_eq!(specs.len(), 1, "验收 tab 不产 webview");
        assert!(!specs[0].visible, "验收 tab 激活时其余全隐藏");
        // 重复打开复用同一 tab
        assert_eq!(p.open_acceptance(), acc_id);
        assert_eq!(p.tabs().len(), 2);
        // 切回网页 tab → webview 复显
        p.select(0);
        assert!(p.desired_webviews()[0].visible);
    }
```

`workspace.rs` tests 追加：

```rust
    #[test]
    fn criteria_check_line_renders_gold_check() {
        assert_eq!(criteria_line(true, "测试全绿"), "✓ 测试全绿");
        assert_eq!(criteria_line(false, "测试全绿"), "○ 测试全绿");
    }

    #[test]
    fn file_change_line_formats_counts() {
        use crate::delivery::FileChange;
        let fc = FileChange { path: "src/a.rs".into(), added: Some(3), removed: Some(1) };
        assert_eq!(file_change_line(&fc), "src/a.rs  +3 −1");
        let un = FileChange { path: "new.txt".into(), added: None, removed: None };
        assert_eq!(file_change_line(&un), "new.txt  (新)");
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app acceptance criteria file_change`
Expected: 编译错误（`open_acceptance`/`criteria_line`/`file_change_line` 未定义）。

- [x] **Step 3: 实现**

`preview.rs`：

```rust
// TabKind 追加变体（文件头 P1f 注释同步删掉"留给 P1f 补"字样）
    /// 验收 tab（P1f）:不产 webview,内容由 iced 直绘。
    Acceptance,
```

```rust
    /// 打开验收 tab:已存在则激活复用（全局至多一个）。
    pub fn open_acceptance(&mut self) -> usize {
        if let Some((idx, tab)) = self
            .tabs
            .iter()
            .enumerate()
            .find(|(_, t)| t.kind == TabKind::Acceptance)
        {
            let id = tab.id;
            self.active = idx;
            return id;
        }
        self.push_tab(TabKind::Acceptance, "验收".to_string())
    }

    /// 当前激活 tab 是否验收 tab。
    pub fn acceptance_active(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|t| t.kind == TabKind::Acceptance)
    }
```

`desired_webviews` 改为（验收 tab 无 webview、激活时全隐藏）：

```rust
    pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
        let acceptance_active = self.acceptance_active();
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(idx, tab)| {
                let url = match &tab.kind {
                    TabKind::File(path) => format!(
                        "dozer://flyfish/host.html?p={}",
                        encode_component(&path.to_string_lossy())
                    ),
                    TabKind::Web { url } => url.clone(),
                    TabKind::Acceptance => return None,
                };
                Some(WebviewSpec {
                    id: tab.id,
                    url,
                    visible: idx == self.active && !acceptance_active,
                })
            })
            .collect()
    }
```

`workspace.rs`：

1. `use crate::delivery::{self, FileChange};`、`use crate::goal::{self, Goal};`
2. Workspace 加字段（两处构造初始化 `acceptance: None,`）与状态结构：

```rust
/// 验收 tab 的一次进行中验收（spec P1f D8）。
pub struct AcceptanceView {
    pub repo: PathBuf,
    /// 来源会话 tab（打回意见注回目标）。
    pub source_tab_id: usize,
    pub goal: Option<Goal>,
    pub changes: Vec<FileChange>,
    pub checked: Vec<bool>,
    pub comment: String,
    pub comment_editing: bool,
    pub error: Option<String>,
    /// 通过后记录版本号（显示"已沉淀 v<n>"）。
    pub accepted_version: Option<u32>,
}
```

```rust
    /// 进行中的验收（验收 tab 内容;None=未打开）。
    acceptance: Option<AcceptanceView>,
```

3. `Message` 追加：

```rust
    /// 验收数据装载完成（repo, 来源 tab_id, goal, 变更清单）。
    AcceptanceLoaded(PathBuf, usize, Option<Goal>, Vec<FileChange>),
    /// 勾选/取消第 n 条标准。
    AcceptanceToggle(usize),
    /// 点击意见输入框进入编辑态。
    AcceptanceCommentClick,
    /// 意见输入事件（main.rs 键盘路由送入,复用 AddrEvent）。
    AcceptanceCommentEvent(AddrEvent),
    /// 点"通过·沉淀"。
    AcceptanceAccept,
    /// 点"打回并注回"。
    AcceptanceReject,
    /// 通过动作结果（Ok(版本号)/Err(红字文案)）。
    AcceptanceDone(Result<u32, String>),
```

4. `Message::AcceptanceOpen` 替换 Task 4 占位：

```rust
            Message::AcceptanceOpen(tab_id) => {
                let Some(tab) = self.tab_by_id_mut(tab_id) else { return };
                tab.delivery_pending = false;
                let cwd = PathBuf::from(tab.info.cwd.clone());
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let loaded = tokio::task::spawn_blocking(move || {
                        let repo = delivery::repo_root(&cwd)?;
                        let goal = std::fs::read_to_string(goal::goal_path(&repo))
                            .ok()
                            .and_then(|md| goal::parse_goal(&md));
                        let changes = delivery::changes(&repo);
                        Some((repo, goal, changes))
                    })
                    .await
                    .ok()
                    .flatten();
                    if let Some((repo, goal, changes)) = loaded {
                        let _ = proxy
                            .send_event(Message::AcceptanceLoaded(repo, tab_id, goal, changes));
                    }
                });
            }
```

5. `update` 追加分支：

```rust
            Message::AcceptanceLoaded(repo, source_tab_id, goal, changes) => {
                let n = goal.as_ref().map(|g| g.criteria.len()).unwrap_or(0);
                self.acceptance = Some(AcceptanceView {
                    repo,
                    source_tab_id,
                    goal,
                    changes,
                    checked: vec![false; n],
                    comment: String::new(),
                    comment_editing: false,
                    error: None,
                    accepted_version: None,
                });
                self.preview.open_acceptance();
            }
            Message::AcceptanceToggle(i) => {
                if let Some(acc) = &mut self.acceptance
                    && let Some(c) = acc.checked.get_mut(i)
                {
                    *c = !*c;
                }
            }
            Message::AcceptanceCommentClick => {
                if let Some(acc) = &mut self.acceptance {
                    acc.comment_editing = true;
                }
            }
            Message::AcceptanceCommentEvent(ev) => {
                if let Some(acc) = &mut self.acceptance {
                    match ev {
                        AddrEvent::Text(s) => acc.comment.push_str(&s),
                        AddrEvent::Backspace => {
                            acc.comment.pop();
                        }
                        AddrEvent::Submit | AddrEvent::Cancel => acc.comment_editing = false,
                    }
                }
            }
            Message::AcceptanceAccept => {
                let Some(acc) = &mut self.acceptance else { return };
                acc.error = None;
                let repo = acc.repo.clone();
                let goal_title = acc.goal.as_ref().map(|g| g.title.clone()).unwrap_or_default();
                let checked: Vec<String> = acc
                    .goal
                    .as_ref()
                    .map(|g| {
                        g.criteria
                            .iter()
                            .zip(&acc.checked)
                            .filter(|(_, c)| **c)
                            .map(|(s, _)| s.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                let comment = acc.comment.clone();
                let client = self.client.clone();
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let repo2 = repo.clone();
                    let accepted =
                        tokio::task::spawn_blocking(move || delivery::accept(&repo2)).await;
                    let result = match accepted {
                        Ok(Ok(n)) => {
                            let ref_name = format!("{}{n}", delivery::ACCEPTED_REF_PREFIX);
                            let ts_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0);
                            if let Err(e) = client
                                .record_acceptance(
                                    &repo.to_string_lossy(),
                                    &goal_title,
                                    &checked,
                                    "accepted",
                                    &comment,
                                    &ref_name,
                                    ts_ms,
                                )
                                .await
                            {
                                // ref 已写成立（真相源）,库失败只提示（spec §3）
                                Err(format!("已沉淀 v{n},但记录落库失败: {e}"))
                            } else {
                                Ok(n)
                            }
                        }
                        Ok(Err(e)) => Err(e.to_string()),
                        Err(e) => Err(format!("任务失败: {e}")),
                    };
                    let _ = proxy.send_event(Message::AcceptanceDone(result));
                });
            }
            Message::AcceptanceReject => {
                let Some(acc) = &mut self.acceptance else { return };
                acc.error = None;
                let comment = acc.comment.trim().to_string();
                let source = acc.source_tab_id;
                let Some(tab) = self.tabs.iter().find(|t| t.tab_id == source) else {
                    if let Some(acc) = &mut self.acceptance {
                        acc.error = Some("会话已结束,意见无处可注".into());
                    }
                    return;
                };
                if !tab.alive {
                    if let Some(acc) = &mut self.acceptance {
                        acc.error = Some("会话已结束,意见无处可注".into());
                    }
                    return;
                }
                let id = tab.info.id.clone();
                let client = self.client.clone();
                let text_out = format!("[Dozer 验收打回] {comment}\n");
                self.handle.spawn(async move {
                    if let Err(e) = client.write(&id, text_out.as_bytes()).await {
                        tracing::warn!("打回注回失败: {e}");
                    }
                });
                // 关验收 tab（打回后回执行现场）
                if let Some(idx) = self
                    .preview
                    .tabs()
                    .iter()
                    .position(|t| t.kind == crate::preview::TabKind::Acceptance)
                {
                    self.preview.close(idx);
                }
                self.acceptance = None;
            }
            Message::AcceptanceDone(result) => {
                if let Some(acc) = &mut self.acceptance {
                    match result {
                        Ok(n) => acc.accepted_version = Some(n),
                        Err(e) => acc.error = Some(e),
                    }
                }
            }
```

6. 公开方法 + 渲染纯函数 + preview_pane 接线：

```rust
    /// 意见输入是否在编辑态（main.rs 键盘路由用）。
    pub fn acceptance_comment_editing(&self) -> bool {
        self.acceptance.as_ref().is_some_and(|a| a.comment_editing)
    }
```

```rust
/// 标准行文案:金勾 ✓ / 空圈 ○。
fn criteria_line(checked: bool, text: &str) -> String {
    format!("{} {}", if checked { "✓" } else { "○" }, text)
}

/// 变更文件行:path  +a −r;未跟踪标 (新)。
fn file_change_line(fc: &FileChange) -> String {
    match (fc.added, fc.removed) {
        (Some(a), Some(r)) => format!("{}  +{a} −{r}", fc.path),
        _ => format!("{}  (新)", fc.path),
    }
}
```

`preview_pane`（`ws.preview.tabs().is_empty()` 分支之前）插入验收内容渲染：

```rust
    if ws.preview.acceptance_active() {
        if let Some(acc) = &ws.acceptance {
            if let Some(n) = acc.accepted_version {
                content = content.push(
                    text(format!("✓ 已沉淀 v{n}")).size(14).color(theme::GOLD),
                );
            } else {
                match &acc.goal {
                    Some(g) => {
                        content = content.push(text(g.title.clone()).size(14).color(theme::CREAM));
                        for (i, c) in g.criteria.iter().enumerate() {
                            let checked = acc.checked.get(i).copied().unwrap_or(false);
                            content = content.push(
                                button(text(criteria_line(checked, c)).size(12).color(
                                    if checked { theme::GOLD } else { theme::BODY },
                                ))
                                .on_press(Message::AcceptanceToggle(i))
                                .style(|_t, _s| button::Style {
                                    background: None,
                                    text_color: theme::BODY,
                                    ..button::Style::default()
                                }),
                            );
                        }
                    }
                    None => {
                        content = content.push(
                            text("未定标——先在仓库写 .dozer/goal.md（首行目标,\n- [ ] 列表为标准）")
                                .size(12)
                                .color(theme::DIM),
                        );
                    }
                }
                content = content.push(text("变更文件").size(12).color(theme::DIM));
                for fc in &acc.changes {
                    let path = acc.repo.join(&fc.path);
                    content = content.push(
                        button(text(file_change_line(fc)).size(12).color(theme::CYAN))
                            .on_press(Message::PreviewOpenPath(path))
                            .style(|_t, _s| button::Style {
                                background: None,
                                text_color: theme::CYAN,
                                ..button::Style::default()
                            }),
                    );
                }
                let editing = acc.comment_editing;
                let comment_text = if editing {
                    format!("{}▏", acc.comment)
                } else if acc.comment.is_empty() {
                    "验收意见…（打回时注回会话）".to_string()
                } else {
                    acc.comment.clone()
                };
                content = content.push(
                    button(text(comment_text).size(12).color(if editing {
                        theme::CREAM
                    } else {
                        theme::DIM
                    }))
                    .on_press(Message::AcceptanceCommentClick)
                    .width(Length::Fill)
                    .style(move |_t, _s| button::Style {
                        background: Some(theme::TERM_BG.into()),
                        text_color: theme::CREAM,
                        border: Border {
                            color: if editing { theme::GOLD } else { theme::BORDER },
                            width: 1.0,
                            radius: 2.0.into(),
                        },
                        ..button::Style::default()
                    }),
                );
                let actions = row![
                    button(text("通过·沉淀").size(12).color(theme::BG))
                        .on_press(Message::AcceptanceAccept)
                        .style(|_t, _s| button::Style {
                            background: Some(theme::GOLD.into()),
                            text_color: theme::BG,
                            border: Border { color: theme::GOLD, width: 1.0, radius: 2.0.into() },
                            ..button::Style::default()
                        }),
                    button(text("打回并注回").size(12).color(theme::RED))
                        .on_press(Message::AcceptanceReject)
                        .style(|_t, _s| button::Style {
                            background: None,
                            text_color: theme::RED,
                            border: Border { color: theme::RED, width: 1.0, radius: 2.0.into() },
                            ..button::Style::default()
                        }),
                ]
                .spacing(8);
                content = content.push(actions);
                if let Some(err) = &acc.error {
                    content = content.push(text(format!("⚠ {err}")).size(12).color(theme::RED));
                }
            }
        }
    }
```

`main.rs` 键盘路由：`if workspace.preview_addr_editing() {` 块整体改为双目标（结构不变，仅分发处按目标选消息）：

```rust
            let addr_target_preview = workspace.preview_addr_editing();
            let addr_target_comment = workspace.acceptance_comment_editing();
            if addr_target_preview || addr_target_comment {
                // ……（原有 addr_event 翻译 match 原样保留）……
                if let Some(ev) = addr_event {
                    let message = if addr_target_preview {
                        Message::PreviewAddrEvent(ev)
                    } else {
                        Message::AcceptanceCommentEvent(ev)
                    };
                    workspace.update(message);
                    window.request_redraw();
                }
                return;
            }
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app`
Expected: 全绿（preview 1 新测 + workspace 2 新测 + 既有）。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app
git commit -m "feat(验收): 验收 tab——预览域挂载 + 标准勾选/意见/通过沉淀/打回注回

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: 全量回归 + 端到端人工验收 + 落档

**Files:**
- Create: `docs/superpowers/specs/2026-07-18-p1f-acceptance.md`
- Modify: `docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`（验收通过后 §3 需求 3 回填）
- Modify: 本计划文件（勾选 checkbox）

**Interfaces:**
- Consumes: 全部前序任务。
- Produces: 用户签字的验收记录；P1g 起点状态。

- [x] **Step 1: 全量回归 + 冒烟**

```bash
cargo test && cargo clippy --all-targets && cargo fmt --check
cargo build -p dozer-app && ./target/aarch64-apple-darwin/debug/dozer & sleep 8 && kill %1
```

Expected: 全绿零警告；app 存活 8 秒。**注意：验收前 `pkill dozerd` 重启新 daemon**（P1e 教训：旧 daemon 版本偏斜）。

- [x] **Step 2: 用户人工验收（dogfooding 本仓；逐项 ✓/✗，验收权在用户）**

```markdown
# P1f 人工验收清单（用户实机执行）
1. 本仓写 `.dozer/goal.md`（首行目标 + 两条 `- [ ]` 标准）→ Dozer 里跑 claude 改点东西 → 回合毕，终端栏金横幅"交付待验收"亮
2. 纯问答一回合（不改文件）→ 横幅不亮
3. 点"进入验收" → 左二出现"验收" tab：目标、标准清单、变更文件（±行数）、意见框、双按钮；webview 内容不抢层
4. 点标准行 → 金勾 ✓ 切换；点变更文件 → Flyfish 打开该文件
5. 意见框输入中文 → 点"打回并注回" → 意见出现在来源会话 claude 输入里,验收 tab 关闭
6. 再让 agent 提交全部变更 → 回合毕横幅亮 → 进验收 → 通过·沉淀 → 显示"已沉淀 v1"；`git for-each-ref refs/dozer/accepted` 见 v1 指向 HEAD
7. 工作区留未提交变更时点"通过" → 红字阻止,ref 不写
8. `sqlite3 ~/Library/Application\ Support/ai.byteboy.dozer/dozer.db 'select verdict,ref_name,acceptor from acceptances'` → 见 accepted 记录,acceptor=user
```

- [x] **Step 3: 结果落档 + Commit**

验收记录写入 `docs/superpowers/specs/2026-07-18-p1f-acceptance.md`（沿用 P1c/d/e 格式：逐项结果表 + 反馈修复流水）；全部通过后规格 §3 需求 3 追加"P1f 达成（<日期>，验收闭环薄片：goal.md 定标 + 交付横幅 + 验收 tab + git ref 沉淀 + SQLite 记录；S0/S0b/S2 全屏态/S3 演进史视图/diff 渲染/机器预判随后续阶段），验收记录见 specs/2026-07-18-p1f-acceptance.md"。

```bash
git add docs
git commit -m "验收：P1f 验收闭环薄片人工验收记录 + 规格回填

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

## Self-Review（已执行）

1. **Spec 覆盖**：D1（项目=cwd 仓库）=T2 repo_root+T4 检测；D2（goal.md）=T1+T5 渲染与缺失提示；D3（交付判定精确定义）=T2 delivery_pending+T4 触发；D4（git CLI/GUI 侧）=T2 全部+T4/T5 spawn_blocking；D5（SQLite+acceptor 留门）=T3；D6（ref 沉淀+脏阻止）=T2 accept+T5 Accept 动作；D7（打回注回）=T5 Reject 动作；D8（验收 tab 复用预览域）=T5 preview 扩展。§3 错误处理五条：非 git（T4 返回 None 不亮）、goal 缺失（T5 None 分支提示）、脏阻止（T2 accept bail→T5 error 红字）、库失败 ref 成立（T5 Accept 的 Err 文案分支）、会话已死（T5 Reject 分支）。§4 人工验收清单=T6 Step 2。
2. **占位符扫描**：无 TBD；T4 的 `AcceptanceOpen` 占位分支在 T5 被显式替换（两边都给了完整代码）。
3. **类型一致性**：`Goal{title,criteria}`、`FileChange{path,added,removed}`、`delivery_pending(bool,Option<&str>,Option<&str>,Option<&str>)->bool`、`accept(&Path)->Result<u32>`、`ACCEPTED_REF_PREFIX`、`AcceptanceRecord{..8 字段}`、`serve(socket,registry,store)`、`record_acceptance(7 参)->Result<()>`、`TabKind::Acceptance`/`open_acceptance()->usize`/`acceptance_active()->bool`、`Message::Acceptance*` 系列在 T1-T5 交叉引用一致；`AddrEvent` 复用自 P1d 定义。
