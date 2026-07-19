//! 文件树状态机（P1g）：懒加载单目录、展开集、可见行摊平。纯数据，不碰
//! iced；展开时同步 read_dir（单目录快）。固定隐藏名单过滤。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// 文件树里恒不显示的目录/文件名。
pub const HIDDEN: [&str; 4] = [".git", "target", "node_modules", ".DS_Store"];

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
}

pub struct FileTree {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    children: HashMap<PathBuf, Vec<Entry>>,
}

/// 读一个目录:剔隐藏名单,目录在前、各自按名排序。read_dir 失败返回空。
fn read_children(dir: &Path) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if HIDDEN.contains(&name.as_str()) {
                return None;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some(Entry {
                path: e.path(),
                name,
                is_dir,
            })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    entries
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let mut children = HashMap::new();
        children.insert(root.clone(), read_children(&root));
        Self {
            root,
            expanded: HashSet::new(),
            children,
        }
    }

    /// 展开/收起一个目录。展开时若未缓存则同步读一次。
    pub fn toggle(&mut self, dir: &Path) {
        if self.expanded.remove(dir) {
            return; // 已展开 → 收起
        }
        self.children
            .entry(dir.to_path_buf())
            .or_insert_with(|| read_children(dir));
        // 空目录/不可读:缓存为空 vec,不标 expanded(无可展开内容)
        if self
            .children
            .get(dir)
            .map(|c| !c.is_empty())
            .unwrap_or(false)
        {
            self.expanded.insert(dir.to_path_buf());
        }
    }

    /// 从 root 的子项起 DFS 摊平成可见行（展开的目录才递归其子项）。
    pub fn visible_rows(&self) -> Vec<TreeRow> {
        let mut out = Vec::new();
        self.push_rows(&self.root, 0, &mut out);
        out
    }

    fn push_rows(&self, dir: &Path, depth: usize, out: &mut Vec<TreeRow>) {
        let Some(entries) = self.children.get(dir) else {
            return;
        };
        for e in entries {
            let expanded = self.expanded.contains(&e.path);
            out.push(TreeRow {
                path: e.path.clone(),
                name: e.name.clone(),
                depth,
                is_dir: e.is_dir,
                expanded,
            });
            if e.is_dir && expanded {
                self.push_rows(&e.path, depth + 1, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mktree() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir(r.join("src")).unwrap();
        std::fs::write(r.join("src/main.rs"), "").unwrap();
        std::fs::write(r.join("README.md"), "").unwrap();
        std::fs::create_dir(r.join(".git")).unwrap(); // 隐藏
        std::fs::create_dir(r.join("target")).unwrap(); // 隐藏
        d
    }

    #[test]
    fn root_rows_hide_and_sort() {
        let d = mktree();
        let t = FileTree::new(d.path().to_path_buf());
        let rows = t.visible_rows();
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src", "README.md"], "目录在前、隐藏名单剔除");
        assert!(rows[0].is_dir && !rows[0].expanded);
        assert_eq!(rows[0].depth, 0);
    }

    #[test]
    fn expand_shows_children_with_depth() {
        let d = mktree();
        let mut t = FileTree::new(d.path().to_path_buf());
        t.toggle(&d.path().join("src"));
        let rows = t.visible_rows();
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src", "main.rs", "README.md"]);
        let child = rows.iter().find(|r| r.name == "main.rs").unwrap();
        assert_eq!(child.depth, 1);
        // 再 toggle 收起
        t.toggle(&d.path().join("src"));
        assert_eq!(t.visible_rows().len(), 2);
    }

    #[test]
    fn unreadable_dir_does_not_panic() {
        let d = tempfile::tempdir().unwrap();
        let mut t = FileTree::new(d.path().to_path_buf());
        // toggle 一个不存在的子目录:不 panic,不产生子行
        t.toggle(&d.path().join("nope"));
        assert!(t.visible_rows().is_empty());
    }
}
