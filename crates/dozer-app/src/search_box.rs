//! 光标编辑原语(`char_to_byte`/`insert_at_cursor`/`delete_before_cursor`/
//! `move_cursor_in`/`CursorDir`)、`draft_with_caret` 合成光标文本。这些
//! 纯字符串/下标操作原来是 `extensions::todo` 的私有实现,抽 `search_box`
//! 模块时挪到这里——不依赖任何 todo 专属类型,`todo.rs` 现在从这里 `use`
//! 回去。搜索框 UI 本体(`view`)与配色(`SearchBoxColors`)已在 Stage 4 随
//! 两个调用方迁 iced 原生 `text_input`/`text_editor` 一起删除;本模块只剩
//! 被 Todo 任务内容编辑(`ContentEvent`/`ContentCursorMove`)与 Todo MARKDOWN
//! 整文件编辑(`MarkdownEvent`)仍依赖的这几个自绘光标原语。

/// 方向键移动自绘光标的方向(`Home`/`End` 跳行首/行尾)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorDir {
    Left,
    Right,
    Home,
    End,
}

/// 字符下标 → 字节下标;`cursor` 越界时回落到串尾(防御性,避免越界 panic)。
pub fn char_to_byte(s: &str, cursor: usize) -> usize {
    s.char_indices()
        .nth(cursor)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// 在 `s` 的字符下标 `cursor` 处插入 `ins`,并把 `cursor` 前移插入的字符数。
/// 自绘输入没有原生光标,插入必须落在光标处而非强行 `push_str` 到行尾。
pub fn insert_at_cursor(s: &mut String, cursor: &mut usize, ins: &str) {
    if ins.is_empty() {
        return;
    }
    let byte = char_to_byte(s, *cursor);
    s.insert_str(byte, ins);
    *cursor += ins.chars().count();
}

/// 删除 `cursor` 前一个字符;已在行首时 no-op。返回是否真的删了字符。
pub fn delete_before_cursor(s: &mut String, cursor: &mut usize) -> bool {
    if *cursor == 0 {
        return false;
    }
    let rem = *cursor - 1;
    let start = char_to_byte(s, rem);
    let end = char_to_byte(s, *cursor);
    s.drain(start..end);
    *cursor = rem;
    true
}

/// 按方向键移动 `cursor`(字符下标)。`Left`/`Right` 单步、`Home`/`End` 跳
/// 行首/行尾。光标恒夹在 `[0, len]`,不会越界。
pub fn move_cursor_in(s: &str, cursor: &mut usize, dir: CursorDir) {
    let len = s.chars().count();
    let c = *cursor as isize;
    *cursor = match dir {
        CursorDir::Left => c.saturating_sub(1),
        CursorDir::Right => (c + 1).min(len as isize),
        CursorDir::Home => 0,
        CursorDir::End => len as isize,
    }
    .clamp(0, len as isize) as usize;
}

/// 把 `draft` 从字符下标 `cursor` 处劈开,中间塞 `▏` 当光标。`cursor`
/// 越界时回落到串尾(`char_to_byte` 的防御性兜底)。
pub fn draft_with_caret(draft: &str, cursor: usize) -> String {
    let byte = char_to_byte(draft, cursor);
    let (before, after) = draft.split_at(byte);
    format!("{before}▏{after}")
}
