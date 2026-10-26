#![allow(dead_code)]
use dozer_model::{
    AgentKind, ConversationSummary, SessionSummaryPayload, SummaryJobInfo, TurnRecord,
};
use dozer_services::{
    BoxFuture, ConversationRows, ConversationService, ServiceError, ServiceErrorKind,
    SummaryService,
};
use std::sync::{Arc, Mutex};
pub(crate) fn conversation_row(
    id: &str,
    title: &str,
) -> (ConversationSummary, Option<SessionSummaryPayload>) {
    (
        ConversationSummary {
            conversation_id: id.into(),
            agent: AgentKind::Claude,
            file_path: format!("/t/{id}.jsonl"),
            title: title.into(),
            first_ts: 0,
            last_ts: 0,
            turn_count: 1,
        },
        None,
    )
}

// ---- FakeConversation ----

struct ConversationState {
    rows: Result<ConversationRows, ServiceError>,
    turns: std::collections::HashMap<String, Vec<TurnRecord>>,
    turns_error: Option<ServiceError>,
    calls: Vec<String>,
}

/// 克隆共享同一份状态。`Default` = 成功且空。
#[derive(Clone)]
pub(crate) struct FakeConversation {
    state: Arc<Mutex<ConversationState>>,
}

impl Default for FakeConversation {
    fn default() -> Self {
        Self::with(Ok(vec![]))
    }
}

impl FakeConversation {
    pub(crate) fn with_turns(self, conversation_id: &str, turns: Vec<TurnRecord>) -> Self {
        self.state
            .lock()
            .unwrap()
            .turns
            .insert(conversation_id.into(), turns);
        self
    }
    pub(crate) fn failing_turn_reads(self, kind: ServiceErrorKind, message: &str) -> Self {
        self.state.lock().unwrap().turns_error = Some(ServiceError::new(kind, message));
        self
    }
    fn with(rows: Result<ConversationRows, ServiceError>) -> Self {
        Self {
            state: Arc::new(Mutex::new(ConversationState {
                turns_error: rows.as_ref().err().cloned(),
                rows,
                turns: std::collections::HashMap::new(),
                calls: vec![],
            })),
        }
    }
    pub(crate) fn with_rows(self, rows: ConversationRows) -> Self {
        self.state.lock().unwrap().rows = Ok(rows);
        self
    }
    pub(crate) fn failing(kind: ServiceErrorKind, message: &str) -> Self {
        Self::with(Err(ServiceError::new(kind, message)))
    }
    pub(crate) fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
}

impl ConversationService for FakeConversation {
    fn get_conversation_turns(
        &self,
        conversation_id: String,
        after_turn_index: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<TurnRecord>, ServiceError>> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(format!(
            "get_conversation_turns({conversation_id},{after_turn_index},{limit})"
        ));
        let result = match &state.turns_error {
            Some(error) => Err(error.clone()),
            None => {
                let mut turns: Vec<_> = state
                    .turns
                    .get(&conversation_id)
                    .into_iter()
                    .flatten()
                    .filter(|turn| turn.turn_index > after_turn_index)
                    .cloned()
                    .collect();
                turns.sort_by_key(|turn| turn.turn_index);
                turns.truncate(limit as usize);
                Ok(turns)
            }
        };
        Box::pin(async move { result })
    }

    fn list_conversations_with_summaries(
        &self,
        cwd: String,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<ConversationRows, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!(
            "list_conversations_with_summaries({cwd},{agent:?},{limit},{offset})"
        ));
        let r = s.rows.clone();
        Box::pin(async move { r })
    }
}

// ---- FakeSummary ----

struct SummaryState {
    submit: Result<i64, ServiceError>,
    jobs: std::collections::VecDeque<Result<Option<SummaryJobInfo>, ServiceError>>,
    calls: Vec<String>,
}

/// 可脚本化的队列与失败:克隆共享调用记录;读完预置序列后返回任务不存在。
#[derive(Clone)]
pub(crate) struct FakeSummary {
    state: Arc<Mutex<SummaryState>>,
}

impl Default for FakeSummary {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(SummaryState {
                submit: Ok(1),
                jobs: Default::default(),
                calls: vec![],
            })),
        }
    }
}

impl FakeSummary {
    pub(crate) fn with_submission(self, result: Result<i64, ServiceError>) -> Self {
        self.state.lock().unwrap().submit = result;
        self
    }
    pub(crate) fn with_jobs(self, jobs: Vec<Result<Option<SummaryJobInfo>, ServiceError>>) -> Self {
        self.state.lock().unwrap().jobs = jobs.into();
        self
    }
    pub(crate) fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
}

impl SummaryService for FakeSummary {
    fn submit_summary_job(
        &self,
        conversation_id: String,
        force: bool,
    ) -> BoxFuture<'_, Result<i64, ServiceError>> {
        let mut state = self.state.lock().unwrap();
        state
            .calls
            .push(format!("submit_summary_job({conversation_id},{force})"));
        let result = state.submit.clone();
        Box::pin(async move { result })
    }
    fn get_summary_job(
        &self,
        job_id: i64,
    ) -> BoxFuture<'_, Result<Option<SummaryJobInfo>, ServiceError>> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(format!("get_summary_job({job_id})"));
        let result = state.jobs.pop_front().unwrap_or(Ok(None));
        Box::pin(async move { result })
    }
}
