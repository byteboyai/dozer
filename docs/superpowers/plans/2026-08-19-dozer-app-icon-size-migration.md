# dozer-app 迁移 theme::icon_size 到 byteui Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `dozer-app` 里约 142 处 `icon_size::` 调用点(21 个文件,3 种引用形态)改成直接引用 `byteui::theme::icon_size`,删除 `dozer-app` 本地的 `theme/icon_size.rs`,并把 `init_scale`/`persist_scale`/`reset_scale` 需要的落盘路径从"内置在函数里"改成"调用方显式传入"。

**Architecture:** 纯文本替换,用一组 sed 规则(9 条,对应 9 个无状态函数)分三种前缀形态套用到 21 个文件——形态 A(`crate::theme::icon_size::NAME(`,14 个文件)、形态 B(`theme::icon_size::NAME(`,经 `use crate::theme;`,1 个文件)、形态 C(裸 `icon_size::NAME(`,经 `use crate::theme::icon_size;` 或 `use super::icon_size;`,6 个文件,替换后要删对应 `use` 行)。`init_scale`/`persist_scale`/`reset_scale` 这 3 个有状态函数(共 4 处调用)不进批量 sed,单独手改插入路径参数。21 个文件相互独立,一个文件一个 Task;首尾各加一个 Task 处理 `ui_scale_path()` 新增和本地文件删除+全量验证。

**Tech Stack:** Rust 2024,`byteui::theme::icon_size`(9 个无状态函数签名不变,`init_scale`/`persist_scale`/`reset_scale` 改吃 `&Path`,已在 `byteui` 建库时原样迁入,已合并 main)。

**Spec:** `docs/superpowers/specs/2026-08-19-dozer-app-icon-size-migration-design.md`

## Global Constraints

- **前提:migration #1(`interaction`)、migration #2(`color`)已合并到 main。** 开工前先跑 `grep -c "byteui" crates/dozer-app/Cargo.toml`(应 ≥ 1)和 `ls crates/dozer-app/src/theme/color.rs 2>&1`(应报 No such file)确认,不满足就先去完成/合并那个迁移。
- **独立分支开发,不直接提交 main。** 在新分支(如 `feature/dozer-app-icon-size-migration`)上完成全部 23 个 Task,提请审阅、通过后再合并回 `main`。每次 `git commit`/`git add` 前先跑 `git branch --show-current` 确认当前分支是自己的迁移分支,不要假设"没主动切过分支就还在原来那条上"——如果这个仓库有多个 agent 共享同一个检出目录并行工作,这一步不能省。
- **Task 1(新增 `ui_scale_path()`)必须最先做,Task 23(删本地文件 + 清理收尾)必须最后做。** 中间状态下 `theme/icon_size.rs` 必须留着,否则未迁移的文件编译不过;`ui_scale_path()` 必须先存在,Task 2(`app.rs`)和 Task 8(`main.rs`)里的手改步骤才有函数可调。
- **不迁移 `theme::font`/`theme::geometry`。** 这两个文件内部对 `icon_size::` 的引用会在各自 Task 里被改指向 `byteui::theme::icon_size`(因为它们 `use super::icon_size;` 引用的正是这次要删的模块),但文件本身、自己的 accessor 函数不删不动——那是下一个独立迁移子项目的范围。
- **不改动 `theme::homespace_color.rs`。** 和 `icon_size` 无依赖关系,不受这次改动影响。
- **形态 A/B/C 三组规则不能在同一个文件里混用。** 形态 C(裸 `icon_size::NAME(`)的规则会把形态 A/B 文件里 `crate::theme::icon_size::row(` 或 `theme::icon_size::row(` 里的 `icon_size::row(` 子串二次命中,产出 `crate::theme::byteui::theme::icon_size::row(` 这种错误的双重替换。每个 Task 只用它标注的那一种规则。

## 替换规则(所有 Task 通用,逐字对照 spec)

**9 个无状态函数的批量 sed 规则**(形态 A/B/C 共用主体,只是前缀不同):

形态 A(`crate::theme::icon_size::NAME(`)专用:

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/crate::theme::icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/crate::theme::icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/crate::theme::icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/crate::theme::icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/crate::theme::icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/crate::theme::icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/crate::theme::icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  <file>
```

形态 B(`theme::icon_size::NAME(`,经 `use crate::theme;`)专用——同 9 条,前缀去掉 `crate::`:

```bash
sed -i '' \
  -e 's/theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/theme::icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/theme::icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/theme::icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/theme::icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/theme::icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/theme::icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/theme::icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  <file>
```

形态 C(裸 `icon_size::NAME(`,经 `use crate::theme::icon_size;` 或 `use super::icon_size;`)专用——同 9 条,不带任何前缀:

```bash
sed -i '' \
  -e 's/icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  <file>
```

**`init_scale`/`persist_scale`/`reset_scale` 不进任何一组批量规则**——4 处调用(`main.rs` 1 处 `init_scale`,`app.rs` 2 处 `persist_scale` + 1 处 `reset_scale`)在各自 Task 里手改,插入 `&crate::theme::ui_scale_path()` 参数。

---

### Task 1: 新增 `ui_scale_path()`

**Files:**
- Modify: `crates/dozer-app/src/theme.rs`

**Interfaces:**
- Produces: `pub(crate) fn ui_scale_path() -> std::path::PathBuf`,供后续 Task 2(`app.rs`)、Task 8(`main.rs`)调用

- [ ] **Step 1: 追加路径函数**

在 `crates/dozer-app/src/theme.rs` 顶部(现有 `pub mod` 声明之前)追加:

```rust
use std::path::PathBuf;

/// `byteui::theme::icon_size` 的 `init_scale`/`persist_scale`/`reset_scale`
/// 需要调用方传入落盘路径(`byteui` 不内置 Dozer 专属路径约定)。
pub(crate) fn ui_scale_path() -> PathBuf {
    dozer_core::paths::config_dir().join("ui_scale.json")
}
```

- [ ] **Step 2: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功(此时函数尚无调用方,可能有 unused warning——不用管,Task 2/8 会消费它)

- [ ] **Step 3: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/theme.rs
git commit -m "refactor(dozer-app): theme.rs 新增 ui_scale_path,为迁移 icon_size 到 byteui 做准备"
```

---

### Task 2: 迁移 `app.rs`(形态 A,25 处,含 3 处手改)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::{rail, row, chevron, tab_arrow, home, tree_row_gap, scale, set_scale, zoom_by, persist_scale, reset_scale}`,`crate::theme::ui_scale_path()`(Task 1)
- Produces: `app.rs` 不再引用本地 `theme::icon_size`

- [ ] **Step 1: 应用形态 A 批量规则**

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/crate::theme::icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/crate::theme::icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/crate::theme::icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/crate::theme::icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/crate::theme::icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/crate::theme::icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/crate::theme::icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/app.rs
```

这条规则不含 `persist_scale`/`reset_scale`,跑完后这两个函数在文件里仍是
`crate::theme::icon_size::persist_scale()`/`reset_scale()` 形式,下一步手改。

- [ ] **Step 2: 手改 3 处 scale 生命周期调用**

`crates/dozer-app/src/app.rs` 里找到以下 3 行(原调用点,搬迁 spec 里记录的
行号约在 4141/4148/4154,替换后行号可能因规则执行方式略有偏移,以文本
内容为准):

```rust
crate::theme::icon_size::persist_scale();
```

出现两次,全部改成:

```rust
byteui::theme::icon_size::persist_scale(&crate::theme::ui_scale_path());
```

以及:

```rust
crate::theme::icon_size::reset_scale();
```

改成:

```rust
byteui::theme::icon_size::reset_scale(&crate::theme::ui_scale_path());
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "crate::theme::icon_size::" crates/dozer-app/src/app.rs`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "refactor(dozer-app): app.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 3: 迁移 `extensions/database.rs`(形态 A,20 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::{row, rail, scale, ...}`(具体用到哪几个由文件内容决定,签名不变)
- Produces: `database.rs` 不再引用本地 `theme::icon_size`

- [ ] **Step 1: 应用形态 A 批量规则**

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/crate::theme::icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/crate::theme::icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/crate::theme::icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/crate::theme::icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/crate::theme::icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/crate::theme::icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/crate::theme::icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/extensions/database.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::theme::icon_size::" crates/dozer-app/src/extensions/database.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/database.rs
git commit -m "refactor(dozer-app): database.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 4: 迁移 `extensions/files.rs`(形态 A,16 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/files.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::{row, rail, chevron, tree_row_gap, scale, ...}`
- Produces: `files.rs` 不再引用本地 `theme::icon_size`

- [ ] **Step 1: 应用形态 A 批量规则**

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/crate::theme::icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/crate::theme::icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/crate::theme::icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/crate::theme::icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/crate::theme::icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/crate::theme::icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/crate::theme::icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/extensions/files.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::theme::icon_size::" crates/dozer-app/src/extensions/files.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/files.rs
git commit -m "refactor(dozer-app): files.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 5: 迁移 `extensions/todo.rs`(形态 A,8 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::{row, ...}`
- Produces: `todo.rs` 不再引用本地 `theme::icon_size`

- [ ] **Step 1: 应用形态 A 批量规则**

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/crate::theme::icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/crate::theme::icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/crate::theme::icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/crate::theme::icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/crate::theme::icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/crate::theme::icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/crate::theme::icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/extensions/todo.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::theme::icon_size::" crates/dozer-app/src/extensions/todo.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "refactor(dozer-app): todo.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 6: 迁移 `extensions/project.rs`(形态 A,7 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::{row, ...}`
- Produces: `project.rs` 不再引用本地 `theme::icon_size`

- [ ] **Step 1: 应用形态 A 批量规则**

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/crate::theme::icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/crate::theme::icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/crate::theme::icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/crate::theme::icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/crate::theme::icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/crate::theme::icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/crate::theme::icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/extensions/project.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::theme::icon_size::" crates/dozer-app/src/extensions/project.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/project.rs
git commit -m "refactor(dozer-app): project.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 7: 迁移 `workspace.rs`(形态 A,4 处)

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::{row, scale}`
- Produces: `workspace.rs` 不再引用本地 `theme::icon_size`

- [ ] **Step 1: 应用形态 A 批量规则**

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/crate::theme::icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/crate::theme::icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/crate::theme::icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/crate::theme::icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/crate::theme::icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/crate::theme::icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/crate::theme::icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/workspace.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::theme::icon_size::" crates/dozer-app/src/workspace.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(dozer-app): workspace.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 8: 迁移 `main.rs`(形态 A,4 处,含 1 处手改)

**Files:**
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::{scale, init_scale}`,`crate::theme::ui_scale_path()`(Task 1)
- Produces: `main.rs` 不再引用本地 `theme::icon_size`

- [ ] **Step 1: 应用形态 A 批量规则**

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/crate::theme::icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/crate::theme::icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/crate::theme::icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/crate::theme::icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/crate::theme::icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/crate::theme::icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/crate::theme::icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/main.rs
```

这条规则不含 `init_scale`,跑完后文件里仍是 `crate::theme::icon_size::init_scale();`,下一步手改。

- [ ] **Step 2: 手改 1 处 `init_scale` 调用**

找到:

```rust
crate::theme::icon_size::init_scale();
```

改成:

```rust
byteui::theme::icon_size::init_scale(&crate::theme::ui_scale_path());
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "crate::theme::icon_size::" crates/dozer-app/src/main.rs`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/main.rs
git commit -m "refactor(dozer-app): main.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 9: 迁移 `extensions/git_log.rs`(形态 A,4 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::{row, ...}`
- Produces: `git_log.rs` 不再引用本地 `theme::icon_size`

- [ ] **Step 1: 应用形态 A 批量规则**

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/crate::theme::icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/crate::theme::icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/crate::theme::icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/crate::theme::icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/crate::theme::icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/crate::theme::icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/crate::theme::icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/extensions/git_log.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::theme::icon_size::" crates/dozer-app/src/extensions/git_log.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/git_log.rs
git commit -m "refactor(dozer-app): git_log.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 10: 迁移 `extensions/browser.rs`(形态 C,4 处 + 删 `use`)

**Files:**
- Modify: `crates/dozer-app/src/extensions/browser.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::{row, ...}`
- Produces: `browser.rs` 不再引用本地 `theme::icon_size`,不再有 `use crate::theme::icon_size;`

- [ ] **Step 1: 应用形态 C 批量规则**

```bash
sed -i '' \
  -e 's/icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/extensions/browser.rs
```

- [ ] **Step 2: 删除不再需要的 `use crate::theme::icon_size;`**

`crates/dozer-app/src/extensions/browser.rs` 里删除这一行:

```rust
use crate::theme::icon_size;
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "^use.*icon_size\|[^:]icon_size::[a-z]" crates/dozer-app/src/extensions/browser.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功(如果报 unused import,说明 Step 2 没删干净)

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/browser.rs
git commit -m "refactor(dozer-app): browser.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 11: 迁移 `homespace.rs`(形态 A,3 处)

**Files:**
- Modify: `crates/dozer-app/src/homespace.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::row`
- Produces: `homespace.rs` 不再引用本地 `theme::icon_size`

- [ ] **Step 1: 应用形态 A 批量规则**

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/crate::theme::icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/crate::theme::icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/crate::theme::icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/crate::theme::icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/crate::theme::icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/crate::theme::icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/crate::theme::icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/homespace.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::theme::icon_size::" crates/dozer-app/src/homespace.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/homespace.rs
git commit -m "refactor(dozer-app): homespace.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 12: 迁移 `extensions/footbar.rs`(形态 C,3 处 + 删 `use`)

**Files:**
- Modify: `crates/dozer-app/src/extensions/footbar.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::{row, ...}`
- Produces: `footbar.rs` 不再引用本地 `theme::icon_size`,不再有 `use crate::theme::icon_size;`

- [ ] **Step 1: 应用形态 C 批量规则**

```bash
sed -i '' \
  -e 's/icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/extensions/footbar.rs
```

- [ ] **Step 2: 删除不再需要的 `use crate::theme::icon_size;`**

`crates/dozer-app/src/extensions/footbar.rs` 里删除这一行:

```rust
use crate::theme::icon_size;
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "^use.*icon_size\|[^:]icon_size::[a-z]" crates/dozer-app/src/extensions/footbar.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/footbar.rs
git commit -m "refactor(dozer-app): footbar.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 13: 迁移 `theme/geometry.rs`(形态 A + 形态 C 混合,25 处 + 删 `use`)

**Files:**
- Modify: `crates/dozer-app/src/theme/geometry.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::{row, ...}`
- Produces: `geometry.rs` 内部不再引用本地 `theme::icon_size`,不再有
  `use super::icon_size;`(`geometry.rs` 文件本身、自己的 32+3 个 accessor
  函数——含 `tree_row_h`/`tree_chrome_top_px`/`tree_chrome_bottom_px`——
  不受影响,仍留在 `dozer-app` 本地,这次只改它们内部对 `icon_size` 的
  依赖指向)

**这个文件比其余 20 个文件多一步**:它同时存在两种引用形态——
`tree_chrome_top_px`/`tree_chrome_bottom_px` 内部用的是 2 处
`crate::theme::icon_size::row(` 完整路径(形态 A),文件里另外 32 个基础
accessor(`icon_rail_width`/`divider_width`/… 等,`geometry.rs` 自己的
公开函数,不是这次迁移对象,但**它们的实现内部**)用的是 23 处裸
`icon_size::rail(`/`row(`/`scale(` 等(形态 C,经 `use super::icon_size;`
引入)。**这 23 处如果不改,Task 23 删除本地 `theme/icon_size.rs` 后
`geometry.rs` 会编译不过**——`use super::icon_size;` 指向的模块已经不存在
了。两组引用都要在本 Task 处理掉,不能像 Task 21(`font.rs`)那样只处理
一种形态。

- [ ] **Step 1: 先套形态 A 规则(处理 2 处完整路径引用)**

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/crate::theme::icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/crate::theme::icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/crate::theme::icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/crate::theme::icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/crate::theme::icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/crate::theme::icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/crate::theme::icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/theme/geometry.rs
```

- [ ] **Step 2: 再套形态 C 规则(处理 23 处裸引用)**

**顺序不能反**:必须先跑 Step 1 再跑 Step 2。如果先跑形态 C 规则,
`crate::theme::icon_size::row(` 里的 `icon_size::row(` 子串会被提前命中,
产出 `crate::theme::byteui::theme::icon_size::row(` 这种错误的双重替换,
之后 Step 1 的形态 A 规则也不会再匹配(前缀已经变了),不会自动纠正。

```bash
sed -i '' \
  -e 's/icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/theme/geometry.rs
```

- [ ] **Step 3: 删除不再需要的 `use super::icon_size;`**

`crates/dozer-app/src/theme/geometry.rs` 里删除这一行:

```rust
use super::icon_size;
```

- [ ] **Step 4: 确认无残留**

Run: `grep -n "^use super::icon_size\|[^:]icon_size::[a-z]" crates/dozer-app/src/theme/geometry.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 5: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功(如果报 unused import,说明 Step 3 没删干净)

- [ ] **Step 6: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/theme/geometry.rs
git commit -m "refactor(dozer-app): geometry.rs 内部 icon_size 引用迁移到 byteui"
```

---

### Task 14: 迁移 `extensions/ssh.rs`(形态 A,2 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::{row, ...}`
- Produces: `ssh.rs` 不再引用本地 `theme::icon_size`

- [ ] **Step 1: 应用形态 A 批量规则**

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/crate::theme::icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/crate::theme::icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/crate::theme::icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/crate::theme::icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/crate::theme::icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/crate::theme::icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/crate::theme::icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/extensions/ssh.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::theme::icon_size::" crates/dozer-app/src/extensions/ssh.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "refactor(dozer-app): ssh.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 15: 迁移 `preview.rs`(形态 B,2 处)

**Files:**
- Modify: `crates/dozer-app/src/preview.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::row`
- Produces: `preview.rs` 不再引用本地 `theme::icon_size`

- [ ] **Step 1: 应用形态 B 批量规则**

```bash
sed -i '' \
  -e 's/theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/theme::icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/theme::icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/theme::icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/theme::icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/theme::icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/theme::icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/theme::icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/preview.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "[^byteui::]theme::icon_size::" crates/dozer-app/src/preview.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/preview.rs
git commit -m "refactor(dozer-app): preview.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 16: 迁移 `theme/region.rs`(形态 C,2 处 + 删 `use`)

**Files:**
- Modify: `crates/dozer-app/src/theme/region.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::scale`
- Produces: `region.rs` 不再引用本地 `theme::icon_size`,不再有 `use super::icon_size;`

- [ ] **Step 1: 应用形态 C 批量规则**

```bash
sed -i '' \
  -e 's/icon_size::rail(/byteui::theme::icon_size::rail(/g' \
  -e 's/icon_size::row(/byteui::theme::icon_size::row(/g' \
  -e 's/icon_size::chevron(/byteui::theme::icon_size::chevron(/g' \
  -e 's/icon_size::tab_arrow(/byteui::theme::icon_size::tab_arrow(/g' \
  -e 's/icon_size::home(/byteui::theme::icon_size::home(/g' \
  -e 's/icon_size::tree_row_gap(/byteui::theme::icon_size::tree_row_gap(/g' \
  -e 's/icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  -e 's/icon_size::set_scale(/byteui::theme::icon_size::set_scale(/g' \
  -e 's/icon_size::zoom_by(/byteui::theme::icon_size::zoom_by(/g' \
  crates/dozer-app/src/theme/region.rs
```

- [ ] **Step 2: 删除不再需要的 `use super::icon_size;`**

`crates/dozer-app/src/theme/region.rs` 里删除这一行:

```rust
use super::icon_size;
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "^use super::icon_size\|[^:]icon_size::[a-z]" crates/dozer-app/src/theme/region.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/theme/region.rs
git commit -m "refactor(dozer-app): region.rs 迁移 icon_size 引用到 byteui"
```

---

### Task 17: 迁移 `menu.rs`(形态 A,1 处)

**Files:**
- Modify: `crates/dozer-app/src/menu.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::row`
- Produces: `menu.rs` 不再引用本地 `theme::icon_size`

- [ ] **Step 1: 应用形态 A 批量规则**

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  crates/dozer-app/src/menu.rs
```

(只有 `row` 一处,单条规则足够,不需要跑全部 9 条)

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::theme::icon_size::" crates/dozer-app/src/menu.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/menu.rs
git commit -m "refactor(dozer-app): menu.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 18: 迁移 `extensions/usage.rs`(形态 A,1 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/usage.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::row`
- Produces: `usage.rs` 不再引用本地 `theme::icon_size`

- [ ] **Step 1: 应用形态 A 批量规则**

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  crates/dozer-app/src/extensions/usage.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::theme::icon_size::" crates/dozer-app/src/extensions/usage.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/usage.rs
git commit -m "refactor(dozer-app): usage.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 19: 迁移 `extensions/ssh/sftp.rs`(形态 A,1 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh/sftp.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::row`
- Produces: `sftp.rs` 不再引用本地 `theme::icon_size`

- [ ] **Step 1: 应用形态 A 批量规则**

```bash
sed -i '' \
  -e 's/crate::theme::icon_size::row(/byteui::theme::icon_size::row(/g' \
  crates/dozer-app/src/extensions/ssh/sftp.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::theme::icon_size::" crates/dozer-app/src/extensions/ssh/sftp.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/ssh/sftp.rs
git commit -m "refactor(dozer-app): sftp.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 20: 迁移 `term_view.rs`(形态 C,1 处 + 删 `use`)

**Files:**
- Modify: `crates/dozer-app/src/term_view.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::row`
- Produces: `term_view.rs` 不再引用本地 `theme::icon_size`,不再有 `use crate::theme::icon_size;`

- [ ] **Step 1: 应用形态 C 规则**

```bash
sed -i '' \
  -e 's/icon_size::row(/byteui::theme::icon_size::row(/g' \
  crates/dozer-app/src/term_view.rs
```

- [ ] **Step 2: 删除不再需要的 `use crate::theme::icon_size;`**

`crates/dozer-app/src/term_view.rs` 里删除这一行:

```rust
use crate::theme::icon_size;
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "^use.*icon_size\|[^:]icon_size::[a-z]" crates/dozer-app/src/term_view.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/term_view.rs
git commit -m "refactor(dozer-app): term_view.rs 迁移 theme::icon_size 到 byteui"
```

---

### Task 21: 迁移 `theme/font.rs`(形态 C,1 处 + 删 `use`)

**Files:**
- Modify: `crates/dozer-app/src/theme/font.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::scale`
- Produces: `font.rs` 内部不再引用本地 `theme::icon_size`,不再有 `use super::icon_size;`(`font.rs` 文件本身、8 个 accessor 函数不受影响,仍留在 `dozer-app` 本地)

- [ ] **Step 1: 应用形态 C 规则**

```bash
sed -i '' \
  -e 's/icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  crates/dozer-app/src/theme/font.rs
```

- [ ] **Step 2: 删除不再需要的 `use super::icon_size;`**

`crates/dozer-app/src/theme/font.rs` 里删除这一行:

```rust
use super::icon_size;
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "^use super::icon_size\|[^:]icon_size::[a-z]" crates/dozer-app/src/theme/font.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/theme/font.rs
git commit -m "refactor(dozer-app): font.rs 内部 icon_size 引用迁移到 byteui"
```

---

### Task 22: 迁移 `theme/homespace_font.rs`(形态 C,1 处 + 删 `use`)

**Files:**
- Modify: `crates/dozer-app/src/theme/homespace_font.rs`

**Interfaces:**
- Consumes: `byteui::theme::icon_size::scale`
- Produces: `homespace_font.rs` 内部不再引用本地 `theme::icon_size`,不再有 `use super::icon_size;`(`homespace_font.rs` 文件本身、自己的字号系统不受影响,不属于 ByteBoy2077 主题迁移范围)

- [ ] **Step 1: 应用形态 C 规则**

```bash
sed -i '' \
  -e 's/icon_size::scale(/byteui::theme::icon_size::scale(/g' \
  crates/dozer-app/src/theme/homespace_font.rs
```

- [ ] **Step 2: 删除不再需要的 `use super::icon_size;`**

`crates/dozer-app/src/theme/homespace_font.rs` 里删除这一行:

```rust
use super::icon_size;
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "^use super::icon_size\|[^:]icon_size::[a-z]" crates/dozer-app/src/theme/homespace_font.rs | grep -v byteui`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/theme/homespace_font.rs
git commit -m "refactor(dozer-app): homespace_font.rs 内部 icon_size 引用迁移到 byteui"
```

---

### Task 23: 删除本地重复实现 + 全量验证

**Files:**
- Delete: `crates/dozer-app/src/theme/icon_size.rs`
- Modify: `crates/dozer-app/src/theme.rs`

**Interfaces:**
- Consumes: Task 1-22 已完成(全仓库零处引用本地 `theme::icon_size` 模块)
- Produces: `dozer-app` 完全依赖 `byteui::theme::icon_size`,不再有重复源码

- [ ] **Step 1: 确认全仓库已无残留引用(收尾前的最后一道保险)**

Run: `grep -rn "crate::theme::icon_size::\|use crate::theme::icon_size;\|use super::icon_size;" crates/dozer-app/src --include="*.rs" | grep -v "crates/dozer-app/src/theme/icon_size.rs"`
Expected: 无输出(如果有输出,说明 Task 1-22 里漏了某个调用点,先回去补上,不要继续本 Task)

Run: `grep -rn "\bicon_size::[a-z_]*(" crates/dozer-app/src --include="*.rs" | grep -v "crates/dozer-app/src/theme/icon_size.rs" | grep -v "byteui::theme::icon_size::"`
Expected: 无输出(捕获任何形态漏改的裸引用)

- [ ] **Step 2: 删除本地重复文件**

```bash
rm crates/dozer-app/src/theme/icon_size.rs
```

- [ ] **Step 3: 清理 `theme.rs` 里的 `pub mod icon_size;`**

`crates/dozer-app/src/theme.rs` 里删除:

```rust
pub mod icon_size;
```

- [ ] **Step 4: 全量编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer`
Expected: 编译成功;测试结果与迁移前基线一致(484 passed / 2 failed——两个已知的、与本次改动无关的 terminal grid 尺寸测试失败)

- [ ] **Step 5: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 无警告、无格式差异

- [ ] **Step 6: 独立临时二进制缩放专项核对**

构建一个独立命名的临时二进制(不要用会撞到用户正在跑的正式 `/Applications/
Dozer AI Coder.app` 或其他调试会话进程名的路径),启动后核对:

- Ctrl + 放大三次、Ctrl - 缩小两次,确认图标/文字/几何布局(rail 宽、
  tab 尺寸、面板间距)整体同步缩放,没有"图标变了文字没变"的分裂
  (这是 spec 里记录的、这次迁移要消掉的耦合风险,必须实测确认)。
- Ctrl+1 还原,确认缩放立即回到出厂默认。
- 退出、重开,确认缩放值(含 Ctrl+1 还原后的出厂默认落盘)跨重启保留。
- 打开 `~/Library/Application Support/dozer/ui_scale.json`(或
  `dozer_core::paths::config_dir()` 实际指向路径),确认内容与最近一次
  放大/还原操作一致。

完成后关闭该临时实例、删除临时二进制,不留后台进程。

- [ ] **Step 7: Commit**

```bash
git branch --show-current
git add -A
git commit -m "refactor(dozer-app): 删除 theme::icon_size 本地重复实现,改用 byteui"
```

---

## 完工验收

1. `git log --oneline` 确认全部 23 个 commit 都在当前分支上,没有漂到 `main`。
2. `grep -rn "crate::theme::icon_size::\|use crate::theme::icon_size;\|use super::icon_size;" crates/dozer-app/src` 全仓库零匹配(排除已删除的 `theme/icon_size.rs`)。
3. `crates/dozer-app/src/theme/icon_size.rs` 已删除。
4. 提请审阅(参考 spec"排期备注"——`theme::font`+`theme::geometry` 是下一个独立迁移子项目,不在这次验收范围内)。
