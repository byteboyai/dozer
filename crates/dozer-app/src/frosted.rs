//! 磨砂噪点贴图共享原语——`menu::shell_frosted`(右键菜单静态弹层)与
//! `dialog::scrim`(弹窗遮罩)共用同一张噪点贴图,原先各自
//! `include_bytes!`/`LazyLock` 一份、图片被独立解码两次,现在抽出来共享。
//! 贴图本身的选型理由(暖白 `#FFE5B4`、`FilterMethod::Nearest` 保颗粒感)
//! 见资源本身用途——两个调用点都是**离散开关的静态浮层**(打开/关闭或
//! hover 状态切换时才重绘一次),这份开销可忽略;像 `tab_widget::
//! tab_overflow_menu` 那样带 `hover_t` 缓动动画、逐帧重绘的弹层不适用,
//! 继续用不带噪点的版本。

use iced_widget::core::{ContentFit, Element, Length};
use iced_widget::image;
use std::sync::LazyLock;

/// 磨砂颗粒贴图层:稀疏、暖白(ByteBoy2077 奶油 `#FFE5B4`)、低透明度噪点,
/// `Length::Fill` 铺满宿主容器(通常是 `Stack` 里与 base 同尺寸的非 base
/// 层)。贴图是静态资源(`assets/textures/menu_noise.png`,256×384、LA
/// 灰度+透明通道),`ContentFit::Fill` 拉伸铺满不追求原比例——噪点没有
/// 方向性特征,拉伸不会露出破绽。`LazyLock` 让 PNG 只解码一次。
pub(crate) fn noise_layer<'a, Msg: 'a>()
-> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    static NOISE: LazyLock<iced_widget::core::image::Handle> = LazyLock::new(|| {
        iced_widget::core::image::Handle::from_bytes(
            include_bytes!("../assets/textures/menu_noise.png").as_slice(),
        )
    });
    image(NOISE.clone())
        .width(Length::Fill)
        .height(Length::Fill)
        .content_fit(ContentFit::Fill)
        // 默认 `FilterMethod::Linear` 双线性缩放会把逐像素随机噪点插值抹成
        // 平滑渐变(实测截图验证),`Nearest` 放大保留每个源像素的硬边界,
        // 才是"磨砂"该有的颗粒感。
        .filter_method(iced_widget::core::image::FilterMethod::Nearest)
        .into()
}
