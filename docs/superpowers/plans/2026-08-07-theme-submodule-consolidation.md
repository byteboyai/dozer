# `theme` 子模块整合 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `theme.rs`/`chrome_style.rs`/`icon_size.rs`/`workspace_font.rs`/`terminal_font.rs`/`workspace_geometry.rs` 六个平级顶层文件合并进 `crates/dozer-app/src/theme/` 目录,全仓库引用路径统一改成 `theme::color::`/`theme::region::`/`theme::font::`/`theme::terminal_font::`/`theme::icon_size::`/`theme::geometry::`,不重导出。纯代码组织重构,不改变任何行为。

**Architecture:** 六个子任务,每个搬一个模块,按依赖顺序排(叶子模块——不依赖其余五个的——先搬,被依赖的模块后搬,避免中间态编译不过):`color`(无依赖)→ `icon_size`(无依赖)→ `terminal_font`(无依赖)→ `region`(依赖 `color`+`icon_size`)→ `font`(依赖 `icon_size`)→ `geometry`(依赖 `icon_size`)。每个子任务内部:`git mv` 文件、修内部互相引用、全仓库批量替换该模块的引用前缀、在 `theme.rs` 里加一行 `pub mod`、在 `main.rs` 删对应 `mod` 声明、编译测试、单独提交。最后一个任务做全量校验。

**Tech Stack:** Rust workspace;纯路径重命名,不涉及新依赖、新类型、新逻辑。

## Global Constraints

- 不改任何颜色值/字号/几何常量的实际数值,不改任何渲染逻辑,不改函数签名(除了它们现在
  归属的模块路径)。
- 不改 `assets/theme/workspace.json`/`assets/theme/terminal.json`。
- `theme::COLOR_CONST` → `theme::color::COLOR_CONST`,其余五个模块 → `theme::MODULE_NAME::`
  ——**不重导出**,`theme.rs`(新的模块入口)只有 `pub mod` 声明,不带任何 `pub use`。
- `terminal_font` 不改名,保留跟 `theme::font` 的区分度。
- 每个子任务结束都要 `cargo build --workspace && cargo test --workspace` 干净通过——这是
  纯路径重命名类改动最重要的安全网,漏改的引用点会直接编译失败,不会是隐蔽的运行时错误。

---

### Task 1: `color`(原 `theme.rs`)

**Files:**
- Move: `crates/dozer-app/src/theme.rs` → `crates/dozer-app/src/theme/color.rs`
- Create: `crates/dozer-app/src/theme.rs`(新的模块入口)
- Modify: 全仓库所有引用 `theme::` 颜色常量/`theme::mix` 的文件(见 Step 3 清单)

- [ ] **Step 1: 搬文件**

`theme.rs`(文件)和 `theme/`(目录)是两个不同的路径,不存在命名冲突,一步到位即可:

```bash
mkdir -p crates/dozer-app/src/theme
git mv crates/dozer-app/src/theme.rs crates/dozer-app/src/theme/color.rs
```

- [ ] **Step 2: 新建 `theme.rs` 模块入口**

```rust
//! 设计 token 系统入口——颜色(`color`)、区域样式(`region`)、UI 字号
//! (`font`)、终端字号(`terminal_font`)、图标尺寸(`icon_size`)、几何常量
//! (`geometry`)。`region`/`font`/`icon_size`/`geometry` 四个子模块编译期
//! 内嵌同一份 `assets/theme/workspace.json`,各自只解析自己关心的顶层
//! 字段;`terminal_font` 内嵌 `assets/theme/terminal.json`。

pub mod color;
```

（其余五个 `pub mod` 声明在后续任务里逐个补上。）

- [ ] **Step 3: 全仓库替换颜色常量引用**

`theme.rs` 现有 21 个 `pub` 项(20 个颜色常量/函数 + `mix`):`BG`、`PANEL`、`TERM_BG`、
`CARD`、`BORDER`、`CREAM`、`BODY`、`DIM`、`GOLD`、`CYAN`、`GREEN`、`PURPLE`、`RED`、
`ORANGE`、`MAGENTA`、`BLUE`、`SCRIM`、`TAB_ACTIVE_BORDER`、`TAB_ACTIVE_BG`、`TAB_HOVER`、
`mix`。用固定的标识符列表做替换(不是盲目替换 `theme::` 前缀——`theme::` 后面还会接其余
五个模块的内容,不能一次性全换),在 `crates/dozer-app/src` 下跑:

```bash
cd crates/dozer-app/src
grep -rl --include='*.rs' -E '\btheme::(BG|PANEL|TERM_BG|CARD|BORDER|CREAM|BODY|DIM|GOLD|CYAN|GREEN|PURPLE|RED|ORANGE|MAGENTA|BLUE|SCRIM|TAB_ACTIVE_BORDER|TAB_ACTIVE_BG|TAB_HOVER|mix)\b' . \
  | xargs sed -i '' -E 's/\btheme::(BG|PANEL|TERM_BG|CARD|BORDER|CREAM|BODY|DIM|GOLD|CYAN|GREEN|PURPLE|RED|ORANGE|MAGENTA|BLUE|SCRIM|TAB_ACTIVE_BORDER|TAB_ACTIVE_BG|TAB_HOVER|mix)\b/theme::color::\1/g'
cd -
```

（macOS 的 `sed -i` 需要 `-i ''` 这个空字符串参数;Linux 上是 `sed -i` 不带这个参数——按
实际运行环境调整。这条命令只替换"`theme::` 紧跟着上面 21 个已知标识符之一"的精确匹配,
不会误伤 `theme::region::xxx` 这类还没搬过去的其他模块引用,也不会误伤注释里提到
"`theme.rs`"这种文字。）

- [ ] **Step 4: 编译 + 测试**

Run: `cargo build --workspace && cargo test --workspace 2>&1 | tail -60`
Expected: 干净通过。若报"找不到 `theme::color::` 里没有的标识符",说明 Step 3 的标识符
列表漏了一个,回 `theme/color.rs` 核对补全。

- [ ] **Step 5: 全仓库确认没有遗留的旧式颜色引用**

Run: `grep -rn -E '\btheme::(BG|PANEL|TERM_BG|CARD|BORDER|CREAM|BODY|DIM|GOLD|CYAN|GREEN|PURPLE|RED|ORANGE|MAGENTA|BLUE|SCRIM|TAB_ACTIVE_BORDER|TAB_ACTIVE_BG|TAB_HOVER|mix)\b' crates/dozer-app/src`
Expected: 无输出(全部已经带上 `color::`)。

- [ ] **Step 6: Commit**

```bash
git add -A crates/dozer-app/src
git commit -m "refactor(dozer-app): move theme.rs into theme/color.rs submodule"
```

---

### Task 2: `icon_size`(原 `icon_size.rs`)

**Files:**
- Move: `crates/dozer-app/src/icon_size.rs` → `crates/dozer-app/src/theme/icon_size.rs`
- Modify: `crates/dozer-app/src/theme.rs`(加 `pub mod icon_size;`)
- Modify: `crates/dozer-app/src/main.rs`(删 `mod icon_size;`)
- Modify: 全仓库所有 `use crate::icon_size;` 与 `icon_size::` 引用点

- [ ] **Step 1: 搬文件**

```bash
git mv crates/dozer-app/src/icon_size.rs crates/dozer-app/src/theme/icon_size.rs
```

- [ ] **Step 2: `theme.rs` 加声明**

```rust
pub mod color;
pub mod icon_size;
```

- [ ] **Step 3: `main.rs` 删旧声明**

删除 `mod icon_size;` 这一行。

- [ ] **Step 4: 全仓库替换引用**

`icon_size.rs` 本身没有内部跨模块 `use`(不依赖 `theme`/`chrome_style` 等其余五个),不用
处理"文件内部引用"这一步,只需要处理"外部文件怎么导入/调用它"。

先处理带 `use crate::icon_size;` 的文件(逐个改成 `use crate::theme::icon_size;`):
`crates/dozer-app/src/chrome_style.rs`、`crates/dozer-app/src/workspace_font.rs`、
`crates/dozer-app/src/workspace_geometry.rs`、`crates/dozer-app/src/term_view.rs`、
`crates/dozer-app/src/workspace.rs`(这几个文件此刻还没搬,`use crate::icon_size;` 先改成
`use crate::theme::icon_size;`,调用点 `icon_size::xxx` 不用变,因为导入后本地名字还是
`icon_size`)。

`extensions/browser.rs`/`extensions/todo.rs` 里是 `use crate::{..., icon_size, ...};` 这种
合并 `use` 写法,把 `icon_size` 从大括号里摘出来,单独一行 `use
crate::theme::icon_size;`,大括号里剩下的其他模块名保留原状(它们各自的搬迁在后续任务
处理)。

再全仓库跑一遍确认没有遗漏的 `use crate::icon_size;`:

```bash
grep -rn "use crate::icon_size" crates/dozer-app/src
```

Expected: 无输出。

（`icon_size::xxx()` 这些调用点本身**不用批量替换**——因为上面把 `use crate::icon_size;`
改成了 `use crate::theme::icon_size;`,本地绑定的名字还是 `icon_size`,调用点文本不变。）

- [ ] **Step 5: 编译 + 测试**

Run: `cargo build --workspace && cargo test --workspace 2>&1 | tail -60`
Expected: 干净通过。

- [ ] **Step 6: Commit**

```bash
git add -A crates/dozer-app/src
git commit -m "refactor(dozer-app): move icon_size.rs into theme/icon_size.rs submodule"
```

---

### Task 3: `terminal_font`(原 `terminal_font.rs`)

**Files:**
- Move: `crates/dozer-app/src/terminal_font.rs` → `crates/dozer-app/src/theme/terminal_font.rs`
- Modify: `crates/dozer-app/src/theme.rs`(加 `pub mod terminal_font;`)
- Modify: `crates/dozer-app/src/main.rs`(删 `mod terminal_font;`)
- Modify: `crates/dozer-app/src/workspace.rs`、`crates/dozer-app/src/term_view.rs`(各自的
  `use crate::terminal_font;` → `use crate::theme::terminal_font;`)

- [ ] **Step 1: 搬文件**

```bash
git mv crates/dozer-app/src/terminal_font.rs crates/dozer-app/src/theme/terminal_font.rs
```

- [ ] **Step 2: `theme.rs` 加声明**

```rust
pub mod color;
pub mod icon_size;
pub mod terminal_font;
```

- [ ] **Step 3: `main.rs` 删旧声明**

删除 `mod terminal_font;` 这一行。

- [ ] **Step 4: 改两处 `use`**

`crates/dozer-app/src/workspace.rs`:`use crate::terminal_font;` → `use crate::theme::terminal_font;`。

`crates/dozer-app/src/term_view.rs`:`use crate::terminal_font;` → `use crate::theme::terminal_font;`。

调用点 `terminal_font::xxx` 不用变(本地绑定名字不变)。

- [ ] **Step 5: 编译 + 测试**

Run: `cargo build --workspace && cargo test --workspace 2>&1 | tail -60`
Expected: 干净通过。

- [ ] **Step 6: Commit**

```bash
git add -A crates/dozer-app/src
git commit -m "refactor(dozer-app): move terminal_font.rs into theme/terminal_font.rs submodule"
```

---

### Task 4: `region`(原 `chrome_style.rs`)

**Files:**
- Move: `crates/dozer-app/src/chrome_style.rs` → `crates/dozer-app/src/theme/region.rs`
- Modify: `crates/dozer-app/src/theme.rs`(加 `pub mod region;`)
- Modify: `crates/dozer-app/src/main.rs`(删 `mod chrome_style;`)
- Modify: 全仓库所有 `use crate::chrome_style;` 与 `chrome_style::` 引用点

**Interfaces:**
- Consumes: `theme::color`、`theme::icon_size`(Task 1/2 已完成,这个任务能正确引用它们是
  前两个任务顺序先做的原因)。

- [ ] **Step 1: 搬文件**

```bash
git mv crates/dozer-app/src/chrome_style.rs crates/dozer-app/src/theme/region.rs
```

- [ ] **Step 2: 顺手把文件内部的跨模块引用改成 `super::`(推荐但非强制)**

进入这一步之前,`chrome_style.rs` 已经被前两个任务改过两处:Task 2 Step 4 把它的
`use crate::icon_size;` 改成了 `use crate::theme::icon_size;`;Task 1 Step 3 的批量替换
把它内部 `resolve_color` 函数里的 `theme::BG`/`theme::GOLD` 等改成了
`theme::color::BG`/`theme::color::GOLD`(`use crate::theme;` 这一行本身没被 Task 1 动过,
仍是 `use crate::theme;`)。这些写法**已经是合法、能编译通过的绝对路径**,不改也没问题。

现在这个文件挪进了 `theme/region.rs`,和 `theme/color.rs`/`theme/icon_size.rs` 变成平级
子模块,可以顺手把这些路径缩短成更符合"同目录平级引用"习惯的 `super::` 写法(纯风格清理,
不是这一步能不能编译过的必要条件):

```rust
use crate::theme::icon_size;
use crate::theme;
```

改成:

```rust
use super::color;
use super::icon_size;
```

文件体内 `resolve_color` 函数里所有 `theme::color::xxx` 改成 `color::xxx`。

- [ ] **Step 3: `theme.rs` 加声明**

```rust
pub mod color;
pub mod icon_size;
pub mod region;
pub mod terminal_font;
```

- [ ] **Step 4: `main.rs` 删旧声明**

删除 `mod chrome_style;` 这一行。

- [ ] **Step 5: 全仓库批量替换**

`chrome_style::` 整个前缀替换成 `theme::region::`(这次不用像颜色常量那样列举标识符——
整个模块都在搬,任何 `chrome_style::xxx` 都该变成 `theme::region::xxx`,没有"部分留在原
命名空间"的情况):

```bash
grep -rl --include='*.rs' 'chrome_style::' crates/dozer-app/src \
  | xargs sed -i '' 's/chrome_style::/theme::region::/g'
```

再单独处理 `use crate::chrome_style;`(没有 `::` 后缀,不会被上面那条命令匹配到)这种裸
导入:

```bash
grep -rln 'use crate::chrome_style;' crates/dozer-app/src \
  | xargs sed -i '' 's/use crate::chrome_style;/use crate::theme::region;/g'
```

（`crates/dozer-app/src/workspace.rs` 是这两条命令预期会命中的文件。）

- [ ] **Step 6: 编译 + 测试**

Run: `cargo build --workspace && cargo test --workspace 2>&1 | tail -60`
Expected: 干净通过。

- [ ] **Step 7: 确认没有遗留引用**

Run: `grep -rn "chrome_style" crates/dozer-app/src`
Expected: 无输出(连注释里的历史提及也搬空了——如果 `theme/region.rs` 自己的文档注释里
提到"原 chrome_style.rs"这种说明性文字,那是允许保留的,写代码时用人工判断区分"代码引用"
和"说明性历史提及",不要机械地要求这条 grep 必须零输出,只要没有编译期需要的代码引用即可)。

- [ ] **Step 8: Commit**

```bash
git add -A crates/dozer-app/src
git commit -m "refactor(dozer-app): move chrome_style.rs into theme/region.rs submodule"
```

---

### Task 5: `font`(原 `workspace_font.rs`)

**Files:**
- Move: `crates/dozer-app/src/workspace_font.rs` → `crates/dozer-app/src/theme/font.rs`
- Modify: `crates/dozer-app/src/theme.rs`(加 `pub mod font;`)
- Modify: `crates/dozer-app/src/main.rs`(删 `mod workspace_font;`)
- Modify: 全仓库所有 `use crate::workspace_font;` 与 `workspace_font::` 引用点

- [ ] **Step 1: 搬文件**

```bash
git mv crates/dozer-app/src/workspace_font.rs crates/dozer-app/src/theme/font.rs
```

- [ ] **Step 2: 顺手把文件内部的跨模块引用改成 `super::`(推荐但非强制)**

`workspace_font.rs` 的 `use crate::icon_size;` 在 Task 2 Step 4 已经被改成了
`use crate::theme::icon_size;`——这是合法的绝对路径,不改也能编译过。现在文件挪进了
`theme/font.rs`,跟 `theme/icon_size.rs` 是平级子模块,可以顺手缩短成
`use super::icon_size;`,纯风格清理。

- [ ] **Step 3: `theme.rs` 加声明**

```rust
pub mod color;
pub mod font;
pub mod icon_size;
pub mod region;
pub mod terminal_font;
```

- [ ] **Step 4: `main.rs` 删旧声明**

删除 `mod workspace_font;` 这一行。

- [ ] **Step 5: 全仓库批量替换**

```bash
grep -rl --include='*.rs' 'workspace_font::' crates/dozer-app/src \
  | xargs sed -i '' 's/workspace_font::/theme::font::/g'
grep -rln 'use crate::workspace_font;' crates/dozer-app/src \
  | xargs sed -i '' 's/use crate::workspace_font;/use crate::theme::font;/g'
```

`extensions/browser.rs`/`extensions/todo.rs`/`extensions/git_log.rs` 里如果是
`use crate::{..., workspace_font, ...};` 这种合并写法,上面第二条命令匹配不到(它们不是
单独一行 `use crate::workspace_font;`),需要手工把 `workspace_font` 从大括号里摘出来,
改成单独一行 `use crate::theme::font;`(参照 Task 2 Step 4 处理 `icon_size` 时的同款
手法)。

- [ ] **Step 6: 编译 + 测试**

Run: `cargo build --workspace && cargo test --workspace 2>&1 | tail -60`
Expected: 干净通过。

- [ ] **Step 7: 确认没有遗留引用**

Run: `grep -rn "workspace_font" crates/dozer-app/src`
Expected: 无编译期代码引用残留(同 Task 4 Step 7 的判断标准)。

- [ ] **Step 8: Commit**

```bash
git add -A crates/dozer-app/src
git commit -m "refactor(dozer-app): move workspace_font.rs into theme/font.rs submodule"
```

---

### Task 6: `geometry`(原 `workspace_geometry.rs`)

**Files:**
- Move: `crates/dozer-app/src/workspace_geometry.rs` → `crates/dozer-app/src/theme/geometry.rs`
- Modify: `crates/dozer-app/src/theme.rs`(加 `pub mod geometry;`)
- Modify: `crates/dozer-app/src/main.rs`(删 `mod workspace_geometry;`)
- Modify: 全仓库所有 `use crate::workspace_geometry;` 与 `workspace_geometry::` 引用点

- [ ] **Step 1: 搬文件**

```bash
git mv crates/dozer-app/src/workspace_geometry.rs crates/dozer-app/src/theme/geometry.rs
```

- [ ] **Step 2: 顺手把文件内部的跨模块引用改成 `super::`(推荐但非强制)**

`workspace_geometry.rs` 的 `use crate::icon_size;` 在 Task 2 Step 4 已经被改成了
`use crate::theme::icon_size;`——合法绝对路径,不改也能编译过。现在文件挪进了
`theme/geometry.rs`,跟 `theme/icon_size.rs` 是平级子模块,可以顺手缩短成
`use super::icon_size;`,纯风格清理。

- [ ] **Step 3: `theme.rs` 加声明(六个全部齐了)**

```rust
pub mod color;
pub mod font;
pub mod geometry;
pub mod icon_size;
pub mod region;
pub mod terminal_font;
```

- [ ] **Step 4: `main.rs` 删旧声明**

删除 `mod workspace_geometry;` 这一行——至此 `main.rs` 里只剩 `mod theme;` 一行代表整个
设计 token 系统,不再有 `chrome_style`/`icon_size`/`workspace_font`/`terminal_font`/
`workspace_geometry` 五个平级声明。

- [ ] **Step 5: 全仓库批量替换**

```bash
grep -rl --include='*.rs' 'workspace_geometry::' crates/dozer-app/src \
  | xargs sed -i '' 's/workspace_geometry::/theme::geometry::/g'
grep -rln 'use crate::workspace_geometry;' crates/dozer-app/src \
  | xargs sed -i '' 's/use crate::workspace_geometry;/use crate::theme::geometry;/g'
```

`extensions/browser.rs` 里如果是 `use crate::{..., workspace_geometry, ...};` 合并写法,
同 Task 5 Step 5 手工摘出来处理。

- [ ] **Step 6: 编译 + 测试**

Run: `cargo build --workspace && cargo test --workspace 2>&1 | tail -60`
Expected: 干净通过。

- [ ] **Step 7: 确认没有遗留引用**

Run: `grep -rn "workspace_geometry" crates/dozer-app/src`
Expected: 无编译期代码引用残留。

- [ ] **Step 8: Commit**

```bash
git add -A crates/dozer-app/src
git commit -m "refactor(dozer-app): move workspace_geometry.rs into theme/geometry.rs submodule"
```

---

### Task 7: 全量校验

**Files:** 无新增/修改(纯校验任务)

- [ ] **Step 1: 全 workspace 构建 + 测试 + clippy + fmt**

Run: `cargo build && cargo test && cargo clippy --all-targets && cargo fmt --check`
Expected: 全部 crate 编译通过、测试全绿、无新增警告、无格式差异。

- [ ] **Step 2: 确认六个旧文件都已经不存在,`theme/` 目录结构完整**

```bash
ls crates/dozer-app/src/theme.rs crates/dozer-app/src/theme/color.rs \
   crates/dozer-app/src/theme/region.rs crates/dozer-app/src/theme/font.rs \
   crates/dozer-app/src/theme/terminal_font.rs crates/dozer-app/src/theme/icon_size.rs \
   crates/dozer-app/src/theme/geometry.rs
ls crates/dozer-app/src/chrome_style.rs crates/dozer-app/src/icon_size.rs \
   crates/dozer-app/src/workspace_font.rs crates/dozer-app/src/terminal_font.rs \
   crates/dozer-app/src/workspace_geometry.rs 2>&1
```

Expected: 第一条 `ls` 全部存在;第二条 `ls` 全部报"不存在"。

- [ ] **Step 3: 全仓库确认没有任何遗留的旧模块路径引用**

```bash
grep -rn -E '\b(chrome_style|workspace_font|workspace_geometry)::' crates/dozer-app/src
grep -rn "use crate::icon_size;\|use crate::terminal_font;" crates/dozer-app/src
grep -rn -E '\btheme::(BG|PANEL|TERM_BG|CARD|BORDER|CREAM|BODY|DIM|GOLD|CYAN|GREEN|PURPLE|RED|ORANGE|MAGENTA|BLUE|SCRIM|TAB_ACTIVE_BORDER|TAB_ACTIVE_BG|TAB_HOVER|mix)\b' crates/dozer-app/src
```

Expected: 三条都无输出。

- [ ] **Step 4: 人工确认没有行为变化**

Run: `cargo run -p dozer-app`,随便打开一个项目走一遍常见界面(终端/预览/浏览器/项目树)——
颜色、字号、图标大小、面板间距应该跟改动前肉眼看不出差别(这本来就是预期,纯路径重命名)。
主要确认没有崩溃/panic(如果哪个 JSON 解析路径没搬对,`chrome_style.rs`/`icon_size.rs`
等文件里"解析失败直接 panic"的设计会在启动时就报出来,不会是运行期才发现的隐蔽问题)。

- [ ] **Step 5: 确认没有遗留未提交的改动**

Run: `git status`
Expected: 干净。
