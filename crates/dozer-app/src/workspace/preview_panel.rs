//! Workspace 与预览面板库(`dozer-panel-preview`)的接缝:把 Files / Project 两组并列字段临时打包成
//! 库的 [`PreviewPanelState`],调用库的 `update`,再把状态放回、执行产出的效果。
//!
//! 库产出的效果里,需要产品参与的(白名单、项目树选中、上下文推送)在这里执行;页签被选中 / 被关闭
//! 这两个需要 `App` 级状态(拖拽武装、悬停清理)的,原样返回给调用方。

use std::path::PathBuf;
use std::sync::Arc;

use dozer_panel_preview::capabilities::PreviewCapabilities;
use dozer_panel_preview::{
    Effect, Message as PanelMessage, PanelBehavior, PreviewPanelState, UpdateCtx, ViewerPanel,
};

use super::*;
use crate::app::{Message, PanelKind};
use crate::preview_state;

/// Dozer 开启全部预览后端(Digger 第一批只开一部分)。
fn dozer_caps() -> PreviewCapabilities {
    PreviewCapabilities::all()
}

fn behavior_of(panel: ViewerPanel) -> PanelBehavior {
    match panel {
        // Files:持久化页签、推送预览上下文、物化只恢复了壳的页签;Project 是会话期状态。
        ViewerPanel::Project => PanelBehavior {
            persist_tabs: false,
            materialize_suspended: false,
        },
        _ => PanelBehavior {
            persist_tabs: true,
            materialize_suspended: true,
        },
    }
}

impl Workspace {
    fn take_preview_panel(&mut self, kind: PanelKind) -> PreviewPanelState {
        match kind {
            PanelKind::Project => PreviewPanelState {
                pane: std::mem::take(&mut self.project_preview),
                error: self.project_preview_error.take(),
                tab_first: self.project_preview_tab_first,
                overflow_anchor: self.project_preview_tab_overflow_anchor.take(),
            },
            _ => PreviewPanelState {
                pane: std::mem::take(&mut self.preview),
                error: self.preview_error.take(),
                tab_first: self.preview_tab_first,
                overflow_anchor: self.preview_tab_overflow_anchor.take(),
            },
        }
    }

    fn put_preview_panel(&mut self, kind: PanelKind, state: PreviewPanelState) {
        match kind {
            PanelKind::Project => {
                self.project_preview = state.pane;
                self.project_preview_error = state.error;
                self.project_preview_tab_first = state.tab_first;
                self.project_preview_tab_overflow_anchor = state.overflow_anchor;
            }
            _ => {
                self.preview = state.pane;
                self.preview_error = state.error;
                self.preview_tab_first = state.tab_first;
                self.preview_tab_overflow_anchor = state.overflow_anchor;
            }
        }
    }

    /// 跑一段库逻辑:打包状态 → `f` → 放回 → 执行效果。返回 App 级通知(`TabSelected` / `TabClosed`)。
    /// `locale` 只影响库里写进状态的本地化文案;`avail_px` 是页签条的真实可用宽度(只有开 / 选页签用)。
    pub(crate) fn preview_run(
        &mut self,
        kind: PanelKind,
        io: &ShellIo,
        locale: &str,
        avail_px: f32,
        f: impl FnOnce(&mut PreviewPanelState, &UpdateCtx<'_>) -> Vec<Effect>,
    ) -> Vec<Effect> {
        let panel = crate::preview::viewer_panel(kind);
        let project_id = self.project_id();
        let project_root = self.project.as_ref().map(|p| PathBuf::from(&p.path));
        let pool_id = self.active_preview_webview_id(kind);
        let caps = dozer_caps();
        let width = |title: &str| preview_tab_display_width(title);
        let mut state = self.take_preview_panel(kind);
        let fx = {
            let ctx = UpdateCtx {
                panel,
                project_id,
                project_root: project_root.as_deref(),
                caps: &caps,
                locale,
                tab_bar_avail_px: avail_px,
                tab_display_width: &width,
                active_webview_pool_id: pool_id,
                behavior: behavior_of(panel),
            };
            f(&mut state, &ctx)
        };
        self.put_preview_panel(kind, state);
        self.run_preview_effects(kind, fx, io)
    }

    /// 处理一条库消息。
    pub(crate) fn preview_dispatch(
        &mut self,
        kind: PanelKind,
        msg: PanelMessage,
        io: &ShellIo,
        locale: &str,
        avail_px: f32,
    ) -> Vec<Effect> {
        self.preview_run(kind, io, locale, avail_px, |st, ctx| {
            dozer_panel_preview::update(st, msg, ctx)
        })
    }

    /// 执行效果:产品侧的在这里做,库侧的交给 `run_effect`,App 级的返回。
    fn run_preview_effects(
        &mut self,
        kind: PanelKind,
        fx: Vec<Effect>,
        io: &ShellIo,
    ) -> Vec<Effect> {
        let mut app_level = Vec::new();
        let project_id = self.project_id();
        let persistence: Arc<dyn dozer_panel_preview::PreviewPersistence> =
            Arc::new(preview_state::persistence());
        for effect in fx {
            match effect {
                Effect::AllowFile(path) => {
                    self.allowed_files
                        .lock()
                        .expect("allowed_files 锁")
                        .insert(path);
                }
                Effect::PathOpened(path) => {
                    if matches!(kind, PanelKind::Project) {
                        self.project_panel.set_selected_link(path);
                    } else {
                        self.files.set_tree_selected(path);
                    }
                }
                Effect::ContextChanged { immediate } => {
                    if immediate {
                        self.flush_preview_context_push(io);
                    } else {
                        self.spawn_preview_context_push(io);
                    }
                }
                e @ (Effect::TabSelected { .. } | Effect::TabClosed { .. }) => app_level.push(e),
                other => {
                    let Some(project_id) = project_id else {
                        continue;
                    };
                    let proxy = io.proxy.clone();
                    dozer_panel_preview::run_effect(
                        other,
                        move |m| {
                            let _ = proxy.send_event(Message::PreviewResult(project_id, kind, m));
                        },
                        &io.handle,
                        persistence.clone(),
                    );
                }
            }
        }
        app_level
    }

    /// 物化一个 `Suspended` 预览壳(重试 / 启动恢复 / 选中 / Agent 命令前)。
    pub(crate) fn load_preview_tab(&mut self, kind: PanelKind, tab_id: usize, io: &ShellIo) {
        self.preview_run(kind, io, "en", 0.0, |st, ctx| {
            let mut fx = Vec::new();
            dozer_panel_preview::load_preview_tab(&mut st.pane, ctx, tab_id, &mut fx);
            fx
        });
    }

    /// 启动 / 切项目时恢复上次的页签壳并物化当前页签(Files 面板)。
    pub(crate) fn restore_preview_state(&mut self, io: &ShellIo) {
        let safe_startup = io.safe_startup;
        let persistence = preview_state::persistence();
        self.preview_run(PanelKind::Files, io, "en", 0.0, |st, ctx| {
            dozer_panel_preview::restore_tabs(st, &persistence, ctx, safe_startup)
        });
    }

    /// 表格 sheet 切换(懒加载未加载的 sheet)。
    pub(crate) fn preview_pane_tabular_action(
        &mut self,
        kind: PanelKind,
        tab_id: usize,
        sheet: usize,
        io: &ShellIo,
    ) {
        self.preview_run(kind, io, "en", 0.0, |st, _| {
            let mut fx = Vec::new();
            dozer_panel_preview::update::tabular_action(&mut st.pane, tab_id, sheet, &mut fx);
            fx
        });
    }

    /// 把 `reload_webviews_for` 攒下的待重载 PlantUML tab 起授权读取。
    pub(crate) fn spawn_pending_plantuml_reloads(&mut self, kind: PanelKind, io: &ShellIo) {
        self.preview_run(kind, io, "en", 0.0, |st, ctx| {
            let mut fx = Vec::new();
            dozer_panel_preview::update::drain_plantuml_reloads(&mut st.pane, ctx, &mut fx);
            fx
        });
    }

    /// 写页签持久化状态并推送预览上下文(Files)。
    pub(crate) fn spawn_preview_state_save(&self, io: &ShellIo) {
        let Some(project_id) = self.project_id() else {
            return;
        };
        let state = self.preview.persisted_snapshot();
        let persistence = preview_state::persistence();
        io.handle.spawn(async move {
            use dozer_panel_preview::PreviewPersistence;
            if let Err(e) = persistence.save(project_id, &state) {
                tracing::warn!(target: "dozer::module::preview", "预览 tab 状态写盘失败: {e}");
            }
        });
    }

    /// flyfish `getSearchState()` 回写的命中总数 / 当前序号。
    pub(crate) fn preview_find_set_webview_state(
        &mut self,
        kind: PanelKind,
        current: usize,
        total: usize,
    ) {
        self.preview_pane_mut(kind)
            .set_find_webview_state(current, total);
    }

    /// 测试用:`kind` 对应面板的错误槽。
    #[cfg(test)]
    pub(crate) fn preview_error_mut(
        &mut self,
        kind: PanelKind,
    ) -> &mut Option<dozer_model::LocalizedText> {
        match kind {
            PanelKind::Project => &mut self.project_preview_error,
            _ => &mut self.preview_error,
        }
    }
}
