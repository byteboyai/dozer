# Browser 面板(收藏夹)迁移到服务 trait(第九个面板)

> 日期:2026-10-11。状态:待用户审阅。依据:`2026-10-09-dozer-services-x0-design.md`(接缝形状、`ServiceError`)、`2026-10-11-conversation-services-design.md`(`Loaded` 载荷改 `Result`、不新增消息变体与 host 臂的先例)、`docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`。

## 1. 已对齐的理解

**目标(沿用前几个面板):** 浏览器面板的收藏夹(`extensions/browser.rs`)可进程内测试/复用——取数/写数只依赖小 trait,测试注入内存实现。
**范围:** 只迁该文件里的 3 个服务调用:`list_bookmarks`、`add_bookmark`、`remove_bookmark`,以及 host 的调用点(`app/update.rs`、`workspace/state.rs`)。
**明确不在范围:** `settings`、`project_create`;其它 host 调用。
**不变量:** 线上协议零变化;乐观更新(先本地插入/移除再发 RPC)逻辑不变;**写操作失败文本逐字不变**(`BookmarksMutated(Err)` → 面板内联 `state.error`)。

**用户已确认的失败口径(用户可见的行为变化):** 收藏夹读取(刷新)失败时——
- 保留本地缓存,**不再清空**(此前 `unwrap_or_default()` 会把失败变成 `BookmarksLoaded(_, vec![])` 整体替换缓存);
- `Unavailable` 静默(顶栏徽标已表达"dozerd 不可用"),写一条 debug 日志;
- 其余(`Rejected`/`Protocol`)写进面板已有的内联 `state.error`,文案 `"收藏夹加载失败: {message}"`(与该面板对写失败的既有内联做法一致,UI 里显示为 `⚠ …`)。

## 2. 现状(读代码得到)

- 三个异步入口:
  - `request_bookmarks_refresh(project_id: Option<i64>, client, handle, emit)`:`list_bookmarks(project_id).await.unwrap_or_default()` → `Message::BookmarksLoaded(project_id, Vec<BookmarkInfo>)`;
  - `update` 的 `Message::BookmarkAdd(scope)`:乐观插入后(`Project` scope 且无 `project_id` 时只改本地)`add_bookmark(scope, target_project_id, &url, &title)` → `Message::BookmarksMutated(project_id, Result<(), String>)`;
  - `update` 的 `Message::BookmarkRemove(id)`:乐观移除后 `remove_bookmark(id)` → `BookmarksMutated`。
- `BookmarksMutated` 之后总是调 `request_bookmarks_refresh`。所以 dozerd 不可用时:这次增删失败,紧接着的刷新也失败,`unwrap_or_default()` 把整个收藏夹(含刚才乐观插入的那条)清空——与 Usage 的 G4 同类缺陷。
- 写失败已可见:`state.error = Some(message)`,视图在地址栏下方渲染 `⚠ {err}`。`State` 是 per-browser 实例(首页 `App.home_browser` + 每工作区 `ws.browser`),**没有 outbox**;`App.home_browser` 不在 `Workspace` 里。
- host 路由:`Message::Browser(BookmarksLoaded(pid, …))` / `BookmarksMutated(pid, …)` 各一个专属臂(`app/update.rs` 约 2899–2904 行),分别进 `browser_bookmarks_loaded` / `browser_bookmarks_mutated`(约 4910–4990 行,`pid` 为 `None` 走 `home_browser`,否则 `with_project`)。
- host 调用点:`app/update.rs` 里 12 处 `browser::update(…, &client | &self.client | &io.client, …)`(首页浏览器与各工作区浏览器,含 `BrowserTitle`/`BrowserNavigated`/`BrowserNewWindow` 等),`workspace/state.rs` 里 2 处 `browser::request_bookmarks_refresh(…, &io.client, …)`。
- 本文件没有 `LOG` 声明;面板日志来源名应是 `web`(`PanelKind::Web` 的面板名,`app/state.rs` 的测试会扫源码强制)。
- 现有测试:约 13 处 `client_for_test()`(指向不存在 socket 的 `Client`)喂给 `update`,断言本地状态(乐观更新、`state.error` 等);`R-CLIENT` 基线:`extensions/browser.rs` 为 1(`use dozer_client::Client;`)。

## 3. 设计

### 3.1 `BookmarkService`(放进 `dozer-services`,仍只依赖 `dozer-model`)

| 方法 | 返回 |
|---|---|
| `list_bookmarks(project_id: Option<i64>)` | `Vec<BookmarkInfo>` |
| `add_bookmark(scope: BookmarkScope, project_id: Option<i64>, url: String, title: String)` | `()` |
| `remove_bookmark(id: i64)` | `()` |

参数按值传,返回 `BoxFuture<'_, Result<_, ServiceError>>`;`BookmarkInfo`、`BookmarkScope` 已在 `dozer-model`。`Services` 增加 `bookmark: Arc<dyn BookmarkService>`;`Client` 实现该 trait(调用同名 inherent 方法,`map_err(into_service_error)`)。

### 3.2 面板与 host

- `Message::BookmarksLoaded` 的载荷改为 `(Option<i64>, Result<Vec<BookmarkInfo>, ServiceError>)`,**不新增消息变体、不新增 host 臂**(`R-HOST-PANEL-ARMS` 不上升);`browser_bookmarks_loaded` 的 `bookmarks` 参数随之改为该 `Result`。
- `update` 对 `BookmarksLoaded(_, result)`:
  - `Ok(b)` → `state.bookmarks = b`(不变);
  - `Err(e)` → 缓存保持不动;`e.kind == Unavailable` 只写 `dozer_core::log_debug!(LOG, …)`;其余 `state.error = Some(format!("收藏夹加载失败: {}", e.message))`。
- 面板顶部声明 `dozer_core::scope!(LOG, panel, "web");`(第一次写日志才声明)。
- `request_bookmarks_refresh`:`list_bookmarks(project_id).await` 直接作为 `BookmarksLoaded` 的 `Result` 发出,不再 `unwrap_or_default()`。
- `update` 的 `client: &Client` 改为 `service: &Arc<dyn BookmarkService>`(与 Todo 的先例同形);`BookmarkAdd`/`BookmarkRemove` 的 spawn 用 `service.clone()`,`.map_err(|e| e.to_string())` → `.map_err(|e| e.message)`(文本相同);`BookmarksMutated` 之后的刷新照旧。
- host:12 处 `browser::update` 与 2 处 `request_bookmarks_refresh` 把各自手里的 client(`&self.client` / `&client` / `&io.client`)换成 `&crate::panel_host::services_for(<client>).bookmark`(逐处机械替换;已有 `let client = …clone()` 的地方改成 `let service = services_for(&…).bookmark`)。

### 3.3 为什么读失败写进内联 `state.error` 而不是 Toast

浏览器 `State` 没有 `outbox`,且首页 `home_browser` 挂在 `App` 上、不在 `Workspace` 里——走 Toast 要给 `State` 加 outbox 并在 `drain_outboxes` 里额外排空 `home_browser`,改动面更大。沿用该面板对写失败的既有内联做法(`state.error` + `⚠`)最小,且用户已确认。

## 4. 步骤(每步先写失败测试;每步一个提交)

- **B0 基线:** 记录测试数、clippy 警告数、`R-CLIENT` 中 `browser.rs`(1)、门禁输出。
- **B1 `BookmarkService` trait:** `dozer-services` 契约测试(内存实现、`tokio::spawn` Send 检查、读失败可与空列表区分)。
- **B2 `Client` 实现:** 回放服务端适配测试(`list` 成功、`remove`/`add` 返回 `()`、`HangUp → Unavailable "daemon 断开"`、`ErrorReply → Rejected "daemon 错误: boom"`)。
- **B3 `Services.bookmark` + 假实现:** `services_for` 与 `base_services()` 补字段;进程内 `FakeBookmark`(预设列表、可让读/写失败、记录调用)。
- **B4 面板与 host 切换:**
  - 面板层测试(用 `FakeBookmark`):`request_bookmarks_refresh` 成功得到 `BookmarksLoaded(pid, Ok)`、失败得到 `BookmarksLoaded(pid, Err(kind/文本))`;`update(BookmarksLoaded(Err))`——`Unavailable` 保留缓存且不写 `state.error`,`Rejected` 保留缓存并写 `"收藏夹加载失败: …"`;`BookmarkAdd` 把 `scope`/目标项目/URL/标题原样传给服务,`Project` scope 且无 `project_id` 时不调服务;`BookmarkRemove` 调 `remove_bookmark(id)`;写失败得到 `BookmarksMutated(_, Err(服务消息))`,文本逐字不变;`BookmarksMutated` 之后触发一次 `list`。
  - 现有约 13 个 `update_*` 测试把 `client_for_test()` 换成 `FakeBookmark` 服务(`R-CLIENT` 才能清零),断言不变。
  - 探针 `host_site` 行号如因替换折行漂移,只改行号。
- **B5 门禁与验收:** `R-CLIENT` 基线中 `browser.rs` 清零;回填 `08-client-use-inventory.md`、`CLAUDE.md` 的 `dozer-services` 行(Usage、Todo、Project、GroupChat、Conversations、CodeHealth、AgentContext、EditHistory、Browser 已迁)。

## 5. 验收标准(机器可验证)

1. `cargo tree -p dozer-services --edges normal` 仍只有 `dozer-model` 与 serde 系。
2. `extensions/browser.rs` 不再含 `dozer_client` / `io.client()`;`R-CLIENT` 在该文件为 0,基线只降。
3. 读失败保留缓存且按档提示、写失败文本逐字不变、`add` 参数传递、`Project` scope 无项目时不发 RPC,均有测试(B4)。
4. 全量测试数 ≥ 基线 + 新增;已知失败(`files::delete_confirm_spec_reflects_pending_target`、并行负载下的间歇失败、`R-HOST-EXECUTORS 0->1`)不算回归也不顺手修。
5. `R-HOST-PANEL-ARMS` 不上升;clippy 警告数不高于基线;`cargo fmt --check`、`scripts/check-log-scope.sh` 通过(含面板日志来源名必须是 `web` 的测试);`model_golden` 仍通过。

## 6. 明确不做

- 不给 `State` 加 outbox、不改 `drain_outboxes`;不改乐观更新逻辑;不改写失败的展示。
- 不迁 `settings`/`project_create`;不改线上协议;不给 `BookmarksLoaded` 补代数保护(登记为后续项)。

## 7. 风险与已知问题

- **host 替换点多(14 处):** 全部一对一,但分布在 `app/update.rs` 的多个分支与闭包里(有的持有 `&self.client`,有的在 `with_focused_project` 闭包里持有 `ShellIo` 的 `&io.client`)。靠编译器逐个找出;探针 `host_site` 预计不漂移,漂移则只改行号。
- **`BookmarksLoaded` 载荷类型变化:** 波及 `browser_bookmarks_loaded` 与两个构造点;编译期可见。
- **现有测试改动面:** 约 13 个 `update_*` 测试只换辅助函数,不改断言;若某个测试依赖"指向不存在 socket 的失败"这一副作用,改为显式用失败的假实现。
- **`Cargo.lock` 噪声:** main 上已提交的锁文件缺 bytegit/byteui 的 `source` 行,干净 worktree 里跑 cargo 会补上;提交前 `git checkout Cargo.lock`(同前几次)。
- **WIP 文件:** 用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist`)在 main 工作区未提交;在独立 worktree 实施,不提交 `Info.plist` 与版本号改动。

## 8. 分支与执行

规格分支 `spec/bookmark-services`(worktree `../dozer-bookmark-spec`);实施另开分支。执行计划由 writing-plans 产出,待本规格审阅通过后进行;合并进 main 前需询问用户。
