//! 规范总结结果与持久任务存储(spec 2026-09-26 第 4 节)。
//!
//! 与旧 `session_summaries`(单表、`session_id` 主键、无覆盖证据)并存,新增
//! 规范结果表 + 任务表 + 批次表 + 关联/中间产物表。**不删除旧表**——旧表
//! 继续作为历史展示数据与 MCP 兼容写入目标,新表是"完整总结"的真相源。

use anyhow::{Context, Result};
use dozer_core::protocol::{
    AgentKind, ConversationSummaryResult, SessionSummaryPayload, SummaryBatchInfo,
    SummaryErrorKind, SummaryJobInfo, SummaryJobStatus, SummaryTrigger,
};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

// ---- enum <-> str 转换 ----

fn agent_to_str(a: AgentKind) -> &'static str {
    a.label()
}

fn agent_from_str(s: &str) -> AgentKind {
    match s {
        "claude" => AgentKind::Claude,
        "codebuddy" => AgentKind::Codebuddy,
        "opencode" => AgentKind::Opencode,
        "codex" => AgentKind::Codex,
        "goose" => AgentKind::Goose,
        "aider" => AgentKind::Aider,
        "v8agent" => AgentKind::V8agent,
        _ => AgentKind::Unknown,
    }
}

fn status_to_str(s: SummaryJobStatus) -> &'static str {
    match s {
        SummaryJobStatus::Queued => "queued",
        SummaryJobStatus::Running => "running",
        SummaryJobStatus::Succeeded => "succeeded",
        SummaryJobStatus::Failed => "failed",
        SummaryJobStatus::Cancelled => "cancelled",
    }
}

fn status_from_str(s: &str) -> SummaryJobStatus {
    match s {
        "running" => SummaryJobStatus::Running,
        "succeeded" => SummaryJobStatus::Succeeded,
        "failed" => SummaryJobStatus::Failed,
        "cancelled" => SummaryJobStatus::Cancelled,
        _ => SummaryJobStatus::Queued,
    }
}

fn trigger_to_str(t: SummaryTrigger) -> &'static str {
    match t {
        SummaryTrigger::Close => "close",
        SummaryTrigger::Shutdown => "shutdown",
        SummaryTrigger::Backfill => "backfill",
        SummaryTrigger::Manual => "manual",
        SummaryTrigger::NaturalExit => "natural_exit",
    }
}

fn trigger_from_str(s: &str) -> SummaryTrigger {
    match s {
        "close" => SummaryTrigger::Close,
        "shutdown" => SummaryTrigger::Shutdown,
        "backfill" => SummaryTrigger::Backfill,
        "manual" => SummaryTrigger::Manual,
        "natural_exit" => SummaryTrigger::NaturalExit,
        _ => SummaryTrigger::Manual,
    }
}

fn error_kind_to_str(k: SummaryErrorKind) -> &'static str {
    match k {
        SummaryErrorKind::ConfigurationRequired => "configuration_required",
        SummaryErrorKind::UnsupportedCapability => "unsupported_capability",
        SummaryErrorKind::Spawn => "spawn",
        SummaryErrorKind::Authentication => "authentication",
        SummaryErrorKind::RateLimit => "rate_limit",
        SummaryErrorKind::Timeout => "timeout",
        SummaryErrorKind::NonzeroExit => "nonzero_exit",
        SummaryErrorKind::InvalidOutput => "invalid_output",
        SummaryErrorKind::InputUnavailable => "input_unavailable",
        SummaryErrorKind::StorageError => "storage_error",
        SummaryErrorKind::BudgetExceeded => "budget_exceeded",
        SummaryErrorKind::Cancelled => "cancelled",
    }
}

fn error_kind_from_str(s: &str) -> SummaryErrorKind {
    match s {
        "unsupported_capability" => SummaryErrorKind::UnsupportedCapability,
        "spawn" => SummaryErrorKind::Spawn,
        "authentication" => SummaryErrorKind::Authentication,
        "rate_limit" => SummaryErrorKind::RateLimit,
        "timeout" => SummaryErrorKind::Timeout,
        "nonzero_exit" => SummaryErrorKind::NonzeroExit,
        "invalid_output" => SummaryErrorKind::InvalidOutput,
        "input_unavailable" => SummaryErrorKind::InputUnavailable,
        "storage_error" => SummaryErrorKind::StorageError,
        "budget_exceeded" => SummaryErrorKind::BudgetExceeded,
        "cancelled" => SummaryErrorKind::Cancelled,
        _ => SummaryErrorKind::ConfigurationRequired,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 从同一 conversation 的多条旧 `session_summaries` 行里选稳定的一条
/// (spec 第 7 节):优先非空 AI,再按 `created_ts_ms` 降序、`session_id` 升序
/// 稳定排序,不任意覆盖。无 AI 或全空时返回 `None`。
pub fn select_legacy_result(
    mut payloads: Vec<SessionSummaryPayload>,
) -> Option<SessionSummaryPayload> {
    payloads.retain(|p| {
        p.status == dozer_core::protocol::SummaryStatus::AiGenerated
            && !p.title.is_empty()
            && !p.summary.is_empty()
    });
    if payloads.is_empty() {
        return None;
    }
    payloads.sort_by(|a, b| {
        b.created_ts_ms
            .cmp(&a.created_ts_ms)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    Some(payloads.into_iter().next().expect("已非空"))
}

pub struct SummaryJobStore {
    conn: Mutex<Connection>,
}

impl SummaryJobStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS conversation_summary_results (
                conversation_id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                facts_json TEXT,
                source_revision TEXT NOT NULL,
                pipeline_version TEXT NOT NULL,
                provider TEXT NOT NULL,
                requested_model TEXT,
                reported_model TEXT,
                coverage TEXT,
                generated_at INTEGER NOT NULL,
                source_job_id INTEGER NOT NULL,
                generation INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS summary_jobs (
                job_id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id TEXT NOT NULL,
                source_session_id TEXT,
                trigger TEXT NOT NULL,
                provider TEXT NOT NULL,
                requested_model TEXT,
                source_revision TEXT NOT NULL,
                pipeline_version TEXT NOT NULL,
                generation INTEGER NOT NULL,
                status TEXT NOT NULL,
                attempt INTEGER NOT NULL DEFAULT 0,
                error_kind TEXT,
                error_detail TEXT,
                created_ts_ms INTEGER NOT NULL,
                updated_ts_ms INTEGER NOT NULL,
                phase TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_summary_jobs_conversation
                ON summary_jobs(conversation_id);
            CREATE INDEX IF NOT EXISTS idx_summary_jobs_status
                ON summary_jobs(status);
            CREATE TABLE IF NOT EXISTS summary_batches (
                batch_id INTEGER PRIMARY KEY AUTOINCREMENT,
                cwd TEXT NOT NULL,
                strategy TEXT NOT NULL,
                total INTEGER NOT NULL DEFAULT 0,
                queued INTEGER NOT NULL DEFAULT 0,
                running INTEGER NOT NULL DEFAULT 0,
                succeeded INTEGER NOT NULL DEFAULT 0,
                failed INTEGER NOT NULL DEFAULT 0,
                cancelled INTEGER NOT NULL DEFAULT 0,
                skipped INTEGER NOT NULL DEFAULT 0,
                created_ts_ms INTEGER NOT NULL,
                updated_ts_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS summary_batch_jobs (
                batch_id INTEGER NOT NULL,
                job_id INTEGER NOT NULL,
                PRIMARY KEY (batch_id, job_id)
            );
            CREATE TABLE IF NOT EXISTS summary_artifacts (
                job_id INTEGER NOT NULL,
                kind TEXT NOT NULL,
                content TEXT NOT NULL,
                PRIMARY KEY (job_id, kind)
            );",
        )
        .context("建规范总结表")?;
        let has_cancelled = conn
            .prepare(
                "SELECT 1 FROM pragma_table_info('summary_batch_jobs') WHERE name='cancelled'",
            )?
            .exists([])?;
        if !has_cancelled {
            conn.execute_batch(
                "ALTER TABLE summary_batch_jobs ADD COLUMN cancelled INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 发布一份规范结果。CAS:只有当 `generation` 不低于已存行的 generation
    /// 时才覆盖(迟到旧 job 不覆盖新结果)。返回是否真正写入(被 CAS 拒绝
    /// 返回 `Ok(false)`)。
    pub fn record_result(
        &self,
        result: &ConversationSummaryResult,
        generation: i64,
    ) -> Result<bool> {
        let mut conn = self.conn.lock().expect("db lock");
        let tx = conn.transaction().context("开事务")?;
        let status: Option<String> = tx
            .query_row(
                "SELECT status FROM summary_jobs WHERE job_id=?1",
                [result.source_job_id],
                |r| r.get(0),
            )
            .optional()?;
        if status.as_deref() == Some("cancelled") {
            return Ok(false);
        }
        let existing: Option<i64> = tx
            .query_row(
                "SELECT generation FROM conversation_summary_results WHERE conversation_id = ?1",
                [&result.conversation_id],
                |r| r.get(0),
            )
            .optional()
            .context("查现有 generation")?;
        if let Some(g) = existing
            && generation < g
        {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO conversation_summary_results
             (conversation_id, title, summary, facts_json, source_revision,
              pipeline_version, provider, requested_model, reported_model,
              coverage, generated_at, source_job_id, generation)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
             ON CONFLICT(conversation_id) DO UPDATE SET
                title = excluded.title,
                summary = excluded.summary,
                facts_json = excluded.facts_json,
                source_revision = excluded.source_revision,
                pipeline_version = excluded.pipeline_version,
                provider = excluded.provider,
                requested_model = excluded.requested_model,
                reported_model = excluded.reported_model,
                coverage = excluded.coverage,
                generated_at = excluded.generated_at,
                source_job_id = excluded.source_job_id,
                generation = excluded.generation",
            rusqlite::params![
                result.conversation_id,
                result.title,
                result.summary,
                result.facts_json,
                result.source_revision,
                result.pipeline_version,
                agent_to_str(result.provider),
                result.requested_model,
                result.reported_model,
                result.coverage,
                result.generated_at,
                result.source_job_id,
                generation,
            ],
        )
        .context("写规范结果")?;
        tx.execute("UPDATE summary_jobs SET status='succeeded',error_kind=NULL,error_detail=NULL WHERE job_id=?1", [result.source_job_id])?;
        tx.commit().context("提交结果事务")?;
        Ok(true)
    }

    pub fn get_result(&self, conversation_id: &str) -> Result<Option<ConversationSummaryResult>> {
        let conn = self.conn.lock().expect("db lock");
        let row = conn
            .query_row(
                "SELECT conversation_id, title, summary, facts_json, source_revision,
                        pipeline_version, provider, requested_model, reported_model,
                        coverage, generated_at, source_job_id
                 FROM conversation_summary_results WHERE conversation_id = ?1",
                [conversation_id],
                |r| self.map_result(r),
            )
            .optional()?;
        Ok(row)
    }

    pub fn get_results_many(
        &self,
        conversation_ids: &[String],
    ) -> Result<HashMap<String, ConversationSummaryResult>> {
        if conversation_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = conversation_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT conversation_id, title, summary, facts_json, source_revision,
                    pipeline_version, provider, requested_model, reported_model,
                    coverage, generated_at, source_job_id
             FROM conversation_summary_results WHERE conversation_id IN ({placeholders})"
        );
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(conversation_ids), |r| {
            self.map_result(r)
        })?;
        let mut out = HashMap::new();
        for r in rows {
            let result = r?;
            out.insert(result.conversation_id.clone(), result);
        }
        Ok(out)
    }

    fn map_result(&self, r: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationSummaryResult> {
        let provider: String = r.get(6)?;
        Ok(ConversationSummaryResult {
            conversation_id: r.get(0)?,
            title: r.get(1)?,
            summary: r.get(2)?,
            facts_json: r.get(3)?,
            source_revision: r.get(4)?,
            pipeline_version: r.get(5)?,
            provider: agent_from_str(&provider),
            requested_model: r.get(7)?,
            reported_model: r.get(8)?,
            coverage: r.get(9)?,
            generated_at: r.get(10)?,
            source_job_id: r.get(11)?,
        })
    }

    /// 为某 conversation 分配下一个单调递增的 generation(现有最大值 + 1)。
    pub fn next_generation(&self, conversation_id: &str) -> Result<i64> {
        let conn = self.conn.lock().expect("db lock");
        let next: i64 = conn.query_row(
            "SELECT COALESCE(MAX(generation), 0) + 1 FROM summary_jobs WHERE conversation_id = ?1",
            [conversation_id],
            |r| r.get(0),
        )?;
        Ok(next)
    }

    /// 创建一条 summary job,返回 job_id。`generation` 由调用方用
    /// `next_generation` 预分配(单条提交需要原子拿号,避免竞态)。
    #[allow(clippy::too_many_arguments)]
    pub fn create_job(
        &self,
        conversation_id: &str,
        source_session_id: Option<&str>,
        trigger: SummaryTrigger,
        provider: AgentKind,
        requested_model: Option<&str>,
        source_revision: &str,
        pipeline_version: &str,
        generation: i64,
        status: SummaryJobStatus,
    ) -> Result<i64> {
        let now = now_ms();
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "INSERT INTO summary_jobs
             (conversation_id, source_session_id, trigger, provider, requested_model,
              source_revision, pipeline_version, generation, status, attempt,
              error_kind, error_detail, created_ts_ms, updated_ts_ms, phase)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,0,NULL,NULL,?10,?10,NULL)",
            rusqlite::params![
                conversation_id,
                source_session_id,
                trigger_to_str(trigger),
                agent_to_str(provider),
                requested_model,
                source_revision,
                pipeline_version,
                generation,
                status_to_str(status),
                now,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_job(&self, job_id: i64) -> Result<Option<SummaryJobInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let row = conn
            .query_row(
                "SELECT job_id, conversation_id, source_session_id, trigger, provider,
                        requested_model, source_revision, pipeline_version, generation,
                        status, attempt, error_kind, error_detail, created_ts_ms,
                        updated_ts_ms, phase
                 FROM summary_jobs WHERE job_id = ?1",
                [job_id],
                |r| self.map_job(r),
            )
            .optional()?;
        Ok(row)
    }

    pub fn list_jobs_by_conversation(&self, conversation_id: &str) -> Result<Vec<SummaryJobInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT job_id, conversation_id, source_session_id, trigger, provider,
                    requested_model, source_revision, pipeline_version, generation,
                    status, attempt, error_kind, error_detail, created_ts_ms,
                    updated_ts_ms, phase
             FROM summary_jobs WHERE conversation_id = ?1 ORDER BY job_id DESC",
        )?;
        let rows = stmt.query_map([conversation_id], |r| self.map_job(r))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn map_job(&self, r: &rusqlite::Row<'_>) -> rusqlite::Result<SummaryJobInfo> {
        let trigger: String = r.get(3)?;
        let provider: String = r.get(4)?;
        let status: String = r.get(9)?;
        let error_kind: Option<String> = r.get(11)?;
        Ok(SummaryJobInfo {
            job_id: r.get(0)?,
            conversation_id: r.get(1)?,
            source_session_id: r.get(2)?,
            trigger: trigger_from_str(&trigger),
            provider: agent_from_str(&provider),
            requested_model: r.get(5)?,
            source_revision: r.get(6)?,
            pipeline_version: r.get(7)?,
            generation: r.get(8)?,
            status: status_from_str(&status),
            attempt: r.get(10)?,
            error_kind: error_kind.map(|k| error_kind_from_str(&k)),
            error_detail: r.get(12)?,
            created_ts_ms: r.get(13)?,
            updated_ts_ms: r.get(14)?,
            phase: r.get(15)?,
        })
    }

    /// 更新 job 状态(含错误分类/脱敏描述/阶段)。`updated_ts_ms` 自动刷新。
    pub fn update_job_status(
        &self,
        job_id: i64,
        status: SummaryJobStatus,
        error_kind: Option<SummaryErrorKind>,
        error_detail: Option<&str>,
        phase: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "UPDATE summary_jobs
             SET status = ?1, error_kind = ?2, error_detail = ?3, phase = ?4,
                 updated_ts_ms = ?5
             WHERE job_id = ?6 AND status != 'cancelled'",
            rusqlite::params![
                status_to_str(status),
                error_kind.map(error_kind_to_str),
                error_detail,
                phase,
                now_ms(),
                job_id,
            ],
        )?;
        Ok(())
    }

    /// 重启恢复:把所有 running 归位 queued(worker 随进程消失,任务不能停在
    /// running)。
    pub fn requeue_running(&self) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "UPDATE summary_jobs SET status = 'queued', updated_ts_ms = ?1
             WHERE status = 'running'",
            [now_ms()],
        )?;
        Ok(())
    }

    /// 原子认领下一个 queued 任务(标记 running 并返回 job_id)。同一时刻只
    /// 允许一个 worker 调用(全局并发 1),但事务保证即使多 worker 也不重复。
    pub fn claim_next_queued(&self) -> Result<Option<i64>> {
        let mut conn = self.conn.lock().expect("db lock");
        let tx = conn.transaction()?;
        let next: Option<i64> = tx
            .query_row(
                "SELECT job_id FROM summary_jobs WHERE status = 'queued'
                 ORDER BY job_id ASC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(job_id) = next {
            tx.execute(
                "UPDATE summary_jobs SET status = 'running', updated_ts_ms = ?1
                 WHERE job_id = ?2",
                rusqlite::params![now_ms(), job_id],
            )?;
        }
        tx.commit()?;
        Ok(next)
    }

    /// 反查 job 所属 batch(可能属于多个,取最早一个)。
    pub fn batch_for_job(&self, job_id: i64) -> Result<Option<i64>> {
        let conn = self.conn.lock().expect("db lock");
        let row = conn
            .query_row(
                "SELECT batch_id FROM summary_batch_jobs WHERE job_id = ?1
                 ORDER BY batch_id ASC LIMIT 1",
                [job_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row)
    }

    /// 列出一个 batch 的所有 job。
    pub fn list_jobs_for_batch(&self, batch_id: i64) -> Result<Vec<SummaryJobInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT j.job_id, j.conversation_id, j.source_session_id, j.trigger, j.provider,
                    j.requested_model, j.source_revision, j.pipeline_version, j.generation,
                    j.status, j.attempt, j.error_kind, j.error_detail, j.created_ts_ms,
                    j.updated_ts_ms, j.phase
             FROM summary_jobs j
             JOIN summary_batch_jobs b ON b.job_id = j.job_id
             WHERE b.batch_id = ?1 ORDER BY j.job_id ASC",
        )?;
        let rows = stmt.query_map([batch_id], |r| self.map_job(r))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn create_batch(&self, cwd: &str, strategy: &str) -> Result<i64> {
        let now = now_ms();
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "INSERT INTO summary_batches (cwd, strategy, total, queued, running, succeeded,
                                          failed, cancelled, skipped, created_ts_ms, updated_ts_ms)
             VALUES (?1, ?2, 0, 0, 0, 0, 0, 0, 0, ?3, ?3)",
            rusqlite::params![cwd, strategy, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn add_batch_job(&self, batch_id: i64, job_id: i64) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "INSERT OR IGNORE INTO summary_batch_jobs (batch_id, job_id) VALUES (?1, ?2)",
            rusqlite::params![batch_id, job_id],
        )?;
        Ok(())
    }

    pub fn get_batch(&self, batch_id: i64) -> Result<Option<SummaryBatchInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let mut row = conn
            .query_row(
                "SELECT batch_id, cwd, strategy, total, queued, running, succeeded,
                        failed, cancelled, skipped, created_ts_ms, updated_ts_ms
                 FROM summary_batches WHERE batch_id = ?1",
                [batch_id],
                |r| {
                    Ok(SummaryBatchInfo {
                        batch_id: r.get(0)?,
                        cwd: r.get(1)?,
                        strategy: r.get(2)?,
                        total: r.get(3)?,
                        queued: r.get(4)?,
                        running: r.get(5)?,
                        succeeded: r.get(6)?,
                        failed: r.get(7)?,
                        cancelled: r.get(8)?,
                        skipped: r.get(9)?,
                        created_ts_ms: r.get(10)?,
                        updated_ts_ms: r.get(11)?,
                    })
                },
            )
            .optional()?;
        if let Some(ref mut batch) = row {
            let mut stmt = conn.prepare(
                "SELECT CASE WHEN b.cancelled=1 THEN 'cancelled' ELSE j.status END, count(*)
                FROM summary_batch_jobs b JOIN summary_jobs j ON j.job_id=b.job_id
                WHERE b.batch_id=?1 GROUP BY 1",
            )?;
            let counts = stmt
                .query_map([batch_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if !counts.is_empty() {
                batch.queued = 0;
                batch.running = 0;
                batch.succeeded = 0;
                batch.failed = 0;
                batch.cancelled = 0;
                for (status, n) in counts {
                    match status.as_str() {
                        "queued" => batch.queued = n,
                        "running" => batch.running = n,
                        "succeeded" => batch.succeeded = n,
                        "failed" => batch.failed = n,
                        "cancelled" => batch.cancelled = n,
                        _ => {}
                    }
                }
                batch.total = batch.queued
                    + batch.running
                    + batch.succeeded
                    + batch.failed
                    + batch.cancelled
                    + batch.skipped;
            }
        }
        Ok(row)
    }

    pub fn cancel_batch(&self, batch_id: i64) -> Result<()> {
        let mut conn = self.conn.lock().expect("db lock");
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE summary_batch_jobs SET cancelled=1 WHERE batch_id=?1
            AND job_id IN (SELECT job_id FROM summary_jobs WHERE status IN ('queued','running'))",
            [batch_id],
        )?;
        tx.execute("UPDATE summary_jobs SET status='cancelled',error_kind='cancelled',error_detail='批次被取消'
            WHERE trigger='backfill' AND status IN ('queued','running')
            AND job_id IN (SELECT job_id FROM summary_batch_jobs WHERE batch_id=?1)
            AND NOT EXISTS (SELECT 1 FROM summary_batch_jobs b WHERE b.job_id=summary_jobs.job_id AND b.cancelled=0)", [batch_id])?;
        tx.commit()?;
        Ok(())
    }

    /// 更新批次计数(由调度层在 job 状态变迁时调用)。
    #[allow(clippy::too_many_arguments)]
    pub fn update_batch_counts(
        &self,
        batch_id: i64,
        total: u32,
        queued: u32,
        running: u32,
        succeeded: u32,
        failed: u32,
        cancelled: u32,
        skipped: u32,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "UPDATE summary_batches SET total=?1, queued=?2, running=?3, succeeded=?4,
                    failed=?5, cancelled=?6, skipped=?7, updated_ts_ms=?8
             WHERE batch_id = ?9",
            rusqlite::params![
                total,
                queued,
                running,
                succeeded,
                failed,
                cancelled,
                skipped,
                now_ms(),
                batch_id
            ],
        )?;
        Ok(())
    }

    /// 持久化 job 的中间产物(规范输入 / chunk 输出 / 阶段进度)。
    pub fn save_artifact(&self, job_id: i64, kind: &str, content: &str) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "INSERT INTO summary_artifacts (job_id, kind, content) VALUES (?1, ?2, ?3)
             ON CONFLICT(job_id, kind) DO UPDATE SET content = excluded.content",
            rusqlite::params![job_id, kind, content],
        )?;
        Ok(())
    }

    pub fn load_artifact(&self, job_id: i64, kind: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("db lock");
        let row = conn
            .query_row(
                "SELECT content FROM summary_artifacts WHERE job_id = ?1 AND kind = ?2",
                rusqlite::params![job_id, kind],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_core::protocol::SummaryStatus;

    fn result(conversation_id: &str, title: &str) -> ConversationSummaryResult {
        ConversationSummaryResult {
            conversation_id: conversation_id.into(),
            title: title.into(),
            summary: "摘要".into(),
            facts_json: None,
            source_revision: "rev-1".into(),
            pipeline_version: "v1".into(),
            provider: AgentKind::Claude,
            requested_model: None,
            reported_model: None,
            coverage: None,
            generated_at: 1,
            source_job_id: 1,
        }
    }

    #[test]
    fn record_and_get_result_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SummaryJobStore::open(&dir.path().join("t.db")).unwrap();
        assert!(store.get_result("c1").unwrap().is_none());
        assert!(store.record_result(&result("c1", "标题"), 1).unwrap());
        let got = store.get_result("c1").unwrap().unwrap();
        assert_eq!(got.title, "标题");
    }

    #[test]
    fn cas_rejects_stale_generation() {
        let dir = tempfile::tempdir().unwrap();
        let store = SummaryJobStore::open(&dir.path().join("t.db")).unwrap();
        assert!(store.record_result(&result("c1", "新"), 2).unwrap());
        // 迟到旧 generation=1 不应覆盖 generation=2 的结果。
        assert!(!store.record_result(&result("c1", "旧"), 1).unwrap());
        assert_eq!(store.get_result("c1").unwrap().unwrap().title, "新");
    }

    #[test]
    fn cas_accepts_equal_or_newer_generation() {
        let dir = tempfile::tempdir().unwrap();
        let store = SummaryJobStore::open(&dir.path().join("t.db")).unwrap();
        assert!(store.record_result(&result("c1", "a"), 1).unwrap());
        assert!(store.record_result(&result("c1", "b"), 1).unwrap());
        assert_eq!(store.get_result("c1").unwrap().unwrap().title, "b");
        assert!(store.record_result(&result("c1", "c"), 3).unwrap());
        assert_eq!(store.get_result("c1").unwrap().unwrap().title, "c");
    }

    #[test]
    fn next_generation_is_monotonic_per_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let store = SummaryJobStore::open(&dir.path().join("t.db")).unwrap();
        assert_eq!(store.next_generation("c1").unwrap(), 1);
        store
            .create_job(
                "c1",
                None,
                SummaryTrigger::Manual,
                AgentKind::Claude,
                None,
                "r",
                "v1",
                1,
                SummaryJobStatus::Queued,
            )
            .unwrap();
        assert_eq!(store.next_generation("c1").unwrap(), 2);
        // 不同 conversation 独立计数。
        assert_eq!(store.next_generation("c2").unwrap(), 1);
    }

    #[test]
    fn job_roundtrip_preserves_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = SummaryJobStore::open(&dir.path().join("t.db")).unwrap();
        let id = store
            .create_job(
                "c1",
                Some("sess-1"),
                SummaryTrigger::Backfill,
                AgentKind::Codex,
                Some("gpt-5"),
                "rev-1",
                "v1",
                1,
                SummaryJobStatus::Queued,
            )
            .unwrap();
        let job = store.get_job(id).unwrap().unwrap();
        assert_eq!(job.conversation_id, "c1");
        assert_eq!(job.source_session_id.as_deref(), Some("sess-1"));
        assert_eq!(job.trigger, SummaryTrigger::Backfill);
        assert_eq!(job.provider, AgentKind::Codex);
        assert_eq!(job.requested_model.as_deref(), Some("gpt-5"));
        assert_eq!(job.status, SummaryJobStatus::Queued);
    }

    #[test]
    fn update_job_status_persists_error_kind() {
        let dir = tempfile::tempdir().unwrap();
        let store = SummaryJobStore::open(&dir.path().join("t.db")).unwrap();
        let id = store
            .create_job(
                "c1",
                None,
                SummaryTrigger::Manual,
                AgentKind::Claude,
                None,
                "r",
                "v1",
                1,
                SummaryJobStatus::Running,
            )
            .unwrap();
        store
            .update_job_status(
                id,
                SummaryJobStatus::Failed,
                Some(SummaryErrorKind::Authentication),
                Some("auth 失败"),
                Some("extract:1/2"),
            )
            .unwrap();
        let job = store.get_job(id).unwrap().unwrap();
        assert_eq!(job.status, SummaryJobStatus::Failed);
        assert_eq!(job.error_kind, Some(SummaryErrorKind::Authentication));
        assert_eq!(job.error_detail.as_deref(), Some("auth 失败"));
        assert_eq!(job.phase.as_deref(), Some("extract:1/2"));
    }

    #[test]
    fn batch_roundtrip_and_counts() {
        let dir = tempfile::tempdir().unwrap();
        let store = SummaryJobStore::open(&dir.path().join("t.db")).unwrap();
        let bid = store.create_batch("/p", "missing_or_stale").unwrap();
        let job_id = store
            .create_job(
                "c1",
                None,
                SummaryTrigger::Backfill,
                AgentKind::Claude,
                None,
                "r",
                "v1",
                1,
                SummaryJobStatus::Queued,
            )
            .unwrap();
        store.add_batch_job(bid, job_id).unwrap();
        store.update_batch_counts(bid, 3, 2, 0, 0, 1, 0, 0).unwrap();
        let batch = store.get_batch(bid).unwrap().unwrap();
        assert_eq!(batch.cwd, "/p");
        assert_eq!(batch.strategy, "missing_or_stale");
        // Stored counters may lag; the read must derive live state from jobs.
        assert_eq!(batch.total, 1);
        assert_eq!(batch.queued, 1);
        assert_eq!(batch.failed, 0);
    }

    #[test]
    fn artifact_roundtrip_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let store = SummaryJobStore::open(&dir.path().join("t.db")).unwrap();
        store.save_artifact(1, "snapshot", "v1").unwrap();
        assert_eq!(
            store.load_artifact(1, "snapshot").unwrap().as_deref(),
            Some("v1")
        );
        store.save_artifact(1, "snapshot", "v2").unwrap();
        assert_eq!(
            store.load_artifact(1, "snapshot").unwrap().as_deref(),
            Some("v2")
        );
        assert!(store.load_artifact(1, "missing").unwrap().is_none());
    }

    #[test]
    fn shared_batches_observe_live_state_and_cancel_independently() {
        let dir = tempfile::tempdir().unwrap();
        let store = SummaryJobStore::open(&dir.path().join("t.db")).unwrap();
        let job = store
            .create_job(
                "c",
                None,
                SummaryTrigger::Backfill,
                AgentKind::Claude,
                None,
                "rev",
                "v2",
                1,
                SummaryJobStatus::Running,
            )
            .unwrap();
        let a = store.create_batch("/p", "repair").unwrap();
        let b = store.create_batch("/p", "repair").unwrap();
        store.add_batch_job(a, job).unwrap();
        store.add_batch_job(b, job).unwrap();
        assert_eq!(store.get_batch(b).unwrap().unwrap().running, 1);
        store.cancel_batch(a).unwrap();
        assert_eq!(
            store.get_job(job).unwrap().unwrap().status,
            SummaryJobStatus::Running
        );
        assert_eq!(store.get_batch(a).unwrap().unwrap().cancelled, 1);
        store
            .update_job_status(job, SummaryJobStatus::Succeeded, None, None, None)
            .unwrap();
        assert_eq!(store.get_batch(b).unwrap().unwrap().succeeded, 1);
        assert_eq!(store.get_batch(a).unwrap().unwrap().cancelled, 1);
    }

    #[test]
    fn cancelled_job_cannot_publish_or_become_successful() {
        let dir = tempfile::tempdir().unwrap();
        let store = SummaryJobStore::open(&dir.path().join("t.db")).unwrap();
        let job = store
            .create_job(
                "c",
                None,
                SummaryTrigger::Backfill,
                AgentKind::Claude,
                None,
                "rev",
                "v2",
                1,
                SummaryJobStatus::Running,
            )
            .unwrap();
        let batch = store.create_batch("/p", "repair").unwrap();
        store.add_batch_job(batch, job).unwrap();
        store.cancel_batch(batch).unwrap();
        let mut summary = result("c", "取消后的结果");
        summary.source_job_id = job;
        assert!(!store.record_result(&summary, 1).unwrap());
        store
            .update_job_status(job, SummaryJobStatus::Succeeded, None, None, None)
            .unwrap();
        assert_eq!(
            store.get_job(job).unwrap().unwrap().status,
            SummaryJobStatus::Cancelled
        );
        assert!(store.get_result("c").unwrap().is_none());
    }

    fn payload(
        session_id: &str,
        title: &str,
        status: SummaryStatus,
        ts: u64,
    ) -> SessionSummaryPayload {
        SessionSummaryPayload {
            session_id: session_id.into(),
            agent_kind: AgentKind::Claude,
            conversation_id: Some("c1".into()),
            title: title.into(),
            summary: "摘要".into(),
            status,
            created_ts_ms: ts,
            task_id: None,
        }
    }

    #[test]
    fn select_legacy_result_prefers_nonempty_ai_then_newest() {
        let rows = vec![
            payload("s1", "空 ai", SummaryStatus::AiGenerated, 1),
            payload("s2", "最新 ai", SummaryStatus::AiGenerated, 10),
            payload("s3", "heuristic", SummaryStatus::HeuristicFallback, 100),
        ];
        let picked = select_legacy_result(rows).unwrap();
        assert_eq!(picked.title, "最新 ai");
    }

    #[test]
    fn select_legacy_result_none_when_all_heuristic_or_empty() {
        let rows = vec![
            payload("s1", "heuristic", SummaryStatus::HeuristicFallback, 1),
            payload("s2", "", SummaryStatus::AiGenerated, 2),
        ];
        assert!(select_legacy_result(rows).is_none());
    }
}
