# dozer-app 迁移 theme::font + theme::geometry 到 byteui Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `dozer-app` 里约 509 处 `font::`/`geometry::` 调用点(22 个文件)改成指向 `byteui::theme::font`/`byteui::theme::geometry`,删除本地 `theme/font.rs`,把本地 `theme/geometry.rs` 从"35 个函数"瘦身成"3 个文件树几何函数"(`tree_row_h`/`tree_chrome_top_px`/`tree_chrome_bottom_px`,它们依赖未迁移/app 专属模块,不能搬进 `byteui`)。

**Architecture:** `font` 是干净的整体迁移(和 migration #1/#2/#3 同手法):7 个函数、两种引用形态、按文件分 Task 套 sed。`geometry` 是部分迁移:32 个基础 accessor 的调用点迁移,3 个 `tree_*` 函数(及其 9 处调用点)原样不动——迁移规则只列 32 个名字,`tree_*` 不在规则里,天然被跳过,不需要额外排除逻辑。`geometry.rs` 文件本身单独一个 Task 处理"删 32 函数 + 死代码 JSON 脚手架 + 3 个过时测试,保留 3 个 `tree_*` 函数并更新它们内部对 `font`/`icon_size` 的引用"。

**Tech Stack:** Rust 2024,`byteui::theme::font`(7 个 accessor,已在 `byteui` 建库时原样迁入且逐字节核对一致)、`byteui::theme::geometry`(32 个 accessor,同样逐字节核对一致,已合并 main)。

**Spec:** `docs/superpowers/specs/2026-08-19-dozer-app-font-geometry-migration-design.md`

## Global Constraints

- **前提:migration #3(`icon_size`)已合并到 main。** 开工前先跑 `ls crates/dozer-app/src/theme/icon_size.rs 2>&1`(应报 No such file)和 `grep -rn "use super::icon_size" crates/dozer-app/src/theme/geometry.rs crates/dozer-app/src/theme/font.rs`(应无输出)确认,不满足就先去完成/合并那个迁移。
- **独立分支开发,不直接提交 main。** 在新分支(如 `feature/dozer-app-font-geometry-migration`)上完成全部 24 个 Task,提请审阅、通过后再合并回 `main`。每次 `git commit`/`git add` 前先跑 `git branch --show-current` 确认当前分支是自己的迁移分支——如果这个仓库检出目录被多个 agent 共享并行工作,这一步不能省;开工前也确认 `git status` 干净或只包含自己预期要改的文件,不要在别的 agent 尚未提交的改动上继续叠加。
- **`geometry` 里只迁移 32 个具名的基础 accessor,`tree_row_h`/`tree_chrome_top_px`/`tree_chrome_bottom_px` 三个函数、它们的 9 处调用点(`app.rs` 6 处 + `extensions/files.rs` 3 处)一律不动。** 所有 geometry 相关的 sed 规则只列这 32 个名字,不包含这 3 个,天然跳过——不要因为"顺手"额外加规则处理它们。
- **font/geometry 两种引用形态不能在同一文件里混用同一条规则跑两遍中间不换前缀。** `app.rs`/`workspace.rs`/`extensions/usage.rs` 这 3 个 geometry 混合形态文件,必须先套形态 A(`crate::theme::geometry::`)规则、再套形态 B(`theme::geometry::`)规则,顺序不能反——先套 B 会把 `crate::theme::geometry::NAME(` 里的 `theme::geometry::NAME(` 子串提前命中,产出错误的双重替换。font 没有混合形态文件,每个文件只套一条规则。
- **Task 23(`geometry.rs` 自身瘦身)、Task 24(删除 `font.rs` + 最终验证)必须在其余 22 个调用方 Task 全部完成之后才能做。** 中间状态下 `font.rs` 全部 7 个函数、`geometry.rs` 全部 35 个函数都要留着,否则未迁移的调用方编译不过。
- **不改动 `theme::homespace_font.rs`/`theme::homespace_color.rs`/`theme::region.rs`/`theme::terminal_font.rs`。** 这次只碰 `font.rs`(删除)和 `geometry.rs`(瘦身),不涉及独立配色/字号系统或未迁移的 `region`/`terminal_font`。

## 替换规则(所有 Task 通用,逐字对照 spec)

**font 形态 A**(`crate::theme::font::NAME(` 完整路径,`database.rs`/`sftp.rs`/`geometry.rs` 内部引用):

```bash
sed -i '' \
  -e 's/crate::theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/crate::theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/crate::theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/crate::theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/crate::theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/crate::theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/crate::theme::font::title(/byteui::theme::font::title(/g' \
  <file>
```

**font 形态 B**(`theme::font::NAME(`,经 `use crate::theme;`,其余 15 个文件):

```bash
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

**geometry 形态 A**(`crate::theme::geometry::NAME(` 完整路径,32 个名字,不含 `tree_*`):

```bash
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
```

**geometry 形态 B**(`theme::geometry::NAME(`,经 `use crate::theme;`,同 32 条去掉 `crate::` 前缀):

```bash
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

---

### Task 1: 迁移 `app.rs`(font 形态 B 11 处 + geometry 形态 A 19 处 + geometry 形态 B 210 处,含 6 处 `tree_chrome_*` 天然跳过)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `byteui::theme::font::*`、`byteui::theme::geometry::*`
- Produces: `app.rs` 里 32 个基础 geometry accessor 和全部 7 个 font accessor 不再引用本地模块;`tree_chrome_top_px`/`tree_chrome_bottom_px` 6 处调用仍指向本地 `theme::geometry`,不受影响

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/app.rs
```

- [ ] **Step 2: 应用 geometry 形态 A 规则(必须先跑,再跑形态 B)**

```bash
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
  crates/dozer-app/src/app.rs
```

- [ ] **Step 3: 应用 geometry 形态 B 规则**

```bash
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
  crates/dozer-app/src/app.rs
```

- [ ] **Step 4: 确认 `tree_chrome_top_px`/`tree_chrome_bottom_px` 6 处调用未被误改**

Run: `grep -n "theme::geometry::tree_chrome_top_px\|theme::geometry::tree_chrome_bottom_px" crates/dozer-app/src/app.rs`
Expected: 6 行输出,全部还是 `theme::geometry::tree_chrome_top_px(`/`tree_chrome_bottom_px(`(不带 `byteui::` 前缀)

- [ ] **Step 5: 确认无残留**

Run: `grep -n "theme::font::\|theme::geometry::" crates/dozer-app/src/app.rs | grep -v byteui | grep -v tree_chrome`
Expected: 无输出

- [ ] **Step 6: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 7: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "refactor(dozer-app): app.rs 迁移 theme::font + theme::geometry 到 byteui"
```

---

### Task 2: 迁移 `workspace.rs`(font 形态 B 40 处 + geometry 形态 A 1 处 + geometry 形态 B 5 处)

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `byteui::theme::font::*`、`byteui::theme::geometry::*`

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/workspace.rs
```

- [ ] **Step 2: 应用 geometry 形态 A 规则(必须先跑)**

```bash
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
  crates/dozer-app/src/workspace.rs
```

- [ ] **Step 3: 应用 geometry 形态 B 规则**

```bash
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
  crates/dozer-app/src/workspace.rs
```

- [ ] **Step 4: 确认无残留**

Run: `grep -n "theme::font::\|theme::geometry::" crates/dozer-app/src/workspace.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 5: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 6: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(dozer-app): workspace.rs 迁移 theme::font + theme::geometry 到 byteui"
```

---

### Task 3: 迁移 `extensions/todo.rs`(font 形态 B,29 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/extensions/todo.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::font::" crates/dozer-app/src/extensions/todo.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "refactor(dozer-app): todo.rs 迁移 theme::font 到 byteui"
```

---

### Task 4: 迁移 `extensions/database.rs`(font 形态 A,24 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`

- [ ] **Step 1: 应用 font 形态 A 规则**

```bash
sed -i '' \
  -e 's/crate::theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/crate::theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/crate::theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/crate::theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/crate::theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/crate::theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/crate::theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/extensions/database.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::theme::font::" crates/dozer-app/src/extensions/database.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/database.rs
git commit -m "refactor(dozer-app): database.rs 迁移 theme::font 到 byteui"
```

---

### Task 5: 迁移 `extensions/git_log.rs`(font 形态 B,23 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs`

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/extensions/git_log.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::font::" crates/dozer-app/src/extensions/git_log.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/git_log.rs
git commit -m "refactor(dozer-app): git_log.rs 迁移 theme::font 到 byteui"
```

---

### Task 6: 迁移 `extensions/ssh.rs`(font 形态 B,19 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/extensions/ssh.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::font::" crates/dozer-app/src/extensions/ssh.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "refactor(dozer-app): ssh.rs 迁移 theme::font 到 byteui"
```

---

### Task 7: 迁移 `extensions/project.rs`(font 形态 B,17 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/extensions/project.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::font::" crates/dozer-app/src/extensions/project.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/project.rs
git commit -m "refactor(dozer-app): project.rs 迁移 theme::font 到 byteui"
```

---

### Task 8: 迁移 `extensions/files.rs`(font 形态 B 15 处 + geometry 形态 A 5 处,含 3 处 `tree_row_h` 天然跳过)

**Files:**
- Modify: `crates/dozer-app/src/extensions/files.rs`

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/extensions/files.rs
```

- [ ] **Step 2: 应用 geometry 形态 A 规则**

```bash
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
  crates/dozer-app/src/extensions/files.rs
```

- [ ] **Step 3: 确认 3 处 `tree_row_h` 未被误改**

Run: `grep -n "crate::theme::geometry::tree_row_h" crates/dozer-app/src/extensions/files.rs`
Expected: 3 行输出,不带 `byteui::` 前缀

- [ ] **Step 4: 确认无残留**

Run: `grep -n "theme::font::\|theme::geometry::" crates/dozer-app/src/extensions/files.rs | grep -v byteui | grep -v tree_row_h`
Expected: 无输出

- [ ] **Step 5: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 6: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/files.rs
git commit -m "refactor(dozer-app): files.rs 迁移 theme::font + theme::geometry 到 byteui"
```

---

### Task 9: 迁移 `extensions/acceptance.rs`(font 形态 B,13 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/acceptance.rs`

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/extensions/acceptance.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::font::" crates/dozer-app/src/extensions/acceptance.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/acceptance.rs
git commit -m "refactor(dozer-app): acceptance.rs 迁移 theme::font 到 byteui"
```

---

### Task 10: 迁移 `preview.rs`(font 形态 B 1 处 + geometry 形态 B 14 处)

**Files:**
- Modify: `crates/dozer-app/src/preview.rs`

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/preview.rs
```

- [ ] **Step 2: 应用 geometry 形态 B 规则**

```bash
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
  crates/dozer-app/src/preview.rs
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "theme::font::\|theme::geometry::" crates/dozer-app/src/preview.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/preview.rs
git commit -m "refactor(dozer-app): preview.rs 迁移 theme::font + theme::geometry 到 byteui"
```

---

### Task 11: 迁移 `extensions/usage.rs`(font 形态 B 10 处 + geometry 形态 A 1 处 + geometry 形态 B 1 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/usage.rs`

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/extensions/usage.rs
```

- [ ] **Step 2: 应用 geometry 形态 A 规则(必须先跑)**

```bash
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
  crates/dozer-app/src/extensions/usage.rs
```

- [ ] **Step 3: 应用 geometry 形态 B 规则**

```bash
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
  crates/dozer-app/src/extensions/usage.rs
```

- [ ] **Step 4: 确认无残留**

Run: `grep -n "theme::font::\|theme::geometry::" crates/dozer-app/src/extensions/usage.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 5: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 6: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/usage.rs
git commit -m "refactor(dozer-app): usage.rs 迁移 theme::font + theme::geometry 到 byteui"
```

---

### Task 12: 迁移 `extensions/search.rs`(font 形态 B,9 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/search.rs`

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/extensions/search.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::font::" crates/dozer-app/src/extensions/search.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/search.rs
git commit -m "refactor(dozer-app): search.rs 迁移 theme::font 到 byteui"
```

---

### Task 13: 迁移 `main.rs`(geometry 形态 A,12 处)

**Files:**
- Modify: `crates/dozer-app/src/main.rs`

- [ ] **Step 1: 应用 geometry 形态 A 规则**

```bash
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
  crates/dozer-app/src/main.rs
```

**注意**:这个文件里实际全是 `theme::geometry::NAME(`(经 `use crate::
theme;`,没有 `crate::` 前缀)——上面这条形态 A 规则(带 `crate::` 前缀)
不会匹配到任何东西。跑完这条后再跑下面的形态 B 规则。

- [ ] **Step 2: 应用 geometry 形态 B 规则**

```bash
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
  crates/dozer-app/src/main.rs
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "theme::geometry::" crates/dozer-app/src/main.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/main.rs
git commit -m "refactor(dozer-app): main.rs 迁移 theme::geometry 到 byteui"
```

---

### Task 14: 迁移 `menu.rs`(font 形态 B 4 处 + geometry 形态 A 5 处)

**Files:**
- Modify: `crates/dozer-app/src/menu.rs`

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/menu.rs
```

- [ ] **Step 2: 应用 geometry 形态 A 规则**

```bash
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
  crates/dozer-app/src/menu.rs
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "theme::font::\|theme::geometry::" crates/dozer-app/src/menu.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/menu.rs
git commit -m "refactor(dozer-app): menu.rs 迁移 theme::font + theme::geometry 到 byteui"
```

---

### Task 15: 迁移 `homespace.rs`(geometry 形态 B,5 处)

**Files:**
- Modify: `crates/dozer-app/src/homespace.rs`

- [ ] **Step 1: 应用 geometry 形态 B 规则**

```bash
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
  crates/dozer-app/src/homespace.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::geometry::" crates/dozer-app/src/homespace.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/homespace.rs
git commit -m "refactor(dozer-app): homespace.rs 迁移 theme::geometry 到 byteui"
```

---

### Task 16: 迁移 `extensions/footbar.rs`(font 形态 B 7 处 + geometry 形态 B 2 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/footbar.rs`

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/extensions/footbar.rs
```

- [ ] **Step 2: 应用 geometry 形态 B 规则**

```bash
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
  crates/dozer-app/src/extensions/footbar.rs
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "theme::font::\|theme::geometry::" crates/dozer-app/src/extensions/footbar.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/footbar.rs
git commit -m "refactor(dozer-app): footbar.rs 迁移 theme::font + theme::geometry 到 byteui"
```

---

### Task 17: 迁移 `extensions/browser.rs`(font 形态 B 7 处 + geometry 形态 B 4 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/browser.rs`

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/extensions/browser.rs
```

- [ ] **Step 2: 应用 geometry 形态 B 规则**

```bash
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
  crates/dozer-app/src/extensions/browser.rs
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "theme::font::\|theme::geometry::" crates/dozer-app/src/extensions/browser.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/browser.rs
git commit -m "refactor(dozer-app): browser.rs 迁移 theme::font + theme::geometry 到 byteui"
```

---

### Task 18: 迁移 `extensions/ssh/sftp.rs`(font 形态 A 4 处 + geometry 形态 A 1 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh/sftp.rs`

- [ ] **Step 1: 应用 font 形态 A 规则**

```bash
sed -i '' \
  -e 's/crate::theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/crate::theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/crate::theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/crate::theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/crate::theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/crate::theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/crate::theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/extensions/ssh/sftp.rs
```

- [ ] **Step 2: 应用 geometry 形态 A 规则**

```bash
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
  crates/dozer-app/src/extensions/ssh/sftp.rs
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "crate::theme::font::\|crate::theme::geometry::" crates/dozer-app/src/extensions/ssh/sftp.rs`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/ssh/sftp.rs
git commit -m "refactor(dozer-app): sftp.rs 迁移 theme::font + theme::geometry 到 byteui"
```

---

### Task 19: 迁移 `diff_render.rs`(font 形态 B,1 处)

**Files:**
- Modify: `crates/dozer-app/src/diff_render.rs`

- [ ] **Step 1: 应用 font 形态 B 规则**

```bash
sed -i '' \
  -e 's/theme::font::dot_sm(/byteui::theme::font::dot_sm(/g' \
  -e 's/theme::font::caption_sm(/byteui::theme::font::caption_sm(/g' \
  -e 's/theme::font::caption(/byteui::theme::font::caption(/g' \
  -e 's/theme::font::label(/byteui::theme::font::label(/g' \
  -e 's/theme::font::body(/byteui::theme::font::body(/g' \
  -e 's/theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/theme::font::title(/byteui::theme::font::title(/g' \
  crates/dozer-app/src/diff_render.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::font::" crates/dozer-app/src/diff_render.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/diff_render.rs
git commit -m "refactor(dozer-app): diff_render.rs 迁移 theme::font 到 byteui"
```

---

### Task 20: 迁移 `layout.rs`(geometry 形态 A,3 处)

**Files:**
- Modify: `crates/dozer-app/src/layout.rs`

- [ ] **Step 1: 应用 geometry 形态 A 规则**

```bash
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
  crates/dozer-app/src/layout.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::theme::geometry::" crates/dozer-app/src/layout.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/layout.rs
git commit -m "refactor(dozer-app): layout.rs 迁移 theme::geometry 到 byteui"
```

---

### Task 21: 迁移 `term_view.rs`(geometry 形态 B,2 处)

**Files:**
- Modify: `crates/dozer-app/src/term_view.rs`

- [ ] **Step 1: 应用 geometry 形态 B 规则**

```bash
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
  crates/dozer-app/src/term_view.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::geometry::" crates/dozer-app/src/term_view.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/term_view.rs
git commit -m "refactor(dozer-app): term_view.rs 迁移 theme::geometry 到 byteui"
```

---

### Task 22: 迁移 `panel_layouts.rs`(geometry 形态 A,2 处)

**Files:**
- Modify: `crates/dozer-app/src/panel_layouts.rs`

- [ ] **Step 1: 应用 geometry 形态 A 规则**

```bash
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
  crates/dozer-app/src/panel_layouts.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::theme::geometry::" crates/dozer-app/src/panel_layouts.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/panel_layouts.rs
git commit -m "refactor(dozer-app): panel_layouts.rs 迁移 theme::geometry 到 byteui"
```

---

### Task 23: `geometry.rs` 自身瘦身(删 32 函数 + JSON 脚手架 + 3 测试,留 3 个 `tree_*` 函数)

**Files:**
- Modify: `crates/dozer-app/src/theme/geometry.rs`

**Interfaces:**
- Consumes: `byteui::theme::font::{subtitle, body}`(`tree_chrome_top_px`
  内部用)、`byteui::theme::icon_size::row`(migration #3 已完成,`tree_
  chrome_top_px`/`tree_chrome_bottom_px` 内部已经指向这个)
- Produces: `tree_row_h() -> f32`、`tree_chrome_top_px() -> f32`、
  `tree_chrome_bottom_px() -> f32` 三个函数继续留在
  `crate::theme::geometry`,签名和行为不变,供 `app.rs`/`extensions/
  files.rs` 继续调用

**这个 Task 必须在 Task 1-22 全部完成之后才能做**——中间状态下 32 个
accessor 还要留着给未迁移的调用方用。

- [ ] **Step 1: 应用 font 形态 A 规则,处理 geometry.rs 内部对 font 的 2 处引用**

```bash
sed -i '' \
  -e 's/crate::theme::font::subtitle(/byteui::theme::font::subtitle(/g' \
  -e 's/crate::theme::font::body(/byteui::theme::font::body(/g' \
  crates/dozer-app/src/theme/geometry.rs
```

- [ ] **Step 2: 删除文件头的历史遗留路径注释**

`crates/dozer-app/src/theme/geometry.rs` 第 1 行删除:

```rust
// crates/dozer-app/src/workspace_geometry.rs
```

- [ ] **Step 3: 替换文件头文档注释**

把原来描述"外壳布局的几何常量……解析 workspace.json 的 geometry 节点"
那一整段(`//!` 开头的多行注释)替换成:

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

- [ ] **Step 4: 删除 `use` 语句、JSON 脚手架、32 个 accessor**

删除以下内容(如果 migration #3 已经清过 `use super::icon_size;`,这一行
不会存在,跳过即可):

```rust
use super::icon_size;
use serde::Deserialize;
use std::sync::LazyLock;

const RAW: &str = include_str!("../../assets/theme/workspace.json");

#[derive(Deserialize)]
struct Geometry {
    // ... 30 个字段
}

#[derive(Deserialize)]
struct RawWorkspaceFile {
    geometry: Geometry,
}

fn load(raw: &str) -> Geometry {
    // ...
}

static GEOMETRY: LazyLock<Geometry> = LazyLock::new(|| load(RAW));
```

以及从 `pub fn icon_rail_width()` 开始到 `pub fn h0_sidebar_width()`
结束的全部 32 个 `pub fn`(保留 `pub fn tree_row_h()` 及其后的
`tree_chrome_top_px`/`tree_chrome_bottom_px` 三个函数不动)。

- [ ] **Step 5: 删除测试模块里 3 个过时测试**

在 `#[cfg(test)] mod tests` 里删除:

```rust
#[test]
fn values_match_pre_migration_literals() {
    // ...
}
```

```rust
#[test]
fn min_window_width_is_derived_not_duplicated_in_json() {
    // ...
}
```

```rust
#[test]
#[should_panic(expected = "workspace.json 格式错误")]
fn malformed_json_panics() {
    // ...
}
```

保留 `tree_geometry_matches_composition` 不动。

- [ ] **Step 6: 确认文件瘦身后的结构**

Run: `grep -n "^pub fn\|^fn \|^struct \|^static \|^const " crates/dozer-app/src/theme/geometry.rs`
Expected: 只剩 `pub fn tree_row_h`、`pub fn tree_chrome_top_px`、
`pub fn tree_chrome_bottom_px` 三个函数,没有 `struct Geometry`、
`static GEOMETRY`、`const RAW`、`fn load`

Run: `grep -c "fn " crates/dozer-app/src/theme/geometry.rs`
Expected: `4`(3 个 `pub fn` + 测试模块里 `tree_geometry_matches_composition`
这一个 `#[test] fn`)

- [ ] **Step 7: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 8: 测试验证**

Run: `cargo test -p dozer-app --bin dozer theme::geometry`
Expected: `tree_geometry_matches_composition` 通过,`values_match_pre_
migration_literals`/`min_window_width_is_derived_not_duplicated_in_json`/
`malformed_json_panics` 已不存在(不会出现在测试列表里)

- [ ] **Step 9: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/theme/geometry.rs
git commit -m "refactor(dozer-app): geometry.rs 瘦身,32 个基础 accessor 迁移到 byteui,只留 3 个 tree_* 函数"
```

---

### Task 24: 删除 `font.rs` + 最终验证

**Files:**
- Delete: `crates/dozer-app/src/theme/font.rs`
- Modify: `crates/dozer-app/src/theme.rs`

**Interfaces:**
- Consumes: Task 1-23 已完成(全仓库零处引用本地 `theme::font` 模块,
  `theme::geometry` 只剩 3 个 `tree_*` 函数)
- Produces: `dozer-app` 完全依赖 `byteui::theme::font`/
  `byteui::theme::geometry`(除 3 个 `tree_*` 函数外),不再有重复源码

- [ ] **Step 1: 确认全仓库已无残留引用(收尾前的最后一道保险)**

Run: `grep -rn "theme::font::[a-z]" crates/dozer-app/src --include="*.rs" | grep -v byteui | grep -v "crates/dozer-app/src/theme/font.rs"`
Expected: 无输出(如果有输出,说明 Task 1-19 里漏了某个调用点,先回去
补上,不要继续本 Task)

Run: `grep -rn "theme::geometry::[a-z]" crates/dozer-app/src --include="*.rs" | grep -v byteui | grep -v tree_row_h | grep -v tree_chrome_top_px | grep -v tree_chrome_bottom_px`
Expected: 无输出(排除掉 3 个 `tree_*` 函数自身调用点后应该全干净)

- [ ] **Step 2: 删除本地重复文件**

```bash
rm crates/dozer-app/src/theme/font.rs
```

- [ ] **Step 3: 清理 `theme.rs` 里的 `pub mod font;`**

`crates/dozer-app/src/theme.rs` 里删除:

```rust
pub mod font;
```

- [ ] **Step 4: 全量编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer`
Expected: 编译成功;测试结果比迁移前基线(484 passed / 2 failed)少 3 个
通过(`geometry.rs` 删掉的 3 个测试),即 481 passed / 2 failed——这个
数字变化是预期的,不是新增失败,失败数应该还是那两个已知的、和本次改动
无关的 terminal grid 尺寸测试。

- [ ] **Step 5: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 无警告、无格式差异

- [ ] **Step 6: 独立临时二进制专项核对**

构建一个独立命名的临时二进制(不要用会撞到用户正在跑的正式 `/Applications/
Dozer AI Coder.app` 或其他调试会话进程名的路径),启动后核对:

- **文字/几何视觉**:四栏骨架尺寸(rail 宽、分隔线宽、窗口最小尺寸)、
  顶栏/状态栏/footbar 高度、右键菜单尺寸、tab 栏尺寸与翻页箭头、滚动条
  宽度、各面板标题/正文/次要文字字号(`todo.rs`/`git_log.rs`/`ssh.rs`
  这几个用字号最密集的面板)。
- **文件树拖拽命中专项**:在文件树里从外部(Finder)拖一个文件到列表
  中间某一行、开头一行、末尾一行,确认插入位置和视觉上瞄准的行一致,
  没有偏差(`tree_row_h`/`tree_chrome_top_px`/`tree_chrome_bottom_px`
  内部引用的 `font::subtitle`/`body` 换了来源,这是本次唯一有真实回归
  风险的地方)。

完成后关闭该临时实例、删除临时二进制,不留后台进程。

- [ ] **Step 7: Commit**

```bash
git branch --show-current
git add -A
git commit -m "refactor(dozer-app): 删除 theme::font 本地重复实现,改用 byteui"
```

---

## 完工验收

1. `git log --oneline` 确认全部 24 个 commit 都在当前分支上,没有漂到 `main`。
2. `grep -rn "theme::font::[a-z]" crates/dozer-app/src | grep -v byteui` 全仓库零匹配(排除已删除的 `theme/font.rs`)。
3. `grep -rn "theme::geometry::[a-z]" crates/dozer-app/src | grep -v byteui | grep -v tree_row_h | grep -v tree_chrome` 全仓库零匹配。
4. `crates/dozer-app/src/theme/font.rs` 已删除;`crates/dozer-app/src/theme/geometry.rs` 只剩 3 个 `tree_*` 函数。
5. 提请审阅。这是 4 个迁移子项目里的最后一个,审阅通过合并后,`byteui` 迁移系列(`interaction`/`color`/`icon_size`/`font`+`geometry`)全部完工。
