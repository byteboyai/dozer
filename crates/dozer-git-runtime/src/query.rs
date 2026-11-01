//! 查询接缝与纯数据结果。
//!
//! 生产实现 [`BytegitBackend`] 每个任务只 `discover_workdir` 一次;所有被请求的
//! 数据类别在同一任务里按需读取。测试可注入自己的 [`QueryBackend`] 来精确断言
//! discover 次数、status 次数、乱序与失败语义。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bytegit::{CommitId, GitError, Repo, RepoIdentity, StatusEntry, StatusOptions};

use crate::identity::Categories;

/// 一个数据类别的结果。失败时保留上一次成功值,让 UI 能显示 stale 而不是清空。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field<T> {
    Ready(T),
    Failed { last: Option<T>, message: String },
}

impl<T: Clone> Field<T> {
    pub fn ready(value: T) -> Self {
        Self::Ready(value)
    }

    /// 可信值(失败时没有)。
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            Self::Failed { .. } => None,
        }
    }

    /// 当前可展示的值:成功值,或失败前保留的最后成功值。
    pub fn last(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            Self::Failed { last, .. } => last.as_ref(),
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Ready(_) => None,
            Self::Failed { message, .. } => Some(message),
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    fn from_result(result: Result<T, String>, previous: &Field<T>) -> Self {
        match result {
            Ok(value) => Self::Ready(value),
            Err(message) => Self::Failed {
                last: previous.last().cloned(),
                message,
            },
        }
    }

    fn fail(self, message: String) -> Self {
        Self::Failed {
            last: self.last().cloned(),
            message,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadData {
    pub branch: Option<String>,
    pub commit: Option<CommitId>,
}

impl From<bytegit::HeadInfo> for HeadData {
    fn from(value: bytegit::HeadInfo) -> Self {
        Self {
            branch: value.branch,
            commit: value.commit,
        }
    }
}

/// 全仓库状态(路径相对仓库根)。宿主再按项目子树投影并换算 UI 路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusData {
    pub entries: Vec<StatusEntry>,
    /// 排除纯 ignored 的真实改动;见 `bytegit::StatusSnapshot::dirty`。
    pub dirty: bool,
}

pub type RemoteData = bytegit::Remote;

/// 一次工作区查询的纯结果。字段级 `Result` 允许 remotes 失败时不清空有效 status。
#[derive(Debug, Clone)]
pub struct QueryOutput {
    pub identity: RepoIdentity,
    pub head: Option<Result<HeadData, String>>,
    pub status: Option<Result<StatusData, String>>,
    pub branches: Option<Result<Vec<String>, String>>,
    pub remotes: Option<Result<Vec<RemoteData>, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryFailure {
    pub message: String,
}

impl QueryFailure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<GitError> for QueryFailure {
    fn from(value: GitError) -> Self {
        Self::new(value.message())
    }
}

#[derive(Debug, Clone)]
pub struct QueryRequest {
    pub path: PathBuf,
    pub categories: Categories,
    pub status_options: StatusOptions,
}

/// 可注入的查询后端。`discover` 用于订阅时解析工作区身份;`query` 在一个任务里
/// 完成一次 discover 与按需读取。
pub trait QueryBackend: Send + Sync + 'static {
    fn discover(&self, path: &Path) -> Result<RepoIdentity, QueryFailure>;

    fn query(&self, request: &QueryRequest) -> Result<QueryOutput, QueryFailure>;
}

/// 生产实现:直接走 bytegit,同步方法由调度器放进 `spawn_blocking`。
#[derive(Debug, Default)]
pub struct BytegitBackend;

impl QueryBackend for BytegitBackend {
    fn discover(&self, path: &Path) -> Result<RepoIdentity, QueryFailure> {
        discover_identity(path)
    }

    fn query(&self, request: &QueryRequest) -> Result<QueryOutput, QueryFailure> {
        let repo = Repo::discover_workdir(&request.path).map_err(QueryFailure::from)?;
        let identity = repo.identity().map_err(QueryFailure::from)?;
        // 裸仓库没有工作区身份;这里提前失败,避免后续状态查询把它当普通仓库。
        identity.workspace_key().map_err(QueryFailure::from)?;

        let head = request
            .categories
            .contains(Categories::HEAD)
            .then(|| repo.head().map(HeadData::from).map_err(message));
        let status = request.categories.contains(Categories::STATUS).then(|| {
            repo.status_snapshot(request.status_options)
                .map(|snapshot| StatusData {
                    entries: snapshot.entries,
                    dirty: snapshot.dirty,
                })
                .map_err(message)
        });
        let branches = request
            .categories
            .contains(Categories::BRANCHES)
            .then(|| repo.local_branches().map_err(message));
        let remotes = request
            .categories
            .contains(Categories::REMOTES)
            .then(|| repo.remotes().map_err(message));

        Ok(QueryOutput {
            identity,
            head,
            status,
            branches,
            remotes,
        })
    }
}

fn discover_identity(path: &Path) -> Result<RepoIdentity, QueryFailure> {
    let repo = Repo::discover_workdir(path).map_err(QueryFailure::from)?;
    let identity = repo.identity().map_err(QueryFailure::from)?;
    identity.workspace_key().map_err(QueryFailure::from)?;
    Ok(identity)
}

fn message(error: GitError) -> String {
    error.message().to_string()
}

/// 一个工作区的最近快照。`generations` 记录每个字段实际对应的版本;某个字段
/// 失败时保留上次成功值并标记 stale。
#[derive(Debug, Clone)]
pub struct WorkspaceSnapshot {
    pub identity: RepoIdentity,
    pub head: Field<HeadData>,
    pub status: Field<StatusData>,
    pub branches: Field<Vec<String>>,
    pub remotes: Field<Vec<RemoteData>>,
    pub generations: crate::identity::Generations,
    pub sampled_at: Instant,
    pub query_duration: Duration,
    /// 查询返回的身份与订阅时不同(例如 `.git` 被重指向)。此快照不应用新仓库
    /// 的数据,宿主应重新订阅。
    pub identity_mismatch: Option<RepoIdentity>,
}

impl WorkspaceSnapshot {
    pub fn empty(identity: RepoIdentity, message: impl Into<String>, sampled_at: Instant) -> Self {
        let message = message.into();
        Self {
            identity,
            head: Field::Failed {
                last: None,
                message: message.clone(),
            },
            status: Field::Failed {
                last: None,
                message: message.clone(),
            },
            branches: Field::Failed {
                last: None,
                message: message.clone(),
            },
            remotes: Field::Failed {
                last: None,
                message,
            },
            generations: crate::identity::Generations::default(),
            sampled_at,
            query_duration: Duration::ZERO,
            identity_mismatch: None,
        }
    }

    /// 任一字段失败(包括保留了 last-good 的 stale 状态)。
    pub fn is_stale(&self) -> bool {
        self.head.error().is_some()
            || self.status.error().is_some()
            || self.branches.error().is_some()
            || self.remotes.error().is_some()
    }

    /// 至少有字段成功过(或状态查询成功并返回空集合)。
    pub fn has_data(&self) -> bool {
        self.head.value().is_some()
            || self.status.value().is_some()
            || self.branches.value().is_some()
            || self.remotes.value().is_some()
    }
}

/// 把一次查询结果合并进上一份快照。只有 `valid` 里的类别会被覆盖;其余类别保留
/// 旧值(通常是更新的 generation 已经在排队)。
pub(crate) fn merge_output(
    previous: Option<&WorkspaceSnapshot>,
    identity: RepoIdentity,
    output: QueryOutput,
    valid: Categories,
    generations: crate::identity::Generations,
    sampled_at: Instant,
    query_duration: Duration,
) -> WorkspaceSnapshot {
    let head_prev = previous.map(|s| s.head.clone()).unwrap_or(Field::Failed {
        last: None,
        message: "尚未查询".into(),
    });
    let status_prev = previous.map(|s| s.status.clone()).unwrap_or(Field::Failed {
        last: None,
        message: "尚未查询".into(),
    });
    let branches_prev = previous
        .map(|s| s.branches.clone())
        .unwrap_or(Field::Failed {
            last: None,
            message: "尚未查询".into(),
        });
    let remotes_prev = previous
        .map(|s| s.remotes.clone())
        .unwrap_or(Field::Failed {
            last: None,
            message: "尚未查询".into(),
        });

    let head = if valid.contains(Categories::HEAD) {
        match output.head {
            Some(result) => Field::from_result(result, &head_prev),
            None => head_prev,
        }
    } else {
        head_prev
    };
    let status = if valid.contains(Categories::STATUS) {
        match output.status {
            Some(result) => Field::from_result(result, &status_prev),
            None => status_prev,
        }
    } else {
        status_prev
    };
    let branches = if valid.contains(Categories::BRANCHES) {
        match output.branches {
            Some(result) => Field::from_result(result, &branches_prev),
            None => branches_prev,
        }
    } else {
        branches_prev
    };
    let remotes = if valid.contains(Categories::REMOTES) {
        match output.remotes {
            Some(result) => Field::from_result(result, &remotes_prev),
            None => remotes_prev,
        }
    } else {
        remotes_prev
    };

    WorkspaceSnapshot {
        identity,
        head,
        status,
        branches,
        remotes,
        generations,
        sampled_at,
        query_duration,
        identity_mismatch: None,
    }
}

/// 整次查询失败(discover/身份层):把这次请求的类别标记为失败,保留 last-good。
pub(crate) fn merge_failure(
    previous: Option<&WorkspaceSnapshot>,
    identity: RepoIdentity,
    categories: Categories,
    message: String,
    generations: crate::identity::Generations,
    sampled_at: Instant,
    query_duration: Duration,
) -> WorkspaceSnapshot {
    let mut snapshot = previous
        .cloned()
        .unwrap_or_else(|| WorkspaceSnapshot::empty(identity.clone(), message.clone(), sampled_at));
    snapshot.identity = identity;
    if categories.contains(Categories::HEAD) {
        snapshot.head = snapshot.head.fail(message.clone());
    }
    if categories.contains(Categories::STATUS) {
        snapshot.status = snapshot.status.fail(message.clone());
    }
    if categories.contains(Categories::BRANCHES) {
        snapshot.branches = snapshot.branches.fail(message.clone());
    }
    if categories.contains(Categories::REMOTES) {
        snapshot.remotes = snapshot.remotes.fail(message);
    }
    snapshot.generations = generations;
    snapshot.sampled_at = sampled_at;
    snapshot.query_duration = query_duration;
    snapshot.identity_mismatch = None;
    snapshot
}

/// 身份不一致:不应用新仓库数据,保留旧值并标记 stale。
pub(crate) fn merge_identity_mismatch(
    previous: Option<&WorkspaceSnapshot>,
    subscribed: RepoIdentity,
    observed: RepoIdentity,
    categories: Categories,
    generations: crate::identity::Generations,
    sampled_at: Instant,
    query_duration: Duration,
) -> WorkspaceSnapshot {
    let message = format!(
        "工作区身份已变化: {}",
        observed
            .workdir_root()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| observed.git_dir().display().to_string())
    );
    let mut snapshot = merge_failure(
        previous,
        subscribed,
        categories,
        message,
        generations,
        sampled_at,
        query_duration,
    );
    snapshot.identity_mismatch = Some(observed);
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_failure_keeps_the_last_ready_value() {
        let field = Field::Ready(42);
        let failed = field.fail("boom".into());
        assert_eq!(failed.value(), None);
        assert_eq!(failed.last(), Some(&42));
        assert_eq!(failed.error(), Some("boom"));
    }

    #[test]
    fn status_data_carries_dirty_and_entries() {
        let data = StatusData {
            entries: Vec::new(),
            dirty: false,
        };
        assert!(!data.dirty);
        assert!(data.entries.is_empty());
    }
}
