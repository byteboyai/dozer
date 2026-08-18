# byteui 组件库 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新建 `crates/byteui`,一个不依赖 `dozer-core`/`dozer-app` 的独立 iced 0.14 组件库:平移 `dozer-app` 现有的设计 token(`theme::color`)与已验证的交互内核(`icon_button_entry`/`tab_core`/`cards`/`scrollbar`),并补齐 amis 命名对齐的 v1 组件(布局/表单/反馈/数据展示四类)。

**Architecture:** 单一 crate,模块按 `theme`(token)→ `interaction`(平移的交互内核)→ `layout`/`form`/`feedback`/`data`(amis 命名对齐的新组件)分层。所有组件是无状态 `fn(参数) -> Element<Message, iced_widget::Theme, iced_renderer::Renderer>`,`Message: Clone`;唯一的状态例外是 `feedback::toast::ToastQueue`,一个不碰 iced 类型的纯数据结构。颜色 token 做成运行时可替换的全局单例(`theme::color::current()`/`set_theme()`),`ByteBoy2077` 是编译期默认值。

**Tech Stack:** Rust 2024,`iced_widget`/`iced_renderer` 0.14(与 `dozer-app` 对齐),`serde`/`serde_json`(编译期内嵌 `assets/theme/workspace.json`)。

**Spec:** `docs/superpowers/specs/2026-08-18-byteui-library-design.md`

## Global Constraints

- **独立分支开发,不直接提交 main。** 在新分支(如 `feature/byteui-library`)上完成全部 9 个 Task,提请审阅、通过后再合并回 `main`——不要求/默认在 `main` 上直接提交(见 `[[feedback-plans-use-worktree-branch]]`)。若使用 subagent-driven-development 派发,dispatch 消息第一句必须是 `cd <worktree 绝对路径> && git branch --show-current` 校验,且 Read/Edit 的 `file_path` 参数必须带完整 worktree 绝对路径前缀(见 `[[feedback-sdd-implementer-main-drift]]`——该环境已知会话有"实现者漂移回 main"的复发历史,校验步骤降低概率但不保证杜绝,控制者收到完工汇报后必须自己 `git log --oneline -3` 核实产出 commit 真的在 worktree 分支上)。
- **`byteui` 不依赖 `dozer-core`、`dozer-app`、`dozer-client`。** 唯一现状里跨了这条线的是 `icon_size.rs` 的 `dozer_core::paths::config_dir()`,Task 4 改成调用方传 `&Path`。
- **v1 只让 `color` token 运行时可替换。** `font`/`geometry`/`icon_size`/`region` 保持现状"编译期从 `workspace.json` 解析、只读 `pub fn` 访问"的模式,不做运行时替换——这是相对 spec 架构描述(统一 `Theme{color,font,geometry,icon_size}`)的一处范围收窄:brainstorming 会话里"可替换主题"的诉求具体落在"金/青/绿/深色背景"这些颜色维度,字号/间距的跨产品差异化需求目前不存在,先不做违反 YAGNI。后续如果出现真实需求,照 `color` 这次的模式(`XxxTokens` struct + `current()`/`set_theme()`)扩展 `font`/`geometry` 即可,接口形状已经立好先例。
- **不做 JSON schema 驱动 UI、不做完整 amis Table、不做 Badge/Tag、不引入 HBox。** 均为 spec"非目标"一节明确排除,任何 Task 都不应该往这些方向发散。
- **不迁移 `dozer-app`。** 这次 9 个 Task 全部只在 `crates/byteui` 内部完成,不touch `crates/dozer-app` 下任何文件——`dozer-app` 的 `icons.rs`/`tabs.rs`/`theme/*` 原样保留,迁移是这个 plan 之外的后续任务。
- **颜色值锁死,禁止改动。** `theme::color::ColorTokens::byteboy2077()` 里的每个十六进制值必须和 `crates/dozer-app/src/theme/color.rs` 现有值逐一对应(这是防漂移要求,不是这次改动的自由度)。

---

## 文件结构总览

```
crates/byteui/
├── Cargo.toml
├── assets/theme/workspace.json          # Task 1:从 dozer-app 原样复制
└── src/
    ├── lib.rs                            # 每个 Task 追加一行 `pub mod`
    ├── theme/
    │   ├── mod.rs                        # Task 2
    │   ├── color.rs                      # Task 2(新写:ColorTokens + current/set_theme)
    │   ├── font.rs                       # Task 3(verbatim 迁移)
    │   ├── geometry.rs                   # Task 3(verbatim 迁移)
    │   └── icon_size.rs                  # Task 4(迁移 + dozer_core 摘除)
    ├── interaction/
    │   ├── mod.rs                        # Task 5
    │   ├── icons.rs                      # Task 5(迁移 + color 调用点改写)
    │   ├── tabs.rs                       # Task 5(verbatim 迁移)
    │   ├── cards.rs                      # Task 5(迁移 + color 调用点改写)
    │   └── scrollbar.rs                  # Task 5(迁移 + color 调用点改写)
    ├── layout/
    │   ├── mod.rs, divider.rs, panel.rs, wrapper.rs, flex.rs   # Task 6
    ├── form/
    │   ├── mod.rs, checkbox.rs, switch.rs, input_text.rs, select.rs   # Task 7
    ├── feedback/
    │   ├── mod.rs, status.rs, progress.rs, toast.rs             # Task 8
    └── data/
        ├── mod.rs, card.rs, list.rs, property.rs                # Task 9
```

`theme::region.rs` **不迁移**——spec 的 crate 结构表里列了它,但检查后发现它是 `dozer-app` 顶栏/面板"整体区域样式"的强业务耦合(`top_bar`/`left_icon_rail`/`preview_pane` 这些字段名本身就是 dozer-app 的面板划分,不是通用组件库该有的抽象),不属于"设计 token"也不属于任何 v1 组件——放进 `byteui` 会立刻违反"不内置 Dozer 专属业务约定"这条 Global Constraint。这是本 plan 相对 spec 文件树的一处修正,记录在此供审阅。

---

### Task 1: crate 脚手架

**Files:**
- Create: `crates/byteui/Cargo.toml`
- Create: `crates/byteui/src/lib.rs`
- Create: `crates/byteui/assets/theme/workspace.json`

**Interfaces:**
- Consumes: 无(第一个 Task)
- Produces: `byteui` crate 本身可编译。后续 Task 都在 `crates/byteui/src/` 下加文件。

- [ ] **Step 1: 创建 Cargo.toml**

```toml
[package]
name = "byteui"
version = "0.1.0"
edition = "2024"
license = "MIT"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
iced_widget = { version = "0.14", features = ["svg"] }
iced_renderer = "0.14"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: 复制 workspace.json**

```bash
mkdir -p crates/byteui/assets/theme
cp crates/dozer-app/assets/theme/workspace.json crates/byteui/assets/theme/workspace.json
```

- [ ] **Step 3: 创建空 lib.rs**

```rust
//! byteui:ByteBoy 系产品共用的 iced 0.14 UI 组件库。
//! 组件命名对齐 amis(<https://baidu.github.io/amis/zh-CN/components>)的
//! 分类与组件名——查对应 amis 文档页面即可理解组件的大致职责边界。
```

- [ ] **Step 4: 确认 workspace 自动纳入新 crate**

`Cargo.toml`(仓库根)的 `[workspace] members = ["crates/*", "spike/*"]` 是 glob,新目录会被自动吸纳,不需要改动根 `Cargo.toml`。运行以下命令确认:

Run: `cargo metadata --no-deps --format-version 1 | python3 -c "import json,sys; print('byteui' in [p['name'] for p in json.load(sys.stdin)['packages']])"`
Expected: `True`

- [ ] **Step 5: 编译验证**

Run: `cargo build -p byteui`
Expected: 编译成功(空 crate,无警告)

- [ ] **Step 6: Commit**

```bash
git add crates/byteui/Cargo.toml crates/byteui/src/lib.rs crates/byteui/assets/theme/workspace.json
git commit -m "feat(byteui): 建 crate 脚手架"
```

---

### Task 2: `theme::color`(可运行时替换的颜色 token)

**Files:**
- Modify: `crates/byteui/src/lib.rs`
- Create: `crates/byteui/src/theme/mod.rs`
- Create: `crates/byteui/src/theme/color.rs`

**Interfaces:**
- Consumes: 无
- Produces: `pub struct byteui::theme::color::ColorTokens`(23 个 `pub` 字段,列在下方代码里)、`pub fn current() -> ColorTokens`、`pub fn set_theme(tokens: ColorTokens)`、`pub fn mix(a: Color, b: Color, t: f32) -> Color`。Task 5/6/7/8/9 全部通过 `crate::theme::color::current().<field>` 取色,不出现硬编码色值。

- [ ] **Step 1: 写失败测试**

`crates/byteui/src/theme/color.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byteboy2077_bg_matches_hex() {
        let t = ColorTokens::byteboy2077();
        assert_eq!(t.bg.r, 0x0a as f32 / 255.0);
        assert_eq!(t.bg.g, 0x0e as f32 / 255.0);
        assert_eq!(t.bg.b, 0x16 as f32 / 255.0);
    }

    #[test]
    fn current_defaults_to_byteboy2077() {
        let c = current();
        assert_eq!(c.gold, ColorTokens::byteboy2077().gold);
    }

    #[test]
    fn set_theme_replaces_current_and_is_visible_globally() {
        let mut custom = ColorTokens::byteboy2077();
        custom.gold = Color::from_rgb8(0x00, 0x00, 0x00);
        set_theme(custom);
        assert_eq!(current().gold, Color::from_rgb8(0x00, 0x00, 0x00));
        // 复原,避免污染同进程里跑在本测试之后的其它测试。
        set_theme(ColorTokens::byteboy2077());
    }

    #[test]
    fn mix_at_zero_and_one_returns_endpoints() {
        let a = Color::from_rgb8(0, 0, 0);
        let b = Color::from_rgb8(255, 255, 255);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p byteui theme::color -- --test-threads=1`
Expected: FAIL,`ColorTokens`/`current`/`set_theme`/`mix` 未定义

- [ ] **Step 3: 写实现**(接在 Step 1 的 `#[cfg(test)]` 块之前)

```rust
//! ByteBoy2077 是编译期默认值,`set_theme` 可在运行时整体替换成另一份
//! 产品的取值——组件内部一律读 `current()`,不再有硬编码色值常量。

use iced_widget::core::Color;
use std::sync::RwLock;

const fn c(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorTokens {
    pub bg: Color,
    pub panel: Color,
    pub term_bg: Color,
    pub card: Color,
    pub border: Color,
    pub cream: Color,
    pub body: Color,
    pub dim: Color,
    pub gold: Color,
    pub cyan: Color,
    pub green: Color,
    pub purple: Color,
    pub red: Color,
    pub ignored: Color,
    pub orange: Color,
    pub magenta: Color,
    pub blue: Color,
    pub lime: Color,
    pub scrim: Color,
    pub tab_active_border: Color,
    pub tab_active_bg: Color,
    pub tab_hover: Color,
    pub desc_bg: Color,
}

impl ColorTokens {
    /// 逐一对应 `crates/dozer-app/src/theme/color.rs` 的锁死值,禁止改动。
    pub const fn byteboy2077() -> Self {
        Self {
            bg: c(0x0a, 0x0e, 0x16),
            panel: c(0x0a, 0x0e, 0x16),
            term_bg: c(0x08, 0x14, 0x1d),
            card: c(0x12, 0x20, 0x2a),
            border: c(0x1c, 0x34, 0x40),
            cream: c(0xFF, 0xE5, 0xB4),
            body: c(0x9A, 0xB4, 0xC4),
            dim: c(0x6B, 0x7F, 0x8F),
            gold: c(0xF2, 0xD9, 0x4E),
            cyan: c(0x47, 0xDE, 0xF0),
            green: c(0x1A, 0xD5, 0x85),
            purple: c(0x95, 0x80, 0xFF),
            red: c(0xFF, 0x6E, 0x6E),
            ignored: c(0x6B, 0x7F, 0x8F),
            orange: c(0xFF, 0x9B, 0x4D),
            magenta: c(0xFF, 0x6E, 0xC7),
            blue: c(0x4D, 0x8C, 0xFF),
            lime: c(0xA3, 0xE6, 0x35),
            scrim: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.55 },
            tab_active_border: c(0xDC, 0xC9, 0xA3),
            tab_active_bg: c(0x15, 0x26, 0x30),
            tab_hover: c(0x15, 0x26, 0x30),
            desc_bg: c(0x15, 0x26, 0x30),
        }
    }
}

static CURRENT: RwLock<ColorTokens> = RwLock::new(ColorTokens::byteboy2077());

/// 当前生效的颜色 token(默认 ByteBoy2077)。
pub fn current() -> ColorTokens {
    *CURRENT.read().expect("byteui color RwLock poisoned")
}

/// 整体替换当前颜色 token——供未来 ByteBoy 产品换主题用,组件代码不用改。
pub fn set_theme(tokens: ColorTokens) {
    *CURRENT.write().expect("byteui color RwLock poisoned") = tokens;
}

/// 两色按 `t`(0..=1)线性插值。`t` 超出 [0,1] 不外夹,调用方保证区间。
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}
```

- [ ] **Step 4: 创建 `theme/mod.rs`**

```rust
pub mod color;
```

- [ ] **Step 5: `lib.rs` 加一行**

```rust
pub mod theme;
```

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test -p byteui theme::color -- --test-threads=1`
Expected: PASS(4 个测试;`--test-threads=1` 是因为 `set_theme` 改写进程级全局状态,并行跑会互相污染)

- [ ] **Step 7: Commit**

```bash
git add crates/byteui/src/lib.rs crates/byteui/src/theme/mod.rs crates/byteui/src/theme/color.rs
git commit -m "feat(byteui): theme::color 可替换单例,ByteBoy2077 默认值"
```

---

### Task 3: `theme::font` + `theme::geometry`(verbatim 迁移)

**Files:**
- Modify: `crates/byteui/src/theme/mod.rs`
- Create: `crates/byteui/src/theme/font.rs`
- Create: `crates/byteui/src/theme/geometry.rs`

**Interfaces:**
- Consumes: 无(不读 `theme::color`,两个模块互相独立)
- Produces: `crates/dozer-app/src/theme/font.rs`/`geometry.rs` 现有的全部 `pub fn`(`font::body()`/`font::caption()`/...、`geometry::rail_button_size()`/`geometry::scrollbar_width()`/... 等)原样可用,签名不变。Task 6/7/8/9 直接调这些函数排版。

- [ ] **Step 1: 逐字节复制两个文件**

```bash
cp crates/dozer-app/src/theme/font.rs crates/byteui/src/theme/font.rs
cp crates/dozer-app/src/theme/geometry.rs crates/byteui/src/theme/geometry.rs
```

两个文件内部只有 `include_str!("../../assets/theme/workspace.json")`(相对路径,Task 1 已经在 `crates/byteui/assets/theme/workspace.json` 放好同名文件,不用改路径)和纯 JSON 解析逻辑,不引用 `crate::` 下任何其它模块、不引用 `dozer_core`(已用 `grep -n "dozer_core\|use crate::" crates/dozer-app/src/theme/font.rs crates/dozer-app/src/theme/geometry.rs` 确认过,零匹配),复制后不需要额外编辑。

- [ ] **Step 2: `theme/mod.rs` 加两行**

```rust
pub mod font;
pub mod geometry;
```

- [ ] **Step 3: 运行测试(含两个文件原有的防漂移锚测试)**

Run: `cargo test -p byteui theme::font theme::geometry`
Expected: PASS,和 `cargo test -p dozer-app theme::font theme::geometry` 迁移前的结果逐条一致(纯代码搬家,数值不该变)

- [ ] **Step 4: Commit**

```bash
git add crates/byteui/src/theme/mod.rs crates/byteui/src/theme/font.rs crates/byteui/src/theme/geometry.rs
git commit -m "feat(byteui): 迁移 theme::font/geometry(verbatim)"
```

---

### Task 4: `theme::icon_size`(迁移 + 摘除 `dozer_core` 依赖)

**Files:**
- Modify: `crates/byteui/src/theme/mod.rs`
- Create: `crates/byteui/src/theme/icon_size.rs`

**Interfaces:**
- Consumes: 无
- Produces: `pub fn rail/row/chevron/tab_arrow/home/tree_row_gap() -> f32`(原样)、`pub fn scale() -> f32`、`pub fn set_scale(f32)`、`pub fn zoom_by(f32)`、`pub fn reset_scale()`(原样)、**签名变化**:`pub fn init_scale(path: &std::path::Path)`、`pub fn persist_scale(path: &std::path::Path)`(原本无参,现在调用方传路径)。`pub const SCALE_MIN: f32`/`SCALE_MAX: f32` 原样。

- [ ] **Step 1: 复制文件**

```bash
cp crates/dozer-app/src/theme/icon_size.rs crates/byteui/src/theme/icon_size.rs
```

- [ ] **Step 2: 摘除 `dozer_core` 依赖,`init_scale`/`persist_scale` 改吃 `&Path`**

在 `crates/byteui/src/theme/icon_size.rs` 里做以下精确替换:

```rust
// 删除这一行(scale_path() 内部用它拼路径,函数本身这步一起删):
// fn scale_path() -> PathBuf {
//     dozer_core::paths::config_dir().join(SCALE_FILE_NAME)
// }
```

把:

```rust
fn load_persisted_scale() -> Option<f32> {
    load_from(&scale_path())
}
```

```rust
fn save_persisted_scale(v: f32) {
    let _ = save_to(&scale_path(), v);
}
```

```rust
pub fn init_scale() {
    if std::env::var("DOZER_ICON_SCALE").is_ok() {
        return;
    }
    if let Some(v) = load_persisted_scale() {
        set_scale(v);
    }
}
```

```rust
pub fn persist_scale() {
    save_persisted_scale(scale());
}
```

```rust
pub fn reset_scale() {
    CURRENT_SCALE.store(u32::MAX, Ordering::Relaxed);
    save_persisted_scale(SIZES.scale);
}
```

改成:

```rust
fn load_persisted_scale(path: &Path) -> Option<f32> {
    load_from(path)
}

fn save_persisted_scale(path: &Path, v: f32) {
    let _ = save_to(path, v);
}

/// 启动时把上次退出前落盘的 scale 读回并应用为当前值,调用方传入落盘路径
/// (`byteui` 不内置任何 Dozer 专属路径约定)。`DOZER_ICON_SCALE` 环境变量
/// 是显式覆盖,优先级高于落盘值。
pub fn init_scale(path: &Path) {
    if std::env::var("DOZER_ICON_SCALE").is_ok() {
        return;
    }
    if let Some(v) = load_persisted_scale(path) {
        set_scale(v);
    }
}

/// 把当前 scale 落盘到调用方指定的路径,供下次启动 `init_scale` 读回。
pub fn persist_scale(path: &Path) {
    save_persisted_scale(path, scale());
}

/// 还原到启动基准 scale;同时把落盘值(调用方指定路径)复位成出厂默认。
pub fn reset_scale(path: &Path) {
    CURRENT_SCALE.store(u32::MAX, Ordering::Relaxed);
    save_persisted_scale(path, SIZES.scale);
}
```

并删除 `scale_path()` 函数本身、以及文件顶部 `const SCALE_FILE_NAME: &str = "ui_scale.json";` 相关的路径拼接逻辑(常量本身可以保留,只是不再被 `scale_path()` 使用;调用方决定文件名的一部分,或者干脆删掉这个常量——两种都可以,选择删掉常量,因为它现在没有任何代码引用了)。

- [ ] **Step 3: 修 `mod tests` 里跟着变签名的调用点**

原测试里 `init_scale()`/`persist_scale()`/`reset_scale()` 无参调用、`scale_path()` 调用,现在需要改成显式传测试用临时路径。找到 `with_real_scale_file`/`init_applies_persisted_and_reset_clears_it` 这两个测试,把:

```rust
fn with_real_scale_file<F: FnOnce()>(f: F) {
    let real = scale_path();
    ...
}
```

改成显式接收路径参数:

```rust
fn with_temp_scale_file<F: FnOnce(&Path)>(f: F) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ui_scale.json");
    f(&path);
    CURRENT_SCALE.store(u32::MAX, Ordering::Relaxed);
}
```

```rust
#[test]
fn init_applies_persisted_and_reset_clears_it() {
    if std::env::var("DOZER_ICON_SCALE").is_ok() {
        return;
    }
    with_temp_scale_file(|path| {
        save_to(path, 2.0).unwrap();
        init_scale(path);
        assert_eq!(scale(), 2.0);

        reset_scale(path);
        assert_eq!(scale(), SIZES.scale);
        assert_eq!(load_from(path), Some(SIZES.scale));
    });
}
```

其余不碰真实文件系统路径的测试(`tokens_match_pre_migration_literals`/`malformed_json_panics`/`persisted_scale_round_trips_and_clamps`/`corrupted_or_missing_scale_file_returns_none`,后两个已经在用 `tempfile::tempdir()` 指向临时路径,和 `scale_path()` 无关)原样保留不动。

- [ ] **Step 4: `theme/mod.rs` 加一行**

```rust
pub mod icon_size;
```

- [ ] **Step 5: 运行测试**

Run: `cargo test -p byteui theme::icon_size -- --test-threads=1`
Expected: PASS(`--test-threads=1` 原因同 Task 2:`CURRENT_SCALE` 是进程级全局,并行跑会互相污染,这是从 `dozer-app` 迁移过来时就已有的既存约束)

- [ ] **Step 6: Commit**

```bash
git add crates/byteui/src/theme/mod.rs crates/byteui/src/theme/icon_size.rs
git commit -m "feat(byteui): 迁移 theme::icon_size,摘除 dozer_core 路径依赖"
```

---

### Task 5: `interaction` 模块(icons/tabs/cards/scrollbar 迁移)

**Files:**
- Modify: `crates/byteui/src/lib.rs`
- Create: `crates/byteui/src/interaction/mod.rs`
- Create: `crates/byteui/src/interaction/icons.rs`
- Create: `crates/byteui/src/interaction/tabs.rs`
- Create: `crates/byteui/src/interaction/cards.rs`
- Create: `crates/byteui/src/interaction/scrollbar.rs`

**Interfaces:**
- Consumes: `crate::theme::color::current()`(Task 2)、`crate::theme::font::body()`(Task 3)、`crate::theme::geometry::{scrollbar_width, scrollbar_thumb_width}`(Task 3)
- Produces: `interaction::icons::{IconKind, view, icon_button_entry, with_tooltip, tooltip_bubble_style, icon_for_file}`、`interaction::tabs::tab_core`、`interaction::cards::{CARD_RADIUS, card_border_color, card_background, button_card, container_card}`、`interaction::scrollbar::{scrollbar, scrollbar_style}`——供 Task 6/8/9 复用(`data::card` 会调 `interaction::cards::container_card`)。

- [ ] **Step 1: 复制四个文件**

```bash
cp crates/dozer-app/src/icons.rs crates/byteui/src/interaction/icons.rs
cp crates/dozer-app/src/tabs.rs crates/byteui/src/interaction/tabs.rs
cp crates/dozer-app/src/theme/cards.rs crates/byteui/src/interaction/cards.rs
cp crates/dozer-app/src/scrollbar.rs crates/byteui/src/interaction/scrollbar.rs
```

`tabs.rs` 不引用 `theme::color`(颜色由调用方算好传入),复制后**不需要任何编辑**。其余三个文件需要 Step 2/3/4 的替换。

- [ ] **Step 2: `icons.rs` 的 color 调用点改写**

`color::GOLD`/`color::DIM`/`color::CARD`/`color::BORDER`/`color::CREAM` 这五个常量在 `interaction/icons.rs` 里从 const 变成了要经 `theme::color::current()` 取的字段,做以下精确替换:

把:

```rust
    let color = if active {
        crate::theme::color::GOLD
    } else {
        crate::theme::color::mix(crate::theme::color::DIM, crate::theme::color::GOLD, hover_t)
    };
```

改成:

```rust
    let colors = crate::theme::color::current();
    let color = if active {
        colors.gold
    } else {
        crate::theme::color::mix(colors.dim, colors.gold, hover_t)
    };
```

把:

```rust
                background: if card {
                    Some(crate::theme::color::CARD.into())
                } else {
                    None
                },
                border: Border {
                    color: if active {
                        crate::theme::color::GOLD
                    } else {
                        Color::TRANSPARENT
                    },
                    ..base_border
                },
```

改成:

```rust
                background: if card {
                    Some(colors.card.into())
                } else {
                    None
                },
                border: Border {
                    color: if active {
                        colors.gold
                    } else {
                        Color::TRANSPARENT
                    },
                    ..base_border
                },
```

(这段在 `.style(move |_t, _status| ...)` 闭包内,`colors` 已在闭包外算好,`move` 闭包会捕获它,不需要在闭包内再调一次 `current()`。)

把 `tooltip_bubble_style` 里的:

```rust
pub fn tooltip_bubble_style() -> impl Fn(&iced_widget::Theme) -> container::Style {
    |_t: &iced_widget::Theme| container::Style {
        background: Some(iced_widget::core::Background::Color(
            crate::theme::color::CARD,
        )),
        border: Border {
            color: crate::theme::color::BORDER,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    }
}
```

改成:

```rust
pub fn tooltip_bubble_style() -> impl Fn(&iced_widget::Theme) -> container::Style {
    |_t: &iced_widget::Theme| {
        let colors = crate::theme::color::current();
        container::Style {
            background: Some(iced_widget::core::Background::Color(colors.card)),
            border: Border {
                color: colors.border,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        }
    }
}
```

把 `with_tooltip` 里的:

```rust
    let bubble = container(text(label).size(12).color(crate::theme::color::CREAM)).padding([5, 9]);
```

改成:

```rust
    let bubble =
        container(text(label).size(12).color(crate::theme::color::current().cream)).padding([5, 9]);
```

- [ ] **Step 3: `cards.rs` 的 color 调用点改写**

把:

```rust
use crate::theme::color;
```

...

```rust
pub fn card_border_color(selected: bool, hovered: bool) -> Color {
    if selected || hovered {
        color::GOLD
    } else {
        color::BORDER
    }
}
```

改成:

```rust
pub fn card_border_color(selected: bool, hovered: bool) -> Color {
    let colors = crate::theme::color::current();
    if selected || hovered {
        colors.gold
    } else {
        colors.border
    }
}
```

把 `button_card` 里的:

```rust
            text_color: color::CREAM,
```

改成:

```rust
            text_color: crate::theme::color::current().cream,
```

`use crate::theme::color;` 这行导入可以删掉(改写后不再直接用 `color::` 前缀,全部经 `crate::theme::color::current()`)。

- [ ] **Step 4: `scrollbar.rs` 的 color 调用点改写**

把:

```rust
pub fn scrollbar_style() -> scrollable::Style {
    let scroller = scrollable::Scroller {
        background: Background::Color(theme::color::TAB_ACTIVE_BORDER),
```

改成:

```rust
pub fn scrollbar_style() -> scrollable::Style {
    let scroller = scrollable::Scroller {
        background: Background::Color(theme::color::current().tab_active_border),
```

`use crate::theme;` 这行导入保留不动(`theme::geometry::scrollbar_width()`/`scrollbar_thumb_width()` 两处调用不受影响)。

- [ ] **Step 5: 创建 `interaction/mod.rs`**

```rust
pub mod cards;
pub mod icons;
pub mod scrollbar;
pub mod tabs;
```

- [ ] **Step 6: `lib.rs` 加一行**

```rust
pub mod interaction;
```

- [ ] **Step 7: 运行测试**

Run: `cargo test -p byteui interaction::`
Expected: PASS——`icons.rs` 原有的 6 个 `icon_for_file` 测试原样通过(这些测试不碰颜色,不受 Step 2 改写影响)

- [ ] **Step 8: 编译检查整个 crate 无警告**

Run: `cargo build -p byteui && cargo clippy -p byteui --all-targets`
Expected: 无 warning/error(尤其确认 Step 3 删掉 `use crate::theme::color;` 后没有遗留的死代码警告)

- [ ] **Step 9: Commit**

```bash
git add crates/byteui/src/lib.rs crates/byteui/src/interaction/
git commit -m "feat(byteui): 迁移 interaction(icons/tabs/cards/scrollbar),接入可替换 color token"
```

---

### Task 6: `layout` 模块(divider/panel/wrapper/flex)

**Files:**
- Modify: `crates/byteui/src/lib.rs`
- Create: `crates/byteui/src/layout/mod.rs`
- Create: `crates/byteui/src/layout/divider.rs`
- Create: `crates/byteui/src/layout/panel.rs`
- Create: `crates/byteui/src/layout/wrapper.rs`
- Create: `crates/byteui/src/layout/flex.rs`

**Interfaces:**
- Consumes: `crate::theme::color::current()`(Task 2)
- Produces: `layout::divider::{horizontal, vertical}`、`layout::panel::view`、`layout::wrapper::view`、`layout::flex::{Direction, view}`

- [ ] **Step 1: 写 `divider.rs` 的失败测试**

amis `divider.md` 语义是纯分隔线,没有可单测的逻辑分支(渲染输出是 `Element`,不可比较)。这个组件没有值得写的单元测试——用编译期类型检查代替:写一个 smoke test 只验证函数能在给定的具体 `Message` 类型下调用成功。

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    enum Msg {}

    #[test]
    fn horizontal_and_vertical_construct_without_panic() {
        let _: iced_widget::core::Element<Msg, iced_widget::Theme, iced_renderer::Renderer> =
            horizontal();
        let _: iced_widget::core::Element<Msg, iced_widget::Theme, iced_renderer::Renderer> =
            vertical();
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p byteui layout::divider`
Expected: FAIL,`horizontal`/`vertical` 未定义

- [ ] **Step 3: 写 `divider.rs` 实现**

```rust
//! amis `divider`(分隔线):<https://baidu.github.io/amis/zh-CN/components/divider>

use iced_widget::core::Element;
use iced_widget::rule::{self, FillMode};

pub fn horizontal<'a, Message: 'a>() -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    rule::horizontal(1.0)
        .style(|_theme: &iced_widget::Theme| rule::Style {
            color: crate::theme::color::current().border,
            radius: 0.0.into(),
            fill_mode: FillMode::Full,
            snap: true,
        })
        .into()
}

pub fn vertical<'a, Message: 'a>() -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    rule::vertical(1.0)
        .style(|_theme: &iced_widget::Theme| rule::Style {
            color: crate::theme::color::current().border,
            radius: 0.0.into(),
            fill_mode: FillMode::Full,
            snap: true,
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    enum Msg {}

    #[test]
    fn horizontal_and_vertical_construct_without_panic() {
        let _: Element<Msg, iced_widget::Theme, iced_renderer::Renderer> = horizontal();
        let _: Element<Msg, iced_widget::Theme, iced_renderer::Renderer> = vertical();
    }
}
```

(把 Step 1 单独写的 `mod tests` 挪进这个文件、去重,不要留两份。)

- [ ] **Step 4: 写 `panel.rs`**

```rust
//! amis `panel`(带描边圆角的信息容器):<https://baidu.github.io/amis/zh-CN/components/panel>

use iced_widget::container;
use iced_widget::core::{Border, Element};

pub fn view<'a, Message: 'a>(
    content: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(content)
        .padding(12)
        .style(|_theme: &iced_widget::Theme| {
            let colors = crate::theme::color::current();
            container::Style {
                background: Some(colors.card.into()),
                border: Border {
                    color: colors.border,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}
```

- [ ] **Step 5: 写 `wrapper.rs`**

```rust
//! amis `wrapper`(无装饰的间距包裹):<https://baidu.github.io/amis/zh-CN/components/wrapper>

use iced_widget::container;
use iced_widget::core::Element;

pub fn view<'a, Message: 'a>(
    content: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    padding: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(content).padding(padding).into()
}
```

- [ ] **Step 6: 写 `flex.rs`**

```rust
//! amis `flex`(css flex 排列封装):<https://baidu.github.io/amis/zh-CN/components/flex>

use iced_widget::core::Element;
use iced_widget::{Column, Row};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Row,
    Column,
}

pub fn view<'a, Message: 'a>(
    direction: Direction,
    gap: f32,
    children: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    match direction {
        Direction::Row => Row::with_children(children).spacing(gap).into(),
        Direction::Column => Column::with_children(children).spacing(gap).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_widget::core::text::Text;

    #[derive(Clone)]
    enum Msg {}

    #[test]
    fn view_row_and_column_construct_without_panic() {
        let child = || -> Element<'static, Msg, iced_widget::Theme, iced_renderer::Renderer> {
            Text::new("x").into()
        };
        let _ = view(Direction::Row, 8.0, vec![child(), child()]);
        let _ = view(Direction::Column, 8.0, vec![child(), child()]);
    }
}
```

- [ ] **Step 7: 创建 `layout/mod.rs`**

```rust
pub mod divider;
pub mod flex;
pub mod panel;
pub mod wrapper;
```

- [ ] **Step 8: `lib.rs` 加一行**

```rust
pub mod layout;
```

- [ ] **Step 9: 运行测试确认通过**

Run: `cargo test -p byteui layout::`
Expected: PASS(3 个 smoke test:`divider`/`flex`)

- [ ] **Step 10: Commit**

```bash
git add crates/byteui/src/lib.rs crates/byteui/src/layout/
git commit -m "feat(byteui): 新增 layout 模块(divider/panel/wrapper/flex)"
```

---

### Task 7: `form` 模块(checkbox/switch/input_text/select)

**Files:**
- Modify: `crates/byteui/src/lib.rs`
- Create: `crates/byteui/src/form/mod.rs`
- Create: `crates/byteui/src/form/checkbox.rs`
- Create: `crates/byteui/src/form/switch.rs`
- Create: `crates/byteui/src/form/input_text.rs`
- Create: `crates/byteui/src/form/select.rs`

**Interfaces:**
- Consumes: `crate::theme::color::current()`(Task 2)
- Produces: `form::checkbox::view`、`form::switch::view`、`form::input_text::view`、`form::select::view`

- [ ] **Step 1: 写 `checkbox.rs`**

```rust
//! amis `form/checkbox`(复选框):<https://baidu.github.io/amis/zh-CN/components/form/checkbox>

use iced_widget::checkbox::{self, Status};
use iced_widget::core::{Border, Element};

pub fn view<'a, Message: Clone + 'a>(
    label: &'a str,
    is_checked: bool,
    on_toggle: impl Fn(bool) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    iced_widget::checkbox(is_checked)
        .label(label)
        .on_toggle(on_toggle)
        .style(|_theme: &iced_widget::Theme, status: Status| {
            let colors = crate::theme::color::current();
            let checked = matches!(
                status,
                Status::Active { is_checked: true } | Status::Hovered { is_checked: true }
            );
            checkbox::Style {
                background: colors.card.into(),
                icon_color: colors.gold,
                border: Border {
                    color: if checked { colors.gold } else { colors.border },
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: Some(colors.cream),
            }
        })
        .into()
}
```

- [ ] **Step 2: 写 `switch.rs`**

```rust
//! amis `form/switch`(开关):<https://baidu.github.io/amis/zh-CN/components/form/switch>

use iced_widget::core::Element;
use iced_widget::toggler::{self, Status};

pub fn view<'a, Message: Clone + 'a>(
    label: &'a str,
    is_on: bool,
    on_toggle: impl Fn(bool) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    iced_widget::toggler(is_on)
        .label(label)
        .on_toggle(on_toggle)
        .style(|_theme: &iced_widget::Theme, status: Status| {
            let colors = crate::theme::color::current();
            let on = matches!(
                status,
                Status::Active { is_toggled: true } | Status::Hovered { is_toggled: true }
            );
            toggler::Style {
                background: if on { colors.gold.into() } else { colors.border.into() },
                background_border_width: 0.0,
                background_border_color: iced_widget::core::Color::TRANSPARENT,
                foreground: colors.cream.into(),
                foreground_border_width: 0.0,
                foreground_border_color: iced_widget::core::Color::TRANSPARENT,
                text_color: Some(colors.cream),
                border_radius: None,
                padding_ratio: 0.1,
            }
        })
        .into()
}
```

- [ ] **Step 3: 写 `input_text.rs`**

```rust
//! amis `form/input-text`(单行文本输入):<https://baidu.github.io/amis/zh-CN/components/form/input-text>

use iced_widget::core::{Border, Element};
use iced_widget::text_input::{self, Status};

pub fn view<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &str,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    iced_widget::text_input(placeholder, value)
        .on_input(on_input)
        .padding(8)
        .style(|_theme: &iced_widget::Theme, status: Status| {
            let colors = crate::theme::color::current();
            let focused = matches!(status, Status::Focused { .. });
            text_input::Style {
                background: colors.card.into(),
                border: Border {
                    color: if focused { colors.gold } else { colors.border },
                    width: 1.0,
                    radius: 6.0.into(),
                },
                icon: colors.dim,
                placeholder: colors.dim,
                value: colors.cream,
                selection: crate::theme::color::mix(colors.gold, colors.card, 0.6),
            }
        })
        .into()
}
```

- [ ] **Step 4: 写 `select.rs`**

```rust
//! amis `form/select`(下拉选择):<https://baidu.github.io/amis/zh-CN/components/form/select>

use iced_widget::core::{Border, Element};
use iced_widget::pick_list::{self, Status};

pub fn view<'a, T, Message>(
    options: &'a [T],
    selected: Option<&'a T>,
    on_select: impl Fn(T) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>
where
    T: ToString + PartialEq + Clone + 'a,
    Message: Clone + 'a,
{
    iced_widget::pick_list(options, selected, on_select)
        .padding(8)
        .style(|_theme: &iced_widget::Theme, _status: Status| {
            let colors = crate::theme::color::current();
            pick_list::Style {
                text_color: colors.cream,
                placeholder_color: colors.dim,
                handle_color: colors.gold,
                background: colors.card.into(),
                border: Border {
                    color: colors.border,
                    width: 1.0,
                    radius: 6.0.into(),
                },
            }
        })
        .into()
}
```

- [ ] **Step 5: 创建 `form/mod.rs`**

```rust
pub mod checkbox;
pub mod input_text;
pub mod select;
pub mod switch;
```

- [ ] **Step 6: `lib.rs` 加一行**

```rust
pub mod form;
```

- [ ] **Step 7: 写调用点测试(验证四个组件在具体 `Message` 类型下能通过编译并构造)**

`crates/byteui/src/form/mod.rs` 追加:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    enum Msg {
        Toggled(bool),
        Input(String),
        Selected(&'static str),
    }

    #[test]
    fn all_form_components_construct_without_panic() {
        let _ = checkbox::view("label", false, Msg::Toggled);
        let _ = switch::view("label", true, Msg::Toggled);
        let _ = input_text::view("placeholder", "value", Msg::Input);
        let options: &[&str] = &["a", "b"];
        let _ = select::view(options, Some(&"a"), Msg::Selected);
    }
}
```

- [ ] **Step 8: 运行测试**

Run: `cargo build -p byteui && cargo test -p byteui form::`
Expected: 编译通过(这一步是四个组件对着真实 iced 0.14 API 类型检查的实质验证——`Style`/`Status` 字段名/数量对不上会在这里直接编译失败),测试 PASS

- [ ] **Step 9: Commit**

```bash
git add crates/byteui/src/lib.rs crates/byteui/src/form/
git commit -m "feat(byteui): 新增 form 模块(checkbox/switch/input_text/select)"
```

---

### Task 8: `feedback` 模块(status/progress/toast)

**Files:**
- Modify: `crates/byteui/src/lib.rs`
- Create: `crates/byteui/src/feedback/mod.rs`
- Create: `crates/byteui/src/feedback/status.rs`
- Create: `crates/byteui/src/feedback/progress.rs`
- Create: `crates/byteui/src/feedback/toast.rs`

**Interfaces:**
- Consumes: `crate::theme::color::current()`(Task 2)、`crate::theme::font::body()`(Task 3)
- Produces: `feedback::status::{Kind, view}`、`feedback::progress::view`、`feedback::toast::{Kind, Toast, ToastQueue, view}`

- [ ] **Step 1: 写 `status.rs`**

```rust
//! amis `status`(成功/失败/进行中状态展示):<https://baidu.github.io/amis/zh-CN/components/status>

use iced_widget::core::{Color, Element};
use iced_widget::{container, row, text};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Success,
    Failed,
    Running,
    Idle,
}

impl Kind {
    fn color(self, colors: &crate::theme::color::ColorTokens) -> Color {
        match self {
            Kind::Success => colors.green,
            Kind::Failed => colors.red,
            Kind::Running => colors.cyan,
            Kind::Idle => colors.dim,
        }
    }
}

pub fn view<'a, Message: 'a>(
    kind: Kind,
    label: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = crate::theme::color::current();
    let dot_color = kind.color(&colors);
    row![
        container(text("●").size(8).color(dot_color)),
        text(label).size(crate::theme::font::body()).color(colors.cream),
    ]
    .spacing(6)
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .into()
}
```

- [ ] **Step 2: 写 `progress.rs`**

```rust
//! amis `progress`(进度条):<https://baidu.github.io/amis/zh-CN/components/progress>

use iced_widget::core::{Border, Color, Element};
use iced_widget::progress_bar;

/// `value` 是 0.0..=1.0 的完成度,超出范围会被夹到区间内。
pub fn view<'a, Message: 'a>(
    value: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    iced_widget::progress_bar(0.0..=1.0, value.clamp(0.0, 1.0))
        .girth(6.0)
        .style(|_theme: &iced_widget::Theme| {
            let colors = crate::theme::color::current();
            progress_bar::Style {
                background: colors.card.into(),
                bar: colors.gold.into(),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 3.0.into(),
                },
            }
        })
        .into()
}
```

- [ ] **Step 3: 写 `toast.rs` 的失败测试**

```rust
//! amis `toast`(轻提示):<https://baidu.github.io/amis/zh-CN/components/toast>
//!
//! `ToastQueue` 是纯数据结构,不碰 iced 事件循环——消费方(如 dozer-app 的
//! `State`)自己持有它,在已有的动画 tick 里调 `retain_active`,在需要弹
//! 提示的地方调 `push`,决定把 `view(&queue)` 结果叠在哪一层。

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Success,
    Error,
    Info,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub kind: Kind,
    pub message: String,
    expires_at: Instant,
}

#[derive(Default, Debug)]
pub struct ToastQueue {
    items: Vec<Toast>,
}

impl ToastQueue {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn push(&mut self, kind: Kind, message: impl Into<String>, ttl: Duration) {
        self.items.push(Toast {
            kind,
            message: message.into(),
            expires_at: Instant::now() + ttl,
        });
    }

    pub fn retain_active(&mut self, now: Instant) {
        self.items.retain(|t| t.expires_at > now);
    }

    pub fn items(&self) -> &[Toast] {
        &self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retain_active_removes_expired_and_keeps_fresh() {
        let mut q = ToastQueue::new();
        q.push(Kind::Info, "old", Duration::from_millis(0));
        q.push(Kind::Success, "fresh", Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(5));
        q.retain_active(Instant::now());
        assert_eq!(q.items().len(), 1);
        assert_eq!(q.items()[0].message, "fresh");
    }

    #[test]
    fn retain_active_on_empty_queue_is_noop() {
        let mut q = ToastQueue::new();
        q.retain_active(Instant::now());
        assert!(q.items().is_empty());
    }

    #[test]
    fn push_appends_without_removing_existing_items() {
        let mut q = ToastQueue::new();
        q.push(Kind::Info, "a", Duration::from_secs(60));
        q.push(Kind::Error, "b", Duration::from_secs(60));
        assert_eq!(q.items().len(), 2);
    }
}
```

- [ ] **Step 4: 运行测试确认通过**(这个文件测试和实现是一起写的,直接跑)

Run: `cargo test -p byteui feedback::toast`
Expected: PASS(3 个测试)

- [ ] **Step 5: 给 `toast.rs` 追加 `view` 渲染函数**(接在 `ToastQueue` impl 块之后、`#[cfg(test)]` 之前)

```rust
use iced_widget::core::{Border, Element};
use iced_widget::{container, Column};

pub fn view<'a, Message: 'a>(
    queue: &ToastQueue,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = crate::theme::color::current();
    let items: Vec<_> = queue
        .items()
        .iter()
        .map(|t| {
            let accent = match t.kind {
                Kind::Success => colors.green,
                Kind::Error => colors.red,
                Kind::Info => colors.cyan,
            };
            container(
                iced_widget::text(t.message.clone())
                    .size(crate::theme::font::body())
                    .color(colors.cream),
            )
            .padding([8, 12])
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(colors.card.into()),
                border: Border {
                    color: accent,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..container::Style::default()
            })
            .into()
        })
        .collect();
    Column::with_children(items).spacing(8).into()
}
```

(注意:文件顶部已有 `use std::time::{Duration, Instant};`,这里新加的 `use iced_widget::core::{Border, Element};`/`use iced_widget::{container, Column};` 放在文件顶部统一收拢,不要留在函数中间。)

- [ ] **Step 6: 创建 `feedback/mod.rs`**

```rust
pub mod progress;
pub mod status;
pub mod toast;
```

- [ ] **Step 7: `lib.rs` 加一行**

```rust
pub mod feedback;
```

- [ ] **Step 8: 全量编译 + 测试**

Run: `cargo build -p byteui && cargo test -p byteui feedback::`
Expected: 编译通过(`status`/`progress` 没有独立单测,靠编译期类型检查验证;`toast` 的 3 个测试 PASS)

- [ ] **Step 9: Commit**

```bash
git add crates/byteui/src/lib.rs crates/byteui/src/feedback/
git commit -m "feat(byteui): 新增 feedback 模块(status/progress/toast)"
```

---

### Task 9: `data` 模块(card/list/property)

**Files:**
- Modify: `crates/byteui/src/lib.rs`
- Create: `crates/byteui/src/data/mod.rs`
- Create: `crates/byteui/src/data/card.rs`
- Create: `crates/byteui/src/data/list.rs`
- Create: `crates/byteui/src/data/property.rs`

**Interfaces:**
- Consumes: `crate::theme::color::current()`(Task 2)、`crate::theme::font::{body, caption}`(Task 3)、`crate::interaction::cards::container_card`(Task 5)
- Produces: `data::card::view`、`data::list::item`、`data::property::{row, view}`

- [ ] **Step 1: 写 `card.rs`**

```rust
//! amis `card`(卡片):<https://baidu.github.io/amis/zh-CN/components/card>
//! 复用 `interaction::cards::container_card` 的三态样式(一般/hover/选中),
//! 这里只加标题+副标题的内容排版约定。

use iced_widget::core::Element;
use iced_widget::{container, text, Column};

pub fn view<'a, Message: 'a>(
    title: &'a str,
    subtitle: Option<&'a str>,
    selected: bool,
    hovered: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = crate::theme::color::current();
    let mut col = Column::new()
        .spacing(4)
        .push(text(title).size(crate::theme::font::body()).color(colors.cream));
    if let Some(sub) = subtitle {
        col = col.push(text(sub).size(crate::theme::font::caption()).color(colors.dim));
    }
    container(col)
        .padding(12)
        .style(move |_theme: &iced_widget::Theme| {
            crate::interaction::cards::container_card(selected, hovered, colors.card)
        })
        .into()
}
```

- [ ] **Step 2: 写 `list.rs`**

```rust
//! amis `list`(列表项行):<https://baidu.github.io/amis/zh-CN/components/list>

use iced_widget::core::{Element, Length};
use iced_widget::{container, text, Row};

pub fn item<'a, Message: 'a>(
    leading: Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
    label: &'a str,
    trailing: Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = crate::theme::color::current();
    let mut r = Row::new()
        .spacing(8)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    if let Some(l) = leading {
        r = r.push(l);
    }
    r = r.push(
        text(label)
            .size(crate::theme::font::body())
            .color(colors.cream)
            .width(Length::Fill),
    );
    if let Some(t) = trailing {
        r = r.push(t);
    }
    container(r).padding([6, 10]).into()
}
```

- [ ] **Step 3: 写 `property.rs`**

```rust
//! amis `property`(key-value 属性网格):<https://baidu.github.io/amis/zh-CN/components/property>

use iced_widget::core::{Element, Length};
use iced_widget::{text, Column, Row};

pub fn row<'a, Message: 'a>(
    key: &'a str,
    value: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = crate::theme::color::current();
    Row::new()
        .spacing(8)
        .push(
            text(key)
                .size(crate::theme::font::caption())
                .color(colors.dim)
                .width(Length::FillPortion(2)),
        )
        .push(
            text(value)
                .size(crate::theme::font::body())
                .color(colors.cream)
                .width(Length::FillPortion(3)),
        )
        .into()
}

pub fn view<'a, Message: 'a>(
    items: Vec<(&'a str, &'a str)>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let rows: Vec<_> = items.into_iter().map(|(k, v)| row(k, v)).collect();
    Column::with_children(rows).spacing(6).into()
}
```

- [ ] **Step 4: 创建 `data/mod.rs`**

```rust
pub mod card;
pub mod list;
pub mod property;
```

- [ ] **Step 5: `lib.rs` 加一行**

```rust
pub mod data;
```

- [ ] **Step 6: 写调用点 smoke test**

`crates/byteui/src/data/mod.rs` 追加:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use iced_widget::core::text::Text;

    #[derive(Clone)]
    enum Msg {}

    #[test]
    fn all_data_components_construct_without_panic() {
        let _: iced_widget::core::Element<Msg, iced_widget::Theme, iced_renderer::Renderer> =
            card::view("title", Some("subtitle"), false, false);
        let leading: iced_widget::core::Element<Msg, iced_widget::Theme, iced_renderer::Renderer> =
            Text::new("*").into();
        let _ = list::item(Some(leading), "label", None);
        let _ = property::view(vec![("key", "value")]);
    }
}
```

- [ ] **Step 7: 全量编译 + 测试**

Run: `cargo build -p byteui && cargo test -p byteui`
Expected: 整个 `byteui` crate(全部 9 个 Task 的内容)编译通过、测试全部 PASS

- [ ] **Step 8: 全量 clippy + fmt 检查**

Run: `cargo clippy -p byteui --all-targets -- -D warnings && cargo fmt -p byteui -- --check`
Expected: 无警告、无格式差异

- [ ] **Step 9: Commit**

```bash
git add crates/byteui/src/lib.rs crates/byteui/src/data/
git commit -m "feat(byteui): 新增 data 模块(card/list/property)"
```

---

## 完工验收

9 个 Task 全部提交后:

1. `cargo build -p byteui`(独立编译,不带 `--workspace`)必须成功,验证"不依赖 `dozer-core`/`dozer-app`"这条 Global Constraint。
2. `cargo test -p byteui`(不加 `--test-threads=1` 的默认并行跑一次)——`theme::color`/`theme::icon_size` 两组测试涉及全局可变状态,如果默认并行跑出现 flaky 失败,是已知原因(测试内部需要 `--test-threads=1` 才稳定,不是新增 bug),记录在案即可,不必现在修。
3. `git log --oneline` 确认 9 个 commit 都在当前分支上,没有漂到 `main`(Global Constraints 里的强制校验项)。
4. 提请审阅(参考 spec 里"排期备注"——`dozer-app` 侧迁移是这个 plan 之外的后续任务,不在这次验收范围内)。
