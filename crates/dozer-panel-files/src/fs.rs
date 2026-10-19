//! 文件树状态机（P1g）：懒加载单目录、展开集、可见行摊平。纯数据，不碰
//! iced；展开时同步 read_dir（单目录快）。固定隐藏名单过滤。

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// 复制路径展示形式：绝对路径 vs 相对项目根目录 vs 仅文件/目录名（含后缀）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathKind {
    Absolute,
    Relative,
    Name,
}

/// 文件树里恒不显示的目录/文件名。
pub const HIDDEN: [&str; 5] = [".git", "target", "node_modules", ".DS_Store", "__pycache__"];

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

struct SearchCache {
    query: String,
    rows: Vec<TreeRow>,
    collapsed: HashSet<PathBuf>,
}

pub struct FileTree {
    search: RefCell<Option<SearchCache>>,
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    children: HashMap<PathBuf, Vec<Entry>>,
    /// 是否显示以 `.` 开头的文件/目录(点文件,如 `.env`、`.gitignore`)。
    /// 默认 false(2026-09 用户口径:文件树默认不显示隐藏文件)——只按
    /// `HIDDEN` 名单过滤的基础上再隐藏所有点文件和点目录;为 true 时只按
    /// `HIDDEN` 名单过滤。搜索框后的"眼睛"按钮(`ToggleDotfiles`)切换此值,
    /// 切换后重读所有已缓存目录让树立刻反映新口径。
    show_dotfiles: bool,
}

/// 读一个目录:剔除隐藏名单(以及关闭点文件时所有 `.` 开头项)、目录在前、
/// 各自按名排序。read_dir 失败返回空。是 `FileTree` 实例方法以便读取
/// `self.show_dotfiles` 口径;`toggle`/`refresh`/`search_rows` 未缓存的目录
/// 都走它,保证手动展开与搜索结果看到的口径一致。
fn read_children(tree: &FileTree, dir: &Path) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let show_dot = tree.show_dotfiles;
    let mut entries: Vec<Entry> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if HIDDEN.contains(&name.as_str()) {
                return None;
            }
            if !show_dot && name.starts_with('.') {
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
        let mut tree = Self {
            search: RefCell::new(None),
            root,
            expanded: HashSet::new(),
            children: HashMap::new(),
            show_dotfiles: false,
        };
        tree.children
            .insert(tree.root.clone(), read_children(&tree, &tree.root));
        tree
    }

    /// 切换"显示/隐藏点文件"(文件树搜索框后的眼睛按钮)。置口径后重读所有
    /// 已缓存目录(`reload_from_disk`),让当前展开的树立刻反映新过滤,无需
    /// 用户手动重新展开。纯偏好项,不清 `expanded`。
    pub fn set_show_dotfiles(&mut self, v: bool) {
        self.show_dotfiles = v;
        self.reload_from_disk();
    }

    /// 当前"显示点文件"口径(文件树眼睛按钮需要读出以选 eye/eye-off 图标)。
    pub fn dotfiles_shown(&self) -> bool {
        self.show_dotfiles
    }

    /// 展开/收起一个目录。展开时若未缓存则同步读一次。哪怕目录是空的
    /// 或不可读,点击后箭头图标（>→V）也要如实反映"已展开"这个状态,
    /// 不能因为无可展开内容就悄悄拒绝翻转。
    pub fn toggle(&mut self, dir: &Path) {
        if let Some(search) = self.search.get_mut().as_mut() {
            if !search.collapsed.remove(dir) {
                search.collapsed.insert(dir.to_path_buf());
            }
            return;
        }
        if self.expanded.remove(dir) {
            return; // 已展开 → 收起
        }
        let children = read_children(self, dir);
        self.children.entry(dir.to_path_buf()).or_insert(children);
        self.expanded.insert(dir.to_path_buf());
    }

    /// 强制重读一个目录的子项缓存（增删改后调用，让 `visible_rows()`
    /// 反映最新磁盘状态）。目录本身若已展开保持展开；未展开的话，本次
    /// 调用不强行展开，只刷新缓存——下次展开时自然是最新的。
    pub fn refresh(&mut self, dir: &Path) {
        self.search.get_mut().take();
        self.children
            .insert(dir.to_path_buf(), read_children(self, dir));
    }

    /// 从磁盘整体重新加载:重读每一个已缓存过的目录(含已收起的——`toggle`
    /// 展开时若缓存已存在不会重读,收起目录的缓存不刷新就会在重新展开时
    /// 冒出陈旧内容),目录若已在磁盘上消失则连同其展开态一并丢弃。
    /// 不重建整棵树(不清 `expanded`),用户当前展开的层级保持不变。
    pub fn reload_from_disk(&mut self) {
        self.search.get_mut().take();
        let dirs: Vec<PathBuf> = self.children.keys().cloned().collect();
        for dir in dirs {
            if dir.is_dir() {
                self.children.insert(dir.clone(), read_children(self, &dir));
            } else {
                self.children.remove(&dir);
                self.expanded.remove(&dir);
            }
        }
    }

    /// 确保目录处于展开态（新建文件/文件夹前调用，让新项有可见位置）。
    /// 与 `toggle` 不同：只展开、不收起——已展开时调用不会把目录翻回
    /// 收起态。新建文件/文件夹的落点必须可见，即便"可见"只是一个空的
    /// 展开态目录（渲染零子行，不 crash、不视觉异常）。
    pub fn ensure_expanded(&mut self, dir: &Path) {
        let children = read_children(self, dir);
        self.children.entry(dir.to_path_buf()).or_insert(children);
        self.expanded.insert(dir.to_path_buf());
    }

    /// 从 root 的子项起 DFS 摊平成可见行（展开的目录才递归其子项）。
    pub fn visible_rows(&self) -> Vec<TreeRow> {
        let mut out = Vec::new();
        self.push_rows(&self.root, 0, &mut out);
        out
    }

    /// 搜索结果按项目、查询缓存，重绘和折叠不重复遍历磁盘。
    /// 搜索独立保存折叠态，退出搜索后恢复普通树的展开态。
    pub fn clear_search(&mut self) {
        self.search.get_mut().take();
    }

    pub fn search_rows(&self, query: &str) -> Vec<TreeRow> {
        if query.is_empty() {
            self.search.borrow_mut().take();
            return self.visible_rows();
        }
        let query = query.to_lowercase();
        let mut cache = self.search.borrow_mut();
        if cache.as_ref().is_none_or(|search| search.query != query) {
            let mut rows = Vec::new();
            let mut keep = HashSet::new();
            // 将条目而非目录压栈，保证父目录后紧接其子树，兄弟顺序不反转。
            let mut stack: Vec<_> = read_children(self, &self.root)
                .into_iter()
                .rev()
                .map(|entry| (entry, 0))
                .collect();
            while let Some((entry, depth)) = stack.pop() {
                if entry.name.to_lowercase().contains(&query) {
                    keep.insert(entry.path.clone());
                    let mut parent = entry.path.parent();
                    while let Some(path) = parent {
                        if path == self.root {
                            break;
                        }
                        keep.insert(path.to_path_buf());
                        parent = path.parent();
                    }
                }
                if entry.is_dir {
                    stack.extend(
                        read_children(self, &entry.path)
                            .into_iter()
                            .rev()
                            .map(|child| (child, depth + 1)),
                    );
                }
                rows.push(TreeRow {
                    path: entry.path,
                    name: entry.name,
                    depth,
                    is_dir: entry.is_dir,
                    expanded: entry.is_dir,
                });
            }
            rows.retain(|row| keep.contains(&row.path));
            *cache = Some(SearchCache {
                query,
                rows,
                collapsed: HashSet::new(),
            });
        }
        let search = cache.as_ref().unwrap();
        let mut hidden_depth = None;
        search
            .rows
            .iter()
            .filter_map(|row| {
                if hidden_depth.is_some_and(|depth| row.depth > depth) {
                    return None;
                }
                hidden_depth = None;
                let mut row = row.clone();
                if row.is_dir && search.collapsed.contains(&row.path) {
                    row.expanded = false;
                    hidden_depth = Some(row.depth);
                }
                Some(row)
            })
            .collect()
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

/// 文件系统操作(复制/粘贴/移动)的结构化错误(Task 07:操作语义 + 原始诊断
/// 分离)。`project.rs` 的这些函数在工作线程里跑、拿不到 locale,故返回结构化
/// 原因,由呈现层按当前 locale 渲染。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectFsError {
    /// 目标已存在(粘贴/移动冲突);`path` 是完整目标路径。
    AlreadyExists { path: PathBuf },
    /// 目录被移动/粘贴进它自己或其子目录。
    IntoOwnSubtree { op: FsOp },
    /// 已复制到目标,但删除源失败;`detail` 为系统报错原文。
    CopiedButDeleteFailed { detail: String },
    /// 底层 I/O 失败;`detail` 为系统报错原文。
    Io { detail: String },
}

/// 触发 `IntoOwnSubtree` 的操作,文案据此区分"移动"/"粘贴"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsOp {
    Move,
    Paste,
}

impl ProjectFsError {
    /// 语言无关的稳定诊断串(英文),用于日志/去重。
    pub fn stable(&self) -> String {
        match self {
            ProjectFsError::AlreadyExists { path } => {
                format!(
                    "An item with the same name already exists: {}",
                    path.display()
                )
            }
            ProjectFsError::IntoOwnSubtree { op } => match op {
                FsOp::Move => {
                    "Cannot move a directory into itself or its own subdirectory".to_string()
                }
                FsOp::Paste => {
                    "Cannot paste a directory into itself or its own subdirectory".to_string()
                }
            },
            ProjectFsError::CopiedButDeleteFailed { detail } => {
                format!("Copied to the target, but deleting the source failed: {detail}")
            }
            ProjectFsError::Io { detail } => detail.clone(),
        }
    }
}

/// 递归复制目录（文件树"粘贴"目标是目录时用）。目标已存在 → `Err`，不覆盖、
/// 不做部分复制后中止的脏状态清理（失败时目标目录可能已建但不完整——
/// 调用方(`paste_item`)已在动手前检查过存在性，这里的检查是防御递归过程中
/// 子目录层面的二次冲突）。
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), ProjectFsError> {
    if dst.exists() {
        return Err(ProjectFsError::AlreadyExists {
            path: dst.to_path_buf(),
        });
    }
    std::fs::create_dir(dst).map_err(io_err)?;
    for entry in std::fs::read_dir(src).map_err(io_err)? {
        let entry = entry.map_err(io_err)?;
        let file_type = entry.file_type().map_err(io_err)?;
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(io_err)?;
        }
    }
    Ok(())
}

fn io_err(e: std::io::Error) -> ProjectFsError {
    ProjectFsError::Io {
        detail: e.to_string(),
    }
}

/// 把 `source`（文件或目录）复制进 `target_dir` 下，用源的文件名。目标已
/// 存在同名项 → `Err`（冲突文案）。成功返回新建出的完整路径。
pub fn paste_item(
    source: &Path,
    source_is_dir: bool,
    target_dir: &Path,
) -> Result<PathBuf, ProjectFsError> {
    let dest = target_dir.join(source.file_name().unwrap_or_default());
    // 目录粘贴进自己或自己的子目录 → dest 落在 source 树内,
    // copy_dir_recursive 会边建目录边把刚建出来的目标又递归进去复制,
    // 无界自增长直至 ENAMETOOLONG/磁盘写满。文件不可能出现这种情况
    // (文件没有"子目录"可落入自身)。
    if source_is_dir && (dest.starts_with(source) || dest == source) {
        return Err(ProjectFsError::IntoOwnSubtree { op: FsOp::Paste });
    }
    if dest.exists() {
        return Err(ProjectFsError::AlreadyExists { path: dest });
    }
    let result = if source_is_dir {
        copy_dir_recursive(source, &dest)
    } else {
        std::fs::copy(source, &dest).map(|_| ()).map_err(io_err)
    };
    result.map(|_| dest)
}

/// 把 `source` 移动到明确指定的完整目标路径 `dest`(目录 + 文件名都由
/// 调用方给定,允许改名)——`move_item` 的通用版本。语义对齐 macOS
/// Finder 拖拽:同文件系统直接 `std::fs::rename`(改目录项、不拷数据);
/// 跨文件系统(rename 返回 `EXDEV`)降级为复制 + 删除源(不复用
/// `paste_item`——那个函数自己按源文件名算 `dest`,这里 `dest` 已经是
/// 调用方定好的完整路径,直接拷到它)。`dest` 已存在 → `Err`;目录移进
/// 自身子树 → `Err`。成功返回 `dest`。供"拖拽移动确认框"(用户可在确认
/// 前改文件名/改目标目录,见 `files::PendingMove`)和 `move_item`(不改名
/// 的既有场景)共用。
pub fn move_item_to(
    source: &Path,
    source_is_dir: bool,
    dest: &Path,
) -> Result<PathBuf, ProjectFsError> {
    // 同 `paste_item`:目录不能移进自己或自己的子目录(无界自增长)。
    if source_is_dir && (dest.starts_with(source) || dest == source) {
        return Err(ProjectFsError::IntoOwnSubtree { op: FsOp::Move });
    }
    if dest.exists() {
        return Err(ProjectFsError::AlreadyExists {
            path: dest.to_path_buf(),
        });
    }
    match std::fs::rename(source, dest) {
        Ok(()) => Ok(dest.to_path_buf()),
        // 跨文件系统:ErrorKind::CrossesDevices(EXDEV)——复制后删源。
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            let copied = if source_is_dir {
                copy_dir_recursive(source, dest)
            } else {
                std::fs::copy(source, dest).map(|_| ()).map_err(io_err)
            };
            match copied {
                Ok(()) => {
                    let cleanup = if source_is_dir {
                        std::fs::remove_dir_all(source)
                    } else {
                        std::fs::remove_file(source)
                    };
                    cleanup.map(|()| dest.to_path_buf()).map_err(|e| {
                        ProjectFsError::CopiedButDeleteFailed {
                            detail: e.to_string(),
                        }
                    })
                }
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(io_err(e)),
    }
}

/// 把 `source`(文件或目录)移动到 `target_dir` 下,用源的文件名——
/// `move_item_to` 的常用简写(仅测试用;生产路径都带目标名,走 `move_item_to`)。
#[cfg(test)]
pub fn move_item(
    source: &Path,
    source_is_dir: bool,
    target_dir: &Path,
) -> Result<PathBuf, ProjectFsError> {
    let dest = target_dir.join(source.file_name().unwrap_or_default());
    move_item_to(source, source_is_dir, &dest)
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
/// 兜底）就退化成绝对路径；名字取最后一段（文件含后缀，目录即目录名），
/// 取不到最后一段（如根路径 `/`）退化成原样。
pub fn path_string(kind: PathKind, path: &Path, project_root: &Path) -> String {
    match kind {
        PathKind::Absolute => path.display().to_string(),
        PathKind::Relative => path
            .strip_prefix(project_root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string()),
        PathKind::Name => path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
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
        // 哪怕目录是空的,ensure_expanded 也无条件展开——这样"新建文件/
        // 文件夹"落点所在的目录才有渲染位置(Critical #2)。
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
    fn toggle_on_empty_dir_flips_expanded_so_chevron_updates() {
        // 空目录点击展开箭头(>)也要变成(V)——即便没有子项可渲染,
        // 用户点了展开就该看到"已展开"的图标反馈,不能悄悄拒绝翻转。
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("empty")).unwrap();
        let mut t = FileTree::new(d.path().to_path_buf());
        t.toggle(&d.path().join("empty"));
        let row = t
            .visible_rows()
            .into_iter()
            .find(|r| r.name == "empty")
            .unwrap();
        assert!(row.expanded);

        // 再点一次要能收起回去。
        t.toggle(&d.path().join("empty"));
        let row = t
            .visible_rows()
            .into_iter()
            .find(|r| r.name == "empty")
            .unwrap();
        assert!(!row.expanded);
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
    fn move_item_moves_file_into_target_dir() {
        let d = tempfile::tempdir().unwrap();
        let src_file = d.path().join("a.txt");
        std::fs::write(&src_file, "hello").unwrap();
        let target_dir = d.path().join("target");
        std::fs::create_dir(&target_dir).unwrap();

        let result = move_item(&src_file, false, &target_dir).unwrap();
        assert_eq!(result, target_dir.join("a.txt"));
        assert_eq!(std::fs::read_to_string(&result).unwrap(), "hello");
        // 源已移除(移动而非复制)
        assert!(!src_file.exists());
    }

    /// `move_item_to` 是 `move_item` 的通用版本,允许改名(目标文件名不必
    /// 跟源一致)——拖拽移动确认框(`files::PendingMove`)靠它实现"改名 +
    /// 改目标目录一起生效"。
    #[test]
    fn move_item_to_renames_while_moving() {
        let d = tempfile::tempdir().unwrap();
        let src_file = d.path().join("a.txt");
        std::fs::write(&src_file, "hello").unwrap();
        let target_dir = d.path().join("target");
        std::fs::create_dir(&target_dir).unwrap();
        let dest = target_dir.join("renamed.txt");

        let result = move_item_to(&src_file, false, &dest).unwrap();
        assert_eq!(result, dest);
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "hello");
        assert!(!src_file.exists());
    }

    #[test]
    fn move_item_moves_dir_recursively_and_removes_source() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("s");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("a.txt"), "A").unwrap();
        std::fs::create_dir(src.join("nested")).unwrap();
        std::fs::write(src.join("nested/b.txt"), "B").unwrap();

        let target_dir = d.path().join("t");
        std::fs::create_dir(&target_dir).unwrap();
        let result = move_item(&src, true, &target_dir).unwrap();
        assert_eq!(result, target_dir.join("s"));
        assert_eq!(
            std::fs::read_to_string(result.join("nested/b.txt")).unwrap(),
            "B"
        );
        // 整个源目录已被移走
        assert!(!src.exists());
    }

    #[test]
    fn move_item_rejects_name_collision_without_touching_anywhere() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("t");
        std::fs::create_dir(&dir).unwrap();
        let src_file = d.path().join("a.txt");
        std::fs::write(&src_file, "hello").unwrap();
        std::fs::write(dir.join("a.txt"), "existing").unwrap();

        let result = move_item(&src_file, false, &dir);
        assert!(result.is_err());
        // 冲突不覆盖目标,也不动源
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "existing"
        );
        assert!(src_file.exists());
    }

    #[test]
    fn move_item_rejects_moving_dir_into_itself_and_descendant() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("a");
        let nested = dir.join("nested");
        std::fs::create_dir(&dir).unwrap();
        std::fs::create_dir(&nested).unwrap();

        assert!(move_item(&dir, true, &dir).is_err());
        assert!(move_item(&dir, true, &nested).is_err());
        // 没有递归出 a/nested/a 这样的产物
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

    /// 树只按 `file_type()` 判目录(不 follow symlink),所以指向目录的
    /// 符号链接会作为"文件"出现、点击时也不会被当目录展开——这是刻意的:
    /// follow 会带来成环(指向祖先)和无界递归,而文件树没有环检测。
    #[cfg(unix)]
    #[test]
    fn symlinked_dir_is_not_followed_as_dir() {
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("inner.txt"), "").unwrap();
        std::os::unix::fs::symlink(&real, d.path().join("link")).unwrap();

        let t = FileTree::new(d.path().to_path_buf());
        let row = t
            .visible_rows()
            .into_iter()
            .find(|r| r.name == "link")
            .expect("symlink shows up as a row");
        assert!(!row.is_dir, "symlink must not be treated as a directory");
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

    #[test]
    fn path_string_name_is_last_component_with_extension() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/src/main.rs");
        let dir = Path::new("/repo/src");
        assert_eq!(path_string(PathKind::Name, file, root), "main.rs");
        assert_eq!(path_string(PathKind::Name, dir, root), "src");
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
    fn toggle_dotfiles_shows_hides_dot_prefix_entries() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir(r.join("src")).unwrap();
        std::fs::write(r.join("src/main.rs"), "").unwrap();
        std::fs::write(r.join(".env"), "").unwrap(); // 点文件(非 HIDDEN 名单)
        std::fs::write(r.join(".gitignore"), "").unwrap(); // 点文件
        std::fs::create_dir(r.join(".git")).unwrap(); // 仍在固定 HIDDEN 名单

        let mut t = FileTree::new(r.to_path_buf());
        // 默认:不显示点文件(.env/.gitignore 都不在;.git 仍按 HIDDEN 名单剔除)。
        assert!(!t.dotfiles_shown());
        let names: Vec<_> = t
            .visible_rows()
            .into_iter()
            .map(|r| r.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["src"]);

        // 展开 src 后显示点文件:set_show_dotfiles 重读全部缓存目录,已展开
        // 的 src 仍保留(不清 expanded)。
        t.toggle(&r.join("src"));
        assert_eq!(t.visible_rows().len(), 2, "展开 src 后可见 src、main.rs");
        t.set_show_dotfiles(true);
        assert!(t.dotfiles_shown());
        let names: Vec<_> = t
            .visible_rows()
            .into_iter()
            .map(|r| r.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["src", "main.rs", ".env", ".gitignore"],
            "点文件全回来,src 展开保留"
        );

        // 重新隐藏点文件,点文件立刻消失。
        t.set_show_dotfiles(false);
        let names: Vec<_> = t
            .visible_rows()
            .into_iter()
            .map(|r| r.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["src", "main.rs"]);
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
    fn search_rows_reaches_nested_unexpanded_dirs() {
        let d = mktree();
        // 建一个未展开的深层嵌套文件:src/util/helpers.rs
        std::fs::create_dir(d.path().join("src/util")).unwrap();
        std::fs::write(d.path().join("src/util/helpers.rs"), "").unwrap();
        let t = FileTree::new(d.path().to_path_buf());
        // 未展开任何目录:visible_rows 只见根层
        assert_eq!(t.visible_rows().len(), 2);
        // 全树搜索必须穿透未展开的深层目录命中,并带回祖先目录形成完整路径:
        // src(深度0) → src/util(深度1) → helpers.rs(深度2)
        let rows = t.search_rows("helpers");
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src", "util", "helpers.rs"]);
        assert_eq!(rows[2].depth, 2);
        assert!(rows[0].is_dir && rows[1].is_dir && !rows[2].is_dir);
    }

    #[test]
    fn search_rows_matches_dir_and_case_insensitive() {
        let d = mktree();
        let t = FileTree::new(d.path().to_path_buf());
        // 目录名匹配 + 大小写不敏感:src 本身命中(无祖先需带回,恰在根层)
        let rows = t.search_rows("SRC");
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src"]);
        assert!(rows[0].is_dir);
        // 空查询退化为根层(与可见树一致)
        let empty = t.search_rows("");
        assert_eq!(empty.len(), 2);
    }

    #[test]
    fn search_keeps_sibling_subtrees_contiguous_and_can_collapse() {
        let d = tempfile::tempdir().unwrap();
        for dir in ["api/common", "src/middlewares"] {
            std::fs::create_dir_all(d.path().join(dir)).unwrap();
            std::fs::write(d.path().join(dir).join("debug.py"), "").unwrap();
        }
        let mut t = FileTree::new(d.path().to_path_buf());
        let rows = t.search_rows("debug");
        assert_eq!(
            rows.iter()
                .map(|r| (r.name.as_str(), r.depth))
                .collect::<Vec<_>>(),
            vec![
                ("api", 0),
                ("common", 1),
                ("debug.py", 2),
                ("src", 0),
                ("middlewares", 1),
                ("debug.py", 2)
            ]
        );
        assert!(rows[0].expanded);
        t.toggle(&d.path().join("api"));
        let rows = t.search_rows("debug");
        assert_eq!(rows.len(), 4);
        assert!(!rows[0].expanded);
        assert_eq!(rows[1].name, "src");
        t.toggle(&d.path().join("api"));
        assert_eq!(t.search_rows("debug").len(), 6);
        t.clear_search();
        assert_eq!(t.visible_rows().len(), 2);
        t.toggle(&d.path().join("api"));
        assert_eq!(t.visible_rows().len(), 3);
    }

    #[test]
    fn search_reuses_snapshot_until_refresh_or_resubmit() {
        let d = mktree();
        let mut t = FileTree::new(d.path().to_path_buf());
        assert!(t.search_rows("new").is_empty());
        std::fs::write(d.path().join("src/new.rs"), "").unwrap();
        assert!(t.search_rows("new").is_empty(), "重绘应复用搜索快照");
        t.refresh(&d.path().join("src"));
        assert_eq!(t.search_rows("new").len(), 2);
        std::fs::write(d.path().join("src/new2.rs"), "").unwrap();
        t.clear_search();
        assert_eq!(t.search_rows("new").len(), 3);
    }

    #[test]
    fn empty_search_preserves_expanded_visible_tree() {
        let d = mktree();
        let mut t = FileTree::new(d.path().to_path_buf());
        t.toggle(&d.path().join("src"));
        assert_eq!(t.search_rows(""), t.visible_rows());
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
