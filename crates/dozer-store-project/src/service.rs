//! 进程内服务实现:把 SQLite 存储包成 `dozer-services` 的 trait,供不经 `dozerd` 的产品(Digger)直接使用。
//!
//! SQLite 调用是同步且很快的;future 在构造时就已算完(`Box::pin(async move { r })`),与 `dozer-app` 的
//! 服务假实现同型,因此在任何执行器上都不会阻塞调度。

use crate::{MemoryStore, ProjectStore};
use dozer_model::{MemoryDetail, MemoryInfo, ProjectInfo};
use dozer_services::{BoxFuture, MemoryService, ProjectService, ServiceError, ServiceErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn rejected(e: anyhow::Error) -> ServiceError {
    ServiceError::new(ServiceErrorKind::Rejected, format!("{e:#}"))
}

/// `ProjectService` 的进程内实现。Project 就是磁盘上的一个目录。
#[derive(Clone)]
pub struct LocalProjectService {
    store: Arc<ProjectStore>,
}

impl LocalProjectService {
    pub fn new(store: Arc<ProjectStore>) -> Self {
        Self { store }
    }
}

impl ProjectService for LocalProjectService {
    fn rename_project(
        &self,
        id: i64,
        name: String,
    ) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>> {
        let trimmed = name.trim();
        // 与 dozerd 的 RenameProject 同规则:空名字不是合法名字。
        let r = if trimmed.is_empty() {
            Err(ServiceError::new(ServiceErrorKind::Rejected, "名称不能为空"))
        } else {
            self.store.rename(id, trimmed).map(Some).map_err(rejected)
        };
        Box::pin(async move { r })
    }

    fn remove_project(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        let r = self.store.remove(id).map_err(rejected);
        Box::pin(async move { r })
    }

    fn open_project(
        &self,
        path: String,
    ) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>> {
        // 契约允许"路径不存在"返回 `Ok(None)`:不是目录就不登记。
        // 末尾分隔符与 `.` 段不应造出第二个项目:`projects.path` 是 UNIQUE 的原样文本,先规整成同一种写法。
        let normalized: PathBuf = Path::new(&path).components().collect();
        let r = if normalized.is_dir() {
            self.store.open(&normalized.to_string_lossy()).map(Some).map_err(rejected)
        } else {
            Ok(None)
        };
        Box::pin(async move { r })
    }

    fn list_projects(&self) -> BoxFuture<'_, Result<Vec<ProjectInfo>, ServiceError>> {
        let r = self.store.list().map_err(rejected);
        Box::pin(async move { r })
    }
}

/// `MemoryService` 的进程内实现。
#[derive(Clone)]
pub struct LocalMemoryService {
    store: Arc<MemoryStore>,
}

impl LocalMemoryService {
    pub fn new(store: Arc<MemoryStore>) -> Self {
        Self { store }
    }
}

impl MemoryService for LocalMemoryService {
    fn list_memories(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<MemoryInfo>, ServiceError>> {
        let r = self.store.list(project_id).map_err(rejected);
        Box::pin(async move { r })
    }

    fn get_memory(
        &self,
        project_id: i64,
        id: i64,
    ) -> BoxFuture<'_, Result<MemoryDetail, ServiceError>> {
        let r = self.store.get(project_id, id).map_err(rejected);
        Box::pin(async move { r })
    }

    fn write_memory(
        &self,
        project_id: i64,
        title: String,
        kind: String,
        description: String,
        body: String,
        actor: String,
    ) -> BoxFuture<'_, Result<MemoryDetail, ServiceError>> {
        let r = self
            .store
            .write(project_id, &title, &kind, &description, &body, &actor)
            .map_err(rejected);
        Box::pin(async move { r })
    }

    fn delete_memory(
        &self,
        project_id: i64,
        id: i64,
        actor: String,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        let r = self.store.delete(project_id, id, &actor).map_err(rejected);
        Box::pin(async move { r })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    fn services() -> (tempfile::TempDir, LocalProjectService, LocalMemoryService) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("projects.db");
        let p = LocalProjectService::new(Arc::new(ProjectStore::new(&db).unwrap()));
        let m = LocalMemoryService::new(Arc::new(MemoryStore::new(&db).unwrap()));
        (dir, p, m)
    }

    #[test]
    fn open_registers_a_directory_and_a_second_open_returns_the_same_id() {
        let (dir, p, _) = services();
        let proj = dir.path().join("book");
        std::fs::create_dir(&proj).unwrap();
        let path = proj.to_string_lossy().into_owned();
        let a = block_on(p.open_project(path.clone())).unwrap().unwrap();
        let b = block_on(p.open_project(path)).unwrap().unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.name, "book");
        assert!(b.last_active_ms > a.last_active_ms, "再次打开只刷新活跃戳");
        assert_eq!(a.created_ms, b.created_ms);
    }

    #[test]
    fn a_trailing_separator_or_dot_segment_is_the_same_project() {
        let (dir, p, _) = services();
        let proj = dir.path().join("book");
        std::fs::create_dir(&proj).unwrap();
        let plain = proj.to_string_lossy().into_owned();
        let a = block_on(p.open_project(plain.clone())).unwrap().unwrap();
        let b = block_on(p.open_project(format!("{plain}/"))).unwrap().unwrap();
        let c = block_on(p.open_project(format!("{plain}/."))).unwrap().unwrap();
        assert_eq!((a.id, a.id), (b.id, c.id));
        assert_eq!(block_on(p.list_projects()).unwrap().len(), 1);
    }

    #[test]
    fn open_of_a_missing_path_or_a_file_is_none_and_registers_nothing() {
        let (dir, p, _) = services();
        let file = dir.path().join("note.md");
        std::fs::write(&file, "x").unwrap();
        for path in [dir.path().join("nope"), file] {
            let got = block_on(p.open_project(path.to_string_lossy().into_owned())).unwrap();
            assert_eq!(got, None);
        }
        assert!(block_on(p.list_projects()).unwrap().is_empty());
    }

    #[test]
    fn a_non_git_directory_updates_at_its_last_active_time() {
        let (dir, p, _) = services();
        let proj = dir.path().join("plain");
        std::fs::create_dir(&proj).unwrap();
        let info = block_on(p.open_project(proj.to_string_lossy().into_owned()))
            .unwrap()
            .unwrap();
        assert_eq!(info.updated_ms, info.last_active_ms);
    }

    #[test]
    fn rename_trims_and_rejects_blank_names() {
        let (dir, p, _) = services();
        let proj = dir.path().join("a");
        std::fs::create_dir(&proj).unwrap();
        let info = block_on(p.open_project(proj.to_string_lossy().into_owned()))
            .unwrap()
            .unwrap();
        let renamed = block_on(p.rename_project(info.id, "  新名字 ".into()))
            .unwrap()
            .unwrap();
        assert_eq!(renamed.name, "新名字");
        let e = block_on(p.rename_project(info.id, "   ".into())).unwrap_err();
        assert_eq!(e.kind, ServiceErrorKind::Rejected);
        let e = block_on(p.rename_project(9999, "x".into())).unwrap_err();
        assert_eq!(e.kind, ServiceErrorKind::Rejected, "不存在的 id 是明确的拒绝,不是静默成功");
    }

    #[test]
    fn remove_deregisters_but_leaves_the_directory_alone() {
        let (dir, p, _) = services();
        let proj = dir.path().join("keep");
        std::fs::create_dir(&proj).unwrap();
        let info = block_on(p.open_project(proj.to_string_lossy().into_owned()))
            .unwrap()
            .unwrap();
        block_on(p.remove_project(info.id)).unwrap();
        assert!(block_on(p.list_projects()).unwrap().is_empty());
        assert!(proj.is_dir());
        assert!(block_on(p.remove_project(info.id)).is_err());
        // 重新登记同一目录拿到的是新 id:旧 id 复用的话,残留在旧 id 下的共享记忆会挂到"新"项目上。
        let again = block_on(p.open_project(proj.to_string_lossy().into_owned()))
            .unwrap()
            .unwrap();
        assert!(again.id > info.id);
    }

    #[test]
    fn list_is_most_recently_opened_first() {
        let (dir, p, _) = services();
        let mut ids = vec![];
        for n in ["a", "b"] {
            let d = dir.path().join(n);
            std::fs::create_dir(&d).unwrap();
            ids.push(
                block_on(p.open_project(d.to_string_lossy().into_owned()))
                    .unwrap()
                    .unwrap()
                    .id,
            );
        }
        let listed: Vec<i64> = block_on(p.list_projects())
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(listed, vec![ids[1], ids[0]]);
    }

    #[test]
    fn memory_write_is_an_upsert_by_title_and_keeps_history() {
        let (_dir, _, m) = services();
        let first = block_on(m.write_memory(
            1, "标题".into(), "project".into(), "d".into(), "正文一".into(), "user".into(),
        ))
        .unwrap();
        let second = block_on(m.write_memory(
            1, "标题".into(), "project".into(), "d".into(), "正文二".into(), "user".into(),
        ))
        .unwrap();
        assert_eq!(first.id, second.id);
        let listed = block_on(m.list_memories(1)).unwrap();
        assert_eq!(listed.len(), 1);
        let got = block_on(m.get_memory(1, first.id)).unwrap();
        assert_eq!(got.body, "正文二");
        assert!(got.history.len() >= 2, "created + updated 两条审计");
        block_on(m.delete_memory(1, first.id, "user".into())).unwrap();
        assert!(block_on(m.list_memories(1)).unwrap().is_empty());
    }

    #[test]
    fn memories_of_different_projects_do_not_mix() {
        let (_dir, _, m) = services();
        block_on(m.write_memory(1, "t".into(), "user".into(), "".into(), "a".into(), "u".into())).unwrap();
        block_on(m.write_memory(2, "t".into(), "user".into(), "".into(), "b".into(), "u".into())).unwrap();
        assert_eq!(block_on(m.list_memories(1)).unwrap().len(), 1);
        assert_eq!(block_on(m.list_memories(2)).unwrap().len(), 1);
    }
}
