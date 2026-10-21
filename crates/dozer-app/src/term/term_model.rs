//! 终端模型已抽到共享库 `dozer-panel-term`(S6b)。本文件只保留转调,
//! 让既有 `crate::term::term_model::*` 调用点不需要改动。

pub use dozer_panel_term::model::*;

// 终端视图仍在 `dozer-app`(S6c 才搬),它的测试用这把锁串行化配色方案
// 切换。`dozer-panel-term` 的 `#[cfg(test)]` 锁在依赖里不可见,这里临时
// 保留一份;视图搬进库后(S6b Task 3)删除。
#[cfg(test)]
pub(crate) static SCHEME_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn lock_scheme() -> std::sync::MutexGuard<'static, ()> {
    SCHEME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
