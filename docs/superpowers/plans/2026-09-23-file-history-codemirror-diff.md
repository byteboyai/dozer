# 文件历史弹窗 Diff 视图改用 CodeMirror Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the file-history overlay's diff pane with the same CodeMirror 6 `unifiedMergeView` already delivered by the git-log-diff plan (old side = a historical commit's blob for this path, new side = the file's **current on-disk content**, not another commit), replacing `colored_diff_lines`.

**Architecture:** This overlay (`platform/file_history_overlay.rs`) is a fully independent native child window with its own `winit::Window` + `iced_wgpu` render loop and, today, zero `wry` presence at all — unlike the git log panel, it cannot reuse the main window's `sync_webview_pool`/`App::preview_desired` machinery (traced through `runtime.rs`: that machinery's IPC-routing closure branches on `binding.panel`, and the main pool's `HashMap<usize,...>` keys are shared across every real Files/Project tab — reusing it here would either silently misroute this webview's events into the tab-based `EditorWebviewEvent` handler or require a new `PanelKind` variant that ripples into unrelated exhaustive matches elsewhere in the app). Instead, `FileHistoryOverlay` gets one bespoke `Option<(wry::WebView, String)>` slot (single webview, not a pool — this popup only ever shows one diff at a time) built with a small, purpose-built `WebViewBuilder` that reuses only the genuinely reusable pieces: `assets::handle_protocol` (the `dozer://` scheme handler), `EditorHostBinding`/`diff_url()`/`SetDiffDocument`/`HostBinding::validate()` from the git-log-diff plan's protocol work, and the same `mode=diff` `unifiedMergeView` JS bundle — no new protocol, no new JS.

**Tech Stack:** Rust (iced 0.14 GPU overlay window, wry, git2), reuses the TypeScript CodeMirror bundle shipped by the git-log-diff plan (no TS changes in this plan).

**Spec:** `docs/superpowers/specs/2026-09-23-file-history-codemirror-diff-design.md` (sister spec to `2026-09-23-git-log-codemirror-diff-design.md`)

## Global Constraints

- **Hard dependency, verify before Task 1's first step:** this plan consumes `crate::extensions::git_log::{DiffBlobContent, MAX_DIFF_BLOB_BYTES}`, `crate::preview::{EditorHostBinding, EditorCommand::SetDiffDocument, HostBinding, dispatch_script, encode_command, parse_event}`, and the `mode=diff` JS bundle — all delivered by `docs/superpowers/plans/2026-09-23-git-log-codemirror-diff.md`. That plan must be **merged to `main`** (not just written) before this one starts. Task 1 Step 1 verifies this by grepping for `DiffBlobContent` in `main`'s `git_log.rs` and failing loudly if it's missing, before touching anything else.
- Diff pane is always read-only — same as the git-log-diff plan, never wire `replace_range`/`save_document`.
- Binary/size classification must use the **exact same** `classify_diff_bytes` function this plan extracts from `git_log.rs` in Task 1 — not a second, possibly-drifting reimplementation.
- **Work on an isolated branch, never commit to `main` directly.** Branch: `feature/file-history-codemirror-diff`, created from a worktree off current `main` (which must already include the merged git-log-diff plan — see the hard dependency above). Every task's first step re-verifies you are on that branch inside a `cd <worktree-abs-path> && git branch --show-current` compound command — never a bare `cd` followed by a separate command (this environment's Bash tool does not preserve `cd` across separate tool calls; every single command in every task must be `cd <worktree-abs-path> && <the real command>`). Read/Edit/Write tool calls must use the worktree's absolute path, never a path that could resolve to the main checkout. After all tasks are done and self-verified, open the branch for review; only merge to `main` after review passes — do not merge or fast-forward `main` yourself as part of this plan.
- This repo has an autonomous process that commits to `main` concurrently and unpredictably (see project memory). Before every `git commit` in this plan, run `git status --short` in the worktree first and confirm only the files this task touched are staged — never `git add -A`/`git add .`.

## Review Focus

- **Deletion-commit edge case**: a file-history entry can be a commit that *deleted* the file (`build()`'s pathspec filter includes deletions) — that commit's tree has no entry at this path. `old_text` for such a selection must resolve to `""` (treated like "no old version"), not an error, not a panic — Task 1.
- **Disk file removed while the popup is open**: `new_text` read (`std::fs::read`) can fail independently of any git operation — must fall back to "not renderable" with a clear reason, not crash the whole diff task — Task 1.
- **Stale selection after fast commit-switching**: user clicks commit A then B before A's content load resolves — mirrors the exact race the git-log-diff plan already solved for its own panel; this plan needs its own guard since `file_history::State` is a separate type — Task 2.
- **Rollback invalidates `new_text`, not just the patch cache**: `RollbackDone`'s success branch already clears `diff_cache`; it must equally clear/reload this plan's `loaded_diff` (the disk content just changed), not leave stale CodeMirror content on screen after a rollback — Task 2.
- **Webview must not outlive the overlay window it's a child of**: closing the popup (`FileHistoryOverlay` dropped) must drop the webview too, and reopening the popup for a different file must not show the previous file's stale content for even one frame — Task 3's `Drop`/rebuild semantics, checked manually in Task 5.

---

### Task 1: Shared `classify_diff_bytes` + file-history's blob-vs-disk content reader

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs` (extract `classify_diff_bytes` from `diff_blob_content`'s private `read_side` closure, promote to a `pub(crate)` module-level function, keep `diff_blob_content`'s external behavior byte-for-byte identical)
- Modify: `crates/dozer-app/src/extensions/file_history.rs` (new `diff_blob_content_against_workdir()`, test module)

**Interfaces:**
- Consumes: `crate::extensions::git_log::{DiffBlobContent, MAX_DIFF_BLOB_BYTES}` (from the merged git-log-diff plan).
- Produces: `crate::extensions::git_log::classify_diff_bytes(bytes: &[u8]) -> Option<String>` (`pub(crate)`, consumed by both modules); `crate::extensions::file_history::diff_blob_content_against_workdir(repo: &git2::Repository, repo_path: &Path, file_path: &Path, oid: git2::Oid) -> Result<DiffBlobContent, String>` (consumed by Task 2).

- [ ] **Step 1: Verify worktree branch and the hard dependency**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git branch --show-current
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git merge-base --is-ancestor $(git log main --all --format=%H -- crates/dozer-app/src/extensions/git_log.rs | xargs -I{} git log -1 --format=%H {} 2>/dev/null | head -1) HEAD 2>&1 || rg -q "pub enum DiffBlobContent" crates/dozer-app/src/extensions/git_log.rs
```

Expected branch: `feature/file-history-codemirror-diff` (create the worktree/branch first via `superpowers:using-git-worktrees` if this is the first task run, **branching from a `main` that already has the git-log-diff plan merged**). The second command is a best-effort sanity check; if it errors out or `DiffBlobContent` genuinely isn't in `git_log.rs` yet, **stop and tell the user** — do not proceed by reimplementing the git-log-diff plan's types here, that would fork the two plans' behavior.

- [ ] **Step 2: Write the failing tests**

Add to `crates/dozer-app/src/extensions/git_log.rs`'s test module, near the existing `diff_blob_content_*` tests:

```rust
    #[test]
    fn classify_diff_bytes_matches_diff_blob_content_behavior() {
        // 提取重构不应该改变行为:同一段字节,`classify_diff_bytes` 的结果
        // 要跟通过 `diff_blob_content` 间接观察到的判定一致(正常文本/
        // 二进制/超限三种)。
        assert_eq!(classify_diff_bytes(b"hello\n"), Some("hello\n".to_string()));
        assert_eq!(classify_diff_bytes(&[0x00, 0x01, 0x02]), None);
        let big = vec![b'a'; MAX_DIFF_BLOB_BYTES + 1];
        assert_eq!(classify_diff_bytes(&big), None);
        let exact = vec![b'a'; MAX_DIFF_BLOB_BYTES];
        assert!(classify_diff_bytes(&exact).is_some());
    }
```

Add to `crates/dozer-app/src/extensions/file_history.rs`'s test module (near `mkrepo()`, `target()`):

```rust
    /// `mkrepo()` 的 c1/c2/c3 之上追加 c4:删除 `a.txt`。
    fn mkrepo_with_delete_commit() -> (tempfile::TempDir, PathBuf) {
        let (dir, repo) = mkrepo();
        git(&repo, &["rm", "-q", "a.txt"]);
        git(&repo, &["commit", "-qm", "c4: delete a.txt"]);
        (dir, repo)
    }

    #[test]
    fn diff_blob_content_against_workdir_reads_historical_and_current() {
        let (_dir, repo) = mkrepo();
        let git_repo = git2::Repository::open(&repo).unwrap();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c1_oid = snapshot.entries[1].oid; // c1: a.txt == "one\n"
        // 磁盘当前内容是 c3 之后的 "two\n"。
        let content = diff_blob_content_against_workdir(
            &git_repo,
            &repo,
            Path::new("a.txt"),
            c1_oid,
        )
        .expect("应能读出历史版本与磁盘当前内容");
        let DiffBlobContent::Text { old_text, new_text } = content else {
            panic!("正常改动文件应判定为可渲染文本");
        };
        assert_eq!(old_text, "one\n");
        assert_eq!(new_text, "two\n");
    }

    #[test]
    fn diff_blob_content_against_workdir_empty_old_side_for_deletion_commit() {
        let (_dir, repo) = mkrepo_with_delete_commit();
        let git_repo = git2::Repository::open(&repo).unwrap();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c4_oid = snapshot.entries[0].oid; // c4: 删除 a.txt,该提交树里没有这个路径
        let content =
            diff_blob_content_against_workdir(&git_repo, &repo, Path::new("a.txt"), c4_oid)
                .expect("删除类历史记录不应报错");
        let DiffBlobContent::Text { old_text, .. } = content else {
            panic!("应判定为可渲染文本(旧侧为空)");
        };
        assert_eq!(old_text, "", "该提交树里没有这个路径,旧侧按空字符串处理");
    }

    #[test]
    fn diff_blob_content_against_workdir_not_renderable_when_disk_file_missing() {
        let (_dir, repo) = mkrepo();
        let git_repo = git2::Repository::open(&repo).unwrap();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c1_oid = snapshot.entries[1].oid;
        std::fs::remove_file(repo.join("a.txt")).unwrap();
        let content =
            diff_blob_content_against_workdir(&git_repo, &repo, Path::new("a.txt"), c1_oid)
                .expect("磁盘文件缺失不应报错,应判定为不可渲染");
        assert!(matches!(content, DiffBlobContent::NotRenderable { .. }));
    }
```

Note: these three new tests call `build(&repo, Path::new("a.txt"), 10)` — confirm this matches `file_history.rs`'s actual `build()` signature (`pub fn build(repo_path: &Path, file_path: &Path, max_count: usize) -> Result<FileHistorySnapshot, String>`) exactly as already defined in this file (read it before writing, it's ~line 246) — the plan's snippet above assumes that exact signature.

- [ ] **Step 3: Run tests to verify they fail to compile**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && cargo test -p dozer-app classify_diff_bytes 2>&1 | tail -30
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && cargo test -p dozer-app diff_blob_content_against_workdir 2>&1 | tail -40
```

Expected: `classify_diff_bytes` not found (still private/nested inside `diff_blob_content`); `diff_blob_content_against_workdir` not found.

- [ ] **Step 4: Extract `classify_diff_bytes` in `git_log.rs`**

Replace the existing `diff_blob_content` function (its private nested `read_side` currently duplicates this logic inline) with:

```rust
/// 给定原始字节,判断能否喂给 CodeMirror diff 渲染:超过
/// [`MAX_DIFF_BLOB_BYTES`] 或含二进制内容(NUL 字节 / 非法 UTF-8)都判定
/// "不可渲染"。`git_log`(commit vs commit)与 `file_history`(commit vs
/// 磁盘实时内容)共用同一份判定,不允许出现第二份可能漂移的实现。
pub(crate) fn classify_diff_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.len() > MAX_DIFF_BLOB_BYTES {
        return None;
    }
    if bytes.contains(&0u8) {
        return None;
    }
    std::str::from_utf8(bytes).ok().map(|s| s.to_string())
}

pub fn diff_blob_content(
    repo: &git2::Repository,
    old_blob: Option<git2::Oid>,
    new_blob: Option<git2::Oid>,
) -> Result<DiffBlobContent, String> {
    fn read_side(repo: &git2::Repository, oid: Option<git2::Oid>) -> Result<Option<String>, String> {
        let Some(oid) = oid else {
            return Ok(Some(String::new()));
        };
        let blob = repo.find_blob(oid).map_err(|e| e.message().to_string())?;
        Ok(classify_diff_bytes(blob.content()))
    }

    let old_text = read_side(repo, old_blob)?;
    let new_text = read_side(repo, new_blob)?;
    match (old_text, new_text) {
        (Some(old_text), Some(new_text)) => Ok(DiffBlobContent::Text { old_text, new_text }),
        _ => Ok(DiffBlobContent::NotRenderable {
            reason: "文件不是文本,或超过大小上限,不支持 CodeMirror 渲染".to_string(),
        }),
    }
}
```

This is a pure refactor (`read_side`'s three checks are now one call to `classify_diff_bytes`) — `diff_blob_content`'s existing tests from the git-log-diff plan must still pass unchanged; that's what Step 5 confirms.

- [ ] **Step 5: Add `diff_blob_content_against_workdir` in `file_history.rs`**

```rust
/// `oid` 对应提交树里 `file_path` 的历史内容 vs 磁盘上 `repo_path.join(
/// file_path)` 的实时内容。跟 `git_log::diff_blob_content`(两个 commit 之间)
/// 的关键差异:new 侧永远来自磁盘,不是另一个 blob;old 侧若该提交树里没有
/// 这个路径(历史记录本身是一次删除),按空字符串处理,不报错——这不是
/// 异常情况,是"文件历史"列表天然会包含的一种记录(`build()` 的 pathspec
/// 过滤只看"这次提交碰过这个路径",删除也算碰过)。
pub fn diff_blob_content_against_workdir(
    repo: &git2::Repository,
    repo_path: &Path,
    file_path: &Path,
    oid: git2::Oid,
) -> Result<crate::extensions::git_log::DiffBlobContent, String> {
    use crate::extensions::git_log::{DiffBlobContent, classify_diff_bytes};

    let commit = repo.find_commit(oid).map_err(|e| e.message().to_string())?;
    let tree = commit.tree().map_err(|e| e.message().to_string())?;
    let old_text = match tree.get_path(file_path) {
        Ok(entry) => {
            let obj = entry
                .to_object(repo)
                .map_err(|e| e.message().to_string())?;
            match obj.as_blob() {
                Some(blob) => classify_diff_bytes(blob.content()),
                None => None, // 路径是目录/子模块,不是文件——判不可渲染。
            }
        }
        Err(_) => Some(String::new()), // 该提交树里没有这个路径:删除类历史记录。
    };
    let new_text = match std::fs::read(repo_path.join(file_path)) {
        Ok(bytes) => classify_diff_bytes(&bytes),
        Err(_) => None, // 磁盘文件已不存在/不可读。
    };
    match (old_text, new_text) {
        (Some(old_text), Some(new_text)) => Ok(DiffBlobContent::Text { old_text, new_text }),
        _ => Ok(DiffBlobContent::NotRenderable {
            reason: "文件不是文本、超过大小上限,或磁盘文件当前不存在,不支持 CodeMirror 渲染"
                .to_string(),
        }),
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && cargo test -p dozer-app git_log:: 2>&1 | tail -60
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && cargo test -p dozer-app file_history:: 2>&1 | tail -60
```

Expected: all new tests pass, and every pre-existing `diff_blob_content_*`/`file_history::` test still passes unchanged (confirms the extraction didn't change behavior).

- [ ] **Step 7: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git status --short
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git add crates/dozer-app/src/extensions/git_log.rs crates/dozer-app/src/extensions/file_history.rs
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git commit -m "$(cat <<'EOF'
feat(file-history): blob-vs-disk diff content reader

Extracts classify_diff_bytes (binary/size gating) out of git_log's
diff_blob_content so both modules share one implementation, then adds
file_history::diff_blob_content_against_workdir, which reads the
historical blob for one side and the live on-disk file for the other
(this popup compares a commit against the working tree, not two
commits). A commit whose diff entry is a deletion has no path in its
own tree; that's treated as an empty old side, not an error.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `file_history::State`/`Message` wiring for diff content loading

**Files:**
- Modify: `crates/dozer-app/src/extensions/file_history.rs` (`State` ~line 68-77, `Message` ~line 116-127, `update()` ~line 133-215, test module)

**Interfaces:**
- Consumes: `diff_blob_content_against_workdir()` (Task 1).
- Produces: `State::loaded_diff() -> Option<&LoadedDiff>`, `State::diff_webview_ready()`/`diff_sent_for()` (consumed by Task 3); `Message::DiffContentLoaded(PathBuf, PathBuf, git2::Oid, Result<DiffBlobContent, String>)`.

- [ ] **Step 1: Verify worktree branch**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git branch --show-current
```

- [ ] **Step 2: Write the failing tests**

Add to `file_history.rs`'s test module, near the existing `snapshot_loaded_ignores_result_for_different_repo_path`-style tests (read that one first — same target-guard shape this task's tests mirror):

```rust
    #[tokio::test]
    async fn diff_content_loaded_ignored_when_target_no_longer_matches() {
        let mut state = Some(State::new(target()));
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DiffContentLoaded(
                PathBuf::from("/tmp/some-other-repo"),
                target().file_path,
                fake_oid(1),
                Ok(crate::extensions::git_log::DiffBlobContent::Text {
                    old_text: "a".into(),
                    new_text: "b".into(),
                }),
            ),
            &handle,
            |_| {},
        );
        assert!(
            state.as_ref().unwrap().loaded_diff().is_none(),
            "repo_path 对不上目标,结果应被丢弃"
        );
    }

    #[tokio::test]
    async fn diff_content_loaded_applied_when_target_matches() {
        let mut state = Some(State::new(target()));
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DiffContentLoaded(
                target().repo_path,
                target().file_path,
                fake_oid(1),
                Ok(crate::extensions::git_log::DiffBlobContent::Text {
                    old_text: "a".into(),
                    new_text: "b".into(),
                }),
            ),
            &handle,
            |_| {},
        );
        let loaded = state
            .as_ref()
            .unwrap()
            .loaded_diff()
            .expect("目标匹配的结果应该落地");
        assert_eq!(loaded.oid, fake_oid(1));
        assert!(matches!(
            loaded.content,
            crate::extensions::git_log::DiffBlobContent::Text { .. }
        ));
    }

    #[tokio::test]
    async fn select_commit_clears_stale_loaded_diff() {
        let mut state = Some(State::new(target()));
        if let Some(s) = &mut state {
            s.loaded_diff = Some(LoadedDiff {
                oid: fake_oid(9),
                content: crate::extensions::git_log::DiffBlobContent::Text {
                    old_text: "old".into(),
                    new_text: "old".into(),
                },
            });
        }
        let handle = tokio::runtime::Handle::current();
        update(&mut state, Message::SelectCommit(fake_oid(2)), &handle, |_| {});
        assert!(
            state.as_ref().unwrap().loaded_diff().is_none(),
            "换选中提交后必须先清空旧内容"
        );
    }

    #[tokio::test]
    async fn rollback_done_success_clears_loaded_diff() {
        let mut state = Some(State::new(target()));
        if let Some(s) = &mut state {
            s.selected = Some(fake_oid(3));
            s.loaded_diff = Some(LoadedDiff {
                oid: fake_oid(3),
                content: crate::extensions::git_log::DiffBlobContent::Text {
                    old_text: "old".into(),
                    new_text: "old".into(),
                },
            });
        }
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::RollbackDone(fake_oid(3), Ok(())),
            &handle,
            |_| {},
        );
        assert!(
            state.as_ref().unwrap().loaded_diff().is_none(),
            "回滚成功后磁盘内容已变,旧的 CodeMirror 内容必须清空、等待重新加载"
        );
    }
```

- [ ] **Step 3: Run to verify failure**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && cargo test -p dozer-app file_history:: 2>&1 | tail -60
```

Expected: `State` has no `loaded_diff` field/method, `Message` has no `DiffContentLoaded` variant, `LoadedDiff` not found.

- [ ] **Step 4: Add `LoadedDiff`, `State` fields, `Message` variant**

Near `FileHistoryTarget` in `file_history.rs`:

```rust
/// 当前已加载、给 CodeMirror diff webview 用的内容——`oid` 是加载时的选中
/// 版本快照,新结果落地前先核对还对不对得上"现在真正选中的",不对就丢弃
/// (同 `DiffLoaded` 的 target 核对手法,但这里额外要核对 `oid` 本身,因为
/// `DiffLoaded`/`DiffContentLoaded` 都只按 `(repo_path, file_path)` 核对
/// 目标,不看 `oid`——两条并行的加载各自要自己的 oid 匹配)。
#[derive(Debug, Clone)]
pub struct LoadedDiff {
    pub oid: git2::Oid,
    pub content: crate::extensions::git_log::DiffBlobContent,
}
```

Add to `State` (~line 68-77):

```rust
    /// 当前选中版本已加载的 diff 内容(CodeMirror webview 用),与
    /// `diff_cache`(patch 文本,给 `colored_diff_lines` 用)并存。
    loaded_diff: Option<LoadedDiff>,
    /// 当前挂载的 diff webview 是否已确认 `ready`。`loaded_diff` 被清空时
    /// (换选中 / 回滚成功)连带置回 false——webview 即将因内容不可渲染或
    /// 即将重新加载而可能被摘掉/换绑,旧的 ready 状态不能带到下一份内容。
    diff_webview_ready: bool,
    /// 最近一次**确认送达** webview 的内容对应的 `oid`。跟 `loaded_diff`
    /// 的 `oid` 不一致就还需要再推一次。
    diff_sent_for: Option<git2::Oid>,
```

Add accessors in `impl State` (near existing `diff_for`):

```rust
    pub fn loaded_diff(&self) -> Option<&LoadedDiff> {
        self.loaded_diff.as_ref()
    }

    pub fn diff_webview_ready(&self) -> bool {
        self.diff_webview_ready
    }

    pub fn diff_sent_for(&self) -> Option<git2::Oid> {
        self.diff_sent_for
    }

    pub(crate) fn set_diff_sent_for(&mut self, oid: git2::Oid) {
        self.diff_sent_for = Some(oid);
    }

    pub(crate) fn set_diff_webview_ready(&mut self, ready: bool) {
        self.diff_webview_ready = ready;
        if ready {
            self.diff_sent_for = None; // 强制下一帧重发一次当前内容。
        }
    }
```

Add to `Message` enum (near `DiffLoaded`):

```rust
    /// 选中版本的 blob/磁盘内容异步加载完成(CodeMirror 用,跟
    /// `DiffLoaded`——patch 文本、给 `colored_diff_lines` 用——并行、各自
    /// 独立缓存)。`(PathBuf, PathBuf)` 同 `DiffLoaded` 的目标核对手法。
    DiffContentLoaded(PathBuf, PathBuf, git2::Oid, Result<crate::extensions::git_log::DiffBlobContent, String>),
```

- [ ] **Step 5: Wire `SelectCommit`/`RollbackDone`/new `DiffContentLoaded` arm**

Modify the existing `Message::SelectCommit(oid)` arm (currently just sets `s.selected` and conditionally calls `spawn_diff`) to also clear the new fields and kick off the content load:

```rust
        Message::SelectCommit(oid) => {
            let Some(s) = state else { return };
            s.selected = Some(oid);
            s.loaded_diff = None;
            s.diff_webview_ready = false;
            s.diff_sent_for = None;
            if !s.diff_cache.contains_key(&oid)
                && let Some(target) = &s.target
            {
                spawn_diff(target, oid, handle, emit);
            }
            if let Some(target) = &s.target {
                spawn_diff_content(target, oid, handle, emit);
            }
        }
```

Modify the existing `Message::RollbackDone(_oid, result)` arm's `Ok(())` branch (currently clears `diff_cache` and re-calls `spawn_diff` for the selected commit) to do the same for the new content:

```rust
                Ok(()) => {
                    s.diff_cache.clear();
                    s.loaded_diff = None;
                    s.diff_webview_ready = false;
                    s.diff_sent_for = None;
                    if let Some(target) = &s.target
                        && let Some(selected) = s.selected
                    {
                        spawn_diff(target, selected, handle, emit);
                        spawn_diff_content(target, selected, handle, emit);
                    }
                }
```

Add the new `DiffContentLoaded` arm (near the existing `DiffLoaded` arm):

```rust
        Message::DiffContentLoaded(repo_path, file_path, oid, result) => {
            let Some(s) = state else { return };
            let Some(target) = &s.target else { return };
            if target.repo_path != repo_path || target.file_path != file_path {
                return; // 目标已切换,丢弃(同 DiffLoaded)。
            }
            if s.selected != Some(oid) {
                return; // 用户已经切到别的版本,这是一条迟到的结果。
            }
            s.loaded_diff = match result {
                Ok(content) => Some(LoadedDiff { oid, content }),
                Err(_) => None,
            };
        }
```

Also update `Message::SnapshotLoaded`'s success branch (currently, when it auto-selects the first entry, calls `spawn_diff(target, oid, handle, emit)`) to also kick off `spawn_diff_content` for that same auto-selected `oid`:

```rust
            if let Some(oid) = first_oid {
                s.selected = Some(oid);
                let target = s.target.as_ref().expect("刚核对过 target 非空");
                spawn_diff(target, oid, handle, emit);
                spawn_diff_content(target, oid, handle, emit);
            }
```

Add the new `spawn_diff_content` helper next to the existing `spawn_diff`:

```rust
fn spawn_diff_content(
    target: &FileHistoryTarget,
    oid: git2::Oid,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let repo_path = target.repo_path.clone();
    let file_path = target.file_path.clone();
    handle.spawn(async move {
        let repo_path2 = repo_path.clone();
        let file_path2 = file_path.clone();
        let result = tokio::task::spawn_blocking(move || {
            let repo = git2::Repository::open(&repo_path2).map_err(|e| e.message().to_string())?;
            diff_blob_content_against_workdir(&repo, &repo_path2, &file_path2, oid)
        })
        .await
        .unwrap_or_else(|e| Err(format!("diff 内容加载任务失败: {e}")));
        emit(Message::DiffContentLoaded(repo_path, file_path, oid, result));
    });
}
```

- [ ] **Step 6: Run tests to verify they pass**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && cargo test -p dozer-app file_history:: 2>&1 | tail -80
```

- [ ] **Step 7: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git status --short
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git add crates/dozer-app/src/extensions/file_history.rs
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git commit -m "$(cat <<'EOF'
feat(file-history): async diff content loading with stale-selection guard

SelectCommit/SnapshotLoaded's auto-select/RollbackDone now also kick
off diff_blob_content_against_workdir alongside the existing patch-text
spawn_diff, in parallel, into a separate loaded_diff field.
DiffContentLoaded discards results that no longer match the current
target or selected oid. Rollback clears loaded_diff too, since it
changes the on-disk content the diff's new side reads.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `FileHistoryOverlay` diff webview (bespoke builder, single slot, own IPC message)

**Files:**
- Modify: `crates/dozer-app/src/app/message.rs` (new `Message::FileHistoryDiffWebviewEvent` variant)
- Modify: `crates/dozer-app/src/app/update.rs` (handler: flip ready flag on `Ready`, ignore everything else)
- Modify: `crates/dozer-app/src/platform/file_history_overlay.rs` (`FileHistoryOverlay` gains `diff_webview: Option<(wry::WebView, String)>`; new `sync_diff_webview` method; geometry helper)
- Modify: `crates/dozer-app/src/platform/window_events.rs` (`sync_file_history_overlay()` ~line 1213-1277, call the new sync method every frame)

**Interfaces:**
- Consumes: `State::loaded_diff()`/`diff_webview_ready()`/`diff_sent_for()`/`set_diff_webview_ready()`/`set_diff_sent_for()` (Task 2), `crate::preview::EditorHostBinding::diff_url()`, `EditorCommand::SetDiffDocument`, `HostBinding`, `dispatch_script`, `encode_command`, `parse_event` (all from the merged git-log-diff plan).
- Produces: a working diff webview inside the file-history popup, created/destroyed/repositioned every frame in step with `app.file_history`'s state.

- [ ] **Step 1: Verify worktree branch**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git branch --show-current
```

- [ ] **Step 2: Add the `Message` variant and its handler**

In `crates/dozer-app/src/app/message.rs`, near wherever the git-log-diff plan added `GitLogDiffWebviewEvent` (read that one first, this mirrors it exactly):

```rust
    /// 文件历史弹窗 diff webview 发回的已校验协议事件。这扇窗口的 webview
    /// 不进主窗口的 `webviews`/`browser_webviews` 池,绑定同样用轻量
    /// `HostBinding`,不走 `EditorWebviewEvent` 的 tab 查找路径(这个弹窗
    /// 没有 tab)。
    FileHistoryDiffWebviewEvent(
        crate::preview::HostBinding,
        crate::preview::WebviewEnvelope<crate::preview::EditorEvent>,
    ),
```

In `crates/dozer-app/src/app/update.rs`, near wherever `Message::GitLogDiffWebviewEvent`'s handler landed:

```rust
            Message::FileHistoryDiffWebviewEvent(_binding, event) => {
                if matches!(event.payload, crate::preview::EditorEvent::Ready { .. })
                    && let Some(s) = self.file_history.as_mut()
                {
                    s.set_diff_webview_ready(true);
                }
                // 其余事件忽略,同 GitLogDiffWebviewEvent 的处理。
            }
```

- [ ] **Step 3: Add the geometry helper**

In `crates/dozer-app/src/platform/file_history_overlay.rs`, near `card_logical_size`:

```rust
/// diff 区域(`file_history::diff_area_view` 的 `content` 子树)在弹窗卡片
/// **自身逻辑坐标系**里的矩形(卡片就是这扇独立窗口的整个客户区,不需要
/// 再加窗口偏移)。跟 `file_history_card` 的实际布局逐项对应:外层
/// `padding(16)`;`title` 一行(`font::subtitle()`,`spacing(12)` 在其后);
/// `body` 是 `row![list(固定 240 宽), spacing(12), diff_area]`;
/// `diff_area_view` 内部是 `column![header, content].spacing(8)`,`header`
/// 一行(`font::label()`)。跟 `git_log` 那份计划的 diff pane 几何算法同一种
/// "近似值,人工验收阶段微调"精度承诺——`rollback_error` 有值时会在
/// `content` 之后再压一行文案,那种情况下这个矩形会比实际渲染区域略高,
/// 已知的已接受偏差,不在这个函数里处理。
fn diff_area_bounds(card_logical: LogicalSize<f32>) -> (f32, f32, f32, f32) {
    let pad = 16.0;
    let title_h = byteui::theme::font::subtitle() as f32 * 1.2;
    let header_h = byteui::theme::font::label() as f32 * 1.2;
    let list_w = 240.0;
    let row_spacing = 12.0;
    let col_spacing = 8.0;

    let x = pad + list_w + row_spacing;
    let y = pad + title_h + row_spacing + header_h + col_spacing;
    let w = (card_logical.width - x - pad).max(0.0);
    let h = (card_logical.height - y - pad).max(0.0);
    (x, y, w, h)
}
```

- [ ] **Step 4: Add `diff_webview` field and `sync_diff_webview`**

Add the field to `FileHistoryOverlay` (near `gpu`/`focus`):

```rust
    /// diff webview 单槽位(不是池——这个弹窗任意时刻最多展示一个 diff)。
    /// `String` 是当前已加载的 URL(导航去重,同主窗口 webview 池的既有
    /// 手法)。`None` = 未挂载(未选中版本 / 内容不可渲染 / 尚未加载完)。
    diff_webview: Option<(wry::WebView, String)>,
```

Initialize it to `None` in `FileHistoryOverlay::open()`'s struct literal (alongside `focus`/`cursor`/`modifiers`).

Add the sync method:

```rust
    /// 每帧调用:按 `app.file_history` 当前状态决定 diff webview 的存在/
    /// URL/矩形,并在 webview 已确认 ready 且有未送达内容时推一次
    /// `SetDiffDocument`。不复用主窗口 `sync_webview_pool`(见本计划顶部
    /// "Architecture"——那套的 IPC 路由按 `binding.panel` 分支,且池 key
    /// 空间是主窗口专属的,生搬到这扇独立窗口上要么错路由要么要新增
    /// `PanelKind` 变体,两者都不值当,这个槽位本来就只服务一个 webview)。
    pub(crate) fn sync_diff_webview(
        &mut self,
        app: &mut App,
        allowed_files: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<PathBuf>>>,
        proxy: winit::event_loop::EventLoopProxy<Message>,
    ) {
        let desired = app.file_history.as_ref().and_then(|s| {
            let loaded = s.loaded_diff()?;
            let crate::extensions::git_log::DiffBlobContent::Text { .. } = &loaded.content else {
                return None;
            };
            let target = s.target()?;
            Some((target.file_path.clone(), loaded.oid))
        });

        let Some((file_path, _oid)) = desired else {
            self.diff_webview = None;
            return;
        };

        let binding = crate::preview::EditorHostBinding::new(
            0,
            crate::app::PanelKind::Files,
            0,
            file_path,
        );
        let url = binding.diff_url(crate::preview::scheme_query_value());
        let logical_size: LogicalSize<f32> =
            self.window.inner_size().to_logical(self.window.scale_factor());
        let card_logical = card_logical_size(logical_size.width, logical_size.height);
        let (x, y, w, h) = diff_area_bounds(card_logical);
        let bounds = wry::Rect {
            position: wry::dpi::LogicalPosition::new(x as f64, y as f64).into(),
            size: wry::dpi::LogicalSize::new(w as f64, h as f64).into(),
        };

        match &mut self.diff_webview {
            Some((view, loaded_url)) => {
                if *loaded_url != url {
                    let _ = view.load_url(&url);
                    *loaded_url = url;
                }
                let _ = view.set_bounds(bounds);
            }
            None => {
                let root = crate::assets::assets_root();
                let ipc_proxy = proxy;
                let expected_binding = binding.clone();
                let built = wry::WebViewBuilder::new()
                    .with_url(&url)
                    .with_bounds(bounds)
                    .with_visible(true)
                    .with_custom_protocol("dozer".into(), move |_id, request| {
                        let allowed = allowed_files.lock().expect("allowed_files 锁");
                        let reply = crate::assets::handle_protocol(
                            &root,
                            &allowed,
                            None,
                            &request.uri().to_string(),
                        );
                        wry::http::Response::builder()
                            .status(reply.status)
                            .header("Content-Type", reply.mime)
                            .body(reply.body)
                            .unwrap()
                    })
                    .with_ipc_handler(move |req| {
                        let body = req.body().as_str();
                        let expected = crate::preview::HostBinding::new(
                            expected_binding.project_id,
                            expected_binding.panel,
                            expected_binding.tab_id,
                            expected_binding.document_id(),
                        );
                        match crate::preview::parse_event(body) {
                            Ok(event) => {
                                if let Err(error) = event.validate(&expected) {
                                    tracing::warn!(%error, "拒绝无效 file-history diff IPC");
                                } else {
                                    let _ = ipc_proxy.send_event(
                                        Message::FileHistoryDiffWebviewEvent(expected, event),
                                    );
                                }
                            }
                            Err(error) => {
                                tracing::warn!(%error, "无法解析 file-history diff IPC");
                            }
                        }
                    })
                    .build_as_child(&self.window);
                match built {
                    Ok(view) => self.diff_webview = Some((view, url)),
                    Err(e) => tracing::warn!("文件历史 diff webview 创建失败: {e}"),
                }
            }
        }

        // 内容推送:webview 已 ready 且当前内容还没送达才推。
        if let Some(s) = app.file_history.as_ref()
            && s.diff_webview_ready()
            && let Some(loaded) = s.loaded_diff()
            && s.diff_sent_for() != Some(loaded.oid)
            && let crate::extensions::git_log::DiffBlobContent::Text { old_text, new_text } =
                &loaded.content
            && let Some((view, _)) = &self.diff_webview
        {
            let language = crate::preview::extension_to_syntax(std::path::Path::new(
                &binding.path,
            ));
            let cmd = crate::preview::EditorCommand::SetDiffDocument {
                old_text: old_text.clone(),
                new_text: new_text.clone(),
                language: language.to_string(),
                revision: 0,
                read_only: true,
            };
            let script = crate::preview::dispatch_script(&crate::preview::encode_command(
                binding.project_id,
                binding.panel,
                binding.tab_id,
                &binding.document_id(),
                0,
                None,
                cmd,
            ));
            let _ = view.evaluate_script(&script);
            if let Some(s) = app.file_history.as_mut() {
                s.set_diff_sent_for(loaded.oid);
            }
        }
    }
```

Before finalizing, grep `preview/code_host.rs` for `EditorHostBinding`'s exact field visibility (`path` field — the snippet above reads `binding.path` for `extension_to_syntax`; confirm that field is at least `pub(crate)` and reachable from `platform::file_history_overlay`, adjusting to a getter if it's private) and confirm `EditorHostBinding` derives `Clone` (needed for the `expected_binding = binding.clone()` capture) — both should already be true from the git-log-diff plan's Task 3, but verify against the real merged code, not this plan's memory of it.

- [ ] **Step 5: Wire the call site in `window_events.rs`**

In `sync_file_history_overlay()` (~line 1213-1277), right before the final `if let Some(overlay) = file_history_overlay { overlay.request_redraw(); }`, add:

```rust
        let Self::Ready {
            app,
            file_history_overlay,
            proxy,
            ..
        } = self
        else {
            return;
        };
        if let Some(overlay) = file_history_overlay {
            overlay.sync_diff_webview(app, app.allowed_files(), proxy.clone());
            overlay.request_redraw();
        }
```

This replaces the existing final `if let Some(overlay) = file_history_overlay { overlay.request_redraw(); }` block — read the function's actual current tail before editing, since the destructure pattern (which fields are already bound at that point vs need re-destructuring) must match what's really there, not be assumed from this snippet.

- [ ] **Step 6: Build**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && cargo build -p dozer-app 2>&1 | tail -100
```

Fix compile errors against real field/method names/visibility discovered in Steps 4-5 (this task is the highest-risk one in this plan — it's new integration code with no existing unit-testable seam, see this task's "Interfaces" note and Task 5's manual QA).

- [ ] **Step 7: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git status --short
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git add crates/dozer-app/src/app/message.rs crates/dozer-app/src/app/update.rs crates/dozer-app/src/platform/file_history_overlay.rs crates/dozer-app/src/platform/window_events.rs
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git commit -m "$(cat <<'EOF'
feat(file-history): bespoke single-slot diff webview on the overlay window

FileHistoryOverlay gets its own wry::WebView (not the main window's
pool), built fresh each time with a minimal WebViewBuilder that reuses
assets::handle_protocol and the git-log-diff plan's EditorHostBinding/
SetDiffDocument/HostBinding protocol pieces. Synced once per frame
from sync_file_history_overlay: create/reload/reposition/tear down to
match app.file_history's loaded_diff, push SetDiffDocument once the
page confirms ready.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: `diff_area_view` routing (webview area vs iced fallback)

**Files:**
- Modify: `crates/dozer-app/src/extensions/file_history.rs` (`diff_area_view()` ~line 506-575)

**Interfaces:**
- Consumes: `State::loaded_diff()` (Task 2).
- Produces: `diff_area_view` reserves an empty area for the webview to composite over when a renderable diff has loaded; otherwise keeps today's `colored_diff_lines`/loading/error rendering.

- [ ] **Step 1: Verify worktree branch**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git branch --show-current
```

- [ ] **Step 2: Update `diff_area_view`'s `content` branch**

Change only the `content` computation inside `diff_area_view` (the `match state.diff_for(oid) { ... }` block, ~line 531-561) — leave `header`/`rollback_error` handling untouched:

```rust
    let content: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match state.loaded_diff() {
            Some(loaded)
                if loaded.oid == oid
                    && matches!(
                        loaded.content,
                        crate::extensions::git_log::DiffBlobContent::Text { .. }
                    ) =>
            {
                // CodeMirror 常开:留一块空区域给 Task 3 挂的 webview 合成
                // (同 git-log-diff 计划 Task 7 的手法)。
                container(iced_widget::Space::new())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
            Some(loaded) if loaded.oid == oid => {
                // 已加载但判定不可渲染(二进制/超限/磁盘文件缺失)。
                container(
                    text("(二进制文件、超出大小上限,或磁盘文件当前不存在,不支持预览)")
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().dim),
                )
                .padding(8)
                .into()
            }
            _ => match state.diff_for(oid) {
                None => byteui::feedback::math_curve::loading_hint(
                    byteui::feedback::math_curve::Curve::RoseThree,
                    "加载中…",
                    48.0,
                ),
                Some(Err(err)) => container(
                    text(err.clone())
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().red),
                )
                .padding(8)
                .into(),
                Some(Ok(patch)) if patch.is_empty() => container(
                    text("内容相同")
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().dim),
                )
                .padding(8)
                .into(),
                Some(Ok(patch)) => {
                    scrollable(crate::extensions::diff_render::colored_diff_lines(patch))
                        .direction(scrollable::Direction::Vertical(
                            byteui::interaction::scrollbar::scrollbar(),
                        ))
                        .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style())
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .into()
                }
            },
        };
```

- [ ] **Step 3: Build and run the full file_history test suite**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && cargo build -p dozer-app 2>&1 | tail -60
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && cargo test -p dozer-app file_history:: 2>&1 | tail -60
```

No new unit tests (pure-rendering `Element`-returning function, consistent with this file's existing test coverage which tests `build`/`update`/content-reading logic, not `view` output shape — same rationale as the git-log-diff plan's equivalent task).

- [ ] **Step 4: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git status --short
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git add crates/dozer-app/src/extensions/file_history.rs
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git commit -m "$(cat <<'EOF'
feat(file-history): diff_area_view reserves webview area when renderable

Same routing shape as the git-log-diff plan's diff_pane_view: an empty
area for the CodeMirror webview when loaded_diff is renderable text,
otherwise the existing patch-text/loading/error rendering.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Workspace-wide verification + manual QA pass

**Files:** none new — this task verifies Tasks 1-4's combined output.

- [ ] **Step 1: Verify worktree branch**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git branch --show-current
```

- [ ] **Step 2: Full workspace build, test, clippy, fmt**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && cargo build 2>&1 | tail -80
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && cargo test -p dozer-app 2>&1 | tail -100
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && cargo clippy --all-targets 2>&1 | tail -100
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && cargo fmt --check
```

Fix anything clippy/fmt flag; if `cargo fmt --check` fails, run `cargo fmt` and re-commit the formatting fix as its own small commit.

- [ ] **Step 3: Manual verification in the running app**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && cargo run -p dozer-app
```

In a project with real git history, right-click a tracked file in the Files panel → "查看此文件历史":
1. Select a version with real changes vs current disk content — diff pane shows CodeMirror with syntax highlighting and char-level diff highlighting, not the old plain-colored patch text.
2. Select the oldest version (root-commit-adjacent) if the current disk content differs — renders correctly.
3. If the file's history includes a commit that deleted it and a later commit re-added it, select the deletion commit — old side renders empty (per Task 1's deletion-commit handling), no crash, no error placeholder.
4. Click "回滚到此版本" (rollback), confirm the CodeMirror content updates to reflect the now-identical old/new content (or the popup's existing "内容相同" messaging, whichever this version's UI shows) rather than showing stale pre-rollback content.
5. Rapidly click through several versions in the commit list — no flash of a previous version's diff content.
6. Close the popup and reopen it (same file or a different one) — no stale webview content is visible even briefly, and no duplicate/leaked webview (watch memory/CPU doesn't creep up after several open/close cycles).
7. Resize the main window while the popup is open (the popup repositions relative to it) — diff webview rect stays reasonably aligned with the visible diff area box (small pixel misalignment is acceptable per Task 3's documented geometry approximation; the webview rendering clearly outside the diff area or not moving at all is not).

- [ ] **Step 4: Report findings**

If manual verification surfaces a real bug (crash, wrong content, leaked webview), fix it as a small additional commit on this same branch before declaring the plan complete. Pixel-alignment roughness consistent with Task 3's documented approximation is expected — note it for the human reviewer, don't chase pixel-perfect alignment here.

- [ ] **Step 5: Final commit and open for review**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git status --short
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-file-history-diff && git log --oneline main..HEAD
```

Confirm the branch's commit log matches the four feature commits from Tasks 1-4 (plus any Step 2 fmt-fix commit) and nothing else leaked in. Do not merge to `main` — hand off for review per the Global Constraints.

---

## Self-Review Notes (writing-plans skill checklist, run against the spec)

1. **Spec coverage**: 架构(独立窗口单槽位绑定,Task 3)/ 数据流(Task 1/2)/ 二进制与磁盘缺失判定(Task 1)/ 测试(每个 task 自带)均有对应任务。规格明确排除的并排视图、独立窗口机制重新设计两项未在计划中出现。
2. **Placeholder scan**: 每处代码都是具体实现,没有 TBD/"add appropriate"/注释占位代替代码。Task 3 Step 4/5 明确标注"这是本计划风险最高的一步,没有既有单测可参照,靠 Step 6 编译 + Task 5 人工验收兜底"——这是诚实的风险声明,不是把设计工作丢给执行者:代码本身是完整、具体、基于已读真实代码写出来的,标注只是提醒"这块比其它 task 更可能在编译时才暴露字段名/可见性细节"。
3. **Type consistency**: `DiffBlobContent`/`classify_diff_bytes`(Task 1,复用 git-log-diff 计划的类型,不重新定义)→ `LoadedDiff.content`(Task 2)→ `sync_diff_webview` 里的 `DiffBlobContent::Text` 解构(Task 3)→ `diff_area_view` 里的同款匹配(Task 4)全程一致。`Message::DiffContentLoaded` 的字段形状在 Task 2(构造)与其自身(解构)里一致;`Message::FileHistoryDiffWebviewEvent` 在 Task 3(两处:发送端的 IPC 闭包、接收端的 `app/update.rs` handler)字段形状一致。
4. **Review Focus wiring**:删除类历史记录边界 → Task 1 测试;磁盘文件缺失 → Task 1 测试;stale 选择竞态 → Task 2 测试;回滚清空 `loaded_diff` → Task 2 测试;webview 生命周期跟随弹窗窗口 → Task 3 的 `Drop` 语义 + Task 5 人工验收(这条没有自动化测试覆盖——`wry::WebView` 的真实创建/销毁在单测环境里不稳定可靠,同 Task 3 主体逻辑一样只能靠人工验收兜底,已在 Task 5 Step 3 第 6 点显式列出验收步骤)。
