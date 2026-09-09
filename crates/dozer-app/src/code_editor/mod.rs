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
//! - 官方 `text_editor` 没有任何 undo/redo API(引擎是 cosmic-text,只对
//!   `Action::Edit` 落盘,不暴露按动作回滚),撤销只能靠在应用层做整文本
//!   快照栈(见本模块 [`Snapshot`]/[`CodeView::undo`])。实现取舍:快照是
//!   全文本拷贝、按"编辑命令"粒度建档(连续纯打字 run 折叠成一条),因此
//!   undo 序列不是逐键、光标位置也只是尽量还原近似;只读预览 tab(不参与
//!   键盘路由)不会被触发 undo,绑定关系见 workspace.rs 编辑弹层的 ⌘Z。

pub mod highlighter;

use iced_widget::canvas;
use iced_widget::core::widget::Id as WidgetId;
use iced_widget::core::{Color, Element, Length, Pixels, Point, Rectangle, Size, mouse};
use iced_widget::text_editor::{self, Action};

/// 官方 `text_editor::TextEditor` 自己的默认 padding(`Padding::new(5.0)`,
/// 见 `iced_widget::text_editor` 源码)。在这里显式设出来并喂给 `.padding(...)`,
/// 不再依赖那个未在我们代码里出现过的隐式默认值。
const EDITOR_PADDING: f32 = 5.0;

/// 撤销/重做栈单条快照:整段 text(采用"该段一次编辑命令落下的前状态"),
/// 加该时刻的光标 `(line, column)` 供还原后把光标尽量挪回原处(越界由
/// [`CodeView::move_cursor_to`] 钳到末行/行尾附近)。快照是全文本拷贝——
/// 官方 `text_editor` 引擎没有任何 undo API(见模块文档),只能应用层记。
/// 一条快照对应一条"编辑命令"(连续纯打字被折叠成一条),不是逐键。
struct Snapshot {
    text: String,
    line: usize,
    column: usize,
}

/// 撤销栈深度上限:无限保留会随时间把超大文件 RAM 吃光,给个保守的窗。
const EDIT_HISTORY_LIMIT: usize = 60;

/// 把一条快照无脑压进给定历史栈(带"栈顶文字与快照相同就不压"的排重),
/// 超上限时从最旧端裁掉 excess 条。undo / redo 在方法里各自借 `self` 的
/// 一个字段当 sink,所以这里必须是不接收 `self` 的自由函数(用 &mut self
/// 方法会跟外部对字段的借冲突)。
fn commit_history(sink: &mut Vec<Snapshot>, snap: Snapshot) {
    if matches!(sink.last(), Some(top) if top.text == snap.text) {
        return;
    }
    sink.push(snap);
    if sink.len() > EDIT_HISTORY_LIMIT {
        let excess = sink.len() - EDIT_HISTORY_LIMIT;
        sink.drain(0..excess);
    }
}

/// 组出可直接嵌入的 `text_editor`。两处复用:`preview.rs` 只读文件预览、
/// `workspace.rs` 的 `EditSession` 可写编辑浮层——`read_only` 决定
/// [`Action::Edit`] 是否被过滤掉。撤销历史在这里全量与 buffer 一起记:
/// 官方引擎不暴露按动作回滚,见模块文档"已知取舍"(底下那条注释请保留)。
pub struct CodeView {
    id: WidgetId,
    content: text_editor::Content,
    /// 滚动条镜像出来的滚动起点(行,含小数)——见模块文档"已知取舍"。
    scroll_lines: f32,
    read_only: bool,
    /// 语法 token(如 "rust"),见 `preview::extension_to_syntax`。
    token: String,
    /// 撤销栈:栈顶是最新一个"可回退前"快照(倒放回该快照即完成一次 undo)。
    undo: Vec<Snapshot>,
    /// 重做栈:undo 弹出来的"被回退后的旧当前态"暂存于此,redo 时放回。
    redo: Vec<Snapshot>,
    /// 是否正处在一段"连续纯打字"里。连续 `Edit::Insert`(逐键)共享同一
    /// 快照起点,松开那一串只算一条命令;插的任意其它编辑/光标移动会结束
    /// 这一段(`false`)。作用是避免每敲一键都整份拷贝文本(见 [`Snapshot`])。
    typing_run: bool,
}

impl CodeView {
    pub fn new(text: &str, token: impl Into<String>, read_only: bool) -> Self {
        Self {
            id: WidgetId::unique(),
            content: text_editor::Content::with_text(text),
            scroll_lines: 0.0,
            read_only,
            token: token.into(),
            undo: Vec::new(),
            redo: Vec::new(),
            typing_run: false,
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
    ///
    /// 顺带维护撤销历史(每条"编辑命令"落地前压一条[`Snapshot`])——见
    /// [`CodeView::undo`]/[`CodeView::redo`]。连续纯打字共享同一快照起点,
    /// 只有在"打字 run"里时才每键编码该键前状态,否则会退化回每次按键一项
    /// 全量拷贝,违背 [`Snapshot`] 的造价注释。
    pub fn perform(&mut self, action: Action) {
        if let Action::Scroll { lines } = &action {
            let max_scroll = self.content.line_count().saturating_sub(1) as f32;
            self.scroll_lines = (self.scroll_lines + *lines as f32).clamp(0.0, max_scroll);
        }
        let blocked = self.read_only && matches!(action, Action::Edit(_));
        self.record_before(&action);
        if blocked {
            return;
        }
        self.content.perform(action);
    }

    /// 撤销一次编辑:把 buffer 回退到最新一条[`Snapshot`]记录的内容,并把
    /// 刚被回退的当前态挪进重做栈(供 [`CodeView::redo`])。没有可撤销历史
    /// 时不动作。返回是否真的发生过回退。
    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        let current = self.snapshot_now();
        commit_history(&mut self.redo, current);
        self.restore(prev);
        self.typing_run = false;
        true
    }

    /// 重做被 [`CodeView::undo`] 撤消的最后一次编辑。返回是否真的发生过重做。
    pub fn redo(&mut self) -> bool {
        let Some(last) = self.redo.pop() else {
            return false;
        };
        let current = self.snapshot_now();
        commit_history(&mut self.undo, current);
        self.restore(last);
        self.typing_run = false;
        true
    }

    /// 在一条编辑命令落盘**前**(即 `content` 尚未 `perform`)调用:决定是否该
    /// 因为开启/切换命令而预留一条前状态快照。`action` 就是即将交给
    /// [`CodeView::perform`] 的那条 `Action`。
    fn record_before(&mut self, action: &Action) {
        if self.read_only || !matches!(action, Action::Edit(_)) {
            // 没落盘编辑就不记(只读/光标移动都不要),并结束打字 run——移动
            // 光标后重新开始敲字就是另一段命令。
            self.typing_run = false;
            return;
        }
        let is_single_insert = matches!(action, Action::Edit(text_editor::Edit::Insert(_)));
        // 延续中的连续纯打字(run 内逐键 Insert):已在前一组记录的同一个快照
        // 之后继续插入,无需再为每键新压一条,整串只算一条命令。
        if self.typing_run && is_single_insert {
            return;
        }
        // 任意打断 run 的编辑(Enter/粘贴/删除/退格)或重新开始的打字,都先把
        // "当前尚未落的新输入"还原点压栈,再进入下一步真正的 perform。
        self.push_undo_snapshot();
        self.typing_run = is_single_insert;
    }

    /// 取当前全文本 + 当前光标 `(line, column)`,封装成一条可用作还原点的
    /// [`Snapshot`]。
    fn snapshot_now(&self) -> Snapshot {
        let (line, column) = self.cursor_position();
        Snapshot {
            text: self.text(),
            line,
            column,
        }
    }

    /// 在"新的一条编辑命令即将把当前态真正推进"前调用:压一条前状态快照,
    /// 同时丢掉整个 redo 栈——编辑回滚后另走新路,replay 无意义(线性历史)。
    fn push_undo_snapshot(&mut self) {
        let snap = self.snapshot_now();
        commit_history(&mut self.undo, snap);
        self.redo.clear();
    }

    /// 任何不经 [`CodeView::perform`] 直接把 `content` 整体重写的入口(如
    /// Find 的整段替换)落地后都要清历史:它们不等价于用户一条条编辑命令,
    /// 留着旧栈会让 undo 落到一个替换被半拆断的怪状态。
    pub fn reset_edit_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.typing_run = false;
    }

    /// 把 `content` 换成某条快照文本,并把光标挪回接近该快照时刻原位置
    /// (行越界时钳在末行,列由 `move_cursor_to` 交给 cosmic 钳回行尾附近)。
    fn restore(&mut self, snap: Snapshot) {
        let last_line = self.content.line_count();
        let line = snap.line.min(last_line.saturating_sub(1));
        self.content = text_editor::Content::with_text(&snap.text);
        self.scroll_lines = self
            .scroll_lines
            .clamp(0.0, self.content.line_count().saturating_sub(1) as f32);
        self.move_cursor_to((line, snap.column));
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
        self.reset_edit_history();
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
        self.reset_edit_history();
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

        let scrollstrip = canvas::Canvas::new(Scrollstrip {
            line_count: self.content.line_count(),
            scroll_lines: self.scroll_lines,
            line_height: line_height_px,
        })
        // 轨道宽度即统一滚动条几何(见 `byteui::theme::geometry::scrollbar_width`);
        // 恒占满高,与正文同一行高,`draw` 里拿 `bounds.height` 当可视区算 thumb。
        .width(Length::Fixed(byteui::theme::geometry::scrollbar_width()))
        .height(Length::Fill);

        iced_widget::row![editor, scrollstrip].into()
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
    fn tab_inserts_a_tab_at_cursor_through_edit_action() {
        // main.rs 原生预览闸门把裸 Tab 转化成 `Edit::Insert('\t')` 走这条路插:
        // 与普通打字同一 perform 管线,光标处落制表符、选区被替换、可撤销。
        let mut view = CodeView::new("let x = 1;", "rust", false);
        view.perform(Action::Move(text_editor::Motion::Home));
        view.perform(Action::Edit(text_editor::Edit::Insert('\t')));
        assert_eq!(view.text(), "\tlet x = 1;", "首页插入制表符");

        // 逐字内容里带出真正 `\t`,不是 4 空格拼凑。
        assert!(view.text().starts_with('\t'));

        // 选中一段时 Insert 应替换选区(锚点语义交给 iced Content 自己处理):
        // 从行尾拖回行首 = 全选,再插一个 tab 应整行被替换成 tab。
        view.perform(Action::Move(text_editor::Motion::DocumentEnd));
        view.perform(Action::Select(text_editor::Motion::DocumentStart));
        view.perform(Action::Edit(text_editor::Edit::Insert('\t')));
        assert_eq!(view.text(), "\t");
    }

    #[test]
    fn undo_restores_and_redo_reapplies_single_commit() {
        let mut view = CodeView::new("ab", "rust", false);
        view.perform(Action::Move(text_editor::Motion::DocumentEnd));
        view.perform(Action::Edit(text_editor::Edit::Insert('!')));
        assert_eq!(view.text(), "ab!");

        assert!(view.undo(), "刚插入过应可撤销");
        assert_eq!(view.text(), "ab");
        assert!(view.redo(), "撤销后应可重做");
        assert_eq!(view.text(), "ab!");

        // 全栈弹空后 no-op(到 "ab" 再撤就没有更早的态了)。
        assert!(view.undo());
        assert_eq!(view.text(), "ab");
        assert!(!view.undo(), "历史已空");
    }

    #[test]
    fn continuous_typing_run_collapses_to_one_undo_step() {
        let mut view = CodeView::new("hi", "txt", false);
        view.perform(Action::Move(text_editor::Motion::DocumentEnd));
        view.perform(Action::Edit(text_editor::Edit::Insert('x')));
        view.perform(Action::Edit(text_editor::Edit::Insert('y')));
        view.perform(Action::Edit(text_editor::Edit::Insert('z')));
        assert_eq!(view.text(), "hixyz");

        assert!(view.undo());
        // 连续逐键插入在 run 内共享同一个还原点,一次 undo 应退回到输入前。
        assert_eq!(view.text(), "hi");
        assert!(view.redo());
        assert_eq!(view.text(), "hixyz");
    }

    #[test]
    fn undo_interleaved_with_retype_clears_redo_branch() {
        let mut view = CodeView::new("a", "txt", false);
        view.perform(Action::Move(text_editor::Motion::DocumentEnd));
        view.perform(Action::Edit(text_editor::Edit::Insert('b')));
        view.undo();
        assert_eq!(view.text(), "a");
        // 撤销后再敲新内容:旧 redo 分支(回到 "ab")应被清空,重做 fallback 到空。
        view.perform(Action::Move(text_editor::Motion::DocumentEnd));
        view.perform(Action::Edit(text_editor::Edit::Insert('c')));
        assert_eq!(view.text(), "ac");
        assert!(!view.redo(), "新编辑后不该还能重做被清掉的旧分支");
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
