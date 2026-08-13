//! SFTP 文件传输(阶段 3):数据模型 + 驱动逻辑,独立子模块避免
//! `extensions/ssh.rs` 继续膨胀(镜像 `extensions/project/links.rs`
//! 的既有拆分先例)。

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
}

/// 远程文件树状态:展开集合 + 子项缓存,镜像 `crate::project::FileTree`
/// 的形状,但独立成自己的类型——本地是同步 `std::fs::read_dir`,远程是
/// 异步 SFTP `readdir` 结果回填,两者填充时机不同,不共用同一个类型。
#[derive(Debug, Default)]
pub struct RemoteTree {
    root: String,
    expanded: HashSet<String>,
    children: HashMap<String, Vec<RemoteEntry>>,
    /// 展开但读取失败的目录 → 错误文案,渲染层据此显示"⚠ 无法读取"而
    /// 不是无限 loading。
    errors: HashMap<String, String>,
}

impl RemoteTree {
    pub fn new(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            ..Self::default()
        }
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    /// 展开/收起某个远程目录。已展开 → 收起(返回 `None`,不需要重新
    /// 请求)。未展开且缓存里没有 → 展开并返回 `Some(path)`(调用方据此
    /// 发起一次异步 `readdir`)。未展开但已有缓存(比如收起后再展开)
    /// → 展开但返回 `None`(不重复请求,直接用缓存)。
    pub fn toggle(&mut self, dir: &str) -> Option<String> {
        if self.expanded.remove(dir) {
            return None;
        }
        self.expanded.insert(dir.to_string());
        if self.children.contains_key(dir) {
            None
        } else {
            Some(dir.to_string())
        }
    }

    /// 异步 `readdir` 结果回填。`Err` 时记进 `errors`,不清 `expanded`
    /// (目录仍显示为"已展开",只是子项区域显示错误文案)。
    pub fn set_children(&mut self, dir: &str, result: Result<Vec<RemoteEntry>, String>) {
        self.errors.remove(dir);
        match result {
            Ok(mut entries) => {
                entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.name.cmp(&b.name),
                });
                self.children.insert(dir.to_string(), entries);
            }
            Err(e) => {
                self.errors.insert(dir.to_string(), e);
            }
        }
    }

    /// 渲染用可见行,形状对齐 `crate::project::TreeRow`,复用同一个行
    /// 渲染函数(见 Task 5)。深度优先遍历 `expanded` 集合,只展开
    /// `expanded` 里存在的目录。
    pub fn visible_rows(&self) -> Vec<crate::project::TreeRow> {
        let mut rows = Vec::new();
        self.push_children(&self.root, 0, &mut rows);
        rows
    }

    fn push_children(&self, dir: &str, depth: usize, out: &mut Vec<crate::project::TreeRow>) {
        let Some(entries) = self.children.get(dir) else {
            return;
        };
        for entry in entries {
            let expanded = self.expanded.contains(&entry.path);
            out.push(crate::project::TreeRow {
                path: std::path::PathBuf::from(&entry.path),
                name: entry.name.clone(),
                depth,
                is_dir: entry.is_dir,
                expanded,
            });
            if entry.is_dir && expanded {
                self.push_children(&entry.path, depth + 1, out);
            }
        }
    }

    /// 某个已展开目录读取失败时的错误文案。
    pub fn error_for(&self, dir: &str) -> Option<&str> {
        self.errors.get(dir).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, name: &str, is_dir: bool) -> RemoteEntry {
        RemoteEntry {
            path: path.to_string(),
            name: name.to_string(),
            is_dir,
        }
    }

    #[test]
    fn toggle_expands_and_requests_on_first_open() {
        let mut tree = RemoteTree::new("/root");
        assert_eq!(tree.toggle("/root"), Some("/root".to_string()));
    }

    #[test]
    fn toggle_collapses_without_requesting() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        assert_eq!(tree.toggle("/root"), None); // 收起
    }

    #[test]
    fn toggle_reopen_with_cache_does_not_request() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children("/root", Ok(vec![entry("/root/a", "a", false)]));
        tree.toggle("/root"); // 收起
        assert_eq!(tree.toggle("/root"), None); // 重新展开,缓存还在,不重复请求
    }

    #[test]
    fn set_children_sorts_dirs_before_files_then_by_name() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children(
            "/root",
            Ok(vec![
                entry("/root/z.txt", "z.txt", false),
                entry("/root/bdir", "bdir", true),
                entry("/root/a.txt", "a.txt", false),
                entry("/root/adir", "adir", true),
            ]),
        );
        let rows = tree.visible_rows();
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["adir", "bdir", "a.txt", "z.txt"]);
    }

    #[test]
    fn set_children_err_recorded_and_does_not_panic_on_visible_rows() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children("/root", Err("权限拒绝".to_string()));
        assert_eq!(tree.error_for("/root"), Some("权限拒绝"));
        assert!(tree.visible_rows().is_empty()); // 没有子项缓存,可见行为空,不 panic
    }

    #[test]
    fn nested_expansion_shows_grandchildren() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children("/root", Ok(vec![entry("/root/sub", "sub", true)]));
        tree.toggle("/root/sub");
        tree.set_children("/root/sub", Ok(vec![entry("/root/sub/f", "f", false)]));
        let rows = tree.visible_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "sub");
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].name, "f");
        assert_eq!(rows[1].depth, 1);
    }

    #[test]
    fn collapsed_dir_hides_its_children_from_visible_rows() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children("/root", Ok(vec![entry("/root/sub", "sub", true)]));
        tree.toggle("/root/sub");
        tree.set_children("/root/sub", Ok(vec![entry("/root/sub/f", "f", false)]));
        tree.toggle("/root/sub"); // 收起
        let rows = tree.visible_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "sub");
    }
}
