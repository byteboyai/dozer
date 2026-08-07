//! 工作区（UI 控件）字号 token 化：`workspace.rs` 里散落的
//! `text(...).size(N)` 字面量收敛成 8 个具名 token，编译期内嵌
//! `assets/theme/workspace.json` 的 `font_sizes` 节点，启动时解析一次。
//! `workspace.json` 同时也是 `chrome_style.rs` 的数据源（`regions` 节点）
//! ——两个模块各自只解析自己关心的顶层字段，互不干扰。
//!
//! 与 `terminal_font.rs`（管终端 pane 的等宽字体，`terminal.json`）和
//! `chrome_style.rs`（管区域外层容器的背景/边框/间距）职责严格分离——
//! 这里只管控件内部文字字号，不越界。解析失败（格式错误、缺字段）直接
//! panic：开发期配置错误，不是需要优雅降级的运行时数据（同
//! `chrome_style.rs` 的定位）。
//!
//! 每个 accessor 返回的字号都乘过 `icon_size::scale()`（全局缩放因子），
//! 因此改 `scale` 即整体缩放全部控件文字，与图标尺寸同步。
use super::icon_size;
use serde::Deserialize;
use std::sync::LazyLock;

const RAW: &str = include_str!("../../assets/theme/workspace.json");

#[derive(Deserialize)]
struct WorkspaceFonts {
    dot_xs: u32,
    dot_sm: u32,
    caption_sm: u32,
    caption: u32,
    label: u32,
    body: u32,
    subtitle: u32,
    title: u32,
}

/// `workspace.json` 顶层结构里本模块只关心的部分——`regions` 节点是
/// `chrome_style.rs` 的地盘，这里不声明，serde 默认忽略未知字段。
#[derive(Deserialize)]
struct RawWorkspaceFile {
    font_sizes: WorkspaceFonts,
}

fn load(raw: &str) -> WorkspaceFonts {
    let file: RawWorkspaceFile =
        serde_json::from_str(raw).expect("workspace.json 格式错误(解析失败,font_sizes 节点)");
    file.font_sizes
}

static SIZES: LazyLock<WorkspaceFonts> = LazyLock::new(|| load(RAW));

pub fn dot_xs() -> u32 {
    scale(SIZES.dot_xs)
}
pub fn dot_sm() -> u32 {
    scale(SIZES.dot_sm)
}
pub fn caption_sm() -> u32 {
    scale(SIZES.caption_sm)
}
pub fn caption() -> u32 {
    scale(SIZES.caption)
}
pub fn label() -> u32 {
    scale(SIZES.label)
}
pub fn body() -> u32 {
    scale(SIZES.body)
}
pub fn subtitle() -> u32 {
    scale(SIZES.subtitle)
}
pub fn title() -> u32 {
    scale(SIZES.title)
}

/// 把设计基准字号按全局 scale 折算成实际像素字号（四舍五入）。
fn scale(base: u32) -> u32 {
    ((base as f32) * icon_size::scale()).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚:8 个 token 的解析结果必须和改动前 workspace.rs 里的
    /// 字面量完全一致——纯代码搬家,数值不该变。
    #[test]
    fn tokens_match_pre_migration_literals() {
        assert_eq!(dot_xs(), 8);
        assert_eq!(dot_sm(), 9);
        assert_eq!(caption_sm(), 10);
        assert_eq!(caption(), 11);
        assert_eq!(label(), 12);
        assert_eq!(body(), 13);
        assert_eq!(subtitle(), 14);
        assert_eq!(title(), 15);
    }

    #[test]
    #[should_panic(expected = "workspace.json 格式错误")]
    fn malformed_json_panics() {
        load(r#"{"font_sizes": {"dot_xs": 8}}"#);
    }
}
