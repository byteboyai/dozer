//! 只读 JSON/JSONL 树预览的数据层。
//!
//! 设计主线：打开大文件时不整体物化 JSON，只对「用户当前看到/展开的节点」
//! 做按需解码（`sonic_rs::get` 每次从头重新解析路径，见 `decode_node` spike 注释）；
//! 所有触盘解码都跑在后台线程上，由上层消息层路由回 UI。

use std::path::Path;

pub const MAX_JSON_CHILDREN: usize = 10_000;
pub const MAX_LEAF_PREVIEW_CHARS: usize = 2_000;
pub const MAX_JSON_LINES: usize = 100_000;

/// 单个节点在文档里的定位：对象取键，数组取下标。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

/// 从根到某节点的路径，共享所有权（flatten/缓存都用它做 key，避免反复深拷贝）。
pub type NodePath = std::rc::Rc<[PathSegment]>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonKind {
    Object,
    Array,
    String,
    Number,
    Bool,
    Null,
}

/// 一个节点「直接子级」的形状，或叶子节点的文本预览。
/// 不递归持有子节点 —— 子节点自己变成一个独立可展开的节点，等它被展开时才解码。
#[derive(Debug, Clone)]
pub enum NodeContent {
    Object {
        entries: Vec<(String, JsonKind)>,
        truncated: bool,
    },
    Array {
        items: Vec<JsonKind>,
        truncated: bool,
    },
    Leaf {
        display: String,
        truncated: bool,
    },
}

#[derive(Debug, Clone)]
pub struct JsonNode {
    pub kind: JsonKind,
    /// 未解码前为 `None`（根节点在加载时即被解码；其余节点惰性填充，见 `apply_node_loaded`）。
    pub content: Option<NodeContent>,
}

/// 一个文档（`.json`）或一行（`.jsonl`）解码后的根，或该根产生的错误。
pub type RootResult = Result<JsonNode, String>;

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

pub struct JsonTreeView {
    /// 源文件路径 —— 每次惰性展开都从磁盘重读（原因见 `expand()` 文档）。
    path: std::path::PathBuf,
    /// 常驻源字节：`.json` 为整文件；`.jsonl`/`.ndjson` 也是整文件
    /// （行靠 `line_ranges` 切片，不另存一份，避免大文件双份内存）。
    bytes: Vec<u8>,
    /// jsonl/ndjson 为 `Some`（与 `roots` 下标对齐）；单个 `.json` 为 `None`。
    line_ranges: Option<Vec<std::ops::Range<usize>>>,
    pub roots: Vec<RootResult>,
    expanded: std::collections::HashSet<NodePath>,
    /// 非根节点的已解码内容，按路径缓存（根节点内容在 `roots[i].content`）。
    decoded: std::collections::HashMap<NodePath, NodeContent>,
    loading_nodes: std::collections::HashSet<NodePath>,
    pub view_mode: ViewMode,
    pub scroll_row: usize,
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
            decoded: std::collections::HashMap::new(),
            loading_nodes: std::collections::HashSet::new(),
            view_mode: ViewMode::Tree,
            scroll_row: 0,
        }
    }

    /// 纯状态转移，不做 IO。`Scroll` 只在下界 0 处收敛 —— 上界取决于
    /// 当前可见行数（展开/折叠会变），由 canvas widget 的 `draw()` 每帧
    /// 用同一个 flatten 函数现算后裁剪，保持唯一真相。
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
        if self.expanded.remove(&path) {
            return None; // 之前是展开的，现在折叠 —— 从不触发加载
        }
        self.expanded.insert(path.clone());
        let already_decoded = if path.is_empty() {
            self.roots
                .first()
                .is_some_and(|r| matches!(r, Ok(n) if n.content.is_some()))
        } else {
            self.decoded.contains_key(&path)
        };
        if already_decoded || !self.loading_nodes.insert(path.clone()) {
            return None;
        }
        Some(NodeExpandRequest {
            path,
            root_index: 0,
        }) // root_index 由 Task 4/5 修正
    }

    /// 后台解码完成；`root_index` 标识哪个根（单个 `.json` 恒为 0，jsonl 为行号）。
    pub fn apply_node_loaded(
        &mut self,
        path: NodePath,
        result: Result<NodeContent, String>,
        root_index: usize,
    ) {
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
///
/// Decode exactly one node's direct-children shape at `path` (empty path
/// = the document root). Does not recurse into children's own children —
/// that only happens on their own later `decode_node` call, when the
/// user expands them.
pub fn decode_node(bytes: &[u8], path: &[PathSegment]) -> Result<NodeContent, String> {
    use sonic_rs::PointerNode;
    let index_path: Vec<PointerNode> = path
        .iter()
        .map(|seg| match seg {
            PathSegment::Key(k) => PointerNode::Key(sonic_rs::FastStr::new(k.clone())),
            PathSegment::Index(i) => PointerNode::Index(*i),
        })
        .collect();
    let mut value = sonic_rs::get(bytes, index_path).map_err(|e| e.to_string())?;
    decode_lazy_value(&mut value)
}

/// `LazyValue -> JsonKind` 映射，`decode_lazy_value` 与 `root_kind_of` 共用。
fn lazy_value_kind(value: &sonic_rs::LazyValue) -> JsonKind {
    use sonic_rs::{JsonType, JsonValueTrait};
    match value.get_type() {
        JsonType::Object => JsonKind::Object,
        JsonType::Array => JsonKind::Array,
        JsonType::String => JsonKind::String,
        JsonType::Number => JsonKind::Number,
        JsonType::Boolean => JsonKind::Bool,
        JsonType::Null => JsonKind::Null,
    }
}

fn decode_lazy_value(value: &mut sonic_rs::LazyValue) -> Result<NodeContent, String> {
    use sonic_rs::{JsonType, JsonValueTrait};
    match value.get_type() {
        JsonType::Object => {
            let mut entries = Vec::new();
            let mut truncated = false;
            if let Some(iter) = value.clone().into_object_iter() {
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
            if let Some(iter) = value.clone().into_array_iter() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_json_tree_extension_covers_formats_case_insensitive() {
        for p in [
            "/tmp/a.json",
            "/tmp/a.jsonl",
            "/tmp/a.ndjson",
            "/tmp/A.JSON",
        ] {
            assert!(is_json_tree_extension(Path::new(p)), "{p}");
        }
        for p in ["/tmp/a.md", "/tmp/a.csv", "/tmp/a.json5", "/tmp/a.jsonc"] {
            assert!(!is_json_tree_extension(Path::new(p)), "{p}");
        }
    }
}

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

    #[test]
    fn decode_node_supports_out_of_order_path_access() {
        let tags_path = [PathSegment::Key("tags".to_string())];
        let meta_path = [PathSegment::Key("meta".to_string())];
        let meta = decode_node(FIXTURE.as_bytes(), &meta_path).unwrap();
        let tags = decode_node(FIXTURE.as_bytes(), &tags_path).unwrap();
        assert!(matches!(meta, NodeContent::Object { .. }));
        assert!(matches!(tags, NodeContent::Array { .. }));
    }

    #[test]
    fn decode_node_caps_object_children_at_max_json_children() {
        let mut obj = String::from("{");
        for i in 0..MAX_JSON_CHILDREN + 5 {
            if i > 0 {
                obj.push(',');
            }
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
        let content =
            decode_node(br#"{"s": "short"}"#, &[PathSegment::Key("s".to_string())]).unwrap();
        let NodeContent::Leaf { display, truncated } = content else {
            panic!("expected leaf");
        };
        assert_eq!(
            display, "\"short\"",
            "as_raw_str includes the JSON quoting, this plan does not strip it — confirm this reads acceptably in the tree UI in Task 6, strip quotes there at render time if not, not here in the data layer"
        );
        assert!(!truncated);
    }
}

pub fn load_json(path: &std::path::Path) -> Result<JsonTreeView, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let root_kind = root_kind_of(&bytes)?;
    let content = decode_node(&bytes, &[])?;
    let root = JsonNode {
        kind: root_kind,
        content: Some(content),
    };
    Ok(JsonTreeView::new(
        path.to_path_buf(),
        bytes,
        None,
        vec![Ok(root)],
    ))
}

/// 只做顶层类型判断，不展开子级 —— 让 `JsonNode.kind` 在 `decode_node`
/// 返回前就准确（`decode_node` 只报告子级的 kind，不报告自己）。
fn root_kind_of(bytes: &[u8]) -> Result<JsonKind, String> {
    use sonic_rs::{JsonValueTrait, PointerNode};
    let empty: Vec<PointerNode> = Vec::new();
    let value = sonic_rs::get(bytes, empty).map_err(|e| e.to_string())?;
    Ok(lazy_value_kind(&value))
}

pub fn load_jsonl(path: &std::path::Path) -> Result<JsonTreeView, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut line_ranges = Vec::new();
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            if start < i {
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
            Ok(JsonNode {
                kind,
                content: Some(content),
            })
        })
        .collect();
    Ok(JsonTreeView::new(
        path.to_path_buf(),
        bytes,
        Some(line_ranges),
        roots,
    ))
}

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
        assert_eq!(
            view.roots.len(),
            MAX_JSON_LINES,
            "must stop at the cap, not read the whole file"
        );
    }
}

pub fn load(path: &std::path::Path) -> Result<JsonTreeView, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "json" => load_json(path),
        "jsonl" | "ndjson" => load_jsonl(path),
        _ => Err(format!("非 JSON 文件: {ext}")),
    }
}

/// `expand()` 解码所依据的字节来源 —— 由调用方（Task 8 的消息处理）
/// 在 spawn 后台线程前，用已有数据构造（绝不借用活的 `&JsonTreeView`，
/// 那会要求跨异步边界借用 view）。
pub enum ExpandBytesSource {
    WholeFile(std::path::PathBuf),
    JsonLine {
        path: std::path::PathBuf,
        byte_range: std::ops::Range<usize>,
    },
}

/// 后台线程按需解码单个节点的入口。
///
/// 注意这里每次展开都从磁盘重读文件，而不是复用初次加载已常驻内存的
/// `bytes` —— 这是 tabular spec 为 `load_sheet` 每次惰性切表都重开
/// workbook 记录过的同一取舍：展开是低频、显式、已被 spinner 覆盖的用户
/// 动作，不是热路径；而跨线程边界维持对 view `bytes` 的活借用，正是那份
/// 取舍要规避的复杂度。未来 reviewer 不要在不重读那份理由的情况下，把它
/// “优化”成新的复杂度。
pub fn expand(source: &ExpandBytesSource, path: &[PathSegment]) -> Result<NodeContent, String> {
    match source {
        ExpandBytesSource::WholeFile(p) => {
            let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
            decode_node(&bytes, path)
        }
        ExpandBytesSource::JsonLine {
            path: p,
            byte_range,
        } => {
            let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
            let line = bytes
                .get(byte_range.clone())
                .ok_or("行范围越界(文件在打开后被改动?)")?;
            decode_node(line, path)
        }
    }
}

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

#[cfg(test)]
mod expand_tests {
    use super::*;

    #[test]
    fn expand_whole_file_decodes_the_requested_path() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.json");
        std::fs::write(&p, r#"{"a": {"b": 1}}"#).unwrap();
        let content = expand(
            &ExpandBytesSource::WholeFile(p),
            &[PathSegment::Key("a".into())],
        )
        .unwrap();
        assert!(matches!(content, NodeContent::Object { .. }));
    }

    #[test]
    fn expand_json_line_decodes_within_that_lines_byte_range() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.jsonl");
        std::fs::write(&p, "{\"x\":1}\n{\"y\":[1,2]}\n").unwrap();
        let first_len = "{\"x\":1}\n".len();
        let second_len = "{\"y\":[1,2]}".len();
        let range = first_len..(first_len + second_len);
        let content = expand(
            &ExpandBytesSource::JsonLine {
                path: p,
                byte_range: range,
            },
            &[PathSegment::Key("y".into())],
        )
        .unwrap();
        assert!(matches!(content, NodeContent::Array { .. }));
    }
}

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

#[cfg(test)]
mod apply_tests {
    use super::*;
    use std::rc::Rc;

    fn empty_view() -> JsonTreeView {
        JsonTreeView::new(
            std::path::PathBuf::from("/tmp/x.json"),
            b"{}".to_vec(),
            None,
            vec![Ok(JsonNode {
                kind: JsonKind::Object,
                content: None,
            })],
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

    #[test]
    fn toggle_expand_on_new_path_expands_and_requests_load_if_undecoded() {
        let mut v = empty_view();
        let path: NodePath = Rc::from(vec![PathSegment::Key("a".to_string())]);
        let req = v.apply(Action::ToggleExpand(path.clone()));
        assert_eq!(
            req,
            Some(NodeExpandRequest {
                path: path.clone(),
                root_index: 0
            })
        );
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
        assert!(
            v.apply(Action::ToggleExpand(path.clone())).is_some(),
            "first expand requests a load"
        );
        v.apply(Action::ToggleExpand(path.clone())); // collapse
        let req = v.apply(Action::ToggleExpand(path.clone())); // expand again, load still in flight
        assert_eq!(
            req, None,
            "a load for this path is already in flight, must not spawn a second one"
        );
    }

    #[test]
    fn expanding_already_decoded_node_does_not_request_load() {
        let mut v = empty_view();
        let path: NodePath = Rc::from(vec![PathSegment::Key("a".to_string())]);
        v.apply(Action::ToggleExpand(path.clone()));
        v.apply_node_loaded(
            path.clone(),
            Ok(NodeContent::Leaf {
                display: "1".into(),
                truncated: false,
            }),
            0,
        );
        v.apply(Action::ToggleExpand(path.clone())); // collapse
        let req = v.apply(Action::ToggleExpand(path.clone())); // expand again, already decoded
        assert_eq!(req, None, "content is cached, no need to re-decode");
    }
}
