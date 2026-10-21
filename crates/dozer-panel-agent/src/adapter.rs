//! 产品注入接缝:宿主请求、宿主事件、能力开关与 hook 安装器(S6c-1)。

use dozer_model::{AgentKind, AgentState};

/// 面板请求产品宿主执行的动作(库内不碰原生菜单等产品专属能力)。
#[derive(Debug, Clone, PartialEq)]
pub enum AgentRequest {
    /// 打开 agent 选择器。`at` 是锚点(逻辑坐标),产品可忽略。
    ShowPicker { at: Option<(f32, f32)> },
}

/// 面板要宿主跟随处理的事件(卡片刷新、会话列表刷新、git/磁盘占用刷新等
/// 跨面板联动仍由产品负责)。
#[derive(Debug, Clone, PartialEq)]
pub enum AgentHostEvent {
    StateChanged {
        tab_id: usize,
        agent: AgentKind,
        state: AgentState,
        transcript_path: Option<String>,
    },
    TurnEnded {
        tab_id: usize,
    },
    SessionExited {
        tab_id: usize,
    },
}

/// 产品能力开关(本期只需要这两项;其余在 S6c-2 随视图加入)。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgentCapabilities {
    pub summary_on_close: bool,
    pub mcp_install: bool,
}

impl AgentCapabilities {
    pub fn all() -> Self {
        Self {
            summary_on_close: true,
            mcp_install: true,
        }
    }
}

/// hook / MCP 安装器:路径与安装实现留在产品(Doze 是 `dozer-hook` 与
/// `dozer-mcp`),库只按需调用。
pub trait HookInstaller: Send + Sync {
    fn ensure_hook(&self, agent: AgentKind);
    fn ensure_mcp(&self, agent: AgentKind);
    /// sibling `dozer-hook` 的绝对路径(Aider launcher 用)。
    fn hook_exe(&self) -> String;
}

/// 卡片"当前工作内容"的任务标题反查(产品实现:Doze 的 Todo 派发记录)。
pub trait TaskTitleSource {
    fn task_title_for_session(&self, session_id: &str) -> Option<String>;
}
