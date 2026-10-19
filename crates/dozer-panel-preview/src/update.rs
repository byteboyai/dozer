//! 预览面板的 `update`:把 Dozer 里散在 `app/update.rs` 与 `workspace/state.rs` 的
//! 「只碰窗格 / 核心类型 / 本地文件」的处理器收成一个函数。副作用一律以 [`Effect`] 返回。
//!
//! 与迁移前的差别只在**怎么表达**副作用(`io.arm_load_timeout` → `Effect::ScheduleLoadTimeout`,
//! `spawn_blocking` → `Effect::*`,`allowed_files.insert` → `Effect::AllowFile`),分支顺序与判定不变。

use std::path::{Path, PathBuf};

use crate::capabilities::{PreviewCapabilities, backend_enabled};
use crate::effects::Effect;
use crate::message::Message;
use crate::panel::PreviewPanelState;
use crate::{
    PANEL_OWNER, PreviewBackend, PreviewKind, PreviewLoadStage, PreviewMode, PreviewPane,
    PreviewPersistence, RouteReason, TabKind, UnsupportedBackend, ViewerPanel,
};

/// 面板级行为差异(Dozer 的 Files 与 Project 两个面板今天就不同)。
#[derive(Debug, Clone, Copy)]
pub struct PanelBehavior {
    /// 打开 / 选中 / 关闭后写页签持久化状态并推送预览上下文(Files:是;Project:会话期状态,否)。
    pub persist_tabs: bool,
    /// 选中 / 重开一个只恢复了壳的页签时物化它(Files:是;Project:不会出现壳)。
    pub materialize_suspended: bool,
}

pub struct UpdateCtx<'a> {
    pub panel: ViewerPanel,
    pub project_id: Option<i64>,
    pub project_root: Option<&'a Path>,
    pub caps: &'a PreviewCapabilities,
    pub locale: &'a str,
    /// 页签条的真实可用宽度(翻页窗口数学用,与视图用同一个值)。
    pub tab_bar_avail_px: f32,
    /// 页签标题的估算展示宽度(产品的字体度量)。
    pub tab_display_width: &'a dyn Fn(&str) -> f32,
    /// 激活页签的 webview 池 key(webview 档查找用),没有则 `None`。
    pub active_webview_pool_id: Option<usize>,
    pub behavior: PanelBehavior,
    /// 恢复目录(脏快照落盘)。`None` = 该产品不持久化恢复数据。
    pub recovery_dir: Option<&'a Path>,
    /// 连续失败计数文件(`PreviewPersistence::failures_path`)。
    pub failures_path: PathBuf,
    /// 该产品是否已经完成启动(安全启动下不应调度索引 / 快照类后台任务)。
    pub safe_startup: bool,
}

/// 处理一条消息,返回待执行的效果。
pub fn update(state: &mut PreviewPanelState, msg: Message, ctx: &UpdateCtx<'_>) -> Vec<Effect> {
    let mut fx = Vec::new();
    match msg {
        Message::OpenPath { path, target_line } => {
            open_path(state, ctx, path, target_line, &mut fx)
        }
        Message::SelectTab(idx) => select_tab(state, ctx, idx, &mut fx),
        Message::CloseTab(idx) => close_tab(state, ctx, idx, &mut fx),
        Message::ToggleRenderMode(idx) => toggle_render_mode(state, ctx, idx, &mut fx),
        Message::TabOverflowToggle { cursor } => {
            state.overflow_anchor = if state.overflow_anchor.is_some() {
                None
            } else {
                Some(cursor)
            };
        }
        Message::TabOverflowDismiss => state.overflow_anchor = None,
        Message::OpenExternal(tab_id) => {
            let path = state
                .pane
                .tabs()
                .iter()
                .find(|t| t.id == tab_id)
                .and_then(|t| match &t.kind {
                    TabKind::File(p) => Some(p.clone()),
                    _ => None,
                });
            if let Some(path) = path.filter(|p| p.is_file()) {
                fx.push(Effect::OpenExternal { path });
            }
        }
        // 失败提示由产品展示(Toast);库不持有通知通道。
        Message::OpenExternalFailed { .. } => {}
        Message::Retry(tab_id) => {
            if state.pane.is_pending_load(tab_id) {
                load_preview_tab(&mut state.pane, ctx, tab_id, &mut fx);
            }
        }
        Message::PlainTextOpen(tab_id) => plain_text_open(&mut state.pane, tab_id, &mut fx),
        Message::ConflictKeep(tab_id) => {
            state.pane.keep_conflict_changes(tab_id);
        }
        Message::ConflictReload(tab_id) => conflict_reload(&mut state.pane, ctx, tab_id, &mut fx),
        Message::TabularTextModeToggle(tab_id) => {
            if let Some(tab) = state.pane.tabs_mut().iter_mut().find(|t| t.id == tab_id)
                && let Some(PreviewBackend::Tabular(tabular)) = tab.backend.as_mut()
            {
                tabular.mode = match tabular.mode {
                    crate::TabularMode::Grid => crate::TabularMode::Text,
                    crate::TabularMode::Text => crate::TabularMode::Grid,
                };
            }
        }
        Message::JsonModeToggle(tab_id) => {
            if let Some((tab_id, generation)) = toggle_json_mode(&mut state.pane, tab_id) {
                fx.push(Effect::ScheduleLoadTimeout {
                    tab_id,
                    generation,
                    stage: PreviewLoadStage::SwitchingMode,
                });
            }
        }
        // 原生 iced 编辑器已退役:保存 / 撤销由 CodeMirror host 自己处理,这几条保持 no-op。
        Message::SaveActive | Message::InsertTab | Message::UndoActive | Message::RedoActive => {}
        Message::FindOpen => find_open(&mut state.pane, ctx, false),
        Message::FindOpenWithReplace => find_open(&mut state.pane, ctx, true),
        Message::FindReplaceToggle => state.pane.toggle_find_replace(),
        Message::FindClose => {
            // 关闭后把焦点拨回其下编辑器,不许键盘焦点悬空在已消失的 widget 上。
            state.pane.close_find();
            state.pane.request_editor_focus();
        }
        Message::FindText(query) => state.pane.find_type(query),
        Message::FindGo(next) => state.pane.find_go(next),
        Message::FindCase(sensitive) => state.pane.set_find_case(sensitive),
        Message::FindReplacement(replacement) => state.pane.set_find_replacement(replacement),
        Message::FindReplaceCurrent => {
            state.pane.replace_current();
        }
        Message::FindReplaceAll => {
            state.pane.replace_all();
        }
        Message::FindWebviewState { current, total } => {
            state.pane.set_find_webview_state(current, total)
        }
        Message::LargeFileSearchClose => state.pane.close_large_file_search(),
        Message::LargeFileSearchSubmit { tab_id, query } => {
            large_file_search_submit(&mut state.pane, tab_id, query, &mut fx)
        }
        Message::LargeFileSearchResults {
            tab_id,
            generation,
            result,
        } => state
            .pane
            .set_large_file_search_results(tab_id, generation, result),
        Message::LargeFileSearchProgress {
            tab_id,
            generation,
            completed,
            total,
        } => state.pane.set_large_file_search_progress(
            tab_id,
            generation,
            crate::PreviewLoadProgress {
                completed,
                total: (total > 0).then_some(total),
            },
        ),
        Message::LargeFileSearchGo(forward) => large_file_search_go(state, forward),
        Message::BlankInfoLoaded { info } => {
            let pane = &mut state.pane;
            pane.blank_info_in_flight = false;
            // 用户中途切了项目 → 旧结果已失配;激活页签已不是 Blank 也丢。
            let same_project = ctx
                .project_root
                .is_some_and(|r| r.to_string_lossy() == info.path.to_string_lossy());
            let active_is_blank = pane
                .tabs
                .get(pane.active)
                .is_some_and(|t| matches!(t.kind, TabKind::Blank));
            if same_project && active_is_blank {
                pane.blank_info = Some(info);
            }
        }
        Message::Profiled {
            tab_id,
            generation,
            result,
        } => profiled(&mut state.pane, ctx, tab_id, generation, result, &mut fx),
        Message::LoadTimeout {
            tab_id,
            generation,
            stage,
        } => {
            // 只有仍在该 generation **且仍停在同 stage**的加载才判超时;否则静默丢弃,
            // 避免迟到超时误杀。
            if state.pane.load_timeout_matches(tab_id, generation, stage) {
                tracing::warn!(target: "dozer::module::preview", panel = ?ctx.panel, tab_id, generation, stage = ?stage, "预览加载阶段超时,进入可重试失败");
                state.pane.fail_load(
                    tab_id,
                    generation,
                    crate::PreviewError::new(
                        crate::i18n::panel_text(
                            PANEL_OWNER,
                            "preview.load_timeout_retry",
                            "Preview load timed out; please retry",
                            ctx.locale,
                        ),
                        true,
                    ),
                );
            }
        }
        Message::WindowIndex { tab_id, result } => {
            window_index(&mut state.pane, ctx, tab_id, result, &mut fx)
        }
        Message::RecoveryWritten(tab_id) => {
            if let Some(tab) = state.pane.tabs_mut().iter_mut().find(|t| t.id == tab_id) {
                tab.recovery_written = true;
            }
        }
        Message::RecoveryRead {
            tab_id,
            generation,
            restore,
        } => recovery_read(&mut state.pane, tab_id, generation, restore),
        Message::TabularLoaded {
            tab_id,
            generation,
            result,
        } => tabular_loaded(&mut state.pane, ctx, tab_id, generation, result, &mut fx),
        Message::TabularSheetLoaded {
            tab_id,
            sheet_index,
            result,
        } => tabular_sheet_loaded(&mut state.pane, tab_id, sheet_index, result),
        Message::PlantUmlLoaded {
            tab_id,
            generation,
            result,
        } => match result {
            // host 报 `rendered` / `failed` 时回带的 revision 与本次推送一致;用 load generation 当渲染世代。
            Ok(doc) => {
                if !state
                    .pane
                    .store_plantuml_document(tab_id, generation, generation, doc)
                {
                    tracing::debug!(target: "dozer::module::preview", tab_id, "丢弃过期的 PlantUML 加载结果");
                }
            }
            Err(error) => state
                .pane
                .apply_plantuml_load_error(tab_id, generation, &error, ctx.locale),
        },
        Message::Host(event) => crate::host_update::host_event(state, event, ctx, &mut fx),
        Message::SaveFinished { tab_id, result } => {
            crate::host_update::save_finished(state, tab_id, result, ctx, &mut fx)
        }
    }
    fx
}

/// 恢复上次的页签壳(`Suspended`),并物化当前页签。`safe_startup` 下只留壳不自动加载。
/// 对应 Dozer `Workspace::restore_preview_state`。
pub fn restore_tabs(
    state: &mut PreviewPanelState,
    persistence: &dyn PreviewPersistence,
    ctx: &UpdateCtx<'_>,
    safe_startup: bool,
) -> Vec<Effect> {
    let mut fx = Vec::new();
    let Some(project_id) = ctx.project_id else {
        return fx;
    };
    let saved = persistence.load(project_id);
    let active_path = saved.active_path.clone();
    let pane = &mut state.pane;
    let mut active_id = None;
    let mut first_id = None;
    for persisted in saved.tabs {
        let path = persisted.path;
        if !path.is_file() {
            continue;
        }
        let is_active = active_path
            .as_deref()
            .map(|active| active == path.as_path())
            .unwrap_or(persisted.active);
        fx.push(Effect::AllowFile(path.clone()));
        let id = pane.push_shell_tab(path, persisted.mode);
        pane.set_pending_view(
            id,
            crate::ViewStateRestore {
                cursor: persisted.cursor,
                selection: persisted.selection,
                top_line: persisted.scroll_anchor.map(|v| v as u32),
                folds: persisted.folds,
            },
        );
        if let Some(t) = persisted.tabular {
            pane.set_pending_tabular(id, t.sheet, t.scroll_row, t.scroll_col);
        }
        first_id.get_or_insert(id);
        if is_active {
            active_id = Some(id);
        }
    }
    // 安全启动下**不自动物化**任何文件(只留壳),避免上次未完成的启动循环。
    if safe_startup {
        tracing::warn!(target: "dozer::module::preview", "安全启动:预览 tab 仅恢复壳,不自动加载");
        return fx;
    }
    if let Some(id) = active_id.or(first_id) {
        if let Some(idx) = pane.tabs().iter().position(|t| t.id == id) {
            pane.select(idx);
        }
        load_preview_tab(pane, ctx, id, &mut fx);
    }
    fx
}

fn persist_and_context(
    state: &PreviewPanelState,
    ctx: &UpdateCtx<'_>,
    fx: &mut Vec<Effect>,
    flush: bool,
) {
    if !ctx.behavior.persist_tabs {
        return;
    }
    if let Some(project_id) = ctx.project_id {
        fx.push(Effect::PersistTabs {
            project_id,
            state: state.pane.persisted_snapshot(),
        });
    }
    fx.push(Effect::ContextChanged { immediate: flush });
}

fn reveal_active(state: &mut PreviewPanelState, ctx: &UpdateCtx<'_>, idx: usize) {
    let widths: Vec<f32> = state
        .pane
        .tabs()
        .iter()
        .map(|t| (ctx.tab_display_width)(&t.title))
        .collect();
    state.tab_first = byteui::interaction::tab_strip::tab_window_reveal(
        &widths,
        4.0,
        ctx.tab_bar_avail_px,
        state.tab_first,
        idx,
    );
}

fn open_path(
    state: &mut PreviewPanelState,
    ctx: &UpdateCtx<'_>,
    path: PathBuf,
    target_line: Option<usize>,
    fx: &mut Vec<Effect>,
) {
    if !path.is_file() {
        state.error = Some(crate::i18n::panel_keyed_args(
            PANEL_OWNER,
            "preview.file_unreadable",
            "The file does not exist or is unreadable: {path}",
            &[("path", &path.display().to_string())],
        ));
        return;
    }
    state.error = None;
    fx.push(Effect::PathOpened(path.clone()));
    fx.push(Effect::AllowFile(path.clone()));

    if let Some(idx) = state.pane.find_existing_file_tab(&path) {
        // 同一文件已开则切过去,不重复开 / 重复读盘。
        state.pane.select(idx);
        // 命中的是只恢复了壳的 tab:物化它。
        if ctx.behavior.materialize_suspended
            && let Some(tab_id) = state.pane.tabs().get(idx).map(|t| t.id)
            && state.pane.is_suspended(tab_id)
        {
            load_preview_tab(&mut state.pane, ctx, tab_id, fx);
        }
        if let Some(line) = target_line
            && let Some(tab) = state.pane.tabs_mut().get_mut(idx)
        {
            tab.pending_jump_line = Some(line);
        }
    } else {
        // UI 线程不读盘——建临时 route 壳(Profiling),画像丢后台。
        let (id, generation) = state.pane.open_path_provisional(path.clone());
        // CodeMirror tab 在 host `ready` 后排队 reveal;webview / 表格类无法跳转光标,target_line 忽略。
        if let Some(line) = target_line
            && let Some(tab) = state.pane.tabs_mut().iter_mut().find(|t| t.id == id)
            && tab.uses_editor_host()
        {
            tab.pending_jump_line = Some(line);
        }
        if let Some(generation) = generation {
            fx.push(Effect::ProbeProfile {
                tab_id: id,
                generation,
                path: path.clone(),
            });
        }
    }
    drain_tabular_loads_into(&mut state.pane, fx);
    let active = state.pane.active_idx();
    reveal_active(state, ctx, active);
    persist_and_context(state, ctx, fx, false);
}

fn select_tab(
    state: &mut PreviewPanelState,
    ctx: &UpdateCtx<'_>,
    idx: usize,
    fx: &mut Vec<Effect>,
) {
    let in_range = idx < state.pane.tabs().len();
    state.pane.select(idx);
    // 切到只恢复了壳的 tab 时物化它(其余 tab 保持 Suspended)。
    if ctx.behavior.materialize_suspended
        && let Some(tab_id) = state.pane.tabs().get(idx).map(|t| t.id)
        && state.pane.is_suspended(tab_id)
    {
        load_preview_tab(&mut state.pane, ctx, tab_id, fx);
    }
    if idx < state.pane.tabs().len() {
        reveal_active(state, ctx, idx);
        state.overflow_anchor = None;
    }
    // 切换 tab:上下文立即变,flush 不等防抖。
    persist_and_context(state, ctx, fx, true);
    if in_range {
        fx.push(Effect::TabSelected { index: idx });
    }
}

fn close_tab(state: &mut PreviewPanelState, ctx: &UpdateCtx<'_>, idx: usize, fx: &mut Vec<Effect>) {
    // 关闭前静默保存:CodeMirror tab 的正文活在 webview 里,Rust 不持有,改为下发
    // `SaveDocument`,等 host 回 `save_requested` 落盘后再关。此路径下这里**不**立刻 close。
    if state.pane.request_save_before_close_if_dirty(idx) {
        return;
    }
    let old_len = state.pane.tabs().len();
    state.pane.close(idx);
    // 关 tab 后位置全变,旧 first 可能越界——归零防御(P1L T5)。
    state.tab_first = 0;
    persist_and_context(state, ctx, fx, true);
    fx.push(Effect::TabClosed {
        index: idx,
        old_len,
    });
}

fn toggle_render_mode(
    state: &mut PreviewPanelState,
    ctx: &UpdateCtx<'_>,
    idx: usize,
    fx: &mut Vec<Effect>,
) {
    let in_code_mode = state
        .pane
        .tabs()
        .get(idx)
        .is_some_and(|t| t.uses_rendered_source_editor());
    if in_code_mode {
        state.pane.exit_code_mode(idx);
    } else if let Err(e) = state.pane.enter_code_mode(idx) {
        state.error = Some(crate::i18n::panel_keyed_args(
            PANEL_OWNER,
            "preview.enter_code_mode_failed",
            "Failed to switch to code mode: {e}",
            &[("e", &e.to_string())],
        ));
        drain_plantuml_reloads(&mut state.pane, ctx, fx);
        return;
    }
    // 切换进入 `SwitchingMode` 且仍在 Loading 时,arm 看门狗(等目标 host 首帧)。
    if let Some(tab) = state.pane.tabs().get(idx)
        && tab.load_state.stage == PreviewLoadStage::SwitchingMode
    {
        fx.push(Effect::ScheduleLoadTimeout {
            tab_id: tab.id,
            generation: tab.load_state.generation,
            stage: PreviewLoadStage::SwitchingMode,
        });
    }
    drain_plantuml_reloads(&mut state.pane, ctx, fx);
}

fn toggle_json_mode(pane: &mut PreviewPane, tab_id: usize) -> Option<(usize, u64)> {
    let backend = pane
        .tabs()
        .iter()
        .find(|t| t.id == tab_id)?
        .backend
        .as_ref()?;
    let target = match backend.current_mode() {
        PreviewMode::Tree => PreviewMode::Text,
        PreviewMode::Text => PreviewMode::Tree,
        _ => return None,
    };
    pane.restore_json_mode(tab_id, target)
}

fn find_open(pane: &mut PreviewPane, ctx: &UpdateCtx<'_>, with_replace: bool) {
    // 窗口化 CodeMirror tab:普通 Find 只搜持有窗口没意义,改为打开整文件流式搜索条
    // (大文件搜索条没有替换行,⌘R 的「默认展开替换」同样忽略)。
    let windowed_id = pane
        .tabs()
        .get(pane.active_idx())
        .filter(|t| t.uses_windowed_editor())
        .map(|t| t.id);
    if let Some(tab_id) = windowed_id {
        pane.open_large_file_search(tab_id);
    } else {
        pane.open_find_on_active(with_replace, ctx.active_webview_pool_id);
        pane.request_find_focus();
    }
}

fn plain_text_open(pane: &mut PreviewPane, tab_id: usize, fx: &mut Vec<Effect>) {
    let windowed = pane
        .tabs()
        .iter()
        .find(|t| t.id == tab_id)
        .is_some_and(|t| t.windowed && matches!(t.kind, TabKind::File(_)));
    if !pane.force_plain_text(tab_id) {
        return;
    }
    if windowed {
        spawn_window_index(pane, tab_id, fx);
    }
}

fn spawn_window_index(pane: &PreviewPane, tab_id: usize, fx: &mut Vec<Effect>) {
    let Some(tab) = pane.tabs().iter().find(|t| t.id == tab_id) else {
        return;
    };
    let TabKind::File(path) = &tab.kind else {
        return;
    };
    // T11:捕获该 tab 的取消信号,tab 关闭 / reload / 切 mode 时置位即尽早退出。
    fx.push(Effect::StartWindowIndex {
        tab_id,
        path: path.clone(),
        cancel: tab.task_cancel_token(),
    });
}

fn conflict_reload(
    pane: &mut PreviewPane,
    ctx: &UpdateCtx<'_>,
    tab_id: usize,
    fx: &mut Vec<Effect>,
) {
    // 第一次点击进入二次确认,再点一次才真正丢弃。
    let armed = pane
        .tabs()
        .iter()
        .find(|t| t.id == tab_id)
        .is_some_and(|t| t.conflict_reload_armed);
    if !armed {
        pane.arm_conflict_reload(tab_id);
        return;
    }
    let Some(path) = pane.discard_conflict_and_reload(tab_id) else {
        return;
    };
    if let Some(project_id) = ctx.project_id {
        fx.push(Effect::ClearRecovery { project_id, path });
    }
}

fn large_file_search_submit(
    pane: &mut PreviewPane,
    tab_id: usize,
    query: String,
    fx: &mut Vec<Effect>,
) {
    // T9 bullet 5:空查询立即取消在途查询并清 loading,不派任务。
    if query.trim().is_empty() {
        pane.clear_large_file_search(tab_id);
        return;
    }
    let Some(tab) = pane.tabs().iter().find(|t| t.id == tab_id) else {
        return;
    };
    let TabKind::File(path) = tab.kind.clone() else {
        return;
    };
    // T9 bullet 2:作废旧 generation(置位共享取消信号),新查询的 generation + 取消句柄捕获进后台任务。
    let Some((generation, cancel)) = pane.start_large_file_search(tab_id, query.clone()) else {
        return;
    };
    fx.push(Effect::StartLargeFileSearch {
        tab_id,
        generation,
        path,
        query,
        cancel,
    });
}

fn large_file_search_go(state: &mut PreviewPanelState, forward: bool) {
    let pane = &mut state.pane;
    let Some(session) = pane.large_file_search.clone() else {
        return;
    };
    if session.hits.is_empty() {
        return;
    }
    let len = session.hits.len();
    let next = if forward {
        (session.current + 1) % len
    } else {
        (session.current + len - 1) % len
    };
    let hit = session.hits[next].clone();
    // 窗口化只读:命中在未加载区,先装窗口再全局 reveal。
    let line = hit.line_no.max(1) as u32;
    pane.queue_windowed_view(session.tab_id, line);
    pane.queue_editor_command(
        session.tab_id,
        crate::EditorCommand::RevealPosition { line, column: 1 },
    );
    if let Some(s) = pane.large_file_search.as_mut() {
        s.current = next;
    }
    state.error = None;
}

/// 低于当前产品开启的后端的页签降级为「不支持」:不产生加载效果,降级页提供「系统应用打开」。
fn gate_backend(pane: &mut PreviewPane, ctx: &UpdateCtx<'_>, tab_id: usize) {
    let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == tab_id) else {
        return;
    };
    let (Some(backend), TabKind::File(path)) = (tab.backend.as_ref(), &tab.kind) else {
        return;
    };
    if backend_enabled(backend, path, ctx.caps) {
        return;
    }
    let reason = RouteReason::BackendDisabled;
    if let Some(route) = tab.route.as_mut() {
        route.kind = PreviewKind::Unsupported;
        route.reason = reason;
    }
    tab.backend = Some(PreviewBackend::Unsupported(UnsupportedBackend { reason }));
    // 降级后不再有运行时对象 / 后台任务(画像可能刚把它判成表格并已排了解析)。
    tab.runtime = crate::PreviewRuntime::None;
    tab.cancel_background();
    pane.pending_tabular_loads.retain(|(id, _)| *id != tab_id);
}

fn profiled(
    pane: &mut PreviewPane,
    ctx: &UpdateCtx<'_>,
    tab_id: usize,
    generation: u64,
    result: Result<crate::FileProfile, String>,
    fx: &mut Vec<Effect>,
) {
    // PlantUML 渲染视图的 include 授权读取在后台跑(CodeMirror 源码模式不需要)。
    let plantuml_path = pane
        .tabs()
        .iter()
        .find(|t| t.id == tab_id)
        .filter(|t| t.uses_plantuml_host())
        .and_then(|t| match &t.kind {
            TabKind::File(p) => Some(p.clone()),
            _ => None,
        });
    match result {
        Ok(profile) => {
            if !pane.apply_profile(tab_id, generation, &profile) {
                // generation 已过期(tab 关闭 / 重开 / 被替换):丢弃。
                return;
            }
            gate_backend(pane, ctx, tab_id);
            let (is_tabular, hosts_host) = pane
                .tabs()
                .iter()
                .find(|t| t.id == tab_id)
                .map(|t| (t.tabular_state().is_some(), t.hosts_any_webview()))
                .unwrap_or((false, false));
            let arm = |fx: &mut Vec<Effect>, stage| {
                fx.push(Effect::ScheduleLoadTimeout {
                    tab_id,
                    generation,
                    stage,
                })
            };
            if is_tabular {
                // 表格:进入 `Parsing`,等后台解析出的首个可显示 sheet 且 ag-grid host 回
                // `ready` + `window_applied` 才 finish;补上看门狗,不永久停在「正在解析文件…」。
                pane.advance_load(tab_id, generation, PreviewLoadStage::Parsing);
                arm(fx, PreviewLoadStage::Parsing);
            } else if !hosts_host {
                // 无 host 的 fallback 页:直接就绪。
                pane.finish_load(tab_id, generation);
            } else {
                // 窗口化与非窗口化 host 都保持 Loading。先进入 `Reserving` 等资源预算;
                // 无编辑器 host 的渲染类(Flyfish / HTML)不参与 reserve,直达 `CreatingHost`。
                let (rendered_host, needs_reserve) = pane
                    .tabs()
                    .iter()
                    .find(|t| t.id == tab_id)
                    .map(|t| {
                        (
                            t.hosts_webview(),
                            t.uses_editor_host() || t.uses_json_editor() || t.uses_plantuml_host(),
                        )
                    })
                    .unwrap_or((false, false));
                if rendered_host {
                    // T8 bullet 5:Rendered host 可能因外链 / 相对资源 / 网络卡住,arm host-ready 超时。
                    arm(fx, PreviewLoadStage::CreatingHost);
                }
                if needs_reserve {
                    // 等 `sync_webview_pool` 回灌 `granted` 再进 `CreatingHost`;等预算最长也有上限。
                    pane.advance_load(tab_id, generation, PreviewLoadStage::Reserving);
                    arm(fx, PreviewLoadStage::Reserving);
                } else {
                    pane.advance_load(tab_id, generation, PreviewLoadStage::CreatingHost);
                    arm(fx, PreviewLoadStage::CreatingHost);
                }
            }
        }
        Err(error) => {
            pane.fail_load(tab_id, generation, crate::PreviewError::new(error, true));
        }
    }
    // 画像落定后,若该 tab 走 PlantUML 渲染 host,起后台授权读取。
    if let (Some(path), Some(root)) = (plantuml_path, ctx.project_root)
        && pane
            .tabs()
            .iter()
            .any(|t| t.id == tab_id && t.uses_plantuml_host())
    {
        fx.push(Effect::LoadPlantUml {
            tab_id,
            generation,
            project_root: root.to_path_buf(),
            path,
        });
    }
    // 画像可能把临时非表格 route 改判为表格:此时才入队,需立即起解析。
    drain_tabular_loads_into(pane, fx);
}

fn window_index(
    pane: &mut PreviewPane,
    ctx: &UpdateCtx<'_>,
    tab_id: usize,
    result: Result<std::sync::Arc<crate::LineIndex>, String>,
    fx: &mut Vec<Effect>,
) {
    // T11 bullet 3:结构化日志(读取字节 + 行数,不记正文)。
    match &result {
        Ok(index) => {
            tracing::debug!(target: "dozer::module::preview", panel = ?ctx.panel, tab_id, bytes = index.file_len(), total_lines = index.total_lines(), "大文件行索引建立完成")
        }
        Err(error) => {
            tracing::warn!(target: "dozer::module::preview", panel = ?ctx.panel, tab_id, %error, "大文件行索引建立失败")
        }
    }
    match result {
        Ok(index) => {
            // 过期索引(文件 revision 已变)必须丢弃,不得套到新内容上。
            if !pane.apply_window_index(tab_id, index) {
                return;
            }
            let jump = pane
                .tabs_mut()
                .iter_mut()
                .find(|t| t.id == tab_id)
                .and_then(|t| t.pending_jump_line.take())
                .map(|l| l as u32);
            // 索引就绪:推初始窗口(有跳转诉求就以目标行为中心,窗口就位后再 reveal)。
            let center = jump.unwrap_or(1);
            // T4:进入 LoadingWindow,等 host 回 `window_applied` 才 finish。
            let generation = pane.load_generation(tab_id);
            pane.advance_load(tab_id, generation, PreviewLoadStage::LoadingWindow);
            fx.push(Effect::ScheduleLoadTimeout {
                tab_id,
                generation,
                stage: PreviewLoadStage::LoadingWindow,
            });
            pane.queue_windowed_view(tab_id, center);
            if let Some(line) = jump {
                pane.queue_editor_command(
                    tab_id,
                    crate::EditorCommand::RevealPosition { line, column: 1 },
                );
            }
        }
        Err(error) => {
            if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == tab_id) {
                tab.web_error = Some(crate::i18n::panel_text_args(
                    PANEL_OWNER,
                    "preview.line_index_failed",
                    "Failed to build line index: {error}",
                    ctx.locale,
                    &[("error", &error)],
                ));
            }
        }
    }
}

fn recovery_read(pane: &mut PreviewPane, tab_id: usize, generation: u64, restore: Option<String>) {
    let Some(text) = restore else {
        return;
    };
    // 通常加载仍在途 → 记入 pending_restore,由 DocumentLoaded 消费;若 recovery 读比 host 就绪还慢,
    // 由本方法就地生成恢复指令。
    if let Some((tab_id, text, revision)) = pane.apply_recovery_restore(tab_id, generation, text) {
        let language = pane
            .tabs()
            .iter()
            .find(|t| t.id == tab_id)
            .and_then(|t| match &t.kind {
                TabKind::File(p) => Some(crate::extension_to_syntax(p)),
                _ => None,
            })
            .unwrap_or_default();
        pane.queue_editor_command(
            tab_id,
            crate::EditorCommand::SetDocument {
                text,
                revision,
                language,
                read_only: false,
            },
        );
    }
}

fn tabular_loaded(
    pane: &mut PreviewPane,
    ctx: &UpdateCtx<'_>,
    tab_id: usize,
    generation: u64,
    result: Result<crate::tabular::TabularView, String>,
    fx: &mut Vec<Effect>,
) {
    // T11:取消是正常结束,不当作解析失败告警。
    match &result {
        Err(error) if error == crate::tabular::TABULAR_CANCELLED => {
            tracing::debug!(target: "dozer::module::preview", panel = ?ctx.panel, tab_id, generation, "表格加载已取消")
        }
        Err(error) => {
            tracing::warn!(target: "dozer::module::preview", panel = ?ctx.panel, tab_id, generation, %error, "表格首次加载失败")
        }
        Ok(view) => {
            tracing::debug!(target: "dozer::module::preview", panel = ?ctx.panel, tab_id, generation, sheets = view.sheet_names.len(), "表格首次加载完成")
        }
    }
    let sheet_to_select = pane.finish_tabular_load(tab_id, generation, result);
    // 恢复的 active sheet 不是首个 → 触发一次懒加载。`select_sheet` 会把滚动重置为 0,
    // 因此要在它之后把持久化的 row/col 重新应用一遍。
    if let Some((sheet, row, col)) = sheet_to_select {
        tabular_action(pane, tab_id, sheet, fx);
        if let Some(view) = pane.tabular_mut(tab_id) {
            view.scroll_row = row;
            view.scroll_col = col;
        }
    }
}

/// 表格交互动作(sheet 切换):尚未加载的 sheet 起懒加载并让 webview 标 loading 态。
pub fn tabular_action(pane: &mut PreviewPane, tab_id: usize, sheet: usize, fx: &mut Vec<Effect>) {
    let Some(request) = pane
        .tabular_mut(tab_id)
        .and_then(|view| view.select_sheet(sheet))
    else {
        return;
    };
    pane.queue_tabular_command(
        tab_id,
        crate::TabularCommand::SetSheetLoading {
            sheet_index: request.index,
            loading: true,
        },
    );
    // T11:sheet 懒加载同样可取消(tab 关闭 / 切换即弃)。
    let cancel = pane
        .tabs()
        .iter()
        .find(|t| t.id == tab_id)
        .map(|t| t.task_cancel_token())
        .unwrap_or_default();
    fx.push(Effect::LoadTabularSheet {
        tab_id,
        request,
        cancel,
    });
}

fn tabular_sheet_loaded(
    pane: &mut PreviewPane,
    tab_id: usize,
    sheet_index: usize,
    result: Result<crate::tabular::Sheet, String>,
) {
    let loaded_ok = result.is_ok();
    if let Some(crate::TabularState::Ready(view)) = pane.tabular_state_mut(tab_id) {
        view.apply_sheet_loaded(sheet_index, result);
    }
    pane.queue_tabular_command(
        tab_id,
        crate::TabularCommand::SetSheetLoading {
            sheet_index,
            loading: false,
        },
    );
    // 用户可能在这次懒加载完成前又切到了别的 sheet——不该把 JS 拽回已不再是目标的 sheet。
    let still_active = pane
        .tabular_mut(tab_id)
        .is_some_and(|view| view.active_sheet == sheet_index);
    if loaded_ok && still_active {
        pane.queue_tabular_command(tab_id, crate::TabularCommand::SelectSheet { sheet_index });
        pane.push_sheet_schema_and_window(tab_id, sheet_index);
    }
}

/// 把窗格刚 `open_path` / `push_tab` 攒下的待加载表格 tab 转成解析效果。
pub fn drain_tabular_loads(pane: &mut PreviewPane) -> Vec<Effect> {
    let mut fx = Vec::new();
    drain_tabular_loads_into(pane, &mut fx);
    fx
}

fn drain_tabular_loads_into(pane: &mut PreviewPane, fx: &mut Vec<Effect>) {
    for (tab_id, path) in pane.take_pending_tabular_loads() {
        // 捕获启动时的 generation:tab 关闭 / 重开 / 重试后到达的旧结果按此丢弃。
        let (generation, cancel) = match pane.tabs().iter().find(|t| t.id == tab_id) {
            Some(tab) => (tab.load_state.generation, tab.task_cancel_token()),
            None => (0, Default::default()),
        };
        fx.push(Effect::LoadTabular {
            tab_id,
            generation,
            path,
            cancel,
        });
    }
}

/// 把 `reload_webviews_for` 攒下的待重载 PlantUML tab 转成授权读取效果。
pub fn drain_plantuml_reloads(pane: &mut PreviewPane, ctx: &UpdateCtx<'_>, fx: &mut Vec<Effect>) {
    let Some(root) = ctx.project_root else {
        return;
    };
    for (tab_id, path) in pane.take_pending_plantuml_reloads() {
        let generation = pane
            .tabs()
            .iter()
            .find(|t| t.id == tab_id)
            .map_or(0, |t| t.load_state.generation);
        fx.push(Effect::LoadPlantUml {
            tab_id,
            generation,
            project_root: root.to_path_buf(),
            path,
        });
    }
}

/// 物化一个 `Suspended` 预览壳:按 route 分派到对应的加载起点。幂等——非 Suspended 直接返回。
/// host 类路由**不在此处 finish**:先进入 `Reserving`(需占预算的 host)或 `CreatingHost`
/// (渲染 host),由后续 ACK 结束加载;纯 iced / fallback 路由无 host 可等,直接 finish。
pub fn load_preview_tab(
    pane: &mut PreviewPane,
    ctx: &UpdateCtx<'_>,
    tab_id: usize,
    fx: &mut Vec<Effect>,
) {
    use PreviewLoadStage as Stage;
    if !pane.is_pending_load(tab_id) {
        return;
    }
    gate_backend(pane, ctx, tab_id);
    let Some(idx) = pane.tabs().iter().position(|t| t.id == tab_id) else {
        return;
    };
    let route_kind = pane.tabs()[idx].route.as_ref().map(|r| r.kind);
    let mode = pane.tabs()[idx].route.as_ref().map(|r| r.default_mode);
    let TabKind::File(path) = pane.tabs()[idx].kind.clone() else {
        return;
    };
    let editor_host = pane.tabs()[idx].uses_editor_host();
    let windowed = pane.tabs()[idx].windowed;
    let needs_reserve_route =
        pane.tabs()[idx].uses_editor_host() || pane.tabs()[idx].uses_json_editor();
    let arm = |fx: &mut Vec<Effect>, generation, stage| {
        fx.push(Effect::ScheduleLoadTimeout {
            tab_id,
            generation,
            stage,
        })
    };
    let load_generation: Option<u64> = match route_kind {
        // CodeMirror(含窗口化只读):editor WebView 自取内容 / 由 Rust 推窗口。
        Some(PreviewKind::Code) if editor_host => pane.begin_load(tab_id, Stage::Reserving),
        // JSON 家族:严格 .json 走 vanilla-jsoneditor host,JSONC/JSON5 走 CodeMirror 文本。
        Some(PreviewKind::Json) => {
            if needs_reserve_route {
                pane.begin_load(tab_id, Stage::Reserving)
            } else {
                pane.begin_load(tab_id, Stage::Parsing)
            }
        }
        // 流式 JSONL/NDJSON(T8):editor host。
        Some(PreviewKind::Streamed) => pane.begin_load(tab_id, Stage::Reserving),
        // 其余 Code 到 editor host。
        Some(PreviewKind::Code) => pane.begin_load(tab_id, Stage::Reserving),
        Some(PreviewKind::Tabular) => {
            // 首次解析:进入 `Parsing`,网格后台加载完成前保持 Loading(含 host 握手看门狗)。
            if let Some(generation) = pane.begin_load(tab_id, Stage::Parsing) {
                pane.set_tabular_loading(tab_id, path.clone());
                arm(fx, generation, Stage::Parsing);
                // CSV/TSV 原文模式:同时让 editor host 就绪;原文 host 需占预算,故回到 `Reserving`。
                if editor_host {
                    pane.advance_load(tab_id, generation, Stage::Reserving);
                }
            }
            None
        }
        // 渲染:交给 Flyfish / 隔离 HTML host,等其 `document_loaded` 才 finish。
        Some(PreviewKind::Rendered) => {
            if let Some(generation) = pane.begin_load(tab_id, Stage::CreatingHost) {
                arm(fx, generation, Stage::CreatingHost);
                Some(generation)
            } else {
                None
            }
        }
        // 外部 / 不支持:无 host,由 fallback 页承载,直接就绪。
        Some(PreviewKind::External) | Some(PreviewKind::Unsupported) => {
            if let Some(generation) = pane.begin_load(tab_id, Stage::CreatingHost) {
                pane.finish_load(tab_id, generation);
            }
            None
        }
        None => None,
    };

    // T10 bullet 3:dirty recovery 读取 + 磁盘冲突分类挪到后台。窗口化正文只读、不走 recovery。
    if !windowed
        && let Some(generation) = load_generation
        && let Some(project_id) = ctx.project_id
    {
        let cancel = pane
            .tabs()
            .iter()
            .find(|t| t.id == tab_id)
            .map(|t| t.task_cancel_token())
            .unwrap_or_default();
        fx.push(Effect::ReadRecovery {
            project_id,
            tab_id,
            generation,
            path,
            cancel,
        });
    }

    // 侧载的后台任务(表格)。
    drain_tabular_loads_into(pane, fx);

    // Rendered 的 Source 持久模式:物化后切到源码视图。
    if mode == Some(PreviewMode::Source)
        && route_kind == Some(PreviewKind::Rendered)
        && let Some(idx) = pane.tabs().iter().position(|t| t.id == tab_id)
        && let Err(error) = pane.enter_code_mode(idx)
    {
        tracing::warn!(target: "dozer::module::preview", %error, "恢复预览源码模式失败,回退到渲染模式");
    }
}
