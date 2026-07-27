# Dozer 预览/终端 chrome 清理 + tab 栏打磨实现计划（P1L 迭代 A）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 清理预览/终端两栏顶部开发期残留、tab 栏经得起多 tab 与误操作、去掉 Flyfish 自带工具栏（dogfood 反馈 #4/#5/#6/#7）。

**Architecture:** 全部改动集中在 `crates/dozer-app/src/workspace.rs`（视图层）+ `crates/dozer-app/assets/flyfish/host.html`（一行）。4 个独立可交付 task：#7 Flyfish 工具栏、#4 去头部标题+几何常量、#6 关闭×进 pill、#5 tab 横向滚动。view() 层改动 headless 只能编译验证，真机视觉留用户验收；唯一可单测项（#4 几何常量）加回归断言锁定。

**Tech Stack:** Rust, iced 0.14（`iced_widget` 的 `container`/`button`/`text`/`row!`/`column!`/`scrollable`/`space`，`scrollable` 是核心 widget 无需 feature）；vendored Flyfish 查看器（`host.html` 里的 `<flyfish-file-viewer>` web component）。

## Global Constraints

- 颜色**只取自 `crates/dozer-app/src/theme.rs`**（BG/PANEL/TERM_BG/CARD/BORDER/CREAM/BODY/DIM/GOLD/CYAN/GREEN/PURPLE/RED），禁止新增硬编码色值。
- 金色 `GOLD` 是甲方动作专属色（勿滥用到 tab/工具栏）。
- 不用 emoji、不引外部图标资源（GB18030 位图字体毒化回退）；沿用已在渲染的几何字形（`● × ＋` 等）。
- iced 0.14 生态；不 fork/改 vendored Flyfish bundle（`flyfish-file-viewer-web-full.iife.js`），只改 `host.html`。
- 每 task 收尾 `cargo clippy -p dozer-app --all-targets` clean、`cargo fmt -p dozer-app -- --check` 干净、`cargo test -p dozer-app` 绿。
- macOS 构建产物 `target/aarch64-apple-darwin/debug/dozer`；headless 环境真机目测那步改跑 `cargo build -p dozer-app` 确认编译，视觉留协调者/用户。
- iced 0.14 API 若与本计划字面不符（如 `scrollable::Direction`/`space::horizontal` 等），按编译器提示与本仓既有用法适配（本仓已用 `iced_widget::space::horizontal()`，勿用不存在的 `horizontal_space()`）。

---

### Task 1: #7 去 Flyfish 内容工具栏（host.html 一行）

**Files:**
- Modify: `crates/dozer-app/assets/flyfish/host.html`

**Interfaces:**
- Produces: 无代码接口；预览 webview 内容不再显示"搜索/下载/打印/html"工具栏。

- [x] **Step 1: 改 host.html——创建 viewer 后关工具栏**

在 `crates/dozer-app/assets/flyfish/host.html` 的 `<script>` 里，`el.setAttribute('theme', 'dark');` 之后、`document.body.appendChild(el);` 之前，加一行：
```js
  el.setAttribute('toolbar', 'false');
```
依据：Flyfish 读 `toolbar` 属性，内部布尔解析器把 `"false"/"0"/"no"/"off"` 判为 `false` → 工具栏不渲染。未知/老版本忽略该属性，无副作用。

- [x] **Step 2: 编译确认无回归**

Run: `cargo build -p dozer-app`
Expected: 编译通过（host.html 是静态资源，改动不影响 Rust 编译；此步仅确认没手滑碰坏别的）。

- [x] **Step 3: 真机目测（留用户）**

Run: `target/aarch64-apple-darwin/debug/dozer` → 打开任一文件预览
Expected: 内容上方不再有"搜索/下载/打印/html"工具栏。headless 环境跳过，报告注明留用户验收。

- [x] **Step 4: Commit**

```bash
git add crates/dozer-app/assets/flyfish/host.html
git commit -m "feat(P1L): 关闭 Flyfish 查看器自带工具栏(toolbar=false)"
```

---

### Task 2: #4 去预览/终端头部开发期标题 + 几何常量跟随

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（删两处 `header`；下调 `PREVIEW_CHROME_TOP_PX`、`CHROME_HEIGHT_PX`；改 `ime_cursor_area` 的 `y0`；加回归测试）

**Interfaces:**
- Consumes: `preview_content_bounds`、`terminal_pane_pixel_size`、`ime_cursor_area` 现有几何。
- Produces: 更新后的三处几何常量/公式（header 行移除后各减 26px）。

- [x] **Step 1: 写失败测试**（追加到 `workspace.rs` 的 `#[cfg(test)] mod tests`）

去掉 header 后：预览 chrome 顶 = `pane padding 8 + tab栏 30 + 地址栏 30 + 2处spacing(4*2=8)` = 76；终端 chrome = `padding 16 + 1处spacing 4 + tab栏 30` = 50。
```rust
#[test]
fn chrome_constants_exclude_removed_header() {
    // #4 去掉 header 行(22px + 一处 spacing 4 = 26)后的期望值,锁死防漂移遮挡。
    assert_eq!(PREVIEW_CHROME_TOP_PX, 76.0, "预览 chrome 顶应为去 header 后的 76");
    assert_eq!(CHROME_HEIGHT_PX, 50.0, "终端 chrome 高应为去 header 后的 50");
}
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app chrome_constants_exclude_removed_header`
Expected: FAIL —— 现值 `PREVIEW_CHROME_TOP_PX=102`、`CHROME_HEIGHT_PX=76`。

- [x] **Step 3: 下调两个常量**

`PREVIEW_CHROME_TOP_PX`（现 `8.0 + 22.0 + 30.0 + 30.0 + 12.0`）改为（去 header 22 + 一处 spacing 4）：
```rust
/// 左二内容区上方的 chrome 高度:pane 上内边距 8 + tab 栏 30 + 地址栏 30
///   + 两处 spacing 4*2。header 行已去(P1L #4),故不含表头项。
const PREVIEW_CHROME_TOP_PX: f32 = 8.0 + 30.0 + 30.0 + 8.0;
```
`CHROME_HEIGHT_PX`（现 `16.0 + 8.0 + 22.0 + 30.0`）改为：
```rust
const CHROME_HEIGHT_PX: f32 = 16.0 + 4.0 + 30.0; // 上下 padding + 1 处 spacing + tab 栏行(header 已去,P1L #4)
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app chrome_constants_exclude_removed_header`
Expected: PASS。

- [x] **Step 5: 删两处 header + 改 ime_cursor_area**

`preview_pane`（约 1946 行）删 `let header = text("预览 · P1d")...;`，把（约 2022 行）
```rust
    let mut content = column![header, tab_bar, addr].spacing(4);
```
改为
```rust
    let mut content = column![tab_bar, addr].spacing(4);
```
`terminal_pane`（约 2063 行）删 `let header = text("终端 · 本计划")...;`，把（约 2065 行）
```rust
    let mut content = column![header, tab_bar(ws)].spacing(4);
```
改为
```rust
    let mut content = column![tab_bar(ws)].spacing(4);
```
`ime_cursor_area` 里终端光标 `y0`（现 `TOP_BAR_HEIGHT + 8.0 + 22.0 + 4.0 + 30.0 + 4.0`，含 header 22 + 一处 spacing 4）改为：
```rust
        // 终端网格上方 chrome:顶栏 44 + 上 padding 8 + tab 栏 30 + spacing 4(header 已去,P1L #4)
        let y0 = TOP_BAR_HEIGHT + 8.0 + 30.0 + 4.0;
```

- [x] **Step 6: 全量测试 + clippy + fmt**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿（含 `preview_content_bounds_is_inside_col2` 若因 y 变化断言失败,按新几何更新其期望区间——y 少了 26,该测试的 y 相关断言需同步）；clippy clean；fmt 无输出。

- [x] **Step 7: 真机目测（留用户）**

Run: `cargo build -p dozer-app`（headless）
Expected（用户实机）：预览/终端顶部无"预览·P1d"/"终端·本计划"标题，tab 栏成顶行；终端网格底部光标行完整可见（几何未错位）；预览 webview 与边框对齐。

- [x] **Step 8: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(P1L): 去预览/终端头部开发期标题 + chrome 几何常量跟随下调 26px(回归测试锁定)"
```

---

### Task 3: #6 关闭 × 收进 tab pill 框内

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`tab_item` 终端 tab；`preview_pane` 内预览 tab 渲染闭包）

**Interfaces:**
- Consumes: `Message::SelectTab`/`CloseTab`（终端）、`Message::PreviewSelectTab`/`PreviewCloseTab`（预览）、`dot_color`、`blink_on`、激活判定。
- Produces: tab 视觉改为单 pill 容器包 `[● 标题 ×]`，激活态样式在容器上，× 在框内右侧。消息不变。

- [x] **Step 1: 改 `tab_item`（终端）——pill 容器包全部**

把 `tab_item`（约 2324 行）末尾结构从
```rust
    let select = button(label).on_press(Message::SelectTab(idx)).style(/* active: CARD+border / else 透明 */);
    let close = button(text("×").size(13).color(theme::DIM)).on_press(Message::CloseTab(idx)).style(/* 透明 */);
    row![select, close].spacing(2).into()
```
改为：select/close 按钮**均透明**（背景 None、无边框），激活态样式移到外层 pill 容器：
```rust
    let select = button(label)
        .on_press(Message::SelectTab(idx))
        .style(|_t, _s| button::Style {
            background: None,
            text_color: theme::CREAM,
            ..button::Style::default()
        });
    let close = button(text("×").size(13).color(theme::DIM))
        .on_press(Message::CloseTab(idx))
        .style(|_t, _s| button::Style {
            background: None,
            text_color: theme::DIM,
            ..button::Style::default()
        });
    container(row![select, close].spacing(2).align_y(iced_widget::core::Alignment::Center))
        .padding([2, 4])
        .style(move |_t: &iced_widget::Theme| {
            if active {
                container::Style {
                    background: Some(theme::CARD.into()),
                    border: Border { color: theme::BORDER, width: 1.0, radius: 6.0.into() },
                    ..container::Style::default()
                }
            } else {
                container::Style::default()
            }
        })
        .into()
```

- [x] **Step 2: 改预览 tab（`preview_pane` 内闭包，约 1950-1975 行）——同款 pill**

预览 tab 现为 `row![select, close].spacing(2)`，select 用 `if active` 分流 CARD/透明 border 的 button 样式。改为与终端一致：select/close 均透明按钮，外层 `container` 承载激活态（CARD 底 + BORDER 边 + radius 6 / 非激活 default），`padding([2,4])`，`.align_y(Center)`。select 仍发 `Message::PreviewSelectTab(idx)`，close 仍发 `Message::PreviewCloseTab(idx)`。

- [x] **Step 3: 编译 + 测试 + clippy + fmt**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿 / clean / 无输出。（`container`/`Alignment` 若缺 import 按提示补。）

- [x] **Step 4: 真机目测（留用户）**

Run: `cargo build -p dozer-app`（headless）
Expected（用户实机）：每个 tab 的 × 落在该 tab 的矩形框内（激活 tab 有 CARD 底 pill，× 在其右侧同框），归属清晰不误解。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(P1L): tab 关闭×收进 pill 框内(终端+预览),激活态样式移至容器"
```

---

### Task 4: #5 tab 栏超宽横向滚动 + 新建按钮常驻

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`tab_bar`（终端）与 `preview_pane` 的 tab 栏装配）

**Interfaces:**
- Consumes: 各 tab item（Task 3 后的 pill）、终端 `＋` 新建按钮、预览"打开文件…"按钮。
- Produces: tab 列表横向可滚动，新建/打开按钮钉在滚动区外右侧常驻。

- [x] **Step 1: 终端 `tab_bar`——tab 列表进横向 scrollable，＋常驻**

`tab_bar`（约 2136 行）现把每个 tab item 与末尾 `＋` 按钮一起 `row(items).spacing(4)`。改为：tab items 与 `＋` 分离——tab items 进横向 `scrollable` 占 `Fill`，`＋` 钉在其右：
```rust
    // items: 仅 tab（不含 ＋）
    let tabs_row = row(items).spacing(4);
    let scroller = iced_widget::scrollable(tabs_row)
        .direction(iced_widget::scrollable::Direction::Horizontal(
            iced_widget::scrollable::Scrollbar::new(),
        ))
        .width(Length::Fill);
    let plus = button(text("＋").size(15).color(theme::CREAM))
        .on_press(Message::NewTab)
        .style(/* 原 ＋ 样式:CARD 底 + BORDER 边 radius 2 */);
    row![scroller, plus].spacing(4).align_y(iced_widget::core::Alignment::Center).into()
```
（`Scrollbar::new()`/`Direction::Horizontal` 若与 0.14 实际 API 不符，按编译器提示适配——目标是横向滚动方向。）

- [x] **Step 2: 预览 tab 栏——同款,"打开文件…"常驻**

`preview_pane` 里 `let tab_bar = row(items).spacing(4);`（约 1993 行，`items` 现含末尾"打开文件…"按钮）改为：把"打开文件…"从 `items` 拆出，tab items 进横向 `scrollable` 占 `Fill`，"打开文件…"钉右：
```rust
    let tabs_row = row(items).spacing(4); // items 仅预览 tab
    let open_btn = button(text("打开文件…").size(13).color(theme::CREAM))
        .on_press(Message::PreviewPickFile)
        .style(/* 原样式 */);
    let tab_bar = row![
        iced_widget::scrollable(tabs_row)
            .direction(iced_widget::scrollable::Direction::Horizontal(
                iced_widget::scrollable::Scrollbar::new(),
            ))
            .width(Length::Fill),
        open_btn
    ]
    .spacing(4)
    .align_y(iced_widget::core::Alignment::Center);
```

- [x] **Step 3: 编译 + 测试 + clippy + fmt**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿 / clean / 无输出。

- [x] **Step 4: 真机目测（留用户）**

Run: `cargo build -p dozer-app`（headless）
Expected（用户实机）：开足够多 tab 超出栏宽时，tab 区可左右滚动；`＋`/"打开文件…"始终常驻右侧可点，不被挤走/滚走。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(P1L): 预览/终端 tab 栏超宽横向滚动 + 新建按钮钉滚动区外常驻"
```

---

## 收尾（全 task 完成后）

- [x] **回归 + 人工验收**：全量 `cargo test -p dozer-app` 绿、clippy/fmt 干净；真机对 4 条逐项 ✓/✗ 记录到 `docs/superpowers/specs/2026-07-27-p1l-acceptance.md`（视觉为主，验收权归用户）。
- [x] **落档**：验收通过后勾选本计划全 box，规格 §7 视需要标注。
- [x] **分支收尾**：`superpowers:finishing-a-development-branch` 合入 main。

## 自检记录（写计划时）

- **Spec 覆盖**：spec §3 #4→Task2、#5→Task4、#6→Task3、#7→Task1；§5 测试策略（#4 几何断言 + 其余 build/视觉）逐 task 落实；§4 降级（空 tab/未知属性）在 Task1/Task4 说明。无遗漏。
- **占位扫描**：无 TBD/TODO；视觉验收留用户是本迭代明确策略（headless）非占位。
- **类型/数值一致**：几何 header=22 + spacing 4=26；PREVIEW_CHROME_TOP_PX 102→76、CHROME_HEIGHT_PX 76→50、ime y0 去 22+4，三处口径一致且测试锁定 76/50。tab 消息名 SelectTab/CloseTab（终端）、PreviewSelectTab/PreviewCloseTab（预览）与现码一致。
- **风险**：iced 0.14 `scrollable::Direction`/`Scrollbar` 实际 API 名可能不同（Task4 已注明按编译器适配，同 P1k `space::horizontal` 前例）。
