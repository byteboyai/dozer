# dozer-app 迁移 theme::color 到 byteui Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `dozer-app` 里 662 处 `theme::color` 调用点(19 个文件走完整路径 + `region.rs` 走裸别名路径)改成直接引用 `byteui::theme::color::{current, mix}`,删除 `dozer-app` 本地的 `theme/color.rs`。

**Architecture:** 纯文本替换(const 访问 → 函数调用取字段),用两组 sed 规则脚本化处理——规则组 A 处理 19 个走完整路径 `theme::color::NAME` 的文件,规则组 B 单独处理走裸 `color::NAME`(经 `use super::color;` 别名)的 `region.rs`。20 个文件相互独立,一个文件一个 Task;最后一个 Task 统一删除本地重复文件、清理 `mod` 声明、跑全量验证。

**Tech Stack:** Rust 2024,`byteui::theme::color`(`ColorTokens`/`current()`/`set_theme()`/`mix()`,已合并 main)。

**Spec:** `docs/superpowers/specs/2026-08-18-dozer-app-color-migration-design.md`

## Global Constraints

- **前提:migration #1(`interaction`)已合并到 main。** 开工前先跑 `grep -c "byteui" crates/dozer-app/Cargo.toml`(应 ≥ 1)和 `ls crates/dozer-app/src/icons.rs`(应报 No such file)确认,不满足就先去完成/合并那个迁移。
- **独立分支开发,不直接提交 main。** 在新分支(如 `feature/dozer-app-color-migration`)上完成全部 21 个 Task,提请审阅、通过后再合并回 `main`(见 `[[feedback-plans-use-worktree-branch]]`)。**这次额外强调:提交前用 `git branch --show-current` 确认当前分支是你自己的迁移分支**——2026-08-18 当天已经发生过一次事故:controller 在共享工作目录里直接 `git commit`,没检查分支,结果提交混进了另一个 agent(migration #1)正在跑的分支历史里,靠临时 worktree 才补救回 main。如果这个仓库有多个 agent 共享同一个检出目录在并行工作,每次 `git commit`/`git add` 前都要先确认分支,不要假设"我这边没切过分支就还在我以为的那条上"。
- **Task 21(删本地文件 + 清理收尾)必须在 Task 1-20 全部完成之后才能做。** 中间状态下 `theme/color.rs` 必须留着,否则还没迁移的文件编译不过。
- **不迁移 `theme::font`/`theme::geometry`/`theme::icon_size`。** 后续两个独立迁移子项目的范围,这次不碰。
- **不改动 `theme/homespace_color.rs`/`theme/homespace_font.rs`。** 和 ByteBoy2077 主题无关联的独立配色系统。
- **规则组 A 和规则组 B 不能混用。** 规则组 B(裸 `color::NAME`)只能用在 `region.rs`;误用在规则组 A 的文件上会把 `theme::color::GOLD` 里的 `color::GOLD` 子串二次误伤,产出错误的双重替换。

## 替换规则(所有 Task 通用,逐字对照 spec)

**规则组 A**(19 个文件,`theme::color::NAME` 完整路径形式;第一条先把 `crate::theme::color::` 归一成 `theme::color::`,再统一处理,避免 `crate::theme::color::GOLD` 被错误替换成不存在的 `crate::byteui::...` 路径):

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  <file>
```

**规则组 B**(仅 `region.rs`,裸 `color::NAME` 形式,同样 23 条只是去掉 `theme::` 前缀,不含 `crate::` 归一化那条、也不含 `mix(` 那条——`region.rs` 里都没有):

```bash
sed -i '' \
  -e 's/color::BG/byteui::theme::color::current().bg/g' \
  -e 's/color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/color::CARD/byteui::theme::color::current().card/g' \
  -e 's/color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/color::BODY/byteui::theme::color::current().body/g' \
  -e 's/color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/color::RED/byteui::theme::color::current().red/g' \
  -e 's/color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/theme/region.rs
```

---

### Task 1: 迁移 `workspace.rs`(规则组 A,100 处)

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`(已在 main 上)
- Produces: `workspace.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/workspace.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/workspace.rs`
Expected: 无输出(23 个大写常量都已转换;`byteui::theme::color::current()` 里的 `current` 是小写,不会被这个 grep 命中)

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**(提交前先确认分支——见 Global Constraints)

```bash
git branch --show-current
git add crates/dozer-app/src/workspace.rs
git commit -m "refactor(dozer-app): workspace.rs 迁移 theme::color 到 byteui"
```

---

### Task 2: 迁移 `extensions/todo.rs`(规则组 A,91 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `todo.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/extensions/todo.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/extensions/todo.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "refactor(dozer-app): todo.rs 迁移 theme::color 到 byteui"
```

---

### Task 3: 迁移 `app.rs`(规则组 A,65 处)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `app.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/app.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/app.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "refactor(dozer-app): app.rs 迁移 theme::color 到 byteui"
```

---

### Task 4: 迁移 `extensions/files.rs`(规则组 A,55 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/files.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `files.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/extensions/files.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/extensions/files.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/files.rs
git commit -m "refactor(dozer-app): files.rs 迁移 theme::color 到 byteui"
```

---

### Task 5: 迁移 `extensions/ssh.rs`(规则组 A,48 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `ssh.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/extensions/ssh.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/extensions/ssh.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "refactor(dozer-app): ssh.rs 迁移 theme::color 到 byteui"
```

---

### Task 6: 迁移 `extensions/project.rs`(规则组 A,47 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `project.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/extensions/project.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/extensions/project.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/project.rs
git commit -m "refactor(dozer-app): project.rs 迁移 theme::color 到 byteui"
```

---

### Task 7: 迁移 `extensions/git_log.rs`(规则组 A,40 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `git_log.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/extensions/git_log.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/extensions/git_log.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/git_log.rs
git commit -m "refactor(dozer-app): git_log.rs 迁移 theme::color 到 byteui"
```

---

### Task 8: 迁移 `extensions/database.rs`(规则组 A,36 处,含 `crate::` 前缀归一化)

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `database.rs` 不再引用本地 `theme::color`

**这个文件是 spec 里点名的、带显式 `crate::theme::color::NAME` 前缀写法的两个文件之一**——规则组 A 第一条 `crate::theme::color::` → `theme::color::` 的归一化在这里会真正生效,不是空操作。

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/extensions/database.rs
```

- [ ] **Step 2: 确认无残留(含 `crate::` 形式)**

Run: `grep -n "theme::color::[A-Z]\|crate::byteui" crates/dozer-app/src/extensions/database.rs`
Expected: 无输出(第二个模式是保险检查——如果归一化规则没生效或顺序错了,`crate::theme::color::GOLD` 会被错误替换成 `crate::byteui::theme::color::current().gold`,这个模式能抓到这种坏结果)

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/database.rs
git commit -m "refactor(dozer-app): database.rs 迁移 theme::color 到 byteui"
```

---

### Task 9: 迁移 `extensions/acceptance.rs`(规则组 A,26 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/acceptance.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `acceptance.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/extensions/acceptance.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/extensions/acceptance.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/acceptance.rs
git commit -m "refactor(dozer-app): acceptance.rs 迁移 theme::color 到 byteui"
```

---

### Task 10: 迁移 `extensions/usage.rs`(规则组 A,24 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/usage.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `usage.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/extensions/usage.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/extensions/usage.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/usage.rs
git commit -m "refactor(dozer-app): usage.rs 迁移 theme::color 到 byteui"
```

---

### Task 11: 迁移 `preview.rs`(规则组 A,23 处)

**Files:**
- Modify: `crates/dozer-app/src/preview.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `preview.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/preview.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/preview.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/preview.rs
git commit -m "refactor(dozer-app): preview.rs 迁移 theme::color 到 byteui"
```

---

### Task 12: 迁移 `extensions/search.rs`(规则组 A,23 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/search.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `search.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/extensions/search.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/extensions/search.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/search.rs
git commit -m "refactor(dozer-app): search.rs 迁移 theme::color 到 byteui"
```

---

### Task 13: 迁移 `extensions/browser.rs`(规则组 A,19 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/browser.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `browser.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/extensions/browser.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/extensions/browser.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/browser.rs
git commit -m "refactor(dozer-app): browser.rs 迁移 theme::color 到 byteui"
```

---

### Task 14: 迁移 `extensions/footbar.rs`(规则组 A,10 处)

**Files:**
- Modify: `crates/dozer-app/src/extensions/footbar.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `footbar.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/extensions/footbar.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/extensions/footbar.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/footbar.rs
git commit -m "refactor(dozer-app): footbar.rs 迁移 theme::color 到 byteui"
```

---

### Task 15: 迁移 `term_view.rs`(规则组 A,7 处)

**Files:**
- Modify: `crates/dozer-app/src/term_view.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `term_view.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/term_view.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/term_view.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/term_view.rs
git commit -m "refactor(dozer-app): term_view.rs 迁移 theme::color 到 byteui"
```

---

### Task 16: 迁移 `extensions/ssh/sftp.rs`(规则组 A,7 处,含 `crate::` 前缀归一化)

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh/sftp.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `sftp.rs` 不再引用本地 `theme::color`

**这是 spec 里点名的另一个带显式 `crate::theme::color::NAME` 前缀写法的文件**(和 Task 8 的 `database.rs` 一起,两个文件共 50 处这种写法)——规则组 A 第一条归一化规则在这里同样会真正生效。

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/extensions/ssh/sftp.rs
```

- [ ] **Step 2: 确认无残留(含 `crate::` 形式)**

Run: `grep -n "theme::color::[A-Z]\|crate::byteui" crates/dozer-app/src/extensions/ssh/sftp.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/ssh/sftp.rs
git commit -m "refactor(dozer-app): sftp.rs 迁移 theme::color 到 byteui"
```

---

### Task 17: 迁移 `menu.rs`(规则组 A,4 处)

**Files:**
- Modify: `crates/dozer-app/src/menu.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `menu.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/menu.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/menu.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/menu.rs
git commit -m "refactor(dozer-app): menu.rs 迁移 theme::color 到 byteui"
```

---

### Task 18: 迁移 `diff_render.rs`(规则组 A,4 处)

**Files:**
- Modify: `crates/dozer-app/src/diff_render.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `diff_render.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/diff_render.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/diff_render.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/diff_render.rs
git commit -m "refactor(dozer-app): diff_render.rs 迁移 theme::color 到 byteui"
```

---

### Task 19: 迁移 `homespace.rs`(规则组 A,3 处)

**Files:**
- Modify: `crates/dozer-app/src/homespace.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::{current, mix}`
- Produces: `homespace.rs` 不再引用本地 `theme::color`

- [ ] **Step 1: 应用规则组 A**

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/homespace.rs
```

- [ ] **Step 2: 确认无残留**

Run: `grep -n "theme::color::[A-Z]" crates/dozer-app/src/homespace.rs`
Expected: 无输出

- [ ] **Step 3: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/homespace.rs
git commit -m "refactor(dozer-app): homespace.rs 迁移 theme::color 到 byteui"
```

---

### Task 20: 迁移 `theme/region.rs`(规则组 B,30 处裸引用 + 删 `use super::color;`)

**Files:**
- Modify: `crates/dozer-app/src/theme/region.rs`

**Interfaces:**
- Consumes: `byteui::theme::color::current`(`region.rs` 没有用到 `mix`,已核对)
- Produces: `region.rs` 不再引用本地 `theme::color`,不再有 `use super::color;`

- [ ] **Step 1: 应用规则组 B**

```bash
sed -i '' \
  -e 's/color::BG/byteui::theme::color::current().bg/g' \
  -e 's/color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/color::CARD/byteui::theme::color::current().card/g' \
  -e 's/color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/color::BODY/byteui::theme::color::current().body/g' \
  -e 's/color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/color::RED/byteui::theme::color::current().red/g' \
  -e 's/color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/theme/region.rs
```

- [ ] **Step 2: 删除不再需要的 `use super::color;`**

`crates/dozer-app/src/theme/region.rs` 顶部第 21 行(替换前的行号)删除:

```rust
use super::color;
```

- [ ] **Step 3: 确认无残留**

Run: `grep -n "\bcolor::[A-Z]\|use super::color" crates/dozer-app/src/theme/region.rs`
Expected: 无输出

- [ ] **Step 4: 编译验证**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功(如果这一步报 `use super::color;` 相关的 unused import 警告变成编译错误,说明 Step 2 的删除没生效,回去检查)

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/theme/region.rs
git commit -m "refactor(dozer-app): region.rs 迁移裸 color:: 引用到 byteui"
```

---

### Task 21: 删除本地重复实现 + 全量验证

**Files:**
- Delete: `crates/dozer-app/src/theme/color.rs`
- Modify: `crates/dozer-app/src/theme/mod.rs`

**Interfaces:**
- Consumes: Task 1-20 已完成(全仓库零处引用本地 `theme::color` 模块)
- Produces: `dozer-app` 完全依赖 `byteui::theme::color`,不再有重复源码

- [ ] **Step 1: 确认全仓库已无残留引用(收尾前的最后一道保险)**

Run: `grep -rn "theme::color::[A-Z]\|\bcolor::[A-Z]" crates/dozer-app/src --include="*.rs" | grep -v "crates/dozer-app/src/theme/color.rs"`
Expected: 无输出(如果有输出,说明 Task 1-20 里漏了某个调用点,先回去补上,不要继续本 Task)

- [ ] **Step 2: 删除本地重复文件**

```bash
rm crates/dozer-app/src/theme/color.rs
```

- [ ] **Step 3: 清理 `theme/mod.rs` 里的 `pub mod color;`**

`crates/dozer-app/src/theme/mod.rs` 里删除:

```rust
pub mod color;
```

- [ ] **Step 4: 全量编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer`
Expected: 编译成功;测试结果与迁移前基线一致(484 passed / 2 failed——两个已知的、与本次改动无关的 terminal grid 尺寸测试失败)

- [ ] **Step 5: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 无警告、无格式差异

- [ ] **Step 6: 独立临时二进制全面视觉核对**

构建一个独立命名的临时二进制(不要用会撞到用户正在跑的正式 `/Applications/
Dozer AI Coder.app` 或其他调试会话进程名的路径),启动后核对:

- 四栏骨架背景色、边框色。
- 各面板文字颜色(标题/正文/次要文字/dim 态)。
- 状态色(成功绿/失败红/进行中青/警示橙,尤其 `acceptance.rs`/`todo.rs`/
  `usage.rs` 这几个用色最密集的面板)。
- 顶栏选中页签描边/背景。
- `region.rs` 驱动的各区域背景色(顶栏/左右 icon rail/各 pane)——单独确认
  一遍,这是 Task 20 唯一覆盖的文件。

完成后关闭该临时实例、删除临时二进制,不留后台进程。

- [ ] **Step 7: Commit**

```bash
git branch --show-current
git add -A
git commit -m "refactor(dozer-app): 删除 theme::color 本地重复实现,改用 byteui"
```

---

## 完工验收

1. `git log --oneline` 确认全部 21 个 commit 都在当前分支上,没有漂到 `main`。
2. `grep -rn "theme::color::[A-Z]\|\bcolor::[A-Z]" crates/dozer-app/src` 全仓库零匹配(排除已删除的 `theme/color.rs`)。
3. `crates/dozer-app/src/theme/color.rs` 已删除。
4. 提请审阅(参考 spec"排期备注"——`theme::font`+`theme::geometry`/`theme::icon_size` 两个后续迁移子项目不在这次验收范围内)。
