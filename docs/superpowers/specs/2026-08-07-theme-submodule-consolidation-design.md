# `theme` 子模块整合设计

**状态:已批准(brainstorming 会话,2026-08-07)**

## 背景

`dozer-app` 里跟"设计 token"相关的代码分散在 5 个平级顶层文件里:`theme.rs`(颜色常量)、
`chrome_style.rs`(区域样式)、`icon_size.rs`(图标尺寸)、`workspace_font.rs`(UI 字号)、
`workspace_geometry.rs`(几何常量),外加语义相关的 `terminal_font.rs`(终端字号)。这五/六个
文件本来就是一套系统——`chrome_style.rs`/`icon_size.rs`/`workspace_font.rs`/
`workspace_geometry.rs` 全部编译期内嵌同一份 `assets/theme/workspace.json`,各自只解析自己
关心的顶层字段(文件自身注释已经写明"互不干扰");`terminal_font.rs` 内嵌
`assets/theme/terminal.json`;`chrome_style.rs` 还会引用 `theme.rs` 的颜色令牌名做字符串
解析。用户提出把它们整合进一个 `theme/` 子模块目录,让 `src/theme/*.rs` 与
`assets/theme/*.json` 的对应关系更直观。

这是纯代码组织重构,跟阶段 1 扩展化重构(git-log/browser/todo 三个试点)是两件独立的事——
不涉及 `Message`/`State`/`update`/`view` 这套模式,单纯是文件/模块路径重组。

## 目标 / 非目标

**目标**:
1. 六个文件的内容原样(逐字节)搬进 `crates/dozer-app/src/theme/` 目录下的六个子模块文件。
2. 顶层 `crates/dozer-app/src/theme.rs` 变成纯模块入口(6 行 `pub mod X;`,不含其他内容)。
3. 全仓库(`dozer-app` crate 内)引用路径统一改成子模块路径,**不重导出**——`theme::GOLD`
   变成 `theme::color::GOLD`,`chrome_style::browser_pane()` 变成
   `theme::region::browser_pane()`,以此类推,六个模块全部一致处理,不为颜色常量搞特例。
4. 六个文件互相之间的引用(`chrome_style.rs` 现在 `use crate::icon_size;`/
   `use crate::theme;`)相应改成同目录下的平级引用(`use super::icon_size;`/
   `use super::color;`)。

**非目标**:
- 不改任何颜色值/字号/几何常量的实际数值。
- 不改任何渲染逻辑、不改任何函数的参数/返回类型。
- 不改 `assets/theme/workspace.json`/`assets/theme/terminal.json` 这两份配置文件本身。
- 不重命名 `terminal_font` 成更短的名字——它跟 `theme::font`(UI 控件字号)本来就是文件
  自己注释里强调过的"职责严格分离",保留区分度,不因为搬进同一目录就合并或改名。
- 不顺手做任何其他清理/重构(比如不去动 `chrome_style.rs` 里颜色解析走字符串匹配这种可以
  改进但跟这次目的无关的设计)。

## 目标文件结构

```
crates/dozer-app/src/theme.rs               # 模块入口,只有 pub mod 声明
crates/dozer-app/src/theme/color.rs         # 原 theme.rs 全部内容(含测试)
crates/dozer-app/src/theme/region.rs        # 原 chrome_style.rs 全部内容(含测试)
crates/dozer-app/src/theme/font.rs          # 原 workspace_font.rs 全部内容(含测试)
crates/dozer-app/src/theme/terminal_font.rs # 原 terminal_font.rs 全部内容(含测试)
crates/dozer-app/src/theme/icon_size.rs     # 原 icon_size.rs 全部内容(含测试)
crates/dozer-app/src/theme/geometry.rs      # 原 workspace_geometry.rs 全部内容(含测试)
```

`crates/dozer-app/src/theme.rs`:

```rust
//! 设计 token 系统入口——颜色(`color`)、区域样式(`region`)、UI 字号
//! (`font`)、终端字号(`terminal_font`)、图标尺寸(`icon_size`)、几何常量
//! (`geometry`)。`region`/`font`/`icon_size`/`geometry` 四个子模块编译期
//! 内嵌同一份 `assets/theme/workspace.json`,各自只解析自己关心的顶层
//! 字段;`terminal_font` 内嵌 `assets/theme/terminal.json`。

pub mod color;
pub mod font;
pub mod geometry;
pub mod icon_size;
pub mod region;
pub mod terminal_font;
```

## 引用路径映射

| 旧路径 | 新路径 |
|---|---|
| `theme::COLOR_CONST`(如 `theme::GOLD`) | `theme::color::COLOR_CONST` |
| `theme::mix(...)` | `theme::color::mix(...)` |
| `chrome_style::xxx()` | `theme::region::xxx()` |
| `workspace_font::xxx()` | `theme::font::xxx()` |
| `terminal_font::xxx` | `theme::terminal_font::xxx` |
| `icon_size::xxx()` | `theme::icon_size::xxx()` |
| `workspace_geometry::xxx()` | `theme::geometry::xxx()` |

`main.rs` 里对应的 `mod` 声明(`mod theme;`/`mod chrome_style;`/`mod icon_size;`/
`mod terminal_font;`/`mod workspace_font;`/`mod workspace_geometry;`)删掉后五个,只留
`mod theme;`。

`theme/region.rs`(原 `chrome_style.rs`)内部:
- `use crate::icon_size;` → `use super::icon_size;`
- `use crate::theme;` → `use super::color;`,内部 `theme::xxx` 引用相应改成 `color::xxx`。

`theme/font.rs`(原 `workspace_font.rs`)内部:
- `use crate::icon_size;` → `use super::icon_size;`

`theme/geometry.rs`(原 `workspace_geometry.rs`)内部:
- `use crate::icon_size;` → `use super::icon_size;`

其余文件(`workspace.rs`、`extensions/git_log.rs`、`extensions/browser.rs`、
`extensions/todo.rs`、`main.rs` 等)里对这六个模块的引用,按上面的路径映射表做全局替换。

## 风险与验证

纯路径重命名,不改变行为,风险主要在"有没有漏改的引用点"而不是逻辑错误。由于
`theme::GOLD` 这类颜色常量引用量极大(`workspace.rs` 单文件内大概率有几百处),手工逐处
改不现实,写实现计划时应该用批量替换(如 `sed`)处理,人工复核的重点是:
- 批量替换有没有误伤(比如某处局部变量/参数恰好也叫 `theme`,不该被替换的字符串字面量或
  注释里出现了 `theme::`/`chrome_style::` 等文字但不是代码引用)。
- 替换后 `cargo build --workspace` 干净通过,`cargo clippy --all-targets` 无新增警告
  (旧路径引用会直接编译失败,不会是"看起来对但语义错"这种隐蔽错误——这类纯路径重命名
  搞错了肯定是编译不过,不是运行时才发现)。
- 全仓库 grep 确认没有遗留的旧模块名引用(`grep -rn "chrome_style::\|workspace_font::\|workspace_geometry::" crates/`,排除 `theme/region.rs`/`theme/font.rs`/`theme/geometry.rs` 内部的历史注释文字提及)。

## 测试策略

六个文件原有的 `#[cfg(test)] mod tests` 原样搬过去,断言内容不变(纯路径搬家,不应该有任何
测试失败)。不新增测试(没有新增行为)。

`cargo build --workspace && cargo test --workspace && cargo clippy --all-targets && cargo fmt --check` 全绿作为唯一验收标准。

## 依赖变更

无。
