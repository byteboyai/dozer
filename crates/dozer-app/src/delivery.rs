//! 交付检测与沉淀：git CLI 薄封装（spec P1f D3/D4/D6）。
//! 全部同步阻塞——调用方负责放进 tokio 任务，不许在 UI 线程直呼。

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const ACCEPTED_REF_PREFIX: &str = "refs/dozer/accepted/";

#[derive(Debug, Clone, PartialEq)]
pub struct FileChange {
    pub path: String,
    /// None = 二进制或未跟踪（numstat 给不出行数）
    pub added: Option<u32>,
    pub removed: Option<u32>,
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let out = git(dir, &["rev-parse", "--show-toplevel"])?;
    let line = out.lines().next()?.trim();
    (!line.is_empty()).then(|| PathBuf::from(line))
}

pub fn is_dirty(repo: &Path) -> bool {
    git(repo, &["status", "--porcelain"]).is_some_and(|s| !s.trim().is_empty())
}

pub fn head_commit(repo: &Path) -> Option<String> {
    let out = git(repo, &["rev-parse", "HEAD"])?;
    let line = out.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

/// 现存最大号 accepted ref：`(n, commit)`。
pub fn last_accepted(repo: &Path) -> Option<(u32, String)> {
    let out = git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            ACCEPTED_REF_PREFIX,
        ],
    )?;
    out.lines()
        .filter_map(|l| {
            let (name, commit) = l.split_once(' ')?;
            let n: u32 = name.strip_prefix(ACCEPTED_REF_PREFIX)?.parse().ok()?;
            Some((n, commit.to_string()))
        })
        .max_by_key(|(n, _)| *n)
}

/// 变更清单：基线=最大 accepted ref（无则 HEAD）对工作区 numstat + 未跟踪文件。
pub fn changes(repo: &Path) -> Vec<FileChange> {
    let mut out = Vec::new();
    let base = last_accepted(repo)
        .map(|(n, _)| format!("{ACCEPTED_REF_PREFIX}{n}"))
        .or_else(|| head_commit(repo).map(|_| "HEAD".to_string()));
    if let Some(base) = base
        && let Some(numstat) = git(repo, &["diff", "--numstat", &base])
    {
        for line in numstat.lines() {
            let mut parts = line.split('\t');
            let (Some(a), Some(r), Some(p)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            out.push(FileChange {
                path: p.to_string(),
                added: a.parse().ok(),
                removed: r.parse().ok(),
            });
        }
    }
    if let Some(status) = git(repo, &["status", "--porcelain"]) {
        for line in status.lines() {
            if let Some(p) = line.strip_prefix("?? ") {
                out.push(FileChange {
                    path: p.trim().to_string(),
                    added: None,
                    removed: None,
                });
            }
        }
    }
    out
}

/// 通过·沉淀：脏工作区拒绝（沉淀物必须是提交，spec D6）。
pub fn accept(repo: &Path) -> Result<u32> {
    if is_dirty(repo) {
        bail!("有未提交变更，先让 agent 提交再沉淀");
    }
    let head = head_commit(repo).context("仓库没有任何提交")?;
    let n = last_accepted(repo).map(|(n, _)| n + 1).unwrap_or(1);
    let refname = format!("{ACCEPTED_REF_PREFIX}{n}");
    let ok = Command::new("git")
        .args(["update-ref", &refname, &head])
        .current_dir(repo)
        .status()
        .context("git update-ref")?
        .success();
    if !ok {
        bail!("写沉淀 ref 失败: {refname}");
    }
    Ok(n)
}

/// 当前分支名（`git rev-parse --abbrev-ref HEAD`）；非 git / 无提交返回 None。
pub fn branch(repo: &Path) -> Option<String> {
    let out = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let line = out.lines().next()?.trim();
    (!line.is_empty() && line != "HEAD").then(|| line.to_string())
}

/// spec D3 的"有变更"精确定义（纯函数，方便矩阵测试）。
pub fn delivery_pending(
    dirty: bool,
    head: Option<&str>,
    accepted: Option<&str>,
    last_turn_head: Option<&str>,
) -> bool {
    if dirty {
        return true;
    }
    match accepted {
        Some(a) => head != Some(a),
        None => head != last_turn_head,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// tempdir 里造一个带一次提交的真 git 仓库
    fn mkrepo() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let st = Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(st.status.success(), "git {args:?}: {st:?}");
        };
        git(&["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "c1"]);
        (dir, repo)
    }

    #[test]
    fn repo_root_and_head_and_dirty() {
        let (_d, repo) = mkrepo();
        let sub = repo.join("sub");
        std::fs::create_dir(&sub).unwrap();
        assert_eq!(
            repo_root(&sub).unwrap().canonicalize().unwrap(),
            repo.canonicalize().unwrap()
        );
        assert!(repo_root(std::path::Path::new("/")).is_none());
        assert!(head_commit(&repo).is_some());
        assert!(!is_dirty(&repo));
        std::fs::write(repo.join("a.txt"), "two\n").unwrap();
        assert!(is_dirty(&repo));
    }

    #[test]
    fn accept_writes_sequential_refs_and_refuses_dirty() {
        let (_d, repo) = mkrepo();
        assert!(last_accepted(&repo).is_none());
        assert_eq!(accept(&repo).unwrap(), 1);
        let (n, commit) = last_accepted(&repo).unwrap();
        assert_eq!(n, 1);
        assert_eq!(commit, head_commit(&repo).unwrap());
        // 再提交一轮 → v2
        std::fs::write(repo.join("b.txt"), "x\n").unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap()
        };
        git(&["add", "."]);
        git(&["commit", "-qm", "c2"]);
        assert_eq!(accept(&repo).unwrap(), 2);
        // 脏工作区拒绝
        std::fs::write(repo.join("a.txt"), "dirty\n").unwrap();
        assert!(accept(&repo).is_err());
    }

    #[test]
    fn changes_lists_modified_and_untracked_against_baseline() {
        let (_d, repo) = mkrepo();
        accept(&repo).unwrap(); // 基线 v1
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap(); // 修改
        std::fs::write(repo.join("new.txt"), "n\n").unwrap(); // 未跟踪
        let ch = changes(&repo);
        let a = ch.iter().find(|c| c.path == "a.txt").expect("a.txt");
        assert_eq!(a.added, Some(1));
        assert_eq!(a.removed, Some(0));
        let n = ch.iter().find(|c| c.path == "new.txt").expect("new.txt");
        assert_eq!(n.added, None, "未跟踪无行数");
    }

    #[test]
    fn branch_of_repo() {
        let (_d, repo) = mkrepo();
        let b = branch(&repo).expect("有分支");
        assert!(b == "main" || b == "master", "分支名: {b}");
        assert!(branch(std::path::Path::new("/")).is_none(), "非 git 无分支");
    }

    #[test]
    fn delivery_pending_matrix() {
        // 脏 → 恒 pending
        assert!(delivery_pending(true, Some("h"), None, Some("h")));
        // 有沉淀 ref：HEAD 偏离才 pending
        assert!(delivery_pending(false, Some("h2"), Some("h1"), None));
        assert!(!delivery_pending(false, Some("h1"), Some("h1"), None));
        // 无 ref：与上回合 HEAD 比
        assert!(delivery_pending(false, Some("h2"), None, Some("h1")));
        assert!(!delivery_pending(false, Some("h1"), None, Some("h1")));
    }
}
