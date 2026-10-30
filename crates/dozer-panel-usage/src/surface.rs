//! Usage 内容侧 webview 接入共享宿主 Surface 执行器(Task 08 / 规格 §5.4)。
//!
//! 迁移前,Usage 面板的内容侧 webview 由 App 自己用一套 `WebviewPushState`
//! 管 ready + 去重,并在 `runtime.rs` 里按固定 `USAGE_CONTENT_ID_OFFSET` 特判
//! IPC 归属。现在这层通用机制收进 `bytehost-desktop` 的 `SurfaceExecutor`。
//!
//! Tier2 Task2 起,**固定单槽的同构部分**(身份、每帧生命周期推进、旧代数拒绝、
//! 重建换代数)收进 `dozer_panel_host::SingleSlotSurface`;本模块只保留 Usage
//! 自己的载荷与脚本构造:
//!
//! - **身份**:固定单槽 `(instance, SurfaceSlot(slot), generation)`。
//! - **载荷构造留在面板**:`current_view_payload` / `encode_usage_push` 仍在本
//!   面板,executor 只按 `PushState` 语义决定何时推(去重、隐藏缓存、重建重发)。
//! - **原生操作不在本层**:创建/销毁由产品 webview 池完成,这里只根据
//!   "句柄是否在池中"驱动状态机。
//!
//! 物理槽 id 与"包成 evaluate_script 脚本"的 `dispatch_script` 都是产品专属,
//! 构造时注入;库不再引用 `crate::app::USAGE_CONTENT_ID_OFFSET` /
//! `crate::preview::dispatch_script`。

use bytehost_core::{PanelInstanceId, SurfaceId, SurfaceSlot};
use dozer_model::Stamped;
use dozer_panel_host::SingleSlotSurface;

use crate::adapter::ScriptDispatch;
use crate::protocol::{UsageViewPayload, encode_usage_push};

/// 推送载荷 = 业务 payload + 语言上下文(spec §8)。把语言上下文并入 `T` 使
/// Surface 的逐帧去重把 locale 也纳入比较:"业务数据不变、仅语言切换"同样触发
/// 一次推送。载荷 → 脚本的转换从 `Stamped` 同时取 stamp 与 payload。
type UsageStamped = Stamped<UsageViewPayload>;

/// Usage 内容侧固定单槽的物理槽 id 由产品注入(保证与产品 webview 池、
/// IPC 路由指向同一槽)。
pub struct UsageSurface {
    inner: SingleSlotSurface<UsageStamped>,
}

impl UsageSurface {
    /// `raw_slot` 是产品侧固定的 Usage 内容槽 id;`dispatch` 把已序列化的
    /// payload 包成产品运行时的 `evaluate_script` 脚本(通常只是
    /// `preview::dispatch_script`)。
    pub fn new(raw_slot: u32, dispatch: ScriptDispatch) -> Self {
        let instance = PanelInstanceId::new(0, 0);
        let slot = SurfaceSlot(raw_slot);
        let inner = SingleSlotSurface::new(instance, slot, move || {
            let dispatch = dispatch;
            Box::new(move |stamped: &UsageStamped| {
                let (locale, payload) = stamped.clone().into_parts();
                dispatch(&encode_usage_push(Some(locale), payload))
            })
        });
        Self { inner }
    }

    /// 当前代数的稳定身份(IPC / ready 归属校验用)。
    pub fn id(&self) -> SurfaceId {
        self.inner.id()
    }

    /// 每帧根据"原生实例是否在池中"推进生命周期(语义见
    /// [`SingleSlotSurface::observe_instance`],含 `native_replaced` 的同帧
    /// 销毁+重建处理)。幂等:同一状态重复调用不产生额外转移。
    pub fn observe_instance(&mut self, in_pool: bool, native_replaced: bool) {
        self.inner.observe_instance(in_pool, native_replaced);
    }

    /// 收到 webview 的 ready 握手(仅内置面板走这条)。
    pub fn apply_ready(&mut self) {
        self.inner.apply_ready();
    }

    /// 更新期望载荷(产品每帧算出新状态都调用)。载荷构造留在面板。
    pub fn set_desired(&mut self, payload: UsageStamped) {
        self.inner.set_desired(payload);
    }

    /// 待推送脚本:Ready 且与上次送达不同才返回,附带物理槽 id,并写回
    /// 送达标记(与迁移前 `pending_push` + `mark_sent` 的调用契约一致)。
    pub fn take_pending_script(&mut self) -> Option<(usize, String)> {
        self.inner
            .take_pending_script()
            .map(|(slot, script)| (slot as usize, script))
    }

    /// 给定一个身份,它是否被当前路由层接受(IPC 归属校验用)。
    pub fn accepts(&self, id: Option<SurfaceId>) -> bool {
        self.inner.accepts(id)
    }

    /// 校验指定身份的归属(测试断言旧代数被拒用)。
    #[cfg(test)]
    pub fn route_of(&self, id: SurfaceId) -> bytehost_desktop::surface::Routed {
        self.inner.route_of(id)
    }
}
