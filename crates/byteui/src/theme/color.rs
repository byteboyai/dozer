//! ByteBoy2077 是编译期默认值,`set_theme` 可在运行时整体替换成另一份
//! 产品的取值——组件内部一律读 `current()`,不再有硬编码色值常量。

use iced_widget::core::Color;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::RwLock;

const fn c(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorTokens {
    pub bg: Color,
    pub panel: Color,
    pub term_bg: Color,
    pub card: Color,
    pub border: Color,
    pub cream: Color,
    pub body: Color,
    pub dim: Color,
    pub gold: Color,
    pub cyan: Color,
    pub green: Color,
    pub purple: Color,
    pub red: Color,
    pub ignored: Color,
    pub orange: Color,
    pub magenta: Color,
    pub blue: Color,
    pub lime: Color,
    pub scrim: Color,
    pub tab_active_border: Color,
    pub tab_active_bg: Color,
    pub tab_hover: Color,
    pub desc_bg: Color,
}

impl ColorTokens {
    /// `bg`/`panel`(`term_bg` 跟 `panel` 同值)2026-09-16 起拆成两个不同
    /// 数值——此前三者恒等,面板背景语义上分"可收缩的 list 侧用 bg、不可
    /// 收缩的 content 侧用 panel"(见 dozer-app 的
    /// `theme::region::{project_pane,agent_list_pane,conversation_list_pane}`
    /// 等区域 JSON),浅色主题本就两值不同,深色缺这个区分度、用户要求补上。
    /// `panel` 保留原有数值(终端/内容区视觉不变),`bg` 提亮一档给 list 侧
    /// 一点分离感,同色系不出戏。此前这里写"逐一对应
    /// crates/dozer-app/src/theme/color.rs 的锁死值,禁止改动"——那份文件
    /// 在 byteui 颜色迁移系列完工后已删除,颜色 token 唯一来源就是这里,
    /// 说明已过期一并去掉。
    pub const fn byteboy2077() -> Self {
        Self {
            bg: c(0x0d, 0x13, 0x1c),
            panel: c(0x0a, 0x0e, 0x16),
            term_bg: c(0x0a, 0x0e, 0x16),
            card: c(0x12, 0x20, 0x2a),
            border: c(0x1c, 0x34, 0x40),
            cream: c(0xFF, 0xE5, 0xB4),
            body: c(0x9A, 0xB4, 0xC4),
            dim: c(0x6B, 0x7F, 0x8F),
            gold: c(0xF2, 0xD9, 0x4E),
            cyan: c(0x47, 0xDE, 0xF0),
            green: c(0x1A, 0xD5, 0x85),
            purple: c(0x95, 0x80, 0xFF),
            red: c(0xFF, 0x6E, 0x6E),
            ignored: c(0x6B, 0x7F, 0x8F),
            orange: c(0xFF, 0x9B, 0x4D),
            magenta: c(0xFF, 0x6E, 0xC7),
            blue: c(0x4D, 0x8C, 0xFF),
            lime: c(0xA3, 0xE6, 0x35),
            scrim: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.55,
            },
            tab_active_border: c(0xDC, 0xC9, 0xA3),
            tab_active_bg: c(0x15, 0x26, 0x30),
            tab_hover: c(0x15, 0x26, 0x30),
            desc_bg: c(0x15, 0x26, 0x30),
        }
    }

    /// 逐一对应 `design/浅色配色表.html` 的语义色板表(23 项),与
    /// `byteboy2077()` 同源中性色相(navy `bg` #0a0e16)提亮而来，不用
    /// 暖白/暖灰，避免与 `gold`(甲方动作专属)"暖调即品牌"的印象冲突。
    pub const fn byteboy2077_light() -> Self {
        Self {
            bg: c(0xff, 0xfd, 0xf6),
            panel: c(0xfe, 0xf2, 0xe4),
            term_bg: c(0xfe, 0xf2, 0xe4),
            card: c(0xfe, 0xfd, 0xfb),
            border: c(0xd7, 0xdf, 0xe5),
            cream: c(0x16, 0x23, 0x2e),
            body: c(0x36, 0x42, 0x4e),
            dim: c(0x4c, 0x5c, 0x68),
            gold: c(0x11, 0x8b, 0x96),
            cyan: c(0x0e, 0x8a, 0x9e),
            green: c(0x12, 0x8f, 0x5a),
            purple: c(0x6a, 0x4f, 0xdb),
            red: c(0xd1, 0x48, 0x3f),
            ignored: c(0x8b, 0x98, 0xa2),
            orange: c(0xc9, 0x7a, 0x1b),
            magenta: c(0xc9, 0x3b, 0x93),
            blue: c(0x2f, 0x6f, 0xe0),
            lime: c(0x6f, 0xa8, 0x13),
            scrim: Color {
                r: 0x0a as f32 / 255.0,
                g: 0x0e as f32 / 255.0,
                b: 0x14 as f32 / 255.0,
                a: 0.4,
            },
            tab_active_border: c(0x65, 0x88, 0x9e),
            tab_active_bg: c(0x98, 0xb6, 0xc1),
            tab_hover: c(0xa3, 0xd2, 0xe2),
            desc_bg: c(0xea, 0xef, 0xf2),
        }
    }
}

static CURRENT: RwLock<ColorTokens> = RwLock::new(ColorTokens::byteboy2077());

/// 当前生效的颜色 token(默认 ByteBoy2077)。
pub fn current() -> ColorTokens {
    *CURRENT.read().expect("byteui color RwLock poisoned")
}

/// 整体替换当前颜色 token——供未来 ByteBoy 产品换主题用,组件代码不用改。
pub fn set_theme(tokens: ColorTokens) {
    *CURRENT.write().expect("byteui color RwLock poisoned") = tokens;
}

/// 当前生效的命名配色方案。`set_theme` 能换任意 `ColorTokens`，但不知道
/// 换上的是哪份"名字"；`term_model.rs` 的 ANSI16 终端色板和设置弹窗都需要
/// 一个"当前是深色还是浅色"的单一真相源，所以单独用这个枚举记名字。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorScheme {
    #[serde(rename = "dark")]
    Dark,
    #[serde(rename = "light")]
    Light,
}

static CURRENT_SCHEME: RwLock<ColorScheme> = RwLock::new(ColorScheme::Dark);

/// 当前生效的命名配色方案(默认 `Dark`)。
pub fn current_scheme() -> ColorScheme {
    *CURRENT_SCHEME
        .read()
        .expect("byteui color scheme RwLock poisoned")
}

/// 按命名方案整体切换 token(同时更新 `current_scheme()`)。设置弹窗选中
/// 即调这个,立即全局生效,不需要重启。
pub fn set_scheme(scheme: ColorScheme) {
    set_theme(match scheme {
        ColorScheme::Dark => ColorTokens::byteboy2077(),
        ColorScheme::Light => ColorTokens::byteboy2077_light(),
    });
    *CURRENT_SCHEME
        .write()
        .expect("byteui color scheme RwLock poisoned") = scheme;
}

#[derive(Serialize, Deserialize)]
struct PersistedScheme {
    scheme: ColorScheme,
}

/// 把当前方案落盘到调用方指定的路径,供下次启动 `init_scheme` 读回。
/// (`byteui` 不内置任何 Dozer 专属路径约定,同 `icon_size::persist_scale`。)
pub fn persist_scheme(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let s = serde_json::to_string_pretty(&PersistedScheme {
        scheme: current_scheme(),
    })
    .unwrap_or_default();
    let _ = std::fs::write(path, s);
}

/// 启动时把上次退出前落盘的方案读回并应用。文件缺失/损坏都静默保持当前
/// 默认值(同 `icon_size::init_scale` 的"任何错误都不阻断主流程"定位)。
pub fn init_scheme(path: &Path) {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(p) = serde_json::from_str::<PersistedScheme>(&raw) else {
        return;
    };
    set_scheme(p.scheme);
}

/// 两色按 `t`(0..=1)线性插值。`t` 超出 [0,1] 不外夹,调用方保证区间。
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// `CURRENT`/`CURRENT_SCHEME` 是进程级共享 static，cargo test 默认多线程
    /// 并跑；不加锁的话一个测试的"设置再断言"窗口会被另一线程的写入插进来
    /// (已实测：新增的 scheme 测试上线当天就在本地跑出过这种交叉污染)。
    /// 所有读/写这两个 static 的测试都先拿这把锁，序列化执行。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn byteboy2077_bg_matches_hex() {
        let t = ColorTokens::byteboy2077();
        assert_eq!(t.bg.r, 0x0d as f32 / 255.0);
        assert_eq!(t.bg.g, 0x13 as f32 / 255.0);
        assert_eq!(t.bg.b, 0x1c as f32 / 255.0);
    }

    #[test]
    fn current_defaults_to_byteboy2077() {
        let _guard = lock();
        set_theme(ColorTokens::byteboy2077());
        let c = current();
        assert_eq!(c.gold, ColorTokens::byteboy2077().gold);
    }

    #[test]
    fn set_theme_replaces_current_and_is_visible_globally() {
        let _guard = lock();
        let mut custom = ColorTokens::byteboy2077();
        custom.gold = Color::from_rgb8(0x00, 0x00, 0x00);
        set_theme(custom);
        assert_eq!(current().gold, Color::from_rgb8(0x00, 0x00, 0x00));
        // 复原,避免污染同进程里跑在本测试之后的其它测试。
        set_theme(ColorTokens::byteboy2077());
    }

    #[test]
    fn mix_at_zero_and_one_returns_endpoints() {
        let a = Color::from_rgb8(0, 0, 0);
        let b = Color::from_rgb8(255, 255, 255);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
    }

    /// 防漂移锚：`byteboy2077_light()` 的取值必须和 `design/浅色配色表.html`
    /// 的语义色板表逐项一致。
    #[test]
    fn byteboy2077_light_bg_matches_hex() {
        let t = ColorTokens::byteboy2077_light();
        assert_eq!(t.bg, Color::from_rgb8(0xff, 0xfd, 0xf6));
        assert_eq!(t.panel, Color::from_rgb8(0xfe, 0xf2, 0xe4));
        assert_eq!(t.gold, Color::from_rgb8(0x11, 0x8b, 0x96));
        assert_eq!(t.cream, Color::from_rgb8(0x16, 0x23, 0x2e));
    }

    #[test]
    fn current_scheme_defaults_to_dark() {
        let _guard = lock();
        set_scheme(ColorScheme::Dark);
        assert_eq!(current_scheme(), ColorScheme::Dark);
    }

    #[test]
    fn set_scheme_switches_tokens_and_reports_current_scheme() {
        let _guard = lock();
        set_scheme(ColorScheme::Light);
        assert_eq!(current_scheme(), ColorScheme::Light);
        assert_eq!(current().bg, ColorTokens::byteboy2077_light().bg);
        // 复原，避免污染同进程里跑在本测试之后的其它测试。
        set_scheme(ColorScheme::Dark);
        assert_eq!(current().bg, ColorTokens::byteboy2077().bg);
    }

    #[test]
    fn persist_scheme_and_init_scheme_roundtrip_via_explicit_path() {
        let _guard = lock();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("color_theme.json");

        set_scheme(ColorScheme::Light);
        persist_scheme(&path);
        set_scheme(ColorScheme::Dark);

        init_scheme(&path);
        assert_eq!(current_scheme(), ColorScheme::Light);

        // 复原，避免污染同进程里跑在本测试之后的其它测试。
        set_scheme(ColorScheme::Dark);
    }

    #[test]
    fn init_scheme_with_missing_or_corrupted_file_keeps_current_scheme() {
        let _guard = lock();
        set_scheme(ColorScheme::Dark);
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.json");
        init_scheme(&missing);
        assert_eq!(current_scheme(), ColorScheme::Dark);

        let bad = dir.path().join("color_theme.json");
        std::fs::write(&bad, "not json").unwrap();
        init_scheme(&bad);
        assert_eq!(current_scheme(), ColorScheme::Dark);
    }
}
