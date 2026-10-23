# S6b 终端控件库 `dozer-panel-term` Implementation Plan

> **迁移说明(2026-10-20):** 本文原在 Digger 仓库,现移入 Dozer 仓库;文内 `docs/superpowers/...` 路径均相对 **Dozer 仓库根**。写作「Digger(仓库 `../digger`)」的部分,实施仍在 Digger 仓库的 worktree 里进行,但文档不在那里。Digger 自有文档(`docs/adr/*`、`2026-10-14-window-shell-design.md`、`docs/superpowers/notes/`)仍在 Digger 仓库。最新要求与决定见 `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md` §0。


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Dozer 里新建共享库 `dozer-panel-term`,把终端的"模型(`alacritty_terminal` 的 VTE 与网格)"和"视图(iced 自绘 canvas 控件)"从 `dozer-app/src/term/` 搬进去;Dozer 改为依赖它,**用户可见行为不变**。本期**只动 Dozer**,Digger 不消费(S6d 接入)。

**Architecture:** 库只含 `term_model.rs`(868 行,31 个测试,**对 `dozer-app` 零耦合**,仅依赖 `alacritty_terminal` 与 byteui 的配色方案)和 `term_view.rs`(856 行,16 个测试)。`term_view` 对内核有四处耦合,全部改成注入:`crate::app::Message` → 库自己的 `TermEvent`,目标 `TermTarget` 变成泛型 `T`,由产品用 `fn(T, TermEvent) -> M` 映射成自己的消息;`terminal_font::{size, line_height_factor}`(读 `assets/theme/terminal.json`)与 `assets::fonts::code_font()` → 产品传入的 `TermStyle`;`grid_size` 从读全局的自由函数变成 `TermStyle` 的方法。`terminal.rs`(330 行,页签栏与右键菜单,依赖 `extensions::agent_context`、`chrome::tab_widget`、i18n)**留在 `dozer-app`**,属于 S6c 的面板层。

**Tech Stack:** Rust、`alacritty_terminal`、iced 0.14(`canvas`)、byteui、`unicode-width`。

**Spec:** `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md`(§6.5、§7 S6b)。模板:`docs/superpowers/plans/2026-10-18-preview-engine-p1-core.md`(同一套 Dozer worktree、黄金测试与门禁做法)。

## Global Constraints

- **依赖闭包:** `cargo tree -p dozer-panel-term` 里不得出现 `dozer-app`、`dozer-client`、`dozer-core`、`dozerd`、`wry`、`portable-pty`(PTY 在守护进程里,库不碰)。
- **Dozer 用户可见行为不变**:被搬走的测试(`term_model` 31 条、`term_view` 16 条)随代码一起走,**不删、不改期望值**;抽库前后 Rust 测试总数守恒。
- **渲染数值不变**:字号、行高、单元格宽 `0.6em`、颜色、光标、滚动条几何、IME 预编辑下划线的计算结果与改造前一致(黄金测试保护)。
- 只用 git tag 依赖;Dozer 仓库内部 `path = "../.."` 依赖除外。**本期不打 tag、不推送、不合并到 `main`**(叠分支规则见 S6 总设计 §0 第 9 条):Digger 不消费终端库,tag 留到整条 Dozer 侧链路验收之后一并发。
- **主工作区别人未提交的改动不动**:Dozer 主工作区有未提交的 `Cargo.lock`、`dozer-app/Cargo.toml`、`Info.plist` 改动。实现在独立 worktree `../dozer-panel-term`、分支 `feat/panel-term` 里做;**分支从 `feat/product-id`(S6a Part A)的当前顶端切出,不从 `main` 切**(`git worktree add ../dozer-panel-term -b feat/panel-term feat/product-id`)。基线测试数是 S6a 顶端的数(3366 通过 / 0 失败 / 4 ignored),不是 `main` 的数。
- 不引入新的日志依赖。
- `terminal.rs` 不搬(见 Architecture);`TermTarget` 枚举(`Shared` / `SshPanel`)留在 `dozer-app`。
- 全局字体注册(`assets::fonts::load_embedded_fonts`、`sanitize_font_db`)是应用级初始化,**留在 `dozer-app`**;库不内嵌字体,字体由调用方经 `TermStyle.font` 传入。

## Review Focus

1. **渲染一字不差:** 对一份覆盖面广的终端输出语料(SGR 前景/背景/反相/粗体/下划线、256 色与真彩色、光标移动、清屏、备用屏、滚动区域、制表符、反相空格当假光标),搬迁前后 `visible_lines`、`cursor`、各模式标志逐项一致。→ Task 1 黄金测试。
2. **宽字符与组合字符:** CJK 全角、emoji、组合附加符、行尾折行处的双宽字符,单元格对齐与占位不变。→ Task 1 黄金语料。
3. **退化尺寸不 panic:** 终端 pane 宽/高为 0 或负、网格缩到 1×1、`resize` 到 0、窗口拖到极小,`grid_size` 返回 `(0, 0)` 或合法值,模型不 panic、不除零。→ Task 2。
4. **脏输入:** 截断的转义序列、非法 UTF-8、超长单行(10 万字节无换行)、超大回滚,`feed` 不 panic、不无限增长内存,返回的应答字节合法。→ Task 2。
5. **滚动条与选区边界:** 无回滚历史时不画条、不响应拖拽;回滚极大(百万行)时滚动条几何没有 `NaN` / `inf`;选区拖出网格边界时钳在边缘格;缩放网格后选区不越界。→ Task 3。

---

## Part A — Dozer(仓库 `../dozer`,worktree `../dozer-panel-term`,分支 `feat/panel-term`)

### Task 1: 基线、黄金测试、空壳 crate、搬模型

**Files:**
- Create:`crates/dozer-panel-term/Cargo.toml`、`src/lib.rs`、`src/model.rs`(搬自 `term_model.rs`)、`crates/dozer-app/tests/fixtures/term_golden.txt`
- Modify:workspace `Cargo.toml`、`crates/dozer-app/Cargo.toml`、`crates/dozer-app/src/term/mod.rs`、`crates/dozer-app/src/term/term_model.rs`(改为 `pub use dozer_panel_term::model::*` 的转调)
- Test:`crates/dozer-app/src/term/term_model.rs`(黄金测试暂放这里)

**Interfaces:**
- Produces(原样搬自 `term_model.rs`,只改 `use` 与可见性):`model::{Cell, TermSize, TerminalModel, ColorScheme?}`;`TerminalModel` 的全部 `pub fn`(`new`、`feed`、`set_answer_dynamic_color`、`resize`、`scroll_display`、`scroll_to_bottom`、`display_offset`、`grid_dims`、`history_len`、`selection_start`、`selection_update`、`selection_text`、`selection_clear`、`visible_lines`、`cursor`、`cursor_visible`、`app_cursor_mode`、`bracketed_paste`、`mouse_report_mode`、`sgr_mouse`);`default_fg_rgb() -> (u8, u8, u8)`(原 `pub(crate)`,现在 `pub`,Dozer 的预览模块仍经 `crate::term::term_model::default_fg_rgb` 取用)。**配色方案仍读 `byteui::theme::color::current_scheme()`**,与现状一致。
- 测试专用的 `SCHEME_TEST_LOCK` / `lock_scheme()` 随测试一起搬,保持 `#[cfg(test)]`。

- [ ] **Step 1: 记录基线。** worktree 里 `cargo test -p dozer-app 2>&1 | grep "^test result"` 记总通过数;再单独记 `cargo test -p dozer-app term::` 的数量(预计 `term_model` 31 + `term_view` 16 = 47,以实测为准)。
- [ ] **Step 2: 写黄金测试(在搬任何代码之前,在 Dozer 现有代码上)。** 在 `term_model.rs` 测试模块加 `golden_corpus_visible_lines_cursor_and_modes`:对一份固定语料(≥ 40 段,覆盖 Review Focus 1、2 列出的全部类别,每段指定 `cols×rows`)逐段 `TerminalModel::new(...)` → `feed(...)`,把 `visible_lines()` 的每个单元格 `(char, fg, bg, flags 的稳定字符串)`、`cursor()`、`cursor_visible()`、`app_cursor_mode()`、`bracketed_paste()`、`mouse_report_mode()`、`sgr_mouse()`、`grid_dims()`、`history_len()` 序列化成一份期望文本,用 `UPDATE_GOLDEN=1` 生成一次写入 `crates/dozer-app/tests/fixtures/term_golden.txt` 并提交。语料里要包含 Dark 与 Light 两种配色方案各一遍(用 `lock_scheme()` 保护)。**黄金文件在搬迁过程中不得重新生成。**
- [ ] **Step 3: 建 crate。** `Cargo.toml`:`alacritty_terminal.workspace = true`、`byteui`(同其它面板库的 tag)、`unicode-width = "0.2"`、`iced_widget`(`features = ["canvas"]`)、`iced_renderer = "0.14"`;**不**加 `dozer-*`。`cargo build -p dozer-panel-term` 通过,`cargo tree -p dozer-panel-term | grep -E "dozer-|wry|portable"` 无输出。
- [ ] **Step 4: 搬 `term_model.rs` 与它的 31 个测试到 `model.rs`;`dozer-app/src/term/term_model.rs` 改为 `pub use dozer_panel_term::model::*;`**(含 `default_fg_rgb`),原有 `use crate::term::term_model::TerminalModel` 的各处调用点不动。
- [ ] **Step 5: 验证。** `cargo test -p dozer-panel-term -p dozer-app` 总数 = 基线 + 1(黄金测试);黄金测试通过。
- [ ] **Step 6: 提交** `refactor(term): 新建 dozer-panel-term,搬终端模型;加渲染黄金测试`。

### Task 2: 模型的退化与脏输入钉死测试

**Files:**
- Test:`crates/dozer-panel-term/src/model.rs`

**Interfaces:** 无新接口。

- [ ] **Step 1: 写测试**(对现状行为写成测试;若现状不满足,**按现状保持**,把缺陷记入 Task 5 报告交用户裁决,不在本计划里顺手修):
  - `new_and_resize_to_zero_or_one_cell_do_not_panic`(`new(0,0)`、`new(1,1)`、`resize(0,0)`、`resize(1,1)` 之后 `feed` 若干字节,`visible_lines`、`cursor`、`selection_text` 都能调);
  - `feeding_truncated_and_invalid_sequences_does_not_panic`(截断的 `ESC [`、`ESC ] 0;` 无结束符、孤立的 UTF-8 续字节、`0xFF 0xFE`、超长 CSI 参数);
  - `very_long_single_line_does_not_grow_unbounded`(10 万字节无换行,`history_len()` 与 `visible_lines().len()` 有界);
  - `scrollback_is_bounded`(喂入 20 万行,`history_len()` 不超过配置上限,上限取现状值并写进测试注释);
  - `selection_dragged_outside_the_grid_is_clamped`(`selection_start`/`selection_update` 用超出 `cols`/`rows` 的坐标,`selection_text()` 只含网格内文本);
  - `resize_smaller_after_selecting_keeps_selection_in_bounds`。
- [ ] **Step 2: 运行。** `cargo test -p dozer-panel-term model::`。已通过的保留为回归测试;失败的暂标 `#[ignore = "既有缺陷,按现状保持,见 S6b 报告"]` 并记录,**不改模型代码**。
- [ ] **Step 3: 验证。** `cargo test -p dozer-panel-term -p dozer-app` 总数 = 基线 + 1 + 新增(被 `#[ignore]` 的也计入"新增",但需在报告里单列)。
- [ ] **Step 4: 提交。**

### Task 3: 搬视图,注入字体与事件

**Files:**
- Create:`crates/dozer-panel-term/src/view.rs`(搬自 `term_view.rs`)、`src/style.rs`
- Modify:`crates/dozer-app/src/term/term_view.rs`(缩成薄适配)、`crates/dozer-app/src/term/terminal.rs`(`TermTarget` 留在这里)、`crates/dozer-app/src/app/app.rs`(`grid_size` 的 4 处调用与相关测试)、`crates/dozer-app/src/app/view.rs`(约 2250 行的 `term_view::view` 调用点)、`crates/dozer-app/src/app/message.rs`(无需改变量,只在适配层映射)
- Test:`crates/dozer-panel-term/src/view.rs`(随代码搬 16 条)

**Interfaces:**
- Produces:
  - `style::TermStyle { pub font: iced_widget::core::Font, pub font_size: f32, pub line_height_factor: f32 }`,`Clone + Debug`。方法:`scaled_font_size(&self) -> f32`(= `font_size * byteui::theme::icon_size::scale()`,与原 `font_size()` 同式)、`cell_width(&self) -> f32`(`scaled_font_size() * 0.6`)、`line_height_px(&self) -> f32`(`scaled_font_size() * line_height_factor`)、`grid_size(&self, width_px: f32, height_px: f32) -> (usize, usize)`(与原自由函数 `grid_size` 的公式与取整完全一致:向下取整、负值钳到 0)。
  - `view::TermEvent`:`Input(Vec<u8>)`、`Scroll(i32)`、`SelStart { col: usize, row: usize, right_half: bool }`、`SelUpdate { col: usize, row: usize, right_half: bool }`(与原 `Message::{TermInput, TermScroll, TermSelStart, TermSelUpdate}` 的载荷一一对应)。
  - `view::view<'a, T, M>(model: &'a TerminalModel, focused: bool, target: T, preedit: Option<&'a str>, style: &'a TermStyle, map: fn(T, TermEvent) -> M) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>`,约束 `T: Copy + 'a`、`M: Clone + 'a`。
  - Dozer 一侧(`dozer-app/src/term/term_view.rs`):`pub fn terminal_style() -> TermStyle`(`font = assets::fonts::code_font()`、`font_size = theme::terminal_font::size()`、`line_height_factor = theme::terminal_font::line_height_factor()`);`pub fn view<'a>(model, focused, target: TermTarget, preedit) -> Element<'a, Message, ...>` 保持**原签名**,内部转调库并用 `fn map(target, ev) -> Message`(`Input` → `Message::TermInput(target, bytes)` 等四条)映射;`pub fn grid_size(w, h) -> (usize, usize)` 保持原签名,内部 `terminal_style().grid_size(w, h)`,使 `app.rs` 里的调用点与相关测试(例如断言 `(95, 41)` 的那条)**一行不改**。

- [ ] **Step 1: 写失败测试**(库里新增,其余随代码搬):
  - `grid_size_matches_the_legacy_formula`(对 `TermStyle { font_size: 14.0, line_height_factor: 1.0, .. }` 与一组 `(w, h)`,与手算的 `floor(w / (14*0.6*scale))` 一致;`scale` 取 `byteui::theme::icon_size::scale()` 的当前值);
  - `grid_size_of_zero_or_negative_pane_is_zero_by_zero`(`(0.0, 0.0)`、`(-5.0, 100.0)`、`(f32::NAN, 10.0)` 不 panic,NaN 当 0 处理);
  - `scrollbar_geom_has_no_nan_or_inf_for_huge_history`(历史 `1_000_000` 行、`display_offset` 取 `0`、`1_000_000`、中间值,`thumb_top` 与 `offset_at` 的结果都是有限数且在 `[0, history]` 内);
  - `scrollbar_is_hidden_and_inert_without_history`(`history == 0` 时 `visible()` 为假、`offset_at` 恒 0);
  - `event_mapping_is_pure`(把 `TermEvent` 经一个测试用的 `map` 转成测试枚举,验证四种事件各自载荷不丢);
  - `cell_at_clamps_to_the_grid`(坐标为负、超出 `cols`/`rows`、恰在边界时返回钳制后的 `(col, row, right_half)`)。
- [ ] **Step 2: 运行确认失败。** `cargo test -p dozer-panel-term view:: style::`。
- [ ] **Step 3: 搬 `term_view.rs` 与它的 16 个测试到 `view.rs`;实现 `TermStyle` 与 `TermEvent`;把 `font_size()` / `cell_width()` / `line_height_px()` / `cell_font()` 里读全局的位置全部改为读传入的 `style`;`canvas::Program` 的 `Message` 参数化为 `M`,`TermCanvas` 带上 `style` 与 `map`。** 画面相关的所有数值公式(`0.6em`、光标描边、滚动条宽、预编辑下划线)逐字保持。
- [ ] **Step 4: Dozer 侧适配(见 Interfaces 最后一条)。** `app/view.rs` 与 `app/app.rs` 的调用点因签名不变而**不需要改**;若编译器要求改动,说明哪里漏了保持原签名,先修适配层。
- [ ] **Step 5: 验证。** `cargo test -p dozer-panel-term -p dozer-app` 总数守恒(两库之和 = 基线 + 1 + Task 2 新增 + 本任务新增);黄金测试通过且黄金文件字节未变(`git diff --stat crates/dozer-app/tests/fixtures/term_golden.txt` 无输出)。
- [ ] **Step 6: 提交。**

### Task 4: 清理、门禁与验收

- [ ] **Step 1: 清理。** `dozer-app/src/term/` 里只剩 `mod.rs`、`terminal.rs`、以及两个薄转调文件(`term_model.rs`、`term_view.rs`);`grep -rn "pub struct TerminalModel\|fn feed\b\|struct TermCanvas" crates` 只在 `dozer-panel-term` 命中。
- [ ] **Step 2: 依赖闭包与登记。** `cargo tree -p dozer-panel-term | grep -E "dozer-(app|client|core)|dozerd|wry|portable"` **无输出**;在 `scripts/audit/check_panel_boundary.py` 与 `scripts/i18n/*.py` 里按 `dozer-panel-todo` / `dozer-panel-usage` 的做法登记新库(库本身没有可本地化文案,若脚本要求每个库有 `locales/`,按脚本现有约定处理,不要给库造空文案)。
- [ ] **Step 3: 全量。** `cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --check`;总数与基线一致(加新增);黄金文件字节未变。
- [ ] **Step 4: 目测验收清单(推迟,用户 2026-10-20 决定:整条 Dozer 侧链路 S6a–S6c、S6d-2 Part A、S6e-1 全部完成后一并验收,本期不阻塞)。** 把下面这份清单原样写进报告,供最后验收使用:打开 Agent 面板与 SSH 面板里的终端——彩色输出、全屏 TUI(vim / htop)、中文与 emoji、光标闪烁与聚焦/未聚焦样式、滚轮与滚动条拖拽、鼠标选区与复制粘贴、窗口缩放时网格随之调整、Ctrl +/- 缩放、深浅色切换、输入法组字预览下划线,与抽库前一致。**因为不做人工验收,本期的信心完全来自黄金测试与测试总数守恒,这两项必须真跑通,不能略。**
- [ ] **Step 5: 提交并留在 `feat/panel-term` 分支上,不合并到 `main`,不打 tag、不推送。** 报告写明:分支基于哪个提交、搬走了什么、`terminal.rs` 与 `TermTarget` 为什么留下、测试总数、黄金测试结论、Task 2 里被 `#[ignore]` 的既有缺陷清单(若有)。

---

## Self-Review

- **Spec 覆盖:** §6.5 "抽 `term_model` 与 `term_view`、不含 PTY、依赖闭包不含 `dozer-app` / `dozer-client` / `dozer-core`" → Global Constraints、Task 1 Step 3、Task 4 Step 2;§7 S6b "抽库、Dozer 改引用" → Task 1、3、4;G3(终端控件抽成共享库)→ 全部。
- **类型一致:** `TermStyle`、`TermEvent`、`view::view` 在 Task 3 的 Interfaces 里一次定义,Dozer 适配层保持原签名,下游(`app.rs`、`app/view.rs`)不改;`TerminalModel` 的方法清单在 Task 1 列出,Task 2、3 的测试只用其中已列方法。
- **风险点:** ① `TermStyle.scaled_font_size` 里的 `byteui::theme::icon_size::scale()` 是 byteui 的全局值,库继续读它(与改造前一致,也是 Digger 将来用同一套缩放的基础);② `Message::TermSelStart` / `TermSelUpdate` 的字段我只核对了 `target` 之外的 `col/row/right_half` 三项的命名意图,实现者搬代码时若发现还有别的字段,按原载荷补进 `TermEvent`,不要丢;③ `Canvas::new(TermCanvas {..})` 的 `canvas::Program` 泛型化可能引出生命周期问题,若 `fn(T, TermEvent) -> M` 不够用(例如映射需要捕获),先停下报告,不要改成 `Box<dyn Fn>` 之类带分配的方案而不说明;④ Task 2 的退化输入测试对现状行为写成测试,若暴露既有缺陷按现状保持并上报,不在抽库里顺手修;⑤ 字体:Digger 需要自己注册一款等宽字体并构造 `TermStyle`,这是 S6d 的事,本期不涉及。
