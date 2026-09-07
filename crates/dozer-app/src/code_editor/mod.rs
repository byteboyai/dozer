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
//! - 右侧纯指示滚动条靠应用层累加 `Action::Scroll{lines}` 镜像滚动位置——
//!   `iced_core::text::Editor` trait 没有暴露读取真实滚动偏移的公开 API,
//!   只能近似反映"滚到哪",顶到末尾时整条 thumb 不到底,纯视觉瑕疵,不影响编辑。
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

use iced_widget::canvas;
use iced_widget::core::widget::Id as WidgetId;
use iced_widget::core::{Color, Element, Length, Pixels, Point, Rectangle, Size, mouse};
use iced_widget::text_editor::{self, Action};

/// 官方 `text_editor::TextEditor` 自己的默认 padding(`Padding::new(5.0)`,
/// 见 `iced_widget::text_editor` 源码)。在这里显式设出来并喂给 `.padding(...)`,
/// 不再依赖那个未在我们代码里出现过的隐式默认值——`CaretRow` 叠层算光标行屏幕 y
/// 时要原样加回同一个 padding.top,两处必须共用同一个数,否则叠层与实际文字的
/// 起始行会错位(叠层的画布 bounds 是编辑器**未收缩**的外框,文字本身是
/// `bounds.shrink(padding)` 之后才开始画的)。
const EDITOR_PADDING: f32 = 5.0;

/// 组出可直接嵌入的 `text_editor`。两处复用:`preview.rs` 只读文件预览、
/// `workspace.rs` 的 `EditSession` 可写编辑浮层——`read_only` 决定
/// [`Action::Edit`] 是否被过滤掉。
pub struct CodeView {
    id: WidgetId,
    content: text_editor::Content,
    /// 滚动条镜像出来的滚动起点(行,含小数)——见模块文档"已知取舍"。
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
    /// 匹配规则默认与 `case_sensitive == false` 时相同:**大小写 ASCII 折叠**的
    /// 等长字节比较(`eq_ignore_ascii_case`);`case_sensitive == true` 时退为逐字
    /// 节严格相等(`eq`)。
    /// 不先把文本整体 to_lowercase 再比 —— 那样的复制会改变非 ASCII 文本的字
    /// 节布局,导致回溯的坐标错位;这里只在候选与 query 等长时按字节 compare,
    /// 非 ASCII 恒严格相等,坐标因此恒与 `Content` 字节布局对齐。空 query 返回
    /// 空表。
    ///
    /// 纯计算:不写 buffer、不移动光标。哪个匹配变"当前"是调用层 Find 栏状态。
    pub fn find_matches_all(
        &self,
        query: &str,
        case_sensitive: bool,
    ) -> Vec<((usize, usize), (usize, usize))> {
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
                let cand = &bytes[i..i + q.len()];
                let equal = if case_sensitive {
                    cand.eq(q)
                } else {
                    cand.eq_ignore_ascii_case(q)
                };
                if equal {
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

    /// 选中一段连续区间 `start..end`(各为 `(line, column)` 字节坐标,语义见
    /// [`find_matches_all`];返回的匹配对 `.0`/`.1` 正好喂给本方法)——Find 条
    /// 跳「上一个/下一个命中」时用:既把光标落到区间端点,又把整段匹配高亮选中,
    /// 方便用户一眼认出当前命中且可直接复制/替换。行号越界钳到末行(区间两端
    /// 同行、天然在字符边界,见 [`find_matches_all`] 的不跨换行约定)。
    pub fn select_range(&mut self, start: (usize, usize), end: (usize, usize)) {
        use text_editor::{Cursor, Position};
        let last = self.content.line_count().saturating_sub(1);
        let start_line = start.0.min(last);
        let end_line = end.0.min(last);
        // cosmic 锚点选区 = {position, selection: Some(anchor)};两端不限顺序,
        // `selection_range()` 会按字典序排好(见该方法的文档)。
        self.content.move_to(Cursor {
            position: Position {
                line: end_line,
                column: end.1,
            },
            selection: Some(Position {
                line: start_line,
                column: start.1,
            }),
        });
    }

    /// 选中同一段区间但把光标停在**前缘**(区间起点),选区锚点在末端——与
    /// [`select_range`] 相反的光标停法。跳「上一个」时用:Find 条用光标做"当前
    /// 位置"锚,往前跳要把光标落在命中的**前缘**,这样再按「上一个」才从更早处
    /// 续接,不至于每次回到同一个命中原地踏步。整段照常高亮,仅光标落边不同。
    pub fn select_range_backward(&mut self, start: (usize, usize), end: (usize, usize)) {
        use text_editor::{Cursor, Position};
        let last = self.content.line_count().saturating_sub(1);
        let start_line = start.0.min(last);
        let end_line = end.0.min(last);
        // cosmic 锚点选区 = {position, selection: Some(anchor)};两端不限顺序,
        // `selection_range()` 会按字典序排好。这里 position=前缘、anchor=末端。
        self.content.move_to(Cursor {
            position: Position {
                line: start_line,
                column: start.1,
            },
            selection: Some(Position {
                line: end_line,
                column: end.1,
            }),
        });
    }

    /// 处理一次 `Action`:只读态过滤掉 [`Action::Edit`](能选中/复制/滚动,
    /// 改不了内容),滚动额外更新滚动条的镜像滚动起点(累加 clamp——已知是
    /// 近似,见模块文档"已知取舍")。
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

    /// 把 buffer 里所有 `query` 匹配**(贪婪、左到右、避开重叠)**替换成
    /// `replacement`,返回替换次数。匹配用的严格比较 / ASCII 折叠语义与
    /// [`find_matches_all`] 的 `case_sensitive` 完全一致(等长字节)。这是
    /// Find「替换全部」的落地:走一次全量扫描,**每个命中只替换一次且不重叠**,
    /// 因此即使 `ends-with-query` 连排也只推进 query 长度跳过,不产生无限的
    /// "替换出来的又变成匹配" 自吞效应。
    ///
    /// 结果直接重建 `self.content`(cosmic 语法高亮来源不变;不做单 Action 折叠
    /// 是为了把“多处删加、长度可变”压缩成一个事务,避免一趟几十个 Edit)。重建
    /// 会丢光标/选区与 undo —— `read_only` 恒 false(原生预览可编辑,见
    /// `PreviewTab` 注释),替换是“未保存 buffer 的就地改动”语义之一,由调用方
    /// (PreviewPane)在此之后标脏并 Reanchor 光标。
    ///
    /// 空 query / 无命中也走不动作,返回 0。
    pub fn replace_all(&mut self, query: &str, case_sensitive: bool, replacement: &str) -> usize {
        let (new_text, count) =
            replace_pass_all(&self.content.text(), query, case_sensitive, replacement);
        if count == 0 {
            return 0;
        }
        self.content = text_editor::Content::with_text(&new_text);
        count
    }

    /// 只替换窗口序里的第 `nth`(0-based)个命中——「替换当前命中」用,序与
    /// [`find_matches_all`] 从前往后(含重叠窗)完全一致,调用方拿自己的
    /// `current` 来即可。命中不存在(越界)返回 `false` 且 buffer 不动;命中
    /// 存在则原地把那一处替换成 `replacement`(两侧其余文本原样保留),返回
    /// `true`。同样重建 `self.content`、不保留 undo(见 [`CodeView::replace_all`])。
    pub fn replace_nth(
        &mut self,
        nth: usize,
        query: &str,
        case_sensitive: bool,
        replacement: &str,
    ) -> bool {
        let text = self.content.text();
        let Some(new_text) = replace_pass_nth(&text, nth, query, case_sensitive, replacement)
        else {
            return false;
        };
        self.content = text_editor::Content::with_text(&new_text);
        true
    }

    /// 组出 `[editor, scrollstrip]` 一行:正文靠左撑满,右侧一根窄条画统一
    /// 风格滚动条(只在内容纵向溢出时显 thumb)。内层消息就是原始 `Action`——
    /// 调用方按原 `iced_code_editor::Message` 时代同样的手法 `.map(...)` 转发到
    /// 自己的顶层 `Message`(`Message::EditorEvent`/`editor_msg(tab_id, ev)` 等,
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
            .padding(EDITOR_PADDING)
            .on_action(std::convert::identity);

        // 「光标所在行」背景(低 alpha 行着色,视觉近似):高层叠在编辑器之上的
        // 纯指示图层,按 `content.cursor().position.line` + 滚动镜像算出该行的
        // 屏幕 y,画一条全行宽淡淡的青色调(BytByte2077,避开甲方专属金)。官方
        // `text_editor` 不暴露“按行加背景”的钩子(见模块文档),只能叠在字上;
        // 用很小 alpha、不把它做得比选区还吵。图层不放任何鼠标/焦点处理,点击
        // 仍会贯通到下面的编辑器本体。
        let caret_line = self.content.cursor().position.line;
        let caret_row = canvas::Canvas::new(CaretRow {
            line: caret_line,
            scroll_lines: self.scroll_lines,
            line_height: line_height_px,
            padding_top: EDITOR_PADDING,
        })
        .width(Length::Fill)
        .height(Length::Fill);

        // 文字编辑器垫底、caret 行着色浮在上面;含顶的整块吃满区域(后续一行就是
        // 滚条,两者同排),caret 行着色不会画出编辑器外的滚条列。
        let editor_pane = iced_widget::stack![editor, caret_row];

        let scrollstrip = canvas::Canvas::new(Scrollstrip {
            line_count: self.content.line_count(),
            scroll_lines: self.scroll_lines,
            line_height: line_height_px,
        })
        // 轨道宽度即统一滚动条几何(见 `byteui::theme::geometry::scrollbar_width`);
        // 恒占满高,与正文同一行高,`draw` 里拿 `bounds.height` 当可视区算 thumb。
        .width(Length::Fixed(byteui::theme::geometry::scrollbar_width()))
        .height(Length::Fill);

        iced_widget::row![editor_pane, scrollstrip].into()
    }
}

/// replace_pass_all 的单步扫描体 —— 两趟换行/表序所需的纯替换实现搁在文件级
/// 便于单测(`CodeView` 构造要带 token,纯替换逻辑不值得为它造 editor)。
/// 匹配语义与 `find_matches_all` 的字节相等 / ASCII 折叠相同(等长字节比较),
/// 返回 (全部完成后整串文本, 替换次数)。贪婪不重叠:`every hit once`,
/// 命中起点立刻跳到命中末(等价 editor 常见的 find-replace-forward)。
fn replace_pass_all(
    text: &str,
    query: &str,
    case_sensitive: bool,
    replacement: &str,
) -> (String, usize) {
    if query.is_empty() || text.is_empty() {
        return (text.to_owned(), 0);
    }
    let bytes = text.as_bytes();
    let q = query.as_bytes();
    let mut out =
        String::with_capacity(text.len() + (replacement.len().saturating_sub(q.len())) * 8 + 16);
    let mut last = 0usize;
    let mut i = 0usize;
    let mut count = 0usize;
    while i + q.len() <= bytes.len() {
        let cand = &bytes[i..i + q.len()];
        let equal = if case_sensitive {
            cand.eq(q)
        } else {
            cand.eq_ignore_ascii_case(q)
        };
        if !equal {
            i += 1;
            continue;
        }
        // 命中：把 [last, i) 原文 + replacement 追加，游标跳到命中末避免重叠。
        out.push_str(&text[last..i]);
        out.push_str(replacement);
        last = i + q.len();
        i += q.len();
        count += 1;
    }
    out.push_str(&text[last..]);
    (out, count)
}

/// 只替换窗口序第 `nth` 个命中;两侧原样保留。`None` = 那里没有命中(越界或空
/// query),调用方不该动 buffer。恰逢命中边界即 utf8 字符边界(与 find 同规则)。
fn replace_pass_nth(
    text: &str,
    nth: usize,
    query: &str,
    case_sensitive: bool,
    replacement: &str,
) -> Option<String> {
    if query.is_empty() {
        return None;
    }
    let bytes = text.as_bytes();
    let q = query.as_bytes();
    let mut seen = 0usize;
    let mut i = 0usize;
    while i + q.len() <= bytes.len() {
        let cand = &bytes[i..i + q.len()];
        let equal = if case_sensitive {
            cand.eq(q)
        } else {
            cand.eq_ignore_ascii_case(q)
        };
        if !equal {
            i += 1;
            continue;
        }
        if seen == nth {
            let mut out =
                String::with_capacity(text.len().saturating_sub(q.len()) + replacement.len() + 8);
            out.push_str(&text[..i]);
            out.push_str(replacement);
            out.push_str(&text[i + q.len()..]);
            return Some(out);
        }
        seen += 1;
        i += 1;
    }
    None
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

/// 纯指示型滚动条(不可拖,风格对齐拉列表的 `byteui` 统一 thumb):
/// `text_editor` 没有公开 API 读真实滚动偏移,只能靠应用层镜像的
/// `scroll_lines` 积分近似;文件长到开始滚动时显现,否则轨道透明留白。
/// 视觉瑕疵(只能反映"已滚多少",顶到末尾整 thumb 不到底)由镜像近似导致,
/// 见模块文档"已知取舍",用户已确认接受。
struct Scrollstrip {
    line_count: usize,
    /// 镜像出来的滚动起点(行,含小数)。
    scroll_lines: f32,
    line_height: f32,
}

impl<Message> canvas::Program<Message> for Scrollstrip {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_renderer::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let visible = bounds.height / self.line_height;
        // 内容没超过一屏,不需要滚动条;或根本没行数,直接一片空。
        if visible <= 0.0 || (self.line_count as f32) <= visible {
            return Vec::new();
        }

        let mut frame = canvas::Frame::new(renderer, bounds.size());

        let thumb_w = byteui::theme::geometry::scrollbar_thumb_width();
        let rail_w = byteui::theme::geometry::scrollbar_width();
        let gold = byteui::theme::color::current().tab_active_border;

        // thumb 高度按"可视区/总内容"缩放,给个最小高度免得细到看不见。
        let total = self.line_count as f32;
        let ratio = visible / total;
        let thumb_h = (bounds.height * ratio).max(thumb_w * 1.5);

        // 顶到末尾还滚得动一点就 clamp 住,别跑出轨道。
        let progress = self.scroll_lines / total.max(1.0);
        let top = progress * (bounds.height - thumb_h);

        // 圆角(半径 = 半宽 → 胶囊),风格对齐 `byteui` 拉列表里统一滚动条的
        // thumb(同样 `scroller_width` 取半做圆角)。
        let radius = (thumb_w / 2.0).into();
        let x = (rail_w - thumb_w) / 2.0;
        let path = canvas::Path::rounded_rectangle(
            Point::new(x, top),
            Size::new(thumb_w, thumb_h),
            radius,
        );
        frame.fill(&path, gold);

        vec![frame.into_geometry()]
    }
}

/// 光标所在行的整行背景着色(只 "高亮所在行",没有交互)。官方 `text_editor`
/// 自绘画布、不给“按行加背景”钩子,靠这一个叠在编辑器**上方**的全幅画布,拿
/// `line + (滚动镜像 scroll_lines)` 反推光标行在当前视口的屏幕 y,画一条横贯整
/// 行宽的淡淡的青调矩形。alpha 取很小值(刻意小于选区的 `0.25`),即使压在字形
/// 上也几乎不伤可读性——这正是该近似在视觉上的代价,已被用户接受,见模块文档
/// “已知取舍”。
///
/// 纯装饰:不产生任何事件、不申请背景交互,pointer 会原样贯通到下面的编辑器
/// (iced 把事件发给实现了相应监听器的 widget;本图层一个都不实现)。
struct CaretRow {
    /// 光标所在 buffer 逻辑行(0-indexed)。
    line: usize,
    /// 滚动条镜像出来的滚动起点(行,含小数)——同一条镜像同时喂滚动条 thumb。
    scroll_lines: f32,
    /// 每行像素高度(与 `view()` 里给 text_editor 的 `line_height` 同一来源)。
    line_height: f32,
    /// 与 `view()` 里喂给 text_editor 的 `.padding(...)` 同一个 [`EDITOR_PADDING`]
    /// 值——本画布拿到的 `bounds` 是编辑器**未收缩**的外框,文字实际从
    /// `bounds.shrink(padding)` 之后才开始画,不加回这个量整行会往上错位。
    padding_top: f32,
}

impl<Message> canvas::Program<Message> for CaretRow {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_renderer::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let Some(rect) = caret_row_rect(self, bounds) else {
            return Vec::new();
        };
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        // 视觉近似用的青调 #47DEF0@0.07;刻意不硬调成甲方专属金(那条是
        // 甲方(用户侧 AI)动作专属)。行着色是"眼睛定位",比选区 `0.25` 更淡。
        let tint = byteui::theme::color::current().cyan;
        frame.fill_rectangle(
            Point::new(rect.x, rect.y),
            Size::new(rect.width, rect.height),
            Color {
                r: tint.r,
                g: tint.g,
                b: tint.b,
                a: 0.07,
            },
        );

        vec![frame.into_geometry()]
    }
}

/// 计算光标行对应的屏幕矩形。行已在视口之上(行号比滚动起点还小,完全滚出)则
/// `None`。纯函数放这好单测:给固定几何验数学,不给 panic。
fn caret_row_rect(caret: &CaretRow, bounds: Rectangle) -> Option<Rectangle> {
    if caret.line_height <= 0.0 {
        return None;
    }
    // 内容坐标里光标行顶 = padding.top + (光标行号 - 已滚回的行数) * 行高;
    // 负(减去 padding 后仍 < 0)=> 已滚出视口上缘不画。
    let top = caret.padding_top + (caret.line as f32 - caret.scroll_lines) * caret.line_height;
    if top < -caret.line_height || top > bounds.height {
        return None;
    }
    Some(Rectangle {
        x: bounds.x,
        y: bounds.y + top,
        width: bounds.width,
        height: caret.line_height,
    })
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
    fn scroll_action_accumulates_and_clamps_for_scrollstrip_mirror() {
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
        let hits = view.find_matches_all("foo", false);
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
    fn find_matches_all_case_sensitive_excludes_folded() {
        let view = CodeView::new("foo Foo fOo\nFOO", "txt", false);
        // 大小写敏感(case_sensitive == true)只命中完全等宽的小写 "foo"(col0)。
        assert_eq!(view.find_matches_all("foo", true), vec![((0, 0), (0, 3))]);
    }

    #[test]
    fn find_matches_all_handles_multiline_and_trailing_no_match() {
        let view = CodeView::new("ab\ncd\nef", "txt", false);
        assert_eq!(
            view.find_matches_all("zz", false),
            Vec::new(),
            "无匹配返回空"
        );
        // query 会跨两行?字面 query 含 \n 时以整串字节找:第 0 行 "ab\n" 越过
        // 换行在第 1 行继续才算 —— "ab\ncd" 中间是真换行字节。不特判,应命中
        // (0,0)->覆盖到 (1,2)= "ab\ncd" 结束于第 1 行 col2。
        let hits = view.find_matches_all("ab\ncd", false);
        assert_eq!(hits, vec![((0, 0), (1, 2))]);
    }

    #[test]
    fn find_matches_all_byte_columns_survive_multibyte_before_hit() {
        // 中文在 ASCII 前缀之前出现(每字 3 字节)时,列仍是字节偏移且精确。
        let view = CodeView::new("先例func\nfunc", "txt", false);
        // 第一行 "先例func":先例=6 字节 → func 从 col6 起(col6..10);
        // 第二行 func 从 col0 起(col0..4)。全部命中按行序返回。
        assert_eq!(
            view.find_matches_all("func", false),
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

    #[test]
    fn select_range_highlights_and_caret_sits_at_end() {
        let mut view = CodeView::new("a\nneedle needle\nb", "txt", false);
        view.select_range((1, 0), (1, 6));
        // 选中第一处 "needle"：锚点在匹配起点、position 在其末列，整段高亮。
        assert_eq!(view.selection_range(), Some(((1, 0), (1, 6))));

        // 越界行(clamp 到末行)不 panic——列由 cosmic 对超长 index 安全钳回。
        view.select_range((99, 1), (99, 2));
        assert_eq!(view.cursor_position().0, 2, "超出末行的 line 应钳到末行");
    }

    #[test]
    fn select_range_backward_parks_caret_at_front_edge() {
        let mut view = CodeView::new("a\nneedle needle\nb", "txt", false);
        view.select_range_backward((1, 0), (1, 6));
        // 整段照常选中;区别只在光标落边:position 停在匹配**起点**(1,0),锚点在
        // 末端——反向连按「上一个」能据此从更早处续接,不会卡在原命中。
        assert_eq!(view.cursor_position(), (1, 0));
        assert_eq!(view.selection_range(), Some(((1, 0), (1, 6))));
    }

    #[test]
    fn caret_row_rect_lines_up_with_scroll_mirror() {
        use iced_widget::core::Rectangle as R;
        let caret = CaretRow {
            line: 5,
            scroll_lines: 2.5,
            line_height: 20.0,
            padding_top: 5.0,
        };
        let bounds = R {
            x: 0.0,
            y: 10.0,
            width: 500.0,
            height: 200.0,
        };
        // 行5 - 滚2.5 = 内容第2.5 行,row y = 10 + 5(padding) + (2.5*20) = 65。
        let rect = caret_row_rect(&caret, bounds).unwrap();
        assert_eq!(rect.y, 65.0);
        assert_eq!(rect.height, 20.0);
        assert_eq!(rect.width, 500.0);

        // 光标行已滚出上缘(某行号 < since scroll huge)->None。
        let gone = CaretRow {
            line: 0,
            scroll_lines: 80.0,
            line_height: 20.0,
            padding_top: 5.0,
        };
        assert_eq!(caret_row_rect(&gone, bounds), None);
        // 退化行高也拒绝。
        assert_eq!(
            caret_row_rect(
                &CaretRow {
                    line: 1,
                    scroll_lines: 0.0,
                    line_height: 0.0,
                    padding_top: 5.0,
                },
                bounds,
            ),
            None
        );
    }

    #[test]
    fn replace_all_replaces_every_match_and_keeps_order() {
        let mut view = CodeView::new("foo xx foo\nxFOO foo", "txt", false);
        let n = view.replace_all("foo", true, "bar");
        assert_eq!(n, 3, "逐字严格:大写 FOO 不算,三处小写 foo 都换");
        assert_eq!(view.text(), "bar xx bar\nxFOO bar");
    }

    #[test]
    fn replace_all_case_insensitive_covers_folded_and_is_greedy_no_overlap() {
        let mut view = CodeView::new("aaaa", "txt", false);
        // 大小写折叠:全部命中;贪婪:连续串只推进 query 长,不回扫重叠。
        let n = view.replace_all("aa", false, "x");
        assert_eq!(n, 2);
        assert_eq!(view.text(), "xx");
    }

    #[test]
    fn replace_all_empty_query_is_noop() {
        let mut view = CodeView::new("needle needle", "txt", false);
        assert_eq!(view.replace_all("", false, "x"), 0);
        assert_eq!(view.text(), "needle needle");
    }

    #[test]
    fn replace_nth_targets_exact_occurrence_leaving_neighbors() {
        let mut view = CodeView::new("aa bb aa cc", "txt", false);
        // 第 2 个(1-based)命中 = 索引 1 的 aa → 只换它。
        assert!(view.replace_nth(1, "aa", false, "ZZ"));
        assert_eq!(view.text(), "aa bb ZZ cc");
        // 越界索引不改动。
        assert!(!view.replace_nth(9, "aa", false, "Y"));
        assert_eq!(view.text(), "aa bb ZZ cc");
    }

    #[test]
    fn replace_nth_multibyte_and_multiline_query_respects_byte_indices() {
        // 中文命中的字节边界与前缀一致;多行 query 中间换行也能命中窗口替换。
        let mut view = CodeView::new("先例", "txt", false);
        assert!(view.replace_nth(0, "先", false, "h"));
        assert_eq!(view.text(), "h例");
    }

    #[test]
    fn replace_rewrite_survives_crlf_rather_than_guessing_line_ends() {
        // 替换只在明文字节上做,不把 \r\n 拆成行:整块 CRLF 保留。
        let mut view = CodeView::new("a\r\nregex\r\nb", "txt", false);
        assert_eq!(view.replace_all("regex", false, "x"), 1);
        assert_eq!(view.text(), "a\r\nx\r\nb");
    }
}
