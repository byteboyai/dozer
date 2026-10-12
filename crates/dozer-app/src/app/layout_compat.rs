//! Dozer 历史的 `layout.json` / `rail_layout` → 共享 `Layout` 的兼容桥
//! (Task 10,计划 clause 157/160)。
//!
//! **薄包装**:Dozer 只提供"`PanelKind` 枚举 ↔ 稳定 `PanelId`"的映射与默认
//! 栏宽;通用迁移流程(版本闸门 / 未知保留 / 原子落盘 + 备份)在
//! `bytehost_desktop::layout`。
//!
//! 稳定 id 命名:`dozer.<slug>`(如 `dozer.files`、`dozer.git-log`);应用面板
//! 沿用现有 `app:<validated-id>`(`PanelId::for_app`)。历史 token 就是
//! `PanelKind` 的 serde 形态(`"Files"`、`"app:<id>"`),见 `app/state.rs`。
//!
//! **接线状态:** 本模块是 Task 10 计划的"Dozer 先以薄包装接入"步骤(clause 160),
//! 尚未接入 app 启动/落盘路径——真实接入需在真实窗口上做布局回归,单列后续提交。
//! 其余条目经 `#[cfg(test)]` 的黄金样本测试验证,故此处暂封 `dead_code`。
#![allow(dead_code)]

use bytehost_core::PanelId;
use bytehost_desktop::layout::LegacyPanelAdapter;

use crate::app::{AppSlot, PanelKind};
use crate::chrome::rail::RailLayout;

/// 内置面板名 → 稳定 `PanelId`(`dozer.<slug>`)。slug 是小写 kebab(见
/// `PanelId` 的字符约束:仅小写字母/数字/`-`/`_`,且每段以字母开头)。
/// 与 `app/state.rs` 的 `BUILTIN_NAMES`(落盘 token)一一对应但不相同:
/// token 是 PascalCase,id slug 是小写 kebab。
pub(crate) fn panel_id_for(kind: PanelKind) -> PanelId {
    match kind {
        PanelKind::App(slot) => PanelId::for_app(slot.id())
            .expect("AppSlot::id 已由 AppSlot::intern 校验,必为合法 app: id"),
        builtin => PanelId::new(format!("dozer.{}", builtin_slug(builtin)))
            .expect("内置面板 slug + dozer. 前缀是合法 id"),
    }
}

/// 内置 `PanelKind` → 稳定 id 的 slug(小写 kebab)。
fn builtin_slug(kind: PanelKind) -> &'static str {
    match kind {
        PanelKind::Files => "files",
        PanelKind::GitLog => "git-log",
        PanelKind::Todo => "todo",
        PanelKind::Project => "project",
        PanelKind::Database => "database",
        PanelKind::Ssh => "ssh",
        PanelKind::Web => "web",
        PanelKind::Agent => "agent",
        PanelKind::GroupChat => "group-chat",
        PanelKind::Conversations => "conversations",
        PanelKind::Usage => "usage",
        PanelKind::CodeHealth => "code-health",
        PanelKind::App(_) => unreachable!("应用面板走 app: 命名空间,不经 slug"),
    }
}

/// slug(`files`)→ 内置 `PanelKind`。
fn builtin_slug_to_kind(slug: &str) -> Option<PanelKind> {
    const ALL: [PanelKind; 12] = [
        PanelKind::Files,
        PanelKind::GitLog,
        PanelKind::Todo,
        PanelKind::Project,
        PanelKind::Database,
        PanelKind::Ssh,
        PanelKind::Web,
        PanelKind::Agent,
        PanelKind::GroupChat,
        PanelKind::Conversations,
        PanelKind::Usage,
        PanelKind::CodeHealth,
    ];
    ALL.into_iter().find(|k| builtin_slug(*k) == slug)
}

/// 稳定 `PanelId` → 内置 `PanelKind`(`dozer.` 命名空间)。应用面板(`app:`)
/// 不经此路,由 `app_slot_for` 处理。
pub(crate) fn builtin_for_panel_id(id: &PanelId) -> Option<PanelKind> {
    let slug = id.as_str().strip_prefix("dozer.")?;
    builtin_slug_to_kind(slug)
}

/// `app:<id>` 稳定 id → `PanelKind::App`。
pub(crate) fn app_slot_for(id: &PanelId) -> Option<PanelKind> {
    let raw = id.as_str().strip_prefix("app:")?;
    AppSlot::intern(raw).map(PanelKind::App)
}

/// 任一稳定 `PanelId` → `PanelKind`(内置或应用)。
pub(crate) fn panel_kind_for(id: &PanelId) -> Option<PanelKind> {
    builtin_for_panel_id(id).or_else(|| app_slot_for(id))
}

/// Dozer 历史栏 token(`"Files"` / `"app:<id>"`)→ 稳定 `PanelId`。
/// 未知/已下线 token 返回 `None`(保留供提示)。
pub(crate) fn token_to_panel_id(token: &str) -> Option<PanelId> {
    if let Some(raw) = token.strip_prefix("app:") {
        let _ = AppSlot::intern(raw)?;
        return PanelId::for_app(raw).ok();
    }
    let kind = crate::app::builtin_kind_by_token(token)?;
    Some(panel_id_for(kind))
}

/// Dozer 的 `LegacyPanelAdapter`:从历史 `layout.json` 切出
/// `rail_layout.left/right` 两栏的 token 序列。
pub(crate) struct DozerLayoutAdapter {
    pub rail_width: f32,
}

impl LegacyPanelAdapter for DozerLayoutAdapter {
    fn rails_from_legacy(&self, legacy: &serde_json::Value) -> Vec<Vec<String>> {
        let side = |key: &str| -> Vec<String> {
            legacy
                .get("rail_layout")
                .and_then(|r| r.get(key))
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        vec![side("left"), side("right")]
    }

    fn map_token(&self, token: &str) -> Option<PanelId> {
        token_to_panel_id(token)
    }

    fn default_rail_width(&self) -> f32 {
        self.rail_width
    }
}

/// 把当前 `RailLayout` 转成产品的默认 `Layout`(`from_default` 输入用)。
/// 这是"产品负责默认布局"(计划 clause 157)的落点:栏序来自 `RailLayout::default()`
/// (即 `panel_registry::catalog().default_rail()`)。
pub(crate) fn default_layout(rail: &RailLayout, rail_width: f32) -> bytehost_core::Layout {
    let to_ids = |kinds: &[PanelKind]| kinds.iter().copied().map(panel_id_for).collect();
    bytehost_core::Layout::from_default(vec![
        bytehost_core::Rail::new(to_ids(&rail.left), rail_width),
        bytehost_core::Rail::new(to_ids(&rail.right), rail_width),
    ])
    .expect("默认栏宽为有限数")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytehost_desktop::layout::{MigrateError, migrate_legacy_layout};

    fn adapter() -> DozerLayoutAdapter {
        DozerLayoutAdapter { rail_width: 300.0 }
    }

    /// 黄金样本(1):Dozer 现役 `layout.json` 形状——`window_width`/`window_height`
    /// + `rail_layout{left,right}`,元素是内置面板名。期望:两栏 token 各映射到
    /// `dozer.<name>` 稳定 id,顺序保持。
    #[test]
    fn golden_current_layout_json_maps_builtin_rails_in_order() {
        let legacy = serde_json::json!({
            "window_width": 1600.0,
            "window_height": 1000.0,
            "rail_layout": {
                "left": ["Project", "Todo", "Files"],
                "right": ["Agent", "Usage", "CodeHealth"]
            }
        });
        let out = migrate_legacy_layout(&legacy, &adapter()).unwrap();
        assert_eq!(out.layout.rails.len(), 2);
        assert_eq!(
            out.layout.rails[0].items,
            vec![
                PanelId::new("dozer.project").unwrap(),
                PanelId::new("dozer.todo").unwrap(),
                PanelId::new("dozer.files").unwrap(),
            ]
        );
        assert_eq!(
            out.layout.rails[1].items,
            vec![
                PanelId::new("dozer.agent").unwrap(),
                PanelId::new("dozer.usage").unwrap(),
                PanelId::new("dozer.code-health").unwrap(),
            ]
        );
        assert!(out.retained.is_empty());
    }

    /// 黄金样本(2):应用面板 token `app:<id>` → `PanelId::for_app`。
    #[test]
    fn golden_app_token_maps_to_app_namespace() {
        let legacy = serde_json::json!({
            "rail_layout": { "left": ["Files"], "right": ["app:excalidraw"] }
        });
        let out = migrate_legacy_layout(&legacy, &adapter()).unwrap();
        assert_eq!(
            out.layout.rails[1].items,
            vec![PanelId::for_app("excalidraw").unwrap()]
        );
    }

    /// 黄金样本(3):老 `layout.json` 里已迁移走的尺寸字段被忽略,缺失的
    /// `rail_layout` 退化成空栏(窗口仍能开,由默认布局在别处补)。
    #[test]
    fn golden_legacy_dims_only_yields_empty_rails() {
        let legacy = serde_json::json!({
            "left_width": 500.0,
            "files_split": 0.35,
            "project_split": 0.4
        });
        let out = migrate_legacy_layout(&legacy, &adapter()).unwrap();
        assert_eq!(out.layout.rails.len(), 2);
        assert!(out.layout.rails.iter().all(|r| r.items.is_empty()));
    }

    /// 更高的 schema 版本 → 拒绝覆盖(防降级破坏)。
    #[test]
    fn golden_newer_schema_is_rejected() {
        let legacy = serde_json::json!({ "schema_version": 99, "rail_layout": {} });
        assert_eq!(
            migrate_legacy_layout(&legacy, &adapter()),
            Err(MigrateError::UnsupportedNewerVersion {
                found: 99,
                supported: bytehost_core::LAYOUT_SCHEMA_VERSION
            })
        );
    }

    /// 已下线 token(既非内置名,也非合法 app id)→ 保留进 `retained`,不丢。
    #[test]
    fn golden_unknown_token_is_retained() {
        let legacy = serde_json::json!({
            "rail_layout": { "left": ["Files", "GoneV1"], "right": [] }
        });
        let out = migrate_legacy_layout(&legacy, &adapter()).unwrap();
        assert_eq!(out.retained, vec!["GoneV1"]);
        assert_eq!(
            out.layout.rails[0].items,
            vec![PanelId::new("dozer.files").unwrap()]
        );
    }

    /// 双向映射:`panel_id_for` 与 `panel_kind_for` 对全部内置面板互逆。
    #[test]
    fn panel_id_and_kind_round_trip_for_all_builtins() {
        for kind in [
            PanelKind::Files,
            PanelKind::GitLog,
            PanelKind::Todo,
            PanelKind::Project,
            PanelKind::Database,
            PanelKind::Ssh,
            PanelKind::Web,
            PanelKind::Agent,
            PanelKind::GroupChat,
            PanelKind::Conversations,
            PanelKind::Usage,
            PanelKind::CodeHealth,
        ] {
            let id = panel_id_for(kind);
            assert_eq!(panel_kind_for(&id), Some(kind), "{kind:?} 往返失败");
        }
    }

    /// 默认布局:栏序沿用 `RailLayout::default()`,id 为稳定形态。
    #[test]
    fn default_layout_mirrors_default_rail_order() {
        let rail = RailLayout::default();
        let layout = default_layout(&rail, 300.0);
        assert_eq!(layout.rails.len(), 2);
        assert_eq!(layout.rails[0].items.len(), rail.left.len());
        assert_eq!(layout.rails[1].items.len(), rail.right.len());
        assert_eq!(layout.rails[0].items[0], panel_id_for(rail.left[0]));
    }
}
