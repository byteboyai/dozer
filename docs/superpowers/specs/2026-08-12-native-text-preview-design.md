# 文件预览:文本/代码类扩展名原生化设计(阶段 1)

**状态:已批准(brainstorming 会话,2026-08-12)**

## 背景

文件预览面板(`crates/dozer-app/src/preview.rs` + `main.rs` 的 wry webview 池)一期就
定为 `wry(WKWebView) + 自托管 Flyfish`(见 `docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`
"WebView 合成"一节),并在设计时就把"Flyfish"定位成 **Preview Engine 抽象的第一个实现**,
不是唯一实现。

这次改动的直接触发点:文件预览面板右键弹出菜单后,预览内容会消失或反过来把菜单盖在下面
——根因是原生子视图(wry webview)恒在 iced 绘制层之上,此前几轮修复都在"菜单盖到预览区
就把整块 webview 藏成零矩形"这个二选一策略里打转,对话记录见本次会话前半段的调试结论。
已把那次修复回退到"webview 照常显示、菜单可能被盖住"的原状(`main.rs`/`app.rs` 已提交)。

借这次回退的机会,复核了原生方案的可行性,发现关键积木已经在仓库里现成:
- `extensions/acceptance.rs` 的 diff 视图已经是纯 iced 原生渲染,不经过 webview。
- `vendor/iced-code-editor`(本仓库已有依赖,`syntect` 语法高亮)已经在"编辑"弹层里跑,
  证明同一版本 syntect + iced 0.14 的组合在这个代码库里是成熟可用的。
- `iced_widget 0.14.2`(项目当前锁定版本)本身就带 `pulldown-cmark` 依赖,原生 Markdown
  渲染是现成能力,不需要新引入。
- 网页预览(`localhost` 产物)已经在此前的浏览器面板扩展化里搬到独立的 `extensions::browser`,
  不在这次讨论范围内,继续用 wry,不受影响。

flyfish 当前体积约 19MB/316 文件,其中 `renderers/text.iife.js`(3.4MB)是这次阶段 1 要
绕开的渲染器,`image.iife.js`(1.4MB)是后续阶段目标,PDF/PPTX 相关(约 4MB)按下述范围
决定保留。

## 目标 / 非目标(阶段 1)

**目标**:
1. `preview.rs` 现有 `is_editable_extension` 白名单覆盖的扩展名
   (rs/toml/md/txt/json/yaml/yml/sh/py/js/ts/tsx/jsx/html/css/xml/log/conf)改走原生
   `iced-code-editor` 只读渲染,不再经过 wry/flyfish。
2. `vendor/iced-code-editor` 的 `CodeEditor` 新增 `read_only(bool)` builder,预览 tab 与
   "编辑"弹层共用同一个渲染实现,只是各自独立的实例、各自不同的 `read_only` 取值。
3. 消灭这个扩展名子集上的原生子视图 z-order 问题——右键菜单/⌘K 等浮层此后对这些 tab 是
   普通 iced 层级关系,不需要任何隐藏/裁剪 hack。

**非目标(留给后续阶段,不在这次 spec/plan 范围内)**:
- 图片(`image.iife.js`)、PDF、Office 长尾格式**不动**,继续走 wry/flyfish,继续沿用
  ⌘K 隐藏预览那套已有机制(P1 spec 里"已定"的部分,这次不碰)。
- Markdown 的**渲染态**(标题/加粗/列表可视化)不在这次范围——阶段 1 里 `.md` 只是作为
  白名单里的一个扩展名,按纯文本+语法高亮显示,跟 `.rs`/`.py` 等同等对待,不解析成
  富文本。真正的 Markdown 渲染视图是独立的后续阶段。
- 不合并"预览"与"编辑"两层为一个组件——继续保持分层(brainstorming 会话已确认),
  `PreviewTab` 与编辑弹层各自持有独立的 `CodeEditor` 实例。
- 不裁剪/精简 flyfish 资产包体积(`text.iife.js` 等文件不删除)——flyfish 是整体预构建
  产物,裁剪单个 renderer 需要改它自己的构建流程,超出这次范围;体积收益作为副产物观察,
  不是这次的验收目标。
- 不处理大文件性能上限——如果原生渲染在实测中对超大文件(如巨型日志)确实有明显卡顿,
  留到发现问题时再补,不预先设计一套文件大小上限机制(YAGNI)。

## 关键语义确认(brainstorming 会话定案)

- **混合方案**:白名单扩展名走原生,PDF/Office 长尾继续走 wry,两者共存于同一个
  Preview Engine 抽象下,按文件类型分发,不是全有全无的替换。
- **分步迁移,文本/代码先行**:这份 spec 只覆盖第一步;图片、Markdown 渲染态、PDF 各自
  是独立后续阶段,各自单独 spec + plan + 验收,不在这次一次性做完。
- **复用而非并行造轮子**:选定"给 `iced-code-editor` 加 `read_only` 标志、预览与编辑共用
  同一渲染实现"这条路(brainstorming 阶段的方案 A),否决了"预览另起一套基于
  `rich_text`/`syntect` 的独立渲染器"(方案 B,会导致两套高亮/主题实现长期维护漂移)和
  "直接复用可编辑的 `CodeEditor` 但忽略其编辑消息"(方案 C,语义别扭且白白初始化了
  光标/LSP 等编辑态脚手架)。
- **原生渲染不需要 wry 那套"期望清单 diff 同步"模式**:`wry::WebView` 因为句柄生命周期
  约束("句柄只活在事件分发环",见 `main.rs` 现有注释)才需要 `sync_webview_pool` 那种
  "期望清单 vs 实际池"的差集协调。`CodeEditor` 是普通 Rust 值,没有这个约束,直接作为
  `PreviewTab` 的字段存在、随 tab 关闭自然 drop 即可,不需要镜像一个 `HashMap<usize, _>`
  池 + 协调逻辑。

## 架构与数据流

### 1. `vendor/iced-code-editor`:新增 `read_only`

```rust
impl CodeEditor {
    pub fn read_only(mut self, value: bool) -> Self {
        self.read_only = value;
        self
    }
}
```

`update()` 顶部对编辑类 `Message`(键入、粘贴、删除、IME 提交等)在 `read_only == true`
时直接短路返回,不进入原有处理分支;滚动、光标移动/选区(用于预览态下的复制)等消息不受
影响,照常处理。默认值 `false`,现有"编辑"弹层调用点不用改一行代码。

### 2. `PreviewTab` 新增字段

```rust
pub struct PreviewTab {
    pub id: usize,
    pub kind: TabKind,
    pub title: String,
    pub reload_nonce: u64,
    /// 仅白名单扩展名文件 tab 有值;非空即代表这个 tab 走原生渲染路径,
    /// `desired_webviews()` 据此把它从 wry 期望清单里排除。
    pub editor: Option<iced_code_editor::CodeEditor>,
}
```

### 3. `open_path` 分支

```rust
fn push_tab(&mut self, kind: TabKind, title: String) -> usize {
    let editor = match &kind {
        TabKind::File(path) if is_editable_extension(path) => {
            // 复用编辑弹层已有的读取+构造逻辑(`Self::extension_to_syntax`),
            // 只多链一次 `.read_only(true)`。读取失败走既有的
            // open_missing_file_sets_preview_error_and_no_tab 路径,不在这里
            // 新增错误处理分支。
            read_and_build_editor(path).ok()
        }
        _ => None,
    };
    // ...原有 push 逻辑,tabs.push 时带上 editor 字段
}
```

`desired_webviews()` 改为过滤掉 `tab.editor.is_some()` 的条目,其余(图片/PDF/未知扩展名)
不变,继续进入 wry 期望清单。

### 4. 外部改动触发的刷新

`bump_reload(tab_id)` 现有实现只对 `reload_nonce` 计数、驱动 wry 侧 URL 换参。这次改为:
若该 tab 有 `editor`,重新读取文件、就地重建一个新的 `CodeEditor`(只读态没有光标/undo
历史值得跨重建保留,直接换新实例比"原地更新缓冲区"更简单可靠);若该 tab 走 wry 路径,
沿用现有 `reload_nonce` 机制不变。两条路径共用同一个调用点(编辑弹层保存后触发),按
`tab.editor` 是否为空分流。

### 5. 渲染

预览面板原来"要么留出 wry bounds 矩形、要么留空"的那个渲染分支,新增第三种情况:
`tab.editor` 有值时,直接把它的 `.view()` 作为普通 iced `Element` 放进那个槽位,参与正常
iced 布局/层级——右键菜单等浮层从此对这类 tab 是普通的"后渲染在上层"关系,不需要
`main.rs` 里那套 bounds 清零判断。

## 错误处理

- **文件读取失败/不存在**:不新增错误路径,复用现有
  `open_missing_file_sets_preview_error_and_no_tab` 行为——不管目标扩展名是否在白名单里,
  读取失败就不开 tab、报现有的预览错误态。
- **白名单扩展名但内容非合法 UTF-8**(扩展名判断为文本,内容实际是二进制):用
  `String::from_utf8_lossy` 兜底显示,不新增专门的错误提示——多数文本查看器的通行做法,
  避免为一个边缘情况新增用户可见的错误状态。
- **超大文件**:不预先设计限制,列为验收阶段要实测的观察项(见非目标)。
- **`read_only` 消息分流出错**(该拦的编辑消息没拦住,或该放行的滚动消息被误拦):这是
  `vendor/iced-code-editor` 改动里唯一的正确性风险点,靠下面的单测直接锁定。

## 测试策略

- **`vendor/iced-code-editor`**:新增单测——构造 `read_only(true)` 的 `CodeEditor`,喂入
  编辑类消息(键入/删除)断言缓冲区内容不变;喂入滚动/光标移动消息断言对应状态确实更新。
- **`preview.rs`**:扩展现有 `open_select_close_tabs`/`desired_webviews_builds_urls_and_visibility`
  风格的测试——断言白名单扩展名文件 tab 不出现在 `desired_webviews()` 输出里,非白名单
  (图片/PDF/未知扩展名)tab 仍然出现;新增 reload 场景断言(编辑弹层保存后,原生 tab 的
  `editor` 内容确实更新)。
- **人工验收**(GUI 变更走此项目既有约定):
  1. 打开一个 `.rs` 文件预览,确认 wry `webviews` 池里不会为这个 tab 创建条目。
  2. 该 tab 打开状态下,在文件树上右键,确认菜单正常显示在最上层、预览内容不消失、不
     被裁剪——这是这次改动要解决的原始 bug 的直接回归验证。
  3. 打开一张图片和一个 PDF,确认两者仍走 wry 路径、⌘K 隐藏预览的既有行为不受影响。
  4. 点击原生预览 tab 的"编辑"按钮,确认弹层正常打开、保存后原生预览 tab 内容同步刷新。
- **回归防护**:`cargo build`、`cargo test -p dozer-app --bin dozer`、
  `cargo clippy --all-targets`、`cargo fmt` 全绿。

## 依赖变更

无新增外部依赖——`syntect`(已在 `dozer-app`/`iced-code-editor` 依赖树里)、
`iced_widget`(已锁定 0.14.2)均为现状复用。`vendor/iced-code-editor` 本身的改动是新增
一个 builder 方法 + `update()` 里一处早退分支,不涉及新依赖。
