# C10 设计注记：原生浮层建窗/渲染机制上收 bytehost-desktop 的接缝选择

**状态：** 已实现（方案 B；C10 的"完全归零"续做留待是否再做方案 A/C 的裁决）。对应验收 `acceptance.md:48` 的 **C10** 行、任务列 `21`。

## 1. 问题

Dozer 有一批**独立原生子窗口**（挂在主窗口之上、盖过主窗口内所有 `iced` 内容与 wry 子视图）：

- 8 个模态卡片宿主（`ConfirmOverlay` 及 7 个定制宿主：`search_overlay.rs`、`file_history_overlay.rs`、
  `settings`、`project_create` 等），全部经 `platform/overlay_window.rs` + `platform/overlay_gpu.rs`；
- Toast（`platform/toast_overlay.rs`），复用同一对底层。

C10 的目标：**把"建窗 + GPU 管线 + 遮罩"这三块机制抽进 `bytehost-desktop`**，产品侧接缝只留
"内容（`Element`）与主题色"，或让产品侧浮层**全量改用共享建窗 API**；届时新增门禁
**`R-OVERLAY-DUP` 归零**。

## 2. 障碍（本设计的核心约束，不可绕过）

`bytehost-desktop` 刻意**不绑渲染器 feature**，也**不依赖 winit**：

- `bytehost/crates/bytehost-desktop/Cargo.toml:19`：`iced = { version = "0.14", default-features = false,
  features = ["tokio"] }` —— 只消费 iced 核心契约（`Element`/`Task`/`window`/`widget`/`futures`），
  **不打开任何渲染器后端**，否则会把 tiny-skia fallback renderer 强制并进消费方（如 `dozer-app`）
  的 iced 依赖树，改变其 `iced_renderer::Renderer` 类型（R2 修复 `fbc3a636`，见 C10 行）。
- winit 目前**不是** `bytehost-desktop` 的依赖；`overlay_window.rs`/`overlay_gpu.rs` 直接大量使用
  `winit::window::Window`/`ActiveEventLoop`/`dpi::*`、`iced_wgpu::{Engine,Renderer,wgpu}`、
  `iced_winit::{conversion,Clipboard,core::*}`、`iced_widget`/`iced_renderer` 的具体类型。

因此"把机制整个搬进 desktop"必然要么给 desktop 加 `iced_wgpu`+`iced_winit`+`winit`（改变其依赖性质），
要么只搬走**不碰渲染器/窗口句柄类型**的那一半。

## 3. 三个文件的机制构成（拆分依据）

| 文件 | 行数 | 依赖渲染器/winit 句柄？ | 可搬性 |
|------|-----|----------------------|--------|
| `overlay_window.rs` | 331 | 部分：纯几何/常量纯函数无依赖；`open_child_window`/`build_child_window`/`open_overlay`/`reposition_overlay`/`enable_overlay_mouse_moved_events` 强依赖 `winit::Window`、`ActiveEventLoop`、`objc2-app-kit` | **拆两半**：几何/常量可无依赖搬走；建窗胶水依赖 winit+objc2 |
| `overlay_gpu.rs` | 287 | 全部：`iced_wgpu::{Engine,Renderer,wgpu}`、`iced_winit`、`winit::Window`、surface 配置、遮罩清屏 | 只有整体搬（需给 desktop 加渲染器依赖） |
| `toast_overlay.rs` | 275 | 部分：`stack_bounds`/`view` 是纯函数（`view` 依赖 `iced_widget`/`iced_renderer`/`byteui` 主题）；`ToastOverlay` 结构体依赖 `winit::Window` + `OverlayGpu` | 纯函数可搬；结构体壳跟着 GPU 走 |

关键观察：**`backdrop_card`/`backdrop_card_shrink`/`centered_card_offset`/`full_window_overlay_bounds`/
`window_logical_size`/`popup_card_size`/`stack_bounds` 之外的建窗与 GPU 胶水，其"产品无关性"是成立的**
（它们不含任何 Dozer 业务，唯一的产品注入点是：窗口 tag 字符串、遮罩色 `scrim`、卡片内容的 `Element`、
以及 `iced_widget::Theme`/`iced_renderer::Renderer` 这两个**类型**）。

## 4. 方案对比

### 方案 A — `overlay` feature-gated 模块（整搬）

给 `bytehost-desktop` 加可选 feature `overlay`，启用时引入 `iced_wgpu`/`iced_winit`/`winit`/`iced_widget`/
`iced_renderer`，把 `overlay_window.rs`+`overlay_gpu.rs` 整体搬入（`OverlayGpu`、`open_child_window*`、
`open_overlay`、`reposition_overlay`、`enable_overlay_mouse_moved_events`、`backdrop_card*`）。

- **默认 feature 关**：不开 `overlay` 的消费方（digger、测试）依赖树**不变**，R2 修复不受影响。
- 开启 `overlay` 的消费方（`dozer-app`）本来就已依赖 `iced_wgpu`/`iced_winit`/`winit`，不会引入新 crate。
- 产品接缝收窄为：传 `tag`、`scrim` 色、内容 `Element`；`ToastOverlay`/`OverlayGpu` 变成宿主类型，
  产品只在其上做业务（Toast 列表、模态内容）。
- **风险**：`overlay` 模块的类型签名要泛型化到"内容 `Element<'a, Message, iced_widget::Theme,
  iced_renderer::Renderer>`"——这正是 desktop 现在**避免**绑定的类型。开了 feature 才暴露，属可控。
  但 feature 打开后，desktop 编译出的 `iced_renderer::Renderer` 必须与消费方**同一份** iced
  （同一 feature 集），否则类型不匹配——这要求 desktop 与 dozer-app 的 iced 版本/feature 严格同源
  （现状：都用 git tag 锁同一 iced 0.14，可满足）。

### 方案 B — 只搬"无渲染器依赖"的一半（推荐先做）

搬进 `bytehost-desktop`（**新增 winit + objc2-app-kit，不加 iced 渲染器 feature**）：

1. 纯几何/常量：`full_window_overlay_bounds`、`centered_card_offset`、`window_logical_size`、
   `popup_card_size`、`POPUP_WIDTH_FRACTION`、`BACKDROP_CORNER_RADIUS`、`stack_bounds`（从 toast 抽出）。
2. 建窗胶水：`open_child_window`/`open_child_window_unfocused`/`build_child_window`、
   `enable_overlay_mouse_moved_events`、`open_overlay`/`reposition_overlay` —— 这些**只碰 `winit::Window`/
   `ActiveEventLoop`/`PhysicalPosition`/`objc2`，不碰 `iced_wgpu`/`iced_renderer`**。
   `open_overlay` 里的 `OverlayGpu::open` 调用抽成**回调/泛型参数**（宿主不认识 GPU 类型）。
3. **`OverlayGpu`（wgpu 管线 + `UserInterface` redraw/dispatch）留在产品侧**，或后续再评估。
4. `backdrop_card*`（依赖 `iced_widget::Theme`/`iced_renderer::Renderer`/`iced_widget::container`）
   暂留产品侧，或作为"依赖 iced 核心 widget（不含渲染后端）"的小块——需确认 `iced_widget` 是否触发
   renderer feature（现状 desktop 已依赖 `iced` 的 `widget` feature，故 `iced_widget` 类型**已在**
   desktop 依赖树内，`backdrop_card*` 可搬）。

- **优点**：不动 desktop 的 iced 渲染器 feature 决策（R2 红线）；把"建窗/定位/鼠标穿透/圆角遮罩几何/
  Toast 堆叠定位"这些**真正重复、且产品无关**的部分收走；产品侧只剩"内容 `Element` + GPU 管线 + 主题色"。
- **缺点**：`R-OVERLAY-DUP` 不能完全归零——`OverlayGpu` 与 wgpu surface 配置仍在产品侧。若 C10 验收
  严格要求"归零"，方案 B 不足。

### 方案 C — 产品侧全量改用共享建窗 API（折中：B + 把 GPU 也搬但 feature-gated）

等价于 A，但保留 B 的"几何/建窗先搬"作为第一步。分两刀落地，降低单次改动风险。

## 5. 推荐

**方案 B 先行，A/C 作为是否"完全归零 `R-OVERLAY-DUP`"的后续裁决。**

理由：

1. R2 修复（`fbc3a636`）明确禁止 desktop 默认绑渲染器 feature——方案 A/C 用 `overlay` feature 可规避，
   但把"打开后必须与消费方 iced 类型同源"变成硬约束，风险集中在类型兼容；方案 B 完全回避。
2. 占重复代码大头、且产品无关的是**建窗 + 定位 + 几何**（`overlay_window.rs` 的绝大部分 + `stack_bounds`），
   方案 B 正好收走，收益高、风险低。
3. `OverlayGpu`（287 行）与 `backdrop_card*` 的产品无关性虽成立，但它们绑 `iced_wgpu`/`iced_renderer`，
   上收需要 A 的类型同源保证——值得单独立项、单独回归，不塞进本刀。

### `R-OVERLAY-DUP` 门禁的形状（无论 A/B 都先立）

在 `check_panel_boundary.py` 新增规则，扫产品侧 `platform/*overlay*.rs` 里**已搬走机制的再实现**：
`with_parent_window`/`with_decorations(false)`/`with_transparent(true)`/`set_cursor_hittest(false)`/
`ns_window.setAcceptsMouseMovedEvents`/`create_surface`/`CompositeAlphaMode::PostMultiplied` 等
"建窗/透明/穿透/alpha 模式"特征串的出现次数。方案 B 落地后基线为**剩余数**（`OverlayGpu` 的
`create_surface`/`PostMultiplied` 等仍在产品侧），方案 A/C 落地后为 **0**。

## 6. 落地步骤（若采纳方案 B）

1. `bytehost-desktop`：新增 `winit`（与 dozer-app 同版本）+ `objc2-app-kit`/`raw-window-handle` 依赖；
   新建 `src/overlay.rs`，搬入第 4 节 B1/B2 的函数（签名用 `winit` 类型，`OverlayGpu::open` 抽成泛型/
   回调参数）。已有 3 个纯几何测试（`full_window_overlay_bounds_matches_main_window_exactly` 等）随迁。
2. `dozer-app`：`overlay_window.rs` 保留为**薄壳**——重导出宿主的建窗函数，`OverlayGpu::open` 仍产品侧
   实现（作为传给宿主 `open_overlay` 的工厂闭包）；`toast_overlay.rs` 的 `stack_bounds` 改调宿主版。
3. 加 `R-OVERLAY-DUP` 门禁 + 基线（剩余 = 方案 B 未搬走的部分）。
4. 回归：`cargo test -p bytehost-desktop`、`cargo test -p dozer-app --bin dozer`、边界门禁、clippy。
5. 若后续裁决"必须归零"，再按方案 A/C 加 `overlay` feature 搬 `OverlayGpu`。

## 7. 未决项

### 7.1 落地记录(方案 B,已实现)

- **bytehost `147285f`:** `bytehost-desktop/src/overlay.rs` 新增(Cargo.toml 加 `winit`/`raw-window-handle`/`objc2-app-kit`,**不**加渲染器 feature);`full_window_overlay_bounds`/`centered_card_offset`/`window_logical_size`/`popup_card_size`/`POPUP_WIDTH_FRACTION`/`BACKDROP_CORNER_RADIUS`/`open_child_window`/`open_child_window_unfocused`/`enable_overlay_mouse_moved_events`/`open_overlay<G>`/`reposition_overlay`/`stack_bounds`;7 个单测(3 几何 + 4 Toast 定位)。
- **dozer `ea1202a0`:** `overlay_window.rs` 缩为薄壳(重导出宿主建窗/几何 + 保留 `backdrop_card*` + 两个注入 `OverlayGpu` 的包装);`toast_overlay.rs` 的 `stack_bounds` 改调宿主。新增 `R-OVERLAY-DUP` 门禁,基线 `platform/overlay_gpu.rs R-OVERLAY-DUP 3`(create_surface + 2× PostMultiplied,方案 B 有意保留产品侧)。
- **验证:** bytehost `cargo test -p bytehost-desktop` 57+8+3+8 全绿、`check-deps.sh` ok;dozer build/`--tests` 绿、clippy 对两文件零告警、`cargo test -p dozer-app --bin dozer` 1968 passed / 1 failed(既有 WIP 漂移)、`check_panel_boundary.py` ok(216/6)、`check-log-scope.sh` ok。

### 7.2 仍未决

- **是否要求 `R-OVERLAY-DUP` 完全归零**（决定停 B 还是续 A/C 把 `OverlayGpu` 也上收——届时基线 3→0）。
- `OverlayGpu::open` 的抽法已按泛型工厂落地(`open_overlay<G>`);`backdrop_card*`(依赖 `iced_widget`)按方案 B 留产品侧。
- Toast 的"点击穿透 + 不聚焦"用独立的 `open_child_window_unfocused`(形状与原来一致)。
