//! 事件循环层的小工具:悬停动画帧间隔、清帧 `RenderPass`、合成命令键
//! 事件。原本全部平铺在 `main.rs` 顶层,Phase 1 结构重组时抽到本模块,逻辑
//! 未做任何改动。

use std::time::Duration;

use iced_wgpu::wgpu;
use iced_winit::core::{Event, SmolStr};

/// 所有按钮悬停动画的帧间隔:约 60fps。配合 `App::advance_hover_anims`
/// 的指数逼近(每拍残余 50%),约 80ms 收敛,给出跟手的 ease-out 过渡。
pub(crate) const HOVER_ANIM_INTERVAL: Duration = Duration::from_millis(16);

/// 清空一帧到给定背景色，不再绘制 spike 阶段的示例三角形
/// （spike B 的 `scene.rs`/wgsl shader 已随本任务删除）。
pub(crate) fn clear<'a>(
    target: &'a wgpu::TextureView,
    encoder: &'a mut wgpu::CommandEncoder,
    background_color: iced_winit::core::Color,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear({
                    let [r, g, b, a] = background_color.into_linear();

                    wgpu::Color {
                        r: r as f64,
                        g: g as f64,
                        b: b as f64,
                        a: a as f64,
                    }
                }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    })
}

/// 合成一个 `⌘/Ctrl(COMMAND)+ch` 的 `KeyPressed` iced 事件。`text_input`/
/// `text_editor` 靠 `key.to_latin(physical_key)` 把它落成 `'x'/'c'/'v'/'a'`,
/// 再检查 `modifiers.command()` 走各自的剪贴板/全选分支;`physical_key` 给
/// 出与字符一致的物理键码,保证 `to_latin` 命中。`COMMAND` 在 mac 上即 ⌘
/// (LOGO),其余平台即 Ctrl(见 iced `keyboard::Modifiers::COMMAND`)。
pub(crate) fn unique_command_event(ch: char) -> Event {
    use iced_winit::core::keyboard::key::Code;
    use iced_winit::core::keyboard::{self, key};
    let code = match ch {
        'a' => Code::KeyA,
        'c' => Code::KeyC,
        'v' => Code::KeyV,
        'x' => Code::KeyX,
        _ => unreachable!("menu_edit_key only maps c/x/v/a"),
    };
    let keyboard_event = keyboard::Event::KeyPressed {
        key: keyboard::Key::Character(SmolStr::new(format!("{ch}"))),
        modified_key: keyboard::Key::Character(SmolStr::new(format!("{ch}"))),
        physical_key: key::Physical::Code(code),
        modifiers: keyboard::Modifiers::COMMAND,
        location: keyboard::Location::Standard,
        text: Some(SmolStr::new(format!("{ch}"))),
        repeat: false,
    };
    Event::Keyboard(keyboard_event)
}
