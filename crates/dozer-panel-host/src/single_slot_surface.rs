//! 固定单槽 Surface 执行器包装(Tier2 Task2)。
//!
//! Usage 内容侧与 CodeHealth 内容侧各有一个"永远只有一个 webview"的 Surface:
//! 身份固定为 `(instance, slot, generation)`,每帧按"原生实例是否仍在池中"推进
//! 生命周期,旧代数的 ready/IPC 一律拒绝,重建后强制重推。这段机制两份实现
//! 逐行同构,只有**载荷类型/脚本构造/物理槽来源**不同。
//!
//! 本模块收掉同构部分;产品差异留在调用方:
//! - `build_script` 工厂:每次重建都要一份新的脚本构造闭包(旧闭包已随登记
//!   移进执行器),所以这里收的是**工厂**而不是单个闭包;
//! - 物理槽 id 由调用方给(Usage 注入、CodeHealth 用固定常量);
//! - 载荷与 `ScriptBuilder` 的具体内容。
//!
//! 语义不变量(与迁移前的两份实现逐条一致):
//! - `native_replaced`:同帧销毁+重建时槽号不变但原生实例已换,必须换代数,
//!   旧实例的 ready/已送达状态不得继承;
//! - 幂等:同一状态重复 `observe_instance` 不产生额外转移。

use bytehost_core::{
    Generation, PanelInstanceId, SurfaceEvent, SurfaceId, SurfaceKind, SurfaceSlot, SurfaceState,
};
use bytehost_desktop::surface::{Routed, ScriptBuilder, SurfaceExecutor};

/// 固定单槽 Surface:一个身份、一个物理槽、一个执行器。
pub struct SingleSlotSurface<T> {
    executor: SurfaceExecutor<T>,
    instance: PanelInstanceId,
    slot: SurfaceSlot,
    generation: u64,
    /// 每次(重新)登记都要一份新的脚本构造闭包;这里保存工厂。
    build_script: Box<dyn Fn() -> ScriptBuilder<T> + Send>,
}

impl<T: PartialEq + Clone + Send + 'static> SingleSlotSurface<T> {
    /// `build_script` 在首次登记与每次重建时各调用一次。
    pub fn new(
        instance: PanelInstanceId,
        slot: SurfaceSlot,
        build_script: impl Fn() -> ScriptBuilder<T> + Send + 'static,
    ) -> Self {
        let mut executor = SurfaceExecutor::new();
        executor.register(
            surface_id(instance, slot, 0),
            SurfaceKind::Builtin,
            slot,
            build_script(),
        );
        Self {
            executor,
            instance,
            slot,
            generation: 0,
            build_script: Box::new(build_script),
        }
    }

    /// 当前代数的稳定身份(IPC / ready 归属校验用)。
    pub fn id(&self) -> SurfaceId {
        surface_id(self.instance, self.slot, self.generation)
    }

    /// 物理槽 id(产品 webview 池 / IPC 路由用)。
    pub fn raw_slot(&self) -> u32 {
        self.slot.0
    }

    /// 每帧根据"原生实例是否在池中"推进生命周期。
    ///
    /// - 在池中:`Absent`/`Destroyed`/`Failed`/未登记 → 发起创建并确认 `Created`
    ///   (池已建好);
    /// - 离池:有实例 → `Destroyed`,并递增代数,使旧实例的在途 IPC 从这一刻起
    ///   被路由层判为过期;
    /// - `native_replaced`:先把旧身份判死(销毁并递增代数),再走创建路径。
    pub fn observe_instance(&mut self, in_pool: bool, native_replaced: bool) {
        if native_replaced {
            let id = self.id();
            let has_instance = self
                .executor
                .state(id)
                .is_some_and(SurfaceState::has_instance);
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
                Some(SurfaceState::Absent | SurfaceState::Destroyed | SurfaceState::Failed { .. })
                    | None
            );
            if needs_create {
                self.executor.begin_create(id);
                self.executor.apply(id, SurfaceEvent::Created);
            }
        } else {
            let has_instance = self
                .executor
                .state(id)
                .is_some_and(SurfaceState::has_instance);
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
    pub fn set_desired(&mut self, value: T) {
        let id = self.id();
        self.executor.set_desired(id, value);
    }

    /// 待推送脚本:Ready 且与上次送达不同才返回,附带物理槽 id,并写回送达标记。
    pub fn take_pending_script(&mut self) -> Option<(u32, String)> {
        let id = self.id();
        let payload = self.executor.pending(id)?.clone();
        let script = self.executor.script_for(id, &payload)?;
        self.executor.mark_sent(id, payload);
        Some((self.slot.0, script))
    }

    /// 给定一个身份,它是否被当前路由层接受(IPC 归属校验用)。
    pub fn accepts(&self, id: Option<SurfaceId>) -> bool {
        id.is_some_and(|id| self.executor.route(id).is_accepted())
    }

    /// 校验指定身份的归属(测试断言旧代数被拒用;消费方的测试也用它)。
    pub fn route_of(&self, id: SurfaceId) -> Routed {
        self.executor.route(id)
    }

    /// 当前是否已握手就绪。
    pub fn is_ready(&self) -> bool {
        matches!(self.executor.state(self.id()), Some(SurfaceState::Ready))
    }

    fn rebuild(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let id = self.id();
        self.executor
            .register(id, SurfaceKind::Builtin, self.slot, (self.build_script)());
    }
}

fn surface_id(instance: PanelInstanceId, slot: SurfaceSlot, generation: u64) -> SurfaceId {
    SurfaceId::new(instance, slot, Generation::new(generation))
}

#[cfg(test)]
mod tests {
    use super::*;

    type Payload = u32;

    fn surface() -> SingleSlotSurface<Payload> {
        SingleSlotSurface::new(PanelInstanceId::new(0, 0), SurfaceSlot(7), || {
            Box::new(|value: &Payload| format!("push({value})"))
        })
    }

    #[test]
    fn ready_and_dedup_follow_the_executor_contract() {
        let mut s = surface();
        s.observe_instance(true, false);
        s.set_desired(1);
        assert!(s.take_pending_script().is_none(), "未 ready 不推");
        s.apply_ready();
        assert_eq!(s.take_pending_script(), Some((7, "push(1)".into())));
        s.set_desired(1);
        assert!(s.take_pending_script().is_none(), "同值不重推");
        assert!(s.is_ready());
    }

    #[test]
    fn leaving_and_reentering_the_pool_opens_a_new_generation_and_resends() {
        let mut s = surface();
        s.observe_instance(true, false);
        s.apply_ready();
        s.set_desired(1);
        assert!(s.take_pending_script().is_some());
        let old = s.id();
        s.observe_instance(false, false);
        s.observe_instance(true, false);
        assert_ne!(s.id(), old, "重建必须换代数");
        assert!(!s.route_of(old).is_accepted(), "旧代数被拒");
        assert!(!s.accepts(Some(old)));
        s.apply_ready();
        s.set_desired(1);
        assert!(s.take_pending_script().is_some(), "新实例强制重发");
    }

    #[test]
    fn same_frame_native_replacement_does_not_inherit_ready() {
        let mut s = surface();
        s.observe_instance(true, false);
        s.apply_ready();
        s.set_desired(1);
        assert!(s.take_pending_script().is_some());
        let old = s.id();
        s.observe_instance(true, true);
        assert_ne!(s.id(), old);
        assert!(!s.route_of(old).is_accepted());
        assert!(s.take_pending_script().is_none(), "新实例尚未握手");
    }
}
