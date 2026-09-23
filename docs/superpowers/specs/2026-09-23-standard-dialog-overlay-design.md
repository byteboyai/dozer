# 标准弹窗组件:模态卡片类弹窗迁移独立原生窗口设计

## 背景与动机

`settings_overlay.rs`(2026-09-18 起)是目前"模态卡片"类弹窗里最新的一个
独立原生窗口消费方,自身文档写道"结构上是 `FileHistoryOverlay` 与
`ProjectCreateOverlay` 两者的混合"——这句话本身就在提示:每加一个消费方就
手写一遍窗口生命周期样板(`open`/`reposition`/`redraw`/`handle_input`/
`Drop`),已经出现明显的复制粘贴漂移。本设计的直接触发点是用户要求"以
settings 的弹窗为基础,重新抽象一个标准弹窗组件",把这个模式沉淀成可复用
组件,并据此制定一份把代码库里其余同类弹窗迁移过去、最后删掉旧的
`Stack`+`scrim` 渲染路径的计划。

`docs/superpowers/specs/2026-09-18-overlay-window-shared-abstraction-design.md`
(以下简称"第一份设计")已经把 `search_overlay.rs` 里通用、非业务专属的机制
抽成了 `platform/overlay_gpu.rs`(`OverlayGpu`)、`platform/overlay_focus.rs`
(`FocusTracker`)、`platform/overlay_window.rs`(`open_child_window`/
`centered_overlay_bounds`)三个共享件,并迁移了 `file_history`。此后
`project_create`、`settings` 又各自新增了一个消费方,均复用这三个共享件,
但"宿主结构体本身"(`window`/`gpu`/`focus`/`open`/`reposition`/`redraw`/
`handle_input` 这套骨架)仍是每个消费方各写一份——四份高度相似但不共享的
代码。

第一份设计明确否决过"一个泛型 `OverlayWindow<Msg>` 包办四类弹窗"(当时的
四类是 `search`/`file_history`,以及尚未迁移的 tab-overflow 下拉/agent
picker),理由是这几类内容形态差异太大(双栏异步 diff vs 简单点击列表 vs
锚点菜单),硬塞一个参数化类型会引入 `Box<dyn Fn>` 与一堆标志位配置,比
分开手写还别扭。**这次要迁移的对象不是那四类**,是另一组此前一直留在主
窗口 `iced::Stack` 里渲染、从未纳入独立窗口机制盘点的"模态卡片"弹窗
(确认框、表单、详情页)。brainstorming 中核实这组对象里多数(5 个)内容
结构完全同质("标题+说明+取消/确认"),已经共用同一个纯视图函数
`dialog::confirm()`——这个同质性是第一份设计讨论的四类里不存在的条件,
使得"一个通用宿主"这次不再是"硬塞进不同形态",而是"内容本来就是同一份
数据结构,只是宿主此前没抽出来"。少数(7 个)仍是形态各异的定制表单/详情/
进度页,继续沿用"各消费方自己写小宿主、共享底层机制"这条第一份设计定下的
路线,不勉强塞进通用类型。

### 这次迁移要修的真实 bug

`App::app_modal_open`(`preview_desired`,`app/app.rs:2990` 附近)与
`App::browser_desired` 里同名的 `app_modal_open`(`app/app.rs:3106`
附近)——决定"要不要显式把 wry webview 设成 `visible=false`"的开关——
目前只检查 `self.text_input_menu.is_some()`。本设计盘点到的 12 个模态卡片
弹窗(见下节)**没有一个纳入这个判断**。wry 原生子视图不听 iced 绘制顺序
摆布,恒在 GPU 内容之上(见 CLAUDE.md 关键裁决),这意味着这 12 个弹窗中
任何一个在 Files/Project/浏览器面板正显示 webview 内容时打开,大概率会被
webview 挡住、部分或完全不可见——brainstorming 中用户确认这不是猜测,是
已经实际遇到过的现象。把它们迁到独立原生窗口(真正的 OS 级窗口,不受 iced
`Stack` 层级摆布,天然不会被 wry 子视图遮挡)会顺带修掉这个 bug,不需要再
给 `app_modal_open` 加另外 12 个判断分支。

## 现状盘点:12 个模态卡片弹窗 vs 出局的锚点菜单/下拉

`app/view.rs` 的 `App::view()` 主体是一条 `if/else if` 优先级链,按当前
匹配到的第一个条件把对应弹窗叠在 `base` 上渲染(`app/view.rs:116-608`)。
链上的分支分两类,靠"背景用 `crate::dialog::scrim`/`scrim_blocking`(整屏
半透明遮罩,模态卡片)"还是"背景用一块透明 `MouseArea`(点击穿透感知,
锚点菜单/下拉)"区分:

**本设计范围内(scrim 类,12 个):**

| # | 触发条件 | 现有视图函数 | 形态 |
|---|---|---|---|
| 1 | `ws.files.tree_delete_confirm_is_some()` | `files::delete_confirm_popup` | confirm |
| 2 | `ws.files.pending_move_is_some()` | `files::move_confirm_popup` | 定制(确认+目标选择器) |
| 3 | `ws.project_panel.delete_pending.is_some()` | `project::project_delete_confirm_popup` | 定制(三选一单选确认,无文本输入) |
| 4 | `ws.project_panel.scaffold_run.is_some()` | `project::scaffold_progress_popup` | 定制,阻塞(`scrim_blocking`,不可取消) |
| 5 | `ws.database.delete_confirm().is_some()` | `database::delete_confirm_popup` | confirm |
| 6 | `ws.database.editing().is_some()` | `database::source_form` | 定制(表单,需 IME) |
| 7 | `self.database.drivers_popup_open()` | `database::drivers_popup` | 定制(管理面板) |
| 8 | `ws.ssh.delete_confirm().is_some()` | `ssh::delete_confirm_popup` | confirm |
| 9 | `ws.ssh.editing().is_some()` | `ssh::host_form` | 定制(表单,需 IME) |
| 10 | `ws.pending_close_tab.is_some()` | `agent_close_confirm_popup` | confirm |
| 11 | `ws.todo.clear_confirm_open()` | `todo::clear_confirm_popup` | confirm |
| 12 | `ws.todo.detail_popup_open()` | `self.todo_detail_popup()` | 定制(详情编辑,需 IME) |

**本设计范围外,原样保留(`MouseArea` 类,锚点菜单/下拉,共 13 处)**:
Files 右键菜单/tab 右键菜单/分支切换下拉、项目链接右键菜单、Todo 分类
右键菜单/分类选择器/状态下拉/日历下拉/派发下拉/状态筛选下拉、输入框右键
菜单(`text_input_menu`)、Database 数据源右键菜单、Agent picker、顶栏
"添加项目"菜单、四处 tab 栏"溢出下拉"(Agent/Files 预览/Project 预览/
SSH/Database)。这些要么已经在 mac 上走 `chrome::native_menu`(NSMenu)
——与 wry 自身的原生右键菜单同源,风格已经统一,不应该动;要么是仅在
`#[cfg(not(target_os = "macos"))]` 下才会触发的死代码(本仓库从未实际
编译/运行过非 mac 平台),继续留给"未来非 mac 真正成为发布目标时"的独立
一份 spec(brainstorming 中用户确认维持第一份设计的既有暂停决定,见记忆
`dozer-overlay-window-abstraction-part2-paused`)。也不包括 `maximize_overlay`
(放大态浮层)与拖拽 ghost 层——它们不是"弹窗",是完全不同的视觉机制。

`search`/`file_history`/`project_create`/`settings` 四个已迁移的独立窗口
消费方不在这次改动范围,只是它们建立的共享机制(见下节)和互斥集合会被
这次新增的 8 个宿主复用/并入。

## 目标 / 非目标

**目标:**

1. 参照 `settings_overlay.rs`/`project_create_overlay.rs`/
   `file_history_overlay.rs` 已验证过的宿主骨架模式(而非改动这三个
   文件本身,见"非目标"),新建一个可直接实例化的**通用确认弹窗宿主**
   `ConfirmOverlay`,覆盖上表 5 个 confirm 形态的弹窗。
2. 上表其余 7 个定制形态的弹窗,各自新建一个小宿主结构体,复用第一份
   设计已抽出的 `OverlayGpu`/`FocusTracker`/`open_child_window` 共享件,
   不引入新的泛型抽象范式(与第一份设计"抽小块,不抽大一统泛型"的取舍
   保持一致)。
3. `OverlayKind`/`close_other_overlays` 从当前 4 个变体扩到 12 个,让这
   次新增的 8 类弹窗与已有 4 类共同纳入同一个"任意时刻只开一个模态卡片
   弹窗"的互斥集合——这本来就是现状 `if/else if` 优先级链隐含的不变量,
   这次只是把它从"靠渲染优先级掩盖"变成显式状态机(与第一份设计"互斥
   机制"一节修的漂移点同一性质)。
4. 全部 12 个迁移完成后,删除 `app/view.rs` 里对应的 12 段 `stack!`
   分支、被替换掉的旧视图函数,以及如果确认不再被任何范围外弹窗引用,
   `crate::dialog::scrim`/`scrim_blocking`。

**非目标:**

- 不迁移上节"范围外"的 13 处锚点菜单/下拉,不重启已暂停的第二份 spec。
- 不改变这 12 个弹窗任何一个的业务逻辑(`State`/`Message`/`update`/
  校验/异步任务)——只改渲染宿主与触发/收起的桥接代码,与
  `file_history`/`search` 当初迁移的既有原则一致。
- 不改动已迁移的 `search`/`file_history`/`project_create`/`settings`
  四个消费方自身的内部实现(只扩展它们共享的 `OverlayKind` 枚举和互斥
  函数)。
- 不新增依赖。
- 不试图让 `ConfirmOverlay` 覆盖 7 个定制弹窗——内容结构不同质,勉强
  塞入没有收益,见"架构"一节。

## 架构

### 1. 通用宿主 `ConfirmOverlay`

新建 `platform/confirm_overlay.rs`:

```rust
/// 覆盖上表 5 个"标题+说明+取消/确认"弹窗的通用宿主。之所以能通用,是
/// 因为这 5 个弹窗的内容已经 100% 由同一个纯数据结构 `dialog::
/// ConfirmDialog<Message>` 描述(`dialog.rs:130-145`)——宿主只需要存住
/// 这份数据,`redraw` 时调一次已有的 `dialog::confirm(spec.clone(), w)`
/// 即可拿到 `Element`,不需要为"内容长什么样"引入 `Box<dyn Fn>` 或标志位。
pub(crate) struct ConfirmOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    spec: dialog::ConfirmDialog<Message>,
}
```

`dialog::ConfirmDialog<Msg>`(`dialog.rs:130-145`)新增
`#[derive(Clone)]`——字段均为 `String`/`Msg`/`Color`/`f32`,`Msg = Message`
已经是 `Clone`,派生零成本。

**几何**:固定逻辑尺寸(同 `settings`/`search`,不像 `file_history` 那样
按主窗口比例缩放)——内容有界(1-3 行说明 + 两个按钮),给一个够用的固定
卡片尺寸(如 420×200 逻辑像素,说明文字换行)即可覆盖当前全部 5 个用例,
不需要每个消费方各自调参。

**桥接:一个 `sync_confirm_overlay`,不是五个。** 每次主循环 dispatch 后
调用一次(与 `sync_search_overlay` 现有调用点同款),按现有 `if/else if`
链的**相同优先级顺序**算出"此刻该显示哪个 confirm 弹窗"(至多一个),与
当前已开的 `ConfirmOverlay.spec` 做 diff:

```rust
fn desired_confirm_spec(ws: &Workspace) -> Option<dialog::ConfirmDialog<Message>> {
    if ws.files.tree_delete_confirm_is_some() {
        Some(files::delete_confirm_spec(&ws.files))
    } else if ws.database.delete_confirm().is_some() {
        Some(database::delete_confirm_spec(&ws.database))
    } else if ws.ssh.delete_confirm().is_some() {
        Some(ssh::delete_confirm_spec(&ws.ssh))
    } else if ws.pending_close_tab.is_some() {
        Some(agent_close_confirm_spec(ws))
    } else if ws.todo.clear_confirm_open() {
        Some(todo::clear_confirm_spec(&ws.todo))
    } else {
        None
    }
}
```

每个消费方现有的 `*_confirm_popup(...) -> Element<...>` 改造成
`*_confirm_spec(...) -> dialog::ConfirmDialog<Message>`——纯粹的返回值
类型改造(标题/说明/按钮文案/颜色原样保留,只是不再自己调 `dialog::
confirm()` 拼 `Element`,改成把这些字段装进 `ConfirmDialog` 返回给调用方
统一拼),不改变各弹窗判断"该不该出现""文案是什么"的既有逻辑。

**关闭:** `ConfirmOverlay::handle_input` 的 Esc 分支与失焦分支统一发送
`self.spec.cancel_msg.clone()`——五个消费方共用同一条关闭路径,不需要
逐个判断"这是哪个弹窗、该发哪条 Cancel 消息"。

### 2. 七个定制宿主

上表 7 个定制弹窗,各自新建一个小宿主文件,结构与 `settings_overlay.rs`
完全同构(`window`/`gpu`/`focus`/`open`/`reposition`/`redraw`/
`handle_input`,需要 IME 的额外 `set_ime_allowed(true)`,需要原生右键
菜单的额外挂靠/归还 `install_content_view`):

| 弹窗 | 新文件 | 需要 IME/原生菜单 | 失焦关闭 |
|---|---|---|---|
| Files 移动确认 | `platform/files_move_overlay.rs` | 是(新文件名可能是中文) | **不接失焦关闭**——"到目录"旁的浏览按钮(`Message::MoveDirBrowse`,`window_events.rs:1566`)会同步弹出原生 `rfd` 目录选择器,那会让本窗口收到一次真实 `Focused(false)`,若照常触发失焦即关闭会把正在填的移动表单整个关掉,同 `ProjectCreateOverlay` 的既有考量 |
| Project 删除确认 | `platform/project_delete_overlay.rs` | 否(纯三选一单选,已读函数体确认无文本输入) | 简单失焦即关闭 |
| Project 修复进度 | `platform/project_scaffold_overlay.rs` | 否(无输入) | `scrim_blocking` 语义原样保留:不接受 Esc/点击/失焦关闭,只有内容里"关闭"按钮(全部步骤完成后才可点)能关 |
| Database 数据源表单 | `platform/database_source_overlay.rs` | 是 | 简单失焦即关闭(已核实表单内无 `rfd::` 调用,不会有嵌套原生选择器) |
| Database 驱动管理 | `platform/database_drivers_overlay.rs` | 否 | 简单失焦即关闭 |
| SSH 主机表单 | `platform/ssh_host_overlay.rs` | 是 | 简单失焦即关闭(已核实表单内无 `rfd::` 调用) |
| Todo 详情 | `platform/todo_detail_overlay.rs` | 是 | 简单失焦即关闭 |

各自的 `State`/`Message`/`update` 不动。视图函数如果现在接收
`window_width: f32` 做比例缩放(如 `*_popup(state, window_size.0)`),
改造成接收调用方给的固定画布(`Length::Fill`),与 `file_history_card`/
`search_card` 当初的调整同理——独立窗口本身已经是量好的画布。

### 3. `OverlayKind` 扩展 + 互斥

`OverlayKind`(`platform/window_events.rs:216-221`)从 4 个变体扩到 12 个:

```rust
pub(crate) enum OverlayKind {
    Search, FileHistory, ProjectCreate, Settings,       // 既有,不动
    Confirm,                                             // 新增:唯一的通用宿主
    FilesMove, ProjectDelete, ProjectScaffold,
    DatabaseSource, DatabaseDrivers, SshHost, TodoDetail, // 新增:7 个定制宿主
}
```

`close_other_overlays` 按现有写法逐一加 8 行 `if keep != OverlayKind::X
{ *x_overlay = None; }`——机械扩展,不改变判断逻辑本身。这把"任意时刻
只开一个模态卡片弹窗"这条现状 `if/else if` 链隐含的不变量,扩展到全部
12 个新宿主 + 已有 4 个,使其成为显式状态机而非渲染优先级副作用。

`Runner::Ready` 新增 8 个 `Option<_>` 字段与 8 个 `sync_*_overlay()`
调用,插入点与现有 `sync_search_overlay`/`sync_file_history_overlay`
相同(每次 dispatch 后)。`window_event` 顶部按 `WindowId` 分流到各宿主
的早退分支,新增 8 段,写法与现有两段一致(互斥机制保证至多一个
`Option` 是 `Some`,分支顺序无所谓)。

## 各消费方改动清单

- `app/view.rs`:删除 12 段 `stack!` 分支(表中 1-12 对应的
  `else if` 分支),`if/else if` 链只保留范围外的 13 处菜单/下拉与最终
  `else` 兜底。
- `extensions/files.rs`:`delete_confirm_popup`→`delete_confirm_spec`;
  `move_confirm_popup` 拆出 `files_move_card`(同 `file_history_card`
  手法)供新宿主调用。
- `extensions/project/view.rs`:`project_delete_confirm_popup`
  (`view.rs:592`)→拆出 `project_delete_card`;
  `scaffold_progress_popup`(`view.rs:516`)→拆出
  `project_scaffold_card`。
- `extensions/database/view.rs`:`delete_confirm_popup`→
  `delete_confirm_spec`;`source_form`→拆出 `database_source_card`;
  `drivers_popup`→拆出 `database_drivers_card`。
- `extensions/ssh.rs`:`delete_confirm_popup`→`delete_confirm_spec`;
  `host_form`→拆出 `ssh_host_card`。
- `extensions/todo/view.rs`:`clear_confirm_popup`→
  `clear_confirm_spec`;`App::todo_detail_popup`(`app/update.rs:4655`,
  是 `App` 的方法而非自由函数)→拆出自由函数 `todo_detail_card`。
- `workspace/view.rs`:`agent_close_confirm_popup`(`view.rs:364`)→
  `agent_close_confirm_spec`。
- `dialog.rs`:`ConfirmDialog` 加 `#[derive(Clone)]`;`scrim`/
  `scrim_blocking` 在 12 个消费方全部迁完后检查是否还有范围外的调用点
  (盘点未发现,预期可删,以实际迁移时的最终 grep 结果为准)。
- 新建 `platform/confirm_overlay.rs`(通用宿主)+ 7 个定制宿主文件(见
  上节表格)。
- `platform/window_events.rs`:`OverlayKind`/`close_other_overlays`
  扩展;`Ready` 新增 8 字段 + 8 个 `sync_*_overlay`;`window_event` 新增
  8 段按 `WindowId` 早退分支;主窗口 `Resized`/`CloseRequested` 各追加
  8 处 reposition/释放。

## 错误处理

与现有 4 个消费方一致:窗口创建失败 `.expect(...)`,资源释放靠
`Drop`,不为新增的 8 个宿主发明另一套降级策略——全部复用
`OverlayGpu::open` 内部统一的失败处理。

`ConfirmOverlay` 的 `spec` 在极端情况下可能与"触发它的那个 ws 状态"
不同步(如 `sync_confirm_overlay` 还没来得及关闭窗口,状态已经在别处被
清空)——处理方式与现有 `ConnectResult` 之类的"过期结果"问题同构:
`handle_input` 发出的 `cancel_msg`/`confirm_msg` 是消息不是直接状态
写入,落到对应扩展的 `update` 里天然会按当前真实状态判断是否还适用
(如 `Editing` 态检查),不会因为弹窗一帧的显示延迟产生错误的双写。

## 测试策略

- 每个 `*_confirm_spec` 纯函数:按现有 `State`/`Message`/`update` 测试
  的写法补单测(断言标题/说明文案/按钮消息,不需要真的起窗口)。
- `close_other_overlays` 扩到 12 变体后的互斥测试:扩展现有穷举式单测。
- `dialog::ConfirmDialog: Clone` 派生:编译期保证,不需要专门测试。
- 窗口生命周期(开/关/拖动跟随/resize 跟随/失焦关闭/连续开关无泄漏/
  与其余 11 类互斥):无自动化测试手段,`cargo run` 人工验证清单,覆盖
  `search_overlay.rs` 原有清单 + 每类新增"打开时其余已开的弹窗被关掉"
  这一项 + 新增"webview 显示预览内容时打开确认框不再被遮挡"这一项
  (对应"背景与动机"一节要修的真实 bug)。

## 排期备注:迁移顺序

第一份设计的"抽出来的共享机制先用一个真实消费方验证"这条方法论在这次
沿用:

1. **通用宿主打底**:先建 `ConfirmOverlay`,只接入其中一个消费方——
   选 Todo 清空列表确认(状态最简单,无异步),验证宿主骨架
   (开/关/定位/焦点/Esc/互斥)全部走通。
2. **通用宿主推广**:其余 4 个 confirm 形态弹窗(files-delete/
   database-delete/ssh-delete/agent-tab-close)接入同一个已验证的
   `ConfirmOverlay`——每个都是提取一个 `*_confirm_spec` + 删旧
   `stack!` 分支的小改动,边际成本低。
3. **七个定制宿主逐个迁移**:每个独立成一个任务/分支,建议顺序上不需要
   IME 的先做(Database 驱动管理、Project 修复进度、Project 删除确认),
   需要 IME/原生菜单挂靠的后做(Database/SSH 表单、Todo 详情),Files
   移动确认单独放在需要 IME 的这一组里最先做——它虽然需要 IME,但额外
   带有"不接失焦关闭"这个后三者都不需要的特殊考量(见上表),先做完能让
   这个例外尽早被验证。
4. **最终清理**:12 个全部迁完后,删 `app/view.rs` 对应分支、删被替换的
   旧视图函数、按最终 grep 结果决定 `dialog::scrim`/`scrim_blocking`
   是否可删。

具体任务拆分、验收标准与分支/worktree 安排由后续 `writing-plans` 阶段
的实现计划给出。
