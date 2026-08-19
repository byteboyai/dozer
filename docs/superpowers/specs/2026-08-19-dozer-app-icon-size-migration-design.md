# dozer-app 迁移 theme::icon_size 到 byteui

**状态:已批准(brainstorming 会话,2026-08-19)**

## 背景

`byteui` 落地后 4 个迁移子项目原定顺序是:`interaction` → `theme::color` →
`theme::font`+`theme::geometry` → `theme::icon_size`(见
`2026-08-18-dozer-app-interaction-migration-design.md`)。前两个已合并
main。**这次迁移把 `icon_size` 从原定第 4 提到第 3 做**,原因是设计
`font`+`geometry` 迁移时发现的一个耦合:`byteui::theme::font`/
`byteui::theme::geometry`(建库时已就位)内部都调用
`byteui::theme::icon_size::scale()`——`byteui` 自己的缩放单例,和
`dozer-app` 本地 `theme/icon_size.rs` 的缩放单例是两份独立的
`AtomicU32`。如果先迁 `font`+`geometry`、`icon_size` 还留在本地,
Ctrl +/- 缩放快捷键(`crate::theme::icon_size::zoom_by`)改的是本地单例,
而迁移后的字号/几何 accessor 读的是 `byteui` 那份单例——两者不再同步,
缩放会出现"图标跟着变、文字和布局尺寸不跟"的分裂。**先做 `icon_size`
消掉这个耦合源头,`font`+`geometry` 顺延成第 4 个。**

`byteui::theme::icon_size`(建库时已经原样迁入)和 `dozer-app` 现有
`theme/icon_size.rs` 相比,**只有一处 API 形状差异**:`init_scale`/
`persist_scale`/`reset_scale` 三个函数在 `byteui` 里改成显式吃调用方
传入的 `path: &Path`(不再内置 `dozer_core::paths::config_dir()`)——这是
`byteui-library` 建库设计里明确写的"依赖边界:不依赖 `dozer-core`"要求
(见 `2026-08-18-byteui-library-design.md`)。其余 9 个函数
(`rail`/`row`/`chevron`/`tab_arrow`/`home`/`tree_row_gap`/`scale`/
`set_scale`/`zoom_by`)签名完全不变。

## 目标 / 非目标

**目标**:

1. 21 个文件、约 140 处 `icon_size::` 调用点(4 种引用形态,见下)改成
   指向 `byteui::theme::icon_size::...`。
2. `theme.rs`(`mod theme` 的入口文件,dozer-app 用 `theme.rs` 而非
   `theme/mod.rs`)新增一个 `pub(crate) fn ui_scale_path() -> PathBuf`,
   把 `dozer_core::paths::config_dir().join("ui_scale.json")` 这条
   Dozer 专属路径约定留在 `dozer-app` 侧——`init_scale`/`persist_scale`/
   `reset_scale` 三个调用点(共 4 处:`main.rs` 1 处 `init_scale`,
   `app.rs` 2 处 `persist_scale`+1 处 `reset_scale`)改成显式传
   `&crate::theme::ui_scale_path()`。
3. 删除 `crates/dozer-app/src/theme/icon_size.rs`,删除 `theme.rs` 里的
   `pub mod icon_size;`。
4. 迁移完成后 `dozer-app` 编译、测试、clippy、fmt 全绿,GUI 缩放行为
   (Ctrl +/-、Ctrl+1 还原、跨重启保留)与迁移前逐一核对无差异。

**非目标**:

- **不迁移 `theme::font`/`theme::geometry`**——这两个内部会改成调用
  `byteui::theme::icon_size::scale()`(因为它们 `use super::icon_size;`
  引用的模块本身就是这次要删的 `icon_size.rs`,不改就编译不过),但
  `font.rs`/`geometry.rs` 文件本身、以及它们自己的 32+8 个 accessor
  函数**依然留在 `dozer-app` 本地**,不删、不改调用方指向。这是下一个
  独立子项目(原定第 4,现按这次调整顺延)的范围。
- **不迁移 `theme::region.rs`/`theme::homespace_font.rs`**——这两个文件
  同样 `use super::icon_size;`,受这次改动直接影响(内部引用要跟着改),
  但文件本身、对外的 accessor 一样不删不动,不是这次的迁移对象。
- **不改变任何缩放行为**。`byteui::theme::icon_size::rail()` 和被删除的
  `crate::theme::icon_size::rail()` 在同一份 `assets/theme/workspace.json`
  基准值下逐字节一致(建 `byteui` 时已用同一份 JSON 原样迁移)。
- **不改动 `theme::homespace_color.rs`**——独立配色系统,不含
  `icon_size` 依赖,不受这次改动影响。

## 架构与数据流

### 前置检查(执行前先确认)

```bash
grep -c "byteui" crates/dozer-app/Cargo.toml   # 应 ≥ 1,确认 migration #1 已加依赖
ls crates/dozer-app/src/theme/color.rs 2>&1    # 应报 No such file,确认 migration #2 已删本地文件
```

### 新增:`ui_scale_path()` 落地在 `theme.rs`

`byteui` 不内置任何 Dozer 专属路径约定,`init_scale`/`persist_scale`/
`reset_scale` 需要的落盘路径必须由 `dozer-app` 自己算好传入。参照
`layout.rs`/`open_projects.rs`/`panel_layouts.rs`/`extensions/todo.rs`
既有的"状态文件路径函数就近放在使用它的模块里"的写法,把这个函数放进
`theme.rs`(`icon_size` 原来的宿主模块的 mod 入口文件):

```rust
// crates/dozer-app/src/theme.rs 顶部追加
use std::path::PathBuf;

/// `byteui::theme::icon_size` 的 `init_scale`/`persist_scale`/`reset_scale`
/// 需要调用方传入落盘路径(`byteui` 不内置 Dozer 专属路径约定)。
pub(crate) fn ui_scale_path() -> PathBuf {
    dozer_core::paths::config_dir().join("ui_scale.json")
}
```

4 个调用点相应改成:

```rust
// main.rs:374(原:crate::theme::icon_size::init_scale();)
crate::theme::icon_size::init_scale(&theme::ui_scale_path());
// ↓ 删本地 icon_size.rs 后改成
byteui::theme::icon_size::init_scale(&theme::ui_scale_path());

// app.rs:4141 / 4148(原:crate::theme::icon_size::persist_scale();)
byteui::theme::icon_size::persist_scale(&theme::ui_scale_path());

// app.rs:4154(原:crate::theme::icon_size::reset_scale();)
byteui::theme::icon_size::reset_scale(&theme::ui_scale_path());
```

这 4 处路径参数必须手改(每处都要插入新参数,不是纯前缀替换),不走下面
的 sed 批量规则。

### 调用点替换规则(3 种引用形态)

**形态 A——`crate::theme::icon_size::NAME(`(直接完整路径,17 个文件)**:

`app.rs`(25)、`extensions/database.rs`(20)、`extensions/files.rs`(16)、
`extensions/todo.rs`(8)、`extensions/project.rs`(7)、`workspace.rs`(4)、
`main.rs`(4,含前述需手改的 1 处 `init_scale`)、`extensions/git_log.rs`(4)、
`homespace.rs`(3)、`theme/geometry.rs`(2,内部引用)、`extensions/ssh.rs`(2)、
`menu.rs`(1)、`extensions/usage.rs`(1)、`extensions/ssh/sftp.rs`(1)。

```bash
sed -i '' -e 's/crate::theme::icon_size::rail(/byteui::theme::icon_size::rail(/g' \
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

`init_scale`/`persist_scale`/`reset_scale` **不进这条批量规则**——按上一节
手改 4 处。`app.rs`/`main.rs` 跑完这条规则后,再单独处理各自那几处
scale 生命周期函数。

**形态 B——`theme::icon_size::NAME(`(经 `use crate::theme;` 引入,1 个
文件:`preview.rs`,2 处)**:同形态 A 九条规则,把 `crate::theme::` 前缀
换成 `theme::` 即可(`preview.rs` 两处都是 `row()`,不涉及 scale 生命周期
函数)。

**形态 C——裸 `icon_size::NAME(`(经 `use crate::theme::icon_size;` 或
`use super::icon_size;` 引入,6 个文件)**:

- `term_view.rs`(1)、`extensions/browser.rs`(4)、`extensions/footbar.rs`(3)
  ——`use crate::theme::icon_size;` 引入,替换后这行 `use` 要删掉(不再需要
  这个别名,否则 unused import 警告)。
- `theme/font.rs`(1 处真实调用,`icon_size::scale()`)、
  `theme/homespace_font.rs`(1 处真实调用,同)、`theme/region.rs`(2 处真实
  调用,`icon_size::scale()`)——`use super::icon_size;` 引入,同样替换后
  删掉这行 `use`。

```bash
sed -i '' -e 's/icon_size::rail(/byteui::theme::icon_size::rail(/g' \
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

**这条规则只能用于形态 C 的 6 个文件**——如果误用在形态 A/B 的文件上,
`crate::theme::icon_size::row(` 里的 `icon_size::row(` 子串会被二次命中,
产出 `crate::theme::byteui::theme::icon_size::row(` 这种错误的双重替换。
形态 A/B 必须先套各自那条(带 `crate::theme::`/`theme::` 前缀的)规则组。

**执行顺序**:形态 A/B 的文件先跑,形态 C 的文件后跑(或者反过来也行,
只要不在同一个文件上混用两条规则)——21 个文件互相独立,谁先谁后不影响
结果,不要在同一份文件里先后套用形态 A/C 两条规则。

### 收尾:删本地文件、清理 `use`、清理 `theme.rs`

```bash
rm crates/dozer-app/src/theme/icon_size.rs
```

`theme.rs` 里删除 `pub mod icon_size;`,新增前述 `ui_scale_path()`。

6 个形态 C 文件里对应的 `use crate::theme::icon_size;` / `use super::icon_size;`
整行删除(`cargo build` 会以 unused import 报出漏删的,不是静默失败)。

## 错误处理

不适用——和 migration #1/#2 同定位:纯路径重写(9 个无状态函数)+
4 处参数注入(3 个有状态函数改吃显式路径),没有新增可能失败的运行时逻辑。
编译器是主要校验手段,`init_scale`/`persist_scale`/`reset_scale` 的 4 处
手改如果漏传路径参数会直接编译报错(参数数量不对),不会静默产生错误
行为。

## 测试策略

1. 每个文件改完后单独跑 `cargo build -p dozer-app --bin dozer`。
2. 全部 21 个文件改完后:`cargo test -p dozer-app --bin dozer`(基线同
   migration #1/#2:484 passed / 2 failed,两个已知的、与本次改动无关的
   terminal grid 尺寸测试失败)、`cargo clippy -p dozer-app --all-targets
   -- -D warnings`、`cargo fmt -p dozer-app -- --check`。
3. 独立命名的临时二进制做一次缩放专项核对(这是这次迁移唯一有实际行为
   风险的部分——`init_scale`/`persist_scale`/`reset_scale` 从"内置路径"
   改成"调用方传路径",传错路径不会编译失败,只会运行时读写错文件):
   - 启动后 Ctrl + 放大三次、Ctrl - 缩小两次,确认图标/文字/几何布局
     (rail 宽、tab 尺寸、面板间距)整体同步缩放,没有"图标变了文字没变"
     的分裂。
   - Ctrl+1 还原,确认缩放立即回到出厂默认。
   - 退出、重开,确认缩放值(含 Ctrl+1 还原后的出厂默认落盘)跨重启保留
     ——检查 `~/Library/Application Support/dozer/ui_scale.json`(或
     `dozer_core::paths::config_dir()` 实际指向路径)内容与放大/还原操作
     一致,不是残留旧版本 `icon_size.rs` 时代的文件路径基准(应该完全相同,
     因为 `ui_scale_path()` 和旧 `scale_path()` 算的是同一条路径,只是
     函数搬了家)。
   完成后关闭该临时实例、删除临时二进制,不留后台进程。

## 排期备注

21 个文件相互独立,可以按任意顺序做、分批提交分批审阅,但同一个文件内
不能混用形态 A/C 两条 sed 规则(见上)。删 `theme/icon_size.rs`、清理
`theme.rs` 的 `pub mod icon_size;`、新增 `ui_scale_path()`,必须在 21 个
文件全部改完之后才能做(中间状态下 `theme/icon_size.rs` 还要留着,否则
未迁移的文件编译不过)。

这次迁移完成后,原定第 4 个子项目(`theme::font`+`theme::geometry`)
不再有 `icon_size` 缩放单例分裂的顾虑,可以按 migration #1/#2/#3 已验证
的"全量改调用点、无本地重导出层"手法直接推进,是这 4 个子项目里最后
一个、也是体量最大的一个(`font.rs` 8 个 accessor + `geometry.rs` 32 个
accessor,此前发现 `geometry.rs` 还多出 3 个依赖 `region`/`terminal_font`/
`workspace::tree_row_font_size` 的函数——`tree_row_h`/`tree_chrome_top_px`/
`tree_chrome_bottom_px`——这 3 个函数依赖未迁移的 app 专属模块,不能整体
搬进 `byteui`,届时需要单独设计"哪些留本地、哪些进 byteui"的拆分方案,
不是一次性整体删文件)。
