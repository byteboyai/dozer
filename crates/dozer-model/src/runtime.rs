use crate::{AgentKind, AgentState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeCapabilities {
    pub terminal_stream_v2: bool,
    pub task_execution_state: bool,
    pub locate_v2: bool,
    pub backfill_state: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamMode {
    Resume,
    Reset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalState {
    pub lifecycle_revision: u64,
    pub agent_revision: u64,
    pub alive: bool,
    pub exit_code: Option<i32>,
    pub output_complete: bool,
    #[serde(default)]
    pub output_truncated: bool,
    pub agent: AgentKind,
    pub agent_state: AgentState,
    pub transcript_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TerminalEvent {
    Snapshot {
        epoch: u64,
        mode: StreamMode,
        start_offset: u64,
        next_offset: u64,
        data: Vec<u8>,
        state: TerminalState,
        degraded: bool,
    },
    Output {
        epoch: u64,
        start_offset: u64,
        end_offset: u64,
        data: Vec<u8>,
        #[serde(default)]
        replay: bool,
    },
    State {
        epoch: u64,
        state: TerminalState,
    },
    ResyncRequired {
        epoch: u64,
    },
    Recovering {
        epoch: u64,
    },
    Failed {
        epoch: u64,
        message: String,
    },
}

impl TerminalEvent {
    pub fn epoch(&self) -> u64 {
        match self {
            Self::Snapshot { epoch, .. }
            | Self::Output { epoch, .. }
            | Self::State { epoch, .. }
            | Self::ResyncRequired { epoch }
            | Self::Recovering { epoch }
            | Self::Failed { epoch, .. } => *epoch,
        }
    }
    pub fn data_len(&self) -> usize {
        match self {
            Self::Snapshot { data, .. } | Self::Output { data, .. } => data.len(),
            _ => 0,
        }
    }
    pub fn next_offset(&self) -> Option<u64> {
        match self {
            Self::Snapshot { next_offset, .. } => Some(*next_offset),
            Self::Output { end_offset, .. } => Some(*end_offset),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillPhase {
    #[default]
    NotStarted,
    Discovering,
    Ingesting,
    Complete,
    Failed,
    Cancelled,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackfillState {
    pub revision: u64,
    pub phase: BackfillPhase,
    pub discovered: u64,
    pub imported: u64,
    pub failed: u64,
    pub pending: u32,
    pub rescan_needed: bool,
    pub last_error: Option<String>,
}
