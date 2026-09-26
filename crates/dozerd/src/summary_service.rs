//! 持久总结调度与修复策略(spec 2026-09-26 第 4、7 节)。
//!
//! 旧 `session_summary_backfill` 只选"没有总结行"的 conversation,且内存态
//! 进度重启即丢。这里换成持久化的 `summary_jobs`/`summary_batches` 队列:
//! 单条提交、批次筛选(缺失/legacy/heuristic/空白/stale/失败)、默认全局并发
//! 1、公平队列、活动任务复用、generation/CAS、重启恢复、取消与批次计数。

use crate::summary_config::{SummaryConfig, SummaryProviderResolution};
use crate::summary_jobs::SummaryJobStore;
use crate::summary_pipeline::{self, FinalSummary, Summarizer};
use crate::summary_snapshot::{self, PIPELINE_VERSION};
use dozer_core::protocol::{
    AgentKind, ConversationSummaryResult, SessionSummaryPayload, SummaryErrorKind,
    SummaryJobStatus, SummaryTrigger,
};
use std::path::PathBuf;
use std::sync::Arc;

/// 需要总结的原因(spec 第 7 节:无结果、旧 heuristic、空白/占位、旧
/// pipeline_version、revision 不匹配、最近失败)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryReason {
    Missing,
    Legacy,
    Stale,
    Failed,
}

/// 对单个 conversation 的总结决策。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryAction {
    /// 当前有效结果,跳过。
    Skip,
    /// 需要生成。
    Generate { reason: SummaryReason },
}

/// 判断某 conversation 是否需要(重新)总结。输入:当前规范结果、旧表 legacy
/// 结果、当前 transcript revision、以及"最近是否有失败且无有效结果"。
pub fn decide_summary_action(
    current_result: Option<&ConversationSummaryResult>,
    legacy_summary: Option<&SessionSummaryPayload>,
    current_revision: &str,
    has_recent_failure: bool,
) -> SummaryAction {
    match current_result {
        None => {
            // 没有规范结果:优先看最近失败(失败且无有效结果可修),否则按
            // legacy(旧 heuristic/空白)或缺失分类。
            if has_recent_failure {
                SummaryAction::Generate {
                    reason: SummaryReason::Failed,
                }
            } else if legacy_summary.is_some() {
                SummaryAction::Generate {
                    reason: SummaryReason::Legacy,
                }
            } else {
                SummaryAction::Generate {
                    reason: SummaryReason::Missing,
                }
            }
        }
        Some(result) => {
            // 有规范结果:revision 不匹配(输入更新)或 pipeline_version 旧 →
            // stale;否则跳过。
            if result.source_revision != current_revision
                || result.pipeline_version != PIPELINE_VERSION
            {
                SummaryAction::Generate {
                    reason: SummaryReason::Stale,
                }
            } else {
                SummaryAction::Skip
            }
        }
    }
}

/// 持久总结调度服务。持有各 store,负责提交/取消/恢复与 worker 执行。
pub struct SummaryService {
    pub jobs: Arc<SummaryJobStore>,
    pub transcripts: Arc<crate::transcripts::TranscriptStore>,
    pub session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
    /// 隔离临时目录的父目录(每个 job 在下面建独立子目录)。
    pub scratch_root: PathBuf,
}

/// 单条提交的输入参数。
pub struct SubmitSpec {
    pub conversation_id: String,
    pub source_session_id: Option<String>,
    pub trigger: SummaryTrigger,
    pub provider: AgentKind,
    pub requested_model: Option<String>,
    /// 强制重新生成(绕过活动任务复用,但仍不并行覆盖同一 conversation)。
    pub force: bool,
}

impl SummaryService {
    /// 提交单条总结任务。返回 job_id(新建或复用活动任务)。
    pub fn submit_single(&self, spec: &SubmitSpec) -> anyhow::Result<i64> {
        // 输入 revision:读当前 transcript 快照的 revision(缺失时为固定值)。
        let turns = self
            .transcripts
            .get_conversation_turns_all(&spec.conversation_id, 100_000)
            .unwrap_or_default();
        let revision = summary_snapshot::revision_of(&turns);

        // 活动任务复用:同 conversation + revision + provider 的 queued/
        // running 任务存在且非 force,直接返回它。
        if !spec.force
            && let Some(existing) = self.find_active_job(
                &spec.conversation_id,
                &revision,
                PIPELINE_VERSION,
                spec.provider,
            )?
        {
            return Ok(existing);
        }

        let generation = self.jobs.next_generation(&spec.conversation_id)?;
        let job_id = self.jobs.create_job(
            &spec.conversation_id,
            spec.source_session_id.as_deref(),
            spec.trigger,
            spec.provider,
            spec.requested_model.as_deref(),
            &revision,
            PIPELINE_VERSION,
            generation,
            SummaryJobStatus::Queued,
        )?;
        Ok(job_id)
    }

    fn find_active_job(
        &self,
        conversation_id: &str,
        revision: &str,
        pipeline_version: &str,
        provider: AgentKind,
    ) -> anyhow::Result<Option<i64>> {
        let jobs = self.jobs.list_jobs_by_conversation(conversation_id)?;
        for j in jobs {
            if matches!(
                j.status,
                SummaryJobStatus::Queued | SummaryJobStatus::Running
            ) && j.source_revision == revision
                && j.pipeline_version == pipeline_version
                && j.provider == provider
            {
                return Ok(Some(j.job_id));
            }
        }
        Ok(None)
    }

    /// 提交一批修复任务:按选择策略筛出需要总结的 conversation,创建 batch +
    /// 关联 job。返回 batch_id 与 job 数。
    pub fn submit_batch(&self, cwd: &str) -> anyhow::Result<(i64, u32)> {
        let conversations = self
            .transcripts
            .list_conversations(cwd, None, u32::MAX, 0)
            .unwrap_or_default();
        let ids: Vec<String> = conversations
            .iter()
            .map(|c| c.conversation_id.clone())
            .collect();
        let legacy = self.session_summaries.get_many(&ids).unwrap_or_default();
        let results = self.jobs.get_results_many(&ids).unwrap_or_default();

        let mut selected = Vec::new();
        for conv in &conversations {
            let turns = self
                .transcripts
                .get_conversation_turns_all(&conv.conversation_id, 100_000)
                .unwrap_or_default();
            if turns.is_empty() {
                continue; // 没有 transcript 数据,不生成空总结。
            }
            let revision = summary_snapshot::revision_of(&turns);
            let current = results.get(&conv.conversation_id);
            let legacy_row = legacy.get(&conv.conversation_id);
            let has_recent_failure = self.jobs_has_recent_failure(&conv.conversation_id)?;
            let action = decide_summary_action(current, legacy_row, &revision, has_recent_failure);
            if matches!(action, SummaryAction::Generate { .. }) {
                selected.push(conv.conversation_id.clone());
            }
        }

        let provider = self.resolve_provider_or_default()?;
        let batch_id = self.jobs.create_batch(cwd, "missing_or_stale")?;
        let mut queued = 0u32;
        for conversation_id in &selected {
            let spec = SubmitSpec {
                conversation_id: conversation_id.clone(),
                source_session_id: None,
                trigger: SummaryTrigger::Backfill,
                provider,
                requested_model: None,
                force: false,
            };
            let job_id = self.submit_single(&spec)?;
            self.jobs.add_batch_job(batch_id, job_id)?;
            queued += 1;
        }
        self.jobs
            .update_batch_counts(batch_id, selected.len() as u32, queued, 0, 0, 0, 0, 0)?;
        Ok((batch_id, selected.len() as u32))
    }

    fn jobs_has_recent_failure(&self, conversation_id: &str) -> anyhow::Result<bool> {
        let jobs = self.jobs.list_jobs_by_conversation(conversation_id)?;
        Ok(jobs.iter().any(|j| j.status == SummaryJobStatus::Failed))
    }

    /// 解析 provider:UI 选择 → summary 配置 → default_agent;未配置返回错误。
    fn resolve_provider_or_default(&self) -> anyhow::Result<AgentKind> {
        match crate::summary_config::resolve_provider(None, None) {
            SummaryProviderResolution::Configured(cfg, _) => Ok(cfg.provider),
            SummaryProviderResolution::Required => {
                anyhow::bail!("未配置 summary provider(需要 summary 配置或 default_agent)")
            }
        }
    }

    /// 重启恢复:把所有 running 归位 queued(worker 已随进程消失)。
    pub fn recover_on_startup(&self) -> anyhow::Result<()> {
        // 直接扫 jobs 表把 running 改 queued。走 SQL 而不是逐条 get_job。
        self.jobs.requeue_running()?;
        Ok(())
    }

    /// 执行一个 job:读 transcript → 跑 pipeline → CAS 发布结果。`summarizer`
    /// 注入(测试用 fake)。成功返回 `Some(FinalSummary)`,失败记录错误并返回
    /// `Err`。
    pub async fn process_job(
        &self,
        job_id: i64,
        config: &SummaryConfig,
        summarizer: &mut dyn Summarizer,
    ) -> anyhow::Result<FinalSummary> {
        let job = self
            .jobs
            .get_job(job_id)?
            .ok_or_else(|| anyhow::anyhow!("job {job_id} 不存在"))?;

        let turns = self
            .transcripts
            .get_conversation_turns_all(&job.conversation_id, 100_000)?;
        let current_revision = summary_snapshot::revision_of(&turns);
        if current_revision != job.source_revision {
            // 输入在任务创建后更新了:结果将带 stale 语义发布,但不声称完整。
            tracing::warn!(
                job_id,
                conversation_id = %job.conversation_id,
                "输入在任务排队期间更新,发布结果将标记 stale"
            );
        }

        let mut budget = summary_pipeline::Budget::new();
        let final_summary = summary_pipeline::run_pipeline(&turns, config, summarizer, &mut budget)
            .await
            .map_err(|e| {
                let kind = pipeline_error_kind(&e);
                let _ = self.jobs.update_job_status(
                    job_id,
                    SummaryJobStatus::Failed,
                    Some(kind),
                    Some(&format!("{e:?}")),
                    None,
                );
                anyhow::anyhow!("pipeline 失败: {e:?}")
            })?;

        let result = ConversationSummaryResult {
            conversation_id: job.conversation_id.clone(),
            title: final_summary.title.clone(),
            summary: final_summary.summary.clone(),
            facts_json: serde_json::to_string(&final_summary.facts).ok(),
            source_revision: job.source_revision.clone(),
            pipeline_version: PIPELINE_VERSION.to_string(),
            provider: job.provider,
            requested_model: job.requested_model.clone(),
            reported_model: None,
            coverage: final_summary.facts.covered_turns.clone(),
            generated_at: now_ms(),
            source_job_id: job_id,
        };
        self.jobs
            .record_result(&result, job.generation)
            .map_err(|e| anyhow::anyhow!("发布结果失败: {e}"))?;
        self.jobs
            .update_job_status(job_id, SummaryJobStatus::Succeeded, None, None, None)?;
        Ok(final_summary)
    }

    /// worker 主循环:串行消费 queued job(全局并发 1),直到 shutdown 置位。
    /// `make_summarizer` 工厂每次 job 构造一个真实/注入 summarizer。
    pub async fn run_worker<F>(
        &self,
        shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
        make_summarizer: F,
    ) where
        F: Fn(AgentKind, &SummaryConfig) -> Box<dyn Summarizer + Send>,
    {
        loop {
            if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            match self.jobs.claim_next_queued() {
                Ok(Some(job_id)) => {
                    if let Some(job) = self.jobs.get_job(job_id).unwrap_or(None) {
                        let config = match self
                            .config_for_provider(job.provider, job.requested_model.clone())
                        {
                            Ok(c) => c,
                            Err(e) => {
                                let _ = self.jobs.update_job_status(
                                    job_id,
                                    SummaryJobStatus::Failed,
                                    Some(SummaryErrorKind::ConfigurationRequired),
                                    Some(&e.to_string()),
                                    None,
                                );
                                self.bump_batch(job_id);
                                continue;
                            }
                        };
                        let mut summarizer = make_summarizer(job.provider, &config);
                        let _ = self.process_job(job_id, &config, summarizer.as_mut()).await;
                        self.bump_batch(job_id);
                    }
                }
                Ok(None) => {
                    // 无任务,短暂等待新提交。
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "claim 下一个任务失败");
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }
    }

    fn config_for_provider(
        &self,
        provider: AgentKind,
        requested_model: Option<String>,
    ) -> anyhow::Result<SummaryConfig> {
        // 以 job 快照的 provider/model 为主,超时/预算用默认。
        Ok(SummaryConfig {
            provider,
            model: requested_model,
            call_timeout_secs: crate::summary_config::DEFAULT_CALL_TIMEOUT_SECS,
            max_retries: crate::summary_config::DEFAULT_MAX_RETRIES,
            input_budget_chars: crate::summary_config::DEFAULT_INPUT_BUDGET_CHARS,
        })
    }

    fn bump_batch(&self, job_id: i64) {
        // 批次计数更新:通过 batch_jobs 反查 batch,按 job 状态重新统计。
        if let Ok(Some(batch_id)) = self.jobs.batch_for_job(job_id) {
            let _ = self.recount_batch(batch_id);
        }
    }

    fn recount_batch(&self, batch_id: i64) -> anyhow::Result<()> {
        // 重新从 batch_jobs 联查每个 job 状态统计,保证计数恒等式。
        let jobs = self.jobs.list_jobs_for_batch(batch_id)?;
        let (mut queued, mut running, mut succeeded, mut failed, mut cancelled) =
            (0u32, 0u32, 0u32, 0u32, 0u32);
        for j in &jobs {
            match j.status {
                SummaryJobStatus::Queued => queued += 1,
                SummaryJobStatus::Running => running += 1,
                SummaryJobStatus::Succeeded => succeeded += 1,
                SummaryJobStatus::Failed => failed += 1,
                SummaryJobStatus::Cancelled => cancelled += 1,
            }
        }
        let total = jobs.len() as u32;
        self.jobs.update_batch_counts(
            batch_id, total, queued, running, succeeded, failed, cancelled, 0,
        )?;
        Ok(())
    }
}

fn pipeline_error_kind(e: &summary_pipeline::PipelineError) -> SummaryErrorKind {
    match e {
        summary_pipeline::PipelineError::BudgetExceeded => SummaryErrorKind::BudgetExceeded,
        summary_pipeline::PipelineError::Invoke(inner) => match inner {
            crate::summary_provider::SummaryInvokeError::ConfigurationRequired => {
                SummaryErrorKind::ConfigurationRequired
            }
            crate::summary_provider::SummaryInvokeError::UnsupportedCapability(_) => {
                SummaryErrorKind::UnsupportedCapability
            }
            crate::summary_provider::SummaryInvokeError::Spawn(_) => SummaryErrorKind::Spawn,
            crate::summary_provider::SummaryInvokeError::Authentication(_) => {
                SummaryErrorKind::Authentication
            }
            crate::summary_provider::SummaryInvokeError::RateLimit(_) => {
                SummaryErrorKind::RateLimit
            }
            crate::summary_provider::SummaryInvokeError::Timeout => SummaryErrorKind::Timeout,
            crate::summary_provider::SummaryInvokeError::NonzeroExit(..) => {
                SummaryErrorKind::NonzeroExit
            }
            crate::summary_provider::SummaryInvokeError::InvalidOutput(_) => {
                SummaryErrorKind::InvalidOutput
            }
            crate::summary_provider::SummaryInvokeError::InputUnavailable => {
                SummaryErrorKind::InputUnavailable
            }
        },
        summary_pipeline::PipelineError::InvalidChunkOutput(_)
        | summary_pipeline::PipelineError::MergeFailed(_) => SummaryErrorKind::InvalidOutput,
        summary_pipeline::PipelineError::InputUnavailable => SummaryErrorKind::InputUnavailable,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(revision: &str, pipeline: &str) -> ConversationSummaryResult {
        ConversationSummaryResult {
            conversation_id: "c1".into(),
            title: "t".into(),
            summary: "s".into(),
            facts_json: None,
            source_revision: revision.into(),
            pipeline_version: pipeline.into(),
            provider: AgentKind::Claude,
            requested_model: None,
            reported_model: None,
            coverage: None,
            generated_at: 1,
            source_job_id: 1,
        }
    }

    fn legacy() -> SessionSummaryPayload {
        SessionSummaryPayload {
            session_id: "s1".into(),
            agent_kind: AgentKind::Claude,
            conversation_id: Some("c1".into()),
            title: "旧".into(),
            summary: "旧摘要".into(),
            status: dozer_core::protocol::SummaryStatus::HeuristicFallback,
            created_ts_ms: 1,
            task_id: None,
        }
    }

    #[test]
    fn decide_missing_when_no_result_and_no_legacy() {
        let a = decide_summary_action(None, None, "rev", false);
        assert_eq!(
            a,
            SummaryAction::Generate {
                reason: SummaryReason::Missing
            }
        );
    }

    #[test]
    fn decide_legacy_when_legacy_but_no_result() {
        let a = decide_summary_action(None, Some(&legacy()), "rev", false);
        assert_eq!(
            a,
            SummaryAction::Generate {
                reason: SummaryReason::Legacy
            }
        );
    }

    #[test]
    fn decide_failed_when_recent_failure_and_no_result() {
        let a = decide_summary_action(None, None, "rev", true);
        assert_eq!(
            a,
            SummaryAction::Generate {
                reason: SummaryReason::Failed
            }
        );
    }

    #[test]
    fn decide_stale_when_revision_mismatch() {
        let a = decide_summary_action(Some(&result("old", "v1")), None, "new", false);
        assert_eq!(
            a,
            SummaryAction::Generate {
                reason: SummaryReason::Stale
            }
        );
    }

    #[test]
    fn decide_stale_when_pipeline_version_old() {
        let a = decide_summary_action(Some(&result("rev", "v0")), None, "rev", false);
        assert_eq!(
            a,
            SummaryAction::Generate {
                reason: SummaryReason::Stale
            }
        );
    }

    #[test]
    fn decide_skip_when_current_and_matching() {
        let a = decide_summary_action(Some(&result("rev", "v1")), None, "rev", false);
        assert_eq!(a, SummaryAction::Skip);
    }

    #[test]
    fn pipeline_error_kind_maps_budget() {
        assert_eq!(
            pipeline_error_kind(&summary_pipeline::PipelineError::BudgetExceeded),
            SummaryErrorKind::BudgetExceeded
        );
    }
}
