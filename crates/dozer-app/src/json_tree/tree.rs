//! 只读 JSON 树的虚拟化渲染（基于 `iced_widget::canvas`）。
//!
//! 与 `tabular/grid.rs` 同源的取舍：`flatten_visible_rows` 的开销只与
//! 「用户当前展开了多少」成正比，永远与文件大小无关；`draw()` 只画落进
//! 可视窗口的行。滚动上界在 `draw()` 内按当前行数现算后裁剪（`apply`
//! 只管到下界 0），保持唯一真相。
//!
//! chevron 用文本字形（`▸`/`▾`）而非 `icons::view` 的 SVG：canvas 的
//! `Frame` 不能直接嵌入 svg widget，而文件树那套 `IconKind::Chevron*`
//! 是 `svg::Handle` 渲染的 `Element`，两者机制不互通。这是对计划里
//! 「chevron 作为可绘制图标」想法的偏离，理由记录于此。

use std::sync::Arc;

use iced_widget::canvas::{self, Frame, Text as CanvasText};
use iced_widget::core::alignment;
use iced_widget::core::event::Event;
use iced_widget::core::mouse;
use iced_widget::core::text::{Alignment, LineHeight, Shaping};
use iced_widget::core::{Color, Element, Font, Length, Pixels, Point, Rectangle};

use super::{Action, JsonKind, JsonTreeView, NodeContent, NodePath, PathSegment};

#[derive(Debug, Clone, PartialEq)]
pub struct VisibleRow {
    pub root_index: usize,
    pub path: NodePath,
    pub depth: usize,
    /// 根为 ""，子节点为 `key` 或 `[i]`。
    pub key_label: String,
    pub kind: JsonKind,
    pub expandable: bool,
    pub expanded: bool,
    /// 合成行（非真实节点）：目前用于「子级被 `MAX_JSON_CHILDREN` 截断」的
    /// 提示行，以及「某行解析失败」的占位行。合成行不可展开、不参与命中。
    pub sentinel: bool,
}

/// 只摊平「当前已展开」的部分 —— 开销与用户打开的程度成正比，永不随
/// 文件大小增长（与 tabular 固定 `total_rows` 模型的不同之处）。
pub fn flatten_visible_rows(view: &JsonTreeView) -> Vec<VisibleRow> {
    let mut rows = Vec::new();
    for (root_index, root) in view.roots.iter().enumerate() {
        let Ok(node) = root else {
            rows.push(VisibleRow {
                root_index,
                path: Arc::from(Vec::<PathSegment>::new()),
                depth: 0,
                key_label: format!("(第 {root_index} 行解析失败)"),
                kind: JsonKind::Null,
                expandable: false,
                expanded: false,
                sentinel: true,
            });
            continue;
        };
        let root_path: NodePath = Arc::from(Vec::<PathSegment>::new());
        push_row_and_children(
            view,
            &mut rows,
            root_index,
            root_path,
            0,
            String::new(),
            node.kind,
        );
    }
    rows
}

#[allow(clippy::too_many_arguments)]
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
    rows.push(VisibleRow {
        root_index,
        path: path.clone(),
        depth,
        key_label,
        kind,
        expandable,
        expanded,
        sentinel: false,
    });
    if !expanded {
        return;
    }
    let Some(content) = view.content_at(root_index, &path) else {
        return; // 已展开但尚未解码 —— 只有这一行，没有子节点可显示
    };
    match content {
        NodeContent::Object { entries, truncated } => {
            for (key, child_kind) in entries {
                let mut segs: Vec<PathSegment> = path.iter().cloned().collect();
                segs.push(PathSegment::Key(key.clone()));
                push_row_and_children(
                    view,
                    rows,
                    root_index,
                    Arc::from(segs),
                    depth + 1,
                    key.clone(),
                    *child_kind,
                );
            }
            if *truncated {
                push_truncation_sentinel(rows, root_index, depth + 1);
            }
        }
        NodeContent::Array { items, truncated } => {
            for (i, child_kind) in items.iter().enumerate() {
                let mut segs: Vec<PathSegment> = path.iter().cloned().collect();
                segs.push(PathSegment::Index(i));
                push_row_and_children(
                    view,
                    rows,
                    root_index,
                    Arc::from(segs),
                    depth + 1,
                    format!("[{i}]"),
                    *child_kind,
                );
            }
            if *truncated {
                push_truncation_sentinel(rows, root_index, depth + 1);
            }
        }
        NodeContent::Leaf { .. } => {} // 叶子没有子节点可递归
    }
}

/// 子级超过 `MAX_JSON_CHILDREN` 时，在末尾插一行提示，让用户知道这里不是
/// 全部（规格：超过上限不做精确统计，展示"还有更多、已截断"即可）。
fn push_truncation_sentinel(rows: &mut Vec<VisibleRow>, root_index: usize, depth: usize) {
    rows.push(VisibleRow {
        root_index,
        path: Arc::from(Vec::<PathSegment>::new()),
        depth,
        key_label: format!("…（仅显示前 {} 项，还有更多）", super::MAX_JSON_CHILDREN),
        kind: JsonKind::Null,
        expandable: false,
        expanded: false,
        sentinel: true,
    });
}

/// 行高 = 字号 × 该因子（与终端行距同源观感，对齐 tabular/grid.rs）。
const ROW_HEIGHT_FACTOR: f32 = 1.5;

/// 渲染几何，由当前字号/scale 每帧现算，`update` 与 `draw` 共用同一套，
/// 保证点击命中区与绘制行高/缩进不会各算一套而漂移。
struct Metrics {
    font_size: f32,
    row_height: f32,
    indent_step: f32,
    chevron_w: f32,
}

impl Metrics {
    fn new() -> Self {
        let font_size = crate::theme::terminal_font::size() * byteui::theme::icon_size::scale();
        let chevron_w = byteui::theme::icon_size::chevron();
        let gap = byteui::theme::icon_size::tree_row_gap();
        Self {
            font_size,
            row_height: font_size * ROW_HEIGHT_FACTOR,
            indent_step: chevron_w + gap,
            chevron_w,
        }
    }

    /// 某一 depth 的行左侧缩进像素。
    fn indent(&self, depth: usize) -> f32 {
        depth as f32 * self.indent_step
    }
}

fn row_text(content: String, position: Point, color: Color, m: &Metrics) -> CanvasText {
    CanvasText {
        content,
        position,
        color,
        size: Pixels(m.font_size),
        line_height: LineHeight::Relative(1.0),
        // 系统默认字体（非代码场景不用 JetBrains Mono，见 CLAUDE.md 字体
        // 统一裁决）；`Shaping::Advanced` 做字体回退——中文等非 ASCII 字形
        // 回退到系统 CJK 字体，`Basic` 明确不做回退会让中文变方块。
        font: Font::default(),
        align_x: Alignment::Left,
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Advanced,
        ..Default::default()
    }
}

pub struct TreeCanvas<'a> {
    view: &'a JsonTreeView,
    rows: Vec<VisibleRow>,
}

impl<'a> TreeCanvas<'a> {
    /// 「展开/折叠」chevron 的命中区：仅 chevron 所在的左侧窄条，避免整行
    /// 点击都触发展开。命中区用与 `draw()` 同一套 `Metrics`，并同样按
    /// 「当前行数」裁剪 scroll 上界，保持点击与绘制一致。
    fn chevron_hit_at(&self, cursor: Point, bounds: Rectangle, m: &Metrics) -> Option<NodePath> {
        if cursor.y < 0.0 || cursor.y > bounds.height {
            return None;
        }
        let row_i = (cursor.y / m.row_height) as usize;
        let rows_visible = ((bounds.height / m.row_height).ceil() as usize).max(1);
        let max_scroll = self.rows.len().saturating_sub(rows_visible);
        let scroll = self.view.scroll_row.min(max_scroll);
        let logical = scroll + row_i;
        let row = self.rows.get(logical)?;
        if !row.expandable {
            return None;
        }
        let left = m.indent(row.depth);
        let right = left + m.chevron_w;
        if cursor.x >= left && cursor.x <= right {
            Some(row.path.clone())
        } else {
            None
        }
    }
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
        let m = Metrics::new();
        match event {
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let dy: i32 = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => (*y * 3.0).round() as i32,
                    mouse::ScrollDelta::Pixels { y, .. } => (y / m.row_height).round() as i32,
                };
                if dy == 0 {
                    return None;
                }
                Some(canvas::Action::publish(Action::Scroll { dy }))
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let cursor = cursor.position()?;
                let path = self.chevron_hit_at(cursor, bounds, &m)?;
                Some(canvas::Action::publish(Action::ToggleExpand(path)))
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_renderer::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let m = Metrics::new();
        let colors = byteui::theme::color::current();
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), colors.panel);

        let rows_visible = ((bounds.height / m.row_height).ceil() as usize).max(1);
        let max_scroll = self.rows.len().saturating_sub(rows_visible);
        let scroll = self.view.scroll_row.min(max_scroll);
        let end = (scroll + rows_visible).min(self.rows.len());

        for (i, row) in self.rows[scroll..end].iter().enumerate() {
            let y = i as f32 * m.row_height;
            let indent = m.indent(row.depth);
            if row.expandable {
                // 见模块注释：canvas 嵌不了 SVG 图标，故用字形。
                let chevron = if row.expanded { "▾" } else { "▸" };
                frame.fill_text(row_text(
                    chevron.to_string(),
                    Point::new(indent, y + (m.row_height - m.font_size) / 2.0),
                    colors.dim,
                    &m,
                ));
            }
            let label = row_label(self.view, row);
            frame.fill_text(row_text(
                label,
                Point::new(
                    indent + m.indent_step,
                    y + (m.row_height - m.font_size) / 2.0,
                ),
                colors.body,
                &m,
            ));
        }
        vec![frame.into_geometry()]
    }
}

/// 一行的显示文本。`key` 前缀 + 值：
/// - 叶子：已解码时显示其 `Leaf.display`（字符串值去掉 JSON 引号后展示——
///   数据层 `as_raw_str()` 带引号，正是计划 Task 1 留的「渲染期再决定要不要
///   去引号」这个口子），带截断标记；未解码时退回 kind。
/// - 容器：显示 kind（`Object`/`Array`），子级跟着展开才逐个出现。
fn row_label(view: &JsonTreeView, row: &VisibleRow) -> String {
    if row.sentinel {
        return row.key_label.clone();
    }
    let prefix = row.key_label.as_str();
    let value = match view.content_at(row.root_index, &row.path) {
        Some(NodeContent::Leaf { display, truncated }) => {
            let mut s = strip_json_quotes(display);
            if *truncated {
                s.push('…');
            }
            s
        }
        _ => format!("{:?}", row.kind),
    };
    if prefix.is_empty() {
        value
    } else {
        format!("{prefix}: {value}")
    }
}

/// 字符串叶子去掉 `as_raw_str()` 自带的首尾 JSON 引号（仅去一层成对引号，
/// 不处理转义——预览够用，`\"` 之类原样显示比自作主张反转义更不容易误导）。
fn strip_json_quotes(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// 把树包成 `Canvas` widget（`Message` 泛型由调用方 `.map` 收敛）。
pub fn view(
    json_view: &JsonTreeView,
) -> Element<'_, Action, iced_widget::Theme, iced_renderer::Renderer> {
    let rows = flatten_visible_rows(json_view);
    canvas::Canvas::new(TreeCanvas {
        view: json_view,
        rows,
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_tree::{JsonNode, JsonTreeView};

    #[test]
    fn strip_json_quotes_removes_one_outer_pair_only() {
        assert_eq!(strip_json_quotes("\"hello\""), "hello");
        assert_eq!(strip_json_quotes("123"), "123");
        assert_eq!(strip_json_quotes("\""), "\"");
        // 不反转义：内层引号原样保留，预览够用。
        assert_eq!(strip_json_quotes("\"a\\\"b\""), "a\\\"b");
    }

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
            entries: vec![
                ("a".into(), JsonKind::Number),
                ("b".into(), JsonKind::String),
            ],
            truncated: false,
        };
        let mut view = view_with_root(JsonKind::Object, Some(content));
        let root_path: NodePath = Arc::from(Vec::<PathSegment>::new());
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
        view.apply(Action::ToggleExpand(Arc::from(Vec::<PathSegment>::new())));
        let child_path: NodePath = Arc::from(vec![PathSegment::Key("child".to_string())]);
        view.apply(Action::ToggleExpand(child_path)); // expand child; not decoded yet
        let rows = flatten_visible_rows(&view);
        assert_eq!(
            rows.len(),
            2,
            "root + child row; child's own children not shown until decoded"
        );
        assert!(
            rows[1].expandable,
            "still shown as an openable object even though not decoded yet"
        );
    }

    #[test]
    fn expanded_truncated_container_appends_a_sentinel_row() {
        let content = NodeContent::Object {
            entries: vec![("a".into(), JsonKind::Number)],
            truncated: true,
        };
        let mut view = view_with_root(JsonKind::Object, Some(content));
        view.apply(Action::ToggleExpand(Arc::from(Vec::<PathSegment>::new())));
        let rows = flatten_visible_rows(&view);
        assert_eq!(rows.len(), 3, "root + child + truncation sentinel");
        assert!(rows[2].sentinel, "末行应是截断提示合成行");
        assert!(!rows[2].expandable);
    }
}
