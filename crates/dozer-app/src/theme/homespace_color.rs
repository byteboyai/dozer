//! 首页(homespace)专属颜色 token:把 `homespace.rs` 里散落的
//! `theme::color::*` 字面量引用收敛成具名 token,编译期内嵌
//! `assets/theme/homespace.json` 的 `colors` 节点,启动时解析一次。
//!
//! 颜色字段同 workspace.json 的 region:支持 `theme.rs` 现成的令牌名
//! (如 `"BORDER"`),或 `#RGB` / `#RRGGBB` 十六进制字面量——复用
//! `region::resolve_color`,保证两套配置用同一套解析规则。解析失败
//! (格式错误、未知颜色令牌名)直接 panic:开发期配置错误,不是需要优雅
//! 降级的运行时数据(同 `region.rs` 的定位)。
use super::region;
use iced_widget::core::Color;
use serde::Deserialize;
use std::sync::LazyLock;

const RAW: &str = include_str!("../../assets/theme/homespace.json");

#[derive(Deserialize)]
struct HomespaceColors {
    /// 错误文案(如 daemon 连接失败提示)颜色。
    error: String,
    /// 次要/标签文字(标题、副标题、占位符、相对时间、路径)颜色。
    dim: String,
    /// 卡片/搜索框/按钮等面内容器的实底背景色(与面板底色一致)。
    card_bg: String,
    /// 卡片主文字(项目名、文件名、对话标题、最近文件标题)颜色。
    cream: String,
    /// 甲方动作专属色(新增项目按钮文字/描边)。
    gold: String,
    /// 卡片/按钮描边色。
    border: String,
}

/// `homespace.json` 顶层结构里本模块只关心的部分——`font_sizes` 节点是
/// `homespace_font.rs` 的地盘,这里不声明,serde 默认忽略未知字段。
#[derive(Deserialize)]
struct RawHomespaceFile {
    colors: HomespaceColors,
}

fn load(raw: &str) -> HomespaceColors {
    let file: RawHomespaceFile =
        serde_json::from_str(raw).expect("homespace.json 格式错误(解析失败,colors 节点)");
    file.colors
}

static COLORS: LazyLock<HomespaceColors> = LazyLock::new(|| load(RAW));

pub fn error() -> Color {
    region::resolve_color(&COLORS.error)
}
pub fn dim() -> Color {
    region::resolve_color(&COLORS.dim)
}
pub fn card_bg() -> Color {
    region::resolve_color(&COLORS.card_bg)
}
pub fn cream() -> Color {
    region::resolve_color(&COLORS.cream)
}
pub fn gold() -> Color {
    region::resolve_color(&COLORS.gold)
}
pub fn border() -> Color {
    region::resolve_color(&COLORS.border)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚:各 token 解析结果必须和配置字面量一致——纯配置搬家,
    /// 数值不该变。以后有人手滑改错 homespace.json,这个测试会炸。
    #[test]
    fn card_bg_matches_config_literal() {
        assert_eq!(card_bg(), Color::from_rgb8(0x0a, 0x0e, 0x16));
    }

    #[test]
    fn tokens_resolve_without_panic() {
        // 加载在首次调用时panic;这里逐一调用,确认配置可正常解析。
        let _ = (error(), dim(), cream(), gold(), border());
    }

    #[test]
    #[should_panic(expected = "未知颜色令牌")]
    fn unknown_color_token_panics() {
        region::resolve_color("NOT_A_REAL_TOKEN");
    }
}
