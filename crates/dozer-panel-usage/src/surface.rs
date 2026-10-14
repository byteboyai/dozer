//! Usage 内容侧 webview 接入共享宿主 Surface 执行器(Task 08 / 规格 §5.4)。
//!
//! 迁移前,Usage 面板的内容侧 webview 由 App 自己用一套 `WebviewPushState`
//! 管 ready + 去重,并在 `runtime.rs` 里按固定 `USAGE_CONTENT_ID_OFFSET` 特判
//! IPC 归属。现在这层通用机制收进 `bytehost-desktop` 的 `SurfaceExecutor`:
//!
//! - **身份**:固定单槽 `(instance, SurfaceSlot(slot), generation)`。
//!   每次 webview 销毁重建递增 generation,旧代数的 ready/IPC 一律被路由层拒绝。
//! - **载荷构造留在面板**:`current_view_payload` / `encode_usage_push` 仍在本
//!   面板,executor 只按 `PushState` 语义决定何时推(去重、隐藏缓存、重建重发)。
//! - **原生操作不在本层**:创建/销毁由产品 webview 池完成,这里只根据
//!   "句柄是否在池中"驱动状态机(Absent → Created → WaitingReady → Ready;
//!   离池 → Destroyed → 下一帧重建)。
//!
//! 库化改动(计划 §Task 19 第 5 条):物理槽 id 与"包成 evaluate_script 脚本"
//! 的 `dispatch_script` 都是产品专属,改为构造时注入;库不再引用
//! `crate::app::USAGE_CONTENT_ID_OFFSET` / `crate::preview::dispatch_script`。

use bytehost_core::{PanelInstanceId, SurfaceEvent, SurfaceId, SurfaceKind, SurfaceSlot};
use bytehost_desktop::surface::SurfaceExecutor;
use dozer_model::Stamped;

use crate::adapter::ScriptDispatch;
use crate::protocol::{UsageViewPayload, encode_usage_push};

/// 推送载荷 = 业务 payload + 语言上下文(spec §8)。把语言上下文并入 `T` 使
/// Surface 的逐帧去重把 locale 也纳入比较:"业务数据不变、仅语言切换"同样触发
/// 一次推送。载荷 → 脚本的转换从 `Stamped` 同时取 stamp 与 payload。
type UsageStamped = Stamped<UsageViewPayload>;

/// Usage 内容侧固定单槽的物理槽 id 由产品注入(保证与产品 webview 池、
/// IPC 路由指向同一槽)。
pub struct UsageSurface {
    executor: SurfaceExecutor<UsageStamped>,
    instance: PanelInstanceId,
    slot: SurfaceSlot,
    /// 物理槽 id(产品 webview 池/IPC 路由用),与 `slot` 同值。
    raw_slot: u32,
    dispatch: ScriptDispatch,
    generation: u64,
}

impl UsageSurface {
    /// `raw_slot` 是产品侧固定的 Usage 内容槽 id;`dispatch` 把已序列化的
    /// payload 包成产品运行时的 `evaluate_script` 脚本(通常只是
    /// `preview::dispatch_script`)。
    pub fn new(raw_slot: u32, dispatch: ScriptDispatch) -> Self {
        let instance = PanelInstanceId::new(0, 0);
        let slot = SurfaceSlot(raw_slot);
        let mut executor = SurfaceExecutor::new();
        executor.register(
            surface_id(instance, slot, 0),
            SurfaceKind::Builtin,
            slot,
            Box::new(move |stamped: &UsageStamped| {
                let (locale, payload) = stamped.clone().into_parts();
                dispatch(&encode_usage_push(Some(locale), payload))
            }),
        );
        Self {
            executor,
            instance,
            slot,
            raw_slot,
            dispatch,
            generation: 0,
        }
    }

    /// 当前代数的稳定身份(IPC / ready 归属校验用)。
    pub fn id(&self) -> SurfaceId {
        surface_id(self.instance, self.slot, self.generation)
    }

    /// 每帧根据"原生实例是否在池中"推进生命周期。
    ///
    /// - 在池中:Absent/Destroyed/Failed → 发起创建并确认 `Created`(池已建好)。
    /// - 离池:有实例 → `Destroyed`,并递增 generation,使旧实例的在途 IPC
    ///   从这一刻起被路由层判为过期。
    ///
    /// **`native_replaced`**:本帧原生侧丢弃并重建了该槽的实例(预算淘汰、
    /// host 种类切换、项目切换整池清空后重建等)。不能仅凭"数字槽还在池里"
    /// 判断实例未变——同帧销毁+重建时槽号不变,但原生实例已换,旧实例已
    /// 发过的 ready/已送达状态绝不能继承给新实例。此标志为真时,先把旧身份
    /// 判死(销毁并递增 generation),再走创建路径。
    ///
    /// 幂等:同一状态重复调用不产生额外转移。
    pub fn observe_instance(&mut self, in_pool: bool, native_replaced: bool) {
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
    pub fn apply_ready(&mut self) {
        let id = self.id();
        self.executor.apply(id, SurfaceEvent::Ready);
    }

    /// 更新期望载荷(产品每帧算出新状态都调用)。载荷构造留在面板。
    pub fn set_desired(&mut self, payload: UsageStamped) {
        let id = self.id();
        self.executor.set_desired(id, payload);
    }

    /// 待推送脚本:Ready 且与上次送达不同才返回,附带物理槽 id,并写回
    /// 送达标记(与迁移前 `pending_push` + `mark_sent` 的调用契约一致)。
    pub fn take_pending_script(&mut self) -> Option<(usize, String)> {
        let id = self.id();
        let payload = self.executor.pending(id)?.clone();
        let script = self.executor.script_for(id, &payload)?;
        self.executor.mark_sent(id, payload);
        Some((self.raw_slot as usize, script))
    }

    /// 给定一个身份,它是否被当前路由层接受(IPC 归属校验用)。
    pub fn accepts(&self, id: Option<SurfaceId>) -> bool {
        id.is_some_and(|id| self.executor.route(id).is_accepted())
    }

    /// 校验指定身份的归属(测试断言旧代数被拒用)。
    #[cfg(test)]
    pub fn route_of(&self, id: SurfaceId) -> bytehost_desktop::surface::Routed {
        self.executor.route(id)
    }

    fn rebuild(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let id = self.id();
        let dispatch = self.dispatch;
        self.executor.register(
            id,
            SurfaceKind::Builtin,
            self.slot,
            Box::new(move |stamped: &UsageStamped| {
                let (locale, payload) = stamped.clone().into_parts();
                dispatch(&encode_usage_push(Some(locale), payload))
            }),
        );
    }
}

fn surface_id(instance: PanelInstanceId, slot: SurfaceSlot, generation: u64) -> SurfaceId {
    SurfaceId::new(instance, slot, bytehost_core::Generation::new(generation))
}
