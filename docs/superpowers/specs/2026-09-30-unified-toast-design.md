# 统一消息 Toast 设计

状态:已按 `docs/superpowers/plans/2026-09-30-unified-toast.md` 实施阶段 1-2(2026-09-30)。未列入 spec §8 的显式未决项之外,不改变一期范围。

## 背景与动机

Dozer 目前没有任何统一的"瞬时消息"机制,各处各自造轮子:

- `agent_context.rs` 有自己的 `notice: Option<String>` + `DismissNotice`,画在终端下方的条里,红字,点击才消失。
- `App.daemon_error: Option<String>` 同时承载"dozerd 已停止"(持久状态)与"打开项目失败"(一次性事件),并被画在三处:`term/terminal.rs`(终端栏)、`chrome/homespace.rs`(首页)、`app/view.rs`(无项目空态)。**终端栏那处会多占一行,挤压 PTY 网格触发 resize**——2026-09-30 已单独删掉那一处(最小修复),但根因是没有合适的提示载体。
- 另有 `tree_error`/`rollback_error`/`save_error`/`scan_error`/`web_error`/`preview_error` 等十余个 `Option<String>` 字段,各自一段 view 代码。
- 约 80 处 `eprintln!`/`tracing` 日志,用户看不到;`extensions/files/update.rs:168` 的注释直接写着"只记日志,不弹 toast"。

这些提示没有统一的级别、时长、堆叠、去重规则,也没有"不占布局"的展示位。

## 目标 / 非目标

**目标**

1. 一个全局、不占布局、自动消失的瞬时消息载体(Toast)。
2. 各 extension 只通过消息触发,不依赖 Toast 内部(遵循"extension 间只能消息通信")。
3. 遵守 CLAUDE.md 关键裁决:**新浮层默认走独立原生子窗口**,天然叠在 webview 之上,不写任何 `preview_desired` 隐藏逻辑。

**非目标**

- 不替换持久状态提示(`daemon_error` 的"dozerd 已停止"、`tree_error`、`git_error` 等带重试上下文的错误)。
- 不替换表单内联校验(如 `project_create.error`)。
- 不做通知中心/历史列表(一期不做,见未决项)。
- 不做非 macOS 回退路径(与"弹窗独立窗口通用化第二份 spec 暂停"同一立场:非 mac 从未实际编译过,等真发布再做)。

## 分类:什么进 Toast,什么不进

| 类别 | 例子 | 载体 |
|---|---|---|
| 瞬时结果 | 已复制、保存成功、打开项目失败、外部应用打开失败、"已发送到终端但未能记录到上下文列表"、删除项目未完全成功 | **Toast** |
| 持久状态 | dozerd 已停止、文件树加载失败、git 错误 | 保留原位(带重试);`daemon_error` 的持久部分改为不占布局的常驻小徽标(第三阶段) |
| 内联校验 | 新建项目表单错误 | 保留在表单内 |

判据:**消息描述的是"刚刚发生了一件事"→ Toast;描述的是"当前处于某状态"→ 状态位。** 一个字段两种语义混用(现在的 `daemon_error`)就是要拆的对象。

## 架构

分两层,便于先落纯逻辑再落窗口。

### 层 1:`ToastCenter`(纯逻辑,无窗口依赖)

新模块 `crates/dozer-app/src/extensions/toast.rs`(沿用 `extensions/` 的 `Message` + `apply` + `view` 三段模式,状态挂在 `App` 上,全局唯一,不 per-project)。

```rust
pub enum Level { Info, Success, Warning, Error }

pub struct Toast {
    pub id: u64,              // 单调递增
    pub level: Level,
    pub text: String,
    pub key: Option<String>,  // 去重键;同 key 再推 = 刷新文本与计时,不新增
    pub expires: Instant,     // v1 所有级别都自动到期(见「实施偏差」)
}

pub struct ToastCenter { items: Vec<Toast>, next_id: u64 }
```

> **实施偏差(v1)**:草案里的 `created`/`hovered`/`expires: Option<Instant>` 未实现。
> v1 窗口整窗点击穿透(见层 2),收不到悬停事件,悬停暂停/手动关闭一并去掉;
> `expires` 恒为 `Instant`(所有级别到期,Error 8s),`created` 无消费方故不存。

规则:

- **时长**:Info/Success 3s,Warning 5s,Error 8s。(草案的"悬停暂停/移出补足"未实现,见上。)
- **堆叠**:最多同屏 3 条(`MAX_VISIBLE`),新的在下;超出时最旧的被挤掉(不排队,瞬时消息过期即无意义)。
- **去重**:相同 `key` 视为同一条(用于"重试失败"这类会连续触发的场景);无 key 时按 `(level, text)` 去重;`key=None` 与 `key=Some` 互不去重。
- **文本归一化**:空白(含换行)折叠成单个空格并去首尾;归一化后为空则忽略,不产生空白 Toast(固定高度几何,多行会撑坏)。
- **API**:`App::push_toast(level, impl AsRef<str>)` / `App::push_toast_keyed(level, text, key)`;extension 侧通过 `Message::Toast(toast::Message)` 冒泡给 App(与 git_log 的 emit 回调模式一致),不直接持有 `ToastCenter`。
- **到期唤醒**:`next_toast_wake() -> Option<Duration>`,返回最近一条到期的剩余时间,无待过期项返回 `None`(不空转)。

### 层 2:`ToastOverlay`(渲染宿主,独立原生小窗口)

放 `platform/toast_overlay.rs`,复用 `overlay_window` 的建窗与 `OverlayGpu` 管线,**但与 8 个模态卡片宿主有三处必须不同**:

1. **不覆盖整窗、不加遮罩。** 模态宿主的 overlay 窗口等于主窗口客户区,遮罩会拦截点击;Toast 窗口只等于"当前 Toast 堆叠的包围盒",定位在主窗口右下角(距右 16px、距下 48px 给 footbar 留位)。窗口本身之外的区域不属于它,点击自然落到主窗口。
2. **不抢焦点、整窗点击穿透。** v1 走 `open_child_window_unfocused`(`with_active(false)`)+ `set_cursor_hittest(false)`:点击落到下面的主窗口,窗口永远不会成为 key window,不会抢终端键盘焦点。**代价**:收不到悬停/点击,故没有"悬停暂停"和"点击关闭"(全部级别自动到期)。要加这两个交互需先重新评估焦点方案(macOS 上点击 `canBecomeKeyWindow == true` 的子窗口会抢终端焦点)。
3. **窗口随堆叠变化 resize / 无 Toast 时销毁。** 有 Toast 时才建窗,清空后关窗;不常驻空窗口。

内容:`level` 决定左侧色条与边框强调色(沿用主题 token:成功 `#1AD585`、警告金、错误红、信息青),文字用系统默认字体(`Font::default()`,非代码场景),`Shaping::Advanced`。几何固定(宽 380、高 56 逻辑像素,文本超出裁剪),所以窗口尺寸只取决于条数,`stack_bounds` 是纯函数;窗口尺寸钳到主窗口内且 ≥1(避免 0/负尺寸导致 wgpu surface 配置失败)。

主窗口移动/resize:沿用 `reposition` 的跟随机制(重新计算右下角锚点)。

### 主循环接入

`platform/window_events.rs::about_to_wait` 的 `wakes` 数组新增一项 `(next_toast_wake.is_some(), …)`(数组长度 6→7),到期那一刻精确唤醒一次做清理与重绘,与 `next_tooltip_wake`/`next_todo_flash_wake` 同模式;`about_to_wait` 每轮开头调 `sync_toast_overlay` 按 `App.toast` 单向驱动窗口(空→销毁;指纹变→重定位重绘)。

## 第一批迁移点

| 位置 | 现状 | 改后 |
|---|---|---|
| `app/update.rs:3922` | 写 `daemon_error`("打开项目失败,请确认 dozerd 正常后重试") | `push_toast_keyed(Error, …, key="open-project-failed")`;不再写 `daemon_error` |
| `app/update.rs:1411` | 写 `daemon_error`("删除项目未完全成功") | Toast(Warning) |
| `agent_context.rs` `notice` 全部赋值处 | 终端下方红字条 | Toast(一律 Error 级);删除 `notice` 字段的视图渲染与 `DismissNotice`,改为 `State::take_notice()` 由 App 取走转 Toast |

**注意 `update.rs:3928`** `self.daemon_error = None` 在打开成功时清空——改后"打开项目失败"不再写该字段,这一行只对"dozerd 已停止"起作用,行为需回归确认。

> **实施偏差(第一批)**:草案列的 `extensions/files/update.rs:168` 外部应用打开失败**移出本批**。原因:`files::update` 只有 files 自己的 `emit`,没有通往 App 级的通道,接入需要单独的设计决定,留作后续。
>
> **级别**:`agent_context` 的 notice 统一按 `Error`(包括"已发送到终端,但未能记录"这条);要区分级别需给 notice 带级别,留作后续。

## 分阶段

1. **阶段 1 — 纯逻辑**:`ToastCenter`、`Level`、`Message::Toast`、`next_toast_wake` 及单测(去重/堆叠上限/悬停暂停/到期)。可先用最朴素的方式渲染验证(不进入生产),不依赖窗口方案。
2. **阶段 2 — 渲染宿主**:`toast_overlay.rs` + `open_child_window_unfocused` + `about_to_wait` 接入 + 第一批迁移点。
3. **阶段 3 — 持久状态拆分(已落地,2026-09-30)**:`daemon_error` 更名 `daemon_unavailable`,只留"dozerd 不可用"这个持久语义(三种写入:启动连不上、设置里停止、打开项目成功即清除),由**顶栏**红色徽标 `topbar::daemon_badge` 展示(选顶栏而不是 footbar:首页/空态/工作区三种页面都有顶栏,footbar 在空态页没有),点击打开设置、悬停看详情;首页与空态页的两处渲染(终端栏那处更早已删)全部删除,`homespace.json` 的 `colors.error` token 一并移除。一次性事件也拆了出来:`Message::DaemonError` 删除,"新建会话失败""attach 新会话失败"改推 Toast。

## 测试

- 单测(阶段 1,**已落地**):去重刷新计时;第 4 条挤掉最旧一条;`next_toast_wake` 在空/有/已过期时的返回值;文本归一化(空/多行/连续空白);`key=None` 与 `key=Some` 互不去重。
- 几何单测(阶段 2,**已落地**):`stack_bounds` 右下角锚点、多条向上生长、Retina 缩放、极小主窗口钳制且 ≥1、`count==0` 按 1。
- 现有回归:`agent_context` 里针对 `notice` 的测试改为断言 `take_notice()`;新增"取走一次后为 `None`"。
- 人工 GUI 验收(阶段 2,无法自动化):A 焦点(终端输入不被打断)、B 叠在 webview 之上、C 与模态并存、D 到期销毁、E 缩放跟随、F 长文本裁剪、G 中文字体、H 上限 3 条。见计划 Task 3 Step 9。

## 风险

- **macOS 焦点行为**:**已通过"建窗不聚焦(`with_active(false)`)+ 整窗点击穿透(`set_cursor_hittest(false)`)"规避**,无需 `objc2`/AppKit 专属代码;spec 曾列为"最大不确定项"的风险随之消除。若日后要给 Toast 加悬停/点击交互,需重新评估焦点方案。
- **与模态弹窗共存**:Toast 是独立子窗口且不接入 `OverlayKind`/`close_other_overlays`,与任何模态弹窗并存,互不关闭。层级上仍依赖 `addChildWindow_ordered(NSWindowAbove)`(与模态宿主同一机制)。
- **窗口频繁创建销毁**:连续触发时开关窗口开销与闪烁;v1 实测接受,备选(常驻透明窗口)未采用。

## 未决项(不擅自定死)

- 是否需要"通知历史"(错过的 Toast 事后能查)。倾向一期不做;若做,与 `tracing` 日志的关系需另议。
- Error 是否默认不自动消失(需用户手动关闭)。当前 8s 自动消失;v1 整窗点击穿透、无法手动关闭,若 8s 过短需连同焦点方案一起重估。
- Toast 是否支持悬停暂停/点击关闭/动作按钮(如"重试"、"打开设置")。v1 因整窗点击穿透全部不做;要加必须先解决 macOS 焦点问题。
- 位置(右下角)是否需要用户可配置。
