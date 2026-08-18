# dozer-app 迁移 interaction 到 byteui Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `dozer-app` 里 `icons`/`tabs`/`theme::cards`/`scrollbar` 四组的约 190 处调用点,从本地模块改成直接引用 `byteui::interaction::{icons,tabs,cards,scrollbar}`,并删除 `dozer-app` 里的重复实现。

**Architecture:** 纯机械的路径前缀替换,不改变任何函数签名或调用参数——`byteui::interaction::*` 与被删除的本地模块逐字节行为一致(`byteui` 建库时原样迁移过去的)。11 个受影响文件相互独立,一个文件一个 Task;最后一个 Task 统一删除本地重复文件、清理 `mod` 声明、跑全量验证。

**Tech Stack:** Rust 2024,`byteui`(`crates/byteui`,已合并 main)。

**Spec:** `docs/superpowers/specs/2026-08-18-dozer-app-interaction-migration-design.md`

## Global Constraints

- **独立分支开发,不直接提交 main。** 在新分支(如 `feature/dozer-app-interaction-migration`)上完成全部 13 个 Task,提请审阅、通过后再合并回 `main`(见 `[[feedback-plans-use-worktree-branch]]`)。若用 subagent-driven-development 派发,dispatch 消息第一句必须是 `cd <worktree 绝对路径> && git branch --show-current` 校验,Read/Edit 的 `file_path` 必须带完整 worktree 绝对路径前缀(见 `[[feedback-sdd-implementer-main-drift]]`——控制者收到完工汇报后必须自己 `git log --oneline -3` 核实产出 commit 真的在 worktree 分支上)。
- **不改变任何视觉/交互行为。** 这是纯路径重构,不是"顺手改进"——`byteui::interaction::*` 和被删除的本地实现逐字节一致。
- **Task 13(删本地文件 + 加依赖收尾)必须在 Task 1-12 全部完成之后才能做。** 中间状态下本地 `icons.rs`/`tabs.rs`/`theme/cards.rs`/`scrollbar.rs` 必须留着,否则还没迁移的文件会编译不过。
- **不迁移 `theme::color`/`theme::font`/`theme::geometry`/`theme::icon_size`。** 这四个是后续独立的迁移子项目,这次 Global Constraints 之外,任何 Task 都不应该顺手碰它们。

## 替换规则(所有 Task 通用,逐字对照 spec)

| 原文本 | 新文本 |
|---|---|
| `use crate::icons;` | `use byteui::interaction::icons;` |
| `use crate::tabs;` | `use byteui::interaction::tabs;` |
| `crate::icons::` | `byteui::interaction::icons::` |
| `crate::tabs::` | `byteui::interaction::tabs::` |
| `crate::theme::cards::` | `byteui::interaction::cards::` |
| `crate::scrollbar::` | `byteui::interaction::scrollbar::` |

六条规则互不重叠(`use crate::icons;` 结尾是 `;`,`crate::icons::` 结尾是 `::`,同一处文本不会同时匹配两条),用字面量替换(不是正则),任意顺序应用结果都一样。

---

### Task 1: 加 `byteui` 依赖

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`

**Interfaces:**
- Consumes: `byteui` crate(`crates/byteui`,已在 main 上)
- Produces: `dozer-app` 可以 `use byteui::...`,供 Task 2-12 使用

- [ ] **Step 1: 加依赖**

在 `crates/dozer-app/Cargo.toml` 的 `[dependencies]` 块里,紧挨着 `dozer-hook = { path = "../dozer-hook" }` 之后加一行:

```toml
byteui = { path = "../byteui" }
```

- [ ] **Step 2: 验证编译未受影响**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功(新依赖此时还没有任何代码引用,不影响现有编译)

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/Cargo.toml Cargo.lock
git commit -m "chore(dozer-app): 加 byteui 依赖"
```

---

### Task 2: 迁移 `app.rs`(37 处,含 icons + tabs 两组)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `byteui::interaction::{icons, tabs}`(Task 1 已加依赖)
- Produces: `app.rs` 不再引用本地 `crate::icons`/`crate::tabs`

- [ ] **Step 1: 应用替换规则**

```bash
sed -i '' \
  -e 's/use crate::icons;/use byteui::interaction::icons;/g' \
  -e 's/use crate::tabs;/use byteui::interaction::tabs;/g' \
  -e 's/crate::icons::/byteui::interaction::icons::/g' \
  -e 's/crate::tabs::/byteui::interaction::tabs::/g' \
  -e 's/crate::theme::cards::/byteui::interaction::cards::/g' \
  -e 's/crate::scrollbar::/byteui::interaction::scrollbar::/g' \
  crates/dozer-app/src/app.rs
```

- [ ] **Step 2: 确认这个文件里不再有旧引用残留**

Run: `grep -n "crate::icons\|crate::tabs\b\|crate::theme::cards\|crate::scrollbar" crates/dozer-app/src/app.rs`
Expected: 无输出(零匹配)

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "refactor(dozer-app): app.rs 迁移 icons/tabs 到 byteui::interaction"
```

---

### Task 3: 迁移 `extensions/files.rs`(27 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/files.rs`

**Interfaces:**
- Consumes: `byteui::interaction::{icons, scrollbar}`
- Produces: `files.rs` 不再引用本地 `crate::icons`/`crate::scrollbar`

- [ ] **Step 1: 应用替换规则**

```bash
sed -i '' \
  -e 's/use crate::icons;/use byteui::interaction::icons;/g' \
  -e 's/use crate::tabs;/use byteui::interaction::tabs;/g' \
  -e 's/crate::icons::/byteui::interaction::icons::/g' \
  -e 's/crate::tabs::/byteui::interaction::tabs::/g' \
  -e 's/crate::theme::cards::/byteui::interaction::cards::/g' \
  -e 's/crate::scrollbar::/byteui::interaction::scrollbar::/g' \
  crates/dozer-app/src/extensions/files.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::icons\|crate::tabs\b\|crate::theme::cards\|crate::scrollbar" crates/dozer-app/src/extensions/files.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/files.rs
git commit -m "refactor(dozer-app): files.rs 迁移 icons/scrollbar 到 byteui::interaction"
```

---

### Task 4: 迁移 `extensions/todo.rs`(23 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`

**Interfaces:**
- Consumes: `byteui::interaction::{icons, cards, scrollbar}`
- Produces: `todo.rs` 不再引用本地 `crate::icons`/`crate::theme::cards`/`crate::scrollbar`

- [ ] **Step 1: 应用替换规则**

```bash
sed -i '' \
  -e 's/use crate::icons;/use byteui::interaction::icons;/g' \
  -e 's/use crate::tabs;/use byteui::interaction::tabs;/g' \
  -e 's/crate::icons::/byteui::interaction::icons::/g' \
  -e 's/crate::tabs::/byteui::interaction::tabs::/g' \
  -e 's/crate::theme::cards::/byteui::interaction::cards::/g' \
  -e 's/crate::scrollbar::/byteui::interaction::scrollbar::/g' \
  crates/dozer-app/src/extensions/todo.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::icons\|crate::tabs\b\|crate::theme::cards\|crate::scrollbar" crates/dozer-app/src/extensions/todo.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "refactor(dozer-app): todo.rs 迁移 icons/cards/scrollbar 到 byteui::interaction"
```

---

### Task 5: 迁移 `extensions/database.rs`(16 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`

**Interfaces:**
- Consumes: `byteui::interaction::{icons, scrollbar}`
- Produces: `database.rs` 不再引用本地 `crate::icons`/`crate::scrollbar`

- [ ] **Step 1: 应用替换规则**

```bash
sed -i '' \
  -e 's/use crate::icons;/use byteui::interaction::icons;/g' \
  -e 's/use crate::tabs;/use byteui::interaction::tabs;/g' \
  -e 's/crate::icons::/byteui::interaction::icons::/g' \
  -e 's/crate::tabs::/byteui::interaction::tabs::/g' \
  -e 's/crate::theme::cards::/byteui::interaction::cards::/g' \
  -e 's/crate::scrollbar::/byteui::interaction::scrollbar::/g' \
  crates/dozer-app/src/extensions/database.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::icons\|crate::tabs\b\|crate::theme::cards\|crate::scrollbar" crates/dozer-app/src/extensions/database.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs
git commit -m "refactor(dozer-app): database.rs 迁移 icons/scrollbar 到 byteui::interaction"
```

---

### Task 6: 迁移 `homespace.rs`(16 处)

**Files:**
- Modify: `crates/dozer-app/src/homespace.rs`

**Interfaces:**
- Consumes: `byteui::interaction::{icons, cards, scrollbar}`
- Produces: `homespace.rs` 不再引用本地 `crate::icons`/`crate::theme::cards`/`crate::scrollbar`

- [ ] **Step 1: 应用替换规则**

```bash
sed -i '' \
  -e 's/use crate::icons;/use byteui::interaction::icons;/g' \
  -e 's/use crate::tabs;/use byteui::interaction::tabs;/g' \
  -e 's/crate::icons::/byteui::interaction::icons::/g' \
  -e 's/crate::tabs::/byteui::interaction::tabs::/g' \
  -e 's/crate::theme::cards::/byteui::interaction::cards::/g' \
  -e 's/crate::scrollbar::/byteui::interaction::scrollbar::/g' \
  crates/dozer-app/src/homespace.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::icons\|crate::tabs\b\|crate::theme::cards\|crate::scrollbar" crates/dozer-app/src/homespace.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/homespace.rs
git commit -m "refactor(dozer-app): homespace.rs 迁移 icons/cards/scrollbar 到 byteui::interaction"
```

---

### Task 7: 迁移 `workspace.rs`(11 处)

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `byteui::interaction::{icons, cards, scrollbar}`
- Produces: `workspace.rs` 不再引用本地 `crate::icons`/`crate::theme::cards`/`crate::scrollbar`(含唯一的命名导入 `use crate::icons::IconKind;`,会被 `crate::icons::` 前缀规则一并转成 `use byteui::interaction::icons::IconKind;`)

- [ ] **Step 1: 应用替换规则**

```bash
sed -i '' \
  -e 's/use crate::icons;/use byteui::interaction::icons;/g' \
  -e 's/use crate::tabs;/use byteui::interaction::tabs;/g' \
  -e 's/crate::icons::/byteui::interaction::icons::/g' \
  -e 's/crate::tabs::/byteui::interaction::tabs::/g' \
  -e 's/crate::theme::cards::/byteui::interaction::cards::/g' \
  -e 's/crate::scrollbar::/byteui::interaction::scrollbar::/g' \
  crates/dozer-app/src/workspace.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::icons\|crate::tabs\b\|crate::theme::cards\|crate::scrollbar" crates/dozer-app/src/workspace.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(dozer-app): workspace.rs 迁移 icons/cards/scrollbar 到 byteui::interaction"
```

---

### Task 8: 迁移 `extensions/git_log.rs`(8 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs`

**Interfaces:**
- Consumes: `byteui::interaction::{icons, cards}`
- Produces: `git_log.rs` 不再引用本地 `crate::icons`/`crate::theme::cards`

- [ ] **Step 1: 应用替换规则**

```bash
sed -i '' \
  -e 's/use crate::icons;/use byteui::interaction::icons;/g' \
  -e 's/use crate::tabs;/use byteui::interaction::tabs;/g' \
  -e 's/crate::icons::/byteui::interaction::icons::/g' \
  -e 's/crate::tabs::/byteui::interaction::tabs::/g' \
  -e 's/crate::theme::cards::/byteui::interaction::cards::/g' \
  -e 's/crate::scrollbar::/byteui::interaction::scrollbar::/g' \
  crates/dozer-app/src/extensions/git_log.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::icons\|crate::tabs\b\|crate::theme::cards\|crate::scrollbar" crates/dozer-app/src/extensions/git_log.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/git_log.rs
git commit -m "refactor(dozer-app): git_log.rs 迁移 icons/cards 到 byteui::interaction"
```

---

### Task 9: 迁移 `extensions/ssh.rs`(8 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`

**Interfaces:**
- Consumes: `byteui::interaction::{icons, cards}`
- Produces: `ssh.rs` 不再引用本地 `crate::icons`/`crate::theme::cards`

- [ ] **Step 1: 应用替换规则**

```bash
sed -i '' \
  -e 's/use crate::icons;/use byteui::interaction::icons;/g' \
  -e 's/use crate::tabs;/use byteui::interaction::tabs;/g' \
  -e 's/crate::icons::/byteui::interaction::icons::/g' \
  -e 's/crate::tabs::/byteui::interaction::tabs::/g' \
  -e 's/crate::theme::cards::/byteui::interaction::cards::/g' \
  -e 's/crate::scrollbar::/byteui::interaction::scrollbar::/g' \
  crates/dozer-app/src/extensions/ssh.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::icons\|crate::tabs\b\|crate::theme::cards\|crate::scrollbar" crates/dozer-app/src/extensions/ssh.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "refactor(dozer-app): ssh.rs 迁移 icons/cards 到 byteui::interaction"
```

---

### Task 10: 迁移 `extensions/ssh/sftp.rs`(6 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh/sftp.rs`

**Interfaces:**
- Consumes: `byteui::interaction::icons`
- Produces: `sftp.rs` 不再引用本地 `crate::icons`

- [ ] **Step 1: 应用替换规则**

```bash
sed -i '' \
  -e 's/use crate::icons;/use byteui::interaction::icons;/g' \
  -e 's/use crate::tabs;/use byteui::interaction::tabs;/g' \
  -e 's/crate::icons::/byteui::interaction::icons::/g' \
  -e 's/crate::tabs::/byteui::interaction::tabs::/g' \
  -e 's/crate::theme::cards::/byteui::interaction::cards::/g' \
  -e 's/crate::scrollbar::/byteui::interaction::scrollbar::/g' \
  crates/dozer-app/src/extensions/ssh/sftp.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::icons\|crate::tabs\b\|crate::theme::cards\|crate::scrollbar" crates/dozer-app/src/extensions/ssh/sftp.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh/sftp.rs
git commit -m "refactor(dozer-app): sftp.rs 迁移 icons 到 byteui::interaction"
```

---

### Task 11: 迁移 `extensions/footbar.rs`(5 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/footbar.rs`

**Interfaces:**
- Consumes: `byteui::interaction::icons`
- Produces: `footbar.rs` 不再引用本地 `crate::icons`

- [ ] **Step 1: 应用替换规则**

```bash
sed -i '' \
  -e 's/use crate::icons;/use byteui::interaction::icons;/g' \
  -e 's/use crate::tabs;/use byteui::interaction::tabs;/g' \
  -e 's/crate::icons::/byteui::interaction::icons::/g' \
  -e 's/crate::tabs::/byteui::interaction::tabs::/g' \
  -e 's/crate::theme::cards::/byteui::interaction::cards::/g' \
  -e 's/crate::scrollbar::/byteui::interaction::scrollbar::/g' \
  crates/dozer-app/src/extensions/footbar.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::icons\|crate::tabs\b\|crate::theme::cards\|crate::scrollbar" crates/dozer-app/src/extensions/footbar.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/footbar.rs
git commit -m "refactor(dozer-app): footbar.rs 迁移 icons 到 byteui::interaction"
```

---

### Task 12: 迁移 `extensions/usage.rs`(2 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/usage.rs`

**Interfaces:**
- Consumes: `byteui::interaction::icons`
- Produces: `usage.rs` 不再引用本地 `crate::icons`

- [ ] **Step 1: 应用替换规则**

```bash
sed -i '' \
  -e 's/use crate::icons;/use byteui::interaction::icons;/g' \
  -e 's/use crate::tabs;/use byteui::interaction::tabs;/g' \
  -e 's/crate::icons::/byteui::interaction::icons::/g' \
  -e 's/crate::tabs::/byteui::interaction::tabs::/g' \
  -e 's/crate::theme::cards::/byteui::interaction::cards::/g' \
  -e 's/crate::scrollbar::/byteui::interaction::scrollbar::/g' \
  crates/dozer-app/src/extensions/usage.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "crate::icons\|crate::tabs\b\|crate::theme::cards\|crate::scrollbar" crates/dozer-app/src/extensions/usage.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/usage.rs
git commit -m "refactor(dozer-app): usage.rs 迁移 icons 到 byteui::interaction"
```

---

### Task 13: 删除本地重复实现 + 全量验证

**Files:**
- Delete: `crates/dozer-app/src/icons.rs`
- Delete: `crates/dozer-app/src/tabs.rs`
- Delete: `crates/dozer-app/src/theme/cards.rs`
- Delete: `crates/dozer-app/src/scrollbar.rs`
- Delete: `crates/dozer-app/assets/icons/`(整个目录)
- Modify: `crates/dozer-app/src/main.rs`
- Modify: `crates/dozer-app/src/theme/mod.rs`

**Interfaces:**
- Consumes: Task 1-12 已完成(全仓库零处引用本地 `icons`/`tabs`/`theme::cards`/`scrollbar` 模块)
- Produces: `dozer-app` 完全依赖 `byteui::interaction::*`,不再有重复源码

- [ ] **Step 1: 确认全仓库已无残留引用(收尾前的最后一道保险)**

Run: `grep -rn "crate::icons\|crate::tabs\b\|crate::theme::cards\|crate::scrollbar" crates/dozer-app/src --include="*.rs"`
Expected: 无输出(如果有输出,说明 Task 1-12 里漏了某个调用点,先回去补上,不要继续本 Task)

- [ ] **Step 2: 删除本地重复文件**

```bash
rm crates/dozer-app/src/icons.rs
rm crates/dozer-app/src/tabs.rs
rm crates/dozer-app/src/theme/cards.rs
rm crates/dozer-app/src/scrollbar.rs
rm -r crates/dozer-app/assets/icons/
```

- [ ] **Step 3: 清理 `main.rs` 里的 `mod` 声明**

`crates/dozer-app/src/main.rs` 里删除这三行(原第 12、23、24 行,内容分别是):

```rust
mod icons;
```

```rust
mod scrollbar;
```

```rust
mod tabs;
```

- [ ] **Step 4: 清理 `theme/mod.rs` 里的 `pub mod cards;`**

`crates/dozer-app/src/theme/mod.rs` 里删除:

```rust
pub mod cards;
```

- [ ] **Step 5: 全量编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer`
Expected: 编译成功;测试结果与迁移前基线一致(484 passed / 2 failed——两个已知的、与本次改动无关的 terminal grid 尺寸测试失败,`app-workspace-split` 合并前就存在,见 `2026-08-12` 迁移记录)

- [ ] **Step 6: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 无警告、无格式差异

- [ ] **Step 7: 独立临时二进制 GUI 视觉核对**

构建一个独立命名的临时二进制(不要用会撞到用户正在跑的正式 `/Applications/Dozer AI Coder.app` 或其他调试会话进程名的路径),启动后核对:

- 左右 icon rail 全部按钮:静止态图标颜色、hover 过渡、点击切换面板,与迁移前逐一比对无差异。
- 顶栏项目页签:选中态、hover、关闭按钮(仅 hover 时可点)、拖拽换位。
- 各类列表卡(Agent/项目/最近/todo/git commit/主机):一般/hover/选中三态的描边与背景色。
- files/database/todo 各面板的竖直滚动条外观(滑块颜色、轨道)。

完成后关闭该临时实例、删除临时二进制,不留后台进程。

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor(dozer-app): 删除 icons/tabs/cards/scrollbar 本地重复实现,改用 byteui"
```

---

## 完工验收

1. `git log --oneline` 确认全部 13 个 commit 都在当前分支上,没有漂到 `main`(Global Constraints 里的强制校验项)。
2. `grep -rn "crate::icons\|crate::tabs\b\|crate::theme::cards\|crate::scrollbar" crates/dozer-app/src` 全仓库零匹配。
3. `crates/dozer-app/src/{icons.rs,tabs.rs,theme/cards.rs,scrollbar.rs}` 和 `crates/dozer-app/assets/icons/` 均已删除。
4. 提请审阅(参考 spec"排期备注"——`theme::color`/`theme::font`+`theme::geometry`/`theme::icon_size` 三个后续迁移子项目不在这次验收范围内)。
