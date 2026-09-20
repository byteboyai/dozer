//! 代码健康度面板的数据层与视图层（spec
//! docs/superpowers/specs/2026-09-20-code-health-panel-design.md）。仿
//! `extensions::usage` 的分层：`aggregate.rs` 做异步扫描/加载，`mod.rs`
//! 挂 `WorkspaceState`，`view.rs`（Task 8）渲染。

mod aggregate;
pub(crate) use aggregate::*;

mod view;
pub(crate) use view::content_pane;

use dozer_codehealth::ProjectReport;

#[derive(Default)]
pub struct WorkspaceState {
    report: Option<ProjectReport>,
    scanned_at_ms: Option<u64>,
    scanning: bool,
    /// 上一次 `Message::Scanned(_, Err(_))` 的错误文案，成功扫描或重新点
    /// "扫描"都会清掉（spec「错误处理」："扫描失败，请重试"，保留上一次
    /// 成功结果的同时要能看到这次失败了）。
    scan_error: Option<String>,
}

impl WorkspaceState {
    pub fn report(&self) -> Option<&ProjectReport> {
        self.report.as_ref()
    }

    pub fn scanned_at_ms(&self) -> Option<u64> {
        self.scanned_at_ms
    }

    pub fn scanning(&self) -> bool {
        self.scanning
    }

    pub fn scan_error(&self) -> Option<&str> {
        self.scan_error.as_deref()
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    /// 面板打开时读落盘缓存的结果。`Option` 为 `None` 代表这个项目还没
    /// 扫描过。
    Loaded(i64, Option<ProjectReport>, Option<u64>),
    /// 手动扫描结果回灌。
    Scanned(i64, Result<ProjectReport, String>),
    /// 点"扫描"按钮。
    ScanRequested,
    /// 问题列表点击某函数：文件路径 + 目标行(1-based)。由内核（`app/update.rs`
    /// 的 `Message::CodeHealth` 分发处）拦截转成顶层 `Message::CodeHealthOpenLocation`，
    /// 不进入本模块自己的 `update`（同 `usage::Message::ToggleListCollapse`
    /// "由内核拦截处理"的既有模式——见 `usage/mod.rs`）。
    OpenLocation(std::path::PathBuf, usize),
}

pub fn update(ws_state: &mut WorkspaceState, msg: Message) {
    match msg {
        Message::Loaded(_project_id, report, scanned_at_ms) => {
            ws_state.report = report;
            ws_state.scanned_at_ms = scanned_at_ms;
        }
        Message::Scanned(_project_id, Ok(report)) => {
            ws_state.scanning = false;
            ws_state.scan_error = None;
            ws_state.report = Some(report);
            ws_state.scanned_at_ms = Some(now_ms());
        }
        Message::Scanned(_project_id, Err(e)) => {
            ws_state.scanning = false;
            ws_state.scan_error = Some(e);
            // 保留上一次成功结果不被覆盖（见 spec「错误处理」）：不 touch
            // ws_state.report/scanned_at_ms。
        }
        Message::ScanRequested => {
            ws_state.scanning = true;
            ws_state.scan_error = None;
        }
        Message::OpenLocation(..) => {
            unreachable!("由内核拦截处理,见 codehealth::Message::OpenLocation 文档")
        }
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
    use dozer_codehealth::HealthTier;

    fn sample_report() -> ProjectReport {
        ProjectReport {
            total_loc: 100,
            total_functions: 5,
            critical_functions: 1,
            scale_tier: HealthTier::Healthy,
            density_tier: HealthTier::Watch,
            overall_tier: HealthTier::Watch,
            functions: vec![],
        }
    }

    #[test]
    fn loaded_populates_report_and_timestamp() {
        let mut ws = WorkspaceState::default();
        update(
            &mut ws,
            Message::Loaded(1, Some(sample_report()), Some(123)),
        );
        assert!(ws.report().is_some());
        assert_eq!(ws.scanned_at_ms(), Some(123));
    }

    #[test]
    fn loaded_none_means_never_scanned() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::Loaded(1, None, None));
        assert!(ws.report().is_none());
        assert_eq!(ws.scanned_at_ms(), None);
    }

    #[test]
    fn scan_requested_sets_scanning_flag() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::ScanRequested);
        assert!(ws.scanning());
    }

    #[test]
    fn scanned_ok_clears_scanning_and_updates_report() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::ScanRequested);
        update(&mut ws, Message::Scanned(1, Ok(sample_report())));
        assert!(!ws.scanning());
        assert!(ws.report().is_some());
        assert!(ws.scanned_at_ms().is_some());
    }

    #[test]
    fn scanned_err_clears_scanning_but_keeps_previous_report() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::Loaded(1, Some(sample_report()), Some(1)));
        update(&mut ws, Message::ScanRequested);
        update(&mut ws, Message::Scanned(1, Err("扫描失败".into())));
        assert!(!ws.scanning());
        assert!(ws.report().is_some(), "失败时应保留上一次成功结果");
        assert_eq!(ws.scanned_at_ms(), Some(1), "失败不应更新时间戳");
        assert_eq!(ws.scan_error(), Some("扫描失败"));
    }

    #[test]
    fn scan_requested_clears_previous_error() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::Scanned(1, Err("扫描失败".into())));
        assert_eq!(ws.scan_error(), Some("扫描失败"));
        update(&mut ws, Message::ScanRequested);
        assert_eq!(ws.scan_error(), None);
    }

    #[test]
    fn scanned_ok_clears_previous_error() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::Scanned(1, Err("扫描失败".into())));
        update(&mut ws, Message::Scanned(1, Ok(sample_report())));
        assert_eq!(ws.scan_error(), None);
    }
}
