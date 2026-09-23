# 文件历史弹窗 Diff 视图改用 CodeMirror 设计

## 背景

`platform/file_history_overlay.rs`（右键 Files 面板文件行"查看此文件历史"
弹窗）的 diff pane 当前同样由 `extensions::diff_render::colored_diff_lines`
纯 iced 渲染（`extensions/file_history.rs:553`），数据来源是
`diff_against_current()`——跟某个历史 commit 版本对比的不是它的父提交，而是
**磁盘上的实时工作区内容**（`diff_tree_to_workdir`），这是和 Git Log 面板
最大的语义差异（Git Log 对比的是两个 commit 之间）。

本设计是
[`2026-09-23-git-log-codemirror-diff-design.md`](2026-09-23-git-log-codemirror-diff-design.md)
（下称"git log 设计"）的姊妹篇，把同一个视觉升级（CodeMirror 6
`unifiedMergeView`）落到文件历史弹窗。**协议层完全复用 git log 设计的产出**
（`EditorCommand::SetDiffDocument`、JS 端 `mode=diff` 的 `unifiedMergeView`
挂载逻辑、二进制/大小判定），本设计只解决一个新问题：**这扇弹窗是完全独立
的原生子窗口**（`FileHistoryOverlay`：自己的 `winit::Window` +
`iced_wgpu` GPU 渲染循环），当前**没有任何 wry/webview 接入**——不是复用
主窗口已有的 webview 宿主基建（`runtime.rs::sync_webview_pool` 那套多
tab 池化 + 资源预算淘汰机制）能解决的,因为那套机制虽然函数签名上不绑定
主窗口，但围绕它的输入转发、resize 联动、IPC 事件泵全部是主窗口专属代码。

这扇弹窗当初（2026-09-18）从"主窗口内 iced 浮层"迁移成独立原生子窗口，
正是为了绕开"webview 恒在 iced 之上"的遮挡问题（见 `platform/
overlay_window.rs` 共享机制、`docs/superpowers/specs/2026-09-18-overlay-
window-shared-abstraction-design.md`）。本设计不推翻那次迁移——继续保持
独立窗口，只是这扇独立窗口自己也接入一个轻量 webview（细节见下）。

## 目标与非目标

### 目标

- 文件历史弹窗选中某次历史提交后，diff pane 用 CodeMirror 6 的
  `unifiedMergeView` 渲染：old_text = 该 commit 树里这个文件路径的 blob
  内容，new_text = **磁盘上的实时文件内容**（不是另一个 commit 的 blob）。
- 在 `FileHistoryOverlay` 自己的窗口上挂载一个**单一、生命周期与弹窗窗口
  本身绑定**的 `wry::WebView`（弹窗开→按需建，弹窗关→随窗口一起销毁），
  不引入 `sync_webview_pool` 的多 tab 池化/资源预算淘汰机制——那套是为
  "同时可能有多个预览 tab"设计的，这里任意时刻只有一个 diff webview，
  规模天然更小。
- 复用 git log 设计已经交付的协议/JS 层（`EditorCommand::
  SetDiffDocument`、`mode=diff` 的 `unifiedMergeView` 挂载、二进制/512KB
  判定函数），不重复实现。
- 二进制/超限/磁盘文件已被删除：优雅降级到占位文案。

### 非目标

- 不改变现有"回滚到某个历史版本"（`RollbackRequest`/`rollback_to`）的行为
  或入口——回滚仍是直接写磁盘的按钮操作，不经过 CodeMirror，diff pane 恒
  只读。
- 不做并排双栏视图（同 git log 设计）。
- 不重新设计独立窗口机制本身——继续沿用 `platform/overlay_window.rs` 的
  现状，只是新增 webview 子视图。

**依赖前提**：本设计的实现必须晚于 git log 设计的协议/JS 部分落地（
`EditorCommand::SetDiffDocument` 变体、`web/editor` 的 `mode=diff`
`unifiedMergeView` 挂载逻辑、blob 二进制/大小判定函数）——本设计直接复用
这些产出，不重新实现一份。若两份计划并行执行，本设计对应的实现任务必须
排在 git log 那份的协议/JS 任务之后。

## 架构

### 单一窗口绑定的 webview，不进主窗口的池

`FileHistoryOverlay`（`platform/file_history_overlay.rs`）新增字段
`diff_webview: Option<wry::WebView>`。生命周期规则：

- 弹窗打开（`FileHistoryOverlay::open`）时不立即创建——保持"按需建"，
  第一次选中的文件可渲染且内容加载完成时才 `build_as_child(&self.window)`
  创建。
- 弹窗关闭（`FileHistoryOverlay` 整体被 drop）时 `diff_webview` 随之
  drop，不需要额外清理逻辑（`wry::WebView` 的 `Drop` 已处理底层资源）。
- 选中的文件切换：webview 实例不销毁重建，复用 git log 设计"内容通过
  `SetDiffDocument` 命令换、`revision` 递增"的既有手法。
- 选中的文件变成不可渲染（二进制/超限/磁盘读取失败）：webview 保留挂载
  （避免频繁创建/销毁的开销与画面闪烁），但不发送新内容，`diff_pane_view`
  改渲染 iced 占位文案覆盖在原本 webview 应该在的区域——具体是"隐藏
  webview 矩形（同主窗口 `App::preview_desired` 现有的"某些浮层需要显式
  隐藏 webview"那一套手法，这里是同一个原理但作用在这扇独立窗口自己的
  webview 上）+ iced 渲染占位文案"，不是销毁 webview。

### IPC：复用同一个 winit event loop 的 proxy

`FileHistoryOverlay::open()` 已经拿到 `el: &ActiveEventLoop`（见
`file_history_overlay.rs:75`）。winit 多窗口应用共享同一个 `EventLoop`/
`EventLoopProxy<Message>`，不需要为这扇窗口的 webview 单独接一套事件泵：
`WebViewBuilder` 的 IPC handler 闭包里拿到的 `proxy`（跟主窗口
`sync_webview_pool` 里传入的是**同一个** `EventLoopProxy<Message>` 实例，
从 `App`/`Runtime` 持有的那一份 clone）即可把 `EditorEvent`/
`WebviewEnvelope` 消息投递回主 `Message` 循环，由 `file_history::update`
处理——不需要 `FileHistoryOverlay` 自己维护消息分派。

### 协议 handler 复用

`WebViewBuilder` 的自定义协议注册（`dozer://editor/` scheme handler）在
`assets.rs` 里已经是一个独立、无状态的函数（供 `runtime.rs` 的
`WebViewBuilder` 调用）。这扇弹窗窗口的 `WebViewBuilder` 直接调用同一个
函数注册,不需要另写一份。

### 绑定信息

复用 `EditorHostBinding`（同 git log 设计），固定值：`project_id = 0`，
`panel`（选一个已有变体，字段只用于 envelope 自洽校验，不是真的"文件
历史属于这个面板"——这扇窗口的 webview 完全不进入主窗口 `sync_webview_pool`
的 `pool: HashMap<usize, ...>`，不会跟任何真实面板的 key 冲突，选哪个
变体都安全；直接选 `PanelKind::Files`，因为文件历史本来就是从 Files
面板右键触发的，语义上最贴近）、`tab_id = 0`。`path` 用当前
`FileHistoryTarget.file_path`。

### 几何

`FileHistoryOverlay::redraw()`/`reposition()` 新增一步：按
`file_history::file_history_card` 卡片布局里 diff 区域（右侧/下方，具体
取决于 `2026-09-17-file-history-popup-design.md` 已定的卡片布局）算出
webview 矩形（逻辑坐标转物理坐标同 `OverlayGpu` 现有 `reconfigure` 的
换算手法），矩形变化时调用 `wry::WebView::set_bounds`（同
`runtime.rs:342` 现有用法一致的 API）更新。

## 数据流

1. 用户在提交列表选一项 → `file_history::Message::SelectCommit(oid)`。
   现有逻辑已经会在 `diff_cache` 未命中时 `spawn_diff`（算 patch 文本，
   继续保留——`DiffFileEntry.patch`/这边的 patch 文本字段不是本设计
   改动对象，两者并存）；新增一步：同时派发一个异步任务读取
   CodeMirror 渲染需要的 old_text/new_text：
   - old_text：`commit.tree()?.get_path(file_path)?.to_object(&repo)?
     .as_blob()`的内容（commit 树里这个路径的 blob；路径在该 commit
     不存在——理论上不会发生，因为 `build()` 已经用 pathspec 过滤过
     只收"确实改过这个路径"的 commit，但仍要处理 `get_path` 返回
     `Err` 的情况，按"不可渲染"回落占位文案，不 panic）。
   - new_text：`std::fs::read(repo_path.join(file_path))`（磁盘实时
     内容，不是 git blob；文件不存在或非 UTF-8 走"不可渲染"回落）。
   - 二进制/超限判定复用 git log 设计交付的判定函数。
2. 结果通过新增 `Message::DiffContentLoaded(repo_path, file_path, oid,
   Result<(String, String, &'static str), String>)` 回落，核对逻辑与
   现有 `DiffLoaded` 完全一致（`repo_path`/`file_path` 不匹配就丢弃——
   不能只核对 `oid`，两个不同文件的历史完全可能共享同一个 commit，见
   现有 `DiffLoaded` 文档注释）。
3. `FileHistoryOverlay` 的 `redraw`/`reposition` 路径按
   `state.diff_content_for(oid)`（新增访问器，同现有 `diff_for()`
   的镜像）是否已加载且可渲染，决定：
   - 已加载且可渲染：`diff_webview` 不存在则创建（挂空壳，等 `ready`
     后发 `SetDiffDocument`）；已存在则若 `revision` 变化就再发一次
     `SetDiffDocument`。
   - 未加载/不可渲染：`diff_webview` 若存在则隐藏其矩形（不销毁），
     `diff_pane_view` 渲染 iced 占位文案。
4. 回滚成功（`RollbackDone` 的 `Ok(())` 分支）：现有逻辑已经
   `diff_cache.clear()` 并对当前选中重新 `spawn_diff`；新增同款重新
   派发 old_text/new_text 读取任务（new_text 必须重新读磁盘，回滚已经
   改变了它）。

## 错误处理

- 二进制/超限：复用 git log 设计的判定函数与 512KB 上限，行为一致。
- 磁盘文件在弹窗开着时被外部删除：`std::fs::read` 返回 `Err` → 判定
  "不可渲染"，占位文案说明原因，不 panic。
- commit 树里找不到该路径（防御性分支，正常不会触发）：同样判定
  "不可渲染"，占位文案。
- 弹窗关闭：`FileHistoryOverlay` 整体 drop，`diff_webview` 随之释放，
  不需要额外清理。

## 测试

- old_text/new_text 读取函数的单测（复用现有 `mkrepo()` fixture）：
  正常改动、二进制短路、超限短路、磁盘文件已被删除、commit 树缺路径
  防御分支。
- `Message::DiffContentLoaded` 的目标核对单测（镜像现有
  `snapshot_loaded_ignores_result_for_different_repo_path` 的写法，
  验证 repo_path/file_path 不匹配时丢弃）。
- `diff_webview` 随 `FileHistoryOverlay` 生命周期创建/销毁的单测（弹窗
  打开且选中可渲染文件→创建；弹窗关闭→释放）。
- 协议层（`SetDiffDocument` 序列化、二进制/超限判定函数本身、JS 端
  `unifiedMergeView` 挂载）**不重复测**——已在 git log 设计的计划里
  覆盖，本设计的计划只测"新增的宿主接线"本身。
- UI 手动验证：正常改动、二进制文件、超大 diff、来回切换选中提交、
  回滚后 diff 内容正确刷新、弹窗关闭后重开不残留旧 webview 内容。
