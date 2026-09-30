//! 全局客户端能力快照(文件预览重构 Phase A)。
//!
//! Dozer 每次启动**只探测一次**硬件(`detect_hardware`,唯一碰 sysinfo 的地方),
//! 由纯函数 `estimate_capabilities` 换算成不可变的应用级预算。业务模块一律
//! 通过 `current()` / `Arc<ClientCapabilities>` 注入读取,不得再次调用 sysinfo。
//!
//! `full_file_load_bytes` 是旧 `native_editor::full_load_max_bytes()` 分档上限的
//! 适配值——公式一字未改,保证旧 editor 分档行为在迁移期完全不变。

use std::sync::{Arc, OnceLock};

dozer_core::scope!(LOG, module, "shell");

/// 启动时一次性探测到的硬件事实(纯数据,无策略)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct HardwareCapabilities {
    pub total_memory_bytes: u64,
    pub available_memory_at_start_bytes: u64,
    pub physical_cpu_count: usize,
    pub logical_cpu_count: usize,
}

/// 由硬件换算出的资源预算。公式是可测试默认值,不是永恒常量。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ResourceBudgets {
    /// 单个 CodeMirror/编辑器实例的驻留上限。
    pub single_editor_bytes: u64,
    /// 全部 preview viewer 的累计驻留上限。
    pub total_preview_bytes: u64,
    /// JSON Tree 后端预算。
    pub json_tree_bytes: u64,
    /// 旧 editor「整文件读取」分档上限的适配值(Phase A 行为不变)。
    pub full_file_load_bytes: u64,
    /// 同时存在的重型 WebView 数量上限。
    pub max_heavy_webviews: usize,
    /// 后台加载并行度上限。
    pub background_parallelism: usize,
}

/// 机器档位,便于诊断与后续按档调整策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum CapabilityTier {
    Low,
    Medium,
    High,
    Ultra,
}

/// 应用级不可变能力快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ClientCapabilities {
    pub hardware: HardwareCapabilities,
    pub budgets: ResourceBudgets,
    pub tier: CapabilityTier,
}

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * 1024 * 1024;

/// sysinfo 探测失败(极端环境)时的保守硬件默认值:按低配小机器处理,
/// 绝不因为探测失败就放开预算。
const CONSERVATIVE_TOTAL_MEMORY: u64 = 4 * GIB;
const CONSERVATIVE_AVAILABLE_MEMORY: u64 = 2 * GIB;

/// 探测本机硬件。**这是唯一调用 sysinfo 的地方**;失败时退回保守默认值并
/// 记 warning,不 panic、不阻塞启动。
pub fn detect_hardware() -> HardwareCapabilities {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.refresh_cpu_all();

    let total = sys.total_memory();
    let available = sys.available_memory();
    let logical = sys.cpus().len();
    let physical = sys.physical_core_count().unwrap_or(0);

    let (total, available, flagged) = if total == 0 {
        (
            CONSERVATIVE_TOTAL_MEMORY,
            CONSERVATIVE_AVAILABLE_MEMORY,
            true,
        )
    } else {
        if available == 0 {
            dozer_core::log_warn!(LOG, "系统可用内存探测为 0,按保守值处理");
        }
        (total, available, false)
    };
    if flagged {
        dozer_core::log_warn!(LOG, "系统总内存探测失败,按保守默认值 (4GiB) 处理");
    }

    HardwareCapabilities {
        total_memory_bytes: total,
        available_memory_at_start_bytes: available.min(total),
        // CPU 数为 0(极少见的探测失败)时按单核处理,保证下游不会除零/退化成
        // 0 并行度。
        physical_cpu_count: physical.max(1),
        logical_cpu_count: logical.max(1),
    }
}

/// 纯预算换算:不读环境、不读全局状态,只依赖入参——可对任意机器组合做
/// 表驱动测试,不需要真实硬件。
pub fn estimate_capabilities(raw: HardwareCapabilities) -> ClientCapabilities {
    let total = raw.total_memory_bytes as u128;
    let available = raw.available_memory_at_start_bytes as u128;

    // single_editor = clamp(min(total * 8%, available * 20%), 64MiB, 512MiB)
    let single = clamp_u64(
        min_u128(total * 8 / 100, available * 20 / 100),
        64 * MIB,
        512 * MIB,
    );
    // total_preview = clamp(min(total * 15%, available * 35%), 128MiB, 1.5GiB)
    let total_preview = clamp_u64(
        min_u128(total * 15 / 100, available * 35 / 100),
        128 * MIB,
        1536 * MIB,
    );
    // json_tree = clamp(single_editor / 2, 32MiB, 256MiB)
    let json_tree = clamp_u64((single / 2) as u128, 32 * MIB, 256 * MIB);
    // full_file_load: 旧 `full_load_max_bytes_for` 的公式,迁移期适配值。
    let full_file_load = full_file_load_bytes_for(raw.total_memory_bytes);

    // 重型 WebView 数与后台并行度按 CPU 缩放但有上下限:数量不是内存预算的
    // 替代品,二者需同时满足。
    let max_heavy_webviews = (raw.logical_cpu_count / 4).clamp(2, 6);
    let background_parallelism = raw.physical_cpu_count.clamp(1, 4);

    ClientCapabilities {
        budgets: ResourceBudgets {
            single_editor_bytes: single,
            total_preview_bytes: total_preview,
            json_tree_bytes: json_tree,
            full_file_load_bytes: full_file_load,
            max_heavy_webviews,
            background_parallelism,
        },
        tier: tier_for(raw),
        hardware: raw,
    }
}

/// 由总内存粗分档(单调:总内存越大档位不降)。
fn tier_for(raw: HardwareCapabilities) -> CapabilityTier {
    match raw.total_memory_bytes {
        t if t < 8 * GIB => CapabilityTier::Low,
        t if t < 20 * GIB => CapabilityTier::Medium,
        t if t < 48 * GIB => CapabilityTier::High,
        _ => CapabilityTier::Ultra,
    }
}

/// 旧 `native_editor::full_load_max_bytes_for` 的公式原样搬来(总内存 10%
/// ÷ 3,钳到 [256MB, 4GB]),保留为纯函数供两处共用与单测。
pub fn full_file_load_bytes_for(total_ram_bytes: u64) -> u64 {
    ((total_ram_bytes as f64 * 0.10 / 3.0) as u64).clamp(256 * MIB, 4 * GIB)
}

fn min_u128(a: u128, b: u128) -> u128 {
    if a < b { a } else { b }
}

/// u128 结果钳进 [lo, hi] 后收回 u64(入参已保证 hi 是常量,不会溢出)。
fn clamp_u64(value: u128, lo: u64, hi: u64) -> u64 {
    (value.clamp(lo as u128, hi as u128)) as u64
}

static INSTALLED: OnceLock<Arc<ClientCapabilities>> = OnceLock::new();

/// 安装本次启动的能力快照。重复安装只保留第一次(启动只探测一次)。
pub fn install(caps: ClientCapabilities) -> Arc<ClientCapabilities> {
    INSTALLED.get_or_init(|| Arc::new(caps)).clone()
}

/// 取当前能力快照。未被显式安装时(测试 / 极端启动路径)按需探测一次并安装
/// ——依然"一进程只探测一次"。
pub fn current() -> Arc<ClientCapabilities> {
    INSTALLED
        .get_or_init(|| Arc::new(estimate_capabilities(detect_hardware())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(total_gib: u64, avail_gib: u64, physical: usize, logical: usize) -> HardwareCapabilities {
        HardwareCapabilities {
            total_memory_bytes: total_gib * GIB,
            available_memory_at_start_bytes: avail_gib * GIB,
            physical_cpu_count: physical,
            logical_cpu_count: logical,
        }
    }

    #[test]
    fn floors_on_tiny_machines() {
        // 512MiB 总内存的小机器,预算被下限托住。
        let caps = estimate_capabilities(HardwareCapabilities {
            total_memory_bytes: 512 * MIB,
            available_memory_at_start_bytes: 256 * MIB,
            physical_cpu_count: 1,
            logical_cpu_count: 2,
        });
        assert_eq!(caps.budgets.single_editor_bytes, 64 * MIB);
        assert_eq!(caps.budgets.total_preview_bytes, 128 * MIB);
        assert_eq!(caps.budgets.json_tree_bytes, 32 * MIB);
        assert_eq!(caps.tier, CapabilityTier::Low);
    }

    #[test]
    fn ceilings_on_huge_machines() {
        let caps = estimate_capabilities(hw(64, 64, 16, 32));
        assert_eq!(caps.budgets.single_editor_bytes, 512 * MIB);
        assert_eq!(caps.budgets.total_preview_bytes, 1536 * MIB);
        assert_eq!(caps.budgets.json_tree_bytes, 256 * MIB);
        assert_eq!(caps.tier, CapabilityTier::Ultra);
    }

    #[test]
    fn mid_range_is_between_floor_and_ceiling() {
        // 总内存大但可用内存中等:单编辑器预算落在上下限之间。
        let caps = estimate_capabilities(hw(16, 2, 8, 16));
        assert!(caps.budgets.single_editor_bytes > 64 * MIB);
        assert!(caps.budgets.single_editor_bytes < 512 * MIB);
        assert_eq!(caps.tier, CapabilityTier::Medium);
    }

    #[test]
    fn low_available_memory_pulls_budget_to_floor() {
        // 总内存很大、可用内存极低:预算应被 available 项拉回下限。
        let caps = estimate_capabilities(hw(64, 0, 8, 16));
        assert_eq!(caps.budgets.single_editor_bytes, 64 * MIB);
        assert_eq!(caps.budgets.total_preview_bytes, 128 * MIB);
    }

    #[test]
    fn budgets_are_monotonic_in_total_memory() {
        let mut prev = estimate_capabilities(hw(4, 4, 4, 4)).budgets;
        for gib in [8u64, 16, 32, 64] {
            let next = estimate_capabilities(hw(gib, gib, 8, 16)).budgets;
            assert!(next.single_editor_bytes >= prev.single_editor_bytes);
            assert!(next.total_preview_bytes >= prev.total_preview_bytes);
            assert!(next.json_tree_bytes >= prev.json_tree_bytes);
            assert!(next.full_file_load_bytes >= prev.full_file_load_bytes);
            prev = next;
        }
    }

    #[test]
    fn cpu_zero_is_normalized_to_one() {
        let raw = HardwareCapabilities {
            total_memory_bytes: 8 * GIB,
            available_memory_at_start_bytes: 8 * GIB,
            physical_cpu_count: 0,
            logical_cpu_count: 0,
        };
        let caps = estimate_capabilities(raw);
        assert!(caps.budgets.background_parallelism >= 1);
        assert!(caps.budgets.max_heavy_webviews >= 2);
    }

    #[test]
    fn heavy_webview_count_scales_and_clamps() {
        assert_eq!(
            estimate_capabilities(hw(8, 8, 2, 4))
                .budgets
                .max_heavy_webviews,
            2
        );
        assert_eq!(
            estimate_capabilities(hw(16, 16, 8, 16))
                .budgets
                .max_heavy_webviews,
            4
        );
        assert_eq!(
            estimate_capabilities(hw(64, 64, 16, 64))
                .budgets
                .max_heavy_webviews,
            6
        );
    }

    #[test]
    fn full_file_load_matches_legacy_formula() {
        // 与 native_editor 旧测试保持同一组期望值,迁移期行为不变的契约。
        assert_eq!(full_file_load_bytes_for(4 * GIB), 256 * MIB);
        assert_eq!(full_file_load_bytes_for(128 * GIB), 4 * GIB);
        assert_eq!(full_file_load_bytes_for(32 * GIB), 1_145_324_612);
    }

    #[test]
    fn available_memory_never_exceeds_total() {
        let raw = detect_hardware();
        assert!(raw.available_memory_at_start_bytes <= raw.total_memory_bytes);
    }
}
