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
    /// 全部变体(含 `Unknown`),顺序即声明顺序。新增变体必须同步这里;
    /// 测试里的无通配穷尽 `match` 会在漏改时让编译失败。
    pub const ALL: [AgentKind; 9] = [
        AgentKind::Unknown,
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::Codex,
        AgentKind::Goose,
        AgentKind::Aider,
        AgentKind::V8agent,
        AgentKind::Qodercn,
    ];

    /// 真实 agent(不含 `Unknown`),保持 `ALL` 的相对顺序。
    pub fn known() -> impl Iterator<Item = AgentKind> {
        Self::ALL.into_iter().filter(|a| *a != AgentKind::Unknown)
    }

    /// `label()` 的反向查找,只认真实 agent。`None` 的含义由调用方决定
    /// (摄取/持久化读回通常 `unwrap_or(Unknown)`)。
    pub fn from_label(s: &str) -> Option<AgentKind> {
        Self::known().find(|a| a.label() == s)
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// 无通配分支的穷尽 match:新增 `AgentKind` 变体时这里会**编译失败**,
    /// 逼着改动者来更新 `AgentKind::ALL`(并在下面的测试里补上新的 label)。
    fn variant_index(a: AgentKind) -> usize {
        match a {
            AgentKind::Unknown => 0,
            AgentKind::Claude => 1,
            AgentKind::Codebuddy => 2,
            AgentKind::Opencode => 3,
            AgentKind::Codex => 4,
            AgentKind::Goose => 5,
            AgentKind::Aider => 6,
            AgentKind::V8agent => 7,
            AgentKind::Qodercn => 8,
        }
    }

    #[test]
    fn all_is_listed_in_variant_order_without_gaps() {
        assert_eq!(AgentKind::ALL.len(), 9);
        for (i, a) in AgentKind::ALL.iter().enumerate() {
            assert_eq!(variant_index(*a), i, "{a:?} 在 ALL 中的位置与声明顺序不符");
        }
    }

    #[test]
    fn known_excludes_only_unknown_and_keeps_order() {
        let known: Vec<_> = AgentKind::known().collect();
        assert_eq!(known.len(), 8);
        assert!(!known.contains(&AgentKind::Unknown));
        assert_eq!(known[0], AgentKind::Claude);
        assert_eq!(known[7], AgentKind::Qodercn);
    }

    /// `label()` 是写进 SQLite 的持久化名,改动任何一个都会让旧数据读不回。
    #[test]
    fn labels_are_stable_persisted_names() {
        let expected = [
            (AgentKind::Unknown, "shell"),
            (AgentKind::Claude, "claude"),
            (AgentKind::Codebuddy, "codebuddy"),
            (AgentKind::Opencode, "opencode"),
            (AgentKind::Codex, "codex"),
            (AgentKind::Goose, "goose"),
            (AgentKind::Aider, "aider"),
            (AgentKind::V8agent, "v8agent"),
            (AgentKind::Qodercn, "qodercn"),
        ];
        for (agent, label) in expected {
            assert_eq!(agent.label(), label, "{agent:?}");
        }
    }

    #[test]
    fn from_label_round_trips_every_known_agent() {
        for a in AgentKind::known() {
            assert_eq!(AgentKind::from_label(a.label()), Some(a), "{a:?}");
        }
    }

    #[test]
    fn from_label_rejects_unknown_shell_and_garbage() {
        assert_eq!(AgentKind::from_label("shell"), None);
        assert_eq!(AgentKind::from_label("unknown"), None);
        assert_eq!(AgentKind::from_label(""), None);
        assert_eq!(AgentKind::from_label("Claude"), None, "大小写敏感,与旧 match 一致");
        assert_eq!(AgentKind::from_label("chatgpt"), None);
    }
}
