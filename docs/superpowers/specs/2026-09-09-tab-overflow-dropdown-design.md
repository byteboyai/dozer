# Tab 组溢出下拉设计

**状态:已批准(brainstorming 会话,2026-09-09)**

## 背景

目前有 5 处 tab 组用 `tab_widget::tab_window` + 左右箭头按钮做"溢出翻页":

1. 终端会话 tab(`terminal.rs:107-179 tab_bar`,状态 `term_tab_first`,
   `Message::TermTabScroll(bool)`)
2. 文件预览 tab(`workspace.rs preview_pane_for`,状态 `preview_tab_first`,
   `Message::PreviewTabScroll`)
3. 项目预览 tab(同一个 `preview_pane_for`,状态 `project_preview_tab_first`,
   `Message::ProjectPreviewTabScroll`)
4. SSH tab(`app.rs:9395-9560 ssh_tab_bar`,状态 `ssh_tab_first`,
   `Message::Ssh(ssh::Message::TabScroll(bool))`)
5. Database 连接 tab(`extensions/database.rs`,状态
   `DatabaseContentState.tab_scroll_first`,
   `Message::Database(database::Message::TabScroll(bool))`)

可视 tab 数很少时,靠一次滚一个的箭头要点很多下才能翻到目标 tab,且看不到
"当前到底还有哪些 tab 挤在外面"的全貌。截图展示的目标效果:tab 组右侧放一个
V(chevron-down)按钮,点开后弹出悬浮列表,列出当前被挤出去看不到的 tab(带图标
+文字+关闭按钮,选中/hover 高亮),点某一项直接跳转过去。

探索还发现一个既有小缺口:现状选中 tab(点击可见 tab、`SelectTab` 等消息)从
不会触发窗口滚动,`first` 只在手动点箭头或关 tab/切项目时改变。目前因为选中的
唯一入口就是点可见 tab 本身,这个缺口不会被触发;但这次改造引入"从下拉里选中
一个隐藏 tab"这个新入口后,如果不管,会出现"选中了但主条上看不到高亮在哪"的
体验问题,顺带一并修掉。

## 目标 / 非目标

**目标**:

1. 上述 5 处调用点的 `<`/`>` 箭头全部替换成"tab 组右侧一个 V 按钮 + 悬浮
   下拉列表"。
2. V 按钮仅在存在溢出(有 tab 被挤出可见区)时渲染;不溢出时完全不出现,不
   走"禁用态灰按钮"那一套。
3. 下拉列表只列当前被挤出去看不到的 tab,不重复列出主条上已可见的 tab。
4. 从下拉里点选一个隐藏 tab 后:该 tab 变为选中态,且主 tab 条的可见窗口
   自动滚动把它带入视野并高亮,下拉自动收起。这一并修掉"选中不自动滚动"的
   既有缺口。
5. 下拉里每一项都能直接点 x 关闭对应 tab,行为与主条上关闭一致。
6. 点击下拉外部区域收起下拉。
7. 抽取一套可被 5 处调用点共享的基础设施(溢出计算 + V 按钮 + 下拉浮层),
   不是各写各的。

**非目标**:

- `extensions/browser.rs` 的多标签页目前完全没有溢出处理(裁剪即隐藏,连
  滚动能力都没有)。这次不顺带补,因为那是"新增能力"而不是"替换现有
  `<`/`>`",范围明显不同,另开事项处理。
- 不新增键盘方向键跨 tab 切换、Esc 关闭下拉等交互;维持现状"点击外部关闭"
  这一种收起下拉的方式。
- 不改变 tab 本身的拖拽排序、右键菜单等其它既有行为。
- SSH/Database 各自的"空白占位 tab"(`SelectBlankTab`)不获得新能力,只要求
  它若被挤进隐藏列表时,下拉行不出现关闭按钮(它本来就不可关闭)。

## 关键语义确认(brainstorming 会话定案)

以下几点在探索后以多选题形式向用户确认,均选择推荐项:

1. **范围**:5 处全部替换,不是只做文件/项目预览两处,保持体验一致。
2. **下拉内容**:只列隐藏的 tab,不列全部 tab 再灰显——match 截图展示的效果。
3. **选中后是否自动滚动**:自动滚动带入可见区,顺带修掉"选中不触发滚动"的
   既有缺口(该缺口现状不会被触发,是因为现在选中的唯一入口就是点可见 tab
   本身;这次新增"下拉选中隐藏 tab"入口后必须处理,否则会退化成"选中了但
   看不到高亮在哪")。
4. **V 按钮显隐**:仅溢出时渲染,不做常驻禁用态。

## 架构

### 共享基础设施(`crates/dozer-app/src/tab_widget.rs`)

**1. 改造 `tab_window`**

返回值从 `(first, can_left, can_right)` 换成一个 `TabOverflow` 结构体:

```rust
pub struct TabOverflow {
    pub first: usize,       // 钳制后的窗口起点(实际渲染从这里开始)
    pub visible_end: usize, // 窗口内最后一个可见 tab 的下一个索引(独占)
}

impl TabOverflow {
    pub fn hidden_before(&self) -> Range<usize> { 0..self.first }
    pub fn hidden_after(&self, len: usize) -> Range<usize> { self.visible_end..len }
    pub fn has_overflow(&self, len: usize) -> bool {
        self.first > 0 || self.visible_end < len
    }
}

pub fn tab_window(widths: &[f32], gap: f32, avail: f32, requested_first: usize) -> TabOverflow
```

底层仍是同一套"按宽度累加钳制"算法(`workspace.rs` 里已有单测覆盖),只是不再
需要方向可用性标记(`can_left`/`can_right`),换成暴露"隐藏区间"给 V 按钮和
下拉列表用。"自动滚动带入可见区"直接复用这个函数:把候选 `first` 设成"被
选中 tab 的原始 index",交给它重新钳制,不需要新算法。

**2. 抽取 `tab_label`**

```rust
pub fn tab_label<'a, M: 'a>(icon: Option<Element<'a, M>>, title: &'a str) -> Element<'a, M>
```

`panel_tab` 里"图标+文字"目前是内联局部变量(`title_row`),没法单独复用。这次
拆成独立函数,横向 tab 与下拉列表行共用同一份渲染逻辑,保证视觉一致。

**3. 新增 `tab_overflow_button`**

```rust
pub fn tab_overflow_button<M: Clone + 'static>(hidden_count: usize, on_press: M) -> Option<Element<'static, M>>
```

`hidden_count == 0` 时返回 `None`,调用方直接跳过渲染这一项。否则用
`icons::icon_button_entry` 包一个 `IconKind::ChevronDown`,尺寸复用现有
`geometry::tab_arrow_button_size()`(18px 热区)+ `icon_size::chevron()`
(12px 图标),与现有下拉 chevron 视觉一致。

**4. 新增 `TabOverflowMenuArgs` + `tab_overflow_menu`**

仿 `tabs::TabCoreArgs` 的具名字段参数结构体风格(闭包走结构体自身泛型参数,
不用 `Box<dyn Fn>`):

```rust
pub struct TabOverflowMenuArgs<'a, M, FSel, FClose>
where
    FSel: Fn(usize) -> M,
    FClose: Fn(usize) -> M,
{
    pub entries: &'a [TabOverflowEntry<'a>], // (原始 index, 图标, 标题, 是否可关闭)
    pub anchor: (f32, f32),                  // 屏幕坐标锚点
    pub on_select: FSel,
    pub on_close: FClose,
    pub on_dismiss: M,
}

pub struct TabOverflowEntry<'a> {
    pub index: usize,
    pub icon: Option<icons::IconKind>, // 交给 tab_label 内部现构建,不预先建 Element
    pub title: &'a str,
    pub closable: bool,
}

pub fn tab_overflow_menu<'a, M, FSel, FClose>(args: TabOverflowMenuArgs<'a, M, FSel, FClose>) -> Element<'a, M>
where
    M: Clone + 'a,
    FSel: Fn(usize) -> M + 'a,
    FClose: Fn(usize) -> M + 'a,
```

内部实现:

- 每一行用 `tabs::tab_core` 包装(选中 + hover 露出关闭按钮),与横向 tab
  交互完全一致;`closable == false` 的行(空白占位 tab)通过 `tab_core` 的
  `close_interactive: false` 关掉关闭按钮。
- 纵向 `column` 装进一个限高的 `scrollable`,避免隐藏 tab 特别多时把菜单
  撑出屏幕。
- 悬浮定位复用 `PreviewTabMenu` 已验证的套路:绝对屏幕坐标换算成
  `padding` + 全屏透明 `MouseArea`(`on_press` 发 `on_dismiss`)+
  `stack!` 叠层。坐标来源不是新机制,而是复用 `App::last_cursor`(全局
  持续追踪的最近光标位置,`main.rs:1011-1019` 每次 `CursorMoved` 刷新)——
  这正是 `project_add_menu_anchor`、`todo` 面板日历/派发/状态锚点等已有
  多处左键弹层共用的"按下瞬间拍快照"套路,直接照抄,不新增追踪逻辑。

### 5 处调用点改造

状态字段模式一致:`xxx_tab_first` 保留(语义从"手动步进的滚动位置"变成
"自动维护的可见窗口起点"),新增 `xxx_tab_overflow_anchor: Option<(f32, f32)>`
(`None` = 下拉关闭,`Some` = 打开且记录了锚点坐标)。

Message 改动模式一致:删除 `XxxTabScroll(bool)`(不再需要手动翻页),新增
`XxxTabOverflowToggle`(点 V:未打开则从 `last_cursor` 打开,已打开则关闭)和
`XxxTabOverflowDismiss`(点外部关闭)。对应的"选中"消息 handler 里追加两行:
把 `xxx_tab_first` 设成被选中 tab 的原始 index(交给 `tab_window` 下一帧重新
钳制,实现自动滚动),并把 anchor 置 `None`(选中后下拉自动收起)。

| # | 状态字段 | Message 位置 | 涉及文件 |
|---|---|---|---|
| 终端 tab | `term_tab_first` | 顶层 `Message` | `terminal.rs`(渲染)+ `app.rs`(match 分支) |
| 文件预览 tab | `preview_tab_first` | 顶层 `Message` | `workspace.rs`(渲染)+ `app.rs`(match 分支) |
| 项目预览 tab | `project_preview_tab_first` | 顶层 `Message` | 同上,同一个 `preview_pane_for` |
| SSH tab | `ssh_tab_first` | 包裹于 `ssh::Message` | `ssh` 模块自己的 update + `app.rs::ssh_tab_bar` |
| Database tab | `DatabaseContentState.tab_scroll_first` | 包裹于 `database::Message` | `extensions/database.rs` 自己的 update + 渲染 |

V 按钮插入位置:严格替换现有 `<`/`>` 箭头对所在的位置,前后其它 trailing
图标(如"…"更多菜单、"+"新建 tab 按钮)相对顺序不变。

## 错误处理

- 下拉打开期间如果因为关闭了其它 tab 或窗口 resize 导致 `has_overflow`
  变为 `false`:渲染层只在 `has_overflow(len)` 为真时才画悬浮菜单,即使
  `xxx_tab_overflow_anchor` 还是 `Some`,避免出现"V 已经消失但菜单还飘在
  那"的游离状态。视觉上表现为 tab 变少后菜单和 V 一起自然收起。
- 关闭 tab 导致隐藏列表变化(可能从"还有隐藏"变成"没有隐藏了")在下一帧
  的 `tab_window` 重新计算里自然反映,不需要额外的失效处理。

## 测试策略

- 纯逻辑单测:更新/新增 `TabOverflow` 相关测试(钳制结果、
  `hidden_before`/`hidden_after`/`has_overflow` 的边界值、"候选 first 设为
  某个越界 index 后重新钳制,产生的窗口包含该 index"这一自动滚动场景),
  沿用 `workspace.rs` 里现有 `tab_window_*` 测试所在的模块位置。
- 手动验证:`cargo run -p dozer-app`,对 5 个面板分别撑出"tab 数超过可视
  宽度"的场景,逐一验证:V 出现时机、下拉内容是否只含隐藏 tab、点选自动
  滚入并高亮、点 x 关闭、点外部关闭、tab 变少后 V 随之消失、SSH/Database
  的空白占位 tab 若被挤入隐藏列表时下拉行没有关闭按钮。
- 不新增 App fixture 级别的自动化 GUI 测试——这在本仓库此前几期(如原生
  输入框 Stage5 的 commit-on-blur 边缘逻辑)是已知可接受的空缺,这次保持
  一致,不为此单独造测试基础设施。

## 排期备注

- 独立 worktree 分支开发,完工经代码审阅通过后再合并 main。
- 建议任务拆分:共享基础设施(`tab_widget.rs` 改造,含 `tab_label` 抽取和
  `tab_window` 返回值改造)打底一个 Task;之后 5 个调用点各开一个 Task,
  互不耦合可并行;最后一个 Task 做人工 GUI 验收清单(逐条对照上面"测试
  策略"里手动验证的 6 项)。
