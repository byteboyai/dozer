//! iced 程序状态，承担 spike B 里 `controls.rs` 的角色。分两层（P2a 多项目
//! 并行）：
//!
//! - [`App`]：main.rs 真正持有的顶层容器，暴露 `view()`/`update()`。它持有
//!   整个程序只有一份的**外壳态**（daemon 客户端/窗口尺寸/图标栏与面板区
//!   的收起-拖宽-放大状态/右键菜单），以及 `projects`——并行打开的 N 个项目
//!   页签。
//! - [`Workspace`]：**单个项目**的全部状态（终端 tab 集合、文件树、预览/
//!   浏览器域、验收与审阅、对话列表）。N 份同时存活、互不干扰；当前聚焦的
//!   那一份经 `App::active_workspace()`/`active_workspace_mut()` 取用。
//!
//! 渲染的是 ByteBoy2077 的图标栏外壳：左右各一条固定宽图标栏，中间是左面板
//! 区（文件列表配对 / Web）与右面板区（Agent 配对 / 对话配对），两侧各自可
//! 收起、可拖宽，每个配对视图各自记住内部"列表:内容"分割比例；任一内容子
//! 面板可放大成覆盖整个中间区域的浮层（`maximize_overlay`）。终端接
//! `TerminalModel` + `term_view::view`，键盘输入直达 `dozerd`。
//!
//! ## 线程模型（配合 `main.rs` 一起看）
//! - UI 线程：winit 事件循环所在线程，`App::update`/`view` 只在这
//!   里跑。`update` 里凡是要碰网络 IO 的地方（写输入、建会话、resize），
//!   一律 `handle.spawn(...)` 丢给 tokio，绝不在这里 `block_on`。项目态方法
//!   要用到的那几个句柄按值打包成 [`ShellIo`] 传进去（见其文档）。
//! - tokio worker 线程：`main` 持有的 `tokio::runtime::Runtime` 驱动。
//!   每个 tab 的 attach 数据流有且只有一个消费者任务
//!   （[`forward_events`]），持有 `mpsc::UnboundedReceiver<TermEvent>`
//!   ——这是"事件流"的唯一物理落点。
//! - 桥接：tokio 任务里通过 `winit::event_loop::EventLoopProxy::send_event`
//!   把 `Message` 送回 UI 线程；`main.rs` 的 `ApplicationHandler::user_event`
//!   收到后调用 `app.update(..)` 并请求重绘。反方向（UI → tokio）
//!   靠 `Handle::spawn`，两个方向都不需要锁。

mod hook;
mod state;
#[cfg(test)]
mod tests;
mod view;

pub(crate) use hook::*;
pub(crate) use state::*;
pub(crate) use view::*;
