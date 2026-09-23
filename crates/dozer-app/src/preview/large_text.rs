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

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
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

/// 分段扫描的单段字节数。超长单行被切成多段,用 `needle.len()-1` 字节重叠
/// 避免跨段漏配——任何单行长度下峰值临时内存都不随整行长度增长。
const SEARCH_SEGMENT_BYTES: usize = 256 * 1024;

/// 逐行流式搜索,**内存有界**:不把整行读入内存(旧实现的
/// `BufRead::split` 遇到 300MB 无换行单行会整行分配)。超长单行按
/// [`SEARCH_SEGMENT_BYTES`] 分段,段间以 `needle.len()-1` 字节重叠避免跨段
/// 漏配;列号用增量 UTF-8 字符计数(continuation byte 不计数),摘要只取命中
/// 附近有限字节,均不复制整行。读取出错原样透传。
///
/// 大小写不敏感时对**段缓冲**做 ASCII 折叠(字节长度不变,故列/摘要仍基于
/// 原字节);非 ASCII 的大小写折叠不参与匹配(罕见,查询多来自单行输入框)。
pub fn stream_search(
    path: &Path,
    query: &str,
    opts: SearchOptions,
) -> std::io::Result<SearchOutcome> {
    let mut hits = Vec::new();
    if query.is_empty() {
        return Ok(SearchOutcome {
            hits,
            total_matches: 0,
            truncated: false,
            lines_scanned: 0,
        });
    }
    let needle: Vec<u8> = if opts.case_sensitive {
        query.as_bytes().to_vec()
    } else {
        query.bytes().map(|b| b.to_ascii_lowercase()).collect()
    };
    let nlen = needle.len();
    let finder = memchr::memmem::Finder::new(&needle);

    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::with_capacity(SEARCH_SEGMENT_BYTES, file);

    let mut total: u64 = 0;
    let mut max_line: u64 = 1;
    // 当前 `combined` 起点(即 overlap 起点)的行状态。
    let mut base_line: u64 = 1;
    let mut base_chars: u64 = 0;
    let mut overlap: Vec<u8> = Vec::new();

    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            break;
        }
        let overlap_len = overlap.len();
        // combined = overlap + 本段(命中可能跨段边界)。
        let combined: Vec<u8> = if overlap.is_empty() {
            buf.to_vec()
        } else {
            let mut v = Vec::with_capacity(overlap_len + buf.len());
            v.extend_from_slice(&overlap);
            v.extend_from_slice(buf);
            v
        };
        let n = buf.len();
        reader.consume(n);

        // 折叠缓冲(仅不敏感时构建);ASCII 折叠不改变字节长度/字符计数。
        let hay: Option<Vec<u8>> = if opts.case_sensitive {
            None
        } else {
            let mut h = combined.clone();
            h.make_ascii_lowercase();
            Some(h)
        };
        let search_bytes: &[u8] = hay.as_deref().unwrap_or(&combined);

        // (A) 先算下一段起点(base_pos)处的行状态,供下一段作为 base。
        let keep = nlen.saturating_sub(1).min(combined.len());
        let base_pos = combined.len() - keep;
        let (new_line, new_chars, seen_max) =
            advance_line_state(base_line, base_chars, &combined[..base_pos]);
        max_line = max_line.max(seen_max);

        // (B) 找本段新出现的命中,并在扫描到命中处时推进行/列计数。
        let mut line = base_line;
        let mut chars = base_chars;
        let mut cursor = 0usize;
        for m in finder.find_iter(search_bytes) {
            // 完全落在 overlap 内的命中上一段已报过,跳过(避免重复)。
            if m + nlen <= overlap_len {
                continue;
            }
            advance_line_state_into(&mut line, &mut chars, &combined[cursor..m]);
            cursor = m;
            total += 1;
            if hits.len() < opts.max_hits {
                hits.push(SearchHit {
                    line: line as u32,
                    column: chars as u32 + 1,
                    text: summarize_bytes(&combined, m, nlen, opts.max_preview_chars),
                });
            }
        }
        max_line = max_line.max(line);

        // 段尾重叠保留;base 状态已算好。
        overlap.clear();
        overlap.extend_from_slice(&combined[base_pos..]);
        base_line = new_line;
        base_chars = new_chars;
    }

    let truncated = total > hits.len() as u64;
    Ok(SearchOutcome {
        hits,
        total_matches: total,
        truncated,
        lines_scanned: max_line,
    })
}

/// 在 `bytes` 上推进行/字符计数:遇到 `\n` 换行归零,其余按 UTF-8 首字节
/// (非 continuation)计一个字符。返回(新行号, 新行内字符数, 期间最大行号)。
fn advance_line_state(mut line: u64, mut chars: u64, bytes: &[u8]) -> (u64, u64, u64) {
    let mut max_line = line;
    for &b in bytes {
        if b == b'\n' {
            line += 1;
            chars = 0;
            max_line = line;
        } else if b & 0xC0 != 0x80 {
            chars += 1;
        }
    }
    (line, chars, max_line)
}

/// 同 [`advance_line_state`],但不需要最大行号(命中扫描路径用)。
fn advance_line_state_into(line: &mut u64, chars: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        if b == b'\n' {
            *line += 1;
            *chars = 0;
        } else if b & 0xC0 != 0x80 {
            *chars += 1;
        }
    }
}

/// 命中附近的有限摘要(只取命中周围有限字节,不复制整行)。以命中处为中心按
/// 字符裁剪并加省略号。
fn summarize_bytes(bytes: &[u8], start: usize, needle_len: usize, max_chars: usize) -> String {
    let radius = max_chars.saturating_mul(2).max(16);
    let mut lo = start.saturating_sub(radius);
    while lo > 0 && lo < bytes.len() && bytes[lo] & 0xC0 == 0x80 {
        lo += 1;
    }
    let hi = (start + needle_len + radius).min(bytes.len());
    let slice = String::from_utf8_lossy(&bytes[lo..hi]);
    let trimmed = slice.trim_end();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= max_chars {
        return trimmed.to_string();
    }
    let hit_char = String::from_utf8_lossy(&bytes[lo..start]).chars().count();
    let half = max_chars / 2;
    let cs = hit_char.saturating_sub(half);
    let ce = (cs + max_chars).min(chars.len());
    let mut out = String::new();
    if cs > 0 {
        out.push('…');
    }
    out.extend(&chars[cs..ce]);
    if ce < chars.len() {
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
        let mut at_line_start = true;
        let mut saw_byte = false;
        loop {
            if should_cancel() {
                return Ok(None);
            }
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                break;
            }
            for (i, &byte) in buf.iter().enumerate() {
                if at_line_start {
                    if line == 1 || (line - 1).is_multiple_of(every) {
                        entries.push(IndexEntry {
                            line,
                            offset: offset + i as u64,
                        });
                    }
                    at_line_start = false;
                }
                saw_byte = true;
                if byte == b'\n' {
                    line = line.saturating_add(1);
                    at_line_start = true;
                }
            }
            let n = buf.len();
            reader.consume(n);
            offset += n as u64;
        }
        let total_lines = if !saw_byte {
            0
        } else if at_line_start {
            line.saturating_sub(1)
        } else {
            line
        };
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
        let mut current = start.line;
        let mut offset = start.offset;
        while current < line {
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                break;
            }
            let mut consumed = 0usize;
            for &byte in buf {
                consumed += 1;
                if byte == b'\n' {
                    current += 1;
                    if current == line {
                        break;
                    }
                }
            }
            reader.consume(consumed);
            offset += consumed as u64;
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
    // 只读“字节上限 + 1”。不能用 `read_until`:单行 300MB 时它会在
    // 我们有机会裁剪前就先分配并读入整行。
    let mut bytes = Vec::with_capacity(max_bytes.saturating_add(1));
    file.take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;

    let wanted_lines = end_line.saturating_sub(start_line).saturating_add(1) as usize;
    let mut complete_lines = 0usize;
    let mut desired_end = None;
    for (i, &byte) in bytes.iter().enumerate() {
        if byte == b'\n' {
            complete_lines += 1;
            if complete_lines == wanted_lines {
                desired_end = Some(i + 1);
                break;
            }
        }
    }
    let over_budget = bytes.len() > max_bytes;
    let take = desired_end.unwrap_or(bytes.len().min(max_bytes));
    let truncated = desired_end.is_none() && over_budget;
    let displayed_lines = if take == 0 {
        0
    } else {
        bytes[..take].iter().filter(|&&b| b == b'\n').count()
            + usize::from(bytes[take - 1] != b'\n')
    };
    let text = String::from_utf8_lossy(&bytes[..take]).into_owned();
    Ok(TextWindow {
        start_line,
        end_line: start_line
            .saturating_add(displayed_lines.saturating_sub(1) as u32)
            .max(start_line),
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

    #[test]
    fn index_offsets_survive_buffer_boundaries_and_crlf() {
        // 内容 > 8KiB(BufReader 默认缓冲区),多行跨缓冲区边界;offset 必须
        // 逐行精确,CRLF 也不影响。
        let mut content = String::new();
        for i in 1..=2000u32 {
            content.push_str(&format!("line-{i:05}\r\n"));
        }
        assert!(content.len() > 16 * 1024);
        let p = tmp("crossbuf", content.as_bytes());
        let idx = LineIndex::build(&p, 50, 3).unwrap();
        assert_eq!(idx.total_lines(), 2000);
        let mut expected = 0u64;
        for (i, line) in content
            .as_bytes()
            .split_inclusive(|&b| b == b'\n')
            .enumerate()
        {
            assert_eq!(
                idx.offset_for_line(&p, i as u32 + 1).unwrap(),
                expected,
                "行 {} offset 不符",
                i + 1
            );
            expected += line.len() as u64;
        }
    }

    #[test]
    fn stream_search_finds_match_across_segment_boundary_in_huge_line() {
        // 单行长度是段大小的 ~2 倍,命中词正好横跨段边界;分段有界扫描必须
        // 不漏配(旧实现会整行读入,本次改为段间重叠)。
        let seg = SEARCH_SEGMENT_BYTES;
        let mut line = vec![b'a'; seg * 2];
        let needle = b"NEEDLE";
        let at = seg - 3; // 横跨第一段末尾与第二段开头
        line[at..at + needle.len()].copy_from_slice(needle);
        line.push(b'\n');
        let p = tmp("crossseg", &line);
        let out = stream_search(
            &p,
            "NEEDLE",
            SearchOptions {
                case_sensitive: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(out.total_matches, 1);
        assert_eq!(out.hits[0].line, 1);
        assert_eq!(out.hits[0].column, (at as u32) + 1, "列 = 命中前字符数 + 1");
    }

    #[test]
    fn stream_search_handles_long_single_line_bounded() {
        // 无换行超长单行:命中在很后面,列号仍精确,且有界扫描不会整行驻留
        // (本测试以"能完成 + 列正确"为准;内存上界由分段实现保证)。
        let mut line = vec![b'x'; SEARCH_SEGMENT_BYTES + 4096];
        line.extend_from_slice(b"target");
        let p = tmp("longline_search", &line);
        let out = stream_search(
            &p,
            "target",
            SearchOptions {
                case_sensitive: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(out.total_matches, 1);
        assert_eq!(
            out.hits[0].column,
            (SEARCH_SEGMENT_BYTES + 4096) as u32 + 1,
            "列 = 命中前字符数 + 1"
        );
    }

    #[test]
    fn read_window_on_300mb_single_line_is_bounded() {
        // 300MB 稀疏单行(全 NUL,稀疏文件,读取便宜):窗口读取量封顶在
        // WINDOW_MAX_BYTES 附近,绝不整行驻留。索引扫描也应完成。
        let dir = std::env::temp_dir().join(format!("dozer_large_300m_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("huge.bin");
        let f = std::fs::File::create(&p).unwrap();
        f.set_len(300 * 1024 * 1024).unwrap();
        drop(f);

        let idx = LineIndex::build(&p, 1000, 1).unwrap();
        assert_eq!(idx.total_lines(), 1, "无换行 → 单行");
        let w = read_window(&p, &idx, 1, 1000, 2000).unwrap();
        assert!(w.truncated, "超长单行应标记截断");
        assert!(
            w.text.len() <= WINDOW_MAX_BYTES,
            "窗口正文不得超过字节上限,实得 {}",
            w.text.len()
        );

        std::fs::remove_file(&p).ok();
    }
}
