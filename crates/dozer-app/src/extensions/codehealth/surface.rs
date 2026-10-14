//! 代码健康度内容侧 webview 接入共享宿主 Surface 执行器(Task 09 / 规格 §5.4)。
//!
//! 与 `extensions::usage::UsageSurface` 同构:把"webview 生命周期 + ready 归属
//! + 旧代 IPC 丢弃 + 逐帧 desired 推送去重"这层通用机制收进 `bytehost-desktop`
//! 的 `SurfaceExecutor`,CodeHealth 只提供载荷与产品语义。
//!
//! - **身份**:固定单槽 `(instance, SurfaceSlot(CODEHEALTH_CONTENT_ID_OFFSET),
//!   generation)`。每次原生实例销毁重建递增 generation,旧代 ready/IPC 一律被
//!   路由层拒绝。
//! - **业务字段留在面板**:`READY_TIMEOUT`/`failed`(加载超时 → 回落原生占位)
//!   是**面板级**降级,不进 Surface,仍由 `WebviewPushState` 持有并在
//!   `take_codehealth_content_script` 里早退(见 api-decisions §9.4)。
//! - **`revision` 由本层持有且单调递增**:envelope 的 `revision` 语义是"前端丢弃
//!   小于已应用值的推送"。新 host 实例的 JS 计数从零开始,持续递增的 `revision`
//!   对其恒为"大于已应用值",可安全接受,故**不**随 generation 归零。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytehost_core::{PanelInstanceId, SurfaceEvent, SurfaceId, SurfaceKind, SurfaceSlot};
use bytehost_desktop::surface::SurfaceExecutor;

use super::protocol::{CodeHealthViewPayload, encode_codehealth_push};

/// 推送载荷 = 业务 payload + 语言上下文(spec §8)。把语言上下文并入 `T` 使
/// Surface 的逐帧去重把 locale 也纳入比较:"业务数据不变、仅语言切换"同样触发
/// 一次推送。载荷 → 脚本的转换从 `Stamped` 同时取 stamp 与 payload。
type CodeHealthStamped = dozer_model::Stamped<CodeHealthViewPayload>;

/// 代码健康度内容侧固定单槽。物理槽 id 复用既有 `CODEHEALTH_CONTENT_ID_OFFSET`,
/// 确保 webview 池、`runtime.rs` IPC 与 Surface 路由三者指向同一槽。
pub(crate) const CODEHEALTH_SURFACE_SLOT: u32 = crate::app::CODEHEALTH_CONTENT_ID_OFFSET as u32;

/// 代码健康度内容侧 webview 的 Surface 管理器。
pub(crate) struct CodeHealthSurface {
    executor: SurfaceExecutor<CodeHealthStamped>,
    instance: PanelInstanceId,
    slot: SurfaceSlot,
    generation: u64,
    /// envelope 的单调 `revision`(从 1 起)。由脚本构造闭包读取并自增。
    revision: Arc<AtomicU64>,
}

impl Default for CodeHealthSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeHealthSurface {
    pub(crate) fn new() -> Self {
        let instance = PanelInstanceId::new(0, 0);
        let slot = SurfaceSlot(CODEHEALTH_SURFACE_SLOT);
        let revision = Arc::new(AtomicU64::new(0));
        let mut executor = SurfaceExecutor::new();
        executor.register(
            surface_id(instance, slot, 0),
            SurfaceKind::Builtin,
            slot,
            build_script(Arc::clone(&revision)),
        );
        Self {
            executor,
            instance,
            slot,
            generation: 0,
            revision,
        }
    }

    /// 当前代数的稳定身份(IPC / ready 归属校验用)。
    pub(crate) fn id(&self) -> SurfaceId {
        surface_id(self.instance, self.slot, self.generation)
    }

    /// 每帧根据"原生实例是否在池中"推进生命周期。语义同 `UsageSurface`。
    pub(crate) fn observe_instance(&mut self, in_pool: bool, native_replaced: bool) {
        if native_replaced {
            let id = self.id();
            let has_instance = self
                .executor
                .state(id)
                .is_some_and(bytehost_core::SurfaceState::has_instance);
            if has_instance {
                self.executor.apply(id, SurfaceEvent::Destroyed);
                self.rebuild();
            }
        }
        let id = self.id();
        if in_pool {
            let state = self.executor.state(id).cloned();
            let needs_create = matches!(
                state,
                Some(
                    bytehost_core::SurfaceState::Absent
                        | bytehost_core::SurfaceState::Destroyed
                        | bytehost_core::SurfaceState::Failed { .. }
                ) | None
            );
            if needs_create {
                self.executor.begin_create(id);
                self.executor.apply(id, SurfaceEvent::Created);
            }
        } else {
            let has_instance = self
                .executor
                .state(id)
                .is_some_and(bytehost_core::SurfaceState::has_instance);
            if has_instance {
                self.executor.apply(id, SurfaceEvent::Destroyed);
                self.rebuild();
            }
        }
    }

    /// 收到 webview 的 ready 握手(仅内置面板走这条)。
    pub(crate) fn apply_ready(&mut self) {
        let id = self.id();
        self.executor.apply(id, SurfaceEvent::Ready);
    }

    /// 更新期望载荷(产品每帧算出新状态都调用)。载荷构造留在面板。
    pub(crate) fn set_desired(&mut self, payload: CodeHealthStamped) {
        let id = self.id();
        self.executor.set_desired(id, payload);
    }

    /// 待推送脚本:Ready 且与上次送达不同才返回,附带物理槽 id,并写回
    /// 送达标记(与 `UsageSurface::take_pending_script` 同一契约)。envelope 的
    /// `revision` 在脚本构造闭包里自增,故本层无需二次记账。
    pub(crate) fn take_pending_script(&mut self) -> Option<(usize, String)> {
        let id = self.id();
        let payload = self.executor.pending(id)?.clone();
        let script = self.executor.script_for(id, &payload)?;
        self.executor.mark_sent(id, payload);
        Some((CODEHEALTH_SURFACE_SLOT as usize, script))
    }

    /// 给定一个身份,它是否被当前路由层接受(IPC 归属校验用)。
    pub(crate) fn accepts(&self, id: Option<SurfaceId>) -> bool {
        id.is_some_and(|id| self.executor.route(id).is_accepted())
    }

    /// 当前是否已握手就绪(加载超时判定的起点参考)。
    pub(crate) fn is_ready(&self) -> bool {
        matches!(
            self.executor.state(self.id()),
            Some(bytehost_core::SurfaceState::Ready)
        )
    }

    /// 当前已发出的最大 revision(测试 / 诊断用)。
    #[cfg(test)]
    pub(crate) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    /// 校验指定身份的归属(测试断言旧代数被拒用)。
    #[cfg(test)]
    pub(crate) fn route_of(&self, id: SurfaceId) -> bytehost_desktop::surface::Routed {
        self.executor.route(id)
    }

    fn rebuild(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let id = self.id();
        self.executor.register(
            id,
            SurfaceKind::Builtin,
            self.slot,
            build_script(Arc::clone(&self.revision)),
        );
    }
}

fn build_script(
    revision: Arc<AtomicU64>,
) -> bytehost_desktop::surface::ScriptBuilder<CodeHealthStamped> {
    Box::new(move |stamped: &CodeHealthStamped| {
        let rev = revision.fetch_add(1, Ordering::Relaxed) + 1;
        let (locale, payload) = stamped.clone().into_parts();
        crate::preview::dispatch_script(&encode_codehealth_push(rev, Some(locale), payload))
    })
}

fn surface_id(instance: PanelInstanceId, slot: SurfaceSlot, generation: u64) -> SurfaceId {
    SurfaceId::new(instance, slot, bytehost_core::Generation::new(generation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::codehealth::current_view_payload;

    fn payload() -> CodeHealthStamped {
        let p = current_view_payload(&crate::extensions::codehealth::WorkspaceState::default());
        dozer_model::Stamped::new(stamp(), p)
    }

    fn stamp() -> dozer_model::LocaleStamp {
        dozer_model::LocaleStamp {
            ui_locale: dozer_model::Locale::default_locale(),
            locale_revision: 0,
        }
    }

    #[test]
    fn pending_push_requires_ready_and_changes() {
        let mut s = CodeHealthSurface::new();
        s.observe_instance(true, false);
        s.set_desired(payload());
        assert!(s.take_pending_script().is_none(), "未 ready 不推");
        s.apply_ready();
        assert!(s.take_pending_script().is_some(), "ready 首推");
        // 同一 desired 再取:已送达,去重。
        s.set_desired(payload());
        assert!(s.take_pending_script().is_none(), "同值不重推");
    }

    /// webview 重建后新实例第一次 ready 必须强制重发(即便 desired 未变)。
    #[test]
    fn rebuild_forces_a_resend_even_when_desired_unchanged() {
        let mut s = CodeHealthSurface::new();
        s.observe_instance(true, false);
        s.apply_ready();
        s.set_desired(payload());
        assert!(s.take_pending_script().is_some());
        s.observe_instance(false, false); // 离池 → Destroyed + 递增代数
        s.observe_instance(true, false); // 重新进池 → 新实例
        s.apply_ready();
        s.set_desired(payload());
        assert!(s.take_pending_script().is_some(), "新实例必须重发");
    }

    /// R1 同型:同帧"销毁+重建"必须开新代数,旧实例 ready/已送达不继承。
    #[test]
    fn same_frame_destroy_and_recreate_opens_a_new_generation() {
        let mut s = CodeHealthSurface::new();
        s.observe_instance(true, false);
        s.apply_ready();
        s.set_desired(payload());
        assert!(s.take_pending_script().is_some(), "旧实例已推快照");
        let old = s.id();
        s.observe_instance(true, true);
        assert!(
            !s.route_of(old).is_accepted(),
            "旧实例的 ready/IPC 必须被拒"
        );
        assert_ne!(s.id(), old, "原生实例替换必须换代数");
        assert!(s.take_pending_script().is_none(), "新实例尚未握手,不得推");
        s.apply_ready();
        s.set_desired(payload());
        assert!(s.take_pending_script().is_some(), "新实例握手后重发");
    }

    /// 旧代数的 IPC 在 webview 重建后被路由层拒绝。
    #[test]
    fn stale_generation_ipc_is_rejected_after_rebuild() {
        let mut s = CodeHealthSurface::new();
        s.observe_instance(true, false);
        s.apply_ready();
        let old = s.id();
        s.observe_instance(false, false);
        s.observe_instance(true, false);
        assert!(!s.route_of(old).is_accepted(), "旧代数归属被拒");
        assert!(s.route_of(s.id()).is_accepted(), "当前代数接受");
        assert!(!s.accepts(Some(old)), "旧代数 accepts=false");
        assert!(!s.accepts(None), "无身份 accepts=false");
        assert!(s.accepts(Some(s.id())), "当前代数 accepts=true");
    }

    /// envelope 的 revision 单调递增(跨重建不归零;新 host 计数从零,恒接受)。
    #[test]
    fn revision_is_monotonic_across_rebuilds() {
        let mut s = CodeHealthSurface::new();
        s.observe_instance(true, false);
        s.apply_ready();
        s.set_desired(payload());
        let (_, first) = s.take_pending_script().expect("首推");
        assert!(
            first.contains(r#"revision\":1"#),
            "首次 revision=1: {first}"
        );
        s.observe_instance(false, false);
        s.observe_instance(true, false);
        s.apply_ready();
        s.set_desired(payload());
        let (_, second) = s.take_pending_script().expect("重建后重发");
        assert!(
            second.contains(r#"revision\":2"#),
            "revision 跨重建继续递增: {second}"
        );
        assert_eq!(s.revision(), 2);
    }
}
