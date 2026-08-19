# dozer-app 迁移 theme::font + theme::geometry 到 byteui

**状态:已批准(brainstorming 会话,2026-08-19)**

## 背景

这是 `byteui` 落地后 4 个迁移子项目里的第 4 个(顺序调整后:`interaction` →
`theme::color` → `theme::icon_size` → **`theme::font`+`theme::geometry`**,
见 `2026-08-19-dozer-app-icon-size-migration-design.md`——`icon_size` 提前
是因为 `byteui::theme::font`/`byteui::theme::geometry` 内部都依赖
`byteui::theme::icon_size::scale()`,必须先确保 `dozer-app` 侧的缩放调用
都已指向同一个单例,这次迁移才不会把"文字/几何缩放"和"图标缩放"拆成两个
不同步的真相源)。

**前提:migration #3(`icon_size`)已合并到 main。** 迁移完成后
`crates/dozer-app/src/theme/icon_size.rs` 已不存在,`font.rs`/`geometry.rs`
内部对 `icon_size` 的引用已经指向 `byteui::theme::icon_size`——这次不需要
再碰这部分。

`byteui::theme::font`(7 个 accessor:`dot_sm`/`caption_sm`/`caption`/
`label`/`body`/`subtitle`/`title`)和 `dozer-app` 现有 `theme/font.rs` **逐
字节一致**(已核对,`diff` 零输出)——纯调用点迁移,和 migration #1/#2/#3
同一套手法,不需要特殊处理。

`byteui::theme::geometry` **不是**这样的干净情况。`dozer-app` 的
`theme/geometry.rs` 在 `byteui` 建库(2026-08-18)之后又长出了 3 个新函数
——`tree_row_h`/`tree_chrome_top_px`/`tree_chrome_bottom_px`(文件树拖拽
命中测试用,`extensions/files.rs` 依赖),`byteui` 那份还没有。摸底发现
这 3 个函数**不能**整体搬进 `byteui`:它们依赖 `crate::theme::region::
project_pane()`(区域样式,未迁移,`region` 迁移不在这 4 个子项目范围
内)、`crate::theme::terminal_font::line_height_factor()`(终端字号,同样
不在范围内、且按其自身文档定位是永久留本地的模块)、`crate::workspace::
tree_row_font_size()`(`workspace.rs` 里的 app 专属函数,不是 theme 模块)
——三个依赖没有一个是可以搬进 `byteui`(不依赖 `dozer-app`)的,搬了就
破坏"`byteui` 零依赖 `dozer-app`"的边界。

**这次迁移因此把 `geometry.rs` 一分为二**:32 个"纯几何常量" accessor
(`icon_rail_width`/`divider_width`/…/`h0_sidebar_width`,和 `byteui` 那份
逐字节一致,已核对)整体迁移、调用点全部改指向 `byteui::theme::geometry`;
3 个 `tree_*` 函数连同它们的调用点(`app.rs` 6 处、`extensions/files.rs`
3 处)**原样留在 `dozer-app` 本地**,`theme/geometry.rs` 这个文件不删,只
是从"35 个函数的大文件"瘦身成"3 个函数的小文件"。摸底同时发现:这 3 个
函数一个都不读 `assets/theme/workspace.json` 的 `geometry` 节点(不引用
`GEOMETRY` 这个 `LazyLock` 静态量的任何字段),所以 32 个 accessor 删除后,
支撑它们的 JSON 解析脚手架(`Geometry` 结构体、`RawWorkspaceFile`、
`RAW`/`load`/`GEOMETRY`)**整体变成死代码**,要一并删除,不是只删 32 个
`pub fn`。测试模块同理:4 个现有测试里 3 个(`values_match_pre_migration_
literals`/`min_window_width_is_derived_not_duplicated_in_json`/
`malformed_json_panics`)测的是即将删除的 32 个 accessor 和 JSON 解析,
已在 `byteui` 那份逐字节核对过(建库时原样迁入),`dozer-app` 这份可以
安全删除;只有 `tree_geometry_matches_composition` 测 3 个留本地的函数,
要保留。

## 目标 / 非目标

**目标**:

1. **`font.rs` 完全删除**:18 个文件(17 个 app 级文件 + `geometry.rs`
   内部 2 处)、约 216 处 `font::` 调用点改成 `byteui::theme::font::...`;
   删除 `crates/dozer-app/src/theme/font.rs`,删除 `theme.rs` 里的
   `pub mod font;`。
2. **`geometry.rs` 部分迁移**:32 个基础 accessor 的调用点(14 个文件,
   约 293 处,不含 `tree_row_h`/`tree_chrome_top_px`/`tree_chrome_bottom_px`
   这 3 个的调用点)改成 `byteui::theme::geometry::...`。
3. `geometry.rs` 文件本身删除 32 个 `pub fn` 及其支撑的 JSON 解析脚手架
   (`Geometry`/`RawWorkspaceFile`/`RAW`/`load`/`GEOMETRY`),删除测试模块
   里对应的 3 个测试;保留 `tree_row_h`/`tree_chrome_top_px`/
   `tree_chrome_bottom_px` 三个函数、`tree_geometry_matches_composition`
   测试,更新文件头部文档注释(不再是"35 个函数解析 workspace.json 的
   geometry 节点",改成"3 个文件树几何推导函数,不解析任何 JSON")。
   `geometry.rs` 内部对 `font::subtitle`/`body` 的 2 处引用(`tree_chrome_
   top_px` 用)改指向 `byteui::theme::font`。
4. 迁移完成后 `dozer-app` 编译、测试、clippy、fmt 全绿,GUI 视觉(全 App
   文字大小、窗口/面板/菜单/滚动条/tab 等几何尺寸)与文件树拖拽命中(依赖
   `tree_row_h`/`tree_chrome_top_px`/`tree_chrome_bottom_px`)与迁移前
   逐一核对无差异。

**非目标**:

- **不迁移 `tree_row_h`/`tree_chrome_top_px`/`tree_chrome_bottom_px` 到
  `byteui`**——见背景,三者依赖未迁移/永久本地/app 专属的模块,迁移会
  破坏 `byteui` 的依赖边界。这 3 个函数、它们的 9 处调用点(`app.rs` 6 +
  `files.rs` 3)、`tree_geometry_matches_composition` 测试**都不改动**。
- **不迁移 `theme::region.rs`/`theme::terminal_font.rs`**——`tree_chrome_
  top_px`/`tree_chrome_bottom_px` 依赖的两个模块,不在这 4 个子项目
  范围内,`region`/`terminal_font` 本身没有迁移计划。
- **不改动 `theme::homespace_font.rs`/`theme::homespace_color.rs`**——
  独立配色/字号系统,和 ByteBoy2077 主题(`theme::font`/`theme::geometry`)
  无关联。
- **不改变任何视觉/几何效果**。`byteui::theme::font::body()` 和被删除的
  `crate::theme::font::body()`、`byteui::theme::geometry::icon_rail_width()`
  和被删除的 `crate::theme::geometry::icon_rail_width()` 在同一份
  `assets/theme/workspace.json` 基准值下逐字节一致(建 `byteui` 时已用
  同一份 JSON 原样迁移,已核对)。

## 架构与数据流

### 前置检查(执行前先确认)

```bash
grep -c "byteui" crates/dozer-app/Cargo.toml       # 应 ≥ 1
ls crates/dozer-app/src/theme/icon_size.rs 2>&1    # 应报 No such file,确认 migration #3 已合并
grep -rn "use super::icon_size" crates/dozer-app/src/theme/geometry.rs crates/dozer-app/src/theme/font.rs
# 应无输出——migration #3 应该已经把这两个文件内部的 icon_size 引用改指向 byteui
```

### `font.rs`:全量删除(和 migration #1/#2/#3 同一手法)

7 个函数名:`dot_sm`/`caption_sm`/`caption`/`label`/`body`/`subtitle`/
`title`。**两种引用形态,不混用**(和 migration #3 的教训一致:同一文件
不能先后套两条前缀不同的规则,否则会把已经替换过的路径二次误伤):

**形态 A——`crate::theme::font::NAME(` 完整路径(3 个文件)**:
`extensions/database.rs`(24)、`extensions/ssh/sftp.rs`(4)、
`theme/geometry.rs`(2,内部引用,在 geometry.rs 自己的 Task 里处理,不算
在下面"18 个调用方文件"里)。

**形态 B——`theme::font::NAME(`,经 `use crate::theme;`(15 个文件)**:
`workspace.rs`(40)、`extensions/todo.rs`(29)、`extensions/git_log.rs`(23)、
`extensions/ssh.rs`(19)、`extensions/project.rs`(17)、`extensions/files.rs`
(15)、`extensions/acceptance.rs`(13)、`app.rs`(11)、`extensions/usage.rs`
(10)、`extensions/search.rs`(9)、`extensions/footbar.rs`(7)、
`extensions/browser.rs`(7)、`menu.rs`(4)、`preview.rs`(1)、
`diff_render.rs`(1)。

没有文件同时出现两种形态(已逐一核对),每个文件套一条规则即可。

```bash
# 形态 A(database.rs / sftp.rs)
sed -i '' \
  -e 's/crate::theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/crate::theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/crate::theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/crate::theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/crate::theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/crate::theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/crate::theme::font::title(/byteui::theme::font::title(/g' \
  <file>

# 形态 B(其余 15 个文件)
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  <file>
```

### `geometry.rs`:32 个基础 accessor 迁移,3 个 `tree_*` 留本地

32 个函数名(**不含** `tree_row_h`/`tree_chrome_top_px`/
`tree_chrome_bottom_px`——规则列表里没有这 3 个名字,它们的调用点无论
用哪种前缀形态都不会被下面的规则匹配到,天然被跳过,不需要额外排除逻辑):

`icon_rail_width` `divider_width` `min_zone_width` `min_split_ratio`
`max_split_ratio` `default_split_ratio` `initial_window_size`
`min_window_height` `min_window_width` `top_bar_height`
`project_tab_max_width` `project_tab_add_button_width` `scrollbar_width`
`scrollbar_thumb_width` `status_bar_height` `footbar_height`
`context_menu_width` `context_menu_height` `chrome_width_px`
`chrome_height_px` `preview_chrome_top_px` `browser_chrome_top_px`
`maximize_overlay_padding` `project_tab_gap` `tab_bar_avail_px`
`rail_button_size` `tab_button_size` `tab_arrow_button_size`
`menu_item_width` `menu_gap` `menu_pad_v` `menu_pad_h` `h0_sidebar_width`

**两种引用形态,和 `font` 一样不能混用,但这次有 3 个文件两种形态都有**
(`app.rs`/`workspace.rs`/`extensions/usage.rs`)——这 3 个文件要先套形态 A
规则、再套形态 B 规则(顺序不能反,原因同 migration #3 Task 13 的教训:
先套 B 会把 `crate::theme::geometry::icon_rail_width(` 里的
`theme::geometry::icon_rail_width(` 子串提前命中,产出
`crate::byteui::theme::geometry::icon_rail_width(` 这种错误路径)。

**形态 A——`crate::theme::geometry::NAME(` 完整路径**:`main.rs`(部分,
12 处全是这个形态)、`menu.rs`(5)、`extensions/files.rs`(5,注意这 5 处
里另有 3 处是 `tree_row_h`,不进这次改动,规则天然跳过)、`layout.rs`(3)、
`panel_layouts.rs`(2)、`extensions/ssh/sftp.rs`(1)、`app.rs`(19,混合
文件,这是其中一种形态的计数)、`workspace.rs`(1,混合文件)、
`extensions/usage.rs`(1,混合文件)。

**形态 B——`theme::geometry::NAME(`,经 `use crate::theme;`**:
`preview.rs`(14)、`homespace.rs`(5)、`extensions/browser.rs`(4)、
`term_view.rs`(2)、`extensions/footbar.rs`(2)、`app.rs`(210,混合文件的
另一种形态,注意这 210 处里有 6 处是 `tree_chrome_top_px`/
`tree_chrome_bottom_px`,不进这次改动,规则天然跳过)、`workspace.rs`(5,
混合文件)、`extensions/usage.rs`(1,混合文件)。

```bash
# 形态 A
sed -i '' \
  -e 's/crate::theme::geometry::icon_rail_width(/byteui::theme::geometry::icon_rail_width(/g' \
  -e 's/crate::theme::geometry::divider_width(/byteui::theme::geometry::divider_width(/g' \
  -e 's/crate::theme::geometry::min_zone_width(/byteui::theme::geometry::min_zone_width(/g' \
  -e 's/crate::theme::geometry::min_split_ratio(/byteui::theme::geometry::min_split_ratio(/g' \
  -e 's/crate::theme::geometry::max_split_ratio(/byteui::theme::geometry::max_split_ratio(/g' \
  -e 's/crate::theme::geometry::default_split_ratio(/byteui::theme::geometry::default_split_ratio(/g' \
  -e 's/crate::theme::geometry::initial_window_size(/byteui::theme::geometry::initial_window_size(/g' \
  -e 's/crate::theme::geometry::min_window_height(/byteui::theme::geometry::min_window_height(/g' \
  -e 's/crate::theme::geometry::min_window_width(/byteui::theme::geometry::min_window_width(/g' \
  -e 's/crate::theme::geometry::top_bar_height(/byteui::theme::geometry::top_bar_height(/g' \
  -e 's/crate::theme::geometry::project_tab_max_width(/byteui::theme::geometry::project_tab_max_width(/g' \
  -e 's/crate::theme::geometry::project_tab_add_button_width(/byteui::theme::geometry::project_tab_add_button_width(/g' \
  -e 's/crate::theme::geometry::scrollbar_width(/byteui::theme::geometry::scrollbar_width(/g' \
  -e 's/crate::theme::geometry::scrollbar_thumb_width(/byteui::theme::geometry::scrollbar_thumb_width(/g' \
  -e 's/crate::theme::geometry::status_bar_height(/byteui::theme::geometry::status_bar_height(/g' \
  -e 's/crate::theme::geometry::footbar_height(/byteui::theme::geometry::footbar_height(/g' \
  -e 's/crate::theme::geometry::context_menu_width(/byteui::theme::geometry::context_menu_width(/g' \
  -e 's/crate::theme::geometry::context_menu_height(/byteui::theme::geometry::context_menu_height(/g' \
  -e 's/crate::theme::geometry::chrome_width_px(/byteui::theme::geometry::chrome_width_px(/g' \
  -e 's/crate::theme::geometry::chrome_height_px(/byteui::theme::geometry::chrome_height_px(/g' \
  -e 's/crate::theme::geometry::preview_chrome_top_px(/byteui::theme::geometry::preview_chrome_top_px(/g' \
  -e 's/crate::theme::geometry::browser_chrome_top_px(/byteui::theme::geometry::browser_chrome_top_px(/g' \
  -e 's/crate::theme::geometry::maximize_overlay_padding(/byteui::theme::geometry::maximize_overlay_padding(/g' \
  -e 's/crate::theme::geometry::project_tab_gap(/byteui::theme::geometry::project_tab_gap(/g' \
  -e 's/crate::theme::geometry::tab_bar_avail_px(/byteui::theme::geometry::tab_bar_avail_px(/g' \
  -e 's/crate::theme::geometry::rail_button_size(/byteui::theme::geometry::rail_button_size(/g' \
  -e 's/crate::theme::geometry::tab_button_size(/byteui::theme::geometry::tab_button_size(/g' \
  -e 's/crate::theme::geometry::tab_arrow_button_size(/byteui::theme::geometry::tab_arrow_button_size(/g' \
  -e 's/crate::theme::geometry::menu_item_width(/byteui::theme::geometry::menu_item_width(/g' \
  -e 's/crate::theme::geometry::menu_gap(/byteui::theme::geometry::menu_gap(/g' \
  -e 's/crate::theme::geometry::menu_pad_v(/byteui::theme::geometry::menu_pad_v(/g' \
  -e 's/crate::theme::geometry::menu_pad_h(/byteui::theme::geometry::menu_pad_h(/g' \
  -e 's/crate::theme::geometry::h0_sidebar_width(/byteui::theme::geometry::h0_sidebar_width(/g' \
  <file>

# 形态 B(同 32 条,前缀去掉 crate::)
sed -i '' \
  -e 's/theme::geometry::icon_rail_width(/byteui::theme::geometry::icon_rail_width(/g' \
  -e 's/theme::geometry::divider_width(/byteui::theme::geometry::divider_width(/g' \
  -e 's/theme::geometry::min_zone_width(/byteui::theme::geometry::min_zone_width(/g' \
  -e 's/theme::geometry::min_split_ratio(/byteui::theme::geometry::min_split_ratio(/g' \
  -e 's/theme::geometry::max_split_ratio(/byteui::theme::geometry::max_split_ratio(/g' \
  -e 's/theme::geometry::default_split_ratio(/byteui::theme::geometry::default_split_ratio(/g' \
  -e 's/theme::geometry::initial_window_size(/byteui::theme::geometry::initial_window_size(/g' \
  -e 's/theme::geometry::min_window_height(/byteui::theme::geometry::min_window_height(/g' \
  -e 's/theme::geometry::min_window_width(/byteui::theme::geometry::min_window_width(/g' \
  -e 's/theme::geometry::top_bar_height(/byteui::theme::geometry::top_bar_height(/g' \
  -e 's/theme::geometry::project_tab_max_width(/byteui::theme::geometry::project_tab_max_width(/g' \
  -e 's/theme::geometry::project_tab_add_button_width(/byteui::theme::geometry::project_tab_add_button_width(/g' \
  -e 's/theme::geometry::scrollbar_width(/byteui::theme::geometry::scrollbar_width(/g' \
  -e 's/theme::geometry::scrollbar_thumb_width(/byteui::theme::geometry::scrollbar_thumb_width(/g' \
  -e 's/theme::geometry::status_bar_height(/byteui::theme::geometry::status_bar_height(/g' \
  -e 's/theme::geometry::footbar_height(/byteui::theme::geometry::footbar_height(/g' \
  -e 's/theme::geometry::context_menu_width(/byteui::theme::geometry::context_menu_width(/g' \
  -e 's/theme::geometry::context_menu_height(/byteui::theme::geometry::context_menu_height(/g' \
  -e 's/theme::geometry::chrome_width_px(/byteui::theme::geometry::chrome_width_px(/g' \
  -e 's/theme::geometry::chrome_height_px(/byteui::theme::geometry::chrome_height_px(/g' \
  -e 's/theme::geometry::preview_chrome_top_px(/byteui::theme::geometry::preview_chrome_top_px(/g' \
  -e 's/theme::geometry::browser_chrome_top_px(/byteui::theme::geometry::browser_chrome_top_px(/g' \
  -e 's/theme::geometry::maximize_overlay_padding(/byteui::theme::geometry::maximize_overlay_padding(/g' \
  -e 's/theme::geometry::project_tab_gap(/byteui::theme::geometry::project_tab_gap(/g' \
  -e 's/theme::geometry::tab_bar_avail_px(/byteui::theme::geometry::tab_bar_avail_px(/g' \
  -e 's/theme::geometry::rail_button_size(/byteui::theme::geometry::rail_button_size(/g' \
  -e 's/theme::geometry::tab_button_size(/byteui::theme::geometry::tab_button_size(/g' \
  -e 's/theme::geometry::tab_arrow_button_size(/byteui::theme::geometry::tab_arrow_button_size(/g' \
  -e 's/theme::geometry::menu_item_width(/byteui::theme::geometry::menu_item_width(/g' \
  -e 's/theme::geometry::menu_gap(/byteui::theme::geometry::menu_gap(/g' \
  -e 's/theme::geometry::menu_pad_v(/byteui::theme::geometry::menu_pad_v(/g' \
  -e 's/theme::geometry::menu_pad_h(/byteui::theme::geometry::menu_pad_h(/g' \
  -e 's/theme::geometry::h0_sidebar_width(/byteui::theme::geometry::h0_sidebar_width(/g' \
  <file>
```

### `geometry.rs` 自身:删 32 个函数 + JSON 脚手架 + 3 个测试,留 3 个函数

按当前文件结构(`crates/dozer-app/src/theme/geometry.rs`,410 行):

- **删除**:第 1 行开头的历史遗留路径注释(`// crates/dozer-app/src/
  workspace_geometry.rs`,和现在的文件路径对不上,是早年重命名遗留的
  死注释,顺手清掉)、文件头文档注释(整段重写,见下)、`use super::
  icon_size;`(如果 migration #3 已经清过就已经不在了)、`use serde::
  Deserialize;`、`use std::sync::LazyLock;`、`RAW` 常量、`Geometry`
  结构体、`RawWorkspaceFile` 结构体、`load` 函数、`GEOMETRY` 静态量、
  32 个 `pub fn`(`icon_rail_width` 到 `h0_sidebar_width`)、测试模块里
  `values_match_pre_migration_literals`/`min_window_width_is_derived_
  not_duplicated_in_json`/`malformed_json_panics` 三个测试。
- **保留**:`tree_row_h`/`tree_chrome_top_px`/`tree_chrome_bottom_px`
  三个函数(内部对 `font::subtitle`/`font::body` 的 2 处引用改成
  `byteui::theme::font::subtitle`/`body`,对 `icon_size::row` 的引用已经
  在 migration #3 里指向 `byteui::theme::icon_size`,对 `region::
  project_pane`/`terminal_font::line_height_factor`/`workspace::
  tree_row_font_size` 的引用不变,仍是 `crate::theme::region::.../
  crate::theme::terminal_font::.../crate::workspace::...`)、测试模块里
  `tree_geometry_matches_composition`(不改动)。
- **新文件头文档注释**(替换原来描述"35 个函数解析 workspace.json 的
  geometry 节点"的那段):

```rust
//! 文件树几何推导:`tree_row_h`(单行高度)、`tree_chrome_top_px`/
//! `tree_chrome_bottom_px`(Scrollable 视口上下的 chrome 高度),供
//! `extensions/files.rs` 的外部拖拽命中测试把窗口 Y 坐标换算成"可见行
//! 序号"。
//!
//! 其余几何 token(图标栏宽、窗口尺寸下限、右键菜单尺寸等)已迁移到
//! `byteui::theme::geometry`——这里只剩这 3 个函数,因为它们依赖
//! `region`/`terminal_font`/`workspace::tree_row_font_size` 这些未迁移
//! 或 app 专属的模块,不能搬进不依赖 `dozer-app` 的 `byteui`。不解析
//! 任何 JSON,纯由其它 token 组合推导。
```

## 错误处理

不适用——和 migration #1/#2/#3 同定位:font 部分是纯路径重写,没有新增
运行时逻辑。geometry 部分多了"删代码"这一步,风险点是**删多了或删
少了**——编译器兜底:如果误删了 `tree_*` 还在用的辅助代码,或者调用方
残留了指向已删除本地函数的引用,`cargo build` 会直接报"找不到函数/
模块",不会静默产生错误行为。

## 测试策略

1. 每个调用方文件改完后单独跑 `cargo build -p dozer-app --bin dozer`。
2. 全部文件改完 + `geometry.rs` 瘦身完成后:`cargo test -p dozer-app
   --bin dozer`(基线同 migration #1/#2/#3:484 passed / 2 failed,两个
   已知的、与本次改动无关的 terminal grid 尺寸测试失败——注意这次删掉
   了 3 个测试,通过总数会比基线少 3,失败数不变,需要在验收时说明这个
   数字变化是预期的,不是新增失败)、`cargo clippy -p dozer-app
   --all-targets -- -D warnings`、`cargo fmt -p dozer-app -- --check`。
3. 独立命名的临时二进制做两组专项核对:
   - **文字/几何视觉核对**(覆盖面全 App,`font`/`geometry` 是使用面
     仅次于 `color` 的 token 类别):四栏骨架尺寸(rail 宽、分隔线宽、
     窗口最小尺寸)、顶栏/状态栏/footbar 高度、右键菜单尺寸、tab 栏
     尺寸与翻页箭头、滚动条宽度、各面板标题/正文/次要文字字号
     (`todo.rs`/`git_log.rs`/`ssh.rs` 这几个用字号最密集的面板)。
   - **文件树拖拽命中专项**(这次唯一有真实回归风险的部分——`tree_row_h`/
     `tree_chrome_top_px`/`tree_chrome_bottom_px` 三个函数本身没改动,
     但它们内部引用的 `font::subtitle`/`body` 改了来源,如果 `byteui`
     那份取值和本地原来的不一致,这三个函数算出的高度会跟着偏,进而让
     外部文件拖拽的目标行判定错位):在文件树里从外部(Finder)拖一个
     文件到列表中间某一行、开头一行、末尾一行,确认插入位置和视觉上
     瞄准的行一致,没有"拖到第 5 行结果插到第 4 或第 6 行"的偏差。
   完成后关闭该临时实例、删除临时二进制,不留后台进程。

## 排期备注

22 个调用方文件相互独立,可以按任意顺序做、分批提交分批审阅,但
`app.rs`/`workspace.rs`/`extensions/usage.rs` 这 3 个 geometry 混合形态
文件内部必须先套形态 A 规则再套形态 B 规则,不能颠倒。`geometry.rs`
自身的瘦身(删 32 函数 + JSON 脚手架 + 3 测试)必须在全部调用方文件改完
之后才能做——中间状态下这些函数还要留着,否则未迁移的调用方编译不过。
`font.rs` 的删除同理,必须在 18 个调用方(17 文件 + `geometry.rs` 内部
引用)全部改完之后。

这是 4 个迁移子项目里的最后一个。完成后 `dozer-app` 的 `theme` 模块只
剩 `geometry`(3 个 `tree_*` 函数)、`region`、`terminal_font`、
`homespace_color`、`homespace_font` 五个本地文件——`color`/`icon_size`/
`font` 三个 token 类别、以及 `interaction` 里的 `icons`/`tabs`/`cards`/
`scrollbar`,全部收敛进 `byteui`,`dozer-app` 不再有和 `byteui` 逐字节
重复的源码。`region`/`terminal_font`/`homespace_color`/`homespace_font`
四个模块目前没有迁移计划(不依赖它们迁移的理由,也不属于这 4 个已批准
子项目的范围)——如果未来要迁,需要单独立项评估,`region`/
`terminal_font` 尤其要先解决"是否值得为了迁移而反过来让 `byteui` 支持
可插拔的区域样式/终端字号配置"这个更大的设计问题,不是照搬这次的手法
就能做。
