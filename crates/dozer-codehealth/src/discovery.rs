//! 可信文件发现与扫描范围（spec 2026-09-21「扫描范围与忽略规则」）。
//!
//! 用 `ignore::WalkBuilder` 遵守 `.gitignore`/`.ignore`/隐藏目录规则，叠加
//! 内置构建目录与项目可选配置 `.dozer/code-health.toml`；把“没被分析”的
//! 文件从静默跳过改为带原因的 [`SkippedFile`] 记录，产出语言统计。

use crate::scan_metadata::{LanguageSummary, SkipReason, SkippedFile};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 内置始终跳过的构建/缓存目录（无论 `.gitignore` 是否列出都排除）。
const BUILTIN_IGNORE_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "build",
    "dist",
    ".venv",
    "venv",
    ".idea",
    ".vscode",
    ".next",
    ".cache",
    "coverage",
    "out",
];

/// 项目可选配置（`.dozer/code-health.toml`）。缺失/解析失败都回落默认空配置，
/// 不阻断扫描。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    /// 额外排除的 glob（相对项目根，gitignore 风格）。
    pub exclude: Vec<String>,
    /// 标记为“生成文件”的 glob——被排除出分析，并记录 `SkipReason::Generated`。
    pub generated: Vec<String>,
}

/// 读取 `.dozer/code-health.toml`；文件不存在或解析失败返回空配置。
pub fn load_project_config(root: &Path) -> ProjectConfig {
    let path = root.join(".dozer").join("code-health.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ProjectConfig::default();
    };
    toml::from_str::<ProjectConfig>(&text).unwrap_or_default()
}

/// 一次文件遍历的结果。
#[derive(Debug, Default)]
pub struct Discovery {
    /// 内置构建目录剪枝后、其它 ignore 规则生效前看到的文件总数。
    pub discovered_count: usize,
    /// 需要语义分析的文件（相对项目根的规范化路径）。
    pub rust_files: Vec<PathBuf>,
    /// 语言统计（含仅统计、不做结构分析的非 Rust 语言）。
    pub languages: Vec<LanguageSummary>,
    /// 被遍历到但未分析的候选文件及原因（Generated / UnsupportedLanguage）。
    /// NonUtf8 / ReadFailed / ParseFailed 由扫描阶段追加（需要读文件内容）。
    pub skipped: Vec<SkippedFile>,
    /// 被规则排除（config exclude/generated + 不支持的语言）的文件数。
    pub excluded_count: usize,
}

/// 把相对路径转成 `ignore`/`globset` 可比的规范字符串（`/` 分隔）。
fn rel_str(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

/// 语言识别：已知扩展名 → 语言标识；`None` = 不支持。
fn language_of(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("rs") => Some("rust"),
        Some("toml") => Some("toml"),
        Some("json") => Some("json"),
        Some("md") | Some("markdown") => Some("markdown"),
        Some("ts") | Some("tsx") => Some("typescript"),
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => Some("javascript"),
        Some("py") => Some("python"),
        Some("go") => Some("go"),
        Some("c") | Some("h") => Some("c"),
        Some("cpp") | Some("cc") | Some("cxx") | Some("hpp") => Some("cpp"),
        Some("java") => Some("java"),
        _ => None,
    }
}

/// 遍历项目根，产出 [`Discovery`]。忽略规则来源：内置构建目录 +
/// `.gitignore`/`.ignore`/隐藏目录（`standard_filters`）+ config `exclude`。
/// config `generated` 与不支持语言的文件**仍然被 yield**，但作为带原因的
/// 跳过项记录（这样扫描范围 UI 能列出“为什么没分析”的清单）。
pub fn discover(root: &Path, config: &ProjectConfig) -> Discovery {
    let mut out = Discovery::default();
    if !root.is_dir() {
        return out;
    }

    let generated = build_globset(&config.generated);
    let excludes = build_globset(&config.exclude);
    // filter_entry 闭包必须 `'static`：把 `root` 与 `excludes` 转成 owned 克隆。
    let root_owned = root.to_path_buf();
    let excludes_for_filter = excludes.clone();

    let mut builder = ignore::WalkBuilder::new(root);
    builder.standard_filters(true);
    // 即使目录不是 git 仓库也遵守 `.gitignore`（代码健康工具不该因为用户
    // 没 `git init` 就无视忽略规则）。
    builder.require_git(false);
    builder.filter_entry(move |entry| {
        // 内置构建目录：目录本身直接剪枝。
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
            && entry
                .file_name()
                .to_str()
                .map(|n| BUILTIN_IGNORE_DIRS.contains(&n))
                .unwrap_or(false)
        {
            return false;
        }
        // config exclude：匹配目录剪枝；文件级排除在主循环用同一 globset 再判。
        if let Ok(rel) = entry.path().strip_prefix(&root_owned)
            && !rel.as_os_str().is_empty()
        {
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir && excludes_for_filter.is_match(rel_str(rel)) {
                return false;
            }
        }
        true
    });

    let mut lang_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut filtered_seen = 0usize;

    for result in builder.build() {
        let Ok(entry) = result else {
            continue;
        };
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        filtered_seen += 1;
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_path_buf();

        if excludes.is_match(rel_str(&rel)) {
            out.excluded_count += 1;
            continue;
        }
        if generated.is_match(rel_str(&rel)) {
            out.skipped.push(SkippedFile {
                path: rel,
                reason: SkipReason::Generated,
            });
            out.excluded_count += 1;
            continue;
        }

        match language_of(&rel) {
            Some("rust") => out.rust_files.push(rel),
            Some(lang) => {
                *lang_counts.entry(lang).or_insert(0) += 1;
            }
            None => {
                out.skipped.push(SkippedFile {
                    path: rel,
                    reason: SkipReason::UnsupportedLanguage,
                });
                out.excluded_count += 1;
            }
        }
    }

    out.rust_files.sort();
    out.discovered_count = count_files_before_ignore(root);
    out.excluded_count += out.discovered_count.saturating_sub(filtered_seen);

    // 语言统计：Rust 语义分析 + 其它语言仅统计。
    let mut languages: Vec<LanguageSummary> = lang_counts
        .into_iter()
        .map(|(language, files)| LanguageSummary {
            language: language.to_string(),
            files,
            analyzed: false,
        })
        .collect();
    languages.push(LanguageSummary {
        language: "rust".to_string(),
        files: out.rust_files.len(),
        analyzed: true,
    });
    languages.sort_by(|a, b| a.language.cmp(&b.language));
    out.languages = languages;

    out
}

/// 第二次轻量遍历只用来统计被 `.gitignore`/`.ignore`/隐藏规则剪掉的文件。
/// 内置大型构建目录仍然剪枝，避免为了一个计数遍历 target/node_modules。
fn count_files_before_ignore(root: &Path) -> usize {
    let mut builder = ignore::WalkBuilder::new(root);
    builder.standard_filters(false);
    builder.filter_entry(|entry| {
        !(entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
            && entry
                .file_name()
                .to_str()
                .map(|n| BUILTIN_IGNORE_DIRS.contains(&n))
                .unwrap_or(false))
    });
    builder
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .count()
}

fn build_globset(patterns: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for pat in patterns {
        if let Ok(g) = Glob::new(pat) {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "ignored.rs\n").unwrap();
        fs::write(dir.path().join("kept.rs"), "fn a() {}").unwrap();
        fs::write(dir.path().join("ignored.rs"), "fn b() {}").unwrap();
        let cfg = ProjectConfig::default();
        let d = discover(dir.path(), &cfg);
        assert_eq!(d.rust_files, vec![PathBuf::from("kept.rs")]);
        assert_eq!(d.discovered_count, 3, "包含 .gitignore 本身");
        assert_eq!(
            d.excluded_count, 2,
            "ignored.rs 与隐藏规则排除的 .gitignore 本身均应计数"
        );
    }

    #[test]
    fn skips_builtin_build_dirs() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("target")).unwrap();
        fs::write(dir.path().join("target/generated.rs"), "fn g() {}").unwrap();
        fs::write(dir.path().join("src_kept.rs"), "fn k() {}").unwrap();
        let cfg = ProjectConfig::default();
        let d = discover(dir.path(), &cfg);
        assert_eq!(d.rust_files, vec![PathBuf::from("src_kept.rs")]);
    }

    #[test]
    fn respects_config_generated_pattern() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".dozer")).unwrap();
        fs::write(
            dir.path().join(".dozer/code-health.toml"),
            "generated = [\"**/*.generated.rs\"]\n",
        )
        .unwrap();
        fs::write(dir.path().join("a.generated.rs"), "fn g() {}").unwrap();
        fs::write(dir.path().join("b.rs"), "fn b() {}").unwrap();
        let cfg = load_project_config(dir.path());
        let d = discover(dir.path(), &cfg);
        assert_eq!(d.rust_files, vec![PathBuf::from("b.rs")]);
        assert_eq!(d.skipped.len(), 1);
        assert_eq!(d.skipped[0].reason, SkipReason::Generated);
    }

    #[test]
    fn config_exclude_drops_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".dozer")).unwrap();
        fs::write(
            dir.path().join(".dozer/code-health.toml"),
            "exclude = [\"vendor/**\"]\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("vendor")).unwrap();
        fs::write(dir.path().join("vendor/lib.rs"), "fn v() {}").unwrap();
        fs::write(dir.path().join("app.rs"), "fn a() {}").unwrap();
        let cfg = load_project_config(dir.path());
        let d = discover(dir.path(), &cfg);
        assert_eq!(d.rust_files, vec![PathBuf::from("app.rs")]);
        assert!(d.excluded_count >= 1);
    }

    #[test]
    fn records_unsupported_language_as_skip() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "fn a() {}").unwrap();
        fs::write(dir.path().join("b.txt"), "hello").unwrap();
        let cfg = ProjectConfig::default();
        let d = discover(dir.path(), &cfg);
        assert_eq!(d.rust_files, vec![PathBuf::from("a.rs")]);
        assert!(
            d.skipped.iter().any(
                |s| s.path == Path::new("b.txt") && s.reason == SkipReason::UnsupportedLanguage
            )
        );
    }

    #[test]
    fn counts_statistical_languages() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "fn a() {}").unwrap();
        fs::write(dir.path().join("b.py"), "x = 1").unwrap();
        let cfg = ProjectConfig::default();
        let d = discover(dir.path(), &cfg);
        let py = d
            .languages
            .iter()
            .find(|l| l.language == "python")
            .expect("python 进入统计");
        assert_eq!(py.files, 1);
        assert!(!py.analyzed);
        let rust = d.languages.iter().find(|l| l.language == "rust").unwrap();
        assert!(rust.analyzed);
    }

    #[test]
    fn empty_dir_has_empty_scope() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ProjectConfig::default();
        let d = discover(dir.path(), &cfg);
        assert!(d.rust_files.is_empty());
        assert!(d.skipped.is_empty());
    }

    #[test]
    fn missing_config_defaults_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load_project_config(dir.path());
        assert!(cfg.exclude.is_empty());
        assert!(cfg.generated.is_empty());
    }
}
