# Git Log 面板 Diff 视图改用 CodeMirror Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the Git Log panel's inline diff pane with CodeMirror 6's `@codemirror/merge` `unifiedMergeView` (real syntax highlighting + char-level diff highlighting) instead of the current plain iced `colored_diff_lines`, reusing the existing `dozer://editor/` webview host infrastructure via a new `mode=diff` bootstrap path.

**Architecture:** Extend the existing `EditorHostBinding`/`WebviewSpec.editor_binding` machinery (already shared by the file-mode and JSON-tree editor hosts) with a third URL flavor bound to a fixed single webview slot (`PanelKind::GitLog`, project_id/tab_id always `0` — git log has no tabs and isn't project-scoped). Content (old/new blob text read via git2, not fetched from disk) is pushed post-mount via a new `EditorCommand::SetDiffDocument`, dispatched through a **new** message path (`Message::GitLogDiffWebviewEvent`) rather than the existing `Message::EditorWebviewEvent` handler, because that handler is hard-wired to the tab-based `PreviewTab` model (`ws.preview`/`ws.project_preview`) that git log does not have.

**Tech Stack:** Rust (iced 0.14, wry, git2), TypeScript (CodeMirror 6, esbuild), existing `serde`-tagged JSON envelope protocol.

**Spec:** `docs/superpowers/specs/2026-09-23-git-log-codemirror-diff-design.md`

## Global Constraints

- Diff pane is always read-only — never send `read_only: false`, never wire `replace_range`/`save_document` for this mode.
- Blob content over 512KB per side (old or new) is treated as "not renderable" — no partial/truncated rendering through CodeMirror.
- Binary detection is strict: any NUL byte, or content that is not valid UTF-8 — no lossy decoding for diff content (unlike file-mode preview's `lossy` flag).
- `PROTOCOL_VERSION` (currently `1` in `webview_protocol.rs`) must stay in lockstep between the new Rust `EditorCommand::SetDiffDocument` variant and the TS `EditorCommand` union — do not bump it for this feature.
- **Work on an isolated branch, never commit to `main` directly.** Branch: `feature/git-log-codemirror-diff`, created from a worktree off current `main`. Every task's first step re-verifies you are on that branch inside a `cd <worktree-abs-path> && git branch --show-current` compound command — never a bare `cd` followed by a separate command (this environment's Bash tool does not preserve `cd` across separate tool calls; every single command in every task must be `cd <worktree-abs-path> && <the real command>`). Read/Edit/Write tool calls must use the worktree's absolute path, never a path that could resolve to the main checkout. After all tasks are done and self-verified, open the branch for review; only merge to `main` after review passes — do not merge or fast-forward `main` yourself as part of this plan.
- This repo has an autonomous process that commits to `main` concurrently and unpredictably (see project memory). Before every `git commit` in this plan, run `git status --short` in the worktree first and confirm only the files this task touched are staged — never `git add -A`/`git add .`.

## Review Focus

- **Binary detection false negative**: a blob with high-bit bytes that happen to form valid UTF-8 but are clearly not source text (e.g. a UTF-8-encoded image metadata blob) — spec says "NUL byte or invalid UTF-8" only, not a broader binary sniff; Task 1's tests pin exactly that boundary so a future change doesn't silently loosen or tighten it.
- **Oversized diff fallback boundary**: exactly-at-512KB and one-byte-over-512KB per side, both sides vs. one side, must be covered so the cap can't drift to "combined size" by accident — Task 1.
- **Added/deleted file edge cases**: `old_file().id()`/`new_file().id()` both being `git2::Oid::zero()` never happens for a real delta, but a delta where only one side is zero (add/delete) must produce `None` for that side, not a spurious blob lookup — Task 1.
- **Stale selection race**: user clicks file A, then quickly clicks file B before A's blob-read task resolves — A's late `DiffContentLoaded` must not overwrite B's already-loaded content or, worse, get rendered while B is selected — Task 4.
- **`panel_token()` triple-duplication drift**: `code_host.rs::panel_token()`, `webview_protocol.rs::HostBinding::panel_token()`, and `webview_protocol.rs::encode_command()`'s inline match are three independent copies of the same `PanelKind -> &str` mapping; adding `PanelKind::GitLog` to only some of them breaks envelope validation silently (messages get dropped with a `tracing::warn!`, no crash, easy to miss in manual testing) — Task 3 updates and tests all three together.

---

### Task 1: `DiffFileEntry` blob fields + blob content extraction

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs` (`DiffFileEntry` struct around line 200, `commit_detail()` around line 495-523, test module around line 1439)

**Interfaces:**
- Produces: `DiffFileEntry.old_blob: Option<git2::Oid>`, `DiffFileEntry.new_blob: Option<git2::Oid>`; `pub fn diff_blob_content(repo_path: &Path, old_blob: Option<git2::Oid>, new_blob: Option<git2::Oid>) -> Result<DiffBlobContent, String>` returning either renderable old/new text or a reason it isn't renderable.

- [ ] **Step 1: Verify worktree branch**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git branch --show-current
```

Expected: `feature/git-log-codemirror-diff` (create the worktree/branch first via `superpowers:using-git-worktrees` if this is the very first task run — do not proceed on any other branch).

- [ ] **Step 2: Write the failing tests for blob-oid population**

Add to the `#[cfg(test)] mod tests` block in `crates/dozer-app/src/extensions/git_log.rs`, near `commit_detail_handles_root_commit_as_all_added` (~line 1465):

```rust
    #[test]
    fn commit_detail_populates_blob_oids_for_added_files() {
        let (_dir, repo) = mkrepo_with_one_commit();
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).expect("应能解析临时仓库");
        let root_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, root_oid).expect("根提交相对空树应该也能算出 diff");
        for f in &detail.files {
            assert_eq!(f.old_blob, None, "根提交没有旧版本,old_blob 必须是 None");
            assert!(f.new_blob.is_some(), "新增文件必须有 new_blob");
        }
    }

    /// 在 `mkrepo_with_one_commit()` 基础上追加一个"修改 a.txt、删除 b.txt"
    /// 的第二个提交,供改动/删除文件的 blob 提取测试用。
    fn mkrepo_with_modify_and_delete_commit() -> (tempfile::TempDir, std::path::PathBuf) {
        let (dir, repo) = mkrepo_with_one_commit();
        let git = |args: &[&str]| {
            let st = std::process::Command::new("git")
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
        std::fs::write(repo.join("a.txt"), "one\nmodified\n").unwrap();
        git(&["rm", "-q", "b.txt"]);
        git(&["add", "a.txt"]);
        git(&["commit", "-qm", "modify and delete"]);
        (dir, repo)
    }

    #[test]
    fn commit_detail_populates_blob_oids_for_modified_and_deleted_files() {
        let (_dir, repo) = mkrepo_with_modify_and_delete_commit();
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).expect("应能解析临时仓库");
        let head_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, head_oid).expect("应能算出第二个提交的 diff");

        let modified = detail
            .files
            .iter()
            .find(|f| f.path == "a.txt")
            .expect("a.txt 应该在改动文件里");
        assert_eq!(modified.status, git2::Delta::Modified);
        assert!(modified.old_blob.is_some());
        assert!(modified.new_blob.is_some());
        assert_ne!(modified.old_blob, modified.new_blob);

        let deleted = detail
            .files
            .iter()
            .find(|f| f.path == "b.txt")
            .expect("b.txt 应该在改动文件里");
        assert_eq!(deleted.status, git2::Delta::Deleted);
        assert!(deleted.old_blob.is_some());
        assert_eq!(deleted.new_blob, None, "删除文件必须是 new_blob = None");
    }

    #[test]
    fn diff_blob_content_reads_modified_file_both_sides() {
        let (_dir, repo) = mkrepo_with_modify_and_delete_commit();
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).expect("应能解析临时仓库");
        let head_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, head_oid).unwrap();
        let modified = detail.files.iter().find(|f| f.path == "a.txt").unwrap();

        let content = diff_blob_content(&repo, modified.old_blob, modified.new_blob)
            .expect("修改文件的 blob 内容应能读出");
        let DiffBlobContent::Text { old_text, new_text } = content else {
            panic!("修改文件应该判定为可渲染文本");
        };
        assert_eq!(old_text, "one\n");
        assert_eq!(new_text, "one\nmodified\n");
    }

    #[test]
    fn diff_blob_content_added_file_has_empty_old_side() {
        let (_dir, repo) = mkrepo_with_one_commit();
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).expect("应能解析临时仓库");
        let root_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, root_oid).unwrap();
        let added = detail.files.iter().find(|f| f.path == "a.txt").unwrap();

        let content = diff_blob_content(&repo, added.old_blob, added.new_blob).unwrap();
        let DiffBlobContent::Text { old_text, new_text } = content else {
            panic!("新增文件应该判定为可渲染文本");
        };
        assert_eq!(old_text, "");
        assert_eq!(new_text, "one\n");
    }

    #[test]
    fn diff_blob_content_deleted_file_has_empty_new_side() {
        let (_dir, repo) = mkrepo_with_modify_and_delete_commit();
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).expect("应能解析临时仓库");
        let head_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, head_oid).unwrap();
        let deleted = detail.files.iter().find(|f| f.path == "b.txt").unwrap();

        let content = diff_blob_content(&repo, deleted.old_blob, deleted.new_blob).unwrap();
        let DiffBlobContent::Text { old_text, new_text } = content else {
            panic!("删除文件应该判定为可渲染文本");
        };
        assert_eq!(old_text, "two\n");
        assert_eq!(new_text, "");
    }

    /// tempdir 里造一个含二进制文件(NUL 字节)的一次提交仓库。
    fn mkrepo_with_binary_commit() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let st = std::process::Command::new("git")
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
        std::fs::write(repo.join("blob.bin"), [0x00u8, 0x01, 0x02, 0xff]).unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "binary"]);
        (dir, repo)
    }

    #[test]
    fn diff_blob_content_rejects_binary_content() {
        let (_dir, repo) = mkrepo_with_binary_commit();
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).expect("应能解析临时仓库");
        let root_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, root_oid).unwrap();
        let bin = detail.files.iter().find(|f| f.path == "blob.bin").unwrap();

        let content = diff_blob_content(&repo, bin.old_blob, bin.new_blob).unwrap();
        assert!(matches!(content, DiffBlobContent::NotRenderable { .. }));
    }

    #[test]
    fn diff_blob_content_rejects_oversized_blob() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let st = std::process::Command::new("git")
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
        // 512KB 上限之上一字节:MAX_DIFF_BLOB_BYTES = 512 * 1024。
        let big = "a".repeat(MAX_DIFF_BLOB_BYTES + 1);
        std::fs::write(repo.join("big.txt"), &big).unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "big"]);
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).unwrap();
        let root_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, root_oid).unwrap();
        let entry = detail.files.iter().find(|f| f.path == "big.txt").unwrap();

        let content = diff_blob_content(&repo, entry.old_blob, entry.new_blob).unwrap();
        assert!(matches!(content, DiffBlobContent::NotRenderable { .. }));
    }

    #[test]
    fn diff_blob_content_accepts_blob_exactly_at_cap() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let st = std::process::Command::new("git")
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
        let exact = "a".repeat(MAX_DIFF_BLOB_BYTES);
        std::fs::write(repo.join("exact.txt"), &exact).unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "exact"]);
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).unwrap();
        let root_oid = snapshot.rows[0].oid;
        let detail = commit_detail(&repo, root_oid).unwrap();
        let entry = detail.files.iter().find(|f| f.path == "exact.txt").unwrap();

        let content = diff_blob_content(&repo, entry.old_blob, entry.new_blob).unwrap();
        assert!(matches!(content, DiffBlobContent::Text { .. }), "恰好等于上限应当可渲染");
    }
```

- [ ] **Step 2b: Run tests to verify they fail to compile (types don't exist yet)**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo test -p dozer-app git_log:: 2>&1 | tail -40
```

Expected: compile error — `DiffFileEntry` has no field `old_blob`, `diff_blob_content`/`DiffBlobContent`/`MAX_DIFF_BLOB_BYTES` not found.

- [ ] **Step 3: Add the fields, the constant, and `diff_blob_content`**

In `crates/dozer-app/src/extensions/git_log.rs`, modify `DiffFileEntry` (currently around line 200-210):

```rust
#[derive(Debug, Clone)]
pub struct DiffFileEntry {
    pub path: String,
    pub status: git2::Delta,
    pub patch: String,
    pub truncated: bool,
    /// 旧版本 blob oid(新增文件为 `None`)。CodeMirror diff 渲染用,与
    /// `patch`(unified patch 文本,给 `colored_diff_lines` 用)并存,互不影响。
    pub old_blob: Option<git2::Oid>,
    /// 新版本 blob oid(删除文件为 `None`)。
    pub new_blob: Option<git2::Oid>,
}

/// 单侧 blob 内容的字节上限(old/new 各自判定),超过就判定"不可渲染"。
pub const MAX_DIFF_BLOB_BYTES: usize = 512 * 1024;

/// [`diff_blob_content`] 的结果:要么是可渲染的双侧文本,要么给出原因
/// (供 UI 占位文案使用)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffBlobContent {
    Text { old_text: String, new_text: String },
    NotRenderable { reason: String },
}

/// 按新增/删除文件语义把 `None` 侧当空字符串处理;非 `None` 侧任一超过
/// [`MAX_DIFF_BLOB_BYTES`] 或含二进制内容(NUL 字节 / 非法 UTF-8)都判定
/// "不可渲染"——不做部分截断渲染。
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
        let content = blob.content();
        if content.len() > MAX_DIFF_BLOB_BYTES {
            return Ok(None);
        }
        if content.contains(&0u8) {
            return Ok(None);
        }
        match std::str::from_utf8(content) {
            Ok(text) => Ok(Some(text.to_string())),
            Err(_) => Ok(None),
        }
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

Overload `diff_blob_content` isn't actually overloaded — the tests above call it with `(&repo, ...)` where `repo` is a `PathBuf`/`&Path` in some tests and expect it to open the repository. Fix the tests instead to match the real signature: `diff_blob_content` takes an **open** `&git2::Repository`, not a path — this matches `commit_detail`'s existing pattern of opening the repo once per call. Update every test call site above from `diff_blob_content(&repo, ...)` (where `repo` is a `PathBuf`) to open the repo first:

```rust
        let git_repo = git2::Repository::open(&repo).unwrap();
        let content = diff_blob_content(&git_repo, modified.old_blob, modified.new_blob)
            .expect("修改文件的 blob 内容应能读出");
```

Apply this same `git2::Repository::open(&repo).unwrap()` fix to all five `diff_blob_content(&repo, ...)` call sites written in Step 2 (the parameter name `repo` in those tests is a `PathBuf`, shadow it with `git_repo` for the open handle, or rename consistently — pick one and apply it uniformly across all five tests before running them).

Now update `commit_detail()`'s file-construction closure (currently around line 507-523) to populate the two new fields:

```rust
    let mut files: Vec<DiffFileEntry> = diff
        .deltas()
        .filter_map(|delta| {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())?
                .to_string_lossy()
                .into_owned();
            let old_blob = (!delta.old_file().id().is_zero()).then(|| delta.old_file().id());
            let new_blob = (!delta.new_file().id().is_zero()).then(|| delta.new_file().id());
            Some(DiffFileEntry {
                path,
                status: delta.status(),
                patch: String::new(), // 下面按文件路径回填
                truncated: false,
                old_blob,
                new_blob,
            })
        })
        .collect();
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo test -p dozer-app git_log:: 2>&1 | tail -60
```

Expected: all new tests pass, and the three pre-existing `commit_detail_*` tests still pass unchanged.

- [ ] **Step 5: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git status --short
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git add crates/dozer-app/src/extensions/git_log.rs
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git commit -m "$(cat <<'EOF'
feat(git-log): extract old/new blob content for diff files

DiffFileEntry gains old_blob/new_blob oids (populated alongside the
existing unified patch text) and a new diff_blob_content() reads both
sides, rejecting binary content and blobs over 512KB per side. This is
groundwork for rendering the diff pane with CodeMirror; the existing
patch-text path (colored_diff_lines) is untouched.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `SetDiffDocument` protocol command (Rust + TS)

**Files:**
- Modify: `crates/dozer-app/src/preview/webview_protocol.rs` (`EditorCommand` enum ~line 142, tests ~line 565)
- Modify: `crates/dozer-app/web/editor/src/protocol.ts` (`EditorCommand` union ~line 61, `decodeCommand` ~line 163)
- Modify: `crates/dozer-app/web/editor/src/protocol.test.ts` (contract test, mirroring commit `88889e5b`'s pattern)

**Interfaces:**
- Consumes: nothing new from earlier tasks.
- Produces: `EditorCommand::SetDiffDocument { old_text: String, new_text: String, language: String, revision: u64, read_only: bool }` (Rust); `{ kind: 'set_diff_document'; old_text: string; new_text: string; language: string; revision: number; read_only: boolean }` (TS), used by Task 5 (Rust send side) and Task 8 (TS receive side).

- [ ] **Step 1: Verify worktree branch**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git branch --show-current
```

Expected: `feature/git-log-codemirror-diff`.

- [ ] **Step 2: Write the failing Rust round-trip test**

Add to `crates/dozer-app/src/preview/webview_protocol.rs`'s `#[cfg(test)] mod tests` (near `encodes_set_window_command`, ~line 566):

```rust
    #[test]
    fn encodes_set_diff_document_command() {
        let s = encode_command(
            0,
            PanelKind::GitLog,
            0,
            "gitlog-diff",
            9,
            None,
            EditorCommand::SetDiffDocument {
                old_text: "old\n".into(),
                new_text: "new\n".into(),
                language: "rust".into(),
                revision: 9,
                read_only: true,
            },
        );
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["payload"]["kind"], "set_diff_document");
        assert_eq!(v["payload"]["old_text"], "old\n");
        assert_eq!(v["payload"]["new_text"], "new\n");
        assert_eq!(v["payload"]["read_only"], true);
        assert_eq!(v["panel"], "gitlog");
    }

    #[test]
    fn set_diff_document_round_trips() {
        let cmd = EditorCommand::SetDiffDocument {
            old_text: "a\nb\n".into(),
            new_text: "a\nc\n".into(),
            language: "python".into(),
            revision: 3,
            read_only: true,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: EditorCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cmd);
    }
```

Note: `assert_eq!(v["panel"], "gitlog")` will only pass once Task 3's `panel_token()` updates land — if you're implementing tasks strictly in order this compiles but the assertion fails until Task 3; that's expected and acceptable since Task 3 depends on Task 2's `EditorCommand` variant existing anyway (Task 3's own tests exercise `panel_token()` directly). If running Task 2 in complete isolation before Task 3, change this one assertion temporarily to `"files"` (today's default fallback for any non-Project panel) and revisit it in Task 3's step where `panel_token()` gets its `PanelKind::GitLog` arm — Task 3 Step 2 below re-adds this exact assertion as `"gitlog"` and must pass then.

- [ ] **Step 3: Run to verify it fails to compile**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo test -p dozer-app webview_protocol:: 2>&1 | tail -30
```

Expected: `no variant named SetDiffDocument found for enum EditorCommand`.

- [ ] **Step 4: Add the Rust variant**

In `crates/dozer-app/src/preview/webview_protocol.rs`, add to `EditorCommand` (after `SetWindow`, before `RevealPosition`, ~line 159):

```rust
    /// Git Log diff pane 专用:CodeMirror `unifiedMergeView` 需要旧/新两份
    /// 完整文档自己跑 diff 算法,不是 unified patch 文本。恒只读
    /// (`read_only` 字段仍保留是为了和其它命令的字段形状一致,当前唯一
    /// 调用方 `runtime.rs` 恒传 `true`)。
    SetDiffDocument {
        old_text: String,
        new_text: String,
        language: String,
        revision: u64,
        read_only: bool,
    },
```

- [ ] **Step 5: Run Rust tests to verify they pass (except the `"gitlog"` assertion, see Step 2 note)**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo test -p dozer-app webview_protocol:: 2>&1 | tail -40
```

- [ ] **Step 6: Add the TS type and decoder**

In `crates/dozer-app/web/editor/src/protocol.ts`, add to the `EditorCommand` union (after `set_window`, ~line 76):

```typescript
  | {
      kind: 'set_diff_document';
      old_text: string;
      new_text: string;
      language: string;
      revision: number;
      read_only: boolean;
    }
```

And to `decodeCommand`'s switch (after `case 'set_window':`, ~line 180):

```typescript
    case 'set_diff_document':
      return typeof c.old_text === 'string' &&
        typeof c.new_text === 'string' &&
        typeof c.language === 'string' &&
        typeof c.revision === 'number' &&
        typeof c.read_only === 'boolean'
        ? (c as unknown as EditorCommand)
        : null;
```

- [ ] **Step 7: Write the failing TS contract test**

Add to `crates/dozer-app/web/editor/src/protocol.test.ts` (find the existing `describe`/`test` block structure used for the commit-`88889e5b` "structured backend/runtime invariant test" and add a sibling case — match the file's existing import style and test runner, which is Node's built-in `node --test` per `package.json`'s `"test": "node --test src/*.test.ts"`):

```typescript
test('decodeCommand accepts set_diff_document with all fields', () => {
  const decoded = decodeCommand({
    kind: 'set_diff_document',
    old_text: 'old\n',
    new_text: 'new\n',
    language: 'rust',
    revision: 5,
    read_only: true,
  });
  assert.deepStrictEqual(decoded, {
    kind: 'set_diff_document',
    old_text: 'old\n',
    new_text: 'new\n',
    language: 'rust',
    revision: 5,
    read_only: true,
  });
});

test('decodeCommand rejects set_diff_document missing a required field', () => {
  const decoded = decodeCommand({
    kind: 'set_diff_document',
    old_text: 'old\n',
    new_text: 'new\n',
    language: 'rust',
    revision: 5,
    // read_only intentionally missing
  });
  assert.strictEqual(decoded, null);
});
```

Before writing these, read the top of `protocol.test.ts` to match its actual import statements (`node:test`/`node:assert` vs. a bundler-based test runner) and existing `test(...)` call shape exactly — copy the style of the nearest existing `decodeCommand` test in that file rather than guessing the import lines.

- [ ] **Step 8: Run TS tests to verify they fail, then pass**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff/crates/dozer-app/web/editor && npm test 2>&1 | tail -40
```

Expected: fails before Step 6's type addition (or rather, `decodeCommand` returns `null` for the new kind since the switch has no case for it — the "accepts" test fails), passes after.

- [ ] **Step 9: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git status --short
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git add crates/dozer-app/src/preview/webview_protocol.rs crates/dozer-app/web/editor/src/protocol.ts crates/dozer-app/web/editor/src/protocol.test.ts
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git commit -m "$(cat <<'EOF'
feat(preview): SetDiffDocument editor command

New EditorCommand variant carrying full old/new document text for the
git log diff pane's CodeMirror unifiedMergeView (which needs both full
documents to run its own diff, not a unified patch string). Mirrored
on the TS side with a matching decodeCommand case and contract test.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `EditorHostBinding::diff_url()` + GitLog id offset + `panel_token()` triple-fix

**Files:**
- Modify: `crates/dozer-app/src/preview/code_host.rs` (`panel_token()` ~line 33, `EditorHostBinding::webview_id()` ~line 61, `panel_and_tab_from_webview_id()` ~line 106, new `diff_url()` method, tests ~line 150)
- Modify: `crates/dozer-app/src/preview/webview_protocol.rs` (`HostBinding::panel_token()` ~line 261, `encode_command()`'s inline match ~line 437)
- Modify: `crates/dozer-app/src/app/app.rs` (new `GIT_LOG_DIFF_ID_OFFSET` constant, near `PROJECT_PREVIEW_ID_OFFSET`/`CONVERSATION_REVIEW_ID_OFFSET` ~line 615-621)

**Interfaces:**
- Consumes: nothing from Task 1/2 directly (independent of them), but the `assert_eq!(v["panel"], "gitlog")` in Task 2's test now passes once this task lands.
- Produces: `EditorHostBinding::diff_url(&self, theme: &str) -> String`; `GIT_LOG_DIFF_ID_OFFSET: usize`; `panel_token(PanelKind::GitLog) == "gitlog"` everywhere it's computed.

- [ ] **Step 1: Verify worktree branch**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git branch --show-current
```

- [ ] **Step 2: Write the failing tests**

Add to `crates/dozer-app/src/preview/code_host.rs`'s test module (~line 150):

```rust
    #[test]
    fn gitlog_panel_token_is_distinct() {
        assert_eq!(panel_token(PanelKind::GitLog), "gitlog");
        assert_eq!(panel_token(PanelKind::Files), "files");
        assert_eq!(panel_token(PanelKind::Project), "project");
    }

    #[test]
    fn gitlog_webview_id_applies_own_offset() {
        let b = EditorHostBinding::new(0, PanelKind::GitLog, 0, PathBuf::from("a.txt"));
        assert_eq!(b.webview_id(), crate::app::GIT_LOG_DIFF_ID_OFFSET);
    }

    #[test]
    fn gitlog_webview_id_inverts_back() {
        assert_eq!(
            panel_and_tab_from_webview_id(crate::app::GIT_LOG_DIFF_ID_OFFSET),
            (PanelKind::GitLog, 0)
        );
    }

    #[test]
    fn diff_url_carries_mode_and_gitlog_panel_token() {
        let b = EditorHostBinding::new(0, PanelKind::GitLog, 0, PathBuf::from("src/lib.rs"));
        let url = b.diff_url("dark");
        assert!(is_editor_url(&url));
        assert!(url.contains("mode=diff"));
        assert!(url.contains("panel=gitlog"));
        assert!(url.contains("ro=1"));
        assert!(url.contains("theme=dark"));
        assert!(url.contains("lang=rust"));
    }
```

Add to `crates/dozer-app/src/preview/webview_protocol.rs`'s test module (near `encodes_command_envelope`, ~line 721):

```rust
    #[test]
    fn host_binding_panel_token_covers_gitlog() {
        let b = HostBinding::new(0, PanelKind::GitLog, 0, "gitlog-diff".into());
        // 不能直接调用私有 `panel_token()`,靠 `validate()` 间接验证:一条
        // 携带 `panel: "gitlog"` 的 envelope 必须通过校验。
        let env = WebviewEnvelope {
            protocol_version: PROTOCOL_VERSION,
            project_id: 0,
            panel: "gitlog".to_string(),
            tab_id: 0,
            document_id: "gitlog-diff".to_string(),
            revision: 0,
            request_id: None,
            payload: EditorEvent::Ready {
                read_only: true,
                language: "rust".to_string(),
            },
        };
        assert!(env.validate(&b).is_ok());
    }
```

- [ ] **Step 3: Run to verify failure**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo test -p dozer-app code_host:: webview_protocol:: 2>&1 | tail -50
```

Expected: `GIT_LOG_DIFF_ID_OFFSET` not found; `panel_token`/`webview_id`/`diff_url` assertions fail (fall through to `"files"` default / method missing).

- [ ] **Step 4: Add the constant**

In `crates/dozer-app/src/app/app.rs`, next to the existing offsets (~line 615-621):

```rust
pub(crate) const PROJECT_PREVIEW_ID_OFFSET: usize = 1_000_000;
pub(crate) const CONVERSATION_REVIEW_ID_OFFSET: usize = 2_000_000;
/// Git Log 面板 diff webview 的固定单槽位 id(不是池的偏移起点——这个面板
/// 没有 tab 概念,任意时刻最多一个 diff webview,直接用这个常量本身当 id)。
pub(crate) const GIT_LOG_DIFF_ID_OFFSET: usize = 3_000_000;
```

- [ ] **Step 5: Update `code_host.rs`'s three functions + add `diff_url()`**

```rust
pub fn panel_token(panel: PanelKind) -> &'static str {
    match panel {
        PanelKind::Project => "project",
        PanelKind::GitLog => "gitlog",
        _ => "files",
    }
}
```

```rust
    pub fn webview_id(&self) -> usize {
        match self.panel {
            PanelKind::Project => PROJECT_PREVIEW_ID_OFFSET + self.tab_id,
            PanelKind::GitLog => crate::app::GIT_LOG_DIFF_ID_OFFSET,
            _ => self.tab_id,
        }
    }
```

```rust
pub fn panel_and_tab_from_webview_id(id: usize) -> (PanelKind, usize) {
    if id == crate::app::GIT_LOG_DIFF_ID_OFFSET {
        (PanelKind::GitLog, 0)
    } else if id >= PROJECT_PREVIEW_ID_OFFSET {
        (PanelKind::Project, id - PROJECT_PREVIEW_ID_OFFSET)
    } else {
        (PanelKind::Files, id)
    }
}
```

Update the `use` line at the top of `code_host.rs` (~line 13) from `use crate::app::{PROJECT_PREVIEW_ID_OFFSET, PanelKind};` to also bring in nothing extra (the new code above references `crate::app::GIT_LOG_DIFF_ID_OFFSET` with its full path so no import change is strictly needed, but for consistency with the existing bare `PROJECT_PREVIEW_ID_OFFSET` usage, add it to the same `use` line instead):

```rust
use crate::app::{GIT_LOG_DIFF_ID_OFFSET, PROJECT_PREVIEW_ID_OFFSET, PanelKind};
```

...and then use the bare `GIT_LOG_DIFF_ID_OFFSET` (not `crate::app::GIT_LOG_DIFF_ID_OFFSET`) in the two functions above and in the test — keep it consistent with how `PROJECT_PREVIEW_ID_OFFSET` is used bare elsewhere in this same file.

Add `diff_url()` next to `url()`/`json_url()` (~line 100):

```rust
    /// editor host 页面 URL,diff 模式:不带 `p=`(不触发文件读取——正文由
    /// `SetDiffDocument` 命令推送),恒 `ro=1`。`path` 仍参与 `document_id`/
    /// 资源预算估算(见 `runtime.rs` 的 `fs::metadata` 兜底),只是不会被
    /// JS 端 `fetch()`。
    pub fn diff_url(&self, theme: &str) -> String {
        format!(
            "{EDITOR_URL_PREFIX}index.html?mode=diff&theme={}&ro=1&doc={}&proj={}&panel={}&tab={}&lang={}&fs={}&lh={}",
            theme,
            super::encode_component(&self.document_id()),
            self.project_id,
            self.panel_token(),
            self.tab_id,
            super::extension_to_syntax(&self.path),
            crate::theme::terminal_font::size(),
            crate::theme::terminal_font::editor_line_height_factor(),
        )
    }
```

- [ ] **Step 6: Update `webview_protocol.rs`'s `HostBinding::panel_token()` and `encode_command()`**

```rust
    fn panel_token(&self) -> &'static str {
        match self.panel {
            PanelKind::Project => "project",
            PanelKind::GitLog => "gitlog",
            _ => "files",
        }
    }
```

```rust
    let env = WebviewEnvelope {
        protocol_version: PROTOCOL_VERSION,
        project_id,
        panel: match panel {
            PanelKind::Project => "project".to_string(),
            PanelKind::GitLog => "gitlog".to_string(),
            _ => "files".to_string(),
        },
        tab_id,
        document_id: document_id.to_string(),
        revision,
        request_id,
        payload: command,
    };
```

- [ ] **Step 7: Run tests to verify they pass**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo test -p dozer-app code_host:: webview_protocol:: 2>&1 | tail -60
```

Also re-run Task 2's `encodes_set_diff_document_command` test now that `panel_token` handles `GitLog`:

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo test -p dozer-app encodes_set_diff_document_command -- --exact 2>&1 | tail -20
```

Expected: passes with `v["panel"] == "gitlog"` now (fix that assertion in Task 2's test file if it was left at `"files"` for isolated-task-order reasons).

- [ ] **Step 8: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git status --short
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git add crates/dozer-app/src/preview/code_host.rs crates/dozer-app/src/preview/webview_protocol.rs crates/dozer-app/src/app/app.rs
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git commit -m "$(cat <<'EOF'
feat(preview): GitLog editor-host binding (diff_url, id offset, panel token)

EditorHostBinding now supports PanelKind::GitLog as a fixed single
webview slot (GIT_LOG_DIFF_ID_OFFSET, not a pooled offset range) with
a new diff_url() that omits the file-fetch path parameter. The
PanelKind -> wire-string mapping existed in three independent places
(code_host::panel_token, HostBinding::panel_token,
encode_command's inline match) and all three needed the same new
arm or envelope validation breaks silently for GitLog webviews.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: git_log state/message wiring for diff content loading

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs` (`State` struct ~line 269, `Message` enum ~line 224, `update()` ~line 358)

**Interfaces:**
- Consumes: `DiffFileEntry.old_blob`/`new_blob`, `diff_blob_content()`, `DiffBlobContent` (Task 1).
- Produces: `State::loaded_diff() -> Option<&LoadedDiff>` (new accessor for Task 6/7 to read); `Message::DiffContentLoaded(git2::Oid, String, Result<DiffBlobContent, String>)`.

- [ ] **Step 1: Verify worktree branch**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git branch --show-current
```

- [ ] **Step 2: Write the failing tests**

The test module already has everything needed for this: a `snapshot_at(repo_path, max_count) -> GitLogSnapshot` helper (~line 1517), and the established idiom for calling `update()` is `#[tokio::test] async fn ...` + `tokio::runtime::Handle::current()` (see `select_commit_sets_selected_and_clears_detail` ~line 1526, `detail_loaded_preselects_first_file` ~line 1634) — not a manually-constructed `Runtime`. `State`'s fields are private but accessible via struct-literal `..State::default()` from within this same module's test submodule (see every test above). Add these three tests near `detail_loaded_preselects_first_file` (~line 1634), reusing that exact pattern:

```rust
    #[tokio::test]
    async fn select_file_clears_stale_diff_and_requests_fresh_load() {
        let repo_path = PathBuf::from("/tmp/repo");
        let commit_oid = git2::Oid::from_bytes(&[7; 20]).unwrap();
        let old_blob = git2::Oid::from_bytes(&[8; 20]).unwrap();
        let new_blob = git2::Oid::from_bytes(&[9; 20]).unwrap();
        let mut state = State {
            cache: Some(snapshot_at(&repo_path, 10)),
            selected: Some(commit_oid),
            detail: Some(Ok(CommitDetail {
                files: vec![DiffFileEntry {
                    path: "a.txt".into(),
                    status: git2::Delta::Modified,
                    patch: "x".into(),
                    truncated: false,
                    old_blob: Some(old_blob),
                    new_blob: Some(new_blob),
                }],
            })),
            loaded_diff: Some(LoadedDiff {
                commit: commit_oid,
                path: "old-selection.txt".into(),
                content: DiffBlobContent::Text {
                    old_text: "a".into(),
                    new_text: "b".into(),
                },
            }),
            ..State::default()
        };

        let handle = tokio::runtime::Handle::current();
        update(&mut state, Message::SelectFile("a.txt".to_string()), &handle, |_| {});

        assert_eq!(state.selected_file.as_deref(), Some("a.txt"));
        assert!(
            state.loaded_diff.is_none(),
            "换选中文件后必须先清空旧内容,不能让 stale 内容闪一下"
        );
    }

    #[tokio::test]
    async fn diff_content_loaded_ignored_when_selection_moved_on() {
        let commit_a = git2::Oid::from_bytes(&[11; 20]).unwrap();
        let mut state = State {
            selected: Some(commit_a),
            selected_file: Some("b.txt".to_string()), // 用户已经切到 b.txt
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        // 一条迟到的 a.txt 结果(用户点过 a.txt 但已经切走了)。
        update(
            &mut state,
            Message::DiffContentLoaded(
                commit_a,
                "a.txt".to_string(),
                Ok(DiffBlobContent::Text {
                    old_text: "x".into(),
                    new_text: "y".into(),
                }),
            ),
            &handle,
            |_| {},
        );
        assert!(
            state.loaded_diff.is_none(),
            "stale 结果(commit/path 跟当前选中对不上)必须被丢弃"
        );
    }

    #[tokio::test]
    async fn diff_content_loaded_applied_when_selection_still_matches() {
        let commit_a = git2::Oid::from_bytes(&[12; 20]).unwrap();
        let mut state = State {
            selected: Some(commit_a),
            selected_file: Some("a.txt".to_string()),
            ..State::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DiffContentLoaded(
                commit_a,
                "a.txt".to_string(),
                Ok(DiffBlobContent::Text {
                    old_text: "x".into(),
                    new_text: "y".into(),
                }),
            ),
            &handle,
            |_| {},
        );
        let loaded = state.loaded_diff.as_ref().expect("匹配当前选择的结果应该落地");
        assert_eq!(loaded.commit, commit_a);
        assert_eq!(loaded.path, "a.txt");
        assert!(matches!(loaded.content, DiffBlobContent::Text { .. }));
    }
```

- [ ] **Step 3: Run to verify failure**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo test -p dozer-app git_log:: 2>&1 | tail -60
```

Expected: `State` has no field `loaded_diff`, `Message` has no variant `DiffContentLoaded`, `LoadedDiff` not found.

- [ ] **Step 4: Add `LoadedDiff`, the `State` field, and the `Message` variant**

In `crates/dozer-app/src/extensions/git_log.rs`, near `DiffBlobContent` (Task 1) or right above `State`:

```rust
/// 当前已加载、给 CodeMirror diff webview 用的内容——`commit`/`path` 是
/// 加载时的选择快照,`SelectFile`/`SelectCommit` 落地新结果前先核对这两个
/// 字段还对不对得上"现在真正选中的",不对就丢弃(stale-guard,同
/// `DetailLoaded` 的 `repo_path`/`oid` 核对手法)。
#[derive(Debug, Clone)]
pub struct LoadedDiff {
    pub commit: git2::Oid,
    pub path: String,
    pub content: DiffBlobContent,
}
```

Add to `State` (~line 269-308, after `selected_file`):

```rust
    /// 当前选中文件已加载的 diff 内容(CodeMirror webview 用)。切
    /// commit/切选中文件时先清空,新结果落地(`DiffContentLoaded`)且仍
    /// 匹配当前选择才重新填入。
    loaded_diff: Option<LoadedDiff>,
```

Add an accessor in `impl State` (~line 310-355):

```rust
    /// 当前已加载、可交给 CodeMirror diff webview 渲染的内容(`None` = 未
    /// 选中文件 / 内容还在加载 / 不可渲染 / 加载失败)。
    pub fn loaded_diff(&self) -> Option<&LoadedDiff> {
        self.loaded_diff.as_ref()
    }
```

Add to `Message` enum (~line 240, after `SelectFile`):

```rust
    /// 选中文件的 blob 内容异步加载完成。`git2::Oid`/`String` 是加载发起时
    /// 的 commit/路径快照,落地前核对仍匹配当前选择,不匹配则丢弃(用户
    /// 手快切换选择后的迟到结果)。
    DiffContentLoaded(git2::Oid, String, Result<DiffBlobContent, String>),
```

- [ ] **Step 5: Wire `SelectFile` to clear+dispatch and handle `DiffContentLoaded`**

Replace the existing `SelectFile` arm in `update()` (currently ~line 382-384, just `state.selected_file = Some(path); None`):

```rust
        Message::SelectFile(path) => {
            state.selected_file = Some(path.clone());
            state.loaded_diff = None;
            let Some(commit) = state.selected else {
                return None;
            };
            let Some(Ok(detail)) = state.detail.as_ref() else {
                return None;
            };
            let Some(entry) = detail.files.iter().find(|f| f.path == path) else {
                return None;
            };
            let (old_blob, new_blob) = (entry.old_blob, entry.new_blob);
            let repo_path = state.cache.as_ref().map(|c| c.repo_path().to_path_buf())?;
            handle.spawn(async move {
                let repo_path2 = repo_path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let repo = git2::Repository::open(&repo_path2).map_err(|e| e.message().to_string())?;
                    diff_blob_content(&repo, old_blob, new_blob)
                })
                .await
                .unwrap_or_else(|e| Err(format!("diff 内容加载任务失败: {e}")));
                emit(Message::DiffContentLoaded(commit, path, result));
            });
            None
        }
        Message::DiffContentLoaded(commit, path, result) => {
            if state.selected != Some(commit) || state.selected_file.as_deref() != Some(path.as_str()) {
                return None; // stale:用户已经切换了选择
            }
            state.loaded_diff = match result {
                Ok(content) => Some(LoadedDiff { commit, path, content }),
                Err(_) => None,
            };
            None
        }
```

Also update `SelectCommit`'s arm (currently ~line 368-381) to clear `loaded_diff` alongside the existing `state.detail = None; state.selected_file = None;` (it doesn't today, and a stale diff from the previous commit would otherwise linger visually for one frame):

```rust
        Message::SelectCommit(oid) => {
            state.selected = Some(oid);
            state.detail = None;
            state.selected_file = None;
            state.loaded_diff = None;
            let repo_path = state.cache.as_ref().map(|c| c.repo_path().to_path_buf())?;
            handle.spawn(async move {
                let repo_path2 = repo_path.clone();
                let result = tokio::task::spawn_blocking(move || commit_detail(&repo_path2, oid))
                    .await
                    .unwrap_or_else(|e| Err(format!("详情加载任务失败: {e}")));
                emit(Message::DetailLoaded(repo_path, oid, result));
            });
            None
        }
```

- [ ] **Step 6: Run tests to verify they pass**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo test -p dozer-app git_log:: 2>&1 | tail -80
```

Fix any mismatch between the test setup you wrote in Step 2 and the real private fields/constructors discovered while reading the surrounding tests (expected — the Step 2 code above is a best-effort sketch of the shape, not a guarantee of `GitLogSnapshot`'s exact private API).

- [ ] **Step 7: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git status --short
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git add crates/dozer-app/src/extensions/git_log.rs
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git commit -m "$(cat <<'EOF'
feat(git-log): async diff content loading with stale-selection guard

SelectFile now kicks an async blob-content load (git_log::update's
existing handle.spawn + spawn_blocking pattern, same shape as
SelectCommit's commit_detail load). DiffContentLoaded discards results
that no longer match the current commit/path selection, so fast
file-switching can't render content for the wrong file.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: `GitLogDiffWebviewEvent` message + runtime dispatch + Ready → queued `SetDiffDocument`

**Important — corrects an earlier draft of this task.** An earlier pass at
this task tried to call `self.webviews.get(...)` / `view.evaluate_script(...)`
directly from inside `App::update()`. That does not compile: `App` does not
own the `wry::WebView` pool at all. Reading `platform/window_events.rs` shows
the pool (`webviews: &mut HashMap<usize, (wry::WebView, String)>`) only
exists inside that file's `Self::Ready { app, webviews, .. }` match arm, and
the established pattern for "Rust decided a webview needs a new command" is a
**queue on `App`/pane state, drained once per frame** by
`apply_pending_editor_commands()` in `window_events.rs` (see
`preview/state.rs:627-630`'s `pending_editor_commands: Vec<(usize,
EditorCommand)>` and `app.rs:979` `take_preview_editor_scripts`, drained at
`window_events.rs:1813-1826`). This task reuses that same queue+drain shape,
sized down to GitLog's single-slot case (no `Vec`, just two flags — see
below) rather than reusing the tab-keyed `Vec`, which doesn't fit a panel
with no tabs.

Also note a real race the queue design must close: if content changes while
the webview is already mounted and previously marked ready, but the actual
push is skipped because the webview id momentarily wasn't in
`available_webview_ids`, the push must **retry on a later frame**, not be
dropped. And if the diff pane is deselected (webview torn down) and later
reselected (a **new** `wry::WebView` instance is created behind the same
pool id), the old "ready" flag must not be trusted for the new instance. Both
are handled below by (a) only clearing the "pending" state once a send
actually reaches an id present in `available_webview_ids`, and (b) resetting
`diff_webview_ready` at the exact points `loaded_diff` is cleared to `None`
(Task 4's `SelectCommit`/`SelectFile` arms), since that's exactly when the
webview would be torn down.

**Files:**
- Modify: `crates/dozer-app/src/app/message.rs` (new `Message` variant, near `EditorWebviewEvent` ~line 571)
- Modify: `crates/dozer-app/src/runtime.rs` (IPC handler branch ~line 480-524)
- Modify: `crates/dozer-app/src/app/update.rs` (new handler, near the existing `Message::EditorWebviewEvent` arm ~line 40)
- Modify: `crates/dozer-app/src/extensions/git_log.rs` (`State` gains `diff_webview_ready`/`diff_sent_for`; `SelectCommit`/`SelectFile` arms from Task 4 also reset `diff_webview_ready = false` alongside their existing `loaded_diff = None`)
- Modify: `crates/dozer-app/src/app/app.rs` (new `App::take_git_log_diff_script`)
- Modify: `crates/dozer-app/src/platform/window_events.rs` (`apply_pending_editor_commands()` ~line 1813-1826, drain the new queue)

**Interfaces:**
- Consumes: `EditorCommand::SetDiffDocument` (Task 2), `EditorHostBinding` with `PanelKind::GitLog` (Task 3), `State::loaded_diff()` (Task 4).
- Produces: `Message::GitLogDiffWebviewEvent(HostBinding, WebviewEnvelope<EditorEvent>)`, dispatched whenever the git log diff webview posts an IPC event; `App::take_git_log_diff_script(&mut self, available_webview_ids: &HashSet<usize>) -> Option<(usize, String)>`, consumed by Task 9's manual verification (no unit test reaches into the real `wry::WebView`, this is exercised end-to-end manually).

- [ ] **Step 1: Verify worktree branch**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git branch --show-current
```

- [ ] **Step 2: Add the `Message` variant and the two new `git_log::State` fields**

In `crates/dozer-app/src/app/message.rs`, after `EditorWebviewEvent` (~line 574):

```rust
    /// Git Log diff webview 发回的已校验协议事件。走独立分支而不是
    /// `EditorWebviewEvent`,因为那个 handler 硬编码假设 `ws.preview`/
    /// `ws.project_preview` 的 tab 模型(`PreviewTab`/`TabKind::File`)——
    /// git log 面板没有 tab,直接命中会要么找不到 tab 静默丢弃事件,要么
    /// (更糟)撞上 Files 预览恰好也是 tab_id=0 的 tab。绑定用轻量
    /// `HostBinding`(不需要 `EditorHostBinding.path`,git log 状态通过
    /// `App::git_log` 直接访问,不经路径查找)。
    GitLogDiffWebviewEvent(
        crate::preview::HostBinding,
        crate::preview::WebviewEnvelope<crate::preview::EditorEvent>,
    ),
```

In `crates/dozer-app/src/extensions/git_log.rs`, add two fields to `State` (next to `loaded_diff` from Task 4):

```rust
    /// 当前挂载的 diff webview 是否已经真正 `ready`(JS 端 `__dozer.dispatch`
    /// 已注册)。只由 `Message::GitLogDiffWebviewEvent` 的 `Ready` 分支置
    /// true;`loaded_diff` 被清空(见 `SelectCommit`/`SelectFile`)时连带置回
    /// false——webview 会被 `desired_webviews()` 判定为不再需要而销毁,
    /// 下次重新挂载是全新实例,必须等它自己的 `Ready` 才能再发命令。
    diff_webview_ready: bool,
    /// 最近一次**确认送达**(`take_git_log_diff_script` 在 webview 真的在池
    /// 里时才更新这个字段)的内容对应的 `(commit, path)`。跟 `loaded_diff`
    /// 的 `(commit, path)` 不一致就还需要再推一次;送达失败(webview 那一
    /// 帧还没进池)不更新,下一帧重试,不会丢内容。
    diff_sent_for: Option<(git2::Oid, String)>,
```

- [ ] **Step 3: Route GitLog-bound IPC events to the new message in `runtime.rs`**

In `crates/dozer-app/src/runtime.rs`, the branch that currently does (around line 480-524):

```rust
                                } else if let Some(binding) = editor_binding.as_ref() {
                                    let expected = crate::preview::HostBinding::new(
                                        binding.project_id,
                                        binding.panel,
                                        binding.tab_id,
                                        binding.document_id(),
                                    );
                                    if is_json_host {
                                        ...
                                    } else {
                                        match crate::preview::parse_event(body) {
                                            Ok(event) => {
                                                if let Err(error) = event.validate(&expected) {
                                                    tracing::warn!(%error, "拒绝无效 editor IPC");
                                                } else {
                                                    let _ = ipc_proxy.send_event(
                                                        Message::EditorWebviewEvent(
                                                            binding.clone(),
                                                            event,
                                                        ),
                                                    );
                                                }
                                            }
                                            Err(error) => {
                                                tracing::warn!(%error, "无法解析 editor IPC");
                                            }
                                        }
                                    }
                                } else {
```

Change the inner `else` branch (the non-JSON editor path) to fork on `binding.panel`:

```rust
                                    } else {
                                        match crate::preview::parse_event(body) {
                                            Ok(event) => {
                                                if let Err(error) = event.validate(&expected) {
                                                    tracing::warn!(%error, "拒绝无效 editor IPC");
                                                } else if binding.panel == crate::app::PanelKind::GitLog {
                                                    let _ = ipc_proxy.send_event(
                                                        Message::GitLogDiffWebviewEvent(
                                                            expected, event,
                                                        ),
                                                    );
                                                } else {
                                                    let _ = ipc_proxy.send_event(
                                                        Message::EditorWebviewEvent(
                                                            binding.clone(),
                                                            event,
                                                        ),
                                                    );
                                                }
                                            }
                                            Err(error) => {
                                                tracing::warn!(%error, "无法解析 editor IPC");
                                            }
                                        }
                                    }
```

(`expected` is already a `HostBinding` built two lines above — reuse it instead of constructing another one, and note it's consumed by value into `GitLogDiffWebviewEvent` so this must be the last use of `expected` in this arm; `EditorWebviewEvent`'s branch keeps using `binding.clone()` as before, unaffected.)

- [ ] **Step 4: Handle `GitLogDiffWebviewEvent` in `app/update.rs` — just flip the readiness flags, don't push directly**

Add a new match arm in `App::update()`, right after the existing `Message::EditorWebviewEvent` arm (which ends around line 200+ — find its closing `}` and insert after):

```rust
            Message::GitLogDiffWebviewEvent(_binding, event) => {
                if matches!(event.payload, crate::preview::EditorEvent::Ready { .. }) {
                    // 真正的推送发生在 `App::take_git_log_diff_script`(每帧轮询,
                    // 见下),这里只翻状态:webview 刚确认 `__dozer.dispatch`
                    // 已注册,`diff_sent_for` 清空强制下一帧重发一次当前内容
                    // (覆盖"webview 被销毁重建,新实例第一次 ready"的场景)。
                    self.git_log.diff_webview_ready = true;
                    self.git_log.diff_sent_for = None;
                }
                // 其余事件(selection_changed/viewport_changed/...):diff 面板
                // 恒只读、不需要 Agent 跳转/保存,忽略即可。
            }
```

- [ ] **Step 5: Add `App::take_git_log_diff_script`, called every frame from `window_events.rs`**

In `crates/dozer-app/src/app/app.rs`, near `take_preview_editor_scripts` (~line 979):

```rust
    /// Git Log diff webview 待下发的 `SetDiffDocument`(至多一条——单槽位,
    /// 不是 `Vec` 队列)。只在 `diff_webview_ready`(已确认 JS 端
    /// `__dozer.dispatch` 就绪)且内容跟 `diff_sent_for` 不一致时才产出;
    /// `available_webview_ids` 不含这个槽位 id 时返回 `None` 但**不**清空
    /// `diff_sent_for` 的"待发"状态(靠调用方不更新它来天然重试,下一帧
    /// webview 进池后会再算一次)。
    pub fn take_git_log_diff_script(
        &mut self,
        available_webview_ids: &std::collections::HashSet<usize>,
    ) -> Option<(usize, String)> {
        if !self.git_log.diff_webview_ready() {
            return None;
        }
        let loaded = self.git_log.loaded_diff()?;
        let crate::extensions::git_log::DiffBlobContent::Text { old_text, new_text } =
            &loaded.content
        else {
            return None;
        };
        let key = (loaded.commit, loaded.path.clone());
        if self.git_log.diff_sent_for() == Some(&key) {
            return None; // 已经送达这份内容,不重复发。
        }
        let id = GIT_LOG_DIFF_ID_OFFSET;
        if !available_webview_ids.contains(&id) {
            return None; // webview 这一帧还没进池,留着待发状态,下帧重试。
        }
        let language = crate::preview::extension_to_syntax(std::path::Path::new(&loaded.path));
        let cmd = crate::preview::EditorCommand::SetDiffDocument {
            old_text: old_text.clone(),
            new_text: new_text.clone(),
            language: language.to_string(),
            revision: 0,
            read_only: true,
        };
        let script = crate::preview::dispatch_script(&crate::preview::encode_command(
            0,
            PanelKind::GitLog,
            0,
            "p0-t0",
            0,
            None,
            cmd,
        ));
        self.git_log.set_diff_sent_for(key);
        Some((id, script))
    }
```

`encode_command`'s `document_id`/`project_id`/`tab_id`/`revision` arguments here must match exactly what `EditorHostBinding::new(0, PanelKind::GitLog, 0, ...)` produces via `.document_id()` (Task 3's fixed singleton binding — confirm the literal string by reading `EditorHostBinding::document_id()`'s format, e.g. `format!("p{}-t{}", project_id, tab_id)`, and use that exact value instead of a hand-typed `"p0-t0"` if it differs) — a mismatch here fails `HostBinding::validate()` silently (`tracing::warn!`, message dropped) the *next* time the webview posts an event, not on this send, so it's easy to miss without checking.

Add three small accessors/mutators to `git_log::State` (next to `loaded_diff()` from Task 4) so `app.rs` doesn't reach into private fields directly:

```rust
    pub fn diff_webview_ready(&self) -> bool {
        self.diff_webview_ready
    }

    pub fn diff_sent_for(&self) -> Option<&(git2::Oid, String)> {
        self.diff_sent_for.as_ref()
    }

    pub(crate) fn set_diff_sent_for(&mut self, key: (git2::Oid, String)) {
        self.diff_sent_for = Some(key);
    }
```

Also update Task 4's `SelectCommit` and `SelectFile` arms (`crates/dozer-app/src/extensions/git_log.rs`) to reset `diff_webview_ready` alongside their existing `loaded_diff = None` — the webview is about to be torn down (`desired_webviews()` goes empty once `loaded_diff` is `None`), so a future remount is a **new** `wry::WebView` instance that has not sent its own `Ready` yet:

```rust
        Message::SelectCommit(oid) => {
            state.selected = Some(oid);
            state.detail = None;
            state.selected_file = None;
            state.loaded_diff = None;
            state.diff_webview_ready = false;
            state.diff_sent_for = None;
            // ...unchanged rest of the arm from Task 4
```

```rust
        Message::SelectFile(path) => {
            state.selected_file = Some(path.clone());
            state.loaded_diff = None;
            state.diff_webview_ready = false;
            state.diff_sent_for = None;
            // ...unchanged rest of the arm from Task 4
```

- [ ] **Step 6: Drain the new queue once per frame in `window_events.rs`**

In `crates/dozer-app/src/platform/window_events.rs`, `apply_pending_editor_commands()` (~line 1813-1826) currently ends its `for kind in [PanelKind::Files, PanelKind::Project]` loop and returns. Add the GitLog drain right after that loop, still inside the same `let Self::Ready { app, webviews, .. } = self else { return };` borrow:

```rust
        if let Some((webview_id, js)) = app.take_git_log_diff_script(&available_webview_ids)
            && let Some((view, _)) = webviews.get(&webview_id)
        {
            let _ = view.evaluate_script(&js);
        }
```

- [ ] **Step 7: Build to catch type errors (no new unit test for this task — it's IPC/frame-pump glue, covered by the manual verification in Task 9; `take_git_log_diff_script`'s branching logic itself has no `wry::WebView` dependency and could be unit tested, but doing so would require constructing a real `git_log::State` plus an `App` — skipped as disproportionate to this task's risk, per the existing codebase's own choice not to unit test `take_preview_editor_scripts` either)**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo build -p dozer-app 2>&1 | tail -80
```

Fix compile errors against the real field/method names discovered while reading `window_events.rs`/`app.rs` in Steps 5-6 (e.g. if `PanelKind` needs an explicit `use` in `app.rs` at the call site, or `GIT_LOG_DIFF_ID_OFFSET`'s visibility needs adjusting for use inside `App`'s own `impl` block — it's `pub(crate)` from Task 3, which already covers this).

- [ ] **Step 8: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git status --short
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git add crates/dozer-app/src/app/message.rs crates/dozer-app/src/runtime.rs crates/dozer-app/src/app/update.rs crates/dozer-app/src/app/app.rs crates/dozer-app/src/platform/window_events.rs crates/dozer-app/src/extensions/git_log.rs
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git commit -m "$(cat <<'EOF'
feat(git-log): queue+drain SetDiffDocument push on webview ready

GitLogDiffWebviewEvent's Ready arm only flips a readiness flag;
App::take_git_log_diff_script (drained once per frame from
window_events.rs's apply_pending_editor_commands, alongside the
existing Files/Project queue) does the actual push once the webview
id is observed in the live pool. This mirrors the existing
pending_editor_commands queue+drain shape instead of trying to reach
a wry::WebView handle from inside App::update, which doesn't have
one. A content push that misses one frame (webview not yet in the
pool) retries on the next, so fast deselect/reselect cycles can't
silently drop content.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: GitLog diff pane geometry + `preview_desired` wiring

**Files:**
- Modify: `crates/dozer-app/src/webview_geometry.rs` (`PanelKind::GitLog` arms in `preview_content_bounds_for`, currently stubbed `(0.0, 0.0, 0.0, 0.0)` at two locations — the maximized branch ~line 100-103 and the normal branch ~line 228)
- Modify: `crates/dozer-app/src/app/app.rs` (`App::preview_desired`, add `PanelKind::GitLog` arm to the `match kind` at ~line 3006-3017)

**Interfaces:**
- Consumes: `State::loaded_diff()` (Task 4), `EditorHostBinding::diff_url()`/`webview_id()` (Task 3).
- Produces: a non-empty `desired_webviews()`-equivalent contribution to `App::preview_desired`'s output whenever GitLog is showing, a file is selected, and its content is loaded and renderable.

- [ ] **Step 1: Verify worktree branch**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git branch --show-current
```

- [ ] **Step 2: Write the failing geometry tests**

The test module already has a `fn test_state() -> ShellState` helper (~line 482, `left_view: PanelKind::Files` by default) and the established idiom for overriding just `left_view`/`left_collapsed` is struct-update syntax `ShellState { left_view: ..., ..test_state() }` (see `preview_content_bounds_conversations_zero_when_left_collapsed` ~line 534-545 for the exact collapsed-test shape to mirror). Add these two tests near it:

```rust
    #[test]
    fn gitlog_diff_bounds_nonzero_when_not_collapsed() {
        let state = ShellState {
            left_view: PanelKind::GitLog,
            ..test_state()
        };
        let (x, y, w, h) = preview_content_bounds_for(Side::Left, 1600.0, 900.0, &state);
        assert!(w > 0.0, "GitLog 显示时 diff pane 矩形宽度应该 > 0");
        assert!(h > 0.0, "GitLog 显示时 diff pane 矩形高度应该 > 0");
        assert!(y > 0.0, "diff pane 应该在窗口顶栏之下");
        assert!(x >= 0.0);
    }

    #[test]
    fn gitlog_diff_bounds_zero_when_side_collapsed() {
        let state = ShellState {
            left_view: PanelKind::GitLog,
            left_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            preview_content_bounds_for(Side::Left, 1600.0, 900.0, &state),
            (0.0, 0.0, 0.0, 0.0)
        );
    }
```

(The `left_collapsed` guard is a single early-return at the very top of `preview_content_bounds_for`, before the `match kind` block — line ~42-48 — shared by every `PanelKind` including the new `GitLog` arm, so this second test passes as soon as the stub is replaced, regardless of the GitLog-specific geometry logic in Step 4.)

- [ ] **Step 3: Run to verify failure (or confirm zero-rect from the current stub)**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo test -p dozer-app webview_geometry:: gitlog 2>&1 | tail -40
```

Expected: `gitlog_diff_bounds_nonzero_when_not_collapsed` fails (`w`/`h` are `0.0` from the current stub); `gitlog_diff_bounds_zero_when_side_collapsed` already passes trivially.

- [ ] **Step 4: Replace both `PanelKind::GitLog` stubs**

Maximized branch (~line 100-103), replace:

```rust
            // Git 提交图是原生 Canvas 绘制,不挂 webview 子视图。
            PanelKind::GitLog
            // Todo 面板同 GitLog,纯 iced 绘制,不挂 webview 子视图。
            | PanelKind::Todo => (0.0, 0.0, 0.0, 0.0),
```

with:

```rust
            // Todo 面板纯 iced 绘制,不挂 webview 子视图。
            PanelKind::Todo => (0.0, 0.0, 0.0, 0.0),
            // Git Log 面板放大态:目前不支持 Git 面板放大(规格未覆盖),
            // 按零矩形处理,同收起态。
            PanelKind::GitLog => (0.0, 0.0, 0.0, 0.0),
```

Normal branch (~line 228), replace:

```rust
        PanelKind::GitLog => (0.0, 0.0, 0.0, 0.0),
        // Todo 面板同 GitLog,纯 iced 绘制,不挂 webview 子视图。
        PanelKind::Todo => (0.0, 0.0, 0.0, 0.0),
```

with:

```rust
        // 右下 diff 内容 pane 的矩形:先按 `git_log_split` 取右配对列(同
        // Files/Project 用 `pair_columns` 取内容列的手法),再在列内部按
        // `git_log_file_diff_split` 纵向切出下半(diff),减去文件计数
        // 头(caption 行 + spacing(6) + 1px 分割线 + padding 上下各
        // 4≈caption*1.2+15)与上下分栏拖拽条厚度
        // (`divider_width()`)得到 diff 容器起点,再减去 diff pane 自己的
        // 路径头行(caption 行 + spacing(4)≈caption*1.2+4)得到 webview
        // 实际落点。像素级精度依赖 `git_log::view()` 的实际渲染结果,
        // 这里同 `layout.rs::apply_row_drag` 的既有精度承诺——近似值,
        // 人工验收阶段按实际效果微调,不追求逐像素对齐。
        PanelKind::GitLog => {
            let y = y_top(0.0);
            let h = h_for(y);
            let cols = pair_columns(zone_w, state.dims.git_log_split, mirrored);
            let col_x = zone_x0 + cols.content_x + m.left;
            let col_w = (cols.content_w - m.left - m.right).max(0.0);
            let caption_line = byteui::theme::font::caption() as f32 * 1.2;
            let file_count_header = caption_line + 6.0 + 1.0 + 8.0;
            let split_divider = byteui::theme::geometry::divider_width();
            let diff_path_header = caption_line + 4.0;
            let (top_portion, bottom_portion) = crate::workspace::split_portions(state.dims.git_log_file_diff_split);
            let total_portion = (top_portion as f32 + bottom_portion as f32).max(1.0);
            let usable = (h - file_count_header - split_divider).max(0.0);
            let diff_container_h = usable * (bottom_portion as f32 / total_portion);
            let diff_container_y = y + file_count_header + split_divider + (usable - diff_container_h);
            let diff_y = diff_container_y + diff_path_header;
            let diff_h = (diff_container_h - diff_path_header).max(0.0);
            (col_x, diff_y, col_w, diff_h)
        }
        // Todo 面板纯 iced 绘制,不挂 webview 子视图。
        PanelKind::Todo => (0.0, 0.0, 0.0, 0.0),
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo test -p dozer-app webview_geometry:: 2>&1 | tail -60
```

- [ ] **Step 6: Wire `App::preview_desired`**

In `crates/dozer-app/src/app/app.rs`, the `match kind` inside `preview_desired` (~line 3006-3017) currently:

```rust
            let (mut specs, id_offset): (Vec<WebviewSpec>, usize) = match kind {
                PanelKind::Files => (ws.preview.desired_webviews(), 0),
                PanelKind::Project => (
                    ws.project_preview.desired_webviews(),
                    PROJECT_PREVIEW_ID_OFFSET,
                ),
                PanelKind::Conversations => (
                    crate::workspace::review_webview_spec(ws.review.as_ref()),
                    CONVERSATION_REVIEW_ID_OFFSET,
                ),
                _ => continue,
            };
```

Add a `PanelKind::GitLog` arm before `_ => continue`:

```rust
                PanelKind::GitLog => (self.git_log_desired_webviews(), 0),
```

Add the helper method on `App` (near `sync_git_log_to_active_project`, end of the `impl App` block in `app.rs`):

```rust
    /// Git Log 面板 diff webview 的期望清单:面板未选中可渲染内容时为空
    /// (`sync_webview_pool` 据此销毁已有实例,面板退回 iced 占位文案)。
    /// 单槽位,不按 tab 池化——见 `EditorHostBinding::webview_id()` 对
    /// `PanelKind::GitLog` 的固定偏移处理。
    fn git_log_desired_webviews(&self) -> Vec<WebviewSpec> {
        let Some(loaded) = self.git_log.loaded_diff() else {
            return Vec::new();
        };
        if !matches!(
            loaded.content,
            crate::extensions::git_log::DiffBlobContent::Text { .. }
        ) {
            return Vec::new();
        }
        let binding = crate::preview::EditorHostBinding::new(
            0,
            PanelKind::GitLog,
            0,
            std::path::PathBuf::from(&loaded.path),
        );
        let url = binding.diff_url(crate::preview::scheme_query_value());
        vec![WebviewSpec {
            id: binding.webview_id(),
            url,
            visible: true,
            editor_binding: Some(binding),
        }]
    }
```

(`scheme_query_value()` is `pub(crate) fn scheme_query_value() -> &'static str` in `preview/webview.rs:57`, re-exported crate-wide via `preview/mod.rs`'s `pub(crate) use webview::*;` — `crate::preview::scheme_query_value()` is the correct path, confirmed by grepping its one existing call site in `preview/view.rs::desired_editor_webviews` which calls it bare via that same re-export, not `crate::theme::...`.)

- [ ] **Step 7: Build to confirm it compiles**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo build -p dozer-app 2>&1 | tail -60
```

- [ ] **Step 8: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git status --short
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git add crates/dozer-app/src/webview_geometry.rs crates/dozer-app/src/app/app.rs
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git commit -m "$(cat <<'EOF'
feat(git-log): diff pane webview geometry + preview_desired wiring

GitLog's two previously-stubbed zero-rect branches in
preview_content_bounds_for now compute the right-bottom diff pane's
actual rect from the existing git_log_split/git_log_file_diff_split
ratios, mirroring the approximate-pixel-offset style already used for
Files/Project (exact alignment left to manual QA tuning, same as the
existing row-drag comment already documents for this panel).
App::preview_desired gains a PanelKind::GitLog arm producing at most
one WebviewSpec, present only when a renderable diff is loaded.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: `diff_pane_view` routing (webview area vs iced fallback)

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs` (`diff_pane_view()` ~line 1037-1100, its caller `view()` ~line 1003)

**Interfaces:**
- Consumes: `State::loaded_diff()` (Task 4).
- Produces: `diff_pane_view` now takes `&State` (not just `detail`/`selected_file`) so it can read `loaded_diff()` and decide whether to reserve an empty area for the webview.

- [ ] **Step 1: Verify worktree branch**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git branch --show-current
```

- [ ] **Step 2: Read how Files/Project reserve empty iced area for their webview**

Before writing code, grep `preview/view.rs` or `workspace/view.rs` for how the Files preview tab's content area leaves a gap for the composited webview (search for a `Space` or empty `container` sized to fill where the webview sits — likely in the function that renders the active preview tab's body when its backend is `PreviewBackend::Editor`/similar). Copy that exact widget shape (almost certainly `container(iced_widget::Space::new()).width(Length::Fill).height(Length::Fill)`, styled with the current background color so there's no flash of the wrong color before the webview paints over it) rather than inventing a new one.

- [ ] **Step 3: Update `diff_pane_view` to branch on `loaded_diff()`**

Change the function signature and body (currently ~line 1037-1100):

```rust
fn diff_pane_view<'a>(
    state: &'a State,
    detail: &'a Result<CommitDetail, String>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Ok(detail) = detail else {
        return container(iced_widget::Space::new()).into();
    };
    let Some(path) = state.selected_file.as_deref() else {
        return container(
            text("未选中文件")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
        .padding(8)
        .into();
    };
    let Some(entry) = detail.files.iter().find(|f| f.path == path) else {
        return container(
            text("未选中文件")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
        .padding(8)
        .into();
    };

    // CodeMirror 常开:已加载出可渲染内容时,只画路径头 + 一块留给 webview
    // 合成的空区域(同 Files 预览 tab 的既有手法);其余情况(未加载/二进制/
    // 超限/加载失败)保留原 iced 文案渲染(`colored_diff_lines` 逐行染色),
    // 不强行往 CodeMirror 里塞任何东西。
    let header = text(entry.path.clone())
        .size(byteui::theme::font::caption())
        .color(byteui::theme::color::current().cream);

    match state.loaded_diff() {
        Some(loaded) if loaded.path == path && matches!(loaded.content, DiffBlobContent::Text { .. }) => {
            column![
                header,
                container(iced_widget::Space::new())
                    .width(Length::Fill)
                    .height(Length::Fill),
            ]
            .spacing(4)
            .into()
        }
        Some(loaded) if loaded.path == path => {
            // 加载完成但判定不可渲染(二进制/超限)——回落文案,不落 webview。
            column![
                header,
                text("(二进制文件或超出大小上限,不支持预览)")
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().dim),
            ]
            .spacing(4)
            .into()
        }
        _ => {
            // 还没加载完(或加载失败/stale):回落到现有 patch 文本渲染,
            // 不留白屏空等。
            let mut content = column![header].spacing(4);
            if entry.patch.is_empty() {
                content = content.push(
                    text("(无 diff 内容)")
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().dim),
                );
            } else {
                content = content.push(crate::extensions::diff_render::colored_diff_lines(
                    &entry.patch,
                ));
            }
            if entry.truncated {
                content = content.push(
                    text("… diff 过长,已截断显示")
                        .size(byteui::theme::font::caption_sm())
                        .color(byteui::theme::color::current().dim),
                );
            }
            scrollable(content)
                .direction(scrollable::Direction::Vertical(
                    byteui::interaction::scrollbar::scrollbar(),
                ))
                .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style())
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
    }
}
```

Update the call site in `view()` (~line 1003):

```rust
                container(diff_pane_view(state, detail))
                    .height(Length::FillPortion(bottom_portion)),
```

(previously `diff_pane_view(detail, state.selected_file.as_deref())`).

- [ ] **Step 4: Build to confirm it compiles, then run the full git_log test suite**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo build -p dozer-app 2>&1 | tail -60
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo test -p dozer-app git_log:: 2>&1 | tail -60
```

No new unit tests here (this is a pure-rendering function returning an `Element`, which this codebase's existing `git_log.rs` tests don't snapshot-test — consistent with the surrounding file's existing test coverage, which tests `commit_detail`/`build`/`update` logic, not `view` output shape).

- [ ] **Step 5: Commit**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git status --short
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git add crates/dozer-app/src/extensions/git_log.rs
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git commit -m "$(cat <<'EOF'
feat(git-log): diff_pane_view reserves webview area when content renderable

Renders the path header plus an empty area for the CodeMirror webview
to composite over when a renderable diff has loaded; otherwise keeps
the existing colored_diff_lines fallback (not loaded yet, binary,
oversized, or load failed) so there's never a blank wait state.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: `mode=diff` bootstrap + `unifiedMergeView` in the editor web bundle

**Files:**
- Modify: `crates/dozer-app/web/editor/package.json` (add `@codemirror/merge` dependency)
- Modify: `crates/dozer-app/web/editor/src/main.ts` (`mode=diff` branch, gate the `__file__` fetch, apply `set_diff_document`)

**Interfaces:**
- Consumes: `set_diff_document` TS type/decoder (Task 2), `languageFor()` (existing, `languages.ts`).
- Produces: a working `unifiedMergeView` rendering when the host is loaded with `mode=diff`.

- [ ] **Step 1: Verify worktree branch**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git branch --show-current
```

- [ ] **Step 2: Add the dependency**

In `crates/dozer-app/web/editor/package.json`, add to `dependencies` (alphabetically, matching the file's existing sort order). Verified against the live npm registry (`npm view @codemirror/merge versions`) at plan-writing time: latest is `6.12.2`, and its own `dependencies` (`npm view @codemirror/merge@6.12.2 dependencies`) require `@codemirror/state@^6.0.0`, `@codemirror/view@^6.17.0`, `@codemirror/language@^6.0.0` — all satisfied by this project's already-pinned `6.7.6`/`6.43.13`/`6.12.4`. Use `6.12.2` exactly, not a guessed number:

```json
    "@codemirror/merge": "6.12.2",
```

Then install and verify it resolves cleanly:

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff/crates/dozer-app/web/editor && npm install
```

Expected: no peer-dependency conflicts reported. If, by the time this task actually runs, npm has a newer `6.x` and `6.12.2` is no longer installable for some reason, adjust to the actual latest compatible `6.x` version npm resolves — but `6.12.2` is a verified real version, not a placeholder, so this should not be needed in practice.

- [ ] **Step 3: Add the `mode` param and gate the file fetch**

In `crates/dozer-app/web/editor/src/main.ts`, near the other `params.get(...)` reads (~line 80-90):

```typescript
const mode = params.get('mode') ?? 'file';
const isDiffMode = mode === 'diff';
```

Change `boot()`'s fetch gate (currently `if (!windowed) { ... fetch ... }`, ~line 596-608):

```typescript
async function boot(): Promise<void> {
  let text = '';
  if (!windowed && !isDiffMode) {
    try {
      const res = await fetch('__file__' + encodePathForFetch(filePath));
      if (res.ok) {
        text = await res.text();
      } else {
        post({ kind: 'failed', message: `读取文件失败: ${res.status}`, recoverable: true });
      }
    } catch (err) {
      post({ kind: 'failed', message: `读取文件异常: ${String(err)}`, recoverable: true });
    }
  }

  if (isDiffMode) {
    mountDiffView();
  } else {
    view = new EditorView({
      state: EditorState.create({ doc: text, extensions: buildExtensions() }),
      parent: document.getElementById('editor')!,
    });
  }
  // ... rest of boot() (lossy/utf16 banner, windowed scrollbar, `post({kind:'ready',...})`, emitViewport(), view.focus())
  // unchanged, EXCEPT: skip the lossy/utf16 banner block and the windowed-scrollbar
  // block entirely when isDiffMode (diff mode is never lossy/utf16/windowed) —
  // wrap both of those existing `if` blocks with `&& !isDiffMode` in their conditions.
```

- [ ] **Step 4: Add `mountDiffView()` using `@codemirror/merge`**

Add near `buildExtensions()`:

```typescript
import { unifiedMergeView } from '@codemirror/merge';

function mountDiffView(): void {
  view = new EditorView({
    state: EditorState.create({
      doc: '',
      extensions: [
        ...buildExtensions(),
        unifiedMergeView({
          original: '',
          mergeControls: false,
          highlightChanges: true,
        }),
      ],
    }),
    parent: document.getElementById('editor')!,
  });
}
```

Note: `@codemirror/merge@6.12.2`'s real exported API (checked directly against `dist/index.d.ts` while writing this plan — downloaded and inspected the published tarball, not guessed) does expose an incremental path: `updateOriginalDoc: StateEffectType<{doc: Text, changes: ChangeSet}>`, dispatchable on the view to update just the "original" side without a full rebuild. Deliberately **not using it** here: correctly constructing the `ChangeSet` argument (and coordinating it in the same transaction as a full-document replace on the "current" side, plus re-running the `languageCompartment`/`readOnlyCompartment` reconfiguration for a possible language change) is real incremental-editing complexity for a code path that only fires when the user clicks a different file in the list — not a hot path, and a brief remount is imperceptible. Step 5 below uses the full `view.setState(EditorState.create({...}))` rebuild unconditionally, matching the existing, already-proven pattern `set_document`/`set_window` use for exactly this kind of "swap the whole document" event. Lower risk, same visible behavior, no benefit foregone.

- [ ] **Step 5: Handle `set_diff_document` in `applyCommand`**

Add a case in the `switch (cmd.kind)` block in `applyCommand()` (~line 487, after `case 'set_document':`), using whichever approach Step 4's `.d.ts` investigation settled on. The full-rebuild fallback (guaranteed correct even if the incremental API turns out to be awkward to wire correctly under time pressure):

```typescript
    case 'set_diff_document': {
      revision = cmd.revision;
      view.setState(
        EditorState.create({
          doc: cmd.new_text,
          extensions: [
            ...buildExtensions(),
            languageCompartment.of(languageFor(cmd.language) ?? []),
            readOnlyCompartment.of(readOnlyExtensions(true)),
            unifiedMergeView({
              original: cmd.old_text,
              mergeControls: false,
              highlightChanges: true,
            }),
          ],
        }),
      );
      break;
    }
```

- [ ] **Step 6: Build the bundle and typecheck**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff/crates/dozer-app/web/editor && npm run typecheck
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff/crates/dozer-app/web/editor && npm run build
```

Expected: typecheck passes; build produces `crates/dozer-app/assets/editor/editor.js`/`editor.css` without errors. Fix any type errors against `@codemirror/merge`'s real exported API surface discovered in Step 4.

- [ ] **Step 7: Commit (including the rebuilt bundle output, per this repo's existing convention of committing `assets/editor/` — verify by checking whether `crates/dozer-app/assets/editor/editor.js` is currently tracked in git before assuming; if it's gitignored, skip adding it and note that in the commit message instead)**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git status --short
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git check-ignore -v crates/dozer-app/assets/editor/editor.js || echo "tracked, will add"
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git add crates/dozer-app/web/editor/package.json crates/dozer-app/web/editor/package-lock.json crates/dozer-app/web/editor/src/main.ts
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git commit -m "$(cat <<'EOF'
feat(editor-web): unifiedMergeView for git log diff mode

New mode=diff bootstrap path in the shared CodeMirror host bundle:
mounts @codemirror/merge's unifiedMergeView instead of fetching a
file, and applies set_diff_document commands by rebuilding the editor
state with both documents (same full-rebuild pattern set_document
already uses).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: Workspace-wide verification + manual QA pass

**Files:** none new — this task verifies Tasks 1-8's combined output.

**Interfaces:** none new.

- [ ] **Step 1: Verify worktree branch**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git branch --show-current
```

- [ ] **Step 2: Full workspace build, test, clippy, fmt**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo build 2>&1 | tail -80
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo test -p dozer-app 2>&1 | tail -100
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo clippy --all-targets 2>&1 | tail -100
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo fmt --check
```

Fix anything clippy/fmt flag; if `cargo fmt --check` fails, run `cargo fmt` and re-commit the formatting fix as its own small commit.

- [ ] **Step 3: Editor web bundle test + typecheck one more time (catches drift if Task 8 was done before later Rust-side renames)**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff/crates/dozer-app/web/editor && npm test
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff/crates/dozer-app/web/editor && npm run typecheck
```

- [ ] **Step 4: Manual verification in the running app**

Per user instructions, UI changes require actually running the app and exercising the golden path plus edge cases before claiming success — automated tests here don't cover rendering/visual correctness. Run:

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && cargo run -p dozer-app
```

Manually check, in a project with real git history:
1. Open the Git Log panel, select a commit, select a modified text file — diff pane shows CodeMirror with real syntax highlighting and char-level diff highlighting (green/red), not the old plain-colored patch text.
2. Select an added file (no parent version) — old side renders empty, whole file shows as added.
3. Select a deleted file (in a commit that deletes one) — new side renders empty, whole file shows as removed.
4. Select a binary file (if the test repo has one, or add one temporarily) — falls back to the "不支持预览" placeholder text, no CodeMirror, no crash.
5. Rapidly click through several files in the file list — no flash of the previous file's diff content, no console/log errors about stale/mismatched envelopes.
6. Resize the window and drag both the `GitLogSplit` and `GitLogFileDiffSplit` dividers — webview rect tracks the resized/dragged panel without visibly detaching from the iced-drawn header/border (small pixel misalignment is acceptable per this task's Global Constraints and Task 6's documented approximation — a full detachment or the webview rendering outside the diff pane's visible box is not).
7. Switch away from the Git Log panel and back — webview is torn down and recreated cleanly (no stale content flash, no duplicate webview).

- [ ] **Step 5: Report findings**

If manual verification (Step 4) surfaces a real bug, fix it as a small additional commit on this same branch before declaring the plan complete — do not silently ship a known-broken interaction. If Step 4 only surfaces pixel-alignment roughness consistent with what Task 6 already flagged as expected, note it for the human reviewer rather than trying to chase pixel-perfect alignment inside this plan (that's explicitly out of scope per the Global Constraints and Task 6's own documentation).

- [ ] **Step 6: Final commit and open for review**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git status --short
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-git-log-diff && git log --oneline main..HEAD
```

Confirm the branch's commit log matches the eight feature commits from Tasks 1-8 (plus any Step 2 fmt-fix commit) and nothing else leaked in. Do not merge to `main` — hand off for review per the Global Constraints.

---

## Self-Review Notes (writing-plans skill checklist, run against the spec)

1. **Spec coverage**: 架构(单槽位绑定, Task 3/6) / 数据流(Task 1/4/5) / 二进制与大小判定(Task 1) / 测试(每个 task 自带) 均有对应任务。规格明确排除的 file_history 弹窗迁移、并排视图、整文件浏览三项未在计划中出现——符合规格"非目标"。
2. **Placeholder scan**: 每个 code step 都是具体代码,没有 TBD/"add appropriate"/"similar to Task N" 这类占位;Task 4/6/8 里少数几处明确标注"必须先读真实代码确认字段名/API 再落笔"的地方,是因为那些字段/API 名称在规划阶段确实无法从已读代码确定(`GitLogSnapshot` 私有构造、`self.webviews` 真实字段名、`@codemirror/merge` 的增量重配置 API),每处都给了一个保底可行方案(读现有相邻测试抄写、或退回已验证过的"整体 setState 重建"手法),不是留白甩给执行者猜。
3. **Type consistency**: `DiffFileEntry.old_blob`/`new_blob`(Task 1)→`diff_blob_content`签名(Task 1)→`git_log::update`里的调用(Task 4)→`LoadedDiff.content: DiffBlobContent`(Task 4)→`Message::GitLogDiffWebviewEvent`处理里的 `DiffBlobContent::Text` 解构(Task 5)→`git_log_desired_webviews`里的 `matches!(loaded.content, ...)`(Task 6)→`diff_pane_view`里的同款匹配(Task 7)全程用同一个 `DiffBlobContent` 类型和字段名,未出现改名不一致。`EditorCommand::SetDiffDocument`(Task 2)字段名在 Task 5(Rust 构造)与 Task 8(TS 解构 `cmd.old_text`/`cmd.new_text`/...)两侧一致。
4. **Review Focus wiring**: 二进制误判/超限边界/新增删除文件边界→Task 1 测试;stale 选择竞态→Task 4 测试;`panel_token` 三处漂移→Task 3 测试(三处全改 + 一个跨 `validate()` 的间接验证测试)。五条全部有对应任务的真实测试,不是空列表。

**协调者复核补充(写计划的 fork 完成后,由发起会话逐条核实并修正)**:上面第 2 条列的几处"待确认"逐一查了真实代码,结果不全是"占位但有保底方案"那么无害——

- `GitLogSnapshot` 私有构造:确认 `snapshot_at()` 测试 helper 已存在(git_log.rs:1517),Task 4 的三个测试已改写成用它 + 现有 `#[tokio::test]`/`Handle::current()` 惯用法(原稿手写了一个不存在的 `GitLogSnapshot::for_test_with_repo_path` 和不必要的手动 `Runtime::new()`)。
- `self.webviews`/`evaluate_script`:**不是字段名猜错,是整个方案在架构上不成立**——`App::update()` 根本拿不到 `wry::WebView` 池(池只活在 `window_events.rs` 的 `Self::Ready{app,webviews,..}` 作用域里)。已重写 Task 5:改成现有 `pending_editor_commands` 队列同款的"排队 + 每帧轮询消费"模式(新增 `App::take_git_log_diff_script`,在 `window_events.rs::apply_pending_editor_commands` 里跟 Files/Project 那段一起消费),并补上一个原稿没考虑到的竞态(webview 被销毁重建后,旧的"已 ready"状态不能延续到新实例,否则内容会静默丢失且不重试)。
- `@codemirror/merge` 增量重配置 API:直接下载 `6.12.2` 的 `dist/index.d.ts` 核实——`updateOriginalDoc`/`mergeControls`/`highlightChanges` 确实存在,但正确拼出 `updateOriginalDoc` 需要的 `ChangeSet` 参数、且要跟"当前文档"那侧的整体替换协调在同一个 transaction 里,复杂度和风险不成比例(这个路径只在用户切换选中文件时触发,不是高频路径)。已把 Task 8 从"读完 .d.ts 再决定"的悬而未决改成明确结论:就用整体 `setState` 重建,不追求增量。
- `@codemirror/merge` 版本号:`6.10.4` 在 npm 上根本不存在(实查 `npm view` 版本列表,6.10.2 之后直接跳 6.11.0)。已换成实查到的最新版 `6.12.2`,并核实过它的 `dependencies` 跟本仓库已锁定的 `@codemirror/state`/`@codemirror/view`/`@codemirror/language` 版本兼容。
- Task 6 的两处 geometry 测试:原稿用注释占位(`/* same ShellState builder ... */`)代替真代码,违反"no placeholders"——已用真实存在的 `test_state()` helper 改写成可编译的测试。
- Task 6 的 `scheme_query_value`:原稿猜的路径是 `crate::theme::scheme_query_value()`,实际是 `crate::preview::scheme_query_value()`(定义在 `preview/webview.rs`,经 `preview/mod.rs` 的 `pub(crate) use webview::*;` 重导出)。已修正。
