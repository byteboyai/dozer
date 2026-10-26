//! 产品组合适配层:把 Dozer 的服务实现(`dozer_client::Client`)适配成面板需要的
//! `dozer_services::Services` 接缝。**Host 机制不在这里**——这里是 Dozer 作为产品
//! 组合自己的实现(bytehost 规格 §5.3:共享执行环境接收泛型服务上下文,产品适配层负责构造)。
//!
//! `panel_host.rs` 只保留宿主只读视图契约与执行原语 `PanelIo`;服务构造挪到本层,
//! 使 host 不再直接依赖具体 `Client` 的适配细节(兼容桥 C4)。

pub(crate) mod agent_panel;
pub(crate) mod app_host;
pub(crate) mod files_panel;
pub(crate) mod preview_panel;
pub(crate) mod project_panel;
pub(crate) mod services;
pub(crate) mod todo_panel;
pub(crate) mod usage_panel;

pub(crate) mod conversations_panel;
