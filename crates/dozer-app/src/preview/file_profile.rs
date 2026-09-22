//! 文件画像(文件预览重构 Phase A):打开/路由前对文件做的**有界**采样,
//! 只读文件头尾各至多 64KiB,绝不因画像扫描整个大文件。
//!
//! 画像结果 `FileProfile` 是路由(`router::classify_preview`)与后续大文件
//! 降级策略的唯一内容依据;encoding / line-ending 信息保留下来供保存策略
//! 复现原始约定(BOM/CRLF)。

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::SystemTime;

/// 头/尾各自最多采样的字节数。
const SAMPLE_BYTES: usize = 64 * 1024;
/// 判定为二进制的控制字符占比阈值(排除 \t \n \r 后的可打印判断)。
const BINARY_CONTROL_RATIO_NUM: usize = 3;
const BINARY_CONTROL_RATIO_DEN: usize = 10;

/// 文件是否是可用的 UTF-8 文本(采样范围内)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Utf8Status {
    Valid,
    Invalid,
}

/// 采样得出的内容类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    Empty,
    Text,
    Binary,
}

/// 采样识别出的文本编码约定(保存策略据此保持原样)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

/// 采样识别出的换行约定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
    Cr,
    Mixed,
    None,
}

/// 一次有界文件采样得到的画像。
#[derive(Debug, Clone, PartialEq)]
pub struct FileProfile {
    pub size_bytes: u64,
    /// 采样范围内可精确统计时给出总行数;只采头尾的大文件为 `None`。
    pub sampled_line_count: Option<u64>,
    /// 采样范围内的最长行字节数(二进制恒为 0)。
    pub sampled_max_line_bytes: usize,
    pub utf8: Utf8Status,
    pub content_kind: ContentKind,
    pub modified: Option<SystemTime>,
    pub encoding: TextEncoding,
    pub line_ending: LineEnding,
    pub has_bom: bool,
}

impl FileProfile {
    fn empty(modified: Option<SystemTime>) -> Self {
        Self {
            size_bytes: 0,
            sampled_line_count: Some(0),
            sampled_max_line_bytes: 0,
            utf8: Utf8Status::Valid,
            content_kind: ContentKind::Empty,
            modified,
            encoding: TextEncoding::Utf8,
            line_ending: LineEnding::None,
            has_bom: false,
        }
    }
}

/// 画像一个文件。只做有界 I/O;读取失败(不存在/权限)原样透传。
pub fn profile_file(path: &Path) -> std::io::Result<FileProfile> {
    let meta = std::fs::metadata(path)?;
    let size = meta.len();
    let modified = meta.modified().ok();

    if size == 0 {
        return Ok(FileProfile::empty(modified));
    }

    let mut file = std::fs::File::open(path)?;
    let head_len = SAMPLE_BYTES.min(size as usize);
    let mut head = vec![0u8; head_len];
    let n = file.read(&mut head)?;
    head.truncate(n);
    let full_sample = (n as u64) == size;

    let tail = if full_sample {
        None
    } else {
        let tail_len = SAMPLE_BYTES.min(size as usize);
        file.seek(SeekFrom::Start(size - tail_len as u64))?;
        let mut t = vec![0u8; tail_len];
        let tn = file.read(&mut t)?;
        t.truncate(tn);
        Some(t)
    };

    Ok(analyze(&head, tail.as_deref(), size, modified))
}

/// 纯分析函数:输入采样字节,产出画像(单测直接构造字节,不碰磁盘)。
pub fn analyze(
    head: &[u8],
    tail: Option<&[u8]>,
    size_bytes: u64,
    modified: Option<SystemTime>,
) -> FileProfile {
    if size_bytes == 0 || (head.is_empty() && tail.is_none_or(|t| t.is_empty())) {
        return FileProfile::empty(modified);
    }
    let (encoding, bom_len) = detect_encoding(head);
    let has_bom = bom_len > 0;

    // 二进制检测可以合并字节计数，但换行/最长行不能跨越两个不相邻样本。
    let sample: Vec<u8> = {
        let mut v = Vec::with_capacity(head.len() + tail.map(|t| t.len()).unwrap_or(0));
        v.extend_from_slice(head);
        if let Some(t) = tail {
            v.extend_from_slice(t);
        }
        v
    };
    let body = &sample[bom_len.min(sample.len())..];
    let has_nul = body.contains(&0);
    let utf8 = if matches!(encoding, TextEncoding::Utf16Le | TextEncoding::Utf16Be) {
        Utf8Status::Invalid
    } else if utf8_prefix_ok(head) && tail.is_none_or(utf8_fragment_ok) {
        Utf8Status::Valid
    } else {
        Utf8Status::Invalid
    };

    let content_kind = if !is_utf16(encoding) && (has_nul || control_ratio_high(body)) {
        ContentKind::Binary
    } else {
        ContentKind::Text
    };

    let full_sample = size_bytes as usize <= head.len();
    let head_body = &head[bom_len.min(head.len())..];
    let line_ending = merge_line_endings(
        detect_line_ending(head_body),
        tail.map(detect_line_ending).unwrap_or(LineEnding::None),
    );
    let (sampled_line_count, sampled_max_line_bytes) = if content_kind == ContentKind::Binary {
        (None, 0)
    } else {
        let max = max_line_bytes(head_body).max(tail.map(max_line_bytes).unwrap_or(0));
        let count = if full_sample {
            Some(count_lines(head))
        } else {
            None
        };
        (count, max)
    };

    FileProfile {
        size_bytes,
        sampled_line_count,
        sampled_max_line_bytes,
        utf8,
        content_kind,
        modified,
        encoding,
        line_ending,
        has_bom,
    }
}

fn is_utf16(e: TextEncoding) -> bool {
    matches!(e, TextEncoding::Utf16Le | TextEncoding::Utf16Be)
}

/// 识别编码与前导 BOM 长度。无 BOM(含空文件)一律按 UTF-8/0。
fn detect_encoding(head: &[u8]) -> (TextEncoding, usize) {
    if head.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (TextEncoding::Utf8Bom, 3)
    } else if head.starts_with(&[0xFF, 0xFE]) {
        (TextEncoding::Utf16Le, 2)
    } else if head.starts_with(&[0xFE, 0xFF]) {
        (TextEncoding::Utf16Be, 2)
    } else {
        (TextEncoding::Utf8, 0)
    }
}

/// 采样前缀是否"合法 UTF-8 前缀":允许末尾因采样截断而产生的不完整多字节
/// 序列,不把它误判成非法编码。
fn utf8_prefix_ok(bytes: &[u8]) -> bool {
    match std::str::from_utf8(bytes) {
        Ok(_) => true,
        Err(e) => {
            // error_len == None 表示字节序列在末尾被截断(需要更多字节),
            // 这正是采样边界会出现的形态,不算非法。
            e.error_len().is_none()
        }
    }
}

/// 尾样本可能从一个 UTF-8 字符中间开始；跳过至多三个前导 continuation
/// byte 后，仍要求其余内容是合法 UTF-8（末尾允许被采样边界截断）。
fn utf8_fragment_ok(bytes: &[u8]) -> bool {
    let start = bytes
        .iter()
        .take(3)
        .take_while(|&&b| b & 0b1100_0000 == 0b1000_0000)
        .count();
    utf8_prefix_ok(&bytes[start..])
}

fn merge_line_endings(a: LineEnding, b: LineEnding) -> LineEnding {
    match (a, b) {
        (LineEnding::None, other) | (other, LineEnding::None) => other,
        (left, right) if left == right => left,
        _ => LineEnding::Mixed,
    }
}

/// 排除 \t \n \r 后,控制字符占比是否超过阈值。
fn control_ratio_high(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let control = bytes
        .iter()
        .filter(|&&b| b < 0x20 && b != b'\t' && b != b'\n' && b != b'\r')
        .count();
    control * BINARY_CONTROL_RATIO_DEN > bytes.len() * BINARY_CONTROL_RATIO_NUM
}

fn detect_line_ending(bytes: &[u8]) -> LineEnding {
    let mut crlf = false;
    let mut lf = false;
    let mut cr = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                crlf = true;
                i += 2;
                continue;
            }
            b'\r' => cr = true,
            b'\n' => lf = true,
            _ => {}
        }
        i += 1;
    }
    match (crlf, lf, cr) {
        (false, false, false) => LineEnding::None,
        (true, false, false) => LineEnding::Crlf,
        (false, true, false) => LineEnding::Lf,
        (false, false, true) => LineEnding::Cr,
        _ => LineEnding::Mixed,
    }
}

/// 采样范围内最长行的字节数(去掉行尾 \r)。
fn max_line_bytes(bytes: &[u8]) -> usize {
    let mut max = 0usize;
    for line in bytes.split(|&b| b == b'\n') {
        let len = if line.last() == Some(&b'\r') {
            line.len() - 1
        } else {
            line.len()
        };
        max = max.max(len);
    }
    max
}

/// 完整采样的换行数推导出的逻辑行数(最后一行无换行也算一行)。
fn count_lines(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    let newlines = bytes.iter().filter(|&&b| b == b'\n').count() as u64;
    if bytes.ends_with(b"\n") {
        newlines
    } else {
        newlines + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dozer_profile_{}_{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(bytes).unwrap();
        path
    }

    #[test]
    fn empty_file() {
        let p = tmp("empty", b"");
        let prof = profile_file(&p).unwrap();
        assert_eq!(prof.size_bytes, 0);
        assert_eq!(prof.content_kind, ContentKind::Empty);
        assert_eq!(prof.utf8, Utf8Status::Valid);
        assert_eq!(prof.sampled_line_count, Some(0));
    }

    #[test]
    fn short_text_counts_lines_exactly() {
        let p = tmp("short", b"alpha\nbeta\ngamma");
        let prof = profile_file(&p).unwrap();
        assert_eq!(prof.content_kind, ContentKind::Text);
        assert_eq!(prof.utf8, Utf8Status::Valid);
        assert_eq!(prof.sampled_line_count, Some(3));
        assert_eq!(prof.line_ending, LineEnding::Lf);
        assert_eq!(prof.sampled_max_line_bytes, 5);
    }

    #[test]
    fn utf8_bom_detected() {
        let p = tmp("bom", b"\xEF\xBB\xBFhello\n");
        let prof = profile_file(&p).unwrap();
        assert!(prof.has_bom);
        assert_eq!(prof.encoding, TextEncoding::Utf8Bom);
        assert_eq!(prof.utf8, Utf8Status::Valid);
        assert_eq!(prof.content_kind, ContentKind::Text);
    }

    #[test]
    fn crlf_detected() {
        let p = tmp("crlf", b"a\r\nb\r\n");
        let prof = profile_file(&p).unwrap();
        assert_eq!(prof.line_ending, LineEnding::Crlf);
    }

    #[test]
    fn invalid_utf8_is_invalid_but_text_without_nul() {
        // 非法 UTF-8(连续 0xFF)但不是二进制(无 NUL、无可打印性塌陷)。
        let p = tmp("invalid", b"caf\xFF reste du texte\n");
        let prof = profile_file(&p).unwrap();
        assert_eq!(prof.utf8, Utf8Status::Invalid);
        assert_eq!(prof.content_kind, ContentKind::Text);
    }

    #[test]
    fn nul_byte_is_binary() {
        let p = tmp("nul", b"text\0more text\n");
        let prof = profile_file(&p).unwrap();
        assert_eq!(prof.content_kind, ContentKind::Binary);
        assert_eq!(prof.sampled_line_count, None);
        assert_eq!(prof.sampled_max_line_bytes, 0);
    }

    #[test]
    fn extension_spoof_binary_content_still_binary() {
        // `.txt` 里塞满 NUL:内容优先于扩展名。
        let p = tmp("spoof.txt", &[0u8; 256]);
        let prof = profile_file(&p).unwrap();
        assert_eq!(prof.content_kind, ContentKind::Binary);
    }

    #[test]
    fn huge_single_line_reports_max_line_and_no_count() {
        // 采样上限之外仍能给出"最长行"的下界信息;行数未知。
        let mut bytes = vec![b'a'; SAMPLE_BYTES + 10_000];
        bytes.push(b'\n');
        let p = tmp("oneline", &bytes);
        let prof = profile_file(&p).unwrap();
        assert_eq!(prof.content_kind, ContentKind::Text);
        assert_eq!(prof.sampled_line_count, None);
        assert!(prof.sampled_max_line_bytes >= SAMPLE_BYTES);
    }

    #[test]
    fn large_file_only_sampled_no_line_count() {
        let mut bytes = Vec::new();
        for _ in 0..(SAMPLE_BYTES / 4 + 100) {
            bytes.extend_from_slice(b"line\n");
        }
        let p = tmp("large", &bytes);
        let prof = profile_file(&p).unwrap();
        assert!(prof.size_bytes > SAMPLE_BYTES as u64);
        assert_eq!(prof.sampled_line_count, None);
        assert_eq!(prof.content_kind, ContentKind::Text);
    }

    #[test]
    fn invalid_utf8_in_tail_is_not_ignored() {
        let mut bytes = vec![b'a'; SAMPLE_BYTES * 2 + 32];
        let last = bytes.len() - 8;
        bytes[last] = 0xff;
        let p = tmp("invalid_tail", &bytes);
        let prof = profile_file(&p).unwrap();
        assert_eq!(prof.utf8, Utf8Status::Invalid);
    }

    #[test]
    fn separate_samples_do_not_invent_crlf_or_join_lines() {
        let head = b"short\nend-with-cr\r";
        let tail = b"\nshort-tail";
        let prof = analyze(head, Some(tail), 1_000_000, None);
        assert_eq!(prof.line_ending, LineEnding::Mixed);
        assert_eq!(prof.sampled_max_line_bytes, "end-with-cr".len());
    }

    #[test]
    fn utf16_bom_treated_as_text() {
        let p = tmp("utf16", b"\xFF\xFEh\x00i\x00");
        let prof = profile_file(&p).unwrap();
        assert_eq!(prof.encoding, TextEncoding::Utf16Le);
        assert_eq!(prof.content_kind, ContentKind::Text);
        assert_eq!(prof.utf8, Utf8Status::Invalid);
    }

    #[test]
    fn analyze_is_pure_and_reusable() {
        let prof = analyze(b"one\ntwo\n", None, 8, None);
        assert_eq!(prof.content_kind, ContentKind::Text);
        assert_eq!(prof.sampled_line_count, Some(2));
    }
}
