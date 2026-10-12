//! Usage 内容侧 webview 接入共享宿主 Surface 执行器(Task 08 / 规格 §5.4)。
//!
//! 迁移前,Usage 面板的内容侧 webview 由 App 自己用一套 `WebviewPushState`
//! 管 ready + 去重,并在 `runtime.rs` 里按固定 `USAGE_CONTENT_ID_OFFSET` 特判
//! IPC 归属。现在这层通用机制收进 `bytehost-desktop` 的 `SurfaceExecutor`:
//!
//! - **身份**:固定单槽 `(instance, SurfaceSlot(USAGE_CONTENT_ID_OFFSET), generation)`。
//!   每次 webview 销毁重建递增 generation,旧代数的 ready/IPC 一律被路由层拒绝。
//! - **载荷构造留在面板**:`current_view_payload` / `encode_usage_push` 仍在本
//!   面板,executor 只按 `PushState` 语义决定何时推(去重、隐藏缓存、重建重发)。
//! - **原生操作不在本层**:创建/销毁由 App 的 webview 池完成,这里只根据
//!   "句柄是否在池中"驱动状态机(Absent → Created → WaitingReady → Ready;
//!   离池 → Destroyed → 下一帧重建)。

use bytehost_core::{PanelInstanceId, SurfaceEvent, SurfaceId, SurfaceKind, SurfaceSlot};
use bytehost_desktop::surface::SurfaceExecutor;

use super::protocol::{UsageViewPayload, encode_usage_push};

/// Usage 内容侧固定单槽。物理槽 id 复用既有 `USAGE_CONTENT_ID_OFFSET`,
/// 确保 webview 池、`runtime.rs` IPC 与 Surface 路由三者指向同一槽。
pub(crate) const USAGE_SURFACE_SLOT: u32 = crate::app::USAGE_CONTENT_ID_OFFSET as u32;

/// Usage 内容侧 webview 的 Surface 管理器。
pub(crate) struct UsageSurface {
    executor: SurfaceExecutor<UsageViewPayload>,
    instance: PanelInstanceId,
    slot: SurfaceSlot,
    generation: u64,
}

impl Default for UsageSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl UsageSurface {
    pub(crate) fn new() -> Self {
        let instance = PanelInstanceId::new(0, 0);
        let slot = SurfaceSlot(USAGE_SURFACE_SLOT);
        let mut executor = SurfaceExecutor::new();
        executor.register(
            surface_id(instance, slot, 0),
            SurfaceKind::Builtin,
            slot,
            Box::new(|payload: &UsageViewPayload| {
                crate::preview::dispatch_script(&encode_usage_push(payload.clone()))
            }),
        );
        Self {
            executor,
            instance,
            slot,
            generation: 0,
        }
    }

    /// 当前代数的稳定身份(IPC / ready 归属校验用)。
    pub(crate) fn id(&self) -> SurfaceId {
        surface_id(self.instance, self.slot, self.generation)
    }

    /// 每帧根据"原生实例是否在池中"推进生命周期。
    ///
    /// - 在池中:Absent/Destroyed/Failed → 发起创建并确认 `Created`(池已建好)。
    /// - 离池:有实例 → `Destroyed`,并递增 generation,使旧实例的在途 IPC
    ///   从这一刻起被路由层判为过期。
    ///
    /// 幂等:同一状态重复调用不产生额外转移。
    pub(crate) fn observe_instance(&mut self, in_pool: bool) {
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
    pub(crate) fn set_desired(&mut self, payload: UsageViewPayload) {
        let id = self.id();
        self.executor.set_desired(id, payload);
    }

    /// 待推送脚本:Ready 且与上次送达不同才返回,附带物理槽 id,并写回
    /// 送达标记(与迁移前 `pending_push` + `mark_sent` 的调用契约一致)。
    pub(crate) fn take_pending_script(&mut self) -> Option<(usize, String)> {
        let id = self.id();
        let payload = self.executor.pending(id)?.clone();
        let script = self.executor.script_for(id, &payload)?;
        self.executor.mark_sent(id, payload);
        Some((USAGE_SURFACE_SLOT as usize, script))
    }

    /// 给定一个身份,它是否被当前路由层接受(IPC 归属校验用)。
    pub(crate) fn accepts(&self, id: Option<SurfaceId>) -> bool {
        id.is_some_and(|id| self.executor.route(id).is_accepted())
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
            Box::new(|payload: &UsageViewPayload| {
                crate::preview::dispatch_script(&encode_usage_push(payload.clone()))
            }),
        );
    }
}

fn surface_id(instance: PanelInstanceId, slot: SurfaceSlot, generation: u64) -> SurfaceId {
    SurfaceId::new(instance, slot, bytehost_core::Generation::new(generation))
}
