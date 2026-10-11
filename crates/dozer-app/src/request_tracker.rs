//! UI request identities. Check on delivery, not in the background task: a result
//! can already be queued when a newer request starts. Each panel instance owns
//! its tracker, so reopening the same project cannot revive old results.
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(1);
fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RequestToken {
    pub project_id: Option<i64>,
    instance: u64,
    lane: Option<&'static str>,
    version: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct RequestTracker(Arc<Inner>);
#[derive(Debug)]
struct Inner {
    instance: u64,
    latest: Mutex<HashMap<&'static str, u64>>,
}
impl Default for RequestTracker {
    fn default() -> Self {
        Self(Arc::new(Inner {
            instance: next(),
            latest: Mutex::default(),
        }))
    }
}
impl RequestTracker {
    pub fn begin(&self, project_id: Option<i64>, lane: &'static str) -> RequestToken {
        let version = next();
        self.0.latest.lock().unwrap().insert(lane, version);
        RequestToken {
            project_id,
            instance: self.0.instance,
            lane: Some(lane),
            version,
        }
    }
    /// Writes must all complete: scope them to the panel, without superseding
    /// other writes. Their follow-up reads still use latest-request validation.
    pub fn scope(&self, project_id: Option<i64>) -> RequestToken {
        RequestToken {
            project_id,
            instance: self.0.instance,
            lane: None,
            version: 0,
        }
    }
    pub fn invalidate(&self, lane: &'static str) {
        self.0.latest.lock().unwrap().insert(lane, next());
    }
    pub fn accepts(&self, token: RequestToken) -> bool {
        token.instance == self.0.instance
            && token
                .lane
                .is_none_or(|lane| self.0.latest.lock().unwrap().get(lane) == Some(&token.version))
    }
}
impl RequestToken {
    pub fn wrap<M: 'static>(
        self,
        emit: impl Fn(M) + Send + 'static,
        envelope: fn(Self, Box<M>) -> M,
    ) -> impl Fn(M) + Clone + Send + Sync + 'static {
        let emit = Arc::new(Mutex::new(emit));
        move |msg| (emit.lock().unwrap())(envelope(self, Box::new(msg)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_results_are_invalidated_and_lanes_are_independent() {
        let requests = RequestTracker::default();
        let old = requests.begin(Some(1), "list");
        let categories = requests.begin(Some(1), "categories");
        let new = requests.begin(Some(1), "list");
        assert!(!requests.accepts(old));
        assert!(requests.accepts(new));
        assert!(requests.accepts(categories));
        requests.invalidate("list");
        assert!(!requests.accepts(new));
    }
    #[test]
    fn another_project_or_reopened_panel_rejects_old_results() {
        let original = RequestTracker::default();
        let token = original.begin(Some(7), "list");
        assert!(!RequestTracker::default().accepts(token));
        assert!(original.accepts(token));
        let write = original.scope(Some(7));
        original.begin(Some(7), "list");
        assert!(original.accepts(write));
        assert!(!RequestTracker::default().accepts(write));
    }
}
