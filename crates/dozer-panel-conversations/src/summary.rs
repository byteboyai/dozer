//! Summary jobs use only injected services and the host event emitter.
use crate::Message;
use dozer_model::SummaryJobStatus;
use dozer_panel_host::RequestTrackers;
use dozer_services::{ConversationService, SummaryService};
use std::{path::PathBuf, sync::Arc, time::Duration};
pub struct SummaryJobCtx {
    pub summary: Arc<dyn SummaryService>,
    pub conversation: Arc<dyn ConversationService>,
    pub requests: RequestTrackers,
    pub project_id: i64,
    pub cwd: PathBuf,
    pub conversation_id: String,
    pub poll_interval: Duration,
}
pub async fn run_summary_job(ctx: SummaryJobCtx, emit: impl Fn(Message) + Send + 'static) {
    let SummaryJobCtx {
        summary,
        conversation,
        requests,
        project_id,
        cwd,
        conversation_id: cid,
        poll_interval,
    } = ctx;
    let job_id = match summary.submit_summary_job(cid.clone(), true).await {
        Ok(id) => id,
        Err(e) => {
            crate::log::warn("提交总结任务失败", &format!("{cid}: {e}"));
            emit(Message::SummaryGenerateFinished(
                project_id,
                cid.clone(),
                Err(crate::i18n::keyed_args(
                    "summary_submit_failed",
                    "⚠ Failed to submit summary task: {error}",
                    &[("error", &e.to_string())],
                )),
            ));
            return;
        }
    };
    loop {
        tokio::time::sleep(poll_interval).await;
        match summary.get_summary_job(job_id).await {
            Ok(Some(job)) => {
                if job.status == SummaryJobStatus::Failed {
                    let err = match job.error_detail {
                        Some(detail) => crate::i18n::keyed_args(
                            "summary_failed",
                            "⚠ Failed to generate summary: {error}",
                            &[("error", &detail)],
                        ),
                        None => crate::i18n::keyed(
                            "summary_failed_unknown",
                            "⚠ Failed to generate summary",
                        ),
                    };
                    emit(Message::SummaryGenerateFinished(
                        project_id,
                        cid.clone(),
                        Err(err),
                    ));
                    return;
                }
                if job.status == SummaryJobStatus::Cancelled {
                    emit(Message::SummaryGenerateFinished(
                        project_id,
                        cid.clone(),
                        Err(crate::i18n::keyed(
                            "summary_cancelled",
                            "Summary task cancelled",
                        )),
                    ));
                    return;
                }
                if matches!(
                    job.status,
                    SummaryJobStatus::Succeeded
                        | SummaryJobStatus::Failed
                        | SummaryJobStatus::Cancelled
                ) {
                    break;
                }
            }
            Ok(None) => {
                crate::log::warn("总结任务不存在", &format!("{job_id}: {cid}"));
                emit(Message::SummaryGenerateFinished(
                    project_id,
                    cid.clone(),
                    Err(crate::i18n::keyed(
                        "summary_job_missing",
                        "Summary task not found, please retry",
                    )),
                ));
                return;
            }
            Err(e) => {
                crate::log::warn("查询总结任务失败", &format!("{job_id}: {cid}: {e}"));
                emit(Message::SummaryGenerateFinished(
                    project_id,
                    cid.clone(),
                    Err(crate::i18n::keyed_args(
                        "summary_poll_failed",
                        "Failed to query summary: {error}",
                        &[("error", &e.to_string())],
                    )),
                ));
                return;
            }
        }
    }
    let request = requests.begin(Some(project_id), "sessions");
    let result: Result<Vec<dozer_model::SessionRow>, dozer_services::ServiceError> = conversation
        .list_conversations_with_summaries(cwd.to_string_lossy().into_owned(), None, 500, 0)
        .await
        .map(|rows| {
            rows.iter()
                .map(|(c, s)| dozer_model::SessionRow::from_row(c, s.as_ref()))
                .collect()
        });
    let completion = result.as_ref().map(|_| ()).map_err(|e| {
        crate::i18n::keyed_args(
            "summary_refresh_failed",
            "Summary generated, but refreshing sessions failed: {error}",
            &[("error", &e.to_string())],
        )
    });
    emit(Message::AsyncResult(
        request,
        Box::new(Message::SessionsRefreshed(project_id, result)),
    ));
    emit(Message::SummaryGenerateFinished(
        project_id, cid, completion,
    ));
}
