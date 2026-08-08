//! 文件树状态机（P1g）：懒加载单目录、展开集、可见行摊平。纯数据，不碰
//! iced；展开时同步 read_dir（单目录快）。固定隐藏名单过滤。

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// 复制路径展示形式：绝对路径 vs 相对项目根目录。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathKind {
    Absolute,
    Relative,
}

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

    /// 强制重读一个目录的子项缓存（增删改后调用，让 `visible_rows()`
    /// 反映最新磁盘状态）。目录本身若已展开保持展开；未展开的话，本次
    /// 调用不强行展开，只刷新缓存——下次展开时自然是最新的。
    pub fn refresh(&mut self, dir: &Path) {
        self.children.insert(dir.to_path_buf(), read_children(dir));
    }

    /// 从磁盘整体重新加载:重读每一个已缓存过的目录(含已收起的——`toggle`
    /// 展开时若缓存已存在不会重读,收起目录的缓存不刷新就会在重新展开时
    /// 冒出陈旧内容),目录若已在磁盘上消失则连同其展开态一并丢弃。
    /// 不重建整棵树(不清 `expanded`),用户当前展开的层级保持不变。
    pub fn reload_from_disk(&mut self) {
        let dirs: Vec<PathBuf> = self.children.keys().cloned().collect();
        for dir in dirs {
            if dir.is_dir() {
                self.children.insert(dir.clone(), read_children(&dir));
            } else {
                self.children.remove(&dir);
                self.expanded.remove(&dir);
            }
        }
    }

    /// 确保目录处于展开态（新建文件/文件夹前调用，让新项有可见位置）。
    /// 与 `toggle` 不同：无条件标记 expanded，哪怕目录当前是空的——
    /// 新建文件/文件夹的落点必须可见，即便"可见"只是一个空的展开态
    /// 目录（渲染零子行，不 crash、不视觉异常）。若走 `toggle` 的
    /// "空目录不标 expanded" 逻辑，右键空目录→新建，编辑框永远不会
    /// 出现在屏幕上，但键盘输入已经在悄悄写进不可见的编辑缓冲区。
    pub fn ensure_expanded(&mut self, dir: &Path) {
        self.children
            .entry(dir.to_path_buf())
            .or_insert_with(|| read_children(dir));
        self.expanded.insert(dir.to_path_buf());
    }

    /// 从 root 的子项起 DFS 摊平成可见行（展开的目录才递归其子项）。
    pub fn visible_rows(&self) -> Vec<TreeRow> {
        let mut out = Vec::new();
        self.push_rows(&self.root, 0, &mut out);
        out
    }

    /// 项目根目录（`FileTree::new` 传入的 `root`）。文件树面板用它显示
    /// "根目录名(完整路径)" 头部。
    pub fn root(&self) -> &Path {
        &self.root
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

/// 递归复制目录（文件树"粘贴"目标是目录时用）。目标已存在 → `Err`，不覆盖、
/// 不做部分复制后中止的脏状态清理（失败时目标目录可能已建但不完整——
/// 调用方(`paste_item`)已在动手前检查过存在性，这里的检查是防御递归过程中
/// 子目录层面的二次冲突）。
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if dst.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} 已存在", dst.display()),
        ));
    }
    std::fs::create_dir(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// 把 `source`（文件或目录）复制进 `target_dir` 下，用源的文件名。目标已
/// 存在同名项 → `Err`（冲突文案）。成功返回新建出的完整路径。
pub fn paste_item(
    source: &Path,
    source_is_dir: bool,
    target_dir: &Path,
) -> Result<PathBuf, String> {
    let dest = target_dir.join(source.file_name().unwrap_or_default());
    // 目录粘贴进自己或自己的子目录 → dest 落在 source 树内,
    // copy_dir_recursive 会边建目录边把刚建出来的目标又递归进去复制,
    // 无界自增长直至 ENAMETOOLONG/磁盘写满。文件不可能出现这种情况
    // (文件没有"子目录"可落入自身)。
    if source_is_dir && (dest.starts_with(source) || dest == source) {
        return Err("不能把目录粘贴到它自己或其子目录里".to_string());
    }
    if dest.exists() {
        return Err(format!("{} 已存在同名项", dest.display()));
    }
    let result = if source_is_dir {
        copy_dir_recursive(source, &dest)
    } else {
        std::fs::copy(source, &dest).map(|_| ())
    };
    result.map(|_| dest).map_err(|e| e.to_string())
}

/// 校验新建/重命名输入框里键入的名字是不是"单一正常路径分量"——不含
/// `/`、不是 `.`/`..`、非空。名字最终会被 `parent_dir.join(name)`
/// 直接拼成路径,若允许 `../x`/`foo/bar` 这类多段输入,拼出来的路径会
/// 跳出目标目录(重命名=把文件移出项目树,新建=建到意料之外的位置)。
pub fn is_single_path_component(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

/// 路径的展示字符串：绝对路径原样；相对路径去掉 `project_root` 前缀，
/// 若不在 `project_root` 之下（理论上树里的项恒在其下，此分支是防御性
/// 兜底）就退化成绝对路径。
pub fn path_string(kind: PathKind, path: &Path, project_root: &Path) -> String {
    match kind {
        PathKind::Absolute => path.display().to_string(),
        PathKind::Relative => path
            .strip_prefix(project_root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_picks_up_new_files_on_disk() {
        let d = mktree();
        let mut t = FileTree::new(d.path().to_path_buf());
        t.toggle(&d.path().join("src")); // 展开,缓存 src 的子项(此时只有 main.rs)
        std::fs::write(d.path().join("src/lib.rs"), "").unwrap();
        // 磁盘变了,refresh 前 visible_rows 还是旧缓存
        let before: Vec<_> = t.visible_rows().iter().map(|r| r.name.clone()).collect();
        assert!(!before.contains(&"lib.rs".to_string()));
        t.refresh(&d.path().join("src"));
        let after: Vec<_> = t.visible_rows().iter().map(|r| r.name.clone()).collect();
        assert!(after.contains(&"lib.rs".to_string()));
    }

    #[test]
    fn reload_from_disk_refreshes_collapsed_dir_cache() {
        let d = mktree();
        let mut t = FileTree::new(d.path().to_path_buf());
        t.toggle(&d.path().join("src")); // 展开并缓存(此时只有 main.rs)
        std::fs::write(d.path().join("src/lib.rs"), "").unwrap();
        t.toggle(&d.path().join("src")); // 收起:缓存不会因收起而刷新
        t.reload_from_disk();
        t.toggle(&d.path().join("src")); // 再展开:命中缓存,应已是重载后的新内容
        let names: Vec<_> = t.visible_rows().iter().map(|r| r.name.clone()).collect();
        assert!(names.contains(&"lib.rs".to_string()));
    }

    #[test]
    fn reload_from_disk_drops_deleted_dir_and_its_expanded_state() {
        let d = mktree();
        let mut t = FileTree::new(d.path().to_path_buf());
        t.toggle(&d.path().join("src"));
        std::fs::remove_dir_all(d.path().join("src")).unwrap();
        t.reload_from_disk();
        let rows = t.visible_rows();
        assert!(rows.iter().all(|r| r.name != "src"));
    }

    #[test]
    fn ensure_expanded_is_idempotent_on_already_expanded() {
        let d = mktree();
        let mut t = FileTree::new(d.path().to_path_buf());
        t.toggle(&d.path().join("src"));
        assert_eq!(t.visible_rows().len(), 3); // src, main.rs, README.md
        t.ensure_expanded(&d.path().join("src")); // 已展开,应无副作用
        assert_eq!(t.visible_rows().len(), 3);
    }

    #[test]
    fn ensure_expanded_expands_a_collapsed_dir() {
        let d = mktree();
        let mut t = FileTree::new(d.path().to_path_buf());
        assert_eq!(t.visible_rows().len(), 2); // src(收起), README.md
        t.ensure_expanded(&d.path().join("src"));
        assert_eq!(t.visible_rows().len(), 3); // src, main.rs, README.md
    }

    #[test]
    fn ensure_expanded_on_empty_dir_expands_anyway() {
        // 与 toggle 不同:ensure_expanded 无条件展开,哪怕目录是空的——
        // 这样"新建文件/文件夹"落点所在的目录才有渲染位置(Critical #2)。
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("empty")).unwrap();
        let mut t = FileTree::new(d.path().to_path_buf());
        t.ensure_expanded(&d.path().join("empty"));
        let row = t
            .visible_rows()
            .into_iter()
            .find(|r| r.name == "empty")
            .unwrap();
        assert!(row.expanded);
    }

    #[test]
    fn copy_dir_recursive_copies_nested_structure() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src_dir");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("a.txt"), "A").unwrap();
        std::fs::create_dir(src.join("nested")).unwrap();
        std::fs::write(src.join("nested/b.txt"), "B").unwrap();

        let dst = d.path().join("dst_dir");
        copy_dir_recursive(&src, &dst).unwrap();

        assert_eq!(std::fs::read_to_string(dst.join("a.txt")).unwrap(), "A");
        assert_eq!(
            std::fs::read_to_string(dst.join("nested/b.txt")).unwrap(),
            "B"
        );
    }

    #[test]
    fn copy_dir_recursive_refuses_existing_target() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src_dir");
        std::fs::create_dir(&src).unwrap();
        let dst = d.path().join("dst_dir");
        std::fs::create_dir(&dst).unwrap(); // 目标已存在
        let result = copy_dir_recursive(&src, &dst);
        assert!(result.is_err());
    }

    #[test]
    fn paste_item_copies_file_into_target_dir() {
        let d = tempfile::tempdir().unwrap();
        let src_file = d.path().join("a.txt");
        std::fs::write(&src_file, "hello").unwrap();
        let target_dir = d.path().join("target");
        std::fs::create_dir(&target_dir).unwrap();

        let result = paste_item(&src_file, false, &target_dir).unwrap();
        assert_eq!(result, target_dir.join("a.txt"));
        assert_eq!(std::fs::read_to_string(&result).unwrap(), "hello");
    }

    #[test]
    fn paste_item_rejects_name_collision() {
        let d = tempfile::tempdir().unwrap();
        let src_file = d.path().join("a.txt");
        std::fs::write(&src_file, "hello").unwrap();
        let target_dir = d.path().join("target");
        std::fs::create_dir(&target_dir).unwrap();
        std::fs::write(target_dir.join("a.txt"), "existing").unwrap(); // 已存在同名

        let result = paste_item(&src_file, false, &target_dir);
        assert!(result.is_err());
        // 冲突不覆盖:目标文件内容应保持原样
        assert_eq!(
            std::fs::read_to_string(target_dir.join("a.txt")).unwrap(),
            "existing"
        );
    }

    #[test]
    fn paste_item_rejects_pasting_dir_into_itself() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("a");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("f.txt"), "hi").unwrap();

        let result = paste_item(&dir, true, &dir);
        assert!(result.is_err());
        // 没有递归出 a/a 这样的产物
        assert!(!dir.join("a").exists());
    }

    #[test]
    fn paste_item_rejects_pasting_dir_into_own_descendant() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("a");
        let nested = dir.join("nested");
        std::fs::create_dir(&dir).unwrap();
        std::fs::create_dir(&nested).unwrap();

        let result = paste_item(&dir, true, &nested);
        assert!(result.is_err());
        assert!(!nested.join("a").exists());
    }

    #[test]
    fn is_single_path_component_accepts_plain_name() {
        assert!(is_single_path_component("foo.txt"));
        assert!(is_single_path_component("新文件.rs"));
    }

    #[test]
    fn is_single_path_component_rejects_parent_traversal_and_separators() {
        assert!(!is_single_path_component("../x"));
        assert!(!is_single_path_component("foo/bar"));
        assert!(!is_single_path_component(".."));
        assert!(!is_single_path_component("."));
        assert!(!is_single_path_component(""));
    }

    #[test]
    fn path_string_absolute_is_full_path() {
        let root = Path::new("/repo");
        let p = Path::new("/repo/src/main.rs");
        assert_eq!(
            path_string(PathKind::Absolute, p, root),
            "/repo/src/main.rs"
        );
    }

    #[test]
    fn path_string_relative_strips_root() {
        let root = Path::new("/repo");
        let p = Path::new("/repo/src/main.rs");
        assert_eq!(path_string(PathKind::Relative, p, root), "src/main.rs");
    }

    #[test]
    fn path_string_relative_falls_back_to_absolute_when_not_under_root() {
        let root = Path::new("/repo");
        let p = Path::new("/other/file.rs");
        assert_eq!(path_string(PathKind::Relative, p, root), "/other/file.rs");
    }

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
