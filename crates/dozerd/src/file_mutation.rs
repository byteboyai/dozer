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
    let parent_real = parent.canonicalize().map_err(|_| LocateError::OutOfBounds)?;
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
}
