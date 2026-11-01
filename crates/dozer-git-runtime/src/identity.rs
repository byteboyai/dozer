//! 订阅身份与刷新类别。
//!
//! 这一层不依赖 UI / 产品模型:宿主给一个 `project_id` 与项目路径,运行时解析出
//! [`bytegit::WorkspaceKey`],再用 [`SubscriptionToken`] 绑定"这一次订阅"。

use bytegit::{WorkspaceChangeKind, WorkspaceKey};

/// 一次订阅的身份。`activation_epoch` 来自运行时进程级单调计数器:关闭重开、
/// A→B→A 或项目改路径都会换新值,旧结果不能冒充新订阅。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubscriptionToken {
    pub project_id: i64,
    pub activation_epoch: u64,
    pub workspace_key: WorkspaceKey,
}

/// 每个数据类别独立的 generation。请求失效时只推进受影响类别;查询结果按
/// 启动时捕获的值校验,不匹配的字段不应用。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Generations {
    pub head: u64,
    pub status: u64,
    pub branches: u64,
    pub remotes: u64,
}

impl Generations {
    pub fn get(self, category: Category) -> u64 {
        match category {
            Category::Head => self.head,
            Category::Status => self.status,
            Category::Branches => self.branches,
            Category::Remotes => self.remotes,
        }
    }

    pub(crate) fn bump(&mut self, categories: Categories) {
        if categories.contains(Categories::HEAD) {
            self.head = self.head.wrapping_add(1);
        }
        if categories.contains(Categories::STATUS) {
            self.status = self.status.wrapping_add(1);
        }
        if categories.contains(Categories::BRANCHES) {
            self.branches = self.branches.wrapping_add(1);
        }
        if categories.contains(Categories::REMOTES) {
            self.remotes = self.remotes.wrapping_add(1);
        }
    }
}

/// 单个数据类别,用于按类别比对 generation。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Head,
    Status,
    Branches,
    Remotes,
}

/// 一组数据类别。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Categories(u8);

impl Categories {
    pub const HEAD: Self = Self(1 << 0);
    pub const STATUS: Self = Self(1 << 1);
    pub const BRANCHES: Self = Self(1 << 2);
    pub const REMOTES: Self = Self(1 << 3);
    pub const ALL: Self = Self(Self::HEAD.0 | Self::STATUS.0 | Self::BRANCHES.0 | Self::REMOTES.0);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    /// 监听原因位 → 需要失效的数据类别。失效矩阵见设计规格 §6.2。
    ///
    /// `Refs` 先只验证 HEAD/branches;若 HEAD 真的变了,运行时在结果应用后
    /// 再补一次 status,避免每次 fetch 都重扫工作区。
    pub fn from_change(kinds: WorkspaceChangeKind) -> Self {
        use WorkspaceChangeKind as K;

        if kinds.contains(K::FULL_RESCAN) {
            return Self::ALL;
        }
        let mut out = Self::empty();
        if kinds.contains(K::WORKDIR) || kinds.contains(K::INDEX) {
            out.insert(Self::STATUS);
        }
        if kinds.contains(K::HEAD) {
            out.insert(Self::STATUS | Self::HEAD | Self::BRANCHES);
        }
        if kinds.contains(K::REFS) {
            out.insert(Self::HEAD | Self::BRANCHES);
        }
        if kinds.contains(K::CONFIG) {
            // 配置可能改 remote、ignore/attributes、branch 设置,保守全量读取。
            out.insert(Self::STATUS | Self::HEAD | Self::BRANCHES | Self::REMOTES);
        }
        if kinds.contains(K::HISTORY_BOUNDARY) {
            out.insert(Self::HEAD | Self::BRANCHES);
        }
        out
    }
}

impl std::ops::BitOr for Categories {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for Categories {
    fn bitor_assign(&mut self, rhs: Self) {
        self.insert(rhs);
    }
}

impl std::ops::BitAnd for Categories {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        self.intersection(rhs)
    }
}

/// 一次刷新请求的原因。手动刷新绕过缓存但仍与本工作区的同类任务合并。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshReason {
    Workdir,
    Index,
    Head,
    Refs,
    Config,
    HistoryBoundary,
    FullRescan,
    /// 订阅首次建立。
    Activated,
    /// 激活/重新聚焦时的重验。
    ActivatedValidation,
    /// 正常(60s)或降级(30s)补偿校验。
    Validation,
    Manual,
}

impl RefreshReason {
    pub fn categories(self) -> Categories {
        match self {
            Self::Workdir | Self::Index => Categories::STATUS,
            Self::Head => Categories::STATUS | Categories::HEAD | Categories::BRANCHES,
            Self::Refs => Categories::HEAD | Categories::BRANCHES,
            Self::Config => {
                Categories::STATUS | Categories::HEAD | Categories::BRANCHES | Categories::REMOTES
            }
            Self::HistoryBoundary => Categories::HEAD | Categories::BRANCHES,
            Self::FullRescan | Self::Activated | Self::ActivatedValidation | Self::Manual => {
                Categories::ALL
            }
            Self::Validation => Categories::ALL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_reasons_map_to_the_invalidation_matrix() {
        use WorkspaceChangeKind as K;
        assert_eq!(Categories::from_change(K::WORKDIR), Categories::STATUS);
        assert_eq!(Categories::from_change(K::INDEX), Categories::STATUS);
        assert_eq!(
            Categories::from_change(K::HEAD),
            Categories::STATUS | Categories::HEAD | Categories::BRANCHES
        );
        assert_eq!(
            Categories::from_change(K::REFS),
            Categories::HEAD | Categories::BRANCHES
        );
        assert_eq!(Categories::from_change(K::CONFIG), Categories::ALL);
        assert_eq!(
            Categories::from_change(K::HISTORY_BOUNDARY),
            Categories::HEAD | Categories::BRANCHES
        );
        assert_eq!(Categories::from_change(K::FULL_RESCAN), Categories::ALL);
        assert_eq!(
            Categories::from_change(K::WORKDIR | K::REFS),
            Categories::STATUS | Categories::HEAD | Categories::BRANCHES
        );
    }

    #[test]
    fn generations_advance_only_for_affected_categories() {
        let mut g = Generations::default();
        g.bump(Categories::STATUS);
        assert_eq!(g.status, 1);
        assert_eq!((g.head, g.branches, g.remotes), (0, 0, 0));
        g.bump(Categories::HEAD | Categories::BRANCHES);
        assert_eq!((g.head, g.status, g.branches), (1, 1, 1));
    }

    #[test]
    fn categories_intersection_is_a_mask() {
        let all = Categories::ALL;
        let mask = Categories::STATUS | Categories::HEAD;
        assert_eq!(all & mask, mask);
        assert!(mask.contains(Categories::STATUS));
        assert!(!mask.contains(Categories::REMOTES));
    }
}
