//! Project 面板 state/update 的测试(逐字搬自 Dozer `extensions/project.rs`)。

use super::*;
use crate::disk::{DISK_USAGE_EXCLUDE, compute_blank_info, dir_size_excluding};
use crate::fakes::testing::{TIMEOUT, offline_ctx, runtime};
use crate::fakes::{FakeMemory, FakeProject, batch};
use crate::i18n::stable;
use crate::scaffold;
use dozer_panel_host::NoticeLevel;
use dozer_services::{ServiceError, ServiceErrorKind};
use std::path::PathBuf;
use std::sync::{Mutex, mpsc};

fn new_ws() -> WorkspaceState {
    WorkspaceState::default()
}

fn test_repo_path() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dozer-project-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn test_services() -> dozer_services::Services {
    crate::fakes::base_services()
}

#[test]
fn git_refreshed_updates_four_fields() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::GitRefreshed(
            1,
            Some("main".to_string()),
            true,
            vec!["https://x.git".to_string()],
        ),
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.branch.as_deref(), Some("main"));
    assert!(ws.dirty);
    assert_eq!(ws.remote_url.as_slice(), ["https://x.git"]);
}

#[test]
fn branch_and_dirty_accessors_read_current_state() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::GitRefreshed(1, Some("main".to_string()), true, vec![]),
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.branch(), Some("main"));
    assert!(ws.dirty());
}

#[test]
fn name_edit_start_prefills_current_name() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::NameEditStart,
        1,
        "旧名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.name_editing.as_deref(), Some("旧名字"));
}

#[test]
fn name_edit_submit_same_name_closes_without_request() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::NameEditStart,
        1,
        "同名",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert!(ws.name_edit_focus_pending, "点项目名应置一次性聚焦标记");
    update(
        &mut ws,
        Message::NameEditSubmit,
        1,
        "同名",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert!(ws.name_editing.is_none());
    assert!(!ws.error.is_some(), "同名不改名不应报错");
}

#[test]
fn name_renamed_ok_clears_editing_state() {
    let mut ws = new_ws();
    ws.name_editing = Some("新名字".into());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let project = dozer_model::ProjectInfo {
        id: 1,
        path: "/repo".into(),
        name: "新名字".into(),
        last_active_ms: 0,
        created_ms: 0,
        updated_ms: 0,
    };
    update(
        &mut ws,
        Message::NameRenamed(1, Ok(project)),
        1,
        "旧名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert!(ws.name_editing.is_none());
    assert!(ws.error.is_none());
}

#[test]
fn name_renamed_err_keeps_editing_state_and_sets_error() {
    let mut ws = new_ws();
    ws.name_editing = Some("新名字".into());
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::NameRenamed(1, Err(Some("连接失败".into()))),
        1,
        "旧名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.name_editing.as_deref(), Some("新名字"));
    assert!(ws.error.is_some());
}

#[test]
fn dir_size_excluding_sums_files_and_skips_excluded_dirs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "12345").unwrap(); // 5 bytes
    std::fs::create_dir(dir.path().join("target")).unwrap();
    std::fs::write(dir.path().join("target").join("big.bin"), vec![0u8; 1000]).unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src").join("b.txt"), "12").unwrap(); // 2 bytes
    let size = dir_size_excluding(dir.path(), &DISK_USAGE_EXCLUDE);
    assert_eq!(size, 7); // 5 + 2, target 整个跳过
}

#[test]
fn dir_size_excluding_empty_dir_is_zero() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(dir_size_excluding(dir.path(), &DISK_USAGE_EXCLUDE), 0);
}

#[test]
fn disk_usage_loaded_sets_field() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::DiskUsageLoaded(1, 12345),
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.disk_usage_bytes, Some(12345));
}

#[test]
fn description_edit_start_prefills_from_field() {
    let mut ws = new_ws();
    ws.description = Some("已有描述".to_string());
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::DescriptionEditStart,
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    let content = ws.description_editing.expect("进入编辑态");
    assert_eq!(content.text(), "已有描述");
}

#[test]
fn description_edit_submit_writes_to_disk_and_clears_editing() {
    let mut ws = new_ws();
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let action = iced_widget::text_editor::Action::Edit(iced_widget::text_editor::Edit::Paste(
        std::sync::Arc::new("新描述内容".to_string()),
    ));
    update(
        &mut ws,
        Message::DescriptionEditStart,
        1,
        "名字",
        dir.path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    update(
        &mut ws,
        Message::DescriptionEditAction(action),
        1,
        "名字",
        dir.path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    update(
        &mut ws,
        Message::DescriptionEditSubmit,
        1,
        "名字",
        dir.path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert!(ws.description_editing.is_none());
    assert_eq!(ws.description.as_deref(), Some("新描述内容"));
    let on_disk = crate::meta::load_description(dir.path());
    assert_eq!(on_disk.as_deref(), Some("新描述内容"));
}

#[test]
fn description_edit_submit_empty_clears_field_and_file() {
    let mut ws = new_ws();
    let dir = tempfile::tempdir().unwrap();
    crate::meta::write_description(dir.path(), "旧描述").unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::DescriptionEditStart,
        1,
        "名字",
        dir.path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    // 清空编辑内容:直接提交空编辑器。
    update(
        &mut ws,
        Message::DescriptionEditSubmit,
        1,
        "名字",
        dir.path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert!(ws.description_editing.is_none());
    assert!(ws.description.is_none());
    assert!(crate::meta::load_description(dir.path()).is_none());
}

#[test]
fn link_add_appends_and_writes_disk() {
    let mut ws = new_ws();
    let repo = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::LinkAdd {
            target: links::LinkTarget::Docs,
            path: PathBuf::from("/repo/README.md"),
            kind: links::LinkKind::File,
        },
        1,
        "名字",
        repo.path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.links.docs.len(), 1);
    let loaded = links::load(repo.path()).unwrap();
    assert_eq!(loaded.docs.len(), 1);
}

#[test]
fn link_add_dedupes_existing_path() {
    let mut ws = new_ws();
    ws.links.docs.push(links::LinkEntry {
        path: PathBuf::from("/repo/README.md"),
        kind: links::LinkKind::File,
    });
    let repo = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::LinkAdd {
            target: links::LinkTarget::Docs,
            path: PathBuf::from("/repo/README.md"),
            kind: links::LinkKind::File,
        },
        1,
        "名字",
        repo.path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.links.docs.len(), 1);
}

#[test]
fn link_remove_deletes_and_writes_disk() {
    let mut ws = new_ws();
    ws.links.docs.push(links::LinkEntry {
        path: PathBuf::from("/repo/README.md"),
        kind: links::LinkKind::File,
    });
    let repo = tempfile::tempdir().unwrap();
    links::save(repo.path(), &ws.links).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::LinkRemove {
            target: links::LinkTarget::Docs,
            index: 0,
        },
        1,
        "名字",
        repo.path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert!(ws.links.docs.is_empty());
    assert!(links::load(repo.path()).unwrap().docs.is_empty());
}

#[test]
fn link_dir_toggle_expands_then_collapses() {
    let mut ws = new_ws();
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir(repo.path().join("docs")).unwrap();
    std::fs::write(repo.path().join("docs").join("a.md"), "").unwrap();
    let docs_path = repo.path().join("docs");
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::LinkDirToggle {
            target: links::LinkTarget::Docs,
            path: docs_path.clone(),
        },
        1,
        "名字",
        repo.path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(
        ws.expanded_link_dirs.get(&docs_path).map(|r| r.len()),
        Some(1)
    );
    update(
        &mut ws,
        Message::LinkDirToggle {
            target: links::LinkTarget::Docs,
            path: docs_path.clone(),
        },
        1,
        "名字",
        repo.path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert!(!ws.expanded_link_dirs.contains_key(&docs_path));
}

#[test]
fn link_dir_toggle_selects_the_entry() {
    let mut ws = new_ws();
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir(repo.path().join("docs")).unwrap();
    let docs_path = repo.path().join("docs");
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::LinkDirToggle {
            target: links::LinkTarget::Docs,
            path: docs_path.clone(),
        },
        1,
        "名字",
        repo.path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.selected_link.as_deref(), Some(docs_path.as_path()));
}

#[test]
fn link_select_only_marks_selection_without_expanding() {
    let mut ws = new_ws();
    let repo = tempfile::tempdir().unwrap();
    let child = repo.path().join("docs").join("sub");
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::LinkSelect {
            target: links::LinkTarget::Docs,
            path: child.clone(),
        },
        1,
        "名字",
        repo.path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.selected_link.as_deref(), Some(child.as_path()));
    assert!(ws.expanded_link_dirs.is_empty());
}

#[test]
fn scaffold_done_is_a_pure_noop() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::ScaffoldDone,
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert!(ws.scaffold_run.is_none());
}

#[test]
fn repair_project_sets_pending_scaffold_run() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::RepairProject,
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    let run = ws.scaffold_run.expect("弹窗状态应已初始化");
    assert_eq!(run.steps.len(), 5);
    assert!(
        run.steps
            .iter()
            .all(|(_, s)| matches!(s, ScaffoldStepState::Pending))
    );
    assert_eq!(run.backfill, BackfillStepState::Pending);
}

#[test]
fn scaffold_step_started_then_finished_updates_that_step_only() {
    let mut ws = new_ws();
    ws.scaffold_run = Some(ScaffoldRunState::pending(true));
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::ScaffoldStepStarted(1, 1),
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    let run = ws.scaffold_run.as_ref().unwrap();
    assert_eq!(run.steps[0].1, ScaffoldStepState::Pending);
    assert_eq!(run.steps[1].1, ScaffoldStepState::Running);

    update(
        &mut ws,
        Message::ScaffoldStepFinished(1, 1, scaffold::ScaffoldStepResult::AlreadyOk),
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    let run = ws.scaffold_run.as_ref().unwrap();
    assert_eq!(
        run.steps[1].1,
        ScaffoldStepState::Done(scaffold::ScaffoldStepResult::AlreadyOk)
    );
}

#[test]
fn transcript_backfill_started_then_finished_updates_last_step_only() {
    let mut ws = new_ws();
    ws.scaffold_run = Some(ScaffoldRunState::pending(true));
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::TranscriptBackfillStarted(1),
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    let run = ws.scaffold_run.as_ref().unwrap();
    let last = run.steps.len() - 1;
    assert_eq!(run.steps[last].1, ScaffoldStepState::Running);
    assert_eq!(run.steps[0].1, ScaffoldStepState::Pending);

    update(
        &mut ws,
        Message::TranscriptBackfillFinished(
            1,
            scaffold::ScaffoldStepResult::Created("导入 3 个历史文件".into()),
        ),
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    let run = ws.scaffold_run.as_ref().unwrap();
    assert_eq!(
        run.steps[last].1,
        ScaffoldStepState::Done(scaffold::ScaffoldStepResult::Created(
            "导入 3 个历史文件".into()
        ))
    );
}

#[test]
fn summary_backfill_progress_marks_done_when_completed_reaches_total() {
    let mut ws = new_ws();
    ws.scaffold_run = Some(ScaffoldRunState::pending(true));
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::SummaryBackfillProgress(1, 2, 5, 2, 0, 0),
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(
        ws.scaffold_run.as_ref().unwrap().backfill,
        BackfillStepState::Running(BackfillProgress {
            completed: 2,
            total: 5,
            succeeded: 2,
            failed: 0,
            skipped: 0,
        })
    );

    update(
        &mut ws,
        Message::SummaryBackfillProgress(1, 5, 5, 5, 0, 0),
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(
        ws.scaffold_run.as_ref().unwrap().backfill,
        BackfillStepState::Done(BackfillProgress {
            completed: 5,
            total: 5,
            succeeded: 5,
            failed: 0,
            skipped: 0,
        })
    );
}

#[test]
fn summary_backfill_progress_of_zero_total_is_immediately_done() {
    let mut ws = new_ws();
    ws.scaffold_run = Some(ScaffoldRunState::pending(true));
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::SummaryBackfillProgress(1, 0, 0, 0, 0, 0),
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(
        ws.scaffold_run.as_ref().unwrap().backfill,
        BackfillStepState::Done(BackfillProgress {
            completed: 0,
            total: 0,
            succeeded: 0,
            failed: 0,
            skipped: 0,
        })
    );
}

#[test]
fn popup_close_only_clears_state_when_all_done() {
    let mut ws = new_ws();
    ws.scaffold_run = Some(ScaffoldRunState::pending(true));
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::ScaffoldPopupClose,
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert!(ws.scaffold_run.is_some(), "未完成时点关闭不应清空状态");
}

#[test]
fn popup_close_clears_state_once_all_steps_and_backfill_are_done() {
    let mut ws = new_ws();
    let mut run = ScaffoldRunState::pending(true);
    for (_, state) in run.steps.iter_mut() {
        *state = ScaffoldStepState::Done(scaffold::ScaffoldStepResult::AlreadyOk);
    }
    run.backfill = BackfillStepState::Done(BackfillProgress {
        completed: 0,
        total: 0,
        succeeded: 0,
        failed: 0,
        skipped: 0,
    });
    ws.scaffold_run = Some(run);
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::ScaffoldPopupClose,
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert!(ws.scaffold_run.is_none());
}

#[test]
fn delete_project_request_then_scope_select_then_cancel() {
    let mut ws_state = WorkspaceState::new(None, links::LinksState::default());
    let noop_services = crate::fakes::base_services();
    let handle = tokio::runtime::Handle::try_current()
        .unwrap_or_else(|_| tokio::runtime::Runtime::new().unwrap().handle().clone());
    let repo = std::path::Path::new("/tmp/demo");

    update(
        &mut ws_state,
        Message::DeleteProjectRequest,
        1,
        "demo",
        repo,
        &noop_services,
        &handle,
        |_| {},
    );
    assert_eq!(
        ws_state.delete_pending,
        Some(delete::DeleteScope::DozerOnly)
    );

    update(
        &mut ws_state,
        Message::DeleteProjectScopeSelect(delete::DeleteScope::WithProjectFiles),
        1,
        "demo",
        repo,
        &noop_services,
        &handle,
        |_| {},
    );
    assert_eq!(
        ws_state.delete_pending,
        Some(delete::DeleteScope::WithProjectFiles)
    );

    update(
        &mut ws_state,
        Message::DeleteProjectCancel,
        1,
        "demo",
        repo,
        &noop_services,
        &handle,
        |_| {},
    );
    assert_eq!(ws_state.delete_pending, None);
}

/// 空白页信息卡:递归时跳过 `DISK_USAGE_EXCLUDE`(`target/`/`.git/`/
/// `node_modules` 等),只算真实项目内容。构造 tempdir 写 3 个文件——1
/// 个普通,2 个分别落在 `target/` 与 `.git/`,断言 size/count 只统计
/// 普通那一个。
#[test]
fn compute_blank_info_excludes_git_target_node_modules() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.txt"), b"hi").expect("write a.txt"); // 2B
    std::fs::create_dir(dir.path().join("target")).expect("mkdir target");
    std::fs::write(dir.path().join("target").join("junk"), b"xxxxxx").expect("write junk"); // 6B
    std::fs::create_dir_all(dir.path().join(".git").join("objects")).expect("mkdir .git");
    std::fs::write(dir.path().join(".git").join("HEAD"), b"junk").expect("write HEAD");
    // 嵌套不计入但要确认递归子树仍被正确剪枝:
    std::fs::create_dir_all(dir.path().join("node_modules").join("pkg"))
        .expect("mkdir node_modules");
    std::fs::write(
        dir.path().join("node_modules").join("pkg").join("index.js"),
        b"xxxxxx",
    )
    .expect("write");

    let info = compute_blank_info(dir.path());
    assert_eq!(info.path, dir.path());
    assert_eq!(info.size_bytes, 2, "只 a.txt 该计入");
    assert_eq!(info.file_count, 1, "只 a.txt 该计入");
}

/// 空白页信息卡:不存在路径不应 panic,view 端对应渲染 "—"。
#[test]
fn compute_blank_info_missing_root_returns_zeros() {
    let info = compute_blank_info(std::path::Path::new("/definitely/not/here/please"));
    assert_eq!(info.size_bytes, 0);
    assert_eq!(info.file_count, 0);
    assert!(info.created.is_none());
    assert!(info.modified.is_none());
}

fn sample_memory_detail() -> dozer_model::MemoryDetail {
    dozer_model::MemoryDetail {
        id: 1,
        project_id: 1,
        title: "t".into(),
        kind: "note".into(),
        description: "d".into(),
        body: "b".into(),
        created_ms: 0,
        created_by: "user".into(),
        updated_ms: 0,
        updated_by: "user".into(),
        history: Vec::new(),
    }
}

#[test]
fn memory_detail_message_maps_ok_and_err() {
    assert!(matches!(
        memory_detail_message(Ok::<_, String>(sample_memory_detail())),
        Message::MemoryDetailLoaded(_)
    ));
    match memory_detail_message(Err::<dozer_model::MemoryDetail, _>("连接被拒")) {
        Message::MemoryDetailLoadFailed(e) => assert_eq!(e, "连接被拒"),
        other => panic!("期望 MemoryDetailLoadFailed,得到 {other:?}"),
    }
}

#[test]
fn memory_detail_load_failed_sets_the_inline_error() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    update(
        &mut ws,
        Message::MemoryDetailLoadFailed("连接被拒".to_string()),
        1,
        "名字",
        &test_repo_path(),
        &test_services(),
        rt.handle(),
        |_| {},
    );
    assert_eq!(
        ws.error.as_ref().map(crate::i18n::resolve_zh),
        Some("读取记忆失败: 连接被拒".to_string())
    );
    assert!(ws.memory_detail.is_none(), "读取失败不应留下半截详情");
}

// ---- H6:切入钩子 ----

#[test]
fn on_activate_requests_the_shared_memories() {
    let rt = runtime();
    let (ctx, rx) = offline_ctx::<Message>(&rt, 7, None);
    let ctx = dozer_panel_host::ActivationCtx {
        io: ctx.io.with_services(FakeMemory::default().into_services()),
        ..ctx
    };
    on_activate(&WorkspaceState::default(), &ctx);
    assert!(matches!(
        rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
        Message::MemoriesLoaded(_)
    ));
}

fn collect() -> (
    impl Fn(Message) + Send + Sync + 'static,
    mpsc::Receiver<Message>,
) {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    (
        move |m| {
            let _ = tx.lock().unwrap().send(m);
        },
        rx,
    )
}

fn memory_info(id: i64, title: &str) -> dozer_model::MemoryInfo {
    dozer_model::MemoryInfo {
        id,
        project_id: 1,
        title: title.into(),
        kind: "project".into(),
        description: String::new(),
        updated_ms: 0,
        updated_by: "user".into(),
    }
}

#[test]
fn unavailable_memory_read_failure_keeps_the_list_and_stays_silent() {
    let mut ws = WorkspaceState::new(None, links::LinksState::default());
    let rt = runtime();
    let services = FakeMemory::default().into_services();
    let (e1, _r1) = collect();
    update(
        &mut ws,
        Message::MemoriesLoaded(vec![memory_info(1, "a")]),
        1,
        "demo",
        std::path::Path::new("/tmp/demo"),
        &services,
        rt.handle(),
        e1,
    );
    let (e2, _r2) = collect();
    update(
        &mut ws,
        Message::MemoriesLoadFailed(ServiceErrorKind::Unavailable, "daemon 断开".into()),
        1,
        "demo",
        std::path::Path::new("/tmp/demo"),
        &services,
        rt.handle(),
        e2,
    );
    assert_eq!(ws.memories().len(), 1, "读失败不得清空已有记忆");
    assert!(
        ws.take_outbox().is_empty(),
        "Unavailable 由顶栏徽标表达,不弹 Toast"
    );
}

#[test]
fn repeated_rejected_memory_read_failures_toast_once_per_text() {
    let mut ws = WorkspaceState::new(None, links::LinksState::default());
    let rt = runtime();
    let services = FakeMemory::default().into_services();
    for _ in 0..3 {
        let (e, _r) = collect();
        update(
            &mut ws,
            Message::MemoriesLoadFailed(ServiceErrorKind::Rejected, "daemon 错误: boom".into()),
            1,
            "demo",
            std::path::Path::new("/tmp/demo"),
            &services,
            rt.handle(),
            e,
        );
    }
    let toasts = ws.take_outbox();
    assert_eq!(toasts.len(), 1, "相同文本 60 秒内只提示一次");
    assert_eq!(toasts[0].level, NoticeLevel::Error);
    assert!(stable(&toasts[0].text).contains("daemon 错误: boom"));
    assert_eq!(toasts[0].key.as_deref(), Some("project-memories-load"));
    let (e, _r) = collect();
    update(
        &mut ws,
        Message::MemoriesLoadFailed(ServiceErrorKind::Protocol, "另一种错误".into()),
        1,
        "demo",
        std::path::Path::new("/tmp/demo"),
        &services,
        rt.handle(),
        e,
    );
    assert_eq!(ws.take_outbox().len(), 1, "换了文本要再提示");
}

#[test]
fn failing_memory_list_emits_load_failed_instead_of_an_empty_loaded() {
    let rt = runtime();
    let memory = FakeMemory::failing_list(ServiceErrorKind::Unavailable, "x")
        .into_services()
        .memory;
    let (emit, rx) = collect();
    request_memories_refresh(&Default::default(), 1, &memory, rt.handle(), emit);
    assert!(
        matches!(
            rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
            Message::MemoriesLoadFailed(ServiceErrorKind::Unavailable, _)
        ),
        "读失败必须发 MemoriesLoadFailed,不得发 MemoriesLoaded(vec![])"
    );
}

#[test]
fn memory_write_failure_text_is_the_service_message_unchanged() {
    let rt = runtime();
    let fake = FakeMemory::failing_writes(ServiceErrorKind::Rejected, "daemon 错误: boom");
    let services = fake.clone().into_services();
    let mut ws = WorkspaceState::new(None, links::LinksState::default());
    let repo = std::path::Path::new("/tmp/demo");
    let (e, _r) = collect();
    update(
        &mut ws,
        Message::MemoryCreateStart,
        1,
        "demo",
        repo,
        &services,
        rt.handle(),
        e,
    );
    let (e, _r) = collect();
    update(
        &mut ws,
        Message::MemoryCreateTitleInput("标题".into()),
        1,
        "demo",
        repo,
        &services,
        rt.handle(),
        e,
    );
    let (e, rx) = collect();
    update(
        &mut ws,
        Message::MemoryCreateSubmit,
        1,
        "demo",
        repo,
        &services,
        rt.handle(),
        e,
    );
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::MemoryMutated(Err(text)) => assert_eq!(text, "daemon 错误: boom"),
        other => panic!("意外消息: {other:?}"),
    }
    assert_eq!(fake.calls(), vec!["write_memory(1,标题)".to_string()]);
}

#[test]
fn renaming_to_a_missing_project_reports_it_does_not_exist() {
    let rt = runtime();
    let project = FakeProject::default().into_services().project; // rename → Ok(None)
    let mut ws = WorkspaceState::new(None, links::LinksState::default());
    ws.name_editing = Some("新名".into());
    let (emit, rx) = collect();
    submit_name_edit(&mut ws, 1, "旧名", project, rt.handle(), emit);
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::NameRenamed(1, Err(None)) => {}
        other => panic!("意外消息: {other:?}"),
    }
}

/// 收集 `spawn_repair_run` 的全部消息,直到出现 `SummaryBackfillFailed` 或批次全部完成的 `SummaryBackfillProgress`。
fn run_repair(project: FakeProject) -> Vec<Message> {
    let rt = runtime();
    let dir = tempfile::tempdir().unwrap();
    let (emit, rx) = collect();
    spawn_repair_run(
        dir.path().to_path_buf(),
        1,
        Some(project.into_services().transcript),
        rt.handle(),
        emit,
    );
    let mut out = Vec::new();
    loop {
        let m = rx.recv_timeout(TIMEOUT).expect("修复流程应在超时内结束");
        let done = matches!(&m, Message::SummaryBackfillFailed(..))
            || matches!(&m, Message::SummaryBackfillProgress(_, completed, total, ..) if completed >= total);
        out.push(m);
        if done {
            return out;
        }
    }
}

#[test]
fn repair_run_success_reports_the_import_count_and_final_progress() {
    let project = FakeProject::default()
        .with_backfill(Ok(2))
        .with_submit(Ok((7, 3)))
        .with_batches(vec![Ok(Some(batch(3, 0, 0, 3, 0, 0)))]);
    let msgs = run_repair(project);
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            Message::TranscriptBackfillFinished(1, scaffold::ScaffoldStepResult::Created(t))
                if stable(t) == "Imported 2 history files"
        )),
        "{msgs:?}"
    );
    assert!(
        matches!(
            msgs.last().unwrap(),
            Message::SummaryBackfillProgress(1, 3, 3, 3, 0, 0)
        ),
        "{msgs:?}"
    );
}

#[test]
fn repair_run_reports_a_missing_batch() {
    let project = FakeProject::default().with_batches(vec![Ok(None)]);
    let msgs = run_repair(project);
    assert!(
        matches!(msgs.last().unwrap(),
            Message::SummaryBackfillFailed(1, t)
                if stable(t)
                    == "Summary batch not found, please retry"),
        "{msgs:?}"
    );
}

#[test]
fn repair_run_reports_a_failed_batch_query_with_the_service_text() {
    let project = FakeProject::default().with_batches(vec![Err(ServiceError::new(
        ServiceErrorKind::Rejected,
        "daemon 错误: nope",
    ))]);
    let msgs = run_repair(project);
    assert!(
        matches!(msgs.last().unwrap(),
            Message::SummaryBackfillFailed(1, t)
                if stable(t) == "Failed to summarize: daemon 错误: nope"),
        "{msgs:?}"
    );
}

#[test]
fn scaffold_run_still_finishes_when_the_backfill_fails() {
    let rt = runtime();
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeProject::default().with_backfill(Err(ServiceError::new(
        ServiceErrorKind::Unavailable,
        "daemon 断开",
    )));
    let (emit, rx) = collect();
    spawn_scaffold_run(
        dir.path().to_path_buf(),
        Some(fake.clone().into_services().transcript),
        rt.handle(),
        emit,
        true,
    );
    assert!(matches!(
        rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
        Message::ScaffoldDone
    ));
    assert_eq!(fake.calls().len(), 1, "补录失败不阻止流程结束,也不重试");
}

#[test]
fn delete_stops_after_a_failed_deregistration() {
    let rt = runtime();
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeProject::default()
        .with_remove(Err(ServiceError::new(ServiceErrorKind::Rejected, "boom")));
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    delete::spawn_delete_project(
        1,
        dir.path().to_path_buf(),
        delete::DeleteScope::WithAgentCache,
        fake.clone().into_services().project,
        Some(fake.clone().into_services().transcript),
        |_, repo| repo.join(".agent-cache"),
        rt.handle(),
        move |id, dereg, errors| {
            let _ = tx.lock().unwrap().send((id, dereg, errors));
        },
    );
    let (id, dereg, errors) = rx.recv_timeout(TIMEOUT).unwrap();
    assert_eq!((id, dereg), (1, false));
    assert_eq!(errors, vec!["Deregister project: boom".to_string()]);
    assert_eq!(
        fake.calls(),
        vec!["remove_project(1)".to_string()],
        "不得继续删转录"
    );
}

#[test]
fn stale_memory_detail_cannot_replace_new_selection_or_reopen_closed_detail() {
    let rt = runtime();
    let services = test_services();
    let repo = test_repo_path();
    let mut state = new_ws();
    let apply = |state: &mut WorkspaceState, msg| {
        update(
            state,
            msg,
            1,
            "project",
            &repo,
            &services,
            rt.handle(),
            |_| {},
        )
    };
    let old = state.requests.begin(Some(1), "detail");
    let new = state.requests.begin(Some(1), "detail");
    let mut detail = sample_memory_detail();
    detail.id = 2;
    apply(
        &mut state,
        Message::AsyncResult(
            new.clone(),
            Box::new(Message::MemoryDetailLoaded(detail.clone())),
        ),
    );
    apply(
        &mut state,
        Message::AsyncResult(
            old.clone(),
            Box::new(Message::MemoryDetailLoaded(sample_memory_detail())),
        ),
    );
    apply(
        &mut state,
        Message::AsyncResult(
            old,
            Box::new(Message::MemoryDetailLoadFailed("old failure".into())),
        ),
    );
    assert_eq!(state.memory_detail.as_ref().unwrap().id, 2);
    assert!(state.error.is_none());
    apply(&mut state, Message::MemoryDetailClose);
    apply(
        &mut state,
        Message::AsyncResult(new, Box::new(Message::MemoryDetailLoaded(detail))),
    );
    assert!(state.memory_detail.is_none());
    let old_list = state.requests.begin(Some(1), "memories");
    let new_list = state.requests.begin(Some(1), "memories");
    apply(
        &mut state,
        Message::AsyncResult(
            new_list.clone(),
            Box::new(Message::MemoriesLoaded(vec![memory_info(2, "new")])),
        ),
    );
    apply(
        &mut state,
        Message::AsyncResult(old_list, Box::new(Message::MemoriesLoaded(vec![]))),
    );
    assert_eq!(state.memories[0].id, 2);
    let mut reopened = new_ws();
    apply(
        &mut reopened,
        Message::AsyncResult(
            new_list,
            Box::new(Message::MemoriesLoaded(vec![memory_info(
                3,
                "old workspace",
            )])),
        ),
    );
    assert!(reopened.memories.is_empty());
}

#[test]
fn rename_to_blank_name_is_rejected_without_touching_the_registry() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let fake = FakeProject::default();
    let services = fake.clone().into_services();
    ws.name_editing = Some("   ".to_string());
    update(
        &mut ws,
        Message::NameEditSubmit,
        1,
        "名字",
        &test_repo_path(),
        &services,
        rt.handle(),
        |_| {},
    );
    assert!(ws.name_editing.is_none());
    assert!(fake.calls().is_empty());
}

#[test]
fn stale_async_result_from_previous_project_is_dropped() {
    let mut ws = new_ws();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let services = test_services();
    let (tx, rx) = mpsc::channel::<Message>();
    request_memories_refresh(&ws.requests, 1, &services.memory, rt.handle(), move |m| {
        let _ = tx.send(m);
    });
    let stale = rx.recv_timeout(TIMEOUT).expect("旧项目的记忆结果应当到达");
    assert!(matches!(
        stale.clone().result_payload(),
        Message::MemoriesLoaded(_)
    ));

    let before = ws.memories.clone();
    update(
        &mut ws,
        stale,
        2,
        "名字",
        &test_repo_path(),
        &services,
        rt.handle(),
        |_| {},
    );
    assert_eq!(ws.memories, before);
}

#[test]
fn scaffold_and_repair_without_transcript_service_skip_backfill_and_still_finish() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let fake = FakeProject::default();

    let (tx, rx) = mpsc::channel::<Message>();
    spawn_scaffold_run(
        test_repo_path(),
        None,
        rt.handle(),
        move |m| {
            let _ = tx.send(m);
        },
        true,
    );
    let scaffold_done = std::iter::from_fn(|| rx.recv_timeout(TIMEOUT).ok())
        .any(|m| matches!(m, Message::ScaffoldDone));
    assert!(scaffold_done, "无转录服务时脚手架仍须发出 ScaffoldDone");

    let step_count = scaffold::scaffold_steps().len();
    let (tx, rx) = mpsc::channel::<Message>();
    spawn_repair_run(test_repo_path(), 1, None, rt.handle(), move |m| {
        let _ = tx.send(m);
    });
    let seen: Vec<Message> = std::iter::from_fn(|| rx.recv_timeout(TIMEOUT).ok()).collect();
    assert_eq!(
        seen.len(),
        step_count * 2,
        "无转录服务时修复只应发出各步骤的开始/结束: {seen:?}"
    );
    assert!(
        fake.calls()
            .iter()
            .all(|c| !c.starts_with("backfill(") && !c.starts_with("submit_summary_batch(")),
        "无转录服务时不应触发补录/总结批次: {:?}",
        fake.calls()
    );
}
