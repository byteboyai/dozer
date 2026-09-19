# JSON Preview (Tree Viewer) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a read-only, tree-based JSON/JSONL preview to `dozer-app`'s Preview module, fast enough to open at the 1GB scale, following the async-loading/streaming/capped-and-virtualized principles established by the Tabular (Excel/CSV) Viewer.

**Architecture:** New `crates/dozer-app/src/json_tree/` module: lazy per-node decoding via `sonic-rs`'s `get()` path API (no full-document materialization), a canvas-based virtualized tree widget (mirrors `tabular/grid.rs`), and the same background-thread + `Message` routing + `loading_hint` async pattern already proven by the tabular work. Unlike tabular, JSON files keep the existing native code editor alive alongside the tree (dual view, toggle button) instead of replacing it.

**Tech Stack:** Rust, iced 0.14 (`iced_widget::canvas`), `sonic-rs` (new dependency), existing `byteui` theme/icon/feedback crates.

**Spec:** `docs/superpowers/specs/2026-09-19-json-preview-design.md`

## Global Constraints

- Read-only. No editing/write-back UI anywhere in this feature (CLAUDE.md core principle).
- No exact totals for anything past a cap — approximate/truncated is correct behavior, not a shortcut (established by the tabular fix; re-litigating this per-task is out of scope).
- `MAX_JSON_CHILDREN = 10_000`, `MAX_LEAF_PREVIEW_CHARS = 2_000`, `MAX_JSON_LINES = 100_000` (exact values from the spec; may be tuned later, not during this plan).
- All parsing/decoding that touches a file runs on a background thread (`tokio::runtime::Handle::spawn_blocking`) — never on the UI thread. This includes the initial open AND every on-demand node expansion.
- Async results route by `ProjectId`, never "currently focused project" (this codebase's standing multi-project invariant — see `App::with_project` doc comment in `crates/dozer-app/src/app/app.rs`).
- New iced UI must NOT use `JetBrains Mono` / `code_font()` for anything except values that are already code (this feature has none — tree labels/values use `Font::default()` per CLAUDE.md's font-unification rule), and any non-ASCII text rendering must use `Shaping::Advanced`, never `Shaping::Basic`.
- Work happens on an isolated git worktree/branch (`feature/json-preview-viewer`), created via the `superpowers:using-git-worktrees` skill before Task 1 starts. Do not commit to `main` directly; this codebase's `main` frequently has other agents' concurrent uncommitted WIP — never stage or revert files you didn't touch (`git status` before every commit, stage by explicit path).
- Every task's code must build (`cargo build -p dozer-app`) and its own tests must pass (`cargo test -p dozer-app <module>::`) before moving to the next task. Run `cargo fmt -p dozer-app` before each commit; if it reformats a file this plan didn't touch, revert that file (same lesson learned during the tabular work).

---

## Reference: types this plan locks in (read this before any task)

These are the exact names/signatures every task below uses. If a task's code disagrees with this block, this block wins — fix the task.

```rust
// crates/dozer-app/src/json_tree/mod.rs

pub const MAX_JSON_CHILDREN: usize = 10_000;
pub const MAX_LEAF_PREVIEW_CHARS: usize = 2_000;
pub const MAX_JSON_LINES: usize = 100_000;

pub fn is_json_tree_extension(path: &std::path::Path) -> bool;

pub type NodePath = std::rc::Rc<[PathSegment]>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonKind {
    Object,
    Array,
    String,
    Number,
    Bool,
    Null,
}

#[derive(Debug, Clone)]
pub enum NodeContent {
    Object { entries: Vec<(String, JsonKind)>, truncated: bool },
    Array { items: Vec<JsonKind>, truncated: bool },
    Leaf { display: String, truncated: bool },
}

#[derive(Debug, Clone)]
pub struct JsonNode {
    pub kind: JsonKind,
    /// `None` until something has decoded this node's direct children /
    /// leaf text at least once (root nodes get this eagerly on load;
    /// everything else is lazy, filled in by `apply_node_loaded`).
    pub content: Option<NodeContent>,
}

/// One JSON document's (or one JSONL line's) decoded root, or the error
/// that root produced. A `.json` file always has exactly one entry here;
/// `.jsonl`/`.ndjson` has one per line (capped at `MAX_JSON_LINES`).
pub type RootResult = Result<JsonNode, String>;

/// **Note:** this shows `JsonTreeView`'s final shape once every task below
/// has landed — Task 2 introduces it with `bytes`/`line_ranges`/`roots`/
/// `expanded`/`loading_nodes`/`view_mode`/`scroll_row` only, and adds
/// `decoded` a few steps later in the same task; `path` is added here (not
/// retrofitted later) specifically so `JsonTreeView::new`'s signature never
/// changes after Task 2 — every task's code below already includes `path`.
pub struct JsonTreeView {
    /// Source file path — re-read from disk on every lazy node expansion
    /// (see `expand()` in Task 5's doc comment for why that's an accepted
    /// tradeoff, not an oversight) and used by `expand_source_for` (Task 9).
    path: std::path::PathBuf,
    /// Resident source bytes: for `.json`, the whole file; for
    /// `.jsonl`/`.ndjson`, also the whole file (lines are sliced out of
    /// it via `line_ranges`, not stored separately — avoids duplicating
    /// large files in memory).
    bytes: Vec<u8>,
    /// `Some` for jsonl/ndjson (index-aligned with `roots`), `None` for a
    /// single `.json` file (root 0 spans the whole `bytes`).
    line_ranges: Option<Vec<std::ops::Range<usize>>>,
    pub roots: Vec<RootResult>,
    expanded: std::collections::HashSet<NodePath>,
    /// Decoded content for non-root nodes, keyed by path (root nodes' own
    /// content lives directly on `roots[i]`'s `JsonNode.content` instead).
    decoded: std::collections::HashMap<NodePath, NodeContent>,
    loading_nodes: std::collections::HashSet<NodePath>,
    pub view_mode: ViewMode,
    pub scroll_row: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Tree,
    RawText,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Scroll { dy: i32 },
    ToggleExpand(NodePath),
    ToggleViewMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeExpandRequest {
    pub path: NodePath,
    pub root_index: usize,
}
```

**Why `NodeContent`'s `entries`/`items` hold `JsonKind` instead of nested `JsonNode`:** a decode call only ever resolves *one* node's direct children shape (kind per child), never recurses — recursing would defeat the whole point of lazy decoding. A child becomes a real, independently-expandable `JsonNode` (with its own `content: None`) only when the tree flattener (Task 7) builds a row for it; the tree widget looks up "is this child's path in `expanded`/does it have a cached decode" rather than the parent's `NodeContent` owning a fully-typed child tree. Task 7 defines the exact row/cache structure that reconciles this — flagged here so Task 3 onward doesn't assume child nodes are pre-built.

**Why `sonic-rs`'s byte-offset bookkeeping from the spec is gone:** verified against `docs.rs/sonic-rs` (see Task 1) that `sonic_rs::get(bytes, path)` re-resolves a path from the start of the given byte slice on every call — no manual byte-range/`ExpandSource` tracking needed, we just keep `bytes` resident and re-invoke `get()` with the target `NodePath` converted to a plain index slice each time a node is expanded. This is simpler than the spec sketched; the spec's design doc will get a short addendum in Task 1's commit message rather than a full rewrite.

---

### Task 1: Module skeleton, core types, and the sonic-rs decode primitive (spike)

This is both the risk-resolution spike from the spec *and* the first real piece of production code — there is no throwaway/separate spike phase, the code written here is what ships.

**Files:**
- Create: `crates/dozer-app/src/json_tree/mod.rs`
- Modify: `crates/dozer-app/Cargo.toml` (add `sonic-rs` dependency)
- Modify: `crates/dozer-app/src/main.rs` or wherever top-level modules are declared, to add `mod json_tree;` (check how `mod tabular;` is currently declared and mirror it exactly)
- Test fixtures: `crates/dozer-app/src/json_tree/mod.rs` inline `#[cfg(test)]` module (uses `tempfile`, already a dev-dependency per the tabular work)

**Interfaces:**
- Produces: `PathSegment`, `NodePath`, `JsonKind`, `NodeContent`, `JsonNode`, `is_json_tree_extension`, and the core decode function `decode_node(bytes: &[u8], path: &[PathSegment]) -> Result<NodeContent, String>` that every later loader task calls.

- [ ] **Step 1: Add the dependency**

```bash
cargo add sonic-rs -p dozer-app
```

Run `cargo tree -p dozer-app -i sonic-rs` afterward to confirm it resolved a stable version, and note the resolved version number in the commit message for this task.

- [ ] **Step 2: Find how `tabular` is registered as a module and mirror it**

```bash
grep -rn "mod tabular" crates/dozer-app/src/
```

Add `mod json_tree;` in the same file, right next to the `mod tabular;` line.

- [ ] **Step 3: Write the failing test for `is_json_tree_extension`**

```rust
// crates/dozer-app/src/json_tree/mod.rs
use std::path::Path;

pub fn is_json_tree_extension(path: &Path) -> bool {
    false // placeholder to make the module compile before Step 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_json_tree_extension_covers_formats_case_insensitive() {
        for p in ["/tmp/a.json", "/tmp/a.jsonl", "/tmp/a.ndjson", "/tmp/A.JSON"] {
            assert!(is_json_tree_extension(Path::new(p)), "{p}");
        }
        for p in ["/tmp/a.md", "/tmp/a.csv", "/tmp/a.json5", "/tmp/a.jsonc"] {
            assert!(!is_json_tree_extension(Path::new(p)), "{p}");
        }
    }
}
```

- [ ] **Step 4: Run it, confirm it fails**

```bash
cargo test -p dozer-app json_tree::tests::is_json_tree_extension_covers_formats_case_insensitive
```
Expected: FAIL (the placeholder always returns `false`, so the positive cases fail).

- [ ] **Step 5: Implement it for real**

```rust
pub fn is_json_tree_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "json" | "jsonl" | "ndjson"
    )
}
```

- [ ] **Step 6: Run it, confirm it passes**

```bash
cargo test -p dozer-app json_tree::tests::is_json_tree_extension_covers_formats_case_insensitive
```
Expected: PASS.

- [ ] **Step 7: Add the core types and capping constants (no decode logic yet)**

```rust
pub const MAX_JSON_CHILDREN: usize = 10_000;
pub const MAX_LEAF_PREVIEW_CHARS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonKind {
    Object,
    Array,
    String,
    Number,
    Bool,
    Null,
}

#[derive(Debug, Clone)]
pub enum NodeContent {
    Object { entries: Vec<(String, JsonKind)>, truncated: bool },
    Array { items: Vec<JsonKind>, truncated: bool },
    Leaf { display: String, truncated: bool },
}

#[derive(Debug, Clone)]
pub struct JsonNode {
    pub kind: JsonKind,
    pub content: Option<NodeContent>,
}
```

- [ ] **Step 8: Write the failing test for decoding a root object's shape**

```rust
#[cfg(test)]
mod decode_tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "name": "dozer",
        "tags": ["a", "b", "c"],
        "meta": {"version": 1, "stable": true}
    }"#;

    #[test]
    fn decode_node_at_root_returns_shape_without_recursing() {
        let content = decode_node(FIXTURE.as_bytes(), &[]).unwrap();
        let NodeContent::Object { entries, truncated } = content else {
            panic!("root should decode as an object");
        };
        assert!(!truncated);
        assert_eq!(
            entries,
            vec![
                ("name".to_string(), JsonKind::String),
                ("tags".to_string(), JsonKind::Array),
                ("meta".to_string(), JsonKind::Object),
            ]
        );
    }
}
```

- [ ] **Step 9: Run it, confirm it fails to compile (no `decode_node` yet)**

```bash
cargo test -p dozer-app json_tree::decode_tests
```
Expected: compile error, `decode_node` not found.

- [ ] **Step 10: Implement `decode_node` using `sonic_rs::get`**

This plan verified the following against `docs.rs/sonic-rs` on 2026-09-19 (not guessed — check `cargo doc -p sonic-rs --open` yourself and confirm nothing changed in whatever version Step 1 actually resolved before trusting this verbatim):

- `sonic_rs::get<Input, Path>(json: Input, path: Path) -> Result<LazyValue>` where `Path::Item: Index` and `Index` is implemented for `&str` and `usize`.
- `LazyValue::get_type(&self) -> JsonType` for type checking.
- `LazyValue::as_raw_str(&self) -> &str` for a leaf's raw text.
- `LazyValue::into_object_iter(self) -> Option<ObjectJsonIter>` yielding `Result<(Cow<str>, LazyValue), Error>` per pair.
- `LazyValue::into_array_iter(self) -> Option<ArrayJsonIter>` — item type not directly confirmed in this plan's research; by symmetry with `ObjectJsonIter` it should be `Result<LazyValue, Error>` per element. **Confirm this against `cargo doc` in this step before relying on it** — if it differs, adjust the array branch below to match, keeping `decode_node`'s own signature unchanged (later tasks depend on it verbatim).

```rust
/// Decode exactly one node's direct-children shape at `path` (empty path
/// = the document root). Does not recurse into children's own children —
/// that only happens on their own later `decode_node` call, when the
/// user expands them (see json_tree module doc for why).
pub fn decode_node(bytes: &[u8], path: &[PathSegment]) -> Result<NodeContent, String> {
    let owned_keys: Vec<PathSegment> = path.to_vec();
    let index_path: Vec<Box<dyn sonic_rs::Index>> = owned_keys
        .into_iter()
        .map(|seg| -> Box<dyn sonic_rs::Index> {
            match seg {
                PathSegment::Key(k) => Box::new(k),
                PathSegment::Index(i) => Box::new(i),
            }
        })
        .collect();
    // NOTE: if `Box<dyn Index>` doesn't satisfy `Path::Item: Index` (dyn
    // trait objects aren't always usable where a concrete `Index` bound is
    // expected), fall back to two separate calls — `sonic_rs::get(bytes,
    // path_of_keys_as_&str)` when every segment is a `Key`, else walk one
    // segment at a time with repeated `.get(single_segment)` calls on the
    // returned `LazyValue` (its own `get<I: Index>` method, confirmed
    // above) — either way, keep this function's signature unchanged.
    let value = sonic_rs::get(bytes, index_path).map_err(|e| e.to_string())?;
    decode_lazy_value(value)
}

fn lazy_value_kind(value: &sonic_rs::LazyValue) -> JsonKind {
    use sonic_rs::JsonType;
    match value.get_type() {
        JsonType::Object => JsonKind::Object,
        JsonType::Array => JsonKind::Array,
        JsonType::String => JsonKind::String,
        JsonType::Number => JsonKind::Number,
        JsonType::Boolean => JsonKind::Bool,
        JsonType::Null => JsonKind::Null,
    }
    // NOTE: confirm `JsonType`'s exact variant names via `cargo doc` — the
    // names above are this plan's best inference from `get_type`'s
    // existence, not independently confirmed. Adjust the match arms to
    // whatever the real enum defines; the function's own signature
    // (`&LazyValue -> JsonKind`) stays as-is, Task 3 depends on it by name.
}

fn decode_lazy_value(value: sonic_rs::LazyValue) -> Result<NodeContent, String> {
    use sonic_rs::JsonType;
    match value.get_type() {
        JsonType::Object => {
            let mut entries = Vec::new();
            let mut truncated = false;
            if let Some(iter) = value.into_object_iter() {
                for pair in iter {
                    let (key, child) = pair.map_err(|e| e.to_string())?;
                    if entries.len() >= MAX_JSON_CHILDREN {
                        truncated = true;
                        break;
                    }
                    entries.push((key.into_owned(), lazy_value_kind(&child)));
                }
            }
            Ok(NodeContent::Object { entries, truncated })
        }
        JsonType::Array => {
            let mut items = Vec::new();
            let mut truncated = false;
            if let Some(iter) = value.into_array_iter() {
                for item in iter {
                    let child = item.map_err(|e| e.to_string())?;
                    if items.len() >= MAX_JSON_CHILDREN {
                        truncated = true;
                        break;
                    }
                    items.push(lazy_value_kind(&child));
                }
            }
            Ok(NodeContent::Array { items, truncated })
        }
        _ => {
            let raw = value.as_raw_str();
            let truncated = raw.chars().count() > MAX_LEAF_PREVIEW_CHARS;
            let display: String = if truncated {
                raw.chars().take(MAX_LEAF_PREVIEW_CHARS).collect()
            } else {
                raw.to_string()
            };
            Ok(NodeContent::Leaf { display, truncated })
        }
    }
}
```

Do not leave any `todo!()`/`unimplemented!()` in the committed code for this step — the two `NOTE:` comments above call out specific, bounded facts to re-verify against real `cargo doc` output (an `Index` trait-object detail and `JsonType`'s exact variant names), not open-ended design gaps; resolve both before moving on, adjusting only the specific lines called out.

- [ ] **Step 11: Run the Step 8 test, confirm it passes**

```bash
cargo test -p dozer-app json_tree::decode_tests::decode_node_at_root_returns_shape_without_recursing
```
Expected: PASS.

- [ ] **Step 12: Write and pass the out-of-order access test (this is the actual risk-resolution check)**

```rust
#[test]
fn decode_node_supports_out_of_order_path_access() {
    let tags_path = [PathSegment::Key("tags".to_string())];
    let meta_path = [PathSegment::Key("meta".to_string())];
    // Decode "meta" first, then "tags" — if sonic-rs's `get` were forward-only
    // and stateful, decoding meta first would make tags fail or return stale
    // data. Both must independently succeed regardless of order.
    let meta = decode_node(FIXTURE.as_bytes(), &meta_path).unwrap();
    let tags = decode_node(FIXTURE.as_bytes(), &tags_path).unwrap();
    assert!(matches!(meta, NodeContent::Object { .. }));
    assert!(matches!(tags, NodeContent::Array { .. }));
}
```

Run it. If this FAILS (not compile-fails — actually fails the assertion, or the second call errors), STOP and re-read the spec's fallback section (`docs/superpowers/specs/2026-09-19-json-preview-design.md`, "回退方案"). Do not proceed to Task 2 with a broken random-access assumption — escalate to a human decision instead of silently working around it, since it invalidates this plan's Task 3 (lazy-per-node expansion) design.

- [ ] **Step 13: Add the truncation-on-large-input test**

```rust
#[test]
fn decode_node_caps_object_children_at_max_json_children() {
    let mut obj = String::from("{");
    for i in 0..MAX_JSON_CHILDREN + 5 {
        if i > 0 { obj.push(','); }
        obj.push_str(&format!("\"k{i}\":{i}"));
    }
    obj.push('}');
    let content = decode_node(obj.as_bytes(), &[]).unwrap();
    let NodeContent::Object { entries, truncated } = content else {
        panic!("expected object");
    };
    assert_eq!(entries.len(), MAX_JSON_CHILDREN);
    assert!(truncated);
}
```

`decode_lazy_value` (Step 10) already gates the object/array branches at `MAX_JSON_CHILDREN` — this step is verification, not new implementation. Run, confirm PASS.

- [ ] **Step 14: Add the leaf-value-length truncation test**

```rust
#[test]
fn decode_node_truncates_long_leaf_strings() {
    let long = "x".repeat(MAX_LEAF_PREVIEW_CHARS + 50);
    let doc = format!("{{\"s\": \"{long}\"}}");
    let content = decode_node(doc.as_bytes(), &[PathSegment::Key("s".to_string())]).unwrap();
    let NodeContent::Leaf { display, truncated } = content else {
        panic!("expected leaf");
    };
    assert_eq!(display.chars().count(), MAX_LEAF_PREVIEW_CHARS);
    assert!(truncated);
}

#[test]
fn decode_node_does_not_mark_short_leaf_as_truncated() {
    let content = decode_node(br#"{"s": "short"}"#, &[PathSegment::Key("s".to_string())]).unwrap();
    let NodeContent::Leaf { display, truncated } = content else {
        panic!("expected leaf");
    };
    assert_eq!(display, "\"short\"", "as_raw_str includes the JSON quoting, this plan does not strip it — confirm this reads acceptably in the tree UI in Task 6, strip quotes there at render time if not, not here in the data layer");
    assert!(!truncated);
}
```

Run both, confirm PASS. Note the second test's assertion documents a real, deliberate open question flagged in-line rather than silently assumed: `as_raw_str()` returns the JSON-encoded text (quotes included for strings), which may or may not be what you want displayed in the tree UI verbatim. Resolve this in Task 6 (strip surrounding quotes for `JsonKind::String` leaves at render time if it reads better, leaving the data layer's `display` field as the faithful raw text) — do not silently change `decode_node`'s output to strip quotes here without updating this test.

- [ ] **Step 15: Record the spike finding as a doc comment**

Add this above `decode_node`:

```rust
/// Spike finding (2026-09-19, verified against a real fixture, not just
/// docs.rs summaries): `sonic_rs::get(bytes, path)` re-resolves `path`
/// from the start of `bytes` on every call and does not require any
/// prior forward traversal — arbitrary-order node expansion works with
/// no manual byte-offset bookkeeping. This simplified the design in
/// `docs/superpowers/specs/2026-09-19-json-preview-design.md`'s
/// `ExpandSource`/`byte_range` sketch away entirely; see that spec's
/// "关键技术决策" section for the original open question this answers.
/// Not yet measured: `get()`'s cost on a ~1GB file on aarch64 (sonic-rs's
/// own docs flag aarch64 as needing more optimization than x86_64) — flag
/// this as a follow-up manual benchmark once Task 10's end-to-end
/// smoke test has a realistic large fixture available, not blocking here.
```

- [ ] **Step 16: Full module test run + commit**

```bash
cargo test -p dozer-app json_tree::
cargo clippy -p dozer-app --all-targets 2>&1 | grep -A5 json_tree
cargo fmt -p dozer-app
git status --short   # confirm only your files changed; revert anything fmt touched that you didn't author
git add crates/dozer-app/Cargo.toml crates/dozer-app/src/json_tree/mod.rs <the file where you added `mod json_tree;`>
git commit -m "feat(json-tree): module skeleton + sonic-rs on-demand decode primitive"
```

---

### Task 2: `JsonTreeView` state machine (`apply`/`apply_node_loaded`) — pure, no IO

**Files:**
- Modify: `crates/dozer-app/src/json_tree/mod.rs`

**Interfaces:**
- Consumes: `PathSegment`, `NodePath`, `JsonKind`, `NodeContent`, `JsonNode`, `RootResult` (Task 1).
- Produces: `JsonTreeView`, `ViewMode`, `Action`, `NodeExpandRequest`, `JsonTreeView::apply`, `JsonTreeView::apply_node_loaded`, `JsonTreeView::new` — all consumed by Task 3 (loaders), Task 6 (canvas widget), and Task 8 (message wiring).

- [ ] **Step 1: Add the types**

```rust
pub type RootResult = Result<JsonNode, String>;

pub struct JsonTreeView {
    path: std::path::PathBuf,
    bytes: Vec<u8>,
    line_ranges: Option<Vec<std::ops::Range<usize>>>,
    pub roots: Vec<RootResult>,
    expanded: std::collections::HashSet<NodePath>,
    loading_nodes: std::collections::HashSet<NodePath>,
    pub view_mode: ViewMode,
    pub scroll_row: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Tree,
    RawText,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Scroll { dy: i32 },
    ToggleExpand(NodePath),
    ToggleViewMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeExpandRequest {
    pub path: NodePath,
    pub root_index: usize,
}

impl JsonTreeView {
    pub fn new(
        path: std::path::PathBuf,
        bytes: Vec<u8>,
        line_ranges: Option<Vec<std::ops::Range<usize>>>,
        roots: Vec<RootResult>,
    ) -> Self {
        Self {
            path,
            bytes,
            line_ranges,
            roots,
            expanded: std::collections::HashSet::new(),
            loading_nodes: std::collections::HashSet::new(),
            view_mode: ViewMode::Tree,
            scroll_row: 0,
        }
    }
}
```

This signature (`path, bytes, line_ranges, roots`, in that order) is final — no later task changes it again (compare the `decoded` field, which Step 8 below adds directly inside `impl JsonTreeView`'s `new()` body without changing the public parameter list, since it always starts empty).

- [ ] **Step 2: Write the failing test for `ToggleViewMode`**

```rust
#[cfg(test)]
mod apply_tests {
    use super::*;
    use std::rc::Rc;

    fn empty_view() -> JsonTreeView {
        JsonTreeView::new(
            std::path::PathBuf::from("/tmp/x.json"),
            b"{}".to_vec(),
            None,
            vec![Ok(JsonNode { kind: JsonKind::Object, content: None })],
        )
    }

    #[test]
    fn toggle_view_mode_flips_between_tree_and_raw_text() {
        let mut v = empty_view();
        assert_eq!(v.view_mode, ViewMode::Tree);
        assert_eq!(v.apply(Action::ToggleViewMode), None);
        assert_eq!(v.view_mode, ViewMode::RawText);
        assert_eq!(v.apply(Action::ToggleViewMode), None);
        assert_eq!(v.view_mode, ViewMode::Tree);
    }
}
```

- [ ] **Step 3: Run it, confirm it fails to compile (`apply` doesn't exist)**

- [ ] **Step 4: Implement `apply` (Scroll + ToggleViewMode only, ToggleExpand next step)**

```rust
impl JsonTreeView {
    /// Pure state transition, no IO. `Scroll` only floors at 0 — the
    /// upper bound depends on how many rows are currently visible, which
    /// changes as the user expands/collapses nodes; that clamp lives in
    /// the canvas widget's `draw()` (Task 6), computed fresh each frame
    /// from the same flatten function the widget uses to render. This is
    /// deliberately different from the tabular viewer's original (buggy)
    /// two-place clamp that a 2026-09 code review had to fix — one source
    /// of truth this time.
    pub fn apply(&mut self, action: Action) -> Option<NodeExpandRequest> {
        match action {
            Action::Scroll { dy } => {
                self.scroll_row = self.scroll_row.saturating_add_signed(dy as isize);
                None
            }
            Action::ToggleViewMode => {
                self.view_mode = match self.view_mode {
                    ViewMode::Tree => ViewMode::RawText,
                    ViewMode::RawText => ViewMode::Tree,
                };
                None
            }
            Action::ToggleExpand(path) => self.toggle_expand(path),
        }
    }

    fn toggle_expand(&mut self, path: NodePath) -> Option<NodeExpandRequest> {
        todo!("Step 6")
    }
}
```

- [ ] **Step 5: Run Step 2's test, confirm PASS**

- [ ] **Step 6: Write the failing tests for `ToggleExpand`**

```rust
#[test]
fn toggle_expand_on_new_path_expands_and_requests_load_if_undecoded() {
    let mut v = empty_view();
    let path: NodePath = Rc::from(vec![PathSegment::Key("a".to_string())]);
    let req = v.apply(Action::ToggleExpand(path.clone()));
    assert_eq!(req, Some(NodeExpandRequest { path: path.clone(), root_index: 0 }));
}

#[test]
fn toggle_expand_twice_collapses_without_requesting_load_again() {
    let mut v = empty_view();
    let path: NodePath = Rc::from(vec![PathSegment::Key("a".to_string())]);
    v.apply(Action::ToggleExpand(path.clone())); // expand: requests load
    let second = v.apply(Action::ToggleExpand(path.clone())); // collapse
    assert_eq!(second, None, "collapsing never triggers a load");
}

#[test]
fn reexpanding_still_loading_node_does_not_requeue_load() {
    let mut v = empty_view();
    let path: NodePath = Rc::from(vec![PathSegment::Key("a".to_string())]);
    assert!(v.apply(Action::ToggleExpand(path.clone())).is_some(), "first expand requests a load");
    v.apply(Action::ToggleExpand(path.clone())); // collapse
    let req = v.apply(Action::ToggleExpand(path.clone())); // expand again, load still in flight
    assert_eq!(req, None, "a load for this path is already in flight, must not spawn a second one");
}

#[test]
fn expanding_already_decoded_node_does_not_request_load() {
    let mut v = empty_view();
    let path: NodePath = Rc::from(vec![PathSegment::Key("a".to_string())]);
    v.apply(Action::ToggleExpand(path.clone()));
    v.apply_node_loaded(path.clone(), Ok(NodeContent::Leaf { display: "1".into(), truncated: false }), 0);
    v.apply(Action::ToggleExpand(path.clone())); // collapse
    let req = v.apply(Action::ToggleExpand(path.clone())); // expand again, already decoded
    assert_eq!(req, None, "content is cached, no need to re-decode");
}
```

- [ ] **Step 7: Run them, confirm they fail (compile error: `toggle_expand`'s `todo!()`, `apply_node_loaded` missing)**

- [ ] **Step 8: Implement `toggle_expand` and `apply_node_loaded`**

Note the cache for decoded non-root nodes needs somewhere to live — add a `decoded: std::collections::HashMap<NodePath, NodeContent>` field to `JsonTreeView` (root nodes' own content lives in `roots[i]`'s `JsonNode.content`, but nested nodes below the root don't have a `JsonNode` slot of their own until decoded, per the "Reference" section's note — this map is that slot).

```rust
// add to JsonTreeView struct:
    decoded: std::collections::HashMap<NodePath, NodeContent>,
// add to JsonTreeView::new:
            decoded: std::collections::HashMap::new(),

impl JsonTreeView {
    fn toggle_expand(&mut self, path: NodePath) -> Option<NodeExpandRequest> {
        if self.expanded.remove(&path) {
            return None; // was expanded, now collapsed — never triggers a load
        }
        self.expanded.insert(path.clone());
        let already_decoded = if path.is_empty() {
            self.roots.first().is_some_and(|r| matches!(r, Ok(n) if n.content.is_some()))
        } else {
            self.decoded.contains_key(&path)
        };
        if already_decoded || !self.loading_nodes.insert(path.clone()) {
            return None;
        }
        Some(NodeExpandRequest { path, root_index: 0 }) // root_index fixed up properly in Task 4/5
    }

    /// Background decode completed; `root_index` identifies which root
    /// (always 0 for a single `.json` file, the line number for jsonl).
    pub fn apply_node_loaded(&mut self, path: NodePath, result: Result<NodeContent, String>, root_index: usize) {
        self.loading_nodes.remove(&path);
        match result {
            Ok(content) => {
                if path.is_empty() {
                    if let Some(Ok(root)) = self.roots.get_mut(root_index) {
                        root.content = Some(content);
                    }
                } else {
                    self.decoded.insert(path, content);
                }
            }
            Err(err) => tracing::warn!(?path, %err, "JSON 节点后台解码失败"),
        }
    }
}
```

- [ ] **Step 9: Run all Step 6 tests, confirm PASS**

- [ ] **Step 10: Full module test run, fmt, commit**

```bash
cargo test -p dozer-app json_tree::
cargo fmt -p dozer-app
git status --short
git add crates/dozer-app/src/json_tree/mod.rs
git commit -m "feat(json-tree): JsonTreeView state machine (apply/apply_node_loaded)"
```

---

### Task 3: `.json` single-file loader

**Files:**
- Modify: `crates/dozer-app/src/json_tree/mod.rs`

**Interfaces:**
- Consumes: `decode_node` (Task 1), `JsonTreeView::new` (Task 2).
- Produces: `load_json(path: &Path) -> Result<JsonTreeView, String>`, consumed by Task 5's `load()` dispatcher.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod load_json_tests {
    use super::*;

    #[test]
    fn load_json_decodes_root_shape_eagerly() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sample.json");
        std::fs::write(&p, r#"{"a": 1, "b": [1,2,3]}"#).unwrap();
        let view = load_json(&p).unwrap();
        assert_eq!(view.roots.len(), 1);
        let root = view.roots[0].as_ref().unwrap();
        assert_eq!(root.kind, JsonKind::Object);
        let NodeContent::Object { entries, .. } = root.content.as_ref().unwrap() else {
            panic!("root content should be decoded already");
        };
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn load_json_rejects_missing_file() {
        assert!(load_json(std::path::Path::new("/tmp/does_not_exist_12345.json")).is_err());
    }
}
```

- [ ] **Step 2: Run, confirm fails (compile error, `load_json` missing)**

- [ ] **Step 3: Implement**

```rust
pub fn load_json(path: &std::path::Path) -> Result<JsonTreeView, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let root_kind = root_kind_of(&bytes)?;
    let content = decode_node(&bytes, &[])?;
    let root = JsonNode { kind: root_kind, content: Some(content) };
    Ok(JsonTreeView::new(path.to_path_buf(), bytes, None, vec![Ok(root)]))
}

/// Cheap top-level type check without decoding children — used so
/// `JsonNode.kind` is accurate even before `decode_node` returns (kept
/// as a separate tiny call so Task 1's `decode_node` doesn't need to also
/// report its own subject's kind, only its children's).
fn root_kind_of(bytes: &[u8]) -> Result<JsonKind, String> {
    // Implement using whichever sonic-rs top-level type-check you found
    // in Task 1 Step 10 (e.g. a `sonic_rs::get(bytes, [])` returning a
    // `LazyValue`, then checking its type the same way `decode_lazy_value`
    // does for children). Do not duplicate decode logic — factor the
    // "LazyValue -> JsonKind" mapping out of Task 1's `decode_lazy_value`
    // into a small `fn lazy_value_kind(&LazyValue) -> JsonKind` both call.
}
```

- [ ] **Step 4: Run tests, confirm PASS**

- [ ] **Step 5: Refactor `decode_lazy_value` (Task 1) to share `lazy_value_kind` with this step's `root_kind_of`, per the comment above — re-run all `json_tree::` tests to confirm nothing broke**

- [ ] **Step 6: fmt, commit**

```bash
cargo test -p dozer-app json_tree::
cargo fmt -p dozer-app
git add crates/dozer-app/src/json_tree/mod.rs
git commit -m "feat(json-tree): load_json single-file loader"
```

---

### Task 4: `.jsonl`/`.ndjson` loader

**Files:**
- Modify: `crates/dozer-app/src/json_tree/mod.rs`

**Interfaces:**
- Consumes: `decode_node`, `lazy_value_kind`/`root_kind_of` pattern (Task 3), `JsonTreeView::new` (Task 2).
- Produces: `load_jsonl(path: &Path) -> Result<JsonTreeView, String>`, `MAX_JSON_LINES`, consumed by Task 5.

- [ ] **Step 1: Add the line-count cap constant**

```rust
// crates/dozer-app/src/json_tree/mod.rs, next to MAX_JSON_CHILDREN/MAX_LEAF_PREVIEW_CHARS
pub const MAX_JSON_LINES: usize = 100_000;
```

- [ ] **Step 2: Write the failing tests**

```rust
#[cfg(test)]
mod load_jsonl_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn load_jsonl_makes_one_root_per_line() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sample.jsonl");
        std::fs::write(&p, "{\"a\":1}\n[1,2,3]\n\"just a string\"\n").unwrap();
        let view = load_jsonl(&p).unwrap();
        assert_eq!(view.roots.len(), 3);
        assert_eq!(view.roots[0].as_ref().unwrap().kind, JsonKind::Object);
        assert_eq!(view.roots[1].as_ref().unwrap().kind, JsonKind::Array);
        assert_eq!(view.roots[2].as_ref().unwrap().kind, JsonKind::String);
    }

    #[test]
    fn load_jsonl_one_bad_line_does_not_break_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sample.jsonl");
        std::fs::write(&p, "{\"a\":1}\nnot json\n{\"c\":3}\n").unwrap();
        let view = load_jsonl(&p).unwrap();
        assert_eq!(view.roots.len(), 3);
        assert!(view.roots[0].is_ok());
        assert!(view.roots[1].is_err());
        assert!(view.roots[2].is_ok());
    }

    #[test]
    fn load_jsonl_stops_reading_at_the_line_cap() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        for i in 0..MAX_JSON_LINES + 5 {
            writeln!(f, "{i}").unwrap();
        }
        let view = load_jsonl(&p).unwrap();
        assert_eq!(view.roots.len(), MAX_JSON_LINES, "must stop at the cap, not read the whole file");
    }
}
```

- [ ] **Step 3: Run, confirm fails**

- [ ] **Step 4: Implement**

```rust
pub fn load_jsonl(path: &std::path::Path) -> Result<JsonTreeView, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut line_ranges = Vec::new();
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            if start < i || i > start {
                line_ranges.push(start..i);
            }
            start = i + 1;
            if line_ranges.len() >= MAX_JSON_LINES {
                break;
            }
        }
    }
    if start < bytes.len() && line_ranges.len() < MAX_JSON_LINES {
        line_ranges.push(start..bytes.len());
    }
    let roots: Vec<RootResult> = line_ranges
        .iter()
        .map(|range| {
            let line = &bytes[range.clone()];
            let kind = root_kind_of(line)?;
            let content = decode_node(line, &[])?;
            Ok(JsonNode { kind, content: Some(content) })
        })
        .collect();
    Ok(JsonTreeView::new(path.to_path_buf(), bytes, Some(line_ranges), roots))
}
```

Note: empty lines (blank lines in the file) will fail to parse as JSON and land as `Err` roots — that's correct behavior per the "one bad line doesn't break others" test, no special-casing needed.

- [ ] **Step 5: Run tests, confirm PASS. `MAX_JSON_LINES + 5` in the cap test writes ~700KB — fine for a unit test, but if it's slow in CI, that's a signal to revisit, not silently reduce the constant.**

- [ ] **Step 6: fmt, commit**

```bash
cargo test -p dozer-app json_tree::
cargo fmt -p dozer-app
git add crates/dozer-app/src/json_tree/mod.rs
git commit -m "feat(json-tree): load_jsonl line-capped loader"
```

---

### Task 5: Unified `load()` dispatcher + `expand()` background-decode entrypoint

**Files:**
- Modify: `crates/dozer-app/src/json_tree/mod.rs`

**Interfaces:**
- Consumes: `load_json`, `load_jsonl` (Tasks 3-4), `decode_node` (Task 1).
- Produces: `load(path: &Path) -> Result<JsonTreeView, String>` and `expand(bytes_source: ExpandBytesSource, path: &[PathSegment]) -> Result<NodeContent, String>` — both consumed by Task 8's message-wiring layer (the background-thread closures call exactly these two functions, nothing else).

- [ ] **Step 1: Write the failing test for dispatch**

```rust
#[cfg(test)]
mod load_dispatch_tests {
    use super::*;

    #[test]
    fn load_dispatches_by_extension() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("a.json");
        std::fs::write(&json_path, "{}").unwrap();
        let view = load(&json_path).unwrap();
        assert_eq!(view.roots.len(), 1);

        let jsonl_path = dir.path().join("a.jsonl");
        std::fs::write(&jsonl_path, "{}\n{}\n").unwrap();
        let view = load(&jsonl_path).unwrap();
        assert_eq!(view.roots.len(), 2);
    }

    #[test]
    fn load_rejects_non_json_extension() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        std::fs::write(&p, "# hi").unwrap();
        assert!(load(&p).is_err());
    }
}
```

- [ ] **Step 2: Run, confirm fails**

- [ ] **Step 3: Implement `load`**

```rust
pub fn load(path: &std::path::Path) -> Result<JsonTreeView, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "json" => load_json(path),
        "jsonl" | "ndjson" => load_jsonl(path),
        _ => Err(format!("非 JSON 文件: {ext}")),
    }
}
```

- [ ] **Step 4: Run, confirm PASS**

- [ ] **Step 5: Add `expand()`, the entrypoint the background thread calls for on-demand node decode**

This needs to know which byte slice to decode against (the whole file for `.json`, or one line's slice for `.jsonl`) — `JsonTreeView` already has `bytes`/`line_ranges` but those are private fields, and `expand()` runs on a background thread that only has a `NodeExpandRequest`, not a live `&JsonTreeView` (the view lives on the UI-thread-owned `PreviewTab`, can't be borrowed across the thread boundary while a load is in flight — same reason tabular's `load_sheet` takes a plain `path: &Path`, not a `&TabularView`). So `expand()` needs to be handed a byte source directly, not the view:

```rust
/// What `expand()` decodes against — built by the caller (Task 8's
/// message handler) from data already on hand before spawning the
/// background thread (never from a live `&JsonTreeView` — that would
/// require borrowing across the async boundary).
pub enum ExpandBytesSource {
    WholeFile(std::path::PathBuf),
    JsonLine { path: std::path::PathBuf, byte_range: std::ops::Range<usize> },
}

pub fn expand(source: &ExpandBytesSource, path: &[PathSegment]) -> Result<NodeContent, String> {
    match source {
        ExpandBytesSource::WholeFile(p) => {
            let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
            decode_node(&bytes, path)
        }
        ExpandBytesSource::JsonLine { path: p, byte_range } => {
            let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
            let line = bytes.get(byte_range.clone()).ok_or("行范围越界(文件在打开后被改动?)")?;
            decode_node(line, path)
        }
    }
}
```

Note this re-reads the file from disk on every expand rather than reusing the resident `bytes` the initial load already has in memory — this is the same accepted tradeoff the tabular spec documented for `load_sheet` re-opening the workbook on every lazy sheet switch (see that spec's `load_sheet` doc comment): expansion is an infrequent, explicit, already-spinner-covered user action, not a hot path, and keeping a live borrow of the view's `bytes` alive across the thread boundary is the kind of complexity that tradeoff was written to avoid. Add a doc comment on `expand()` saying exactly this, referencing the tabular precedent, so a future reviewer doesn't "fix" it into new complexity without re-reading why.

Add the accompanying test:

```rust
#[cfg(test)]
mod expand_tests {
    use super::*;

    #[test]
    fn expand_whole_file_decodes_the_requested_path() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.json");
        std::fs::write(&p, r#"{"a": {"b": 1}}"#).unwrap();
        let content = expand(&ExpandBytesSource::WholeFile(p), &[PathSegment::Key("a".into())]).unwrap();
        assert!(matches!(content, NodeContent::Object { .. }));
    }

    #[test]
    fn expand_json_line_decodes_within_that_lines_byte_range() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.jsonl");
        std::fs::write(&p, "{\"x\":1}\n{\"y\":[1,2]}\n").unwrap();
        // second line's byte range: after the first line + its newline
        let first_len = "{\"x\":1}\n".len();
        let second_len = "{\"y\":[1,2]}".len();
        let range = first_len..(first_len + second_len);
        let content = expand(
            &ExpandBytesSource::JsonLine { path: p, byte_range: range },
            &[PathSegment::Key("y".into())],
        ).unwrap();
        assert!(matches!(content, NodeContent::Array { .. }));
    }
}
```

- [ ] **Step 6: Run all `expand_tests`, confirm PASS**

- [ ] **Step 7: Full module suite, fmt, commit**

```bash
cargo test -p dozer-app json_tree::
cargo fmt -p dozer-app
git add crates/dozer-app/src/json_tree/mod.rs
git commit -m "feat(json-tree): load() dispatcher + expand() background-decode entrypoint"
```

---

### Task 6: Flatten-visible-rows (pure) + canvas virtualized tree widget

**Files:**
- Create: `crates/dozer-app/src/json_tree/tree.rs`
- Modify: `crates/dozer-app/src/json_tree/mod.rs` (add `pub mod tree;`)

**Interfaces:**
- Consumes: `JsonTreeView` (private fields `expanded`/`decoded`/`roots` need `pub(crate)` visibility or accessor methods added in this task — add read-only accessors rather than making fields `pub`, matching how Task 2 kept `bytes`/`line_ranges` private), `PathSegment`, `NodePath`, `JsonKind`, `NodeContent`, `Action`.
- Produces: `flatten_visible_rows(view: &JsonTreeView) -> Vec<VisibleRow>` (pure, tested), `tree::view(view: &JsonTreeView) -> Element<Action>` (canvas widget, consumed by Task 7).

- [ ] **Step 1: Add read-only accessors `JsonTreeView` needs for row flattening**

```rust
// crates/dozer-app/src/json_tree/mod.rs, in impl JsonTreeView
    pub fn is_expanded(&self, path: &NodePath) -> bool {
        self.expanded.contains(path)
    }

    /// The decoded content at `path`, if any — root paths (`path.is_empty()`
    /// checked by caller against the right root) come from `roots`, deeper
    /// paths from the `decoded` cache. Returns `None` if not decoded yet
    /// (still loading or not requested).
    pub fn content_at(&self, root_index: usize, path: &NodePath) -> Option<&NodeContent> {
        if path.is_empty() {
            self.roots.get(root_index)?.as_ref().ok()?.content.as_ref()
        } else {
            self.decoded.get(path)
        }
    }
```

- [ ] **Step 2: Write the failing test for flattening**

```rust
// crates/dozer-app/src/json_tree/tree.rs
use super::{Action, JsonKind, JsonTreeView, NodeContent, NodePath, PathSegment};

#[derive(Debug, Clone, PartialEq)]
pub struct VisibleRow {
    pub root_index: usize,
    pub path: NodePath,
    pub depth: usize,
    pub key_label: String, // "" for the root, "key" or "[i]" for children
    pub kind: JsonKind,
    pub expandable: bool,
    pub expanded: bool,
}

/// Flattens only what's currently expanded — cost is proportional to
/// "how much the user has opened", never to file size (see module doc
/// on why this differs from tabular's fixed-`total_rows` model).
pub fn flatten_visible_rows(view: &JsonTreeView) -> Vec<VisibleRow> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_tree::{JsonNode, JsonTreeView};
    use std::rc::Rc;

    fn view_with_root(kind: JsonKind, content: Option<NodeContent>) -> JsonTreeView {
        JsonTreeView::new(
            std::path::PathBuf::from("/tmp/x.json"),
            b"{}".to_vec(),
            None,
            vec![Ok(JsonNode { kind, content })],
        )
    }

    #[test]
    fn collapsed_root_produces_exactly_one_row() {
        let content = NodeContent::Object {
            entries: vec![("a".into(), JsonKind::Number)],
            truncated: false,
        };
        let view = view_with_root(JsonKind::Object, Some(content));
        let rows = flatten_visible_rows(&view);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].depth, 0);
        assert!(rows[0].expandable);
        assert!(!rows[0].expanded);
    }

    #[test]
    fn expanded_root_shows_its_direct_children_too() {
        let content = NodeContent::Object {
            entries: vec![("a".into(), JsonKind::Number), ("b".into(), JsonKind::String)],
            truncated: false,
        };
        let mut view = view_with_root(JsonKind::Object, Some(content));
        let root_path: NodePath = Rc::from(Vec::<PathSegment>::new());
        view.apply(Action::ToggleExpand(root_path));
        let rows = flatten_visible_rows(&view);
        assert_eq!(rows.len(), 3, "root + 2 children");
        assert_eq!(rows[1].key_label, "a");
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[2].key_label, "b");
    }

    #[test]
    fn expanded_child_with_no_decoded_content_yet_is_a_leaf_row_marked_loading() {
        let content = NodeContent::Object {
            entries: vec![("child".into(), JsonKind::Object)],
            truncated: false,
        };
        let mut view = view_with_root(JsonKind::Object, Some(content));
        view.apply(Action::ToggleExpand(Rc::from(Vec::<PathSegment>::new())));
        let child_path: NodePath = Rc::from(vec![PathSegment::Key("child".to_string())]);
        view.apply(Action::ToggleExpand(child_path)); // expand child; not decoded yet
        let rows = flatten_visible_rows(&view);
        assert_eq!(rows.len(), 2, "root + child row; child's own children not shown until decoded");
        assert!(rows[1].expandable, "still shown as an openable object even though not decoded yet");
    }
}
```

- [ ] **Step 3: Run, confirm fails (`unimplemented!()` panics on any call)**

- [ ] **Step 4: Implement `flatten_visible_rows`**

```rust
pub fn flatten_visible_rows(view: &JsonTreeView) -> Vec<VisibleRow> {
    let mut rows = Vec::new();
    for (root_index, root) in view.roots.iter().enumerate() {
        let Ok(node) = root else {
            rows.push(VisibleRow {
                root_index,
                path: std::rc::Rc::from(Vec::<PathSegment>::new()),
                depth: 0,
                key_label: format!("(第 {root_index} 行解析失败)"),
                kind: JsonKind::Null,
                expandable: false,
                expanded: false,
            });
            continue;
        };
        let root_path: NodePath = std::rc::Rc::from(Vec::<PathSegment>::new());
        push_row_and_children(view, &mut rows, root_index, root_path, 0, String::new(), node.kind);
    }
    rows
}

fn push_row_and_children(
    view: &JsonTreeView,
    rows: &mut Vec<VisibleRow>,
    root_index: usize,
    path: NodePath,
    depth: usize,
    key_label: String,
    kind: JsonKind,
) {
    let expandable = matches!(kind, JsonKind::Object | JsonKind::Array);
    let expanded = view.is_expanded(&path);
    rows.push(VisibleRow { root_index, path: path.clone(), depth, key_label, kind, expandable, expanded });
    if !expanded {
        return;
    }
    let Some(content) = view.content_at(root_index, &path) else {
        return; // expanded but not decoded yet — just the one row, no children to show
    };
    match content {
        NodeContent::Object { entries, .. } => {
            for (key, child_kind) in entries {
                let mut segs: Vec<PathSegment> = path.iter().cloned().collect();
                segs.push(PathSegment::Key(key.clone()));
                push_row_and_children(view, rows, root_index, std::rc::Rc::from(segs), depth + 1, key.clone(), *child_kind);
            }
        }
        NodeContent::Array { items, .. } => {
            for (i, child_kind) in items.iter().enumerate() {
                let mut segs: Vec<PathSegment> = path.iter().cloned().collect();
                segs.push(PathSegment::Index(i));
                push_row_and_children(view, rows, root_index, std::rc::Rc::from(segs), depth + 1, format!("[{i}]"), *child_kind);
            }
        }
        NodeContent::Leaf { .. } => {} // leaves have no children to recurse into
    }
}
```

- [ ] **Step 5: Run all `tree::tests`, confirm PASS**

- [ ] **Step 6: Add the canvas `Program` (manual verification only past this point — mirrors `tabular/grid.rs`'s precedent of not unit-testing `draw()`/`update()` themselves, only their pure helpers, which `flatten_visible_rows` already covers)**

```rust
use iced_widget::canvas::{self, Frame};
use iced_widget::core::event::Event;
use iced_widget::core::text::Shaping;
use iced_widget::core::{mouse, Element, Font, Length, Point, Rectangle, Size};

struct TreeCanvas<'a> {
    view: &'a JsonTreeView,
    rows: Vec<VisibleRow>,
}

impl<'a> canvas::Program<Action> for TreeCanvas<'a> {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Action>> {
        // Wheel scroll -> Action::Scroll{dy}, same delta handling as
        // tabular/grid.rs's Program::update (copy that match arm's
        // structure, dy only — this widget doesn't scroll horizontally).
        // Left click within a row's chevron hit box (row_h tall, chevron
        // width + indent*depth wide, computed the same way as row
        // rendering below) -> Action::ToggleExpand(row.path.clone()) for
        // whichever row the click's y falls into (only if row.expandable).
        todo!("copy tabular/grid.rs's WheelScrolled handling for dy; add mouse::Event::ButtonPressed hit-testing against self.rows using the same row_h metric draw() uses below")
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_renderer::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let colors = byteui::theme::color::current();
        let font_size = crate::theme::terminal_font::size() * byteui::theme::icon_size::scale();
        let row_h = font_size * 1.5; // same ROW_HEIGHT_FACTOR as tabular/grid.rs
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), colors.panel);

        let rows_visible = ((bounds.height / row_h).ceil() as usize).max(1);
        let max_scroll = self.rows.len().saturating_sub(rows_visible);
        let scroll = self.view.scroll_row.min(max_scroll);
        let end = (scroll + rows_visible).min(self.rows.len());

        for (i, row) in self.rows[scroll..end].iter().enumerate() {
            let y = i as f32 * row_h;
            let indent = row.depth as f32 * (byteui::theme::icon_size::chevron() + byteui::theme::icon_size::tree_row_gap());
            if row.expandable {
                let chevron = if row.expanded { "▾" } else { "▸" }; // replace with the real ChevronDown/ChevronRight icon glyph draw, matching Files tree's icons::view() usage — plain glyph here only as a placeholder for hit-box math, must be swapped for the real icon before this task is done
                frame.fill_text(canvas::Text {
                    content: chevron.to_string(),
                    position: Point::new(indent, y),
                    size: iced_widget::core::Pixels(font_size),
                    color: colors.dim,
                    font: Font::default(),
                    shaping: Shaping::Advanced,
                    ..Default::default()
                });
            }
            let label = if row.key_label.is_empty() {
                format!("{:?}", row.kind)
            } else {
                format!("{}: {:?}", row.key_label, row.kind)
            };
            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(indent + byteui::theme::icon_size::chevron() + byteui::theme::icon_size::tree_row_gap(), y),
                size: iced_widget::core::Pixels(font_size),
                color: colors.body,
                font: Font::default(),
                shaping: Shaping::Advanced,
                ..Default::default()
            });
        }
        vec![frame.into_geometry()]
    }
}

pub fn view(json_view: &JsonTreeView) -> Element<'_, Action, iced_widget::Theme, iced_renderer::Renderer> {
    let rows = flatten_visible_rows(json_view);
    canvas::Canvas::new(TreeCanvas { view: json_view, rows })
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
```

Before committing: replace the `todo!()` in `update()` with real hit-testing code (copy `tabular/grid.rs`'s `Program::update`'s `WheelScrolled` arm verbatim for the scroll half; write the chevron click hit-test using the same `row_h`/`indent` math `draw()` uses, so the two stay in sync — extract both into a shared `struct Metrics` like `tabular/grid.rs` does, rather than duplicating the row-height formula in two places). Replace the `▾`/`▸` placeholder glyphs with a real `byteui::interaction::icons::view(IconKind::ChevronDown/ChevronRight, ...)` draw call — check how `tabular/grid.rs` or the Files tree draws icons inside a canvas frame (icons drawn via `iced_widget::text` glyphs vs an actual icon font/image) and match that mechanism, don't invent a new one.

- [ ] **Step 7: Build (this task has no automated test for `draw`/`update` themselves, per the tabular precedent — verify by building, then Task 10's manual smoke test is where this actually gets exercised)**

```bash
cargo build -p dozer-app
cargo test -p dozer-app json_tree::
```

- [ ] **Step 8: fmt, commit**

```bash
cargo fmt -p dozer-app
git add crates/dozer-app/src/json_tree/mod.rs crates/dozer-app/src/json_tree/tree.rs
git commit -m "feat(json-tree): virtualized canvas tree widget"
```

---

### Task 7: `json_tree::view.rs` — view assembly, Tree/RawText toggle, loading/truncation UI

**Files:**
- Create: `crates/dozer-app/src/json_tree/view.rs`
- Modify: `crates/dozer-app/src/json_tree/mod.rs` (add `pub mod view;`)

**Interfaces:**
- Consumes: `tree::view` (Task 6), `Action`, `ViewMode`, `JsonTreeView` (Task 2).
- Produces: `JsonTreeView::view(&self) -> Element<Action>`, consumed by Task 9's `workspace/view.rs` integration.

- [ ] **Step 1: Implement (no new pure logic to TDD here — this is layout assembly, verified by build + Task 10's manual pass, same as `tabular/view.rs`)**

```rust
use iced_widget::core::{Element, Length};
use iced_widget::{column, container, text};

use super::{Action, JsonTreeView, ViewMode};

impl JsonTreeView {
    pub fn view(&self) -> Element<'_, Action, iced_widget::Theme, iced_renderer::Renderer> {
        let colors = byteui::theme::color::current();
        let mut col: iced_widget::Column<'_, Action, iced_widget::Theme, iced_renderer::Renderer> =
            column![].width(Length::Fill).height(Length::Fill);

        // Toggle button: reuse byteui::interaction::icons::icon_button_entry
        // per CLAUDE.md's icon-button reuse rule — check that helper's exact
        // signature (crates/byteui/src/interaction/icons.rs) and wire an
        // icon representing "raw text" (e.g. a document/code glyph) with
        // `.on_press(Action::ToggleViewMode)`. Do not hand-roll a
        // MouseArea+on_enter/on_exit button here.
        let toggle_label = match self.view_mode {
            ViewMode::Tree => "查看原始文本",
            ViewMode::RawText => "查看 Tree",
        };
        col = col.push(
            container(text(toggle_label).size(byteui::theme::font::label()).color(colors.dim))
                .width(Length::Fill)
                .padding([4, 8]),
        );

        if self.view_mode == ViewMode::RawText {
            // The raw text itself is NOT rendered here — the outer caller
            // (Task 9's workspace/view.rs) renders the tab's existing
            // `editor` (CodeView) when `view_mode == RawText`, this method
            // only ever renders the Tree half or the toggle control. This
            // mirrors the spec's "dual population" design: json_tree::view
            // owns the toggle button; the outer match arm owns which body
            // (tree canvas vs CodeView) sits below it.
            return col.into();
        }

        col = col.push(
            container(super::tree::view(self))
                .width(Length::Fill)
                .height(Length::Fill),
        );
        col.into()
    }
}
```

- [ ] **Step 2: Build**

```bash
cargo build -p dozer-app
```

- [ ] **Step 3: fmt, commit**

```bash
cargo fmt -p dozer-app
git add crates/dozer-app/src/json_tree/mod.rs crates/dozer-app/src/json_tree/view.rs
git commit -m "feat(json-tree): view assembly with Tree/RawText toggle"
```

---

### Task 8: Message plumbing + async spawn wiring

Mirrors the tabular viewer's `Message::TabularLoaded`/`TabularSheetLoaded` + `spawn_pending_tabular_loads`/`preview_pane_tabular_action` pattern exactly. If anything here seems ambiguous, go read the actual committed code for those (in `crates/dozer-app/src/app/message.rs`, `crates/dozer-app/src/app/update.rs`, `crates/dozer-app/src/workspace/state.rs`) rather than guessing — this task is a structural copy, not a new design.

**Files:**
- Modify: `crates/dozer-app/src/app/message.rs`
- Modify: `crates/dozer-app/src/app/update.rs`
- Modify: `crates/dozer-app/src/workspace/state.rs`

**Interfaces:**
- Consumes: `json_tree::{load, expand, ExpandBytesSource, Action, NodeExpandRequest, JsonTreeView}` (Tasks 1-6), `PanelKind`, `ProjectId`, `ShellIo` (existing).
- Produces: `Message::JsonTreeAction`, `Message::JsonTreeLoaded`, `Message::JsonNodeLoaded`, `Workspace::preview_pane_json_tree_action`, `Workspace::spawn_pending_json_tree_loads` — consumed by Task 9.

- [ ] **Step 1: Add the Message variants**

```rust
// crates/dozer-app/src/app/message.rs, next to TabularLoaded/TabularSheetLoaded
JsonTreeAction(PanelKind, usize, crate::json_tree::Action),
JsonTreeLoaded(ProjectId, PanelKind, usize, Result<crate::json_tree::JsonTreeView, String>),
JsonNodeLoaded(
    ProjectId,
    PanelKind,
    usize,
    crate::json_tree::NodePath,
    usize, // root_index
    Result<crate::json_tree::NodeContent, String>,
),
```

`JsonTreeView` needs `#[derive(Debug, Clone)]` for the same reason `TabularView` does (the enclosing `Message` enum derives `Clone`/`Debug`) — add those derives to `JsonTreeView`, `JsonNode`, `NodeContent`, `PathSegment` now if not already present from earlier tasks (check; `PathSegment` already has them from Task 1).

- [ ] **Step 2: Build, fix any missing-derive compile errors**

```bash
cargo build -p dozer-app
```

- [ ] **Step 3: Add the `update.rs` match arms**

```rust
// next to the existing Message::TabularAction/TabularLoaded/TabularSheetLoaded arms
Message::JsonTreeAction(kind, tab_id, action) => {
    self.with_focused_project(move |ws, io| {
        ws.preview_pane_json_tree_action(kind, tab_id, action, io);
    });
}
Message::JsonTreeLoaded(project_id, kind, tab_id, result) => {
    self.with_project(project_id, move |ws, _io| {
        let pane = if kind == PanelKind::Project { &mut ws.project_preview } else { &mut ws.preview };
        let Ok(view) = result else {
            tracing::warn!("JSON 首次加载失败,tab 停留在 Loading");
            return;
        };
        if let Some(slot) = pane.json_tree_state_mut(tab_id) {
            *slot = crate::preview::JsonTreeState::Ready(view);
        }
    });
}
Message::JsonNodeLoaded(project_id, kind, tab_id, path, root_index, result) => {
    self.with_project(project_id, move |ws, _io| {
        let pane = if kind == PanelKind::Project { &mut ws.project_preview } else { &mut ws.preview };
        if let Some(crate::preview::JsonTreeState::Ready(view)) = pane.json_tree_state_mut(tab_id) {
            view.apply_node_loaded(path, result, root_index);
        }
    });
}
```

This references `pane.json_tree_state_mut` and `crate::preview::JsonTreeState` which don't exist until Task 9 — that's expected, Task 9 depends on this task's `Message` variants and this task's arms reference Task 9's accessor by name in advance (same forward dependency shape tabular's own `update.rs` had between its message-wiring and `PreviewPane` changes). This task will not compile standalone; run the build at the end of Task 9 instead, not here — skip Step 2's build-verification for these specific arms (do still build after Step 1's derive additions, which are independently valid), and note that explicitly in this task's commit message so nobody's confused by a red build after this task alone.

- [ ] **Step 4: Add `Workspace::preview_pane_json_tree_action` and `Workspace::spawn_pending_json_tree_loads`**

```rust
// crates/dozer-app/src/workspace/state.rs, next to preview_pane_tabular_action / spawn_pending_tabular_loads
pub fn preview_pane_json_tree_action(
    &mut self,
    kind: PanelKind,
    tab_id: usize,
    action: crate::json_tree::Action,
    io: &ShellIo,
) {
    let Some(project_id) = self.project_id() else { return; };
    let pane = if kind == PanelKind::Project { &mut self.project_preview } else { &mut self.preview };
    let Some(request) = pane.json_tree_mut(tab_id).and_then(|view| view.apply(action)) else { return; };
    // The background closure needs an ExpandBytesSource, not a live view
    // reference (same reasoning as json_tree::expand's doc comment) —
    // pane.json_tree_mut(tab_id) above already returned, so build the
    // source from data the pane can hand us by value before the borrow
    // ends. Add a `JsonTreeView::expand_source_for(&self, root_index: usize)
    // -> json_tree::ExpandBytesSource` accessor in Task 5/6 if it's not
    // already there when you reach this step — it needs the tab's file
    // path (store it on JsonTreeView in Task 2/3 if not already present;
    // check before adding a duplicate field) and, for jsonl, that root's
    // line_ranges entry.
    let Some(source) = pane.json_tree_mut(tab_id).map(|v| v.expand_source_for(request.root_index)) else { return; };
    let proxy = io.proxy.clone();
    io.handle.spawn_blocking(move || {
        let result = crate::json_tree::expand(&source, &request.path);
        let _ = proxy.send_event(Message::JsonNodeLoaded(project_id, kind, tab_id, request.path.clone(), request.root_index, result));
    });
}

pub fn spawn_pending_json_tree_loads(&mut self, kind: PanelKind, io: &ShellIo) {
    let Some(project_id) = self.project_id() else { return; };
    let pane = if kind == PanelKind::Project { &mut self.project_preview } else { &mut self.preview };
    for (tab_id, path) in pane.take_pending_json_tree_loads() {
        let proxy = io.proxy.clone();
        io.handle.spawn_blocking(move || {
            let result = crate::json_tree::load(&path);
            let _ = proxy.send_event(Message::JsonTreeLoaded(project_id, kind, tab_id, result));
        });
    }
}
```

This references `pane.json_tree_mut`, `.take_pending_json_tree_loads()`, and `JsonTreeView::expand_source_for` — all defined in Task 9. Same forward-dependency note as Step 3: this file will not build standalone until Task 9 lands. Do not try to make it compile in isolation; that would mean inventing Task 9's interface twice and risking a mismatch (the exact failure mode `writing-plans`' type-consistency check exists to catch) — implement Task 9 immediately after this task, in the same sitting if possible, and build once at the end of Task 9.

- [ ] **Step 5: Commit (no test run — see notes above; Task 9's commit is where this first builds green)**

```bash
git add crates/dozer-app/src/app/message.rs crates/dozer-app/src/app/update.rs crates/dozer-app/src/workspace/state.rs
git commit -m "feat(json-tree): message + async spawn wiring (depends on Task 9 to build)"
```

---

### Task 9: `PreviewTab` integration — dual `editor`+`json_tree` population, derived flags, routing

This is the task that makes Task 8 actually compile, and the one the spec flagged as needing the most care (dual population is different from how tabular fully replaced `editor`).

**Files:**
- Modify: `crates/dozer-app/src/preview/state.rs`
- Modify: `crates/dozer-app/src/preview/view.rs`
- Modify: `crates/dozer-app/src/workspace/view.rs`

**Interfaces:**
- Consumes: `json_tree::{load, is_json_tree_extension, JsonTreeView, Action, ExpandBytesSource}` (Tasks 1-6), `Message::JsonTree*` (Task 8).
- Produces: `PreviewTab.json_tree: Option<JsonTreeState>`, `JsonTreeState::{Loading, Ready}`, `PreviewPane::{json_tree_mut, json_tree_state_mut, take_pending_json_tree_loads}` — closing the loop with Task 8.

- [ ] **Step 1: Add `JsonTreeState` and the `PreviewTab` field**

```rust
// crates/dozer-app/src/preview/state.rs, next to TabularState
pub enum JsonTreeState {
    Loading,
    Ready(crate::json_tree::JsonTreeView),
}

// in struct PreviewTab, next to `tabular`:
    pub json_tree: Option<JsonTreeState>,
```

Update `placeholder_tab` (in `preview/view.rs`) and every test-fixture `PreviewTab { .. }` literal to include `json_tree: None` — search for them:

```bash
grep -rn "PreviewTab {" crates/dozer-app/src/preview/
```

Update the `Debug` impl for `PreviewTab` too, same pattern as `tabular`:

```rust
.field("json_tree", &self.json_tree.is_some())
```

- [ ] **Step 2: Write the failing test for dual population**

```rust
// crates/dozer-app/src/preview/view.rs, in #[cfg(test)] mod tests
#[test]
fn open_json_file_populates_both_editor_and_json_tree() {
    let p = std::env::temp_dir().join(format!("json_route_{}.json", std::process::id()));
    std::fs::write(&p, "{}").unwrap();
    let mut pane = PreviewPane::default();
    pane.open_path(p.clone());
    let tab = &pane.tabs()[pane.active_idx()];
    assert!(tab.editor.is_some(), "JSON 应该仍然进原生代码编辑器(双视图之一)");
    assert!(
        matches!(tab.json_tree, Some(JsonTreeState::Loading)),
        "JSON tab 应该同时进入 json_tree 的 Loading 态"
    );
    std::fs::remove_file(p).ok();
}

#[test]
fn json_tab_is_excluded_from_webview_pool() {
    let p = std::env::temp_dir().join(format!("json_webview_{}.json", std::process::id()));
    std::fs::write(&p, "{}").unwrap();
    let mut pane = PreviewPane::default();
    pane.open_path(p.clone());
    assert!(pane.desired_webviews().is_empty(), "json tab 不该进 webview 池");
    std::fs::remove_file(p).ok();
}
```

- [ ] **Step 3: Run, confirm fails (compile error / `json_tree` always `None`)**

- [ ] **Step 4: Wire `push_tab`**

Find the `push_tab` function (already modified once for tabular — the `editor`/`tabular` construction block). Add JSON handling alongside, without touching the existing tabular/editor branches:

```rust
// after the existing `editor` construction, before `PreviewTab { .. }` is built:
let json_tree = match &kind {
    TabKind::File(path) if crate::json_tree::is_json_tree_extension(path) => {
        self.pending_json_tree_loads.push((id, path.clone()));
        Some(JsonTreeState::Loading)
    }
    _ => None,
};
```

Add `json_tree` to the `PreviewTab { .. }` struct literal, and add the queue field to `PreviewPane`:

```rust
// preview/state.rs, PreviewPane struct, next to pending_tabular_loads
pub(crate) pending_json_tree_loads: Vec<(usize, PathBuf)>,
// and in its Default impl:
pending_json_tree_loads: Vec::new(),
```

- [ ] **Step 5: Run Step 2's tests, confirm the first one passes; the webview one likely still fails — continue to Step 6**

- [ ] **Step 6: Update the webview-pool judgment**

Find every place `tabular.is_none()`/`tabular.is_some()` was added to a pre-existing check during the tabular work (`desired_webviews`, `active_webview_id`, `select`'s `is_webview_file`) — same list the tabular spec documented. Add `&& t.json_tree.is_none()` next to each `&& t.tabular.is_none()`:

```bash
grep -n "tabular.is_none()\|tabular.is_some()" crates/dozer-app/src/preview/view.rs
```

Do **not** touch `active_tab_is_native()` — per the spec, JSON's `editor` is already populated, so `editor.is_some() || tabular.is_some()` already returns `true` for JSON tabs with no change needed. Confirm this by reading the function, not by assuming; if it turns out `active_tab_is_native` has some other condition this plan's author didn't anticipate, that's a real finding — note it, don't silently patch around it without understanding why.

- [ ] **Step 7: Run Step 2's tests again, confirm both PASS**

- [ ] **Step 8: Add the `PreviewPane` accessors Task 8 depends on**

```rust
// preview/view.rs, next to tabular_mut/tabular_state_mut/take_pending_tabular_loads
pub fn json_tree_mut(&mut self, tab_id: usize) -> Option<&mut crate::json_tree::JsonTreeView> {
    self.tabs
        .iter_mut()
        .find(|t| t.id == tab_id)
        .and_then(|t| t.json_tree.as_mut())
        .and_then(|t| match t {
            JsonTreeState::Ready(view) => Some(view),
            JsonTreeState::Loading => None,
        })
}

pub fn json_tree_state_mut(&mut self, tab_id: usize) -> Option<&mut JsonTreeState> {
    self.tabs.iter_mut().find(|t| t.id == tab_id).and_then(|t| t.json_tree.as_mut())
}

pub fn take_pending_json_tree_loads(&mut self) -> Vec<(usize, PathBuf)> {
    std::mem::take(&mut self.pending_json_tree_loads)
}
```

- [ ] **Step 9: Add `JsonTreeView::expand_source_for`, which Task 8's `preview_pane_json_tree_action` calls**

`JsonTreeView` already has the `path` field (added in Task 2, populated by `load_json`/`load_jsonl` since Tasks 3-4) — this step only adds the accessor method, no field/signature changes needed.

```rust
// crates/dozer-app/src/json_tree/mod.rs
impl JsonTreeView {
    pub fn expand_source_for(&self, root_index: usize) -> ExpandBytesSource {
        match &self.line_ranges {
            None => ExpandBytesSource::WholeFile(self.path.clone()),
            Some(ranges) => ExpandBytesSource::JsonLine {
                path: self.path.clone(),
                byte_range: ranges[root_index].clone(),
            },
        }
    }
}
```

Add a unit test for this in `json_tree::mod.rs`'s test module (both branches: a `.json` view returns `WholeFile`, a `.jsonl` view returns `JsonLine` with the right range for a non-zero root index).

- [ ] **Step 10: Wire the toggle button / view-mode routing into `workspace/view.rs`**

Find the tabular render arm added earlier (`else if let Some(tabular) = &active_tab.tabular { match tabular { TabularState::Ready(view) => ..., TabularState::Loading => ... } }`). Add a JSON arm right after it, before the `else if active_tab.kind == TabKind::Blank` arm:

```rust
} else if let Some(json_tree) = &active_tab.json_tree {
    match json_tree {
        crate::preview::JsonTreeState::Ready(view) if view.view_mode == crate::json_tree::ViewMode::RawText => {
            // Toggle is in RawText mode: render the existing editor (same
            // as the plain-text-editor branch above this whole if/else
            // chain does for non-JSON files), but we're inside the
            // json_tree arm so it needs its own render call here — find
            // that existing editor-rendering block earlier in this same
            // function and reuse its exact Element-building call, do not
            // rewrite it.
            let tab_id = active_tab.id;
            let panel = find_panel();
            if let Some(editor) = &active_tab.editor {
                content = content.push(
                    container(editor.view().map(move |ev| editor_msg(tab_id, ev)))
                        .width(Length::Fill)
                        .height(Length::Fill),
                );
            }
            // Still need the toggle button visible even in RawText mode —
            // render json_tree's view() ABOVE the editor for its header/
            // toggle row, or restructure json_tree::view() (Task 7) to take
            // the body as a parameter instead of always rendering the tree
            // canvas. Decide which and make json_tree::view()'s signature
            // match — if you change it from `fn view(&self) -> Element`,
            // go back and update Task 7's code to match, don't leave two
            // diverging versions.
        }
        crate::preview::JsonTreeState::Ready(view) => {
            let tab_id = active_tab.id;
            let panel = find_panel();
            content = content.push(
                container(view.view().map(move |act| Message::JsonTreeAction(panel, tab_id, act)))
                    .width(Length::Fill)
                    .height(Length::Fill),
            );
        }
        crate::preview::JsonTreeState::Loading => {
            content = content.push(byteui::feedback::math_curve::loading_hint(
                byteui::feedback::math_curve::Curve::RoseThree,
                "正在打开 JSON…",
                48.0,
            ));
        }
    }
}
```

The comment block calling out the toggle-button-visible-in-RawText-mode problem is a real open design gap this plan is handing to whoever implements this step — resolve it concretely (pick one of the two options named, implement it, delete the comment) rather than leaving the ambiguity in committed code. Whichever you pick, add a test in `preview/view.rs` asserting the toggle button's `Element` tree exists in both `ViewMode::Tree` and `ViewMode::RawText` (a snapshot-style check isn't available here; instead assert on `JsonTreeView::view_mode` transitions via `apply(Action::ToggleViewMode)` the same way Task 2's tests did, which at least proves the state machine side is right — full visual verification happens in Task 10's manual pass).

- [ ] **Step 11: Wire `preview_open_path`/`project_preview_open_path` to spawn pending JSON loads**

Find the two call sites Task 9 of the tabular plan touched (`app/update.rs`'s `preview_open_path` and `project_preview_open_path`, right after `ws.preview.open_path(path); ws.spawn_pending_tabular_loads(PanelKind::Files, io);`). Add the JSON equivalent right next to it:

```rust
ws.preview.open_path(path);
ws.spawn_pending_tabular_loads(PanelKind::Files, io);
ws.spawn_pending_json_tree_loads(PanelKind::Files, io);
```

(and the `PanelKind::Project` equivalent in `project_preview_open_path`.)

- [ ] **Step 12: Wire `restore_preview_state`**

Find `restore_preview_state`'s existing `self.spawn_pending_tabular_loads(PanelKind::Files, io);` line (added during the tabular work) and add:

```rust
self.spawn_pending_json_tree_loads(PanelKind::Files, io);
```

- [ ] **Step 13: Full build + full test suite**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
```

Expected: everything green except the one pre-existing, environment-specific `extensions::git_log::tests::build_marks_head_branch_and_labels` failure (only reproduces inside a git worktree, already confirmed unrelated and pre-existing during the tabular work — do not investigate it again, do not let it block this task).

- [ ] **Step 14: fmt, commit**

```bash
cargo fmt -p dozer-app
git status --short   # revert any file this plan didn't touch that fmt reformatted
git add crates/dozer-app/src/preview/state.rs crates/dozer-app/src/preview/view.rs crates/dozer-app/src/workspace/view.rs crates/dozer-app/src/app/message.rs crates/dozer-app/src/app/update.rs crates/dozer-app/src/workspace/state.rs
git commit -m "feat(json-tree): PreviewTab dual-population routing, closes message-wiring loop"
```

---

### Task 10: End-to-end verification

**Files:** none (verification only).

- [ ] **Step 1: Full suite one more time from a clean build**

```bash
cargo clean -p dozer-app
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
```

Confirm no new clippy warnings in any `json_tree`/`preview`/`workspace`/`app` file this plan touched (pre-existing unrelated warnings, e.g. `GitProvider::ALL`, are not this plan's concern).

- [ ] **Step 2: Generate large manual-test fixtures**

```bash
python3 -c "
import json, random
data = {'records': [{'id': i, 'name': f'item-{i}', 'tags': ['a','b','c'], 'nested': {'x': i, 'y': i*2}} for i in range(2_000_000)]}
with open('/tmp/big_test.json', 'w') as f:
    json.dump(data, f)
"
python3 -c "
import json
with open('/tmp/big_test.jsonl', 'w') as f:
    for i in range(500_000):
        f.write(json.dumps({'id': i, 'name': f'item-{i}', 'tags': ['a','b','c']}) + '\n')
"
ls -lh /tmp/big_test.json /tmp/big_test.jsonl
```

(Adjust the record count if these don't land near the 1GB target on this machine — the point is a realistic large-file test, not an exact byte count.)

- [ ] **Step 3: Manual smoke test — run the actual app**

```bash
cargo run -p dozer-app
```

Checklist (check off each, note anything that fails instead of silently skipping it):
- [ ] Open `/tmp/big_test.json` from the Files tree. It should show the loading spinner briefly, then a collapsed root row — not freeze the window.
- [ ] Expand the root, then expand `records` (an array of 2M items) — this should show a capped view (10,000 items + truncation indicator per the spec), not attempt to render 2M rows.
- [ ] Expand a few individual array items in non-sequential order (e.g. item 500, then item 2, then item 9,999) — this is the actual test of Task 1's random-order-access finding under real UI interaction, not just the unit test's synthetic fixture.
- [ ] Toggle to raw text view — should show the actual JSON text, scrollable, no lag.
- [ ] Toggle back to Tree — should NOT re-parse the whole file (should be instant; if it visibly re-triggers a loading spinner, that's a bug — the toggle must reuse already-loaded state, check `Action::ToggleViewMode`'s handling doesn't accidentally clear anything).
- [ ] Open `/tmp/big_test.jsonl`. Root list should show 500,000 root rows... **wait**: `MAX_JSON_LINES = 100_000` — this fixture has 500K lines, so confirm it's capped at exactly 100,000 root rows and a truncation indicator is shown, not all 500K.
- [ ] Scroll through the jsonl root list — should be smooth (virtualized), not laggy.
- [ ] Close and reopen the project (or restart the app) with these tabs still open from last session — confirm they restore via `restore_preview_state` and re-load in the background without blocking startup.
- [ ] Open a small malformed `.json` file (e.g. `echo '{not valid' > /tmp/bad.json`) — confirm the tab doesn't crash the app; per the spec's error-handling table it should stay on `Loading` (an intentional accepted gap, not a bug — don't "fix" this during verification, just confirm it doesn't crash).
- [ ] Open a `.jsonl` file with one bad line among good ones — confirm only that one root row shows an error, the rest render normally.

- [ ] **Step 4: Record findings**

If anything in Step 3's checklist failed, do not commit a "done" state — go back to the relevant task above, fix it there (respecting that task's original design intent), re-run that task's own tests, then re-run this task's Step 1 and Step 3 checklist from the top.

If everything passed: update this plan file, checking off every remaining `- [ ]` box in this document (all tasks, all steps), and commit:

```bash
git add docs/superpowers/plans/2026-09-19-json-preview-viewer.md
git commit -m "docs: mark json-tree preview plan complete"
```

- [ ] **Step 5: Report back**

Summarize for the human: what was built, the confirmed sonic-rs on-demand random-access finding (Task 1), any deviations from the spec discovered during implementation (the `ExpandSource`/byte-range simplification from Task 1 is one known one — list any others found along the way), and the current branch/worktree name ready for review and merge into `main`.
