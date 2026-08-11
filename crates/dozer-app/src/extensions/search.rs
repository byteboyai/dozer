//! 文件树右键"搜索"弹窗：作用域(目录子树/单文件)内的全文内容搜索。瞬态弹窗，
//! 不挂 `LeftView`/左侧图标栏，形制参考文件编辑弹层 `edit_modal`。不做搜索历史/
//! 索引/后台预扫描，见
//! `docs/superpowers/specs/2026-08-12-tree-search-in-context-menu-design.md`。

use grep_regex::RegexMatcherBuilder;
use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkMatch};
use std::path::{Path, PathBuf};

/// 搜索作用域：右键目标。
#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    /// 目录整棵子树。
    Dir(PathBuf),
    /// 单文件。
    File(PathBuf),
}

/// 一条命中。
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub path: PathBuf,
    pub line_no: u64,
    pub line_text: String,
}

/// 收集单条命中行的 `Sink`：拷出路径/行号/命中行文本。
struct HitSink {
    path: PathBuf,
    hits: Vec<SearchHit>,
}

impl Sink for HitSink {
    type Error = std::io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        // `mat.bytes()` 是整行(含尾随换行的原始字节),直接 lossy 转成展示文本。
        self.hits.push(SearchHit {
            path: self.path.clone(),
            line_no: mat.line_number().unwrap_or(0),
            line_text: String::from_utf8_lossy(mat.bytes()).into_owned(),
        });
        Ok(true)
    }
}

/// 在 scope 内做字面子串(大小写不敏感)搜索，按文件分组返回，组内按行号升序。
/// 返回 `Vec<(绝对路径字符串, 该文件的命中)>`，键恒为整段绝对路径，相对展示交给
/// view 层用 `project_root` 换算。空查询直接返回空结果，不发起搜索。
///
/// 目录作用域用 `ignore::WalkBuilder` 递归(尊重 `.gitignore` 与隐藏文件;
/// `require_git(false)` 让目录即便不在 git 仓库内也应用根目录的 `.gitignore`,
/// 同 ripgrep 的 `--no-require-git` 口径);
/// 文件作用域只搜那一个文件。二进制/非 UTF-8 行用 lossy 转换，不 panic。
pub fn search_scope(scope: &Scope, query: &str) -> Result<Vec<(String, Vec<SearchHit>)>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let matcher = grep_regex::RegexMatcherBuilder::new()
        .case_insensitive(true)
        .build(query)
        .map_err(|e| format!("搜索模式无效: {e}"))?;

    let mut searcher = SearcherBuilder::new().line_number(true).build();

    // 作用域内所有待搜文件(绝对路径)。
    let mut files: Vec<PathBuf> = Vec::new();
    match scope {
        Scope::File(p) => files.push(p.clone()),
        Scope::Dir(p) => {
            for entry in ignore::WalkBuilder::new(p).require_git(false).build() {
                match entry {
                    Ok(de) if de.file_type().map(|ft| ft.is_file()).unwrap_or(false) => {
                        files.push(de.path().to_path_buf());
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("搜索遍历跳过错误条目: {e}");
                    }
                }
            }
        }
    }

    // 已发现命中文件的路径 → 其结果组(用绝对路径字符串作键)。
    let mut by_file: std::collections::BTreeMap<String, Vec<SearchHit>> =
        std::collections::BTreeMap::new();
    for path in files {
        let mut sink = HitSink {
            path: path.clone(),
            hits: Vec::new(),
        };
        if searcher.search_path(&matcher, &path, &mut sink).is_err() {
            continue; // 读不了的(权限/消失)跳过,不 panic
        }
        if !sink.hits.is_empty() {
            by_file.insert(path.display().to_string(), sink.hits);
        }
    }
    Ok(by_file.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &str) -> PathBuf {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn empty_query_yields_no_results() {
        let dir = tempfile::tempdir().unwrap();
        let f = write(dir.path(), "a.txt", "needle\n");
        let hits = search_scope(&Scope::File(f), "  ").unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn single_file_matches_with_line_number() {
        let dir = tempfile::tempdir().unwrap();
        let f = write(dir.path(), "a.txt", "first line\nneedle here\nthird line\n");
        let hits = search_scope(&Scope::File(f), "needle").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1.len(), 1);
        assert_eq!(hits[0].1[0].line_no, 2);
        assert!(hits[0].1[0].line_text.contains("needle here"));
    }

    #[test]
    fn search_is_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let f = write(dir.path(), "a.txt", "HELLO world\n");
        let hits = search_scope(&Scope::File(f), "hello").unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn dir_scope_recurses_and_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".gitignore", "ignored.txt\n");
        write(dir.path(), "keep.txt", "needle keep\n");
        write(dir.path(), "ignored.txt", "needle ignored\n");
        write(dir.path(), "nested/sub.txt", "needle nested\n");
        let hits = search_scope(&Scope::Dir(dir.path().to_path_buf()), "needle").unwrap();
        // 只命中 keep.txt 与 nested/sub.txt,忽略文件不参与。
        let keys: Vec<&str> = hits.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.iter().any(|k| k.ends_with("keep.txt")));
        assert!(keys.iter().any(|k| k.ends_with("nested/sub.txt")));
        assert!(!keys.iter().any(|k| k.ends_with("ignored.txt")));
    }

    #[test]
    fn binary_utf8_file_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let f = write(dir.path(), "bin.dat", "needle text\n");
        std::fs::write(&f, [0xFF, 0x21, b'\n']).unwrap(); // 非法 UTF-8 + 非 ASCII 第一字节
        let hits = search_scope(&Scope::File(f), "anything").unwrap();
        // 不 panic 即可;二进制内容不命中查询词,结果可为空。
        let _ = hits;
    }

    #[test]
    fn file_scope_does_not_recurse() {
        let dir = tempfile::tempdir().unwrap();
        let f = write(dir.path(), "a.txt", "needle\n");
        // 在 scope 外另外造一个会命中的文件,证明文件作用域不扫它。
        write(dir.path(), "b.txt", "needle b\n");
        let hits = search_scope(&Scope::File(f), "needle").unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].0.ends_with("a.txt"));
    }
}
