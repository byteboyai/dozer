//! 全局客户端能力快照(文件预览重构 Phase A)。
//!
//! Dozer 每次启动**只探测一次**硬件(`detect_hardware`,唯一碰 sysinfo 的地方),
//! 由纯函数 `estimate_capabilities` 换算成不可变的应用级预算。业务模块一律
//! 通过 `current()` / `Arc<ClientCapabilities>` 注入读取,不得再次调用 sysinfo。
//!
//! `full_file_load_bytes` 是旧 `native_editor::full_load_max_bytes()` 分档上限的
//! 适配值——公式一字未改,保证旧 editor 分档行为在迁移期完全不变。

use std::sync::{Arc, OnceLock};

pub use dozer_preview_core::capabilities::*;

dozer_core::scope!(LOG, module, "shell");

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

    #[test]
    fn available_memory_never_exceeds_total() {
        let raw = detect_hardware();
        assert!(raw.available_memory_at_start_bytes <= raw.total_memory_bytes);
    }
}
