# bytegit 写操作 `git2` 化评估

**日期：** 2026-10-03
**范围：** bytegit P5 —— `init` / `clone` / `checkout_branch` 能否用 `git2`（libgit2）替代命令行 `git`。
**结论：** `init` 可以换 `git2`；`clone` 与 `checkout_branch` 必须留在命令行。规格 §4.6 与 O2 的结论即本文。

## 环境

- macOS，`git` 2.55.0
- libgit2 1.9.7（`git2 0.21` 的 `libgit2-sys 0.18.8`）
- `git2` **不开任何 feature**（`default = []`，即 `https=false ssh=false`）
- 评估程序在隔离的 `HOME`/配置下运行（`GIT_CONFIG_NOSYSTEM=1`、`GIT_CONFIG_GLOBAL`、`XDG_CONFIG_HOME` 均指向临时目录），可重跑。

## 结论一：`init` → 可以换 `git2`

初始分支遵循 `init.defaultBranch`（`trunk` 配置下命令行与 libgit2 都是 `trunk`；没配置都是 `master`）；写出的 `config` 与命令行**逐键相同**；对已有仓库重复 `init` 都无害；`init.templateDir` 指向的自定义模板都被应用。差异只有无关紧要的：命令行会复制 14 个 `hooks/*.sample` 示例文件，libgit2 只放一个 `hooks/README.sample`；libgit2 总会建 `info/`。

**一个必须手工补齐的差异**：在不存在的目录上，libgit2 会 `mkdir -p` 并成功，命令行（在不存在的工作目录里）根本跑不起来——`bytegit::init` 显式要求目录已存在，保持旧行为。

> 实验陷阱（写进附录 A 的注释）：libgit2 会缓存第一次用到的配置搜索路径，不在每个用例里显式重设 `set_search_path`，后面用例的 `HOME` 切换对它无效，会误以为"libgit2 不读 `init.defaultBranch`"。

## 结论二：`clone` → 必须留命令行

本构建的 libgit2 `https=false ssh=false`：`https://…` 报 `there is no TLS stream available`，`git@…`/`ssh://…` 报 `unsupported URL protocol`。要支持就得给 `git2` 开 `https`/`ssh` feature，拉进 OpenSSL 与 libssh2——与本仓库"走 rustls、不引入 OpenSSL"（见 `dozer-app/Cargo.toml` 里 `reqwest` 的注释）的取向冲突；而且即使开了，libgit2 也读不到 `~/.ssh/config`（主机别名、`ProxyCommand`）、`core.sshCommand`、系统凭据助手的完整配置，用户用命令行能 clone 的仓库用它不一定能。

（本机没有可用的网络与凭据环境，**网络 clone 的认证行为未实测**，以上依据是实测的错误信息与 libgit2 的已知限制。）

## 结论三：`checkout_branch` → 必须留命令行

对 10 个场景，`git2` 的安全 checkout 与命令行**成功/失败一致、最终 HEAD 与工作区一致**：

1. 干净切换；
2. 不冲突的未暂存改动（两分支该文件相同）；
3. 冲突的未暂存改动（两分支该文件不同）；
4. 不冲突的已暂存改动；
5. 冲突的已暂存改动；
6. 未跟踪文件挡路；
7. 已暂存的新文件；
8. 未暂存的删除；
9. 目标就是当前分支；
10. 分支不存在。

但有三处**无法用 `git2` 补上**的差异：

1. **不执行 `post-checkout` hook**（命令行执行）。
2. **不执行外部 smudge/clean 过滤器**：配置 `filter.up.smudge = tr a-z A-Z` 后，命令行切换得到 `A2`，`git2` 得到 `a2`。Git LFS 就是靠这个机制，换成 `git2` 会让 LFS 文件以指针文本落在工作区。
3. **报错信息退化**：冲突时命令行给出多行原文（`error: Your local changes to the following files would be overwritten by checkout: a.txt … Please commit your changes or stash them…`，`files/view.rs` 把它原样显示在分支栏下），`git2` 只有 `1 conflict prevents checkout`；分支不存在时的文案也不同。

这三处已由 `bytegit/src/write.rs` 的 `checkout_branch_runs_the_post_checkout_hook`、`checkout_branch_applies_external_filters` 与三条"报错点名文件"的测试钉死；dozer 侧 `delivery.rs` 的 13 条刻画测试另行固定旧实现的行为。

## 如何重跑

在临时 crate 里：

```bash
cargo add git2@0.21 tempfile
```

把附录 A、B 的两个文件放进 `src/bin/`，然后：

```bash
cargo run --bin init_clone
cargo run --bin checkout
```

---

## 附录 A：评估程序——`init` 与 `clone`（`src/bin/init_clone.rs`）

```rust
//! 评估 `init`/`clone` 能否用 git2 替代命令行(bytegit P5)。
//! 结论:`init` 可以(libgit2 自己会读 `init.defaultBranch`,下面显式接线与否结果相同);
//! `clone` 不行(本构建的 libgit2 没有 https/ssh 传输)。
//! 注意:libgit2 会缓存第一次用到的配置搜索路径,每个用例必须显式重设,否则后面用例的
//! HOME 切换对它无效、会得出错误结论。
//! 运行:`cargo run --bin init_clone`(需要系统 git;只依赖 git2 与 tempfile,不开 https/ssh feature)。
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

fn cli_init(dir: &Path, home: &Path) -> bool {
    Command::new("git")
        .arg("init")
        .current_dir(dir)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("xdg"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .output()
        .unwrap()
        .status
        .success()
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
    let mut es: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    es.sort_by_key(|e| e.file_name());
    for e in es {
        let p = e.path();
        let rel = p.strip_prefix(base).unwrap().to_string_lossy().to_string();
        if rel.starts_with("objects/") && rel != "objects/info" && rel != "objects/pack" {
            continue;
        }
        out.push(rel);
        if p.is_dir() {
            walk(base, &p, out);
        }
    }
}

/// (.git 下的文件清单, HEAD 内容, config 键值)
fn snapshot(repo: &Path) -> (Vec<String>, String, BTreeMap<String, String>) {
    let g = repo.join(".git");
    let mut files = vec![];
    walk(&g, &g, &mut files);
    let head = std::fs::read_to_string(g.join("HEAD")).unwrap().trim().to_string();
    let mut cfg = BTreeMap::new();
    for l in std::fs::read_to_string(g.join("config")).unwrap().lines() {
        if let Some((k, v)) = l.trim().split_once('=') {
            cfg.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    (files, head, cfg)
}

/// `with_default_branch`:git2 一侧是否显式读 `init.defaultBranch` 再传给 `initial_head`。
fn init_case(label: &str, gitconfig: &str, template: bool, with_default_branch: bool) {
    let home = tempfile::tempdir().unwrap();
    let tpl = home.path().join("tpl");
    std::fs::write(
        home.path().join(".gitconfig"),
        gitconfig.replace("TPL", &tpl.to_string_lossy()),
    )
    .unwrap();
    if template {
        std::fs::create_dir_all(tpl.join("hooks")).unwrap();
        std::fs::write(tpl.join("hooks").join("post-checkout"), "#!/bin/sh\n").unwrap();
    }
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let cli_ok = cli_init(a.path(), home.path());
    // libgit2 缓存第一次用到的配置搜索路径,每个用例都要显式重设。
    unsafe {
        git2::opts::set_search_path(git2::ConfigLevel::Global, home.path()).unwrap();
        git2::opts::set_search_path(git2::ConfigLevel::XDG, home.path().join("xdg")).unwrap();
        git2::opts::set_search_path(git2::ConfigLevel::System, home.path().join("nosystem")).unwrap();
    }
    let mut opts = git2::RepositoryInitOptions::new();
    if with_default_branch {
        if let Ok(name) = git2::Config::open_default().and_then(|c| c.get_string("init.defaultbranch")) {
            opts.initial_head(&name);
        }
    }
    let g2_ok = git2::Repository::init_opts(b.path(), &opts).is_ok();
    let ((fa, ha, ca), (fb, hb, cb)) = (snapshot(a.path()), snapshot(b.path()));
    println!("== init [{label}] default-branch-wired={with_default_branch} cli_ok={cli_ok} git2_ok={g2_ok}");
    println!("   HEAD cli={ha:?} git2={hb:?}");
    println!("   files only in cli : {} 个(如 {:?})", fa.iter().filter(|x| !fb.contains(x)).count(), fa.iter().find(|x| !fb.contains(x)));
    println!("   files only in git2: {:?}", fb.iter().filter(|x| !fa.contains(x)).collect::<Vec<_>>());
    let keys: BTreeSet<_> = ca.keys().chain(cb.keys()).cloned().collect();
    let diffs: Vec<_> = keys.into_iter().filter(|k| ca.get(k) != cb.get(k)).collect();
    println!("   config 键差异: {diffs:?}");
}

fn main() {
    init_case("no config", "", false, false);
    init_case("init.defaultBranch=trunk", "[init]\n\tdefaultBranch = trunk\n", false, false);
    init_case("init.defaultBranch=trunk", "[init]\n\tdefaultBranch = trunk\n", false, true);
    init_case("init.templateDir custom", "[init]\n\ttemplateDir = TPL\n", true, true);

    let d = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    cli_init(d.path(), home.path());
    println!("== reinit existing repo: git2_ok={}", git2::Repository::init(d.path()).is_ok());
    let missing = d.path().join("nope");
    println!(
        "== init in nonexistent dir: git2_ok={} (created the dir: {})",
        git2::Repository::init(&missing).is_ok(),
        missing.exists()
    );
    let file = d.path().join("afile");
    std::fs::write(&file, "x").unwrap();
    println!("== init on a file path: git2_ok={}", git2::Repository::init(&file).is_ok());

    let t = tempfile::tempdir().unwrap();
    for url in ["https://example.invalid/x.git", "git@example.invalid:x/y.git", "ssh://git@example.invalid/x/y.git"] {
        let r = git2::Repository::clone(url, t.path().join("c"));
        println!("== git2 clone {url}: ok={} err={:?}", r.is_ok(), r.err().map(|e| e.message().to_string()));
        let _ = std::fs::remove_dir_all(t.path().join("c"));
    }
    let v = git2::Version::get();
    println!("libgit2 {:?}: https={} ssh={}", v.libgit2_version(), v.https(), v.ssh());
}
```

**原始输出：**

```text
== init [no config] default-branch-wired=false cli_ok=true git2_ok=true
   HEAD cli="ref: refs/heads/master" git2="ref: refs/heads/master"
   files only in cli : 14 个(如 "hooks/applypatch-msg.sample")
   files only in git2: ["info", "hooks/README.sample"]
   config 键差异: []
== init [init.defaultBranch=trunk] default-branch-wired=false cli_ok=true git2_ok=true
   HEAD cli="ref: refs/heads/trunk" git2="ref: refs/heads/trunk"
   files only in cli : 14 个(如 "hooks/applypatch-msg.sample")
   files only in git2: ["info", "hooks/README.sample"]
   config 键差异: []
== init [init.defaultBranch=trunk] default-branch-wired=true cli_ok=true git2_ok=true
   HEAD cli="ref: refs/heads/trunk" git2="ref: refs/heads/trunk"
   files only in cli : 14 个(如 "hooks/applypatch-msg.sample")
   files only in git2: ["info", "hooks/README.sample"]
   config 键差异: []
== init [init.templateDir custom] default-branch-wired=true cli_ok=true git2_ok=true
   HEAD cli="ref: refs/heads/master" git2="ref: refs/heads/master"
   files only in cli : 14 个(如 "hooks/applypatch-msg.sample")
   files only in git2: ["info", "hooks/README.sample"]
   config 键差异: []
== reinit existing repo: git2_ok=true
== init in nonexistent dir: git2_ok=true (created the dir: true)
== init on a file path: git2_ok=false
== git2 clone https://example.invalid/x.git: ok=false err=Some("there is no TLS stream available")
== git2 clone git@example.invalid:x/y.git: ok=false err=Some("unsupported URL protocol")
== git2 clone ssh://git@example.invalid/x/y.git: ok=false err=Some("unsupported URL protocol")
libgit2 [1, 9, 7]: https=false ssh=false
```

## 附录 B：评估程序——`checkout`（`src/bin/checkout.rs`）

```rust
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new("git").args(args).current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME","t").env("GIT_AUTHOR_EMAIL","t@t").env("GIT_COMMITTER_NAME","t").env("GIT_COMMITTER_EMAIL","t@t")
        .output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).trim().to_string())
}

/// main: a.txt b.txt ; feature: a.txt changed, c.txt added. 当前在 main。
fn fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap(); let p = d.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "a1\n").unwrap(); std::fs::write(p.join("b.txt"), "b1\n").unwrap();
    git(p, &["add", "."]); git(p, &["commit", "-qm", "base"]);
    git(p, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(p.join("a.txt"), "a2\n").unwrap(); std::fs::write(p.join("c.txt"), "c1\n").unwrap();
    git(p, &["add", "."]); git(p, &["commit", "-qm", "feature"]);
    git(p, &["checkout", "-q", "main"]);
    d
}

fn git2_checkout(repo: &Path, name: &str) -> Result<(), String> {
    let r = git2::Repository::open(repo).map_err(|e| e.message().to_string())?;
    let (obj, reference) = r.revparse_ext(&format!("refs/heads/{name}")).map_err(|e| e.message().to_string())?;
    let _ = reference;
    let mut cb = git2::build::CheckoutBuilder::new();
    cb.safe();
    r.checkout_tree(&obj, Some(&mut cb)).map_err(|e| e.message().to_string())?;
    r.set_head(&format!("refs/heads/{name}")).map_err(|e| e.message().to_string())?;
    Ok(())
}

fn state(p: &Path) -> String {
    let (_, head) = git(p, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let (_, st) = git(p, &["status", "--porcelain"]);
    let mut files = vec![];
    for e in std::fs::read_dir(p).unwrap().flatten() { let n = e.file_name().to_string_lossy().to_string(); if n != ".git" { files.push(format!("{n}={:?}", std::fs::read_to_string(e.path()).unwrap_or_default().trim())); } }
    files.sort();
    format!("HEAD={head} | status={:?} | files={files:?}", st.replace('\n', ";"))
}

fn scenario(label: &str, target: &str, setup: &dyn Fn(&Path)) {
    let a = fixture(); let b = fixture();
    setup(a.path()); setup(b.path());
    let (ok, msg) = git(a.path(), &["checkout", target]);
    let r = git2_checkout(b.path(), target);
    let sa = state(a.path()); let sb = state(b.path());
    let same = ok == r.is_ok() && sa == sb;
    println!("-- {label}: cli_ok={ok} git2_ok={} {}", r.is_ok(), if same { "SAME-STATE" } else { "DIFFERENT" });
    if !same { println!("   cli  : {sa}\n   git2 : {sb}"); }
    println!("   cli msg : {:?}", msg.replace('\n', " / ").chars().take(150).collect::<String>());
    println!("   git2 msg: {:?}", r.err().unwrap_or_default());
}

fn main() {
    scenario("clean switch", "feature", &|_| {});
    scenario("unstaged edit in b.txt (same on both branches)", "feature", &|p| std::fs::write(p.join("b.txt"), "b-local\n").unwrap());
    scenario("unstaged edit in a.txt (differs between branches)", "feature", &|p| std::fs::write(p.join("a.txt"), "a-local\n").unwrap());
    scenario("staged edit in b.txt", "feature", &|p| { std::fs::write(p.join("b.txt"), "b-staged\n").unwrap(); git(p, &["add", "b.txt"]); });
    scenario("staged edit in a.txt (differs)", "feature", &|p| { std::fs::write(p.join("a.txt"), "a-staged\n").unwrap(); git(p, &["add", "a.txt"]); });
    scenario("untracked c.txt would be overwritten", "feature", &|p| std::fs::write(p.join("c.txt"), "mine\n").unwrap());
    scenario("staged brand-new file d.txt", "feature", &|p| { std::fs::write(p.join("d.txt"), "d\n").unwrap(); git(p, &["add", "d.txt"]); });
    scenario("unstaged deletion of b.txt", "feature", &|p| std::fs::remove_file(p.join("b.txt")).unwrap());
    scenario("target == current", "main", &|_| {});
    scenario("missing branch", "nope", &|_| {});
    // hooks and LFS-style filters
    let a = fixture(); let b = fixture();
    for p in [a.path(), b.path()] {
        let hook = p.join(".git/hooks/post-checkout");
        std::fs::write(&hook, "#!/bin/sh\ntouch hook-ran\n").unwrap();
        use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    git(a.path(), &["checkout", "feature"]); let _ = git2_checkout(b.path(), "feature");
    println!("-- post-checkout hook: cli ran={} git2 ran={}", a.path().join("hook-ran").exists(), b.path().join("hook-ran").exists());
    // smudge filter (what git-lfs uses)
    let a = fixture(); let b = fixture();
    for p in [a.path(), b.path()] {
        std::fs::write(p.join(".git/info/attributes"), "a.txt filter=up\n").unwrap();
        git(p, &["config", "filter.up.smudge", "tr a-z A-Z"]); git(p, &["config", "filter.up.clean", "cat"]);
    }
    git(a.path(), &["checkout", "feature"]); let _ = git2_checkout(b.path(), "feature");
    println!("-- external smudge filter: cli a.txt={:?} git2 a.txt={:?}", std::fs::read_to_string(a.path().join("a.txt")).unwrap().trim(), std::fs::read_to_string(b.path().join("a.txt")).unwrap().trim());
    let _: PathBuf = PathBuf::new();
}
```

**原始输出：**

```text
-- clean switch: cli_ok=true git2_ok=true SAME-STATE
   cli msg : "Switched to branch 'feature'"
   git2 msg: ""
-- unstaged edit in b.txt (same on both branches): cli_ok=true git2_ok=true SAME-STATE
   cli msg : "Switched to branch 'feature' / M\tb.txt"
   git2 msg: ""
-- unstaged edit in a.txt (differs between branches): cli_ok=false git2_ok=false SAME-STATE
   cli msg : "error: Your local changes to the following files would be overwritten by checkout: / \ta.txt / Please commit your changes or stash them before you switch branches. / Aborting"
   git2 msg: "1 conflict prevents checkout"
-- staged edit in b.txt: cli_ok=true git2_ok=true SAME-STATE
   cli msg : "Switched to branch 'feature' / M\tb.txt"
   git2 msg: ""
-- staged edit in a.txt (differs): cli_ok=false git2_ok=false SAME-STATE
   cli msg : "error: Your local changes to the following files would be overwritten by checkout: / \ta.txt / Please commit your changes or stash them before you switch branches. / Aborting"
   git2 msg: "1 conflict prevents checkout"
-- untracked c.txt would be overwritten: cli_ok=false git2_ok=false SAME-STATE
   cli msg : "error: The following untracked working tree files would be overwritten by checkout: / \tc.txt / Please move or remove them before you switch branches. / Aborting"
   git2 msg: "1 conflict prevents checkout"
-- staged brand-new file d.txt: cli_ok=true git2_ok=true SAME-STATE
   cli msg : "Switched to branch 'feature' / A\td.txt"
   git2 msg: ""
-- unstaged deletion of b.txt: cli_ok=true git2_ok=true SAME-STATE
   cli msg : "Switched to branch 'feature' / D\tb.txt"
   git2 msg: ""
-- target == current: cli_ok=true git2_ok=true SAME-STATE
   cli msg : "Already on 'main'"
   git2 msg: ""
-- missing branch: cli_ok=false git2_ok=false SAME-STATE
   cli msg : "error: pathspec 'nope' did not match any file(s) known to git"
   git2 msg: "reference 'refs/heads/nope' not found; class=Reference (4); code=NotFound (-3)"
-- post-checkout hook: cli ran=true git2 ran=false
-- external smudge filter: cli a.txt="A2" git2 a.txt="a2"
```
