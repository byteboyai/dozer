//! 运行时胶水:daemon 启动序列、operation 遍历、webview 池同步。Phase 1
//! 结构重组时从 `main.rs` 抽出,逻辑保持原样。

use std::process::{Command, Stdio};
use std::time::Duration;

use iced_winit::runtime::user_interface::UserInterface;
use wry::WebViewBuilderExtDarwin;

use crate::app::{App, Message};

dozer_core::scope!(LOG, module, "runtime");

/// daemon 连不上时的自动拉起：优先用 `current_exe` 同目录下的 `dozerd`
/// 二进制（cargo workspace 构建后与 `dozer` 落在同一个 target 目录），
/// 找不到就退化到 PATH 查找（`Command::new("dozerd")` 交给 shell/PATH 解析）。
pub(crate) fn spawn_dozerd() {
    let same_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("dozerd")));

    let mut command = match same_dir {
        Some(path) if path.exists() => Command::new(path),
        _ => Command::new("dozerd"),
    };

    match command.stdin(Stdio::null()).spawn() {
        Ok(_child) => dozer_core::log_info!(LOG, "已自动拉起 dozerd"),
        Err(e) => dozer_core::log_error!(LOG, "自动拉起 dozerd 失败: {e}"),
    }
}

/// 启动序列第一步：探测 daemon 是否可用（一次 `list()` 往返）。连不上
/// 就自动 spawn `dozerd`，隔 1 秒重试，最多 3 次；全部失败则返回错误
/// 文案，交给调用方决定如何展示（不阻塞窗口创建本身）。
pub(crate) async fn ensure_daemon(client: &dozer_client::Client) -> Result<(), String> {
    if client.list().await.is_ok() {
        return Ok(());
    }

    dozer_core::log_warn!(LOG, "daemon 未响应，尝试自动拉起 dozerd");
    spawn_dozerd();

    let mut last_err = String::from("daemon 未响应");
    for attempt in 1..=3 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        match client.list().await {
            Ok(_) => return Ok(()),
            Err(e) => {
                last_err = e.to_string();
                dozer_core::log_warn!(LOG, attempt, "重试连接 dozerd 仍失败: {last_err}");
            }
        }
    }
    Err(format!("无法连接 dozerd（已重试 3 次）：{last_err}"))
}

/// 启动序列：连 daemon（失败则 spawn dozerd 重试）→ 成功则做 GUI 级
/// 会话恢复（`App::bootstrap`），失败则降级为错误态
/// （`App::with_daemon_error`）。整段在 `main()` 用
/// `runtime.block_on` 驱动，此时窗口尚未创建，不占用任何"正在跑的"
/// UI 线程。
pub(crate) async fn build_app(
    client: dozer_client::Client,
    handle: tokio::runtime::Handle,
    proxy: winit::event_loop::EventLoopProxy<Message>,
) -> App {
    // 全局客户端能力:整个进程**只探测一次**,安装进只读快照后由 App/Workspace
    // 注入使用。这里输出一条结构化启动日志(只含硬件容量与预算,不含任何路径/
    // 文件名等敏感信息)。
    let capabilities = crate::capabilities::install(crate::capabilities::estimate_capabilities(
        crate::capabilities::detect_hardware(),
    ));
    let caps = &capabilities;
    dozer_core::log_info!(LOG,
        total_memory_bytes = caps.hardware.total_memory_bytes,
        available_memory_bytes = caps.hardware.available_memory_at_start_bytes,
        physical_cpus = caps.hardware.physical_cpu_count,
        logical_cpus = caps.hardware.logical_cpu_count,
        tier = ?caps.tier,
        single_editor_bytes = caps.budgets.single_editor_bytes,
        total_preview_bytes = caps.budgets.total_preview_bytes,
        json_tree_bytes = caps.budgets.json_tree_bytes,
        full_file_load_bytes = caps.budgets.full_file_load_bytes,
        max_heavy_webviews = caps.budgets.max_heavy_webviews,
        background_parallelism = caps.budgets.background_parallelism,
        "客户端能力快照已就绪"
    );

    // 安全启动(Phase C Task 7):上次启动若没走完(in_progress 标记残留),
    // 本次只恢复 tab 壳、不自动加载问题文件,避免启动死循环。先写 in_progress,
    // 启动序列走完再写 done。
    let marker = crate::preview::marker_path();
    let safe_startup = crate::preview::was_interrupted_from(&marker);
    if safe_startup {
        dozer_core::log_warn!(LOG, "检测到上次启动未完成,进入安全启动(仅恢复 tab 壳)");
    }
    let _ = crate::preview::write_status_to(&marker, crate::preview::STATUS_IN_PROGRESS);

    let app = match ensure_daemon(&client).await {
        Ok(()) => App::bootstrap(client, handle, proxy, capabilities, safe_startup).await,
        Err(message) => {
            App::with_daemon_error(client, handle, proxy, message, capabilities, safe_startup)
        }
    };
    let _ = crate::preview::write_status_to(&marker, crate::preview::STATUS_DONE);
    app
}

/// 执行一次 `operation` 遍历(程序化聚焦/滚动/每帧真实焦点镜像查询)。
///
/// 此前这里有一段 `catch_unwind` 兜底,是因为 vendored `iced-code-editor`
/// 内部用 `iced_aw::ContextMenu` 包代码画布,`iced_aw 0.13.1` 的
/// `ContextMenu::operate` 在菜单展开时有布局层级 panic——那个依赖已经随
/// `code_editor` 模块的官方 `text_editor` 替换一起删除(不再引入 `iced_aw`),
/// 兜底不再需要,恢复正常的 panic 行为。
pub(crate) fn run_operate(
    interface: &mut UserInterface<Message, iced_widget::Theme, iced_renderer::Renderer>,
    renderer: &mut iced_renderer::Renderer,
    operation: &mut dyn iced_winit::core::widget::operation::Operation,
) {
    interface.operate(renderer, operation);
}

/// 只给命中 `target` 的 focusable 调 `.focus()`,不碰任何其它 widget——官方
/// `operation::focusable::focus` 会把树里除 target 外的全部 focusable
/// `unfocus()`(见其源码,命中之外一律 `state.unfocus()`),不能直接拿来给
/// 代码编辑器补聚焦,那样会把 Find 输入框刚拿到的真焦点撵掉。
///
/// 用途:文件内搜索(⌘F)跳到某个命中时,原生 `iced_widget::text_editor`
/// 的选区高亮只在**自己持有真 iced 焦点**时才画(`draw()` 里
/// `if let Some(focus) = state.focus.as_ref()` 才会渲染 `Selection::Range`,
/// vendored 源码已核实),但这一刻真正的键盘焦点理应留在 Find 输入框(用户
/// 还要继续敲字)。用这个操作让编辑器**也**进入"已聚焦"态、只为了画出选区,
/// 不影响谁在接收键盘事件——键盘路由是这份代码库自己在 main.rs 顶层按
/// `*_focused()`/`current_focus` 这套粗粒度信号决定放行给哪个字段(见
/// `WindowEvent::KeyboardInput` 分支),不是靠 iced 内部 `is_focused()`
/// 反查"谁该收这个键",所以编辑器"看起来聚焦"不会导致它偷吃 Find 输入框
/// 正在敲的字符。见 `preview::PreviewPane::pending_editor_reveal_focus`
/// 文档(2026-09 用户实测反馈:查找跳到第 n 个命中,代码里必须真的选中那段
/// 文字)。
pub(crate) struct FocusAlso {
    pub(crate) target: iced_winit::core::widget::Id,
}

impl<T> iced_winit::core::widget::Operation<T> for FocusAlso {
    fn focusable(
        &mut self,
        id: Option<&iced_winit::core::widget::Id>,
        _bounds: iced_winit::core::Rectangle,
        state: &mut dyn iced_winit::core::widget::operation::Focusable,
    ) {
        if id == Some(&self.target) {
            state.focus();
        }
    }

    fn traverse(
        &mut self,
        operate: &mut dyn FnMut(&mut dyn iced_winit::core::widget::Operation<T>),
    ) {
        operate(self);
    }
}

/// 只对命中 `targets` 里某个 id 的 focusable 调 `.unfocus()`,不碰任何其它
/// widget——`operation::focusable::unfocus()`(无目标版本)会让**当前持有
/// 焦点的那个 widget**失焦,不管它是谁;这在"预览原生编辑器该让出焦点"
/// 的场景里是错的,因为触发这次 unfocus 的同一次点击,可能恰好正在把焦点
/// 给**另一个**原生 `text_input`(比如常驻搜索框)——无目标 unfocus 会把
/// 这个刚拿到的新焦点也一并抹掉(2026-09 用户反馈:文件树/Todo/Git Log
/// 常驻搜索框、右键搜索弹窗查询框统统点了打不出字,根因就是这个)。用这个
/// 操作把"让出焦点"限定到预览编辑器自己的 id 上,不影响其它 widget。
pub(crate) struct UnfocusTargets {
    pub(crate) targets: Vec<iced_winit::core::widget::Id>,
}

impl<T> iced_winit::core::widget::Operation<T> for UnfocusTargets {
    fn focusable(
        &mut self,
        id: Option<&iced_winit::core::widget::Id>,
        _bounds: iced_winit::core::Rectangle,
        state: &mut dyn iced_winit::core::widget::operation::Focusable,
    ) {
        if let Some(id) = id
            && self.targets.contains(id)
        {
            state.unfocus();
        }
    }

    fn traverse(
        &mut self,
        operate: &mut dyn FnMut(&mut dyn iced_winit::core::widget::Operation<T>),
    ) {
        operate(self);
    }
}

/// `sync_previews` 的差集同步逻辑,预览池/浏览器池共用同一套算法,
/// 各自传各自的 `pool`/`specs`,互不干扰。每条 spec 自带各自的矩形
/// (不再是整批共用一个)——支持两个不同的 webview 面板(如 `Files` 在
/// 左栏、`Project` 在右栏)同时出现在同一个池里,各自摆在各自的位置。
/// 见 spec "webview 面板的镜像 bounds(2026-08-19 Stage 4a 审阅后修订)"
/// 一节。
/// `sync_webview_pool` 的结果:哪些驻留 viewer 被淘汰 / reserve 被拒。调用方
/// (持 `App`)据此把对应 tab 迁移到 `Suspended`/`Failed`,避免"只删池句柄却
/// 保持 Ready"导致的逐帧重建抖动(T3)。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct PoolSyncOutcome {
    pub evicted: Vec<crate::preview::ViewerKey>,
    /// T10:本次**新建**且 reserve 被批准的 host,携带其 loading `generation`,
    /// 由调用方推进该 tab 从 `Reserving` 到 `CreatingHost`(世代校验)。
    pub granted: Vec<(crate::preview::ViewerKey, u64)>,
    /// T10:reserve 被拒的 host,携带其 loading `generation`,由调用方把该
    /// tab 迁到可解释的 Failed 终态并作废该世代(不再无限动画)。
    pub denied: Vec<(crate::preview::ViewerKey, u64)>,
}

/// 应用 webview 里注入的脚本:只转发焦点/拖拽松开/缩放三类按键与鼠标事件(与预览 webview 的前三件套同款,
/// 但**没有**查找、光标样式、标题回报——应用页面自己管这些)。
const APP_INIT_SCRIPT: &str = "document.addEventListener('mousedown',function(){window.ipc.postMessage('focus')},true);document.addEventListener('mouseup',function(){window.ipc.postMessage('mouseup')},true);document.addEventListener('keydown',function(e){if(e.ctrlKey){var c=e.code,k=e.key;if(c==='Equal'||k==='+'||k==='='){e.preventDefault();window.ipc.postMessage('zoom_in');}else if(c==='Minus'||k==='-'){e.preventDefault();window.ipc.postMessage('zoom_out');}else if(c==='Digit1'||k==='1'){e.preventDefault();window.ipc.postMessage('zoom_reset');}}},true);";

/// 创建一个应用 webview(`app_webview` 模块文档说明了信任模型)。与 `sync_webview_pool` 里给预览/浏览器
/// 用的构建路径的区别——**应用代码不可信**:
/// - 不注册 `dozer://` 自定义协议(预览 host/审阅快照/允许文件都只在那条协议后面);
/// - IPC 只认 `focus`/`mouseup`/`zoom_*` 四类白名单消息,其余一律丢弃;
/// - 导航只放行本应用 origin(`AppOrigin::allows_navigation`),`window.open`/新窗口一律拒绝,
///   下载不处理(没有 download handler = 取消);
/// - 每应用独立的 WKWebsiteDataStore(macOS 14+;更老的系统 wry 会退回默认存储)。
///
/// `spec.url` 不是该形状的应用地址时**不创建**(返回 `None`,记日志):fail closed。
fn build_app_webview(
    window: &winit::window::Window,
    spec: &crate::preview::WebviewSpec,
    bounds: wry::Rect,
    proxy: winit::event_loop::EventLoopProxy<Message>,
) -> Option<wry::WebView> {
    let Some(origin) = crate::app_webview::AppOrigin::from_url(&spec.url) else {
        dozer_core::log_error!(LOG, "应用 webview 的地址不是应用站点形状,拒绝创建");
        return None;
    };
    let webview_id = spec.id;
    let app_id = origin.app_id().to_owned();
    let ipc_proxy = proxy;
    let built = wry::WebViewBuilder::new()
        .with_url(&spec.url)
        .with_bounds(bounds)
        .with_visible(spec.visible)
        .with_allow_link_preview(false)
        .with_data_store_identifier(crate::app_webview::data_store_identifier(&app_id))
        .with_initialization_script(APP_INIT_SCRIPT)
        .with_ipc_handler(move |req| {
            let message = match req.body().as_str() {
                "mouseup" => Message::WebViewMouseUp,
                "focus" => Message::AppWebViewFocused(webview_id),
                "zoom_in" => Message::ZoomIn,
                "zoom_out" => Message::ZoomOut,
                "zoom_reset" => Message::ZoomReset,
                _ => return,
            };
            let _ = ipc_proxy.send_event(message);
        })
        .with_navigation_handler({
            let app_id = app_id.clone();
            move |url| {
                let allowed = origin.allows_navigation(&url);
                if !allowed {
                    // 只记 scheme+host:被拒的地址可能带任意查询串。
                    let target = url::Url::parse(&url)
                        .map(|u| format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")))
                        .unwrap_or_else(|_| "无法解析的地址".into());
                    dozer_core::log_warn!(LOG, app = %app_id, target = %target, "应用尝试离开自己的 origin,已拒绝");
                }
                allowed
            }
        })
        .with_new_window_req_handler(|_url, _features| wry::NewWindowResponse::Deny)
        .build_as_child(window);
    match built {
        Ok(view) => Some(view),
        Err(e) => {
            dozer_core::log_error!(LOG, app = %app_id, "创建应用 webview 失败: {e}");
            None
        }
    }
}

pub(crate) fn sync_webview_pool(
    window: &winit::window::Window,
    pool: &mut std::collections::HashMap<usize, (wry::WebView, String)>,
    specs: Vec<(crate::preview::WebviewSpec, wry::Rect)>,
    allowed_files: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>>,
    review_snapshot: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    proxy: winit::event_loop::EventLoopProxy<Message>,
    report_title: bool,
) -> PoolSyncOutcome {
    let mut outcome = PoolSyncOutcome::default();
    // CodeMirror editor host 在真正创建 WKWebView 之前 reserve；不足时先
    // 释放本池中 manager 指定的候选，再重试，避免编辑器 tab 累积把机器拖垮。
    let mut approved = Vec::with_capacity(specs.len());
    let mut manager = crate::preview::global_manager()
        .lock()
        .expect("preview resource manager lock");
    let mut keep = std::collections::HashSet::new();
    // 需要计入预算的 host 有两类:CodeMirror/JSON Tree 编辑 host(身份来自
    // `editor_binding`)与 T8 起的 PlantUML Rendered host(身份来自
    // `reserve` 的 `ReserveHint`,无编辑器绑定)。两者的 `(project, panel, tab)`
    // 只有一套用于 reserve/register/evict/回灌台账。
    let current_project = specs.iter().find_map(|(s, _)| {
        s.editor_binding
            .as_ref()
            .map(|b| b.project_id)
            .or_else(|| s.reserve.as_ref().map(|r| r.project_id))
    });
    for (spec, bounds) in specs {
        // 解析本条 spec 的预算身份与开销;不参与 reserve 的 host 直接放行。
        let budget = spec
            .editor_binding
            .as_ref()
            .map(|binding| {
                let bytes = std::fs::metadata(&binding.path)
                    .map(|m| m.len().saturating_mul(2).saturating_add(1024 * 1024))
                    .unwrap_or(1024 * 1024)
                    .min(manager.budgets().single_editor_bytes);
                (
                    (binding.project_id, binding.panel, binding.tab_id),
                    bytes,
                    true,
                    crate::preview::ViewerHostKind::Editor,
                )
            })
            .or_else(|| {
                spec.reserve.as_ref().map(|hint| {
                    (
                        hint.key(),
                        hint.cost.estimated_bytes,
                        hint.cost.heavy_webview,
                        hint.cost.kind,
                    )
                })
            });
        let Some((key, bytes, heavy, kind)) = budget else {
            approved.push((spec, bounds));
            continue;
        };
        let (project_id, panel, tab_id) = key;
        // T10:host 若在 `Reserving` 等待预算,携带其 loading 世代回灌调用方。
        let loading_generation = spec.loading_generation;
        keep.insert((panel, tab_id));
        if !manager.contains(key) {
            let reservation =
                manager.try_reserve(bytes, heavy, current_project.unwrap_or(project_id));
            if let crate::preview::Reservation::NeedEviction(keys) = reservation {
                for (project, panel, tab) in keys {
                    let marker = format!("proj={project}");
                    let panel_marker = format!("panel={}", crate::preview::panel_token(panel));
                    let tab_marker = format!("tab={tab}");
                    let ids: Vec<usize> = pool
                        .iter()
                        .filter(|(_, (_, url))| {
                            url.contains(&marker)
                                && url.contains(&panel_marker)
                                && url.contains(&tab_marker)
                        })
                        .map(|(id, _)| *id)
                        .collect();
                    for id in ids {
                        pool.remove(&id);
                    }
                    manager.release((project, panel, tab));
                    outcome.evicted.push((project, panel, tab));
                }
            }
            if !matches!(
                manager.try_reserve(bytes, heavy, project_id),
                crate::preview::Reservation::Granted
            ) {
                dozer_core::log_warn!(
                    LOG,
                    project_id,
                    tab_id,
                    heavy,
                    "preview resource budget denied webview"
                );
                outcome.denied.push((key, loading_generation.unwrap_or(0)));
                continue;
            }
            manager.register(crate::preview::ViewerRegistration {
                project_id,
                panel,
                tab_id,
                estimated_bytes: bytes,
                heavy_webview: heavy,
                kind,
                active: spec.visible,
                dirty: false,
                has_recovery: false,
                saving: false,
                agent_writing: false,
                last_accessed: 0,
            });
            // T10:新 host 获批,回灌世代让 tab 从 `Reserving` 进入 `CreatingHost`。
            if let Some(generation) = loading_generation {
                outcome.granted.push((key, generation));
            }
        } else {
            manager.set_active(key, spec.visible);
            manager.touch(key);
        }
        approved.push((spec, bounds));
    }
    if let Some(project_id) = current_project {
        manager.prune_project(project_id, &keep);
    }
    // T8 bullet 8:资源诊断——按宿主种类报告驻留数与估算字节(成本估算,
    // 不含任何文件正文)。debug 级,面板过滤见 `dozer::module::runtime`。
    let diag = manager.diagnostics();
    dozer_core::log_debug!(
        LOG,
        resident_count = diag.resident_count,
        total_resident_bytes = diag.total_resident_bytes,
        heavy_webviews = diag.heavy_webviews,
        max_heavy_webviews = diag.max_heavy_webviews,
        plantuml_resident = diag
            .by_kind
            .iter()
            .find(|(k, _)| *k == crate::preview::ViewerHostKind::PlantUml)
            .map(|(_, n)| *n)
            .unwrap_or(0),
        by_kind = ?diag.by_kind,
        bytes_by_kind = ?diag.bytes_by_kind,
        "预览资源诊断"
    );
    drop(manager);
    let specs = approved;
    let desired_hosts: std::collections::HashMap<usize, bool> = specs
        .iter()
        .map(|(spec, _)| (spec.id, spec.editor_binding.is_some()))
        .collect();
    // IPC handler 捕获了创建时的 editor binding，因此同一个池 key 从
    // flyfish 切成 editor（或反向）时必须重建，不能只 load_url。
    pool.retain(|id, (_, loaded_url)| {
        desired_hosts.get(id).is_some_and(|expects_editor| {
            *expects_editor == crate::preview::is_host_url(loaded_url)
        })
    });

    for (spec, bounds) in specs {
        match pool.get_mut(&spec.id) {
            Some((view, loaded_url)) => {
                // 去重判断要同时看两个来源,缺一个都会闪:
                // - `view.url()`(webview 实际地址):网页内超链接让 webview
                //   自行导航后,这里才反映新地址。只信它会导致地址栏驱动
                //   的导航`load_url` 后、真提交前那几帧里 `view.url()`
                //   仍是旧值,每一帧都重发一次 `load_url`(反复重启导航,
                //   页面不停闪烁)。
                // - `loaded_url`(我们曾下达的命令地址):只信它会在网页内
                //   跳转完成后把已加载好的目标页误判成"要加载"再整页重载
                //   一次。`view.url()` 失败(罕见)时退回缓存的 `loaded_url`。
                // 合起来:只有**既没到过、也没下达过**这个地址时才真正加载,
                // 既挡住地址栏提交后的重复导航,又不打扰网页内跳转。
                let current = view.url().unwrap_or_else(|_| loaded_url.clone());
                if current != spec.url && *loaded_url != spec.url {
                    if let Err(e) = view.load_url(&spec.url) {
                        dozer_core::log_warn!(LOG, "预览导航失败: {e}");
                    }
                    *loaded_url = spec.url.clone();
                    // 导航会重置 WKWebView 的 pageZoom,重建后把当前
                    // 全局 UI 缩放补回去,否则预览字号会跳回 100%。
                    let _ = view.zoom(byteui::theme::icon_size::scale() as f64);
                }
                let _ = view.set_bounds(bounds);
                // park_offscreen 的宿主必须保持 `set_visible(true)`:它虽被
                // 摆到窗口外不可见,但 WebKit 只在视图「非 hidden」时才跑
                // rAF,一旦这里跟着 `spec.visible=false` 真隐藏,docx 等依赖
                // rAF 的渲染器又会卡死(见 `WebviewSpec::park_offscreen`)。
                let _ = view.set_visible(spec.visible || spec.park_offscreen);
            }
            None if crate::app_webview::is_app_webview_id(spec.id) => {
                // 应用面板(第三方/agent 生成的代码):走**单独的受限构建路径**,不装 `dozer://` 协议。
                if let Some(view) = build_app_webview(window, &spec, bounds, proxy.clone()) {
                    let _ = view.zoom(byteui::theme::icon_size::scale() as f64);
                    pool.insert(spec.id, (view, spec.url.clone()));
                }
            }
            None => {
                let allowed = std::sync::Arc::clone(&allowed_files);
                let review_snapshot = std::sync::Arc::clone(&review_snapshot);
                let root = crate::assets::assets_root();
                let ipc_proxy = proxy.clone();
                let nav_proxy = proxy.clone();
                let webview_id = spec.id;
                let editor_binding = spec.editor_binding.clone();
                let is_json_host = crate::preview::is_json_editor_url(&spec.url);
                let is_tabular_host = crate::preview::is_tabular_url(&spec.url);
                // T9:Flyfish host 的绑定从 URL 查询串解析(proj/panel/tab/doc),
                // host 回传的 envelope 据此校验归属。
                let flyfish_binding = crate::preview::flyfish_binding_from_url(&spec.url);
                // `flyfish_binding_from_url` 同时覆盖 Flyfish、隔离 HTML 与
                // image-annotate host；只有 Flyfish 暴露 `searchDocument` 等文档搜索
                // API。这个标记还用于把 WebView 聚焦态的 Cmd/Ctrl+F 路由回 Dozer
                // Find 条。
                let is_flyfish_host = spec.url.starts_with("dozer://flyfish/");
                // image-annotate host 的 envelope payload 与 Flyfish 不同
                // (annotations_changed),解析器必须分开,不能几何共享。
                let is_image_annotate_host = spec.url.starts_with("dozer://image-annotate/");
                // PlantUML viewer host 的 envelope(ready/rendered/failed/
                // open_source)与 Flyfish 不同,解析器必须分开;binding 同样从
                // URL 解析(PlantUML host `editor_binding` 为 `None`)。
                let is_plantuml_host = spec.url.starts_with("dozer://plantuml-viewer/");
                // 常驻注入脚本:焦点/拖拽/缩放三件套(所有 webview);浏览器
                // 面板(`report_title`)额外附一段"页面标题回报":把
                // `window.__dozer_webview` 记成本 webview 的 id,页面
                // DOMContentLoaded 与 `document.title` 变化(MutationObserver)
                // 时经 IPC 发 `title:<id>:<title>`,让 tab 标题在页面加载
                // 完成后从 URL 切换到 HTML `<title>`。预览 webview 不掺和。
                let mut script = String::from(
                    "document.addEventListener('mousedown',function(){window.ipc.postMessage('focus')},true);document.addEventListener('mouseup',function(){window.ipc.postMessage('mouseup')},true);document.addEventListener('keydown',function(e){if(e.ctrlKey){var c=e.code,k=e.key;if(c==='Equal'||k==='+'||k==='='){e.preventDefault();window.ipc.postMessage('zoom_in');}else if(c==='Minus'||k==='-'){e.preventDefault();window.ipc.postMessage('zoom_out');}else if(c==='Digit1'||k==='1'){e.preventDefault();window.ipc.postMessage('zoom_reset');}}},true);",
                );
                // 文本光标 + Ctrl/Cmd+C 复制(所有 webview):
                // - 默认让普通文字显示 I 形文本光标;链接/按钮等可交互元素
                //   仍是手形(继承自 `cursor: text` 时会被 `cursor: pointer`
                //   覆盖,因为后写、同为 `!important`)。
                // - Ctrl+C / Cmd+C 都拦截复制当前选区:webview 成为 first
                //   responder 后 keydown 被它吃掉、到不了 winit,得在这段
                //   注入 JS 里自己拦。用 `document.execCommand('copy')`
                //   (用户手势触发,WKWebView 会放行);个别方案拿不到选区
                //   或失败时再退回 `navigator.clipboard`。`Cmd+Meta` 双键
                //   都按,保证两套快捷键一致触发。用 `e.code` 判断,避免
                //   键盘布局差异影响 `e.key`。
                if editor_binding.is_none() {
                    script.push_str(
                        "(function(){var s=document.createElement('style');s.textContent=\"html,body,body *{cursor:text!important}a,a *,button,*[role='link'],*[role='button'],summary,*[onclick],label[for]{cursor:pointer!important}\";(document.head||document.documentElement).appendChild(s);document.addEventListener('keydown',function(e){if((e.ctrlKey||e.metaKey)&&e.code==='KeyC'){e.preventDefault();var ok=false;try{ok=document.execCommand('copy')}catch(_){}if(!ok){var g=window.getSelection&&window.getSelection();if(g&&g.toString()){try{navigator.clipboard.writeText(g.toString()).then(function(){},function(){})}catch(_){}}}}});})();",
                    );
                    if is_flyfish_host {
                        // Flyfish host 明确关闭了自带 toolbar，因此页面内没有可聚焦
                        // 的搜索框。WebView 聚焦时直接把 Cmd/Ctrl+F 交还给 Dozer，
                        // 由统一 Find 条收 query，再调用 Flyfish `searchDocument`。
                        script.push_str(
                            "document.addEventListener('keydown',function(e){if((e.metaKey||e.ctrlKey)&&e.code==='KeyF'){e.preventDefault();e.stopPropagation();window.ipc.postMessage('find_preview');}},true);",
                        );
                    } else {
                        // 浏览器/隔离 HTML 等普通页面仍优先使用页面自己的搜索框，
                        // 找不到时退回平台原生页内查找。
                        script.push_str(
                            "(function(){function _dozPick(root){var best=null;function walk(n){if(!n||n.nodeType!==1&&n.nodeType!==9&&n.nodeType!==11)return;var list=n.querySelectorAll?n.querySelectorAll('input[type=search]'):[];for(var i=0;i<list.length;i++){var el=list[i];if(el.disabled||el.readOnly)continue;if(!best)best=el;}if(!best){var any=n.querySelectorAll?n.querySelectorAll('input[type=text],input:not([type])'):[];for(var j=0;j<any.length;j++){var a=any[j];if(a.disabled||a.readOnly)continue;var box=a.closest&&(a.closest('[role=search]')||a.closest('.file-viewer-web-search')||a.closest('form[role=search]'));if(box){best=a;break;}}}var hosts=n.querySelectorAll?n.querySelectorAll('*'):[];for(var k=0;k<hosts.length;k++){var sr=hosts[k].shadowRoot;if(sr)walk(sr);}}walk(root||document);return best;}document.addEventListener('keydown',function(e){if((e.metaKey||e.ctrlKey)&&e.code==='KeyF'){e.preventDefault();var box=_dozPick(document);if(box){try{box.focus();if(box.select)box.select();}catch(_){}window.ipc.postMessage('find_page');}else{window.ipc.postMessage('find_native');}}},true);})();",
                        );
                    }
                }
                if report_title {
                    script.push_str("window.__dozer_webview=");
                    script.push_str(webview_id.to_string().as_str());
                    script.push_str(
                        ";function _dt(){window.ipc.postMessage('title:'+String(__dozer_webview)+':'+document.title)}if(document.readyState==='complete'){_dt()}else{document.addEventListener('DOMContentLoaded',_dt)}new MutationObserver(_dt).observe(document.documentElement||document,{childList:true,subtree:true,attributeFilter:['title']});",
                    );
                }
                let mut builder = wry::WebViewBuilder::new()
                    .with_url(&spec.url)
                    .with_bounds(bounds)
                    // 同更新路径:park_offscreen 的宿主虽然被摆到窗口外,也必须
                    // 以 visible 创建,保证 WebKit 跑 rAF(见 `WebviewSpec::park_offscreen`)。
                    .with_visible(spec.visible || spec.park_offscreen)
                    // 关掉 macOS 的链接预览 force-click/long-press peek 浮层。
                    // (这只影响长按/重压预览,不影响普通单击跳转;普通单击
                    // 跳转真正缺的那块是下方的 `on_page_load`/`new_window_req`
                    // 回报钩子。)
                    .with_allow_link_preview(false)
                    // 子 webview 上的 mousedown winit 收不到,这里注入 JS
                    // 在捕获阶段监听 mousedown,经 IPC 通知宿主调 view.focus()
                    // 让 WKWebView 成为 first responder(否则 ⌘C 选区复制
                    // 走不通:WKWebView 不是 first responder 时 keyDown 不到它)。
                    // 顺带监听 mouseup:页签拖拽拖进 webview 后在这里松开,
                    // winit 收不到 `Released`,靠这条 IPC 结束拖拽
                    // (`WebViewMouseUp`)。
                    .with_initialization_script(script)
                    .with_ipc_handler(move |_req| {
                        let body = _req.body().as_str();
                        match body {
                            "mouseup" => {
                                let _ = ipc_proxy.send_event(Message::WebViewMouseUp);
                            }
                            // 子 webview 抢到键盘焦点(首次点击后 WKWebView
                            // 成为 first responder)时,Ctrl++/-/1 这类全局缩放
                            // 快捷键不会再回到 winit,会被 webview 自己吃掉。
                            // 这里在 keydown 捕获阶段拦下这几个组合键、转发给
                            // 宿主走 `icon_size::zoom_by`,使首页/预览/浏览器
                            // 各种 webview 聚焦态下缩放都和终端/自绘面板一致。
                            "zoom_in" => {
                                let _ = ipc_proxy.send_event(Message::ZoomIn);
                            }
                            "zoom_out" => {
                                let _ = ipc_proxy.send_event(Message::ZoomOut);
                            }
                            "zoom_reset" => {
                                let _ = ipc_proxy.send_event(Message::ZoomReset);
                            }
                            "focus" => {
                                let _ = ipc_proxy.send_event(Message::WebViewFocused);
                            }
                            // Flyfish WebView 获得 first responder 后，Cmd/Ctrl+F
                            // 不会再到达 winit。直接进入现有 Dozer Find 状态机；
                            // 查询和跳转随后仍由 Flyfish 搜索 API 执行。
                            "find_preview" if is_flyfish_host => {
                                if let Some(binding) = flyfish_binding.as_ref() {
                                    let _ = ipc_proxy
                                        .send_event(Message::PreviewFindOpen(binding.panel));
                                }
                            }
                            // ⌘F 页内查找:JS 找到并聚焦了页面自带搜索框
                            // (`find_page`),或没找到、要退回原生查找条
                            // (`find_native`)。二者的原生副作用(设 first
                            // responder / 调 WKWebView `findString:`)都要落到
                            // 持有 webview 句柄的事件环里,所以带 webview id
                            // 送回主循环,由 `dispatch` 在 `webviews`/
                            // `browser_webviews` 池里找回句柄执行(同
                            // `WebViewFocused`/`BrowserNav` 的"句柄只活在
                            // 事件分发环"手法)。
                            "find_page" => {
                                let _ = ipc_proxy.send_event(Message::WebViewFindFocus(webview_id));
                            }
                            "find_native" => {
                                let _ =
                                    ipc_proxy.send_event(Message::WebViewFindNative(webview_id));
                            }
                            // 浏览器面板报回页面 HTML 标题:`title:<id>:<title>`。
                            // `splitn(2, ':')` 只拆第一个冒号,标题里再带冒号
                            // 也不被误拆。空标题页面仍发,由 `browser::update`
                            // 的空标题守卫决定不覆盖 URL 标题。
                            title_msg if title_msg.starts_with("title:") => {
                                let rest = &title_msg["title:".len()..];
                                let mut it = rest.splitn(2, ':');
                                if let (Some(id_s), Some(title)) = (it.next(), it.next())
                                    && let Ok(id) = id_s.parse::<usize>()
                                {
                                    let _ = ipc_proxy
                                        .send_event(Message::BrowserTitle(id, title.to_string()));
                                }
                            }
                            _ => {
                                // T9:Flyfish host 回传的 envelope(JSON,以 `{` 起)。
                                let looks_like_envelope = body.starts_with('{');
                                if let Some(binding) = flyfish_binding.as_ref()
                                    && is_image_annotate_host
                                    && looks_like_envelope
                                {
                                    match crate::preview::parse_image_annotate_event(body) {
                                        Ok(event) => {
                                            if let Err(error) = event.validate(binding) {
                                                dozer_core::log_warn!(LOG, %error, "拒绝无效 image-annotate IPC");
                                            } else {
                                                let _ = ipc_proxy.send_event(
                                                    Message::ImageAnnotateEvent(
                                                        binding.clone(),
                                                        event,
                                                    ),
                                                );
                                            }
                                        }
                                        Err(error) => {
                                            dozer_core::log_warn!(LOG, %error, "无法解析 image-annotate IPC");
                                        }
                                    }
                                } else if let Some(binding) = flyfish_binding.as_ref()
                                    && is_plantuml_host
                                    && looks_like_envelope
                                {
                                    match crate::preview::parse_plantuml_event(body) {
                                        Ok(event) => {
                                            if let Err(error) = event.validate(binding) {
                                                dozer_core::log_warn!(LOG, %error, "拒绝无效 plantuml IPC");
                                            } else {
                                                let _ = ipc_proxy.send_event(
                                                    Message::PlantUmlEvent(
                                                        binding.clone(),
                                                        event,
                                                    ),
                                                );
                                            }
                                        }
                                        Err(error) => {
                                            dozer_core::log_warn!(LOG, %error, "无法解析 plantuml IPC");
                                        }
                                    }
                                } else if let Some(binding) = flyfish_binding.as_ref()
                                    && looks_like_envelope
                                {
                                    match crate::preview::parse_flyfish_event(body) {
                                        Ok(event) => {
                                            if let Err(error) = event.validate(binding) {
                                                dozer_core::log_warn!(LOG, %error, "拒绝无效 flyfish IPC");
                                            } else {
                                                let _ = ipc_proxy.send_event(
                                                    Message::FlyfishEvent(binding.clone(), event),
                                                );
                                            }
                                        }
                                        Err(error) => {
                                            dozer_core::log_warn!(LOG, %error, "无法解析 flyfish IPC");
                                        }
                                    }
                                } else if webview_id == crate::app::USAGE_CONTENT_ID_OFFSET
                                    && looks_like_envelope
                                {
                                    // usage-content 不是 CodeMirror/JSON/Flyfish
                                    // 家族,不复用那三者任何一个 binding,直接按固定
                                    // webview id 判断(单槽面板,没有 tab/document 身份
                                    // 需要携带)。
                                    match crate::extensions::usage::parse_usage_event(body) {
                                        Ok(event) => {
                                            let _ = ipc_proxy.send_event(
                                                Message::UsageContentWebviewEvent(event),
                                            );
                                        }
                                        Err(error) => {
                                            dozer_core::log_warn!(LOG, %error, "无法解析 usage-content IPC");
                                        }
                                    }
                                } else if webview_id == crate::app::CODEHEALTH_CONTENT_ID_OFFSET
                                    && looks_like_envelope
                                {
                                    // codehealth-content 同 usage-content:固定单槽,按固定
                                    // webview id 识别,不复用任何 binding。
                                    match crate::extensions::codehealth::parse_codehealth_event(body) {
                                        Ok(event) => {
                                            let _ = ipc_proxy.send_event(
                                                Message::CodeHealthContentWebviewEvent(event),
                                            );
                                        }
                                        Err(error) => {
                                            dozer_core::log_warn!(LOG, %error, "无法解析 codehealth-content IPC");
                                        }
                                    }
                                } else if webview_id == crate::app::TODO_CONTENT_ID_OFFSET
                                    && looks_like_envelope
                                {
                                    // todo-content 同 codehealth-content:固定单槽,按固定
                                    // webview id 识别,不复用任何 binding。
                                    match crate::extensions::todo::parse_todo_event(body) {
                                        Ok(event) => {
                                            let _ = ipc_proxy.send_event(
                                                Message::TodoContentWebviewEvent(event),
                                            );
                                        }
                                        Err(error) => {
                                            dozer_core::log_warn!(LOG, %error, "无法解析 todo-content IPC");
                                        }
                                    }
                                } else if webview_id == crate::app::GROUP_CHAT_CONTENT_ID_OFFSET
                                    && looks_like_envelope
                                {
                                    // group-chat-content 同 todo-content:固定单槽,按固定
                                    // webview id 识别,不复用任何 binding。
                                    match crate::extensions::group_chat::parse_group_chat_event(body) {
                                        Ok(event) => {
                                            let _ = ipc_proxy.send_event(
                                                Message::GroupChatContentWebviewEvent(event),
                                            );
                                        }
                                        Err(error) => {
                                            dozer_core::log_warn!(LOG, %error, "无法解析 group-chat-content IPC");
                                        }
                                    }
                                } else if webview_id == crate::app::CONVERSATION_REVIEW_ID_OFFSET
                                    && looks_like_envelope
                                {
                                    // review-trace 单槽非 tab,没有专属 binding,
                                    // 按固定 webview id 识别(同 usage-content)。
                                    match crate::preview::parse_review_trace_event(body) {
                                        Ok(env) => {
                                            let _ = ipc_proxy.send_event(
                                                Message::ReviewTraceWebviewEvent(env.payload),
                                            );
                                        }
                                        Err(error) => {
                                            dozer_core::log_warn!(LOG, %error, "无法解析 review-trace IPC");
                                        }
                                    }
                                } else if let Some(binding) = editor_binding.as_ref() {
                                    let expected = crate::preview::HostBinding::new(
                                        binding.project_id,
                                        binding.panel,
                                        binding.tab_id,
                                        binding.document_id(),
                                    );
                                    if is_tabular_host {
                                        match crate::preview::parse_tabular_event(body) {
                                            Ok(event) => {
                                                if let Err(error) = event.validate(&expected) {
                                                    dozer_core::log_warn!(LOG, %error, "拒绝无效 tabular IPC");
                                                } else {
                                                    let _ = ipc_proxy.send_event(
                                                        Message::TabularHostEvent(
                                                            binding.clone(),
                                                            event,
                                                        ),
                                                    );
                                                }
                                            }
                                            Err(error) => {
                                                dozer_core::log_warn!(LOG, %error, "无法解析 tabular IPC");
                                            }
                                        }
                                    } else if is_json_host {
                                        match crate::preview::parse_json_event(body) {
                                            Ok(event) => {
                                                if let Err(error) = event.validate(&expected) {
                                                    dozer_core::log_warn!(LOG, %error, "拒绝无效 json-editor IPC");
                                                } else {
                                                    let _ = ipc_proxy.send_event(
                                                        Message::JsonEditorEvent(
                                                            binding.clone(),
                                                            event,
                                                        ),
                                                    );
                                                }
                                            }
                                            Err(error) => {
                                                dozer_core::log_warn!(LOG, %error, "无法解析 json-editor IPC");
                                            }
                                        }
                                    } else {
                                        match crate::preview::parse_event(body) {
                                            Ok(event) => {
                                                if let Err(error) = event.validate(&expected) {
                                                    dozer_core::log_warn!(LOG, %error, "拒绝无效 editor IPC");
                                                } else if binding.panel
                                                    == crate::app::PanelKind::GitLog
                                                {
                                                    let _ = ipc_proxy.send_event(
                                                        Message::GitLogDiffWebviewEvent(
                                                            expected, event,
                                                        ),
                                                    );
                                                } else {
                                                    let _ = ipc_proxy.send_event(
                                                        Message::EditorWebviewEvent(
                                                            binding.clone(),
                                                            event,
                                                        ),
                                                    );
                                                }
                                            }
                                            Err(error) => {
                                                dozer_core::log_warn!(LOG, %error, "无法解析 editor IPC");
                                            }
                                        }
                                    }
                                } else {
                                    let _ = ipc_proxy.send_event(Message::WebViewFocused);
                                }
                            }
                        }
                    });
                // 浏览器面板 webview 额外挂两个导航回报钩子(预览 webview
                // 不需要):
                // 1) `on_page_load`:网页内超链接让 webview 自行导航后,把
                //    真实目标 URL 报回宿主(`Message::BrowserNavigated`),让
                //    地址栏/页签跟随页面跳转——此前没有任何机制告诉浏览器
                //    面板"页面自己跳走了",点链接后面板仍停在旧 URL;
                //    2) `new_window_req`:`target="_blank"`/`window.open`
                //    请求新窗口,wry 默认 `createWebViewWith:` 返回 nil 会
                //    静默丢弃这类点击。这里拒绝 OS 新窗口、改在浏览器面板
                //    里新开一个 tab。
                if report_title {
                    // `on_page_load` 与 `new_window_req` 是两个 `move` 闭包,
                    // 不能共用同一个 `nav_proxy`(第一个构造时就把原始值移走),
                    // 这里各留一份 clone。
                    let page_proxy = nav_proxy.clone();
                    let win_proxy = nav_proxy;
                    builder = builder
                        .with_on_page_load_handler(move |event, url| {
                            // 只在整页**加载完成**时回报一次(`Finished` 对应
                            // WKWebView `didFinishNavigation`,第二个参数就是
                            // 主 frame 的当前地址,不会被子 frame/资源加载
                            // 刷屏)。`Started` 也会触发主 frame 提交,但完成态
                            // 更稳,避免回报过早。
                            if matches!(event, wry::PageLoadEvent::Finished) {
                                let _ = page_proxy
                                    .send_event(Message::BrowserNavigated(webview_id, url));
                            }
                        })
                        .with_new_window_req_handler(move |url, _features| {
                            let _ = win_proxy.send_event(Message::BrowserNewWindow(url));
                            wry::NewWindowResponse::Deny
                        });
                }
                let built = builder
                    .with_custom_protocol("dozer".into(), move |_id, request| {
                        let allowed = allowed.lock().expect("allowed_files 锁");
                        let review_data = review_snapshot.lock().expect("review snapshot 锁");
                        let reply = crate::assets::handle_protocol(
                            &root,
                            &allowed,
                            review_data.as_deref(),
                            &request.uri().to_string(),
                        );
                        wry::http::Response::builder()
                            .status(reply.status)
                            .header("Content-Type", reply.mime)
                            .body(std::borrow::Cow::Owned(reply.body))
                            .expect("构造协议应答")
                    })
                    .build_as_child(window);
                match built {
                    Ok(view) => {
                        // 新 webview 按当前全局 UI 缩放初始化,使预览字号
                        // 跟着 ⌘/Ctrl +/- 一起缩放(`WebView::zoom` 在
                        // macOS 11+ 走 WKWebView 的 pageZoom)。
                        let _ = view.zoom(byteui::theme::icon_size::scale() as f64);
                        pool.insert(spec.id, (view, spec.url.clone()));
                    }
                    Err(e) => dozer_core::log_error!(LOG, "创建预览 webview 失败: {e}"),
                }
            }
        }
    }
    outcome
}
