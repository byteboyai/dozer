//! 只读 JSON/JSONL 树预览的数据层。
//!
//! 设计主线：打开大文件时不整体物化 JSON，只对「用户当前看到/展开的节点」
//! 做按需解码（`sonic_rs::get` 每次从头重新解析路径，见 `decode_node` spike 注释）；
//! 所有触盘解码都跑在后台线程上，由上层消息层路由回 UI。

use std::path::Path;

pub const MAX_JSON_CHILDREN: usize = 10_000;
pub const MAX_LEAF_PREVIEW_CHARS: usize = 2_000;

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
