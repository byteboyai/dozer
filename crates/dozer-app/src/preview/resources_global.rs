//! Dozer 一侧的进程级预览资源管理器单例,以及 `PanelKind` ↔ 核心 `ViewerPanel` 的边界映射。
//!
//! 核心 `dozer-preview-core::resources` 只有纯数据结构 `ResourceManager`(预算由构造时显式传入,
//! 不读任何全局);"一个进程一份、用本机探测到的预算构造"是 Dozer 的产品决定,留在这里。
//! Digger 将来按自己的预算构造并持有自己的实例。

use std::sync::{Mutex, OnceLock};

use dozer_preview_core::resources::{ResourceManager, ViewerPanel};

use crate::app::PanelKind;

static GLOBAL_MANAGER: OnceLock<Mutex<ResourceManager>> = OnceLock::new();

/// 进程级共享资源管理器。所有项目和预览面板共用同一份预算。
pub fn global_manager() -> &'static Mutex<ResourceManager> {
    GLOBAL_MANAGER
        .get_or_init(|| Mutex::new(ResourceManager::new(crate::capabilities::current().budgets)))
}

/// 面板 → 预算归属面板。非预览面板(不会产生 viewer)落到 `Files`,与 `panel_token` 里
/// `_ => "files"` 的既有兜底一致。
pub(crate) fn viewer_panel(kind: PanelKind) -> ViewerPanel {
    match kind {
        PanelKind::Files => ViewerPanel::Files,
        PanelKind::Project => ViewerPanel::Project,
        PanelKind::GitLog => ViewerPanel::GitLog,
        _ => ViewerPanel::Files,
    }
}

impl From<ViewerPanel> for PanelKind {
    fn from(panel: ViewerPanel) -> Self {
        match panel {
            ViewerPanel::Files => PanelKind::Files,
            ViewerPanel::Project => PanelKind::Project,
            ViewerPanel::GitLog => PanelKind::GitLog,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_panel_maps_the_three_preview_hosts_and_falls_back_to_files() {
        assert_eq!(viewer_panel(PanelKind::Files), ViewerPanel::Files);
        assert_eq!(viewer_panel(PanelKind::Project), ViewerPanel::Project);
        assert_eq!(viewer_panel(PanelKind::GitLog), ViewerPanel::GitLog);
        for other in [
            PanelKind::Todo,
            PanelKind::Database,
            PanelKind::Ssh,
            PanelKind::Web,
            PanelKind::Agent,
            PanelKind::GroupChat,
            PanelKind::Conversations,
            PanelKind::Usage,
            PanelKind::CodeHealth,
        ] {
            assert_eq!(viewer_panel(other), ViewerPanel::Files, "{other:?}");
        }
    }

    #[test]
    fn the_mapping_roundtrips_for_every_viewer_panel() {
        for panel in [
            ViewerPanel::Files,
            ViewerPanel::Project,
            ViewerPanel::GitLog,
        ] {
            assert_eq!(viewer_panel(PanelKind::from(panel)), panel);
        }
    }

    #[test]
    fn the_global_manager_is_one_instance_with_the_probed_budgets() {
        let a = global_manager() as *const _;
        let b = global_manager() as *const _;
        assert_eq!(a, b);
        let budgets = *global_manager().lock().unwrap().budgets();
        assert_eq!(budgets, crate::capabilities::current().budgets);
    }
}
