//! 受限 PlantUML include resolver(设计 §6,Task 5)。
//!
//! Rust 侧的职责边界(设计 §6.3 末段):**不**完整实现 PlantUML 预处理器。
//! 这里只做三件事:
//!
//! 1. 从根源码里静态识别明确的 include 指令(`!include` / `!include_once` /
//!    `!includesub`);
//! 2. 逐个授权、canonicalize,并验证最终路径仍在项目根内(挡住 `..` 与 symlink
//!    逃逸);递归展开嵌套 include,建立官方引擎可见的**受限虚拟文件集合**;
//! 3. 收集去重、稳定排序的依赖列表,供文件监听(Task 7)与确定性测试使用。
//!
//! 宏/条件语义(`${var}` 拼接、`!if` 分支里的 include 等)**不在这里展开**——
//! 动态构造后无法静态确定目标的 include 一律拒绝,最终预处理仍由官方引擎执行。
//!
//! `PlantUmlInclude.path` 是**项目相对规范化路径**(POSIX 分隔符),与设计 §5.2
//! 一致;前端据此把内容注入引擎的虚拟文件系统。

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use super::PlantUmlInclude;

/// §6.3 资源上限。测试把它们钉死;数值调整需同步 spec 并复核。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlantUmlLimits {
    /// 根源码字节上限(超出不渲染,保留源码只读退路)。
    pub max_root_bytes: u64,
    /// include 深度上限。
    pub max_depth: u32,
    /// include 文件数上限(去重后的实际读取文件数)。
    pub max_include_files: usize,
    /// include 总字节上限(去重后所有被 include 文件的内容总和)。
    pub max_total_include_bytes: u64,
}

impl Default for PlantUmlLimits {
    fn default() -> Self {
        Self {
            max_root_bytes: 4 * 1024 * 1024,
            max_depth: 16,
            max_include_files: 128,
            max_total_include_bytes: 16 * 1024 * 1024,
        }
    }
}

/// 成功加载后的文档:根源码 + 授权 include 虚拟文件 + 依赖路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlantUmlDocument {
    /// 根源码(UTF-8)。
    pub source: String,
    /// 所有可交给引擎的 include 文件(去重,按 `path` 排序稳定)。
    pub includes: Vec<PlantUmlInclude>,
    /// 依赖的**绝对 canonical 路径**(含根文件),去重且排序稳定。
    /// 供 Task 7 反向索引;UI/错误只展示项目相对路径。
    pub dependencies: Vec<PathBuf>,
}

/// 加载失败。全部具名,调用方据此决定终态文案(策略拒绝不得伪装成可重试的
/// 引擎错误)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlantUmlLoadError {
    /// 根源码超过 `max_root_bytes`。
    RootTooLarge { bytes: u64, limit: u64 },
    /// 根源码读取失败。
    RootUnreadable { path: PathBuf },
    /// 根源码不是合法 UTF-8。
    RootNotUtf8 { path: PathBuf },
    /// 相对当前 include 层的深度超过 `max_depth`。`chain` 是安全的相对链。
    TooDeep {
        depth: u32,
        limit: u32,
        chain: Vec<String>,
    },
    /// 去重后的 include 文件数超过 `max_include_files`。
    TooManyFiles { count: usize, limit: usize },
    /// include 内容总字节超过 `max_total_include_bytes`。
    TotalTooLarge { bytes: u64, limit: u64 },
    /// 检测到 include 环。`chain` 从环的起点到重复处。
    Cycle { chain: Vec<String> },
    /// 远程 include(`!includeurl`,或 `!include http(s)://...`):明确拒绝。
    RemoteInclude { target: String },
    /// `file://` include。
    FileUrlInclude { target: String },
    /// 绝对路径 include。
    AbsoluteInclude { target: String },
    /// 规范化后越出项目根(含 `..` 逃逸)。
    OutsideRoot { target: String },
    /// 目标不存在或无法读取。
    IncludeUnreadable { target: String },
    /// 目标不是普通文件(目录/设备/管道等)。
    NotRegularFile { target: String },
    /// 目标不是合法 UTF-8。
    IncludeNotUtf8 { target: String },
    /// include 指令语法无法静态授权(空目标/动态构造等)。
    UnresolvableInclude { target: String },
}

impl std::fmt::Display for PlantUmlLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RootTooLarge { bytes, limit } => {
                write!(f, "根文件过大({bytes} 字节,上限 {limit})")
            }
            Self::RootUnreadable { .. } => write!(f, "无法读取根文件"),
            Self::RootNotUtf8 { .. } => write!(f, "根文件不是 UTF-8 文本"),
            Self::TooDeep { depth, limit, .. } => {
                write!(f, "include 嵌套过深({depth} 层,上限 {limit})")
            }
            Self::TooManyFiles { count, limit } => {
                write!(f, "include 文件过多({count} 个,上限 {limit})")
            }
            Self::TotalTooLarge { bytes, limit } => {
                write!(f, "include 内容过大({bytes} 字节,上限 {limit})")
            }
            Self::Cycle { .. } => write!(f, "include 存在环"),
            Self::RemoteInclude { .. } => write!(f, "远程 include 已禁用"),
            Self::FileUrlInclude { .. } => write!(f, "file:// include 已禁用"),
            Self::AbsoluteInclude { .. } => write!(f, "绝对路径 include 已禁用"),
            Self::OutsideRoot { .. } => write!(f, "include 越出项目根"),
            Self::IncludeUnreadable { .. } => write!(f, "无法读取 include 文件"),
            Self::NotRegularFile { .. } => write!(f, "include 目标不是普通文件"),
            Self::IncludeNotUtf8 { .. } => write!(f, "include 文件不是 UTF-8 文本"),
            Self::UnresolvableInclude { .. } => write!(f, "无法静态解析该 include"),
        }
    }
}

impl std::error::Error for PlantUmlLoadError {}

/// 加载并授权一个 PlantUML 文档。`project_root` 与 `source_path` 都会
/// canonicalize;include 最终路径必须仍在 `project_root` 内。
pub fn load_document(
    project_root: &Path,
    source_path: &Path,
    limits: PlantUmlLimits,
) -> Result<PlantUmlDocument, PlantUmlLoadError> {
    let root =
        std::fs::canonicalize(project_root).map_err(|_| PlantUmlLoadError::RootUnreadable {
            path: project_root.to_path_buf(),
        })?;
    let source_abs =
        std::fs::canonicalize(source_path).map_err(|_| PlantUmlLoadError::RootUnreadable {
            path: source_path.to_path_buf(),
        })?;

    let meta = std::fs::metadata(&source_abs).map_err(|_| PlantUmlLoadError::RootUnreadable {
        path: source_path.to_path_buf(),
    })?;
    if !meta.is_file() {
        return Err(PlantUmlLoadError::RootUnreadable {
            path: source_path.to_path_buf(),
        });
    }
    if meta.len() > limits.max_root_bytes {
        return Err(PlantUmlLoadError::RootTooLarge {
            bytes: meta.len(),
            limit: limits.max_root_bytes,
        });
    }
    let source = read_utf8_file(&source_abs).map_err(|_| PlantUmlLoadError::RootNotUtf8 {
        path: source_path.to_path_buf(),
    })?;

    let mut resolver = Resolver {
        root: root.clone(),
        limits,
        seen: BTreeSet::new(),
        dependencies: BTreeSet::new(),
        includes: Vec::new(),
        total_bytes: 0,
    };

    resolver.dependencies.insert(source_abs.clone());
    let source_dir = source_abs
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.clone());
    let mut chain = vec![project_relative(&root, &source_abs)];
    resolver.expand(&source, &source_dir, 0, &mut chain)?;
    resolver.seen.insert(source_abs);

    resolver.includes.sort_by(|a, b| a.path.cmp(&b.path));
    resolver.includes.dedup_by(|a, b| a.path == b.path);

    debug_assert_eq!(
        resolver.includes.len(),
        resolver.seen.len().saturating_sub(1),
        "includes 与 seen(去掉根文件)应一一对应"
    );

    Ok(PlantUmlDocument {
        source,
        includes: resolver.includes,
        dependencies: resolver.dependencies.into_iter().collect(),
    })
}

struct Resolver {
    root: PathBuf,
    limits: PlantUmlLimits,
    /// 已读取并展开过的 include 文件(canonical 绝对路径)。
    seen: BTreeSet<PathBuf>,
    /// 全部依赖(canonical),含根文件。
    dependencies: BTreeSet<PathBuf>,
    includes: Vec<PlantUmlInclude>,
    total_bytes: u64,
}

impl Resolver {
    /// 扫描 `content` 里的 include 并逐个展开。`dir` 是相对路径基准;
    /// `depth` 是当前文件相对根的深度;`chain` 是从根到当前文件的相对链。
    fn expand(
        &mut self,
        content: &str,
        dir: &Path,
        depth: u32,
        chain: &mut Vec<String>,
    ) -> Result<(), PlantUmlLoadError> {
        for directive in scan_includes(content) {
            if depth + 1 > self.limits.max_depth {
                return Err(PlantUmlLoadError::TooDeep {
                    depth: depth + 1,
                    limit: self.limits.max_depth,
                    chain: chain.clone(),
                });
            }
            let resolved = resolve_target(&directive, dir, &self.root)?;

            if chain.iter().any(|c| self.root.join(c) == resolved) {
                let mut cycle = chain.clone();
                cycle.push(project_relative(&self.root, &resolved));
                return Err(PlantUmlLoadError::Cycle { chain: cycle });
            }
            if self.seen.contains(&resolved) {
                continue;
            }
            self.seen.insert(resolved.clone());

            let meta =
                std::fs::metadata(&resolved).map_err(|_| PlantUmlLoadError::IncludeUnreadable {
                    target: directive.target.clone(),
                })?;
            if !meta.is_file() {
                return Err(PlantUmlLoadError::NotRegularFile {
                    target: directive.target.clone(),
                });
            }
            if self.seen.len() > self.limits.max_include_files {
                return Err(PlantUmlLoadError::TooManyFiles {
                    count: self.seen.len(),
                    limit: self.limits.max_include_files,
                });
            }
            self.total_bytes = self.total_bytes.saturating_add(meta.len());
            if self.total_bytes > self.limits.max_total_include_bytes {
                return Err(PlantUmlLoadError::TotalTooLarge {
                    bytes: self.total_bytes,
                    limit: self.limits.max_total_include_bytes,
                });
            }

            let include_content =
                read_utf8_file(&resolved).map_err(|_| PlantUmlLoadError::IncludeNotUtf8 {
                    target: directive.target.clone(),
                })?;

            let rel = project_relative(&self.root, &resolved);
            self.dependencies.insert(resolved.clone());
            self.includes.push(PlantUmlInclude {
                path: rel.clone(),
                content: include_content.clone(),
            });

            let child_dir = resolved
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.root.clone());
            chain.push(rel);
            self.expand(&include_content, &child_dir, depth + 1, chain)?;
            chain.pop();
        }
        Ok(())
    }
}

/// 把 include 目标解析为 canonical 绝对路径,并施加授权规则。
fn resolve_target(
    directive: &Directive,
    dir: &Path,
    root: &Path,
) -> Result<PathBuf, PlantUmlLoadError> {
    let target = directive.target.trim();

    if directive.kind == DirectiveKind::Url || is_url(target) {
        return Err(PlantUmlLoadError::RemoteInclude {
            target: target.to_string(),
        });
    }
    if target.to_ascii_lowercase().starts_with("file://") {
        return Err(PlantUmlLoadError::FileUrlInclude {
            target: target.to_string(),
        });
    }
    // 尖括号 `<C4/C4_Context>` 是随应用发布的 stdlib,由引擎经 vendored
    // 命名空间解析,不是项目文件——不读盘,也不进依赖集合。
    if target.starts_with('<') && target.ends_with('>') {
        return Err(PlantUmlLoadError::UnresolvableInclude {
            target: target.to_string(),
        });
    }
    // 动态构造(`${var}` / `%(...)` / `$!var`)无法静态授权。
    if target.contains("${") || target.contains("%(") || target.contains("$!") {
        return Err(PlantUmlLoadError::UnresolvableInclude {
            target: target.to_string(),
        });
    }
    if target.is_empty() {
        return Err(PlantUmlLoadError::UnresolvableInclude {
            target: target.to_string(),
        });
    }

    // `!includesub file!tag` 与 PlantUML 的 `file!sub` 语法:取 `!` 前的文件部分。
    let file_part = target.split('!').next().unwrap_or(target).trim();
    let joined = PathBuf::from(file_part);
    if joined.is_absolute() {
        return Err(PlantUmlLoadError::AbsoluteInclude {
            target: target.to_string(),
        });
    }
    // 纯词法拒绝显式 `..`(错误更友好),再由 canonicalize 复核 symlink 逃逸。
    if joined.components().any(|c| c == Component::ParentDir) {
        return Err(PlantUmlLoadError::OutsideRoot {
            target: target.to_string(),
        });
    }

    let candidate = dir.join(&joined);
    let canon =
        std::fs::canonicalize(&candidate).map_err(|_| PlantUmlLoadError::IncludeUnreadable {
            target: target.to_string(),
        })?;
    if !canon.starts_with(root) {
        return Err(PlantUmlLoadError::OutsideRoot {
            target: target.to_string(),
        });
    }
    Ok(canon)
}

fn is_url(target: &str) -> bool {
    let lower = target.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ftp://")
        || lower.starts_with("ftp.")
        || lower.starts_with("//")
}

fn read_utf8_file(path: &Path) -> Result<String, ()> {
    let bytes = std::fs::read(path).map_err(|_| ())?;
    String::from_utf8(bytes).map_err(|_| ())
}

/// 项目相对规范路径(POSIX 分隔符)。不在根内时退化为文件名。
pub(crate) fn project_relative(root: &Path, abs: &Path) -> String {
    match abs.strip_prefix(root) {
        Ok(rel) => include_key(rel),
        Err(_) => abs
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

/// 把规范路径渲染为引擎虚拟文件系统里的键(项目相对、POSIX 分隔符)。
pub fn include_key(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(seg) => Some(seg.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

// ---- 指令扫描 ----

/// 一条被静态识别的 include 指令。
#[derive(Debug, Clone)]
struct Directive {
    /// 原始目标文本(未做路径解析;用于错误展示)。
    target: String,
    /// 指令种类,决定授权规则。
    kind: DirectiveKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectiveKind {
    /// `!include` / `!include_once` / `!includesub`:项目内相对或 stdlib。
    Local,
    /// `!includeurl`:远程,拒绝。
    Url,
}

/// 逐行静态扫描 include 指令。只识别行首(允许前导空白)的 `!include*`。
/// 不做预处理展开:动态目标在授权阶段被拒。
fn scan_includes(source: &str) -> Vec<Directive> {
    let mut out = Vec::new();
    for raw_line in source.split('\n') {
        let line = raw_line.trim_start();
        let Some(rest) = line.strip_prefix('!') else {
            continue;
        };
        let (keyword, arg) = match rest.split_once(char::is_whitespace) {
            Some((kw, arg)) => (kw, arg.trim()),
            // 裸 `!include`(无参数)无法解析;保留空目标交由授权阶段拒绝。
            None => (rest, ""),
        };
        let kind = match keyword {
            "include" | "include_once" | "includesub" => DirectiveKind::Local,
            "includeurl" | "include_url" => DirectiveKind::Url,
            _ => continue,
        };
        // 去掉行内注释(PlantUML 用 `/'` 起块注释;这里只处理尾随 `/'` 片段)
        // 与尾随空白。
        let arg = arg.split(" /'").next().unwrap_or(arg).trim();
        out.push(Directive {
            target: arg.to_string(),
            kind,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    fn load(root: &Path, source: &Path) -> Result<PlantUmlDocument, PlantUmlLoadError> {
        load_document(root, source, PlantUmlLimits::default())
    }

    fn include_paths(doc: &PlantUmlDocument) -> Vec<String> {
        doc.includes.iter().map(|i| i.path.clone()).collect()
    }

    #[test]
    fn resolves_relative_include_same_dir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("main.puml"),
            "@startuml\n!include ./common.puml\n@enduml\n",
        );
        write(&root.join("common.puml"), "!define X 1\n");

        let doc = load(root, &root.join("main.puml")).unwrap();
        assert_eq!(include_paths(&doc), vec!["common.puml"]);
        assert_eq!(doc.includes[0].content, "!define X 1\n");
        assert_eq!(doc.dependencies.len(), 2);
    }

    #[test]
    fn resolves_nested_includes_relative_to_including_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("docs/main.puml"),
            "@startuml\n!include sub/a.puml\n@enduml\n",
        );
        write(&root.join("docs/sub/a.puml"), "!include b.puml\n");
        write(&root.join("docs/sub/b.puml"), "!define Y 2\n");

        let doc = load(root, &root.join("docs/main.puml")).unwrap();
        let paths = include_paths(&doc);
        assert_eq!(paths, vec!["docs/sub/a.puml", "docs/sub/b.puml"]);
        // 依赖稳定排序(canonical 绝对路径)。
        let mut sorted = doc.dependencies.clone();
        sorted.sort();
        assert_eq!(doc.dependencies, sorted);
    }

    #[test]
    fn dedupes_shared_include_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("main.puml"),
            "!include a.puml\n!include_once a.puml\n!include b.puml\n",
        );
        write(&root.join("a.puml"), "!include shared.puml\n");
        write(&root.join("b.puml"), "!include shared.puml\n");
        write(&root.join("shared.puml"), "shared\n");

        let doc = load(root, &root.join("main.puml")).unwrap();
        assert_eq!(include_paths(&doc), vec!["a.puml", "b.puml", "shared.puml"]);
        assert_eq!(doc.dependencies.len(), 4);
    }

    #[test]
    fn detects_include_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("a.puml"), "!include b.puml\n");
        write(&root.join("b.puml"), "!include a.puml\n");

        match load(root, &root.join("a.puml")) {
            Err(PlantUmlLoadError::Cycle { chain }) => {
                assert!(chain.contains(&"a.puml".to_string()));
                assert!(chain.contains(&"b.puml".to_string()));
            }
            other => panic!("expected Cycle, got {other:?}"),
        }
    }

    #[test]
    fn supports_includesub_extracting_file_part() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("main.puml"), "!includesub part.puml!BLOCK\n");
        write(&root.join("part.puml"), "//BLOCK\nAlice -> Bob\n//END\n");

        let doc = load(root, &root.join("main.puml")).unwrap();
        assert_eq!(include_paths(&doc), vec!["part.puml"]);
    }

    #[test]
    fn angle_bracket_stdlib_is_not_a_project_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("main.puml"), "!include <C4/C4_Context>\n");

        match load(root, &root.join("main.puml")) {
            Err(PlantUmlLoadError::UnresolvableInclude { target }) => {
                assert_eq!(target, "<C4/C4_Context>")
            }
            other => panic!("expected UnresolvableInclude, got {other:?}"),
        }
    }

    #[test]
    fn rejects_includeurl_and_plain_urls() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for body in [
            "!includeurl https://example.com/x.puml\n",
            "!include http://example.com/x.puml\n",
            "!include //example.com/x.puml\n",
        ] {
            write(&root.join("main.puml"), body);
            match load(root, &root.join("main.puml")) {
                Err(PlantUmlLoadError::RemoteInclude { .. }) => {}
                other => panic!("expected RemoteInclude for {body:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn rejects_file_url_and_absolute_and_dynamic() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        type Case = (&'static str, fn(&PlantUmlLoadError) -> bool);
        let cases: &[Case] = &[
            ("!include file:///etc/passwd\n", |e| {
                matches!(e, PlantUmlLoadError::FileUrlInclude { .. })
            }),
            ("!include /etc/passwd\n", |e| {
                matches!(e, PlantUmlLoadError::AbsoluteInclude { .. })
            }),
            ("!include ${here}/x.puml\n", |e| {
                matches!(e, PlantUmlLoadError::UnresolvableInclude { .. })
            }),
            ("!include\n", |e| {
                matches!(e, PlantUmlLoadError::UnresolvableInclude { .. })
            }),
        ];
        for (body, check) in cases {
            write(&root.join("main.puml"), body);
            match load(root, &root.join("main.puml")) {
                Err(e) => assert!(check(&e), "wrong error for {body:?}: {e:?}"),
                Ok(_) => panic!("expected error for {body:?}"),
            }
        }
    }

    #[test]
    fn rejects_parent_dir_escape_lexically() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("docs/main.puml"), "!include ../outside.puml\n");
        match load(root, &root.join("docs/main.puml")) {
            Err(PlantUmlLoadError::OutsideRoot { target }) => {
                assert_eq!(target, "../outside.puml")
            }
            other => panic!("expected OutsideRoot, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape_even_when_text_target_is_in_root() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("secret.puml"), "leak\n");

        // 根内一个指向根外的 symlink。
        symlink(outside.path().join("secret.puml"), root.join("link.puml")).unwrap();
        write(&root.join("main.puml"), "!include link.puml\n");

        match load(root, &root.join("main.puml")) {
            Err(PlantUmlLoadError::OutsideRoot { target }) => assert_eq!(target, "link.puml"),
            other => panic!("expected OutsideRoot, got {other:?}"),
        }
    }

    #[test]
    fn rejects_non_utf8_include() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("main.puml"), "!include bad.puml\n");
        fs::write(root.join("bad.puml"), [0xff, 0xfe, 0x00]).unwrap();
        match load(root, &root.join("main.puml")) {
            Err(PlantUmlLoadError::IncludeNotUtf8 { target }) => assert_eq!(target, "bad.puml"),
            other => panic!("expected IncludeNotUtf8, got {other:?}"),
        }
    }

    #[test]
    fn rejects_directory_include() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("subdir")).unwrap();
        write(&root.join("main.puml"), "!include subdir\n");
        match load(root, &root.join("main.puml")) {
            Err(PlantUmlLoadError::NotRegularFile { target }) => assert_eq!(target, "subdir"),
            other => panic!("expected NotRegularFile, got {other:?}"),
        }
    }

    #[test]
    fn rejects_missing_include() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("main.puml"), "!include nope.puml\n");
        match load(root, &root.join("main.puml")) {
            Err(PlantUmlLoadError::IncludeUnreadable { target }) => {
                assert_eq!(target, "nope.puml")
            }
            other => panic!("expected IncludeUnreadable, got {other:?}"),
        }
    }

    #[test]
    fn rejects_oversized_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("main.puml"), "0123456789\n");
        let limits = PlantUmlLimits {
            max_root_bytes: 4,
            ..PlantUmlLimits::default()
        };
        match load_document(root, &root.join("main.puml"), limits) {
            Err(PlantUmlLoadError::RootTooLarge { limit, .. }) => assert_eq!(limit, 4),
            other => panic!("expected RootTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn rejects_too_deep_chain() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("l0.puml"), "!include l1.puml\n");
        write(&root.join("l1.puml"), "!include l2.puml\n");
        write(&root.join("l2.puml"), "!include l3.puml\n");
        write(&root.join("l3.puml"), "leaf\n");
        let limits = PlantUmlLimits {
            max_depth: 2,
            ..PlantUmlLimits::default()
        };
        match load_document(root, &root.join("l0.puml"), limits) {
            Err(PlantUmlLoadError::TooDeep { depth, limit, .. }) => {
                assert_eq!(depth, 3);
                assert_eq!(limit, 2);
            }
            other => panic!("expected TooDeep, got {other:?}"),
        }
    }

    #[test]
    fn rejects_too_many_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut body = String::new();
        for i in 0..5 {
            body.push_str(&format!("!include f{i}.puml\n"));
            write(&root.join(format!("f{i}.puml")), "x\n");
        }
        write(&root.join("main.puml"), &body);
        let limits = PlantUmlLimits {
            max_include_files: 3,
            ..PlantUmlLimits::default()
        };
        match load_document(root, &root.join("main.puml"), limits) {
            Err(PlantUmlLoadError::TooManyFiles { limit, .. }) => assert_eq!(limit, 3),
            other => panic!("expected TooManyFiles, got {other:?}"),
        }
    }

    #[test]
    fn rejects_total_bytes_over_limit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("main.puml"),
            "!include a.puml\n!include b.puml\n",
        );
        write(&root.join("a.puml"), "aaaaaaaaaa\n");
        write(&root.join("b.puml"), "bbbbbbbbbb\n");
        let limits = PlantUmlLimits {
            max_total_include_bytes: 15,
            ..PlantUmlLimits::default()
        };
        match load_document(root, &root.join("main.puml"), limits) {
            Err(PlantUmlLoadError::TotalTooLarge { limit, .. }) => assert_eq!(limit, 15),
            other => panic!("expected TotalTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn non_utf8_root_is_named_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("main.puml"), [0xff, 0xfe]).unwrap();
        match load(root, &root.join("main.puml")) {
            Err(PlantUmlLoadError::RootNotUtf8 { .. }) => {}
            other => panic!("expected RootNotUtf8, got {other:?}"),
        }
    }

    #[test]
    fn scan_ignores_non_include_directives_and_comments() {
        let src = "@startuml\n'!include nope.puml\n!define X 1\n!theme plain\n";
        assert!(scan_includes(src).is_empty());
    }

    #[test]
    fn scan_reads_directive_with_leading_whitespace() {
        let src = "  !include spaced.puml\n";
        let found = scan_includes(src);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].target, "spaced.puml");
        assert_eq!(found[0].kind, DirectiveKind::Local);
    }
}
