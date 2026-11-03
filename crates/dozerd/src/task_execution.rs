//! 任务执行 attempt 的持久化与准入。
//!
//! `todos` 只表达人工完成/搁置;执行结果单独落在
//! `task_execution_attempts`。同一任务同时最多一个 `queued/running` attempt
//! (SQLite partial unique index),进程崩溃或 panic 时由 RAII token 标为
//! `interrupted`,重启时再次把遗留 Running 收敛为 Interrupted。
//!
//! 成功/失败回合与 attempt 状态在**同一个 SQLite 事务**里提交,避免“进程
//! 成功但状态没落库”被误当成普通 AI 回合。

use crate::transcripts::{TranscriptStore, append_task_turn_tx};
use anyhow::{Context, Result};
use dozer_core::protocol::{TaskExecutionInfo, TaskExecutionStatus};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

const ATTEMPT_COLUMNS: &str = "attempt_id, todo_id, input_generation, idempotency_key, \
    status, error_kind, error_message, started_ms, finished_ms, conversation_id, created_ms";

fn status_from_str(value: &str) -> TaskExecutionStatus {
    TaskExecutionStatus::from_str(value).unwrap_or(TaskExecutionStatus::Interrupted)
}

fn row_to_attempt(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskExecutionInfo> {
    Ok(TaskExecutionInfo {
        attempt_id: row.get(0)?,
        todo_id: row.get(1)?,
        input_generation: row.get(2)?,
        idempotency_key: row.get(3)?,
        status: status_from_str(&row.get::<_, String>(4)?),
        error_kind: row.get(5)?,
        error_message: row.get(6)?,
        started_ms: row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
        finished_ms: row.get::<_, Option<i64>>(8)?.map(|v| v as u64),
        conversation_id: row.get(9)?,
        created_ms: row.get::<_, i64>(10)? as u64,
    })
}

#[derive(Debug)]
pub enum AdmissionError {
    /// 同一任务已有 queued/running attempt。
    InFlight,
    /// 相同 idempotency_key 已用于不同输入。
    IdempotencyConflict,
    Other(anyhow::Error),
}

impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdmissionError::InFlight => write!(f, "该任务正在处理中"),
            AdmissionError::IdempotencyConflict => {
                write!(f, "idempotency_key 已用于不同输入")
            }
            AdmissionError::Other(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AdmissionError {}

impl From<anyhow::Error> for AdmissionError {
    fn from(error: anyhow::Error) -> Self {
        AdmissionError::Other(error)
    }
}

impl From<rusqlite::Error> for AdmissionError {
    fn from(error: rusqlite::Error) -> Self {
        AdmissionError::Other(error.into())
    }
}

/// attempt 执行期间需要写回 AI 回合的上下文。
#[derive(Debug, Clone)]
pub struct AttemptContext {
    pub conversation_id: String,
    pub agent: dozer_core::protocol::AgentKind,
    pub project_dir: String,
    pub task_title: String,
}

pub enum Admission {
    Started(RunningAttempt),
    /// 命中 idempotency_key,直接返回原 attempt,不重复执行。
    Existing(TaskExecutionInfo),
}

impl std::fmt::Debug for Admission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Admission::Started(attempt) => {
                write!(f, "Admission::Started({})", attempt.attempt_id)
            }
            Admission::Existing(info) => write!(f, "Admission::Existing({})", info.attempt_id),
        }
    }
}

pub struct TaskExecutionStore {
    conn: Mutex<Connection>,
    stopping: tokio::sync::watch::Sender<bool>,
    workers: Arc<tokio::sync::Semaphore>,
    waiting: std::sync::atomic::AtomicUsize,
}

impl TaskExecutionStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建 attempt 库目录")?;
        }
        let conn = Connection::open(path).context("打开 attempt SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS task_execution_attempts (
                attempt_id TEXT PRIMARY KEY,
                todo_id INTEGER NOT NULL,
                input_generation INTEGER NOT NULL,
                input_fingerprint TEXT NOT NULL,
                idempotency_key TEXT,
                status TEXT NOT NULL,
                error_kind TEXT,
                error_message TEXT,
                started_ms INTEGER,
                finished_ms INTEGER,
                conversation_id TEXT,
                created_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_task_attempts_todo
                ON task_execution_attempts(todo_id, created_ms);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_task_attempts_one_running
                ON task_execution_attempts(todo_id)
                WHERE status IN ('queued', 'running');
             CREATE UNIQUE INDEX IF NOT EXISTS idx_task_attempts_idempotency
                ON task_execution_attempts(todo_id, idempotency_key)
                WHERE idempotency_key IS NOT NULL;",
        )
        .context("建 attempt 表")?;
        Ok(Self {
            conn: Mutex::new(conn),
            stopping: tokio::sync::watch::channel(false).0,
            workers: Arc::new(tokio::sync::Semaphore::new(4)),
            waiting: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Dedicated task workers never occupy the ordinary RPC IO pool while waiting for an agent.
    pub async fn run_blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, String> {
        use std::sync::atomic::Ordering;
        struct Waiting<'a>(&'a std::sync::atomic::AtomicUsize);
        impl Drop for Waiting<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        if self.waiting.fetch_add(1, Ordering::SeqCst) >= 32 {
            self.waiting.fetch_sub(1, Ordering::SeqCst);
            return Err("Busy: task worker queue full".into());
        }
        let waiting = Waiting(&self.waiting);
        let mut cancel = self.stopping.subscribe();
        if *cancel.borrow() {
            return Err("Unavailable: task runtime draining".into());
        }
        let permit = tokio::select! {_=cancel.changed()=>return Err("Unavailable: task runtime draining".into()),p=tokio::time::timeout(std::time::Duration::from_secs(5),self.workers.clone().acquire_owned())=>p.map_err(|_|"Busy: task worker wait timed out")?.map_err(|_|"Unavailable: task worker stopped")?};
        drop(waiting);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        })
        .await
        .map_err(|e| format!("task worker stopped: {e}"))
    }
    pub fn stop(&self) {
        self.stopping.send_replace(true);
    }
    pub fn cancellation(&self) -> tokio::sync::watch::Receiver<bool> {
        self.stopping.subscribe()
    }
    pub fn is_stopping(&self) -> bool {
        *self.stopping.borrow()
    }
    pub fn recover_running(&self) -> Result<usize> {
        let conn = self.conn.lock().expect("db lock");
        let now = now_ms() as i64;
        Ok(conn.execute(
            "UPDATE task_execution_attempts
             SET status = 'interrupted', error_kind = 'restart',
                 error_message = 'daemon restarted while attempt was running',
                 finished_ms = ?1
             WHERE status IN ('queued', 'running')",
            [now],
        )?)
    }

    pub fn latest(&self, todo_id: i64) -> Result<Option<TaskExecutionInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "SELECT {ATTEMPT_COLUMNS} FROM task_execution_attempts
             WHERE todo_id = ?1 ORDER BY created_ms DESC, rowid DESC LIMIT 1"
        );
        Ok(conn.query_row(&sql, [todo_id], row_to_attempt).optional()?)
    }

    pub fn get(&self, attempt_id: &str) -> Result<Option<TaskExecutionInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let sql =
            format!("SELECT {ATTEMPT_COLUMNS} FROM task_execution_attempts WHERE attempt_id = ?1");
        Ok(conn
            .query_row(&sql, [attempt_id], row_to_attempt)
            .optional()?)
    }

    /// 准入一次执行。`self` 用 `Arc` 是为了返回持有 store 的 RAII token。
    pub fn admit(
        self: &Arc<Self>,
        todo_id: i64,
        input_fingerprint: &str,
        idempotency_key: Option<&str>,
        context: AttemptContext,
    ) -> std::result::Result<Admission, AdmissionError> {
        let conn = self.conn.lock().expect("db lock");
        if self.is_stopping() {
            return Err(AdmissionError::Other(anyhow::anyhow!(
                "Unavailable: task runtime draining"
            )));
        }
        if let Some(key) = idempotency_key {
            let sql = format!(
                "SELECT {ATTEMPT_COLUMNS} FROM task_execution_attempts
                 WHERE todo_id = ?1 AND idempotency_key = ?2"
            );
            if let Some(existing) = conn
                .query_row(&sql, params![todo_id, key], row_to_attempt)
                .optional()?
            {
                let fingerprint: String = conn.query_row(
                    "SELECT input_fingerprint FROM task_execution_attempts WHERE attempt_id = ?1",
                    [&existing.attempt_id],
                    |row| row.get(0),
                )?;
                if fingerprint != input_fingerprint {
                    return Err(AdmissionError::IdempotencyConflict);
                }
                return Ok(Admission::Existing(existing));
            }
        }

        let attempt_id = uuid::Uuid::new_v4().to_string();
        let now = now_ms() as i64;
        let insert = conn.execute(
            "INSERT INTO task_execution_attempts
             (attempt_id, todo_id, input_generation, input_fingerprint, idempotency_key,
              status, started_ms, conversation_id, created_ms)
             VALUES (?1, ?2, 0, ?3, ?4, 'running', ?5, ?6, ?5)",
            params![
                attempt_id,
                todo_id,
                input_fingerprint,
                idempotency_key,
                now,
                context.conversation_id
            ],
        );
        match insert {
            Ok(_) => {}
            Err(error) if is_unique_violation(&error) => {
                return Err(AdmissionError::InFlight);
            }
            Err(error) => return Err(AdmissionError::Other(error.into())),
        }
        drop(conn);

        let info = self
            .get(&attempt_id)?
            .ok_or_else(|| AdmissionError::Other(anyhow::anyhow!("attempt 写入后读不到")))?;
        Ok(Admission::Started(RunningAttempt {
            store: Arc::downgrade(self),
            attempt_id,
            context,
            finished: false,
            _info: info,
        }))
    }

    fn set_input_generation(&self, attempt_id: &str, generation: i64) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "UPDATE task_execution_attempts SET input_generation = ?1 WHERE attempt_id = ?2",
            params![generation, attempt_id],
        )?;
        Ok(())
    }

    fn interrupt(&self, attempt_id: &str, kind: &str) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        let now = now_ms() as i64;
        conn.execute(
            "UPDATE task_execution_attempts
             SET status = 'interrupted', error_kind = ?1, finished_ms = ?2
             WHERE attempt_id = ?3 AND status IN ('queued', 'running')",
            params![kind, now, attempt_id],
        )?;
        Ok(())
    }

    fn finish_success(
        &self,
        attempt_id: &str,
        context: &AttemptContext,
        ai_content: &str,
    ) -> Result<TaskExecutionInfo> {
        let mut conn = self.conn.lock().expect("db lock");
        let tx = conn.transaction()?;
        append_task_turn_tx(
            &tx,
            &context.conversation_id,
            context.agent,
            &context.project_dir,
            &context.task_title,
            "ai",
            ai_content,
            false,
        )?;
        let now = now_ms() as i64;
        tx.execute(
            "UPDATE task_execution_attempts
             SET status = 'succeeded', finished_ms = ?1
             WHERE attempt_id = ?2",
            params![now, attempt_id],
        )?;
        let info = tx.query_row(
            &format!("SELECT {ATTEMPT_COLUMNS} FROM task_execution_attempts WHERE attempt_id = ?1"),
            [attempt_id],
            row_to_attempt,
        )?;
        tx.commit()?;
        Ok(info)
    }

    fn finish_failure(
        &self,
        attempt_id: &str,
        context: &AttemptContext,
        error_kind: &str,
        error_message: &str,
    ) -> Result<TaskExecutionInfo> {
        let content = format!(
            "任务执行失败: {}",
            crate::task_processor::truncate_for_turn(error_message)
        );
        let mut conn = self.conn.lock().expect("db lock");
        let tx = conn.transaction()?;
        append_task_turn_tx(
            &tx,
            &context.conversation_id,
            context.agent,
            &context.project_dir,
            &context.task_title,
            "ai",
            &content,
            true,
        )?;
        let now = now_ms() as i64;
        tx.execute(
            "UPDATE task_execution_attempts
             SET status = 'failed', error_kind = ?1, error_message = ?2, finished_ms = ?3
             WHERE attempt_id = ?4",
            params![error_kind, error_message, now, attempt_id],
        )?;
        let info = tx.query_row(
            &format!("SELECT {ATTEMPT_COLUMNS} FROM task_execution_attempts WHERE attempt_id = ?1"),
            [attempt_id],
            row_to_attempt,
        )?;
        tx.commit()?;
        Ok(info)
    }
}

fn is_unique_violation(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::ConstraintViolation,
                ..
            },
            _
        )
    )
}

/// 一次正在执行的 attempt。Drop 时若仍未结束,说明取消/panic/提前返回,
/// 持久化状态置为 Interrupted,不会永久留在内存 in_flight。
pub struct RunningAttempt {
    store: Weak<TaskExecutionStore>,
    attempt_id: String,
    context: AttemptContext,
    finished: bool,
    _info: TaskExecutionInfo,
}

impl RunningAttempt {
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }

    pub fn context(&self) -> &AttemptContext {
        &self.context
    }

    pub fn cancellation(&self) -> tokio::sync::watch::Receiver<bool> {
        self.store
            .upgrade()
            .expect("attempt owner")
            .stopping
            .subscribe()
    }
    pub fn info(&self) -> &TaskExecutionInfo {
        &self._info
    }

    /// 人类输入已持久化后,把本次 attempt 的输入代次更新为最新 human 回合索引。
    pub fn mark_input_persisted(&self, transcripts: &TranscriptStore) -> Result<()> {
        let generation = transcripts.latest_human_turn_index(&self.context.conversation_id)?;
        if let Some(store) = self.store.upgrade() {
            store.set_input_generation(&self.attempt_id, generation)?;
        }
        Ok(())
    }

    pub fn finish_success(mut self, ai_content: &str) -> Result<TaskExecutionInfo> {
        let store = self
            .store
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("attempt store 已释放"))?;
        let info = store.finish_success(&self.attempt_id, &self.context, ai_content)?;
        self.finished = true;
        Ok(info)
    }

    pub fn finish_failure(
        mut self,
        error_kind: &str,
        error_message: &str,
    ) -> Result<TaskExecutionInfo> {
        let store = self
            .store
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("attempt store 已释放"))?;
        let info =
            store.finish_failure(&self.attempt_id, &self.context, error_kind, error_message)?;
        self.finished = true;
        Ok(info)
    }
}

impl Drop for RunningAttempt {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Some(store) = self.store.upgrade() {
            let _ = store.interrupt(&self.attempt_id, "aborted");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_core::protocol::AgentKind;

    fn fixture() -> (
        tempfile::TempDir,
        Arc<TaskExecutionStore>,
        Arc<TranscriptStore>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = Arc::new(TaskExecutionStore::open(&db).unwrap());
        let transcripts = Arc::new(TranscriptStore::open(&db).unwrap());
        (dir, store, transcripts)
    }

    fn context() -> AttemptContext {
        AttemptContext {
            conversation_id: "task-1".into(),
            agent: AgentKind::Claude,
            project_dir: "/tmp/proj".into(),
            task_title: "任务".into(),
        }
    }

    #[test]
    fn admit_persists_running_and_rejects_second_attempt() {
        let (_dir, store, _transcripts) = fixture();
        let admission = store.admit(1, "fingerprint-a", None, context()).unwrap();
        let Admission::Started(attempt) = admission else {
            panic!("应首次准入");
        };
        assert_eq!(
            store.latest(1).unwrap().unwrap().status,
            TaskExecutionStatus::Running
        );
        let err = store
            .admit(1, "fingerprint-b", None, context())
            .unwrap_err();
        assert!(matches!(err, AdmissionError::InFlight));
        drop(attempt);
    }

    #[test]
    fn dropping_running_attempt_marks_it_interrupted() {
        let (_dir, store, _transcripts) = fixture();
        let Admission::Started(attempt) = store.admit(1, "fingerprint", None, context()).unwrap()
        else {
            panic!()
        };
        drop(attempt);
        let latest = store.latest(1).unwrap().unwrap();
        assert_eq!(latest.status, TaskExecutionStatus::Interrupted);
        assert_eq!(latest.error_kind.as_deref(), Some("aborted"));
    }

    #[test]
    fn same_idempotency_key_returns_original_attempt() {
        let (_dir, store, _transcripts) = fixture();
        let Admission::Started(attempt) = store
            .admit(1, "fingerprint", Some("key-1"), context())
            .unwrap()
        else {
            panic!()
        };
        let first = attempt.attempt_id().to_string();
        let admission = store
            .admit(1, "fingerprint", Some("key-1"), context())
            .unwrap();
        let Admission::Existing(existing) = admission else {
            panic!("应返回原 attempt");
        };
        assert_eq!(existing.attempt_id, first);
        drop(attempt);
    }

    #[test]
    fn same_key_with_different_input_is_rejected() {
        let (_dir, store, _transcripts) = fixture();
        let Admission::Started(attempt) = store
            .admit(1, "fingerprint-a", Some("key-1"), context())
            .unwrap()
        else {
            panic!()
        };
        let err = store
            .admit(1, "fingerprint-b", Some("key-1"), context())
            .unwrap_err();
        assert!(matches!(err, AdmissionError::IdempotencyConflict));
        drop(attempt);
    }

    #[test]
    fn success_commit_writes_ai_turn_and_succeeded_atomically() {
        let (_dir, store, transcripts) = fixture();
        let Admission::Started(attempt) = store.admit(1, "fingerprint", None, context()).unwrap()
        else {
            panic!()
        };
        transcripts
            .record_task_human_turn("task-1", AgentKind::Claude, "/tmp/proj", "任务", "开始")
            .unwrap();
        attempt.mark_input_persisted(&transcripts).unwrap();
        let info = attempt.finish_success("完成").unwrap();
        assert_eq!(info.status, TaskExecutionStatus::Succeeded);
        let turns = transcripts
            .get_conversation_turns("task-1", -1, 10)
            .unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].content, "完成");
        assert!(!turns[1].is_error);
    }

    #[test]
    fn failure_commit_records_error_turn_and_failed_status() {
        let (_dir, store, transcripts) = fixture();
        let Admission::Started(attempt) = store.admit(1, "fingerprint", None, context()).unwrap()
        else {
            panic!()
        };
        transcripts
            .record_task_human_turn("task-1", AgentKind::Claude, "/tmp/proj", "任务", "开始")
            .unwrap();
        attempt.mark_input_persisted(&transcripts).unwrap();
        let info = attempt.finish_failure("nonzero_exit", "exit 42").unwrap();
        assert_eq!(info.status, TaskExecutionStatus::Failed);
        let turns = transcripts
            .get_conversation_turns("task-1", -1, 10)
            .unwrap();
        assert_eq!(turns.len(), 2);
        assert!(turns[1].is_error);
        assert!(turns[1].content.contains("exit 42"));
    }

    #[test]
    fn recover_running_marks_interrupted() {
        let (_dir, store, _transcripts) = fixture();
        let Admission::Started(attempt) = store.admit(1, "fingerprint", None, context()).unwrap()
        else {
            panic!()
        };
        // 模拟重启:token 未正常结束,但先手动 recover(不 drop,避免 Drop 再写)。
        std::mem::forget(attempt);
        assert_eq!(store.recover_running().unwrap(), 1);
        let latest = store.latest(1).unwrap().unwrap();
        assert_eq!(latest.status, TaskExecutionStatus::Interrupted);
        assert_eq!(latest.error_kind.as_deref(), Some("restart"));
    }
    #[test]
    fn stopping_cancels_current_attempt_and_rejects_new_admission() {
        let (_dir, store, _transcripts) = fixture();
        let Admission::Started(attempt) = store.admit(1, "a", None, context()).unwrap() else {
            panic!()
        };
        let cancellation = attempt.cancellation();
        store.stop();
        assert!(*cancellation.borrow());
        assert!(store.admit(2, "b", None, context()).is_err());
        drop(attempt);
        assert_eq!(
            store.latest(1).unwrap().unwrap().status,
            TaskExecutionStatus::Interrupted
        );
    }
    #[tokio::test]
    async fn cancelling_queued_task_worker_releases_waiting_admission() {
        let (_dir, store, _transcripts) = fixture();
        let held = store.workers.clone().acquire_many_owned(4).await.unwrap();
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let check = ran.clone();
        let owner = store.clone();
        let task = tokio::spawn(async move {
            owner
                .run_blocking(move || check.store(true, std::sync::atomic::Ordering::SeqCst))
                .await
        });
        while store.waiting.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        store.stop();
        assert!(task.await.unwrap().is_err());
        assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(store.waiting.load(std::sync::atomic::Ordering::SeqCst), 0);
        drop(held);
    }
    #[tokio::test]
    async fn panicking_task_worker_releases_its_execution_slot() {
        let (_dir, store, _transcripts) = fixture();
        assert!(
            store
                .run_blocking(|| panic!("fixture task panic"))
                .await
                .is_err()
        );
        assert_eq!(store.workers.available_permits(), 4);
        assert_eq!(store.run_blocking(|| 42).await.unwrap(), 42);
    }
    #[test]
    fn successful_process_with_failed_database_commit_is_not_reported_as_succeeded() {
        let (_dir, store, transcripts) = fixture();
        let Admission::Started(attempt) = store.admit(1, "fingerprint", None, context()).unwrap()
        else {
            panic!()
        };
        store.conn.lock().unwrap().execute_batch("CREATE TEMP TRIGGER fail_success BEFORE UPDATE ON task_execution_attempts WHEN NEW.status='succeeded' BEGIN SELECT RAISE(FAIL,'injected commit failure'); END;").unwrap();
        assert!(attempt.finish_success("external work completed").is_err());
        assert_eq!(
            store.latest(1).unwrap().unwrap().status,
            TaskExecutionStatus::Interrupted
        );
        assert!(
            transcripts
                .get_conversation_turns("task-1", -1, 10)
                .unwrap()
                .is_empty()
        );
    }
}
