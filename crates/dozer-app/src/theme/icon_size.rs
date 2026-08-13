//! 图标尺寸 token 化：`workspace.rs` 里散落的 `icons::view(kind, N.0, ...)`
//! 字面量收敛成具名 token + 一个全局 `scale`，编译期内嵌
//! `assets/theme/workspace.json` 的 `icon_sizes` 节点，启动时解析一次。
//! 与 `workspace_font.rs`(字号) / `chrome_style.rs`(区域样式) 职责分离——
//! 这里只管控件内部图标的"设计基准尺寸"与"整体缩放因子"，不越界。
//!
//! 本模块所有尺寸 accessor（`rail`/`row`/`chevron`/`tree_row_gap`）返回的值
//! 都已乘过 `scale()`，因此改 `scale` 即整体缩放全部图标与图标相关间距
//! （一个旋钮控制全局）；`icons::view` 是纯渲染入口，不再二次乘 scale。
//! 改 `rail/row/chevron/tree_row_gap` 则只调某类位置的相对大小。解析失败
//! （格式错误、缺字段）直接 panic：开发期配置错误，不是需要优雅降级的
//! 运行时数据（同 `workspace_font.rs` 定位）。
//!
//! `scale()` 是**运行时可变**的：启动默认值取 `workspace.json` 的
//! `icon_sizes.scale`，可被环境变量 `DOZER_ICON_SCALE` 覆盖；运行时由
//! `set_scale` / `zoom_by`（Ctrl + / Ctrl - 快捷键入口）改写，下一帧布局
//! 即按新值重排——所有 accessor 每帧都实时读 `scale()`，不缓存缩放结果。
//!
//! 改过的 scale 需要**跨重启保留**：用户在会话里放大/缩小后退出，下次重开
//! `dozer` 应回到退出时的 scale。`init_scale` 在建窗前把上次退出前落盘的
//! 值读回应用；`persist_scale` 在每次缩放后把当前值写进
//! `dozer_core::paths::config_dir()/ui_scale.json`；`reset_scale`（Ctrl+1）
//! 同时把落盘值复位成出厂默认，保证"还原"在重启后仍生效。环境变量
//! `DOZER_ICON_SCALE` 是显式覆盖，优先级高于落盘值（见 `init_scale`）。
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU32, Ordering};

const RAW: &str = include_str!("../../assets/theme/workspace.json");

#[derive(Deserialize)]
struct IconSizes {
    /// 主导航图标栏按钮(左/右 rail)与顶栏设置齿轮：16x16。
    rail: f32,
    /// 文件树行 / 右键菜单项 / tab 箭头 / 最大化按钮等：14x14。
    row: f32,
    /// 文件树展开/收起箭头：12x12（比同行文件图标略小）。
    chevron: f32,
    /// 面板 tab 栏翻页箭头（`<` / `>`）：10x10，比文件树 chevron 略小，
    /// 让翻页箭头在密集的 tab 行里更精致、不抢标题视觉。
    tab_arrow: f32,
    /// 顶栏 "Dozer Home" tab 的品牌图标(house)：12x12，比 rail 略小以让
    /// 字标更聚焦。
    home: f32,
    /// 文件树行内"箭头↔图标"之间的间距（设计基准 2px）。
    tree_row_gap: f32,
    /// 全局缩放因子：1.0 = 设计基准；调到 1.5 即全部图标放大 50%。
    scale: f32,
}

/// `workspace.json` 顶层结构里本模块只关心的部分——`regions` / `font_sizes`
/// / `geometry` 节点是别处地盘，这里不声明，serde 默认忽略未知字段。
#[derive(Deserialize)]
struct RawWorkspaceFile {
    icon_sizes: IconSizes,
}

fn load(raw: &str) -> IconSizes {
    let file: RawWorkspaceFile =
        serde_json::from_str(raw).expect("workspace.json 格式错误(解析失败,icon_sizes 节点)");
    file.icon_sizes
}

static SIZES: LazyLock<IconSizes> = LazyLock::new(|| load(RAW));

pub fn rail() -> f32 {
    SIZES.rail * scale()
}
pub fn row() -> f32 {
    SIZES.row * scale()
}
pub fn chevron() -> f32 {
    SIZES.chevron * scale()
}
/// 面板 tab 栏翻页箭头（`<` / `>`）尺寸，已含全局 scale。比文件树
/// `chevron` 略小。
pub fn tab_arrow() -> f32 {
    SIZES.tab_arrow * scale()
}
/// 顶栏 "Dozer Home" tab 品牌图标尺寸，已含全局 scale。
pub fn home() -> f32 {
    SIZES.home * scale()
}
/// 文件树行内"箭头↔图标"间距，已含全局 scale。
pub fn tree_row_gap() -> f32 {
    SIZES.tree_row_gap * scale()
}
/// 全局缩放因子的运行时当前值（逻辑像素倍数）。`u32::MAX` 是哨兵，表示
/// "尚未被运行时改写"，此时回落到 `SIZES.scale`（JSON/环境变量基准值）。
static CURRENT_SCALE: AtomicU32 = AtomicU32::new(u32::MAX);

/// 启动基准 scale：`DOZER_ICON_SCALE` 环境变量优先，否则用 JSON 的 `scale`。
fn base_scale() -> f32 {
    std::env::var("DOZER_ICON_SCALE")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|&v| v > 0.0)
        .unwrap_or(SIZES.scale)
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
/// 让 `scale()` 回落到 `base_scale()`（`DOZER_ICON_SCALE` 或 JSON `scale`）；
/// 同时把落盘值复位成出厂默认，使"还原"在下次重启后依然生效。
pub fn reset_scale() {
    CURRENT_SCALE.store(u32::MAX, Ordering::Relaxed);
    save_persisted_scale(SIZES.scale);
}

/// 启动时把上次退出前落盘的 scale 读回并应用为当前值。在 `main()` 建窗前
/// 调用一次，确保首帧几何按存盘 scale 排布。环境变量 `DOZER_ICON_SCALE`
/// 是显式覆盖，优先级高于落盘值——设了就跳过存盘值，让 `scale()` 回落到
/// 环境变量（运行时改写仍走 `set_scale`）。没有/非法存盘值时保持出厂默认。
pub fn init_scale() {
    if std::env::var("DOZER_ICON_SCALE").is_ok() {
        return;
    }
    if let Some(v) = load_persisted_scale() {
        set_scale(v);
    }
}

/// 把当前 scale 落盘，供下次启动 `init_scale` 读回。Ctrl +/- 缩放后调用。
pub fn persist_scale() {
    save_persisted_scale(scale());
}

const SCALE_FILE_NAME: &str = "ui_scale.json";

#[derive(Serialize, Deserialize)]
struct PersistedScale {
    scale: f32,
}

fn scale_path() -> PathBuf {
    dozer_core::paths::config_dir().join(SCALE_FILE_NAME)
}

/// 读存盘 scale：文件缺失/损坏/越界/非有限数都回落 `None`（调用方保持默认）。
fn load_persisted_scale() -> Option<f32> {
    load_from(&scale_path())
}

/// 写存盘 scale：目录不存在先建；任何 IO 失败静默（缩放是体验增强，不阻断
/// 主流程，同 `dozer-hook` 的"任何错误都静默"定位）。
fn save_persisted_scale(v: f32) {
    let _ = save_to(&scale_path(), v);
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

    /// 防漂移锚:3 个 token + scale 的解析结果必须和改动前 workspace.rs 里
    /// 的字面量完全一致——纯代码搬家,数值不该变。
    #[test]
    fn tokens_match_pre_migration_literals() {
        assert_eq!(rail(), 16.0);
        assert_eq!(row(), 14.0);
        assert_eq!(chevron(), 12.0);
        assert_eq!(tab_arrow(), 9.0);
        assert_eq!(scale(), 1.0);
    }

    #[test]
    #[should_panic(expected = "workspace.json 格式错误")]
    fn malformed_json_panics() {
        load(r#"{"icon_sizes": {"rail": 16.0}}"#);
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

    /// 备份/还原真实 config_dir/ui_scale.json，避免单测污染用户真实偏好。
    fn with_real_scale_file<F: FnOnce()>(f: F) {
        let real = scale_path();
        let backup = real.with_extension("bak");
        let had = real.exists();
        if had {
            let _ = std::fs::copy(&real, &backup);
        }
        f();
        if had {
            let _ = std::fs::copy(&backup, &real);
            let _ = std::fs::remove_file(&backup);
        } else {
            let _ = std::fs::remove_file(&real);
        }
        CURRENT_SCALE.store(u32::MAX, Ordering::Relaxed);
    }

    /// `init_scale` 把落盘值应用成当前 scale；`reset_scale` 清除运行时改写
    /// 并落盘复位成出厂默认。两者都操作真实 config_dir，故用备份还原包裹。
    #[test]
    fn init_applies_persisted_and_reset_clears_it() {
        // 显式覆盖下 `init_scale` 会跳过落盘值，此时本测试路径不适用。
        if std::env::var("DOZER_ICON_SCALE").is_ok() {
            return;
        }
        with_real_scale_file(|| {
            save_to(&scale_path(), 2.0).unwrap();
            init_scale();
            assert_eq!(scale(), 2.0);

            reset_scale();
            assert_eq!(scale(), SIZES.scale);
            assert_eq!(load_from(&scale_path()), Some(SIZES.scale));
        });
    }
}
