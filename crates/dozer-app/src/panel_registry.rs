//! 面板注册清单(bytehost H7):host 只认"清单"——有哪些面板、各自的标题/图标、默认放哪一栏、
//! 什么顺序——而清单本身由**产品组合根**(Dozer 见 `product.rs`)在启动时给出。
//!
//! 默认栏位与默认顺序属于产品(同一个面板在 Dozer 与 Digger 里的默认位置本来就不同),所以描述符
//! (`PanelDescriptor`)里只放面板自身的属性,默认布局是 `PanelCatalog` 的另一份输入。"恰好这些面板"
//! 的布局校验由清单驱动,不再写死"恰 12 个"。
//!
//! 本模块**不**做的事(留给 H8):`PanelKind` 仍是封闭枚举,每面板展开的状态字段与 `Message` 包装
//! 不动;切入钩子的分发(`fire_panel_switch_in`)仍是 host 的 `match`——各面板钩子的状态参数与消息
//! 类型各不相同,没有统一函数签名可以放进清单。

use std::sync::OnceLock;

use byteui::interaction::icons::IconKind;

use crate::app::{PanelKind, Side};
use crate::chrome::rail::RailLayout;

/// 一个面板自身的属性(不含默认栏位——那是产品的布局偏好)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PanelDescriptor {
    pub id: PanelKind,
    /// 图标栏 tooltip 文案。
    pub title: &'static str,
    pub icon: IconKind,
}

/// 产品给 host 的面板清单:描述符 + 默认布局 + 旧版落盘布局的迁移钩子。
pub(crate) struct PanelCatalog {
    descriptors: Vec<PanelDescriptor>,
    default_left: Vec<PanelKind>,
    default_right: Vec<PanelKind>,
    migrate_legacy: fn(RailLayout) -> RailLayout,
}

impl PanelCatalog {
    /// 校验后构造:描述符不重复且非空;默认布局两栏都非空、不重复、只含已注册面板,并且恰好覆盖全部
    /// 已注册面板(默认布局本身就是一个合法布局)。不满足返回说明原因的错误。
    pub(crate) fn new(
        descriptors: Vec<PanelDescriptor>,
        default_left: Vec<PanelKind>,
        default_right: Vec<PanelKind>,
        migrate_legacy: fn(RailLayout) -> RailLayout,
    ) -> Result<Self, String> {
        if descriptors.is_empty() {
            return Err("面板清单为空".into());
        }
        let mut ids: Vec<PanelKind> = descriptors.iter().map(|d| d.id).collect();
        ids.sort_by_key(|k| format!("{k:?}"));
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err("面板描述符里有重复的面板".into());
        }
        let catalog = Self {
            descriptors,
            default_left,
            default_right,
            migrate_legacy,
        };
        if !catalog.accepts(&catalog.default_rail()) {
            return Err(
                "默认布局不合法:两栏都要非空,且恰好覆盖全部已注册面板、不重复、不含未注册面板"
                    .into(),
            );
        }
        Ok(catalog)
    }

    /// 已注册的面板数。
    pub(crate) fn len(&self) -> usize {
        self.descriptors.len()
    }

    pub(crate) fn descriptor(&self, id: PanelKind) -> Option<&PanelDescriptor> {
        self.descriptors.iter().find(|d| d.id == id)
    }

    /// 产品给的默认布局(两栏各自的面板与顺序)。
    pub(crate) fn default_rail(&self) -> RailLayout {
        RailLayout {
            left: self.default_left.clone(),
            right: self.default_right.clone(),
        }
    }

    /// 面板默认挂在哪条栏;没注册的面板返回 `None`。
    pub(crate) fn default_side(&self, id: PanelKind) -> Option<Side> {
        if matches!(id, PanelKind::App(_)) {
            return Some(APP_DEFAULT_SIDE);
        }
        if self.default_left.contains(&id) {
            Some(Side::Left)
        } else if self.default_right.contains(&id) {
            Some(Side::Right)
        } else {
            None
        }
    }

    /// 旧版落盘布局的迁移(产品自己的历史,由组合根提供)。
    pub(crate) fn migrate_legacy(&self, rail: RailLayout) -> RailLayout {
        (self.migrate_legacy)(rail)
    }

    /// `rail` 是不是一个合法布局:**内置面板**两栏各至少一个,且合计恰好是全部已注册面板(每个一次,
    /// 不多不少)。应用条目(`PanelKind::App`)不在清单里、数量随安装变化,所以不计入这些检查——
    /// 只要求它们不重复;它们与已安装集合的对齐由 `RailLayout::sync_apps` 负责。
    pub(crate) fn accepts(&self, rail: &RailLayout) -> bool {
        let builtin = |panels: &[PanelKind]| -> Vec<PanelKind> {
            panels
                .iter()
                .copied()
                .filter(|k| !matches!(k, PanelKind::App(_)))
                .collect()
        };
        let (left, right) = (builtin(&rail.left), builtin(&rail.right));
        if left.is_empty() || right.is_empty() {
            return false;
        }
        let mut apps: Vec<PanelKind> = rail
            .left
            .iter()
            .chain(rail.right.iter())
            .copied()
            .filter(|k| matches!(k, PanelKind::App(_)))
            .collect();
        apps.sort_by_key(|k| format!("{k:?}"));
        if apps.windows(2).any(|w| w[0] == w[1]) {
            return false;
        }
        let mut all: Vec<PanelKind> = left.into_iter().chain(right).collect();
        all.sort_by_key(|k| format!("{k:?}"));
        if all.windows(2).any(|w| w[0] == w[1]) {
            return false;
        }
        all.len() == self.len() && all.iter().all(|k| self.descriptor(*k).is_some())
    }
}

/// 新安装的应用在图标栏里默认挂的栏(A3 固定左栏;要做成产品可配置再提进 `PanelCatalog`)。
pub(crate) const APP_DEFAULT_SIDE: Side = Side::Left;

static CATALOG: OnceLock<PanelCatalog> = OnceLock::new();

/// 启动时由组合根调用一次(`main()` 在任何布局读取之前)。已经装过则返回错误,不覆盖。
pub(crate) fn install(catalog: PanelCatalog) -> Result<(), &'static str> {
    CATALOG.set(catalog).map_err(|_| "面板清单已经注册过")
}

/// 当前产品的面板清单。生产里必须已 `install`;测试里自动装 Dozer 的清单。
pub(crate) fn catalog() -> &'static PanelCatalog {
    #[cfg(test)]
    {
        CATALOG.get_or_init(crate::product::dozer_catalog)
    }
    #[cfg(not(test))]
    {
        CATALOG
            .get()
            .expect("面板清单未注册:main() 必须在任何布局读取之前调用 panel_registry::install")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{PanelKind, Side};
    use crate::chrome::rail::RailLayout;
    use byteui::interaction::icons::IconKind;

    fn desc(id: PanelKind) -> PanelDescriptor {
        PanelDescriptor {
            id,
            title: "t",
            icon: IconKind::Globe,
        }
    }

    fn keep(rail: RailLayout) -> RailLayout {
        rail
    }

    /// Digger 式的 3 面板小清单:Files、Todo 在左,Agent 在右。
    fn small() -> PanelCatalog {
        PanelCatalog::new(
            vec![
                desc(PanelKind::Files),
                desc(PanelKind::Todo),
                desc(PanelKind::Agent),
            ],
            vec![PanelKind::Todo, PanelKind::Files],
            vec![PanelKind::Agent],
            keep,
        )
        .unwrap()
    }

    fn rail(left: &[PanelKind], right: &[PanelKind]) -> RailLayout {
        RailLayout {
            left: left.to_vec(),
            right: right.to_vec(),
        }
    }

    #[test]
    fn default_rail_and_sides_follow_the_composition_roots_layout_not_the_enum() {
        let c = small();
        assert_eq!(c.len(), 3);
        assert_eq!(
            c.default_rail(),
            rail(&[PanelKind::Todo, PanelKind::Files], &[PanelKind::Agent])
        );
        assert_eq!(c.default_side(PanelKind::Files), Some(Side::Left));
        assert_eq!(c.default_side(PanelKind::Agent), Some(Side::Right));
        assert_eq!(
            c.default_side(PanelKind::Web),
            None,
            "没注册的面板没有默认栏"
        );
        assert!(c.descriptor(PanelKind::Todo).is_some());
        assert!(c.descriptor(PanelKind::Web).is_none());
    }

    #[test]
    fn accepts_exactly_the_registered_panels_in_any_arrangement() {
        use PanelKind::*;
        let c = small();
        assert!(c.accepts(&rail(&[Todo, Files], &[Agent])));
        assert!(c.accepts(&rail(&[Agent], &[Files, Todo])), "换栏换序都合法");
        assert!(!c.accepts(&rail(&[Todo, Files], &[])), "任一栏为空不合法");
        assert!(!c.accepts(&rail(&[], &[Todo, Files, Agent])));
        assert!(!c.accepts(&rail(&[Todo], &[Agent])), "缺一个面板");
        assert!(!c.accepts(&rail(&[Todo, Todo], &[Agent])), "重复");
        assert!(
            !c.accepts(&rail(&[Todo, Files, Web], &[Agent])),
            "多出清单之外的面板"
        );
    }

    #[test]
    fn new_rejects_inconsistent_catalogs() {
        use PanelKind::*;
        let go = |descs: Vec<PanelKind>, l: Vec<PanelKind>, r: Vec<PanelKind>| {
            PanelCatalog::new(descs.into_iter().map(desc).collect(), l, r, keep).err()
        };
        assert!(go(vec![Files, Agent], vec![Files], vec![Agent]).is_none());
        assert!(go(vec![], vec![], vec![]).is_some(), "空清单");
        assert!(
            go(vec![Files, Files, Agent], vec![Files], vec![Agent]).is_some(),
            "描述重复"
        );
        assert!(
            go(vec![Files, Agent], vec![], vec![Files, Agent]).is_some(),
            "左栏为空"
        );
        assert!(
            go(vec![Files, Agent], vec![Files, Agent], vec![]).is_some(),
            "右栏为空"
        );
        assert!(
            go(vec![Files, Agent], vec![Files], vec![Files, Agent]).is_some(),
            "默认布局里重复"
        );
        assert!(
            go(vec![Files, Agent], vec![Files], vec![Web]).is_some(),
            "默认布局里有未注册面板"
        );
        assert!(
            go(vec![Files, Agent, Todo], vec![Files], vec![Agent]).is_some(),
            "注册了却没放进默认布局"
        );
    }

    #[test]
    fn migrate_legacy_is_the_composition_roots_hook() {
        fn to_default(_: RailLayout) -> RailLayout {
            rail(&[PanelKind::Files], &[PanelKind::Agent])
        }
        let c = PanelCatalog::new(
            vec![desc(PanelKind::Files), desc(PanelKind::Agent)],
            vec![PanelKind::Files],
            vec![PanelKind::Agent],
            to_default,
        )
        .unwrap();
        let out = c.migrate_legacy(rail(&[PanelKind::Web], &[]));
        assert_eq!(out, rail(&[PanelKind::Files], &[PanelKind::Agent]));
    }

    /// 全局清单只能装一次(第二次装返回错误,不覆盖)。测试里 `catalog()` 已自动装好 Dozer 清单。
    #[test]
    fn install_is_one_shot() {
        let _ = catalog();
        assert!(install(small()).is_err());
        assert_eq!(catalog().len(), 12, "没被覆盖");
    }

    /// `PanelKind` 新增变体时这里必须同步:穷举 match 没有通配,漏了编译不过。
    fn every_kind() -> Vec<PanelKind> {
        use PanelKind::*;
        let all = [
            Files,
            GitLog,
            Todo,
            Project,
            Database,
            Ssh,
            Web,
            Agent,
            GroupChat,
            Conversations,
            Usage,
            CodeHealth,
        ];
        for k in all {
            match k {
                Files | GitLog | Todo | Project | Database | Ssh | Web | Agent | GroupChat
                | Conversations | Usage | CodeHealth | App(_) => {}
            }
        }
        all.to_vec()
    }

    #[test]
    fn dozer_catalog_registers_every_panel_kind_exactly_once() {
        let c = crate::product::dozer_catalog();
        assert_eq!(c.len(), every_kind().len());
        for k in every_kind() {
            assert!(c.descriptor(k).is_some(), "{k:?} 没注册");
            assert!(c.default_side(k).is_some(), "{k:?} 不在默认布局里");
        }
    }

    #[test]
    fn accepts_ignores_app_entries_but_rejects_duplicates_among_them() {
        use PanelKind::*;
        let c = small();
        let a = PanelKind::App(crate::app::AppSlot::intern("reg-app").unwrap());
        assert!(
            c.accepts(&rail(&[Todo, Files, a], &[Agent])),
            "应用条目不计入清单"
        );
        assert!(c.accepts(&rail(&[Todo, Files], &[Agent, a])));
        assert!(
            !c.accepts(&rail(&[Todo, Files, a], &[Agent, a])),
            "同一个应用出现两次"
        );
        assert!(
            !c.accepts(&rail(&[a], &[Todo, Files, Agent])),
            "左栏只有应用、没有内置面板"
        );
        assert_eq!(c.default_side(a), Some(APP_DEFAULT_SIDE));
        assert!(c.descriptor(a).is_none(), "应用不在描述符里");
    }
}
