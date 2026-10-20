//! `update` / `restore_tabs` 的库内测试(假时钟不需要:所有超时都是 `Effect`,不真等)。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::capabilities::PreviewCapabilities;
use crate::host_ipc::HostEvent;
use crate::*;

fn ctx<'a>(caps: &'a PreviewCapabilities, root: Option<&'a Path>) -> UpdateCtx<'a> {
    UpdateCtx {
        panel: ViewerPanel::Files,
        project_id: Some(7),
        project_root: root,
        caps,
        locale: "en",
        tab_bar_avail_px: 600.0,
        tab_display_width: &|t: &str| t.len() as f32 * 8.0 + 30.0,
        active_webview_pool_id: None,
        behavior: PanelBehavior {
            persist_tabs: true,
            materialize_suspended: true,
        },
        recovery_dir: None,
        failures_path: PathBuf::from("/nonexistent/preview_failures.json"),
        safe_startup: false,
    }
}

fn file(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

fn open(
    state: &mut PreviewPanelState,
    c: &UpdateCtx<'_>,
    path: &Path,
) -> (usize, u64, Vec<Effect>) {
    let fx = update(
        state,
        Message::OpenPath {
            path: path.to_path_buf(),
            target_line: None,
        },
        c,
    );
    let (tab_id, generation) = fx
        .iter()
        .find_map(|e| match e {
            Effect::ProbeProfile {
                tab_id, generation, ..
            } => Some((*tab_id, *generation)),
            _ => None,
        })
        .expect("OpenPath 应产生 ProbeProfile");
    (tab_id, generation, fx)
}

fn profile(path: &Path) -> FileProfile {
    profile_file(path).unwrap()
}

#[test]
fn open_path_probes_the_profile_then_routes_by_the_profile() {
    let dir = tempfile::tempdir().unwrap();
    let md = file(dir.path(), "a.md", "# hi\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, generation, fx) = open(&mut st, &c, &md);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::AllowFile(p) if p == &md))
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::PathOpened(p) if p == &md))
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::PersistTabs { project_id: 7, .. }))
    );
    let fx = update(
        &mut st,
        Message::Profiled {
            tab_id: id,
            generation,
            result: Ok(profile(&md)),
        },
        &c,
    );
    // Markdown 是渲染 host:进入 CreatingHost 并 arm 看门狗。
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::ScheduleLoadTimeout {
            stage: PreviewLoadStage::CreatingHost,
            ..
        }
    )));
}

#[test]
fn opening_a_missing_file_sets_the_panel_error_and_no_effects() {
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let fx = update(
        &mut st,
        Message::OpenPath {
            path: PathBuf::from("/definitely/not/here.rs"),
            target_line: None,
        },
        &c,
    );
    assert!(fx.is_empty());
    assert!(st.error.is_some());
    assert_eq!(st.pane.tabs().len(), 1, "只有 Blank 占位");
}

#[test]
fn opening_the_same_path_twice_keeps_one_tab_and_selects_it() {
    let dir = tempfile::tempdir().unwrap();
    let a = file(dir.path(), "a.md", "a");
    let b = file(dir.path(), "b.md", "b");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    open(&mut st, &c, &a);
    open(&mut st, &c, &b);
    assert_eq!(st.pane.active_idx(), 2);
    let fx = update(
        &mut st,
        Message::OpenPath {
            path: a,
            target_line: None,
        },
        &c,
    );
    assert!(!fx.iter().any(|e| matches!(e, Effect::ProbeProfile { .. })));
    assert_eq!(st.pane.tabs().len(), 3);
    assert_eq!(st.pane.active_idx(), 1);
}

#[test]
fn disabled_backends_degrade_to_unsupported_and_never_load() {
    let dir = tempfile::tempdir().unwrap();
    // (扩展名, 只开其余能力时的预期)——每种后端关掉自己的开关,其余全开。
    type Off = fn(&mut PreviewCapabilities);
    let cases: [(&str, &str, Off); 5] = [
        ("a.rs", "fn main() {}\n", |c| c.code = false),
        ("a.md", "# x\n", |c| c.rendered = false),
        ("a.json", "{}\n", |c| c.json = false),
        ("a.jsonl", "{}\n", |c| c.streamed = false),
        ("a.csv", "a,b\n1,2\n", |c| c.tabular = false),
    ];
    for (name, body, off) in cases {
        let path = file(dir.path(), name, body);
        let mut caps = PreviewCapabilities::all();
        off(&mut caps);
        let c = ctx(&caps, None);
        let mut st = PreviewPanelState::default();
        let (id, generation, _) = open(&mut st, &c, &path);
        let fx = update(
            &mut st,
            Message::Profiled {
                tab_id: id,
                generation,
                result: Ok(profile(&path)),
            },
            &c,
        );
        let tab = st.pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(
            matches!(tab.backend, Some(PreviewBackend::Unsupported(_))),
            "{name}: 关闭后应降级"
        );
        assert!(!tab.hosts_any_webview(), "{name}");
        assert!(
            !fx.iter().any(|e| matches!(
                e,
                Effect::ScheduleLoadTimeout { .. }
                    | Effect::LoadTabular { .. }
                    | Effect::LoadPlantUml { .. }
            )),
            "{name}: 降级后不产生加载效果"
        );
        // 全开时同一文件不降级。
        let all = PreviewCapabilities::all();
        let c = ctx(&all, None);
        let mut st = PreviewPanelState::default();
        let (id, generation, _) = open(&mut st, &c, &path);
        update(
            &mut st,
            Message::Profiled {
                tab_id: id,
                generation,
                result: Ok(profile(&path)),
            },
            &c,
        );
        let tab = st.pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(
            !matches!(tab.backend, Some(PreviewBackend::Unsupported(_))),
            "{name}: 全开时不应降级"
        );
    }
}

#[test]
fn external_open_action_is_available_on_every_degraded_tab() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::default();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, generation, _) = open(&mut st, &c, &path);
    update(
        &mut st,
        Message::Profiled {
            tab_id: id,
            generation,
            result: Ok(profile(&path)),
        },
        &c,
    );
    let fx = update(&mut st, Message::OpenExternal(id), &c);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::OpenExternal { path: p } if p == &path))
    );
}

#[test]
fn save_and_legacy_editor_messages_are_noops() {
    let dir = tempfile::tempdir().unwrap();
    let png = file(dir.path(), "a.png", "");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    // .png 走 wry(无 editor):撤销 / 重做 / 保存应静默 no-op,不 panic、不乱标脏。
    open(&mut st, &c, &png);
    for m in [
        Message::SaveActive,
        Message::InsertTab,
        Message::UndoActive,
        Message::RedoActive,
    ] {
        assert!(update(&mut st, m, &c).is_empty());
    }
    assert!(!st.pane.tabs()[st.pane.active_idx()].dirty);
}

#[test]
fn conflict_reload_needs_two_clicks_and_clears_the_recovery_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, _, _) = open(&mut st, &c, &path);
    let tab = st.pane.tabs_mut().iter_mut().find(|t| t.id == id).unwrap();
    tab.dirty = true;
    let _ = tab.mark_disk_conflict();
    let first = update(&mut st, Message::ConflictReload(id), &c);
    assert!(first.is_empty(), "第一次点击只武装");
    assert!(
        st.pane
            .tabs()
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .conflict_reload_armed
    );
    let second = update(&mut st, Message::ConflictReload(id), &c);
    assert!(
        second
            .iter()
            .any(|e| matches!(e, Effect::ClearRecovery { project_id: 7, path: p } if p == &path))
    );
    assert!(st.pane.active_conflict().is_none());
}

#[test]
fn conflict_keep_clears_the_conflict_without_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(dir.path(), "a.rs", "x");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, _, _) = open(&mut st, &c, &path);
    let tab = st.pane.tabs_mut().iter_mut().find(|t| t.id == id).unwrap();
    tab.dirty = true;
    let _ = tab.mark_disk_conflict();
    assert!(update(&mut st, Message::ConflictKeep(id), &c).is_empty());
    assert!(st.pane.active_conflict().is_none());
}

#[test]
fn load_timeout_for_an_older_generation_is_ignored_and_current_marks_failed_once() {
    let dir = tempfile::tempdir().unwrap();
    let md = file(dir.path(), "a.md", "# x");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, generation, _) = open(&mut st, &c, &md);
    update(
        &mut st,
        Message::Profiled {
            tab_id: id,
            generation,
            result: Ok(profile(&md)),
        },
        &c,
    );
    let stage = st
        .pane
        .tabs()
        .iter()
        .find(|t| t.id == id)
        .unwrap()
        .load_state
        .stage;
    let stale = Message::LoadTimeout {
        tab_id: id,
        generation: generation.wrapping_add(99),
        stage,
    };
    update(&mut st, stale, &c);
    let tab = st.pane.tabs().iter().find(|t| t.id == id).unwrap();
    assert!(!tab.backend_state.is_failed(), "旧世代的超时被忽略");
    let live = Message::LoadTimeout {
        tab_id: id,
        generation,
        stage,
    };
    update(&mut st, live.clone(), &c);
    assert!(
        st.pane
            .tabs()
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .backend_state
            .is_failed()
    );
    // 再来一次不应 panic,也不重复改变状态。
    update(&mut st, live, &c);
    assert!(
        st.pane
            .tabs()
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .backend_state
            .is_failed()
    );
}

#[test]
fn recovery_messages_for_a_closed_tab_are_ignored() {
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    assert!(update(&mut st, Message::RecoveryWritten(999), &c).is_empty());
    assert!(
        update(
            &mut st,
            Message::RecoveryRead {
                tab_id: 999,
                generation: 1,
                restore: Some("text".into())
            },
            &c
        )
        .is_empty()
    );
    assert!(st.pane.take_pending_editor_commands().is_empty());
}

#[test]
fn blank_info_for_another_project_or_a_non_blank_active_tab_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let caps = PreviewCapabilities::all();
    let info = |path: &Path| BlankPaneInfo {
        path: path.to_path_buf(),
        size_bytes: 1,
        file_count: 1,
        created: None,
        modified: None,
    };
    let mut st = PreviewPanelState::default();
    st.pane.blank_info_in_flight = true;
    let c = ctx(&caps, Some(&root));
    update(&mut st, Message::BlankInfoLoaded { info: info(&root) }, &c);
    assert!(!st.pane.blank_info_in_flight);
    assert!(
        st.pane.blank_info.is_some(),
        "项目一致且激活为 Blank → 接收"
    );
    let mut other = PreviewPanelState::default();
    update(
        &mut other,
        Message::BlankInfoLoaded {
            info: info(Path::new("/elsewhere")),
        },
        &c,
    );
    assert!(other.pane.blank_info.is_none(), "项目不符 → 丢弃");
    let f = file(dir.path(), "a.rs", "x");
    let mut busy = PreviewPanelState::default();
    open(&mut busy, &c, &f);
    update(
        &mut busy,
        Message::BlankInfoLoaded { info: info(&root) },
        &c,
    );
    assert!(busy.pane.blank_info.is_none(), "激活页签不是 Blank → 丢弃");
}

#[test]
fn large_file_search_drops_progress_and_results_of_an_old_query() {
    let dir = tempfile::tempdir().unwrap();
    let f = file(dir.path(), "big.log", "needle\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, _, _) = open(&mut st, &c, &f);
    st.pane.open_large_file_search(id);
    let fx1 = update(
        &mut st,
        Message::LargeFileSearchSubmit {
            tab_id: id,
            query: "needle".into(),
        },
        &c,
    );
    let g1 = fx1
        .iter()
        .find_map(|e| match e {
            Effect::StartLargeFileSearch { generation, .. } => Some(*generation),
            _ => None,
        })
        .expect("提交产生搜索效果");
    let fx2 = update(
        &mut st,
        Message::LargeFileSearchSubmit {
            tab_id: id,
            query: "other".into(),
        },
        &c,
    );
    let g2 = fx2
        .iter()
        .find_map(|e| match e {
            Effect::StartLargeFileSearch { generation, .. } => Some(*generation),
            _ => None,
        })
        .unwrap();
    assert_ne!(g1, g2);
    update(
        &mut st,
        Message::LargeFileSearchProgress {
            tab_id: id,
            generation: g1,
            completed: 5,
            total: 10,
        },
        &c,
    );
    assert!(
        st.pane
            .large_file_search
            .as_ref()
            .unwrap()
            .progress
            .is_none(),
        "旧查询的进度被丢"
    );
    update(
        &mut st,
        Message::LargeFileSearchResults {
            tab_id: id,
            generation: g1,
            result: Err("late".into()),
        },
        &c,
    );
    assert!(
        st.pane.large_file_search.as_ref().unwrap().error.is_none(),
        "旧查询的结果被丢"
    );
    // 空查询:取消在途并清 loading,不派任务。
    let fx3 = update(
        &mut st,
        Message::LargeFileSearchSubmit {
            tab_id: id,
            query: "  ".into(),
        },
        &c,
    );
    assert!(fx3.is_empty());
    assert!(!st.pane.large_file_search.as_ref().unwrap().running);
}

#[test]
fn update_never_panics_on_messages_for_missing_tab_ids() {
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let bad = 4242;
    let msgs = vec![
        Message::OpenExternal(bad),
        Message::Retry(bad),
        Message::PlainTextOpen(bad),
        Message::ConflictKeep(bad),
        Message::ConflictReload(bad),
        Message::TabularTextModeToggle(bad),
        Message::JsonModeToggle(bad),
        Message::LargeFileSearchSubmit {
            tab_id: bad,
            query: "q".into(),
        },
        Message::LargeFileSearchResults {
            tab_id: bad,
            generation: 1,
            result: Err("e".into()),
        },
        Message::LargeFileSearchProgress {
            tab_id: bad,
            generation: 1,
            completed: 1,
            total: 2,
        },
        Message::Profiled {
            tab_id: bad,
            generation: 1,
            result: Err("e".into()),
        },
        Message::LoadTimeout {
            tab_id: bad,
            generation: 1,
            stage: PreviewLoadStage::Parsing,
        },
        Message::WindowIndex {
            tab_id: bad,
            result: Err("e".into()),
        },
        Message::RecoveryWritten(bad),
        Message::RecoveryRead {
            tab_id: bad,
            generation: 1,
            restore: None,
        },
        Message::TabularLoaded {
            tab_id: bad,
            generation: 1,
            result: Err("e".into()),
        },
        Message::TabularSheetLoaded {
            tab_id: bad,
            sheet_index: 0,
            result: Err("e".into()),
        },
        Message::SelectTab(99),
        Message::CloseTab(99),
        Message::ToggleRenderMode(99),
        Message::FindOpen,
        Message::FindText("x".into()),
        Message::FindGo(true),
        Message::LargeFileSearchGo(true),
        Message::TabOverflowToggle { cursor: (1.0, 2.0) },
        Message::TabOverflowDismiss,
    ];
    for m in msgs {
        let _ = update(&mut st, m, &c);
    }
    assert_eq!(st.pane.tabs().len(), 1);
}

#[test]
fn closing_a_tab_resets_tab_first_and_reports_old_len() {
    let dir = tempfile::tempdir().unwrap();
    let a = file(dir.path(), "a.md", "a");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    open(&mut st, &c, &a);
    st.tab_first = 3;
    let fx = update(&mut st, Message::CloseTab(1), &c);
    assert_eq!(st.tab_first, 0);
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::TabClosed {
            index: 1,
            old_len: 2
        }
    )));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::ContextChanged { immediate: true }))
    );
}

#[test]
fn project_style_panel_neither_persists_nor_pushes_context() {
    let dir = tempfile::tempdir().unwrap();
    let a = file(dir.path(), "a.md", "a");
    let caps = PreviewCapabilities::all();
    let mut c = ctx(&caps, None);
    c.behavior = PanelBehavior {
        persist_tabs: false,
        materialize_suspended: false,
    };
    let mut st = PreviewPanelState::default();
    let (_, _, fx) = open(&mut st, &c, &a);
    assert!(!fx.iter().any(|e| matches!(
        e,
        Effect::PersistTabs { .. } | Effect::ContextChanged { .. }
    )));
}

#[derive(Default)]
struct FakePersistence {
    state: Mutex<PreviewState>,
}

impl PreviewPersistence for FakePersistence {
    fn load(&self, _project_id: i64) -> PreviewState {
        self.state.lock().unwrap().clone()
    }
    fn save(&self, _project_id: i64, state: &PreviewState) -> std::io::Result<()> {
        *self.state.lock().unwrap() = state.clone();
        Ok(())
    }
    fn recovery_dir(&self) -> PathBuf {
        PathBuf::from("/nonexistent")
    }
}

#[test]
fn restore_tabs_rebuilds_shells_allows_files_and_materializes_the_active_one_once() {
    let dir = tempfile::tempdir().unwrap();
    let a = file(dir.path(), "a.md", "a");
    let b = file(dir.path(), "b.md", "b");
    let gone = dir.path().join("gone.md");
    let persistence = FakePersistence::default();
    persistence
        .save(
            7,
            &PreviewState {
                tabs: vec![
                    PersistedPreviewTab {
                        path: a.clone(),
                        ..Default::default()
                    },
                    PersistedPreviewTab {
                        path: gone,
                        ..Default::default()
                    },
                    PersistedPreviewTab {
                        path: b.clone(),
                        active: true,
                        ..Default::default()
                    },
                ],
                active_path: Some(b.clone()),
                ..Default::default()
            },
        )
        .unwrap();
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let fx = restore_tabs(&mut st, &persistence, &c, false);
    assert_eq!(st.pane.tabs().len(), 3, "占位 + 两个存在的文件");
    let allowed: Vec<&PathBuf> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::AllowFile(p) => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(allowed, vec![&a, &b]);
    let active = &st.pane.tabs()[st.pane.active_idx()];
    assert!(matches!(&active.kind, TabKind::File(p) if p == &b));
    assert!(!st.pane.is_suspended(active.id), "active 被物化");
    let other = st
        .pane
        .tabs()
        .iter()
        .find(|t| matches!(&t.kind, TabKind::File(p) if p == &a))
        .unwrap();
    assert!(st.pane.is_suspended(other.id), "其余保持壳");
}

#[test]
fn safe_startup_restores_shells_only() {
    let dir = tempfile::tempdir().unwrap();
    let a = file(dir.path(), "a.md", "a");
    let persistence = FakePersistence::default();
    persistence
        .save(
            7,
            &PreviewState {
                tabs: vec![PersistedPreviewTab {
                    path: a,
                    active: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let fx = restore_tabs(&mut st, &persistence, &c, true);
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::ScheduleLoadTimeout { .. }))
    );
    assert!(
        st.pane
            .tabs()
            .iter()
            .skip(1)
            .all(|t| st.pane.is_suspended(t.id))
    );
}

#[test]
fn persisted_snapshot_skips_blank_and_marks_the_active_file() {
    let dir = tempfile::tempdir().unwrap();
    let a = file(dir.path(), "a.md", "a");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    open(&mut st, &c, &a);
    let snap = st.pane.persisted_snapshot();
    assert_eq!(snap.tabs.len(), 1);
    assert!(snap.tabs[0].active);
    assert_eq!(snap.active_path.as_deref(), Some(a.as_path()));
}

fn run_and_collect(effect: Effect, n: usize) -> Vec<Message> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let persistence: std::sync::Arc<dyn PreviewPersistence> =
        std::sync::Arc::new(FakePersistence::default());
    run_effect(effect, move |m| drop(tx.send(m)), rt.handle(), persistence);
    (0..n)
        .map(|_| {
            rx.recv_timeout(std::time::Duration::from_secs(5))
                .expect("效果应回投消息")
        })
        .collect()
}

#[test]
fn run_effect_probe_profile_replies_with_the_profile() {
    let dir = tempfile::tempdir().unwrap();
    let f = file(dir.path(), "a.rs", "fn main() {}\n");
    let got = run_and_collect(
        Effect::ProbeProfile {
            tab_id: 3,
            generation: 9,
            path: f,
        },
        1,
    );
    assert!(matches!(
        &got[0],
        Message::Profiled {
            tab_id: 3,
            generation: 9,
            result: Ok(_)
        }
    ));
}

#[test]
fn run_effect_probe_profile_reports_unreadable_files_as_errors() {
    let got = run_and_collect(
        Effect::ProbeProfile {
            tab_id: 1,
            generation: 1,
            path: PathBuf::from("/no/such/file"),
        },
        1,
    );
    assert!(matches!(&got[0], Message::Profiled { result: Err(_), .. }));
}

#[test]
fn run_effect_window_index_builds_for_a_real_file() {
    let dir = tempfile::tempdir().unwrap();
    let f = file(dir.path(), "a.log", "1\n2\n3\n");
    let got = run_and_collect(
        Effect::StartWindowIndex {
            tab_id: 2,
            path: f,
            cancel: Default::default(),
        },
        1,
    );
    assert!(matches!(
        &got[0],
        Message::WindowIndex {
            tab_id: 2,
            result: Ok(_)
        }
    ));
}

fn search_effect(f: PathBuf, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Effect {
    Effect::StartLargeFileSearch {
        tab_id: 1,
        generation: 4,
        path: f,
        query: "needle".into(),
        cancel,
    }
}

#[test]
fn run_effect_large_file_search_returns_results_after_optional_progress() {
    let dir = tempfile::tempdir().unwrap();
    let f = file(dir.path(), "a.log", "alpha\nneedle here\nomega\n");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let persistence: std::sync::Arc<dyn PreviewPersistence> =
        std::sync::Arc::new(FakePersistence::default());
    run_effect(
        search_effect(f, Default::default()),
        move |m| drop(tx.send(m)),
        rt.handle(),
        persistence,
    );
    loop {
        match rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("应有结果")
        {
            Message::LargeFileSearchProgress { .. } => continue,
            Message::LargeFileSearchResults {
                generation: 4,
                result: Ok(o),
                ..
            } => {
                assert_eq!(o.hits.len(), 1);
                assert_eq!(o.hits[0].line, 2);
                break;
            }
            other => panic!("意外消息: {other:?}"),
        }
    }
}

#[test]
fn run_effect_cancelled_large_file_search_never_reports_results() {
    let dir = tempfile::tempdir().unwrap();
    let f = file(dir.path(), "a.log", "needle\n");
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let persistence: std::sync::Arc<dyn PreviewPersistence> =
        std::sync::Arc::new(FakePersistence::default());
    run_effect(
        search_effect(f, cancel),
        move |m| drop(tx.send(m)),
        rt.handle(),
        persistence,
    );
    std::thread::sleep(std::time::Duration::from_millis(300));
    while let Ok(m) = rx.try_recv() {
        assert!(
            !matches!(m, Message::LargeFileSearchResults { .. }),
            "取消后不得回投结果: {m:?}"
        );
    }
}

#[test]
fn run_effect_notifications_are_noops() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let persistence: std::sync::Arc<dyn PreviewPersistence> =
        std::sync::Arc::new(FakePersistence::default());
    for e in [
        Effect::AllowFile(PathBuf::from("/x")),
        Effect::PathOpened(PathBuf::from("/x")),
        Effect::ContextChanged { immediate: true },
        Effect::TabSelected { index: 1 },
        Effect::TabClosed {
            index: 1,
            old_len: 2,
        },
    ] {
        run_effect(
            e,
            |_| panic!("通知类效果不该回投消息"),
            rt.handle(),
            persistence.clone(),
        );
    }
}

// —— P4:页面事件进库 ——

fn editor_binding(panel: ViewerPanel, tab_id: usize, path: &Path) -> EditorHostBinding {
    EditorHostBinding::new(7, panel, tab_id, path.to_path_buf())
}

fn editor_event(
    binding: &EditorHostBinding,
    revision: u64,
    payload: EditorEvent,
) -> WebviewEnvelope<EditorEvent> {
    WebviewEnvelope {
        protocol_version: 1,
        project_id: binding.project_id,
        panel: binding.panel_token().to_string(),
        tab_id: binding.tab_id,
        document_id: binding.document_id(),
        revision,
        request_id: None,
        payload,
    }
}

fn open_markdown(st: &mut PreviewPanelState, c: &UpdateCtx<'_>, path: &Path) -> (usize, u64) {
    let (id, generation, _) = open(st, c, path);
    let _ = update(
        st,
        Message::Profiled {
            tab_id: id,
            generation,
            result: Ok(profile(path)),
        },
        c,
    );
    let tab = st.pane.tabs().iter().find(|t| t.id == id).unwrap();
    (id, tab.load_state.generation)
}

#[test]
fn editor_ready_advances_to_reading_and_arms_the_watchdog() {
    let dir = tempfile::tempdir().unwrap();
    let md = file(dir.path(), "a.md", "# hi\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &md);
    let binding = editor_binding(ViewerPanel::Files, id, &md);
    let fx = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::Ready {
                    read_only: false,
                    language: "markdown".into(),
                },
            ),
        }),
        &c,
    );
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::ScheduleLoadTimeout {
            tab_id,
            stage: PreviewLoadStage::Reading,
            ..
        } if *tab_id == id
    )));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::ContextChanged { .. }))
    );
    let tab = st.pane.tabs().iter().find(|t| t.id == id).unwrap();
    assert_eq!(tab.web_revision, generation);
}

#[test]
fn editor_document_loaded_finishes_the_load() {
    let dir = tempfile::tempdir().unwrap();
    let md = file(dir.path(), "a.md", "# hi\n");
    let caps = PreviewCapabilities::all();
    let mut c = ctx(&caps, None);
    c.failures_path = dir.path().join("preview_failures.json");
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &md);
    let binding = editor_binding(ViewerPanel::Files, id, &md);
    let _ = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::Ready {
                    read_only: false,
                    language: "markdown".into(),
                },
            ),
        }),
        &c,
    );
    let fx = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::DocumentLoaded {
                    revision: generation,
                    bytes: 6,
                    error: None,
                },
            ),
        }),
        &c,
    );
    let tab = st.pane.tabs().iter().find(|t| t.id == id).unwrap();
    assert!(!tab.load_state.is_active(), "正文落地后结束加载");
    assert!(tab.web_error.is_none());
    assert!(!fx.iter().any(|e| matches!(
        e,
        Effect::ScheduleLoadTimeout {
            stage: PreviewLoadStage::Reading,
            ..
        }
    )));
}

#[test]
fn editor_save_requested_emits_save_file_and_save_finished_clears_dirty() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let mut c = ctx(&caps, None);
    c.failures_path = dir.path().join("preview_failures.json");
    c.recovery_dir = Some(dir.path());
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    let binding = editor_binding(ViewerPanel::Files, id, &rs);
    let _ = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::Ready {
                    read_only: false,
                    language: "rust".into(),
                },
            ),
        }),
        &c,
    );
    let fx = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::DocumentChanged {
                    revision: generation,
                    length: 24,
                    changes: Vec::new(),
                },
            ),
        }),
        &c,
    );
    assert!(fx.is_empty(), "DocumentChanged 只标脏");
    assert!(st.pane.tabs().iter().find(|t| t.id == id).unwrap().dirty);

    let fx = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::SaveRequested {
                    revision: generation,
                    text: "fn main() { println!(); }\n".into(),
                },
            ),
        }),
        &c,
    );
    let saved = fx
        .iter()
        .find_map(|e| match e {
            Effect::SaveFile {
                path,
                text,
                revision,
                ..
            } => Some((path.clone(), text.clone(), *revision)),
            _ => None,
        })
        .expect("SaveRequested 应产出 SaveFile");
    assert_eq!(saved.0, rs);
    assert_eq!(saved.1, "fn main() { println!(); }\n");
    assert_eq!(saved.2, generation, "SaveFile 要带上发起时的 revision");

    let fx = update(
        &mut st,
        Message::SaveFinished {
            tab_id: id,
            revision: generation,
            result: Ok(()),
        },
        &c,
    );
    let tab = st.pane.tabs().iter().find(|t| t.id == id).unwrap();
    assert!(!tab.dirty);
    assert!(!tab.recovery_written);
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::ClearRecoverySnapshot { path, .. } if *path == rs
    )));
}

#[test]
fn save_finished_then_close_reports_the_real_index_and_resets_tab_first() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    let binding = editor_binding(ViewerPanel::Files, id, &rs);
    let _ = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::Ready {
                    read_only: false,
                    language: "rust".into(),
                },
            ),
        }),
        &c,
    );
    let _ = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::DocumentChanged {
                    revision: generation,
                    length: 24,
                    changes: Vec::new(),
                },
            ),
        }),
        &c,
    );
    let idx = st.pane.tabs().iter().position(|t| t.id == id).unwrap();
    let _ = update(&mut st, Message::CloseTab(idx), &c);
    assert!(st.pane.has_pending_close(id));
    st.tab_first = 3;
    let old_len = st.pane.tabs().len();
    let fx = update(
        &mut st,
        Message::SaveFinished {
            tab_id: id,
            revision: generation,
            result: Ok(()),
        },
        &c,
    );
    assert!(st.pane.tabs().iter().all(|t| t.id != id), "保存回执后关掉");
    assert_eq!(st.tab_first, 0, "关页签后 tab_first 归零");
    let closed = fx
        .iter()
        .find_map(|e| match e {
            Effect::TabClosed { index, old_len } => Some((*index, *old_len)),
            _ => None,
        })
        .expect("应报 TabClosed");
    assert_eq!(closed, (idx, old_len), "TabClosed 不能是 0/0 哑值");
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::PersistTabs { project_id: 7, .. })),
        "关页签后要持久化新状态"
    );
}

/// 写盘在途期间用户又改了正文:旧写盘的回执不能清掉新的脏标,也不能清冲突
/// 基线(D-I1)。
#[test]
fn save_finished_with_a_stale_revision_keeps_the_tab_dirty() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let mut c = ctx(&caps, None);
    c.failures_path = dir.path().join("preview_failures.json");
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    let binding = editor_binding(ViewerPanel::Files, id, &rs);
    let changed = |rev: u64| {
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                rev,
                EditorEvent::DocumentChanged {
                    revision: rev,
                    length: 24,
                    changes: Vec::new(),
                },
            ),
        })
    };
    let _ = update(&mut st, changed(generation), &c);
    // 写盘发起时 revision = generation;回执前又来了一次编辑。
    let _ = update(&mut st, changed(generation + 1), &c);
    let _ = update(
        &mut st,
        Message::SaveFinished {
            tab_id: id,
            revision: generation,
            result: Ok(()),
        },
        &c,
    );
    let tab = st.pane.tabs().iter().find(|t| t.id == id).unwrap();
    assert!(tab.dirty, "旧写盘不能清掉写盘后才产生的编辑");

    // 回执的 revision 追平当前 → 正常清脏。
    let _ = update(
        &mut st,
        Message::SaveFinished {
            tab_id: id,
            revision: generation + 1,
            result: Ok(()),
        },
        &c,
    );
    assert!(!st.pane.tabs().iter().find(|t| t.id == id).unwrap().dirty);
}

/// N2:revision 过期的回执**不能**清恢复快照——页签还脏,磁盘上的恢复文件是
/// 崩溃后唯一能找回这段时间编辑的东西。
#[test]
fn stale_save_receipt_keeps_the_recovery_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let mut c = ctx(&caps, None);
    c.recovery_dir = Some(dir.path());
    c.failures_path = dir.path().join("preview_failures.json");
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    let binding = editor_binding(ViewerPanel::Files, id, &rs);
    let changed = |rev: u64| {
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                rev,
                EditorEvent::DocumentChanged {
                    revision: rev,
                    length: 24,
                    changes: Vec::new(),
                },
            ),
        })
    };
    let _ = update(&mut st, changed(generation), &c);
    // 写盘发起后又有编辑:回执的 revision 落后于当前正文。
    let _ = update(&mut st, changed(generation + 1), &c);
    let fx = update(
        &mut st,
        Message::SaveFinished {
            tab_id: id,
            revision: generation,
            result: Ok(()),
        },
        &c,
    );
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::ClearRecoverySnapshot { .. })),
        "过期回执不能删恢复快照"
    );
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::PersistTabs { .. })),
        "过期回执不能把旧内容写进落盘快照"
    );
    let tab = st.pane.tabs().iter().find(|t| t.id == id).unwrap();
    assert!(tab.dirty, "页签仍是脏的");

    // 追平的回执才清快照。
    let fx = update(
        &mut st,
        Message::SaveFinished {
            tab_id: id,
            revision: generation + 1,
            result: Ok(()),
        },
        &c,
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::ClearRecoverySnapshot { .. })),
        "当前内容的回执应清掉恢复快照"
    );
}

/// N2:写盘期间又有编辑、且用户已点了关闭 —— 过期回执不能关页签(会丢掉新编辑)。
#[test]
fn stale_save_receipt_does_not_close_a_tab_with_newer_edits() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let mut c = ctx(&caps, None);
    c.failures_path = dir.path().join("preview_failures.json");
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    let binding = editor_binding(ViewerPanel::Files, id, &rs);
    let event = |rev: u64, event: EditorEvent| {
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(&binding, rev, event),
        })
    };
    // 编辑器就绪 → 正文变更(脏)。
    let _ = update(
        &mut st,
        event(
            generation,
            EditorEvent::Ready {
                read_only: false,
                language: "rust".into(),
            },
        ),
        &c,
    );
    let _ = update(
        &mut st,
        event(
            generation,
            EditorEvent::DocumentChanged {
                revision: generation,
                length: 24,
                changes: Vec::new(),
            },
        ),
        &c,
    );
    assert!(st.pane.tabs().iter().find(|t| t.id == id).unwrap().dirty);

    // 用户点了关闭:脏 CodeMirror 页签应进 `pending_close`,不立刻消失。
    let idx = st.pane.tabs().iter().position(|t| t.id == id).unwrap();
    let _ = update(&mut st, Message::CloseTab(idx), &c);
    assert!(st.pane.has_pending_close(id), "脏页签关闭应等保存");

    // 保存在途时又编辑了一次(envelope 与 payload 的 revision 必须一致,
    // `DocumentChanged` 才采纳——见 host_update 的 doc_revision == revision)。
    let _ = update(
        &mut st,
        event(
            generation + 1,
            EditorEvent::DocumentChanged {
                revision: generation + 1,
                length: 30,
                changes: Vec::new(),
            },
        ),
        &c,
    );
    assert_eq!(
        st.pane
            .tabs()
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .web_revision,
        generation + 1,
        "新编辑应推进 web_revision"
    );

    let fx = update(
        &mut st,
        Message::SaveFinished {
            tab_id: id,
            revision: generation,
            result: Ok(()),
        },
        &c,
    );
    assert!(
        st.pane.tabs().iter().any(|t| t.id == id),
        "过期回执不能关掉带新编辑的页签"
    );
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::TabClosed { .. })),
        "不能报 TabClosed"
    );
    assert!(
        st.pane.has_pending_close(id),
        "关闭意图保留,等下一次按新 revision 保存"
    );

    // 新的回执追平后正常关闭。
    let fx = update(
        &mut st,
        Message::SaveFinished {
            tab_id: id,
            revision: generation + 1,
            result: Ok(()),
        },
        &c,
    );
    assert!(st.pane.tabs().iter().all(|t| t.id != id), "追平后关掉");
    assert!(fx.iter().any(|e| matches!(e, Effect::TabClosed { .. })));
}

#[test]
fn editor_save_finished_error_sets_web_error() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let mut c = ctx(&caps, None);
    c.failures_path = dir.path().join("preview_failures.json");
    let mut st = PreviewPanelState::default();
    let (id, _) = open_markdown(&mut st, &c, &rs);
    let _ = update(
        &mut st,
        Message::SaveFinished {
            tab_id: id,
            revision: 0,
            result: Err("disk full".into()),
        },
        &c,
    );
    let tab = st.pane.tabs().iter().find(|t| t.id == id).unwrap();
    assert!(tab.web_error.as_deref().unwrap().contains("disk full"));
}

#[test]
fn editor_snapshot_writes_recovery_only_with_a_recovery_dir() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    let binding = editor_binding(ViewerPanel::Files, id, &rs);
    let snapshot = |rev| {
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                rev,
                EditorEvent::Snapshot {
                    revision: rev,
                    text: "fn main() {}\n".into(),
                },
            ),
        })
    };
    let fx = update(&mut st, snapshot(generation), &c);
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::WriteRecoverySnapshot { .. })),
        "没有恢复目录的产品不落快照"
    );
    let mut c2 = ctx(&caps, None);
    c2.recovery_dir = Some(dir.path());
    let fx = update(&mut st, snapshot(generation + 1), &c2);
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::WriteRecoverySnapshot { tab_id, .. } if *tab_id == id
    )));
}

#[test]
fn save_and_close_closes_even_when_the_tab_is_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    let binding = editor_binding(ViewerPanel::Files, id, &rs);
    let _ = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::Ready {
                    read_only: false,
                    language: "rust".into(),
                },
            ),
        }),
        &c,
    );
    let _ = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::DocumentChanged {
                    revision: generation,
                    length: 24,
                    changes: Vec::new(),
                },
            ),
        }),
        &c,
    );
    let tab = st.pane.tabs_mut().iter_mut().find(|t| t.id == id).unwrap();
    tab.backend = Some(PreviewBackend::Code(crate::CodeBackend {
        mode: crate::CodeMode::ReadOnly,
        language: "rust".into(),
    }));
    let idx = st.pane.tabs().iter().position(|t| t.id == id).unwrap();
    let _ = update(&mut st, Message::CloseTab(idx), &c);
    assert!(st.pane.has_pending_close(id), "脏页签应进入保存后关闭");
    let fx = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::SaveRequested {
                    revision: generation,
                    text: "fn main() {}\n".into(),
                },
            ),
        }),
        &c,
    );
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::SaveFile { .. })),
        "只读不允许写盘"
    );
    assert!(
        st.pane.tabs().iter().all(|t| t.id != id),
        "只读页签也必须关掉"
    );
    assert!(!st.pane.has_pending_close(id));
    assert!(fx.iter().any(|e| matches!(e, Effect::TabClosed { .. })));
}

#[test]
fn save_and_close_closes_even_when_there_is_a_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    let binding = editor_binding(ViewerPanel::Files, id, &rs);
    let _ = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::Ready {
                    read_only: false,
                    language: "rust".into(),
                },
            ),
        }),
        &c,
    );
    let _ = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::DocumentChanged {
                    revision: generation,
                    length: 24,
                    changes: Vec::new(),
                },
            ),
        }),
        &c,
    );
    let idx = st.pane.tabs().iter().position(|t| t.id == id).unwrap();
    let _ = update(&mut st, Message::CloseTab(idx), &c);
    assert!(st.pane.has_pending_close(id));
    let tab = st.pane.tabs_mut().iter_mut().find(|t| t.id == id).unwrap();
    tab.conflict = Some(std::time::UNIX_EPOCH);
    let fx = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::SaveRequested {
                    revision: generation,
                    text: "fn main() {}\n".into(),
                },
            ),
        }),
        &c,
    );
    assert!(!fx.iter().any(|e| matches!(e, Effect::SaveFile { .. })));
    assert!(
        st.pane.tabs().iter().all(|t| t.id != id),
        "冲突页签也必须关掉"
    );
    assert!(!st.pane.has_pending_close(id));
}

#[test]
fn save_and_close_closes_even_when_the_revision_is_stale() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    let binding = editor_binding(ViewerPanel::Files, id, &rs);
    let _ = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::Ready {
                    read_only: false,
                    language: "rust".into(),
                },
            ),
        }),
        &c,
    );
    let _ = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::DocumentChanged {
                    revision: generation,
                    length: 24,
                    changes: Vec::new(),
                },
            ),
        }),
        &c,
    );
    let idx = st.pane.tabs().iter().position(|t| t.id == id).unwrap();
    let _ = update(&mut st, Message::CloseTab(idx), &c);
    assert!(st.pane.has_pending_close(id));
    let log = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::SaveRequested {
                    revision: generation + 1,
                    text: "stale".into(),
                },
            ),
        }),
        &c,
    );
    assert!(
        !log.iter().any(|e| matches!(e, Effect::SaveFile { .. })),
        "revision 不符不写盘"
    );
    assert!(
        st.pane.tabs().iter().all(|t| t.id != id),
        "revision 不符也必须关掉"
    );
    assert!(!st.pane.has_pending_close(id));
}

#[test]
fn editor_context_menu_requested_emits_selection_context_menu() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    let binding = editor_binding(ViewerPanel::Files, id, &rs);
    let fx = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::ContextMenuRequested {
                    x: 10.0,
                    y: 20.0,
                    range: TextRange {
                        start: TextPosition { line: 1, column: 1 },
                        end: TextPosition { line: 1, column: 3 },
                    },
                    selected_text: "fn".into(),
                },
            ),
        }),
        &c,
    );
    let effect = fx
        .iter()
        .find_map(|e| match e {
            Effect::SelectionContextMenu {
                tab_id,
                path,
                x,
                y,
                selected_text,
                ..
            } => Some((*tab_id, path.clone(), *x, *y, selected_text.clone())),
            _ => None,
        })
        .expect("ContextMenuRequested 应产出 SelectionContextMenu");
    assert_eq!(effect, (id, rs, 10.0, 20.0, "fn".to_string()));
}

#[test]
fn editor_failed_bumps_the_failure_counter() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let mut c = ctx(&caps, None);
    c.failures_path = dir.path().join("preview_failures.json");
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    let binding = editor_binding(ViewerPanel::Files, id, &rs);
    for _ in 0..FAILURE_THRESHOLD {
        let _ = update(
            &mut st,
            Message::Host(HostEvent::Editor {
                binding: binding.clone(),
                envelope: editor_event(
                    &binding,
                    generation,
                    EditorEvent::Failed {
                        message: "boom".into(),
                        recoverable: true,
                    },
                ),
            }),
            &c,
        );
    }
    let raw = std::fs::read_to_string(&c.failures_path).unwrap();
    assert!(raw.contains("a.rs"), "失败计数按路径落盘: {raw}");
    let tab = st.pane.tabs().iter().find(|t| t.id == id).unwrap();
    let error = tab.web_error.as_deref().unwrap();
    assert!(error.contains("boom"), "{error}");
    assert!(error.contains('3'), "达到阈值应附连续失败次数: {error}");
}

#[test]
fn editor_event_for_a_foreign_path_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let other = file(dir.path(), "b.rs", "fn other() {}\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, _generation) = open_markdown(&mut st, &c, &rs);
    let revision = st
        .pane
        .tabs()
        .iter()
        .find(|t| t.id == id)
        .unwrap()
        .load_state
        .generation;
    // binding 与事件的 envelope 一致(都是 b.rs),但与 tab 的 a.rs 不符。
    let binding = editor_binding(ViewerPanel::Files, id, &other);
    let fx = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                revision,
                EditorEvent::Ready {
                    read_only: false,
                    language: "rust".into(),
                },
            ),
        }),
        &c,
    );
    assert!(fx.is_empty(), "{fx:?}");
    let tab = st.pane.tabs().iter().find(|t| t.id == id).unwrap();
    assert!(tab.load_state.is_active(), "路径失配的 ready 不得推进阶段");
}

/// D-I3:保存请求里的绑定路径与页签绑定路径不符时必须被拒绝——不产出 `SaveFile`,页签不变脏。
#[test]
fn save_is_rejected_when_the_event_path_differs_from_the_bound_path() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let other = file(dir.path(), "b.rs", "fn other() {}\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, Some(dir.path()));
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    // 事件自洽地指向 b.rs(binding 与 envelope 一致),但页签绑定的是 a.rs。
    let foreign = editor_binding(ViewerPanel::Files, id, &other);
    let fx = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: foreign.clone(),
            envelope: editor_event(
                &foreign,
                generation,
                EditorEvent::SaveRequested {
                    revision: generation,
                    text: "fn evil() {}\n".into(),
                },
            ),
        }),
        &c,
    );
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::SaveFile { .. })),
        "路径不符的保存不得落盘: {fx:?}"
    );
    let tab = st.pane.tabs().iter().find(|t| t.id == id).unwrap();
    assert!(!tab.dirty, "被拒绝的保存不得改动页签");
    assert_eq!(std::fs::read_to_string(&rs).unwrap(), "fn main() {}\n");
}

/// D-I3:已关闭页签的 host 事件是空操作,不能复活页签或产生效果。
#[test]
fn host_event_for_a_closed_tab_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    let binding = editor_binding(ViewerPanel::Files, id, &rs);
    let idx = st.pane.tabs().iter().position(|t| t.id == id).unwrap();
    let _ = update(&mut st, Message::CloseTab(idx), &c);
    assert!(st.pane.tabs().iter().all(|t| t.id != id));
    let before_len = st.pane.tabs().len();
    for payload in [
        EditorEvent::Ready {
            read_only: false,
            language: "rust".into(),
        },
        EditorEvent::DocumentChanged {
            revision: generation,
            length: 12,
            changes: Vec::new(),
        },
        EditorEvent::SaveRequested {
            revision: generation,
            text: "fn main() {}\n".into(),
        },
    ] {
        let fx = update(
            &mut st,
            Message::Host(HostEvent::Editor {
                binding: binding.clone(),
                envelope: editor_event(&binding, generation, payload),
            }),
            &c,
        );
        assert!(fx.is_empty(), "已关闭页签的事件不得产生效果: {fx:?}");
        assert_eq!(st.pane.tabs().len(), before_len, "不得复活页签");
    }
}

/// D-I3:比页签当前 revision 更旧的 host 事件被丢弃——旧 replay 不能把
/// `web_revision` 推回去,也不能触发保存。
#[test]
fn host_event_from_a_stale_generation_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    let binding = editor_binding(ViewerPanel::Files, id, &rs);
    // 先把页签推进到一个更新的 revision。
    let newer = generation + 5;
    let _ = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                newer,
                EditorEvent::DocumentChanged {
                    revision: newer,
                    length: 20,
                    changes: Vec::new(),
                },
            ),
        }),
        &c,
    );
    assert_eq!(
        st.pane
            .tabs()
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .web_revision,
        newer
    );

    // 一个更旧 generation 的事件。
    let fx = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::SaveRequested {
                    revision: generation,
                    text: "old".into(),
                },
            ),
        }),
        &c,
    );
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::SaveFile { .. })),
        "过期 generation 不得触发写盘: {fx:?}"
    );
    assert_eq!(
        st.pane
            .tabs()
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .web_revision,
        newer,
        "过期事件不得把 web_revision 推回去"
    );
}

/// D-I3:保存路径必须落在项目根内。根外的文件即使能打开,也不能从这里写回。
#[test]
fn save_is_rejected_outside_the_project_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let inside_file = file(dir.path(), "proj/in.rs", "fn main() {}\n");
    let outside_file = file(dir.path(), "outside/out.rs", "fn other() {}\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, Some(&root));
    let mut st = PreviewPanelState::default();

    // 根内:正常写盘。
    let (id, generation) = open_markdown(&mut st, &c, &inside_file);
    let binding = editor_binding(ViewerPanel::Files, id, &inside_file);
    let _ = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::Ready {
                    read_only: false,
                    language: "rust".into(),
                },
            ),
        }),
        &c,
    );
    let fx = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::SaveRequested {
                    revision: generation,
                    text: "fn main() { changed(); }\n".into(),
                },
            ),
        }),
        &c,
    );
    assert!(
        fx.iter().any(|e| matches!(e, Effect::SaveFile { .. })),
        "根内文件应允许保存: {fx:?}"
    );

    // 根外:拒绝,报错不给 SaveFile。
    let (out_id, out_gen) = open_markdown(&mut st, &c, &outside_file);
    let out_binding = editor_binding(ViewerPanel::Files, out_id, &outside_file);
    let _ = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: out_binding.clone(),
            envelope: editor_event(
                &out_binding,
                out_gen,
                EditorEvent::Ready {
                    read_only: false,
                    language: "rust".into(),
                },
            ),
        }),
        &c,
    );
    let fx = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: out_binding.clone(),
            envelope: editor_event(
                &out_binding,
                out_gen,
                EditorEvent::SaveRequested {
                    revision: out_gen,
                    text: "evil".into(),
                },
            ),
        }),
        &c,
    );
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::SaveFile { .. })),
        "根外文件不得写盘: {fx:?}"
    );
    let tab = st.pane.tabs().iter().find(|t| t.id == out_id).unwrap();
    assert!(
        tab.web_error.as_deref().is_some_and(|e| !e.is_empty()),
        "根外保存要给出可见错误"
    );
    assert_eq!(
        std::fs::read_to_string(&outside_file).unwrap(),
        "fn other() {}\n",
        "磁盘内容不得被改写"
    );
}

/// D-I3:保存正文超过单条消息上限时直接拒绝,不写盘、不改页签内容。
#[test]
fn oversized_text_change_is_rejected_without_changing_the_tab() {
    let dir = tempfile::tempdir().unwrap();
    let rs = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities::all();
    let c = ctx(&caps, Some(dir.path()));
    let mut st = PreviewPanelState::default();
    let (id, generation) = open_markdown(&mut st, &c, &rs);
    let binding = editor_binding(ViewerPanel::Files, id, &rs);
    let _ = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::Ready {
                    read_only: false,
                    language: "rust".into(),
                },
            ),
        }),
        &c,
    );
    let before = st
        .pane
        .tabs()
        .iter()
        .find(|t| t.id == id)
        .unwrap()
        .web_revision;
    let huge = "a".repeat(webview_protocol::MAX_MESSAGE_BYTES + 1);
    let fx = update(
        &mut st,
        Message::Host(HostEvent::Editor {
            binding: binding.clone(),
            envelope: editor_event(
                &binding,
                generation,
                EditorEvent::SaveRequested {
                    revision: generation,
                    text: huge,
                },
            ),
        }),
        &c,
    );
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::SaveFile { .. })),
        "超大正文不得写盘: {fx:?}"
    );
    let tab = st.pane.tabs().iter().find(|t| t.id == id).unwrap();
    assert!(
        tab.web_error.as_deref().is_some_and(|e| !e.is_empty()),
        "超大正文要给出可见错误"
    );
    assert_eq!(tab.web_revision, before, "页签 revision 不得被超大正文推进");
    assert_eq!(
        std::fs::read_to_string(&rs).unwrap(),
        "fn main() {}\n",
        "磁盘内容不得被改写"
    );
}

#[test]
fn take_host_scripts_skips_unpooled_webviews_and_encodes_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let md = file(dir.path(), "a.rs", "fn main() {}\n");
    let caps = PreviewCapabilities {
        rendered: false,
        ..PreviewCapabilities::all()
    };
    let c = ctx(&caps, None);
    let mut st = PreviewPanelState::default();
    let (id, _) = open_markdown(&mut st, &c, &md);
    st.pane
        .queue_editor_command(id, EditorCommand::RevealPosition { line: 1, column: 1 });
    let webview_id = EditorHostBinding::new(7, ViewerPanel::Files, id, md.clone()).webview_id();

    let scripts =
        st.pane
            .take_host_scripts(&std::collections::HashSet::new(), 7, ViewerPanel::Files);
    assert!(scripts.is_empty(), "未进池的命令必须留队");
    let scripts = st.pane.take_host_scripts(
        &std::collections::HashSet::from([webview_id]),
        7,
        ViewerPanel::Files,
    );
    assert_eq!(scripts.len(), 1, "{scripts:?}");
    assert_eq!(scripts[0].0, webview_id);
    assert!(scripts[0].1.contains("__dozer.dispatch"));
    assert!(scripts[0].1.contains("reveal_position"));
}

/// D-I3:`path_within_root` fail-closed——目标尚不存在时 canonical 它的父目录;指向根外的符号链接目录、
/// 根本不存在的根、不存在的父目录一律拒绝。
#[test]
fn path_within_root_is_fail_closed() {
    use crate::host_update::path_within_root;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(root.join("ok.txt"), "x").unwrap();
    // 根内:已存在 / 尚不存在(父目录在根内)。
    assert!(path_within_root(&root, &root.join("ok.txt")));
    assert!(path_within_root(&root, &root.join("sub/new.txt")));
    // 词法 `..`。
    assert!(!path_within_root(
        &root,
        &root.join("sub/../../outside/x.txt")
    ));
    // 根内的符号链接指向根外:已存在的文件与尚不存在的新文件都要拒绝。
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        std::fs::write(outside.join("secret.txt"), "s").unwrap();
        assert!(!path_within_root(&root, &root.join("link/secret.txt")));
        assert!(!path_within_root(&root, &root.join("link/new.txt")));
    }
    // 父目录不存在 / 根不存在:拒绝,不凭词法放行。
    assert!(!path_within_root(&root, &root.join("missing_dir/new.txt")));
    assert!(!path_within_root(
        &dir.path().join("no_such_root"),
        &dir.path().join("no_such_root/a.txt")
    ));
    // 根外。
    assert!(!path_within_root(&root, &outside.join("secret.txt")));
}
