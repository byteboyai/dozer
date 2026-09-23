//! 大文本的流式搜索与稀疏行索引(文件预览重构 Phase C Task 2)。
//!
//! 两条独立能力,都不把整文件读进内存:
//! - [`stream_search`]:逐行扫描,返回 1-based 行列 + 有限摘要;命中列表封顶
//!   但保留匹配总数与截断说明。
//! - [`LineIndex`]:每 N 行记录一个 byte offset 的稀疏索引;`offset_for_line`
//!   从最近索引 seek 后有限扫描,`read_window` 读目标附近的有界窗口。

// Phase C 建立的策略/资源/流式模块,消费方(Windowed viewer、资源接线)接入前
// 部分 API 暂未被非测试代码调用;显式允许,避免 dead_code 噪声。
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

/// 一条搜索命中(1-based 行列)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub line: u32,
    /// 1-based 字符列(不含行尾换行)。
    pub column: u32,
    /// 命中附近的有限摘要。
    pub text: String,
}

/// 搜索选项。
#[derive(Debug, Clone, Copy)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    /// 返回的命中条数上限(超出即截断,但总数仍统计)。
    pub max_hits: usize,
    /// 每条摘要的最大字符数。
    pub max_preview_chars: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            case_sensitive: false,
            max_hits: 500,
            max_preview_chars: 160,
        }
    }
}

/// 搜索结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOutcome {
    pub hits: Vec<SearchHit>,
    /// 整个文件里的匹配总数(可能远大于 `hits.len()`)。
    pub total_matches: u64,
    /// 命中列表是否因 `max_hits` 被截断。
    pub truncated: bool,
    /// 扫描过的行数。
    pub lines_scanned: u64,
}

/// 逐行流式搜索。文件不会被整体读入内存;读取出错原样透传。
pub fn stream_search(
    path: &Path,
    query: &str,
    opts: SearchOptions,
) -> std::io::Result<SearchOutcome> {
    let mut hits = Vec::new();
    let mut total: u64 = 0;
    let mut lines_scanned: u64 = 0;
    if query.is_empty() {
        return Ok(SearchOutcome {
            hits,
            total_matches: 0,
            truncated: false,
            lines_scanned: 0,
        });
    }
    let needle = if opts.case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };

    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);
    for (idx, line) in reader.split(b'\n').enumerate() {
        let raw = line?;
        lines_scanned = idx as u64 + 1;
        let line_no = (idx + 1) as u32;
        // 去掉行尾 \r(CRLF),列/摘要都基于去 \r 后的内容。
        let bytes = raw.strip_suffix(b"\r").unwrap_or(&raw);
        let text = String::from_utf8_lossy(bytes);
        let hay = if opts.case_sensitive {
            text.to_string()
        } else {
            text.to_lowercase()
        };
        let mut from = 0usize;
        while let Some(pos) = hay[from..].find(&needle) {
            let byte_pos = from + pos;
            total += 1;
            if hits.len() < opts.max_hits {
                let column = text[..byte_pos].chars().count() as u32 + 1;
                hits.push(SearchHit {
                    line: line_no,
                    column,
                    text: summarize(&text, byte_pos, opts.max_preview_chars),
                });
            }
            // 前进到下个字符边界,避免零宽/重叠死循环。
            from = byte_pos + needle.len().max(1);
            if from > hay.len() {
                break;
            }
        }
    }

    let truncated = total > hits.len() as u64;
    Ok(SearchOutcome {
        hits,
        total_matches: total,
        truncated,
        lines_scanned,
    })
}

/// 命中附近的有限摘要(以命中处为中心,裁剪并加省略号)。
fn summarize(text: &str, byte_pos: usize, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    // byte_pos 对应的字符下标。
    let char_pos = text[..byte_pos].chars().count();
    if chars.len() <= max_chars {
        return text.trim_end().to_string();
    }
    let half = max_chars / 2;
    let start = char_pos.saturating_sub(half);
    let end = (start + max_chars).min(chars.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&chars[start..end]);
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// 稀疏行索引:每 `every` 行记录 `(行号, byte offset)`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    every: u32,
    entries: Vec<IndexEntry>,
    total_lines: u32,
    file_len: u64,
    /// 建立索引时的文件 revision;用于失效判定。
    revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntry {
    pub line: u32,
    pub offset: u64,
}

impl LineIndex {
    /// 建立索引。`every` 至少为 1。`revision` 供 [`Self::is_valid_for`] 失效。
    pub fn build(path: &Path, every: u32, revision: u64) -> std::io::Result<Self> {
        Self::build_cancellable(path, every, revision, || false).map(|o| o.expect("not cancelled"))
    }

    /// 可取消的索引构建。`should_cancel` 返回 true 时提前中止并返回 `Ok(None)`,
    /// 让调用方能在 UI 侧放弃(不阻塞首屏)。
    pub fn build_cancellable(
        path: &Path,
        every: u32,
        revision: u64,
        should_cancel: impl Fn() -> bool,
    ) -> std::io::Result<Option<Self>> {
        let every = every.max(1);
        let file = std::fs::File::open(path)?;
        let file_len = file.metadata()?.len();
        let mut reader = BufReader::new(file);
        let mut entries = Vec::new();
        let mut offset: u64 = 0;
        let mut line: u32 = 1;
        let mut buf = Vec::new();
        loop {
            if should_cancel() {
                return Ok(None);
            }
            buf.clear();
            let n = reader.read_until(b'\n', &mut buf)?;
            if n == 0 {
                break;
            }
            if line == 1 || (line - 1).is_multiple_of(every) {
                entries.push(IndexEntry { line, offset });
            }
            offset += n as u64;
            line += 1;
        }
        let total_lines = if file_len == 0 { 0 } else { line - 1 };
        Ok(Some(Self {
            every,
            entries,
            total_lines,
            file_len,
            revision,
        }))
    }

    pub fn every(&self) -> u32 {
        self.every
    }

    pub fn total_lines(&self) -> u32 {
        self.total_lines
    }

    pub fn file_len(&self) -> u64 {
        self.file_len
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 索引是否对给定 revision 仍有效(文件被替换后必须重建)。
    pub fn is_valid_for(&self, revision: u64) -> bool {
        self.revision == revision
    }

    /// 取最接近但不超过 `line` 的索引项。
    fn nearest_entry(&self, line: u32) -> IndexEntry {
        match self.entries.binary_search_by_key(&line, |e| e.line) {
            Ok(i) => self.entries[i],
            Err(0) => self
                .entries
                .first()
                .copied()
                .unwrap_or(IndexEntry { line: 1, offset: 0 }),
            Err(i) => self.entries[i - 1],
        }
    }

    /// 从最近索引 seek 后有限扫描到目标行,返回其 byte offset。
    pub fn offset_for_line(&self, path: &Path, line: u32) -> std::io::Result<u64> {
        let line = line.clamp(1, self.total_lines.max(1));
        let start = self.nearest_entry(line);
        let mut file = std::fs::File::open(path)?;
        file.seek(SeekFrom::Start(start.offset))?;
        let mut reader = BufReader::new(file);
        let mut buf = Vec::new();
        let mut current = start.line;
        let mut offset = start.offset;
        while current < line {
            buf.clear();
            let n = reader.read_until(b'\n', &mut buf)?;
            if n == 0 {
                break;
            }
            offset += n as u64;
            current += 1;
        }
        Ok(offset)
    }
}

/// 有界窗口:目标行附近的一段(带全局起始行号)。
///
/// `truncated` 表示窗口因**字节上限**被截断:单行过长时只保留该行前若干个
/// 字节,后续内容不再包含(展示端应给出"本行过长,已截断"提示,避免把
/// 整条超长行推进 WebView 导致空白/卡死)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextWindow {
    pub start_line: u32,
    pub end_line: u32,
    pub text: String,
    pub truncated: bool,
}

/// 窗口正文的默认字节上限。按行限幅(WINDOW_BEFORE/AFTER)对"几千行但每行很
/// 短"的文件足够,但对**单行本身就有数百 MB**的病态文件失效——按行取窗口会把
/// 整行读进来。这里再按字节兜一层:窗口内累计正文不超过该值,超长单行只保留
/// 前若干字节并标记截断。
pub const WINDOW_MAX_BYTES: usize = 512 * 1024;

/// 读取目标行附近的有界窗口 `[center-before, center+after]`(钳到合法行范围),
/// 并再受 [`WINDOW_MAX_BYTES`] 字节上限约束。
pub fn read_window(
    path: &Path,
    index: &LineIndex,
    center_line: u32,
    before: u32,
    after: u32,
) -> std::io::Result<TextWindow> {
    read_window_capped(path, index, center_line, before, after, WINDOW_MAX_BYTES)
}

/// 同 [`read_window`],显式指定字节上限(测试与特殊调用方用)。
pub fn read_window_capped(
    path: &Path,
    index: &LineIndex,
    center_line: u32,
    before: u32,
    after: u32,
    max_bytes: usize,
) -> std::io::Result<TextWindow> {
    let total = index.total_lines().max(1);
    let start_line = center_line.saturating_sub(before).max(1);
    let end_line = (center_line.saturating_add(after)).min(total);
    let offset = index.offset_for_line(path, start_line)?;
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::new(file);
    let mut text = String::new();
    let mut buf = Vec::new();
    let mut line = start_line;
    let mut truncated = false;
    while line <= end_line {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        if text.len() + buf.len() > max_bytes {
            // 预算内还能放下多少:尽量补齐到上限,只保留该行前缀。
            let room = max_bytes.saturating_sub(text.len());
            if room > 0 {
                text.push_str(&String::from_utf8_lossy(&buf[..room]));
            }
            truncated = true;
            break;
        }
        text.push_str(&String::from_utf8_lossy(&buf));
        line += 1;
    }
    Ok(TextWindow {
        start_line,
        end_line: line.saturating_sub(1).max(start_line),
        text,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str, content: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dozer_large_{}_{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.txt");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(content).unwrap();
        p
    }

    #[test]
    fn stream_search_basic_lines_and_columns() {
        let p = tmp("basic", b"alpha\nbeta alpha\ngamma\n");
        let out = stream_search(
            &p,
            "alpha",
            SearchOptions {
                case_sensitive: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(out.total_matches, 2);
        assert_eq!(out.hits[0].line, 1);
        assert_eq!(out.hits[0].column, 1);
        assert_eq!(out.hits[1].line, 2);
        assert_eq!(out.hits[1].column, 6);
        assert!(!out.truncated);
    }

    #[test]
    fn stream_search_case_insensitive_by_default() {
        let p = tmp("case", b"Alpha\n");
        let out = stream_search(&p, "alpha", SearchOptions::default()).unwrap();
        assert_eq!(out.total_matches, 1);
    }

    #[test]
    fn stream_search_truncates_hits_but_keeps_total() {
        let p = tmp("cap", b"x\nx\nx\nx\nx\n");
        let out = stream_search(
            &p,
            "x",
            SearchOptions {
                max_hits: 2,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(out.hits.len(), 2);
        assert_eq!(out.total_matches, 5);
        assert!(out.truncated);
    }

    #[test]
    fn stream_search_handles_crlf_and_utf8_columns() {
        // 中文各占一个字符列;"目标"在 UTF-8 里是多字节,列应为字符数而非字节数。
        let p = tmp("utf8", "一二目标三\r\n".as_bytes());
        let out = stream_search(
            &p,
            "目标",
            SearchOptions {
                case_sensitive: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(out.hits[0].line, 1);
        assert_eq!(out.hits[0].column, 3, "“目标”前面有两个字符");
        assert!(!out.hits[0].text.ends_with('\r'));
    }

    #[test]
    fn stream_search_empty_query_is_noop() {
        let p = tmp("emptyq", b"a\nb\n");
        let out = stream_search(&p, "", SearchOptions::default()).unwrap();
        assert_eq!(out.total_matches, 0);
        assert!(out.hits.is_empty());
    }

    #[test]
    fn index_offsets_resolve_each_line() {
        let content = b"l1\nl2\nl3\nl4\nl5\n";
        let p = tmp("index", content);
        let idx = LineIndex::build(&p, 2, 7).unwrap();
        assert_eq!(idx.total_lines(), 5);
        assert_eq!(idx.revision(), 7);
        assert!(idx.is_valid_for(7) && !idx.is_valid_for(8));
        // 逐行 offset 必须精确匹配每行起始字节。
        let mut expected = 0u64;
        for (i, line) in content.split_inclusive(|&b| b == b'\n').enumerate() {
            assert_eq!(idx.offset_for_line(&p, i as u32 + 1).unwrap(), expected);
            expected += line.len() as u64;
        }
    }

    #[test]
    fn index_handles_missing_trailing_newline_and_empty() {
        let p = tmp("nonl", b"a\nb");
        let idx = LineIndex::build(&p, 1, 1).unwrap();
        assert_eq!(idx.total_lines(), 2);
        assert_eq!(idx.offset_for_line(&p, 2).unwrap(), 2);

        let empty = tmp("empty", b"");
        let idx = LineIndex::build(&empty, 1, 1).unwrap();
        assert_eq!(idx.total_lines(), 0);
    }

    #[test]
    fn read_window_returns_bounded_slice_with_global_lines() {
        let mut content = String::new();
        for i in 1..=20 {
            content.push_str(&format!("line-{i}\n"));
        }
        let p = tmp("window", content.as_bytes());
        let idx = LineIndex::build(&p, 3, 1).unwrap();
        let w = read_window(&p, &idx, 10, 2, 2).unwrap();
        assert_eq!(w.start_line, 8);
        assert_eq!(w.end_line, 12);
        assert!(w.text.starts_with("line-8\n"));
        assert!(w.text.contains("line-12\n"));
        assert!(!w.truncated);
    }

    #[test]
    fn read_window_clamps_to_file_bounds() {
        let p = tmp("clamp", b"one\ntwo\nthree\n");
        let idx = LineIndex::build(&p, 1, 1).unwrap();
        let w = read_window(&p, &idx, 1, 5, 100).unwrap();
        assert_eq!(w.start_line, 1);
        assert_eq!(w.end_line, 3);
        assert_eq!(w.text, "one\ntwo\nthree\n");
        assert!(!w.truncated);
    }

    #[test]
    fn read_window_truncates_giant_single_line_by_bytes() {
        // 单行 1MiB(远超上限):窗口只保留前 max_bytes 字节并标记截断,
        // 不能把整行读进来(病态 huge.txt / long_line.rs 的空白根因)。
        let line = "a".repeat(1024 * 1024);
        let p = tmp("giantline", line.as_bytes());
        let idx = LineIndex::build(&p, 1000, 1).unwrap();
        assert_eq!(idx.total_lines(), 1);
        let w = read_window_capped(&p, &idx, 1, 1000, 2000, 4096).unwrap();
        assert_eq!(w.text.len(), 4096);
        assert!(w.truncated);
    }

    #[test]
    fn read_window_stops_at_byte_budget_across_lines() {
        // 多行累计超预算:装到放不下为止,后续行不再包含,标记截断。
        let mut content = String::new();
        for i in 1..=100 {
            content.push_str(&format!("line-{i:04}\n"));
        }
        let p = tmp("budget", content.as_bytes());
        let idx = LineIndex::build(&p, 1000, 1).unwrap();
        let w = read_window_capped(&p, &idx, 1, 0, 100, 50).unwrap();
        assert!(w.truncated);
        assert!(w.text.len() <= 50);
        assert!(w.end_line < 100);
    }

    #[test]
    fn index_build_is_cancellable() {
        let mut content = String::new();
        for i in 0..1000 {
            content.push_str(&format!("l{i}\n"));
        }
        let p = tmp("cancel", content.as_bytes());
        let built = LineIndex::build_cancellable(&p, 10, 1, || true).unwrap();
        assert!(built.is_none(), "取消后应返回 None");
    }
}
