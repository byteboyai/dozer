//! Agent 面板的 Dozer 适配层(S6c-1):hook/MCP 安装器,以及把宿主事件
//! 落回 Dozer 的跨面板联动。

use dozer_model::AgentKind;
use dozer_panel_agent::adapter::HookInstaller;
use std::path::PathBuf;

/// 转调 Dozer 既有 hook/MCP 安装实现(路径与安装逻辑留在产品侧)。
pub(crate) struct DozerHooks;

impl HookInstaller for DozerHooks {
    fn ensure_hook(&self, agent: AgentKind) {
        crate::workspace::ensure_hook_installed(agent);
    }

    fn ensure_mcp(&self, agent: AgentKind) {
        crate::workspace::ensure_mcp_installed(agent);
    }

    fn hook_exe(&self) -> String {
        crate::workspace::dozer_hook_binary_path(
            &std::env::current_exe().unwrap_or_else(|_| PathBuf::from("dozer")),
        )
        .to_string_lossy()
        .into_owned()
    }
}
