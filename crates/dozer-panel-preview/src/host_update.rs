//! 页面事件(`host_ipc::HostEvent`)与保存回执的处理(S5b-2 P4)。
//!
//! 从 `dozer-app/src/app/update.rs` 搬来的「只碰窗格 / 核心类型」的那一半。分支
//! 顺序与判定逐条保持原样;副作用一律以 [`Effect`] 返回:
//!
//! - 写盘 → [`Effect::SaveFile`],清快照 → [`Effect::ClearRecoverySnapshot`],
//!   建索引 → [`Effect::BuildLineIndex`],脏快照 → [`Effect::WriteRecoverySnapshot`],
//!   看门狗 → [`Effect::ScheduleLoadTimeout`];
//! - 选区右键 → [`Effect::SelectionContextMenu`](产品自己弹菜单,Dozer 保留原行为,
//!   Digger 忽略);
//! - 失败计数落到 `PreviewPersistence::failures_path()`(Q1)。
//!
//! `GitLogDiff` / `ReviewTrace` 两个宿主绑定在面板之外的单槽上,仍由产品处理。

use std::path::PathBuf;
use std::sync::Arc;

use crate::effects::Effect;
use crate::host_ipc::HostEvent;
use crate::panel::PreviewPanelState;
use crate::update::UpdateCtx;
use crate::{
    BackendState, EditorEvent, FlyfishEvent, JsonEvent, PANEL_OWNER, PlantUmlEvent, PreviewError,
    PreviewLoadStage, PreviewPane, PreviewRuntime, SaveGate, TabKind, TabularEvent,
};

const TARGET: &str = "dozer::module::preview";

/// 一条页面事件。`ctx` 里的 `project_id` 必须与事件绑定一致(产品已按项目路由)。
pub fn host_event(
    state: &mut PreviewPanelState,
    event: HostEvent,
    ctx: &UpdateCtx<'_>,
    fx: &mut Vec<Effect>,
) {
    match event {
        HostEvent::Editor { binding, envelope } => editor_event(state, &binding, envelope, ctx, fx),
        HostEvent::Json { binding, envelope } => {
            json_event(&mut state.pane, &binding, envelope, ctx, fx)
        }
        HostEvent::Tabular { binding, envelope } => {
            tabular_event(&mut state.pane, &binding, envelope, ctx, fx)
        }
        HostEvent::Flyfish { binding, envelope } => {
            if ctx.caps.rendered {
                flyfish_event(&mut state.pane, &binding, envelope, ctx);
            }
        }
        HostEvent::ImageAnnotate { binding, envelope } => {
            state
                .pane
                .apply_image_annotate_event(binding.tab_id, envelope.payload);
        }
        HostEvent::PlantUml { binding, envelope } => {
            plantuml_event(&mut state.pane, &binding, envelope);
        }
        // 面板外单槽宿主:产品处理。
        HostEvent::GitLogDiff { .. } | HostEvent::ReviewTrace(_) => {}
    }
}

/// 落盘回执的目标:清脏 + 清恢复快照 + (若在等关闭)关掉页签。`saved_revision`
/// 是发起写盘时的页签 revision,回执时与当前 `web_revision` 比对——期间的编辑
/// 不能被这次旧写盘清掉。
pub fn save_finished(
    state: &mut PreviewPanelState,
    ctx: &UpdateCtx<'_>,
    tab_id: usize,
    revision: u64,
    result: Result<(), String>,
    fx: &mut Vec<Effect>,
) {
    // `None` = 这次回执不能当作"已落盘":写盘期间又有编辑(`revision` 落后于
    // 页签的 `web_revision`),或写盘失败。
    let mut saved_path = None;
    let mut receipt_is_current = false;
    {
        let pane = &mut state.pane;
        let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        match result {
            Ok(()) => {
                // 写盘期间若又有编辑(`revision` 已前进),这次旧写盘不能清脏,
                // 也**不能**清恢复快照——那会让崩溃丢掉这段时间的编辑(N2)。
                if revision >= tab.web_revision {
                    tab.dirty = false;
                    tab.recovery_written = false;
                    tab.conflict_baseline = None;
                    if let TabKind::File(path) = &tab.kind {
                        saved_path = Some(path.clone());
                    }
                    receipt_is_current = true;
                }
            }
            Err(error) => {
                tab.web_error = Some(localized(
                    "text_save.failed",
                    "Save failed: {message}",
                    ctx.locale,
                    &[("message", &error)],
                ));
            }
        }
    }
    if let (Some(path), Some(dir), Some(project_id)) =
        (saved_path, ctx.recovery_dir, ctx.project_id)
    {
        fx.push(Effect::ClearRecoverySnapshot {
            dir: dir.to_path_buf(),
            project_id,
            path,
        });
    }
    if ctx.behavior.persist_tabs
        && let Some(project_id) = ctx.project_id
        && receipt_is_current
    {
        // 仅在回执对应当前内容时更新落盘快照;过期回执会把旧内容写回去。
        fx.push(Effect::PersistTabs {
            project_id,
            state: state.pane.persisted_snapshot(),
        });
    }
    if !receipt_is_current {
        // N2:revision 过期(或写盘失败)时**不关**页签——关掉就丢了写盘期间
        // 的新编辑。`pending_close` 保留,页签仍显示脏标;用户下一次保存 / 关闭
        // 会按新的 revision 重新走一遍(库不替用户决定覆盖)。
        fx.push(Effect::ContextChanged { immediate: false });
        return;
    }
    // 「保存后关闭」:只有**当前内容**落盘成功后才真正移除 tab。
    if state.pane.take_pending_close(tab_id) {
        let old_len = state.pane.tabs().len();
        let idx = state
            .pane
            .tabs()
            .iter()
            .position(|t| t.id == tab_id)
            .unwrap_or(0);
        state.pane.close(idx);
        state.tab_first = 0;
        fx.push(Effect::TabClosed {
            index: idx,
            old_len,
        });
    }
    if ctx.behavior.persist_tabs
        && let Some(project_id) = ctx.project_id
    {
        fx.push(Effect::PersistTabs {
            project_id,
            state: state.pane.persisted_snapshot(),
        });
    }
    fx.push(Effect::ContextChanged { immediate: false });
}
fn localized(key: &str, fallback: &str, locale: &str, args: &[(&str, &str)]) -> String {
    if args.is_empty() {
        crate::i18n::panel_text(PANEL_OWNER, key, fallback, locale)
    } else {
        crate::i18n::panel_text_args(PANEL_OWNER, key, fallback, locale, args)
    }
}

/// D-I3:保存路径必须落在项目根内。先看词法分量(挡住 `..` 逃逸),再把两侧 canonical 后做前缀比较
/// (挡住指向根外的符号链接)。目标文件可能不存在(刚被删 / 另存为新文件):这时 canonical 它**已存在的
/// 父目录**再拼回文件名,同样要落在根里;任何一步解析不了都**拒绝**(fail-closed),不再凭词法放行。
pub(crate) fn path_within_root(root: &std::path::Path, path: &std::path::Path) -> bool {
    // `strip_prefix` 按分量匹配,但 `proj/../outside` 的词法前缀也是 `proj`,
    // 所以相对部分里出现 `..` 一律拒绝。
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    if rel
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return false;
    }
    let Ok(croot) = std::fs::canonicalize(root) else {
        return false;
    };
    let resolved = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => {
            let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                return false;
            };
            match std::fs::canonicalize(parent) {
                Ok(p) => p.join(name),
                Err(_) => return false,
            }
        }
    };
    resolved.starts_with(&croot)
}

fn editor_event(
    state: &mut PreviewPanelState,
    binding: &crate::EditorHostBinding,
    envelope: crate::WebviewEnvelope<EditorEvent>,
    ctx: &UpdateCtx<'_>,
    fx: &mut Vec<Effect>,
) {
    let pane = &mut state.pane;
    let event = envelope.payload;
    let revision = envelope.revision;
    // 关闭前保存:先探明,后面 tab 借用内不能再借 pane。
    let want_close = pane.has_pending_close(binding.tab_id);
    // N2:只有"保存这件事已经有结论"的事件才能收尾关闭。编辑类事件(正文又变了 /
    // 快照 / 选区)不代表保存结束,那时关页签会把保存在途期间的新编辑丢掉。
    let mut close_settled = false;
    let mut save_queued = false;
    let mut pending_reveal: Option<(usize, u32)> = None;
    let mut context_changed = false;
    let mut build_index: Option<(usize, PathBuf, u64, Arc<std::sync::atomic::AtomicBool>)> = None;
    let mut window_request: Option<(usize, u32)> = None;
    let mut pending_restore_cmd: Option<(usize, String, u64)> = None;
    let mut pending_view_cmd: Option<(usize, crate::EditorCommand)> = None;
    let mut find_request: Option<usize> = None;
    let mut window_applied: Option<usize> = None;
    {
        let Some(tab) = pane
            .tabs_mut()
            .iter_mut()
            .find(|tab| tab.id == binding.tab_id)
        else {
            return;
        };
        let TabKind::File(path) = &tab.kind else {
            return;
        };
        if path != &binding.path || revision < tab.web_revision {
            return;
        }
        let path = path.clone();
        match event {
            EditorEvent::Ready { .. } => {
                tab.web_revision = revision;
                tab.web_error = None;
                crate::reset_failure_to(&ctx.failures_path, &path);
                // 只有仍在该次加载(在途)时才接受 host `ready`。
                if tab.load_state.is_active() {
                    if tab.uses_windowed_editor() {
                        // 窗口化 host 的 `ready` 只代表 host JS 初始化完成(空 doc),
                        // 正文要等首个 SetWindow。
                        tab.load_state.advance(PreviewLoadStage::Indexing);
                        fx.push(Effect::ScheduleLoadTimeout {
                            tab_id: tab.id,
                            generation: tab.load_state.generation,
                            stage: PreviewLoadStage::Indexing,
                        });
                        if tab.window_index().is_none() {
                            build_index = Some((
                                tab.id,
                                path.clone(),
                                tab.web_revision,
                                tab.task_cancel_token(),
                            ));
                        }
                    } else {
                        // 非窗口化 host 的 `ready` 仅代表 host JS 初始化完成——正文由
                        // host 自行 fetch,待其回 `document_loaded` 才 finish。
                        tab.load_state.advance(PreviewLoadStage::Reading);
                        fx.push(Effect::ScheduleLoadTimeout {
                            tab_id: tab.id,
                            generation: tab.load_state.generation,
                            stage: PreviewLoadStage::Reading,
                        });
                    }
                }
                context_changed = true;
            }
            EditorEvent::DocumentLoaded {
                revision: doc_rev,
                bytes: _,
                error,
            } => {
                tab.web_revision = revision;
                let ack_active = tab.load_state.is_active();
                if ack_active {
                    if let Some(message) = error {
                        tab.web_error = Some(message.clone());
                        let _ = tab
                            .backend_state
                            .try_transition(BackendState::Failed(PreviewError::new(message, true)));
                        tab.load_state.finish();
                    } else if !tab.uses_windowed_editor() && doc_rev == revision {
                        tab.web_error = None;
                        crate::reset_failure_to(&ctx.failures_path, &path);
                        let _ = tab.backend_state.try_transition(BackendState::Ready);
                        tab.load_state.finish();
                        if let Some(line) = tab.pending_jump_line.take() {
                            pending_reveal = Some((tab.id, line as u32));
                        }
                        // ready 观测(不记文件内容)。
                        if let Some(observe) = tab.load_observe.take() {
                            let ready_ms = observe.elapsed_ms();
                            let first_frame_ms = observe.first_frame_offset_ms();
                            tracing::info!(target: TARGET,
                                panel = ?ctx.panel,
                                tab_id = tab.id,
                                generation = observe.generation,
                                ready_ms,
                                first_frame_ms = ?first_frame_ms,
                                first_frame_to_ready_ms = first_frame_ms
                                    .map(|f| ready_ms.saturating_sub(f)),
                                "预览 tab ready(三段延迟)"
                            );
                        }
                        // recovery 恢复:回推正文并重新标脏。
                        if let Some(text) = tab.pending_restore.take() {
                            tab.dirty = true;
                            tab.recovery_written = true;
                            pending_restore_cmd = Some((tab.id, text, tab.web_revision));
                        }
                        // 视图状态恢复:一次 RestoreViewState(空快照不发命令)。
                        if let Some(state) = tab.pending_view.take()
                            && !state.is_empty()
                        {
                            pending_view_cmd = Some((
                                tab.id,
                                crate::EditorCommand::RestoreViewState {
                                    cursor: state.cursor,
                                    selection: state.selection,
                                    top_line: state.top_line,
                                    folds: state.folds,
                                },
                            ));
                        }
                    }
                }
                context_changed = true;
            }
            EditorEvent::SelectionChanged {
                anchor,
                head,
                selected_text,
                ..
            } => {
                tab.web_revision = revision;
                tab.web_selection = Some(crate::TextRange {
                    start: anchor,
                    end: head,
                });
                // 选区文本上限与 `dozer-model` 的协议常量一致(4096 字符)。
                tab.web_selected_text = selected_text.map(|text| text.chars().take(4096).collect());
                context_changed = true;
            }
            EditorEvent::ViewportChanged { from_line, to_line } => {
                tab.web_revision = revision;
                tab.web_viewport = Some((from_line, to_line));
                context_changed = true;
            }
            // host 报的 `revision` 与 envelope 的一致时才能采纳(旧事件 replay
            // 不能把 newer 的 `web_revision` 推回去)。
            EditorEvent::DocumentChanged {
                revision: doc_revision,
                ..
            } => {
                if doc_revision == revision && doc_revision >= tab.web_revision {
                    tab.web_revision = doc_revision;
                    tab.dirty = true;
                }
            }
            EditorEvent::SaveRequested {
                revision: save_revision,
                text: save_text,
            } => {
                // 关闭前保存的 tab:不管 revision 是否对齐,落盘尝试完都必须完成
                // 关闭,否则 tab 卡在等待里永远关不掉。
                match tab.save_gate() {
                    SaveGate::ReadOnly => {
                        tab.web_error = Some(localized(
                            "text_save.readonly",
                            "This file is read-only or non-UTF-8 text; saving is disabled to avoid corrupting the original",
                            ctx.locale,
                            &[],
                        ));
                        close_settled = true;
                    }
                    SaveGate::Conflict => {
                        tab.web_error = Some(localized(
                            "text_save.conflict",
                            "The file on disk changed externally; first choose \"Keep my changes\" or \"Reload from disk\".",
                            ctx.locale,
                            &[],
                        ));
                        close_settled = true;
                    }
                    SaveGate::Allow => {
                        // D-I3:路径必须仍在项目根内。`binding.path` 是 Rust 侧绑定的
                        // 路径(`path` 已被前面校验过与它一致),所以看 `path`。
                        let outside_root = ctx
                            .project_root
                            .is_some_and(|root| !path_within_root(root, &path));
                        // D-I3:IPC 侧只挡「整条消息」的 8 MiB;一条合法的
                        // `save_requested` 里塞接近上限的正文仍会让写盘阻塞很久。
                        // 保存正文本身单独限一次,超限直接报错、不落盘。
                        let too_large =
                            save_text.len() > crate::webview_protocol::MAX_MESSAGE_BYTES;
                        if too_large {
                            tab.web_error = Some(localized(
                                "text_save.too_large",
                                "This document is too large to save.",
                                ctx.locale,
                                &[],
                            ));
                            close_settled = true;
                        } else if outside_root {
                            tab.web_error = Some(localized(
                                "text_save.outside_root",
                                "This file is outside the project; refusing to save.",
                                ctx.locale,
                                &[],
                            ));
                            close_settled = true;
                        } else if save_revision == revision && save_revision == tab.web_revision {
                            save_queued = true;
                            fx.push(Effect::SaveFile {
                                tab_id: tab.id,
                                path: path.clone(),
                                text: save_text,
                                revision: save_revision,
                            });
                        } else if save_revision > tab.web_revision {
                            // 保存内容比页签新(host 之后又静默推进过):页签里的
                            // 东西已经全在这份保存里了,关闭是安全的。
                            close_settled = true;
                        } else if !want_close {
                            // 非关闭场景下的过期保存:与旧行为一致,丢弃。
                            return;
                        } else {
                            // N2:关闭时保存内容比页签旧 —— 落盘会覆盖掉保存期间
                            // 产生的新编辑,关页签则直接丢掉它们。两者都错,所以
                            // 保留 `pending_close`,等 host 再回一次对齐当前
                            // revision 的保存。
                            close_settled = false;
                        }
                    }
                }
            }
            EditorEvent::Snapshot {
                revision: snap_revision,
                text,
            } => {
                // 防抖脏快照:原子写 recovery。仅在 revision 不回退时接受。
                if snap_revision >= tab.web_revision {
                    tab.web_revision = snap_revision;
                    if let (Some(dir), Some(project_id)) = (ctx.recovery_dir, ctx.project_id) {
                        fx.push(Effect::WriteRecoverySnapshot {
                            dir: dir.to_path_buf(),
                            project_id,
                            path: path.clone(),
                            revision: snap_revision,
                            cursor: tab.web_selection.map(|range| range.end),
                            selection: tab.web_selection,
                            top_line: tab.web_viewport.map(|(from, _)| from),
                            text,
                            tab_id: tab.id,
                        });
                    }
                }
            }
            EditorEvent::ViewState {
                cursor,
                selection,
                top_line,
                folds,
            } => {
                tab.web_revision = revision;
                tab.web_selection = selection;
                tab.web_view_state = Some(crate::ViewStateRestore {
                    cursor: Some(cursor),
                    selection,
                    top_line: Some(top_line),
                    folds,
                });
                if top_line > 0 {
                    let to = tab.web_viewport.map(|(_, to)| to).unwrap_or(top_line);
                    tab.web_viewport = Some((top_line, to));
                }
                context_changed = true;
            }
            EditorEvent::WindowRequest { anchor_line, .. } => {
                window_request = Some((tab.id, anchor_line));
            }
            EditorEvent::WindowApplied { start_line: _ } => {
                window_applied = Some(tab.id);
                context_changed = true;
            }
            EditorEvent::FindRequest => {
                find_request = Some(tab.id);
            }
            EditorEvent::Failed {
                message,
                recoverable,
            } => {
                // 连续失败计数:达到阈值后提示降级,而不是无限自动重试。
                let count = crate::bump_failure_to(&ctx.failures_path, &path);
                let detail = if count >= crate::FAILURE_THRESHOLD {
                    localized(
                        "text_save.repeated_failure",
                        "{message} (failed {count} times in a row; switch to plain text or open externally)",
                        ctx.locale,
                        &[("message", message.as_str()), ("count", &count.to_string())],
                    )
                } else {
                    message.clone()
                };
                tab.web_error = Some(detail.clone());
                let _ = tab
                    .backend_state
                    .try_transition(BackendState::Failed(PreviewError::new(detail, recoverable)));
                if tab.load_state.is_active() {
                    tab.load_state.finish();
                }
            }
            EditorEvent::FocusChanged { .. } => {}
            EditorEvent::ContextMenuRequested {
                x,
                y,
                range,
                selected_text,
            } => {
                fx.push(Effect::SelectionContextMenu {
                    panel: ctx.panel,
                    tab_id: binding.tab_id,
                    path: path.clone(),
                    x,
                    y,
                    range,
                    selected_text,
                });
            }
        }
    }
    if let Some((tab_id, line)) = pending_reveal {
        pane.queue_editor_command(
            tab_id,
            crate::EditorCommand::RevealPosition { line, column: 1 },
        );
    }
    if let Some((tab_id, path, revision, cancel)) = build_index {
        fx.push(Effect::BuildLineIndex {
            tab_id,
            path,
            revision,
            cancel,
        });
    }
    if let Some((tab_id, anchor_line)) = window_request {
        pane.queue_windowed_view(tab_id, anchor_line);
    }
    if let Some(tab_id) = window_applied {
        let active = pane
            .tabs()
            .iter()
            .find(|t| t.id == tab_id)
            .is_some_and(|t| t.load_state.is_active());
        if active {
            let generation = pane.load_generation(tab_id);
            pane.finish_load(tab_id, generation);
        }
    }
    if let Some((tab_id, text, revision)) = pending_restore_cmd {
        let language = crate::extension_to_syntax(&binding.path);
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
    if let Some((tab_id, cmd)) = pending_view_cmd {
        pane.queue_editor_command(tab_id, cmd);
    }
    if let Some(tab_id) = find_request {
        pane.open_large_file_search(tab_id);
    }
    // 关闭与否由产品在 `SaveFinished` 里收尾(见 `save_finished`);只有真正排了
    // 写盘的那条路径有这个收尾,其余路径(只读 / 冲突 / revision 不符)必须在这里
    // 自己把关闭做完,否则页签永远卡在 `pending_close`。
    let pending_close = want_close && close_settled && !save_queued;
    if context_changed {
        fx.push(Effect::ContextChanged { immediate: false });
    }
    if pending_close {
        close_pending_tab(state, binding.tab_id, ctx, fx);
    }
}

fn close_pending_tab(
    state: &mut PreviewPanelState,
    tab_id: usize,
    ctx: &UpdateCtx<'_>,
    fx: &mut Vec<Effect>,
) {
    let pane = &mut state.pane;
    if !pane.take_pending_close(tab_id) {
        return;
    }
    let old_len = pane.tabs().len();
    let idx = pane.tabs().iter().position(|t| t.id == tab_id).unwrap_or(0);
    pane.close(idx);
    state.tab_first = 0;
    if ctx.behavior.persist_tabs
        && let Some(project_id) = ctx.project_id
    {
        fx.push(Effect::PersistTabs {
            project_id,
            state: pane.persisted_snapshot(),
        });
    }
    fx.push(Effect::TabClosed {
        index: idx,
        old_len,
    });
    fx.push(Effect::ContextChanged { immediate: true });
}

fn json_event(
    pane: &mut PreviewPane,
    binding: &crate::EditorHostBinding,
    envelope: crate::WebviewEnvelope<JsonEvent>,
    _ctx: &UpdateCtx<'_>,
    _fx: &mut Vec<Effect>,
) {
    let event = envelope.payload;
    let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == binding.tab_id) else {
        return;
    };
    match event {
        JsonEvent::Ready { .. } => {
            // `ready` 只代表 host JS 就绪,不代表正文可见。
            tab.web_error = None;
        }
        JsonEvent::DocumentLoaded {
            revision,
            bytes: _,
            error,
        } => {
            // 非在途(迟到)的 ACK 不结束新加载。
            if !tab.load_state.is_active() {
                return;
            }
            if let Some(message) = error {
                tab.web_error = Some(message.clone());
                let _ = tab
                    .backend_state
                    .try_transition(BackendState::Failed(PreviewError::new(message, true)));
                tab.load_state.finish();
            } else {
                if revision >= tab.web_revision {
                    tab.web_revision = revision;
                }
                let _ = tab.backend_state.try_transition(BackendState::Ready);
                tab.load_state.finish();
            }
        }
        JsonEvent::DocumentChanged { revision, .. } => {
            if revision >= tab.web_revision {
                tab.web_revision = revision;
            }
        }
        JsonEvent::Failed {
            message,
            recoverable,
        } => {
            tab.web_error = Some(message.clone());
            let _ = tab
                .backend_state
                .try_transition(BackendState::Failed(PreviewError::new(
                    message,
                    recoverable,
                )));
            if tab.load_state.is_active() {
                tab.load_state.finish();
            }
        }
    }
}

fn tabular_event(
    pane: &mut PreviewPane,
    binding: &crate::EditorHostBinding,
    envelope: crate::WebviewEnvelope<TabularEvent>,
    ctx: &UpdateCtx<'_>,
    fx: &mut Vec<Effect>,
) {
    // 表格 host 只在产品带表格能力时运行;没有能力时整条丢弃。
    if !ctx.caps.tabular {
        return;
    }
    let event = envelope.payload;
    let mut push_initial = false;
    let mut restore_view: Option<(usize, u32, u32)> = None;
    let mut window_push: Option<(usize, u32, Vec<Vec<String>>, u64)> = None;
    let mut sheet_to_select: Option<usize> = None;
    {
        let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == binding.tab_id) else {
            return;
        };
        match event {
            TabularEvent::Ready => {
                tab.web_error = None;
                tab.tabular_host_ready = true;
                push_initial = true;
            }
            TabularEvent::WindowApplied { start_row: _ } => {
                // 首窗真正挂上才 finish,避免"空网格+行号1"的中间态露出。
                if tab.load_state.is_active() {
                    let _ = tab.backend_state.try_transition(BackendState::Ready);
                    tab.load_state.finish();
                }
                if let Some(view) = tab.tabular_view() {
                    let (sheet_index, start_row, start_col) = (
                        view.active_sheet,
                        view.scroll_row as u32,
                        view.scroll_col as u32,
                    );
                    if start_row != 0 || start_col != 0 {
                        restore_view = Some((sheet_index, start_row, start_col));
                    }
                }
            }
            TabularEvent::WindowRequest {
                sheet_index,
                start_row,
                end_row,
            } => {
                // 零 IO:对已在内存的 Sheet.rows 切片。sheet 已切走的迟到请求丢弃。
                let revision = tab.web_revision;
                if let Some(view) = tab.tabular_view()
                    && view.active_sheet == sheet_index
                    && let Some(sheet) = view.active_sheet()
                {
                    let start = (start_row as usize).min(sheet.rows.len());
                    // `.max(start)`:防御 end_row < start_row 时裸切片 panic。
                    let end = (end_row as usize).min(sheet.rows.len()).max(start);
                    let rows = sheet.rows[start..end].to_vec();
                    window_push = Some((sheet_index, start as u32, rows, revision));
                }
            }
            TabularEvent::SheetSelected { index } => {
                sheet_to_select = Some(index);
            }
            TabularEvent::Failed {
                message,
                recoverable,
            } => {
                tab.web_error = Some(message.clone());
                let _ = tab
                    .backend_state
                    .try_transition(BackendState::Failed(PreviewError::new(
                        message,
                        recoverable,
                    )));
                if tab.load_state.is_active() {
                    tab.load_state.finish();
                }
            }
        }
    }
    if push_initial {
        pane.try_push_initial_tabular_state(binding.tab_id);
    }
    if let Some((sheet_index, start_row, start_col)) = restore_view {
        pane.queue_tabular_command(
            binding.tab_id,
            crate::TabularCommand::RestoreViewState {
                sheet_index,
                start_row,
                start_col,
            },
        );
    }
    if let Some((sheet_index, start_row, rows, revision)) = window_push {
        pane.queue_tabular_command(
            binding.tab_id,
            crate::TabularCommand::SetWindow {
                sheet_index,
                start_row,
                rows,
                revision,
            },
        );
    }
    if let Some(index) = sheet_to_select {
        crate::update::tabular_action(pane, binding.tab_id, index, fx);
    }
}

fn flyfish_event(
    pane: &mut PreviewPane,
    binding: &crate::HostBinding,
    envelope: crate::WebviewEnvelope<FlyfishEvent>,
    ctx: &UpdateCtx<'_>,
) {
    let event = envelope.payload;
    // 搜索状态:只在本次 webview Find 会话正锁定该 tab 时回填。
    if let FlyfishEvent::SearchState { current, total } = event {
        let matches = pane
            .find_state()
            .is_some_and(|f| f.is_webview && f.tab_id == binding.tab_id);
        if matches {
            pane.set_find_webview_state(current, total);
        }
        return;
    }
    let _ = ctx;
    let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == binding.tab_id) else {
        return;
    };
    match event {
        FlyfishEvent::Ready => {
            // `ready` 只代表 host 脚本初始化完成,不代表正文已渲染。
            tab.web_error = None;
        }
        FlyfishEvent::DocumentLoaded { error, .. } => {
            if !tab.load_state.is_active() {
                return;
            }
            if let Some(message) = error {
                tab.runtime = PreviewRuntime::None;
                tab.web_error = Some(message.clone());
                let _ = tab
                    .backend_state
                    .try_transition(BackendState::Failed(PreviewError::new(message, true)));
                tab.load_state.finish();
            } else {
                tab.web_error = None;
                let _ = tab.backend_state.try_transition(BackendState::Ready);
                tab.load_state.finish();
            }
        }
        FlyfishEvent::Title { title } => {
            // 文件预览 tab 的标题恒为文件名,不采用 host 回传的页面默认标题。
            if let TabKind::File(path) = &tab.kind {
                if let Some(name) = path.file_name() {
                    tab.title = name.to_string_lossy().into_owned();
                }
            } else if !title.is_empty() {
                tab.title = title;
            }
        }
        FlyfishEvent::Failed {
            message,
            recoverable,
        } => {
            tab.runtime = PreviewRuntime::None;
            tab.web_error = Some(message.clone());
            let _ = tab
                .backend_state
                .try_transition(BackendState::Failed(PreviewError::new(
                    message,
                    recoverable,
                )));
            if tab.load_state.is_active() {
                tab.load_state.finish();
            }
        }
        FlyfishEvent::SearchState { .. } => {}
    }
}

fn plantuml_event(
    pane: &mut PreviewPane,
    binding: &crate::HostBinding,
    envelope: crate::WebviewEnvelope<PlantUmlEvent>,
) {
    let event_revision = envelope.revision;
    let open_source = pane.apply_plantuml_event(binding.tab_id, event_revision, envelope.payload);
    let Some(line) = open_source else {
        return;
    };
    // 用户点"查看源码":切到 Source 源码模式;带行号则排队跳行。
    let Some(idx) = pane.tabs().iter().position(|t| t.id == binding.tab_id) else {
        return;
    };
    let tab_id = binding.tab_id;
    if pane.enter_code_mode(idx).is_ok()
        && let Some(line) = line
    {
        pane.queue_editor_command(
            tab_id,
            crate::EditorCommand::RevealPosition { line, column: 1 },
        );
    }
}
