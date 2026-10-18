//! 同源 Project 面板库(S5a):Dozer 与 Digger 共用的项目信息面板状态、更新与视图。
//!
//! 依赖闭包**不含** `dozer-app` / `dozer-client` / `dozer-core`(见 spec §5)。产品专属语义经注入
//! 提供,见 [`adapter`];请求追踪 / 激活上下文 / 提示发件箱来自共享的 `dozer-panel-host`。
//!
//! 库只持有面板自身的渲染态([`WorkspaceState`])与视图 / 更新逻辑;项目名、git 快照、共享记忆
//! 都经 [`dozer_services`] 的 `ProjectService` / `MemoryService` 取数,库不碰磁盘上的项目注册表。

pub mod adapter;
pub mod delete;
pub mod disk;
pub mod i18n;
pub mod links;
pub mod meta;
pub mod scaffold;

/// 面板 owner id:`locales/`(随库)。
pub const PROJECT_OWNER: &str = "dozer.project";


