//! Agent 精确修改的核心逻辑(v0.1 意向文档里的 Mutation Engine,TextAdapter
//! 一种实现):路径解析/边界校验、UTF-8 校验、`locate_in_file` 搜索、
//! `apply_precise_edit` 的 Conflict Detection + 写盘 + 坐标重算。纯逻辑,不
//! 依赖任何 MCP/wire 类型,方便直接用 tempdir 测试。

use dozer_core::protocol::LocateMatch;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub enum LocateError {
    /// 路径落在项目根目录子树之外。
    OutOfBounds,
    /// 目标文件不存在。
    NotFound,
    /// 二进制/非 UTF-8,内容原因见携带的字符串。
    Unwritable(String),
    /// `query` 是空字符串。
    EmptyQuery,
}

/// 把 `rel_path`(agent 传入的项目内相对路径)解析成绝对路径,并校验落在
/// `project_root` 子树内。**边界校验基于父目录的 `canonicalize` 结果,不是
/// 目标文件本身**——如果对整条拼好的路径直接 `canonicalize`,一个越界但目标
/// 文件恰好不存在的路径(比如 `../nonexistent.txt`)会因为 `canonicalize`
/// 对不存在路径报错而被误判成 `NotFound`,而不是这次真正该报的
/// `OutOfBounds`,让越界检查形同虚设。父目录(`../` 之类的 `..` 段都在这一步
/// 被解析掉)通常总是存在的,`starts_with` 校验只针对父目录做,和目标文件
/// 存不存在无关;目标文件是否存在放在这之后单独判断,统一映射成
/// `NotFound`。符号链接逃逸同样被父目录的 `canonicalize` 挡住。
pub fn resolve_project_path(project_root: &Path, rel_path: &str) -> Result<PathBuf, LocateError> {
    let root = project_root
        .canonicalize()
        .map_err(|_| LocateError::OutOfBounds)?;
    let joined = project_root.join(rel_path);
    let parent = joined.parent().ok_or(LocateError::OutOfBounds)?;
    let parent_real = parent
        .canonicalize()
        .map_err(|_| LocateError::OutOfBounds)?;
    if !parent_real.starts_with(&root) {
        return Err(LocateError::OutOfBounds);
    }
    let file_name = joined.file_name().ok_or(LocateError::OutOfBounds)?;
    let resolved = parent_real.join(file_name);
    if !resolved.is_file() {
        return Err(LocateError::NotFound);
    }
    Ok(resolved)
}

/// 读文件并校验是合法 UTF-8;二进制/非 UTF-8 一律拒绝(比 GUI 那套 lossy 编码
/// 检测更严格——Phase 1 daemon 侧不复用 GUI 的编码探测栈,宁可对某些 GUI 能
/// lossy 打开的文件也拒绝写,不做更复杂的探测)。
fn read_utf8(path: &Path) -> Result<String, LocateError> {
    let bytes = std::fs::read(path).map_err(|_| LocateError::NotFound)?;
    String::from_utf8(bytes).map_err(|_| LocateError::Unwritable("非 UTF-8 或二进制文件".into()))
}

/// 把 0-based 字节偏移转成 1-based (line, column);`column` 按 Unicode
/// 标量值(`chars().count()`)计数,不是 UTF-16 code unit——已知限制见计划的
/// Global Constraints。
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

pub fn locate_in_file(
    project_root: &Path,
    rel_path: &str,
    query: &str,
) -> Result<Vec<LocateMatch>, LocateError> {
    if query.is_empty() {
        return Err(LocateError::EmptyQuery);
    }
    let path = resolve_project_path(project_root, rel_path)?;
    let text = read_utf8(&path)?;
    let mut matches = Vec::new();
    for (byte_start, _) in text.match_indices(query) {
        let byte_end = byte_start + query.len();
        let (start_line, start_col) = offset_to_line_col(&text, byte_start);
        let (end_line, end_col) = offset_to_line_col(&text, byte_end);
        let context_start = text[..byte_start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let context_end = text[byte_end..]
            .find('\n')
            .map(|i| byte_end + i)
            .unwrap_or(text.len());
        matches.push(LocateMatch {
            start_line,
            start_col,
            end_line,
            end_col,
            context: text[context_start..context_end].to_string(),
        });
    }
    Ok(matches)
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

/// 把 1-based (line, column) 转成字节偏移;越界/坐标非法时返回 `None`,调用方
/// 统一映射成 `LocateError::Unwritable`(和"文件类型不可写"共用同一个变体——
/// 从调用方视角都是"这次编辑请求本身有问题,不是环境/权限问题")。
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

/// 精确替换 `[start_line,start_col]`~`[end_line,end_col]` 区间。返回值的外层
/// `Result` 是"这次调用本身合不合法"(路径/坐标/编码),内层 `Result` 是
/// Conflict Detection 的结果:`Ok(AppliedEdit)` 表示已成功写盘,`Err(String)`
/// 表示 `expected_text` 跟磁盘实际内容不一致(附带磁盘实际内容),这种情况下
/// **不写盘**。
pub fn apply_precise_edit(
    project_root: &Path,
    rel_path: &str,
    input: ApplyEditInput,
) -> Result<Result<AppliedEdit, String>, LocateError> {
    let path = resolve_project_path(project_root, rel_path)?;
    let text = read_utf8(&path)?;

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

    let mut new_content = String::with_capacity(text.len() - (end - start) + input.new_text.len());
    new_content.push_str(&text[..start]);
    new_content.push_str(&input.new_text);
    new_content.push_str(&text[end..]);
    std::fs::write(&path, &new_content)
        .map_err(|e| LocateError::Unwritable(format!("写盘失败: {e}")))?;

    let new_end_byte = start + input.new_text.len();
    let (new_start_line, new_start_col) = offset_to_line_col(&new_content, start);
    let (new_end_line, new_end_col) = offset_to_line_col(&new_content, new_end_byte);

    Ok(Ok(AppliedEdit {
        new_start_line,
        new_start_col,
        new_end_line,
        new_end_col,
        old_text: actual.to_string(),
    }))
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
        assert!(matches!(err, LocateError::Unwritable(_)));
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
}
