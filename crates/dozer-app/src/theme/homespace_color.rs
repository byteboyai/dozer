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
    /// 按钮 hover/按下态描边色(统一按钮规范,见 `dialog::
    /// action_button_border_color`)。
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
pub fn border() -> Color {
    region::resolve_color(&COLORS.border)
}
pub fn gold() -> Color {
    region::resolve_color(&COLORS.gold)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `byteui::theme::color` 的当前配色方案是进程级共享 static，cargo
    /// test 默认多线程并跑；下面两个测试都依赖它处在 `Dark`/显式切换,
    /// 加锁序列化避免交叉(同 `region.rs` 测试模块同名锁的顾虑)。
    static SCHEME_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_scheme() -> std::sync::MutexGuard<'static, ()> {
        SCHEME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 防漂移锚:各 token 解析结果必须和配置字面量一致——纯配置搬家,
    /// 数值不该变。以后有人手滑改错 homespace.json,这个测试会炸。
    #[test]
    fn card_bg_matches_config_literal() {
        let _guard = lock_scheme();
        assert_eq!(card_bg(), Color::from_rgb8(0x0a, 0x0e, 0x16));
    }

    #[test]
    fn tokens_resolve_without_panic() {
        // 加载在首次调用时panic;这里逐一调用,确认配置可正常解析。
        let _ = (error(), dim(), cream(), border(), gold());
    }

    #[test]
    #[should_panic(expected = "未知颜色令牌")]
    fn unknown_color_token_panics() {
        region::resolve_color("NOT_A_REAL_TOKEN");
    }

    /// `byteui::theme::color` 的当前配色方案是进程级共享 static，加锁避免
    /// 和别的测试交叉，同 `region.rs` 测试模块同名锁的顾虑。防回归:
    /// `card_bg` 曾经在 `homespace.json` 里写死 `#0a0e16` 字面量而不是
    /// `PANEL` 令牌名,导致运行时切主题对首页卡片背景毫无作用
    /// (2026-09-15 与 `region.rs` 的 `top_bar`/`background` 同批修复)。
    #[test]
    fn card_bg_tracks_live_scheme_switch() {
        let _guard = lock_scheme();
        byteui::theme::color::set_scheme(byteui::theme::color::ColorScheme::Light);
        assert_eq!(card_bg(), byteui::theme::color::current().panel);
        byteui::theme::color::set_scheme(byteui::theme::color::ColorScheme::Dark);
        assert_eq!(card_bg(), byteui::theme::color::current().panel);
    }
}
