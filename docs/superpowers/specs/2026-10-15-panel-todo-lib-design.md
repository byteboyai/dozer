# S3a 把 todo 面板抽成共享库 `dozer-panel-todo`

- 日期:2026-10-15
- 状态:待评审
- 路径:架构级(新增共享 crate,重组 `dozer-app` 里一个面板的代码归属)
- 起点:Dozer `main` 的 `7cdbffd0`(worktree `../dozer-panel-todo`,分支 `feat/panel-todo-lib`)
- 相关:`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`、`docs/dozer-v2/digger-bytehost-integration/`(用量面板当年的抽库记录:Task 19/20);Digger 一侧的上游 spec:`digger/docs/superpowers/specs/2026-10-14-window-shell-design.md`(S1)、`2026-10-15-usage-pair-design.md`(S2)

## 1. 用户意图

Digger 要重用 Dozer 的 todo、agent、群聊、会话、用量、项目、files 面板,并且 Dozer 里改了这些面板,Digger 要能同步。用量已经是共享库(`dozer-panel-usage`),S2 只是把它接进 Digger 的窗口骨架。**todo 还不是共享库**,代码在 `dozer-app/src/extensions/todo/` 里,和 Dozer 内核有耦合。所以 Digger 接入 todo(S3b)之前,要先在 Dozer 里把它抽成共享库(本文,S3a)。

用户已确认的拆分:**先做 S3a(Dozer 里抽库),再做 S3b(Digger 接入)**。

## 2. 非目标

- 不改 Dozer 任何用户可见行为(外观、交互、数据)。
- 不改 `dozerd` 里 `TodoService` 的存储实现,不改 MCP 的 todo 工具。
- 不抽 `platform/todo_detail_overlay.rs`:它依赖 Dozer 的浮层窗口与 GPU 基础设施(`overlay_window`、`overlay_gpu`),Digger 没有这套东西。
- 不升级任何依赖,不顺手抽别的面板。
- 不做 Digger 的任何改动(S3b)。
- 不决定 Digger 的 todo 语义(Dozer 文档 Q11:"选题 vs 验收项")和 Digger 的存储。那是 S3b 的问题。

## 3. 现状

| 部分 | 位置 | 规模 |
|---|---|---|
| 面板 Rust 代码 | `crates/dozer-app/src/extensions/todo/{mod,state,update,protocol,filter,category_drag,view}.rs` | 约 4,300 行 |
| 前端应用(TSX,日历 / 拖拽排序 / 筛选等) | `crates/dozer-app/web/todo-content/` → 构建到 `assets/todo-content/` | 含自己的单元测试 |
| 本地化文案 | `crates/dozer-app/locales/panels/todo/` | |
| 原生详情浮层 | `crates/dozer-app/src/platform/todo_detail_overlay.rs` | 193 行 |
| 服务接缝 | `dozer-services::TodoService`(trait)、`dozer-model::todo`(数据类型) | 已是共享 |
| 存储后端 | `dozerd/src/todo.rs`、`todo_category.rs` | 不动 |

耦合面(逐文件核对过,共这几类):

| 依赖 | 出现在 | 处理 |
|---|---|---|
| `crate::app::{HoverId, PanelKind}`、`home_panel_head`、折叠按钮、`tab_divider`、`workspace::agent_icon` | `view.rs`、`protocol.rs` | 注入:`PanelDisplay` 适配 trait |
| `crate::app::{tree_drag_past_threshold, tree_drag_held_long_enough}` | `category_drag.rs` | 纯函数,搬进新库 |
| `crate::app::TextInputTarget`(文本框右键菜单) | `state.rs`、`view.rs` | 库只发携带不透明目标的消息,产品弹菜单 |
| `crate::extensions::toast::{Outbox, Pending, Level}` | `state.rs`、`update.rs` | 换成库自己的发件箱类型,产品排空 |
| `crate::panel_host::{RequestTrackers, ActivationCtx, scope_of, project_of, RequestTokenWrapExt}` | `state.rs`、`update.rs`、`mod.rs` | 换成库自带的版本(用量库已有同型的 `ActivationCtx`) |
| `crate::i18n::HOST_OWNER`、`crate::theme` | `protocol.rs`、`view.rs` | 库自带 `i18n` 模块(同用量);主题走 byteui |
| `crate::service_fakes::FakeTodo`(仅测试) | `mod.rs` 测试 | 测试替身随库走 |
| 详情浮层(`todo_detail_overlay.rs` 调 `extensions::todo`) | `platform/` | 留在 Dozer;库只发"请求打开详情"消息 |

## 4. 设计

### 4.1 crate 边界

新建 `crates/dozer-panel-todo`,与 `dozer-panel-usage` 并列。**依赖闭包不含 `dozer-app` / `dozer-client` / `dozer-core`**:允许 `dozer-model`、`dozer-services`、byteui、`bytehost-core` / `bytehost-desktop`、iced、`serde*`。

```
dozer-panel-todo/
  Cargo.toml
  src/lib.rs          公开面,重新导出
  src/state.rs        WorkspaceState、Message、on_activate、take_outbox
  src/update.rs       update
  src/protocol.rs     webview 协议(parse_*_event / 推送编码)
  src/filter.rs       筛选
  src/category_drag.rs 分类拖拽(含从 app 搬来的两个阈值纯函数)
  src/view.rs         content_pane / list_pane
  src/adapter.rs      PanelDisplay、TodoOutbox / TodoNotice、请求追踪与激活上下文
  src/i18n.rs         本库文案(locales/ 随库)
  assets/todo-content/ 构建产物(唯一来源)
  web/todo-content/   TSX 源码与测试
  locales/            本地化文案
```

`web_assets_root()` 返回库内 `assets/todo-content`,Dozer 与将来的 Digger 都从这条路径取资源,不各留副本(同用量)。

### 4.2 适配接缝

- **`PanelDisplay<M>`**(同用量的同名 trait,形状沿用,只加 todo 确实需要的方法):面板头、折叠按钮、折叠悬停、分隔线、agent 图标与颜色。`M` 是产品的顶层消息类型;库的 `Message` 由产品用 `wrap` 包进去。
- **发件箱**:`TodoOutbox` / `TodoNotice`(同用量的 `UsageOutbox`),产品排空后转成自己的 toast。
- **请求追踪与激活**:库自带 `ActivationCtx` 与请求代数管理,不依赖 `dozer-app::panel_host`。Dozer 一侧的 `panel_host` 保持原样,仅在组合处把自己的运行时句柄交给库的 `ActivationCtx`。
- **文本菜单 / 打开详情**:库发 `Message::TextInputMenuRequest(..)` 与 `Message::OpenDetail(todo_id)`,产品决定怎么响应。Digger 暂不接详情浮层,面板其余功能不受影响。

### 4.3 Dozer 一侧

`dozer-app` 的 `extensions/todo/` 缩成"重新导出 + 注册包装":
- 把库的 `Message` 纳入顶层 `Message`;
- 实现 `PanelDisplay`(用现有的 `HoverId`、`PanelKind`、`home_panel_head` 等);
- 在 `PanelSelect(PanelKind::Todo)` 分支构造库的 `ActivationCtx` 调 `on_activate`;
- 处理 `OpenDetail` / 文本菜单消息,打开现有的详情浮层。

## 5. 分步(每一步 Dozer 都能编译且测试全绿)

1. **起步基线**:在 worktree 里记录 `dozer-app`、前端各自的测试数量与结果,作为抽库前后对比的基准。
2. **建空壳 crate,搬纯逻辑**:`filter`、协议编解码、拖拽阈值函数。Dozer 改引用新库。
3. **搬状态与更新**:换上库自带的请求追踪、发件箱、激活上下文。
4. **加 `PanelDisplay`,搬视图**:Dozer 实现适配。
5. **搬前端应用与本地化文案**:构建脚本改到库内;Dozer 的资源引用改指向 `web_assets_root()`。
6. **清理**:删掉 `dozer-app` 里无调用者的旧路径,确认没有残留的重复实现。

## 6. 验证

- **测试数量守恒**:基线与终态,Rust(`dozer-app` + 新库)与前端的测试总数相同,全绿;被搬走的测试随代码一起走,不删、不改期望值。
- **前端构建产物对比**:抽库前后构建一次,比较产物(忽略时间戳等非内容差异)。
- **依赖闭包检查**:新库 `cargo tree` 不含 `dozer-app` / `dozer-client` / `dozer-core`(沿用用量库的同类检查方式)。
- **无重复实现**:grep 确认 Dozer 里不再保留 todo 的状态 / 更新 / 视图副本。
- **做不到、留给用户**:Dozer 里 todo 面板的外观与交互目测(增删改、拖拽排序、分类拖拽、日历、详情浮层)。

## 7. 风险

1. **Dozer 同时在做 i18n 与依赖升级**,文案和 `locale` 参数可能与搬动冲突。缓解:固定起点、分支短命、搬动不改文案内容。
2. **`PanelDisplay` 形状过窄或过宽**。缓解:沿用用量已走通的形状,只加 todo 必须的方法;不在这里设计"通用面板 trait"。
3. **前端构建产物在不同机器上不完全一致**。缓解:比较时忽略时间戳等非内容差异,只比内容。
4. **推送门槛**:Digger 通过 git `rev` 依赖 Dozer,S3b 需要 3a 的某个已推送提交。合入主线与推送由用户决定,不是 3a 自己的步骤。
5. **详情浮层留在 Dozer 意味着库里有一条"打开详情"的出口消息**,Digger 起步时不处理它。需要在 S3b 里明确面板这一功能的缺位对用户可见的影响。

## 8. 交付物与 3b 的接口

3a 完成时,库的公开面就是 3b 需要的全部:`WorkspaceState`、`Message`、`update`、`on_activate`、`content_pane` / `list_pane`、`PanelDisplay`、`TodoOutbox`、`web_assets_root()`。Digger 另需实现 `TodoService`(S3b)。
