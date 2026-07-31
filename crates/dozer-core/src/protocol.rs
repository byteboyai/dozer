use serde::{Deserialize, Serialize, de::DeserializeOwned};

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
}

impl AgentKind {
    /// 展示用短标签（对话历史副行、GUI 角标）。
    pub fn label(&self) -> &'static str {
        match self {
            AgentKind::Unknown => "未知",
            AgentKind::Claude => "claude",
            AgentKind::Codebuddy => "codebuddy",
            AgentKind::Opencode => "opencode",
        }
    }
}

/// 项目（甲方资产域的根；P1g）。id 为 dozerd SQLite 主键。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub last_active_ms: u64,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    ListSessions,
    CreateSession {
        name: String,
        command: String,
        args: Vec<String>,
        cwd: String,
        cols: u16,
        rows: u16,
        /// 这个会话属于哪个项目——GUI 侧发起 CreateSession 时必须显式指定，
        /// 不再像单项目时代那样只靠一个隐式全局"当前项目"（P2a）。
        project_id: i64,
    },
    Attach {
        session_id: String,
        from_offset: u64,
    },
    Write {
        session_id: String,
        data_b64: String,
    },
    Resize {
        session_id: String,
        cols: u16,
        rows: u16,
    },
    Kill {
        session_id: String,
    },
    /// dozer-hook 单向上报的 agent hook 事件；data 原样透传（P1f 消费）。
    HookEvent {
        session_id: String,
        event: String,
        ts_ms: u64,
        data: serde_json::Value,
    },
    /// 验收通过的结构性记录（spec P1f D5）；acceptor 由 daemon 侧补 "user"。
    RecordAcceptance {
        repo: String,
        goal: String,
        criteria_checked: Vec<String>,
        verdict: String,
        comment: String,
        ref_name: String,
        ts_ms: u64,
    },
    /// 打开一个目录为项目（已存在则更新活跃时间，返回该项目信息）。
    /// P2a 起不再有"顺带置为当前项目"的副作用——daemon 不维护活跃项目
    /// 概念，"当前显示哪个项目"完全是 GUI 侧的本地状态。
    OpenProject {
        path: String,
    },
    /// 列出所有项目（按活跃时间倒序）。
    ListProjects,
    /// 取某仓库的验收次数（项目卡"N 次验收"用）。
    GetAcceptanceCount {
        repo: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Sessions {
        sessions: Vec<SessionInfo>,
    },
    Created {
        session: SessionInfo,
    },
    Attached {
        session_id: String,
        snapshot_b64: String,
        next_offset: u64,
    },
    Output {
        session_id: String,
        data_b64: String,
        offset: u64,
    },
    Exited {
        session_id: String,
        code: Option<i32>,
    },
    Ok,
    Error {
        message: String,
    },
    /// hook 事件引起的状态变更，随 attach 流广播给该会话的订阅者。
    AgentEvent {
        session_id: String,
        state: AgentState,
        event: String,
        ts_ms: u64,
        /// 该会话最新已知的 transcript 路径（hook data 携带；无则 None）。
        transcript_path: Option<String>,
    },
    /// 项目列表。
    Projects {
        projects: Vec<ProjectInfo>,
    },
    /// 单个/当前项目（无则 None）。
    Project {
        project: Option<ProjectInfo>,
    },
    /// 验收次数。
    AcceptanceCount {
        count: u64,
    },
}

pub fn encode_line<T: Serialize>(value: &T) -> String {
    let mut s = serde_json::to_string(value).expect("protocol types always serialize");
    s.push('\n');
    s
}

pub fn decode_line<T: DeserializeOwned>(line: &str) -> anyhow::Result<T> {
    Ok(serde_json::from_str(line)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrips_as_single_json_line() {
        let req = Request::CreateSession {
            name: "主线".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "cat".into()],
            cwd: "/tmp".into(),
            cols: 80,
            rows: 24,
            project_id: 1,
        };
        let line = encode_line(&req);
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let back: Request = decode_line(line.trim()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn reply_tag_is_snake_case() {
        let line = encode_line(&Reply::Attached {
            session_id: "s1".into(),
            snapshot_b64: "aGk=".into(),
            next_offset: 2,
        });
        assert!(line.contains(r#""type":"attached""#));
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_line::<Request>("not json").is_err());
    }

    #[test]
    fn hook_event_roundtrips() {
        let req = Request::HookEvent {
            session_id: "s1".into(),
            event: "Stop".into(),
            ts_ms: 123,
            data: serde_json::json!({"transcript_path": "/tmp/t.jsonl"}),
        };
        let line = encode_line(&req);
        let back: Request = decode_line(line.trim()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn agent_event_reply_tags_snake_case() {
        let line = encode_line(&Reply::AgentEvent {
            session_id: "s1".into(),
            state: AgentState::AwaitingInput,
            event: "Notification".into(),
            ts_ms: 5,
            transcript_path: None,
        });
        assert!(line.contains(r#""type":"agent_event""#));
        assert!(line.contains(r#""state":"awaiting_input""#));
    }

    #[test]
    fn project_messages_roundtrip() {
        let req = Request::OpenProject {
            path: "/repo/x".into(),
        };
        assert_eq!(
            decode_line::<Request>(encode_line(&req).trim()).unwrap(),
            req
        );

        let reply = Reply::Projects {
            projects: vec![ProjectInfo {
                id: 1,
                path: "/repo/x".into(),
                name: "x".into(),
                last_active_ms: 5,
            }],
        };
        let line = encode_line(&reply);
        assert!(line.contains(r#""type":"projects""#));
        assert_eq!(decode_line::<Reply>(line.trim()).unwrap(), reply);

        let reply = Reply::Project { project: None };
        assert_eq!(
            decode_line::<Reply>(encode_line(&reply).trim()).unwrap(),
            reply
        );
    }

    #[test]
    fn record_acceptance_roundtrips() {
        let req = Request::RecordAcceptance {
            repo: "/r".into(),
            goal: "目标".into(),
            criteria_checked: vec!["测试全绿".into()],
            verdict: "accepted".into(),
            comment: "".into(),
            ref_name: "refs/dozer/accepted/1".into(),
            ts_ms: 9,
        };
        let back: Request = decode_line(encode_line(&req).trim()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn agent_event_carries_transcript_path() {
        let line = encode_line(&Reply::AgentEvent {
            session_id: "s".into(),
            state: AgentState::Running,
            event: "UserPromptSubmit".into(),
            ts_ms: 1,
            transcript_path: Some("/t/x.jsonl".into()),
        });
        assert!(line.contains("/t/x.jsonl"));
        match decode_line::<Reply>(line.trim()).unwrap() {
            Reply::AgentEvent {
                transcript_path, ..
            } => assert_eq!(transcript_path.as_deref(), Some("/t/x.jsonl")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn old_session_info_without_transcript_path_decodes_none() {
        let old =
            r#"{"id":"a","name":"n","command":"/bin/sh","cwd":"/tmp","alive":true,"created_ms":1}"#;
        let info: SessionInfo = decode_line(old).unwrap();
        assert_eq!(info.transcript_path, None);
    }

    #[test]
    fn acceptance_count_request_roundtrips() {
        let req = Request::GetAcceptanceCount { repo: "/r".into() };
        let back: Request = decode_line(&encode_line(&req)).unwrap();
        assert_eq!(back, req);
        let rep = Reply::AcceptanceCount { count: 12 };
        let back: Reply = decode_line(&encode_line(&rep)).unwrap();
        assert_eq!(back, rep);
    }

    #[test]
    fn old_session_info_without_agent_state_decodes_as_idle() {
        // P1b-d 时代的 SessionInfo JSON（无 agent_state 字段）必须可解
        let old =
            r#"{"id":"a","name":"n","command":"/bin/sh","cwd":"/tmp","alive":true,"created_ms":1}"#;
        let info: SessionInfo = decode_line(old).unwrap();
        assert_eq!(info.agent_state, AgentState::Idle);
    }

    #[test]
    fn session_info_carries_project_id() {
        let info = SessionInfo {
            id: "a".into(),
            name: "n".into(),
            command: "/bin/sh".into(),
            cwd: "/tmp".into(),
            alive: true,
            created_ms: 1,
            agent_state: AgentState::Idle,
            transcript_path: None,
            project_id: Some(7),
            agent: AgentKind::Unknown,
        };
        let line = encode_line(&info);
        let back: SessionInfo = decode_line(line.trim()).unwrap();
        assert_eq!(back.project_id, Some(7));
    }

    #[test]
    fn old_session_info_without_project_id_decodes_none() {
        // 迁移期：daemon 重启前已存活的会话首次读出时没有 project_id 字段。
        let old =
            r#"{"id":"a","name":"n","command":"/bin/sh","cwd":"/tmp","alive":true,"created_ms":1}"#;
        let info: SessionInfo = decode_line(old).unwrap();
        assert_eq!(info.project_id, None);
    }

    #[test]
    fn create_session_request_carries_project_id() {
        let req = Request::CreateSession {
            name: "主线".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "cat".into()],
            cwd: "/tmp".into(),
            cols: 80,
            rows: 24,
            project_id: 3,
        };
        let line = encode_line(&req);
        let back: Request = decode_line(line.trim()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn agent_kind_defaults_to_unknown() {
        assert_eq!(AgentKind::default(), AgentKind::Unknown);
    }

    #[test]
    fn agent_kind_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&AgentKind::Codebuddy).unwrap(), "\"codebuddy\"");
        assert_eq!(serde_json::to_string(&AgentKind::Opencode).unwrap(), "\"opencode\"");
        assert_eq!(
            serde_json::from_str::<AgentKind>("\"claude\"").unwrap(),
            AgentKind::Claude
        );
    }

    #[test]
    fn agent_kind_label_matches_variant() {
        assert_eq!(AgentKind::Unknown.label(), "未知");
        assert_eq!(AgentKind::Claude.label(), "claude");
        assert_eq!(AgentKind::Codebuddy.label(), "codebuddy");
        assert_eq!(AgentKind::Opencode.label(), "opencode");
    }

    #[test]
    fn session_info_carries_agent() {
        let info = SessionInfo {
            id: "a".into(),
            name: "n".into(),
            command: "/bin/sh".into(),
            cwd: "/tmp".into(),
            alive: true,
            created_ms: 1,
            agent_state: AgentState::Idle,
            transcript_path: None,
            project_id: Some(7),
            agent: AgentKind::Claude,
        };
        let line = encode_line(&info);
        let back: SessionInfo = decode_line(line.trim()).unwrap();
        assert_eq!(back.agent, AgentKind::Claude);
    }

    #[test]
    fn old_session_info_without_agent_decodes_unknown() {
        let old =
            r#"{"id":"a","name":"n","command":"/bin/sh","cwd":"/tmp","alive":true,"created_ms":1}"#;
        let info: SessionInfo = decode_line(old).unwrap();
        assert_eq!(info.agent, AgentKind::Unknown);
    }
}
