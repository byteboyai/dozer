//! Coalescing transcript owner. At most 256 distinct queued/running sources and two readers.
use crate::transcripts::TranscriptStore;
use dozer_core::protocol::{AgentKind, BackfillPhase, BackfillState};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, Weak},
};
dozer_core::scope!(LOG, module, "summary");
#[derive(Default)]
struct Pending {
    sources: HashMap<PathBuf, (AgentKind, bool, bool)>,
    ready: VecDeque<PathBuf>,
    stopped: bool,
    scans: usize,
    status: BackfillState,
}
#[derive(Default)]
pub struct Ingestion {
    state: Arc<(Mutex<Pending>, Condvar)>,
}
impl Ingestion {
    pub fn start(store: Weak<TranscriptStore>) -> Self {
        let this = Self::default();
        for index in 0..2 {
            let state = this.state.clone();
            let store = store.clone();
            std::thread::Builder::new()
                .name(format!("transcript-{index}"))
                .spawn(move || {
                    loop {
                        let (lock, wake) = &*state;
                        let (path, agent) = {
                            let mut s = lock.lock().unwrap();
                            loop {
                                if s.stopped {
                                    return;
                                }
                                if let Some(path) = s.ready.pop_front() {
                                    if let Some((agent, running, dirty)) = s.sources.get_mut(&path)
                                    {
                                        *running = true;
                                        *dirty = false;
                                        break (path, *agent);
                                    }
                                    continue;
                                }
                                s = wake.wait(s).unwrap();
                            }
                        };
                        let Some(store) = store.upgrade() else {
                            return;
                        };
                        let result = store.ingest_batch(agent, &path);
                        let more = matches!(&result, Ok(true));
                        let mut s = lock.lock().unwrap();
                        if let Err(error) = result {
                            s.status.failed += 1;
                            s.status.last_error = Some("Transcript source ingestion failed".into());
                            dozer_core::log_warn!(LOG,error=%error,"transcript ingestion failed");
                        } else if !more {
                            s.status.imported += 1;
                        }
                        let stopped = s.stopped;
                        if let Some((_, running, dirty)) = s.sources.get_mut(&path) {
                            if (*dirty || more) && !stopped {
                                *running = false;
                                s.ready.push_back(path.clone());
                            } else {
                                s.sources.remove(&path);
                            }
                        }
                        s.status.revision = s.status.revision.wrapping_add(1);
                        s.status.pending = s.sources.len() as u32;
                        if !s.stopped
                            && s.scans == 0
                            && s.sources.is_empty()
                            && s.status.phase != BackfillPhase::NotStarted
                        {
                            s.status.phase = if s.status.failed == 0 {
                                BackfillPhase::Complete
                            } else {
                                BackfillPhase::Failed
                            };
                        }
                        wake.notify_all();
                    }
                })
                .expect("transcript worker");
        }
        {
            let state = this.state.clone();
            std::thread::Builder::new()
                .name("transcript-rescan".into())
                .spawn(move || {
                    loop {
                        {
                            let (lock, wake) = &*state;
                            let mut s = lock.lock().unwrap();
                            while !s.stopped && !s.status.rescan_needed {
                                s = wake.wait(s).unwrap();
                            }
                            if s.stopped {
                                return;
                            }
                            s.status.rescan_needed = false;
                            s.scans += 1;
                            s.status.phase = BackfillPhase::Discovering;
                        }
                        let discovery = crate::transcripts::scan::visit_all_cancellable(
                            &dozer_core::agent_paths::home_dir(),
                            || !state.0.lock().unwrap().stopped,
                            |agent, path| {
                                loop {
                                    if state.0.lock().unwrap().stopped {
                                        return false;
                                    }
                                    if Self::enqueue(&state, agent, path.clone(), false).is_ok() {
                                        return true;
                                    }
                                    let guard = state.0.lock().unwrap();
                                    let _guard = state
                                        .1
                                        .wait_timeout(guard, std::time::Duration::from_millis(100))
                                        .unwrap();
                                }
                            },
                        );
                        if discovery.failed > 0 {
                            let mut s = state.0.lock().unwrap();
                            s.status.failed = s.status.failed.saturating_add(discovery.failed);
                            s.status.last_error =
                                Some("Transcript directory discovery failed".into());
                        }
                        Self::finish_scan(&state);
                    }
                })
                .expect("transcript rescan owner");
        }
        this
    }
    pub fn begin_scan(&self) {
        let mut s = self.state.0.lock().unwrap();
        s.scans += 1;
        s.status.phase = BackfillPhase::Discovering;
    }
    fn finish_scan(state: &Arc<(Mutex<Pending>, Condvar)>) {
        let mut s = state.0.lock().unwrap();
        s.scans = s.scans.saturating_sub(1);
        if !s.stopped && s.scans == 0 {
            s.status.phase = if !s.sources.is_empty() {
                BackfillPhase::Ingesting
            } else if s.status.failed > 0 {
                BackfillPhase::Failed
            } else {
                BackfillPhase::Complete
            };
        }
    }
    pub fn end_scan(&self) {
        Self::finish_scan(&self.state);
    }
    pub fn submit_scan(&self, agent: AgentKind, path: PathBuf) -> Result<(), String> {
        Self::enqueue(&self.state, agent, path, false)
    }
    pub fn submit(&self, agent: AgentKind, path: PathBuf) -> Result<(), String> {
        Self::enqueue(&self.state, agent, path, true)
    }
    fn enqueue(
        state: &Arc<(Mutex<Pending>, Condvar)>,
        agent: AgentKind,
        path: PathBuf,
        rescan: bool,
    ) -> Result<(), String> {
        let key = path.components().collect::<PathBuf>();
        let (lock, wake) = &**state;
        let mut s = lock.lock().unwrap();
        if s.stopped {
            return Err("Unavailable: ingestion stopped".into());
        }
        if let Some((existing, running, dirty)) = s.sources.get_mut(&key) {
            if *existing != agent {
                return Err("transcript agent identity collision".into());
            }
            if *running {
                *dirty = true;
            }
            return Ok(());
        }
        if s.sources.len() >= 256 {
            if rescan {
                s.status.rescan_needed = true;
                s.status.failed += 1;
                s.status.last_error = Some(
                    "Busy: source overflow; rescan scheduled, unknown sources require retry".into(),
                );
                wake.notify_all();
            }
            return Err("Busy: ingestion source queue full; retry hook".into());
        }
        if matches!(
            s.status.phase,
            BackfillPhase::NotStarted | BackfillPhase::Complete
        ) {
            s.status.phase = BackfillPhase::Ingesting;
        }
        s.ready.push_back(key.clone());
        s.sources.insert(key, (agent, false, false));
        s.status.revision = s.status.revision.wrapping_add(1);
        s.status.pending = s.sources.len() as u32;
        wake.notify_all();
        Ok(())
    }
    pub fn status(&self) -> BackfillState {
        self.state.0.lock().unwrap().status.clone()
    }
    pub fn update(&self, f: impl FnOnce(&mut BackfillState)) {
        let mut s = self.state.0.lock().unwrap();
        f(&mut s.status);
        if s.stopped {
            s.status.phase = BackfillPhase::Cancelled;
        }
        s.status.revision = s.status.revision.wrapping_add(1);
    }
    pub fn stopped(&self) -> bool {
        self.state.0.lock().unwrap().stopped
    }
    pub fn stop(&self) {
        let mut s = self.state.0.lock().unwrap();
        s.stopped = true;
        s.sources.retain(|_, (_, running, _)| *running);
        s.ready.clear();
        s.status.revision = s.status.revision.wrapping_add(1);
        s.status.pending = s.sources.len() as u32;
        s.status.phase = BackfillPhase::Cancelled;
        self.state.1.notify_all();
    }
    pub fn wait_capacity(&self) {
        let (lock, wake) = &*self.state;
        let s = lock.lock().unwrap();
        let _guard = wake
            .wait_timeout_while(s, std::time::Duration::from_millis(100), |s| {
                s.sources.len() >= 256 && !s.stopped
            })
            .unwrap();
    }
}
impl Drop for Ingestion {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn late_scan_update_cannot_overwrite_cancellation() {
        let queue = Ingestion::default();
        queue.stop();
        queue.update(|s| s.phase = BackfillPhase::Ingesting);
        assert_eq!(queue.status().phase, BackfillPhase::Cancelled);
    }
    #[test]
    fn repeated_source_is_coalesced_and_distinct_sources_are_bounded() {
        let queue = Ingestion::default();
        for _ in 0..1000 {
            queue
                .submit(AgentKind::Claude, PathBuf::from("same.jsonl"))
                .unwrap();
        }
        assert_eq!(queue.status().pending, 1);
        for i in 1..256 {
            queue
                .submit(AgentKind::Claude, PathBuf::from(format!("{i}.jsonl")))
                .unwrap();
        }
        assert_eq!(queue.status().pending, 256);
        assert!(
            queue
                .submit(AgentKind::Claude, "overflow.jsonl".into())
                .is_err()
        );
        assert!(queue.status().rescan_needed);
        queue.stop();
        assert!(
            queue
                .submit(AgentKind::Claude, "same.jsonl".into())
                .is_err()
        );
    }
}
