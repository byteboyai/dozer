//! `update` 产出的效果。纯数据;`run_effect` 执行其中「本地 IO / 后台任务」的那一类并把结果以
//! [`Message`] 回投。需要产品参与的效果(白名单、上下文推送、悬停清理……)是**通知**,
//! 由适配层自行处理,`run_effect` 对它们是 no-op。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::tabular::SheetLoadRequest;
use crate::{Message, PreviewLoadStage, PreviewPersistence, PreviewState, ViewerPanel};

#[derive(Debug, Clone)]
pub enum Effect {
    // —— 库执行 ——
    /// 后台画像(有界采样),结果 `Message::Profiled`。
    ProbeProfile {
        tab_id: usize,
        generation: u64,
        path: PathBuf,
    },
    /// 睡 `stage_timeout(stage)` 后回投 `Message::LoadTimeout`。
    ScheduleLoadTimeout {
        tab_id: usize,
        generation: u64,
        stage: PreviewLoadStage,
    },
    /// 读脏 tab 的未保存快照并与磁盘分类,结果 `Message::RecoveryRead`。
    ReadRecovery {
        project_id: i64,
        tab_id: usize,
        generation: u64,
        path: PathBuf,
        cancel: Arc<AtomicBool>,
    },
    ClearRecovery {
        project_id: i64,
        path: PathBuf,
    },
    /// 为窗口化 tab 建稀疏行索引,结果 `Message::WindowIndex`。
    StartWindowIndex {
        tab_id: usize,
        path: PathBuf,
        cancel: Arc<AtomicBool>,
    },
    /// 表格首次解析(需 `tabular-loader`),结果 `Message::TabularLoaded`。
    LoadTabular {
        tab_id: usize,
        generation: u64,
        path: PathBuf,
        cancel: Arc<AtomicBool>,
    },
    /// 表格某个 sheet 的懒加载,结果 `Message::TabularSheetLoaded`。
    LoadTabularSheet {
        tab_id: usize,
        request: SheetLoadRequest,
        cancel: Arc<AtomicBool>,
    },
    /// PlantUML 授权读取,结果 `Message::PlantUmlLoaded`。
    LoadPlantUml {
        tab_id: usize,
        generation: u64,
        project_root: PathBuf,
        path: PathBuf,
    },
    /// 整文件流式搜索,进度 / 结果 `Message::LargeFileSearch{Progress,Results}`。
    StartLargeFileSearch {
        tab_id: usize,
        generation: u64,
        path: PathBuf,
        query: String,
        cancel: Arc<AtomicBool>,
    },
    /// 交给系统默认应用打开(不执行文件本身);失败回投 `Message::OpenExternalFailed`。
    OpenExternal {
        path: PathBuf,
    },
    /// 把编辑器缓冲原子写回绑定路径(`save_text_atomic`),结果 `Message::SaveFinished`。
    /// 路径来自 Rust 侧页签绑定,不采信页面自报;`run_effect` 不校验项目根(Task 2
    /// 的校验在 `update` 里做,委托方拿到的已经是校验过的路径)。
    SaveFile {
        tab_id: usize,
        path: PathBuf,
        text: String,
    },
    /// 把一段已编码的注入脚本交给产品的 webview 宿主执行(恢复正文 / 视图状态)。
    /// 页面事件处理体排出的「立即注入一条 host 脚本」。宿主每帧调
    /// `PreviewPane::take_host_scripts` 时把这里排的一并取走。
    PushHostScript {
        webview_id: usize,
        script: String,
    },
    /// 后台建窗口化行索引,结果 `Message::WindowIndex`。
    BuildLineIndex {
        tab_id: usize,
        path: PathBuf,
        revision: u64,
        cancel: Arc<AtomicBool>,
    },
    /// 防抖脏快照原子写 recovery;成功回 `Message::RecoveryWritten`。
    WriteRecoverySnapshot {
        dir: PathBuf,
        project_id: i64,
        path: PathBuf,
        revision: u64,
        cursor: Option<crate::TextPosition>,
        selection: Option<crate::TextRange>,
        top_line: Option<u32>,
        text: String,
        tab_id: usize,
    },
    /// 保存成功后清掉该文件的 recovery 快照。
    ClearRecoverySnapshot {
        dir: PathBuf,
        project_id: i64,
        path: PathBuf,
    },
    /// 异步写页签持久化状态。
    PersistTabs {
        project_id: i64,
        state: PreviewState,
    },
    // —— 通知产品 ——
    /// 协议白名单:用户显式打开过的路径(P3 起白名单的所有权收进协议处理器)。
    AllowFile(PathBuf),
    /// 一个路径刚被打开 / 选中(Files:项目树同步选中;Project:项目链接同步选中)。
    PathOpened(PathBuf),
    /// 预览上下文变了(MCP 上下文推送)。`immediate` = 不等防抖窗口。
    ContextChanged {
        immediate: bool,
    },
    /// 页签被选中(产品据此武装拖拽换位)。
    TabSelected {
        index: usize,
    },
    /// 页签被关闭(产品据此清理悬停记录)。
    TabClosed {
        index: usize,
        old_len: usize,
    },
    /// 预览内选区右键:坐标是 webview 本地坐标,产品自己换算窗口坐标并弹菜单
    /// (Dozer 有「复制 / 发送给 Agent」,Digger 第一批忽略)。
    SelectionContextMenu {
        panel: ViewerPanel,
        tab_id: usize,
        path: PathBuf,
        x: f32,
        y: f32,
        range: crate::TextRange,
        selected_text: String,
    },
}

/// 执行一个效果。`emit` 把结果消息送回产品的事件环;`handle` 是产品的 tokio 运行时。
pub fn run_effect(
    effect: Effect,
    emit: impl Fn(Message) + Send + Sync + 'static,
    handle: &tokio::runtime::Handle,
    persistence: Arc<dyn PreviewPersistence>,
) {
    let emit = Arc::new(emit);
    match effect {
        Effect::ProbeProfile {
            tab_id,
            generation,
            path,
        } => {
            handle.spawn_blocking(move || {
                let result = crate::profile_file(&path).map_err(|e| e.to_string());
                emit(Message::Profiled {
                    tab_id,
                    generation,
                    result,
                });
            });
        }
        Effect::ScheduleLoadTimeout {
            tab_id,
            generation,
            stage,
        } => {
            let duration = crate::stage_timeout(stage);
            handle.spawn(async move {
                tokio::time::sleep(duration).await;
                emit(Message::LoadTimeout {
                    tab_id,
                    generation,
                    stage,
                });
            });
        }
        Effect::ReadRecovery {
            project_id,
            tab_id,
            generation,
            path,
            cancel,
        } => {
            let dir = persistence.recovery_dir();
            handle.spawn(async move {
                let restore = tokio::task::spawn_blocking(move || {
                    read_recovery(&dir, project_id, &path, &cancel)
                })
                .await
                .unwrap_or(None);
                emit(Message::RecoveryRead {
                    tab_id,
                    generation,
                    restore,
                });
            });
        }
        Effect::ClearRecovery { project_id, path } => {
            let dir = persistence.recovery_dir();
            handle.spawn_blocking(move || {
                crate::clear_snapshot(&dir, project_id, crate::path_key(&path));
            });
        }
        Effect::StartWindowIndex {
            tab_id,
            path,
            cancel,
        } => {
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    let cancelled = {
                        let cancel = cancel.clone();
                        move || cancel.load(std::sync::atomic::Ordering::Relaxed)
                    };
                    crate::LineIndex::build_cancellable(&path, 1000, 0, cancelled)
                        .and_then(|index| {
                            index.ok_or_else(|| std::io::Error::other("index cancelled"))
                        })
                        .map(Arc::new)
                        .map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                emit(Message::WindowIndex { tab_id, result });
            });
        }
        Effect::LoadTabular {
            tab_id,
            generation,
            path,
            cancel,
        } => {
            handle.spawn_blocking(move || {
                emit(Message::TabularLoaded {
                    tab_id,
                    generation,
                    result: load_tabular(&path, &cancel),
                });
            });
        }
        Effect::LoadTabularSheet {
            tab_id,
            request,
            cancel,
        } => {
            handle.spawn_blocking(move || {
                emit(Message::TabularSheetLoaded {
                    tab_id,
                    sheet_index: request.index,
                    result: load_tabular_sheet(&request, &cancel),
                });
            });
        }
        Effect::LoadPlantUml {
            tab_id,
            generation,
            project_root,
            path,
        } => {
            handle.spawn_blocking(move || {
                let result =
                    crate::load_document(&project_root, &path, crate::PlantUmlLimits::default());
                emit(Message::PlantUmlLoaded {
                    tab_id,
                    generation,
                    result,
                });
            });
        }
        Effect::StartLargeFileSearch {
            tab_id,
            generation,
            path,
            query,
            cancel,
        } => {
            let progress_emit = emit.clone();
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    // 进度限频:最快每 100ms 回灌一次(长文件扫描不至于把 UI 事件队列打爆)。
                    let mut last = std::time::Instant::now()
                        .checked_sub(std::time::Duration::from_secs(1))
                        .unwrap_or_else(std::time::Instant::now);
                    let should_cancel = {
                        let cancel = cancel.clone();
                        move || cancel.load(std::sync::atomic::Ordering::Relaxed)
                    };
                    crate::stream_search_cancellable(
                        &path,
                        &query,
                        crate::SearchOptions::default(),
                        should_cancel,
                        move |p| {
                            let now = std::time::Instant::now();
                            if now.duration_since(last) < std::time::Duration::from_millis(100) {
                                return;
                            }
                            last = now;
                            progress_emit(Message::LargeFileSearchProgress {
                                tab_id,
                                generation,
                                completed: p.completed,
                                total: p.total,
                            });
                        },
                    )
                    .map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                // 取消(`Ok(None)`):已被新查询 / 关闭作废,静默丢弃。
                let result = match result {
                    Ok(Some(outcome)) => Ok(outcome),
                    Ok(None) => return,
                    Err(error) => Err(error),
                };
                emit(Message::LargeFileSearchResults {
                    tab_id,
                    generation,
                    result,
                });
            });
        }
        Effect::OpenExternal { path } => {
            // 只交给系统默认应用打开,不隐式执行文件本身。
            #[cfg(target_os = "macos")]
            let program = "open";
            #[cfg(not(target_os = "macos"))]
            let program = "xdg-open";
            handle.spawn_blocking(move || {
                // `spawn()` 成功不代表打开成功(应用找不到 / 被拒时进程随后才非零退出),等退出状态。
                let outcome = std::process::Command::new(program)
                    .arg(&path)
                    .status()
                    .map_err(|e| e.to_string())
                    .and_then(|status| {
                        status
                            .success()
                            .then_some(())
                            .ok_or_else(|| format!("{program} exited with {status}"))
                    });
                if let Err(error) = outcome {
                    emit(Message::OpenExternalFailed { path, error });
                }
            });
        }
        Effect::SaveFile { tab_id, path, text } => {
            handle.spawn_blocking(move || {
                let result = crate::save_text_atomic(&path, &text).map_err(|e| e.to_string());
                emit(Message::SaveFinished { tab_id, result });
            });
        }
        Effect::BuildLineIndex {
            tab_id,
            path,
            revision,
            cancel,
        } => {
            handle.spawn_blocking(move || {
                // 按 chunk 检查取消信号(置位即提前返回 `Ok(None)`)。
                let cancelled = {
                    let cancel = cancel.clone();
                    move || cancel.load(std::sync::atomic::Ordering::Relaxed)
                };
                let result = crate::LineIndex::build_cancellable(&path, 1000, revision, cancelled)
                    .and_then(|index| index.ok_or_else(|| std::io::Error::other("index cancelled")))
                    .map(std::sync::Arc::new)
                    .map_err(|e| e.to_string());
                emit(Message::WindowIndex { tab_id, result });
            });
        }
        Effect::WriteRecoverySnapshot {
            dir,
            project_id,
            path,
            revision,
            cursor,
            selection,
            top_line,
            text,
            tab_id,
        } => {
            handle.spawn_blocking(move || {
                let ok = crate::profile_file(&path)
                    .map(|profile| {
                        let manifest = crate::RecoveryManifest::from_profile(
                            path.clone(),
                            &profile,
                            revision,
                            cursor,
                            selection,
                            top_line,
                        );
                        crate::write_snapshot(
                            &dir,
                            project_id,
                            crate::path_key(&path),
                            &manifest,
                            &text,
                        )
                        .is_ok()
                    })
                    .unwrap_or(false);
                if ok {
                    emit(Message::RecoveryWritten(tab_id));
                }
            });
        }
        Effect::ClearRecoverySnapshot {
            dir,
            project_id,
            path,
        } => {
            handle.spawn_blocking(move || {
                crate::clear_snapshot(&dir, project_id, crate::path_key(&path));
            });
        }
        Effect::PersistTabs { project_id, state } => {
            handle.spawn(async move {
                if let Err(error) = persistence.save(project_id, &state) {
                    tracing::warn!(target: "dozer::module::preview", %error, "预览 tab 状态写盘失败");
                }
            });
        }
        Effect::AllowFile(_)
        | Effect::PathOpened(_)
        | Effect::ContextChanged { .. }
        | Effect::TabSelected { .. }
        | Effect::TabClosed { .. }
        | Effect::SelectionContextMenu { .. } => {}
        Effect::PushHostScript { webview_id, script } => {
            let _ = (webview_id, script);
        }
    }
}

fn read_recovery(
    dir: &std::path::Path,
    project_id: i64,
    path: &std::path::Path,
    cancel: &AtomicBool,
) -> Option<String> {
    use std::sync::atomic::Ordering::Relaxed;
    if cancel.load(Relaxed) {
        return None;
    }
    let key = crate::path_key(path);
    let (manifest, text) = crate::read_snapshot(dir, project_id, key)?;
    // 读完再查一次:长读期间被取消则不回灌。
    if cancel.load(Relaxed) {
        return None;
    }
    let disk = crate::profile_file(path).ok();
    if crate::classify_recovery(&manifest, disk.as_ref()) != crate::RecoveryResolution::Restore {
        return None;
    }
    // 快照正文与磁盘一致 → 本就没有未保存改动,不是有效 recovery
    // (老版本会在「打开即失焦」时为未修改文件写快照,这里兜底丢弃)。
    if std::fs::read_to_string(path).ok().as_deref() == Some(text.as_str()) {
        crate::clear_snapshot(dir, project_id, key);
        return None;
    }
    Some(text)
}

#[cfg(feature = "tabular-loader")]
fn load_tabular(
    path: &std::path::Path,
    cancel: &Arc<AtomicBool>,
) -> Result<crate::tabular::TabularView, String> {
    let cancel = cancel.clone();
    crate::tabular::load_cancellable(path, move || {
        cancel.load(std::sync::atomic::Ordering::Relaxed)
    })
}

#[cfg(not(feature = "tabular-loader"))]
fn load_tabular(
    _path: &std::path::Path,
    _cancel: &Arc<AtomicBool>,
) -> Result<crate::tabular::TabularView, String> {
    Err("tabular loader is not enabled in this build".to_string())
}

#[cfg(feature = "tabular-loader")]
fn load_tabular_sheet(
    request: &SheetLoadRequest,
    cancel: &Arc<AtomicBool>,
) -> Result<crate::tabular::Sheet, String> {
    let cancel = cancel.clone();
    crate::tabular::load_sheet_cancellable(&request.path, &request.name, move || {
        cancel.load(std::sync::atomic::Ordering::Relaxed)
    })
}

#[cfg(not(feature = "tabular-loader"))]
fn load_tabular_sheet(
    _request: &SheetLoadRequest,
    _cancel: &Arc<AtomicBool>,
) -> Result<crate::tabular::Sheet, String> {
    Err("tabular loader is not enabled in this build".to_string())
}
