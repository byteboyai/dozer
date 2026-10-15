use serde::{Deserialize, Serialize};

/// agent 会话状态（hook 事件驱动的四态机；spec P1e D6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    #[default]
    Idle,
    Running,
    AwaitingInput,
    TurnEnded,
}

/// 会话当前归属的 agent（协议层从 P1e-P1j 时代的"隐式恒 Claude"升级为
/// 显式字段；P2b 多 agent 支持第一步）。`Unknown` 是首个 hook 事件到达前
/// 的默认值，也是老协议帧缺该字段时的回落值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    #[default]
    Unknown,
    Claude,
    Codebuddy,
    Opencode,
    Codex,
    Goose,
    Aider,
    V8agent,
    Qodercn,
}

impl AgentKind {
    /// 展示用短标签（对话历史副行、GUI 角标）。同时也是启动器菜单键入
    /// 的 CLI 命令名——六家均已核实与官方命令名一致（见计划 Global
    /// Constraints）。与 `display_label()` 的差异：用户能看见的展示名
    /// 走 `display_label()`(品牌大小写/有空格),命令键入/对账走
    /// `label()`(全小写 CLI 名)——如 `Claude` 的展示是 "Claude"、命令
    /// 是 "claude"。
    pub fn label(&self) -> &'static str {
        match self {
            AgentKind::Unknown => "shell",
            AgentKind::Claude => "claude",
            AgentKind::Codebuddy => "codebuddy",
            AgentKind::Opencode => "opencode",
            AgentKind::Codex => "codex",
            AgentKind::Goose => "goose",
            AgentKind::Aider => "aider",
            AgentKind::V8agent => "v8agent",
            AgentKind::Qodercn => "qodercn",
        }
    }

    /// 用户可见的展示名（Agent 面板分组头等品牌位），与 `label()`
    /// 的 CLI 命令名区分开——如 `Claude` 的品牌是 "Claude"，但实际命令行
    /// 是 `claude`，二者不能混用。改动这里要同步去改
    /// `dozer-app::workspace::view::agent_picker_spec` 的字面量菜单。
    pub fn display_label(&self) -> &'static str {
        match self {
            AgentKind::Unknown => "shell",
            AgentKind::Claude => "Claude",
            AgentKind::Codebuddy => "CodeBuddy",
            AgentKind::Opencode => "OpenCode",
            AgentKind::Codex => "Codex",
            AgentKind::Goose => "Goose",
            AgentKind::Aider => "Aider",
            AgentKind::V8agent => "v8agent",
            AgentKind::Qodercn => "Qoder CN",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub cwd: String,
    pub alive: bool,
    pub created_ms: u64,
    /// 会话内 agent 的最新状态；旧协议帧无此字段时回落 Idle。
    #[serde(default)]
    pub agent_state: AgentState,
    /// 当前会话 agent 的 transcript 文件路径（Claude Code JSONL；hook 携带）。
    #[serde(default)]
    pub transcript_path: Option<String>,
    /// 会话归属的项目 id（多项目并行；P2a）。`None` 表示迁移期孤儿会话——
    /// daemon 重启前已存活、早于本字段引入时创建的会话，首次读出时没有
    /// 归属信息，不强行捏造一个。
    #[serde(default)]
    pub project_id: Option<i64>,
    /// 会话归属的 agent；首个 hook 事件到达前恒 `Unknown`（P2b）。
    #[serde(default)]
    pub agent: AgentKind,
}
