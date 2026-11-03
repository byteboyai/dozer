//! Agent 精确修改的核心逻辑(v0.1 意向文档里的 Mutation Engine,TextAdapter
//! 一种实现):路径解析/边界校验、UTF-8 校验、`locate_in_file` 搜索、
//! `apply_precise_edit` 的 Conflict Detection + 写盘 + 坐标重算。纯逻辑,不
//! 依赖任何 MCP/wire 类型,方便直接用 tempdir 测试。

use dozer_core::protocol::{LocateMatch, LocateReport};
use dozer_file_io::{
    AtomicWriteError, AtomicWriteOutcome, FileError, ProjectRoot, Revision, WriteOptions,
    write_atomic,
};
use std::path::{Path, PathBuf};

/// `locate_in_file` 的输入正文上限。超过时明确返回 `Limit`,不静默截断。
const MAX_LOCATE_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// 单次 locate 返回的命中条数上限。
const MAX_LOCATE_MATCHES: usize = 1000;
/// 单条 context 保留的 Unicode 标量上限。
const MAX_CONTEXT_SCALARS: usize = 1024;
/// 编码后的 JSON 回复预算，包含转义与外层 envelope。
const MAX_LOCATE_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum LocateError {
    /// 路径落在项目根目录子树之外,或是绝对路径/包含 `..`。
    OutOfBounds,
    /// 路径任一级是符号链接(包括末级)。
    SymlinkRejected,
    /// 目标文件不存在。
    NotFound,
    /// 二进制/非 UTF-8,或不是普通文件。
    UnsupportedFile(String),
    /// 底层 IO 错误。
    Io(String),
    /// 二进制/非 UTF-8 或坐标不可写(保留旧文案形状)。
    Unwritable(String),
    /// `query` 是空字符串。
    EmptyQuery,
    /// 超出本轮显式预算,结果不完整。
    Limit(String),
}

fn map_file_error(error: FileError) -> LocateError {
    match error {
        FileError::Limit(reason) => LocateError::Limit(reason),
        FileError::PathEscape => LocateError::OutOfBounds,
        FileError::SymlinkRejected => LocateError::SymlinkRejected,
        FileError::NotFound => LocateError::NotFound,
        FileError::UnsupportedFile(reason) => LocateError::UnsupportedFile(reason),
        FileError::UnsupportedMetadata(reason) => LocateError::Unwritable(reason),
        FileError::Conflict => LocateError::Unwritable("文件在编辑期间被其他写入修改".into()),
        FileError::LockTimeout => LocateError::Unwritable("等待文件写锁超时".into()),
        FileError::Io(error) => LocateError::Io(error.to_string()),
    }
}

fn open_root(project_root: &Path) -> Result<ProjectRoot, LocateError> {
    ProjectRoot::open(project_root).map_err(map_file_error)
}

/// 兼容入口:把相对路径解析成绝对路径。内部已通过受控打开校验路径边界和
/// 符号链接;调用方不应在之后按这个字符串再次打开(那会重新暴露替换竞态)。
pub fn resolve_project_path(project_root: &Path, rel_path: &str) -> Result<PathBuf, LocateError> {
    let root = open_root(project_root)?;
    root.open_file(rel_path).map_err(map_file_error)?;
    let parent = root.parent_handle(rel_path).map_err(map_file_error)?;
    Ok(parent.parent_path.join(parent.name()))
}

/// 把 0-based 字节偏移转成 1-based (line, column);`column` 按 Unicode
/// 标量值计数。保留给少数一次性调用;`locate_in_file` 使用单向游标避免
/// 每次命中从头扫描。
fn offset_to_line_col(text: &str, byte_offset: usize) -> (u32, u32) {
    let mut line = 1u32;
    let mut last_newline_byte = 0usize;
    for (i, b) in text.as_bytes()[..byte_offset].iter().enumerate() {
        if *b == b'\n' {
            line += 1;
            last_newline_byte = i + 1;
        }
    }
    let col = text[last_newline_byte..byte_offset].chars().count() as u32 + 1;
    (line, col)
}

/// 单向游标:按字节递增维护 1-based 行号/列号与当前行起点。匹配按位置
/// 递增遍历,因此整体是 `O(N + 返回内容)`,不会为每个命中从文件头重扫。
struct LineCursor {
    byte: usize,
    line: u32,
    col: u32,
    line_start: usize,
}

impl LineCursor {
    fn new() -> Self {
        Self {
            byte: 0,
            line: 1,
            col: 1,
            line_start: 0,
        }
    }

    fn advance_to(&mut self, text: &str, target: usize) {
        let bytes = text.as_bytes();
        let mut index = self.byte;
        while index < target {
            if bytes[index] == b'\n' {
                self.line += 1;
                self.col = 1;
                self.line_start = index + 1;
            } else if text.is_char_boundary(index) {
                self.col += 1;
            }
            index += 1;
        }
        self.byte = target;
    }
}

/// 把 context 裁到预算内;裁剪不改变真实命中坐标,但用显式标记说明不完整。
fn bounded_context(text: &str, start: usize, end: usize) -> String {
    let context = &text[start..end];
    let mut count = 0usize;
    for (byte_index, _) in context.char_indices() {
        if count == MAX_CONTEXT_SCALARS {
            return format!("{}…[context truncated]", &context[..byte_index]);
        }
        count += 1;
    }
    context.to_string()
}

pub fn locate_in_file(
    project_root: &Path,
    rel_path: &str,
    query: &str,
) -> Result<Vec<LocateMatch>, LocateError> {
    let report = locate_in_file_report(project_root, rel_path, query)?;
    if !report.complete {
        return Err(LocateError::Limit(
            report.limit_reason.unwrap_or_else(|| "结果超出预算".into()),
        ));
    }
    Ok(report.matches)
}

/// LocateV2:预算内返回已知前缀,并显式声明 `complete/limit_reason/revision`。
pub fn locate_in_file_report(
    project_root: &Path,
    rel_path: &str,
    query: &str,
) -> Result<LocateReport, LocateError> {
    if query.is_empty() {
        return Err(LocateError::EmptyQuery);
    }
    let root = open_root(project_root)?;
    let opened = root.open_file(rel_path).map_err(map_file_error)?;
    let revision = Revision::of(&opened.metadata);
    if opened.metadata.len > MAX_LOCATE_FILE_BYTES {
        return Err(LocateError::Limit(format!(
            "文件超过 locate 输入上限 {} 字节",
            MAX_LOCATE_FILE_BYTES
        )));
    }
    let bytes = opened
        .read_limited(MAX_LOCATE_FILE_BYTES as usize)
        .map_err(map_file_error)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| LocateError::UnsupportedFile("非 UTF-8 或二进制文件".into()))?;

    let mut matches = Vec::new();
    let mut cursor = LineCursor::new();
    // 同一行的大量命中复用行边界,避免每次 `rfind`/`find` 重新扫整行。
    let mut context_cache: Option<(usize, usize)> = None;
    let mut response_bytes = 0usize;
    let mut limit_reason: Option<String> = None;

    for (byte_start, _) in text.match_indices(query) {
        if matches.len() >= MAX_LOCATE_MATCHES {
            limit_reason = Some(format!("命中超过 {} 条,结果不完整", MAX_LOCATE_MATCHES));
            break;
        }
        let byte_end = byte_start + query.len();
        cursor.advance_to(&text, byte_start);
        let (start_line, start_col) = (cursor.line, cursor.col);
        let context_start = cursor.line_start;
        cursor.advance_to(&text, byte_end);
        let (end_line, end_col) = (cursor.line, cursor.col);

        let (context_start, context_end) = match context_cache {
            Some((cached_start, cached_end))
                if byte_start >= cached_start && byte_end <= cached_end =>
            {
                (cached_start, cached_end)
            }
            _ => {
                let context_end = text[byte_end..]
                    .find('\n')
                    .map(|offset| byte_end + offset)
                    .unwrap_or(text.len());
                context_cache = Some((context_start, context_end));
                (context_start, context_end)
            }
        };
        let context = bounded_context(&text, context_start, context_end);
        let candidate = LocateMatch {
            start_line,
            start_col,
            end_line,
            end_col,
            context,
        };
        let encoded_size = serde_json::to_vec(&candidate)
            .map_err(|e| LocateError::Io(e.to_string()))?
            .len();
        let next_total = response_bytes.saturating_add(encoded_size + 1);
        // Reserve the report/reply envelope, revision and bounded limit reason.
        if next_total.saturating_add(1024) > MAX_LOCATE_RESPONSE_BYTES {
            limit_reason = Some(format!(
                "locate 回复超过 {} 字节预算,结果不完整",
                MAX_LOCATE_RESPONSE_BYTES
            ));
            break;
        }
        response_bytes = next_total;
        matches.push(candidate);
    }
    Ok(LocateReport {
        matches,
        complete: limit_reason.is_none(),
        limit_reason,
        revision: Some(revision.value()),
    })
}

pub struct ApplyEditInput {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub expected_text: String,
    pub new_text: String,
}

#[derive(Debug)]
pub struct AppliedEdit {
    pub new_start_line: u32,
    pub new_start_col: u32,
    pub new_end_line: u32,
    pub new_end_col: u32,
    pub old_text: String,
}

/// 把 1-based (line, column) 转成字节偏移;越界/坐标非法时返回 `None`。
fn line_col_to_offset(text: &str, line: u32, col: u32) -> Option<usize> {
    if line == 0 || col == 0 {
        return None;
    }
    let mut current_line = 1u32;
    let mut line_start = 0usize;
    if line > 1 {
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                current_line += 1;
                if current_line == line {
                    line_start = i + 1;
                    break;
                }
            }
        }
        if current_line != line {
            return None;
        }
    }
    let line_end = text[line_start..]
        .find('\n')
        .map(|i| line_start + i)
        .unwrap_or(text.len());
    let line_text = &text[line_start..line_end];
    for (count, (byte_i, _ch)) in line_text.char_indices().enumerate() {
        if count as u32 + 1 == col {
            return Some(line_start + byte_i);
        }
    }
    if line_text.chars().count() as u32 + 1 == col {
        Some(line_start + line_text.len())
    } else {
        None
    }
}

/// 精确替换 `[start_line,start_col]`~`[end_line,end_col]` 区间。外层 `Result`
/// 是调用本身合不合法;内层 `Err(String)` 表示 `expected_text` 与磁盘实际
/// 内容不一致(附带磁盘实际内容),此时不写盘。
///
/// 写入使用受控打开 + 跨进程写锁 + 原子替换,并保持权限/元数据。
pub fn apply_precise_edit(
    project_root: &Path,
    rel_path: &str,
    input: ApplyEditInput,
) -> Result<Result<AppliedEdit, String>, LocateError> {
    let root = open_root(project_root)?;
    // 极端情况下(读后立刻有遵守同一把锁的写入)重试一次;仍冲突则明确失败。
    for attempt in 0..2 {
        let opened = root.open_file(rel_path).map_err(map_file_error)?;
        let revision = Revision::of(&opened.metadata);
        let bytes = opened
            .read_limited(MAX_LOCATE_FILE_BYTES as usize)
            .map_err(map_file_error)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| LocateError::UnsupportedFile("非 UTF-8 或二进制文件".into()))?;

        let start = line_col_to_offset(&text, input.start_line, input.start_col)
            .ok_or_else(|| LocateError::Unwritable("坐标超出文件范围".into()))?;
        let end = line_col_to_offset(&text, input.end_line, input.end_col)
            .ok_or_else(|| LocateError::Unwritable("坐标超出文件范围".into()))?;
        if start > end {
            return Err(LocateError::Unwritable(
                "start 坐标必须不晚于 end 坐标".into(),
            ));
        }

        let actual = &text[start..end];
        if actual != input.expected_text {
            return Ok(Err(actual.to_string()));
        }

        let new_len = text.len() - (end - start) + input.new_text.len();
        if new_len > MAX_LOCATE_FILE_BYTES as usize {
            return Err(LocateError::Limit(
                "edited file exceeds 16MiB budget".into(),
            ));
        }
        let mut new_content = String::with_capacity(new_len);
        new_content.push_str(&text[..start]);
        new_content.push_str(&input.new_text);
        new_content.push_str(&text[end..]);

        let options = WriteOptions {
            expected_revision: Some(revision),
            create: false,
            ..WriteOptions::default()
        };
        match write_atomic(&root, rel_path, new_content.as_bytes(), &options) {
            Ok(AtomicWriteOutcome::Committed { .. })
            | Ok(AtomicWriteOutcome::CommittedDurabilityUnknown { .. }) => {
                let new_end_byte = start + input.new_text.len();
                let (new_start_line, new_start_col) = offset_to_line_col(&new_content, start);
                let (new_end_line, new_end_col) = offset_to_line_col(&new_content, new_end_byte);
                return Ok(Ok(AppliedEdit {
                    new_start_line,
                    new_start_col,
                    new_end_line,
                    new_end_col,
                    old_text: actual.to_string(),
                }));
            }
            Err(AtomicWriteError::NotCommitted(FileError::Conflict)) if attempt == 0 => continue,
            Err(AtomicWriteError::NotCommitted(error)) => {
                return Err(map_file_error(error));
            }
            Err(AtomicWriteError::CommittedDurabilityUnknown { warning, .. }) => {
                return Err(LocateError::Unwritable(format!(
                    "内容已提交但持久性未确认: {warning}"
                )));
            }
        }
    }
    Err(LocateError::Unwritable(
        "文件在编辑期间被其他写入反复修改".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn locate_unique_match_returns_correct_coordinates() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "line one\nline two\nline three\n").unwrap();
        let matches = locate_in_file(dir.path(), "a.txt", "line two").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].start_line, 2);
        assert_eq!(matches[0].start_col, 1);
        assert_eq!(matches[0].end_line, 2);
        assert_eq!(matches[0].end_col, 9);
    }

    #[test]
    fn locate_multiple_matches_returns_all_candidates() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "foo\nfoo\nbar\n").unwrap();
        let matches = locate_in_file(dir.path(), "a.txt", "foo").unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].start_line, 1);
        assert_eq!(matches[1].start_line, 2);
    }

    #[test]
    fn locate_rejects_empty_query() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "content\n").unwrap();
        let err = locate_in_file(dir.path(), "a.txt", "").unwrap_err();
        assert!(matches!(err, LocateError::EmptyQuery));
    }

    #[test]
    fn locate_rejects_path_traversal() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "content\n").unwrap();
        let err = locate_in_file(dir.path(), "../a.txt", "content").unwrap_err();
        assert!(matches!(err, LocateError::OutOfBounds));

        let err2 = locate_in_file(dir.path(), "/etc/passwd", "root").unwrap_err();
        assert!(matches!(err2, LocateError::OutOfBounds));
    }

    #[test]
    fn locate_missing_file_returns_not_found() {
        let dir = project();
        let err = locate_in_file(dir.path(), "does-not-exist.txt", "x").unwrap_err();
        assert!(matches!(err, LocateError::NotFound));
    }

    #[test]
    fn locate_rejects_non_utf8_file() {
        let dir = project();
        fs::write(dir.path().join("bin.dat"), [0xFF, 0xFE, 0x00, 0x01]).unwrap();
        let err = locate_in_file(dir.path(), "bin.dat", "x").unwrap_err();
        assert!(matches!(err, LocateError::UnsupportedFile(_)));
    }

    #[test]
    fn apply_edit_replaces_range_and_computes_new_coordinates() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "line one\nline two\nline three\n").unwrap();
        let result = apply_precise_edit(
            dir.path(),
            "a.txt",
            ApplyEditInput {
                start_line: 2,
                start_col: 1,
                end_line: 2,
                end_col: 9,
                expected_text: "line two".into(),
                new_text: "replaced".into(),
            },
        )
        .unwrap();
        let applied = result.expect("不应冲突");
        assert_eq!(applied.new_start_line, 2);
        assert_eq!(applied.new_start_col, 1);
        assert_eq!(applied.new_end_line, 2);
        assert_eq!(applied.new_end_col, 9); // "replaced" 长度同为 8
        assert_eq!(applied.old_text, "line two");

        let on_disk = fs::read_to_string(dir.path().join("a.txt")).unwrap();
        assert_eq!(on_disk, "line one\nreplaced\nline three\n");
    }

    #[test]
    fn apply_edit_handles_line_count_change_in_new_coordinates() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "keep\nreplace me\nkeep too\n").unwrap();
        let result = apply_precise_edit(
            dir.path(),
            "a.txt",
            ApplyEditInput {
                start_line: 2,
                start_col: 1,
                end_line: 2,
                end_col: 11,
                expected_text: "replace me".into(),
                new_text: "one\ntwo\nthree".into(),
            },
        )
        .unwrap()
        .expect("不应冲突");
        assert_eq!(result.new_start_line, 2);
        assert_eq!(result.new_start_col, 1);
        assert_eq!(result.new_end_line, 4);
        assert_eq!(result.new_end_col, 6); // "three" 长度 5

        let on_disk = fs::read_to_string(dir.path().join("a.txt")).unwrap();
        assert_eq!(on_disk, "keep\none\ntwo\nthree\nkeep too\n");
    }

    #[test]
    fn apply_edit_conflict_when_expected_text_mismatches() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "actual content\n").unwrap();
        let result = apply_precise_edit(
            dir.path(),
            "a.txt",
            ApplyEditInput {
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 15,
                expected_text: "stale content".into(),
                new_text: "new".into(),
            },
        )
        .unwrap();
        let conflict = result.expect_err("应产生冲突");
        assert_eq!(conflict, "actual content");
        // 冲突时不应写盘。
        let on_disk = fs::read_to_string(dir.path().join("a.txt")).unwrap();
        assert_eq!(on_disk, "actual content\n");
    }

    #[test]
    fn apply_edit_rejects_reversed_coordinates() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "line one\nline two\n").unwrap();
        let err = apply_precise_edit(
            dir.path(),
            "a.txt",
            ApplyEditInput {
                start_line: 2,
                start_col: 1,
                end_line: 1,
                end_col: 1,
                expected_text: String::new(),
                new_text: "x".into(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, LocateError::Unwritable(_)));
    }

    #[test]
    fn apply_edit_rejects_out_of_range_coordinates() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "only one line\n").unwrap();
        let err = apply_precise_edit(
            dir.path(),
            "a.txt",
            ApplyEditInput {
                start_line: 99,
                start_col: 1,
                end_line: 99,
                end_col: 5,
                expected_text: String::new(),
                new_text: "x".into(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, LocateError::Unwritable(_)));
    }

    #[test]
    fn apply_edit_rejects_path_traversal() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "content\n").unwrap();
        let err = apply_precise_edit(
            dir.path(),
            "../a.txt",
            ApplyEditInput {
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 1,
                expected_text: String::new(),
                new_text: "x".into(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, LocateError::OutOfBounds));
    }

    #[cfg(unix)]
    #[test]
    fn terminal_symlink_cannot_read_or_edit_outside_file() {
        use std::os::unix::fs::symlink;
        let dir = project();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("outside.txt");
        fs::write(&outside_file, "old text\n").unwrap();
        symlink(&outside_file, dir.path().join("link.txt")).unwrap();

        let locate_err = locate_in_file(dir.path(), "link.txt", "old").unwrap_err();
        assert!(matches!(locate_err, LocateError::SymlinkRejected));

        let edit_err = apply_precise_edit(
            dir.path(),
            "link.txt",
            ApplyEditInput {
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 9,
                expected_text: "old text".into(),
                new_text: "NEW text".into(),
            },
        )
        .unwrap_err();
        assert!(matches!(edit_err, LocateError::SymlinkRejected));
        assert_eq!(fs::read_to_string(&outside_file).unwrap(), "old text\n");
    }

    #[cfg(unix)]
    #[test]
    fn parent_symlink_cannot_read_outside_file() {
        use std::os::unix::fs::symlink;
        let dir = project();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("a.txt"), "outside\n").unwrap();
        symlink(outside.path(), dir.path().join("sub")).unwrap();
        let err = locate_in_file(dir.path(), "sub/a.txt", "outside").unwrap_err();
        assert!(matches!(err, LocateError::SymlinkRejected));
    }

    #[test]
    fn locate_coordinates_use_unicode_scalars_and_keep_half_open_range() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "😀 中文\nnext\n").unwrap();
        let matches = locate_in_file(dir.path(), "a.txt", "中文").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].start_line, 1);
        assert_eq!(matches[0].start_col, 3); // 😀 = 1 scalar, space = 1
        assert_eq!(matches[0].end_line, 1);
        assert_eq!(matches[0].end_col, 5);
    }

    #[test]
    fn locate_multi_line_query_keeps_start_and_end_coordinates() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        let matches = locate_in_file(dir.path(), "a.txt", "one\ntwo").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!((matches[0].start_line, matches[0].start_col), (1, 1));
        assert_eq!((matches[0].end_line, matches[0].end_col), (2, 4));
        assert_eq!(matches[0].context, "one\ntwo");
    }

    #[test]
    fn locate_many_matches_on_one_long_line_keeps_coordinates_and_context() {
        let dir = project();
        let line = "x".repeat(MAX_LOCATE_MATCHES - 1);
        fs::write(dir.path().join("a.txt"), &line).unwrap();
        let report = locate_in_file_report(dir.path(), "a.txt", "x").unwrap();
        assert!(!report.complete);
        assert!(matches!(
            locate_in_file(dir.path(), "a.txt", "x"),
            Err(LocateError::Limit(_))
        ));
        let matches = report.matches;
        assert!(!matches.is_empty());
        assert!(matches.len() < MAX_LOCATE_MATCHES - 1);
        assert_eq!(matches.last().unwrap().start_col, matches.len() as u32);
        assert_eq!(matches[0].context, line);
        assert!(matches.iter().all(|m| m.context == line));
    }

    #[test]
    fn locate_truncates_context_without_rescanning_whole_long_line() {
        let dir = project();
        let text = format!("{}x", "y".repeat(20_000));
        fs::write(dir.path().join("a.txt"), text).unwrap();
        let matches = locate_in_file(dir.path(), "a.txt", "x").unwrap();
        assert_eq!(matches.len(), 1);
        assert!(matches[0].context.ends_with("…[context truncated]"));
        assert!(
            matches[0].context.chars().count() <= MAX_CONTEXT_SCALARS + 30,
            "context 只应保留预算内前缀 + 标记"
        );
    }

    #[test]
    fn locate_rejects_over_limit_match_count_instead_of_pretending_complete() {
        let dir = project();
        let mut text = String::new();
        for _ in 0..=MAX_LOCATE_MATCHES {
            text.push_str("x\n");
        }
        fs::write(dir.path().join("a.txt"), text).unwrap();
        let err = locate_in_file(dir.path(), "a.txt", "x").unwrap_err();
        assert!(matches!(err, LocateError::Limit(_)));

        let report = locate_in_file_report(dir.path(), "a.txt", "x").unwrap();
        assert!(!report.complete);
        assert_eq!(report.matches.len(), MAX_LOCATE_MATCHES);
        assert!(report.limit_reason.is_some());
        assert!(report.revision.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn edit_preserves_executable_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = project();
        let path = dir.path().join("run.sh");
        fs::write(&path, "echo old\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        apply_precise_edit(
            dir.path(),
            "run.sh",
            ApplyEditInput {
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 9,
                expected_text: "echo old".into(),
                new_text: "echo new".into(),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "echo new\n");
    }
    #[test]
    fn locate_counts_json_escaping_in_its_reply_budget() {
        let dir = project();
        let line = "\0".repeat(1024) + "x\n";
        fs::write(dir.path().join("a.txt"), line.repeat(200)).unwrap();
        let report = locate_in_file_report(dir.path(), "a.txt", "x").unwrap();
        assert!(!report.complete);
        assert!(!report.matches.is_empty());
        assert!(serde_json::to_vec(&report).unwrap().len() + 128 <= MAX_LOCATE_RESPONSE_BYTES);
    }
}
