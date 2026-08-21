# 共享 webview 几何代码抽取设计

**状态:已批准(brainstorming 会话,2026-08-21)**

## 背景

`app.rs` 拆分系列第三个试点,继 Rail(`rail.rs`)、Terminal(`terminal.rs`)
之后。原计划沿用 Rail→Terminal→Preview 的既定顺序做"Preview(文件预览/
编辑器)"试点,但摸底发现这个名字底下其实是两块性质完全不同的代码:

1. **真正的 Preview 业务域**(文件预览 tab、编辑器状态、`preview.rs`/
   `preview_state.rs` 已经拆得比较干净)——`preview_open_path`/
   `preview_select_tab`/`preview_edit_event`/`preview_desired` 等方法,
   主体是可见性闸门 + 项目路由 + 读写 `Workspace`,属于内核编排,同
   Rail 的 `panel_select`/Terminal 的 `term_input` 一个道理,这轮**不动**。
2. **共享 webview 几何计算**——`preview_content_bounds_for`/
   `left_files_tree_bounds_for`/`is_in_preview_column` 三个纯函数(合计
   360 行,外加 15 个专门测试它们的单测,约 330 行),被 `Files`/
   `Project`/`Web` 三个挂了原生 wry webview 的面板共用,跟"文件预览"这个
   业务域本身无关,是被历史命名("预览面板最早只有文件预览一种 webview
   用途")污染的通用几何代码。三者的文档注释里明确写着"三个函数(webview
   矩形、文件树命中、焦点路由)都靠这一份[配对列宽公式]算"。

用户 2026-08-21 讨论时选择这轮先只做第 2 块(第 1 块留到以后单独一轮,
如果还值得做的话)。

摸底还发现这三个函数依赖一批**极度共享**的纯几何辅助(`pair_content_width`
18 处调用、`pair_columns` 17 处、`pair_x0_and_width` 13 处、
`left_zone_width` 12 处、`right_zone_width` 11 处、`maximized_box_x_range`
5 处、`maximized_box_height` 4 处)——这些辅助服务的范围远不止这三个目标
函数(`view()` 里大量非 webview 面板的渲染、Rail 图标定位等都在用),不能
跟着搬走,只能放宽可见性供新模块跨模块调用。这个交叉依赖面比 Rail(0 个
此类辅助)、Terminal(2 个:`tab_bar`/`active_tab_view`)都宽,是这次从
Bounded 升级成 Architectural(走完整 spec+plan)的原因。

## 目标 / 非目标

**目标**:

1. 新建 `crates/dozer-app/src/webview_geometry.rs`(纯函数模块,不进
   `extensions/` 目录,同 `rail.rs`/`terminal.rs` 量级)。
2. 三个函数原样搬入(签名、函数体逻辑完全不变):
   `preview_content_bounds_for`、`left_files_tree_bounds_for`、
   `is_in_preview_column`。
3. 七个共享辅助**留在 `app.rs`**,可见性从私有升级 `pub(crate)`(纯
   visibility 变更,函数体不动):
   - `pair_content_width`(私有 `fn` → `pub(crate) fn`)
   - `pair_x0_and_width`(私有 `fn` → `pub(crate) fn`)
   - `maximized_box_x_range`(私有 `fn` → `pub(crate) fn`)
   - `maximized_box_height`(私有 `fn` → `pub(crate) fn`)
   - `pair_columns`(私有 `fn` → `pub(crate) fn`)
   - `PairColumns` 结构体本身 + 其 4 个字段(`list_x`/`list_w`/
     `content_x`/`content_w`)全私有 → 结构体与字段一并升级
     `pub(crate)`(`pair_columns` 返回它,新模块要读它的字段,只升级
     函数、不升级字段类型和字段本身会编译不过)。
   - `left_zone_width`/`right_zone_width` 已经是 `pub(crate) fn`,不用
     动。
   - `pair_list_content_width`(`pair_columns` 内部私有调用,三个目标
     函数不直接调它)**不升级可见性**,继续全私有。
4. 三个目标函数的现有调用点加 `webview_geometry::` 前缀:
   - `app.rs`:`ime_cursor_area` 内 2 处 `preview_content_bounds_for`
     调用、`preview_desired` 内 2 处、`files_drop_target` 内 1 处
     `left_files_tree_bounds_for` 调用。
   - `main.rs`:1 处 `app::is_in_preview_column` 调用(焦点路由,原生
     事件层)。
5. 15 个既有测试(全部在 `app.rs` 底部单个 `#[cfg(test)] mod tests`
   块内,`use super::*;`)整体搬进 `webview_geometry.rs` 自己的
   `#[cfg(test)] mod tests`,断言不变:
   `preview_content_bounds_is_inside_left_content_column`/
   `preview_content_bounds_web_view_spans_whole_left_zone`/
   `preview_content_bounds_web_view_shrinks_when_bookmarks_open`/
   `preview_content_bounds_web_view_mirrored_bookmarks_content_follows_render_order`/
   `preview_content_bounds_zero_when_left_collapsed`/
   `preview_content_bounds_zero_when_right_maximized`/
   `preview_content_bounds_left_maximized_files_matches_overlay_geometry`/
   `preview_content_bounds_left_maximized_web_spans_whole_overlay_box`/
   `preview_column_hit_test_respects_maximized_state`/
   `preview_content_bounds_bare_for_right_panel_on_left`/
   `is_in_preview_column_false_for_right_panel_on_left`/
   `preview_content_bounds_never_negative`/`preview_column_hit_test`/
   `preview_column_hit_test_left_collapsed_is_never_hit`/
   `is_in_preview_column_returns_project_when_project_on_right`。
   `left_files_tree_bounds_for` 没有专门的单测,不涉及测试搬迁。

**非目标**:

- 不动真正的 Preview 业务域(`preview_open_path`/`preview_select_tab`/
  `preview_edit_event`/`preview_tab_editor_event`/`preview_desired`/
  `preview_tab_context_menu_popup` 等)——它们要么是内核编排,要么是这轮
  没打算处理的 view 代码,留到"以后要不要单独立项做 Preview 业务域"再说,
  这次不夹带。
- 不搬 7 个共享几何辅助本身(`pair_content_width` 等)——它们服务范围
  远超这三个目标函数(`view()` 里大量非 webview 渲染代码也在用),搬了
  会变成"抽取整个配对布局数学内核",跟这次目标("挪走这三个大函数,不动
  它们的公共依赖")不是一回事,且会牵动 ~80 处不相关调用点,超出这次
  scope。
- 不改变任何函数的输入输出行为——纯代码搬迁 + 可见性放宽,不允许顺手
  修任何几何公式(哪怕看起来像 bug)。
- 不建 `Extension` trait/运行时注册表,不拆独立 crate(延续 Rail/
  Terminal 定下的方向)。

## 关键语义确认(brainstorming 会话定案)

- **`left_files_tree_bounds_for` 之前被漏数了一次**——最初摸底只提到
  `preview_content_bounds_for`/`is_in_preview_column` 两个函数,后来在
  `preview_content_bounds_for` 紧邻的下一个函数处发现了它,而且三者的
  文档注释本来就是绑在一起写的("三个函数...都靠这一份[公式]算")。写
  实现计划时以这份 spec 的清单(三个函数)为准,不要只按"Preview" 字面
  搜索。
- **`PairColumns` 的字段可见性容易漏**——`pair_columns()` 返回
  `PairColumns` 值,三个目标函数搬到新模块后要跨模块读它的
  `.list_x`/`.list_w`/`.content_x`/`.content_w` 四个字段。只放宽结构体
  本身的可见性(`pub(crate) struct PairColumns`)不够,字段各自也要标
  `pub(crate)`,否则编译报 private field 错误。这是本次审计特意点出来
  防止实现时漏掉的一处。
- **`pair_list_content_width` 不用动**——虽然名字看起来跟 `pair_columns`
  系出同源,但它只被 `pair_columns` 内部调用,三个目标函数不直接引用它,
  继续保持全私有。
- **`webview_geometry` 这个模块名不完全精确但已经是最贴切的**——
  `left_files_tree_bounds_for` 算的其实是文件树列表列(非 webview 本身)
  的矩形,不是严格意义上的"webview"几何,但它和另外两个函数是同一份
  配对列宽公式的三个消费方、文档注释里绑在一起讲,拆开放没有实际收益,
  仍归进 `webview_geometry.rs`。

## 架构与数据流

### 1. `webview_geometry.rs` 模块内容

```rust
// 三个目标函数(签名与现有 app.rs 内定义完全一致,原样搬迁,函数体不变)
pub fn preview_content_bounds_for(
    side: Side,
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32, f32, f32)

pub fn left_files_tree_bounds_for(
    side: Side,
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32, f32, f32)

pub fn is_in_preview_column(x: f32, window_width: f32, state: &ShellState) -> Option<PanelKind>
```

三者都是 `pub fn`(现状已经是 `pub`,搬迁不降级)——`is_in_preview_column`
被 `main.rs` 跨 crate 边界内的顶层调用,`preview_content_bounds_for` 被
`app.rs` 多处 `impl App` 方法调用,维持 `pub` 而非收紧成 `pub(crate)`,
避免不必要的额外收紧超出这次改动范围。

依赖 `crate::app::{PanelKind, Side, ShellState, MaximizedPane,
PairColumns, pair_content_width, pair_x0_and_width, maximized_box_x_range,
maximized_box_height, pair_columns}` 与 `crate::theme`——沿用 Rail/
Terminal 已确立的"新模块 `use crate::app::{...}` 直接导入裸名"的写法,
不做 `app::` 逐处限定。

### 2. `app.rs` 侧改动

七个共享辅助只改可见性,函数体/结构体字段类型逐字不变:

```rust
pub(crate) struct PairColumns {
    pub(crate) list_x: f32,
    pub(crate) list_w: f32,
    pub(crate) content_x: f32,
    pub(crate) content_w: f32,
}

pub(crate) fn pair_content_width(zone_width: f32) -> f32 { .. }
pub(crate) fn pair_columns(pair_w: f32, split: f32, mirrored: bool) -> PairColumns { .. }
pub(crate) fn pair_x0_and_width(side: Side, window_width: f32, state: &ShellState) -> (f32, f32) { .. }
pub(crate) fn maximized_box_x_range(window_width: f32) -> (f32, f32) { .. }
pub(crate) fn maximized_box_height(window_height: f32) -> f32 { .. }
```

五处真实调用点(均在 `impl App` 内,方法本身不搬)加前缀:

```rust
// ime_cursor_area(2 处)
let (bx, by, _bw, _bh) = webview_geometry::preview_content_bounds_for(side, window_w, window_h, &state);

// preview_desired(2 处)
let bounds = webview_geometry::preview_content_bounds_for(side, window_width, window_height, &self.shell_state());

// files_drop_target(1 处)
let bounds = webview_geometry::left_files_tree_bounds_for(side, window_w, window_h, &self.shell_state());
```

`app.rs` 顶部 `use` 区块加一行 `use crate::webview_geometry;`(按字母序,
`use crate::transcript::ReviewEntry;` 与 `use crate::workspace::{..}` 之间)。

### 3. `main.rs` 侧改动

`main.rs` 现状没有为 `app` 模块开 `use`,直接以 `app::is_in_preview_column(..)`
路径调用(`main.rs` 是 crate 根,`app`/`webview_geometry` 都是它
`mod` 出来的直接子模块,可以直接用模块名当路径前缀,不需要 `use`)。
延续这个写法,不新增 `use` 语句,只把调用点的模块前缀从 `app::` 换成
`webview_geometry::`:

```rust
let intent = match webview_geometry::is_in_preview_column(logical_x, logical_w, &state) {
```

### 4. 模块注册

`main.rs` 的 `mod` 列表按字母序插入 `mod webview_geometry;`(`mod
transcript;` 与 `mod workspace;` 之间)。

## 错误处理

不涉及。三个函数都是确定性纯几何计算,不读写文件、不发起网络/IPC 请求,
不存在失败路径,搬迁前后行为不变。

## 测试策略

- 15 个既有测试原样搬进 `webview_geometry.rs` 的 `#[cfg(test)] mod
  tests`(`use super::*;`),断言与测试数据不变。
- 不新增测试——纯代码搬迁 + 可见性放宽,行为契约由"编译期防回归 + 现有
  测试全绿"验证。
- 人工验收:切换 Files/Project/Web 三个挂 webview 的面板,确认 webview
  位置/尺寸不变(含镜像态、放大态、浏览器收藏夹展开/收起、文件树列表
  拖拽命中);验证浏览器地址栏与验收评论框的 IME 候选窗位置不变(间接
  依赖 `preview_content_bounds_for`,见 `ime_cursor_area`)；验证外部
  文件拖入文件树目录行仍能命中(`files_drop_target`)。
- 编译期防回归:`cargo build && cargo test -p dozer-app && cargo clippy
  -p dozer-app --all-targets && cargo fmt --check` 全绿(注意这次
  `main.rs` 也改动,建议跑全 workspace `cargo build`/`cargo clippy`
  而不只是 `-p dozer-app`)。

## 依赖变更

无新增依赖。
