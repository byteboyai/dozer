//! 官方 iced `text_editor` + syntect 语法高亮的组合组件,preview.rs 原生文本
//! 预览与 workspace.rs 编辑浮层共用,替代 vendored `iced-code-editor`(2026-09
//! 删除,见 git 历史)。删除动机:vendored 版本内部用 `iced_aw::ContextMenu`
//! 包代码画布,`iced_aw 0.13.1` 的 `ContextMenu::operate` 在菜单展开时有布局
//! 层级 panic,`main.rs` 曾需要给全应用每次 `operate()` 遍历套 `catch_unwind`
//! 兜底——补丁面覆盖全应用而不是编辑器局部,是换掉整个依赖的直接原因。
//!
//! 用途演进(2026-09-06):原生文本预览不再强制只读 —— preview.rs 现在用
//! `read_only=false` 建编辑器,用户可选中/复制/就地编辑;保存与脏标记由
//! `workspace.rs`(`PreviewPane`/`EditSession`)各自负责。`read_only` 参数仍保留,
//! 供(将来)确需只读展示的场景;分派逻辑不变:只读时滤掉 `Action::Edit`。
//!
//! 已知取舍(用户已确认接受):
//! - 自建行号 gutter 靠应用层累加 `Action::Scroll{lines}` 镜像滚动位置——
//!   `iced_core::text::Editor` trait 没有暴露读取真实滚动偏移的公开 API,
//!   文件末尾附近可能出现行号栏先于正文变空/漂移,纯视觉瑕疵,不影响编辑。
//! - 光标/选区的逻辑坐标(`cursor_position`/`selection_range`,供 MCP
//!   `get_preview_context` 报行号给 agent)不走这个滚动镜像,直接读
//!   `Content::cursor()` 的 `Position{line,column}`,精确无漂移风险。
//! - 官方 `text_editor` 没有内置可拖拽滚动条 UI(只能滚轮/触控板滚动),
//!   也没有内置右键菜单——后者正是移除的目的(vendored 版本的内部右键菜单
//!   就是上面那个 panic 的根源)。
//! - 字号/行高按 `theme::terminal_font` 现算现用(每次 `view()` 都从全局
//!   `icon_size::scale()` 重新推导),不像 vendored 版本需要显式
//!   `set_font_size`/`set_layout_metrics` 再手动 `resync`——Ctrl ± 缩放
//!   天然生效,不需要专门的 "resync_editor_font_metrics" 步骤。

pub mod highlighter;

use iced_widget::core::widget::Id as WidgetId;
use iced_widget::core::{Color, Element, Length, Pixels, Point, Rectangle, mouse};
use iced_widget::text_editor::{self, Action};
use iced_widget::{canvas, row};

/// 行号 gutter 固定宽度(未乘全局 scale;`view()` 里按 `icon_size::scale()`
/// 再放大,与终端/图标缩放同步)。
const GUTTER_WIDTH: f32 = 44.0;

/// 组合了行号 gutter 的 `text_editor`。两处复用:`preview.rs` 只读文件预览、
/// `workspace.rs` 的 `EditSession` 可写编辑浮层——`read_only` 决定
/// [`Action::Edit`] 是否被过滤掉。
pub struct CodeView {
    id: WidgetId,
    content: text_editor::Content,
    /// gutter 镜像出来的滚动位置(行,含小数)——见模块文档"已知取舍"。
    scroll_lines: f32,
    read_only: bool,
    /// 语法 token(如 "rust"),见 `preview::extension_to_syntax`。
    token: String,
}

impl CodeView {
    pub fn new(text: &str, token: impl Into<String>, read_only: bool) -> Self {
        Self {
            id: WidgetId::unique(),
            content: text_editor::Content::with_text(text),
            scroll_lines: 0.0,
            read_only,
            token: token.into(),
        }
    }

    /// 当前完整文本内容(脏标记 = `text() != saved_content`)。
    pub fn text(&self) -> String {
        self.content.text()
    }

    /// 0-indexed `(line, column)`——供 `preview_context_from_editor_state`
    /// 组装 MCP `get_preview_context` 用,不依赖滚动镜像。
    pub fn cursor_position(&self) -> (usize, usize) {
        let cursor = self.content.cursor();
        (cursor.position.line, cursor.position.column)
    }

    pub fn has_selection(&self) -> bool {
        self.content.cursor().selection.is_some()
    }

    /// 选区的 `(start, end)`(均 0-indexed `(line, column)`,已按文档序排好,
    /// 与光标在选区哪一端无关)。没有选区时 `None`。
    pub fn selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        let cursor = self.content.cursor();
        cursor.selection.map(|anchor| {
            let a = (anchor.line, anchor.column);
            let b = (cursor.position.line, cursor.position.column);
            if a <= b { (a, b) } else { (b, a) }
        })
    }

    /// `view()` 里 `.id(...)` 挂的同一个 id——程序化聚焦(`operation::
    /// focusable::focus`)用,替代 vendored 版本的 `request_focus()`。
    pub fn focus_id(&self) -> WidgetId {
        self.id.clone()
    }

    /// 处理一次 `Action`:只读态过滤掉 [`Action::Edit`](能选中/复制/滚动,
    /// 改不了内容),滚动额外驱动 gutter 的镜像滚动位置(钳制到
    /// `[0, line_count()-1]`——已知在文件末尾附近可能与真实滚动位置漂移,
    /// 见模块文档)。
    pub fn perform(&mut self, action: Action) {
        if let Action::Scroll { lines } = action {
            let max_scroll = self.content.line_count().saturating_sub(1) as f32;
            self.scroll_lines = (self.scroll_lines + lines as f32).clamp(0.0, max_scroll);
        }
        if self.read_only && matches!(action, Action::Edit(_)) {
            return;
        }
        self.content.perform(action);
    }

    /// 组出 `[gutter, editor]` 一行,内层消息就是原始 `Action`——调用方按
    /// 原 `iced_code_editor::Message` 时代同样的手法 `.map(...)` 转发到自己
    /// 的顶层 `Message`(`Message::EditorEvent`/`editor_msg(tab_id, ev)` 等,
    /// 调用点不用改)。
    pub fn view<'a>(&'a self) -> Element<'a, Action, iced_widget::Theme, iced_renderer::Renderer> {
        let scale = byteui::theme::icon_size::scale();
        let font_size = crate::theme::terminal_font::size() * scale;
        let line_height_px = font_size * crate::theme::terminal_font::line_height_factor();

        let editor = text_editor::TextEditor::new(&self.content)
            .id(self.id.clone())
            .height(Length::Fill)
            .font(crate::fonts::code_font())
            .size(Pixels(font_size))
            .line_height(Pixels(line_height_px))
            .highlight_with::<highlighter::Highlighter>(
                highlighter::Settings {
                    token: self.token.clone(),
                },
                |highlight, _theme| highlight.to_format(),
            )
            .style(editor_style)
            .on_action(std::convert::identity);

        let gutter = canvas::Canvas::new(Gutter {
            line_count: self.content.line_count(),
            scroll_lines: self.scroll_lines,
            line_height: line_height_px,
        })
        .width(Length::Fixed(GUTTER_WIDTH * scale))
        .height(Length::Fill);

        row![gutter, editor].into()
    }
}

/// 编辑器 chrome 对齐到 ByteBoy2077 配色——官方 `text_editor::Style` 字段比
/// vendored 版本简单得多(没有 gutter/滚动条/右键菜单的概念,那些 UI 官方
/// widget 本来就不画),这里只覆盖它实际暴露的几项。
fn editor_style(_theme: &iced_widget::Theme, _status: text_editor::Status) -> text_editor::Style {
    use iced_widget::core::{Background, Border};
    let palette = byteui::theme::color::current();
    let cyan = palette.cyan;
    text_editor::Style {
        background: Background::Color(palette.bg),
        border: Border::default(),
        placeholder: palette.dim,
        value: palette.cream,
        selection: Color {
            r: cyan.r,
            g: cyan.g,
            b: cyan.b,
            a: 0.25,
        },
    }
}

/// 自建行号列:不跟 `text_editor` 共享滚动状态,靠应用层镜像的
/// `scroll_lines` 算每行该画在哪个 y 坐标(见模块文档"已知取舍")。
struct Gutter {
    line_count: usize,
    scroll_lines: f32,
    line_height: f32,
}

impl<Message> canvas::Program<Message> for Gutter {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_renderer::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        let first_line = self.scroll_lines.floor() as usize;
        let sub_pixel_offset = self.scroll_lines.fract() * self.line_height;
        let visible_rows = (bounds.height / self.line_height).ceil() as usize + 1;
        let color = byteui::theme::color::current().dim;
        let font = crate::fonts::code_font();

        for row in 0..visible_rows {
            let line_no = first_line + row;
            if line_no >= self.line_count {
                break;
            }

            let y = row as f32 * self.line_height - sub_pixel_offset;

            frame.fill_text(canvas::Text {
                content: format!("{:>4}", line_no + 1),
                position: Point::new(4.0, y + 2.0),
                color,
                size: Pixels(byteui::theme::font::body() as f32 * 0.85),
                font,
                ..canvas::Text::default()
            });
        }

        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_filters_edit_but_allows_move_and_scroll() {
        let mut view = CodeView::new("fn one() {}\nfn two() {}", "rust", true);
        let before = view.text();

        view.perform(Action::Edit(text_editor::Edit::Insert('x')));
        assert_eq!(view.text(), before, "只读态不应接受编辑动作");

        view.perform(Action::Move(text_editor::Motion::Right));
        // Move 不改内容,只验证不 panic、状态可继续操作。
        assert_eq!(view.text(), before);
    }

    #[test]
    fn writable_view_accepts_edits() {
        let mut view = CodeView::new("ab", "rust", false);
        view.perform(Action::Move(text_editor::Motion::DocumentEnd));
        view.perform(Action::Edit(text_editor::Edit::Insert('!')));
        assert_eq!(view.text(), "ab!");
    }

    #[test]
    fn cursor_position_and_selection_are_zero_indexed() {
        let mut view = CodeView::new("hello\nworld", "txt", false);
        assert_eq!(view.cursor_position(), (0, 0));
        assert!(!view.has_selection());
        assert_eq!(view.selection_range(), None);

        view.perform(Action::Move(text_editor::Motion::DocumentEnd));
        assert_eq!(view.cursor_position(), (1, 5));

        view.perform(Action::Select(text_editor::Motion::DocumentStart));
        assert!(view.has_selection());
        assert_eq!(view.selection_range(), Some(((0, 0), (1, 5))));
    }

    #[test]
    fn scroll_action_accumulates_and_clamps_gutter_offset() {
        let mut view = CodeView::new(&"line\n".repeat(10), "txt", true);
        assert_eq!(view.scroll_lines, 0.0);

        view.perform(Action::Scroll { lines: 3 });
        assert_eq!(view.scroll_lines, 3.0);

        // 钳制到 line_count()-1,不应超过。
        view.perform(Action::Scroll { lines: 100 });
        assert_eq!(view.scroll_lines, (view.content.line_count() - 1) as f32);

        view.perform(Action::Scroll { lines: -100 });
        assert_eq!(view.scroll_lines, 0.0);
    }
}
