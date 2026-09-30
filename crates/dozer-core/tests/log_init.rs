//! `init_at` 装的是进程级全局 subscriber,只能装一次,所以整个初始化流程放进
//! 同一个 `#[test]`(独立进程),按"正常目录 → 不可创建目录"顺序执行。
#![cfg(feature = "logging")]

use dozer_core::log::{Component, Console, init_at};
use std::fs;

dozer_core::scope!(LOG, panel, "todo");

#[test]
fn init_at_writes_banner_and_scoped_lines_then_degrades_without_panic() {
    let dir = tempfile::tempdir().unwrap();
    let guard = init_at(Component::App, "9.9.9", dir.path(), Console::None);
    dozer_core::log_warn!(LOG, "hello-from-todo");
    drop(guard); // 释放 WorkerGuard,把缓冲刷到文件

    let file = fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .find(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("dozer-app.log.")
        })
        .expect("应生成 dozer-app.log.<日期>");
    let text = fs::read_to_string(file.path()).unwrap();
    assert!(text.contains("9.9.9"), "缺启动横幅版本: {text}");
    assert!(text.contains("dozer::module::log"), "缺横幅来源: {text}");
    assert!(text.contains("dozer::panel::todo"), "缺面板来源: {text}");
    assert!(text.contains("hello-from-todo"), "{text}");

    // 日志目录不可创建(位置被普通文件占着);全局 subscriber 也已装过:
    // 两种降级叠加,都必须静默而不是 panic。
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, "x").unwrap();
    let _guard = init_at(
        Component::Daemon,
        "9.9.9",
        &blocker.join("logs"),
        Console::None,
    );
}
