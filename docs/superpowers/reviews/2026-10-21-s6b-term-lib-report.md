# S6b 终端控件库 `dozer-panel-term` 实施报告

日期：2026-10-21。

计划：[2026-10-18-agent-s6b-term-lib.md](../plans/2026-10-18-agent-s6b-term-lib.md)。
规格：[2026-10-18-agent-panel-s6-design.md](../specs/2026-10-18-agent-panel-s6-design.md) §6.5、§7 S6b。

## 结论

Dozer 侧 S6b 完成：终端模型与 iced 自绘视图搬进共享库 `crates/dozer-panel-term`，Dozer 改为依赖它，用户可见行为不变。分支 `feat/panel-term` **基于 `feat/product-id` 的顶端 `72a3544c`**，未合并到 `main`、未打 tag、未推送。Digger 本期不消费（按 S6 规格 §0 第 2、9 条，tag 留到整条 Dozer 侧链路验收之后）。

本分支共 3 个提交：

```
1f960cb1 refactor(term): 搬终端视图到 dozer-panel-term,注入 TermStyle/TermEvent
3b869318 test(term): 钉死模型的退化尺寸与脏输入现状
1d123afa refactor(term): 新建 dozer-panel-term,搬终端模型;加渲染黄金测试
```

## 搬走了什么

| 从 `dozer-app/src/term/` | 到 `dozer-panel-term/src/` | 改动 |
|---|---|---|
| `term_model.rs`（31 测试） | `model.rs` | 只改 `use` 与 `default_fg_rgb` 的可见性（`pub(crate)` → `pub`）；测试原样搬走 |
| `term_view.rs`（16 测试） | `view.rs` | 事件载荷改 `TermEvent`，字体/字号/行高改注入 `TermStyle`，`canvas::Program` 的 `Message` 泛型化为 `M`，目标泛型化为 `T`；绘制数值公式逐字保持 |
| — | `style.rs` | 新增 `TermStyle`（`font` / `font_size` / `line_height_factor`）及 `scaled_font_size` / `cell_width` / `line_height_px` / `grid_size` |
| — | `lib.rs` | 库入口与边界说明 |

`dozer-app/src/term/term_model.rs` 与 `term_view.rs` 变成薄适配层：前者 `pub use dozer_panel_term::model::*`；后者用 `assets::fonts::code_font()` + `theme::terminal_font` 构造 `TermStyle`，把 `TermEvent` 映射回 `Message::{TermInput, TermScroll, TermSelStart, TermSelUpdate}`，并保持 `view` / `grid_size` 的原签名——`app.rs` / `app/view.rs` / `terminal.rs` 的调用点一行未改（含 `grid_size(...) == (95, 41)` 的断言）。

### 为什么 `terminal.rs` 与 `TermTarget` 留在 `dozer-app`

`terminal.rs`（页签栏与右键菜单）依赖 `extensions::agent_context`、`chrome::tab_widget` 与 i18n，是产品面板层，按计划属于 S6c；`TermTarget`（`Shared` / `SshPanel`）是 Dozer 自己的消息路由类型，不是终端控件的一部分。库不内嵌字体：全局字体注册（`assets::fonts::load_embedded_fonts` / `sanitize_font_db`）仍是应用级初始化，字体经 `TermStyle.font` 传入。

## 依赖闭包与门禁

- `cargo tree -p dozer-panel-term | grep -E "dozer-(app|client|core)|dozerd|wry|portable"`：**无输出**。
- `dozer-panel-term` 依赖：`alacritty_terminal`（workspace）、`byteui`（tag `v0.4.5`）、`unicode-width 0.2`、`iced_widget 0.14`（canvas）、`iced_renderer 0.14`。
- 已按 `dozer-panel-todo` / `dozer-panel-usage` 的方式登记新库：
  - `scripts/i18n/scan_sources.py`：加入 `crates/dozer-panel-term/src` 扫描根，owner 暂归 `dozer.agent`；
  - `scripts/i18n/build_inventory.py`：文件任务归到 09；**并把 `crates/dozer-panel-term/src/model.rs` 加进 `EXCLUDED_FILES`**（审核 B1：旧路径 `term_model.rs` 本来就在排除表里，搬家后新路径没跟上，`#[must_use]` 提示被扫成未分类 UI 候选）；
  - `scripts/i18n/check_migration.py`：加入默认源根。
  - 库当前**没有可本地化文案**，因此没有 `locales/`、也没有造空文案。**更正：** 本报告初稿写的"`check_migration.py` 仍为 `unresolved=0`"在 B1 修复之前不成立（当时是 `unresolved=1`，退出码 1）；修复之后实测 `python3 scripts/i18n/check_migration.py` 退出码 0、`inventory: total=2677 unresolved=0`。`check_migration.py` 是重跑 `build_inventory.build()` 判断的，**不读**已提交的 `docs/dozer-v2/i18n/inventory.json`，该文件含后续迁移任务写回的记录，**不要用 `build_inventory.py` 覆盖式重新生成它**（实测会产生 2.3 万行无关 diff 并清零 `migrated`）。
- `python3 scripts/audit/check_panel_boundary.py`：ok（`dozer-app` 内引用数未上升）。

## 测试总数与黄金测试

基线（`feat/product-id` 顶端 `72a3544c`，本 worktree 实测）：

- `cargo test -p dozer-app`：1333 passed / 2 ignored；
- `cargo test -p dozer-app term::`：47 passed（`term_model` 31 + `term_view` 16）；
- `cargo test --workspace`：**3352 passed / 4 failed / 4 ignored**。

> 计划里写的基线「3366 通过 / 0 失败 / 4 ignored」在本分支上无法复现：`dozerd --test product_isolation` 的 4 条测试在基线提交上就失败，原因是测试引用的 `crates/dozerd/examples/product_daemon.rs` 在该提交上并不存在（`cargo build -p dozerd --example product_daemon` 报 `no example target named product_daemon`）。这与 S6b 无关，未在本期内修。

抽库后：

- `cargo test -p dozer-app`：**1286 passed / 2 ignored**（1333 − 搬走的 47）；
- `cargo test -p dozer-panel-term`：**60 passed / 1 ignored**；
- `cargo test --workspace`：**3365 passed / 4 failed / 5 ignored**。

即相对基线 **+13 passed（黄金 1 + Task 2 的 6 + Task 3 的 6）与 +1 ignored（Task 2 的零尺寸既有缺陷）**；两个 crate 合计 3365 + 4 + 5 = 3374，相对基线 3360 正好 +14，符合"总数守恒 + 新增"。

其它门禁：

- `cargo build --workspace`：通过；
- `cargo clippy --workspace --all-targets`：退出码 0（无 error；既有 warning 与本期无关）；
- `cargo fmt --check`：**失败，但只涉及基线就未格式化干净的文件**——`crates/dozer-app/src/app/view.rs`、`crates/dozer-app/src/extensions/files/{state,tree}.rs`、`crates/dozer-panel-files/{dialogs,state,tests,tree,update,view}.rs`。本期新增/修改的 `dozer-panel-term/src/*` 与两个薄适配文件均通过 rustfmt，未改动上述既有文件。

**黄金测试结论：通过。** `crates/dozer-app/tests/fixtures/term_golden.txt`（490 行，41 段语料 × Dark/Light 两种配色）在搬任何代码之前生成并提交；抽模型、抽视图后测试原样通过，**黄金文件自生成后字节未变**（`git diff 72a3544c..HEAD` 里只有新增，没有后续修改）。覆盖 SGR 前景/背景/反相/粗体/下划线、256 色与真彩色、光标移动/保存恢复、清屏、备用屏、滚动区域、制表符、反相空格假光标、CJK/emoji/组合附加符、行尾折行双宽字符，并逐格序列化 `(char, fg, bg, flags)`、光标、各模式标志、网格尺寸与回滚行数。

## Task 2 中被 `#[ignore]` 的既有缺陷

唯一一条：`model::tests::new_and_resize_to_zero_or_one_cell_do_not_panic`，忽略原因 `既有缺陷,按现状保持,见 S6b 报告`。

现状（`alacritty_terminal 0.26` 的网格不支持 0 行/0 列）：

- `rows == 0`：`new(0,0)` / `new(1,0)` 的快照可读，但 `feed` 在 `grid/storage.rs:221` 的 `compute_index` 断言（`requested.0 < visible_lines`）处 panic；
- `cols == 0`：`new(0,1)` 以及 `resize(0, *)` 之后连 `visible_lines()` 都会 panic（末列索引下溢）。

按计划未在抽库时顺手修；`1×1` 网格已由 `one_by_one_grid_does_not_panic` 覆盖且通过。模型层其余退化/脏输入钉死测试（截断 CSI/OSC、非法 UTF-8、超长 CSI 参数、10 万字节单行、20 万行回滚上限、选区拖出网格、缩小网格后选区）全部通过。

## 目测验收清单（推迟，原样保留）

用户 2026-10-20 决定：整条 Dozer 侧链路（S6a–S6c、S6d-2 Part A、S6e-1）全部完成后一并验收，本期不阻塞。最终验收时请逐项确认：

打开 Agent 面板与 SSH 面板里的终端——彩色输出、全屏 TUI(vim / htop)、中文与 emoji、光标闪烁与聚焦/未聚焦样式、滚轮与滚动条拖拽、鼠标选区与复制粘贴、窗口缩放时网格随之调整、Ctrl +/- 缩放、深浅色切换、输入法组字预览下划线,与抽库前一致。

本期的信心来自上述黄金测试与测试总数守恒；两项均已真跑通。
