//! 预览上下文的内存态缓存：`dozer-app` 推、`dozer-mcp` 查。纯内存、不落盘
//! ——daemon 重启即清空，下次 `dozer-app` 一变化就会重新推（见设计文档
//! "非目标"一节）。

use dozer_core::protocol::PreviewContext;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct PreviewContextStore {
    inner: Mutex<HashMap<i64, PreviewContext>>,
}

impl PreviewContextStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// `context: None` 表示清除该项目当前记录（当前无活动文本预览）。
    pub fn update(&self, project_id: i64, context: Option<PreviewContext>) {
        let mut map = self.inner.lock().expect("preview_context 锁");
        match context {
            Some(ctx) => {
                map.insert(project_id, ctx);
            }
            None => {
                map.remove(&project_id);
            }
        }
    }

    pub fn get(&self, project_id: i64) -> Option<PreviewContext> {
        self.inner
            .lock()
            .expect("preview_context 锁")
            .get(&project_id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(path: &str) -> PreviewContext {
        PreviewContext {
            path: path.into(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 1,
            has_selection: false,
            updated_at_ms: 1000,
        }
    }

    #[test]
    fn get_before_any_update_returns_none() {
        let store = PreviewContextStore::new();
        assert_eq!(store.get(1), None);
    }

    #[test]
    fn update_then_get_round_trips() {
        let store = PreviewContextStore::new();
        store.update(1, Some(ctx("/a.rs")));
        assert_eq!(store.get(1), Some(ctx("/a.rs")));
    }

    #[test]
    fn second_update_overwrites_first() {
        let store = PreviewContextStore::new();
        store.update(1, Some(ctx("/a.rs")));
        store.update(1, Some(ctx("/b.rs")));
        assert_eq!(store.get(1), Some(ctx("/b.rs")));
    }

    #[test]
    fn update_with_none_clears() {
        let store = PreviewContextStore::new();
        store.update(1, Some(ctx("/a.rs")));
        store.update(1, None);
        assert_eq!(store.get(1), None);
    }

    #[test]
    fn different_projects_are_independent() {
        let store = PreviewContextStore::new();
        store.update(1, Some(ctx("/a.rs")));
        assert_eq!(store.get(2), None);
        assert_eq!(store.get(1), Some(ctx("/a.rs")));
    }
}
