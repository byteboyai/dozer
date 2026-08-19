# byteui font/geometry/icon_size 基础尺寸改为运行时可替换 token

**状态:已批准(brainstorming 会话,2026-08-19)**

## 背景

`byteui` 落地时(`2026-08-18-byteui-library-design.md`)`theme::color` 用的
是"编译期 Rust 默认值 + 运行时 `set_theme` 整体替换"(`ColorTokens::
byteboy2077()` 是 `const fn`,`current()`/`set_theme()` 走 `RwLock`,组件
代码一律读 `current()`,不碰硬编码色值)。但 `theme::font`/`theme::geometry`/
`theme::icon_size` 三个模块没有照这个模式做——它们把
`crates/dozer-app/assets/theme/workspace.json` **原样复制**了一份到
`crates/byteui/assets/theme/workspace.json`,靠 `include_str!` 在
`byteui` 自己的 crate 里编译期内嵌解析,和 `dozer-app` 那份是两个独立
文件、两份独立数据。

这个不一致已经造成真实的数据漂移:审计时发现 `byteui` 那份 `workspace.json`
的 `font_sizes` 字段(`dot_sm`/`caption_sm`/…/`title`)比 `dozer-app` 那份
大 1。由于 `theme::font`/`theme::geometry`/`theme::icon_size` 的 4 个
迁移子项目(`2026-08-18`~`2026-08-19` 系列 spec)已经把 `dozer-app` 所有
调用点改成指向 `byteui::theme::font::body()` 这类函数,这份漂移意味着
`dozer-app` 实际渲染出的字号已经不是 `dozer-app` 自己 JSON 里写的值,
而是 `byteui` 那份复制品的值——`byteui` 本该是"零依赖 `dozer-app` 的
通用组件库",却意外变成了"`dozer-app` 的隐藏配置来源",这正是四个迁移
子项目当初要消灭的"同一概念两处实现各自长歪"问题,只是这次出现在
`byteui` 自己内部而不是调用点。

这次审计过程中,`b3f2763`("全局字号基准由 13px 抬到 14px 对齐终端字号")
又往这份漂移的方向新增了一次提交——直接改 `byteui` 那份 `workspace.json`
(`dot_sm`/…/`title` 各 +1,现为 10/11/12/13/14/15/16),commit message 里
明确写"`dozer-app` 那份 `font_sizes` 是死配置,已还原避免误导"。这次
改动**必须以 `byteui` 那份当前值为准**(它是最新、被实际渲染使用的值),
`dozer-app` 自己的 `assets/theme/workspace.json` 里 `font_sizes` 字段
(目前仍是旧的 9/10/11/12/13/14/15)要在这次改动里同步更新,`dozer-app`
才能重新变回唯一真相源,而不是继续读着一份过时数据。



**这次改动把 `font`/`geometry`/`icon_size` 的静态基础尺寸(不含
`icon_size` 已经独立于 JSON 的运行时 `scale` 缩放机制)迁到和 `color`
完全一致的模式**:`byteui` 不再内嵌任何 `workspace.json`,只提供
"token 结构体形状 + 一份 Rust 编译期默认值 + 运行时可整体替换"这套机制;
`dozer-app` 保留自己唯一一份 `assets/theme/workspace.json`,启动时解析
后调用 `byteui` 的 `set_theme()` 把这份唯一真相写进去。

## 目标 / 非目标

**目标**:

1. `byteui::theme::font` 新增 `FontTokens` 结构体(7 字段,对应现有 7 个
   accessor)、`FontTokens::byteboy2077()` const 默认值、`current()`/
   `set_theme()`(`RwLock` 包装,和 `color` 同款);删除 `include_str!`/
   `LazyLock<WorkspaceFonts>`/`RawWorkspaceFile`/`load` 这套 JSON 解析
   脚手架。7 个 accessor(`dot_sm`/…/`title`)内部改读 `current()`,
   函数签名不变。
2. `byteui::theme::geometry` 同样新增 `GeometryTokens`(30 字段)、
   `byteboy2077()`、`current()`/`set_theme()`,删除对应 JSON 脚手架。
   32 个 accessor 签名不变,其中 28 个直接读某个字段的内部改读
   `current()`(`initial_window_width`/`initial_window_height` 两个字段
   被 `initial_window_size()` 一个函数合并返回元组,故 30 字段对应 29
   个"读字段"函数,再加 1 个下述例外后共 28+1);**以下 4 个不改动,继续
   保持现状**——`default_split_ratio()`(硬编码字面量 `0.35`,从不在
   JSON 里)、`scrollbar_width()`/`scrollbar_thumb_width()`(硬编码字面量
   `10.0`/`4.0`,同样从不在 JSON 里)、`min_window_width()`(推导函数,
   调用其它 3 个 accessor 算出,不直接对应任何字段)。这 4 个函数现状
   如此,**不属于这次"消灭 JSON 重复"的范围,不新增字段把它们也塞进
   `GeometryTokens`**——那是额外的"把硬编码值也做成可配置 token"的
   功能扩展,超出这次目标,YAGNI。
3. `byteui::theme::icon_size` 新增 `IconSizeTokens`(7 字段:`rail`/
   `row`/`chevron`/`tab_arrow`/`home`/`tree_row_gap`/`scale`,最后这个
   `scale` 字段是"设计基准缩放值",和运行时可变的 `CURRENT_SCALE`
   `AtomicU32` 是两回事,不要混淆)、`byteboy2077()`、`current()`/
   `set_theme()`,删除对应 JSON 脚手架。6 个尺寸 accessor 内部改读
   `current()`;`scale()`/`set_scale()`/`zoom_by()`/`init_scale(path)`/
   `persist_scale(path)`/`reset_scale(path)` 这套运行时缩放机制**完全
   不改**(它已经是正确的"调用方传路径"设计,不涉及这次要解决的 JSON
   重复问题)。
4. 删除 `crates/byteui/assets/theme/workspace.json` 整个文件,`byteui`
   不再内嵌任何 Dozer 专属配置数据。**删除前先把 `font_sizes` 当前值
   (10/11/12/13/14/15/16,`b3f2763` 刚定的)同步进 `crates/dozer-app/
   assets/theme/workspace.json`**——那份文件目前 `font_sizes` 仍是旧值
   (9/10/11/12/13/14/15),这次迁移完成后 `dozer-app` 重新变回唯一真相
   源,必须先把最新值搬回去,不能让这次改动意外把字号退回旧值。
   `geometry`/`icon_sizes`/`regions` 三个节点两份文件当前完全一致,不
   需要额外同步。
5. `dozer-app` 新增 `theme::init()`(放在 `theme.rs`,和现有
   `ui_scale_path()` 同一个文件),`main.rs` 在建窗前调用一次:读
   `crates/dozer-app/assets/theme/workspace.json`(文件位置、内嵌方式
   不变,仍是 `dozer-app` 自己的 `include_str!`),把 `font_sizes`/
   `geometry`/`icon_sizes` 三个节点分别 `serde_json::from_value` 进
   `byteui::theme::font::FontTokens`/`geometry::GeometryTokens`/
   `icon_size::IconSizeTokens`(三个结构体需要 `#[derive(Deserialize)]`,
   字段名就是和 JSON 的契约),调用三次对应 `set_theme()`。
6. 迁移完成后 `dozer-app` 编译、测试、clippy、fmt 全绿,GUI 视觉(字号/
   几何尺寸/图标基础尺寸)与迁移前(以 `dozer-app` 自己 JSON 的值为准,
   不是 `byteui` 那份已经漂移的复制品)逐一核对无差异。

**非目标**:

- **不改变 `icon_size` 的运行时缩放机制**(`scale()`/`set_scale()`/
  `zoom_by()`/`init_scale`/`persist_scale`/`reset_scale`,以及
  `ui_scale_path()` 落盘约定)。这套已经是这次想要的模式(调用方传参、
  不内置 Dozer 路径),不在改动范围。
- **不改变 `theme::color` 的既有实现**——它已经是这次要复制的目标模式,
  这次不碰,只作为参照。
- **不改变 `theme::region`/`theme::terminal_font`/`theme::homespace_color`/
  `theme::homespace_font`**——四者都没有迁移计划(见
  `2026-08-19-dozer-app-font-geometry-migration-design.md` 排期备注),
  这次也不涉及。`region.rs` 的 `regions` JSON 节点、`terminal_font.rs`
  的 `terminal.json` 都保持原样(`dozer-app` 本地 `include_str!`,`byteui`
  从未复制过这两份)。
- **不改变任何调用点**。`byteui::theme::font::body()` 这类函数签名和
  调用方式一字不变——四个迁移子项目改过的约 500+ 处调用点不需要重新碰。
- **不改变任何视觉效果**(相对当前实际生效值——即 `byteui` 那份
  `workspace.json` 删除前的最新值,`font_sizes` 是 `b3f2763` 刚定的
  10/11/12/13/14/15/16,不是 `dozer-app` 那份目前仍滞后的旧值)。
  `FontTokens::byteboy2077()`/`GeometryTokens::byteboy2077()`/
  `IconSizeTokens::byteboy2077()` 三个默认值仅供 `byteui` 自己没被显式
  `set_theme()` 时的兜底(理论上不会发生,因为 `dozer-app::main()` 建窗
  前一定会调用一次 `theme::init()`),数值上没有强制要求和任何一方
  一致,但为了避免开发期兜底路径出现明显走样,仍取当前实际生效值作为
  这份默认值。

## 架构与数据流

### `byteui::theme::font` 改动示例(`geometry`/`icon_size` 同构,篇幅原因
只展开 `font` 一份,`geometry`/`icon_size` 在实现计划里逐字段列全)

```rust
// crates/byteui/src/theme/font.rs
use serde::Deserialize;
use std::sync::RwLock;

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct FontTokens {
    pub dot_sm: u32,
    pub caption_sm: u32,
    pub caption: u32,
    pub label: u32,
    pub body: u32,
    pub subtitle: u32,
    pub title: u32,
}

impl FontTokens {
    /// 逐一对应 `b3f2763` 定的当前基准值(body=14px,对齐终端字号),
    /// 仅作未显式 `set_theme()` 时的兜底默认值——实现时先确认
    /// `crates/byteui/assets/theme/workspace.json` 删除前的最新
    /// `font_sizes` 取值,这里只是示例,不是最终要抄的数字来源。
    pub const fn byteboy2077() -> Self {
        Self {
            dot_sm: 10,
            caption_sm: 11,
            caption: 12,
            label: 13,
            body: 14,
            subtitle: 15,
            title: 16,
        }
    }
}

static CURRENT: RwLock<FontTokens> = RwLock::new(FontTokens::byteboy2077());

pub fn current() -> FontTokens {
    *CURRENT.read().expect("byteui font RwLock poisoned")
}

/// 整体替换当前字号 token——供调用方(如 dozer-app::theme::init())在
/// 启动时用自己的 workspace.json 覆盖默认值。
pub fn set_theme(tokens: FontTokens) {
    *CURRENT.write().expect("byteui font RwLock poisoned") = tokens;
}

pub fn dot_sm() -> u32 {
    scale(current().dot_sm)
}
pub fn caption_sm() -> u32 {
    scale(current().caption_sm)
}
pub fn caption() -> u32 {
    scale(current().caption)
}
pub fn label() -> u32 {
    scale(current().label)
}
pub fn body() -> u32 {
    scale(current().body)
}
pub fn subtitle() -> u32 {
    scale(current().subtitle)
}
pub fn title() -> u32 {
    scale(current().title)
}

fn scale(base: u32) -> u32 {
    ((base as f32) * super::icon_size::scale()).round() as u32
}
```

`geometry.rs` 的 `GeometryTokens` 30 个字段和现有 `Geometry` 结构体逐一
对应(字段名不变);`icon_size.rs` 的 `IconSizeTokens` 7 个字段
(`rail`/`row`/`chevron`/`tab_arrow`/`home`/`tree_row_gap`/`scale`)和
现有 `IconSizes` 结构体逐一对应。三处的 `RAW`/`LazyLock`/`load`/
`RawWorkspaceFile`(或等价命名)全部删除,`use serde::Deserialize;` 保留
(结构体需要),`use std::sync::LazyLock;` 删除,新增
`use std::sync::RwLock;`。

`geometry.rs` 里 `min_window_width()` 这类**推导函数**(不直接读某个
JSON 字段,而是调用其它 accessor 算出来的)不受影响,原样保留:

```rust
pub fn min_window_width() -> f32 {
    2.0 * icon_rail_width() + divider_width() + 2.0 * min_zone_width()
}
```

### `dozer-app::theme::init()`

```rust
// crates/dozer-app/src/theme.rs 新增
use byteui::theme::{font::FontTokens, geometry::GeometryTokens, icon_size::IconSizeTokens};

const WORKSPACE_JSON: &str = include_str!("../assets/theme/workspace.json");

#[derive(serde::Deserialize)]
struct RawWorkspaceFile {
    font_sizes: FontTokens,
    geometry: GeometryTokens,
    icon_sizes: IconSizeTokens,
}

/// 启动时把 dozer-app 自己的 `workspace.json` 灌进 byteui 三个 token
/// 模块,取代它们编译期内置的 ByteBoy2077 默认值。必须在建窗、任何
/// 渲染逻辑跑之前调用一次(main.rs 里紧挨着现有 `byteui::theme::
/// icon_size::init_scale(&ui_scale_path())` 之前)。解析失败(格式
/// 错误、缺字段)直接 panic——开发期配置错误,不是需要优雅降级的
/// 运行时数据(同 `region.rs`/`terminal_font.rs` 一贯的定位)。
pub(crate) fn init() {
    let raw: RawWorkspaceFile =
        serde_json::from_str(WORKSPACE_JSON).expect("workspace.json 格式错误(解析失败)");
    byteui::theme::font::set_theme(raw.font_sizes);
    byteui::theme::geometry::set_theme(raw.geometry);
    byteui::theme::icon_size::set_theme(raw.icon_sizes);
}
```

`main.rs` 里的调用点(紧挨在现有 `byteui::theme::icon_size::init_scale(
&crate::theme::ui_scale_path())` 之前,先建立 token 基准值,再恢复持久化
的缩放倍数——两者是正交的,谁先谁后不影响正确性,但保持"先建基准再套
缩放"这个顺序更符合直觉):

```rust
crate::theme::init();
byteui::theme::icon_size::init_scale(&crate::theme::ui_scale_path());
```

### 同步 `dozer-app` 的 `workspace.json`,再删除 `byteui` 自带的那份

**顺序不能反**——先把 `byteui` 那份最新的 `font_sizes` 抄回 `dozer-app`
自己的 `workspace.json`(其余 `geometry`/`icon_sizes`/`regions` 三个节点
两份当前一致,不用抄),再删 `byteui` 那份文件:

```bash
python3 - <<'EOF'
import json
byteui = json.load(open("crates/byteui/assets/theme/workspace.json"))
dozer = json.load(open("crates/dozer-app/assets/theme/workspace.json"))
dozer["font_sizes"] = byteui["font_sizes"]
json.dump(dozer, open("crates/dozer-app/assets/theme/workspace.json", "w"), indent=2, ensure_ascii=False)
EOF
rm crates/byteui/assets/theme/workspace.json
```

(用 `python3`/`json` 而不是手改,是为了保留 `dozer-app` 那份文件其余
字段和格式不被无意改动;真正执行时用 `git diff` 确认这一步只改了
`font_sizes` 那 7 行。)

删除后 `crates/byteui/assets/theme/` 目录为空,一并删除该空目录。
`region.rs`/`terminal_font.rs` 从未被 `byteui` 复制过,不受影响。

## 错误处理

`dozer-app::theme::init()` 解析失败直接 panic,和 `region.rs`/
`terminal_font.rs` 一贯的"开发期配置错误"定位一致——这是本来就存在的
错误处理策略(原先 `byteui` 内部的 `load()` 函数就是这么做的,只是
panic 点从 `byteui` crate 内部搬到了 `dozer-app::theme::init()`)。

`byteui` 三个模块的 `current()`/`set_theme()` 用 `RwLock`,`.expect(...)`
处理中毒(poisoned)——和 `color.rs` 完全一致的错误处理,不是这次新引入
的模式。

## 测试策略

1. `byteui` 三个模块各自的单测按 `color.rs` 的模式重写(不再有
   `malformed_json_panics` 这类测试,因为 JSON 解析已经不在 `byteui`
   里了):
   - `byteboy2077_matches_dozer_app_baseline`:断言 `byteboy2077()` 的
     每个字段值和 `dozer-app` 当前 `assets/theme/workspace.json` 里的
     字面量一致(纯代码搬家,数值不该变——延续现有
     `tokens_match_pre_migration_literals`/`values_match_pre_migration_
     literals` 这类"防漂移锚"测试的精神,只是断言对象从"解析 JSON 的
     结果"变成"`byteboy2077()` 常量本身")。
   - `current_defaults_to_byteboy2077`:未调用 `set_theme()` 时
     `current()` 应等于默认值。
   - `set_theme_replaces_current_and_is_visible_globally`:替换后
     `current()` 立即反映新值;测试末尾调用
     `set_theme(XxxTokens::byteboy2077())` 复原,避免污染同进程里跑在
     本测试之后的其它测试(照抄 `color.rs` 现有测试的写法)。
2. `dozer-app::theme::init()` 新增测试:用一份构造好的合法 JSON 字符串
   验证三个 `set_theme()` 都被正确调用(可以检查调用后 `byteui::theme::
   font::current()` 等于预期值);另一个测试验证格式错误的 JSON 触发
   panic(`#[should_panic]`)。
3. `cargo build -p byteui`(独立编译,验证不依赖 `dozer-app`)、
   `cargo build -p dozer-app --bin dozer`、`cargo test -p byteui`、
   `cargo test -p dozer-app --bin dozer`(测试数量会因为删掉
   `malformed_json_panics` 这类 JSON 解析专属测试、新增
   `byteboy2077_matches_dozer_app_baseline` 这类新测试而有增减,不强求
   总数和之前基线一致,只要求没有非预期失败)、`cargo clippy --all-targets
   -- -D warnings`、`cargo fmt -- --check`(两个 crate 都跑)。
4. 独立命名的临时二进制做一次视觉核对(这次唯一有真实回归风险的地方:
   `FontTokens::byteboy2077()`/`GeometryTokens::byteboy2077()`/
   `IconSizeTokens::byteboy2077()` 手抄 `dozer-app` JSON 值时如果抄错
   一个数字,不会编译报错,只会运行时渲染出偏差):全 App 字号/几何尺寸
   走一遍,和迁移前(以 `dozer-app` 自己 JSON 为准)比对无差异。完成后
   关闭该临时实例、删除临时二进制,不留后台进程。

## 排期备注

`font`/`geometry`/`icon_size` 三个模块相互独立,可以按任意顺序做、分批
提交分批审阅(不像四个迁移子项目那样有调用点覆盖全仓库的问题——这次
改动完全封装在 `byteui` crate 内部 + `dozer-app::theme::init()` 一个
新函数,不需要碰任何业务代码调用点)。删除
`crates/byteui/assets/theme/workspace.json` 建议放在三个模块都改完之后
再做,避免中间状态下某个还没改完的模块的 `include_str!` 找不到文件。

这次改动之后,`byteui` 不再内嵌任何 Dozer 专属数据——`assets/theme/`
目录整个消失,`assets/icons/` 目录(SVG 图标资源)不受影响,那些是
`byteui::interaction::icons` 真正通用、不含 Dozer 专属数值的静态资源,
继续留在 `byteui` 里。
