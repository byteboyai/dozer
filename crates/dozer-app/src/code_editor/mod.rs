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

    /// 文件内搜索(文件预览原生 tab 的 Find):返回 `query` 在当前 buffer 里的
    /// 全部匹配,每个匹配给一对逻辑坐标 —— `.0` 是匹配起始、`.1` 是匹配**结束
    /// 边界**——调用层拿 `.0` 调 [`CodeView::move_cursor_to`] 落光标 / 定位。
    ///
    /// 语义约定(与 cosmic-text 光标一致):坐标 `(line, column)` 中 `line` 是
    /// buffer 逻辑行(由明文换行划分,不随 wrapping 变);`column` 是**该行内的
    /// 字节偏移**——cosmic 的 `Cursor { line, index }` 里 `index` 与
    /// `position.column` 直通(iced_graphics `move_to` 把 column 原样写入
    /// `set_cursor.index`,已核实)。匹配区间永不跨到换行字节之外,因此返回的
    /// 起止都落在真实字符边界。
    ///
    /// 匹配规则:**大小写 ASCII 折叠**的等长字节比较(`eq_ignore_ascii_case`)。
    /// 不先把文本整体 to_lowercase 再比 —— 那样的复制会改变非 ASCII 文本的字
    /// 节布局,导致回溯的坐标错位;这里只在候选与 query 等长时按字节 compare,
    /// A-Z 忽略大小写,非 ASCII 严格相等,坐标因此恒与 `Content` 字节布局对齐。
    /// 空 query 返回空表。
    ///
    /// 纯计算:不写 buffer、不移动光标。哪个匹配变"当前"是调用层 Find 栏状态。
    pub fn find_matches_all(&self, query: &str) -> Vec<((usize, usize), (usize, usize))> {
        if query.is_empty() {
            return Vec::new();
        }
        // 初次扫出全部匹配的 [start_byte, end_byte) 区间(在全串字节坐标上)。
        let text = self.content.text();
        let bytes = text.as_bytes();
        let q = query.as_bytes();
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        if q.len() <= bytes.len() {
            for i in 0..=(bytes.len() - q.len()) {
                if bytes[i..i + q.len()].eq_ignore_ascii_case(q) {
                    ranges.push((i, i + q.len()));
                }
            }
        }
        if ranges.is_empty() {
            return Vec::new();
        }
        // 每行字节起点:按 `\n` 字节切出每逻辑行的起始字节。Dozer 保存统一用
        // `\n`(Lf),`\r\n` 里的 `\r` 会原样保留在行文本字节内、不额外造空行;
        // 孤例 Cr/LfCr 分隔的文档极少见,坐标仅此一次以 \n 计,可接受。
        let mut line_starts = vec![0usize];
        for (i, b) in bytes.iter().enumerate() {
            if *b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        let locate = |abs: usize| -> (usize, usize) {
            // abs >= len 不会发生(abs 是文本字节内位置);等于某行起点 => 该行
            // (Ok);不在起点集且居两起点之间 => Err(插入点)-1 回到前一行;末尾行
            // (无尾随 \n)起点之后绝不越 len,Err = len => 钳到某实行。
            let line = match line_starts.binary_search(&abs) {
                Ok(l) => l,
                Err(l) => l.saturating_sub(1).min(line_starts.len().saturating_sub(1)),
            };
            (line, abs - line_starts[line])
        };
        ranges
            .into_iter()
            .map(|(s, e)| (locate(s), locate(e)))
            .collect()
    }

    /// 把光标移到某逻辑 `(line, column)`(字节列,语义见 [`find_matches_all`])——
    /// 调用层"跳到第 n 个匹配"的实际落点。只落光标不上选区。行号越界钳到末行;
    /// cosmic `set_cursor` 对超出该行字节长的 index 会安全钳到行尾附近,只读/可
    /// 写态通用。
    pub fn move_cursor_to(&mut self, (line, column): (usize, usize)) {
        use text_editor::{Cursor, Position};
        let last = self.content.line_count().saturating_sub(1);
        let line = line.min(last);
        self.content.move_to(Cursor {
            position: Position { line, column },
            selection: None,
        });
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

    #[test]
    fn find_matches_all_reports_line_col_with_ascii_fold() {
        let view = CodeView::new("hey foo Bar\nbaz\nfoo quux FOO", "txt", false);
        // "foo" 大小写 ASCII 折叠:命中第 0 行 "foo"(col4,但开头小写 foo 于 col4
        // 也被折叠——不进 "Bar")、第 2 行 "foo"(col0) 与 "FOO"(col9)。注意
        // 第 0 行还有一处小写 "foo"—不重复,共 3 处(0,0 的 "hey foo"之外没有)。
        let hits = view.find_matches_all("foo");
        assert_eq!(
            hits,
            vec![
                ((0, 4), (0, 7)),  // 行0 "foo"
                ((2, 0), (2, 3)),  // 行2 "foo"
                ((2, 9), (2, 12)), // 行2 "FOO"(col9 起)
            ]
        );
    }

    #[test]
    fn find_matches_all_handles_multiline_and_trailing_no_match() {
        let view = CodeView::new("ab\ncd\nef", "txt", false);
        assert_eq!(view.find_matches_all("zz"), Vec::new(), "无匹配返回空");
        // query 会跨两行?字面 query 含 \n 时以整串字节找:第 0 行 "ab\n" 越过
        // 换行在第 1 行继续才算 —— "ab\ncd" 中间是真换行字节。不特判,应命中
        // (0,0)->覆盖到 (1,2)= "ab\ncd" 结束于第 1 行 col2。
        let hits = view.find_matches_all("ab\ncd");
        assert_eq!(hits, vec![((0, 0), (1, 2))]);
    }

    #[test]
    fn find_matches_all_byte_columns_survive_multibyte_before_hit() {
        // 中文在 ASCII 前缀之前出现(每字 3 字节)时,列仍是字节偏移且精确。
        let view = CodeView::new("先例func\nfunc", "txt", false);
        // 第一行 "先例func":先例=6 字节 → func 从 col6 起(col6..10);
        // 第二行 func 从 col0 起(col0..4)。全部命中按行序返回。
        assert_eq!(
            view.find_matches_all("func"),
            vec![((0, 6), (0, 10)), ((1, 0), (1, 4))]
        );
    }

    #[test]
    fn move_cursor_to_lands_and_roundtrips() {
        let mut view = CodeView::new("a\nneedle here\nb", "txt", false);
        view.move_cursor_to((1, 0));
        assert_eq!(view.cursor_position(), (1, 0), "光标应落在第 1 行第 0 列");
        // 跳到"needle"起始(4)并可读回(position.column == 字节列)。
        view.move_cursor_to((1, 4));
        assert_eq!(view.cursor_position().0, 1);
        // 越界行钳到末行。
        view.move_cursor_to((99, 1));
        assert_eq!(view.cursor_position().0, 2, "超出末行的 line 应钳到末行");
    }
}
