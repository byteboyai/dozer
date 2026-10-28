# X1 Task 1 盘点:页签部件与菜单弹层的三份实现

日期:2026-10-28。对应计划 `docs/superpowers/plans/2026-10-28-x1-cross-repo-migration.md` Task 1。**只读盘点,没有改任何代码。** 版本基线:Dozer `main` `33a66834`、Digger `main` `2a23894`、byteui `v0.4.6`、bytehost `v0.4.12`。

## 0. 结论(先看这个)

1. **计划漏了一份复制品。** Digger 有**两份**页签复制品:`adapters/agent_tabs.rs`(618 行,绑死产品 `Msg`)和 `adapters/tab_chrome.rs`(435 行,泛型 `M`,预览页签用)。两份各自带一套 `tab_label` / `tab_container_style` / `panel_tab` / `overflow_button` / `overflow_menu`,与 Dozer 原件 `chrome/tab_widget.rs` 的同名函数几乎逐行相同。迁移范围应是 **1 个原件 + 2 份复制**。
2. **菜单外壳有一个硬依赖不在 byteui。** Dozer `menu::shell` 读 `crate::theme::region::context_menu()`,它来自 Dozer 本地的 `assets/theme/workspace.json`;`byteui::theme::region` 里**没有** `context_menu()`(只有 7 个外壳区域)。Digger 的 `menu_overlay` 是自己手写了一份同名区域(圆角 9、内边距 8、间距 1)。迁移需要把 `context_menu` 区域加进 byteui,并且 Digger 的 `scripts/check-shell-regions.sh`(写死"7 个区域")要同步改。
3. **两边菜单背景透明度不同,是一处需要你裁决的可见差异。** Dozer `context_menu()` 的背景 alpha 是 `0xfa/255 ≈ 0.98`;Digger 刻意改成 **1.0**,并留了测试 `the_menu_background_is_fully_opaque`,理由是 wgpu 线性混合下 0.98 会让底下的亮字约 15% 透出。迁入 byteui 只能选一个值。
4. **其余差异很小**:都集中在"Digger 去掉了悬停动画"和"Digger 把泛型 `M` 换成了产品 `Msg`",前者是产品约定(D2),后者迁入后自然消失。
5. **范围收敛**:Dozer 原件里有一批**不该迁**的东西(预览模式按钮、依赖 Dozer 国际化的按钮、`frosted` 噪点层)。

## 1. 菜单:`chrome/menu.rs`(Dozer,260 行,无测试)vs `adapters/menu_overlay.rs`(Digger,422 行,4 个测试)

| 符号 | Dozer | Digger | 差异 | 归宿 |
|------|:----:|:-----:|------|------|
| `item` / `item_locked` | ✓ | — | Digger 无对应 | 迁 byteui(通用) |
| `item_row` | ✓(泛型 `Msg`,宽 `Fixed(menu_item_width)`) | ✓(私有,绑 `Msg`,宽 `Fill`) | 逻辑相同;宽度不同(Digger 由外层 `rows_size` 固定宽);Dozer 用常量 `MENU_HOVER_RADIUS = 6`,Digger 硬编码 6 | 迁 byteui,取 Dozer 版;Digger 调用点补宽度参数或改用 `item_row_fill` |
| `item_row_fill` | ✓ | — | | 迁 byteui |
| `icon_leading` | ✓(`pub(crate)`) | ✓(私有) | 相同 | 迁 byteui,公开 |
| `separator` | ✓(宽 `Fixed(menu_item_width)`) | ✓(宽 `Fill`) | 留白同为 `menu_pad_h`;宽度不同 | 迁 byteui |
| `shell` | ✓(读 `theme::region::context_menu()`) | ✓ 名同义不同:`shell(content, anchor, width, on_dismiss)` 额外带整窗遮罩与定位 | **语义不同**:Dozer 的 `shell` 只是外框;Digger 的 `shell` 是"外框 + 遮罩 + 定位"。阴影参数相同(alpha 0.45、偏移 (0,10)、模糊 28) | 外框迁 byteui;遮罩+定位另拆(见下) |
| `shell_frosted` | ✓(依赖 Dozer 本地 `crate::frosted::noise_layer`) | — | 噪点贴图在 Dozer 本地 | **留 Dozer**(除非 `frosted` 也迁,不在本计划) |
| `menu_button`(私有) | ✓ | 合并在 `item_row` 里 | 同逻辑 | 随 `item_row` |
| `MenuAnchor` + `clamped` | — | ✓ | Digger 独有;Dozer 的弹层定位散在 `app/view.rs`(锚点钳制写在各调用点) | 迁 byteui(新共享 API) |
| `MenuOverlay<M>` / `OverlayRow` / `view` / `view_rows` / `menu_size` / `rows_size` | — | ✓ | **`MenuOverlay<M>` 的条目类型是 `dozer_panel_files::menu::MenuItem`**——byteui 不能依赖 Dozer 库 crate,这一块**不能整体迁**;`OverlayRow` + `view_rows` + `rows_size` 是产品无关的,可迁(去掉对 `product::Msg` 的绑定,改泛型) | `OverlayRow`/`view_rows`/`rows_size`/`MenuAnchor` 迁 byteui(泛型化);`MenuOverlay`/`view` 留 Digger |
| `region()`(Digger 私有) | — | ✓ 手写:圆角 9、内边距 `8*scale`、间距 `1*scale`、背景 `panel` 且 **alpha=1.0** | 见结论 3 | 并入 byteui 的 `context_menu()` 区域 |

**对 Dozer 的影响面**:引用 `chrome::menu` / `menu::item*` / `menu::shell` / `menu::separator` 的文件 14 个。若 `menu.rs` 缩成 `pub use byteui::…` 重导出,这些调用点不用改。

**风险点**:
- `menu::shell` 的语义在两边不同(外框 vs 外框+遮罩+定位)。迁入 byteui 时必须给 Digger 版换名(建议 `overlay_shell`),不能靠同名覆盖。
- Digger 的 `row_height()` 用 `font::label() as f32 * 1.2` 估算行高,与真实渲染高度可能有出入(只用于钳制估算)。迁入后保持原行为,不顺手"修正"。

## 2. 页签:`chrome/tab_widget.rs`(Dozer 原件,717 行,4 个测试)vs Digger 两份

| 符号 | Dozer `tab_widget.rs` | Digger `agent_tabs.rs` | Digger `tab_chrome.rs` | 归宿 |
|------|----------------------|----------------------|----------------------|------|
| `PANEL_TAB_PAD_LEFT/X/Y` = 10 / 4 / 0 | `pub(crate)` | 私有重复 | 私有重复 | 迁 byteui,公开 |
| `NO_TAB_W_LIMIT` | ✓ | 内联 `f32::MAX` | ✓ 重复 | 迁 byteui |
| `tab_label` | ✓ 泛型 `M`,颜色用 `color::mix(dim, gold, hover_t)` | ✓ 绑 `Msg`,颜色走 `title_color(active, hover)`(同公式) | ✓ 泛型 | 迁 byteui,取 Dozer 版 |
| `tab_container_style` | ✓(选中 `card`+1px 边框+圆角 6;悬停 `tab_hover` alpha=hover) | ✓ 同 | ✓ 同 | 迁 byteui |
| `PanelTabArgs` + `panel_tab` | ✓ 泛型,参数结构体(11 个参数已 Builder 化) | ✓ 绑 `Msg`,**位置参数 6 个**,输入是库类型 `TabArgs<Msg>`(带 `dot`) | ✓ 泛型 | 迁 byteui 取 Dozer 版;Digger 两份改为"库 `TabArgs` → byteui `PanelTabArgs`"的映射 |
| `controlled_tooltip` | `app/view.rs` 里,带 `position` 参数,延迟取 `HOVER_TOOLTIP_DELAY`(=`dozer_panel_browser::TAB_TOOLTIP_DELAY`=2s) | agent_tabs 里自带一份,**位置恒 Top**,延迟取 `agent::TOOLTIP_DELAY`(2s) | 同样自带 | 迁 byteui,`delay` 做参数或常量(两边值同为 2s);**测试 `the_tooltip_widget_tree_shape_does_not_depend_on_show` 两边都有,随迁** |
| `top_bar_font` | `app/view.rs` | `tab_font()` 重复 | 同 | 迁 byteui(`Weight::Medium`) |
| `tab_overflow_button` / `overflow_button` | ✓(`ChevronsDown`,`icon_button_entry`) | ✓ 同 | ✓ 同 | 迁 byteui |
| `tab_overflow_items`(原生 NSMenu 项,仅 macOS) | ✓(4 个测试) | — | — | 迁 byteui(依赖 `byteui::native_menu::Item`,byteui 本来就有);Dozer 原路径重导出 |
| `tab_overflow_menu`(iced 版,非 mac 回退) | ✓ 泛型,`TabOverflowMenuArgs` | ✓ `overflow_menu`(绑 `Msg`,位置参数 8 个,几何用自带 `overflow_geometry`:220–560·scale、钳窗口) | ✓ `overflow_menu` | 迁 byteui;Digger 的 `overflow_geometry` 与 Dozer 的"`avail_w = window - anchor - 24`、最小/最大宽常量"是同一算法的两种写法,**迁移前要写一条等价性测试**(同输入同输出) |
| `tab_render_mode_button` / `tab_tabular_mode_button` / `tab_json_mode_button` | ✓ | — | `mode_button`(单个) | **留 Dozer**:预览专属;其中两个依赖 `crate::i18n::host_text`。Digger 的 `mode_button` 留 Digger |
| `combined_hover` / `title_color` / `close_color` / `tab_bar_avail_px` / `TAB_BAR_RESERVE` | 内联在 `panel_tab` | ✓ 公开函数 | — | `combined_hover`/`title_color`/`close_color` 迁 byteui(纯函数、已有测试价值);`tab_bar_avail_px` 与 `TAB_BAR_RESERVE`(72)是 Digger 页签条布局,**留 Digger** |
| `text_width_units` / `tab_display_width` | — | — | ✓ | **留 Digger**(库要求的宽度估算) |
| 悬停 | 渐变(`hover_t` 动画表驱动) | 阶跃(0/1) | 阶跃(调用方恒传 0) | **产品约定 D2,不是缺陷**;byteui 部件只吃 `hover_t: f32`,两边照常传各自的值 |

**测试**:Dozer `tab_widget.rs` 4 条(全是 `tab_overflow_items`),`agent_tabs.rs` 12 条,`tab_chrome.rs` 4 条,`menu_overlay.rs` 4 条。**`panel_tab` / `tab_label` / `tab_container_style` 在 Dozer 原件里没有单测**——迁移前没有"黄金度量"可抄,计划里 Task 3/4 的"黄金值"要靠在**搬家前**对 Dozer 现状跑一次度量取得(这是 Task 3/4 的第 0 步,见下)。

**对 Dozer 的影响面**:`tab_widget::` / `panel_tab(` 在 `dozer-app` 里约 36 处引用。重导出后调用点不动。

**对 Digger 的影响面**:`agent_tabs::` 在 `agent_tabs.rs` 之外有 5 处引用;`menu_overlay` 被 `product.rs`、`agent_display.rs`、`files.rs`、`agent_tabs.rs`、`mod.rs` 引用。

## 3. 对计划的修正(需要你确认后我更新计划)

| # | 修正 | 原因 |
|---|------|------|
| P1 | Task 4/6 的范围加上 `tab_chrome.rs`(第二份复制) | 结论 1 |
| P2 | Task 3 增加子步骤:byteui 新增 `theme::region::context_menu()`,并把 Digger `scripts/check-shell-regions.sh` 从"7 个区域"改成"8 个",Dozer `workspace.json` 的对应项保持不变 | 结论 2 |
| P3 | Task 3 里 Digger 版 `shell` 改名 `overlay_shell`,避免与 Dozer 的 `shell` 同名异义 | §1 风险点 |
| P4 | Task 3/4 各加"第 0 步":在**搬家前**对 Dozer 现状跑一次度量快照(`panel_tab`、`tab_container_style`、菜单外框的内边距/圆角/间距在缩放 1 和 2 下的数值),存成黄金值,搬家后逐项比对 | Dozer 原件没有这些单测 |
| P5 | Task 4 增加一条等价性测试:Digger `overflow_geometry` 与 Dozer `tab_overflow_menu` 内联几何在同一组输入上输出相同 | §2 |
| P6 | `MenuOverlay<M>` / `view`(依赖 `dozer_panel_files::menu::MenuItem`)**不迁**,只迁 `OverlayRow`/`view_rows`/`rows_size`/`MenuAnchor`(泛型化) | byteui 不能依赖 Dozer 库 crate |

## 4. 需要你裁决的点

1. **菜单背景 alpha:0.98(Dozer 现状)还是 1.0(Digger 现状)?** 选 1.0 会让 Dozer 的 iced 自绘菜单从微透明变成不透明(我没有逐个核对 Dozer 里还有哪些菜单走 iced 自绘;原生 NSMenu 不受影响)。Digger 的测试说明 0.98 会让亮字透出,这是已知缺陷。**我的建议:统一为 1.0**,但它改变 Dozer 的可见行为,所以由你定。
2. **P1–P6 是否按此修正计划。**
3. **悬停:** 本盘点把"Digger 无悬停动画"当作产品约定 D2,迁移后保持不变(byteui 部件只吃 `hover_t`)。如果你想借这次把 Digger 也做成渐变,那是新功能,另议。

## 5. 阶段 B 预检(`native_host.rs`,仅事实,不作结论)

- `adapters/native_host.rs` 367 行,自身 0 个单测;它的行为测试在使用方的测试文件里(如 `group_chat_native_tests.rs`),迁移时要跟着找。
- 使用方 4 个(`usage_native`、`todo_native`、`group_chat_native`、`review_native`)+ 独立的 `markdown/host.rs`(276 行,自带 wry builder 与读自身源码的钉死测试)。
- `bytehost-desktop` 里已有另外两处 wry builder(`app_webview.rs`、`panel.rs`),`native_host` 会是第三处;它的 `Action { Destroy, Create, Navigate, Zoom, SetBounds, Park, Show }` + `plan` 是"按 URL/identity 导航的可信子 webview"模型,而 `surface::lifecycle::LifecycleAction { Create{html,rect}, SetBounds, Evaluate, Destroy, Hide, Resume }` 是"推 HTML + 脚本的面板 surface"模型,**两者不同,不要强行合并**。
- Dozer 侧没有对应的"可信子 webview"执行器(走 webview 池),所以阶段 B 只有 Digger 受益,Dozer 只需要为钉版一致而重钉。
