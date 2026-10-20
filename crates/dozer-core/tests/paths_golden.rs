//! 产品身份改造(S6a Task 1)的黄金测试:锁定 Dozer 默认产品的
//! 数据目录、socket、日志目录在给定 `HOME` 下的**字节级**取值。
//! 改造 `paths.rs` 前先在旧代码上跑通并记录期望值;改造后本文件
//! **不得修改**且必须原样通过——即证明用户可见行为未变。
//!
//! `HOME` 是进程级环境变量,同进程并行测试会互相踩;因此所有
//! HOME 敏感的断言放在同一个 `#[test]` 里顺序执行。

use dozer_core::paths::{ProductId, config_dir, logs_dir, product, socket_path, state_dir};
use std::path::Path;

/// 固定的假 HOME:路径推导只读环境变量、不落盘,目录无需真实存在。
const GOLDEN_HOME: &str = "/tmp/dozer-paths-golden-home";

#[test]
fn default_paths_are_byte_identical_to_the_pre_product_refactor() {
    unsafe { std::env::set_var("HOME", GOLDEN_HOME) };

    let home = Path::new(GOLDEN_HOME);
    let app_support = home.join("Library").join("Application Support");
    // directories 6 在 macOS 上用 "qualifier.organization.application"
    // 作为目录名(ProjectDirs 带 qualifier 时不再只用 application);
    // `state_dir()` 在 macOS 为 None,现有代码回退 `data_local_dir()`,
    // 与 config 同目录。这些都是改造前的现状,黄金测试照单全收。
    let base = app_support.join("ai.byteboy.dozer");

    assert_eq!(config_dir(), base, "config_dir()");
    assert_eq!(state_dir(), base, "state_dir()");
    assert_eq!(socket_path(), base.join("dozerd.sock"), "socket_path()");
    assert_eq!(logs_dir(), base.join("logs"), "logs_dir()");
}

/// 未安装其它产品时,进程身份钉死为 Dozer:默认产品的全套命名
/// 必须与 `ProductId::dozer()` 一致。
#[test]
fn product_default_is_dozer() {
    let p = product();
    assert_eq!(p.name, "dozer");
    assert_eq!(p.session_env, "DOZER_SESSION_ID");
    assert_eq!(p.hook_bin, "dozer-hook");
    assert_eq!(p.socket_file, "dozerd.sock");
    assert_eq!(p.db_file, "dozer.db");
    assert_eq!(*p, ProductId::dozer(), "默认产品就是 dozer()");
}
