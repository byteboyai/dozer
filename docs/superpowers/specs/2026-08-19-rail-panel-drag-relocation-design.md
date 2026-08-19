# workspace 图标栏面板拖拽换栏(左右互移)

**状态:已批准(brainstorming 会话,2026-08-19)**

## 背景

workspace(项目工作区,区别于首页 `homespace`)左右各有一条图标栏,分别
固定驱动 `LeftView`(`Files`/`GitLog`/`Todo`/`Project`/`Database`/`Ssh`/
`Web`,7 个)和 `RightView`(`Agent`/`Conversations`/`Usage`/`Acceptance`,
4 个)——这不只是"图标摆在哪条栏"的问题:`LeftView`/`RightView` 是两个
**独立的 Rust 枚举类型**,`ShellState.left_view: LeftView`/
`right_view: RightView` 是两个不同类型的字段,`app.rs` 里所有面板几何
(webview bounds)、视图渲染分发、hover 追踪(`RailButton` 11 个固定
variant)都按这两个类型分别 `match`,互不相通。

用户要求:图标栏上的按钮可以从左栏拖到右栏、也可以从右栏拖到左栏;移动
后面板跟着开在按钮所在的那一侧;移动后如果面板内部本来是"列表在左、
内容在右"这种左右两栏布局(典型例子:文件面板——项目树在左、文件预览
在右),换到另一侧栏后这个内部左右顺序也要跟着镜像(文件预览在左、
项目树在右)。

摸底发现 11 个面板里,3 个(`Files`/`Project`/`Web`)挂了原生 `wry`
webview,webview 的像素级摆位(`x, y, w, h`)是按"以左图标栏宽度为基准
向右偏移"精确算出来的(`preview_content_bounds`/`browser_content_bounds`
等函数),换到右栏需要一份反向(以右边缘为基准向左偏移)的镜像几何,不能
简单参数化复用——这是这次改动风险最集中的地方。其余 8 个面板纯 iced
渲染,没有这层顾虑。

**范围只覆盖 workspace(项目工作区)的左右图标栏。** 首页
(`homespace::HomeLeftView`/`HomeRightView`)是完全独立的另一套面板/
图标系统,不在这次范围内。

## 目标 / 非目标

**目标**:

1. `LeftView`(7)+ `RightView`(4)合并成一个统一的 `PanelKind` 枚举
   (11 个 variant),`ShellState` 改成 `left_active: PanelKind`/
   `right_active: PanelKind`(依然各自非 `Option`——见下方"栏不可清空"
   规则)。
2. 新增 `RailLayout { left: Vec<PanelKind>, right: Vec<PanelKind> }`,
   持久化进 `layout.json`(现有 `ShellLayout` 结构体新增字段,
   `#[serde(default)]` 走已有的老文件兼容套路,默认值 = 现状的
   7+4 固定分组)。这是"每个面板当前在哪条栏、栏内什么顺序"的唯一真相源。
3. 图标栏按钮从"11 个手写 `icon_button_entry` 调用堆成的静态
   `column!`"改成"按 `RailLayout.left`/`.right` 顺序遍历渲染"——
   一份渲染代码,两条栏共用,不再是 `left_icon_rail`/`right_icon_rail`
   两份几乎重复的函数体各写 7/4 次调用。
4. 拖拽:按住图标栏按钮拖动,可在**同栏内重新排序**(复用现有
   `TabDrag` 换位手法),也可以**拖过图标栏之间的分隔区**换到另一栏
   (新增能力,`TabDrag` 原本不支持跨组)。松手完成移动后:
   - 目标栏新增该面板(插到指针悬停的位置),源栏移除它。
   - 该面板成为目标栏的 `active`(自动展开,跟随"移动后在按钮所在一侧
     打开面板"的要求)。
   - 若该面板移动前是源栏的 `active`,源栏的 `active` 退回栏内相邻的
     下一个面板(不能是空——见下方规则)。
   - 移动结果落盘(`layout.json`)。
5. **内部左右镜像规则**(适用于 8 个有"横向两栏"内部布局的面板——见下方
   完整清单与豁免清单):**规则是"面板当前不在默认栏时,把该面板现有
   `row!` 的两个子元素渲染顺序整体反过来",不依赖任何"哪个子元素是
   语义上的列表/内容"这类命名假设**——摸底 Stage 3 时发现 8 个面板里
   6 个(`Files`/`GitLog`/`Todo`/`Project`/`Ssh`/`Web`)默认是"列表在前
   (渲染在左)、内容在后(渲染在右)",但 `Agent`/`Conversations` 两个
   默认恰好反过来("内容在前/渲染在左、列表在后/渲染在右"——原有代码
   注释明确写"两个配对都是内容侧渲染在左、列表侧渲染在右"),不是笔误,
   是这两个面板原有设计就选了和其余 6 个不同的默认顺序。所以镜像规则
   统一表述成"反转当前 `row!` 顺序",不表述成"列表永远排在语义左侧"
   ——后者对 `Agent`/`Conversations` 是错的。`PanelDims` 里对应的 split
   比例字段(如 `files_split`)语义不变,只是画的时候两个子元素谁先谁后
   会跟着 `row!` 顺序一起换,调用方要把 portion 值和它现在对应的那个
   子元素配对传对,不能笔误配反。
6. 3 个 webview 面板(`Files`/`Project`/`Web`)新增镜像版 webview
   bounds 函数,在该面板当前 docked 在右栏时启用,保证 webview 像素位置
   跟纯 iced 部分的镜像布局对齐。
7. 迁移完成后 `dozer-app` 编译、测试、clippy、fmt 全绿;GUI 上验证:
   任意面板可以两个方向拖动换栏、换栏后自动展开在新一侧、8 个有内部
   两栏布局的面板换栏后内部顺序正确镜像、3 个 webview 面板换到右栏后
   webview 像素位置与纯 iced 布局吻合、重启后换栏结果保留。

**非目标**:

- **不改动首页(`homespace`)的图标栏/面板系统**——完全独立的另一套
  `HomeLeftView`/`HomeRightView`,不共享这次的 `PanelKind`/`RailLayout`。
- **不支持"栏清空"**:任一栏移到只剩 0 个面板时,阻止这次拖放(视觉上
  ghost 元素弹回原位,不产生任何状态变化)——避免设计"空栏渲染成什么
  样"这一整套边界情况。11 个面板、每栏至少留 1 个,栏内面板数量上限
  不设(两条栏现状分别是 7/4,不太可能出现"全塞一栏"的极端场景,真出现
  也只是图标栏变长,不是这次要处理的问题)。
- **不改变 8 个面板各自的具体渲染代码逻辑**(表格怎么画、按钮怎么排),
  只改变"语义左项/语义右项先渲染谁"这一层顺序决策,以及外层
  `left_zone`/`right_zone` 容器切换。
- **不改变 `PanelDims` 十个 split 字段的语义或默认值**——`files_split`
  换栏前后都是"语义左项(项目树)占比",不因为换栏重新定义成"当前视觉
  左侧占比"。
- **不新增"栏内可以有 0 个 active 面板"的状态**——`left_active`/
  `right_active` 保持非 `Option`,和现状一致。

## 架构与数据流

### 面板清单与内部布局分类

| `PanelKind` | 默认栏 | 内部两栏(横向)? | 默认渲染顺序(第一项/第二项) | webview? |
|---|---|---|---|---|
| `Files` | 左 | 是(`files_split`) | 项目树 / 文件预览 | **是** |
| `GitLog` | 左 | 是(`git_log_split`,外层)| commit 列表 / [文件列表+diff] | 否 |
| `Todo` | 左 | 是(`todo_split`) | 分类导航 / 列表&内容 | 否 |
| `Project` | 左 | 是(`project_split`) | 项目信息 / 项目预览 | **是** |
| `Database` | 左 | 否(单栏) | — | 否 |
| `Ssh` | 左 | 是(`ssh_split`) | 主机列表 / 内嵌终端 | 否 |
| `Web` | 左 | 是(`browser_bookmarks_split`,仅 `bookmarks_open` 时才有两栏,收起时单栏、镜像是 no-op) | 网页内容 / 收藏夹侧栏 | **是** |
| `Agent` | 右 | 是(`agent_split`) | **终端 / Agent 列表**(注意:内容在前,和上面 6 个"列表在前"相反,原有设计如此) | 否 |
| `Conversations` | 右 | 是(`conversations_split`) | **对话审阅 / 对话列表**(同上,内容在前) | 否 |
| `Usage` | 右 | 否(单栏) | — | 否 |
| `Acceptance` | 右 | 否(单栏) | — | 否 |

`GitLog` 的 `git_log_file_diff_split` 是右侧区域内部**纵向**(上下)分割
(文件列表在上、diff 在下),没有左右方向的镜像意义,换栏时不受影响,
只有外层 commit 列表 / [文件列表+diff] 这一层横向切分会镜像。

`Web` 的字段名和语义方向刻意反着命名(`browser_bookmarks_split` 存的是
"网页内容占比",不是"收藏夹侧栏占比")——按本设计"语义左项占比不变"的
规则,`Web` 的语义左项 = 网页内容,语义右项 = 收藏夹侧栏,镜像时网页
内容渲染到镜像后的语义左侧,收藏夹侧栏渲染到语义右侧,和其余 7 个字段
在"字段存的是语义左项占比"这一点上是一致的,不需要为 `Web` 单独开例外
逻辑。

`Database`/`Usage`/`Acceptance` 三个单栏面板换栏是纯粹的"整体从左边挪到
右边渲染",没有内部顺序要处理。

### `PanelKind` 与 `RailLayout`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelKind {
    Files, GitLog, Todo, Project, Database, Ssh, Web,
    Agent, Conversations, Usage, Acceptance,
}

impl PanelKind {
    /// 默认所在栏——`RailLayout::default()` 与"镜像规则的基准方向"
    /// 都从这里读,只维护一处。
    pub fn default_side(self) -> Side {
        match self {
            Self::Files | Self::GitLog | Self::Todo | Self::Project
            | Self::Database | Self::Ssh | Self::Web => Side::Left,
            Self::Agent | Self::Conversations | Self::Usage | Self::Acceptance => Side::Right,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side { Left, Right }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RailLayout {
    pub left: Vec<PanelKind>,
    pub right: Vec<PanelKind>,
}

impl Default for RailLayout {
    fn default() -> Self {
        Self {
            left: vec![PanelKind::Project, PanelKind::Todo, PanelKind::Files,
                       PanelKind::GitLog, PanelKind::Database, PanelKind::Ssh, PanelKind::Web],
            right: vec![PanelKind::Agent, PanelKind::Conversations,
                        PanelKind::Usage, PanelKind::Acceptance],
        }
    }
}
```

配一对 `fn side(&self, side: Side) -> &Vec<PanelKind>`/
`fn side_mut(&mut self, side: Side) -> &mut Vec<PanelKind>` 访问器,按
`Side` 取对应字段——拖拽逻辑、渲染入口都通过这两个访问器操作,不直接读
`.left`/`.right` 字段,避免"改了 `Side` 匹配却忘了同步改字段名"这类
两处硬编码不同步的错误。

`left`/`right` 顺序即渲染顺序(现有 `left_icon_rail` 里项目/待办/文件/
Git/数据库/SSH/浏览器这个顺序原样保留成默认值,不因为这次重构顺手调整)。

`RailLayout` 挂进现有 `ShellLayout`(`#[serde(default)]` 字段,老
`layout.json` 缺这个字段时退化成上面的默认值,和 `window_width` 等字段
现有的兼容套路一致)。**消毒规则**:`load_from` 里追加校验——若
`left`/`right` 任一为空,或两边合起来不是恰好 11 个不重复的
`PanelKind`(说明手改/版本不一致导致的坏数据),整个 `RailLayout` 回落
`default()`,不做"部分修复"(比如缺一个面板就补在默认栏),避免中间态
比"直接用默认值"更难排查。

### `RailButton`/`HoverId` 化简

现有 `RailButton` 11 个固定 variant(`LeftFiles`/`RightAgent`/…)按面板
所在栏区分,这次统一成:

```rust
pub enum RailButton {
    Panel(PanelKind),
    // HomeProjectList/HomeRecents/HomeBrowser 三个首页专属 variant 不动
    HomeProjectList,
    HomeRecents,
    HomeBrowser,
}
```

面板只会同时存在于一条栏,hover 态不需要按"当前在哪条栏"再区分一次;
`HoverId::Rail(RailButton::Panel(kind))` 换栏前后是同一个 key,过渡期间
hover 动画状态不会因为换栏突然重置。

### 图标栏渲染:静态 `column!` → 按 `RailLayout` 遍历

`left_icon_rail`/`right_icon_rail` 现状是 7/4 次几乎重复的
`icons::icon_button_entry(...)` 调用手写在 `column!` 里。改成:

```rust
fn icon_rail<'a>(
    app: &'a App,
    side: Side,
    panels: &'a [PanelKind],
    active: PanelKind,
    collapsed: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = match side {
        Side::Left => theme::region::left_icon_rail(),
        Side::Right => theme::region::right_icon_rail(),
    };
    let open = !collapsed;
    let mut content = column![].spacing(region.gap).padding(region.padding);
    for (idx, &kind) in panels.iter().enumerate() {
        content = content.push(rail_button_entry(app, side, kind, idx, kind == active && open));
    }
    container(content)
        .width(Length::Fixed(byteui::theme::geometry::icon_rail_width()))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}
```

`rail_button_entry` 是新增的每按钮渲染函数,取代原来 11 处内联调用:
按 `kind` 查表取 `IconKind`/label 文案(原来分散在 11 处调用里的字面量,
收进一个 `fn panel_icon(kind: PanelKind) -> (icons::IconKind, &'static str)`
`match`),`icon_button_entry` 的 `on_press` 消息统一成
`Message::PanelSelect(kind)`(取代 `LeftIconSelect`/`RightIconSelect`
两个消息——选中一个面板不再需要调用方指定它在哪一侧,`update` 里从
`RailLayout` 反查该面板当前在哪条栏)。按钮外层额外包一层拖拽感应
(见下一节),取代原来纯 `icon_button_entry` 返回值直接进 `column!`。

### 拖拽:同栏重排 + 跨栏移动

复用 `TabDrag` 的成熟手法(按下开始追踪、`MouseArea::on_move` 持续上报
悬停位置、`winit` 原生 `MouseInput{Released}` 收尾),新增一个专属的
拖拽状态和消息组,不复用 `TabDrag`/`TabGroup`(`TabGroup` 现有 variant
`Project`/`ProjectPreview`/`Browser` 语义是"页签换位",图标栏换栏除了
重排还要跨栏移动,合并进同一个类型会让 `tab_drag_move` 那种"跨组
no-op"的校验逻辑变复杂,新开一个更清楚):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailDrag {
    pub source_side: Side,
    pub source_index: usize,
}
```

`App` 新增 `rail_drag: Option<RailDrag>` 字段(同 `tab_drag` 现状平级)。

新增消息:

```rust
RailDragMove { side: Side, index: usize },
RailDragEnd,
```

每个 rail 按钮的 `MouseArea::on_move` 上报 `Message::RailDragMove { side, index }`
(`side`/`index` 是**当前悬停到的**按钮所在的栏与栏内位置,不是拖拽源)。
`main.rs` 现有 `MouseInput{Released}` 分支(`app.dragging_tab().is_some()`
那条)追加一个 `app.rail_drag.is_some()` 判断,同样发 `RailDragEnd`。

```rust
pub struct RailDrag {
    pub source_side: Side,
    pub source_index: usize,
    /// 悬停到另一栏时记录目标位置;`RailDragEnd` 才真正提交搬移。
    /// 悬停回源栏(或还没悬停到任何另一栏位置)时是 `None`。
    pub pending_cross_side: Option<(Side, usize)>,
}

fn rail_drag_move(&mut self, side: Side, to: usize) {
    let Some(drag) = self.rail_drag else { return };
    if drag.source_side == side {
        // 同栏重排:等价于 tab_drag_move 的换位逻辑。
        let panels = self.rail_layout.side_mut(side);
        let from = drag.source_index;
        if from == to || from >= panels.len() || to >= panels.len() {
            return;
        }
        let kind = panels.remove(from);
        panels.insert(to, kind);
        self.rail_drag = Some(RailDrag {
            source_side: side,
            source_index: to,
            pending_cross_side: None,
        });
    } else {
        // 跨栏:只更新悬停目标,不搬移、不落盘,交给 RailDragEnd 统一处理。
        if let Some(drag) = &mut self.rail_drag {
            drag.pending_cross_side = Some((side, to));
        }
    }
}
```

`RailDragEnd` 时:若 `pending_cross_side` 有值,执行真正的跨栏移动
(源栏 `remove`,目标栏按 `pending_cross_side` 的位置 `insert`),
被移动面板成为目标栏新 `active`;若移动前是源栏 `active`,源栏 `active`
退回源栏(移除后)索引 `min(原索引, 源栏新长度-1)` 处的面板(等价于
"退到相邻的下一个,数组末尾就退到新的最后一个")。同栏重排结束时不改
`active`(拖的即便不是当前 `active` 面板,重排也不该意外切换选中态)。
两种情况结束后都调一次 `layout::save`(`RailLayout` 变了才需要存盘,
純同栏重排也算"顺序变了"要存)。

**跨栏空栏保护**:`RailDragEnd` 提交前检查"源栏移除这个面板后是否还剩
至少 1 个",不满足就整个操作 no-op(状态不变,`rail_drag` 清空,不落盘)
——对应"不支持栏清空"这条非目标。

### 面板选中(`Message::PanelSelect`)与栏收起

`Message::PanelSelect(kind)` 取代 `LeftIconSelect`/`RightIconSelect`:
处理时先查 `kind` 当前在 `rail_layout.left` 还是 `.right`(两者其一,
`RailLayout` 消毒规则保证不重不漏),再按现有 `LeftIconSelect`/
`RightIconSelect` 各自的"已选中再点则收起栏"逻辑处理对应侧的
`*_active`/`*_collapsed`。

### webview 面板的镜像 bounds

`Files`/`Project`/`Web` 三个面板各自新增一个 `*_bounds_right` 版本几何
函数,与现有 `*_bounds`(隐式只服务左栏)并列,内部把"以
`icon_rail_width()` 为左基准向右偏移"换成"以
`window_width - icon_rail_width()` 为右基准向左偏移",宽度计算方向
相应镜像(原来 `content_w` 从左边界量,镜像版从右边界量)。调用处
(`preview_content_bounds` 等函数)在入口按"该面板当前 docked 在哪栏"
分派到两个版本之一,不改变各自内部对`pair_list_content_width`等既有
辅助函数的调用方式(那些函数只关心"这一片矩形多宽",不关心矩形挂在
窗口哪一侧,可以直接复用)。

## 错误处理

`RailLayout` 反序列化失败或消毒不通过(非 11 个不重复面板/任一栏为空)
直接回落 `default()`,不 panic——这是运行时持久化数据(用户拖拽产生的
偏好),不是开发期配置,策略与现有 `sanitize_shell_layout` 一致(同一
`load_from` 函数里追加,不是新建一条独立错误处理路径)。

拖拽过程中的中间状态(`rail_drag`)不落盘,只有 `RailDragEnd` 成功提交
后才存一次——中途任何 panic/崩溃不会留下"半栏搬完"的坏数据。

## 测试策略

1. `PanelKind::default_side()`/`RailLayout::default()` 的防漂移锚测试:
   11 个面板不重不漏分到左右两栏,与现状 7/4 分组逐一对应。
2. `RailLayout` 序列化 round-trip + 消毒:合法数据原样读回;任一栏
   为空、面板重复、面板数不是 11、字段整体缺失(老 `layout.json`)
   四种坏数据分别回落 `default()`。
3. `rail_drag_move`/`RailDragEnd` 逻辑测试(不依赖真实鼠标事件,直接
   调 `App` 方法,同现有 `tab_drag_move` 测试手法):
   - 同栏重排:换位后顺序正确,`active` 不变。
   - 跨栏移动非 active 面板:目标栏插入位置正确,源栏移除,目标栏
     `active` 变成该面板,源栏 `active` 不变。
   - 跨栏移动当前 active 面板:源栏 `active` 退到相邻面板(含"移动的是
     源栏最后一个可选面板之外的某个"和"移动后源栏只剩一个,退到那个"
     两种边界)。
   - 跨栏移动会导致源栏清空:no-op,拖拽状态清空但 `RailLayout` 不变。
4. 8 个有内部横向两栏布局的面板,各自补一个"镜像后语义左右项渲染顺序
   互换,split 比例数值不变"的单测(结构化断言,不追求像素级 UI 快照)。
5. `Files`/`Project`/`Web` 三个 webview 面板的 `*_bounds_right` 与
   `*_bounds`(现有)在"镜像输入"下应该产出镜像输出的性质测试(例如
   固定窗口宽度下,左栏版本的 `x` 与右栏版本的 `x + w` 应该关于窗口
   中轴对称——不追求逐像素相等,只验证镜像关系成立)。
6. `cargo build -p dozer-app --bin dozer`、`cargo test -p dozer-app
   --bin dozer`、`cargo clippy --all-targets -- -D warnings`、
   `cargo fmt -- --check`。
7. 独立命名的临时二进制做 GUI 验证(这次风险最集中的地方,单测覆盖不到
   真实拖拽手感和 webview 像素位置):
   - 从左栏拖一个纯 iced 面板(如 Todo)到右栏,松手后面板自动在右栏
     展开,左栏收起态正确退回相邻面板。
   - 从右栏拖回左栏,验证反向同样正确。
   - 拖 `Files`(webview 面板)到右栏,验证项目树/文件预览镜像顺序正确、
     文件预览 webview 像素位置与镜像后的 iced 布局对齐(不能出现 webview
     叠在项目树上或悬空的情况)。
   - 拖 `Web`(浏览器)到右栏,验证网页 webview 位置正确、收藏夹侧栏
     跟着镜像。
   - 同栏内拖拽重排两个图标顺序,验证顺序生效且不影响 `active`。
   - 尝试把某一栏拖到只剩 0 个(比如右栏 4 个依次全拖到左栏),验证
     最后一次拖动被正确挡住(右栏保留最后 1 个,不清空)。
   - 退出重开,验证栏顺序/换栏结果、`active`/`collapsed` 状态跨重启
     保留。
   完成后关闭该临时实例、删除临时二进制,不留后台进程。

## 排期备注

这是这次会话里最大的一次改动,预计实现计划会落在 40-60+ Task 量级
(参照:`app.rs` 单文件 9400+ 行,`LeftView`/`RightView` 相关的
`match` 分支摸底就有 30+ 处,每处都要按"该面板可能出现在任一栏"重新
过一遍)。建议实现阶段用 `subagent-driven-development`(独立 Task 逐个
派发、两阶段审阅),不要一次性大改——参考这次会话里 `byteui` 迁移系列
的手法:先把"数据模型 + 持久化 + 消毒"这一层单独落地并测试(`PanelKind`/
`RailLayout`/`ShellState` 字段改名),再落"图标栏渲染改遍历"(风险中等,
只影响图标栏那一小块 UI),再落"8 个面板的镜像渲染顺序"(可以按面板
逐个 Task,互相独立),最后落"拖拽交互 + 3 个 webview 面板的镜像
bounds"(风险最集中,建议留最后、留最多验证时间)。

`ShellState`/`RailButton`/`Message::LeftIconSelect`/`RightIconSelect`
改名涉及的调用点数量目前未逐一摸底(不像 `byteui` 系列迁移那样能给出
精确调用点计数)——写实现计划前建议先起一个"摸底 Task"专门跑
`grep -c` 把 `LeftView::`/`RightView::`/`LeftIconSelect`/`RightIconSelect`/
`RailButton::Left*`/`RailButton::Right*` 在 `app.rs`(以及是否溢出到
`workspace.rs`/`extensions/*.rs`)的精确出现次数摸清楚,再据此拆分
实现计划的 Task 粒度,不要凭这份 spec 里的估算数字直接定 Task 数量。
