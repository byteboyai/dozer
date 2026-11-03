use crate::session::{Session, SessionSpec};
use anyhow::{Result, anyhow};
use dozer_core::protocol::SessionInfo;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const DEFAULT_TOTAL_HISTORY_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_MAX_EXITED: usize = 128;
const DEFAULT_HISTORY_TTL_MS: u64 = 24 * 60 * 60 * 1000;
const DEFAULT_MAX_METADATA: usize = 1024;

#[derive(Debug, Clone, Copy)]
struct HistoryLimits {
    total_bytes: usize,
    max_exited: usize,
    ttl_ms: u64,
    max_metadata: usize,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        Self {
            total_bytes: DEFAULT_TOTAL_HISTORY_BYTES,
            max_exited: DEFAULT_MAX_EXITED,
            ttl_ms: DEFAULT_HISTORY_TTL_MS,
            max_metadata: DEFAULT_MAX_METADATA,
        }
    }
}

#[derive(Default)]
struct CreationGate {
    draining: bool,
    pending: usize,
}
struct PendingCreate<'a>(&'a Mutex<CreationGate>);
impl Drop for PendingCreate<'_> {
    fn drop(&mut self) {
        self.0.lock().expect("creation gate").pending -= 1;
    }
}

pub struct SessionRegistry {
    map: Mutex<HashMap<String, Arc<Session>>>,
    limits: HistoryLimits,
    creation_gate: Mutex<CreationGate>,
    shutdown_deadline: std::sync::OnceLock<std::time::Instant>,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            creation_gate: Mutex::new(CreationGate::default()),
            shutdown_deadline: std::sync::OnceLock::new(),
            limits: HistoryLimits::default(),
        }
    }

    #[cfg(test)]
    fn with_limits(
        total_bytes: usize,
        max_exited: usize,
        ttl_ms: u64,
        max_metadata: usize,
    ) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            creation_gate: Mutex::new(CreationGate::default()),
            shutdown_deadline: std::sync::OnceLock::new(),
            limits: HistoryLimits {
                total_bytes,
                max_exited,
                ttl_ms,
                max_metadata,
            },
        }
    }

    pub fn create(&self, spec: SessionSpec) -> Result<Arc<Session>> {
        self.create_with(|| Session::spawn(spec))
    }
    fn create_with(&self, spawn: impl FnOnce() -> Result<Session>) -> Result<Arc<Session>> {
        {
            let mut gate = self.creation_gate.lock().expect("creation gate");
            if gate.draining {
                anyhow::bail!("Unavailable: daemon is draining");
            }
            let mut map = self.map.lock().expect("registry lock");
            let mut admission_limits = self.limits;
            if self.limits.total_bytes >= crate::ring::SCROLLBACK_CAP {
                admission_limits.total_bytes = self
                    .limits
                    .total_bytes
                    .saturating_sub((gate.pending + 1) * crate::ring::SCROLLBACK_CAP);
            }
            enforce_limits(&mut map, admission_limits);
            if self.limits.total_bytes >= crate::ring::SCROLLBACK_CAP
                && reserved_history_bytes(&map) + (gate.pending + 1) * crate::ring::SCROLLBACK_CAP
                    > self.limits.total_bytes
            {
                anyhow::bail!("Busy: terminal history byte budget reached");
            }
            if map.values().filter(|s| s.info().alive).count() + gate.pending >= 64
                || map.len() + gate.pending >= DEFAULT_MAX_METADATA
            {
                anyhow::bail!("Busy: session capacity reached");
            }
            gate.pending += 1;
        }
        let _pending = PendingCreate(&self.creation_gate);
        let session = Arc::new(spawn()?);
        let gate = self.creation_gate.lock().expect("creation gate");
        if gate.draining {
            let _ = session.kill();
            anyhow::bail!("Unavailable: creation cancelled during shutdown");
        }
        let mut map = self.map.lock().expect("registry lock");
        map.insert(session.id().into(), session.clone());
        enforce_limits(&mut map, self.limits);
        Ok(session)
    }
    pub fn pending_creates(&self) -> usize {
        self.creation_gate.lock().expect("creation gate").pending
    }

    pub fn shutdown_deadline(&self) -> std::time::Instant {
        *self
            .shutdown_deadline
            .get_or_init(|| std::time::Instant::now() + std::time::Duration::from_secs(10))
    }
    pub fn begin_draining(&self) {
        self.shutdown_deadline();
        self.creation_gate.lock().expect("creation gate").draining = true;
    }
    pub fn is_draining(&self) -> bool {
        self.creation_gate.lock().expect("creation gate").draining
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.map.lock().expect("registry lock").get(id).cloned()
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let mut map = self.map.lock().expect("registry lock");
        enforce_limits(&mut map, self.limits);
        let mut v: Vec<SessionInfo> = map.values().map(|s| s.info()).collect();
        v.sort_by_key(|i| i.created_ms);
        v
    }

    pub fn kill(&self, id: &str) -> Result<()> {
        self.get(id)
            .ok_or_else(|| anyhow!("会话不存在: {id}"))?
            .kill()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 按总字节 / 退出会话数 / TTL / 元信息条数逐出**已退出且无活动订阅**的
/// 正文;活动会话永不因历史预算被杀。达到元信息上限时再移除最老的整条
/// 记录;有活动订阅的记录保留。
fn reserved_history_bytes(map: &HashMap<String, Arc<Session>>) -> usize {
    map.values()
        .map(|s| {
            if s.info().alive {
                crate::ring::SCROLLBACK_CAP
            } else {
                s.history_len()
            }
        })
        .sum()
}

fn enforce_limits(map: &mut HashMap<String, Arc<Session>>, limits: HistoryLimits) {
    let now = now_ms();
    let mut exited: Vec<(u64, String, Arc<Session>)> = map
        .values()
        .filter(|session| !session.info().alive)
        .map(|session| {
            (
                session.exited_ms().unwrap_or(session.info().created_ms),
                session.id().to_string(),
                session.clone(),
            )
        })
        .collect();
    exited.sort_by_key(|(exited_ms, _, _)| *exited_ms);

    // Reserve the full active ring capacity so later PTY writes cannot exceed
    // the shared budget between registry operations.
    let mut total_bytes: usize = reserved_history_bytes(map);
    let mut retained = exited
        .iter()
        .filter(|(_, _, s)| !s.history_evicted())
        .count();
    for (exited_ms, _, session) in &exited {
        if session.subscriber_count() > 0 || session.retention_pinned() {
            continue;
        }
        let over_bytes = total_bytes > limits.total_bytes;
        let over_count = retained > limits.max_exited;
        let expired = now.saturating_sub(*exited_ms) > limits.ttl_ms;
        if (over_bytes || over_count || expired) && !session.history_evicted() {
            total_bytes = total_bytes.saturating_sub(session.history_len());
            session.evict_history();
            retained = retained.saturating_sub(1);
        }
    }

    if map.len() > limits.max_metadata {
        let mut removable: Vec<(u64, String)> = exited
            .iter()
            .filter(|(_, _, session)| {
                session.subscriber_count() == 0 && !session.retention_pinned()
            })
            .map(|(exited_ms, id, _)| (*exited_ms, id.clone()))
            .collect();
        removable.sort_by_key(|(exited_ms, _)| *exited_ms);
        for (_, id) in removable {
            if map.len() <= limits.max_metadata {
                break;
            }
            if let Some(session) = map.remove(&id) {
                session.evict_history();
            }
        }
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

    async fn wait_exited(session: &Session) {
        for _ in 0..200 {
            if session.exit_status().is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("session did not exit");
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
    async fn history_budget_evicts_oldest_exited_without_killing_active() {
        let reg = SessionRegistry::with_limits(1, 128, u64::MAX, 1024);
        let old = reg.create(spec("printf 0123456789; exit 0")).unwrap();
        wait_exited(&old).await;
        for _ in 0..100 {
            if old.history_len() > 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(old.history_len() > 1);
        let active = reg.create(spec("sleep 5")).unwrap();
        let _ = reg.list();
        assert!(old.history_evicted(), "超预算的已退出会话正文应被逐出");
        assert_eq!(old.history_len(), 0);
        assert!(active.info().alive, "活动会话不能因历史预算被杀");
        reg.kill(&active.id()).unwrap();
    }

    #[tokio::test]
    async fn metadata_limit_removes_oldest_exited_record() {
        let reg = SessionRegistry::with_limits(
            DEFAULT_TOTAL_HISTORY_BYTES,
            DEFAULT_MAX_EXITED,
            u64::MAX,
            1,
        );
        let first = reg.create(spec("exit 0")).unwrap();
        let first_id = first.id().to_string();
        wait_exited(&first).await;
        let second = reg.create(spec("sleep 5")).unwrap();
        let _ = reg.list();
        assert!(
            reg.get(&first_id).is_none(),
            "超过元信息上限应移除最老已退出记录"
        );
        assert!(reg.get(second.id()).is_some(), "活动会话必须保留");
        reg.kill(second.id()).unwrap();
    }

    #[tokio::test]
    async fn get_unknown_is_none_and_kill_unknown_errs() {
        let reg = SessionRegistry::new();
        assert!(reg.get("nope").is_none());
        assert!(reg.kill("nope").is_err());
    }
    #[test]
    fn draining_during_spawn_rejects_late_registration_and_reaps_child() {
        let registry = Arc::new(SessionRegistry::new());
        let copy = registry.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (child_tx, child_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            copy.create_with(|| {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                let session = Session::spawn(spec("sleep 30"))?;
                child_tx.send(session.runtime_child_for_test()).unwrap();
                Ok(session)
            })
        });
        started_rx.recv().unwrap();
        assert_eq!(registry.pending_creates(), 1);
        registry.begin_draining();
        release_tx.send(()).unwrap();
        assert!(thread.join().unwrap().is_err());
        assert_eq!(registry.pending_creates(), 0);
        assert!(registry.list().is_empty());
        let child = child_rx.recv().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while child.lock().unwrap().is_some() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(child.lock().unwrap().is_none());
    }
    #[tokio::test]
    async fn count_limit_keeps_newest_exited_history() {
        let registry = SessionRegistry::with_limits(usize::MAX, 1, u64::MAX, 1024);
        let first = registry.create(spec("printf first")).unwrap();
        first.wait_for_exit().await;
        let mut output = first.subscribe();
        while first.attach_snapshot(0).exit_status.is_none() {
            let _ = output.recv().await;
        }
        drop(output);
        let second = registry.create(spec("printf second")).unwrap();
        second.wait_for_exit().await;
        let mut output = second.subscribe();
        while second.attach_snapshot(0).exit_status.is_none() {
            let _ = output.recv().await;
        }
        drop(output);
        registry.list();
        assert!(first.history_evicted());
        assert!(!second.history_evicted());
        assert_eq!(second.snapshot().0, b"second");
    }
    #[tokio::test]
    async fn active_ring_capacity_is_reserved_before_spawn() {
        let registry =
            SessionRegistry::with_limits(2 * crate::ring::SCROLLBACK_CAP, 128, u64::MAX, 1024);
        let first = registry.create(spec("sleep 30")).unwrap();
        let second = registry.create(spec("sleep 30")).unwrap();
        assert!(registry.create(spec("sleep 30")).is_err());
        assert!(first.info().alive && second.info().alive);
        first.kill().unwrap();
        second.kill().unwrap();
    }
}
