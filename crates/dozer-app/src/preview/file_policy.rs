//! 文本文件策略决策器(文件预览重构 Phase C Task 1)。
//!
//! 由 [`FileProfile`] + 全局 [`ResourceBudgets`] **纯函数**决定一个文本文件
//! 走哪一档:可编辑 CodeMirror / 只读高亮 / 只读纯文本 / 窗口化。系数与绝对
//! 护栏是可测默认值(见规格 §"文件画像与能力降级"),不是永恒常量。
//!
//! 决策必须可解释:返回的 `TextPolicyDecision` 带具体文件画像、预算与降级项,
//! 不允许只回一个"大文件"的 bool。用户的单次强制尝试只影响本次,不改全局预算
//! (语义由调用方保证——本函数是纯函数,不持有预算)。

// Phase C 建立的策略/资源/流式模块,消费方(Windowed viewer、资源接线)接入前
// 部分 API 暂未被非测试代码调用;显式允许,避免 dead_code 噪声。
#![allow(dead_code)]

use crate::capabilities::ResourceBudgets;
use crate::preview::file_profile::FileProfile;

/// 文本文件四档策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFilePolicy {
    /// 工作集在预算内且 <= 30MiB:完整可编辑 CodeMirror。
    EditableCode,
    /// 只读但保留语法高亮:<= 64MiB 且高亮估算在预算内。
    ReadOnlyHighlighted,
    /// 只读纯文本(关闭高亮/折叠):<= 128MiB 且纯文本估算在预算内。
    ReadOnlyPlain,
    /// 更大/超预算:Rust 窗口化/流式查看。
    Windowed,
}

const MIB: u64 = 1024 * 1024;

/// 估算系数(工作集相对文件字节的倍数)。
pub const EDITABLE_FACTOR: u64 = 9;
pub const HIGHLIGHT_FACTOR: u64 = 6;
pub const PLAIN_FACTOR: u64 = 4;

/// 绝对护栏(MiB)。
pub const EDITABLE_MAX_BYTES: u64 = 30 * MIB;
pub const HIGHLIGHT_MAX_BYTES: u64 = 64 * MIB;
pub const PLAIN_MAX_BYTES: u64 = 128 * MIB;

/// 单行规则阈值。
pub const WRAP_OFF_LINE_BYTES: u64 = 100 * 1024;
pub const HIGHLIGHT_OFF_LINE_BYTES: u64 = MIB;
pub const FORCE_WINDOWED_LINE_BYTES: u64 = 5 * MIB;

/// 一档策略下的能力开关(由文件画像进一步收窄)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextCapabilities {
    pub editable: bool,
    pub wrap: bool,
    pub highlight: bool,
    pub fold: bool,
}

/// 决策结果,带可展示的降级说明与估算值。
#[derive(Debug, Clone, PartialEq)]
pub struct TextPolicyDecision {
    pub policy: TextFilePolicy,
    pub capabilities: TextCapabilities,
    /// 该档的工作集估算字节数。
    pub estimated_bytes: u64,
    /// 命中的绝对护栏(字节),窗口化时为 `None`(无护栏,直接流式)。
    pub guard_bytes: Option<u64>,
    /// 人类可读的降级说明(含文件大小/最长行/预算)。
    pub reason: String,
}

impl TextPolicyDecision {
    pub fn is_windowed(&self) -> bool {
        self.policy == TextFilePolicy::Windowed
    }
}

/// 纯决策:文件画像 + 预算 → 策略。`size` 用 `u64::saturating_mul`,不溢出。
pub fn decide_text_policy(profile: &FileProfile, budgets: &ResourceBudgets) -> TextPolicyDecision {
    let size = profile.size_bytes;
    let max_line = profile.sampled_max_line_bytes as u64;
    let budget = budgets.single_editor_bytes;

    let editable_est = size.saturating_mul(EDITABLE_FACTOR);
    let highlight_est = size.saturating_mul(HIGHLIGHT_FACTOR);
    let plain_est = size.saturating_mul(PLAIN_FACTOR);

    // 1) 超长单行的硬规则优先:>5MiB 直接窗口化(无论文件多大)。
    if max_line > FORCE_WINDOWED_LINE_BYTES {
        return windowed(
            plain_est,
            format!(
                "最长行 {} 超过 5MiB,强制窗口化(文件 {} 字节)",
                max_line, size
            ),
            max_line,
        );
    }

    // 2) 逐档判定,取第一档同时满足"绝对护栏"与"工作集预算"的。
    let (policy, estimated, guard, base_reason) =
        if size <= EDITABLE_MAX_BYTES && editable_est <= budget {
            (
                TextFilePolicy::EditableCode,
                editable_est,
                EDITABLE_MAX_BYTES,
                "可编辑工作集在预算内且 <= 30MiB",
            )
        } else if size <= HIGHLIGHT_MAX_BYTES && highlight_est <= budget {
            (
                TextFilePolicy::ReadOnlyHighlighted,
                highlight_est,
                HIGHLIGHT_MAX_BYTES,
                "可编辑超预算/超 30MiB,只读高亮在预算内且 <= 64MiB",
            )
        } else if size <= PLAIN_MAX_BYTES && plain_est <= budget {
            (
                TextFilePolicy::ReadOnlyPlain,
                plain_est,
                PLAIN_MAX_BYTES,
                "高亮超预算/超 64MiB,只读纯文本在预算内且 <= 128MiB",
            )
        } else {
            return windowed(
                plain_est,
                format!(
                    "文件 {} 字节或工作集超过单编辑器预算 {} 字节,窗口化",
                    size, budget
                ),
                max_line,
            );
        };

    let editable = policy == TextFilePolicy::EditableCode;
    let highlight = matches!(
        policy,
        TextFilePolicy::EditableCode | TextFilePolicy::ReadOnlyHighlighted
    ) && max_line <= HIGHLIGHT_OFF_LINE_BYTES;
    let fold = highlight;
    let wrap = max_line <= WRAP_OFF_LINE_BYTES;

    let mut reason = format!("{base_reason}(文件 {size} 字节, 最长行 {max_line} 字节)");
    if !wrap {
        reason.push_str(";最长行 >100KiB,关闭自动换行");
    }
    if !highlight {
        reason.push_str(";最长行 >1MiB,关闭高亮/折叠");
    }

    TextPolicyDecision {
        policy,
        capabilities: TextCapabilities {
            editable,
            wrap,
            highlight,
            fold,
        },
        estimated_bytes: estimated,
        guard_bytes: Some(guard),
        reason,
    }
}

fn windowed(estimated: u64, reason: String, max_line: u64) -> TextPolicyDecision {
    TextPolicyDecision {
        policy: TextFilePolicy::Windowed,
        capabilities: TextCapabilities {
            editable: false,
            wrap: max_line <= WRAP_OFF_LINE_BYTES,
            highlight: false,
            fold: false,
        },
        estimated_bytes: estimated,
        guard_bytes: None,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::file_profile::{
        ContentKind, FileProfile, LineEnding, TextEncoding, Utf8Status,
    };
    use std::time::SystemTime;

    fn budget(single_editor: u64) -> ResourceBudgets {
        ResourceBudgets {
            single_editor_bytes: single_editor,
            total_preview_bytes: single_editor * 2,
            json_tree_bytes: single_editor / 2,
            full_file_load_bytes: single_editor,
            max_heavy_webviews: 4,
            background_parallelism: 2,
        }
    }

    fn profile(size: u64, max_line: usize) -> FileProfile {
        FileProfile {
            size_bytes: size,
            sampled_line_count: None,
            sampled_max_line_bytes: max_line,
            utf8: Utf8Status::Valid,
            content_kind: ContentKind::Text,
            modified: None::<SystemTime>,
            encoding: TextEncoding::Utf8,
            line_ending: LineEnding::Lf,
            has_bom: false,
        }
    }

    #[test]
    fn small_file_editable_on_normal_budget() {
        let d = decide_text_policy(&profile(MIB, 80), &budget(512 * MIB));
        assert_eq!(d.policy, TextFilePolicy::EditableCode);
        assert!(d.capabilities.editable && d.capabilities.wrap && d.capabilities.highlight);
        assert!(!d.reason.is_empty());
    }

    #[test]
    fn low_budget_pushes_to_read_only_plain_or_windowed() {
        // 单编辑器预算只有 64MiB:20MiB 文件 → 可编辑估算 180MiB 超预算,
        // 高亮 120MiB 也超,纯文本 80MiB 仍超 → 窗口化。
        let d = decide_text_policy(&profile(20 * MIB, 80), &budget(64 * MIB));
        assert_eq!(d.policy, TextFilePolicy::Windowed);
        assert!(!d.capabilities.editable);
    }

    #[test]
    fn boundary_editable_max_is_inclusive() {
        let b = budget(512 * MIB);
        let at = decide_text_policy(&profile(EDITABLE_MAX_BYTES, 80), &b);
        assert_eq!(at.policy, TextFilePolicy::EditableCode);
        let over = decide_text_policy(&profile(EDITABLE_MAX_BYTES + 1, 80), &b);
        assert!(matches!(
            over.policy,
            TextFilePolicy::ReadOnlyHighlighted | TextFilePolicy::ReadOnlyPlain
        ));
    }

    #[test]
    fn boundary_highlight_to_plain() {
        let b = budget(512 * MIB);
        let at = decide_text_policy(&profile(HIGHLIGHT_MAX_BYTES, 80), &b);
        assert_eq!(at.policy, TextFilePolicy::ReadOnlyHighlighted);
        let over = decide_text_policy(&profile(HIGHLIGHT_MAX_BYTES + 1, 80), &b);
        assert_eq!(over.policy, TextFilePolicy::ReadOnlyPlain);
    }

    #[test]
    fn boundary_plain_to_windowed() {
        let b = budget(1024 * MIB);
        let at = decide_text_policy(&profile(PLAIN_MAX_BYTES, 80), &b);
        assert_eq!(at.policy, TextFilePolicy::ReadOnlyPlain);
        let over = decide_text_policy(&profile(PLAIN_MAX_BYTES + 1, 80), &b);
        assert_eq!(over.policy, TextFilePolicy::Windowed);
    }

    #[test]
    fn long_line_disables_wrap_then_highlight_then_forces_windowed() {
        let b = budget(1024 * MIB);
        let wrap_off = decide_text_policy(&profile(MIB, (WRAP_OFF_LINE_BYTES + 1) as usize), &b);
        assert!(!wrap_off.capabilities.wrap);
        assert!(wrap_off.capabilities.highlight);

        let hl_off = decide_text_policy(&profile(MIB, (HIGHLIGHT_OFF_LINE_BYTES + 1) as usize), &b);
        assert!(!hl_off.capabilities.highlight && !hl_off.capabilities.fold);
        assert!(!hl_off.capabilities.wrap);

        let forced =
            decide_text_policy(&profile(MIB, (FORCE_WINDOWED_LINE_BYTES + 1) as usize), &b);
        assert_eq!(forced.policy, TextFilePolicy::Windowed);
    }

    #[test]
    fn overflow_is_saturating_not_wrapping() {
        // 极大 size * 9 不应溢出回绕成小值而误判为可编辑。
        let d = decide_text_policy(&profile(u64::MAX / 2, 80), &budget(4096 * MIB));
        assert_eq!(d.policy, TextFilePolicy::Windowed);
    }

    #[test]
    fn non_text_profile_still_decides_by_size() {
        // 决策器只按 size/line;binary/非 UTF-8 的只读化在路由层另行处理。
        let mut p = profile(1000, 40);
        p.content_kind = ContentKind::Binary;
        p.utf8 = Utf8Status::Invalid;
        let d = decide_text_policy(&p, &budget(64 * MIB));
        assert_eq!(d.policy, TextFilePolicy::EditableCode);
    }

    #[test]
    fn high_budget_stays_editable_further() {
        let small = budget(512 * MIB);
        let big = budget(4096 * MIB);
        // 固定文件在两档预算下不降级。
        let f = profile(20 * MIB, 80);
        assert_eq!(
            decide_text_policy(&f, &small).policy,
            decide_text_policy(&f, &big).policy
        );
    }
}
