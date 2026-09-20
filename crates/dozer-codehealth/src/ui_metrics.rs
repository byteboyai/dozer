use crate::report::HealthTier;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawLiteralFinding {
    pub file: PathBuf,
    pub line: usize,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DuplicateCluster {
    pub occurrences: Vec<(PathBuf, usize)>,
    pub node_count: usize,
}

/// spec「颜色硬编码」：`0` Healthy，`1..=15` Watch，`>15` Critical。
pub fn color_tier(count: usize) -> HealthTier {
    if count > 15 {
        HealthTier::Critical
    } else if count >= 1 {
        HealthTier::Watch
    } else {
        HealthTier::Healthy
    }
}

/// spec「边距硬编码」：`0..=10` Healthy，`11..=50` Watch，`>50` Critical。
pub fn spacing_tier(count: usize) -> HealthTier {
    if count > 50 {
        HealthTier::Critical
    } else if count >= 11 {
        HealthTier::Watch
    } else {
        HealthTier::Healthy
    }
}

/// spec「字体硬编码」：`0` Healthy，`1..=5` Watch，`>5` Critical。
pub fn font_tier(count: usize) -> HealthTier {
    if count > 5 {
        HealthTier::Critical
    } else if count >= 1 {
        HealthTier::Watch
    } else {
        HealthTier::Healthy
    }
}

/// spec「组件树嵌套深度」：`<=2` Healthy，`3..=4` Watch，`>4` Critical。
pub fn nesting_depth_tier(depth: usize) -> HealthTier {
    if depth > 4 {
        HealthTier::Critical
    } else if depth >= 3 {
        HealthTier::Watch
    } else {
        HealthTier::Healthy
    }
}

/// spec「事件回调密度」：`0..=3` Healthy，`4..=6` Watch，`>6` Critical。
pub fn event_handler_tier(count: usize) -> HealthTier {
    if count > 6 {
        HealthTier::Critical
    } else if count >= 4 {
        HealthTier::Watch
    } else {
        HealthTier::Healthy
    }
}

/// spec「组件化重复结构」：`3..=5` Watch，`>5` Critical。调用方保证只对
/// `occurrences >= 3`（已成簇）的情况调用，`<3` 不构成簇、不会进这个函数。
pub fn cluster_tier(occurrences: usize) -> HealthTier {
    if occurrences > 5 {
        HealthTier::Critical
    } else {
        HealthTier::Watch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_tier_boundaries() {
        assert_eq!(color_tier(0), HealthTier::Healthy);
        assert_eq!(color_tier(1), HealthTier::Watch);
        assert_eq!(color_tier(15), HealthTier::Watch);
        assert_eq!(color_tier(16), HealthTier::Critical);
    }

    #[test]
    fn spacing_tier_boundaries() {
        assert_eq!(spacing_tier(10), HealthTier::Healthy);
        assert_eq!(spacing_tier(11), HealthTier::Watch);
        assert_eq!(spacing_tier(50), HealthTier::Watch);
        assert_eq!(spacing_tier(51), HealthTier::Critical);
    }

    #[test]
    fn font_tier_boundaries() {
        assert_eq!(font_tier(0), HealthTier::Healthy);
        assert_eq!(font_tier(1), HealthTier::Watch);
        assert_eq!(font_tier(5), HealthTier::Watch);
        assert_eq!(font_tier(6), HealthTier::Critical);
    }

    #[test]
    fn nesting_depth_tier_boundaries() {
        assert_eq!(nesting_depth_tier(2), HealthTier::Healthy);
        assert_eq!(nesting_depth_tier(3), HealthTier::Watch);
        assert_eq!(nesting_depth_tier(4), HealthTier::Watch);
        assert_eq!(nesting_depth_tier(5), HealthTier::Critical);
    }

    #[test]
    fn event_handler_tier_boundaries() {
        assert_eq!(event_handler_tier(3), HealthTier::Healthy);
        assert_eq!(event_handler_tier(4), HealthTier::Watch);
        assert_eq!(event_handler_tier(6), HealthTier::Watch);
        assert_eq!(event_handler_tier(7), HealthTier::Critical);
    }

    #[test]
    fn cluster_tier_boundaries() {
        assert_eq!(cluster_tier(3), HealthTier::Watch);
        assert_eq!(cluster_tier(5), HealthTier::Watch);
        assert_eq!(cluster_tier(6), HealthTier::Critical);
    }
}
