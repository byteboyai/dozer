# workspace.rs 字号 Token 化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `workspace.rs` 里 87 处零散的 `text(...).size(N)` 字面量收敛成 8 个具名字号 token，编译期从 `assets/theme/fonts.json` 加载，镜像既有 `chrome_style.rs`/`regions.json` 的模式。

**Architecture:** 新模块 `crates/dozer-app/src/font_style.rs`（`include_str!` 编译期内嵌 JSON，`LazyLock` 启动时解析一次，暴露 8 个 `pub fn xxx() -> u32` 访问函数）+ `workspace.rs` 里 87 处调用点从 `.size(N)` 改为 `.size(font_style::xxx())`。纯代码搬家，不改变任何渲染数值。

**Tech Stack:** Rust、serde/serde_json（`dozer-app` 已有依赖，无需新增）、iced 0.14（`text().size(impl Into<Pixels>)`，`Pixels` 仅对 `f32`/`u32` 实现 `From`，故 token 返回类型定为 `u32`）。

## Global Constraints

- 数值本身不变——8 个 token 的值与迁移前字面量逐一相等（8/9/10/11/12/13/14/15），不合并、不新增档位。
- `font_style.rs` 与 `chrome_style.rs` 职责严格分离，不合并进同一模块/同一 JSON 文件（`chrome_style.rs` 头部注释已声明"区域内部控件样式不在这里"）。
- JSON 解析失败（格式错误、缺字段）直接 `panic`，不做运行时降级——与 `chrome_style.rs`/`theme.rs` 现有哲学一致。
- 不涉及 `term_view.rs` 终端字号、字体家族、字重、行高——本轮只做字号这一个维度。
- 单主题（ByteBoy2077），扁平文件结构（`assets/theme/fonts.json`），不为多主题切换预留目录层级——一期范围（`docs/superpowers/specs/2026-07-14-dozer-phase1-design.md` 第 46/115 行）明确排除多主题。

参考规格：`docs/superpowers/specs/2026-08-04-workspace-font-size-tokens-design.md`

---

## Task 1: `fonts.json` + `font_style.rs` 模块

**Files:**
- Create: `crates/dozer-app/assets/theme/fonts.json`
- Create: `crates/dozer-app/src/font_style.rs`
- Modify: `crates/dozer-app/src/main.rs`（新增 `mod font_style;`）
- Test: `crates/dozer-app/src/font_style.rs`（`#[cfg(test)] mod tests`，同文件内嵌）

**Interfaces:**
- Consumes: 无（本任务是叶子模块，不依赖 `workspace.rs`/`chrome_style.rs`）。
- Produces: `pub fn dot_xs() -> u32`、`pub fn dot_sm() -> u32`、`pub fn caption_sm() -> u32`、`pub fn caption() -> u32`、`pub fn label() -> u32`、`pub fn body() -> u32`、`pub fn subtitle() -> u32`、`pub fn title() -> u32`——Task 2 迁移 `workspace.rs` 时调用这 8 个函数。

- [ ] **Step 1: 写 `fonts.json`**

```json
{
  "dot_xs": 8,
  "dot_sm": 9,
  "caption_sm": 10,
  "caption": 11,
  "label": 12,
  "body": 13,
  "subtitle": 14,
  "title": 15
}
```

- [ ] **Step 2: 写 `font_style.rs`（实现 + 测试一并写完，模块过于薄，拆两步无收益）**

```rust
// crates/dozer-app/src/font_style.rs
//! 正文字号 token 化：`workspace.rs` 里散落的 `text(...).size(N)` 字面量收敛
//! 成 8 个具名 token，编译期内嵌 `assets/theme/fonts.json`，启动时解析一次。
//!
//! 与 `chrome_style.rs`（管区域外层容器的背景/边框/间距）职责严格分离——
//! 这里只管控件内部文字字号，不越界。解析失败（格式错误、缺字段）直接
//! panic：开发期配置错误，不是需要优雅降级的运行时数据（同 `chrome_style.rs`
//! 的定位）。
use serde::Deserialize;
use std::sync::LazyLock;

const RAW: &str = include_str!("../assets/theme/fonts.json");

#[derive(Deserialize)]
struct FontSizes {
    dot_xs: u32,
    dot_sm: u32,
    caption_sm: u32,
    caption: u32,
    label: u32,
    body: u32,
    subtitle: u32,
    title: u32,
}

fn load(raw: &str) -> FontSizes {
    serde_json::from_str(raw).expect("fonts.json 格式错误(解析失败)")
}

static SIZES: LazyLock<FontSizes> = LazyLock::new(|| load(RAW));

pub fn dot_xs() -> u32 {
    SIZES.dot_xs
}
pub fn dot_sm() -> u32 {
    SIZES.dot_sm
}
pub fn caption_sm() -> u32 {
    SIZES.caption_sm
}
pub fn caption() -> u32 {
    SIZES.caption
}
pub fn label() -> u32 {
    SIZES.label
}
pub fn body() -> u32 {
    SIZES.body
}
pub fn subtitle() -> u32 {
    SIZES.subtitle
}
pub fn title() -> u32 {
    SIZES.title
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚:8 个 token 的解析结果必须和改动前 workspace.rs 里的
    /// 字面量完全一致——纯代码搬家,数值不该变。
    #[test]
    fn tokens_match_pre_migration_literals() {
        assert_eq!(dot_xs(), 8);
        assert_eq!(dot_sm(), 9);
        assert_eq!(caption_sm(), 10);
        assert_eq!(caption(), 11);
        assert_eq!(label(), 12);
        assert_eq!(body(), 13);
        assert_eq!(subtitle(), 14);
        assert_eq!(title(), 15);
    }

    #[test]
    #[should_panic(expected = "fonts.json 格式错误")]
    fn malformed_json_panics() {
        load(r#"{"dot_xs": 8}"#);
    }
}
```

- [ ] **Step 3: 在 `main.rs` 注册模块**

在 `crates/dozer-app/src/main.rs` 里，`mod fonts;`（第 5 行，字体消毒模块，勿与本任务混淆）前面按字母序插入：

```rust
mod font_style;
```

插入后第 1-6 行应为：

```rust
mod assets;
mod chrome_style;
mod conversation;
mod delivery;
mod font_style;
mod fonts;
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app font_style`
Expected: `tokens_match_pre_migration_literals` 与 `malformed_json_panics` 均 `ok`，无编译错误。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/assets/theme/fonts.json crates/dozer-app/src/font_style.rs crates/dozer-app/src/main.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): add font-size token module (fonts.json)

Introduces 8 named font-size tokens (dot_xs..title) loaded from a
compiled-in JSON file, mirroring the existing chrome_style.rs pattern.
Not yet consumed anywhere — workspace.rs migration is a separate task.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: 迁移 `workspace.rs` 87 处 `.size(N)` 调用点

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: Task 1 产出的 `font_style::dot_xs()`/`dot_sm()`/`caption_sm()`/`caption()`/`label()`/`body()`/`subtitle()`/`title()`（均 `-> u32`）。
- Produces: 无新公开接口——这是纯调用点替换。

- [ ] **Step 1: 加 import**

在 `crates/dozer-app/src/workspace.rs:32-46` 的 `use crate::` 块里，按字母序在 `use crate::delivery::{...};`（第 34 行）之后、`use crate::goal::{...};`（第 35 行）之前插入：

```rust
use crate::font_style;
```

- [ ] **Step 2: 迁移前记录基线数量**

Run: `grep -c '\.size(' crates/dozer-app/src/workspace.rs`
Expected: `87`

- [ ] **Step 3: 脚本化替换全部 8 个数值**

这 8 个模式已在设计阶段核实为 `workspace.rs` 里 `.size(` 调用的全集（无其他非文本用途的 `.size(`调用、无三位数干扰值），可安全整文件替换：

```bash
sed -i '' \
  -e 's/\.size(15)/.size(font_style::title())/g' \
  -e 's/\.size(14)/.size(font_style::subtitle())/g' \
  -e 's/\.size(13)/.size(font_style::body())/g' \
  -e 's/\.size(12)/.size(font_style::label())/g' \
  -e 's/\.size(11)/.size(font_style::caption())/g' \
  -e 's/\.size(10)/.size(font_style::caption_sm())/g' \
  -e 's/\.size(9)/.size(font_style::dot_sm())/g' \
  -e 's/\.size(8)/.size(font_style::dot_xs())/g' \
  crates/dozer-app/src/workspace.rs
```

- [ ] **Step 4: 验证替换完整、无残留裸字面量**

Run:
```bash
grep -cE '\.size\(font_style::' crates/dozer-app/src/workspace.rs
grep -E '\.size\([0-9]+\)' crates/dozer-app/src/workspace.rs
```
Expected: 第一条输出 `87`；第二条无输出（zero matches，说明 8 个目标数值已全部替换，且未误伤非目标字面量——因为目标 8 个值本来就是文件里 `.size(` 的全集）。

- [ ] **Step 5: 格式化并检查行宽**

`.size(font_style::title())` 比 `.size(15)` 长，个别调用点可能超出 rustfmt 默认行宽需要重新换行：

Run: `cargo fmt -p dozer-app`
Run: `cargo fmt -p dozer-app --check`
Expected: 第二条无输出（无残留格式差异）。

- [ ] **Step 6: 编译 + 测试 + clippy**

Run: `cargo build -p dozer-app`
Expected: 编译通过，无 `font_style` 相关警告/错误。

Run: `cargo test -p dozer-app`
Expected: 全部既有测试通过（包括 Task 1 的 `font_style::tests`、既有 `chrome_style::tests`），无回归。

Run: `cargo clippy -p dozer-app --all-targets`
Expected: 无新增 warning。

- [ ] **Step 7: 真机视觉验收**

Run: `cargo run -p dozer-app`
打开应用，目测顶栏标题、对话列表、文件树、状态圆点等改动过字号的区域——应与迁移前像素级一致（纯搬家，无视觉变化）。确认后关闭应用。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "$(cat <<'EOF'
refactor(dozer-app): migrate workspace.rs text sizes to font_style tokens

Replaces all 87 .size(N) literals with named font_style:: accessors
(dot_xs..title). Pixel-identical — values unchanged, pure call-site swap.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: 收尾核对

**Files:** 无新增/修改，仅验证。

**Interfaces:** 无。

- [ ] **Step 1: 全量 workspace 编译核对**

Run: `cargo build --workspace`
Expected: 全 workspace（`dozer-core`/`dozerd`/`dozer-app`/`dozer-hook`/`legacy-boy`）编译通过，无跨 crate 回归（本轮改动只涉及 `dozer-app`，此步用于确认没有意外触碰其他 crate）。

- [ ] **Step 2: 确认无遗留裸字号字面量**

Run: `grep -rn '\.size([0-9]' crates/dozer-app/src/`
Expected: 无输出（`workspace.rs` 已全部迁移，且此前已确认全仓库仅 `workspace.rs` 使用过这种字面量）。

- [ ] **Step 3: 最终 fmt + clippy 全量检查**

Run: `cargo fmt --check && cargo clippy --all-targets`
Expected: 两者均无输出/无 warning（若发现与本轮改动无关的既有 fmt 差异，不在本任务修复范围，跳过即可）。
