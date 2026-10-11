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
