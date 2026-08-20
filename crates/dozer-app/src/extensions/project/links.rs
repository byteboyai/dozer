//! 文档/Agent 记忆虚拟链接:数据模型、发现算法、`.dozer/links.json` 存取。
//! 见设计文档"虚拟链接(文档 + Agent 记忆)"一节。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LinkTarget {
    Docs,
    Memory,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LinkKind {
    File,
    Dir,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkEntry {
    pub path: PathBuf,
    pub kind: LinkKind,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LinksState {
    pub docs: Vec<LinkEntry>,
    pub memory: Vec<LinkEntry>,
}

impl LinksState {
    pub fn list(&self, target: LinkTarget) -> &[LinkEntry] {
        match target {
            LinkTarget::Docs => &self.docs,
            LinkTarget::Memory => &self.memory,
        }
    }

    pub fn list_mut(&mut self, target: LinkTarget) -> &mut Vec<LinkEntry> {
        match target {
            LinkTarget::Docs => &mut self.docs,
            LinkTarget::Memory => &mut self.memory,
        }
    }
}

fn links_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("links.json")
}

/// 文件不存在 → `None`(调用方据此判断"要不要跑首次自动发现")。文件存在但
/// 损坏(反序列化失败)→ `Some(LinksState::default())`,不当作"文件不存在"
/// ——损坏就是空,不会触发重新自动发现覆盖用户已有的手动改动假象。
pub fn load(repo: &Path) -> Option<LinksState> {
    let text = std::fs::read_to_string(links_path(repo)).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_default())
}

pub fn save(repo: &Path, state: &LinksState) -> std::io::Result<()> {
    let path = links_path(repo);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let s = serde_json::to_string_pretty(state).unwrap_or_default();
    std::fs::write(path, s)
}

const DOC_FILE_PREFIXES: [&str; 4] = ["readme", "changelog", "contributing", "license"];
const DOC_DIR_NAMES: [&str; 4] = ["docs", "doc", "design", "documentation"];

/// Agent 记忆自动搜集的**项目根目录内**文件(忽略大小写精确匹配)。这些是
/// agent 写在仓库里的指令/记忆文件,不算项目文档,归到 Agent 记忆区。
const MEMORY_FILE_NAMES: [&str; 3] = ["claude.md", "codebudy.md", "agents.md"];

/// Agent 记忆自动搜集的**项目根目录内**目录(忽略大小写精确匹配)。
const MEMORY_DIR_NAMES: [&str; 3] = [".claude", ".codebudy", ".cursor"];

/// 首次发现:根目录直接子项(不递归)。文件名(忽略大小写)以
/// `readme`/`changelog`/`contributing`/`license` 开头,或目录名(忽略大小写)
/// 精确匹配 `docs`/`doc`/`design`/`documentation`。agent 指令文件
/// (claude.md/codebudy.md/agents.md)不属于文档,归到 Agent 记忆(见
/// `discover_memory`)。结果顺序:文件在前、目录在后,组内按名排序。
pub fn discover_docs(repo: &Path) -> Vec<LinkEntry> {
    let Ok(rd) = std::fs::read_dir(repo) else {
        return Vec::new();
    };
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut dirs: Vec<(String, PathBuf)> = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let lower = name.to_lowercase();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            if DOC_DIR_NAMES.contains(&lower.as_str()) {
                dirs.push((name, entry.path()));
            }
        } else if DOC_FILE_PREFIXES.iter().any(|p| lower.starts_with(p)) {
            files.push((name, entry.path()));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    files
        .into_iter()
        .map(|(_, path)| LinkEntry {
            path,
            kind: LinkKind::File,
        })
        .chain(dirs.into_iter().map(|(_, path)| LinkEntry {
            path,
            kind: LinkKind::Dir,
        }))
        .collect()
}

/// 首次发现,分两块:
///
/// 1. **项目根目录内**的 agent 指令/记忆文件与目录——文件精确匹配
///    `claude.md`/`codebudy.md`/`agents.md`,目录精确匹配
///    `.claude`/`.codebudy`/`.cursor`(均忽略大小写)。
/// 2. **仓库外**的三个 agent 存储目录:`claude_project_dir/memory`、
///    `codebuddy_project_dir`、`opencode_project_dir`,存在的才收进结果(不存在
///    的静默跳过,不算错误)。
///
/// 结果顺序:仓库外目录在前,随后项目根文件、项目根目录,组内按名排序。
pub fn discover_memory(repo: &Path) -> Vec<LinkEntry> {
    discover_memory_in(&dozer_core::agent_paths::home_dir(), repo)
}

fn discover_memory_in(home: &Path, repo: &Path) -> Vec<LinkEntry> {
    let mut entries: Vec<LinkEntry> = Vec::new();

    let home_candidates = [
        dozer_core::agent_paths::claude_project_dir_in(home, repo).join("memory"),
        dozer_core::agent_paths::codebuddy_project_dir_in(home, repo),
        dozer_core::agent_paths::opencode_project_dir_in(home, repo),
    ];
    for path in home_candidates.into_iter().filter(|p| p.is_dir()) {
        entries.push(LinkEntry {
            path,
            kind: LinkKind::Dir,
        });
    }

    if let Ok(rd) = std::fs::read_dir(repo) {
        let mut files: Vec<(String, PathBuf)> = Vec::new();
        let mut dirs: Vec<(String, PathBuf)> = Vec::new();
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let lower = name.to_lowercase();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if MEMORY_DIR_NAMES.contains(&lower.as_str()) {
                    dirs.push((name, entry.path()));
                }
            } else if MEMORY_FILE_NAMES.contains(&lower.as_str()) {
                files.push((name, entry.path()));
            }
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        dirs.sort_by(|a, b| a.0.cmp(&b.0));
        entries.extend(files.into_iter().map(|(_, path)| LinkEntry {
            path,
            kind: LinkKind::File,
        }));
        entries.extend(dirs.into_iter().map(|(_, path)| LinkEntry {
            path,
            kind: LinkKind::Dir,
        }));
    }

    entries
}

/// `load` 返回 `None`(文件不存在,首次打开)时跑两个 `discover_*` 拼出初始
/// `LinksState` 并立即 `save`;返回 `Some(state)` 直接用,不再跑发现。
pub fn load_or_discover(repo: &Path) -> LinksState {
    if let Some(state) = load(repo) {
        return state;
    }
    let state = LinksState {
        docs: discover_docs(repo),
        memory: discover_memory(repo),
    };
    let _ = save(repo, &state);
    state
}

#[derive(Debug, Clone, PartialEq)]
pub struct DirRow {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// 目录类型链接就地展开用:单层 `read_dir`,目录在前、按名排序。读不到
/// (权限/不存在)返回空 vec,不报错。
pub fn read_dir_row(path: &Path) -> Vec<DirRow> {
    let Ok(rd) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut rows: Vec<DirRow> = rd
        .flatten()
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            DirRow {
                path: e.path(),
                name,
                is_dir,
            }
        })
        .collect();
    rows.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let state = LinksState {
            docs: vec![LinkEntry {
                path: PathBuf::from("/repo/README.md"),
                kind: LinkKind::File,
            }],
            memory: vec![],
        };
        save(dir.path(), &state).unwrap();
        assert_eq!(load(dir.path()), Some(state));
    }

    #[test]
    fn load_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), None);
    }

    #[test]
    fn load_corrupt_file_returns_default_not_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".dozer")).unwrap();
        std::fs::write(dir.path().join(".dozer").join("links.json"), "not json").unwrap();
        assert_eq!(load(dir.path()), Some(LinksState::default()));
    }

    #[test]
    fn discover_docs_finds_readme_and_docs_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "").unwrap();
        std::fs::write(dir.path().join("CHANGELOG.md"), "").unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap(); // 不匹配,应忽略
        let found = discover_docs(dir.path());
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].path, dir.path().join("CHANGELOG.md")); // 文件按名排序
        assert_eq!(found[0].kind, LinkKind::File);
        assert_eq!(found[1].path, dir.path().join("README.md"));
        assert_eq!(found[2].path, dir.path().join("docs")); // 目录排在文件之后
        assert_eq!(found[2].kind, LinkKind::Dir);
    }

    #[test]
    fn discover_docs_excludes_agent_instruction_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "").unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "").unwrap();
        std::fs::write(dir.path().join("CODEBUDY.md"), "").unwrap();
        // agent 指令文件不属于项目文档,文档发现不应捡到任何一个。
        assert!(discover_docs(dir.path()).is_empty());
    }

    #[test]
    fn discover_docs_case_insensitive_and_no_recursion() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("readme.txt"), "").unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("docs").join("README.md"), "").unwrap(); // 不该被发现,非根目录
        let found = discover_docs(dir.path());
        assert_eq!(found.len(), 2); // readme.txt + docs 目录本身
    }

    #[test]
    fn discover_docs_empty_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(discover_docs(dir.path()).is_empty());
    }

    #[test]
    fn discover_memory_finds_existing_agent_dirs() {
        let home = tempfile::tempdir().unwrap();
        let repo = PathBuf::from("/repo/x");
        let claude_memory =
            dozer_core::agent_paths::claude_project_dir_in(home.path(), &repo).join("memory");
        std::fs::create_dir_all(&claude_memory).unwrap();
        let found = discover_memory_in(home.path(), &repo);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, claude_memory);
        assert_eq!(found[0].kind, LinkKind::Dir);
    }

    #[test]
    fn discover_memory_finds_project_root_agent_files_and_dirs() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("CLAUDE.md"), "").unwrap();
        std::fs::write(repo.path().join("codebudy.md"), "").unwrap();
        std::fs::write(repo.path().join("AGENTS.md"), "").unwrap();
        std::fs::create_dir(repo.path().join(".claude")).unwrap();
        std::fs::create_dir(repo.path().join(".codebudy")).unwrap();
        std::fs::create_dir(repo.path().join(".cursor")).unwrap();
        let found = discover_memory_in(home.path(), repo.path());
        // 3 个文件 + 3 个目录
        assert_eq!(found.len(), 6);
        // 文件在前、目录在后,组内按名排序。
        assert_eq!(found[0].path, repo.path().join("AGENTS.md"));
        assert_eq!(found[0].kind, LinkKind::File);
        assert_eq!(found[1].path, repo.path().join("CLAUDE.md"));
        assert_eq!(found[1].kind, LinkKind::File);
        assert_eq!(found[2].path, repo.path().join("codebudy.md"));
        assert_eq!(found[2].kind, LinkKind::File);
        assert_eq!(found[3].path, repo.path().join(".claude"));
        assert_eq!(found[3].kind, LinkKind::Dir);
        assert_eq!(found[4].path, repo.path().join(".codebudy"));
        assert_eq!(found[4].kind, LinkKind::Dir);
        assert_eq!(found[5].path, repo.path().join(".cursor"));
        assert_eq!(found[5].kind, LinkKind::Dir);
    }

    #[test]
    fn discover_memory_none_when_nothing_exists() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        assert!(discover_memory_in(home.path(), repo.path()).is_empty());
    }

    #[test]
    fn load_or_discover_runs_discovery_when_no_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "").unwrap();
        let state = load_or_discover(dir.path());
        assert_eq!(state.docs.len(), 1);
        // 第二次调用不再重新发现——已经落盘,即使根目录多了一个新文件也不会
        // 被捡进来。
        std::fs::write(dir.path().join("CHANGELOG.md"), "").unwrap();
        let state2 = load_or_discover(dir.path());
        assert_eq!(state2.docs.len(), 1);
    }

    #[test]
    fn load_or_discover_uses_existing_file_without_rediscovering() {
        let dir = tempfile::tempdir().unwrap();
        let manual = LinksState {
            docs: vec![],
            memory: vec![],
        };
        save(dir.path(), &manual).unwrap();
        std::fs::write(dir.path().join("README.md"), "").unwrap();
        let state = load_or_discover(dir.path());
        assert!(state.docs.is_empty()); // 不会因为磁盘上有 README 就补进来
    }

    #[test]
    fn read_dir_row_lists_dirs_before_files_sorted_by_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();
        std::fs::create_dir(dir.path().join("a_dir")).unwrap();
        let rows = read_dir_row(dir.path());
        assert_eq!(rows.len(), 2);
        assert!(rows[0].is_dir);
        assert_eq!(rows[0].name, "a_dir");
        assert_eq!(rows[1].name, "b.txt");
    }

    #[test]
    fn read_dir_row_missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_dir_row(&dir.path().join("nope")).is_empty());
    }
}
