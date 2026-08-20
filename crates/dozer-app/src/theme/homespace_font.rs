//! 首页(homespace)专属字号 token:把 `homespace.rs` 里散落的
//! `text(...).size(N)` 字面量收敛成 3 个具名 token,编译期内嵌
//! `assets/theme/homespace.json` 的 `font_sizes` 节点,启动时解析一次。
//! `homespace.json` 同时也是 `homespace_color.rs` 的数据源(`colors` 节点)
//! ——两个模块各自只解析自己关心的顶层字段,互不干扰。
//!
//! 与 `font.rs`(管工作区控件字号)职责分离——这里只管首页控件内部文字
//! 字号。每个 accessor 返回的字号都乘过 `byteui::theme::icon_size::scale()`(全局缩放因子),
//! 因此改 `scale` 即整体缩放首页文字,与图标尺寸同步。
use serde::Deserialize;
use std::sync::LazyLock;

const RAW: &str = include_str!("../../assets/theme/homespace.json");

#[derive(Deserialize)]
struct HomespaceFonts {
    caption_sm: u32,
    label: u32,
    body: u32,
    subtitle: u32,
}

/// `homespace.json` 顶层结构里本模块只关心的部分——`colors` 节点是
/// `homespace_color.rs` 的地盘,这里不声明,serde 默认忽略未知字段。
#[derive(Deserialize)]
struct RawHomespaceFile {
    font_sizes: HomespaceFonts,
}

fn load(raw: &str) -> HomespaceFonts {
    let file: RawHomespaceFile =
        serde_json::from_str(raw).expect("homespace.json 格式错误(解析失败,font_sizes 节点)");
    file.font_sizes
}

static SIZES: LazyLock<HomespaceFonts> = LazyLock::new(|| load(RAW));

fn scale(v: u32) -> u32 {
    ((v as f32) * byteui::theme::icon_size::scale()).round() as u32
}

pub fn caption_sm() -> u32 {
    scale(SIZES.caption_sm)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚:默认 scale=1 时,字号必须和 homespace.json 字面量一致。
    /// 基准已抬到 14px(body=14)以对齐终端字号,层级比例保持不变。
    #[test]
    fn sizes_match_config_literals_at_default_scale() {
        assert_eq!(caption_sm(), 11);
        assert_eq!(label(), 13);
        assert_eq!(body(), 14);
        assert_eq!(subtitle(), 15);
    }
}
