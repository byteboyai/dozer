use crate::session::{Session, SessionSpec};
use anyhow::{Result, anyhow};
use dozer_core::protocol::SessionInfo;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct SessionRegistry {
    map: Mutex<HashMap<String, Arc<Session>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&self, spec: SessionSpec) -> Result<Arc<Session>> {
        let s = Arc::new(Session::spawn(spec)?);
        self.map
            .lock()
            .expect("registry lock")
            .insert(s.id().to_string(), s.clone());
        Ok(s)
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.map.lock().expect("registry lock").get(id).cloned()
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let mut v: Vec<SessionInfo> = self
            .map
            .lock()
            .expect("registry lock")
            .values()
            .map(|s| s.info())
            .collect();
        v.sort_by_key(|i| i.created_ms);
        v
    }

    pub fn kill(&self, id: &str) -> Result<()> {
        self.get(id)
            .ok_or_else(|| anyhow!("会话不存在: {id}"))?
            .kill()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionSpec;
    use std::time::Duration;

    fn spec(cmd: &str) -> SessionSpec {
        SessionSpec {
            name: "t".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), cmd.into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
            project_id: 1,
        }
    }

    #[tokio::test]
    async fn create_get_list_roundtrip() {
        let reg = SessionRegistry::new();
        let s = reg.create(spec("sleep 5")).unwrap();
        let id = s.id().to_string();
        assert!(reg.get(&id).is_some());
        assert_eq!(reg.list().len(), 1);
        assert_eq!(reg.list()[0].id, id);
        reg.kill(&id).unwrap();
    }

    #[tokio::test]
    async fn killed_session_stays_listed_with_scrollback() {
        let reg = SessionRegistry::new();
        let s = reg.create(spec("printf tomb; sleep 5")).unwrap();
        let id = s.id().to_string();
        // 等输出落缓冲
        for _ in 0..100 {
            if s.snapshot().0.windows(4).any(|w| w == b"tomb") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        reg.kill(&id).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let listed = reg.list();
        assert_eq!(listed.len(), 1, "dead session remains listed");
        assert!(!listed[0].alive);
        let (data, _) = reg.get(&id).unwrap().snapshot();
        assert!(
            data.windows(4).any(|w| w == b"tomb"),
            "scrollback survives kill"
        );
    }

    #[tokio::test]
    async fn get_unknown_is_none_and_kill_unknown_errs() {
        let reg = SessionRegistry::new();
        assert!(reg.get("nope").is_none());
        assert!(reg.kill("nope").is_err());
    }
}
