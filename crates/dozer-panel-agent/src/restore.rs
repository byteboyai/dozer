//! 共享的会话恢复 IO 段。

use dozer_model::{SessionEvent, SessionInfo};
use dozer_services::{BoxFuture, SessionService};
use tokio::sync::mpsc::UnboundedReceiver;

/// 一个已 attach 的、可由产品在 UI 线程装配的存活会话。
pub struct RestoredSession {
    pub info: SessionInfo,
    pub snapshot: Vec<u8>,
    pub events: UnboundedReceiver<SessionEvent>,
}

/// 列出某项目仍存活的会话，并逐个 attach。
///
/// 单个 attach 失败不会妨碍其它会话恢复；列举失败则返回空集合，由调用方
/// 决定是否创建默认 shell。
pub fn fetch_restored(
    svc: &dyn SessionService,
    project_id: i64,
) -> BoxFuture<'_, Vec<RestoredSession>> {
    Box::pin(async move {
        let Ok(sessions) = svc.list_sessions().await else {
            return Vec::new();
        };
        let mut restored = Vec::new();
        for info in sessions
            .into_iter()
            .filter(|info| info.alive && info.project_id == Some(project_id))
        {
            match svc.attach(info.id.clone(), 0).await {
                Ok((snapshot, _next_offset, events)) => restored.push(RestoredSession {
                    info,
                    snapshot,
                    events,
                }),
                Err(error) => {
                    tracing::warn!(session = %info.id, "attach failed while restoring session: {error}")
                }
            }
        }
        restored
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_model::{AgentKind, AgentState};
    use dozer_services::{ServiceError, ServiceErrorKind, SessionAttach, SessionSpec};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    type AttachResults = HashMap<String, Result<Vec<u8>, ServiceError>>;

    #[derive(Clone)]
    struct FakeService {
        sessions: Vec<SessionInfo>,
        attaches: Arc<Mutex<AttachResults>>,
    }

    impl SessionService for FakeService {
        fn list_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionInfo>, ServiceError>> {
            let sessions = self.sessions.clone();
            Box::pin(async move { Ok(sessions) })
        }
        fn create(&self, _: SessionSpec) -> BoxFuture<'_, Result<SessionInfo, ServiceError>> {
            unreachable!("restore never creates sessions")
        }
        fn attach(&self, id: String, _: u64) -> BoxFuture<'_, Result<SessionAttach, ServiceError>> {
            let result = self.attaches.lock().unwrap().remove(&id).unwrap();
            Box::pin(async move {
                let snapshot = result?;
                let (_tx, events) = tokio::sync::mpsc::unbounded_channel();
                Ok((snapshot, 0, events))
            })
        }
        fn write(&self, _: String, _: Vec<u8>) -> BoxFuture<'_, Result<(), ServiceError>> {
            unreachable!()
        }
        fn resize(&self, _: String, _: u16, _: u16) -> BoxFuture<'_, Result<(), ServiceError>> {
            unreachable!()
        }
        fn kill(&self, _: String) -> BoxFuture<'_, Result<(), ServiceError>> {
            unreachable!()
        }
        fn close_with_summary(&self, _: String) -> BoxFuture<'_, Result<(), ServiceError>> {
            unreachable!()
        }
    }

    fn session(id: &str, project_id: Option<i64>, alive: bool) -> SessionInfo {
        SessionInfo {
            id: id.into(),
            name: id.into(),
            command: "sh".into(),
            cwd: "/tmp".into(),
            alive,
            created_ms: 0,
            agent_state: AgentState::Idle,
            transcript_path: None,
            project_id,
            agent: AgentKind::Unknown,
        }
    }

    #[tokio::test]
    async fn fetch_restored_returns_only_live_sessions_of_the_project() {
        let svc = FakeService {
            sessions: vec![
                session("wanted", Some(7), true),
                session("other", Some(8), true),
                session("dead", Some(7), false),
            ],
            attaches: Arc::new(Mutex::new(HashMap::from([(
                String::from("wanted"),
                Ok(b"snapshot".to_vec()),
            )]))),
        };
        let restored = fetch_restored(&svc, 7).await;
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].info.id, "wanted");
        assert_eq!(restored[0].snapshot, b"snapshot");
    }

    #[tokio::test]
    async fn a_failing_attach_skips_that_session_only() {
        let svc = FakeService {
            sessions: vec![
                session("bad", Some(7), true),
                session("good", Some(7), true),
            ],
            attaches: Arc::new(Mutex::new(HashMap::from([
                (
                    String::from("bad"),
                    Err(ServiceError::new(ServiceErrorKind::Rejected, "no")),
                ),
                (String::from("good"), Ok(b"ok".to_vec())),
            ]))),
        };
        let restored = fetch_restored(&svc, 7).await;
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].info.id, "good");
    }
}
