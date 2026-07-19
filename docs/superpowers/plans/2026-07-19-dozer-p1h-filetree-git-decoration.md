# Dozer P1h 文件树 git 装饰 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 左一文件树按 git 状态给文件行上色 + 尾缀字符（改/新/删），目录 rollup（含深层变更即标金），打开项目与每次回合结束时刷新。

**Architecture:** 纯 GUI 侧，无协议改动。新增 `delivery::file_statuses`（解析 `git status --porcelain` 成绝对路径→状态）与 `dir_has_change`（rollup 纯函数）；workspace 存 `git_statuses` map，复用扩展后的 `ProjectGitRefreshed` 消息在打开项目/回合结束时刷新（GUI 侧 spawn_blocking）；装饰在 `project_pane` 渲染层查表叠加，`FileTree` 状态机不变。

**Tech Stack:** Rust、git CLI（复用 `delivery::git`）、iced 0.14、tempfile（测试真 git 仓库）。

**Spec:** `docs/superpowers/specs/2026-07-19-dozer-p1h-filetree-git-decoration-design.md`

## Global Constraints

- 上色沿用 P1g "金=有改动" 约定：`Modified`→`theme::GOLD`+`•`、`New`→`theme::GREEN`+`+`、`Deleted`→`theme::RED`+`−`；未变文件保持 `theme::CYAN`、未变目录 `theme::BODY`。
- git 调用在 GUI 侧 `spawn_blocking`，不阻塞 UI 线程；dozerd 不参与（哑管道）。
- 装饰是渲染层叠加，不进 `FileTree` 状态机（正交、刷新节奏不同）。
- 非 git 项目：`file_statuses` 返回空 map，树无装饰不崩。
- 测试全 headless；UI 上色归末任务人工验收。
- commit 中文、结尾 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`；每 Task 收尾 `cargo clippy --all-targets && cargo fmt` 零警告。

---

### Task 1: delivery.rs——file_statuses + dir_has_change

**Files:**
- Modify: `crates/dozer-app/src/delivery.rs`

**Interfaces:**
- Produces:
  - `FileStatus`（`Modified|New|Deleted`；Debug/Clone/Copy/PartialEq）
  - `file_statuses(repo: &Path) -> std::collections::HashMap<PathBuf, FileStatus>`（绝对路径→状态；非 git 返回空）
  - `dir_has_change(dir: &Path, changed: &[PathBuf]) -> bool`

- [x] **Step 1: 写失败测试**（`delivery.rs` 的 `mod tests` 追加）

```rust
    #[test]
    fn file_statuses_maps_modified_new_deleted() {
        let (_d, repo) = mkrepo(); // 已有 helper：含 a.txt 一次提交
        std::fs::write(repo.join("a.txt"), "changed\n").unwrap(); // 改
        std::fs::write(repo.join("new.txt"), "n\n").unwrap(); // 未跟踪
        std::fs::remove_file(repo.join("b.txt")).ok(); // b 不存在,忽略
        // 造一个已跟踪再删的:先加提交 c,再删
        std::fs::write(repo.join("c.txt"), "c\n").unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
        };
        git(&["add", "c.txt"]);
        git(&["commit", "-qm", "c"]);
        std::fs::remove_file(repo.join("c.txt")).unwrap(); // 删已跟踪

        let m = file_statuses(&repo);
        assert_eq!(m.get(&repo.join("a.txt")), Some(&FileStatus::Modified));
        assert_eq!(m.get(&repo.join("new.txt")), Some(&FileStatus::New));
        assert_eq!(m.get(&repo.join("c.txt")), Some(&FileStatus::Deleted));
        assert!(file_statuses(std::path::Path::new("/")).is_empty(), "非 git 空");
    }

    #[test]
    fn dir_has_change_prefix_match() {
        use std::path::PathBuf;
        let changed = vec![PathBuf::from("/r/src/a.rs"), PathBuf::from("/r/README.md")];
        assert!(dir_has_change(std::path::Path::new("/r/src"), &changed));
        assert!(dir_has_change(std::path::Path::new("/r"), &changed), "深层也命中");
        assert!(!dir_has_change(std::path::Path::new("/r/docs"), &changed));
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app file_statuses dir_has_change`
Expected: 编译错误（`FileStatus`/函数未定义）。

- [x] **Step 3: 实现**

`delivery.rs` 头部 `use` 增 `HashMap`（若无）：`use std::collections::HashMap;`（放到现有 `use` 区）。加类型与函数（放在 `branch` 附近）：

```rust
/// 文件树装饰用的 git 状态（P1h）。粗粒度三态,不分暂存/工作区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    New,
    Deleted,
}

/// `git status --porcelain` → 绝对路径 → 状态。非 git / 失败返回空。
/// 码映射:`??`/含 `A`→New;含 `D`→Deleted;其余→Modified。重命名取箭头后的新名。
pub fn file_statuses(repo: &Path) -> HashMap<PathBuf, FileStatus> {
    let mut map = HashMap::new();
    let Some(out) = git(repo, &["status", "--porcelain"]) else {
        return map;
    };
    for line in out.lines() {
        if line.len() < 4 {
            continue;
        }
        let code = &line[..2];
        let rest = &line[3..];
        let path = rest.rsplit(" -> ").next().unwrap_or(rest).trim();
        if path.is_empty() {
            continue;
        }
        let status = if code == "??" || code.contains('A') {
            FileStatus::New
        } else if code.contains('D') {
            FileStatus::Deleted
        } else {
            FileStatus::Modified
        };
        map.insert(repo.join(path), status);
    }
    map
}

/// 目录（含深层）下是否有任一变更路径（rollup 判定）。
pub fn dir_has_change(dir: &Path, changed: &[PathBuf]) -> bool {
    changed.iter().any(|c| c.starts_with(dir))
}
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app file_statuses dir_has_change`
Expected: 2 测试通过。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/delivery.rs
git commit -m "feat(项目): delivery::file_statuses(porcelain→状态) + dir_has_change(rollup)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: workspace 接入装饰——刷新 + 渲染

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: Task 1 `FileStatus`、`file_statuses`、`dir_has_change`。
- Produces:
  - `Workspace` 字段 `git_statuses: HashMap<PathBuf, FileStatus>`
  - `Message::ProjectGitRefreshed(Option<String>, bool, HashMap<PathBuf, FileStatus>)`（第三参新增）
  - `Workspace::spawn_project_git_refresh(&self)`
  - `decoration_for(status: FileStatus) -> (Color, &'static str)`（纯函数）

- [x] **Step 1: 写失败测试**（`workspace.rs` 的 `mod tests` 追加）

```rust
    #[test]
    fn decoration_maps_status_to_color_and_marker() {
        use crate::delivery::FileStatus;
        assert_eq!(decoration_for(FileStatus::Modified), (theme::GOLD, "•"));
        assert_eq!(decoration_for(FileStatus::New), (theme::GREEN, "+"));
        assert_eq!(decoration_for(FileStatus::Deleted), (theme::RED, "−"));
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app decoration_maps`
Expected: 编译错误（`decoration_for` 未定义）。

- [x] **Step 3: 实现**

`workspace.rs`：

1. 导入：`use crate::delivery::{self, FileChange, FileStatus};`（现为 `{self, FileChange}`，补 `FileStatus`）。

2. `Message::ProjectGitRefreshed(Option<String>, bool)` 改为：

```rust
    /// 项目:git 分支/脏/文件状态刷新结果。
    ProjectGitRefreshed(Option<String>, bool, HashMap<PathBuf, FileStatus>),
```

3. `Workspace` 加字段（两处构造 `bootstrap`/`with_daemon_error` 初始化 `git_statuses: HashMap::new(),`）：

```rust
    /// 当前项目的 git 文件状态（路径→状态；文件树装饰用）。
    git_statuses: HashMap<PathBuf, FileStatus>,
```

4. `ProjectGitRefreshed` handler 改为三参并存 statuses：

```rust
            Message::ProjectGitRefreshed(branch, dirty, statuses) => {
                self.branch = branch;
                self.dirty = dirty;
                self.git_statuses = statuses;
            }
```

5. 抽刷新方法（放 `impl Workspace` 内，靠近 `spawn_new_tab`）：

```rust
    /// 异步刷新当前项目的 git 分支/脏/文件状态（打开项目 + 回合结束触发）。
    fn spawn_project_git_refresh(&self) {
        let Some(p) = &self.project else {
            return;
        };
        let repo = PathBuf::from(&p.path);
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let (b, d, s) = tokio::task::spawn_blocking(move || {
                (
                    delivery::branch(&repo),
                    delivery::is_dirty(&repo),
                    delivery::file_statuses(&repo),
                )
            })
            .await
            .unwrap_or((None, false, HashMap::new()));
            let _ = proxy.send_event(Message::ProjectGitRefreshed(b, d, s));
        });
    }
```

6. `ProjectOpened` handler：把原来的内联 git spawn 换成设完 `self.project` 后调 `spawn_project_git_refresh`。将

```rust
                self.branch = None;
                self.dirty = false;
                if let Some(p) = &project {
                    let repo = PathBuf::from(&p.path);
                    let proxy = self.proxy.clone();
                    self.handle.spawn(async move {
                        let (b, d) = tokio::task::spawn_blocking(move || {
                            (delivery::branch(&repo), delivery::is_dirty(&repo))
                        })
                        .await
                        .unwrap_or((None, false));
                        let _ = proxy.send_event(Message::ProjectGitRefreshed(b, d));
                    });
                }
                self.project = project;
            }
```

替换为：

```rust
                self.branch = None;
                self.dirty = false;
                self.git_statuses = HashMap::new();
                self.project = project;
                self.spawn_project_git_refresh();
            }
```

7. `DeliveryChecked` handler 末尾追加一行（回合结束后刷新树装饰）：

```rust
                self.spawn_project_git_refresh();
```

（放在该分支 `if let Some(tab) = self.tab_by_id_mut(tab_id) { ... }` 之后、分支结束前。）

8. 纯函数 + 渲染接线。`decoration_for`（放文件级函数区，如 `project_branch_label` 附近）：

```rust
/// 文件 git 状态 → (颜色, 尾缀字符)。金=改/绿=新/红=删（沿用 P1g 金脏约定）。
fn decoration_for(status: FileStatus) -> (Color, &'static str) {
    match status {
        FileStatus::Modified => (theme::GOLD, "•"),
        FileStatus::New => (theme::GREEN, "+"),
        FileStatus::Deleted => (theme::RED, "−"),
    }
}
```

`project_pane` 的文件树循环：把

```rust
            if let Some(tree) = &ws.file_tree {
                for row in tree.visible_rows() {
                    let indent = "  ".repeat(row.depth);
                    let glyph = if row.is_dir {
                        if row.expanded { "▾ " } else { "▸ " }
                    } else {
                        "  "
                    };
                    let label = format!("{indent}{glyph}{}", row.name);
                    let msg = if row.is_dir {
                        Message::ProjectTreeToggle(row.path.clone())
                    } else {
                        Message::PreviewOpenPath(row.path.clone())
                    };
                    let color = if row.is_dir { theme::BODY } else { theme::CYAN };
```

改为（循环外先收集变更路径，循环内查装饰）：

```rust
            if let Some(tree) = &ws.file_tree {
                let changed: Vec<PathBuf> = ws.git_statuses.keys().cloned().collect();
                for row in tree.visible_rows() {
                    let indent = "  ".repeat(row.depth);
                    let glyph = if row.is_dir {
                        if row.expanded { "▾ " } else { "▸ " }
                    } else {
                        "  "
                    };
                    // 装饰:文件查自身状态;目录 rollup(含深层变更即金)。
                    let deco: Option<(Color, &'static str)> = if row.is_dir {
                        delivery::dir_has_change(&row.path, &changed).then_some((theme::GOLD, "•"))
                    } else {
                        ws.git_statuses.get(&row.path).map(|s| decoration_for(*s))
                    };
                    let (color, suffix) = match deco {
                        Some((c, mark)) => (c, format!(" {mark}")),
                        None => (
                            if row.is_dir { theme::BODY } else { theme::CYAN },
                            String::new(),
                        ),
                    };
                    let label = format!("{indent}{glyph}{}{suffix}", row.name);
                    let msg = if row.is_dir {
                        Message::ProjectTreeToggle(row.path.clone())
                    } else {
                        Message::PreviewOpenPath(row.path.clone())
                    };
```

（原 `let color = ...` 行删除；下方 `button(text(label)...color(color))` 不变。）

- [x] **Step 4: 跑测试确认通过 + 冒烟**

```bash
cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt
cargo build -p dozer-app && ./target/aarch64-apple-darwin/debug/dozer & sleep 8 && kill %1
```

Expected: 测试全绿、零警告；app 存活 8 秒无 panic。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(项目): 文件树 git 装饰——状态上色/尾缀字符 + 目录 rollup + 打开项目/回合结束刷新

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: 全量回归 + 人工验收 + 落档

**Files:**
- Create: `docs/superpowers/specs/2026-07-19-p1h-acceptance.md`
- Modify: `docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`（验收通过后 §7 追加 P1h 达成）
- Modify: 本计划文件（勾选 checkbox）

**Interfaces:**
- Consumes: 全部前序任务。
- Produces: 用户签字的验收记录；下一阶段起点。

- [x] **Step 1: 全量回归 + 冒烟**

```bash
cargo test && cargo clippy --all-targets && cargo fmt --check
cargo build -p dozer-app && ./target/aarch64-apple-darwin/debug/dozer & sleep 8 && kill %1
```

Expected: 全绿零警告；app 存活 8 秒。**验收前 `pkill dozerd` 重启新 daemon。**

- [ ] **Step 2: 用户人工验收（逐项 ✓/✗，验收权在用户）**

```markdown
# P1h 人工验收清单（用户实机执行）
1. 打开本仓为项目 → 改一个已跟踪文件 → 文件树对应行变金 + 尾缀 `•`，其所在目录（折叠时）也标金 `•`
2. 新建一个未跟踪文件 → 文件树对应行变绿 + `+`（若在可见/展开处）
3. 删一个已跟踪文件 → 对应行（若可见）红 + `−`
4. 让 claude 改文件 → 回合结束后树装饰自动刷新（无需手动重开项目）
5. `git checkout`/提交清干净 → 下次回合结束或重开项目 → 装饰消失
6. 打开非 git 目录为项目 → 文件树无装饰、不崩
```

- [ ] **Step 3: 结果落档 + Commit**

验收记录写入 `docs/superpowers/specs/2026-07-19-p1h-acceptance.md`（沿用格式：逐项结果表 + 反馈修复流水）；全部通过后规格 §7 项目栏段落追加"P1h 达成（<日期>，文件树 git 装饰：金/绿/红上色 + 尾缀字符 + 目录 rollup + 打开/回合结束刷新），验收记录见 specs/2026-07-19-p1h-acceptance.md"。

```bash
git add docs
git commit -m "验收：P1h 文件树 git 装饰人工验收记录 + 规格回填

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

## Self-Review（已执行）

1. **Spec 覆盖**：D1（porcelain→FileStatus，绝对路径）=T1 `file_statuses`；D2（金/绿/红 + 尾缀）=T2 `decoration_for` + 渲染；D3（目录 rollup）=T1 `dir_has_change` + T2 渲染；D4（复用 ProjectGitRefreshed，打开+回合结束刷新）=T2 消息扩参 + `spawn_project_git_refresh` 两处调用；D5（GUI spawn_blocking，dozerd 不参与）=T2 刷新方法。错误处理三条：非 git 空 map（T1 `git` 失败返回空）、porcelain 畸形行跳过（T1 `line.len()<4` / 空路径跳过）、变更文件不可见不影响（渲染只查可见行 + rollup）。
2. **占位符扫描**：无 TBD；T2 渲染改动给了完整前后代码块。
3. **类型一致性**：`FileStatus{Modified,New,Deleted}`（Copy）、`file_statuses(&Path)->HashMap<PathBuf,FileStatus>`、`dir_has_change(&Path,&[PathBuf])->bool`、`ProjectGitRefreshed(Option<String>,bool,HashMap<PathBuf,FileStatus>)`、`spawn_project_git_refresh(&self)`、`decoration_for(FileStatus)->(Color,&'static str)` 在 T1/T2 交叉一致。
