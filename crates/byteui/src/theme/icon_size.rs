//! 图标尺寸 token 化：`ByteBoy2077` 是编译期默认值，`set_theme` 可在
//! 运行时整体替换成另一份产品的取值（同 `theme::color`/`theme::font`/
//! `theme::geometry` 的模式）——组件内部一律读 `current()`，不直接引用
//! `byteboy2077()`。这里只管控件内部图标的"设计基准尺寸"，不越界。
//!
//! 本模块所有尺寸 accessor（`rail`/`row`/`chevron`/`tree_row_gap`）返回的值
//! 都已乘过 `scale()`，因此改 `scale` 即整体缩放全部图标与图标相关间距
//! （一个旋钮控制全局）；`icons::view` 是纯渲染入口，不再二次乘 scale。
//! 改 `rail/row/chevron/tree_row_gap` 则只调某类位置的相对大小。
//!
//! `scale()` 是**运行时可变**的：启动默认值取 `IconSizeTokens.scale`
//! （见下方 `current().scale`），可被环境变量 `DOZER_ICON_SCALE` 覆盖；
//! 运行时由 `set_scale` / `zoom_by`（Ctrl + / Ctrl - 快捷键入口）改写，
//! 下一帧布局即按新值重排——所有 accessor 每帧都实时读 `scale()`，不
//! 缓存缩放结果。**这套运行时缩放机制和 `IconSizeTokens`（静态设计基准
//! 尺寸）是两回事，不要混淆**：`set_theme` 换的是"设计基准值"，
//! `set_scale`/`zoom_by` 改的是"运行时倍数"，两者独立正交。
//!
//! 改过的 scale 需要**跨重启保留**：用户在会话里放大/缩小后退出，下次重开
//! 应回到退出时的 scale。`init_scale`/`persist_scale`/`reset_scale` 都改吃
//! 调用方传入的落盘路径（`byteui` 不内置任何 Dozer 专属路径约定）；环境
//! 变量 `DOZER_ICON_SCALE` 是显式覆盖，优先级高于落盘值（见 `init_scale`）。
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct IconSizeTokens {
    /// 主导航图标栏按钮(左/右 rail)与顶栏设置齿轮：16x16。
    pub rail: f32,
    /// 文件树行 / 右键菜单项 / tab 箭头 / 最大化按钮等：14x14。
    pub row: f32,
    /// 文件树展开/收起箭头：12x12（比同行文件图标略小）。
    pub chevron: f32,
    /// 面板 tab 栏翻页箭头（`<` / `>`）：10x10，比文件树 chevron 略小。
    pub tab_arrow: f32,
    /// 顶栏 "Dozer Home" tab 的品牌图标(house)：12x12。
    pub home: f32,
    /// 文件树行内"箭头↔图标"之间的间距（设计基准 2px）。
    pub tree_row_gap: f32,
    /// 设计基准缩放因子：1.0 = 设计基准；作为 `base_scale()` 的兜底值，
    /// 和运行时可变的 `CURRENT_SCALE` 是两回事。
    pub scale: f32,
}

impl IconSizeTokens {
    /// 逐一对应 `dozer-app` 当前 `assets/theme/workspace.json` 的
    /// `icon_sizes` 节点，仅作未显式 `set_theme()` 时的兜底默认值。
    pub const fn byteboy2077() -> Self {
        Self {
            rail: 16.0,
            row: 14.0,
            chevron: 12.0,
            tab_arrow: 9.0,
            home: 12.0,
            tree_row_gap: 2.0,
            scale: 1.0,
        }
    }
}

static CURRENT: RwLock<IconSizeTokens> = RwLock::new(IconSizeTokens::byteboy2077());

/// 当前生效的图标尺寸 token（默认 ByteBoy2077）。
pub fn current() -> IconSizeTokens {
    *CURRENT.read().expect("byteui icon_size RwLock poisoned")
}

/// 整体替换当前图标尺寸 token——供调用方（如 `dozer-app::theme::init()`）
/// 在启动时用自己的 `workspace.json` 覆盖默认值。**不影响**运行时缩放
/// 倍数（`CURRENT_SCALE`），那是独立机制，见模块文档。
pub fn set_theme(tokens: IconSizeTokens) {
    *CURRENT.write().expect("byteui icon_size RwLock poisoned") = tokens;
}

pub fn rail() -> f32 {
    current().rail * scale()
}
pub fn row() -> f32 {
    current().row * scale()
}
pub fn chevron() -> f32 {
    current().chevron * scale()
}
/// 面板 tab 栏翻页箭头（`<` / `>`）尺寸，已含全局 scale。
pub fn tab_arrow() -> f32 {
    current().tab_arrow * scale()
}
/// 顶栏 "Dozer Home" tab 品牌图标尺寸，已含全局 scale。
pub fn home() -> f32 {
    current().home * scale()
}
/// 文件树行内"箭头↔图标"间距，已含全局 scale。
pub fn tree_row_gap() -> f32 {
    current().tree_row_gap * scale()
}

/// 全局缩放因子的运行时当前值（逻辑像素倍数）。`u32::MAX` 是哨兵，表示
/// "尚未被运行时改写"，此时回落到 `current().scale`（token 基准值）。
static CURRENT_SCALE: AtomicU32 = AtomicU32::new(u32::MAX);

/// 启动基准 scale：`DOZER_ICON_SCALE` 环境变量优先，否则用 token 的 `scale`。
fn base_scale() -> f32 {
    std::env::var("DOZER_ICON_SCALE")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|&v| v > 0.0)
        .unwrap_or(current().scale)
}

/// 全局缩放因子：所有 token accessor 都会乘它，因此改这一个值即整体缩放
/// 全部图标与图标相关间距（一个旋钮控制全局）。运行时可变——见
/// `set_scale` / `zoom_by`。
pub fn scale() -> f32 {
    let bits = CURRENT_SCALE.load(Ordering::Relaxed);
    if bits == u32::MAX {
        base_scale()
    } else {
        f32::from_bits(bits)
    }
}

/// 把全局 scale 直接设为目标值，超出 `[SCALE_MIN, SCALE_MAX]` 会被钳制。
/// 下一帧布局自动按新值重排。
pub fn set_scale(target: f32) {
    let clamped = target.clamp(SCALE_MIN, SCALE_MAX);
    CURRENT_SCALE.store(clamped.to_bits(), Ordering::Relaxed);
}

/// 相对缩放（Ctrl + / Ctrl - 的入口）：在当前值基础上乘 `factor`。
pub fn zoom_by(factor: f32) {
    set_scale(scale() * factor);
}

/// 还原到启动基准 scale（Ctrl+1 入口）：清空运行时改写，
/// 让 `scale()` 回落到 `base_scale()`（`DOZER_ICON_SCALE` 或 token `scale`）；
/// 同时把落盘值复位成出厂默认，使"还原"在下次重启后依然生效。
pub fn reset_scale(path: &Path) {
    CURRENT_SCALE.store(u32::MAX, Ordering::Relaxed);
    save_persisted_scale(path, current().scale);
}

/// 启动时把上次退出前落盘的 scale 读回并应用为当前值,调用方传入落盘路径
/// (`byteui` 不内置任何 Dozer 专属路径约定)。`DOZER_ICON_SCALE` 环境变量
/// 是显式覆盖,优先级高于落盘值。
pub fn init_scale(path: &Path) {
    if std::env::var("DOZER_ICON_SCALE").is_ok() {
        return;
    }
    if let Some(v) = load_persisted_scale(path) {
        set_scale(v);
    }
}

/// 把当前 scale 落盘到调用方指定的路径,供下次启动 `init_scale` 读回。
pub fn persist_scale(path: &Path) {
    save_persisted_scale(path, scale());
}

#[derive(Serialize, Deserialize)]
struct PersistedScale {
    scale: f32,
}

/// 读存盘 scale：文件缺失/损坏/越界/非有限数都回落 `None`（调用方保持默认）。
fn load_persisted_scale(path: &Path) -> Option<f32> {
    load_from(path)
}

/// 写存盘 scale：目录不存在先建；任何 IO 失败静默（缩放是体验增强，不阻断
/// 主流程，同 `dozer-hook` 的"任何错误都静默"定位）。
fn save_persisted_scale(path: &Path, v: f32) {
    let _ = save_to(path, v);
}

/// 显式路径读取，供单测指向临时文件，不碰用户真实配置目录。
/// 越界值夹回 `[SCALE_MIN, SCALE_MAX]`（手改过的大数不至于让 UI 失控），
/// 非有限数/非正数视为无效，回落 `None`。
pub(crate) fn load_from(path: &Path) -> Option<f32> {
    let raw = std::fs::read_to_string(path).ok()?;
    let p: PersistedScale = serde_json::from_str(&raw).ok()?;
    if p.scale.is_finite() && p.scale > 0.0 {
        Some(p.scale.clamp(SCALE_MIN, SCALE_MAX))
    } else {
        None
    }
}

/// 显式路径写入，供单测指向临时文件，不碰用户真实配置目录。
pub(crate) fn save_to(path: &Path, v: f32) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let s = serde_json::to_string_pretty(&PersistedScale { scale: v }).unwrap_or_default();
    std::fs::write(path, s)
}

/// UI 缩放允许的下限/上限（逻辑像素倍数），防止缩到不可读或放到失控。
pub const SCALE_MIN: f32 = 0.5;
pub const SCALE_MAX: f32 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚：`byteboy2077()` 的每个字段值必须和 `dozer-app` 当前
    /// `assets/theme/workspace.json` 的 `icon_sizes` 字面量一致。
    #[test]
    fn byteboy2077_matches_dozer_app_baseline() {
        let t = IconSizeTokens::byteboy2077();
        assert_eq!(t.rail, 16.0);
        assert_eq!(t.row, 14.0);
        assert_eq!(t.chevron, 12.0);
        assert_eq!(t.tab_arrow, 9.0);
        assert_eq!(t.home, 12.0);
        assert_eq!(t.tree_row_gap, 2.0);
        assert_eq!(t.scale, 1.0);
    }

    #[test]
    fn current_defaults_to_byteboy2077() {
        assert_eq!(current().rail, IconSizeTokens::byteboy2077().rail);
    }

    #[test]
    fn set_theme_replaces_current_and_is_visible_globally() {
        let mut custom = IconSizeTokens::byteboy2077();
        custom.rail = 999.0;
        set_theme(custom);
        assert_eq!(current().rail, 999.0);
        // 复原，避免污染同进程里跑在本测试之后的其它测试。
        set_theme(IconSizeTokens::byteboy2077());
    }

    #[test]
    fn accessors_reflect_current_at_default_scale() {
        set_theme(IconSizeTokens::byteboy2077());
        CURRENT_SCALE.store(u32::MAX, Ordering::Relaxed);
        assert_eq!(rail(), 16.0);
        assert_eq!(row(), 14.0);
        assert_eq!(chevron(), 12.0);
        assert_eq!(tab_arrow(), 9.0);
    }

    /// 落盘 round-trip：写出去的值读回来和写的一致，且会夹进合法范围。
    #[test]
    fn persisted_scale_round_trips_and_clamps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui_scale.json");

        save_to(&path, 1.5).unwrap();
        assert_eq!(load_from(&path), Some(1.5));

        // 越界值落盘后被夹回合法范围（写入时不夹，读取时夹）。
        save_to(&path, 99.0).unwrap();
        assert_eq!(load_from(&path), Some(SCALE_MAX));
        save_to(&path, 0.01).unwrap();
        assert_eq!(load_from(&path), Some(SCALE_MIN));
    }

    /// 损坏/缺失/非有限数文件都回落 `None`，不污染默认 scale。
    #[test]
    fn corrupted_or_missing_scale_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.json");
        assert_eq!(load_from(&missing), None);

        let bad = dir.path().join("ui_scale.json");
        std::fs::write(&bad, "not json").unwrap();
        assert_eq!(load_from(&bad), None);

        std::fs::write(&bad, r#"{"scale": "x"}"#).unwrap();
        assert_eq!(load_from(&bad), None);
    }

    /// 用临时目录里的 `ui_scale.json` 当落盘路径，隔离用户真实配置目录；
    /// 跑完复位运行时改写，避免污染同进程里之后的测试。
    fn with_temp_scale_file<F: FnOnce(&Path)>(f: F) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui_scale.json");
        f(&path);
        CURRENT_SCALE.store(u32::MAX, Ordering::Relaxed);
    }

    /// `init_scale` 把落盘值应用成当前 scale；`reset_scale` 清除运行时改写
    /// 并落盘复位成出厂默认。两者都吃显式临时路径，不再碰用户真实配置目录。
    #[test]
    fn init_applies_persisted_and_reset_clears_it() {
        if std::env::var("DOZER_ICON_SCALE").is_ok() {
            return;
        }
        with_temp_scale_file(|path| {
            save_to(path, 2.0).unwrap();
            init_scale(path);
            assert_eq!(scale(), 2.0);

            reset_scale(path);
            assert_eq!(scale(), current().scale);
            assert_eq!(load_from(path), Some(current().scale));
        });
    }
}
