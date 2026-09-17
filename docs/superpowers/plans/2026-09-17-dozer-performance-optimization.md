# Dozer 性能优化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地性能分析里的四个高优先项——终端渲染热路径去掉"每帧 6000 次 `RwLock` 读"与"每帧 72KB 全量分配"、文件树目录 git 状态聚合从 O(N×M) 降到 O(M×depth)、daemon 侧 ring buffer 去掉逐字节拷贝冗余。

**Architecture:** 四处分属四个文件域,互不重叠,除 Task 2 依赖 Task 1 外可独立并行。Task 1/2 改 `term_model.rs`(颜色解析下传 + `Rc` 脏标记缓存);Task 3 改 `delivery.rs` + `files/*`(预聚合目录状态);Task 4 改 `dozerd/src/ring.rs`(分块存储)。全部不改可观察行为、不改 UDS 协议面、不新增依赖。

**Tech Stack:** Rust workspace;不新增依赖。

## Global Constraints

- **在独立分支上开发**:建分支 `feature/perf-terminal-and-git-status`(或对应 worktree),完成后提请审阅,通过再合并回 `main`。
- **这个计划基于当前 `main`(commit `0301e72`)分析**。工作目录被多个并行会话共享,开工前用本文档的 `grep -n` 模式核对实际行号。
- **不改变任何可观察行为**——终端颜色/内容、目录着色、attach 快照字节内容全部与改动前一致。
- 每个任务结束都要 `cargo build -p dozer-app --bin dozer && cargo build -p dozerd && cargo test -p dozer-app --bin dozer && cargo test -p dozerd && cargo fmt -- --check` 干净通过;`cargo test -p dozer-app --bin dozer` 数字与开工前一致(开工前先跑一次记下基线)。
- 设计文档:`docs/superpowers/specs/2026-09-17-dozer-performance-optimization-design.md`,有疑问以它为准。

---

### Task 1: 终端逐格颜色查询去锁

**Files:**
- Modify: `crates/dozer-app/src/term/term_model.rs`
- Modify: `crates/dozer-app/src/term/term_view.rs`

**Interfaces:**
- Consumes:`byteui::theme::color::{current_scheme, current}`(仍用,只是每帧从数千次降为一次)。
- Produces:`named_color_rgb`/`indexed_to_rgb`/`fg_to_rgb`/`bg_to_rgb` 四个私有 helper 改为接收预解析的 `&[(u8,u8,u8); 16]` + `(u8,u8,u8)` 参数。公开的 `ansi16_color`/`default_fg_rgb`(`pub(crate)`)签名不变。

- [ ] **Step 1: 改四个私有 helper 的签名与内部实现**

定位:

```bash
command grep -n "fn named_color_rgb\|fn indexed_to_rgb\|fn fg_to_rgb\|fn bg_to_rgb" crates/dozer-app/src/term/term_model.rs
```

预期在 `term_model.rs:142/157/175/183` 附近,原文:

```rust
fn named_color_rgb(name: NamedColor) -> Option<(u8, u8, u8)> {
    let idx = name as usize;
    let table = ansi16();
    if idx < table.len() {
        return Some(table[idx]);
    }
    match name {
        NamedColor::Background => None,
        _ => Some(default_fg()),
    }
}

fn indexed_to_rgb(idx: u8) -> (u8, u8, u8) {
    match idx {
        0..=15 => ansi16()[idx as usize],
        16..=231 => {
            let i = idx - 16;
            let r = i / 36;
            let g = (i % 36) / 6;
            let b = i % 6;
            let scale = |c: u8| if c == 0 { 0 } else { 55 + c * 40 };
            (scale(r), scale(g), scale(b))
        }
        232..=255 => {
            let level = 8 + (idx - 232) * 10;
            (level, level, level)
        }
    }
}

fn fg_to_rgb(color: AnsiColor) -> (u8, u8, u8) {
    match color {
        AnsiColor::Named(name) => named_color_rgb(name).unwrap_or_else(default_fg),
        AnsiColor::Spec(rgb) => (rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(idx) => indexed_to_rgb(idx),
    }
}

fn bg_to_rgb(color: AnsiColor) -> Option<(u8, u8, u8)> {
    match color {
        AnsiColor::Named(name) => named_color_rgb(name),
        AnsiColor::Spec(rgb) => Some((rgb.r, rgb.g, rgb.b)),
        AnsiColor::Indexed(idx) => Some(indexed_to_rgb(idx)),
    }
}
```

改成(把 `ansi16()`/`default_fg()` 的解析结果作为参数下传):

```rust
fn named_color_rgb(
    name: NamedColor,
    table: &[(u8, u8, u8); 16],
    dfg: (u8, u8, u8),
) -> Option<(u8, u8, u8)> {
    let idx = name as usize;
    if idx < table.len() {
        return Some(table[idx]);
    }
    match name {
        NamedColor::Background => None,
        _ => Some(dfg),
    }
}

fn indexed_to_rgb(idx: u8, table: &[(u8, u8, u8); 16]) -> (u8, u8, u8) {
    match idx {
        0..=15 => table[idx as usize],
        16..=231 => {
            let i = idx - 16;
            let r = i / 36;
            let g = (i % 36) / 6;
            let b = i % 6;
            let scale = |c: u8| if c == 0 { 0 } else { 55 + c * 40 };
            (scale(r), scale(g), scale(b))
        }
        232..=255 => {
            let level = 8 + (idx - 232) * 10;
            (level, level, level)
        }
    }
}

fn fg_to_rgb(color: AnsiColor, table: &[(u8, u8, u8); 16], dfg: (u8, u8, u8)) -> (u8, u8, u8) {
    match color {
        AnsiColor::Named(name) => named_color_rgb(name, table, dfg).unwrap_or(dfg),
        AnsiColor::Spec(rgb) => (rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(idx) => indexed_to_rgb(idx, table),
    }
}

fn bg_to_rgb(
    color: AnsiColor,
    table: &[(u8, u8, u8); 16],
    dfg: (u8, u8, u8),
) -> Option<(u8, u8, u8)> {
    match color {
        AnsiColor::Named(name) => named_color_rgb(name, table, dfg),
        AnsiColor::Spec(rgb) => Some((rgb.r, rgb.g, rgb.b)),
        AnsiColor::Indexed(idx) => Some(indexed_to_rgb(idx, table)),
    }
}
```

- [ ] **Step 2: `visible_lines` 顶部解析一次,逐格改用下传参数**

定位:

```bash
command grep -n "pub fn visible_lines" crates/dozer-app/src/term/term_model.rs
```

预期在 `term_model.rs:357`。在 `let selection = ...` 之后、`(0..rows)` 之前插入:

```rust
        let table = ansi16();
        let dfg = default_fg();
```

并把闭包内的两处调用:

```rust
                            fg: fg_to_rgb(cell.fg),
                            bg: bg_to_rgb(cell.bg),
```

改成:

```rust
                            fg: fg_to_rgb(cell.fg, table, dfg),
                            bg: bg_to_rgb(cell.bg, table, dfg),
```

- [ ] **Step 3: `draw` 顶部取一次颜色 token**

定位:

```bash
command grep -n "fn draw" crates/dozer-app/src/term/term_view.rs
```

预期在 `term_view.rs:346`。在 `draw` 顶部(拿到 `self.model`/`bounds` 后)取一次 token,替换本函数内散落的 `byteui::theme::color::current()` 调用。现有相关行:

```rust
        let [tbr, tbg, tbb, _] = byteui::theme::color::current().term_bg.into_rgba8();
```

改成先取一次:

```rust
        let tokens = byteui::theme::color::current();
        let [tbr, tbg, tbb, _] = tokens.term_bg.into_rgba8();
```

并把后面光标块(`tokens.cream`)、选区叠加(`Color { a: 0.25, ..tokens.cyan }`)、IME 预览(`tokens.cream`)、滚动条(`tokens.border`/`tokens.tab_active_border`)处对 `byteui::theme::color::current()` 的引用替换为 `tokens` 的对应字段。`cell_font`/`fill_cell_text` 等 helper 不涉及 token,不动。

- [ ] **Step 4: 编译 + 格式检查**

Run: `cargo build -p dozer-app --bin dozer && cargo fmt -- --check`
Expected: 编译成功、无新增 warning、`fmt --check` 无差异。

- [ ] **Step 5: `cargo test`**

Run: `cargo test -p dozer-app --bin dozer`
Expected: 与开工前基线一致(term_model 的颜色/ANSI 相关测试应全部绿——它们走 `visible_lines` 路径,不改行为)。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-app/src/term/term_model.rs crates/dozer-app/src/term/term_view.rs
git commit -m "$(cat <<'EOF'
perf(app): 终端逐格颜色解析去锁,每帧锁读取从数千次降为一次

visible_lines() 逐格调用 fg_to_rgb/bg_to_rgb,各自落到 ansi16()/
default_fg(),而两者都读 byteui::theme::color::current_scheme() 的
RwLock——100x30 网格每帧约 6000 次锁获取。改成 visible_lines() 顶部
解析一次 table+dfg,作为参数下传四个私有 helper;draw() 同样在顶部取
一次颜色 token 替换散落的 current() 调用。纯重构,不改可观察行为。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `visible_lines` 脏标记 + `Rc` 缓存

**Files:**
- Modify: `crates/dozer-app/src/term/term_model.rs`
- Modify: `crates/dozer-app/src/term/term_view.rs`

**Interfaces:**
- Consumes:Task 1 改后的 `fg_to_rgb`/`bg_to_rgb`(带 `table`/`dfg` 参数)。
- Produces:`TerminalModel` 新增私有字段 `cache: RefCell<Option<Rc<Vec<Vec<Cell>>>>>`;`visible_lines` 返回类型 `Vec<Vec<Cell>>` → `Rc<Vec<Vec<Cell>>>`。所有 mutator 增加缓存失效一行。

- [ ] **Step 1: 加缓存字段**

定位:

```bash
command grep -n "pub struct TerminalModel" crates/dozer-app/src/term/term_model.rs
```

预期在 `term_model.rs:242`。在 `responses: PtyResponses,` 之后加:

```rust
    /// 上次 `visible_lines()` 的构建结果;`None` = 脏(需要重建)。用 `Rc`
    /// 是为了 `visible_lines(&self)` 干净时能零拷贝返回一份共享句柄,
    /// 不破坏 `TermCanvas` 对模型的不可变借用(`draw` 只能拿到 `&self`)。
    cache: RefCell<Option<Rc<Vec<Vec<Cell>>>>>,
```

`RefCell`/`Rc` 已在本文件 `use`(`term_model.rs:15-16`),无需新增 import。

- [ ] **Step 2: 构造点初始化**

定位 `TerminalModel::new`(`term_model.rs:251`)的 `Self { term, parser, responses }` 字面量,加 `cache: RefCell::new(None),`。

- [ ] **Step 3: `visible_lines` 改返回 `Rc` 并走缓存**

定位 `term_model.rs:357`,把签名与 body 改成:

```rust
    pub fn visible_lines(&self) -> Rc<Vec<Vec<Cell>>> {
        if let Some(cached) = &*self.cache.borrow() {
            return Rc::clone(cached);
        }
        let grid = self.term.grid();
        let cols = grid.columns();
        let rows = grid.screen_lines();
        let offset = grid.display_offset() as i32;
        let selection = self
            .term
            .selection
            .as_ref()
            .and_then(|s| s.to_range(&self.term));
        let table = ansi16();
        let dfg = default_fg();

        let built: Vec<Vec<Cell>> = (0..rows)
            .map(|row| {
                let grid_line = Line(row as i32 - offset);
                let line = &grid[grid_line];
                (0..cols)
                    .map(|col| {
                        let cell = &line[Column(col)];
                        let spacer = cell.flags.contains(Flags::WIDE_CHAR_SPACER);
                        Cell {
                            ch: if spacer { ' ' } else { cell.c },
                            fg: fg_to_rgb(cell.fg, table, dfg),
                            bg: bg_to_rgb(cell.bg, table, dfg),
                            bold: cell.flags.contains(Flags::BOLD),
                            wide: cell.flags.contains(Flags::WIDE_CHAR),
                            spacer,
                            selected: selection
                                .is_some_and(|r| r.contains(Point::new(grid_line, Column(col)))),
                            inverse: cell.flags.contains(Flags::INVERSE),
                        }
                    })
                    .collect()
            })
            .collect();

        let shared = Rc::new(built);
        *self.cache.borrow_mut() = Some(Rc::clone(&shared));
        shared
    }
```

- [ ] **Step 4: 所有 mutator 加缓存失效**

在以下方法**开头**各加一行 `*self.cache.borrow_mut() = None;`:

- `feed`(`term_model.rs:271`)
- `resize`(`:286`)
- `scroll_display`(`:296`)
- `scroll_to_bottom`(`:301`)
- `selection_start`(`:330`)
- `selection_update`(`:337`)
- `selection_clear`(`:351`)

`feed` 开头加的是(其余同理):

```rust
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        *self.cache.borrow_mut() = None;
        let Self { term, parser, .. } = self;
        ...
```

`set_answer_dynamic_color`(`:281`)不影响网格,不加。

- [ ] **Step 5: 改 `draw` 的取用方式(一般无需改动)**

定位 `term_view.rs:355` 的 `let lines = self.model.visible_lines();`。返回类型变成 `Rc` 后,`lines.iter()`/`lines.get(cursor_row)`/`lines.len()` 都经 `Deref` 照常工作,**这一行本身不用改**。若编译报"类型不匹配",确认没有对 `lines` 做需要 `&mut` 或按值移动的操作(当前没有)。

- [ ] **Step 6: 修测试里的三处 `.remove(0)`**

定位:

```bash
command grep -n "visible_lines().remove(0)" crates/dozer-app/src/term/term_view.rs
```

预期 `term_view.rs:534/543/561`。把 `t.visible_lines().remove(0)` 改成 `t.visible_lines()[0].clone()`(返回 `Rc` 后不能 `.remove`)。

- [ ] **Step 7: 处理 `&t.visible_lines()[...]` 形式的 `let` 绑定**

`Rc` 经 `Deref` 索引可能不触发"临时值生命周期延长"(与返回 `Vec` 时不同)。若 `cargo build` 在 `term_model.rs` 测试里报 E0716(`temporary value dropped while borrowed`),把这几处(预期 `term_model.rs:556/574/707`)改成先绑定再索引:

```rust
        let lines = t.visible_lines();
        let cell = &lines[0][0];
```

其余 `assert_eq!(line_text(&t.visible_lines()[0]), ...)` 这种**语句内临时值**的用法(临时值活到语句结束,`line_text` 在语句内消费)不受影响,不用改。

- [ ] **Step 8: 编译 + 格式检查 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo fmt -- --check && cargo test -p dozer-app --bin dozer`
Expected: 编译成功、无新增 warning、`fmt --check` 无差异、测试数字与基线一致。

- [ ] **Step 9: 人工验证**

用独立命名临时二进制(不碰用户正式 app)起一个实例:打开项目、跑一个会持续输出的 agent 会话,观察终端滚动流畅、光标闪烁正常、拖选文字高亮正确(验证 `selection_*` 失效缓存这条链路),滚动历史回看(`scroll_display`)后内容正确(验证 `scroll_*` 失效)。验证完关闭并删除临时二进制。

- [ ] **Step 10: 提交**

```bash
git add crates/dozer-app/src/term/term_model.rs crates/dozer-app/src/term/term_view.rs
git commit -m "$(cat <<'EOF'
perf(app): visible_lines 用 Rc 脏标记缓存,免每帧全量重建网格

draw() 每帧 visible_lines() 全量 collect 一个 cols×rows 的 Cell 网格
(100x30 ≈ 72KB),任何 request_redraw 都触发一次分配+释放。改成 RefCell
缓存 Rc<Vec<Vec<Cell>>>,所有改网格/视口/选区的 mutator 置脏,干净时
零拷贝返回共享句柄。不用 canvas::Cache 是因为终端模型在 widget 树之外
被 TermOutput 消息变更,canvas 自己的 update 收不到失效信号。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: 文件树目录 git 状态预聚合

**Files:**
- Modify: `crates/dozer-app/src/delivery.rs`
- Modify: `crates/dozer-app/src/extensions/files/state.rs`
- Modify: `crates/dozer-app/src/extensions/files/update.rs`
- Modify: `crates/dozer-app/src/extensions/files/view.rs`

**Interfaces:**
- Produces:`delivery::rollup_dir_statuses(&HashMap<PathBuf, FileGitStatus>) -> HashMap<PathBuf, TreeState>`(新公开函数);`WorkspaceState` 新增 `dir_statuses: HashMap<PathBuf, TreeState>` 字段。
- Consumes:`state_priority`/`TreeState`/`FileGitStatus`(delivery.rs 内已有);`StatusesRefreshed` 消息(update.rs)。

- [ ] **Step 1: 新增 `rollup_dir_statuses`**

定位 `delivery.rs` 的 `dir_status`(`:190`)。在它之后新增:

```rust
/// 预聚合:每个目录 → 子孙改动里优先级最高的 `TreeState`(语义与
/// `dir_status` 完全一致,只是把 O(N×M) 的逐行全表扫描换成一次
/// O(M×depth) 的祖先传播)。文件树渲染时按路径 O(1) 查表。被忽略的
/// 条目不向上传播(同 `dir_status` 的既有语义);目录自身被忽略(精确
/// 路径在 `statuses` 里标 `ignored`)时整目录落 `Ignored`。
pub fn rollup_dir_statuses(
    statuses: &HashMap<PathBuf, FileGitStatus>,
) -> HashMap<PathBuf, TreeState> {
    let mut dirs: HashMap<PathBuf, TreeState> = HashMap::new();
    for (path, st) in statuses {
        if st.ignored {
            dirs.insert(path.clone(), TreeState::Ignored);
            continue;
        }
        let state = TreeState::from(*st);
        let mut cur = path.parent();
        while let Some(dir) = cur {
            let entry = dirs.entry(dir.to_path_buf()).or_insert(state);
            if state_priority(state) > state_priority(*entry) {
                *entry = state;
            }
            cur = dir.parent();
        }
    }
    dirs
}
```

注意:被忽略**目录**里再出现非 ignored 子孙在实际中不会发生(git 不遍历被忽略目录),故上面"目录自身 Ignored 可能被后代覆盖"的极端冲突不做特殊处理;现有 `dir_status` 测试已覆盖"仅被忽略子孙不向上传播"与"目录自身忽略"两个语义,继续作为防漂移锚。

- [ ] **Step 2: 补单测**

在 `delivery.rs` 测试模块加一个 `rollup_dir_statuses` 测试,复用现有 `dir_status_*` 测试的 fixture 手法,断言:某目录的聚合值 == 对同一目录调 `dir_status` 的结果;并验证跨层传播(子目录改动能滚到根)。至少覆盖"untracked 覆盖 modified"与"被忽略文件不传播"两例。

- [ ] **Step 3: `WorkspaceState` 加字段**

定位:

```bash
command grep -n "git_statuses: HashMap<PathBuf, FileGitStatus>" crates/dozer-app/src/extensions/files/state.rs
```

预期在 `state.rs:64`。在它之后加:

```rust
    pub(crate) git_statuses: HashMap<PathBuf, FileGitStatus>,
    /// 目录 → 聚合 git 状态(由 `delivery::rollup_dir_statuses` 在
    /// `StatusesRefreshed` 时一并算出),文件树渲染 O(1) 查表,取代每行
    /// 一次 `dir_status` 的全表扫描(2026-09-17 性能优化)。
    pub(crate) dir_statuses: HashMap<PathBuf, delivery::TreeState>,
```

`delivery` 模块需在本文件可见——若 `state.rs` 尚未 `use crate::delivery`,补上(它已经引用了 `FileGitStatus`,通常随 `super::*` 或直接 `use` 带入,核对后补齐 `TreeState` 的引入)。

- [ ] **Step 4: 重置点一并清空**

定位 `state.rs:452` 的 `self.git_statuses = HashMap::new();`,紧跟加 `self.dir_statuses = HashMap::new();`。

- [ ] **Step 5: `StatusesRefreshed` 时一并算出**

定位:

```bash
command grep -n "git_statuses = statuses" crates/dozer-app/src/extensions/files/update.rs
```

预期 `update.rs:33`。改成:

```rust
        Message::StatusesRefreshed(project_id, statuses) => {
            ws_state.dir_statuses = delivery::rollup_dir_statuses(&statuses);
            ws_state.git_statuses = statuses;
            spawn_git_info_load(ws_state, project_id, handle, emit);
        }
```

(先算 `dir_statuses` 再移走 `statuses`,避免借用冲突。)

- [ ] **Step 6: 四处 `dir_status` 改查表**

定位(开工前 grep 核对):

```bash
command grep -n "delivery::dir_status" crates/dozer-app/src/extensions/files/view.rs
```

预期 `view.rs:109/200/524/679`。分别替换:

- `:109` `let root_state = delivery::dir_status(root, &ws_state.git_statuses)` → `let root_state = ws_state.dir_statuses.get(root).copied()`(注意去掉原 `.unwrap_or(TreeState::Unchanged)`,改为 `get(...).copied().unwrap_or(...)` 或直接在 `.get().copied()` 后接 `.unwrap_or(delivery::TreeState::Unchanged)`)。
- `:200` `delivery::dir_status(&row.path, &ws_state.git_statuses).unwrap_or(...)` → `ws_state.dir_statuses.get(&row.path).copied().unwrap_or(...)`。
- `:524` 与 `:679` `.and_then(|r| delivery::dir_status(r, &ws_state.git_statuses))` → `.and_then(|r| ws_state.dir_statuses.get(r).copied())`。

替换后把对 `delivery::dir_status` 的直接调用从渲染路径移除(保留 `dir_status` 函数本身与其测试不变,供测试与未来兼容)。

- [ ] **Step 7: 编译 + 格式检查 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo fmt -- --check && cargo test -p dozer-app --bin dozer`
Expected: 编译成功、无新增 warning、测试数字与基线一致(含新增 `rollup_dir_statuses` 用例)。

- [ ] **Step 8: 人工验证**

用独立命名临时二进制:打开一个改动较多(含未跟踪/已修改/已忽略文件)的仓库,核对文件树目录颜色与改动前一致(重点看嵌套目录的聚合色、根目录色、分支栏 dirty 态)。验证完关闭并删除临时二进制。

- [ ] **Step 9: 提交**

```bash
git add crates/dozer-app/src/delivery.rs crates/dozer-app/src/extensions/files/state.rs crates/dozer-app/src/extensions/files/update.rs crates/dozer-app/src/extensions/files/view.rs
git commit -m "$(cat <<'EOF'
perf(app): 文件树目录 git 状态预聚合,渲染从 O(N×M) 降到 O(M×depth)

dir_status 对每个目录遍历整个 git_statuses,文件树渲染时每个目录节点
调一次,大仓库(几千目录 × 数百改动)明显卡顿。新增 rollup_dir_statuses
把每个改动文件的状态沿祖先链传播一次,缓存到 WorkspaceState,渲染改
查表 O(1)。语义与 dir_status 完全一致,保留旧函数与测试作锚。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: `RingBuffer` 分块存储减少拷贝

**Files:**
- Modify: `crates/dozerd/src/ring.rs`

**Interfaces:**
- Produces:`RingBuffer` 内部 `buf: VecDeque<u8>` → `VecDeque<Box<[u8]>>` + `len: usize` 字节计数。公开方法 `new/push/total_written/snapshot/read_from` 签名不变。
- Consumes:无新增。

- [ ] **Step 1: 改结构体与 `new`**

定位 `ring.rs:7` 的 `pub struct RingBuffer`。原文:

```rust
pub struct RingBuffer {
    cap: usize,
    buf: VecDeque<u8>,
    total: u64,
}
```

改成:

```rust
pub struct RingBuffer {
    cap: usize,
    buf: VecDeque<Box<[u8]>>,
    /// 当前缓冲内的字节总数(= 各 chunk 长度之和),等价于旧 `buf.len()`。
    len: usize,
    total: u64,
}
```

`new`(`:14`)改成 `Self { cap, buf: VecDeque::new(), len: 0, total: 0 }`。

- [ ] **Step 2: 改 `push`**

原文(`:22`):

```rust
    pub fn push(&mut self, data: &[u8]) -> u64 {
        self.buf.extend(data.iter().copied());
        while self.buf.len() > self.cap {
            self.buf.pop_front();
        }
        self.total += data.len() as u64;
        self.total
    }
```

改成:

```rust
    pub fn push(&mut self, data: &[u8]) -> u64 {
        self.buf.push_back(data.to_vec().into_boxed_slice());
        self.len += data.len();
        while self.len > self.cap {
            if let Some(front) = self.buf.pop_front() {
                self.len -= front.len();
            }
        }
        self.total += data.len() as u64;
        self.total
    }
```

- [ ] **Step 3: 改 `window_start`/`snapshot`/`read_from`**

原文(`:36/40/44`):

```rust
    fn window_start(&self) -> u64 {
        self.total - self.buf.len() as u64
    }

    pub fn snapshot(&self) -> (Vec<u8>, u64) {
        (self.buf.iter().copied().collect(), self.total)
    }

    pub fn read_from(&self, offset: u64) -> Option<Vec<u8>> {
        if offset > self.total {
            return None;
        }
        if offset < self.window_start() {
            return None; // 已被逐出，调用方应退回全量 snapshot
        }
        let skip = (offset - self.window_start()) as usize;
        Some(self.buf.iter().skip(skip).copied().collect())
    }
```

改成:

```rust
    fn window_start(&self) -> u64 {
        self.total - self.len as u64
    }

    pub fn snapshot(&self) -> (Vec<u8>, u64) {
        let mut out = Vec::with_capacity(self.len);
        for chunk in &self.buf {
            out.extend_from_slice(chunk);
        }
        (out, self.total)
    }

    pub fn read_from(&self, offset: u64) -> Option<Vec<u8>> {
        if offset > self.total {
            return None;
        }
        if offset < self.window_start() {
            return None; // 已被逐出，调用方应退回全量 snapshot
        }
        let skip = (offset - self.window_start()) as usize;
        let mut out = Vec::with_capacity(self.len - skip);
        let mut remaining = skip;
        for chunk in &self.buf {
            if remaining >= chunk.len() {
                remaining -= chunk.len();
                continue;
            }
            out.extend_from_slice(&chunk[remaining..]);
            remaining = 0;
        }
        Some(out)
    }
```

- [ ] **Step 4: 补分块边界测试**

在 `ring.rs` 测试模块加一例:交替 `push` 大块与小块(如先 `push` 4 字节、再 `push` 8 字节、再 `push` 2 字节),断言 `snapshot()` 拼出的字节流与逐次 push 顺序一致、`read_from` 跨 chunk 边界取尾部正确、逐出后 `window_start` 落到 chunk 边界上也能正确 `read_from`。既有五个测试应全部保持绿(它们只断言字节内容/offset/逐出,不关心内部表示)。

- [ ] **Step 5: 编译 + 格式检查 + 测试**

Run: `cargo build -p dozerd && cargo fmt -- --check && cargo test -p dozerd`
Expected: 编译成功、无新增 warning、`fmt --check` 无差异、`ring` 测试全绿。

- [ ] **Step 6: 提交**

```bash
git add crates/dozerd/src/ring.rs
git commit -m "$(cat <<'EOF'
perf(dozerd): RingBuffer 改分块存储,消除逐字节 copied 拷贝

push 用 extend(data.iter().copied()) 逐字节拷,snapshot/read_from 用
iter().copied().collect() 整段拷。改成 VecDeque<Box<[u8]>> 分块存储 +
字节计数,追加直接 extend 一个 chunk,读取先 with_capacity 再
extend_from_slice。公开 API 与 SCROLLBACK_CAP 语义不变,既有测试全绿。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## 完工检查

四个任务都完成后:

- [ ] `cargo build`(全 workspace)无 warning。
- [ ] `cargo test -p dozer-app --bin dozer` 与开工前基线一致(外加 `rollup_dir_statuses`/ring 分块的新用例)。
- [ ] `cargo test -p dozerd` 全绿。
- [ ] `cargo fmt -- --check` 无差异。
- [ ] `cargo clippy -p dozer-app --all-targets && cargo clippy -p dozerd --all-targets` 无新增 lint。
- [ ] 提请审阅,通过后合并回 `main`。
