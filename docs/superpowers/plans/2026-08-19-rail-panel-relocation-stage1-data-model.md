# workspace 图标栏面板拖拽换栏 · Stage 1:数据模型统一 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `LeftView`(7 variant)+ `RightView`(4 variant)合并成一个
统一的 `PanelKind`(11 variant),新增 `Side`/`RailLayout` 数据结构并接入
持久化。**这一阶段不改变任何行为**——面板依然固定在各自原来那一侧,
没有拖拽、没有镜像渲染、图标栏依然是静态 `column!`。跑完这个 Stage,
GUI 应该和改动前逐像素一致,唯一区别是内部类型换了名字、`layout.json`
多存了一份(暂时没人读)的 `RailLayout`。

**Architecture:** `LeftView`/`RightView` 两个类型定义合并成一个
`PanelKind`(variant 名字逐一沿用,不改名——`LeftView::Files` 直接变成
`PanelKind::Files`)。因为 Rust 的类型系统要求同一次编译里所有引用
必须一致,这次合并**不能**像 `byteui` 迁移系列那样按文件独立处理、
每个文件改完单独 `cargo build` 通过——把类型从 7/4 variant 扩到 11
variant 后,原来穷尽 7 个或 4 个 variant 的 `match` 会变成不穷尽(还缺
另外 4 个或 7 个),要逐个补 `_ => unreachable!(...)` 兜底分支,这一步
只有等**全部**引用点都指向新类型后才能靠 `cargo check` 报错去逐条修完。
所以这个 Stage 的中间 Task **不会**每个都编译通过,只有最后一个 Task
才是"全绿"的验证点——这是和之前几次 `byteui` 迁移计划的关键差异,
执行时不要因为中途 `cargo build` 报错就以为哪一步做错了。

**Tech Stack:** Rust 2024,serde(`RailLayout` 持久化复用现有
`ShellLayout`/`layout.rs` 的 `#[serde(default)]` 老文件兼容套路)。

**Spec:** `docs/superpowers/specs/2026-08-19-rail-panel-drag-relocation-design.md`

## Global Constraints

- **独立分支开发,不直接提交 main。** 在新分支(如
  `feature/rail-panel-relocation-stage1`)上完成全部 7 个 Task,提请
  审阅、通过后再合并回 `main`。每次 `git commit`/`git add` 前先跑
  `git branch --show-current` 确认当前分支是自己的迁移分支;开工前
  `git status` 确认干净,发现不相关的改动要 `git stash push -u -m "..."`
  保留,不要清掉。
- **这一阶段不接入任何新行为。** `RailLayout` 落盘但暂时没有渲染/交互
  逻辑读它;`PanelKind` 只是 `LeftView`+`RightView` 的类型合并,取值范围
  在这个 Stage 结束时依然严格符合"7 个原 `LeftView` variant 只会出现在
  `left_view` 字段、4 个原 `RightView` variant 只会出现在 `right_view`
  字段"这条不变式——所有因此变得"不穷尽"的 `match` 补的兜底分支必须是
  `unreachable!(...)`(带清楚说明"Stage 4 加拖拽前这个分支不会走到"的
  文案),不是随手拍一个默认行为糊弄过去。
- **Task 1-6 中间状态不要求 `cargo build` 通过**,但每个 Task 结束时
  用该 Task 描述的**残留检查 grep** 确认"这个 Task 该改的都改了、不多
  不少",作为该 Task 自己范围内的正确性把关。Task 7 是唯一要求全量编译
  通过的验证点。
- **不改 `RailButton`/`Message::LeftIconSelect`/`RightIconSelect`/
  `left_icon_select`/`right_icon_select` 的整体形状**(除了给
  `LeftIconSelect`/`right_icon_select`/`left_icon_select` 的参数类型
  从 `LeftView`/`RightView` 换成 `PanelKind`——纯换类型标注,函数体、
  调用方式不动)。把两个 select 消息/处理函数合并成一个side-agnostic 的
  `PanelSelect`,以及让图标栏改成按 `RailLayout` 遍历渲染,是 Stage 2
  的范围,这次不做。

---

### Task 1: 新增 `Side`/`RailLayout` 类型 + 持久化(纯新增,不碰旧代码)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`(新增类型定义,建议紧邻现有
  `ShellLayout` 定义处)
- Modify: `crates/dozer-app/src/layout.rs`(消毒规则追加)

**Interfaces:**
- Produces: `pub enum Side { Left, Right }`、
  `pub struct RailLayout { pub left: Vec<PanelKind>, pub right: Vec<PanelKind> }`
  (`PanelKind` 在 Task 2 才定义,这个 Task 先把 `RailLayout` 写好,
  Task 2 完成后才能真正编译通过——这是这个 Stage 里少数几个"写代码时
  用到还不存在的类型"的地方,属于预期中的中间不可编译状态)、
  `RailLayout::side(&self, side: Side) -> &Vec<PanelKind>`/
  `side_mut(&mut self, side: Side) -> &mut Vec<PanelKind>`、
  `RailLayout::default()`。

- [ ] **Step 1: 在 `app.rs` 里 `ShellLayout` 定义附近新增 `Side`/`RailLayout`**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// 每个面板当前挂在哪条图标栏、栏内什么顺序——图标栏拖拽换栏(Stage 4)
/// 的唯一真相源。这个 Stage 只负责定义类型 + 持久化,渲染/交互还没有
/// 任何地方读它(那是 Stage 2/4 的范围),所以此刻它的值必然等于
/// `default()`,不会有别的取值出现。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RailLayout {
    pub left: Vec<PanelKind>,
    pub right: Vec<PanelKind>,
}

impl RailLayout {
    pub fn side(&self, side: Side) -> &Vec<PanelKind> {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    pub fn side_mut(&mut self, side: Side) -> &mut Vec<PanelKind> {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }
}

impl Default for RailLayout {
    fn default() -> Self {
        Self {
            left: vec![
                PanelKind::Project,
                PanelKind::Todo,
                PanelKind::Files,
                PanelKind::GitLog,
                PanelKind::Database,
                PanelKind::Ssh,
                PanelKind::Web,
            ],
            right: vec![
                PanelKind::Agent,
                PanelKind::Conversations,
                PanelKind::Usage,
                PanelKind::Acceptance,
            ],
        }
    }
}
```

- [ ] **Step 2: `ShellLayout` 新增 `rail_layout` 字段**

在 `pub struct ShellLayout { ... }` 里追加:

```rust
    #[serde(default)]
    pub rail_layout: RailLayout,
```

`ShellLayout` 的 `impl Default` 里(现有 `window_width`/`window_height`
两个字段旁边)追加:

```rust
            rail_layout: RailLayout::default(),
```

- [ ] **Step 3: `layout.rs` 消毒规则追加**

`crates/dozer-app/src/layout.rs` 的 `load_from`(或
`sanitize_shell_layout`,视现有函数划分而定——先读一遍现有
`sanitize_shell_layout` 函数体确认插入点)追加:若
`layout.rail_layout.left`/`.right` 两者合计不是恰好 11 个不重复的
`PanelKind`,或任一为空,整个 `rail_layout` 字段回落
`RailLayout::default()`(不做部分修复)。

```rust
fn sanitize_rail_layout(rail: RailLayout) -> RailLayout {
    if rail.left.is_empty() || rail.right.is_empty() {
        return RailLayout::default();
    }
    let mut all: Vec<_> = rail.left.iter().chain(rail.right.iter()).collect();
    all.sort_by_key(|k| format!("{k:?}"));
    all.dedup();
    if all.len() != 11 || rail.left.len() + rail.right.len() != 11 {
        return RailLayout::default();
    }
    rail
}
```

在 `sanitize_shell_layout`(或 `load_from`,按现状调用点)里对
`loaded.rail_layout` 调这个函数。

- [ ] **Step 4: 新增测试**

在 `app.rs`(`RailLayout` 定义附近)或 `layout.rs` 现有测试模块里追加:

```rust
#[test]
fn rail_layout_default_covers_all_panels_without_duplicates() {
    let rail = RailLayout::default();
    assert_eq!(rail.left.len(), 7);
    assert_eq!(rail.right.len(), 4);
    let mut all: Vec<_> = rail.left.iter().chain(rail.right.iter()).collect();
    all.sort_by_key(|k| format!("{k:?}"));
    all.dedup();
    assert_eq!(all.len(), 11, "11 个面板不重不漏分到左右两栏");
}

#[test]
fn rail_layout_side_accessors_map_correctly() {
    let rail = RailLayout::default();
    assert_eq!(rail.side(Side::Left), &rail.left);
    assert_eq!(rail.side(Side::Right), &rail.right);
}
```

在 `layout.rs` 测试模块里追加(模仿现有 `sanitize_shell_layout` 测试的
写法,用临时文件路径,不碰用户真实配置目录):

```rust
#[test]
fn corrupted_rail_layout_falls_back_to_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layout.json");
    std::fs::write(&path, r#"{"rail_layout": {"left": [], "right": ["Agent"]}}"#).unwrap();
    let loaded = load_from(&path);
    assert_eq!(loaded.rail_layout, RailLayout::default());
}
```

- [ ] **Step 5: Commit(先不 build——`PanelKind` 还没定义,Task 2 才补上)**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs crates/dozer-app/src/layout.rs
git commit -m "feat(dozer-app): 新增 Side/RailLayout 类型 + 持久化,尚未接入任何渲染逻辑"
```

---

### Task 2: 定义 `PanelKind`,让 Task 1 的代码先能编译

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces: `pub enum PanelKind { Files, GitLog, Todo, Project, Database, Ssh, Web, Agent, Conversations, Usage, Acceptance }`
  (`Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize`)。
  `LeftView`/`RightView` 两个旧枚举**先不删**(Task 7 才删)——这个
  Task 结束后代码库处于"`PanelKind` 已存在、`LeftView`/`RightView`
  也还在、两者互不相干"的过渡态,预期不能整体编译通过(`RailLayout`
  能编译了,但 `App`/`ShellState`/`PanelLayout` 等结构体的
  `left_view`/`right_view` 字段还是旧类型,和后续 Task 要做的retype
  无关,此刻不会报错;真正的编译错误来源是 Task 1 里提到的
  `RailLayout::default()` 引用 `PanelKind::X`——这个 Task 让它们
  终于能解析)。

- [ ] **Step 1: 在 `LeftView`/`RightView` 定义附近新增 `PanelKind`**

```rust
/// `LeftView`(7)+ `RightView`(4)的合并类型——workspace 图标栏拖拽
/// 换栏功能(见 `2026-08-19-rail-panel-drag-relocation-design.md`)的
/// 统一面板标识。Stage 1(这次)只做类型定义,`left_view`/`right_view`
/// 等字段还没退型完成前,`LeftView`/`RightView` 两个旧类型仍并存
/// (Task 7 删除)。variant 名字逐一沿用旧枚举,不改名。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelKind {
    Files,
    GitLog,
    Todo,
    Project,
    Database,
    Ssh,
    Web,
    Agent,
    Conversations,
    Usage,
    Acceptance,
}
```

- [ ] **Step 2: 编译检查(此刻应该只报 `PanelKind` 相关的"未使用"警告,
  不应该报 `RailLayout`/`Side` 相关的解析错误)**

Run: `cargo check -p dozer-app --bin dozer 2>&1 | grep -c "^error"`
Expected: 输出 `0`(允许有 warning,不允许有 error——如果这里报错,
说明 Task 1 写的 `RailLayout::default()` 引用的 variant 名字和这里
定义的对不上,回去核对拼写)。

- [ ] **Step 3: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 新增 PanelKind(LeftView+RightView 合并类型),旧枚举暂未删除"
```

---

### Task 3: 4 处结构体字段/函数参数类型从 `LeftView`/`RightView` 退型成 `PanelKind`

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `PanelKind`(Task 2)
- Produces: `PanelLayout.left_view: PanelKind`、`PanelLayout.right_view: PanelKind`、
  `ShellState.left_view: PanelKind`、`ShellState.right_view: PanelKind`、
  `App.left_view: PanelKind`、`App.right_view: PanelKind`、
  `keyboard_term_target(left_view: PanelKind, ...)`。**这个 Task 结束后
  代码库大范围编译不过(预期中)**——所有原来读写这 4 处的调用点还在用
  `LeftView::X`/`RightView::Y`,类型不匹配,Task 4/5/6 逐个修。

- [ ] **Step 1: `PanelLayout` 结构体(`app.rs` 里 `pub(crate) struct PanelLayout`,
  当前 `left_view: LeftView`/`right_view: RightView` 两行)**

把这两行的类型标注从 `LeftView`/`RightView` 改成 `PanelKind`:

```rust
    pub left_view: PanelKind,
    pub right_view: PanelKind,
```

- [ ] **Step 2: `ShellState` 结构体(当前 `left_view: LeftView`/
  `right_view: RightView` 两行)**

同样把类型标注改成 `PanelKind`。

- [ ] **Step 3: `App` 结构体(当前 `left_view: LeftView`/
  `right_view: RightView` 两个私有字段,紧邻 `panel_layouts:
  HashMap<i64, PanelLayout>`)**

同样把类型标注改成 `PanelKind`。

- [ ] **Step 4: `keyboard_term_target` 函数签名**

```rust
pub(crate) fn keyboard_term_target(
    left_view: PanelKind,
    active_zone: Option<ZoneSide>,
) -> TermTarget {
```

- [ ] **Step 5: `left_icon_select`/`right_icon_select` 方法签名**

```rust
    fn left_icon_select(&mut self, v: PanelKind) {
```

```rust
    fn right_icon_select(&mut self, v: PanelKind) {
```

(方法体内部这次先不动——Task 4 的全局 `LeftView::`/`RightView::` 替换
会把方法体里的 `LeftView::GitLog`/`RightView::Usage` 这类比较也换成
`PanelKind::`,到时候一起处理,不需要在这个 Task 里手动改方法体。)

- [ ] **Step 6: `Message::LeftIconSelect`/`RightIconSelect` 变体签名**

```rust
    LeftIconSelect(PanelKind),
    ...
    RightIconSelect(PanelKind),
```

- [ ] **Step 7: 残留检查——确认这 4 处结构体 + 2 个函数签名 + 1 个消息
  变体已经全部退型,没有漏改**

Run: `grep -n "left_view: LeftView\|right_view: RightView\|left_view: LeftView,\|left_view: LeftView)\|v: LeftView\|v: RightView\|LeftIconSelect(LeftView)\|RightIconSelect(RightView)" crates/dozer-app/src/app.rs`
Expected: 无输出

- [ ] **Step 8: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "refactor(dozer-app): PanelLayout/ShellState/App 的 left_view/right_view 字段retype 为 PanelKind"
```

---

### Task 4: `app.rs` 全局 `LeftView::`/`RightView::` → `PanelKind::` + 补穷尽性兜底分支

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: Task 3 完成的字段退型
- Produces: `app.rs` 内部对 `LeftView::X`/`RightView::Y` 的引用全部改写
  成 `PanelKind::X`/`PanelKind::Y`;因合并成 11-variant 类型而不再穷尽
  的 `match` 补 `_ => unreachable!(...)` 分支。

摸底时确认 `app.rs` 里 `LeftView::`/`RightView::` 各有约 74/31 处
(2026-08-19 摸底数字,执行时以 `grep -c` 现测为准,不要直接信这个
历史数字)。这一步是这个 Stage 里改动量最大的单个 Task。

- [ ] **Step 1: 全局替换**

```bash
sed -i '' \
  -e 's/LeftView::/PanelKind::/g' \
  -e 's/RightView::/PanelKind::/g' \
  crates/dozer-app/src/app.rs
```

**这条 sed 会连带替换掉 `HomeLeftView::`/`HomeRightView::`(首页专属,
和这次改动无关)里的 `LeftView::`/`RightView::` 子串,产出错误的
`HomePanelKind::`。** 跑完这条规则后必须立即执行 Step 2 检查并修正。

- [ ] **Step 2: 修正 `HomeLeftView`/`HomeRightView` 被误伤的部分**

Run: `grep -n "HomePanelKind::" crates/dozer-app/src/app.rs`

对每一处命中,把 `HomePanelKind::` 改回 `HomeLeftView::` 或
`HomeRightView::`(按上下文——原来是 `HomeLeftView::` 的现在应该是
`HomeLeftView::`,原来是 `HomeRightView::` 的现在应该是
`HomeRightView::`;这两个类型定义在 `homespace.rs`,这个 Task 不碰
那个文件,只处理 `app.rs` 里对它们的引用)。也检查
`crates/dozer-app/src/homespace.rs`/`crates/dozer-app/src/extensions/
usage.rs` 等文件是否被误伤(这条 sed 只跑在 `app.rs` 上,理论上不会
波及其它文件,但仍需跑一次全仓库检查确认):

Run: `grep -rn "HomePanelKind::" crates/dozer-app/src --include="*.rs"`
Expected: 修正完后无输出

- [ ] **Step 3: 用 `cargo check` 找出所有因 11-variant 化而不再穷尽的
  `match`,逐个补 `unreachable!` 兜底分支**

Run: `cargo check -p dozer-app --bin dozer 2>&1 | grep -B2 "non-exhaustive patterns"`

对每一处报错,打开对应位置,在 `match` 最后追加:

```rust
            _ => unreachable!(
                "Stage 1(数据模型统一)阶段面板还固定在各自原侧,\
                 state.left_view 不会取到对侧的 PanelKind——Stage 4 \
                 加拖拽后这里要重新设计,不能再用 unreachable"
            ),
```

(`unreachable!`/`format!` 的 `{}` 是插值语法,文案里不要出现花括号;
每处的被匹配对象名字——上面示例是 `state.left_view`——要换成实际代码
里那个 `match` 表达式对应的变量名,不要整个 `app.rs` 里贴一模一样的
字符串。字符串末尾的 `\` 是行内续行转义,不产生换行,不是必需项,只是
让长文案在源码里换行更好读。)

重复"跑 `cargo check` → 补一处 → 再跑"直到不再报
`non-exhaustive patterns`。**这一步不要求整个 crate 编译通过**(其它
文件如 `panel_layouts.rs`/`main.rs` 还没改,会报别的错误),只要求
`non-exhaustive patterns` 类别的错误在 `app.rs` 范围内清零:

Run: `cargo check -p dozer-app --bin dozer 2>&1 | grep "non-exhaustive patterns" | grep "app.rs"`
Expected: 无输出

- [ ] **Step 4: 确认 `app.rs` 里不再有 `LeftView`/`RightView` 残留
  (类型名本身,不只是 `::` 调用形式——`Message::LeftIconSelect`
  这类已在 Task 3 处理过的部分应该也不在此列)**

Run: `grep -n "\bLeftView\b\|\bRightView\b" crates/dozer-app/src/app.rs`
Expected: 无输出

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "refactor(dozer-app): app.rs 全量 LeftView::/RightView:: 改写为 PanelKind::,补穷尽性兜底分支"
```

---

### Task 5: `panel_layouts.rs` 迁移

**Files:**
- Modify: `crates/dozer-app/src/panel_layouts.rs`

**Interfaces:**
- Consumes: `PanelKind`(Task 2)

- [ ] **Step 1: 修正 import**

找到:

```rust
    use crate::app::{LeftView, PanelDims, RightView};
```

改成:

```rust
    use crate::app::{PanelDims, PanelKind};
```

- [ ] **Step 2: 全局替换该文件内的 `LeftView::`/`RightView::`**

```bash
sed -i '' \
  -e 's/LeftView::/PanelKind::/g' \
  -e 's/RightView::/PanelKind::/g' \
  crates/dozer-app/src/panel_layouts.rs
```

- [ ] **Step 3: 残留检查**

Run: `grep -n "\bLeftView\b\|\bRightView\b" crates/dozer-app/src/panel_layouts.rs`
Expected: 无输出

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/panel_layouts.rs
git commit -m "refactor(dozer-app): panel_layouts.rs 迁移到 PanelKind"
```

---

### Task 6: `main.rs` 迁移

**Files:**
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: `PanelKind`(Task 2)

- [ ] **Step 1: 修正 import**

找到:

```rust
use app::{App, LeftView, Message};
```

改成:

```rust
use app::{App, Message, PanelKind};
```

- [ ] **Step 2: 全局替换该文件内的 `LeftView::`**(`main.rs` 没有
  `RightView::` 引用,只替换 `LeftView::` 即可)

```bash
sed -i '' -e 's/LeftView::/PanelKind::/g' crates/dozer-app/src/main.rs
```

- [ ] **Step 3: 修正 `app.left_view()` 调用点的类型注解(如果有显式标注)**

Run: `grep -n "left_view()" crates/dozer-app/src/main.rs`

检查 `crate::app::LeftView::Todo` 这类完整路径写法(2087 行附近,
`matches!(app.left_view(), crate::app::LeftView::Todo)`)是否被 Step 2
的 sed 正确处理——`crate::app::LeftView::Todo` 这种带完整路径前缀的
写法,`LeftView::` 子串一样会被替换成 `PanelKind::`,产出
`crate::app::PanelKind::Todo`,这是正确结果,不需要额外处理,这一步
只是确认 sed 确实覆盖到了这种写法。

- [ ] **Step 4: 残留检查**

Run: `grep -n "\bLeftView\b\|\bRightView\b" crates/dozer-app/src/main.rs`
Expected: 无输出

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/main.rs
git commit -m "refactor(dozer-app): main.rs 迁移到 PanelKind"
```

---

### Task 7: 删除 `LeftView`/`RightView` 旧枚举 + 全量验证 + 外围文档注释扫尾

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/workspace.rs`(文档注释,非代码)
- Modify: `crates/dozer-app/src/extensions/usage.rs`(文档注释,非代码)
- Modify: `crates/dozer-app/src/extensions/project.rs`(文档注释,非代码)
- Modify: `crates/dozer-app/src/extensions/todo.rs`(文档注释,非代码)

**Interfaces:**
- Consumes: Task 1-6 已完成(全仓库零处引用 `LeftView`/`RightView` 作为
  真实类型)

- [ ] **Step 1: 全仓库确认已无残留引用(收尾前的最后一道保险,排除
  `homespace.rs` 的 `HomeLeftView`/`HomeRightView`——那是首页专属类型,
  不受这次改动影响,天然会被下面的 grep pattern 排除,因为 pattern 是
  精确词边界匹配 `LeftView`/`RightView` 不含 `Home` 前缀)**

Run: `grep -rn "\bLeftView\b\|\bRightView\b" crates/dozer-app/src --include="*.rs"`
Expected: 无输出(如果有输出,说明 Task 1-6 里漏了某处,先回去补上,
不要继续本 Task)

- [ ] **Step 2: 删除 `LeftView`/`RightView` 枚举定义**

在 `app.rs` 里找到:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LeftView {
    Files,
    GitLog,
    Todo,
    Project,
    Database,
    Ssh,
    Web,
}
```

和:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RightView {
    Agent,
    Conversations,
    Usage,
    Acceptance,
}
```

整段删除(含各自的文档注释)。

- [ ] **Step 3: 外围文件文档注释扫尾(不影响编译,只是让注释不继续
  提旧类型名,保持文档准确)**

以下 4 个文件里的文档注释提到 `LeftView`/`RightView`(不是真实代码
引用,是 `///` 注释里的文字),各自找到对应行,把 `LeftView`/`RightView`
改成 `PanelKind`:

- `crates/dozer-app/src/workspace.rs`:2 处(`不挂 LeftView`、
  `LeftView::Files 在没有打开项目时的占位`)
- `crates/dozer-app/src/extensions/usage.rs`:2 处(`RightIconSelect(RightView::Usage)`)
- `crates/dozer-app/src/extensions/project.rs`:1 处(`LeftView::Project`)
- `crates/dozer-app/src/extensions/todo.rs`:1 处(`LeftView::Todo`)

- [ ] **Step 4: 全量编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功(这是这个 Stage 第一次、也是唯一一次要求整体
编译通过的检查点)

- [ ] **Step 5: 全量测试**

Run: `cargo test -p dozer-app --bin dozer`
Expected: 全部通过,测试数量比 Stage 1 开工前的基线略多(Task 1 新增
的 `RailLayout` 相关测试),不强求总数精确匹配某个数字,只要求没有
非预期失败。

- [ ] **Step 6: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 无新增警告(执行前可以先在 `main` 上跑一遍同样的命令留一份
基线输出,和这次结果比对,确认没有新增,已知的历史遗留警告不算)、
无格式差异。

- [ ] **Step 7: 独立临时二进制视觉核对**

构建一个独立命名的临时二进制(不要用会撞到用户正在跑的正式
`/Applications/Dozer AI Coder.app` 或其他调试会话进程名的路径),启动
后核对:**11 个面板全部行为与改动前完全一致**——每个图标点击后开在
原来的那一侧,内部布局(项目树/预览的左右顺序等)一字不变,退出重开
后行为依然一致。这个 Stage 纯粹是类型层面的重构,GUI 上应该看不出
任何差异;如果看出差异,说明某处 `unreachable!` 兜底分支实际被走到了
(意味着摸底阶段对"这个 variant 不会出现在这个字段"的假设有误,需要
回去排查是 Task 4 哪个 `match` 补错了兜底逻辑,而不是继续往下走)。

完成后关闭该临时实例、删除临时二进制,不留后台进程。

- [ ] **Step 8: Commit**

```bash
git branch --show-current
git add -A
git commit -m "refactor(dozer-app): 删除 LeftView/RightView 旧枚举,Stage 1(数据模型统一)收尾"
```

---

## 完工验收

1. `git log --oneline` 确认全部 7 个 Task 的 commit 都在当前分支上,
   没有漂到 `main`。
2. `grep -rn "\bLeftView\b\|\bRightView\b" crates/dozer-app/src` 全仓库
   零匹配(`HomeLeftView`/`HomeRightView` 除外)。
3. `cargo build`/`cargo test`/`cargo clippy`/`cargo fmt --check` 全绿。
4. GUI 视觉核对:11 个面板行为与 Stage 1 开工前逐一致,零差异。
5. 提请审阅。审阅通过合并后,Stage 2(图标栏渲染改遍历
   `RailLayout` + `PanelSelect` 消息统一)才能开工——它依赖这次
   `PanelKind`/`RailLayout` 已经落地。
