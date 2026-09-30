//! `App::update` 消息分发 + 全部 handler 方法。Phase 3 结构重组时从
//! `app/app.rs` 拆出,逻辑保持原样。

use crate::chrome::homespace::{self, load_home_recents};
use crate::chrome::rail;
use crate::chrome::tab_widget;
use crate::extensions::browser;
use crate::extensions::codehealth;
use crate::extensions::conversations;
use crate::extensions::database;
use crate::extensions::file_history;
use crate::extensions::files;
use crate::extensions::footbar;
use crate::extensions::git_log;
use crate::extensions::project;
use crate::extensions::project_create;
use crate::extensions::search;
use crate::extensions::settings;
use crate::extensions::ssh;
use crate::extensions::todo;
use crate::extensions::usage;
use crate::git_watch;
use crate::term::terminal;
use crate::workspace::{
    CONVERSATION_DETAIL_PAGE_SIZE, RestorePayload, ReviewSource, ReviewView, SshOut,
    TabAttachedArgs, TabBackend, Workspace, exited_marker, preview_tab_display_width,
    relative_time_text, review_should_refresh_on_turn, spawn_disk_usage_refresh,
    spawn_project_git_refresh, tab_display_width, tab_title,
};
use byteui::interaction::icons;
use dozer_core::protocol::{
    AgentKind, AgentState, BookmarkInfo, ProjectInfo, SummaryJobStatus, SummaryTrigger,
};
use iced_widget::core::{Border, Element, Length, Padding};
use iced_widget::{button, column, container, text};
use std::path::PathBuf;

use super::*;

/// review-trace webview 报回 `document_loaded` 后落地:"当前 nonce 的内容
/// 已可显示"。抽成纯函数便于 headless 单测(构造完整 `App` 成本过高)。
///
/// `None`/不等于 `nonce` 都算"未 loaded";事件本身不携带"我是为哪个 nonce
/// 报的"(协议从简,见 spec"风险与边界"),因此只能把 `loaded_nonce` 设成
/// **事件到达那一刻的当前 nonce**——快速连切两次时,第一次的过期事件会提前
/// 让新 nonce 追上(已知限制,不在本任务修复范围)。
pub(crate) fn mark_review_loaded(review: &mut Option<crate::workspace::ReviewView>) {
    if let Some(rv) = review {
        rv.loaded_nonce = Some(rv.nonce);
    }
}

/// 会话列表刷新带回新总结后，同步当前详情页及其 WebView 快照。
///
/// 详情正文由 `review_snapshot` 提供，单改 `ReviewView.summary_*` 不会让已经
/// 加载的 WebView 重新取数；这里同时推进 nonce 并返回新的序列化快照。
fn refresh_open_conversation_summary(
    review: &mut Option<ReviewView>,
    review_nonce: &mut u64,
    row: &crate::conversation::SessionRow,
) -> Option<String> {
    let rv = review.as_mut()?;
    if !matches!(&rv.source, ReviewSource::Conversation(cid) if cid == &row.conversation_id) {
        return None;
    }
    if rv.summary_title.as_deref() == Some(row.display_title.as_str())
        && rv.summary_text == row.summary
    {
        return None;
    }

    rv.summary_title = Some(row.display_title.clone());
    rv.summary_text = row.summary.clone();
    *review_nonce = review_nonce.wrapping_add(1);
    rv.nonce = *review_nonce;
    let snapshot = ReviewSnapshot {
        entries: &rv.entries,
        agent_label: rv.agent.label(),
        summary_title: rv.summary_title.clone(),
        summary_text: rv.summary_text.clone(),
        summary_time: rv.summary_time.clone(),
    };
    serde_json::to_string(&snapshot).ok()
}

impl App {
    pub fn update(&mut self, message: Message) {
        match message {
            Message::GitLogDiffWebviewEvent(_binding, event) => {
                if matches!(event.payload, crate::preview::EditorEvent::Ready { .. }) {
                    // 真正的推送发生在 `App::take_git_log_diff_script`(每帧轮询,
                    // 见下),这里只翻状态:webview 刚确认 `__dozer.dispatch`
                    // 已注册,`diff_sent_for` 清空强制下一帧重发一次当前内容
                    // (覆盖"webview 被销毁重建,新实例第一次 ready"的场景)。
                    self.git_log.set_diff_webview_ready(true);
                    self.git_log.clear_diff_sent_for();
                }
                // 其余事件(selection_changed/viewport_changed/...):diff 面板
                // 恒只读、不需要 Agent 跳转/保存,忽略即可。
            }
            Message::FileHistoryDiffWebviewEvent(_binding, event) => {
                if matches!(event.payload, crate::preview::EditorEvent::Ready { .. })
                    && let Some(s) = self.file_history.as_mut()
                {
                    s.set_diff_webview_ready(true);
                }
                // 其余事件忽略,同 GitLogDiffWebviewEvent 的处理。
            }
            Message::EditHistory(msg) => self.edit_history_message(msg),
            Message::EditHistoryDiffWebviewEvent(_binding, event) => {
                if matches!(event.payload, crate::preview::EditorEvent::Ready { .. })
                    && let Some(s) = self.edit_history.as_mut()
                {
                    s.set_diff_webview_ready(true);
                }
            }
            Message::UsageContentWebviewEvent(event) => {
                if matches!(event, crate::extensions::usage::UsageWebviewEvent::Ready) {
                    // `set_ready(true)` 内部已经清空 `last_sent`,强制下一帧重发
                    // 一次当前内容(覆盖"webview 被销毁重建,新实例第一次 ready"的
                    // 场景),不需要在这里再显式清一次。
                    self.usage_webview.set_ready(true);
                }
            }
            Message::EditorWebviewEvent(binding, event) => {
                self.with_project(binding.project_id, move |ws, io| {
                    let pane = if binding.panel == PanelKind::Project {
                        &mut ws.project_preview
                    } else {
                        &mut ws.preview
                    };
                    // `Ready` 时若该 tab 有"打开后跳转到某行"的诉求(代码健康度
                    // 面板),取出来在 tab 借用结束后排队一条 reveal 命令。
                    let mut pending_reveal: Option<(usize, u32)> = None;
                    // 是否需要把上下文(路径/光标/选区/可见行/revision)推给
                    // dozerd:选区/可见范围/就绪变化都算。
                    let mut context_changed = false;
                    // 窗口化 viewer:待建立行索引 / 待推送相邻窗口。
                    // T11:元组末项为该 tab 的后台任务取消信号(索引构建按 chunk 检查)。
                    let mut build_index: Option<(
                        usize,
                        PathBuf,
                        u64,
                        std::sync::Arc<std::sync::atomic::AtomicBool>,
                    )> = None;
                    let mut window_request: Option<(usize, u32)> = None;
                    // recovery 恢复:ready 后回推正文 + 重新标脏。
                    let mut pending_restore_cmd: Option<(usize, String, u64)> = None;
                    // 视图状态恢复:ready 后应用一次 cursor/selection/top-line。
                    let mut pending_view_cmd: Option<(usize, crate::preview::EditorCommand)> = None;
                    // 窗口化 ⌘F:打开整文件搜索条。
                    let mut find_request: Option<usize> = None;
                    // T4:窗口化收到 `window_applied`(正文已挂上)→ 结束加载。
                    let mut window_applied: Option<usize> = None;
                    // 关闭前保存:host 回 `save_requested` 落盘成功后,若这条
                    // tab 正等"保存后关闭",记下 id 在 tab 借用结束后关闭。
                    let mut close_after_save: Option<usize> = None;
                    // 先(在可变借出 tab 之前)探测本 tab 是否在等"保存后关闭"。
                    let want_close = pane.has_pending_close(binding.tab_id);
                    {
                        let Some(tab) = pane
                            .tabs_mut()
                            .iter_mut()
                            .find(|tab| tab.id == binding.tab_id)
                        else {
                            return;
                        };
                        let crate::preview::TabKind::File(path) = &tab.kind else {
                            return;
                        };
                        if path != &binding.path || event.revision < tab.web_revision {
                            return;
                        }
                        use crate::preview::EditorEvent;
                        match event.payload {
                            EditorEvent::Ready { .. } => {
                                tab.web_revision = event.revision;
                                tab.web_error = None;
                                // 加载成功:清零该文件连续失败计数。
                                crate::preview::reset_failure_to(
                                    &crate::preview::failures_path(),
                                    path,
                                );
                                // T10 bullet 4:只有仍在该次加载(在途)时才接受
                                // host `ready`。非在途(已 finish/被更晚加载替换)
                                // 的迟到 ready 不改阶段、不建索引,避免旧 host 的
                                // 结果把新 loading 或就绪态带偏。
                                 if tab.load_state.is_active() {
                                     if tab.uses_windowed_editor() {
                                         // T4:窗口化 host 的 `ready` 只代表 host JS
                                         // 初始化完成(空 doc),正文要等首个 SetWindow。
                                         // 保持 `Loading`,推进到 `Indexing`(索引建好
                                         // 后推首窗,收 `window_applied` ACK 才 finish)。
                                         tab.load_state
                                             .advance(crate::preview::PreviewLoadStage::Indexing);
                                         // T11:arm `Indexing` 看门狗(建索引 + 首窗)。
                                         io.arm_load_timeout(
                                             binding.project_id,
                                             binding.panel,
                                             tab.id,
                                             tab.load_state.generation,
                                             crate::preview::PreviewLoadStage::Indexing,
                                         );
                                         if tab.window_index().is_none() {
                                             build_index = Some((
                                                 tab.id,
                                                 path.clone(),
                                                 tab.web_revision,
                                                 tab.task_cancel_token(),
                                             ));
                                         }
                                         // 窗口化不需要 recovery/视图恢复(只读、正文由
                                         // SetWindow 决定),也不走下面的 pending_reveal。
                                     } else {
                                         // T5:非窗口化 host 的 `ready` 仅代表 host JS
                                         // 初始化完成——正文由 host 自行 fetch,待其回
                                         // `document_loaded` 才 finish。此处保持 Loading,
                                         // 推进到 `Reading`(host 会显示 loading,正文
                                         // 未挂上前不 finish)。
                                         tab.load_state
                                             .advance(crate::preview::PreviewLoadStage::Reading);
                                         // T11:arm `Reading` 看门狗(等 document_loaded)。
                                         io.arm_load_timeout(
                                             binding.project_id,
                                             binding.panel,
                                             tab.id,
                                             tab.load_state.generation,
                                             crate::preview::PreviewLoadStage::Reading,
                                         );
                                     }
                                 }
                                context_changed = true;
                            }
                            EditorEvent::DocumentLoaded {
                                revision: doc_rev,
                                bytes: _,
                                error,
                            } => {
                                // T5:非窗口化正文落地(或读取失败)的终态判定。
                                // 窗口化 tab 不走本事件(其正文由 SetWindow 决定)。
                                tab.web_revision = event.revision;
                                // T10 bullet 4:非在途(已 finish/被更晚加载替换)的
                                // 迟到 `document_loaded` 不得结束新 loading 或把就绪
                                // tab 重新置态。此处仍更新 revision(下方已按 revision
                                // 去回归),但不改阶段/终态。
                                let ack_active = tab.load_state.is_active();
                                if ack_active {
                                    if let Some(message) = error {
                                    tab.web_error = Some(message.clone());
                                    let _ = tab.backend_state.try_transition(
                                        crate::preview::BackendState::Failed(
                                            crate::preview::PreviewError::new(message, true),
                                        ),
                                    );
                                    // 结束本次 loading(回到 Idle 并作废在途结果)。
                                    tab.load_state.finish();
                                } else if !tab.uses_windowed_editor() && doc_rev == event.revision {
                                    tab.web_error = None;
                                    // 加载成功:清零该文件连续失败计数。
                                    crate::preview::reset_failure_to(
                                        &crate::preview::failures_path(),
                                        path,
                                    );
                                    let _ = tab
                                        .backend_state
                                        .try_transition(crate::preview::BackendState::Ready);
                                    // 正文可见 → 结束本次 loading。
                                    tab.load_state.finish();
                                    if let Some(line) = tab.pending_jump_line.take() {
                                        pending_reveal = Some((tab.id, line as u32));
                                    }
                                    // T11 bullet 3/4:ready 观测(不记文件内容)。
                                    if let Some(observe) = tab.load_observe.take() {
                                        let ready_ms = observe.elapsed_ms();
                                        let first_frame_ms = observe.first_frame_offset_ms();
                                        tracing::info!(
                                            panel = ?binding.panel,
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
                                        pending_restore_cmd =
                                            Some((tab.id, text, tab.web_revision));
                                    }
                                    // 视图状态恢复(T11):一次 RestoreViewState 应用
                                    // folds → selection/cursor → scroll(顺序由 host
                                    // 保证);空快照不发命令。
                                    if let Some(state) = tab.pending_view.take()
                                        && !state.is_empty()
                                    {
                                        pending_view_cmd = Some((
                                            tab.id,
                                            crate::preview::EditorCommand::RestoreViewState {
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
                                tab.web_revision = event.revision;
                                tab.web_selection = Some(crate::preview::TextRange {
                                    start: anchor,
                                    end: head,
                                });
                                tab.web_selected_text = selected_text.map(|text| {
                                    text.chars()
                                        .take(dozer_core::protocol::PREVIEW_SELECTED_TEXT_MAX_CHARS)
                                        .collect()
                                });
                                context_changed = true;
                            }
                            EditorEvent::ViewportChanged { from_line, to_line } => {
                                tab.web_revision = event.revision;
                                tab.web_viewport = Some((from_line, to_line));
                                context_changed = true;
                            }
                            EditorEvent::DocumentChanged { revision, .. } => {
                                if revision == event.revision && revision >= tab.web_revision {
                                    tab.web_revision = revision;
                                    tab.dirty = true;
                                }
                            }
                            EditorEvent::SaveRequested { revision, text } => {
                                // 关闭前保存的 tab:不管 revision 是否对齐,落盘
                                // 尝试完都必须完成关闭,否则 tab 卡在等待里永远
                                // 关不掉。`want_close` 在借出 tab 之前已探明。
                                //
                                // 先克隆路径,结束对 `tab.kind` 的不可变借用,
                                // 才能调 `tab.save_gate()`(需要 &mut)。
                                let save_path = path.clone();
                                // T6/T10:只读 / 有损编码 / 未处理的磁盘冲突一律
                                // 拒绝写盘;冲突需用户先选择保留或重载。
                                match tab.save_gate() {
                                    crate::preview::SaveGate::ReadOnly => {
                                        tab.web_error = Some(
                                            "该文件为只读/非 UTF-8 文本,已禁用保存以免破坏原文件"
                                                .into(),
                                        );
                                    }
                                    crate::preview::SaveGate::Conflict => {
                                        tab.web_error = Some(
                                            "磁盘文件已被外部修改,请先选择「保留我的修改」或「重载磁盘」。"
                                                .into(),
                                        );
                                    }
                                    crate::preview::SaveGate::Allow => {
                                        if revision == event.revision
                                            && revision == tab.web_revision
                                        {
                                            match crate::preview::save_text_atomic(
                                                &save_path,
                                                &text,
                                            ) {
                                                Ok(()) => {
                                                    tab.dirty = false;
                                                    tab.recovery_written = false;
                                                    tab.conflict_baseline = None;
                                                    // 正常保存:清掉该文件的 recovery snapshot。
                                                    let project_id = binding.project_id;
                                                    let restore_path = save_path.clone();
                                                    io.handle.spawn(async move {
                                                        let _ = tokio::task::spawn_blocking(
                                                            move || {
                                                                crate::preview::clear_snapshot(
                                                                    &crate::preview::recovery_dir(),
                                                                    project_id,
                                                                    crate::preview::path_key(
                                                                        &restore_path,
                                                                    ),
                                                                );
                                                            },
                                                        )
                                                        .await;
                                                    });
                                                }
                                                Err(error) => {
                                                    tab.web_error =
                                                        Some(format!("保存失败: {error}"))
                                                }
                                            }
                                        } else if !want_close {
                                            // 非关闭场景下的过期保存:与旧行为一致,丢弃。
                                            return;
                                        }
                                    }
                                }
                                if want_close {
                                    close_after_save = Some(tab.id);
                                }
                            }
                            EditorEvent::Snapshot { revision, text } => {
                                // 防抖脏快照:原子写 recovery。仅在 revision 不回退
                                // 时接受,避免过期快照覆盖新内容。
                                if revision >= tab.web_revision {
                                    tab.web_revision = revision;
                                    let project_id = binding.project_id;
                                    let panel = binding.panel;
                                    let tab_id = tab.id;
                                    let snap_path = path.clone();
                                    let recovery_cursor = tab.web_selection.map(|range| range.end);
                                    let recovery_selection = tab.web_selection;
                                    let recovery_top_line = tab.web_viewport.map(|(from, _)| from);
                                    let proxy = io.proxy.clone();
                                    io.handle.spawn(async move {
                                        let ok = tokio::task::spawn_blocking(move || {
                                            let Ok(profile) =
                                                crate::preview::profile_file(&snap_path)
                                            else {
                                                return false;
                                            };
                                            let manifest =
                                                crate::preview::RecoveryManifest::from_profile(
                                                    snap_path.clone(),
                                                    &profile,
                                                    revision,
                                                    recovery_cursor,
                                                    recovery_selection,
                                                    recovery_top_line,
                                                );
                                            crate::preview::write_snapshot(
                                                &crate::preview::recovery_dir(),
                                                project_id,
                                                crate::preview::path_key(&snap_path),
                                                &manifest,
                                                &text,
                                            )
                                            .is_ok()
                                        })
                                        .await
                                        .unwrap_or(false);
                                        if ok {
                                            let _ =
                                                proxy.send_event(Message::PreviewRecoveryWritten(
                                                    project_id, panel, tab_id,
                                                ));
                                        }
                                    });
                                }
                            }
                            EditorEvent::ViewState {
                                cursor,
                                selection,
                                top_line,
                                folds,
                            } => {
                                tab.web_revision = event.revision;
                                tab.web_selection = selection;
                                // T11:完整镜像,供持久化与淘汰前序列化。
                                tab.web_view_state = Some(crate::preview::ViewStateRestore {
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
                                // 窗口化 viewer 请求相邻窗口:记下目标锚点行,
                                // tab 借用结束后在 pane 上排队装窗口。
                                window_request = Some((tab.id, anchor_line));
                            }
                            EditorEvent::WindowApplied { start_line: _ } => {
                                // T4:窗口化首窗(或相邻窗口)正文已挂上 → 结束
                                // 加载,host 变可见。generation 由 tab 当前的
                                // load_state 决定,借用结束后再调 `finish_load`。
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
                                // 连续失败计数:达到阈值后提示降级(纯文本/
                                // 外部打开),而不是无限自动重试。
                                let count = crate::preview::bump_failure_to(
                                    &crate::preview::failures_path(),
                                    path,
                                );
                                let detail = if count >= crate::preview::FAILURE_THRESHOLD {
                                    format!(
                                        "{message}(已连续失败 {count} 次,请改用纯文本/外部打开)"
                                    )
                                } else {
                                    message.clone()
                                };
                                tab.web_error = Some(detail.clone());
                                let _ = tab.backend_state.try_transition(
                                    crate::preview::BackendState::Failed(
                                        crate::preview::PreviewError::new(detail, recoverable),
                                    ),
                                );
                                // T5:host 明确失败 → 结束本次 loading。T10:非在途
                                // (迟到失败)不改世代,避免误作废新加载。
                                if tab.load_state.is_active() {
                                    tab.load_state.finish();
                                }
                            }
                            EditorEvent::FocusChanged { .. } => {}
                            // Phase 2:预览内选区右键。闭包内拿不到 `self`
                            // (几何/终端可见性),把原始 webview 坐标与绑定
                            // 信息回投一条顶层消息,在 `with_project` 外弹菜单。
                            EditorEvent::ContextMenuRequested {
                                x,
                                y,
                                range,
                                selected_text,
                            } => {
                                let _ = io.proxy.send_event(Message::PreviewSelectionMenuOpen {
                                    project_id: binding.project_id,
                                    panel: binding.panel,
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
                            crate::preview::EditorCommand::RevealPosition { line, column: 1 },
                        );
                    }
                    if let Some((tab_id, path, revision, cancel)) = build_index {
                        let project_id = binding.project_id;
                        let panel = binding.panel;
                        let proxy = io.proxy.clone();
                        io.handle.spawn(async move {
                            let result = tokio::task::spawn_blocking(move || {
                                // T11:按 chunk 检查取消信号(置位即提前返回 `Ok(None)`)。
                                let cancelled = {
                                    let cancel = cancel.clone();
                                    move || cancel.load(std::sync::atomic::Ordering::Relaxed)
                                };
                                crate::preview::LineIndex::build_cancellable(
                                    &path,
                                    1000,
                                    revision,
                                    cancelled,
                                )
                                .and_then(|index| {
                                    index.ok_or_else(|| std::io::Error::other("index cancelled"))
                                })
                                .map(std::sync::Arc::new)
                                .map_err(|e| e.to_string())
                            })
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()));
                            let _ = proxy.send_event(Message::PreviewWindowIndex(
                                project_id, panel, tab_id, result,
                            ));
                        });
                    }
                    if let Some((tab_id, anchor_line)) = window_request {
                        pane.queue_windowed_view(tab_id, anchor_line);
                    }
                    // T4:窗口正文已挂上 → 结束加载(host 变可见)。用 tab 当前
                    // generation 结束精确的这一次加载。
                    if let Some(tab_id) = window_applied {
                        // T10 bullet 4:仅在该 tab 仍有加载在途时接受首窗 ACK,避免
                        // 迟到 `window_applied` 结束已被重试/替换的新加载。
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
                        pane.queue_editor_command(
                            tab_id,
                            crate::preview::EditorCommand::SetDocument {
                                text,
                                revision,
                                language: crate::preview::extension_to_syntax(&binding.path),
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
                    // "保存后关闭":落盘完成(或过期丢弃)后真正移除 tab,
                    // 收尾与同步关闭路径一致(重置 first / 存状态 / flush 上下文)。
                    // 已知缺口:这条路径没有调用 `App::dehover_after_tab_close`
                    // (在 `with_project` 闭包内拿不到 `self.hover_anims`)——
                    // 若被延迟关闭的这个 tab 恰好正被悬停,会重现该方法文档
                    // 描述的孤儿 hover 值问题,只是触发条件更窄(得先撞上脏
                    // CodeMirror tab 关闭走异步保存这条分支)。
                    if let Some(tab_id) = close_after_save
                        && pane.take_pending_close(tab_id)
                    {
                        pane.close_by_id(tab_id);
                        if binding.panel == PanelKind::Project {
                            ws.project_preview_tab_first = 0;
                        } else {
                            ws.preview_tab_first = 0;
                        }
                        ws.spawn_preview_state_save(io);
                        ws.flush_preview_context_push(io);
                    }
                    if context_changed {
                        // 第二段链路(dozer-app → dozerd)自带 250ms 防抖;这里
                        // 只管触发,不假设复用 editor→app 那段的节流。
                        ws.spawn_preview_context_push(io);
                    }
                });
            }
            Message::JsonEditorEvent(binding, event) => {
                self.with_project(binding.project_id, move |ws, _io| {
                    let pane = if binding.panel == PanelKind::Project {
                        &mut ws.project_preview
                    } else {
                        &mut ws.preview
                    };
                    if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == binding.tab_id) {
                        use crate::preview::JsonEvent;
                        match event.payload {
                            JsonEvent::Ready { .. } => {
                                tab.web_error = None;
                                // T6:`ready` 只代表 host JS 就绪,不代表正文可见;
                                // 加载在这个阶段保持 Loading,等 `document_loaded`。
                            }
                            JsonEvent::DocumentLoaded {
                                revision,
                                bytes: _,
                                error,
                            } => {
                                // T10 bullet 4:非在途(迟到)的 ACK 不结束新加载。
                                if !tab.load_state.is_active() {
                                    return;
                                }
                                if let Some(message) = error {
                                    tab.web_error = Some(message.clone());
                                    let _ = tab.backend_state.try_transition(
                                        crate::preview::BackendState::Failed(
                                            crate::preview::PreviewError::new(message, true),
                                        ),
                                    );
                                    tab.load_state.finish();
                                } else {
                                    if revision >= tab.web_revision {
                                        tab.web_revision = revision;
                                    }
                                    let _ = tab
                                        .backend_state
                                        .try_transition(crate::preview::BackendState::Ready);
                                    // T6:JSON host 正文真正挂上 → 结束加载。
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
                                let _ = tab.backend_state.try_transition(
                                    crate::preview::BackendState::Failed(
                                        crate::preview::PreviewError::new(message, recoverable),
                                    ),
                                );
                                // T5/T6:host 失败 → 结束加载。T10:非在途不改世代。
                                if tab.load_state.is_active() {
                                    tab.load_state.finish();
                                }
                            }
                        }
                    }
                });
            }
            Message::TabularHostEvent(binding, event) => {
                self.with_project(binding.project_id, move |ws, io| {
                    let panel = binding.panel;
                    let mut push_initial = false;
                    let mut restore_view: Option<(usize, u32, u32)> = None;
                    let mut window_push: Option<(usize, u32, Vec<Vec<String>>, u64)> = None;
                    let mut sheet_to_select: Option<usize> = None;
                    {
                        let pane = if panel == PanelKind::Project {
                            &mut ws.project_preview
                        } else {
                            &mut ws.preview
                        };
                        let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == binding.tab_id)
                        else {
                            return;
                        };
                        use crate::preview::TabularEvent;
                        match event.payload {
                            TabularEvent::Ready => {
                                tab.web_error = None;
                                tab.tabular_host_ready = true;
                                push_initial = true;
                            }
                            TabularEvent::WindowApplied { start_row: _ } => {
                                // 首窗真正挂上才 finish,避免"空网格+行号1"
                                // 的中间态露出。非在途(迟到 ACK)不改终态。
                                if tab.load_state.is_active() {
                                    let _ = tab
                                        .backend_state
                                        .try_transition(crate::preview::BackendState::Ready);
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
                                // 零 IO:对已在内存的 Sheet.rows 切片。
                                // sheet_index 与当前活动 sheet 不一致(用户
                                // 已经切走)的迟到请求直接丢弃,不回窗口。
                                let revision = tab.web_revision;
                                if let Some(view) = tab.tabular_view()
                                    && view.active_sheet == sheet_index
                                    && let Some(sheet) = view.active_sheet()
                                {
                                    let start = (start_row as usize).min(sheet.rows.len());
                                    // `.max(start)`:防御性防越界——不可信的
                                    // IPC 输入若带 end_row < start_row,裸切片
                                    // 会直接 panic(见 webview_protocol.rs
                                    // "解析失败不 panic" 的既定原则)。
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
                                let _ = tab.backend_state.try_transition(
                                    crate::preview::BackendState::Failed(
                                        crate::preview::PreviewError::new(message, recoverable),
                                    ),
                                );
                                if tab.load_state.is_active() {
                                    tab.load_state.finish();
                                }
                            }
                        }
                    }
                    let pane = if panel == PanelKind::Project {
                        &mut ws.project_preview
                    } else {
                        &mut ws.preview
                    };
                    if push_initial {
                        pane.try_push_initial_tabular_state(binding.tab_id);
                    }
                    if let Some((sheet_index, start_row, start_col)) = restore_view {
                        pane.queue_tabular_command(
                            binding.tab_id,
                            crate::preview::TabularCommand::RestoreViewState {
                                sheet_index,
                                start_row,
                                start_col,
                            },
                        );
                    }
                    if let Some((sheet_index, start_row, rows, revision)) = window_push {
                        pane.queue_tabular_command(
                            binding.tab_id,
                            crate::preview::TabularCommand::SetWindow {
                                sheet_index,
                                start_row,
                                rows,
                                revision,
                            },
                        );
                    }
                    if let Some(index) = sheet_to_select {
                        ws.preview_pane_tabular_action(panel, binding.tab_id, index, io);
                    }
                });
            }
            Message::ReviewTraceWebviewEvent(event) => {
                let crate::preview::ReviewTraceEvent::DocumentLoaded = event;
                self.with_focused_project(|ws, _io| {
                    mark_review_loaded(&mut ws.review);
                });
            }
            Message::FlyfishEvent(binding, event) => {
                self.with_project(binding.project_id, move |ws, _io| {
                    use crate::preview::FlyfishEvent;
                    // 搜索状态:只在本次 webview Find 会话正锁定该 tab 时回填。
                    if let FlyfishEvent::SearchState { current, total } = event.payload {
                        let pane = if binding.panel == PanelKind::Project {
                            &mut ws.project_preview
                        } else {
                            &mut ws.preview
                        };
                        let matches = pane
                            .find_state()
                            .is_some_and(|f| f.is_webview && f.tab_id == binding.tab_id);
                        if matches {
                            ws.preview_find_set_webview_state(binding.panel, current, total);
                        }
                        return;
                    }
                    let pane = if binding.panel == PanelKind::Project {
                        &mut ws.project_preview
                    } else {
                        &mut ws.preview
                    };
                    if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == binding.tab_id) {
                        match event.payload {
                            FlyfishEvent::Ready => {
                                // T8:`ready` 只代表 host 脚本初始化完成,不代表
                                // 正文已渲染。清错误、保持 Loading,等
                                // `document_loaded` 才可见 + finish(否则会露出
                                // 空白原生子视图盖住 iced loading)。
                                tab.web_error = None;
                            }
                            FlyfishEvent::DocumentLoaded { error, .. } => {
                                // T10 bullet 4:非在途(迟到)的 ACK 不结束新加载。
                                if !tab.load_state.is_active() {
                                    return;
                                }
                                if let Some(message) = error {
                                    // host 报告正文加载失败:回落统一 Failed 终态
                                    // (`hosts_webview` 随后为 false → 原生子视图
                                    // 从 desired pool 移除,fallback 页不被盖住)。
                                    tab.runtime = crate::preview::PreviewRuntime::None;
                                    tab.web_error = Some(message.clone());
                                    let _ = tab.backend_state.try_transition(
                                        crate::preview::BackendState::Failed(
                                            crate::preview::PreviewError::new(message, true),
                                        ),
                                    );
                                    tab.load_state.finish();
                                } else {
                                    // T8:正文首帧就绪 → 置 Ready 并 finish 加载
                                    // (初始加载 / Source→Rendered 切换均经此)。
                                    tab.web_error = None;
                                    let _ = tab
                                        .backend_state
                                        .try_transition(crate::preview::BackendState::Ready);
                                    tab.load_state.finish();
                                }
                            }
                            FlyfishEvent::Title { title } => {
                                // 文件预览 tab 的标题恒为文件名,不采用 flyfish
                                // host 回传的页面默认标题(如 "Dozer Preview")——
                                // 否则 flyfish 加载完会把 tab 标题覆盖成它自己的
                                // 页面标题。从 tab 路径派生文件名覆盖;非文件 tab
                                // (本 app 中 flyfish host 不存在此类)才回退用回传标题。
                                if let crate::preview::TabKind::File(path) = &tab.kind {
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
                                // T9:渲染失败回落统一 Failed 终态(T1 fallback 页)。
                                tab.runtime = crate::preview::PreviewRuntime::None;
                                tab.web_error = Some(message.clone());
                                let _ = tab.backend_state.try_transition(
                                    crate::preview::BackendState::Failed(
                                        crate::preview::PreviewError::new(message, recoverable),
                                    ),
                                );
                                // T10:非在途不改世代。
                                if tab.load_state.is_active() {
                                    tab.load_state.finish();
                                }
                            }
                            FlyfishEvent::SearchState { .. } => {}
                        }
                    }
                });
            }
            Message::ImageAnnotateEvent(binding, event) => {
                self.with_project(binding.project_id, move |ws, _io| {
                    let pane = if binding.panel == PanelKind::Project {
                        &mut ws.project_preview
                    } else {
                        &mut ws.preview
                    };
                    pane.apply_image_annotate_event(binding.tab_id, event.payload);
                });
            }
            Message::PreviewCommandsFetched(project_id, commands) => {
                self.with_project(project_id, move |ws, io| {
                    for cmd in commands {
                        let outcome = ws.apply_preview_command(&cmd, io);
                        let client = io.client.clone();
                        io.handle.spawn(async move {
                            if let Err(e) = client.report_preview_command_outcome(outcome).await {
                                tracing::warn!(%e, "回报预览命令结果失败");
                            }
                        });
                    }
                });
            }
            Message::PreviewWindowIndex(project_id, panel, tab_id, result) => {
                self.with_project(project_id, move |ws, io| {
                    // T11 bullet 3:结构化日志(读取字节 + 行数,不记正文)。
                    match &result {
                        Ok(index) => tracing::debug!(
                            panel = ?panel,
                            tab_id,
                            bytes = index.file_len(),
                            total_lines = index.total_lines(),
                            "大文件行索引建立完成"
                        ),
                        Err(error) => {
                            tracing::warn!(panel = ?panel, tab_id, %error, "大文件行索引建立失败")
                        }
                    }
                    match result {
                        Ok(index) => {
                            let pane = if panel == PanelKind::Project {
                                &mut ws.project_preview
                            } else {
                                &mut ws.preview
                            };
                            // 过期索引(文件 revision 已变)必须丢弃,不得套到
                            // 新内容上——见 `PreviewPane::apply_window_index`。
                            if pane.apply_window_index(tab_id, index) {
                                let jump = pane
                                    .tabs_mut()
                                    .iter_mut()
                                    .find(|t| t.id == tab_id)
                                    .and_then(|t| t.pending_jump_line.take())
                                    .map(|l| l as u32);
                                // 索引就绪:推初始窗口(有跳转诉求就以目标行为中心,
                                // 窗口就位后再 reveal)。
                                let center = jump.unwrap_or(1);
                                // T4:进入 LoadingWindow,等 host 回 `window_applied`
                                // 才 finish(此前 host 保持 hidden/loading)。
                                let generation = pane.load_generation(tab_id);
                                pane.advance_load(
                                    tab_id,
                                    generation,
                                    crate::preview::PreviewLoadStage::LoadingWindow,
                                );
                                // T11:arm `LoadingWindow` 看门狗(等 window_applied)。
                                io.arm_load_timeout(
                                    project_id,
                                    panel,
                                    tab_id,
                                    generation,
                                    crate::preview::PreviewLoadStage::LoadingWindow,
                                );
                                pane.queue_windowed_view(tab_id, center);
                                if let Some(line) = jump {
                                    pane.queue_editor_command(
                                        tab_id,
                                        crate::preview::EditorCommand::RevealPosition {
                                            line,
                                            column: 1,
                                        },
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            let pane = if panel == PanelKind::Project {
                                &mut ws.project_preview
                            } else {
                                &mut ws.preview
                            };
                            if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == tab_id) {
                                tab.web_error = Some(format!("建立行索引失败: {error}"));
                            }
                        }
                    }
                });
            }
            Message::PreviewProfiled(project_id, panel, tab_id, generation, result) => {
                self.with_project(project_id, move |ws, io| {
                    let pane = if panel == PanelKind::Project {
                        &mut ws.project_preview
                    } else {
                        &mut ws.preview
                    };
                    match result {
                        Ok(profile) => {
                            if !pane.apply_profile(tab_id, generation, &profile) {
                                // generation 已过期(tab 关闭/重开/被替换):丢弃。
                                return;
                            }
                            // 画像落定:表格继续走它自己的后台解析(TabularLoaded
                            // 结束);窗口化继续走索引 + 首窗(T4,PreviewWindowIndex
                            // 期间保持 Loading);其余 host(CodeMirror/JSON/Flyfish)
                            // 一律推进到 `CreatingHost`,等 host 自己的
                            // `document_loaded` / 渲染完成信号才 finish(T5/T6/T8)。
                            //
                            // 例外:纯 iced 承载、无 host 的场景(Unsupported/External
                            // 走 fallback 页)没有 host 可等,直接 finish。
                            let (is_tabular, hosts_host) = pane
                                .tabs()
                                .iter()
                                .find(|t| t.id == tab_id)
                                .map(|t| (t.tabular_state().is_some(), t.hosts_any_webview()))
                                .unwrap_or((false, false));
                            if is_tabular {
                                // 表格:进入 `Parsing`,等后台解析出的首个可显示
                                // sheet(TabularLoaded)且 ag-grid host 回
                                // `ready`+`window_applied` 才 finish(T7 bullet 2)。
                                // CSV/TSV 原文模式另有 editor host,由 `load_preview_tab`
                                // 的恢复路径处理;正常打开路径这里不再 finish。
                                //
                                // 这段实际等的是"解析 + host 握手"两件事,任一个卡住
                                // (如 host 从未报 `window_applied`)先前都没有看门狗,
                                // 会永久停在"正在解析文件…"(2026-09-28 复现:CSV 文件
                                // 打开后卡死,数据其实已解析完成)。补上与 Reserving/
                                // CreatingHost/LoadingWindow 同款的超时兜底。
                                pane.advance_load(
                                    tab_id,
                                    generation,
                                    crate::preview::PreviewLoadStage::Parsing,
                                );
                                io.arm_load_timeout(
                                    project_id,
                                    panel,
                                    tab_id,
                                    generation,
                                    crate::preview::PreviewLoadStage::Parsing,
                                );
                            } else if !hosts_host {
                                // 无 host 的 fallback 页:直接就绪。
                                pane.finish_load(tab_id, generation);
                            } else {
                                // 窗口化与非窗口化 host 都保持 Loading。T10:先进入
                                // `Reserving` 等待资源预算;reserve 获批由
                                // `apply_preview_pool_evictions` 的 `granted` 台账推进到
                                // `CreatingHost`,之后 host 信号才 finish
                                // (窗口化:ready→Indexing→首窗→window_applied;
                                // 非窗口化:ready→Reading→document_loaded)。
                                //
                                // 无编辑器 host 的渲染类(Rendered/Flyfish/HTML)不参与
                                // reserve,直接就绪到 `CreatingHost`。
                                //
                                // T8 bullet 5:Rendered(Flyfish/隔离 HTML)host 可能
                                // 因外链/相对资源/网络卡住,arm 一个 host-ready 超时,
                                // 超时后进入可重试 Failed,不永久转圈。其余 host 的
                                // 分阶段超时归 T11。
                                let (rendered_host, needs_reserve) = pane
                                    .tabs()
                                    .iter()
                                    .find(|t| t.id == tab_id)
                                    .map(|t| {
                                        (
                                            t.hosts_webview(),
                                            t.uses_editor_host() || t.uses_json_editor(),
                                        )
                                    })
                                    .unwrap_or((false, false));
                                if rendered_host {
                                    io.arm_load_timeout(
                                        project_id,
                                        panel,
                                        tab_id,
                                        generation,
                                        crate::preview::PreviewLoadStage::CreatingHost,
                                    );
                                }
                                if needs_reserve {
                                    // 等 `sync_webview_pool` 回灌 `granted` 再进
                                    // `CreatingHost`;届时若 host 已就绪,`grant_reserve`
                                    // 为 no-op(非 `Reserving`)。
                                    pane.advance_load(
                                        tab_id,
                                        generation,
                                        crate::preview::PreviewLoadStage::Reserving,
                                    );
                                    // T11:等预算最长也有个上限;若 `granted` 迟迟不来
                                    // 则超时进入可重试 Failed(而非永久 Reserving)。
                                    io.arm_load_timeout(
                                        project_id,
                                        panel,
                                        tab_id,
                                        generation,
                                        crate::preview::PreviewLoadStage::Reserving,
                                    );
                                } else {
                                    pane.advance_load(
                                        tab_id,
                                        generation,
                                        crate::preview::PreviewLoadStage::CreatingHost,
                                    );
                                    io.arm_load_timeout(
                                        project_id,
                                        panel,
                                        tab_id,
                                        generation,
                                        crate::preview::PreviewLoadStage::CreatingHost,
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            pane.fail_load(
                                tab_id,
                                generation,
                                crate::preview::PreviewError::new(error, true),
                            );
                        }
                    }
                    // 画像可能把临时非表格 route 改判为表格:此时才入队,需立即
                    // spawn(建壳时已入队的由调用方 `preview_open_path` 那侧已 spawn)。
                    ws.spawn_pending_tabular_loads(panel, io);
                });
            }
            Message::PreviewLoadTimeout(project_id, panel, tab_id, generation, stage) => {
                self.with_project(project_id, move |ws, _io| {
                    let pane = if panel == PanelKind::Project {
                        &mut ws.project_preview
                    } else {
                        &mut ws.preview
                    };
                    // T11:只有仍在该 generation **且仍停在同 stage**的加载才判超时。
                    // stage 不符说明已推进到下一阶段(会有新的超时接管),generation
                    // 不符说明已重开/被替换——两者都静默丢弃,避免迟到超时误杀。
                    if pane.load_timeout_matches(tab_id, generation, stage) {
                        tracing::warn!(
                            panel = ?panel,
                            tab_id,
                            generation,
                            stage = ?stage,
                            "预览加载阶段超时,进入可重试失败"
                        );
                        pane.fail_load(
                            tab_id,
                            generation,
                            crate::preview::PreviewError::new(
                                "预览加载超时,请重试".to_string(),
                                true,
                            ),
                        );
                    }
                });
            }
            Message::PreviewRecoveryWritten(project_id, panel, tab_id) => {
                self.with_project(project_id, move |ws, _io| {
                    let pane = if panel == PanelKind::Project {
                        &mut ws.project_preview
                    } else {
                        &mut ws.preview
                    };
                    if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == tab_id) {
                        tab.recovery_written = true;
                    }
                });
            }
            Message::PreviewRecoveryRead(project_id, panel, tab_id, generation, restore) => {
                self.with_project(project_id, move |ws, _io| {
                    let pane = if panel == PanelKind::Project {
                        &mut ws.project_preview
                    } else {
                        &mut ws.preview
                    };
                    let Some(text) = restore else {
                        return;
                    };
                    // 通常加载仍在途 → 记入 pending_restore,由 DocumentLoaded 消费;
                    // 若 recovery 读比 host 就绪还慢,由本方法就地生成恢复指令。
                    if let Some((tab_id, text, revision)) =
                        pane.apply_recovery_restore(tab_id, generation, text)
                    {
                        let language = pane
                            .tabs()
                            .iter()
                            .find(|t| t.id == tab_id)
                            .and_then(|t| match &t.kind {
                                crate::preview::TabKind::File(p) => {
                                    Some(crate::preview::extension_to_syntax(p))
                                }
                                _ => None,
                            })
                            .unwrap_or_default();
                        pane.queue_editor_command(
                            tab_id,
                            crate::preview::EditorCommand::SetDocument {
                                text,
                                revision,
                                language,
                                read_only: false,
                            },
                        );
                    }
                });
            }
            Message::TermInput(target, bytes) => self.term_input(target, bytes),
            Message::TermImePreedit(target, text) => {
                if target == self.keyboard_term_target() {
                    self.term_ime_preedit = text;
                }
            }
            Message::TermOutput(project_id, tab_id, bytes) => {
                self.term_output(project_id, tab_id, bytes)
            }
            Message::SessionExited(project_id, tab_id) => {
                self.with_project(project_id, |ws, _io| {
                    if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                        tab.alive = false;
                        // 本地标记行，非会话真实输出；应答无处可写，丢弃。
                        let _ = tab.model.feed(&exited_marker());
                    }
                });
            }
            Message::AgentStateChanged(project_id, tab_id, agent, state, transcript_path) => {
                self.agent_state_changed(project_id, tab_id, agent, state, transcript_path)
            }
            Message::AgentCardRefreshed(
                project_id,
                tab_id,
                llm_model,
                mode,
                activity,
                workspace,
            ) => {
                self.with_project(project_id, |ws, _io| {
                    crate::workspace::apply_agent_card_refresh(
                        &mut ws.tabs,
                        tab_id,
                        llm_model,
                        mode,
                        activity,
                        workspace,
                    );
                });
            }
            Message::ReviewLoaded(project_id, source, append, result) => {
                self.with_project(project_id, |ws, _io| {
                    if result.is_ok() {
                        ws.review_nonce = ws.review_nonce.wrapping_add(1);
                    }
                    let nonce = ws.review_nonce;
                    if let Some(rv) = &mut ws.review
                        && rv.source == source
                    {
                        match result {
                            Ok(entries) => {
                                // 快照写入必须放在 `rv.source == source` 判断
                                // 通过之后——这是它跟旧实现(main.rs 里的裸
                                // `static`,过期/乱序结果也会无条件覆盖)的
                                // 关键区别,过期加载结果到这里已经被
                                // 上面的守卫挡在外面,不会再污染快照。
                                merge_review_entries(&mut rv.entries, entries, append);
                                let snapshot = ReviewSnapshot {
                                    entries: &rv.entries,
                                    agent_label: rv.agent.label(),
                                    summary_title: rv.summary_title.clone(),
                                    summary_text: rv.summary_text.clone(),
                                    summary_time: rv.summary_time.clone(),
                                };
                                let json = serde_json::to_string(&snapshot).unwrap_or_default();
                                *ws.review_snapshot.lock().expect("review snapshot 锁") =
                                    Some(json);
                                rv.nonce = nonce;
                                rv.error = None;
                            }
                            Err(e) => rv.error = Some(e),
                        }
                    }
                });
            }
            Message::Conversations(conversations::Message::SessionsRefreshed(
                project_id,
                result,
            )) => {
                self.with_project(project_id, move |ws, _io| {
                    conversations::update(
                        &mut ws.conversations,
                        conversations::Message::SessionsRefreshed(project_id, result),
                    );
                    let open_cid = ws.review.as_ref().and_then(|rv| match &rv.source {
                        ReviewSource::Conversation(cid) => Some(cid.clone()),
                        ReviewSource::Session(_) => None,
                    });
                    if let Some(row) = open_cid.as_deref().and_then(|cid| {
                        ws.conversations
                            .sessions()
                            .and_then(|rows| rows.iter().find(|r| r.conversation_id == cid))
                    }) && let Some(json) =
                        refresh_open_conversation_summary(&mut ws.review, &mut ws.review_nonce, row)
                    {
                        *ws.review_snapshot.lock().expect("review snapshot 锁") = Some(json);
                    }
                });
            }
            Message::Conversations(conversations::Message::SessionOpen(conversation_id, agent)) => {
                self.conversation_session_open(conversation_id, agent);
            }
            Message::Conversations(conversations::Message::DetailLoadMore(
                conversation_id,
                after_turn_index,
            )) => {
                self.with_focused_project(|ws, io| {
                    ws.spawn_review_load_conversation(
                        io,
                        conversation_id,
                        after_turn_index,
                        CONVERSATION_DETAIL_PAGE_SIZE,
                        true,
                    );
                });
            }
            Message::Conversations(conversations::Message::SummaryGenerate(
                conversation_id,
                agent,
            )) => {
                self.conversation_summary_generate(conversation_id, agent);
            }
            Message::Conversations(conversations::Message::SummaryGenerateFinished(
                project_id,
                conversation_id,
                result,
            )) => {
                self.with_project(project_id, move |ws, _io| {
                    ws.conversations
                        .finish_summary_generation(conversation_id, result);
                });
            }
            Message::Conversations(conversations::Message::Hover(id, h)) => self.set_hover(id, h),
            Message::Conversations(conversations::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Conversations(conversations::Message::AgentPickerOpen) => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = self
                        .active_workspace()
                        .map(|ws| conversations::agent_picker_items(&ws.conversations))
                        .unwrap_or_default();
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(Message::Conversations(msg));
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.with_focused_project(|ws, _io| {
                        conversations::update(
                            &mut ws.conversations,
                            conversations::Message::AgentPickerOpen,
                        );
                    });
                }
            }
            Message::Conversations(msg) => {
                self.with_focused_project(|ws, _io| {
                    conversations::update(&mut ws.conversations, msg);
                });
            }
            Message::ProjectDeleteDone(errors) => {
                if !errors.is_empty() {
                    self.daemon_error = Some(format!("删除项目未完全成功: {}", errors.join("; ")));
                }
            }
            Message::Usage(msg @ usage::Message::Loaded(project_id, ..)) => {
                self.with_project(project_id, move |ws, _io| {
                    usage::update(&mut ws.usage, msg);
                });
            }
            Message::Usage(usage::Message::ToggleListCollapse) => {
                self.toggle_panel_list_collapse(PanelKind::Usage);
            }
            Message::Usage(usage::Message::Hover(id, h)) => self.set_hover(id, h),
            Message::Usage(msg) => {
                self.with_focused_project(|ws, _io| {
                    usage::update(&mut ws.usage, msg);
                });
            }
            Message::CodeHealth(msg @ codehealth::Message::Loaded(project_id, ..)) => {
                self.with_project(project_id, move |ws, _io| {
                    codehealth::update(&mut ws.codehealth, msg);
                });
            }
            Message::CodeHealth(msg @ codehealth::Message::Scanned(project_id, ..)) => {
                self.with_project(project_id, move |ws, _io| {
                    codehealth::update(&mut ws.codehealth, msg);
                });
            }
            Message::CodeHealth(codehealth::Message::OpenLocation(path, line)) => {
                self.code_health_open_location(path, line);
            }
            Message::CodeHealth(codehealth::Message::AnalyzeFinding(id)) => {
                self.code_health_analyze_finding(id);
            }
            Message::CodeHealth(codehealth::Message::ScanRequested) => {
                self.with_focused_project(|ws, io| {
                    codehealth::update(&mut ws.codehealth, codehealth::Message::ScanRequested);
                    ws.spawn_codehealth_scan(io);
                });
            }
            Message::CodeHealth(msg) => {
                self.with_focused_project(|ws, _io| {
                    codehealth::update(&mut ws.codehealth, msg);
                });
            }
            Message::SelectTab(idx) => self.select_tab(idx),
            Message::SelectTabNoDrag(idx) => self.select_tab_no_drag(idx),
            Message::CloseTab(idx) => {
                let mut closed_old_len = None;
                self.with_focused_project(|ws, io| {
                    // 目标会话仍在 Running/AwaitingInput → 先弹确认框,不直接
                    // 关(避免误关正在跑/等输入的 agent 会话)。其它状态(IDle/
                    // TurnEnded/已退出)直接关,保持原手感。
                    let confirm_needed = ws.tabs.get(idx).is_some_and(|t| {
                        matches!(
                            t.agent_state,
                            AgentState::Running | AgentState::AwaitingInput
                        )
                    });
                    if confirm_needed {
                        ws.pending_close_tab = ws.tabs.get(idx).map(|t| t.info.id.clone());
                    } else {
                        closed_old_len = Some(ws.tabs.len());
                        ws.close_tab(io, idx);
                        ws.ensure_project_terminal(io);
                    }
                });
                // 见 `App::dehover_after_tab_close` 文档:关掉的 tab 若正被
                // 悬停,残留的 hover 记录会被将来复用同一下标的新 tab 继承。
                if let Some(old_len) = closed_old_len {
                    self.dehover_after_tab_close(
                        HoverId::TermTabItem,
                        HoverId::TermTabClose,
                        idx,
                        old_len,
                    );
                }
            }
            Message::TermTabCloseConfirm => {
                let mut closed = None;
                self.with_focused_project(|ws, io| {
                    // 按 id 现查当前下标,不信打开确认框那一刻存的下标——
                    // 弹窗展示期间主窗口仍可交互,tab 列表可能已经变了
                    // (见 `pending_close_tab` 字段文档)。查不到说明这个
                    // 会话已经通过别的路径被关掉,安全地什么都不做,好过
                    // 用陈旧下标关掉列表里当前占着那个位置的另一个会话。
                    if let Some(id) = ws.pending_close_tab.take()
                        && let Some(idx) = ws.tabs.iter().position(|t| t.info.id == id)
                    {
                        closed = Some((idx, ws.tabs.len()));
                        ws.close_tab(io, idx);
                        ws.ensure_project_terminal(io);
                    }
                });
                if let Some((idx, old_len)) = closed {
                    self.dehover_after_tab_close(
                        HoverId::TermTabItem,
                        HoverId::TermTabClose,
                        idx,
                        old_len,
                    );
                }
            }
            Message::TermTabCloseCancel => {
                self.with_focused_project(|ws, _io| {
                    ws.pending_close_tab = None;
                });
            }
            Message::AgentPickerToggle => {
                #[cfg(target_os = "macos")]
                {
                    let (x, y) = self.last_cursor;
                    let items = crate::workspace::agent_picker_items();
                    if let Some(msg) = crate::chrome::native_menu::show(items, (x, y)) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.with_focused_project(|ws, _io| {
                        ws.agent_picker_open = !ws.agent_picker_open;
                    });
                }
            }
            Message::AgentPickerClose => {
                self.with_focused_project(|ws, _io| {
                    ws.agent_picker_open = false;
                });
            }
            Message::AgentPickerSelect(agent) => {
                self.with_focused_project(|ws, io| {
                    ws.agent_picker_open = false;
                    ws.spawn_new_tab(io, agent, None);
                });
            }
            Message::ProjectAddMenuToggle => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let open_ids: std::collections::HashSet<i64> =
                        self.projects.keys().copied().collect();
                    // 激活页签判定与 `project_tabs_row` 同一套互斥口径。
                    let active_project_id = (self.current_page == AppPage::Workspace)
                        .then_some(self.active_project_id)
                        .flatten();
                    let items = crate::chrome::topbar::project_add_menu_items(
                        &self.recent_projects,
                        &open_ids,
                        active_project_id,
                    );
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        // 经事件循环代理回送选择结果,使其重新走 `Runner::dispatch`
                        // ——rfd 文件夹选择器、剪贴板等原生副作用只在 `dispatch`
                        // 拦截层启动。直接 `self.update(msg)` 会绕过该层,导致
                        // 「打开项目」(`ProjectTabPickFolder`)静默失效(macOS 原生
                        // 菜单同步返回、不经 iced 事件管线)。自洽型菜单项
                        // (如「创建项目」)经 `dispatch` 同样正常。
                        let _ = self.proxy.send_event(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.project_add_menu_open = !self.project_add_menu_open;
                    if self.project_add_menu_open {
                        // 点"＋"时的光标逻辑坐标,作为菜单弹出锚点——同
                        // `todo::set_calendar_anchor`/`set_dispatch_anchor` 手法。
                        self.project_add_menu_anchor = self.last_cursor;
                    }
                }
            }
            Message::ProjectAddMenuClose => {
                self.project_add_menu_open = false;
            }
            Message::Todo(todo::Message::AssignAgent(idx, agent)) => {
                self.todo_assign_agent(idx, agent)
            }
            Message::Todo(todo::Message::DetailOpen(idx)) => self.todo_detail_open(idx),
            Message::Todo(todo::Message::DetailReplySubmit) => self.todo_detail_process(),
            // 数据库连接测试的异步结果带显式 `project_id`——用户可能在等待
            // 期间切走了项目页签,必须按自带 id 路由,不能用当前聚焦项目
            // (同 `TabAttached`/`ProjectSlotLoaded` 那批异步消息的约定,见设计
            // 文档"结果经 `emit` 回传"一节)。特化分支必须排在通配
            // `Message::Database(msg)` **之前**,否则永远匹配不到。
            Message::Database(database::Message::TestConnectionResult(
                project_id,
                source_id,
                result,
            )) => self.database_test_connection_result(project_id, source_id, result),
            // schema 树两个异步结果同 `TestConnectionResult` 口径:自带 project_id,
            // 按自带 id 路由,不能用当前聚焦项目。特化分支必须排在通配
            // `Message::Database(msg)` 之前,否则永远匹配不到。
            Message::Database(database::Message::TablesLoaded(project_id, source_id, result)) => {
                self.database_tables_loaded(project_id, source_id, result)
            }
            Message::Database(database::Message::ColumnsLoaded {
                project_id,
                source_id,
                schema,
                table,
                result,
            }) => self.database_columns_loaded(project_id, source_id, schema, table, result),
            Message::Database(database::Message::BrowseResult(
                project_id,
                tab_id,
                run_seq,
                result,
            )) => self.database_browse_result(project_id, tab_id, run_seq, result),
            Message::Database(database::Message::QueryResult(
                project_id,
                tab_id,
                run_seq,
                result,
            )) => self.database_query_result(project_id, tab_id, run_seq, result),
            Message::Database(database::Message::SourceContextMenu(source_id)) => {
                self.database_source_context_menu(source_id);
            }
            Message::Database(database::Message::TabHover(target, idx, hovered)) => {
                // 内容窗格 tab 本体/关闭按钮的悬停,转发成 `HoverId`(同
                // `ToolbarHover` 的口径)。
                let id = match target {
                    database::DatabaseTabHoverTarget::Title => HoverId::DatabaseTabItem(idx),
                    database::DatabaseTabHoverTarget::Close => HoverId::DatabaseTabClose(idx),
                };
                self.set_hover(id, hovered);
            }
            Message::Database(database::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Database(database::Message::ToggleListCollapse) => {
                self.toggle_panel_list_collapse(PanelKind::Database);
            }
            Message::Database(database::Message::TabOverflowToggle) => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = self
                        .active_workspace()
                        .map(|ws| database::tab_overflow_items(&ws.database))
                        .unwrap_or_default();
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(Message::Database(msg));
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.database.content_mut().toggle_tab_overflow(last_cursor);
                    });
                }
            }
            Message::Database(database::Message::TabOverflowDismiss) => {
                self.with_focused_project(|ws, _io| {
                    ws.database.content_mut().dismiss_tab_overflow();
                });
            }
            Message::Database(database::Message::Hover(id, h)) => self.set_hover(id, h),
            Message::Database(msg) => self.database_message(msg),
            Message::Todo(msg) => match msg {
                todo::Message::Hover(id, h) => self.set_hover(id, h),
                todo::Message::TextInputMenuOpen(target) => {
                    self.update(Message::TextInputMenuOpen(target));
                }
                todo::Message::ToggleListCollapse => {
                    self.toggle_panel_list_collapse(PanelKind::Todo);
                }
                todo::Message::CategoryContextMenuOpen(id) => {
                    self.todo_category_context_menu(id);
                }
                // 以下几种分类动作都是从右键菜单里点出来的:先关掉菜单本
                // 身(浮层 if-else 链里 `category_context_menu` 分支排在
                // `category_picker` 之前,不关会导致"移动到..."开了选择器
                // 却永远被菜单盖住),镜像 `files.rs::RenameStart` 落盘动作
                // 时 `app_state.context_menu = None;` 的既有口径。
                todo::Message::CategoryReparentPickerOpen(id) => {
                    self.category_context_menu = None;
                    self.todo_category_picker_open(CategoryPickerTarget::Category(id));
                }
                todo::Message::CategoryNewChild(_)
                | todo::Message::CategoryNewSibling(_)
                | todo::Message::CategoryDelete(_)
                | todo::Message::CategoryRenameStart(_)
                | todo::Message::CategoryMoveSibling(_, _) => {
                    self.category_context_menu = None;
                    self.todo_message(msg);
                }
                todo::Message::CategoryPickerOpenForTodo(todo_id) => {
                    self.todo_category_picker_open(CategoryPickerTarget::Todo(todo_id));
                }
                todo::Message::DispatchOpen(idx) => {
                    #[cfg(target_os = "macos")]
                    {
                        let last_cursor = self.last_cursor;
                        let items = todo::dispatch_items(idx);
                        if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                            self.update(Message::Todo(msg));
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        self.todo_message(todo::Message::DispatchOpen(idx));
                    }
                }
                todo::Message::StatusOpen(idx) => {
                    #[cfg(target_os = "macos")]
                    {
                        let last_cursor = self.last_cursor;
                        let items = todo::status_items(idx);
                        if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                            self.update(Message::Todo(msg));
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        self.todo_message(todo::Message::StatusOpen(idx));
                    }
                }
                other => self.todo_message(other),
            },
            Message::TodoDetailLoaded(idx, turns) => {
                self.with_focused_project(move |ws, _io| {
                    if ws.todo.detail_open_idx() == Some(idx) {
                        ws.todo.replace_detail_turns(turns);
                    }
                });
            }
            // 文件树右键"搜索"弹窗:`SearchResults` 带 `project_id`,异步结果
            // 按所属项目路由(用户可能已切走);其余交互投当前聚焦项目。
            Message::Search(search::Message::SearchResults(project_id, result)) => {
                self.search_results(project_id, result)
            }
            Message::Search(search::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Search(msg) => {
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                self.with_focused_project(move |ws, _io| {
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Search(m));
                    };
                    search::update(&mut ws.search, msg, project_id, &handle, emit);
                });
            }
            Message::TabAttached(project_id, tab_id, info, snapshot, picked_agent) => {
                let term_tab_bar_avail_px = self.terminal_tab_bar_avail_px();
                self.with_project(project_id, move |ws, io| {
                    ws.on_tab_attached(TabAttachedArgs {
                        cols: io.cols,
                        rows: io.rows,
                        tab_id,
                        info,
                        snapshot,
                        picked_agent,
                        term_tab_bar_avail_px,
                    })
                });
            }
            Message::PaneResized {
                cols,
                rows,
                ssh_cols,
                ssh_rows,
            } => self.pane_resized(cols, rows, ssh_cols, ssh_rows),
            Message::ColumnDragStart(divider) => {
                self.dragging = Some(divider);
            }
            Message::ColumnDrag {
                window_width,
                logical_x,
            } => {
                if let Some(divider) = self.dragging {
                    let state = self.shell_state();
                    self.dims = apply_column_drag(state, divider, window_width, logical_x);
                }
            }
            Message::ColumnDragEnd => {
                self.dragging = None;
                self.on_shell_layout_changed();
            }
            Message::RowDragStart(divider) => {
                self.dragging_row = Some(divider);
            }
            Message::RowDrag {
                window_height,
                logical_y,
            } => {
                if let Some(divider) = self.dragging_row {
                    match divider {
                        RowDivider::GitLogFileDiffSplit => {
                            let state = self.shell_state();
                            self.dims = apply_row_drag(state, divider, window_height, logical_y);
                        }
                        // 新增任务框高度:基线 = 框底 = 左面板区底 =
                        // `window_height - footbar_height`(顶栏在 `base`
                        // 之上,不参与);高度 = 基线 - 光标 y,向上拉变高。
                        // 上限再夹一道,避免列表区被压没(留约 140px)。
                        RowDivider::TodoAddGrow => {
                            let baseline =
                                window_height - byteui::theme::geometry::footbar_height();
                            let max_h =
                                (baseline - byteui::theme::geometry::top_bar_height() - 140.0)
                                    .max(todo::ADD_INPUT_MIN_HEIGHT);
                            let h = (baseline - logical_y).clamp(todo::ADD_INPUT_MIN_HEIGHT, max_h);
                            if let Some(ws) = self.active_workspace_mut() {
                                ws.todo.set_add_input_height(h);
                            }
                        }
                    }
                }
            }
            Message::RowDragEnd => {
                self.dragging_row = None;
                self.on_shell_layout_changed();
            }
            Message::TabDragMove { group, index } => {
                self.tab_drag_move(group, index);
            }
            Message::TabDragEnd => {
                self.end_tab_drag();
            }
            Message::RailDragMove { side, index } => {
                self.rail_drag_move(side, index);
            }
            Message::RailDragEnd => {
                self.end_rail_drag();
            }
            Message::TodoDragEnd => {
                self.todo_message(todo::Message::DragEnd);
            }
            Message::PanelSelect(v) => self.panel_select(v),
            Message::ToggleFileTreeCollapse => self.toggle_files_tree_collapse(),
            Message::TogglePanelListCollapse(kind) => self.toggle_panel_list_collapse(kind),
            Message::TextInputMenuOpen(target) => {
                #[cfg(target_os = "macos")]
                {
                    let (x, y) = self.files.last_right_click();
                    let items = text_input_menu_items(&target);
                    if let Some(msg) = crate::chrome::native_menu::show(items, (x, y))
                        && let Some(ch) = crate::platform::window_events::menu_edit_key(&msg)
                    {
                        self.pending_native_menu_edit_key = Some((ch, target.id.clone()));
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    // 与其它右键菜单互斥——关掉别的,只留本菜单(同时避免互相顶)。
                    self.files.close_context_menu();
                    self.project_link_menu = None;
                    let (x, y) = self.files.last_right_click();
                    self.text_input_menu = Some(TextInputMenu {
                        x,
                        y,
                        target: target.clone(),
                    });
                    // 右键不聚焦 iced 输入框(只有左键会),菜单的复制/粘贴需要通过
                    // `interface.operate` 把焦点移到目标输入,否则合成回的 ⌘+c/v
                    // 事件作用不到它。记录待聚焦 id,本帧后由 `apply_pending_focus`
                    // 应用(main.rs)。
                    self.pending_text_input_focus = Some(target.id);
                }
            }
            Message::TextInputMenuClose => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuCut => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuCopy => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuPaste => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuSelectAll => {
                self.text_input_menu = None;
            }
            Message::Hover(id, h) => {
                self.set_hover(id, h);
            }
            Message::MaximizeClose => {
                self.maximized = None;
                self.sync_terminal_grid();
            }
            Message::TopBarDoubleClick => {
                self.pending_zoom_toggle = true;
            }
            Message::TopBarHome => self.top_bar_home(),
            Message::HomeRecentsLoaded(files, convs) => {
                self.home_recent_files = files;
                self.home_recent_conversations = convs;
                self.home_recents_loaded = true;
            }
            Message::HomeLeftIconSelect(v) => {
                self.home_left_view = v;
            }
            Message::HomeMoreProjects => self.home_project_pages += 1,
            Message::HomeProjectSearchInput(s) => self.home_project_search_draft = s,
            Message::HomeProjectSearchSubmit => self.commit_home_project_search(),
            Message::HomeRightIconSelect(v) => {
                self.home_right_view = v;
            }
            Message::HomeBrowser(msg) => {
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::HomeBrowser(m));
                };
                browser::update(&mut self.home_browser, msg, None, &client, &handle, emit);
            }
            Message::BrowserTitle(id, title) => {
                let msg = browser::Message::TitleLoaded(id, title);
                if self.is_home() {
                    // 首页全局浏览器:webview 属于 `home_browser`,直接落地。
                    browser::update(
                        &mut self.home_browser,
                        msg,
                        None,
                        &self.client,
                        &self.handle,
                        |_| {},
                    );
                } else {
                    // 工作区浏览器:webview id 落在当前聚焦工作区的 `ws.browser`。
                    self.with_focused_project(|ws, io| {
                        browser::update(
                            &mut ws.browser,
                            msg,
                            ws.project.as_ref().map(|p| p.id),
                            &io.client,
                            &io.handle,
                            |_| {},
                        );
                    });
                }
            }
            Message::BrowserNavigated(id, url) => {
                let msg = browser::Message::Loaded(id, url);
                if self.is_home() {
                    browser::update(
                        &mut self.home_browser,
                        msg,
                        None,
                        &self.client,
                        &self.handle,
                        |_| {},
                    );
                } else {
                    self.with_focused_project(|ws, io| {
                        browser::update(
                            &mut ws.browser,
                            msg,
                            ws.project.as_ref().map(|p| p.id),
                            &io.client,
                            &io.handle,
                            |_| {},
                        );
                    });
                }
            }
            Message::BrowserNewWindow(url) => {
                let msg = browser::Message::OpenUrl(url);
                if self.is_home() {
                    browser::update(
                        &mut self.home_browser,
                        msg,
                        None,
                        &self.client,
                        &self.handle,
                        |_| {},
                    );
                } else {
                    self.with_focused_project(|ws, io| {
                        browser::update(
                            &mut ws.browser,
                            msg,
                            ws.project.as_ref().map(|p| p.id),
                            &io.client,
                            &io.handle,
                            |_| {},
                        );
                    });
                }
            }
            Message::Noop => {}
            Message::DaemonError(message) => self.daemon_error = Some(message),
            Message::TermScroll(target, delta) => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        terminal::TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        terminal::TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.scroll_display(delta);
                    }
                });
            }
            Message::TermTabOverflowToggle => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = match self.active_workspace() {
                        Some(ws) => {
                            let entries: Vec<(usize, String, bool)> = ws
                                .tabs
                                .iter()
                                .enumerate()
                                .map(|(idx, tab)| {
                                    (
                                        idx,
                                        tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
                                        idx == ws.active,
                                    )
                                })
                                .collect();
                            tab_widget::tab_overflow_items(&entries, Message::SelectTab)
                        }
                        None => Vec::new(),
                    };
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.term_tab_overflow_anchor = if ws.term_tab_overflow_anchor.is_some() {
                            None
                        } else {
                            Some(last_cursor)
                        };
                    });
                }
            }
            Message::TermTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.term_tab_overflow_anchor = None;
                });
            }
            Message::PreviewTabOverflowToggle => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = match self.active_workspace() {
                        Some(ws) => {
                            let entries: Vec<(usize, String, bool)> = ws
                                .preview
                                .tabs()
                                .iter()
                                .enumerate()
                                .map(|(idx, tab)| {
                                    (idx, tab.title.clone(), idx == ws.preview.active_idx())
                                })
                                .collect();
                            tab_widget::tab_overflow_items(&entries, Message::PreviewSelectTab)
                        }
                        None => Vec::new(),
                    };
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.preview_tab_overflow_anchor = if ws.preview_tab_overflow_anchor.is_some()
                        {
                            None
                        } else {
                            Some(last_cursor)
                        };
                    });
                }
            }
            Message::PreviewTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.preview_tab_overflow_anchor = None;
                });
            }
            Message::TermSelStart {
                target,
                col,
                row,
                right,
            } => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        terminal::TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        terminal::TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.selection_start(col, row, right);
                    }
                });
            }
            Message::TermSelUpdate {
                target,
                col,
                row,
                right,
            } => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        terminal::TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        terminal::TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.selection_update(col, row, right);
                    }
                });
            }
            Message::TermPaste(target, text) => self.term_paste(target, text),
            Message::PreviewOpenPath(path) => self.preview_open_path(path),
            // 空白页信息卡回灌:路由到 spawn 时记录的项目(用户中途切项目则
            // `info.path != ws.project.path` → 直接丢)。同框 `blank_info_in_flight`
            // 先清回 false,再做 stale guard(active tab 已不是 Blank 也丢)。
            Message::PreviewBlankInfoLoaded(project_id, kind, info) => {
                self.with_project(project_id, move |ws, _io| {
                    let pane = match kind {
                        PanelKind::Project => &mut ws.project_preview,
                        _ => &mut ws.preview,
                    };
                    pane.blank_info_in_flight = false;
                    let current_root = ws.project.as_ref().map(|p| p.path.as_str());
                    if !current_root.is_some_and(|r| r == info.path.to_string_lossy().as_ref()) {
                        return;
                    }
                    let active_is_blank = pane
                        .tabs
                        .get(pane.active)
                        .is_some_and(|t| matches!(t.kind, crate::preview::TabKind::Blank));
                    if !active_is_blank {
                        return;
                    }
                    pane.blank_info = Some(info);
                });
            }
            Message::PreviewLargeFileSearchClose(kind) => {
                self.with_focused_project(move |ws, _io| match kind {
                    PanelKind::Project => ws.project_preview.close_large_file_search(),
                    _ => ws.preview.close_large_file_search(),
                });
            }
            Message::PreviewLargeFileSearchSubmit(kind, tab_id, query) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                self.with_focused_project(move |ws, io| {
                    let pane = match kind {
                        PanelKind::Project => &mut ws.project_preview,
                        _ => &mut ws.preview,
                    };
                    // T9 bullet 5:空查询立即取消在途查询并清 loading,不派任务。
                    if query.trim().is_empty() {
                        pane.clear_large_file_search(tab_id);
                        return;
                    }
                    let Some(tab) = pane.tabs().iter().find(|t| t.id == tab_id) else {
                        return;
                    };
                    let crate::preview::TabKind::File(path) = tab.kind.clone() else {
                        return;
                    };
                    // T9 bullet 2:作废旧 generation(置位共享取消信号),新查询
                    // 的 generation + 取消句柄捕获进后台任务。
                    let Some((generation, cancel)) =
                        pane.start_large_file_search(tab_id, query.clone())
                    else {
                        return;
                    };
                    let proxy = io.proxy.clone();
                    let progress_proxy = io.proxy.clone();
                    io.handle.spawn(async move {
                        let result = tokio::task::spawn_blocking(move || {
                            // 进度限频:最快每 100ms 回灌一次(命中与进度都
                            // 封顶,长文件扫描不至于把 UI 事件队列打爆)。
                            let mut last = std::time::Instant::now()
                                .checked_sub(std::time::Duration::from_secs(1))
                                .unwrap_or_else(std::time::Instant::now);
                            let should_cancel = {
                                let cancel = cancel.clone();
                                move || cancel.load(std::sync::atomic::Ordering::Relaxed)
                            };
                            // 窗口化:在整文件上流式搜索(不只搜持有窗口)。
                            crate::preview::stream_search_cancellable(
                                &path,
                                &query,
                                crate::preview::SearchOptions::default(),
                                should_cancel,
                                move |p| {
                                    let now = std::time::Instant::now();
                                    if now.duration_since(last)
                                        < std::time::Duration::from_millis(100)
                                    {
                                        return;
                                    }
                                    last = now;
                                    let _ = progress_proxy.send_event(
                                        Message::PreviewLargeFileSearchProgress(
                                            project_id,
                                            kind,
                                            tab_id,
                                            generation,
                                            p.completed,
                                            p.total,
                                        ),
                                    );
                                },
                            )
                            .map_err(|e| e.to_string())
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                        // 取消返回 `Ok(None)`:任务已被新查询/关闭作废,结果与
                        // generation 都过期,直接丢弃(T9 bullet 2)。
                        let message = match result {
                            Ok(Some(outcome)) => Ok(outcome),
                            // 取消(`Ok(None)`):已被新查询/关闭作废,静默丢弃。
                            Ok(None) => return,
                            Err(error) => Err(error),
                        };
                        let _ = proxy.send_event(Message::PreviewLargeFileSearchResults(
                            project_id, kind, tab_id, generation, message,
                        ));
                    });
                });
            }
            Message::PreviewLargeFileSearchResults(
                project_id,
                kind,
                tab_id,
                generation,
                result,
            ) => {
                self.with_project(project_id, move |ws, _io| {
                    let pane = match kind {
                        PanelKind::Project => &mut ws.project_preview,
                        _ => &mut ws.preview,
                    };
                    pane.set_large_file_search_results(tab_id, generation, result);
                });
            }
            Message::PreviewLargeFileSearchProgress(
                project_id,
                kind,
                tab_id,
                generation,
                completed,
                total,
            ) => {
                self.with_project(project_id, move |ws, _io| {
                    let pane = match kind {
                        PanelKind::Project => &mut ws.project_preview,
                        _ => &mut ws.preview,
                    };
                    pane.set_large_file_search_progress(
                        tab_id,
                        generation,
                        crate::preview::PreviewLoadProgress {
                            completed,
                            total: (total > 0).then_some(total),
                        },
                    );
                });
            }
            Message::PreviewLargeFileSearchGo(kind, forward) => {
                self.with_focused_project(move |ws, _io| {
                    let pane = match kind {
                        PanelKind::Project => &mut ws.project_preview,
                        _ => &mut ws.preview,
                    };
                    let Some(session) = pane.large_file_search.clone() else {
                        return;
                    };
                    if session.hits.is_empty() {
                        return;
                    }
                    let next = if forward {
                        (session.current + 1) % session.hits.len()
                    } else {
                        (session.current + session.hits.len() - 1) % session.hits.len()
                    };
                    let hit = session.hits[next].clone();
                    let error_slot = match kind {
                        PanelKind::Project => &mut ws.project_preview_error,
                        _ => &mut ws.preview_error,
                    };
                    // 窗口化只读:命中在未加载区,先装窗口再全局 reveal。
                    let line = hit.line_no.max(1) as u32;
                    pane.queue_windowed_view(session.tab_id, line);
                    pane.queue_editor_command(
                        session.tab_id,
                        crate::preview::EditorCommand::RevealPosition { line, column: 1 },
                    );
                    *error_slot = None;
                    if let Some(s) = pane.large_file_search.as_mut() {
                        s.current = next;
                    }
                });
            }
            Message::PreviewSelectTab(idx) => self.preview_select_tab(idx),
            Message::PreviewOpenExternal(kind, tab_id) => {
                self.with_focused_project(move |ws, _io| {
                    let pane = match kind {
                        PanelKind::Project => &ws.project_preview,
                        _ => &ws.preview,
                    };
                    let Some(tab) = pane.tabs().iter().find(|t| t.id == tab_id) else {
                        return;
                    };
                    let crate::preview::TabKind::File(path) = &tab.kind else {
                        return;
                    };
                    if !path.is_file() {
                        return;
                    }
                    // 只交给系统默认应用打开,不隐式执行文件本身。
                    #[cfg(target_os = "macos")]
                    let program = "open";
                    #[cfg(not(target_os = "macos"))]
                    let program = "xdg-open";
                    if let Err(e) = std::process::Command::new(program).arg(path).spawn() {
                        tracing::warn!(%e, "外部打开失败");
                    }
                });
            }
            Message::PreviewRetry(kind, tab_id) => {
                self.with_focused_project(move |ws, io| {
                    ws.preview_retry(kind, tab_id, io);
                });
            }
            Message::PreviewPlainTextOpen(kind, tab_id) => {
                self.with_focused_project(move |ws, io| {
                    ws.preview_plain_text_open(kind, tab_id, io);
                });
            }
            Message::PreviewConflictKeep(kind, tab_id) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_conflict_keep(kind, tab_id);
                });
            }
            Message::PreviewConflictReload(kind, tab_id) => {
                self.with_focused_project(move |ws, io| {
                    ws.preview_conflict_reload(kind, tab_id, io);
                });
            }
            Message::PreviewTabularTextModeToggle(kind, tab_id) => {
                self.with_focused_project(move |ws, _io| {
                    let pane = if kind == PanelKind::Project {
                        &mut ws.project_preview
                    } else {
                        &mut ws.preview
                    };
                    if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == tab_id)
                        && let Some(crate::preview::PreviewBackend::Tabular(tabular)) =
                            tab.backend.as_mut()
                    {
                        tabular.mode = match tabular.mode {
                            crate::preview::TabularMode::Grid => crate::preview::TabularMode::Text,
                            crate::preview::TabularMode::Text => crate::preview::TabularMode::Grid,
                        };
                    }
                });
            }
            Message::PreviewJsonModeToggle(kind, tab_id) => {
                self.with_focused_project(move |ws, io| {
                    if let Some((tab_id, generation)) =
                        ws.preview_pane_toggle_json_mode(kind, tab_id)
                        && let Some(project_id) = ws.project.as_ref().map(|p| p.id)
                    {
                        io.arm_load_timeout(
                            project_id,
                            kind,
                            tab_id,
                            generation,
                            crate::preview::PreviewLoadStage::SwitchingMode,
                        );
                    }
                });
            }
            Message::PreviewCloseTab(idx) => {
                let mut closed_old_len = None;
                self.with_focused_project(|ws, io| {
                    // 关闭前静默保存该 tab 的就地改动。老 iced editor 走
                    // `preview_pane_save_at` 就地落盘;CodeMirror tab 的正文
                    // 活在 webview 里,Rust 不持有,改为下发 `SaveDocument`,
                    // 等 host 回 `save_requested` 落盘后再关(见该分支的
                    // `take_pending_close`)。此路径下这里**不**立刻 close。
                    if ws.preview.request_save_before_close_if_dirty(idx) {
                        return;
                    }
                    ws.preview_pane_save_at(PanelKind::Files, idx);
                    closed_old_len = Some(ws.preview.tabs().len());
                    ws.preview.close(idx);
                    // 关 tab 后位置全变，旧 first 可能越界——归零防御（P1L T5）。
                    ws.preview_tab_first = 0;
                    ws.spawn_preview_state_save(io);
                    // 关闭:上下文多半变了,立即 flush,不等防抖窗口。
                    ws.flush_preview_context_push(io);
                });
                // 见 `App::dehover_after_tab_close` 文档:关掉的 tab 若正被
                // 悬停,残留的 hover 记录会被将来复用同一下标的新 tab 继承。
                if let Some(old_len) = closed_old_len {
                    self.dehover_after_tab_close(
                        HoverId::PreviewTabItem,
                        HoverId::PreviewTabClose,
                        idx,
                        old_len,
                    );
                }
            }
            Message::PreviewToggleRenderMode(idx) => {
                self.with_focused_project(|ws, io| {
                    if let Some((tab_id, generation)) =
                        ws.preview_pane_toggle_render_mode(PanelKind::Files, idx)
                        && let Some(project_id) = ws.project.as_ref().map(|p| p.id)
                    {
                        io.arm_load_timeout(
                            project_id,
                            PanelKind::Files,
                            tab_id,
                            generation,
                            crate::preview::PreviewLoadStage::SwitchingMode,
                        );
                    }
                });
            }
            Message::PreviewSaveActive(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_pane_save_active(kind));
            }
            Message::PreviewTabInsertTab(kind) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_pane_active_editor_event(
                        kind,
                        iced_widget::text_editor::Action::Edit(
                            iced_widget::text_editor::Edit::Insert('\t'),
                        ),
                    );
                });
            }
            Message::PreviewUndoActive(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_pane_undo_active(kind));
            }
            Message::PreviewRedoActive(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_pane_redo_active(kind));
            }
            Message::PreviewFindOpen(kind) => {
                self.with_focused_project(move |ws, _io| {
                    // 窗口化 CodeMirror tab:普通 Find 只搜持有窗口没意义,改为
                    // 打开整文件流式搜索条(与 host `find_request` 同一条 session)。
                    let windowed_id = {
                        let pane = match kind {
                            PanelKind::Project => &ws.project_preview,
                            _ => &ws.preview,
                        };
                        let idx = pane.active_idx();
                        pane.tabs()
                            .get(idx)
                            .filter(|t| t.uses_windowed_editor())
                            .map(|t| t.id)
                    };
                    if let Some(tab_id) = windowed_id {
                        let pane = match kind {
                            PanelKind::Project => &mut ws.project_preview,
                            _ => &mut ws.preview,
                        };
                        pane.open_large_file_search(tab_id);
                    } else {
                        ws.preview_find_open(kind);
                    }
                });
            }
            Message::PreviewFindOpenWithReplace(kind) => {
                // 窗口化大文件没有"替换"概念,同样分流到整文件搜索条(忽略
                // "默认展开替换行"这个语义,大文件搜索条本来就没有替换行)。
                self.with_focused_project(move |ws, _io| {
                    let windowed_id = {
                        let pane = match kind {
                            PanelKind::Project => &ws.project_preview,
                            _ => &ws.preview,
                        };
                        let idx = pane.active_idx();
                        pane.tabs()
                            .get(idx)
                            .filter(|t| t.uses_windowed_editor())
                            .map(|t| t.id)
                    };
                    if let Some(tab_id) = windowed_id {
                        let pane = match kind {
                            PanelKind::Project => &mut ws.project_preview,
                            _ => &mut ws.preview,
                        };
                        pane.open_large_file_search(tab_id);
                    } else {
                        ws.preview_find_open_with_replace(kind);
                    }
                });
            }
            Message::PreviewFindReplaceToggle(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_toggle_replace(kind));
            }
            Message::PreviewFindClose(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_close(kind));
            }
            Message::PreviewFindText(kind, query) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_type(kind, query));
            }
            Message::PreviewFindGo(kind, next) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_go(kind, next));
            }
            Message::PreviewFindCase(kind, sensitive) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_case(kind, sensitive));
            }
            Message::PreviewFindReplacement(kind, repl) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_find_set_replacement(kind, repl);
                });
            }
            Message::PreviewFindReplaceCurrent(kind) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_find_replace_current(kind);
                });
            }
            Message::PreviewFindReplaceAll(kind) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_find_replace_all(kind);
                });
            }
            Message::PreviewFindWebviewState(kind, current, total) => {
                // flyfish `getSearchState()` 回写:直接落进 `kind` 面板当前
                // webview Find 会话的 `current`/`count`。这是异步回调(经
                // `EventLoopProxy` 送回),不依赖任何 webview 句柄,在 `App`
                // 顶部 update 里落库即可——真正的 JS 注入在 `window_events::
                // apply_pending_preview_find`(句柄只在那里拿得到)。iced 在
                // 处理完这条消息后会自然重绘,n/m 计数立即刷新。
                self.preview_find_set_webview_state(kind, current, total);
            }
            Message::TabularLoaded(project_id, kind, tab_id, generation, result) => {
                self.with_project(project_id, move |ws, io| {
                    // T11:取消是正常结束,不当作解析失败告警。
                    match &result {
                        Err(error) if error == crate::tabular::TABULAR_CANCELLED => {
                            tracing::debug!(panel = ?kind, tab_id, generation, "表格加载已取消");
                        }
                        Err(error) => {
                            tracing::warn!(panel = ?kind, tab_id, generation, %error, "表格首次加载失败");
                        }
                        Ok(view) => {
                            tracing::debug!(
                                panel = ?kind,
                                tab_id,
                                generation,
                                sheets = view.sheet_names.len(),
                                "表格首次加载完成"
                            );
                        }
                    }
                    let sheet_to_select = {
                        let pane = if kind == PanelKind::Project {
                            &mut ws.project_preview
                        } else {
                            &mut ws.preview
                        };
                        pane.finish_tabular_load(tab_id, generation, result)
                    };
                    // 恢复的 active sheet 不是首个 → 触发一次懒加载。
                    // `select_sheet`(经 `preview_pane_tabular_action`)会把
                    // scroll_row/scroll_col 重置为 0(正常切 sheet 的预期
                    // 行为),因此要在它之后把持久化的 row/col 重新应用
                    // 一遍,否则恢复到非首个 sheet 的滚动位置会被静默清零。
                    if let Some((sheet, row, col)) = sheet_to_select {
                        ws.preview_pane_tabular_action(kind, tab_id, sheet, io);
                        let pane = if kind == PanelKind::Project {
                            &mut ws.project_preview
                        } else {
                            &mut ws.preview
                        };
                        if let Some(view) = pane.tabular_mut(tab_id) {
                            view.scroll_row = row;
                            view.scroll_col = col;
                        }
                    }
                });
            }
            Message::TabularSheetLoaded(project_id, kind, tab_id, sheet_index, result) => {
                self.with_project(project_id, move |ws, _io| {
                    let pane = if kind == PanelKind::Project {
                        &mut ws.project_preview
                    } else {
                        &mut ws.preview
                    };
                    let loaded_ok = result.is_ok();
                    if let Some(crate::preview::TabularState::Ready(view)) =
                        pane.tabular_state_mut(tab_id)
                    {
                        view.apply_sheet_loaded(sheet_index, result);
                    }
                    pane.queue_tabular_command(
                        tab_id,
                        crate::preview::TabularCommand::SetSheetLoading {
                            sheet_index,
                            loading: false,
                        },
                    );
                    // 用户可能在这次懒加载完成前又切到了别的 sheet
                    // (rapid switch)——`view.active_sheet` 已经不是
                    // `sheet_index` 时,不该把 JS 拽回这个已经不再是目标的
                    // sheet(否则会看到一个没数据的空白网格)。
                    let still_active = pane
                        .tabular_mut(tab_id)
                        .is_some_and(|view| view.active_sheet == sheet_index);
                    if loaded_ok && still_active {
                        pane.queue_tabular_command(
                            tab_id,
                            crate::preview::TabularCommand::SelectSheet { sheet_index },
                        );
                        pane.push_sheet_schema_and_window(tab_id, sheet_index);
                    }
                });
            }
            Message::ProjectPreviewOpenPath(path) => self.project_preview_open_path(path),
            Message::ProjectPreviewSelectTab(idx) => self.project_preview_select_tab(idx),
            Message::ProjectPreviewCloseTab(idx) => {
                let mut closed_old_len = None;
                self.with_focused_project(|ws, _io| {
                    // 关闭前静默保存,语义同 `PreviewCloseTab`:CodeMirror tab
                    // 下发 `SaveDocument` 等回落盘再关,其余就地 `save_at` 后关。
                    if ws.project_preview.request_save_before_close_if_dirty(idx) {
                        return;
                    }
                    ws.preview_pane_save_at(PanelKind::Project, idx);
                    closed_old_len = Some(ws.project_preview.tabs().len());
                    ws.project_preview.close(idx);
                    ws.project_preview_tab_first = 0;
                });
                // 见 `App::dehover_after_tab_close` 文档:关掉的 tab 若正被
                // 悬停,残留的 hover 记录会被将来复用同一下标的新 tab 继承。
                if let Some(old_len) = closed_old_len {
                    self.dehover_after_tab_close(
                        HoverId::ProjectPreviewTabItem,
                        HoverId::ProjectPreviewTabClose,
                        idx,
                        old_len,
                    );
                }
            }
            Message::ProjectPreviewToggleRenderMode(idx) => {
                self.with_focused_project(|ws, io| {
                    if let Some((tab_id, generation)) =
                        ws.preview_pane_toggle_render_mode(PanelKind::Project, idx)
                        && let Some(project_id) = ws.project.as_ref().map(|p| p.id)
                    {
                        io.arm_load_timeout(
                            project_id,
                            PanelKind::Project,
                            tab_id,
                            generation,
                            crate::preview::PreviewLoadStage::SwitchingMode,
                        );
                    }
                });
            }
            Message::ProjectPreviewTabOverflowToggle => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = match self.active_workspace() {
                        Some(ws) => {
                            let entries: Vec<(usize, String, bool)> = ws
                                .project_preview
                                .tabs()
                                .iter()
                                .enumerate()
                                .map(|(idx, tab)| {
                                    (
                                        idx,
                                        tab.title.clone(),
                                        idx == ws.project_preview.active_idx(),
                                    )
                                })
                                .collect();
                            tab_widget::tab_overflow_items(
                                &entries,
                                Message::ProjectPreviewSelectTab,
                            )
                        }
                        None => Vec::new(),
                    };
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.project_preview_tab_overflow_anchor =
                            if ws.project_preview_tab_overflow_anchor.is_some() {
                                None
                            } else {
                                Some(last_cursor)
                            };
                    });
                }
            }
            Message::ProjectPreviewTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.project_preview_tab_overflow_anchor = None;
                });
            }
            Message::Browser(browser::Message::BookmarksLoaded(pid, bookmarks)) => {
                self.browser_bookmarks_loaded(pid, bookmarks)
            }
            Message::Browser(browser::Message::BookmarksMutated(pid, res)) => {
                self.browser_bookmarks_mutated(pid, res)
            }
            Message::Browser(browser::Message::DragHover(idx)) => {
                // 浏览器 tab 脱的换位:光标扫过 `idx` 页签 → 走共同换位逻辑。
                self.tab_drag_move(TabGroup::Browser, idx);
            }
            Message::Browser(browser::Message::ColumnDragStart) => {
                self.update(Message::ColumnDragStart(Divider::BrowserBookmarksSplit));
            }
            Message::Browser(browser::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Browser(msg) => self.browser_message(msg),
            Message::ProjectSelect(id) => self.project_select(id),
            Message::ProjectTabPickFolder => {
                // 副作用在 main.rs(rfd 文件夹选择);从新增项目菜单触发时顺带
                // 关掉菜单,同 `project_select` 的处理口径。
                self.project_add_menu_open = false;
            }
            // rfd 弹窗在 main.rs 里同步处理,选中后转成 project::Message::LinkAdd
            // 再回送到这里;这条顶层消息本身不需要 App::update 处理任何东西。
            Message::ProjectLinkPick(_) => {}
            Message::ProjectLinkContextMenuClose => {
                self.project_link_menu = None;
            }
            Message::CategoryContextMenuClose => {
                self.category_context_menu = None;
            }
            Message::CategoryPickerClose => {
                self.category_picker = None;
            }
            Message::CategoryPickerSelect(chosen) => {
                let Some(picker) = self.category_picker.take() else {
                    return;
                };
                match picker.target {
                    CategoryPickerTarget::Todo(todo_id) => {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        handle.spawn(async move {
                            let res = client
                                .set_todo_category(todo_id, chosen)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string());
                            let _ = proxy
                                .send_event(Message::Todo(todo::Message::CategoryMutated(res)));
                        });
                    }
                    CategoryPickerTarget::Category(category_id) => {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        handle.spawn(async move {
                            let res = client
                                .reparent_category(category_id, chosen)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string());
                            let _ = proxy
                                .send_event(Message::Todo(todo::Message::CategoryMutated(res)));
                        });
                    }
                }
            }
            Message::DatabaseSourceContextMenuClose => {
                self.database_source_menu = None;
            }
            Message::ProjectTabOpen(path) => {
                let client = self.client.clone();
                let proxy = self.proxy.clone();
                let path_s = path.to_string_lossy().into_owned();
                self.handle.spawn(async move {
                    let opened = client.open_project(&path_s).await.ok().flatten();
                    let recent = client.list_projects().await.unwrap_or_default();
                    let _ = proxy.send_event(Message::ProjectTabOpened(opened, recent, false));
                });
            }
            Message::ProjectTabOpened(project, recent, skip_git_init) => {
                self.project_tab_opened(project, recent, skip_git_init)
            }
            Message::ProjectTabSwitch(id) => self.project_tab_switch(id),
            Message::ProjectTabClose(id) => self.project_tab_close(id),
            Message::ProjectSlotLoaded(id, payload) => self.project_slot_loaded(id, payload),
            Message::ProjectFsChanged(project_id, changes) => {
                self.project_fs_changed(project_id, changes)
            }
            Message::GitLog(git_log::Message::ColumnDragStart) => {
                // Git Log 三栏布局里左右分割线开始拖拽——扩展发不了 app 级
                // 拖拽消息,由内核代发。
                self.update(Message::ColumnDragStart(Divider::GitLogSplit));
            }
            Message::GitLog(git_log::Message::RowDragStart) => {
                self.update(Message::RowDragStart(RowDivider::GitLogFileDiffSplit));
            }
            Message::GitLog(git_log::Message::BranchPickerOpen) => {
                #[cfg(target_os = "macos")]
                {
                    // macOS:版本选择菜单直接弹原生 NSMenu(与 tab 组下拉/
                    // 文件树分支菜单同款,2026-09-28),选中即转 `BranchSwitch`
                    // 走下方既有 checkout 回路。原生菜单是同步模态的,没有
                    // "先弹层、列表异步落地再填充"的窗口,故首次展开(分支
                    // 列表还没缓存)时直接在主线程同步查一次本地分支——纯
                    // 本地 git refs 读取,毫秒级,可接受。
                    if self.git_log.branches_is_empty()
                        && let Some(repo_path) = self
                            .active_workspace()
                            .and_then(|ws| ws.active_project_path())
                    {
                        let branches =
                            crate::delivery::local_branches(&repo_path).unwrap_or_default();
                        let dirty = crate::delivery::is_dirty(&repo_path);
                        git_log::update(
                            &mut self.git_log,
                            git_log::Message::BranchesLoaded(repo_path, branches, dirty),
                            &self.handle,
                            |_| {},
                        );
                    }
                    let (head_branch, branches, dirty) = self.git_log.branch_picker_snapshot();
                    let last_cursor = self.last_cursor;
                    let items = crate::workspace::branch_picker_items(
                        head_branch.as_deref(),
                        &branches,
                        dirty,
                        |name| Message::GitLog(git_log::Message::BranchSwitch(name)),
                    );
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    // 非 macOS:iced 弹层兜底——先把"展开"状态位落地(纯状态
                    // 机部分仍走 update),首次展开且还没缓存过分支列表时,
                    // 顺带异步查一次本地分支。
                    let handle = self.handle.clone();
                    let proxy = self.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::GitLog(m));
                    };
                    let needs_fetch = self.git_log.branches_is_empty();
                    git_log::update(
                        &mut self.git_log,
                        git_log::Message::BranchPickerOpen,
                        &handle,
                        emit.clone(),
                    );
                    if needs_fetch
                        && let Some(repo_path) = self
                            .active_workspace()
                            .and_then(|ws| ws.active_project_path())
                    {
                        self.handle.spawn(async move {
                            let repo_path2 = repo_path.clone();
                            let (branches, dirty) = tokio::task::spawn_blocking(move || {
                                let branches = crate::delivery::local_branches(&repo_path2)
                                    .unwrap_or_default();
                                let dirty = crate::delivery::is_dirty(&repo_path2);
                                (branches, dirty)
                            })
                            .await
                            .unwrap_or_default();
                            emit(git_log::Message::BranchesLoaded(repo_path, branches, dirty));
                        });
                    }
                }
            }
            Message::GitLog(git_log::Message::BranchSwitch(name)) => {
                let Some(repo_path) = self
                    .active_workspace()
                    .and_then(|ws| ws.active_project_path())
                else {
                    return;
                };
                self.git_log.set_branch_switch_pending(true);
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let repo_path2 = repo_path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        crate::delivery::checkout_branch(&repo_path2, &name)
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let _ = proxy
                        .send_event(Message::GitLog(git_log::Message::BranchSwitchDone(result)));
                });
            }
            Message::GitLog(git_log::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::GitLog(msg) => {
                if let git_log::Message::Hover(id, h) = msg {
                    self.set_hover(id, h);
                    return;
                }
                // 分支切换成功后(checkout 改了 HEAD/工作区),commit 列表要重拉。
                let is_branch_switch_success =
                    matches!(&msg, git_log::Message::BranchSwitchDone(Ok(())));
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::GitLog(m));
                };
                if let Some(next) = git_log::update(&mut self.git_log, msg, &handle, emit.clone()) {
                    self.update(Message::GitLog(next));
                }
                if is_branch_switch_success
                    && let Some(repo_path) = self
                        .active_workspace()
                        .and_then(|ws| ws.active_project_path())
                {
                    let max_count = self.git_log.cache_max_count();
                    git_log::request_refresh(
                        &mut self.git_log,
                        repo_path,
                        max_count,
                        &handle,
                        emit,
                    );
                }
            }
            Message::FileHistory(msg) => {
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::FileHistory(m));
                };
                file_history::update(&mut self.file_history, msg, &handle, emit);
            }
            Message::AgentContext(project_id, msg) => self.agent_context_message(project_id, msg),
            Message::ProjectCreateOpen => {
                self.project_create = Some(project_create::State::default());
            }
            Message::ProjectCreate(project_create::Message::GoToSettings) => {
                self.project_create = None;
                self.settings = Some(settings::State::load(self.daemon_error.as_deref()));
            }
            Message::ProjectCreate(project_create::Message::Done(Ok((
                project,
                recent,
                skip_git_init,
            )))) => {
                self.project_create = None;
                self.project_tab_opened(project, recent, skip_git_init);
            }
            // 失败分支故意不在这里拦截:`project_create::update` 自己的
            // `Done` 处理会把错误填进 `State::error` 并保持对话框打开
            // (模块文档已经这么承诺),所以让它落进下面的泛化转发分支
            // 走那条路径,不在内核这里重复处理、更不能整体关掉对话框——
            // 那样会把用户已经填好的表单内容(根目录/名称/描述)连同错误
            // 一起丢掉(代码评审 finding:Failed clone/create always
            // discards the dialog)。
            Message::ProjectCreate(msg) => {
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::ProjectCreate(m));
                };
                project_create::update(&mut self.project_create, msg, &client, &handle, emit);
            }
            Message::Files(files::Message::CopyPath(path, kind)) => {
                let _ = (path, kind); // main.rs 拦截处理写剪贴板,这里维持现状空分支
            }
            Message::Files(files::Message::OpenSearch(path, is_dir)) => {
                // 右键菜单"搜索":跨 `files::Message` 边界,由内核把它映射成
                // `search::Message::SearchOpen`。先关右键菜单(否则搜索弹窗
                // dismiss 一关,旧菜单又冒回来),作用域由 `is_dir` 决定——目录
                // 按目录递归搜,文件只搜单文件。
                self.files.close_context_menu();
                let scope = if is_dir {
                    search::Scope::Dir(path)
                } else {
                    search::Scope::File(path)
                };
                self.update(Message::Search(search::Message::SearchOpen(scope)));
            }
            Message::Files(files::Message::FileHistoryOpen(path)) => {
                // 右键"查看此文件历史":先收起右键菜单(同 OpenSearch 的既有
                // 约定),再解析出仓库相对路径、组出 `FileHistoryTarget`、
                // 异步跑一次 `build()`。
                self.files.close_context_menu();
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let repo_path = PathBuf::from(&project.path);
                let Ok(file_path) = path.strip_prefix(&repo_path).map(|p| p.to_path_buf()) else {
                    return;
                };
                let target = file_history::FileHistoryTarget {
                    project_id,
                    repo_path: repo_path.clone(),
                    file_path: file_path.clone(),
                };
                self.file_history = Some(file_history::State::new(target));
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                handle.spawn(async move {
                    let repo_path2 = repo_path.clone();
                    let file_path2 = file_path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        file_history::build(
                            &repo_path2,
                            &file_path2,
                            file_history::DEFAULT_MAX_COUNT,
                        )
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("加载失败: {e}")));
                    let _ = proxy.send_event(Message::FileHistory(
                        file_history::Message::SnapshotLoaded(repo_path, file_path, result),
                    ));
                });
            }
            Message::Files(files::Message::FileHistoryRollbackPrevious(path)) => {
                // 右键"回滚到上一版本":先收起右键菜单,再解析出仓库相对路径,
                // 算上一版本 Oid 并 `rollback_to` 还原(同 `FileHistoryOpen` 在
                // app 层接线的写法,但这里是"一键还原"而非"打开历史浮层")。
                self.files.close_context_menu();
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let repo_path = PathBuf::from(&project.path);
                let Ok(file_path) = path.strip_prefix(&repo_path).map(|p| p.to_path_buf()) else {
                    return;
                };
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                handle.spawn(async move {
                    let repo_path2 = repo_path.clone();
                    let file_path2 = file_path.clone();
                    let path2 = path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        file_history::previous_oid(&repo_path2, &file_path2).and_then(|opt| {
                            match opt {
                                Some(oid) => {
                                    file_history::rollback_to(&repo_path2, &file_path2, oid)
                                }
                                None => Err("该文件没有可回滚的历史版本".to_string()),
                            }
                        })
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("回滚任务失败: {e}")));
                    let _ = proxy.send_event(Message::Files(
                        files::Message::FileHistoryRollbackDone(project_id, path2, result),
                    ));
                });
            }
            Message::Files(
                msg @ (files::Message::StatusesRefreshed(project_id, ..)
                | files::Message::PasteDone(project_id, ..)
                | files::Message::OpDone { project_id, .. }
                | files::Message::GitInfoLoaded(project_id, ..)
                | files::Message::BranchSwitchDone(project_id, ..)
                | files::Message::GitInitDone(project_id, ..)
                | files::Message::FileDropDone(project_id, ..)),
            ) => self.files_project_message(project_id, msg),

            Message::Files(files::Message::TabReloadFromDisk(path)) => {
                // 预览 tab 右键"从磁盘重新加载":先收起菜单,再在两个预览面板里
                // 按路径找出对应 tab,调 `PreviewPane::bump_reload` 重建编辑器/
                // 推进 webview 的 `reload_nonce` 让 webview 重新 `load_url` 读盘
                // 最新内容(原生 editor 档重读磁盘、wry 档换 URL 重载,见
                // `preview.rs::PreviewPane::bump_reload` 文档)。路径同时命中两
                // 个面板的概率极低(同一项目),找到即止。
                self.files.close_context_menu();
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                if let Some(id) = ws.preview.find_existing_file_tab(&path) {
                    ws.preview.bump_reload(id);
                    return;
                }
                if let Some(id) = ws.project_preview.find_existing_file_tab(&path) {
                    ws.project_preview.bump_reload(id);
                }
            }

            Message::Files(files::Message::TabContextMenuCloseTab(_kind, path)) => {
                // 预览 tab 右键"关闭":先收起菜单,再在两个预览面板里按路径定位
                // 出对应 tab,调 `PreviewPane::close` 把它关掉(原生 editor 档释放
                // 编辑器、wry 档回收 webview,见 `preview.rs::PreviewPane::close`
                // 文档)。路径同时命中两个面板的概率极低(同一项目),找到即止。
                self.files.close_context_menu();
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                if let Some(idx) = ws.preview.find_existing_file_tab(&path) {
                    ws.preview.close(idx);
                    return;
                }
                if let Some(idx) = ws.project_preview.find_existing_file_tab(&path) {
                    ws.project_preview.close(idx);
                }
            }

            Message::Files(files::Message::ToolbarHover(target, hovered)) => {
                // 文件树工具行 icon 按钮的 hover:本面板不挂 App 的 hover 动画
                // 表,把进入/离开转发成 `HoverId` 由内核统一驱动动画进度。
                let id = match target {
                    files::FilesToolbarTarget::SearchSubmit => HoverId::FilesSearchSubmit,
                    files::FilesToolbarTarget::Dotfiles => HoverId::FilesDotfiles,
                    files::FilesToolbarTarget::BranchSwitch => HoverId::FilesBranchSwitch,
                };
                self.set_hover(id, hovered);
            }
            Message::Files(files::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            // 树内行被按下:只武装拖拽(供 main.rs 全局松开左键时收尾),
            // **不**立即执行任何点击语义(既不展开/折叠目录,也不打开文件)。
            // 两者都推迟到真正松开、且确认这其实只是一次单击(未越过拖拽
            // 确认阈值)时才在 `TreeDragEnd` 里补做——展开目录会让下方行
            // 布局位移,若按下就立即展开,静止不动的光标可能被 iced 判定成
            // 树内行被按下:只武装 `Pending`(见 `TreeDragPhase` 文档)——
            // `Pending` 期间完全没有任何反应,不挂 `on_move`、不展开目录、
            // 不打开文件。真正推进到 `Dragging`(越过距离+时长两道阈值)由
            // `maybe_confirm_tree_drag` 在每次 `CursorMoved` 时判断(main.rs
            // 调用),不在这里做。
            Message::Files(files::Message::TreeRowPress { path, is_dir }) => {
                let press_pos = self.last_cursor;
                if let Some(ws) = self.active_workspace_mut() {
                    ws.files
                        .arm_tree_drag(path, is_dir, press_pos, std::time::Instant::now());
                }
            }
            // 树内拖拽松开左键:main.rs 发这条。`confirmed` 就是"这场拖拽有
            // 没有走到 `Dragging` 阶段"——由 `maybe_confirm_tree_drag` 在
            // 越过阈值那一刻就已经推进过一次,这里直接读结果,不重新算
            // 距离/时长(2026-09 用户实测反馈带诊断日志实锤过纯距离阈值挡
            // 不住 trackpad 快速点按的真实位移,才改成阈值判断只在
            // `Pending → Dragging` 转换时做一次、结果落进状态机里的这个
            // 设计,见 `TreeDragPhase` 文档)。
            Message::Files(files::Message::TreeDragRelease) => {
                let confirmed = self
                    .active_workspace()
                    .is_some_and(|ws| ws.files.tree_drag_confirmed());
                self.update(Message::Files(files::Message::TreeDragEnd(confirmed)));
            }
            // 树行双击:目录复用 `Message::Toggle` 那条本地消息直接切换展开
            // 态(同点箭头效果),文件跨到 `PreviewOpenPath`——该面板本身
            // 不认识这条消息,见 `files::Message::TreeRowDoubleClick` 文档。
            Message::Files(files::Message::TreeRowDoubleClick { path, is_dir }) => {
                if is_dir {
                    self.update(Message::Files(files::Message::Toggle(path)));
                } else {
                    self.update(Message::PreviewOpenPath(path));
                }
            }
            #[cfg(target_os = "macos")]
            Message::Files(files::Message::BranchPickerOpen) => {
                // macOS:文件树 git 底栏的版本选择菜单直接弹原生 NSMenu(与
                // tab 组下拉/git log 面板同款,2026-09-28)——`files::update`
                // 的 app_state 够不到内核 `last_cursor`,故与
                // `GitLog(BranchPickerOpen)` 一样在内核拦截。选中经
                // `Message::Files(BranchSwitch)` 走既有 checkout 回路。iced
                // 弹层只在非 macOS 兜底平台保留。
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let items = crate::workspace::branch_picker_items(
                    ws.files.current_branch.as_deref(),
                    &ws.files.git_branches,
                    ws.files.git_dirty(),
                    files::Message::BranchSwitch,
                );
                if let Some(msg) = crate::chrome::native_menu::show(items, self.last_cursor) {
                    self.update(Message::Files(msg));
                }
            }
            Message::Files(files::Message::RequestSendToAgentTerminal {
                text,
                is_dir,
                relative,
            }) => {
                // 文件树右键"添加到 Agent 上下文":`files::update` 拼好模板文本
                // 后经 `emit` 送回内核,这里写进当前激活的 agent 终端输入框
                // (真正的 PTY 句柄只有内核有)。
                //
                // 先发起落库(异步),再写终端;落库失败不影响粘贴——上下文条会
                // 显示"已发送到终端,但未能记录到上下文列表"(见
                // `agent_context::apply`)。
                if let Some(project_id) = self.active_project_id {
                    let client = self.client.clone();
                    let handle = self.handle.clone();
                    let proxy = self.proxy.clone();
                    crate::extensions::agent_context::request_add(
                        project_id,
                        is_dir,
                        relative,
                        &client,
                        &handle,
                        move |m| {
                            let _ = proxy.send_event(Message::AgentContext(project_id, m));
                        },
                    );
                }
                self.term_paste(terminal::TermTarget::Shared, text);
            }
            Message::Files(msg) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Files(m));
                };
                let external_apps = self.external_apps.clone();
                let agent_terminal_visible = self.terminal_visible();
                let app_files = &mut self.files;
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                files::update(
                    &mut ws.files,
                    app_files,
                    msg,
                    project_id,
                    &handle,
                    emit,
                    &external_apps,
                    agent_terminal_visible,
                );
            }
            Message::Project(
                msg @ (project::Message::GitRefreshed(project_id, ..)
                | project::Message::NameRenamed(project_id, ..)
                | project::Message::DiskUsageLoaded(project_id, ..)
                | project::Message::ScaffoldStepStarted(project_id, ..)
                | project::Message::ScaffoldStepFinished(project_id, ..)
                | project::Message::TranscriptBackfillStarted(project_id, ..)
                | project::Message::TranscriptBackfillFinished(project_id, ..)
                | project::Message::SummaryBackfillProgress(project_id, ..)
                | project::Message::SummaryBackfillFailed(project_id, ..)),
            ) => {
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let current_name = project.name.clone();
                let repo_path = std::path::PathBuf::from(&project.path);
                // `NameRenamed(Ok(updated))` 要把顶栏项目页签等读的 `ws.project`
                // 缓存一并更新——这是这个面板第一次出现需要内核介入(而不是纯
                // 委托给 `project::update`)的消息。
                if let project::Message::NameRenamed(_, Ok(updated)) = &msg {
                    ws.project = Some(updated.clone());
                }
                // 补总结进度追到 Done 时,弹窗外面缓存的 `conversation_sessions`
                // (`spawn_conversations_refresh` 唯一写入点)不会自动感知
                // `session_summaries` 表的新增行——不重新拉一次,对话列表会一直
                // 显示"未总结"直到用户重开项目 tab 或触发别的回合结束刷新。
                let refresh_conversations = matches!(
                    &msg,
                    project::Message::SummaryBackfillProgress(
                        _,
                        completed,
                        total,
                        ..
                    ) if completed >= total
                );
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Project(m));
                };
                project::update(
                    &mut ws.project_panel,
                    msg,
                    project_id,
                    &current_name,
                    &repo_path,
                    &client,
                    &handle,
                    emit,
                );
                if refresh_conversations {
                    self.with_project(project_id, |ws, io| {
                        ws.spawn_conversations_refresh(io);
                    });
                }
            }
            Message::Project(project::Message::OpenLink(path)) => {
                // 项目链接打开的文件进 Project 面板右配对的预览(`ws.project_preview`),
                // 不冲进 Files 预览——两条预览各自独立,互相不打扰。
                self.update(Message::ProjectPreviewOpenPath(path));
            }
            Message::Project(project::Message::Pick(target)) => {
                // 文件/目录选择器依赖 macOS 主线程原生能力(rfd/NSOpenPanel 模态,
                // 见 main.rs `pick_file_or_dir`),必须由 main.rs 的 winit 事件循环
                // 里 `dispatch` 拦截同步执行。这里只用代理把这条消息回灌回事件循环
                // ——不能 `self.update(Message::ProjectLinkPick(..))` 直调:那是同步
                // 递归,只会命中 `App::update` 里那格 no-op,绝不会触发文件选择弹窗。
                let _ = self.proxy.send_event(Message::ProjectLinkPick(target));
            }
            Message::Project(project::Message::LinkContextMenu { target, index }) => {
                self.project_link_context_menu(target, index);
            }
            Message::Project(project::Message::LinkRemove { target, index }) => {
                // 删除来自行内右键菜单:落 `LinkRemove` 时把菜单浮层一并收起,
                // 然后委托 `project::update` 真正执行删除(含越界校验与保存失败
                // 回滚,见 `project.rs` 的 `Message::LinkRemove`)。不能
                // `self.update(同一条 LinkRemove)` 直调——那会命中本分支自身
                // 再次匹配 `LinkRemove`,无限递归爆栈。
                self.project_link_menu = None;
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let current_name = project.name.clone();
                let repo_path = std::path::PathBuf::from(&project.path);
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Project(m));
                };
                project::update(
                    &mut ws.project_panel,
                    project::Message::LinkRemove { target, index },
                    project_id,
                    &current_name,
                    &repo_path,
                    &client,
                    &handle,
                    emit,
                );
            }
            Message::Project(project::Message::DeleteProjectConfirm) => {
                self.project_delete_confirm();
            }
            Message::Project(project::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Project(project::Message::ToolbarHover(target, hovered)) => {
                // Project 面板头部"＋"按钮的 hover:本面板不挂 App 的 hover 动画表,
                // 把进入/离开转发成 `HoverId` 由内核统一驱动动画进度(同
                // `Message::Files(files::Message::ToolbarHover(..))` 的既有先例)。
                let id = match target {
                    project::ProjectToolbarTarget::Docs => HoverId::ProjectDocsAdd,
                    project::ProjectToolbarTarget::Remote => HoverId::ProjectRemoteAdd,
                    project::ProjectToolbarTarget::Memory => HoverId::ProjectMemoryAdd,
                };
                self.set_hover(id, hovered);
            }
            Message::Project(msg) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let current_name = project.name.clone();
                let repo_path = std::path::PathBuf::from(&project.path);
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Project(m));
                };
                project::update(
                    &mut ws.project_panel,
                    msg,
                    project_id,
                    &current_name,
                    &repo_path,
                    &client,
                    &handle,
                    emit,
                );
            }
            Message::Ssh(ssh::Message::TestConnectionResult(project_id, host_id, result)) => {
                self.ssh_test_connection_result(project_id, host_id, result)
            }
            Message::Ssh(ssh::Message::UnknownKeyDetected(
                project_id,
                host_id,
                fingerprint,
                key_bytes,
            )) => self.ssh_unknown_key_detected(project_id, host_id, fingerprint, key_bytes),
            Message::Ssh(ssh::Message::KeyChanged(project_id, host_id, fingerprint)) => {
                self.ssh_key_changed(project_id, host_id, fingerprint)
            }
            // 点"终端"按钮:与既有 `TestConnection`/其它同步交互消息不同,
            // 这个消息不走 `ssh::update`(它要新建一个 tab,需要 `&mut
            // Workspace` 整体,`ssh::update` 只拿得到 `&mut ws.ssh`)——
            // 拦截在通配 `Message::Ssh(msg)` 之前,直接调 `Workspace::
            // spawn_ssh_tab`。
            Message::Ssh(ssh::Message::OpenSshTab(host_id, ssh::SshTabKind::Terminal)) => {
                // 新 SSH 终端从一开始就用 SSH 面板自己的网格(不是共享终端的
                // 列数)——在闭包里再借 `self` 会与 `with_focused_project` 的
                // `&mut self` 冲突,先取到局变量。
                let ssh_cols = self.ssh_cols;
                let ssh_rows = self.ssh_rows;
                self.with_focused_project(|ws, io| {
                    // 已经开着这台主机的终端 tab 就直接切过去,不重新握手
                    // 连一遍(阶段 3 SFTP 决定"每个 tab 独立新建连接",但
                    // 终端 tab 本来就是"一台主机一条常驻连接",重复点
                    // "终端"图标应该是切换焦点而不是叠加新连接)。
                    let already_open = ws
                        .ssh_tabs
                        .iter()
                        .any(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()));
                    if already_open {
                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Terminal);
                    } else {
                        ws.ssh.record_reopen_after_trust(host_id.clone());
                        ws.spawn_ssh_tab(io, host_id, ssh_cols, ssh_rows);
                    }
                });
            }
            // Sftp 阶段 3:真实打开一个 SFTP tab(独立连接 + 命令通道)。
            Message::Ssh(ssh::Message::OpenSshTab(host_id, ssh::SshTabKind::Sftp)) => {
                self.with_focused_project(|ws, io| {
                    if ws.sftp_tabs.contains_key(&host_id) {
                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Sftp);
                    } else {
                        ws.spawn_sftp_tab(io, host_id.clone());
                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Sftp);
                    }
                });
            }
            Message::Ssh(ssh::Message::CloseSshTab(host_id, kind)) => {
                self.with_focused_project(|ws, io| match kind {
                    ssh::SshTabKind::Terminal => ws.close_ssh_tab(io, &host_id, kind),
                    ssh::SshTabKind::Sftp => {
                        ws.sftp_tabs.remove(&host_id);
                        if ws.ssh_active.as_ref().map(|(h, k)| (h.as_str(), *k))
                            == Some((host_id.as_str(), ssh::SshTabKind::Sftp))
                        {
                            ws.ssh_active = None; // 简化处理:关掉 SFTP tab 后不自动
                            // 切到其它 tab,和终端 tab 关闭后的
                            // "切到剩下第一个"逻辑不强行统一,
                            // 因为 ssh_tabs/sftp_tabs 是两个不同
                            // 集合,统一切换逻辑收益不大,YAGNI。
                        }
                    }
                });
            }
            Message::Ssh(ssh::Message::SelectSshTab(host_id, kind)) => {
                self.with_focused_project(|ws, _io| {
                    ws.select_ssh_tab(host_id, kind);
                    let mut widths: Vec<f32> = vec![tab_display_width("空白")];
                    widths.extend(ws.ssh_tabs.iter().map(|t| {
                        tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name))
                    }));
                    widths.extend(ws.sftp_tabs.keys().map(|host_id| {
                        tab_display_width(
                            &ws.ssh
                                .hosts()
                                .iter()
                                .find(|h| &h.id == host_id)
                                .map(|h| h.name.clone())
                                .unwrap_or_else(|| host_id.clone()),
                        )
                    }));
                    let target = match &ws.ssh_active {
                        None => 0,
                        Some((active_host, ssh::SshTabKind::Terminal)) => ws
                            .ssh_tabs
                            .iter()
                            .position(|t| {
                                t.info.id.strip_prefix("ssh:") == Some(active_host.as_str())
                            })
                            .map(|i| i + 1)
                            .unwrap_or(0),
                        Some((active_host, ssh::SshTabKind::Sftp)) => ws
                            .sftp_tabs
                            .keys()
                            .position(|h| h == active_host)
                            .map(|i| i + 1 + ws.ssh_tabs.len())
                            .unwrap_or(0),
                    };
                    ws.ssh_tab_first = tab_widget::tab_window_reveal(
                        &widths,
                        4.0,
                        byteui::theme::geometry::tab_bar_avail_px(),
                        ws.ssh_tab_first,
                        target,
                    );
                    ws.ssh_tab_overflow_anchor = None;
                });
            }
            // 点固定的"空白"占位 tab:它不对应 `ssh_tabs`/`sftp_tabs` 里
            // 任何一条记录,选中态就是 `ssh_active == None`。
            Message::Ssh(ssh::Message::SelectBlankTab) => {
                self.with_focused_project(|ws, _io| {
                    ws.ssh_active = None;
                    let mut widths: Vec<f32> = vec![tab_display_width("空白")];
                    widths.extend(ws.ssh_tabs.iter().map(|t| {
                        tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name))
                    }));
                    widths.extend(ws.sftp_tabs.keys().map(|host_id| {
                        tab_display_width(
                            &ws.ssh
                                .hosts()
                                .iter()
                                .find(|h| &h.id == host_id)
                                .map(|h| h.name.clone())
                                .unwrap_or_else(|| host_id.clone()),
                        )
                    }));
                    ws.ssh_tab_first = tab_widget::tab_window_reveal(
                        &widths,
                        4.0,
                        byteui::theme::geometry::tab_bar_avail_px(),
                        ws.ssh_tab_first,
                        0,
                    );
                    ws.ssh_tab_overflow_anchor = None;
                });
            }
            Message::Ssh(ssh::Message::TabOverflowToggle) => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = match self.active_workspace() {
                        Some(ws) => {
                            let mut entries: Vec<(usize, String, bool)> = Vec::new();
                            for (i, tab) in ws.ssh_tabs.iter().enumerate() {
                                let idx = i + 1;
                                let host_id = tab
                                    .info
                                    .id
                                    .strip_prefix("ssh:")
                                    .unwrap_or(&tab.info.id)
                                    .to_string();
                                entries.push((
                                    idx,
                                    tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
                                    ws.ssh_active.as_ref().is_some_and(|(h, k)| {
                                        h == &host_id && *k == ssh::SshTabKind::Terminal
                                    }),
                                ));
                            }
                            for (i, (host_id, _)) in ws.sftp_tabs.iter().enumerate() {
                                let idx = i + 1 + ws.ssh_tabs.len();
                                let label = ws
                                    .ssh
                                    .hosts()
                                    .iter()
                                    .find(|h| &h.id == host_id)
                                    .map(|h| h.name.clone())
                                    .unwrap_or_else(|| host_id.clone());
                                entries.push((
                                    idx,
                                    label,
                                    ws.ssh_active.as_ref()
                                        == Some(&(host_id.clone(), ssh::SshTabKind::Sftp)),
                                ));
                            }
                            tab_widget::tab_overflow_items(&entries, |idx| {
                                ssh_tab_overflow_select_message(ws, idx)
                            })
                        }
                        None => Vec::new(),
                    };
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.ssh_tab_overflow_anchor = if ws.ssh_tab_overflow_anchor.is_some() {
                            None
                        } else {
                            Some(last_cursor)
                        };
                    });
                }
            }
            Message::Ssh(ssh::Message::TabOverflowDismiss) => {
                self.with_focused_project(|ws, _io| {
                    ws.ssh_tab_overflow_anchor = None;
                });
            }
            // SFTP tab 内部交互:按 host_id 路由到 `sftp::route`,真正的
            // 处理逻辑在那边(sftp::Message 有 7+ 个变体,内容又都操作
            // `ws.sftp_tabs`,摊平会让这里的大 match 更难读)。
            Message::Ssh(ssh::Message::Sftp(ssh::sftp::Message::ContextMenuOpen {
                host_id,
                is_local,
                path,
            })) => {
                #[cfg(target_os = "macos")]
                {
                    // 先落选中态(同 `sftp::route` 的 ContextMenuOpen 分支),
                    // 再同步弹原生菜单,结果经 `Ssh(Sftp(msg))` 回路由。
                    self.with_focused_project(|ws, io| {
                        ssh::sftp::route(
                            ws,
                            io,
                            ssh::sftp::Message::ContextMenuOpen {
                                host_id: host_id.clone(),
                                is_local,
                                path: path.clone(),
                            },
                        );
                    });
                    let (x, y) = self.files.last_right_click();
                    let items = ssh::sftp::context_menu_items(&host_id, is_local);
                    if let Some(msg) = crate::chrome::native_menu::show(items, (x, y)) {
                        self.update(Message::Ssh(ssh::Message::Sftp(msg)));
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.with_focused_project(|ws, io| {
                        ssh::sftp::route(
                            ws,
                            io,
                            ssh::sftp::Message::ContextMenuOpen {
                                host_id,
                                is_local,
                                path,
                            },
                        );
                    });
                }
            }
            Message::Ssh(ssh::Message::Sftp(msg)) => {
                self.with_focused_project(|ws, io| {
                    ssh::sftp::route(ws, io, msg);
                });
            }
            // 终端连接失败:先做内核层面的清理(pending/ssh_out_pending
            // 两处暂存——这次连接没能走到 `TabAttached`,不清理会一直占着
            // 这两个 map 的位置),再转给 `ssh::update` 落卡片状态(同
            // `TestConnectionResult` 的路由口径,带显式 project_id,套用
            // 一模一样的 `with_project` 外壳)。
            Message::Ssh(ssh::Message::TerminalConnectFailed(project_id, host_id, tab_id, err)) => {
                self.ssh_terminal_connect_failed(project_id, host_id, tab_id, err)
            }
            Message::Ssh(ssh::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Ssh(msg) => {
                if let ssh::Message::Hover(id, h) = msg {
                    self.set_hover(id, h);
                    return;
                }
                self.with_focused_project(|ws, io| {
                    let Some(project) = ws.project.as_ref() else {
                        return;
                    };
                    let project_id = project.id;
                    let repo_path = PathBuf::from(&project.path);
                    let handle = io.handle.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Ssh(m));
                    };
                    ssh::update(&mut ws.ssh, msg, project_id, &repo_path, &handle, emit);
                });
            }
            Message::Footbar(msg) => {
                // App 级 + 纯展示,不带 project_id,不需要
                // `with_project`/`with_focused_project`,直接更新。
                footbar::update(&mut self.footbar, msg);
            }
            Message::ZoomIn => {
                byteui::theme::icon_size::zoom_by(UI_ZOOM_STEP);
                byteui::theme::icon_size::persist_scale(&crate::theme::ui_scale_path());
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            Message::ZoomOut => {
                byteui::theme::icon_size::zoom_by(1.0 / UI_ZOOM_STEP);
                byteui::theme::icon_size::persist_scale(&crate::theme::ui_scale_path());
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            Message::ZoomReset => {
                byteui::theme::icon_size::reset_scale(&crate::theme::ui_scale_path());
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            Message::PreviewSelectionMenuOpen {
                project_id,
                panel,
                tab_id,
                path,
                x,
                y,
                range,
                selected_text,
            } => {
                // 闭包外:换算窗口坐标(webview 本地 + webview 原点),判断当前
                // 是否有可见/激活的 agent 终端(无则菜单项置灰,不报错)。
                let side = self.shell_state().layout.rail_layout.side_of(panel);
                let (x0, y0, _, _) = crate::webview_geometry::preview_content_bounds_for(
                    side,
                    self.window_size.0,
                    self.window_size.1,
                    &self.shell_state(),
                );
                let menu = PreviewSelectionContextMenu {
                    project_id,
                    x: x0 + x,
                    y: y0 + y,
                    panel,
                    tab_id,
                    path,
                    range,
                    selected_text,
                    agent_terminal_visible: self.terminal_visible(),
                };
                #[cfg(target_os = "macos")]
                {
                    // 原生菜单阻塞弹出,mac 不存状态,弹完即取返回值。
                    let items = files::preview_selection_context_menu_items(&menu);
                    let pos = (menu.x, menu.y);
                    if let Some(msg) =
                        crate::chrome::native_menu::show_align_no_icon_left(items, pos)
                    {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.preview_context_menu = Some(menu);
                }
            }
            Message::SendSelectionToAgent {
                project_id,
                path,
                range,
                selected_text,
            } => {
                self.preview_context_menu = None;
                // 非 mac 的 iced 弹层不阻塞输入,用户可能在菜单开着期间切到
                // 别的项目标签再点"发送给 Agent"——`project_id` 是建菜单时
                // 捕获的发起项目,与"当前激活"的项目不再一致就直接放弃,不
                // 能把内容送错项目(同 `term_input` 里"看不见的地方不能
                // 敲字"的既有原则)。
                if self.active_project_id != Some(project_id) {
                    return;
                }
                let Some(ws) = self.active_workspace() else {
                    return;
                };
                let Some(tree) = ws.files.file_tree.as_ref() else {
                    return;
                };
                let relative = crate::project::path_string(
                    crate::project::PathKind::Relative,
                    &path,
                    tree.root(),
                );
                let text = files::selection_reference_text(&relative, range, &selected_text);
                self.term_paste(terminal::TermTarget::Shared, text);
            }
            Message::PreviewSelectionMenuClose => {
                self.preview_context_menu = None;
            }
            Message::SettingsOpen => {
                self.settings = Some(settings::State::load(self.daemon_error.as_deref()));
            }
            Message::Settings(msg) => {
                // 主题切换要在设置 update(它会 set_scheme 改全局配色)之后,把
                // 所有已开预览 webview 按新主题重载——flyfish 的 theme 是 URL
                // 参数(见 preview::flyfish_url),不重载不会跟着变。两个预览
                // 面板(Files/Project)各推进各自 wry tab 的 reload_nonce。
                let theme_changed = matches!(msg, settings::Message::ThemeSelected(_));
                // 停止/重新启动 dozerd 的结果要顺带更新 `daemon_error`——
                // 这是"daemon 连不上"的整程序共享状态(`app/view.rs`/
                // `term/terminal.rs` 已经在读),不新建 UI 组件(spec「复用
                // App.daemon_error」)。跟 `theme_changed` 一样,要在
                // `msg` 被 move 进 `settings::update` 之前取值。
                let stop_succeeded = matches!(msg, settings::Message::AdvancedStopResult(Ok(())));
                let restart_succeeded =
                    matches!(msg, settings::Message::AdvancedRestartResult(Ok(())));
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Settings(m));
                };
                settings::update(&mut self.settings, msg, &client, &handle, emit);
                if stop_succeeded {
                    self.daemon_error = Some("dozerd 已停止,部分功能不可用".to_string());
                }
                if restart_succeeded {
                    self.daemon_error = None;
                }
                if theme_changed && let Some(ws) = self.active_workspace_mut() {
                    ws.preview.reload_all_webviews_for_theme();
                    ws.project_preview.reload_all_webviews_for_theme();
                }
            }
            // WebViewFocused 只在 main.rs 的 dispatch 里设 pending_focus,
            // App::update 无需处理。
            Message::WebViewFocused => {}
            // 鼠标在子 webview 上松开(见 `WebViewMouseUp` 文档):一并结束页签
            // 拖拽,避免"松开还能继续拖"。
            Message::WebViewMouseUp => self.end_tab_drag(),
            // ⌘F 页内查找(见两条消息的文档):原生副作用(设 first
            // responder / 调 WKWebView `findString:`)都在 main.rs 的
            // `dispatch` 里对 webview 句柄执行,`App::update` 无需处理。
            Message::WebViewFindFocus(_) | Message::WebViewFindNative(_) => {}
        }
    }

    pub(crate) fn project_select(&mut self, id: i64) {
        // 从新增项目菜单点选时顺带关掉菜单(菜单本来就该在选中后消失);
        // 从其它入口(首页最近项目卡片)调用时这里恒为 false,no-op。
        self.project_add_menu_open = false;
        // 切项目不再通知 daemon:"活跃项目"是 GUI 侧的概念了(P2a
        // Task 1-3 删掉了 SetActiveProject)。
        //
        // 这个项目已经开着页签(`Loaded` 或还没促成的 `Stub`)时,点最近
        // 项目卡片就只是"切到那个页签",走与点页签完全相同的非破坏性
        // 路径——绝不能杀掉任何已有页签的会话(设计文档 §2)。
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            self.adopt_panel_layout(id);
            self.maximized = None;
            self.current_page = AppPage::Workspace;
            self.ensure_loaded(id);
            // 清放大态改变了终端 pane 的像素尺寸,网格必须跟着重算:
            // `terminal_grid_state` 把 `maximized` 算进去,不重算的话
            // PTY 会一直停在放大时的 cols/rows,直到某个无关的几何事件
            // 偶然触发一次重算(最终审查 Required Fix #2)。
            self.sync_terminal_grid();
            self.persist_open_projects();
            return;
        }
        // 还没开着:作为**新页签**打开(与顶栏"＋"同一条 `ProjectTabOpened`
        // 落地路径),而不是把当前页签的内容换掉——多页签下"点一张最近
        // 项目卡片"的直觉是"再开一个",不是"把手上这个换掉"。
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let recent = client.list_projects().await.unwrap_or_default();
            let opened = recent.iter().find(|p| p.id == id).cloned();
            let _ = proxy.send_event(Message::ProjectTabOpened(opened, recent, false));
        });
    }

    pub(crate) fn project_tab_opened(
        &mut self,
        project: Option<ProjectInfo>,
        recent: Vec<ProjectInfo>,
        skip_git_init: bool,
    ) {
        self.recent_projects = recent.clone();
        // `None` = 这次打开失败(daemon 不通/回 `Reply::Error`)。硬性
        // 要求:失败绝不能落进任何 `Workspace`,否则会留下"有界面、没
        // 归属项目"的破状态,用户一点 tab 栏的"＋"就 panic
        // (`spawn_new_tab` 的 expect)。失败文案挂到 App 级的
        // `daemon_error` 上——它不依赖任何 `Workspace` 存在,一个项目
        // 都没打开时空态视图也画得出来(Required Fix #1)。
        let Some(project) = project else {
            tracing::warn!("打开项目页签失败,页签集合保持不变");
            self.daemon_error = Some("打开项目失败,请确认 dozerd 正常后重试".to_string());
            self.with_focused_project(move |ws, _io| {
                ws.recent_projects = recent;
            });
            return;
        };
        self.daemon_error = None;
        // 放大态是外壳态,换页签后留着只会挡住新页签的界面。
        self.maximized = None;
        let id = project.id;
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            self.adopt_panel_layout(id);
            // 这个项目已经开着页签了:只前台化,绝不改写它的内容——
            // 那会把这个页签既有的终端全关掉、文件树对话列表全清空重来。
            self.current_page = AppPage::Workspace;
            self.ensure_loaded(id);
            // 前台化同样是"换到另一个项目"(focus_project_tab 是唯一一条
            // 换项目的路),Git Log 面板开着时要补同步,理由同
            // `ProjectTabSwitch`。
            if self.git_log_panel_active() {
                self.sync_git_log_to_active_project();
            }
            self.with_focused_project(move |ws, _io| {
                ws.recent_projects = recent;
            });
            self.sync_terminal_grid(); // 清放大态后重算网格,理由见 `ProjectSelect`
            self.persist_open_projects();
            return;
        }
        let io = self.shell_io();
        let repo_path = PathBuf::from(&project.path);
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.recent_projects = recent;
        ws.adopt_project(&io, project);
        self.projects
            .insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
        self.project_order.push(id);
        self.active_project_id = Some(id);
        // 换成新项目的面板布局(它自己没存过就退化成默认)。
        self.adopt_panel_layout(id);
        self.current_page = AppPage::Workspace;
        // 全新页签同样过一遍 Git Log 同步(默认布局没开它时是廉价 no-op)。
        if self.git_log_panel_active() {
            self.sync_git_log_to_active_project();
        }
        self.sync_terminal_grid(); // 同上
        self.persist_open_projects();
        // 新开的项目 tab(区别于"已开着、只是前台化"那条 `focus_project_tab`
        // 早退分支):静默跑一次 ensure(README/.dozer/git/agent 历史),不
        // 展示结果(见 project_scaffold 设计"打开即 ensure"一节)。
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Project(m));
        };
        project::spawn_scaffold_run(repo_path, client, &handle, emit, skip_git_init);
    }

    pub(crate) fn project_tab_switch(&mut self, id: i64) {
        // 切页签只有两件事:改 `active_project_id`、必要时促成 `Stub`。
        // 没有任何内容改写,因此后台项目的终端/预览/审阅原样留着,切
        // 回来还是刚才那副样子。
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if !focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            return;
        }
        self.adopt_panel_layout(id);
        self.maximized = None;
        self.current_page = AppPage::Workspace;
        self.ensure_loaded(id);
        // Git Log 面板已经开着的话,提交图缓存是 `App` 级的、不随项目
        // 页签走(见 `sync_git_log_to_active_project` 文档),不补这一
        // 下切页签会让提交图停在上一个项目,跟同一面板里已经按新项目
        // 刷新的 worktree 速览条对不上。两栏都查:GitLog 被拖到右栏后
        // `left_view` 不再是它,只查左栏会漏(用户实测:右栏 Git Log 切
        // 页签后一直显示前一项目的提交)。
        if self.git_log_panel_active() {
            self.sync_git_log_to_active_project();
        }
        // 清放大态后必须重算终端网格。`PaneResized` 那条分支只在**窗口
        // 几何变化**时触发,清 `maximized` 不会自己走到那里;而
        // `terminal_grid_state` 把 `maximized` 算进公式,不重算的话
        // "在项目 A 放大终端 → 切到 B"会让 A 的 PTY 停在放大时的
        // cols/rows(最终审查 Required Fix #2)。重算是幂等的:算出来
        // 与当前 `cols/rows` 相同时 `PaneResized` 的去重会原地返回。
        self.sync_terminal_grid();
        self.persist_open_projects();
        // 按下项目页签＝选中＋准备被拖走(同终端/预览页签)。
        if let Some(idx) = self.project_order.iter().position(|p| *p == id) {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Project,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    pub(crate) fn project_tab_close(&mut self, id: i64) {
        // 关掉当前页签前先把它的面板布局原样存下(焦点还在它身上,
        // `stash` 会记进 `id` 那份),以后重开还能恢复。
        self.stash_active_panel_layout();
        let io = self.shell_io();
        let Some(slot) = take_project_tab(
            &mut self.projects,
            &mut self.project_order,
            &mut self.active_project_id,
            id,
        ) else {
            return;
        };
        if let WorkspaceSlot::Loaded(mut ws) = slot {
            // 关页签 = 结束该项目下所有会话(abort 转发任务 + kill
            // daemon 侧会话)。不 kill 的话会话会继续在 daemon 上跑,
            // 还会被下次 bootstrap 恢复出来。
            ws.close_all_tabs_for_switch(&io);
        }
        self.maximized = None;
        // 焦点被 `take_project_tab` 挪到了邻居页签上,而那个邻居可能还
        // 是个懒加载 `Stub`——`view()` 走的是只读的 `active_workspace()`,
        // 它**不促成** `Stub`,于是界面会画成"未打开任何项目",尽管顶栏
        // 那个页签明明高亮着。必须在这里显式促成(最终审查 Required
        // Fix #3)。
        if let Some(next) = self.active_project_id {
            // 焦点被挪到了邻居页签,把它的面板布局换上来。
            self.adopt_panel_layout(next);
            self.ensure_loaded(next);
            // 邻居页签的布局里开着 Git Log 的话,提交图缓存还停在刚被关
            // 掉的那个项目上——同 `ProjectTabSwitch` 的补同步理由。
            if self.git_log_panel_active() {
                self.sync_git_log_to_active_project();
            }
        }
        self.sync_terminal_grid(); // 清放大态后重算网格,理由同 `ProjectTabSwitch`
        self.persist_open_projects();
    }

    /// "删除项目"确认弹窗的"删除"按钮触发,由 `Message::Project(project::
    /// Message::DeleteProjectConfirm)` 拦截调用(见该分支注释)。这个操作
    /// 一定作用在当前聚焦的项目上——删除按钮本来就在那个项目自己的面板
    /// 里,不存在"删除一个没打开的项目"这回事。
    pub(crate) fn project_delete_confirm(&mut self) {
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(scope) = ws.project_panel.delete_pending.take() else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = PathBuf::from(&project.path);
        // 关 tab 必须在发起删除请求之前——删除一旦成功,这个项目在
        // dozerd/磁盘上都可能已经不存在了,`Workspace` 不该继续留着。
        self.project_tab_close(project_id);
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let on_done = move |errors: Vec<String>| {
            let _ = proxy.send_event(Message::ProjectDeleteDone(errors));
        };
        project::delete::spawn_delete_project(
            project_id, repo_path, scope, client, &handle, on_done,
        );
    }

    pub(crate) fn project_slot_loaded(&mut self, id: i64, payload: RestorePayload) {
        let Some(restore) = payload.take() else {
            return; // 信封已被取走(理论上不会发生),没有素材可落地
        };
        // 只在槽位仍是那份"加载中"占位时落地。两种落空情形:
        // - 页签在促成完成前被用户关掉了(槽位已不存在);
        // - 槽位已经被别的路径换成了真正的内容(比如
        //   `ProjectTabOpened` 的 `adopt_project`)。
        // 两种情形下这份素材都没人要了,但它已经 attach 上了该项目在
        // daemon 上的存活会话——直接 drop 只是断开事件流,daemon 侧
        // 会话仍在跑,会变成"没有任何页签持有、却还占着 PTY"的野会话。
        // 所以按关页签的语义结束掉它们(`ProjectTabClose` 同款处理)。
        // 落地的同时把占位那份 `allowed_files` 句柄接过来:main.rs 的
        // webview 池只在 `active_project_id` **变化**时才清空,它看不见
        // "同一个项目换了一份 `Workspace` 对象"。促成窗口期里用户点开
        // 的文件预览已经建出一个 id 0 的 webview,其 `dozer://` 协议
        // 闭包捕获的是**占位那一个** `Arc`;新 `Workspace` 若另起一个
        // `Arc`,`restore_preview_state` 重开的 id 0 会被
        // `sync_webview_pool` 认成"这个 id 已经有 webview 了"而只调
        // `load_url`,于是文件请求走的还是旧 `Arc` 的白名单 → 对不上
        // → 空白预览。这与 Required Fix #3 是同一个失效模式,只是触发
        // 点从"切项目"变成"促成换对象"。共用同一个 `Arc` 即可,而且
        // 不损失已经建好的 webview(比清空池更省一次导航)。
        let inherited = match self.projects.get(&id) {
            Some(WorkspaceSlot::Loaded(cur)) if cur.loading => Some(cur.allowed_files()),
            _ => None,
        };
        let landed = inherited.is_some();
        if !landed {
            let client = self.client.clone();
            let ids: Vec<String> = restore
                .sessions
                .iter()
                .map(|(info, _, _)| info.id.clone())
                .collect();
            let task = self.handle.spawn(async move {
                for sid in ids {
                    if let Err(e) = client.kill(&sid).await {
                        tracing::warn!("丢弃过期促成结果时结束会话失败: {e}");
                    }
                }
            });
            if let Ok(mut pending) = self.pending_exit_tasks.lock() {
                pending.push(task);
            }
            return;
        }
        let io = self.shell_io();
        let mut ws = Workspace::from_restore(&io, *restore, inherited);
        // 重挂出来的会话,终端模型是按 `DEFAULT_COLS`×`DEFAULT_ROWS`
        // 建的,得按当前窗口几何纠正一次。这里**不能**指望
        // `sync_terminal_grid`:它算出来的网格与 `self.cols/rows` 相同
        // 时 `PaneResized` 会原地返回(去重),于是这份新装配的
        // `Workspace` 会一直停在 80×24。直接对它自己 resize 一次——
        // 共享与 SSH 两个 pane 各按自己跟踪的网格分别纠正。
        ws.resize_all(&io, io.cols, io.rows, self.ssh_cols, self.ssh_rows);
        self.projects
            .insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
        // 恢复出来的工作区变为 Loaded:补拉一次上下文列表,重启后条上不是空的。
        self.agent_context_refresh(id);
    }

    pub(crate) fn project_fs_changed(
        &mut self,
        project_id: ProjectId,
        changes: git_watch::FsChanges,
    ) {
        // 工作区类变更:文件树 + 打开的 webview 预览即时跟进。`notify` 递上
        // 的是具体变更路径 `changes.paths`,文件树按"受影响即相关"整棵从盘重
        // 读已缓存目录(`reload_tree_from_disk`,只重读已展开/缓存过的层,开销
        // 小),预览则只重载路径命中的 webview tab。
        if changes.relevance == Some(git_watch::Relevance::Workdir)
            || changes.relevance == Some(git_watch::Relevance::GitRefs)
        {
            self.with_project(project_id, |ws, _io| {
                ws.files.reload_tree_from_disk();
                ws.preview.reload_webviews_for(&changes.paths);
                ws.project_preview.reload_webviews_for(&changes.paths);
            });
        }
        self.with_project(project_id, |ws, io| {
            let Some(project) = &ws.project else { return };
            let repo_path = PathBuf::from(&project.path);
            spawn_project_git_refresh(project_id, repo_path.clone(), io);
            spawn_disk_usage_refresh(project_id, repo_path, io);
        });
        // 只有 `.git` 引用类变化(分支切换/外部提交/其他 worktree
        // 提交)才值得重建 Git Log 快照——纯工作区文件编辑不影响
        // 提交历史,重算是纯浪费。`git_log_cache` 是 `App` 级、不是
        // 按项目分的(见 `sync_git_log_to_active_project`),所以这里
        // 必须先核实这条事件本来就是"当前聚焦项目"发出的
        // (`project_id == self.active_project_id`)——否则后台项目
        // 的引用变化会拿"缓存路径恰好等于前台项目路径"这个巧合当
        // 通行证,把前台正打开的详情/选中态平白清掉,而其实什么都
        // 没变。项目 id 匹配之外再核一次路径,双保险防状态漂移。
        if changes.relevance == Some(git_watch::Relevance::GitRefs)
            && self.active_project_id == Some(project_id)
            && let Some(repo_path) = self.git_log.cache_repo_path().map(|p| p.to_path_buf())
            && self
                .active_workspace()
                .and_then(|ws| ws.active_project_path())
                .as_deref()
                == Some(repo_path.as_path())
        {
            // 引用变化只是要"内容不变、重新拉一遍",窗口大小维持原样——
            // 用 `cache_max_count()`(读当前缓存的 max_count),不是"加载
            // 更多"专用、会 `+LOAD_MORE_STEP` 的 `next_load_more_count()`。
            let max = self.git_log.cache_max_count();
            let handle = self.handle.clone();
            let proxy = self.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::GitLog(m));
            };
            git_log::request_refresh(&mut self.git_log, repo_path, max, &handle, emit);
        }
    }

    pub(crate) fn database_test_connection_result(
        &mut self,
        project_id: i64,
        source_id: String,
        result: Result<(), String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::TestConnectionResult(project_id, source_id, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    pub(crate) fn database_browse_result(
        &mut self,
        project_id: i64,
        tab_id: usize,
        run_seq: u64,
        result: Result<database::BrowsePage, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::BrowseResult(project_id, tab_id, run_seq, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    pub(crate) fn database_query_result(
        &mut self,
        project_id: i64,
        tab_id: usize,
        run_seq: u64,
        result: Result<database::QueryOutcome, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::QueryResult(project_id, tab_id, run_seq, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    pub(crate) fn database_tables_loaded(
        &mut self,
        project_id: i64,
        source_id: String,
        result: Result<Vec<database::TableRef>, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::TablesLoaded(project_id, source_id, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    pub(crate) fn database_columns_loaded(
        &mut self,
        project_id: i64,
        source_id: String,
        schema: Option<String>,
        table: String,
        result: Result<Vec<database::ColumnInfo>, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::ColumnsLoaded {
                project_id,
                source_id,
                schema,
                table,
                result,
            },
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    pub(crate) fn database_message(&mut self, msg: database::Message) {
        // 四个动作都可能来自数据源树 header 行的右键菜单——菜单本体没有
        // "点了就自动收起"的行为(popup 盖在 dismiss 遮罩之上,点菜单项本身
        // 吃不到遮罩的点击),落地时顺手收掉,不来自菜单时该字段本就是
        // `None`,无副作用。
        if matches!(
            msg,
            database::Message::TestConnection(_)
                | database::Message::EditSourceStart(_)
                | database::Message::DeleteSourceRequest(_)
                | database::Message::SchemaRefresh(_)
        ) {
            self.database_source_menu = None;
        }
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            msg,
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    pub(crate) fn ssh_test_connection_result(
        &mut self,
        project_id: i64,
        host_id: String,
        result: Result<(), String>,
    ) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::TestConnectionResult(project_id, host_id, result),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    pub(crate) fn ssh_unknown_key_detected(
        &mut self,
        project_id: i64,
        host_id: String,
        fingerprint: String,
        key_bytes: Vec<u8>,
    ) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::UnknownKeyDetected(project_id, host_id, fingerprint, key_bytes),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    pub(crate) fn ssh_key_changed(
        &mut self,
        project_id: i64,
        host_id: String,
        fingerprint: String,
    ) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::KeyChanged(project_id, host_id, fingerprint),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    pub(crate) fn ssh_terminal_connect_failed(
        &mut self,
        project_id: i64,
        host_id: String,
        tab_id: usize,
        err: String,
    ) {
        self.with_project(project_id, move |ws, io| {
            ws.pending.remove(&tab_id);
            ws.ssh_out_pending.remove(&tab_id);
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::TerminalConnectFailed(project_id, host_id, tab_id, err),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    /// 指派任务给某个 agent 种类,纯记录,不触发任何执行。异步确认经
    /// `Mutated` 刷新列表——与其它写操作同一条乐观更新链路。
    pub(crate) fn todo_assign_agent(&mut self, idx: usize, agent: dozer_core::protocol::AgentKind) {
        let Some(id) = self
            .active_workspace()
            .and_then(|ws| ws.todo.items().get(idx))
            .map(|item| item.id)
        else {
            return;
        };
        self.with_focused_project(|ws, _io| {
            ws.todo.close_dispatch_popup();
        });
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            let res = client
                .assign_todo_agent(id, agent)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = proxy.send_event(Message::Todo(todo::Message::Mutated(res)));
        });
    }

    /// 打开任务详情弹窗:先本地记下 `idx`(弹窗定位/后续"处理"要用),再
    /// 异步拉 `GetTodoDetail`。RPC 结果经专门的 `Message::TodoDetailLoaded`
    /// 落地——`todo::Message::Mutated` 那条通用刷新链路只刷 `items`/
    /// `categories`,不携带回合数据,不能复用。
    pub(crate) fn todo_detail_open(&mut self, idx: usize) {
        let Some(id) = self
            .active_workspace()
            .and_then(|ws| ws.todo.items().get(idx))
            .map(|item| item.id)
        else {
            return;
        };
        self.with_focused_project(|ws, _io| {
            ws.todo.open_detail(idx, Vec::new());
        });
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            if let Ok((_, turns)) = client.get_todo_detail(id).await {
                let _ = proxy.send_event(Message::TodoDetailLoaded(idx, turns));
            }
        });
    }

    /// 详情弹窗"处理"按钮:乐观插入已经在 `todo::update`(`DetailReplySubmit`
    /// 分支)做过,这里只管发 `ProcessTodoNow` RPC 并在结果回来后用服务端
    /// 权威回合列表刷新。耗时可能到 10 分钟,走 `handle.spawn` 不阻塞 UI。
    pub(crate) fn todo_detail_process(&mut self) {
        let Some((idx, id, reply_text)) = self.active_workspace().and_then(|ws| {
            let idx = ws.todo.detail_open_idx()?;
            let id = ws.todo.items().get(idx)?.id;
            Some((idx, id, ws.todo.last_reply_text()))
        }) else {
            return;
        };
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            let _ = client.process_todo_now(id, reply_text.as_deref()).await;
            if let Ok((_, turns)) = client.get_todo_detail(id).await {
                let _ = proxy.send_event(Message::TodoDetailLoaded(idx, turns));
            }
        });
    }

    pub(crate) fn todo_message(&mut self, msg: todo::Message) {
        // 新增任务框高度拖拽:只在 app 层接管,置 `dragging_row`,后续
        // `CursorMoved` → `RowDrag` 由 `update` 统一换算高度写回
        // `ws.todo`(见 `RowDrag` 的 `TodoAddGrow` 分支)。这条不到
        // `todo::update`(那里有 no-op arm 保持 match 穷尽)。
        if let todo::Message::AddResizeStart = msg {
            self.dragging_row = Some(RowDivider::TodoAddGrow);
            return;
        }
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let last_cursor = self.last_cursor;
        // 异步结果/写确认透过 `proxy` 重发回主循环,回调里会借用 `self`
        // 的 client/handle ——闭包捕获是 move 出来的副本,行得通(同
        // `Message::Search` 分支的既有手法)。
        let emit = move |m: todo::Message| {
            let _ = proxy.send_event(Message::Todo(m));
        };
        self.with_focused_project(move |ws, _io| {
            // 点日历按钮时的光标逻辑坐标,作为窗口级 overlay 的弹出锚点——
            // 先记下再交给 `todo::update` 展开(它只管 `calendar_open`/`calendar_view`)。
            if matches!(msg, todo::Message::CalendarOpen(_)) {
                ws.todo.set_calendar_anchor(last_cursor);
            }
            // 点"指派"按钮时的光标逻辑坐标,作为派发选择层 overlay 的弹出锚点。
            if matches!(msg, todo::Message::DispatchOpen(_)) {
                ws.todo.set_dispatch_anchor(last_cursor);
            }
            // 点卡片左下"状态"按钮的光标逻辑坐标,作为状态下拉选择层 overlay
            // 的弹出锚点。`StatusOpen` 自身交给 `todo::update` 展开(它只改
            // `status_open`)。
            if matches!(msg, todo::Message::StatusOpen(_)) {
                ws.todo.set_status_anchor(last_cursor);
            }
            // 点搜索框左前"状态"segment 按钮时的光标逻辑坐标,作为搜索框状态
            // 筛选浮层 overlay 的弹出锚点。`StatusFilterOpen` 自身交给
            // `todo::update` 展开(它只改 `status_filter_open`)。
            if matches!(msg, todo::Message::StatusFilterOpen) {
                ws.todo.set_status_filter_anchor(last_cursor);
            }
            todo::update(&mut ws.todo, msg, project_id, &client, &handle, emit);
        });
    }

    pub(crate) fn browser_bookmarks_loaded(
        &mut self,
        pid: Option<i64>,
        bookmarks: Vec<BookmarkInfo>,
    ) {
        if pid.is_none() {
            // 首页全局浏览器(或无项目工作区)的收藏列表刷新。
            let handle = self.handle.clone();
            let client = self.client.clone();
            let proxy = self.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::HomeBrowser(m));
            };
            browser::update(
                &mut self.home_browser,
                browser::Message::BookmarksLoaded(None, bookmarks),
                None,
                &client,
                &handle,
                emit,
            );
            return;
        }
        self.with_project(pid.unwrap(), move |ws, io| {
            let pid = pid.unwrap();
            let handle = io.handle.clone();
            let client = io.client.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(
                &mut ws.browser,
                browser::Message::BookmarksLoaded(Some(pid), bookmarks),
                Some(pid),
                &client,
                &handle,
                emit,
            );
        });
    }

    pub(crate) fn browser_bookmarks_mutated(&mut self, pid: Option<i64>, res: Result<(), String>) {
        if pid.is_none() {
            // 首页全局浏览器(或无项目工作区)的添加/删除结果回调。
            let handle = self.handle.clone();
            let client = self.client.clone();
            let proxy = self.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::HomeBrowser(m));
            };
            browser::update(
                &mut self.home_browser,
                browser::Message::BookmarksMutated(None, res),
                None,
                &client,
                &handle,
                emit,
            );
            return;
        }
        self.with_project(pid.unwrap(), move |ws, io| {
            let pid = pid.unwrap();
            let handle = io.handle.clone();
            let client = io.client.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(
                &mut ws.browser,
                browser::Message::BookmarksMutated(Some(pid), res),
                Some(pid),
                &client,
                &handle,
                emit,
            );
        });
    }

    pub(crate) fn browser_message(&mut self, msg: browser::Message) {
        // 按下浏览器页签＝选中＋准备被拖走(`SelectTab` 在
        // `browser::update` 里真正选中为 `active`,这里按它记下拖起源)。
        let was_select = matches!(msg, browser::Message::SelectTab(_));
        self.with_focused_project(|ws, io| {
            let project_id = ws.project.as_ref().map(|p| p.id);
            let client = io.client.clone();
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(&mut ws.browser, msg, project_id, &client, &handle, emit);
        });
        if was_select
            && let Some(ws) = self.active_workspace()
            && ws.browser.active_tab_idx() < ws.browser.tab_count()
        {
            let active = ws.browser.active_tab_idx();
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Browser,
                source: active,
                press_pos: self.last_cursor,
            });
        }
    }

    pub(crate) fn term_input(&mut self, target: terminal::TermTarget, bytes: Vec<u8>) {
        // 终端不在屏上时丢弃按键(不报错、不写 PTY):否则用户在读
        // 对话审阅时敲的回车/方向键会静默提交给隐藏在后面的 agent
        // 会话(Fix round 2 #3)。
        let visible = match target {
            terminal::TermTarget::Shared => self.terminal_visible(),
            terminal::TermTarget::SshPanel => self.ssh_terminal_visible(), // Task 12 新增
        };
        if !visible {
            return;
        }
        self.with_focused_project(|ws, io| {
            match target {
                terminal::TermTarget::Shared => {
                    // 键入即回底 + 清选区：正在回看历史时一敲键盘，视口跳回
                    // 实时输出（常规终端语义），再把字节写给 daemon。
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.scroll_to_bottom();
                        tab.model.selection_clear();
                    }
                    ws.send_input(io, bytes);
                }
                terminal::TermTarget::SshPanel => {
                    if let Some(tab) = ws.ssh_active_tab_mut() {
                        tab.model.scroll_to_bottom();
                        tab.model.selection_clear();
                    }
                    ws.ssh_send_input(io, bytes);
                }
            }
        });
    }

    pub(crate) fn term_output(&mut self, project_id: ProjectId, tab_id: usize, bytes: Vec<u8>) {
        self.with_project(project_id, |ws, io| {
            let Some(tab) = ws.tab_by_id_mut(tab_id) else {
                return;
            };
            // 实时输出可能含设备查询（DSR/DA 等），应答必须写回 PTY
            // ——atuin/claude 等 TUI 依赖它（此前丢弃导致探测超时）。
            tab.ingest_osc(&bytes);
            let responses = tab.model.feed(&bytes);
            if responses.is_empty() || !tab.alive {
                return;
            }
            match &tab.backend {
                TabBackend::Daemon => {
                    let client = io.client.clone();
                    let id = tab.info.id.clone();
                    io.handle.spawn(async move {
                        if let Err(e) = client.write(&id, &responses).await {
                            tracing::warn!("回写终端查询应答失败: {e}");
                        }
                    });
                }
                TabBackend::Ssh { out } => {
                    let _ = out.send(SshOut::Data(responses));
                }
            }
        });
    }

    pub(crate) fn term_paste(&mut self, target: terminal::TermTarget, text: String) {
        // 同 TermInput 的可见性闸门(Fix round 3):⌘V 粘贴走同一条
        // PTY 写入路径,粘贴内容若含换行还会在看不见的会话里直接
        // 执行,比单个按键更危险,必须同样拦截。
        let visible = match target {
            terminal::TermTarget::Shared => self.terminal_visible(),
            terminal::TermTarget::SshPanel => self.ssh_terminal_visible(),
        };
        if !visible {
            return;
        }
        self.with_focused_project(move |ws, io| {
            let bracketed = match target {
                terminal::TermTarget::Shared => {
                    ws.tabs.get(ws.active).map(|t| t.model.bracketed_paste())
                }
                terminal::TermTarget::SshPanel => ws
                    .ssh_tabs
                    .iter()
                    .find(|t| {
                        ws.ssh_active.as_ref().is_some_and(|(h, _)| {
                            t.info.id.strip_prefix("ssh:") == Some(h.as_str())
                        })
                    })
                    .map(|t| t.model.bracketed_paste()),
            };
            let Some(bracketed) = bracketed else {
                return;
            };
            match target {
                terminal::TermTarget::Shared => {
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.scroll_to_bottom();
                    }
                }
                terminal::TermTarget::SshPanel => {
                    if let Some(tab) = ws.ssh_active_tab_mut() {
                        tab.model.scroll_to_bottom();
                    }
                }
            }
            let bytes = if bracketed {
                let mut b = b"\x1b[200~".to_vec();
                b.extend_from_slice(text.as_bytes());
                b.extend_from_slice(b"\x1b[201~");
                b
            } else {
                text.into_bytes()
            };
            match target {
                terminal::TermTarget::Shared => ws.send_input(io, bytes),
                terminal::TermTarget::SshPanel => ws.ssh_send_input(io, bytes),
            }
        });
    }

    pub(crate) fn select_tab(&mut self, idx: usize) {
        // 按下页签＝选中＋准备被拖走:选中仍是唯一的语义,但顺带记下
        // "这一页签正被按住",随后鼠标划过其它页签时 `on_move` 触发
        // `TabDragMove` 完成换位;松开时 main.rs `TabDragEnd` 收尾。
        // 只应该被 tab 栏本身的按钮调用——见 `Message::SelectTab` 文档。
        self.select_tab_no_drag(idx);
        if let Some(ws) = self.active_workspace()
            && idx < ws.tabs.len()
        {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Terminal,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    /// `select_tab` 去掉"武装拖拽状态机"那部分,给非 tab 栏的调用方
    /// (Agent 面板右侧卡片列表)用——见 `Message::SelectTabNoDrag` 文档。
    pub(crate) fn select_tab_no_drag(&mut self, idx: usize) {
        // `with_focused_project` 的闭包里借的是 `ws`,拿不到 `self`——真实
        // 可用宽度得在借用开始前算好(同 `preview_select_tab` 那批调用方的
        // 既有先例)。
        let avail_px = self.terminal_tab_bar_avail_px();
        self.with_focused_project(move |ws, _io| {
            if idx < ws.tabs.len() {
                ws.active = idx;
                // 从 tab 栏的 V 下拉里选中某一项:选中后把主条滚入可见窗口
                // (若该项仍横向可见则不受影响,见 `tab_window_reveal`),并收起
                // 下拉——避免"选中了却看不见在哪"。下拉列的是组内全部 tab,
                // 高亮常驻在可见宽度内时不会多跳一行。
                let widths: Vec<f32> = ws
                    .tabs
                    .iter()
                    .map(|t| tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name)))
                    .collect();
                ws.term_tab_first =
                    tab_widget::tab_window_reveal(&widths, 4.0, avail_px, ws.term_tab_first, idx);
                ws.term_tab_overflow_anchor = None;
            }
        });
        // `term_ime_preedit` 是 `App` 上唯一一份、不按 tab 分的组字预览态
        // (见该字段文档),只在真正 `Ime::Commit`/组字取消时才清空——切
        // tab 不会清。`TermTarget::Shared` 这道"该不该显示"闸门只判断
        // "键盘现在归不归共享终端条",不区分具体哪个 tab,于是新切过去的
        // tab 会直接"继承"上一个 tab 还没提交完的组字预览文字/候选词
        // (2026-08-21 用户实测反馈:切 tab 后串台,选完字对方也不会真的
        // 收到那些字——因为提交字节确实是发给切换后的新 tab 的 PTY,只是
        // 预览视觉是借来的)。切 tab 时无条件清掉(即使 idx 越界导致上面
        // 没真的切,清掉一份陈旧组字预览也没有副作用),避免这份陈旧状态
        // 被新激活的 tab 误当成自己的组字预览渲染出来。
        self.term_ime_preedit = None;
    }

    pub(crate) fn pane_resized(&mut self, cols: u16, rows: u16, ssh_cols: u16, ssh_rows: u16) {
        if cols == 0 || rows == 0 {
            return;
        }
        // 共享与 SSH 两个网格各自带独立去重:任一真变了都要往 dev 文件里
        // propagate,不能因为共享网格没动就跳掉 SSH 网格的同步。
        let shared_changed = (cols, rows) != (self.cols, self.rows);
        let ssh_changed = (ssh_cols, ssh_rows) != (self.ssh_cols, self.ssh_rows);
        if !shared_changed && !ssh_changed {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.ssh_cols = ssh_cols;
        self.ssh_rows = ssh_rows;
        let io = self.shell_io();
        // 终端网格是窗口级的:并行打开的每个项目各有一套终端 tab,但它们
        // 共用同一批 pane。只改当前项目的话,切回后台项目会看到一个停在
        // 旧网格、和 pane 对不上的画面,直到用户偶然再拖一次窗口才纠正——
        // 所以这里对所有已加载项目一起改(`Stub` 还没有任何 tab,促成时
        // 自然按当时的 `io.cols/rows`)。共享/SSH 各自带独立网格下发。
        for slot in self.projects.values_mut() {
            if let WorkspaceSlot::Loaded(ws) = slot {
                ws.resize_all(&io, cols, rows, ssh_cols, ssh_rows);
            }
        }
    }

    pub(crate) fn panel_select(&mut self, kind: PanelKind) {
        let side = self.shell_layout.rail_layout.side_of(kind);
        // 武装拖拽态:按住图标＝准备拖(同 `TabDrag` 的"按下即武装"手法)。
        // 同栏重排 / 跨栏移动都是靠渲染层挂在图标上的 `on_move` 驱动
        // (`Message::RailDragMove`),`MouseMotion` 期间逐帧上报；这里只记下
        // "从哪栏的哪个位置开始拖"。`RailLayout` 的不变式(sanitize 已保证
        // 10 个面板不重不漏分到两栏)确保 `kind` 一定能在 `side_of` 返回的
        // 那一栏里被 `position` 找到。
        let source_index = self
            .shell_layout
            .rail_layout
            .side(side)
            .iter()
            .position(|&k| k == kind)
            .expect("kind 应该在 side_of 返回的那一侧里,sanitize 已保证不变式");
        self.rail_drag = Some(rail::RailDrag {
            source_side: side,
            source_index,
            origin_index: source_index,
            pending_cross_side: None,
            press_pos: self.last_cursor,
        });
        // 点当前已激活的图标:退回未选中并收起对应面板区;但若对侧面板区
        // 也已收起,当前侧就是最后一个还开着的 zone,不能关(两侧对称)。
        let switched = match side {
            Side::Left => {
                if self.left_view == kind {
                    if !self.right_collapsed {
                        self.left_collapsed = !self.left_collapsed;
                    }
                    false
                } else {
                    self.left_view = kind;
                    self.left_collapsed = false;
                    true
                }
            }
            Side::Right => {
                if self.right_view == kind {
                    if !self.left_collapsed {
                        self.right_collapsed = !self.right_collapsed;
                    }
                    false
                } else {
                    self.right_view = kind;
                    self.right_collapsed = false;
                    true
                }
            }
        };
        // 面板专属的"切入时动作"。原左栏处理器把 GitLog/Todo/
        // Database/Project/Ssh 的触发放在 if/else 之后的无条件
        // `if self.left_view == PanelKind::X` 里——收起/展开当前激活的特殊
        // 面板也会跑一遍;原右栏处理器把 Usage 放在 else(真正
        // 切换)分支里——只有切换时才触发。为保持逐像素零差异,左侧面板恒
        // 触发、右侧面板仅在真正切换时触发(默认布局下它们恰好按这个分侧;
        // Stage 4 拖拽换栏后这里再按 `rail_layout.side_of` 重新对齐各面板
        // 的触发语义)。
        let fire = match side {
            Side::Left => true,
            Side::Right => switched,
        };
        if fire {
            match kind {
                PanelKind::GitLog => self.sync_git_log_to_active_project(),
                PanelKind::Todo => {
                    if let Some(project_id) = self.active_project_id {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        let emit = move |m: todo::Message| {
                            let _ = proxy.send_event(Message::Todo(m));
                        };
                        let emit_todos = emit.clone();
                        todo::request_todos_refresh(project_id, &client, &handle, emit_todos);
                        todo::request_categories_refresh(project_id, &client, &handle, emit);
                    }
                }
                PanelKind::Database => self.with_focused_project(|ws, _io| {
                    if let Some(project) = ws.project.as_ref() {
                        database::reload_from_disk(
                            &mut ws.database,
                            std::path::Path::new(&project.path),
                        );
                    }
                }),
                PanelKind::Project => {
                    self.ensure_project_readme_and_reveal();
                    if let Some(project_id) = self.active_project_id {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        let emit = move |m: project::Message| {
                            let _ = proxy.send_event(Message::Project(m));
                        };
                        project::request_memories_refresh(project_id, &client, &handle, emit);
                    }
                }
                PanelKind::Ssh => self.with_focused_project(|ws, _io| {
                    if let Some(project) = ws.project.as_ref() {
                        ssh::reload_from_disk(&mut ws.ssh, std::path::Path::new(&project.path));
                    }
                }),
                PanelKind::Usage => self.with_focused_project(|ws, io| {
                    ws.usage.set_loading(true);
                    ws.spawn_usage_refresh(io);
                }),
                // 代码健康度面板切入时只读上次落盘结果，不自动扫描（spec：
                // 手动触发，与 Usage 的"打开即自动扫"是明确的行为差异）。
                PanelKind::CodeHealth => self.with_focused_project(|ws, io| {
                    ws.spawn_codehealth_load(io);
                }),
                // 会话列表原本只在项目打开时和回合结束时刷新,切进这个面板时
                // 没有任何补救手段——离开一段时间再切回来看到的还是上次的
                // 快照。补一次切入即刷新,同 `Usage` 面板的既有口径。
                PanelKind::Conversations => self.with_focused_project(|ws, io| {
                    ws.spawn_conversations_refresh(io);
                }),
                PanelKind::Files | PanelKind::Web | PanelKind::Agent => {}
            }
        }
        // 图标栏点击一律退出放大态。放大态浮层不拦图标栏上的点击
        // (遮罩两侧垫的是无交互 Space,点击穿到下层图标按钮),所以
        // "放大左侧 → 点文件夹图标收起左侧"是可达的:不清 `maximized`
        // 就会留下一个空的金色描边浮层,只能点变暗区才能脱身
        // (Fix round 2 #2)。切换本侧显示什么内容时,放大态本也不该
        // 存活,无条件清最简单也最不容易出意外。
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    /// 文件预览右上角按钮:翻转文件树列表子栏的展开/收起。只改一个布尔
    /// (`dims.files_tree_collapsed`),不动 `files_split` 比例(展开时按原比例
    /// 恢复)。收起态下文件树列表不渲染、预览拿满整个配对宽度。与
    /// `panel_select` 一样退出放大态并落盘/重算网格。
    pub(crate) fn toggle_files_tree_collapse(&mut self) {
        self.dims.files_tree_collapsed = !self.dims.files_tree_collapsed;
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    /// 该面板当前是否偏离了默认栏——8 个有内部两栏布局的面板据此决定
    /// 渲染顺序要不要反转。这个 Stage 结束时 `RailLayout` 只可能是
    /// `default()`,所以这个函数在正常运行时恒返回 `false`;它的分支
    /// 靠单元测试直接构造非默认 `RailLayout` 来触发验证,不依赖 GUI
    /// 能不能拖拽出这个状态(Stage 4 才有拖拽)。
    pub(crate) fn panel_mirrored(&self, kind: PanelKind) -> bool {
        rail::panel_mirrored_in(&self.shell_layout.rail_layout, kind)
    }

    /// 文件树列表子栏当前是否被收起(文件预览右上角按钮切换)。暴露只读
    /// 的 `dims.files_tree_collapsed` 给 workspace 层渲染收起按钮时用,
    /// `dims` 字段本身保持模块私有。
    pub(crate) fn files_tree_collapsed(&self) -> bool {
        self.dims.files_tree_collapsed
    }

    /// 七个两栏面板(Project/Todo/Database/Ssh/Agent/Conversations/Usage)的
    /// 列表列当前是否被收起。语义同 `files_tree_collapsed`:列表不渲染、内容
    /// 拿满配对宽度,split 比例保留(展开时按原宽度恢复)。
    pub(crate) fn list_collapsed(&self, kind: PanelKind) -> bool {
        match kind {
            PanelKind::Project => self.dims.project_list_collapsed,
            PanelKind::Todo => self.dims.todo_list_collapsed,
            PanelKind::Database => self.dims.database_list_collapsed,
            PanelKind::Ssh => self.dims.ssh_list_collapsed,
            PanelKind::Agent => self.dims.agent_list_collapsed,
            PanelKind::Conversations => self.dims.conversations_list_collapsed,
            PanelKind::Usage => self.dims.usage_list_collapsed,
            _ => false,
        }
    }

    /// 翻转某两栏面板列表列的展开/收起(`Message::TogglePanelListCollapse` 的
    /// 处理)。只改对应布尔、不动 split 比例,并像 `panel_select` 一样退出
    /// 放大态 + 落盘/重算网格。非两栏面板(`Files` 走独立的
    /// `files_tree_collapsed`,其余单/两栏面板无此能力)直接忽略。
    pub(crate) fn toggle_panel_list_collapse(&mut self, kind: PanelKind) {
        let flag = match kind {
            PanelKind::Project => &mut self.dims.project_list_collapsed,
            PanelKind::Todo => &mut self.dims.todo_list_collapsed,
            PanelKind::Database => &mut self.dims.database_list_collapsed,
            PanelKind::Ssh => &mut self.dims.ssh_list_collapsed,
            PanelKind::Agent => &mut self.dims.agent_list_collapsed,
            PanelKind::Conversations => &mut self.dims.conversations_list_collapsed,
            PanelKind::Usage => &mut self.dims.usage_list_collapsed,
            _ => return,
        };
        *flag = !*flag;
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    /// 内容侧"收起/展开列表列"按钮:用户点击某面板内容区的按钮翻转其列表列
    /// 显隐。语义完全对齐文件预览的 `FileTreeCollapse` 按钮(见 `preview_pane_for`),
    /// 只是图标按该面板当前所在栏(左/右)与收起态四选一、tooltip 由调用方
    /// 给静态文案。泛型 `M` 兼容顶层 `Message` 与各扩展模块的本地 `Message`。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        kind: PanelKind,
        collapsed: bool,
        hover_id: HoverId,
        tooltip_collapse: &'a str,
        tooltip_expand: &'a str,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
        let side = self.shell_layout.rail_layout.side_of(kind);
        let (icon, tooltip) = match (side, collapsed) {
            (Side::Left, false) => (icons::IconKind::PanelLeftClose, tooltip_collapse),
            (Side::Left, true) => (icons::IconKind::PanelLeftOpen, tooltip_expand),
            (Side::Right, false) => (icons::IconKind::PanelRightClose, tooltip_collapse),
            (Side::Right, true) => (icons::IconKind::PanelRightOpen, tooltip_expand),
        };
        icons::icon_button_entry(
            icon,
            byteui::theme::icon_size::row(),
            false,
            false,
            self.hover_progress(hover_id),
            false,
            byteui::theme::icon_size::row() + 6.0,
            true,
            on_select,
            on_hover,
            tooltip,
        )
    }

    pub(crate) fn top_bar_home(&mut self) {
        self.current_page = AppPage::Home;
        self.home_recents_loaded = false;
        self.home_left_view = homespace::HomeLeftView::default();
        self.home_project_pages = 1;
        self.home_project_search.clear();
        self.home_project_search_draft.clear();
        self.home_project_search_focused = false;
        self.home_right_view = homespace::HomeRightView::default();
        let projects: Vec<ProjectInfo> = self.recent_projects.iter().take(5).cloned().collect();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let (files, convs) = load_home_recents(&client, &projects).await;
            let _ = proxy.send_event(Message::HomeRecentsLoaded(files, convs));
        });
    }

    /// T3:把一个文件 tab 的画像(`profile_file`,有界采样 + 病态长首行探测)
    /// 丢到 `spawn_blocking` 后台线程跑,UI 线程不做任何文件 I/O。完成后经
    /// [`Message::PreviewProfiled`] 回灌,按 `project_id` 路由 + `generation`
    /// 闸门丢弃过期结果。`EventLoopProxy` 发送失败(App 已退出)= 任务自然结束。
    fn spawn_preview_profile(
        ws: &Workspace,
        io: &crate::workspace::ShellIo,
        panel: PanelKind,
        tab_id: usize,
        generation: u64,
        path: &std::path::Path,
    ) {
        let Some(project_id) = ws.project_id() else {
            return;
        };
        let path = path.to_path_buf();
        let proxy = io.proxy.clone();
        io.handle.spawn_blocking(move || {
            let result = crate::preview::profile_file(&path).map_err(|e| e.to_string());
            let _ = proxy.send_event(Message::PreviewProfiled(
                project_id, panel, tab_id, generation, result,
            ));
        });
    }

    /// 打开文件预览:统一走 `PreviewPane::open_path` 按路由得到 editor host /
    /// JSON host / tabular / webview 后端。老 iced 原生编辑器候选的
    /// `insert_loading_tab` + 后台 `read_and_build_native_editor` 异步构造
    /// 路径已随 Phase D 退役。
    pub(crate) fn preview_open_path(&mut self, path: PathBuf) {
        self.preview_open_path_at(path, None);
    }

    /// 上下文条的消息路由。`project_id` 来自信封,异步应答即使在用户切到别的
    /// 项目之后到达,也只写回发起它的那个 `Workspace`。
    pub(crate) fn agent_context_message(
        &mut self,
        project_id: i64,
        msg: crate::extensions::agent_context::Message,
    ) {
        use crate::extensions::agent_context as ctx;
        let Some(root) = loaded_workspace_mut(&mut self.projects, project_id)
            .and_then(|ws| ws.project.as_ref().map(|p| PathBuf::from(&p.path)))
        else {
            return;
        };
        match msg {
            ctx::Message::Open(id) => {
                let Some(entry) = loaded_workspace_mut(&mut self.projects, project_id)
                    .and_then(|ws| ws.agent_context.entry(id).cloned())
                else {
                    return;
                };
                let path = root.join(&entry.info.entity_ref);
                if entry.info.entity_kind == "dir" {
                    // 目录:在文件树里选中它(不展开/收起,避免误 toggle)。
                    if let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) {
                        ws.files.set_tree_selected(path);
                    }
                } else {
                    self.preview_open_path(path);
                }
            }
            ctx::Message::OpenHistory(item) => {
                let filter = item.and_then(|id| {
                    loaded_workspace_mut(&mut self.projects, project_id)
                        .and_then(|ws| ws.agent_context.entry(id))
                        .map(|e| e.info.entity_ref.clone())
                });
                self.open_edit_history(project_id, filter);
            }
            other => {
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::AgentContext(project_id, m));
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                ctx::update(
                    &mut ws.agent_context,
                    project_id,
                    root,
                    other,
                    &client,
                    &handle,
                    emit,
                );
            }
        }
    }

    /// 拉取某项目的上下文列表(切入项目时调用;工作区未加载则空操作)。
    pub(crate) fn agent_context_refresh(&mut self, project_id: i64) {
        let Some(root) = loaded_workspace_mut(&mut self.projects, project_id)
            .and_then(|ws| ws.project.as_ref().map(|p| PathBuf::from(&p.path)))
        else {
            return;
        };
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        crate::extensions::agent_context::request_refresh(
            project_id,
            root,
            &client,
            &handle,
            move |m| {
                let _ = proxy.send_event(Message::AgentContext(project_id, m));
            },
        );
    }

    /// 打开修改历史弹窗(独立原生窗口由 `window_events` 的 sync 按
    /// `edit_history.is_some()` 开出)。`filter`:上下文项的 `entity_ref`,
    /// `None` = 全部。
    pub(crate) fn open_edit_history(&mut self, project_id: i64, filter: Option<String>) {
        self.edit_history = Some(crate::extensions::edit_history::State::new(
            project_id,
            filter.clone(),
            self.terminal_visible(),
        ));
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        crate::extensions::edit_history::request_load(
            project_id,
            filter,
            &client,
            &handle,
            move |m| {
                let _ = proxy.send_event(Message::EditHistory(m));
            },
        );
    }

    pub(crate) fn edit_history_message(&mut self, msg: crate::extensions::edit_history::Message) {
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        crate::extensions::edit_history::update(
            &mut self.edit_history,
            msg,
            &client,
            &handle,
            move |m| {
                let _ = proxy.send_event(Message::EditHistory(m));
            },
        );
    }

    pub(crate) fn preview_open_path_at(&mut self, path: PathBuf, target_line: Option<usize>) {
        // 同 `preview_select_tab`:`preview_tab_bar_avail_px` 要 `&self`,
        // 得在 `with_focused_project` 的 `&mut self` 借用之前先算好。
        let avail_w = self.preview_tab_bar_avail_px(PanelKind::Files);
        if self.active_project_id.is_none() {
            return;
        }
        self.with_focused_project(move |ws, io| {
            if !path.is_file() {
                ws.preview_error = Some(format!("文件不存在或不可读: {}", path.display()));
                return;
            }
            ws.preview_error = None;
            ws.files.set_tree_selected(path.clone());
            ws.allowed_files
                .lock()
                .expect("allowed_files 锁")
                .insert(path.clone());

            if let Some(idx) = ws.preview.find_existing_file_tab(&path) {
                // 同一文件已开则切过去,不重复开/重复读盘。
                ws.preview.select(idx);
                // 命中的是只恢复了壳的 tab:物化它。
                if let Some(tab_id) = ws.preview.tabs().get(idx).map(|t| t.id)
                    && ws.preview.is_suspended(tab_id)
                {
                    ws.load_preview_tab(PanelKind::Files, tab_id, io);
                }
                if let Some(line) = target_line
                    && let Some(tab) = ws.preview.tabs_mut().get_mut(idx)
                {
                    tab.pending_jump_line = Some(line);
                }
            } else {
                // T3:UI 线程不读盘——建临时 route 壳(Profiling),画像丢后台。
                let (id, generation) = ws.preview.open_path_provisional(path.clone());
                // CodeMirror tab 在 host `ready` 后由 `EditorWebviewEvent`
                // 排队 reveal;webview/表格类仍无法跳转光标,target_line 忽略。
                if let Some(line) = target_line
                    && let Some(tab) = ws.preview.tabs_mut().iter_mut().find(|t| t.id == id)
                    && tab.uses_editor_host()
                {
                    tab.pending_jump_line = Some(line);
                }
                if let Some(generation) = generation {
                    Self::spawn_preview_profile(ws, io, PanelKind::Files, id, generation, &path);
                }
            }
            ws.spawn_pending_tabular_loads(PanelKind::Files, io);
            // 新 tab 落在末尾(复用已开的文件则落在该文件原来的位置)——
            // 用跟 `preview_select_tab` 同一套 `tab_window_reveal`,把窗口
            // 起点钳到"包含这个新激活 tab"的位置,而不是无脑滚回最左
            // (此前 `= 0` 的写法:tab 一多,新开的文件反而被滚出可见区,
            // 2026-09-14 用户反馈"新打开文件时 tab 应该跳转到对应位置")。
            let active = ws.preview.active_idx();
            let widths: Vec<f32> = ws
                .preview
                .tabs()
                .iter()
                .map(|t| preview_tab_display_width(&t.title))
                .collect();
            ws.preview_tab_first =
                tab_widget::tab_window_reveal(&widths, 4.0, avail_w, ws.preview_tab_first, active);
            ws.spawn_preview_state_save(io);
            ws.spawn_preview_context_push(io);
        });
    }

    /// 代码健康度面板"点击函数跳转"入口：确保 Files 面板可见，再打开该
    /// 文件并跳到目标行。`panel_select` 在已选中同一面板时会触发"收起/
    /// 展开"的 toggle 副作用（见 `panel_select` 文档），这里先判断避免
    /// 误触。发现项的路径是相对项目根的规范化路径，这里解析成绝对路径。
    pub(crate) fn code_health_open_location(&mut self, path: PathBuf, line: usize) {
        if self.right_view != PanelKind::Files {
            self.panel_select(PanelKind::Files);
        }
        let resolved = if path.is_absolute() {
            path
        } else {
            self.active_workspace()
                .and_then(|ws| ws.active_project_path())
                .map(|root| root.join(&path))
                .unwrap_or(path)
        };
        self.preview_open_path_at(resolved, Some(line));
    }

    /// 代码健康度"交给 Agent 分析"：按 finding ID 从当前报告解析发现，生成
    /// 只读诊断上下文，送入当前 agent 会话输入区（不自动发送，用户仍需主动
    /// 回车）。报告更新后找不到 ID 时降级为无操作（不伪造发现）。
    pub(crate) fn code_health_analyze_finding(&mut self, id: String) {
        // 先只读解析出诊断文本（不可变借用，离开块即释放）。
        let text = {
            let Some(ws) = self.active_workspace() else {
                return;
            };
            let Some(report) = ws.codehealth.report() else {
                return;
            };
            let Some(finding) = report.findings.iter().find(|f| f.id == id) else {
                return;
            };
            let change = ws.codehealth.diff().map(|d| {
                if d.new.iter().any(|x| x.id == id) {
                    dozer_codehealth::FindingChange::New
                } else if d.worsened.iter().any(|x| x.id == id) {
                    dozer_codehealth::FindingChange::Worsened
                } else if d.improved.iter().any(|x| x.id == id) {
                    dozer_codehealth::FindingChange::Improved
                } else {
                    dozer_codehealth::FindingChange::Persisting
                }
            });
            let reasons: Vec<String> = ws
                .codehealth
                .hotspots()
                .iter()
                .find(|h| h.finding.id == id)
                .map(codehealth::view_model::hotspot_reasons)
                .unwrap_or_default();
            let scope = codehealth::view_model::scope_summary(&ws.codehealth);
            codehealth::view_model::analyze_finding_text(finding, change, &reasons, scope.as_ref())
        };

        // 送入当前会话输入区（无换行 → 不自动发送）。
        let bytes = text.into_bytes();
        self.with_focused_project(move |ws, io| {
            ws.send_input(io, bytes);
        });
    }

    pub(crate) fn preview_select_tab(&mut self, idx: usize) {
        let arming = self
            .active_workspace()
            .map(|ws| idx < ws.preview.tabs().len())
            .unwrap_or(false);
        // 得在借用 `ws` 之前算好——`preview_tab_bar_avail_px` 要 `&self`,
        // 跟下面 `with_focused_project` 内部的 `&mut self` 借用冲突,必须
        // 提前拿到这个值再原样传进闭包(同渲染侧 `preview_pane_for` 用的
        // 是同一个真实宽度,窗口起点算法两边才不会算出不一致的结果)。
        let avail_w = self.preview_tab_bar_avail_px(PanelKind::Files);
        self.with_focused_project(|ws, io| {
            ws.preview.select(idx);
            // Phase C Task 5:切到只恢复了壳的 tab 时物化它(其余 tab 保持 Suspended)。
            if let Some(tab_id) = ws.preview.tabs().get(idx).map(|t| t.id)
                && ws.preview.is_suspended(tab_id)
            {
                ws.load_preview_tab(PanelKind::Files, tab_id, io);
            }
            if idx < ws.preview.tabs().len() {
                let widths: Vec<f32> = ws
                    .preview
                    .tabs()
                    .iter()
                    .map(|t| preview_tab_display_width(&t.title))
                    .collect();
                ws.preview_tab_first =
                    tab_widget::tab_window_reveal(&widths, 4.0, avail_w, ws.preview_tab_first, idx);
                ws.preview_tab_overflow_anchor = None;
            }
            ws.spawn_preview_state_save(io);
            // 切换 tab:上下文立即变,flush 到 dozerd 不等防抖。
            ws.flush_preview_context_push(io);
        });
        if arming {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Preview,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    /// Project 面板右配对预览打开文件:写入 `ws.project_preview`(独立的
    /// `PreviewPane`),完全不碰 Files 预览的 `ws.preview`/`ws.files`。
    /// tab 是项目链接点开产生的会话期状态,不持久化、也不向 daemon 推上下文,
    /// 避免与 Files 预览那份持久化 `preview_state` 互相覆盖。
    pub(crate) fn project_preview_open_path(&mut self, path: PathBuf) {
        let avail_w = self.preview_tab_bar_avail_px(PanelKind::Project);
        if self.active_project_id.is_none() {
            return;
        }
        self.with_focused_project(move |ws, io| {
            if !path.is_file() {
                ws.project_preview_error = Some(format!("文件不存在或不可读: {}", path.display()));
                return;
            }
            ws.project_preview_error = None;
            ws.project_panel.set_selected_link(path.clone());
            ws.allowed_files
                .lock()
                .expect("allowed_files 锁")
                .insert(path.clone());

            if let Some(idx) = ws.project_preview.find_existing_file_tab(&path) {
                ws.project_preview.select(idx);
            } else {
                // T3:同 Files 预览——建临时 route 壳,画像丢后台。
                let (id, generation) = ws.project_preview.open_path_provisional(path.clone());
                if let Some(generation) = generation {
                    Self::spawn_preview_profile(ws, io, PanelKind::Project, id, generation, &path);
                }
            }
            ws.spawn_pending_tabular_loads(PanelKind::Project, io);
            // 新 tab 落在末尾(或复用已开文件原位),用 `tab_window_reveal`
            // 钳出包含它的窗口起点,不再无脑滚回最左(同 Files 预览)。
            let active = ws.project_preview.active_idx();
            let widths: Vec<f32> = ws
                .project_preview
                .tabs()
                .iter()
                .map(|t| preview_tab_display_width(&t.title))
                .collect();
            ws.project_preview_tab_first = tab_widget::tab_window_reveal(
                &widths,
                4.0,
                avail_w,
                ws.project_preview_tab_first,
                active,
            );
        });
    }

    /// 项目信息面板切入时调用:确保项目根目录有一份 `README.md`(没有就按
    /// 项目名 + 描述生成,已有则原样保留),然后**一律**在右侧配套预览窗打
    /// 开这份 README(首次切进来就让它展示项目文档)。
    ///
    /// 打开/生成依赖同一份"可读"保障——README 创建失败或不可读时静默返回,
    /// 绝不拿一个空文件去占预览,也绝不让面板切入失败。
    pub(crate) fn ensure_project_readme_and_reveal(&mut self) {
        let Some(project) = self.active_workspace().and_then(|ws| ws.project.clone()) else {
            return;
        };
        let Some(readme) =
            ensure_project_readme(&std::path::PathBuf::from(&project.path), &project.name)
        else {
            return;
        };
        self.project_preview_open_path(readme);
    }

    pub(crate) fn project_preview_select_tab(&mut self, idx: usize) {
        let arming = self
            .active_workspace()
            .map(|ws| idx < ws.project_preview.tabs().len())
            .unwrap_or(false);
        // 同 `preview_select_tab`:提前算好真实可用宽度,避免跟
        // `with_focused_project` 的 `&mut self` 借用冲突。
        let avail_w = self.preview_tab_bar_avail_px(PanelKind::Project);
        self.with_focused_project(|ws, _io| {
            ws.project_preview.select(idx);
            if idx < ws.project_preview.tabs().len() {
                let widths: Vec<f32> = ws
                    .project_preview
                    .tabs()
                    .iter()
                    .map(|t| preview_tab_display_width(&t.title))
                    .collect();
                ws.project_preview_tab_first = tab_widget::tab_window_reveal(
                    &widths,
                    4.0,
                    avail_w,
                    ws.project_preview_tab_first,
                    idx,
                );
                ws.project_preview_tab_overflow_anchor = None;
            }
        });
        if arming {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::ProjectPreview,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    pub(crate) fn agent_state_changed(
        &mut self,
        project_id: ProjectId,
        tab_id: usize,
        agent: AgentKind,
        state: AgentState,
        transcript_path: Option<String>,
    ) {
        self.with_project(project_id, |ws, io| {
            // 当前项目路径先取出（下面要 &mut 借 tab，冲突）；重锚:项目优先。
            let active_repo = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
            // 卡片刷新要传的数据在这里先摘出来（`Option` 同时充当"tab 是否
            // 存在"的哨兵）：`tab`（来自 `ws.tab_by_id_mut`）借的是整个
            // `ws`，下面 TurnEnded 分支还要再用 `tab`，中间插一句
            // `ws.spawn_agent_card_refresh`（借 `&ws`）会跟这个 `&mut ws`
            // 借用重叠、过不了借用检查；摘成局部变量、挪到这个 `if let`
            // 块结束、`tab` 借用已经释放之后再调用，规避这个冲突,语义不变
            // (仍然是"tab 存在就必调用一次,不进 TurnEnded 条件分支")。
            let mut card_refresh_args: Option<(Option<String>, PathBuf)> = None;
            if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                tab.agent_state = state;
                tab.agent = agent;
                // 补一道:picker 不是唯一入口(比如在已开着的纯 shell tab 里
                // 手打 `opencode`),那种情况下 `on_tab_attached` 拿不到
                // picker 提示,只能靠这条 hook 上报事后补上——虽然大概率已经
                // 错过了 agent 进程启动瞬间那波终端探测查询,但仍是"跟真实
                // agent 保持同步"的唯一后备信号。
                tab.model
                    .set_answer_dynamic_color(agent == AgentKind::Opencode);
                if let Some(tp) = transcript_path {
                    tab.transcript_path = Some(tp);
                }
                tracing::info!(tab_id, ?state, "agent 状态变更");
                card_refresh_args = Some((tab.transcript_path.clone(), tab.effective_cwd()));
            }
            if let Some((transcript_path, cwd)) = card_refresh_args {
                ws.spawn_agent_card_refresh(
                    io,
                    tab_id,
                    agent,
                    transcript_path,
                    cwd,
                    active_repo.clone(),
                );
            }
            // 回合结束后刷新会话列表(transcript 增长/新增；P1j)与项目 git/
            // 磁盘占用状态(文件树装饰随之更新；P1h)。这两项刷新原先分别挂在
            // 会话列表自己的耦合链路、以及已删除的验收检测异步回调
            // (`delivery_checked`)上——验收闭环删除后,后者连带的刷新触发点
            // 也没了,这里改成回合结束就无条件触发,不再依赖任何验收检测结果
            // (前半"会话列表"这条 P1j 当年就已经这样修过一次,这次是把后半
            // "git/磁盘占用"也补齐同样的处理)。
            if state == AgentState::TurnEnded {
                ws.spawn_conversations_refresh(io);
                if let Some(project) = &ws.project {
                    let repo_path = PathBuf::from(&project.path);
                    spawn_project_git_refresh(project_id, repo_path.clone(), io);
                    spawn_disk_usage_refresh(project_id, repo_path, io);
                }
            }
            // 审阅 tab 若开着且属本会话,回合结束重解析 transcript（P1i）。
            if state == AgentState::TurnEnded
                && let Some(rv) = &ws.review
                && review_should_refresh_on_turn(&rv.source, tab_id)
                && let Some((path, tab_agent)) = ws
                    .tabs
                    .iter()
                    .find(|t| t.tab_id == tab_id)
                    .and_then(|t| t.transcript_path.clone().map(|p| (p, t.agent)))
            {
                ws.spawn_review_load(
                    io,
                    ReviewSource::Session(tab_id),
                    path,
                    tab_agent,
                    -1,
                    10_000,
                );
            }
        });
    }

    /// 点击对话面板扁平列表里的某一行——只把审阅面板加载到这一个回合
    /// 点对话面板会话列表某一行 → 打开该 session 的详情审阅:整段回合
    /// 列表 + 总结展示区(标题/全文)。`agent` 由调用方随行内数据一并传入。
    /// 总结数据直接取自已加载的 `SessionRow`,不为此单独发请求(2026-08-27)。
    pub(crate) fn conversation_session_open(&mut self, conversation_id: String, agent: AgentKind) {
        self.with_focused_project(move |ws, io| {
            let Some(row) = ws
                .conversations
                .sessions()
                .and_then(|rows| rows.iter().find(|r| r.conversation_id == conversation_id))
            else {
                // 列表刷新与点击之间的竞态(极小概率):这一行已经不在当前
                // 列表里了,直接不打开详情,不 panic、不报错弹窗。
                return;
            };
            let summary_title = row.display_title.clone();
            let summary_text = row.summary.clone();
            let last_ts = row.last_ts;
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let source = ReviewSource::Conversation(conversation_id.clone());
            ws.review = Some(ReviewView {
                source: source.clone(),
                entries: Vec::new(),
                error: None,
                agent,
                nonce: 0,
                summary_title: Some(summary_title),
                summary_text,
                summary_time: Some(relative_time_text(last_ts, now_ms)),
                loaded_nonce: None,
            });
            ws.spawn_review_load_conversation(
                io,
                conversation_id,
                -1,
                CONVERSATION_DETAIL_PAGE_SIZE,
                false,
            );
        });
    }

    /// 会话详情面板"生成总结"按钮:对某 conversation 提交 V2 总结任务
    /// (Manual + force),轮询到终态后刷新列表 + 重新打开详情(spec 2026-09-26
    /// 第 8 节:会话详情提供生成/重试)。刷新与重开两条消息按序到达,主循环
    /// 串行处理,`SessionsRefreshed` 先更新列表、`SessionOpen` 后用新 summary。
    pub(crate) fn conversation_summary_generate(
        &mut self,
        conversation_id: String,
        _agent: AgentKind,
    ) {
        let Some((project_id, cwd)) = self
            .active_workspace()
            .and_then(|ws| ws.project.as_ref())
            .map(|p| (p.id, p.path.clone()))
        else {
            return;
        };
        let started = self.active_workspace_mut().is_some_and(|ws| {
            ws.conversations
                .start_summary_generation(conversation_id.clone())
        });
        if !started {
            return;
        }
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let cid = conversation_id.clone();
        handle.spawn(async move {
            let job_id = match client
                .submit_summary_job(&cid, None, SummaryTrigger::Manual, None, None, true)
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::warn!(error = %e, %cid, "提交总结任务失败");
                    let _ = proxy.send_event(Message::Conversations(
                        conversations::Message::SummaryGenerateFinished(
                            project_id,
                            cid.clone(),
                            Err(e.to_string()),
                        ),
                    ));
                    return;
                }
            };
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                match client.get_summary_job(job_id).await {
                    Ok(Some(job)) => {
                        if job.status == SummaryJobStatus::Failed {
                            let error = job.error_detail.unwrap_or_else(|| "未知错误".into());
                            let _ = proxy.send_event(Message::Conversations(
                                conversations::Message::SummaryGenerateFinished(
                                    project_id,
                                    cid.clone(),
                                    Err(error),
                                ),
                            ));
                            return;
                        }
                        if job.status == SummaryJobStatus::Cancelled {
                            let _ = proxy.send_event(Message::Conversations(
                                conversations::Message::SummaryGenerateFinished(
                                    project_id,
                                    cid.clone(),
                                    Err("任务已取消".into()),
                                ),
                            ));
                            return;
                        }
                        if matches!(
                            job.status,
                            SummaryJobStatus::Succeeded
                                | SummaryJobStatus::Failed
                                | SummaryJobStatus::Cancelled
                        ) {
                            break;
                        }
                    }
                    Ok(None) => {
                        tracing::warn!(job_id, %cid, "总结任务不存在");
                        let _ = proxy.send_event(Message::Conversations(
                            conversations::Message::SummaryGenerateFinished(
                                project_id,
                                cid.clone(),
                                Err("总结任务不存在，请重试".into()),
                            ),
                        ));
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, job_id, %cid, "查询总结任务失败");
                        let _ = proxy.send_event(Message::Conversations(
                            conversations::Message::SummaryGenerateFinished(
                                project_id,
                                cid.clone(),
                                Err(format!("查询总结失败：{e}")),
                            ),
                        ));
                        return;
                    }
                }
            }
            let result = client
                .list_conversations_with_summaries(&cwd, None, 500, 0)
                .await
                .map(|rows| {
                    rows.iter()
                        .map(|(c, s)| crate::conversation::SessionRow::from_row(c, s.as_ref()))
                        .collect()
                })
                .map_err(|e| e.to_string());
            let completion = result
                .as_ref()
                .map(|_| ())
                .map_err(|e| format!("总结已生成，但刷新会话失败：{e}"));
            let _ = proxy.send_event(Message::Conversations(
                conversations::Message::SessionsRefreshed(project_id, result),
            ));
            let _ = proxy.send_event(Message::Conversations(
                conversations::Message::SummaryGenerateFinished(project_id, cid, completion),
            ));
        });
    }

    pub(crate) fn search_results(
        &mut self,
        project_id: i64,
        result: Result<Vec<(String, Vec<search::SearchHit>)>, String>,
    ) {
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        self.with_project(project_id, move |ws, _io| {
            let emit = move |m| {
                let _ = proxy.send_event(Message::Search(m));
            };
            search::update(
                &mut ws.search,
                search::Message::SearchResults(project_id, result),
                project_id,
                &handle,
                emit,
            );
        });
    }

    pub(crate) fn files_project_message(&mut self, project_id: i64, msg: files::Message) {
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let agent_terminal_visible = self.terminal_visible();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Files(m));
        };
        let app_files = &mut self.files;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let external_apps = self.external_apps.clone();
        files::update(
            &mut ws.files,
            app_files,
            msg,
            project_id,
            &handle,
            emit,
            &external_apps,
            agent_terminal_visible,
        );
    }

    /// Project 面板链接行右键菜单浮层:当前只含"删除"。定位坐标复用
    /// `files.last_right_click`(main.rs 任意右键都会先写入),"删除"回
    /// `project::Message::LinkRemove`。
    pub(crate) fn project_link_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.project_link_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            vec![crate::chrome::menu::item::<Message>(
                Some(icons::IconKind::Trash),
                "删除",
                Message::Project(project::Message::LinkRemove {
                    target: menu.target,
                    index: menu.index,
                }),
            )];

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::chrome::menu::shell_frosted(items, Length::Shrink);
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: menu.y,
                left: menu.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    /// Todo 分类树节点右键菜单浮层:新建子/同级分类、重命名、删除、上移/
    /// 下移、移动到...。定位坐标复用 `files.last_right_click`。
    pub(crate) fn category_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.category_context_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        // 右键的是"全部"/"未分类"伪节点:菜单只含"新建分类"(新建顶层
        // 分类,不挂在任何真实名字下——那两个只是视图桶)。真实节点才给
        // 完整节点操作集。
        let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            match menu.id {
                None => vec![crate::chrome::menu::item::<Message>(
                    Some(icons::IconKind::SquarePlus),
                    "新建分类",
                    Message::Todo(todo::Message::CategoryNewSibling(None)),
                )],
                Some(id) => {
                    // 被右键节点的 parent_id,给"新建同级分类"用(同级 =
                    // 挂在同一个 parent_id 下)。取不到就退化为顶层。
                    let sibling_parent_id = self.active_workspace().and_then(|ws| {
                        ws.todo
                            .categories()
                            .iter()
                            .find(|c| c.id == id)
                            .and_then(|c| c.parent_id)
                    });
                    vec![
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::SquarePlus),
                            "新建子分类",
                            Message::Todo(todo::Message::CategoryNewChild(id)),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::SquarePlus),
                            "新建同级分类",
                            Message::Todo(todo::Message::CategoryNewSibling(sibling_parent_id)),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::ChevronUp),
                            "上移",
                            Message::Todo(todo::Message::CategoryMoveSibling(
                                id,
                                dozer_core::protocol::CategoryMoveDirection::Up,
                            )),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::ChevronDown),
                            "下移",
                            Message::Todo(todo::Message::CategoryMoveSibling(
                                id,
                                dozer_core::protocol::CategoryMoveDirection::Down,
                            )),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::FolderOpen),
                            "移动到...",
                            Message::Todo(todo::Message::CategoryReparentPickerOpen(id)),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::Rename),
                            "重命名",
                            Message::Todo(todo::Message::CategoryRenameStart(id)),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::Trash),
                            "删除",
                            Message::Todo(todo::Message::CategoryDelete(id)),
                        ),
                    ]
                }
            };

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::chrome::menu::shell_frosted(items, Length::Shrink);
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: menu.y,
                left: menu.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    /// 分类选择器浮层:列出当前项目的全部分类节点(全展开按 depth 缩进
    /// 平铺),点"未分类"或某个节点即把 `category_picker` 目标落盘并关闭
    /// (Category → reparent,Todo → set_todo_category)。浮层只服务这两类
    /// "把一个节点挂到某个分类"的动作 —— 搜索框的分类速滤已移除(改为
    /// 按状态过滤),不再有 `Filter` 目标。
    pub(crate) fn category_picker_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let Some(picker) = &self.category_picker else {
            return column![].into();
        };
        let categories = self
            .active_workspace()
            .map(|ws| ws.todo.categories().to_vec())
            .unwrap_or_default();
        let mut list = column![].spacing(2);

        // "未分类"钉在树顶(`None` = 未分类)。
        list = list.push(
            button(text("未分类").size(byteui::theme::font::body()))
                .on_press(Message::CategoryPickerSelect(None))
                .width(Length::Fill)
                .padding([6, 10]),
        );
        // 复用一份没有展开态(全展开)的拍平——选择器只做单次选择,不需要
        // 折叠交互,直接把整棵树按 depth 缩进平铺出来最简单。
        let all_expanded: std::collections::HashSet<i64> =
            categories.iter().map(|c| c.id).collect();
        for row in todo::visible_category_rows(&categories, &all_expanded) {
            let id = row.id;
            let click = Message::CategoryPickerSelect(Some(id));
            list = list.push(
                button(
                    text(format!("{}{}", "  ".repeat(row.depth), row.name))
                        .size(byteui::theme::font::body()),
                )
                .on_press(click)
                .width(Length::Fill)
                .padding([6, 10]),
            );
        }
        let list =
            container(list.width(Length::Fixed(220.0))).style(move |_t: &iced_widget::Theme| {
                container::Style {
                    background: Some(byteui::theme::color::current().panel.into()),
                    border: Border {
                        color: byteui::theme::color::current().border,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..container::Style::default()
                }
            });
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: picker.y,
                left: picker.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    /// 数据库面板数据源树 header 行右键菜单浮层:测试连接/编辑/删除/刷新。
    /// 定位坐标复用 `files.last_right_click`。"刷新"只有该数据源当前已
    /// 展开(有 schema 树数据)才可点,未展开时置灰——展开动作本身走左键
    /// 点 header,不进这个菜单。
    pub(crate) fn database_source_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.database_source_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let source_id = menu.source_id.clone();
        let expanded = self
            .active_workspace()
            .is_some_and(|ws| ws.database.is_expanded(&source_id));
        let dim = byteui::theme::color::current().dim;
        let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = vec![
            crate::chrome::menu::item::<Message>(
                Some(icons::IconKind::RefreshCw),
                "测试连接",
                Message::Database(database::Message::TestConnection(source_id.clone())),
            ),
            crate::chrome::menu::item::<Message>(
                Some(icons::IconKind::Settings),
                "编辑",
                Message::Database(database::Message::EditSourceStart(source_id.clone())),
            ),
            crate::chrome::menu::item::<Message>(
                Some(icons::IconKind::Trash),
                "删除",
                Message::Database(database::Message::DeleteSourceRequest(source_id.clone())),
            ),
        ];
        items.push(if expanded {
            crate::chrome::menu::item::<Message>(
                Some(icons::IconKind::RotateCw),
                "刷新",
                Message::Database(database::Message::SchemaRefresh(source_id.clone())),
            )
        } else {
            crate::chrome::menu::item_locked(Some(icons::IconKind::RotateCw), "刷新", dim)
        });

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::chrome::menu::shell_frosted(
                items,
                Length::Fixed(byteui::theme::geometry::menu_item_width()),
            );
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: menu.y,
                left: menu.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    /// 输入框右键菜单浮层:固定四项(剪切/复制/粘贴/全选),定位坐标复用
    /// `files.last_right_click`(main.rs 任意右键都会先写入,`TextInputMenuOpen`
    /// 已用它填好 `x/y`)。动作消息回 main.rs——由它合成回 ⌘/Ctrl+`x`/`c`/`v`/`a`
    /// 键盘事件作用到被右键的输入。密码框(`secure`)禁用剪切/复制(置灰)。
    pub(crate) fn text_input_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.text_input_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let dim = byteui::theme::color::current().dim;
        let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            Vec::new();
        if menu.target.secure {
            // 密码框:复制/剪切同原生快捷键一样被禁用,置灰不可点。
            items.push(crate::chrome::menu::item_locked(
                Some(icons::IconKind::Scissors),
                "剪切",
                dim,
            ));
            items.push(crate::chrome::menu::item_locked(
                Some(icons::IconKind::Copy),
                "复制",
                dim,
            ));
        } else {
            items.push(crate::chrome::menu::item(
                Some(icons::IconKind::Scissors),
                "剪切",
                Message::TextInputMenuCut,
            ));
            items.push(crate::chrome::menu::item(
                Some(icons::IconKind::Copy),
                "复制",
                Message::TextInputMenuCopy,
            ));
        }
        items.push(crate::chrome::menu::item(
            Some(icons::IconKind::ClipboardPaste),
            "粘贴",
            Message::TextInputMenuPaste,
        ));
        items.push(crate::chrome::menu::item(
            Some(icons::IconKind::SelectAll),
            "全选",
            Message::TextInputMenuSelectAll,
        ));

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::chrome::menu::shell_frosted(
                items,
                Length::Fixed(byteui::theme::geometry::menu_item_width()),
            );
        // 常规右键菜单就地向下/向上弹即可,这里输入框多用在面板内容区,直接
        // 以光标为左上锚弹出(必要时可在下方再夹窗口高度,留待需要时加)。
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(iced_widget::core::alignment::Vertical::Top)
            .padding(Padding {
                top: menu.y,
                left: menu.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }
}

#[cfg(test)]
mod review_trace_tests {
    use super::*;
    use crate::transcript::ReviewEntry;
    use crate::workspace::ReviewSource;
    use dozer_core::protocol::AgentKind;

    fn review(nonce: u64, loaded_nonce: Option<u64>) -> crate::workspace::ReviewView {
        crate::workspace::ReviewView {
            source: ReviewSource::Conversation("c1".into()),
            entries: vec![ReviewEntry::Human { text: "hi".into() }],
            error: None,
            agent: AgentKind::Claude,
            nonce,
            summary_title: None,
            summary_text: None,
            summary_time: None,
            loaded_nonce,
        }
    }

    #[test]
    fn sets_loaded_nonce_to_current_nonce() {
        let mut review = Some(review(5, None));
        mark_review_loaded(&mut review);
        assert_eq!(review.as_ref().unwrap().loaded_nonce, Some(5));
    }

    /// 对应 Review Focus"过期事件":事件到达时 nonce 已经自增,`loaded_nonce`
    /// 只能追到**当前**值(6)而非事件本该对应的旧值——已知限制,事件不携带
    /// nonce。这条测试记录该行为,避免未来被误当成未测疏漏。
    #[test]
    fn stale_event_still_advances_to_current_nonce_known_limitation() {
        let mut review = Some(review(6, None));
        mark_review_loaded(&mut review);
        assert_eq!(review.as_ref().unwrap().loaded_nonce, Some(6));
    }

    #[test]
    fn no_op_when_review_is_none() {
        let mut review: Option<crate::workspace::ReviewView> = None;
        mark_review_loaded(&mut review);
        assert!(review.is_none());
    }

    #[test]
    fn refreshed_summary_rebuilds_snapshot_and_advances_nonce() {
        let mut review = Some(review(5, Some(5)));
        let mut nonce = 5;
        let row = crate::conversation::SessionRow {
            conversation_id: "c1".into(),
            agent: AgentKind::Claude,
            last_ts: 1,
            display_title: "新标题".into(),
            summary: Some("新总结".into()),
            summary_status: Some(dozer_core::protocol::SummaryStatus::AiGenerated),
            task_id: None,
        };

        let json = refresh_open_conversation_summary(&mut review, &mut nonce, &row)
            .expect("总结变化应重建快照");

        let rv = review.as_ref().unwrap();
        assert_eq!(rv.summary_title.as_deref(), Some("新标题"));
        assert_eq!(rv.summary_text.as_deref(), Some("新总结"));
        assert_eq!(rv.nonce, 6);
        assert_eq!(nonce, 6);
        assert_eq!(
            rv.loaded_nonce,
            Some(5),
            "旧加载水位应使新 WebView 进入加载态"
        );
        assert!(json.contains("新标题"));
        assert!(json.contains("新总结"));
    }

    #[test]
    fn unchanged_summary_does_not_reload_webview() {
        let mut value = review(5, Some(5));
        value.summary_title = Some("已有标题".into());
        value.summary_text = Some("已有总结".into());
        let mut review = Some(value);
        let mut nonce = 5;
        let row = crate::conversation::SessionRow {
            conversation_id: "c1".into(),
            agent: AgentKind::Claude,
            last_ts: 1,
            display_title: "已有标题".into(),
            summary: Some("已有总结".into()),
            summary_status: Some(dozer_core::protocol::SummaryStatus::AiGenerated),
            task_id: None,
        };

        assert!(refresh_open_conversation_summary(&mut review, &mut nonce, &row).is_none());
        assert_eq!(nonce, 5);
        assert_eq!(review.as_ref().unwrap().nonce, 5);
    }
}
