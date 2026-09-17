//! 运行时胶水:daemon 启动序列、operation 遍历、webview 池同步。Phase 1
//! 结构重组时从 `main.rs` 抽出,逻辑保持原样。

use std::process::{Command, Stdio};
use std::time::Duration;

use iced_winit::runtime::user_interface::UserInterface;
use wry::WebViewBuilderExtDarwin;

use crate::app::{App, Message};

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
        Ok(_child) => tracing::info!("已自动拉起 dozerd"),
        Err(e) => tracing::error!("自动拉起 dozerd 失败: {e}"),
    }
}

/// 启动序列第一步：探测 daemon 是否可用（一次 `list()` 往返）。连不上
/// 就自动 spawn `dozerd`，隔 1 秒重试，最多 3 次；全部失败则返回错误
/// 文案，交给调用方决定如何展示（不阻塞窗口创建本身）。
pub(crate) async fn ensure_daemon(client: &dozer_client::Client) -> Result<(), String> {
    if client.list().await.is_ok() {
        return Ok(());
    }

    tracing::warn!("daemon 未响应，尝试自动拉起 dozerd");
    spawn_dozerd();

    let mut last_err = String::from("daemon 未响应");
    for attempt in 1..=3 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        match client.list().await {
            Ok(_) => return Ok(()),
            Err(e) => {
                last_err = e.to_string();
                tracing::warn!(attempt, "重试连接 dozerd 仍失败: {last_err}");
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
    match ensure_daemon(&client).await {
        Ok(()) => App::bootstrap(client, handle, proxy).await,
        Err(message) => App::with_daemon_error(client, handle, proxy, message),
    }
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
pub(crate) fn sync_webview_pool(
    window: &winit::window::Window,
    pool: &mut std::collections::HashMap<usize, (wry::WebView, String)>,
    specs: Vec<(crate::preview::WebviewSpec, wry::Rect)>,
    allowed_files: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>>,
    review_snapshot: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    proxy: winit::event_loop::EventLoopProxy<Message>,
    report_title: bool,
) {
    let desired_ids: std::collections::HashSet<usize> = specs.iter().map(|(s, _)| s.id).collect();
    pool.retain(|id, _| desired_ids.contains(id));

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
                        tracing::warn!("预览导航失败: {e}");
                    }
                    *loaded_url = spec.url.clone();
                    // 导航会重置 WKWebView 的 pageZoom,重建后把当前
                    // 全局 UI 缩放补回去,否则预览字号会跳回 100%。
                    let _ = view.zoom(byteui::theme::icon_size::scale() as f64);
                }
                let _ = view.set_bounds(bounds);
                let _ = view.set_visible(spec.visible);
            }
            None => {
                let allowed = std::sync::Arc::clone(&allowed_files);
                let review_snapshot = std::sync::Arc::clone(&review_snapshot);
                let root = crate::assets::assets_root();
                let ipc_proxy = proxy.clone();
                let nav_proxy = proxy.clone();
                let webview_id = spec.id;
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
                script.push_str(
                    "(function(){var s=document.createElement('style');s.textContent=\"html,body,body *{cursor:text!important}a,a *,button,*[role='link'],*[role='button'],summary,*[onclick],label[for]{cursor:pointer!important}\";(document.head||document.documentElement).appendChild(s);document.addEventListener('keydown',function(e){if((e.ctrlKey||e.metaKey)&&e.code==='KeyC'){e.preventDefault();var ok=false;try{ok=document.execCommand('copy')}catch(_){}if(!ok){var g=window.getSelection&&window.getSelection();if(g&&g.toString()){try{navigator.clipboard.writeText(g.toString()).then(function(){},function(){})}catch(_){}}}}});})();",
                );
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
                    .with_visible(spec.visible)
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
                                let _ = ipc_proxy.send_event(Message::WebViewFocused);
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
                    Err(e) => tracing::error!("创建预览 webview 失败: {e}"),
                }
            }
        }
    }
}
