# Task 02 API 决策：窗口入口、生命周期与泛型

> 计划 Task 02 产出，回填规格 §11 闸门 1（iced×wry 窗口集成）。
> 证据来源：`D/spike/editor-host/`（真实 iced 窗口 + wry 子视图，已跑通 `SELFTEST=basic/policy/merge`）、`Z/crates/dozer-app/src/{app,platform,preview}`、`B/spike/desktop-contract/`（本任务新增探针）。
> 所有签名以**编译成功**为准；探针资源最小化，不伪装成已迁生产面板。

## 1. 共同 iced/wry 版本（不看 README，只看锁与依赖）

从实际依赖确定，不凭文档猜兼容：

| 来源 | iced | wry | winit |
|---|---|---|---|
| `Z/Cargo.lock`（Dozer，`a4cbf340`） | `iced_wgpu 0.14.0` / `iced_widget 0.14.2` / `iced_winit 0.14.1` | `0.55.1` | `0.30.13` |
| `D/spike/editor-host/Cargo.toml` | `iced = "0.14"`（feature `tokio`） | `wry = "0.55.1"` | 传递自 iced |
| `byteui`（Dozer 依赖） | iced 0.14 生态（与 Dozer 同锁） | — | — |

**决定：** 桌面宿主层统一 `iced = "0.14"` + `wry = "0.55.1"`，与 `byteui 0.4.1` 的 iced 版本一致；升级时 `iced`/`byteui` 必须一起升（CLAUDE.md 既有约束）。探针 `B/spike/desktop-contract` 用同一组版本独立验证。

## 2. 窗口入口（复用现有事件循环，不新建竞争循环）

`D/spike/editor-host/src/main.rs:1224` 的可复用入口模式：

```rust
fn main() -> iced::Result {
    iced::application(boot, update, view)
        .title("…")
        .subscription(subscription)   // window::resize_events() + IPC 流
        .window_size(Size::new(1100.0, 720.0))
        .run()
}
```

关键约束（`main.rs:42-46,496-504,580-590,1206,1224`）：

1. **唯一事件循环**：`iced::application(...).run()`。宿主不另起 winit 循环；wry 子视图在主循环内创建。
2. **wry 子视图在主线程、用 `window::run` 创建**（`main.rs:580-590`）：
   ```rust
   window::run(id, move |w| {
       let (wb, _) = hosts::builder(&spec);
       let wv = wb.with_html(html).build_as_child(&w).map_err(|e| e.to_string())?;
       HOST.with(|h| *h.borrow_mut() = Some(wv));
       Ok(())
   }).map(Message::HostBuilt)
   ```
   `window::run` 返回 `Task<Message>`，与其它任务合流；创建失败经 `Message::HostBuilt(Err)` 回传，不 panic。
3. **非 Send 句柄放 thread_local**：`thread_local! { static HOST: RefCell<Option<wry::WebView>> }`（`main.rs:42-44`），另有 `PROBES: RefCell<Vec<WebView>>`（`main.rs:476-478`）。`WebView` 非 `Send`，不得跨线程；所有操作（`set_bounds`/`evaluate_script`/`focus`）经 `window::run` 或 `HOST.with`（主线程）。
4. **IPC 上行经 channel → iced stream**：wry `ipc_handler` 只 `tx.send(body)`（`hosts.rs:209-215`），主线程侧 `Subscription::run` 从 tokio `unbounded_channel` 收进 `Message`（`main.rs:1213-1220`）。宿主回调不阻塞 UI 线程。
5. **下行用 `evaluate_script`**（`main.rs:539-546`）：`window::run(id, move |_| HOST.with(... evaluate_script(&js)))`。

**决定：** desktop 层采用同一入口——`iced::application` 单循环 + `window::run` 建/驱动 wry 子视图 + thread_local 持非 Send 句柄 + channel↔Subscription 桥接 IPC。Dozer 现有 `Z/crates/dozer-app/src/platform/window.rs` 与 `runtime.rs::build_app_webview` 已是此模式的既有实现，迁移时对齐而非重写。

## 3. 几何、DPI 与原生 overlay

- **子视图矩形 = 逻辑坐标**：`hosts.rs` 用 `wry::Rect { position: LogicalPosition, size: LogicalSize }`（`hosts.rs:243-246`）；DPI 缩放由 wry/winit 处理，调用方只给逻辑值。
- **resize 事件驱动**：`window::resize_events()`（`main.rs:1212`）→ `Message::Resized(id, size)` → 重算并 `set_bounds`。
- **webview 恒在 iced 之上**：任何盖在宿主上的 iced 元素（顶栏、恢复条、冲突条）都必须把它**下推**。spike 的 `host_top()`（`main.rs:302-306`）汇总所有条高，`sync_bounds()`（`main.rs:309-313`）在每次消息后比对 `applied_top` 并重设 bounds。Dozer 的等价机制是 `App::preview_desired` + `webview_geometry`（CLAUDE.md 明示）。
- **原生 overlay**：独立原生子窗口（Dozer `platform/overlay_window.rs`）天然叠在 webview 之上，不需要下推逻辑；这是新浮层的默认机制。

**决定：** 几何/DPI 逻辑进 `bytehost-core`（纯 DTO/计算，无 iced 依赖）；`set_bounds` 调用与 webview 生命周期进 `bytehost-desktop`（持有 iced/wry）。下推语义由 desktop 层统一（"所有 iced 条高汇总后下推 webview"），与 Dozer `preview_desired` 对齐。

## 4. 生命周期（创建/隐藏/恢复/销毁，无重复原生实例）

spike 观察（`main.rs:496-504,597-607`）：wry 子视图在整个窗口生命周期内**只创建一次**；隐藏/恢复靠 bounds 归零/恢复或直接复用同一实例，不重复 `build_as_child`。

**决定：**
- 一个 Surface↔一个 wry 实例（`thread_local` 池 + 稳定 slot id）；重启面板不重建实例，只改 bounds/内容。
- 创建失败（`HostBuilt(Err)`）不 panic；交由上层转为面板可见错误。
- 销毁：窗口关闭时 drop `thread_local` 池；不跨窗口复用（对齐 Dozer `APP_CONTENT_ID_OFFSET` 的 id 段反查）。

## 5. 泛型与类型契约（双产品编译证据）

Task 02 要求：**两个不同 ProductMessage/服务上下文类型编译相同 Host 回调接口**，并验证 iced `Element` 生命周期、异步 `Send`、主线程非 `Send` 句柄边界。

**拟签名（探针中编译通过，见 §7 证据）：**

```rust
// bytehost-core：纯描述，无 iced/wry 依赖
pub struct SurfaceSpec {        // 一个 Surface 的描述：种类、绑定、初始内容、bounds(slot)
    pub slot: SurfaceSlot,
    pub binding: Binding,        // project/panel/document
    pub content: SurfaceContent, // Html / Url / Empty
}

// bytehost-desktop：持有 iced/wry；产品以泛型 M + 回调接入，不把产品类型搬进 Host
pub trait Product<M> {
    fn update(&mut self, msg: M) -> Task<M>;
    fn view(&self) -> Element<'_, M>;
    fn host_message(&self) -> Option<HostMessage<M>>; // 面板/服务上下文的包装
}

pub enum HostMessage<M> {
    Product(M),                   // 产品自己的消息
    Surface(SurfaceEvent),        // 宿主通用事件（resize/hidden/resumed/destroyed/ipc）
}
```

- **ProductMessage 泛型**：`HostMessage<M>` 对任意产品消息 `M` 成立；Dozer 用其 `Message`，Digger 用自己的 `ProductMessage`，两者都实例化同一 `Product<M>`/`HostMessage<M>`。
- **服务上下文泛型**：执行器接口接收泛型服务上下文 `C`（实现 `Services`-like trait 的形状），Tokio/iced 的具体实现不进 core。
- **Send 边界**：后台任务与 IPC channel 的内容必须 `Send`；`WebView`/`Element`/非 Send 句柄只在主线程（thread_local），不进 `Task` 的 future。
- **Element 生命周期**：`view(&self) -> Element<'_, M>` 借用 `self`，与 iced 0.14 的 `Element<'a, M>` 一致；回调经 `M` 派发，不在 view 内建 wry。

## 6. Surface 描述能表达两类推送形状

Task 02 要求：用 **Usage**（推 token/成本/时间序列）与 **CodeHealth**（推诊断列表/严重度）两种不同推送形状验证同一 `SurfaceSpec`/`SurfaceEvent` 描述够用。

| 面板 | 推送形状 | 由 Surface 描述承载 |
|---|---|---|
| Usage | 时序聚合（按天/按 agent 的 token/cost）+ 汇总条 | `SurfaceContent::Html` + `SurfaceEvent::Data(crate::SurfacePayload::Table)` |
| CodeHealth | 诊断条目列表 + 严重度分类 | `SurfaceContent::Html` + `SurfaceEvent::Data(crate::SurfacePayload::List)` |

**决定：** Surface 层只描述"一个 webview 面板 + 一个数据绑定 + 一个事件通道"，数据形状（表格/列表/时序）作为 payload 变体，**不**在 core 里假设 Usage 或 CodeHealth 的业务字段。这样两类推送都用同一描述表达。探针以一个 payload 枚举编译验证两条路径。

## 7. core / desktop 分工与编译证据

| 层 | crate | 放什么 | 依赖 |
|---|---|---|---|
| 纯描述 | `bytehost-core` | `SurfaceSpec`、`Binding`、`SurfaceSlot`、布局/路由 DTO | 仅 serde（无 iced/wry） |
| iced view 回调 + wry 实现 | `bytehost-desktop` | `Product<M>`、`HostMessage<M>`、`window::run`/`build_as_child`、`set_bounds`、IPC 桥 | `iced 0.14` + `wry 0.55.1` |
| 应用平台 | `bytehost-apps`/`-client`/`-panel`/`-webview` | 现有四 crate，保持框架中立 | 见 `scripts/check-deps.sh` |

**编译证据：** 见 `B/spike/desktop-contract/`（`cargo build`/`cargo test` 通过，见 ledger Task 02 条目）。探针证明：
1. 无业务模型窗口：iced 循环 + wry `build_as_child` + 主线程创建/销毁；
2. 两个不同 `ProductMessage` 类型（如 `DozerMsg`/`DiggerMsg`）实例化同一 `Product<M>`/`HostMessage<M>`；
3. Usage 与 CodeHealth 两种 payload 形状由同一 Surface 描述表达。

## 8. 对规格 §11 闸门 1 的回填结论

**闸门 1（iced×wry 窗口集成）：** 已由 `D/spike/editor-host`（真实窗口）+ `B/spike/desktop-contract`（双产品类型编译）解决。锁定版本 `iced 0.14` / `wry 0.55.1`；入口 `iced::application` 单循环 + `window::run` 建 wry 子视图；主线程所有权 thread_local；几何逻辑值 + DPI 由 wry 处理；webview 恒在 iced 之上由 desktop 层统一下推。**结论：成立，可开始 Task 04+ 的搬文件。**

## 9. Task 09 设计：CodeHealth 第二消费者与信任类别分离

> 计划 §Task 09 `:140-149`。本节是动手前的设计闸门，锁定三件事：CodeHealth 如何接入既有 Surface API、信任类别在**哪一层**建模、以及信任边界用什么测试钉住。

### 9.1 现状（设计输入）

Dozer 目前有 **六条** wry 构造路径，但真正的信任分界只有两类：

| 类别 | 构造入口 | `dozer://` 自定义协议 | IPC 形状 | 代表 kind |
|---|---|---|---|---|
| **应用（不可信）** | `runtime.rs::build_app_webview` | **无** | nonce 白名单（`focus`/`mouseup`/`zoom_*`），`AppIpc::parse` | `PanelKind::App` |
| **宿主/预览（可信）** | `runtime.rs::sync_webview_pool` 内联 builder | **有**（`assets::handle_protocol`） | 字符串 + envelope（`parse_*_event` + `HostBinding`） | editor / JSON / tabular / flyfish / html / image-annotate / plantuml / review-trace / git-log diff / usage-content / codehealth-content / todo-content / group-chat-content / browser |

- 预览池这条 builder 供 **14+ 种** kind 共用，仅靠 `spec.url`/`spec.editor_binding` 在 IPC fallback 里分支（`runtime.rs:502-518`、`:649-857`）。
- `WebviewSpec`（`preview/webview.rs:32`）已带 `surface: Option<SurfaceId>`（Task 08 加入），仅 Usage 槽为 `Some`。
- `bytehost-desktop` 只知道 core 的 `SurfaceKind{Builtin,Application}`，**且只用它算 `ready_policy`**；`panel::build_child` 是单一 HTML builder，不知道信任类别，也不接受 URL / 协议 / IPC 配置（`panel.rs:60-63`）。

**CodeHealth 与 Usage 同形**：都由 `sync_webview_pool` 通用 builder 建造，走 `dozer://…-content/host.html`，唯一差别是 CodeHealth 的 `WebviewPushState` 更"重"（`revision` 单调计数嵌进 envelope、`READY_TIMEOUT` 加载超时→`failed`、`waiting_since`）。这几个字段是 **CodeHealth 的业务/协议语义**（envelope 版本 + 面板级降级），**不是** Surface 生命周期语义。

### 9.2 决定 1：信任类别建模在 **Dozer 的 builder 分派**，不在 bytehost-desktop

规格 §5.4 `:124` "信任类别决定不同 builder；第三方应用 builder 不复用带编辑权限的 editor builder" 是**产品侧**约束：内容协议、IPC 白名单、CSP、nonce、data store 全是 Dozer 的产品语义，bytehost 只是机制层。因此：

- **不**在 `bytehost-desktop` 引入 `Editor/Preview/Application` 四值信任枚举，也**不**让 `panel::build_child` 长出 URL/协议/IPC 参数——那会把 Dozer 的产品策略塞进宿主库，违反 §5.4 `:126` "编辑器内容协议、Preview 渲染和业务数据不会因此移入 Host"。
- 信任类别落在 **Dozer 新建的 `preview::trust`（或 `webview_build`）模块**：一个 `WebviewTrust` 枚举 + 每类一个**构造配置**（builder 参数集），是唯一分派点。`runtime.rs::sync_webview_pool` 的 `None` 分支改为按 `spec.trust` 取配置，而不是继续堆 `if is_*_host`。目的：让"应用 builder 不带 `dozer://`、带 nonce-IPC"成为**类型/构造层**的保证，而非散落的字符串判断。

### 9.3 决定 2：信任类别取值与归属

`WebviewTrust`（Dozer 产品枚举，非 bytehost）：

| 值 | `dozer://` | IPC | devtools/download/new-window | 归属的 kind |
|---|---|---|---|---|
| `Application` | 否 | nonce 白名单 | 严格关闭 | `PanelKind::App` |
| `Editor` | 是 | envelope（`HostBinding`/`editor_binding`） | 现状 | editor / JSON / tabular / git-log diff |
| `Preview` | 是 | envelope（`HostBinding` 或固定槽 id） | 现状 | flyfish / html / image-annotate / plantuml / review-trace / browser |
| `BuiltinPanel` | 是 | 固定槽 id + `parse_*_event` | 现状 | usage-content / codehealth-content / todo-content / group-chat-content |

- `Application` 与其余三类的**真正差异**是"自定义协议 + 业务 IPC 是否可得"，这正是 §Task 09 clause 146 要测试的边界。
- `Editor`/`Preview`/`BuiltinPanel` 共享几何与生命周期（都走预览池 + `WebviewSpec`），差异只在内容协议（editor 绑定 vs rendered host 绑定 vs 固定槽），**分的是构造配置不是三种 builder 实现**——符合 §145 "公共几何与生命周期共享，产品内容协议分离"。
- `WebviewTrust` 可由 `spec` 的既有字段**派生**（app id 区间 → Application；固定槽 id → BuiltinPanel；`editor_binding.is_some()`→Editor；否则 Preview），避免新增易漏填的字段。派生函数 `WebviewTrust::of(&WebviewSpec)` 是唯一入口。

### 9.4 决定 3：CodeHealth 接入 Surface（对齐 Usage，业务字段留在面板）

`WebviewPushState` 的 `revision`/`READY_TIMEOUT`/`failed` 保留在 `extensions::codehealth`，**Surface 只接管**：逐帧 desired 推送、ready 归属校验、旧代 IPC 丢弃、几何联动。做法与 `UsageSurface` 同构：

- 新增 `extensions::codehealth::CodeHealthSurface`：持有 `SurfaceExecutor<CodeHealthViewPayload>`，固定身份 `PanelInstanceId(0,0)` + `SurfaceSlot(CODEHEALTH_CONTENT_ID_OFFSET)`。
- `revision` 语义：**保持现状（单调递增，不随 generation 重置）**，除非实现时验证到前端会跨实例保留计数。前端注释（`protocol.rs:515`）是"丢弃小于已应用值的推送"——它按 host 实例的 JS 上下文记账，新实例计数自然从零开始，故持续递增的 `revision` 对新 host 是"大于已应用值"，可安全接受。决定：`revision` 由 `CodeHealthSurface` 内持有、单调递增；**不**在 `observe_instance` 归零（改动会凭空引入一个未验证的前端契约依赖）。payload 构造仍在 `extensions::codehealth::current_view_payload`。
- `READY_TIMEOUT`/`failed`：**不进 Surface**。Surface 的 `SurfaceState::Failed{error}` 表达创建失败；CodeHealth 的"加载超时→落回原生占位"是**面板级**降级，继续由面板持有并在 `take_codehealth_content_script` 里早退（与 Usage 的 `loading()` 早退同型）。
- `WebviewSpec.surface` 对 CodeHealth 槽填 `Some(self.codehealth_surface.id())`；`CodeHealthContentWebviewEvent` 携带 `Option<SurfaceId>`（对齐 Usage Task 08 的 `UsageContentWebviewEvent(.., Option<SurfaceId>)`），Ready 先经 `accepts` 再 `apply_ready`。
- **禁止**新增 `if panel == codehealth`：第二消费者暴露的通用缺口修到 `SurfaceExecutor`（例如若 CodeHealth 需要"进池时业务钩子"，则给 executor 加 `observe_instance` 返回值/回调，而非在 Dozer 侧特判）。

### 9.5 决定 4：信任边界测试（clause 146）

三条钉死测试，全部 source-scan + 行为双管：

1. **应用拿不到自定义协议**：已有 `app_webview::build_app_webview_pins_the_restrictive_settings`（断言 `build_app_webview` 体内**无** `with_custom_protocol`）。Task 09 增补：断言信任分派点（新模块）中 `Application` 分支构造的配置**不含** `dozer` scheme，且 `sync_webview_pool` 只在非 `Application` 分支 `with_custom_protocol`。
2. **应用拿不到业务 IPC**：断言 `Application` 配置的 IPC 只接 `AppIpc::parse`（nonce 白名单），且 `runtime.rs` 的 envelope fallback 分支（`parse_usage_event`/`parse_codehealth_event`/`parse_event`…）在 `is_app_webview_id` 守卫之后，永不触达应用 webview（现有结构已满足，加断言钉住）。
3. **未知 Surface/消息拒绝**：Unit 测试 `SurfaceExecutor::route` 对未知 `SurfaceId` 返回 `UnknownSurface`、对旧代数返回 `StaleGeneration`（bytehost 侧已有）；Dozer 侧新增：`accepts(Option<SurfaceId>)` 对 `None`（老消息/无身份）与错代数返回 false 且不 `apply_ready`（Usage 已有测试，CodeHealth 复用同型）。

### 9.6 提交切片

按计划"一个可审阅提交组"：`refactor(codehealth): adopt shared surface lifecycle`（Dozer）。bytehost 若因 9.4 暴露的通用缺口需要改动，**单独一个** `feat(desktop): …` 提交，不与 Dozer 接入混在一起。若第二消费者**未**暴露通用缺口（很可能：Usage 已把 Surface 打磨到够用），则 bytehost 本轮零改动，实现记录里写明"无通用缺口，公共层未改"。

### 9.7 实施结果（2026-10-12）

- **bytehost 零改动（符合 9.6 预期）。** 第二消费方未暴露 `bytehost-desktop`/`bytehost-core` 的通用缺口——`SurfaceExecutor` 接口对 CodeHealth 的形状完全够用。本任务 bytehost 无提交。
- **信任建模落地 Dozer：** `preview::trust::WebviewTrust` + `WebviewTrust::of(&WebviewSpec)`（见 9.2/9.3）；`runtime.rs` 池分派改用 `is_application()`；可信宿主分支 `debug_assert!(trust.allows_custom_protocol())` 后装 `dozer://`（9.5-1 由类型派生层保证，`build_app_webview_pins_the_restrictive_settings` 交叉验证）。
- **CodeHealth 接入 Surface：** `extensions::codehealth::surface::CodeHealthSurface`；`WebviewPushState` 收敛为面板级降级（`waiting_since`/`failed`），ready/revision/去重/重建重发移入 Surface（9.4）。
- **测试：** 9.5 三条全部落地——(1) `WebviewTrust::app_id_derives_application` 断言 `allows_custom_protocol()==false` + 既有 `build_app_webview` 源码扫描；(2) 既有 nonce 白名单源码扫描（`AppIpc::parse(`）；(3) `CodeHealthSurface::stale_generation_ipc_is_rejected_after_rebuild`。
- **提交：** `9b7b5121`（Dozer）。
- **漂移守卫：** `usage_probe::host::host_site_citations_point_at_the_code_they_name` 因本任务行号位移同步修正 7 处引用后通过。

## 10. Task 10 设计：历史布局迁移的分工与范围

### 10.1 范围裁决

用户批准"兼容桥 + 黄金样本"：只抽取可移植的 **布局 DTO 映射**（Dozer 历史 `layout.json`/`rail_layout` → `bytehost-core::Layout`），含迁移前备份、未知更高版本拒绝、原子落盘。rail/pane 渲染与 focus/overlay **暂留 Dozer**，作为已记录、待逐条登记的兼容桥。理由：`chrome/rail.rs`（1444 行）与 `app/layout.rs`（1514 行）与 `PanelKind`/`App`/`ShellState` 深度耦合，整块抽取有像素行为风险；clause 160 明令不把"文件搬走"当作行为验证。

### 10.2 决定 1：分工——产品给映射，宿主给流程（计划 clause 157）

- **宿主（`bytehost-desktop::layout`）**：与产品无关的迁移机制——版本闸门、未知 token 保留、尺寸钳制、原子落盘 + 一次性备份。
- **产品（`dozer-app::app::layout_compat`）**：`PanelKind` ↔ 稳定 `PanelId` 映射与默认栏序/栏宽。
- **接缝 `LegacyPanelAdapter`**：`rails_from_legacy`（切产品历史 JSON 的栏 token）+ `map_token`（token → `PanelId`，`None` = 已下线）+ `default_rail_width`。宿主不解释任何产品 token。

### 10.3 决定 2：稳定 id 命名

- 内置面板 → `dozer.<slug>`，`slug` 为**小写 kebab**（`files`/`git-log`/`group-chat`/`code-health`…）。**不能用落盘 token**（PascalCase）：`PanelId` 只接受小写字母/数字/`-`/`_`，且每段以字母开头。
- 应用面板 → `app:<validated-id>`（`PanelId::for_app`），沿用现有 `AppSlot::id` 形状。
- 历史 token（`"Files"`、`"app:<id>"`）经 `token_to_panel_id` 翻译；`builtin_kind_by_token` 与 `state.rs::BUILTIN_NAMES` 同源，避免第二份真相。

### 10.4 决定 3：未知保留的两层语义

- 连 `PanelId` 形状都构不出（产品彻底不认识）→ 宿主 `MigrationOutcome.retained: Vec<String>`（原始 token，供提示）。
- 是合法 `PanelId` 但当前目录未注册 → `Layout::retained_unknown`（由 `reconcile_with_catalog` / 真实读盘 `Layout::load` 摘取）。
- 两者都不静默丢弃、不阻塞开窗。

### 10.5 决定 4：接线分期

薄包装先行（clause 160），**本提交不接入 app 启动/落盘路径**——真实接入需在真实窗口上做布局回归，单列后续提交。故模块级 `#![allow(dead_code)]` + 注释；黄金样本测试经 `#[cfg(test)]` 验证机制本身。

### 10.6 实施结果（2026-10-12）

- **bytehost（`f3edcef`）**：`bytehost-desktop/src/layout.rs`（`LegacyPanelAdapter`/`migrate_legacy_layout`/`migrate_legacy_file`/`atomic_save_layout`/`reconcile_with_catalog`）+ `tests/layout_compat.rs`（8 测试，假产品适配器）+ `lib.rs` 注册/再导出 + `serde_json` 依赖。
- **dozer（`a152a51d`）**：`app/layout_compat.rs`（映射 + 黄金样本 7）+ `app/state.rs::builtin_kind_by_token` + `app/mod.rs` 注册。
- **验证：** bytehost fmt/clippy(`-D warnings`)/全 workspace 测试/`check-deps.sh` 全绿；dozer fmt/clippy 干净、`cargo test -p dozer-app --bin dozer` 2025 通过 / 1 预存在失败、边界门禁仅预存在项。
- **未决：** 真实读盘路径接线、rail/pane/focus/overlay 兼容桥登记。

### 10.7 焦点/浮层/rail 抽取裁决（2026-10-12）

- **只抽零依赖机制。** 侦察后确认：建窗机制（`open_overlay`/`open_child_window`/`backdrop_card`）会把 `iced_wgpu`/`iced_widget`/`iced_renderer`/`winit` 与**渲染器类型**一起拉进 bytehost-desktop，与该 crate 刻意"不打开渲染器 feature、不改消费方 `iced_renderer` 类型"的约束直接冲突（`Cargo.toml:12-14`；这正是 R2 修复 `fbc3a636` 移除的 `iced_wgpu` 强绑）。且 `winit` 尚非其依赖，`winit::dpi::*`/`winit::Window` 类型会引入新依赖面。rail/pane 需要尚不存在的 `PanelId` 键控容器/拖拽/折叠 API；焦点路由整体以 `PanelKind` 为键。**故本轮只抽纯 `FocusTracker`**。
- **决定：** bytehost-desktop `focus.rs`（`FocusTracker`/`focus_transition`，与产品无关，仅记聚焦历史）；Dozer 删私有副本并改引宿主类型。建窗、rail/pane、焦点路由三项**登记为兼容桥 C10/C11/C12**（`acceptance.md` §2），待 Task 21 收口或后续设计出 `PanelId` 键控 API 后再抽。
- **实施结果（2026-10-12）：** bytehost `93b2ac3`（`focus.rs` + 注册/再导出）；dozer `239aa399`（删 `platform/overlay_focus.rs`，10 宿主改引 `bytehost_desktop::FocusTracker`）。验证见 ledger「Task 10 续」。

## 11. Task 12 设计：共享应用服务的管理能力（公共 API 兼容记录）

Task 03 `gates.md` 闸门 3 登记的两处 `AppService` 缺口 + 现场核实的第三项，落到 `bytehost-apps`。以下三组为新增/变更的公共 API，**均为追加式**（既有签名保持或改为兼容包装），wire 形状（`AppRequest`/`AppReply`）不动。

### 11.1 同根服务所有者互斥（新增 `service_lock`）

- **决定：** 用 `flock(2)` 而非 pid 文件。内核在持有者进程退出（含崩溃/kill）时自动释放 → 无需"陈旧锁"清理，也不会误判"还活着"；锁按打开文件描述归属，同进程另开 fd 再 `LOCK_EX|LOCK_NB` 同样失败 → 同进程双启动也挡住。
- **公共面：** `service_lock::{ServiceLock::acquire(root) -> Result<Self, LockError>, LockError::{AlreadyOwned, Io}}`，`#[cfg(all(feature="server", unix))]`。
- **契约：** `AppService::start` 保持"永不失败"——锁冲突与"端口被占/目录不可用"同路 → `State::Unavailable(reason)`，失败方**绝不**触碰第一方 gateway/进程。fd 显式 `O_CLOEXEC`（含 `fcntl` 双保险），spawn 的应用进程不继承锁。
- **`Drop` 语义：** 显式 `LOCK_UN` 再关 fd。原因：`flock` 锁隶打开文件描述，`fork`→`exec` 窗口内被并发 `Command::spawn` 的子进程短暂共享，单靠 close 偶发不释放（实测复现，非计划项，见 ledger）。

### 11.2 受控端口重置（新增，`AppService` 关联函数）

- **决定：** 重置是"服务停着时改一个既定文件"的管理动作，做成**与数据根绑定**的关联函数，不新增 wire 请求（重置的前提是服务已停，wire 请求本身打不到活着的服务）。
- **公共面：** `AppService::reset_port(root, requested) -> Result<u16, PortResetError>`、`clear_port(root) -> Result<(), PortResetError>`；`PortResetError::{ServiceRunning, OutOfRange, Io}`。
- **契约：** 三步——占有权校验（取同一把锁，服务在跑即 `ServiceRunning`）→ 原子持久化（复用 `port::persist_port`，临时文件 + rename）→ 返回新端口；写失败时旧 `gateway.json` 原样保留（可恢复）。**只改 `<root>/gateway.json`**，不暴露任意写内部文件。`clear_port` 用于"忘掉坏端口、下次重选"。规格式重试/重启流程（撤 Surface、说明 origin 影响、确认、重启）由 Task 13 app-host 编排。

### 11.3 可观察 shutdown（保留兼容入口）

- **决定：** 不改 `shutdown()` 的 `()` 签名（否则现有调用方 `apps.shutdown().await;` 会因 unused `Result` 在 `-D warnings` 下报错）；改为在其旁新增可观察入口。
- **公共面：** `shutdown_reporting() -> Result<ShutdownReport, ShutdownError>`、`shutdown_reporting_within(timeout)`；`ShutdownReport{app_failures: Vec<(AppId, String)>, problems: Vec<(String, String)>, gateway_released: bool}`（含 `is_clean()`）、`ShutdownError::{TaskFailed, TimedOut, GatewayNotReleased}`、常量 `AppService::SHUTDOWN_TIMEOUT`（30s）。
- **契约：** `shutdown()` 是 `shutdown_reporting()` 的日志包装；超时/收尾 panic 也**必释放所有者锁**（否则进程退出后数据根再起不来），遗留子进程交由下次启动的孤儿回收（按进程身份核对，不信可复用的 PID）兜底。测试以返回结构证明，不靠日志推断。

### 11.4 孤儿回收（已实现，只验证）

- `process/supervise.rs` `reap_orphan` 已按命令行 + `lstart` + 进程组组长三重比对；测试 `orphans_are_matched_by_start_time_and_never_by_a_bogus_or_non_leader_pid`（`:676`）覆盖起始时间不符 / 伪造 PID（0、1、`u32::MAX`）/ 非组长 PID，故障场景仅针对测试自建的 `sleep` 进程组。**零代码改动。**

### 11.5 提交切片

`ccb32a8` 互斥 → `b332684` 管理接口 → `7daa064` 退出观测 → `3ab1c34` 锁释放修正(B)。

## 12. Task 13 设计：Digger app-host 适配层（泛型边界与身份）

### 12.1 决定 1：`AppHostApi` 非 dyn-safe → 全程泛型，不用 `dyn`

`bytehost_client::AppHostApi` 用 RPITIT 返回 `impl Future`，**不是 dyn-compatible**，不能 `Arc<dyn AppHostApi>`。因此 `digger-app-host` 的 `Executor<A: AppHostApi>` / `HostService<A>` 全程泛型：生产 `A = InProcess`（Digger 进程内嵌入的真实服务），测试 `A = FakeHost`。后端句柄放 `Arc<tokio::sync::RwLock<A>>` 之后，"启动占位 → 真正服务"经 `*guard = new_value` 就地替换而不改类型；`InProcess` 非 `Clone`，替换时用其内部 `Arc<AppService>` 重建。

### 12.2 决定 2：`AppKeyId`（`Copy` 的稳定面板键）

面板状态机要求 `K: AppKey + Copy`；`bytehost_apps::id::AppId` 持 `String`、非 `Copy`。`digger-app-host` 定义 `AppKeyId(u32)` + 进程级驻留表（`Box::leak` id 串），实现 `bytehost_panel::key::AppKey`，并带 `into_app_id()`。**注意 `slot.app_id()` 解析到 trait 方法返回 `&str`；需 owned `AppId` 时用 `slot.into_app_id()`。**

### 12.3 决定 3：身份台账在 Digger 侧，不在 bytehost

`RequestTable`（`ScopeId`/`PanelInstanceId`/`generation`/`sequence`）落在 `digger-app-host`，与 Dozer 的 `bytehost_core::request::run_scoped`（R3）同构但独立——scope/panel 身份是产品编排概念。**卸载 `invalidate_app` 只作废该应用的 UI 结果；写操作（卸载本身）身份不受影响，仍由服务完成。面板关闭 `invalidate_panel` 作废该实例全部结果。**

### 12.4 决定 4：执行器只发"已批准"请求，审批门留给组合根

`Executor::run` / `run_install` 只把 `bytehost-panel` 状态机吐出的 effect 翻成对 `AppHostApi` 的异步调用；`Install{approved,source}` 里的 `approved` 由安装审批状态机（组合根，Task 14）把守。执行器不碰状态机、不替产品做信任决策。

### 12.5 提交切片

`2305775`(D) 单提交：crate 骨架 + fake + 泛型执行器 + 身份台账 + 订阅映射 + 生命周期 + 单测/集成测试。

### 12.6 实施结果（2026-10-12）

- `cargo test -p digger-app-host`：21 单测 + 12 集成全绿（含真实临时数据根嵌入 `InProcess`）。
- `cargo test --workspace` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` / `scripts/check-host-deps.sh` 全绿。
- 待 Task 14 接入 `digger-desktop` UI（本 crate 尚未被 `desktop` 依赖）。

## 13. Task 14 设计：产品信任策略与安装审批 UI（产品裁决只在适配层）

### 13.1 决定 1：产品信任裁决只在 Rust 适配层（`trust.rs`）

`trust::policy_for(SourceChoice) -> SourcePolicy{provenance,trust}` 给出**本机来源** `Local/Trusted`、**URL** `ThirdParty/Untrusted`；组合根在派发 `Effect::Plan` 时把它注入 `run_install_with_policy(.., Option<SourcePolicy>)`。返回的计划再经 `trust::review(&InstallPlan) -> Review{Allowed,Rejected}` 复核：不受信来源 + 进程型 runtime(`node`/`python`) 一律拒绝，只允许 `static_web`。**GUI 不自己算信任级别**——服务端对 `AppSource::Url` 无论调用方传什么都会重解析为 `ThirdParty/Untrusted`（本地来源保留调用方值），故 `trust.rs` 提供的是本机默认。这条把"产品信任决策"固定在适配层，与规格 §6.3 clause `:161`（本轮允许用户明确批准的本地可信 Python/Node 应用；Agent/URL 按 Untrusted 仅 static_web）一致。

### 13.2 决定 2：`Effect::Plan` 的信任级别需经旁路注入

共享状态机的 `Effect::Plan(AppSource)` **只带来源、不带信任级别**（信任级别是产品语义，不属于宿主通用状态机）。因此执行器新增 `run_install_with_policy(scope,panel,generation,effect,Option<SourcePolicy>)`，`run_install` 委托并传 `None`（默认 `LOCAL_TRUSTED`）。组合根在 `dispatch` 里按用户当前来源算出策略传入。

### 13.3 决定 3：审批身份 == 展示计划；签名与防重在状态机

批准走状态机自身逻辑：`ApproveClicked` 只在 `Reviewing` 生效，就地 `plan.clone().approve(Approval{approver: "dozer-gui", ..})` 后进 `Installing`，**不重算、不改写**；重复点击此时不在 `Reviewing`，不再吐 `Install` effect。来源切换会清掉 `Reviewing|Failed`（审批卡立刻作废），迟到计划结果按当前 flow 丢弃。**已知共享状态机在 `Planning` 窗口内不清理来源切换**，但 Digger 的来源单选行**只在 `Flow::Idle` 渲染**（`Planning` 期间被"正在检查应用…"取代），该窗口在 UI 里不可达，身份端到端成立——不属 Task 14 的适配层缺陷，不在本层额外兜底。

### 13.4 决定 4：修复 `install_service` 的 `try_write().expect` panic 风险（Task 13 遗留）

`HostService::install_service` 原用 `try_write().expect("no concurrent readers")` 换后端——有并发读即 panic。改为：**genesis 任务在 tokio 工作线程用 `api.write().await` 换后端**（窗口线程不能阻塞等锁），再回 `ServiceReady`；`install_service` 只记录 `Arc<AppService>` 供 shutdown，并在锁空闲时做等价的就地后备写入（`try_write` 失败即跳过，后端已是真服务）。新增测试钉住时序："`ServiceReady` 到达时共享后端已是真服务"。

### 13.5 决定 5：`Outcome` 回流用 `Task::stream`；应用服务延迟到首次打开

`AppsState::new()` **不在构造期** `start_service`（会脱离 tokio 运行时 panic），改在首次 `Message::Opened`（永远在 iced 运行时内）触发。回流通道的接收端唯一、不该反复重建，用 `iced::Task::stream` 在 `boot` 挂一次（而非 `Subscription`），并 `Task::batch` 进 boot task。`Message` 需 `Clone + Debug`，故 `Outcome` 以 `Arc<Outcome>` 携带（自带手写 `Debug`）；`ServiceReady`/`ServiceFailed` 不经 `Message` 中转，直接在 `deliver` 内按 `Arc` 借用处理。原生选目录/压缩包用 `rfd::AsyncFileDialog` + `Task::perform`（iced 0.14 无 winit 主线程钩子）。

### 13.6 提交切片

`a21b730`(D) 单提交：`trust.rs` + `run_install_with_policy`/`InstallAction` + `service.rs` 修复 + `testing.rs` 便捷构造 + `install_flow.rs`/`embedded.rs` + `desktop/src/apps/*` UI + `product.rs`/`main.rs` 接线。

### 13.7 实施结果（2026-10-12）

- `cargo test -p digger-app-host`：28 单测（含 `trust` 5 + `InstallAction` 2）+ 14 `embedded` + 6 `install_flow` 全绿。
- `cargo test -p digger-desktop`：9 单测全绿；`cargo test --workspace` 全绿。
- `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` / `scripts/check-host-deps.sh` 全绿。
- **真实窗口交互验收与 Task 08 人工项合并**（受 OS 权限限制）；运行时/日志/回滚/卸载 UI 为 Task 15 占位。

