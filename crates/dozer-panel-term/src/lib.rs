//! 共享终端控件库(S6b):终端模型 + iced 自绘视图。
//!
//! - [`model`] 包装 `alacritty_terminal` 的 VTE 解析器与网格状态,对外只
//!   暴露纯数据快照;对渲染层零耦合。
//! - [`view`] 把模型快照画成 iced canvas 控件。字体、字号、行高与消息映射
//!   由调用方经 [`style::TermStyle`] / [`view::view`] 的 `map` 注入,库里
//!   不读产品应用的状态。
//!
//! 本库不含 PTY(在守护进程里),也不内嵌字体(调用方注册并传入)。

pub mod input;
pub mod model;
pub mod osc;
pub mod style;
pub mod view;
