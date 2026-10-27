//! 地址栏焦点机制:`view()` 给 `text_input` 挂稳定 id,渲染循环每帧用 `CaptureAddrFocus`
//! 问出它是否持有 iced 真实焦点。`ADDR_FOCUSED` 是进程级单份(只在本库里)。

use iced_widget::core::Rectangle;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};

/// 地址栏稳定的 iced widget id:`view()` 里 `.id()` 挂给真正的
/// `text_input`,`CaptureAddrFocus` 每帧靠它在 widget 树里认出这一个(同
/// `extensions::files::search_field_id` 的既有手法)。
pub fn addr_field_id() -> Id {
    Id::new("browser-addr-box")
}

static ADDR_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式)地址栏上一帧是否持有 iced 内部真实焦点。`main.rs`
/// 渲染循环每帧跑完 `CaptureAddrFocus` 后立刻调用本函数,把结果塞进当前
/// `Workspace`(`State::set_addr_focused`)——`static` 只是临时桥接(同
/// `extensions::files::take_search_focused` 的既有手法,含消费式复位避免
/// 地址栏不可见的帧卡死上一次 `true` 永久堵死终端键盘转发的理由)。
pub fn take_addr_focused() -> bool {
    std::mem::replace(&mut *ADDR_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍,把 `addr_field_id()` 命中的
/// `text_input` 当前是否持有 iced 焦点写进 `ADDR_FOCUSED`。`traverse`
/// **必须**调用传入的 `operate` 闭包才会继续递归子节点——地址栏嵌在
/// `row!`/`container!` 里,空 `traverse` 会导致 `Row`/`Column` 的
/// `operate()` 直接跳过子节点,`focusable()` 永远不会被触达(见
/// `extensions::files::CaptureSearchFocus` 修复过的同款 Critical bug,
/// commit `b8281cd`,这次从一开始就不能再犯)。
pub struct CaptureAddrFocus;
impl Operation<()> for CaptureAddrFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&addr_field_id()) {
            *ADDR_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addr_field_id_is_stable_across_calls() {
        assert_eq!(addr_field_id(), addr_field_id());
    }
}
