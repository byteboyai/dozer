# Dozer 预览/终端 chrome 清理 + tab 栏打磨设计（P1L，迭代 A）

> 状态：设计中，待用户终审。
> 需求来源：2026-07-27 P1k dogfood 攒的 7 条反馈（`scratchpad/dogfood-feedback.md`）。
> 本 spec 只覆盖**迭代 A**（反馈 #4/#5/#6/#7）；#1 分栏拖动、#3 右键菜单、#2 图标基建各自后续迭代（顺序 A→B→D→C，用户确认）。
> 上游规格：`docs/superpowers/specs/2026-07-14-dozer-phase1-design.md` §7。

## 1. 目标

清理预览/终端两栏顶部的开发期残留、把 tab 栏做得经得起多 tab 与误操作，去掉预览内容里第三方查看器自带的工具栏。四条都小、聚在预览/终端同一区域、低风险，合为一个迭代一把清。

## 2. 现状

- **预览栏** `preview_pane`：`column![header, tab_bar, addr]`，`header = text("预览 · P1d")`（开发期 label）；`tab_bar = row(items).spacing(4)` 无溢出处理，每个 tab 是 `row![select, close].spacing(2)`（关闭 × 是选择按钮的兄弟）；末尾"打开文件…"按钮。
- **终端栏** `terminal_pane`：同构——`header = text("终端 · …")`、`tab_bar` 同款无滚动、`tab_item` 也是 `row![select, close]`，末尾 `＋` 新建。激活 tab 的 pill 态（CARD 底+边框，P1k Task4 加）只包住 select 按钮，× 落在 pill 外。
- **预览内容** 经 `dozer://flyfish/host.html?p=<abs>` 由 vendored Flyfish 查看器（`flyfish-file-viewer-web-full.iife.js`）渲染；查看器自带一条"搜索/下载/打印/html"工具栏。
- **几何耦合**：`PREVIEW_CHROME_TOP_PX = 8+22+30+30+12`（22=header 行、30=tab 栏、30=地址栏）喂 webview bounds；`CHROME_HEIGHT_PX = 16+8+22+30`（22=header、30=tab 栏）喂终端网格换算。header 行一旦移除，这两个常量必须同步下调，否则 webview/终端网格错位（P1k 终审逮到过同类回归）。

## 3. 设计

### #4 去头部开发期标题

- `preview_pane`：`column![header, tab_bar, addr]` → `column![tab_bar, addr]`，删 `header` 绑定。tab 栏升为该栏顶行。
- `terminal_pane`：删其 `header` 文本，tab 栏升为顶行。
- **几何跟随**：header 行 22px + 一处 spacing 4 = 26px 消失。
  - `PREVIEW_CHROME_TOP_PX` 减 26（去掉 `22` 项并少一处 spacing）。
  - `CHROME_HEIGHT_PX` 减 26（去掉 `22` 项并少一处 spacing）。
  - `ime_cursor_area` 里终端光标 `y0` 的 header 项（`22.0 + 一处 spacing`）一并去掉。
  - 加回归测试锁定新常量值（断言 chrome 分解不再含 header 项，仿 P1k 的 `terminal_pane_height_excludes_top_and_status_bars`）。

### #5 tab 栏横向滚动

- 预览与终端两处 `tab_bar` 的 `row(items)` 包进 iced `scrollable`（横向 `Direction::Horizontal`），宽度 `Length::Fill`。
- **新建/打开按钮钉在滚动区外右侧常驻**：`row![scrollable(tabs).width(Fill), 新建按钮]`——tab 溢出时中段滚动，`＋`/"打开文件…"永远可达。
- 横向滚动条视觉先用 iced 默认（可能一条细横条）；细化排后续，不阻塞本迭代。

### #6 关闭 × 收进 tab 框

- `tab_item`（终端）与预览 tab 渲染现为 `row![select, close].spacing(2)`，激活 pill 只包 select。
- 改为单个 **pill 容器**包住 `row![● + 标题(select 透明按钮) + ×(close 透明按钮)]`：pill 容器承载激活态样式（CARD 底 + BORDER 边 + radius 6），内层选择/关闭按钮均透明背景无边框。× 落在框内右侧。
- select 仍发 `SelectTab(idx)`/`PreviewSelectTab(idx)`，close 仍发 `CloseTab(idx)`/`PreviewCloseTab(idx)`——消息不变，只重排容器归属。
- 闪烁点逻辑（`blink_on`）与激活判定不变。

### #7 去 Flyfish 工具栏

- `crates/dozer-app/assets/flyfish/host.html`：创建 `<flyfish-file-viewer>` 后加 `el.setAttribute('toolbar', 'false')`。
- 依据：Flyfish 读 `toolbar` 属性，经内部布尔解析器把 `"false"/"0"/"no"/"off"` 判为 `false`，工具栏不渲染。一行，不改/不 fork vendored bundle。

## 4. 错误处理与降级

- 无 tab 时 tab 栏为空、`scrollable` 空内容不报错；新建按钮常驻可用。
- host.html 属性对老/不识别该属性的 Flyfish 版本无副作用（未知属性被忽略）。
- 颜色一律取自 `theme.rs`，不新增硬编码色值。

## 5. 测试策略

- **#4 几何**：可判定，加断言测试锁定 `PREVIEW_CHROME_TOP_PX`/`CHROME_HEIGHT_PX` 新值与 chrome 分解一致（防再漂移致遮挡）。
- **#5/#6**：view 层结构改动，iced `view()` 难单元断言；headless 环境实施方只跑 `cargo build -p dozer-app` 确认编译，**真机视觉留用户验收**（tab 溢出滚动、× 在框内、按钮常驻可达）。
- **#7**：静态资源一行改动，真机目测工具栏消失，留用户验收。
- 每步收尾 `cargo clippy --all-targets && cargo fmt -- --check` 干净、`cargo test -p dozer-app` 绿。

## 6. 非目标

横向滚动条的视觉细化、tab 拖拽重排、tab 中键关闭、Flyfish 其它工具（下载/打印）的按需保留——本迭代不做。分栏拖动(#1)、右键菜单(#3)、图标基建(#2)属后续迭代。
