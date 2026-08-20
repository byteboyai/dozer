# 输入框改用 iced 原生 text_input/text_editor

**状态:已批准(brainstorming 会话,2026-08-20)**

## 背景

用户实测反馈:全 app 的输入框普遍缺失最基础的文本编辑体验——不显示真
光标、方向键/鼠标/拖拽/shift+方向键都不能定位或选中文字、文本框不随内容
自增高、输入法候选框不跟光标走、拼词过程中的字符也不在光标处显示。

排查确认根因不是某处遗漏,是架构性的:`main.rs` 的键盘路由只认三种焦点
目标(`FocusIntent::Terminal`/`Preview(PanelKind)`/`Browser`——PTY 终端或
某个 wry 预览/浏览器 webview),**没有"某个 iced 原生控件正拿着焦点"这第
四种情况**。所以除了 SSH/Database 连接表单(2026-08-20 kooky-review-
followups 那次改动改用了真正的 `iced_widget::text_input`,但尚未过人工
GUI 走查)之外,全 app 其余输入框都刻意避开 iced 原生 `text_input`——代码
里的原话解释是"原生输入没法让 main.rs 知道它挂着焦点,打字会同步漏进已
聚焦的终端",于是全改成手写渲染,键盘走 `main.rs` 自己的拦截层路由成各
面板私有的 `AddrEvent`/`SearchEvent`/`AddEvent` 等消息。

进一步查了 iced 0.14 源码(`iced_widget::text_input`/`text_editor`),发现
这不是"需要自己补的空白"——iced 内部已经完整实现了真光标渲染、鼠标定位/
拖拽选中/shift+方向键选区、以及 `Ime::Preedit` 拼词过程展示 + 通过
`shell.request_input_method(...)` 自动把 OS 候选框定位到光标处。真正
要解决的只是 `main.rs` 那层"键盘该转发给终端还是该放行给 iced 正常处理"
的路由判断——而且这条判断链已经是一长串"某某弹层打开就提前 return,不
走终端转发"的既有写法(见 `main.rs` "浏览器地址栏/验收意见/项目树行内
编辑态/..." 那段,`to_self_drawn_input` 及其后的 `addr_message` 分派),
架构上能再插一条同款判断。

## 目标 / 非目标

**目标**:

1. 以下 4 个输入框从手写渲染改为 iced 原生控件:
   - Todo 添加框(`extensions::todo`,`AddEvent`/`AddCursorMove`)——多行
     自增高,换 `iced_widget::text_editor`。
   - 文件树搜索框(`extensions::files`,`SearchEvent`,`app.search_editing()`)
     ——单行,换 `iced_widget::text_input`。当前完全没有光标下标概念
     (纯末尾追加/删除,连方向键都不支持),是本次改动量最大的一个。
   - 浏览器地址栏(`extensions::browser`,`AddrEvent`,
     `app.browser_addr_editing()`)——单行,同上,也没有光标概念。
   - 首页项目搜索框(`HomeProjectSearchEvent`,
     `app.home_project_search_editing()`)——单行,当前经
     `crate::search_box::view` 渲染,已有光标下标+方向键。
2. `byteui` 新增/扩展薄层组件供以上 4 处及未来其它输入框复用(不新增
   业务逻辑,只出组件):
   - 扩展 `byteui::form::input_text`,加一个可选 `id` 用于外部驱动
     `text_input::focus(id)`。
   - 新增 `byteui::form` 下的多行自增高组件(包 `iced_widget::text_editor`),
     样式对齐 `input_text`(卡片底色+描边,聚焦金框由 iced 内置
     `Status::Focused` 驱动,不需要应用层自己维护"是否聚焦"布尔量画边框)。
3. `main.rs` 键盘拦截链新增一条"原生输入框编辑态"放行判断,命中时让事件
   正常流入 iced 的 `update()`(不转发给终端/webview),不破坏既有的
   `to_self_drawn_input` 分支优先级与 Esc 提前 return 那批判断。
4. **SSH/Database 连接表单纳入本次路由修复范围**(不需要换控件,已经是
   真 `text_input`):用"表单当前是否打开"这个更粗粒度的信号(两侧状态
   都已有对称的 `pub fn editing(&self) -> Option<&XxxDraft>` 访问器——
   `ssh.rs:119`/`database.rs:556`——`.is_some()` 即可)接入同一条放行
   判断,并补完 kooky-review-followups 那次遗留的人工 GUI 走查
   (该走查此前专门验证的是控件外观,不包含本次要修的"终端焦点漏键"场景,
   需要重新走一遍)。
5. 清理随之产生的死代码(见"清理"一节)。
6. 迁移完成后 `dozer-app`/`byteui` 编译、测试、clippy、fmt 全绿;人工
   GUI 走查全部 6 个输入框(4 迁移 + 2 路由验证),逐条核对:真光标显示、
   方向键移动、鼠标点击定位、鼠标拖拽选中、shift+方向键选中、(Todo 添加
   框)随内容自增高、IME 候选框跟随光标+拼词过程可见,以及**关键回归项**
   ——旁边有终端 tab 处于焦点时,在这 6 个输入框任一个里打字/粘贴/用
   方向键,均不漏进终端。

**非目标**:

- **不覆盖本次发现的其它自绘输入面**:验收意见框(`acceptance.rs`,
  `AddrEvent`)、项目树行内编辑/项目名称编辑(`app.tree_editing()`/
  `app.project_name_editing()`)、右键"搜索"弹窗查询框
  (`app.search_popup_editing()`)、Todo 任务内容编辑与 Todo MARKDOWN
  整文件编辑(`app.todo_content_editing()`/`app.todo_markdown_editing()`,
  这两个和 Todo 添加框共用 `search_box.rs` 的光标数学原语,但渲染和消息
  都是独立的,不在本次改动范围)。这些留给后续 spec,处理方式预期是同一
  套路由放行机制 + 逐个换控件,不是要重新设计。
- **不改变 `AddrEvent` 类型本身**:`acceptance.rs`/`extensions::search`/
  `extensions::project` 三处非目标消费方继续用它,类型不删、不改字段。
- **不追求给 byteui 新组件设计成"能覆盖所有未来输入场景"的通用抽象**
  ——只解决当前 4+2 个具体调用点需要的能力(单行/多行、可选 focus id、
  现有配色体系),YAGNI。

## 架构与数据流

### 键盘路由:新增一条放行判断

`main.rs` 现有 `to_self_drawn_input`(11 个 `app.xxx_editing()` 判断
或运算,决定要不要把这次按键路由成某个面板的 `AddrEvent` 消息,见
`main.rs` "浏览器地址栏/验收意见/..." 注释段)保持不变,不动它覆盖的
6 个非目标输入面。

新增一个并列判断 `to_native_text_input`,由 6 个新查询方法(名字待实现
计划阶段定,下面是语义占位)或运算而成——注意这些**不是**复用现有的
`app.search_editing()`/`app.browser_addr_editing()`/
`app.home_project_search_editing()`/`app.todo_add_editing()`(那 4 个是
旧的"自绘输入编辑态"信号,随迁移一起废弃/改名,语义上是同一件事的新
实现,不是新增一条并存的判断):

```
to_native_text_input =
    <files 搜索框编辑态>       // 原 app.search_editing() 迁移后的等价物
    || <浏览器地址栏编辑态>    // 原 app.browser_addr_editing() 的等价物
    || <首页项目搜索框编辑态>  // 原 app.home_project_search_editing() 的等价物
    || <Todo 添加框编辑态>     // 原 app.todo_add_editing() 的等价物
    || <ssh 表单开着>          // 粗粒度:表单开着就算,不做到字段级
    || <database 表单开着>
```

命中时,当前这个 `WindowEvent::KeyboardInput`/`Ime`/鼠标事件**提前
return,但不吞掉、不转成自绘消息**——直接放行,让它继续走 iced 标准
事件转换管线(`iced_winit::conversion::window_event` → `program.update()`),
由聚焦的 `text_input`/`text_editor` 自己处理光标/选区/IME。这与现有
`FocusIntent::Preview` 分支"键盘焦点在预览列时同样放行"是同一手法
(`main.rs:1018` 附近),不是新发明的模式。

优先级上,`to_native_text_input` 与 `to_self_drawn_input` 理论不会同时
为真(两者互相排斥的编辑态,分属不同面板/时刻),但判断顺序上把
`to_native_text_input` 放在 `to_self_drawn_input` 检查**之前**,避免
未来某个自绘面板意外和某个原生输入面板同时报"编辑态为真"时,自绘分支
抢先把按键吞给了错误的消息目标。

### 焦点生命周期:应用层"进入编辑态"的信号不变,只是驱动的东西变了

4 个迁移目标现有"点进去才进编辑态"的消息路径(`SearchEditStart` 一类)
保留,不重新设计交互触发方式。区别是原来这个消息只翻转一个 `editing:
bool` 给渲染层拼假光标用,现在还要多返回一个 `iced_winit::runtime::Task`
——调 `text_input::focus(id)`(或 `text_editor::focus(id)`),把 iced
内部真正的控件焦点也一起打开。失焦沿用现有点击别处的 `blur_inputs`
类机制,不新增失焦触发路径。

`editing: bool` 这个应用层状态本身依然保留(用于渲染态如"是否显示占位
符"等语义决定),但**不再用来画边框金色高亮**——这部分交给 iced 内置的
`Status::Focused` 驱动 `byteui::form::input_text`/新多行组件自己的
`.style()` 闭包,不需要应用层同步一份"是否聚焦"信号进组件参数。

### 光标位置的 `cursor: usize` 状态怎么处理

Files 搜索框、浏览器地址栏当前完全没有光标下标(纯末尾追加/删除),
迁移后这个状态直接消失——不需要了,`text_input` 自己管理光标位置。

首页项目搜索框、Todo 添加框当前有 `cursor: usize`(经
`crate::search_box::{move_cursor_in, insert_at_cursor, ...}` 维护),
迁移后同样整个状态消失,改用 `iced_widget::text_input::Value`/
`text_editor::Content` 内部维护的真实光标——应用层不再需要
`AddCursorMove(CursorDir)` 这类消息,`Message::AddEvent(AddrEvent)`/
`HomeProjectSearchEvent(AddrEvent)` 这两个消息签名本身也可能因此简化
(具体是"整个消息类型换成 iced 的 `text_input::Action`"还是"保留精简后的
自定义事件包一层",留给实现计划阶段按 `on_input`/`on_action` 的实际签名
决定)。

## byteui 新组件

- **`byteui::form::input_text`**:现有 `view(placeholder, value, secure,
  on_input)` 签名不破坏性变更前提下,新增一个变体或可选参数暴露
  `id: text_input::Id`,供调用方在"进入编辑态"时把 `text_input::focus(id)`
  的 `Task` 传回 iced 运行时。SSH/Database 表单已经在用这个函数,不需要
  它们改调用点(除非要给它们的字段也做初始焦点管理,这属于路由验证的
  "锦上添花",非本次强制项)。
- **新增多行自增高组件**(暂定 `byteui::form::text_area`,具体命名/签名
  留给实现计划):包一层 `iced_widget::text_editor`,`Content` 自然撑高,
  样式对齐 `input_text`(卡片底色/描边/聚焦金框)。目前唯一调用方是 Todo
  添加框。

两个组件都只负责渲染与暴露 `Id`,"什么时候该聚焦/失焦"的判断权归应用层
(同 `PanelKind` 的"多消费方数据编排权收归内核"原则的镜像版本——这里是
"组件不替调用方决定交互时机")。

## 清理

- `files.rs`/`browser.rs` 迁移后不再消费 `AddrEvent`,但类型本身保留
  (`acceptance.rs`/`extensions::search`/`extensions::project` 三个非目标
  消费方还在用)。`files.rs::search_box_widget`(纯末尾追加渲染的旧实现)
  与 `files.rs::Message::SearchEvent`/主流程里对应的处理分支一并删除。
- `crate::search_box` 模块**不能整体删除**——虽然 `view()`/
  `SearchBoxColors` 在 Todo 添加框与首页项目搜索框都迁移后确实不再有
  调用方,可以删,但 `move_cursor_in`/`insert_at_cursor`/
  `delete_before_cursor`/`char_to_byte`/`CursorDir` 这几个光标数学自由
  函数**仍被 Todo 任务内容编辑(`todo_content_editing`)与 Todo MARKDOWN
  整文件编辑(`todo_markdown_editing`)依赖**(`todo.rs` 顶部 `use
  crate::search_box::{...}`,`pub use crate::search_box::CursorDir`)——这两
  个不在本次范围内,不能跟着削掉它们的依赖。清理范围精确到:删
  `search_box::view`/`SearchBoxColors`,保留其余自由函数与 `CursorDir`。
- `App::ime_cursor_area`(`app.rs:3948`)里 `browser_addr_editing()` 那个
  用"预览列上部近似坐标"顶替精确光标位置的分支删除——iced 迁移后自己
  通过 `shell.request_input_method` 算准确位置,不再需要这个近似兜底。
  `acceptance_comment_editing()` 分支保留(验收意见框不在本次范围)。

## 测试

- 单元测试:`byteui` 新增/扩展组件的构造性测试(参照
  `form::mod::all_form_components_construct_without_panic` 的既有写法);
  `main.rs` 路由判断优先级用现有 `rail_drag_tests` 一类的纯逻辑抽取手法
  (若 `to_native_text_input`/`to_self_drawn_input` 的组合判断能抽成自由
  函数,写表驱动的组合测试)。
- 手工 GUI 走查是本次不可替代的验证环节(自动化测不出"光标是否真的能拖
  选中文字""IME 候选框是否跟手"这类像素级/系统集成行为),覆盖目标一节
  第 6 条列的全部检查项,对 6 个输入框逐一过。**重点验证回归项**:
  每个输入框都要在"旁边有终端 tab 正在跑"的前提下测,确认按键/粘贴/
  方向键不再(或本来就没有)漏进终端。
