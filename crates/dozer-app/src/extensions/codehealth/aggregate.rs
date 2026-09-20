//! 异步扫描/加载：仿 `extensions::usage::spawn_refresh` 的写法
//! （`crates/dozer-app/src/extensions/usage/mod.rs`）——`handle.spawn`
//! 起一个 tokio 任务，CPU 密集的本地扫描丢 `spawn_blocking`，dozerd
//! 往返走 `client`，结果经 `emit` 回灌成 `Message`。

use super::Message;
use dozer_codehealth::ProjectReport;
use std::path::PathBuf;

/// 面板打开时调用：只读落盘缓存，不触发扫描（spec：手动触发，不自动扫）。
pub fn spawn_load_cached(
    project_id: i64,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let cached = client.get_code_health_report(project_id).await.ok().flatten();
        let (report, scanned_at_ms) = match cached {
            Some(info) => {
                let report: Option<ProjectReport> = serde_json::from_str(&info.report_json).ok();
                (report, Some(info.scanned_at_ms))
            }
            None => (None, None),
        };
        emit(Message::Loaded(project_id, report, scanned_at_ms));
    });
}

/// 点"扫描"按钮：本地跑 `dozer_codehealth::scan_project`（CPU/IO 密集,
/// `spawn_blocking`），成功后异步落盘到 dozerd，再回灌 UI。
pub fn spawn_scan(
    project_id: i64,
    project_path: PathBuf,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            dozer_codehealth::scan_project(&project_path)
        })
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("扫描任务被取消: {e}")));
        match result {
            Ok(report) => {
                if let Ok(report_json) = serde_json::to_string(&report) {
                    let _ = client
                        .save_code_health_report(
                            project_id,
                            report_json,
                            report.total_loc as u64,
                            report.total_functions as u64,
                            report.critical_functions as u64,
                            format!("{:?}", report.overall_tier),
                        )
                        .await;
                }
                emit(Message::Scanned(project_id, Ok(report)));
            }
            Err(e) => emit(Message::Scanned(project_id, Err(e.to_string()))),
        }
    });
}
