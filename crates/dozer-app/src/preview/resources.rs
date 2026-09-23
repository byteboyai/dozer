//! 跨项目的预览资源管理器(文件预览重构 Phase C Task 4)。
//!
//! 在所有项目间登记每个驻留 viewer 的估算字节、重型 WebView 数、active/dirty、
//! 是否已有 recovery、saving/agent 写入等状态,并在新加载前 reserve；不足时按
//! 规则挑可淘汰者(后台项目干净 → 当前项目非活动干净 → 有 recovery 的脏 tab)。
//!
//! **内存预算与重型 WebView 数必须同时满足**,数量不是内存预算的替代品。
//! 不可淘汰:active、saving、agent 写入中、以及**无 recovery 的脏 tab**。

// 资源管理器已接入 runtime 的 webview 池预算;诊断/按 key 取用等 API 目前仅
// 单测与后续诊断页使用,显式允许 dead_code 噪声。
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::capabilities::ResourceBudgets;

/// viewer 归属键(项目 + 面板 + tab)。**必须含面板**:Files 与 Project 两个
/// 预览面板各有独立的 `tab_id` 空间,只用 `(project_id, tab_id)` 会让两边同号
/// tab 互相顶掉预算/Touch(T3)。
pub type ViewerKey = (i64, crate::app::PanelKind, usize);

/// 一个驻留 viewer 的登记信息(纯数据,由调用方在创建/销毁 viewer 时维护)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewerRegistration {
    pub project_id: i64,
    pub panel: crate::app::PanelKind,
    pub tab_id: usize,
    pub estimated_bytes: u64,
    /// 是否占用一个"重型 WebView"名额(CodeMirror/Flyfish)。
    pub heavy_webview: bool,
    pub active: bool,
    pub dirty: bool,
    /// 脏内容是否已有 recovery snapshot(有才允许被淘汰)。
    pub has_recovery: bool,
    pub saving: bool,
    pub agent_writing: bool,
    /// 最近访问的单调时钟值(越大越新)。
    pub last_accessed: u64,
}

impl ViewerRegistration {
    pub fn new(project_id: i64, panel: crate::app::PanelKind, tab_id: usize) -> Self {
        Self {
            project_id,
            panel,
            tab_id,
            estimated_bytes: 0,
            heavy_webview: false,
            active: false,
            dirty: false,
            has_recovery: false,
            saving: false,
            agent_writing: false,
            last_accessed: 0,
        }
    }

    pub fn key(&self) -> ViewerKey {
        (self.project_id, self.panel, self.tab_id)
    }

    /// 是否可以被淘汰。active / saving / agent 写入 / 无 recovery 的脏 tab 不可。
    pub fn evictable(&self) -> bool {
        !self.active && !self.saving && !self.agent_writing && !(self.dirty && !self.has_recovery)
    }
}

/// reserve 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reservation {
    /// 预算内,直接加载。
    Granted,
    /// 需先按顺序淘汰这些 viewer 才能加载。
    NeedEviction(Vec<ViewerKey>),
    /// 无论如何都放不下(含淘汰后仍超,或没有任何可淘汰者)。
    Denied { reason: String },
}

/// 一个 viewer 的估算成本与是否占重型 WebView 名额(T3)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewerCost {
    pub estimated_bytes: u64,
    pub heavy_webview: bool,
}

/// 窗口化 viewer 的常驻估算(稀疏索引 + 有界窗口),**不随文件大小增长**。
pub const WINDOWED_RESIDENT_BYTES: u64 = 4 * 1024 * 1024;

/// 按 backend 种类 + 是否窗口化估算一个 tab 的成本(T3):
/// - 窗口化:常驻只有稀疏索引 + 有界窗口,不随文件线性增长;
/// - CodeMirror / vanilla-jsoneditor(Tree)/ Flyfish 渲染:重型 WebView,按文件大小估;
/// - Markdown/HTML 的 Source 模式(若由 CodeMirror 承载)也算重型;
/// - Tabular 网格:iced 原生(不占重型名额),按文件大小估;
/// - External/Unsupported:无 viewer,0 成本。
pub fn estimate_cost(
    backend: Option<&crate::preview::PreviewBackend>,
    windowed: bool,
    file_size: u64,
) -> ViewerCost {
    use crate::preview::PreviewBackend;
    let Some(backend) = backend else {
        return ViewerCost {
            estimated_bytes: 0,
            heavy_webview: false,
        };
    };
    match backend {
        PreviewBackend::Code(_) => ViewerCost {
            estimated_bytes: if windowed {
                WINDOWED_RESIDENT_BYTES
            } else {
                file_size
            },
            heavy_webview: true,
        },
        PreviewBackend::Json(_) => ViewerCost {
            estimated_bytes: file_size,
            heavy_webview: true,
        },
        PreviewBackend::Streamed(_) => ViewerCost {
            estimated_bytes: if windowed {
                WINDOWED_RESIDENT_BYTES
            } else {
                file_size
            },
            heavy_webview: true,
        },
        PreviewBackend::Rendered(rendered) => ViewerCost {
            estimated_bytes: file_size,
            heavy_webview: matches!(rendered.mode, crate::preview::RenderedMode::Rendered),
        },
        PreviewBackend::Tabular(_) => ViewerCost {
            estimated_bytes: file_size,
            heavy_webview: false,
        },
        PreviewBackend::External(_) | PreviewBackend::Unsupported(_) => ViewerCost {
            estimated_bytes: 0,
            heavy_webview: false,
        },
    }
}

/// 诊断快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDiagnostics {
    pub resident_count: usize,
    pub total_resident_bytes: u64,
    pub total_preview_bytes_budget: u64,
    pub heavy_webviews: usize,
    pub max_heavy_webviews: usize,
}

/// 全局资源管理器(按项目 id 跨项目共享一份)。
pub struct ResourceManager {
    budgets: ResourceBudgets,
    registrations: HashMap<ViewerKey, ViewerRegistration>,
    clock: u64,
}

static GLOBAL_MANAGER: OnceLock<Mutex<ResourceManager>> = OnceLock::new();

/// 进程级共享资源管理器。所有项目和预览面板共用同一份预算。
pub fn global_manager() -> &'static Mutex<ResourceManager> {
    GLOBAL_MANAGER
        .get_or_init(|| Mutex::new(ResourceManager::new(crate::capabilities::current().budgets)))
}

impl ResourceManager {
    pub fn new(budgets: ResourceBudgets) -> Self {
        Self {
            budgets,
            registrations: HashMap::new(),
            clock: 0,
        }
    }

    pub fn budgets(&self) -> &ResourceBudgets {
        &self.budgets
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// 登记一个新的驻留 viewer(若已存在则覆盖,不叠加字节)。
    pub fn register(&mut self, mut reg: ViewerRegistration) {
        reg.last_accessed = self.tick();
        self.registrations.insert(reg.key(), reg);
    }

    /// 访问(刷新 LRU 时间)。
    pub fn touch(&mut self, key: ViewerKey) {
        let t = self.tick();
        if let Some(r) = self.registrations.get_mut(&key) {
            r.last_accessed = t;
        }
    }

    /// 销毁 viewer 后释放预算(可重复调用,幂等)。
    pub fn release(&mut self, key: ViewerKey) {
        self.registrations.remove(&key);
    }

    pub fn get(&self, key: ViewerKey) -> Option<&ViewerRegistration> {
        self.registrations.get(&key)
    }

    pub fn get_mut(&mut self, key: ViewerKey) -> Option<&mut ViewerRegistration> {
        self.registrations.get_mut(&key)
    }

    pub fn contains(&self, key: ViewerKey) -> bool {
        self.registrations.contains_key(&key)
    }

    /// 删除某项目当前帧已经不再驻留的 viewer。
    pub fn prune_project(
        &mut self,
        project_id: i64,
        keep: &std::collections::HashSet<(crate::app::PanelKind, usize)>,
    ) {
        self.registrations.retain(|(project, panel, tab), _| {
            *project != project_id || keep.contains(&(*panel, *tab))
        });
    }

    pub fn clear(&mut self) {
        self.registrations.clear();
    }

    pub fn set_active(&mut self, key: ViewerKey, active: bool) {
        if let Some(r) = self.registrations.get_mut(&key) {
            r.active = active;
        }
    }

    pub fn total_resident_bytes(&self) -> u64 {
        self.registrations.values().map(|r| r.estimated_bytes).sum()
    }

    pub fn heavy_webviews(&self) -> usize {
        self.registrations
            .values()
            .filter(|r| r.heavy_webview)
            .count()
    }

    pub fn diagnostics(&self) -> ResourceDiagnostics {
        ResourceDiagnostics {
            resident_count: self.registrations.len(),
            total_resident_bytes: self.total_resident_bytes(),
            total_preview_bytes_budget: self.budgets.total_preview_bytes,
            heavy_webviews: self.heavy_webviews(),
            max_heavy_webviews: self.budgets.max_heavy_webviews,
        }
    }

    /// 尝试为一个新 viewer 预留 `bytes`(可能占一个重型名额)。`current_project`
    /// 决定淘汰优先级(后台项目优先)。
    ///
    /// 注意:本方法**不**修改登记表来腾挪空间(不偷偷淘汰);它只报告"要不要
    /// 淘汰、淘汰谁"。调用方真正销毁 viewer 后再 `release` 并重新 reserve,
    /// 保证"销毁实际 viewer 后才归还预算"。
    pub fn try_reserve(&self, bytes: u64, heavy: bool, current_project: i64) -> Reservation {
        // 单个 viewer 就超过总预算 → 直接拒绝(不可能靠淘汰别人腾出)。
        if bytes > self.budgets.total_preview_bytes {
            return Reservation::Denied {
                reason: format!(
                    "单 viewer {bytes} 字节超过总预览预算 {}",
                    self.budgets.total_preview_bytes
                ),
            };
        }
        let need_bytes = self.total_resident_bytes().saturating_add(bytes);
        let need_heavy = if heavy {
            self.heavy_webviews() + 1
        } else {
            self.heavy_webviews()
        };
        let bytes_ok = need_bytes <= self.budgets.total_preview_bytes;
        let heavy_ok = need_heavy <= self.budgets.max_heavy_webviews;
        if bytes_ok && heavy_ok {
            return Reservation::Granted;
        }

        // 先淘汰候选,再看淘汰后是否满足两个约束。
        let candidates = self.eviction_order(current_project);
        let mut chosen = Vec::new();
        let mut bytes_after = need_bytes;
        let mut heavy_after = need_heavy;
        for &key in &candidates {
            if bytes_after <= self.budgets.total_preview_bytes
                && heavy_after <= self.budgets.max_heavy_webviews
            {
                break;
            }
            let Some(reg) = self.registrations.get(&key) else {
                continue;
            };
            bytes_after = bytes_after.saturating_sub(reg.estimated_bytes);
            if reg.heavy_webview {
                heavy_after = heavy_after.saturating_sub(1);
            }
            chosen.push(key);
        }

        if bytes_after <= self.budgets.total_preview_bytes
            && heavy_after <= self.budgets.max_heavy_webviews
            && !chosen.is_empty()
        {
            Reservation::NeedEviction(chosen)
        } else {
            Reservation::Denied {
                reason: if candidates.is_empty() {
                    "超预算且没有可淘汰的 viewer".to_string()
                } else {
                    "即使淘汰全部可淘汰 viewer 仍超预算".to_string()
                },
            }
        }
    }

    /// 可淘汰者按优先级排序:后台项目干净 → 后台有 recovery 的脏 → 当前项目
    /// 非活动干净 → 当前项目有 recovery 的脏;每组内按 LRU(最久未访问优先)。
    pub fn eviction_order(&self, current_project: i64) -> Vec<ViewerKey> {
        let mut candidates: Vec<&ViewerRegistration> = self
            .registrations
            .values()
            .filter(|r| r.evictable())
            .collect();
        candidates.sort_by_key(|r| {
            let background = r.project_id != current_project;
            let clean = !r.dirty;
            let group = match (background, clean) {
                (true, true) => 0u8,
                (true, false) => 1,
                (false, true) => 2,
                (false, false) => 3,
            };
            (group, r.last_accessed)
        });
        candidates.into_iter().map(|r| r.key()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::PanelKind;

    fn budgets() -> ResourceBudgets {
        ResourceBudgets {
            single_editor_bytes: 64 * 1024 * 1024,
            total_preview_bytes: 100,
            json_tree_bytes: 32 * 1024 * 1024,
            full_file_load_bytes: 64 * 1024 * 1024,
            max_heavy_webviews: 2,
            background_parallelism: 2,
        }
    }

    fn k(project: i64, tab: usize) -> ViewerKey {
        (project, PanelKind::Files, tab)
    }

    fn reg(project: i64, tab: usize, bytes: u64, heavy: bool) -> ViewerRegistration {
        ViewerRegistration {
            estimated_bytes: bytes,
            heavy_webview: heavy,
            ..ViewerRegistration::new(project, PanelKind::Files, tab)
        }
    }

    #[test]
    fn register_accumulates_bytes_and_heavy() {
        let mut m = ResourceManager::new(budgets());
        m.register(reg(1, 1, 40, true));
        m.register(reg(2, 1, 30, true));
        assert_eq!(m.total_resident_bytes(), 70);
        assert_eq!(m.heavy_webviews(), 2);
        let d = m.diagnostics();
        assert_eq!(d.resident_count, 2);
        assert_eq!(d.total_preview_bytes_budget, 100);
    }

    #[test]
    fn same_tab_id_across_panels_does_not_collide() {
        // T3:Files 与 Project 面板各有 tab_id 空间,同号不得互相顶掉。
        let mut m = ResourceManager::new(budgets());
        m.register(reg(1, 1, 40, true));
        let mut project = ViewerRegistration::new(1, PanelKind::Project, 1);
        project.estimated_bytes = 30;
        project.heavy_webview = true;
        m.register(project);
        assert_eq!(m.total_resident_bytes(), 70);
        assert_eq!(m.diagnostics().resident_count, 2);
        m.release((1, PanelKind::Files, 1));
        assert_eq!(m.total_resident_bytes(), 30, "只释放 Files 那份");
    }

    #[test]
    fn reserve_within_budget_is_granted() {
        let m = ResourceManager::new(budgets());
        assert_eq!(m.try_reserve(50, true, 1), Reservation::Granted);
    }

    #[test]
    fn reserve_over_bytes_needs_eviction() {
        let mut m = ResourceManager::new(budgets());
        m.register(reg(1, 1, 40, true));
        // 40 + 70 = 110 > 100 → 需要淘汰 40。
        assert_eq!(
            m.try_reserve(70, false, 1),
            Reservation::NeedEviction(vec![k(1, 1)])
        );
    }

    #[test]
    fn heavy_count_limit_is_independent_of_bytes() {
        let mut m = ResourceManager::new(budgets());
        // 字节远未超,但重型名额已满(2/2)。
        m.register(reg(1, 1, 1, true));
        m.register(reg(1, 2, 1, true));
        match m.try_reserve(1, true, 1) {
            Reservation::NeedEviction(v) => assert_eq!(v.len(), 1),
            other => panic!("应需淘汰一个重型 viewer,得到 {other:?}"),
        }
    }

    #[test]
    fn non_evictable_are_skipped() {
        let mut m = ResourceManager::new(budgets());
        let mut active = reg(2, 1, 40, true);
        active.active = true;
        m.register(active);
        // 唯一占用者不可淘汰 → 拒绝。
        assert!(matches!(
            m.try_reserve(70, false, 1),
            Reservation::Denied { .. }
        ));
    }

    #[test]
    fn dirty_without_recovery_is_not_evictable_but_with_recovery_is() {
        let mut m = ResourceManager::new(budgets());
        let mut dirty = reg(2, 1, 40, true);
        dirty.dirty = true;
        dirty.has_recovery = false;
        m.register(dirty);
        assert!(matches!(
            m.try_reserve(70, false, 1),
            Reservation::Denied { .. }
        ));

        // 补上 recovery 后即可淘汰。
        m.get_mut(k(2, 1)).unwrap().has_recovery = true;
        assert_eq!(
            m.try_reserve(70, false, 1),
            Reservation::NeedEviction(vec![k(2, 1)])
        );
    }

    #[test]
    fn eviction_order_prefers_background_clean_then_current_then_dirty() {
        let mut m = ResourceManager::new(budgets());
        // 当前项目(1)非活动脏(有 recovery)。
        let mut d_current = reg(1, 9, 10, false);
        d_current.dirty = true;
        d_current.has_recovery = true;
        m.register(d_current);
        // 当前项目(1)非活动干净。
        m.register(reg(1, 2, 10, false));
        // 后台项目(2)脏(有 recovery)。
        let mut d_bg = reg(2, 1, 10, false);
        d_bg.dirty = true;
        d_bg.has_recovery = true;
        m.register(d_bg);
        // 后台项目(3)干净。
        m.register(reg(3, 1, 10, false));

        let order = m.eviction_order(1);
        // 期望:后台干净(3,1) → 后台脏(2,1) → 当前干净(1,2) → 当前脏(1,9)。
        assert_eq!(order, vec![k(3, 1), k(2, 1), k(1, 2), k(1, 9)]);
    }

    #[test]
    fn eviction_order_uses_lru_within_group() {
        let mut m = ResourceManager::new(budgets());
        m.register(reg(2, 1, 10, false));
        m.register(reg(2, 2, 10, false));
        m.touch(k(2, 1)); // 1 比 2 更新 → 先淘汰 2。
        assert_eq!(m.eviction_order(1), vec![k(2, 2), k(2, 1)]);
    }

    #[test]
    fn oversized_single_viewer_is_denied() {
        let m = ResourceManager::new(budgets());
        assert!(matches!(
            m.try_reserve(200, false, 1),
            Reservation::Denied { .. }
        ));
    }

    #[test]
    fn release_frees_budget_and_is_idempotent() {
        let mut m = ResourceManager::new(budgets());
        m.register(reg(1, 1, 40, true));
        m.release(k(1, 1));
        m.release(k(1, 1));
        assert_eq!(m.total_resident_bytes(), 0);
        assert_eq!(m.heavy_webviews(), 0);
    }

    #[test]
    fn register_existing_key_does_not_double_count() {
        let mut m = ResourceManager::new(budgets());
        m.register(reg(1, 1, 40, true));
        m.register(reg(1, 1, 30, true));
        assert_eq!(m.total_resident_bytes(), 30);
        assert_eq!(m.diagnostics().resident_count, 1);
    }

    #[test]
    fn estimate_cost_classifies_backends() {
        use crate::preview::PreviewBackend;
        // 窗口化:常驻固定,不随文件增长,重型。
        let windowed = estimate_cost(
            Some(&PreviewBackend::Code(crate::preview::CodeBackend {
                mode: crate::preview::CodeMode::ReadOnly,
                language: "rust".into(),
            })),
            true,
            500 * 1024 * 1024,
        );
        assert_eq!(windowed.estimated_bytes, WINDOWED_RESIDENT_BYTES);
        assert!(windowed.heavy_webview);
        // External/Unsupported:无 viewer,0 成本。
        let ext = PreviewBackend::External(crate::preview::ExternalBackend {
            reason: crate::preview::RouteReason::ArchiveFallback,
        });
        assert_eq!(
            estimate_cost(Some(&ext), false, 1234),
            ViewerCost {
                estimated_bytes: 0,
                heavy_webview: false
            }
        );
    }
}
