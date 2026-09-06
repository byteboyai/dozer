# 原生文本预览:就地可编辑 + ⌘S 保存 + 文件内搜索 Implementation Plan

> **Status:** 草案,待用户确认后实施。GUI 无法自动目视验证,分阶段提交,每阶段
> cargo test -p dozer-app / clippy -D warnings / fmt 全绿。

**Goal:** 回答 2026-09-06 用户两项:
1. **使用 text editor 打开的文件都不要启用只读模式** —— 原生预览 tab 的 `CodeView`
   建成可写(`read_only=false`),用户可拖选/复制/直接改,配套「就地保存 + 关前兜底」。
2. **补上文件内搜索** —— 原生预览编辑器加 ⌘F 的 Find 条并跳转当前匹配。

**背景裁决(用户已答):** 保存行为 = **直接在预览里可编辑 + ⌘S 保存**(不靠右键
EditSession 弹层那套主保存逻辑);弹层本身「可暂时保留」,本期不动 `EditSession` 相关
注册/右键项,只把预览 tab 自己的编辑器改成可写。

**架构关键事实(调研归档,2026-09-06):**
- `PreviewPane`(preview.rs)是**单一结构体**,Files 与 Project 各持一个实例
  (`ws.preview` / `ws.project_preview`)。把可写/脏/搜索做在 `PreviewPane`(PreviewTab)
  上一个通用实现,两个面板**自然同享**,无需分叉两份(只在消息/前缀/error 落点上按
  `PreviewPaneKind` 选)。Searchable 状态同样放 Pane 上按 kind 触发,天然隔离。
- `CodeView`(code_editor/mod.rs)`Content` 为官方 iced `text_editor::Content`;对
  搜索接 dot:已知 `Content::move_to(Cursor)` 无条件写光标(iced_widget
  text_editor.rs:518-524 已读证实),因此「跳到第 n 个匹配」=先在该 CodeView 里算出
  `(line,col)`,再 `move_to`.而 `Content::text()`/`line(i)` 已有,可行字符串定位→
  位置转换(line 是 buffer 逻辑行,不随 wrapping 变——与 `cursor_position()` 同义,
  见 code_editor/mod.rs:65-68)。
- 键盘:(main.rs:1191-1195)原生预览编辑器激活且 `current_focus==Preview` 时,
  这道门 `return`,把一切(含 ⌘)交给 iced/widget 自己消化。所以 ⌘S(保存)/⌘F(搜索)
  必须**插入 1191 判断体内部、在其 `return` 之前**,且仅在该分支命中时转 app 消息;
  它天然不会与终端 ⌘C/⌘V(1234)或 Ctrl 缩放(1298)冲突。Esc 关 Find:Escape 默认
  会经 iced 管线落成 `Binding::Unfocus`(iced text_editor from_key_press Escape→
  Unfocus),Find 条自带 Esc→close 需当作编辑器外层浮层,由 main.rs 提前拦(仿
  1153-1168 Esc 弹层瀑布加一段 find_open && Escape → close)。
- 脏标记当前无处存(PreviewTab 无 saved_content;`code_editor` 之 read_only 过滤变消失
  = 唯一 semantics 变化)。补每 tab `dirty: bool` + 保存重置的清晰标记位最省。

## Global Constraints
- 独立分支 `feature/native-preview-editable-find`,不直接提交 main;完成自测后提请审阅再合并。
- UI 标签英文、注释繁体中文、ByteBoy2077 配色;icon/tab 复用既有统一组件,不手写新一族
  MouseArea+hover。
- 关键字非贪心且小步;不做「同时双编辑器同一文件」的去重(EditSession 本期保留,两者并存的
  轻微不一致(最后保存者胜)是已知取舍,注释说明,不做跨 buffer 同步)。
- 每任务:`cargo build`(改动 crate)+ `cargo test -p dozer-app` + `cargo clippy --all-targets -- -D warnings` + `cargo fmt`。
- 不做 GUI 自动化自检;行为以保证编译 + 单测覆盖核心状态转移 + 键盘路由读代码评审为准。

---

## Task 1:原生预览 CodeView 建造成可写(去只读)

**Files:** `crates/dozer-app/src/preview.rs`
**改动:**
- `read_and_build_native_editor`(preview.rs:102-119)第三参 `true` → `false`。
  这使它同时供 `push_tab`(266)与原生 tab 的 `bump_reload`(430)重建用的两条路径都变可写。
- 更新该函数与 `TabKind::File`/`is_editable_extension` 上方大量「只读预览」注释,并同步
  `code_editor/mod.rs` 模块文档中「preview.rs 只读/workspace 编辑弹层可写」的描述,
  避免文档与代码脱节。

删除指向「只读右键编辑入口」的注释段落(因现在预览本身可写,那一段意图已过时)。

## Task 2:PreviewTab 加待保存脏标记,保存后清除;reload 后视为已保存

**Files:** `crates/dozer-app/src/preview.rs`
**改动:**
- `PreviewTab` 加字段 `dirty: bool`(初值 false,构造处设 false)。仅原生编辑器 tab 用得
  到,`Blank`/`webview` tab 恒 false。`Debug` 手写在字段列表补 `.field("dirty", ...)`。
- 三处一次性「buffer 与磁盘一致」判定由 Tab 持有者显式置 false,不靠全量文本比较:
  1. `push_tab` 原生 editor 构造成功后(返回前)该 tab 的 dirty=false;
  2. `bump_reload` 原生分支持盘重建成功后 `dirty=false`;
  3. `reload_at`(右键“刷新”路径)最终走 bump_reload→覆盖到 2。
- 新增只读访问 `pub fn is_dirty(&self, idx) -> bool` / 或方法在 Pane 上按下标/given tab。
- 新增一个 `pub fn mark_dirty(tab_id)` 由转发编辑事件层在**确为文本变更的 Action**时调用
  (见 Task 3)。

## Task 3:Workspace 把向外翻的编辑事件改为“文本变更→mark_dirty + 其余映射”

**Files:** `crates/dozer-app/src/workspace.rs`
**改动:** `preview_tab_editor_event`(817)/`project_preview_tab_editor_event`(825):
每条原生 editor 事件不再只 `editor.perform`,先判定该 Action 是否属于会改正文的类别
(`Action::Edit(_)` 即插字/退格/删除/粘贴/回本/IME paste——唯一能改内容者),是则对
对应 Pane+`tab_id` 记 dirty。用 `matches!(action, EditorAction::Edit(_))`(类型对齐处
type 为 `iced_widget::text_editor::Action`,别名 EditorAction)。

## Task 4:⌘S 保存当前激活原生预览 tab → 磁盘 + 清脏

**Files:** workspace.rs / app.rs
**改动:**
- `Workspace` 加方法 `preview_pane_save(PreviewPaneKind)`:按该 pane 激活 tab 取
  `CodeView::path`(TabKind::File)与 `editor.text()`, `std::fs::write`;成功则对该 tab
  清 `dirty`,失败写 `preview_error/ project_preview_error`(与既有两个 error 字段通道一致,
  workspace.rs:3410 已负责渲染)。
- `app.rs` 加两个消息(或复用 PreviewPaneKind 泛型一个)+ handler 转发到上面方法(对 Files /
  Project 独立),并分别映射 main.rs 新快捷键(见 Task 5)。

## Task 5:⌘S / ⌘F 键盘路由

**Files:** `crates/dozer-app/src/main.rs`
**改动:** 在 native-preview gate(main.rs:1191-1195)**内部、return 之前**插:
```
if let FocusIntent::Preview(kind) = *current_focus
    && app.active_preview_tab_has_native_editor(kind)
{
    // ⌘S 保存原生预览激活 tab;⌘F 打开该 tab 的 Find 条;其余行为照旧交给 iced 编辑器自己。
    ... super_key + Character("s") =>  Save
        super_key + Character("f") =>  FindShow
    return;
}
```
(把命中 super 的那两种抽成早期 return,否则其余按键维持「return 放行给 iced」原状)——
注意这些**必须先于同段 return**,且只在 focus 恰为待原生预览时触发,不会截其它界面。

Esc 关 Find:main.rs 顶部 Esc 弹层瀑布(约 1153-1168)附近加一段:find_bar_open 且
Esc Pressed → close find(+ 顺手把键盘焦点从 find 输入框收走)并 return。
- Find 输入框是真实 iced 原生 text_input → 还需让它进 1208-1225「原生放行闸门」，把
  输入框本身的聚焦识别接进 app(仿 app.files_search_focused 之类),否则打进去的每个字符
  会被视作要转给终端。见 Task 8 一并列出。

## Task 6:脏原生 tab 关闭/项目切换前确认(兜底)

**Files:** workspace.rs / app.rs
**范围取舍:** 为在保留 EditSession 的前提下不重复实现一套平行 modal,复用既有
`Message::{PreviewEditCloseRequest, PreviewEditConfirmDiscard/Cancel}` 的最小手法与
`edit_discard_confirm_popup`(workspace.rs:3582-3649)那一套 UI 形态?
→ 不行——那是绑定在 edit_session 的 bool。正确最小方案:新增一组 Workspace 侧字段
`pending_dirty_close: Option<DirtyClose{ pane:PreviewPaneKind, idx:usize }>` 与 App 层
模态把它叠进现有浮层链(app.rs 7682 那条 else if 链新增一个优先级),按钮回
`Message::{PreviewDiscardAndClose, PreviewKeepOpen,pane,idx}`。
- `PreviewCloseTab(idx)` / `ProjectPreviewCloseTab(idx)` handler(应是 app.rs:4918 那组)
  在直接 `ws.preview.close(idx)` 前判该 tab `is_dirty`;脏→把 close 转成 pending_dirty_close
  存下(不真实关),并顺带记一条「取消后不重复询问」?由 Keep 静默。
- `close_all_tabs_for_switch`(workspace.rs:1135,项目页签×/AgentPicker)原本 `clear_all`
  无确认;脏且多个/单个时可能整栏丢弃。为最小正确性:若 pane 内存在 dirty 原生 tab,先弹
  一次「项目切换会丢弃未保存的文件改动」确认(dis/allow) 而不真切，确认后再空。给一个
  `Workspace::has_dirty_native_tab(kind)`。
为空化彻底，本期不做「逐 tab 逐个确认」，只做一次整体确认(有脏→要你点头)，命中风险最大的
项目页签关闭路径即可防丢。

## Task 7:CodeView 增加 Find 数据模型与跳转(纯状态+单测)

**Files:** `crates/dozer-app/src/code_editor/mod.rs`
**能力(直接在 CodeView 上?不,CodeView 目前是“无状态渲染器”,跳转光标要落进 content):**
加:
- `pub fn match_find(query, start_byte_from_cursor or from 0)` → 不持有任何新状态,纯函数给出
  全部匹配的 **(line,col) 或字节区间**。用一个内部迭代 util。大小写不敏感可选(默认 insens)。
  单行/跨行都支持(找到的区间可能跨逻辑行——但跳转用 move_to 到首字符即可;把全部匹配端点
  记回)。仅文档说明不接高亮覆盖(官方文本编辑器没有公开文字层绘制 API,不开 gfx 工程)。确认
  MVP 只做**跳到第 n 个匹配首部** + 计数,高亮叠色面不做(需自绘 canvas 文字层,超本轮 UI
  范围,另行开题)。
- `pub fn select_next_find(query, from) -> (bool/*found*/, next)`:在匹配序列上推进，直到跨到
  行尾 wrap。返回位置集;真正落盘 postion 由调用层决定怎么 map 进 `Content.move_to`。因为
  dozer CodeView 的 content 本体是私有,把「找到就 set 光标 + 返回匹配号」直接做成方法,
  内部调用 `self.content.move_to(text_editor::Cursor…)`。
  但 move_to 需要的 `Cursor` 类型由 iced 给:注意它需要包 `Selection::Range`/Caret 信息——
  构造一个 caret 型 Position 再 move_to（详见勘探：move_to(Cursor) 把一个 selection 即光标
  “reflect”过去，能放确定点位，测试往返断言 cursor_position() 命中）。Cursor 的
  Range/Caret 形态需读 ced_text cursor 具体体再定——以实现时为真（勘探读到 Content::move_to
  公开接受 editor cursor，能放 Caret(Position{line,col}) 就够）—— 若那天不能放纯 caret，就退
  到构造 `Selection::Range(anchor_same_as_active)`这种 caret 化 range 的探法，并用
  `cursor_position()` roundtrip 单测兜住。这正是要写单测的原因（code_editor 现有 tests 一组，
  新增几个求 position roundtrip / 多行 / 大小写 insens / no-match none）。
  尽量把 Content 交互限制成 move_to + read-only 读,绝不自己编辑 text（搜索不改 buffer）。

## Task 8:Find 条 UI + typing/next/prev/close/⌘G 接线

**Files:** workspace.rs / app.rs(+ tab_widget 若需要)
- `PreviewPane` 加 `find: Option<FindBar{ query, match_no, count, open_at_active_id }>`(纯 UI
  状态)。每 pane 各自持 find → Files/Project 互相独立、⌘F 只开当前 focus pane 的。
- workspace.rs preview_pane_for 原生 tab 内容区(编辑器 view 下方/浮层)在 find.is_some 时：
  - 顶部一行(「查找」+ 输入框 text_input + n/m + 上一/下一 箭头 + ×)或底部横条，样式复用
    text_input / icon_button_entry;文字 ByteBoy 配色。确认输入框可聚焦(接 Task 5 放行闸门)。
  - 每个 query 变更触发 Task 7 select/find 并把匹配号计进状态标签。
- 输入框文本空 → 显示“在文档中输入以搜索”,匹配 0 灰。
- ⏎ 下一、Shift-⏎/⌥ 上一(选项)。选中后当前匹配即灰色定位。
- 消息：`PreviewFindToggle(kind)`、`PreviewFindText(kind,String)`、`PreviewFindNav(kind,direction)`
  + ×。放 app.rs；Update 里调找 CodeView methods（因为 buffer 在 PreviewTab，把「执行 find→set
  cursor」放在 Workspace 层对 PreviewPane 调，不把 Pane 暴露给 main）。

## Task 9:`bump_reload` 与保存的一致性一条已知取舍

文档化:
- 外部 FS 变更仍只跟踪 webview tab(preview.rs `reload_webviews_for` 445-470 不看原生 tab),
  原生 tab 手动“刷新”走 bump_reload 重建实例 → **会丢弃未保存改动**。可写后这种丢失
  “变得真实” → Task 6 已给“项目切换”兜底；手动刷新重置这一条要在右键菜单“刷新”触发前
  判脏（脏则转 pending_dirty_close 风格确认）。实现时若发现触发点繁，就把“刷新”按脏分
  支到确认(`reload_at` 上面由 handler 判 dirty)。
注：这是守住“不会悄悄丢”的全部要点。不做“保存时把其它 wry 同文件刷新”的跨层联动
（现阶段只存原生 tab 自身）。

---

## 验证
每个 Task 完整后:对应文件 `cargo test -p dozer-app` + clippy/fmt；核心行为靠 Task 2(构造后
dirty=false)、Task 4(save 清脏+error 通道)、Task 7(position roundtrip / 多行 / 大小写 / no-match)
上述新增测试覆盖;⌘S/⌘F 路由靠 main.rs 阶段读代码评审 + 现有键盘放行结构是否回归(即 preview
gate 后新增早退不破坏“其余按键供 iced”的口径)。

## Non-Goals (本期硬边界)
- 不做语法高亮叠 layer / 行内多重高亮标记(搜索仅移动光标+计数)。
- 不动右键 EditSession 弹层(保留)。
- 不做跨 webview/原生 同文件保存联动。
- 不做 ⌘G 前进“自动 stop at 0→wrap”、⌘⇧G 后退之外的自定义循环导航样式（仅基础 next/prev）。
- 不做 grep-in-project；Find 仅当前文件。
