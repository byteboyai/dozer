# dozer-app 迁移 interaction 模块到 byteui

**状态:已批准(brainstorming 会话,2026-08-18)**

## 背景

`byteui`(见 `2026-08-18-byteui-library-design.md`)已经把 `dozer-app` 的
`icons.rs`(`IconKind`/`icon_button_entry`/`with_tooltip`/`icon_for_file`)、
`tabs.rs`(`tab_core`)、`theme/cards.rs`(`button_card`/`container_card`)、
`scrollbar.rs`(`scrollbar`/`scrollbar_style`)平移进 `crates/byteui/src/
interaction/`,代码逐字节一致(仅颜色常量改成经 `theme::color::current()`
取值)。但 `dozer-app` 里的原文件还留着——现在是两份完全独立、内容却本该
一致的源码,任何一处以后被改动而另一处没跟上,就会重蹈当初建 `byteui` 时
想解决的"同一个概念两处实现各自长歪"的覆辙(`2026-08-12-tab-icon-button-
shared-components-design.md` 那次教训)。

这是 `byteui` 落地后 4 个迁移子项目里的第 1 个(顺序:`interaction` →
`theme::color` → `theme::font`+`theme::geometry` → `theme::icon_size`),
选它打头阵是因为 API 形状完全没变、调用点最少(190 处、11 个文件),适合
验证"删本地文件、调用点直接指向 `byteui::`"这套迁移手法,再把手法复用到
后面三个体量大得多的子项目上。

**迁移策略是"全量改调用点"**(brainstorming 会话里明确选择,放弃了"重导出/
派生 const,零调用点改动"的低风险替代方案)——迁移完成后 `dozer-app` 源码树
里不留任何 `icons.rs`/`tabs.rs`/`theme/cards.rs`/`scrollbar.rs`,每个调用点
都直接写 `byteui::interaction::...`,没有本地重导出层。

## 目标 / 非目标

**目标**:

1. `dozer-app` 新增依赖 `byteui = { path = "../byteui" }`。
2. 删除 `crates/dozer-app/src/icons.rs`、`tabs.rs`、`theme/cards.rs`、
   `scrollbar.rs`,以及 `crates/dozer-app/assets/icons/` 整个目录(已确认
   除 `icons.rs` 外没有任何 `.rs`/`.toml`/`build.rs` 引用这个目录)。
3. 11 个文件、约 190 处调用点,把 `crate::icons::`/`use crate::icons`/裸
   `icons::`(靠 `use crate::icons;` 引入)改写成 `byteui::interaction::
   icons::`(同理 `tabs`/`theme::cards`/`crate::scrollbar` 三组);纯前缀
   替换,不改变任何调用参数或语义。
4. 迁移完成后 `dozer-app` 编译、测试、clippy、fmt 全绿,GUI 视觉与迁移前
   逐一比对无差异(rail 图标、项目页签、卡片三态、滚动条)。

**非目标**:

- **不迁移 `theme::color`/`theme::font`/`theme::geometry`/`theme::icon_size`**
  ——这四个是后续 3 个独立子项目的范围,各自单独走 spec → plan → 分支 →
  审阅,不在这次改动。
- **不改变任何视觉/交互行为**。这是纯粹的"调用点指向哪里"重构,不是"顺手
  改进"——迁移前后 `IconKind`/`icon_button_entry`/`tab_core`/
  `button_card`/`container_card`/`scrollbar`/`scrollbar_style` 的行为
  逐字节一致(`byteui` 里就是原样迁移过去的)。
- **不合并/重构任何调用方的业务逻辑**。只改 import 路径和函数调用的限定
  前缀,调用方(`app.rs`/各 `extensions/*.rs`)自己的布局/状态管理代码
  一行不动。

## 架构与数据流

### 依赖与文件删除

```toml
# crates/dozer-app/Cargo.toml [dependencies] 加一行
byteui = { path = "../byteui" }
```

```bash
rm crates/dozer-app/src/icons.rs
rm crates/dozer-app/src/tabs.rs
rm crates/dozer-app/src/theme/cards.rs
rm crates/dozer-app/src/scrollbar.rs
rm -r crates/dozer-app/assets/icons/
```

`crates/dozer-app/src/theme/mod.rs` 里 `pub mod cards;` 那一行同步删除;
`dozer-app` 是纯 bin crate(没有 `lib.rs`),三个模块声明都在
`crates/dozer-app/src/main.rs`(已核对:`mod icons;` 第 12 行、
`mod scrollbar;` 第 23 行、`mod tabs;` 第 24 行),同步删除这三行。

### 调用点替换规则(四组,机械、无语义变化)

| 原引用 | 新引用 |
|---|---|
| `crate::icons::` | `byteui::interaction::icons::` |
| `use crate::icons;` | `use byteui::interaction::icons;` |
| `use crate::icons::IconKind;`(`workspace.rs` 独有,命名导入) | `use byteui::interaction::icons::IconKind;` |
| `crate::tabs::`(仅 `app.rs`) | `byteui::interaction::tabs::` |
| `use crate::tabs;`(仅 `app.rs`) | `use byteui::interaction::tabs;` |
| `crate::theme::cards::` | `byteui::interaction::cards::` |
| `crate::scrollbar::` | `byteui::interaction::scrollbar::` |

已核对过:全仓库 190 处引用没有一处靠 `use crate::icons::{GOLD, ...}` 这类
把裸标识符引入作用域再裸用的写法,全部经完整限定路径调用,替换不会漏改
或改出编译错误。

### 受影响文件与每文件调用点数(11 个文件,按数量降序)

| 文件 | 调用点数 |
|---|---|
| `crates/dozer-app/src/app.rs` | 37 |
| `crates/dozer-app/src/extensions/files.rs` | 27 |
| `crates/dozer-app/src/extensions/todo.rs` | 23 |
| `crates/dozer-app/src/extensions/database.rs` | 16 |
| `crates/dozer-app/src/homespace.rs` | 16 |
| `crates/dozer-app/src/workspace.rs` | 11 |
| `crates/dozer-app/src/extensions/git_log.rs` | 8 |
| `crates/dozer-app/src/extensions/ssh.rs` | 8 |
| `crates/dozer-app/src/extensions/ssh/sftp.rs` | 6 |
| `crates/dozer-app/src/extensions/footbar.rs` | 5 |
| `crates/dozer-app/src/extensions/usage.rs` | 2 |

## 错误处理

不适用——纯路径重写,没有新增可能失败的运行时逻辑。编译器本身就是这次
改动最主要的校验手段:任何一处漏改都会在 `cargo build -p dozer-app` 时
报"找不到 `crate::icons`"之类的错误,而不会静默产生错误行为。

## 测试策略

1. 每个文件改完后单独跑 `cargo build -p dozer-app --bin dozer`,确认这一
   个文件的改动没有引入编译错误——不要攒到最后才编译,漏改的定位会更
   麻烦。
2. 11 个文件全部改完后:`cargo test -p dozer-app --bin dozer`(预期通过数
   与迁移前一致——参考 `2026-08-12` 那次迁移记录的基线:484 passed / 2
   failed,两个已知的、与本次改动无关的 terminal grid 尺寸测试失败)、
   `cargo clippy -p dozer-app --all-targets -- -D warnings`、
   `cargo fmt -p dozer-app -- --check`。
3. 构建一个独立命名的临时二进制(不用会撞到用户正在跑的正式 `/Applications/
   Dozer AI Coder.app` 或其他会话调试实例的路径/进程名),做一次 GUI 视觉
   核对:
   - 左右 icon rail 全部按钮:静止态图标颜色、hover 过渡、点击切换面板。
   - 顶栏项目页签:选中/hover/关闭按钮/拖拽换位。
   - 各类列表卡(Agent/项目/最近/todo/git commit/主机):一般/hover/选中
     三态描边与背景。
   - 各面板(files/database/todo)的竖直滚动条外观。
   完成后关闭该临时实例、删除临时二进制,不留后台进程。

## 排期备注

11 个文件相互独立(改动范围不重叠,`app.rs` 是唯一同时涉及 `icons`+
`tabs` 两组的文件,其余文件只涉及 `icons`/`cards`/`scrollbar` 中的一到两
组),可以按任意顺序做,也可以分多次提交分多次审阅。删依赖文件
(`icons.rs`/`tabs.rs`/`theme/cards.rs`/`scrollbar.rs`/`assets/icons/`)和
加 `Cargo.toml` 依赖这一步必须在改完全部 11 个文件之后才能做(改的过程中
这些文件还要留着,否则中间状态编译不过)。
