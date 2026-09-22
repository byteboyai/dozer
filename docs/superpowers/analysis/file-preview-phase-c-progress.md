# 文件预览 Phase C 进度(大文件 / 恢复 / 资源管理)

> 对应计划:`docs/superpowers/plans/2026-09-22-large-file-session-resource-management.md`。
> 2026-09-22。

## 已完成(纯逻辑模块,不碰热点文件)

### Task 1:文件策略决策器 `preview/file_policy.rs`
- `TextFilePolicy { EditableCode, ReadOnlyHighlighted, ReadOnlyPlain, Windowed }`。
- 系数 9x/6x/4x;绝对护栏 30/64/128MiB;单行规则 >100KiB 关 wrap、>1MiB 关
  高亮/折叠、>5MiB 强制 Windowed。
- `decide_text_policy(profile, budgets) -> TextPolicyDecision`,带
  `estimated_bytes` / `guard_bytes` / `capabilities` 与**可展示的具体 reason**
  (含文件大小、最长行、预算),不是"大文件"bool。纯函数、`saturating_mul` 防溢出、
  单次强制不在此改全局预算(调用方语义)。
- 测试:低/高预算、30/64/128MiB 边界、单行三档、溢出饱和、单调性。

### Task 2:流式搜索与稀疏行索引 `preview/large_text.rs`
- `stream_search(path, query, SearchOptions) -> SearchOutcome`:逐行流式,返回
  1-based 行列 + 有限摘要;命中列表封顶但**保留匹配总数**与 `truncated`。
- `LineIndex::build/build_cancellable`(每 N 行一个 byte offset,可取消)、
  `offset_for_line`(最近索引 seek 后有限扫描)、`read_window`(目标附近有界
  窗口 + 全局行号)、`is_valid_for(revision)` 失效判定。
- 测试:行/列(含 UTF-8 字符列)、CRLF、命中封顶但总数保留、空 query、逐行
  offset 精确、无尾换行、空文件、窗口钳位、可取消。

### Task 4:全局资源管理器 `preview/resources.rs`
- `ResourceManager` 跨项目登记 `ViewerRegistration`(估算字节、重型 WebView、
  active/dirty/has_recovery/saving/agent_writing、LRU `last_accessed`)。
- `try_reserve(bytes, heavy, current_project) -> Granted | NeedEviction(keys) |
  Denied`:内存与重型名额**同时**满足;不偷偷淘汰,只报告淘汰谁(销毁后再归还
  预算)。
- `eviction_order`:后台项目干净 → 后台有 recovery 脏 → 当前项目非活动干净 →
  当前项目有 recovery 脏,组内 LRU。不可淘汰:active/saving/agent 写入/
  无 recovery 脏 tab。
- `diagnostics()` 诊断快照。测试:预算/名额、不可淘汰跳过、脏有无 recovery、
  淘汰顺序、LRU、超大单件拒绝、release 幂等、重复登记不叠加。

### Task 6:Dirty recovery snapshot `preview/recovery.rs`(纯逻辑)
- 版本化 `RecoveryManifest`(path / 基准 mtime+len / editor revision / encoding /
  line-ending / cursor / selection / top_line)+ 正文快照,写到
  `<config_dir>/preview_recovery/`;`write_snapshot` 临时文件 + rename 原子写。
- `read_snapshot` 缺文件/损坏/未知版本一律 `None`(不 panic);
  `clear_snapshot` 正常保存后清理;`classify_recovery` 对照磁盘 → Restore /
  ConflictDiskChanged / ConflictMissingFile。
- 测试:round-trip、缺失/损坏/未知版本、空正文、磁盘未变恢复、磁盘变更冲突、
  文件缺失冲突、清理无残留、原子写无临时残留、真实 `profile_file` 集成。

### Task 5:Suspended tab 恢复(已接线)
- `restore_preview_state` 现在**只建 Suspended 壳**(`push_shell_tab`:route/
  backend 定好,但不读盘、不建 viewer/WebView),首屏只物化**当前项目当前
  文件**;没有 active 时物化第一个。历史 tab 再多,启动工作量不随 tab 数线性
  增长。
- `hosts_webview` 排除 `Suspended`;切 tab(`preview_select_tab`)/重新打开已
  存在的壳(`preview_open_path_at`)时经 `Workspace::load_preview_tab` 按 route
  物化:CodeMirror 直接 Ready(正文由 host 自取);老 iced/JSON/Streamed 走
  异步读盘 + `apply_native_load`;Tabular 走后台加载;渲染/外部直接就绪。
- 持久 mode 只在该 route 支持时覆盖默认值(Json Tree/Text 已同步到 backend)。
- 测试:壳 Suspended 且不进 WebView 池、物化 Loading→Ready 幂等、持久 mode
  覆盖与失效回退。

### 长单行探测 + `file_policy` 接线
- `file_profile::profile_file` 增加**有界首行探测**:采样窗(64KiB)内无换行且
  文件更大时,最多再读 6MiB 探首行长度 —— 否则 Phase C 的"关 wrap(>100KiB)/
  关高亮(>1MiB)/强制窗口化(>5MiB)"单行规则永远触发不了。探到的是首行下界。
- `route_and_backend` 的只读判据改用 `file_policy`:非 EditableCode(超 30MiB/
  超预算)或非文本 → 只读。
- `PreviewTab.windowed`(由 `path_is_windowed` 定)+ `uses_codemirror()` 排除
  窗口化文件;`is_native_editor_candidate`/`push_tab`/`desired_editor_webviews`
  协同,让窗口化文件在 feature 打开时**仍走老 iced 分块只读**,不整载进
  CodeMirror WebView(窗口化专用 viewer 留待后续)。

### Task 3:Windowed Viewer(已接线)
- **协议**:`EditorCommand::SetWindow { text, start_line, total_lines, revision }`
  与 `EditorEvent::WindowRequest { edge, anchor_line }`(`WindowEdge`);前后端
  镜像 + 解析/编码测试。
- **前端**(`web/editor`):URL `windowed=1` 进入窗口化只读模式 —— 不自行拉取
  正文;`set_window` 替换为窗口内容并用 `lineNumbers({formatNumber})` 显示
  **全局行号基数**;选区/viewport 事件换算成全局 1-based 行号对外;滚到持有
  窗口边界时(节流 400ms)发 `window_request`;`replace_range`/⌘S 恒拒绝。
- **Rust**:`PreviewTab.window_index`(稀疏索引)+ `uses_windowed_editor()`;
  `desired_editor_webviews` 对窗口化 tab 产出带 `windowed=1` 的 editor spec;
  editor `Ready` 后后台建 `LineIndex` → `Message::PreviewWindowIndex` → 推初始
  窗口(`Workspace::queue_windowed_view`,以目标行/首行为中心,`WINDOW_BEFORE=1000`
  /`WINDOW_AFTER=2000` 读有界窗口);`WindowRequest` 回推相邻窗口;Agent jump
  在索引就绪后先装窗口再 `reveal_position`(命令按队列顺序注入)。
- `hosts_webview` 对窗口化 tab 返回 false(走 editor host,不另起 Flyfish)。

## 仍未完成

1. **Task 6 接线**:把 `recovery.rs` 接进脏 tab 的防抖快照、启动恢复、保存后
   清理(需要 editor host 新增"周期上报正文"事件或保存/失焦时快照)。
2. **Task 7 安全启动与运行时反馈**:启动进行/完成标记、失败计数、ready latency。
3. Windowed 的折叠/全文搜索明确禁用(已只读、无保存);恢复时把持久化的
   cursor/selection/scroll anchor 应用到编辑器视图。

## 验证

- `cargo check`/`build -p dozer-app --all-targets`(默认与 `--features codemirror`):
  通过(链接成功)。
- `cargo test -p dozer-app`:默认 **1248 passed / 0 failed**、feature **1231
  passed / 0 failed**(另有 1 ignored)。
- 前端:`tsc --noEmit`、`npm test`(6 passed)、`npm run build` 通过(产物已更新)。
- `cargo fmt --check`、`cargo clippy` 干净(仅既有 `file_history.rs` warning)。
