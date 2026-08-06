// crates/dozer-app/src/git_watch.rs
//! 项目工作区的实时文件系统监听(D4):debounce 后触发一次 git 状态刷新,
//! 不用再等用户手动切页签/等回合结束。

use notify::Watcher;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 一次文件系统事件相对这个仓库"值不值得触发刷新"、触发的话算哪一类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relevance {
    /// 工作区文件改动(status/暂存态可能变了)。
    Workdir,
    /// `.git` 引用类文件变了(HEAD/index/refs/packed-refs)——分支切换、
    /// 外部提交、其他 worktree 提交都会碰这几个文件,除了刷新文件树状态,
    /// 调用方通常还想顺带重建 Git Log 快照。
    GitRefs,
}

/// `changed` 是否值得触发刷新,值得的话是哪一类。`.git` 目录整体在
/// `project::HIDDEN` 排除名单里,但其中的 HEAD/index/refs/packed-refs
/// 要单独放行(D4)。
fn is_relevant_path(repo: &Path, changed: &Path) -> Option<Relevance> {
    let rel = changed.strip_prefix(repo).ok()?;
    let mut parts = rel.components();
    let Some(std::path::Component::Normal(first)) = parts.next() else {
        return Some(Relevance::Workdir); // 仓库根自身的事件,极少见,当作相关
    };
    let first = first.to_string_lossy();
    if first == ".git" {
        let rest: PathBuf = parts.collect();
        let rest_str = rest.to_string_lossy();
        if rest_str == "HEAD" || rest_str == "index" || rest_str == "packed-refs" {
            return Some(Relevance::GitRefs);
        }
        if rest.starts_with("refs") {
            return Some(Relevance::GitRefs);
        }
        return None; // .git 下其余内容(objects/ 等)不关心
    }
    if crate::project::HIDDEN.contains(&first.as_ref()) {
        return None;
    }
    Some(Relevance::Workdir)
}

/// 一个存活的监听器。Drop 时自动停止(RAII)——调用方(`Workspace`)不需要
/// 在项目切换/关闭的每个路径上手动喊停,`Workspace` 自己被 drop 掉的时候
/// 这个字段跟着 drop,监听自然停。
pub struct Handle {
    _watcher: notify::RecommendedWatcher,
}

/// 启动一个仓库根的 debounced 监听。`on_change` 在 debounce 窗口(连续事件
/// 间隔小于 `debounce` 就算同一批)结束后,拿这批事件里"最值得上报"的
/// `Relevance` 调用一次——`GitRefs` 优先于 `Workdir`(一批事件里只要有一个
/// 引用类变化,调用方就该顺带重建 Git Log 快照)。
///
/// 内部用 `tokio::sync::mpsc` 把 notify 的同步回调(跑在 notify 自己的后台
/// 线程上)和这里的 debounce 循环(跑在调用方传入的 tokio runtime 上)串起来,
/// 不引入 `notify-debouncer-*` 系列额外依赖(规格 §4 只 accept 了 `notify`
/// 本身)。
pub fn start(
    handle: &tokio::runtime::Handle,
    repo: PathBuf,
    debounce: Duration,
    mut on_change: impl FnMut(Relevance) + Send + 'static,
) -> notify::Result<Handle> {
    // FSEvents 上报的是路径形如 `/private/var/...`,而 `repo` 常常是符号
    // 链接后的 `/var/...`(macOS 上 `/var` → `/private/var`)。不统一会让
    // `strip_prefix` 失败、所有事件都被当成无关。这里先 canonicalize,让
    // `watch` 与 `is_relevant_path` 都基于同一套规范路径比对。
    let repo = repo.canonicalize().unwrap_or(repo);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Relevance>();
    let filter_repo = repo.clone();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        let mut best: Option<Relevance> = None;
        for path in &event.paths {
            match is_relevant_path(&filter_repo, path) {
                Some(Relevance::GitRefs) => {
                    best = Some(Relevance::GitRefs);
                    break; // GitRefs 优先级最高,找到就不用再看这批里其余路径
                }
                Some(Relevance::Workdir) => best = best.or(Some(Relevance::Workdir)),
                None => {}
            }
        }
        if let Some(r) = best {
            let _ = tx.send(r);
        }
    })?;
    watcher.watch(&repo, notify::RecursiveMode::Recursive)?;

    handle.spawn(async move {
        while let Some(first) = rx.recv().await {
            let mut best = first;
            // 合并这个 debounce 窗口内接下来到达的信号;超时或 channel 关闭即止
            while let Ok(Some(r)) = tokio::time::timeout(debounce, rx.recv()).await {
                if r == Relevance::GitRefs {
                    best = Relevance::GitRefs;
                }
            }
            on_change(best);
        }
    });

    Ok(Handle { _watcher: watcher })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workdir_file_is_relevant() {
        let repo = Path::new("/r");
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/src/main.rs")),
            Some(Relevance::Workdir)
        );
    }

    #[test]
    fn hidden_dirs_are_not_relevant() {
        let repo = Path::new("/r");
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/target/debug/foo")),
            None
        );
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/node_modules/x/index.js")),
            None
        );
        assert_eq!(is_relevant_path(repo, Path::new("/r/.DS_Store")), None);
    }

    #[test]
    fn git_control_files_are_relevant_as_git_refs() {
        let repo = Path::new("/r");
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/.git/HEAD")),
            Some(Relevance::GitRefs)
        );
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/.git/index")),
            Some(Relevance::GitRefs)
        );
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/.git/refs/heads/main")),
            Some(Relevance::GitRefs)
        );
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/.git/packed-refs")),
            Some(Relevance::GitRefs)
        );
    }

    #[test]
    fn other_git_internals_are_not_relevant() {
        let repo = Path::new("/r");
        assert_eq!(
            is_relevant_path(repo, Path::new("/r/.git/objects/ab/cdef")),
            None
        );
    }

    #[test]
    fn start_debounces_rapid_writes_into_one_callback() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        std::fs::create_dir_all(repo.join(".git")).unwrap(); // 让 .git 存在,贴近真实仓库

        let rt = tokio::runtime::Runtime::new().unwrap();
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count2 = count.clone();

        let _handle = rt.block_on(async {
            start(
                &tokio::runtime::Handle::current(),
                repo.clone(),
                Duration::from_millis(100),
                move |_relevance| {
                    count2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                },
            )
            .expect("watcher 应能启动")
        });

        // 100ms debounce 窗口内连续写 5 次
        for i in 0..5 {
            std::fs::write(repo.join(format!("f{i}.txt")), "x").unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
        // 等 debounce 窗口过完 + 一点余量(FSEvents 异步投递,多等一点)
        rt.block_on(async { tokio::time::sleep(Duration::from_millis(500)).await });

        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "5 次快速写入应合并成 1 次回调"
        );
    }

    #[test]
    fn start_ignores_hidden_dir_writes() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        std::fs::create_dir_all(repo.join("target")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count2 = count.clone();

        let _handle = rt.block_on(async {
            start(
                &tokio::runtime::Handle::current(),
                repo.clone(),
                Duration::from_millis(100),
                move |_relevance| {
                    count2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                },
            )
            .unwrap()
        });

        std::fs::write(repo.join("target").join("build-artifact"), "x").unwrap();
        rt.block_on(async { tokio::time::sleep(Duration::from_millis(300)).await });

        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "target/ 下的改动不该触发回调"
        );
    }
}
