# GB 级文本文件编辑器打开性能优化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让代码编辑器打开任意大小的文本文件都不再卡死 UI 线程,GB 级文件按内存动态分档只读打开,超出内存预算的部分分块加载,只读档搜索走磁盘流式扫描。

**Architecture:** 根因是 `iced_widget::text_editor::Content::with_text()` 在底层 `cosmic_text::Buffer::set_text` 尚未设置视口高度时就无界 `shape_until_scroll`(把全文档一次性做字体 shaping)。改用"先建空 buffer、真内容走 `Action::Edit(Edit::Paste(..))` 插入"绕开这条无界路径(`insert_at` 只切行、不 shaping,真正的 shaping 在下一次 `update()` 时已经是有界窗口)。在此基础上按机器内存动态分三档(编辑/只读整读/只读分块),打开流程异步化,只读大文件搜索复用已有的 `grep-searcher` 磁盘流式扫描。

**Tech Stack:** Rust, iced 0.14(`iced_widget::text_editor`/`cosmic-text` 0.15 内部),`sysinfo` 0.32(新增,查总内存),`tokio`(已有 workspace 依赖,复用 `ShellIo::handle`/`EventLoopProxy` 既有异步模式),`grep-searcher`/`grep-regex`(已有,复用 `extensions/search.rs::search_scope`)。

**Spec:** `docs/superpowers/specs/2026-09-19-large-file-editor-performance-design.md`

## Global Constraints

- 编辑档上限固定 `EDIT_MODE_MAX_BYTES = 20 * 1024 * 1024`(20MB),不随机器内存缩放。
- 只读整读档上限 `full_load_max_bytes() = clamp(total_ram_bytes × 0.10 / 3, 256MB, 4GB)`,总内存查询失败时退化为 512MB 默认值,不 panic。
- 不引入 `memmap2`/`ropey`;不做真正的无限大文件支持,超出整读上限一律走"只读+分块"。
- 只读档(整读/分块两种)一律 `read_only: true`:禁止编辑、不分配 undo 快照栈、隐藏保存入口。
- 分块边界必须按合法 UTF-8 字符边界对齐,不产生非法 UTF-8 或丢字符。
- 在独立分支(如 `feature/large-file-editor-perf`)上开发,不直接提交到 main;全部任务完成、自测通过后提请审阅,审阅通过再合并回 main。
- 构建/检查命令:`cargo build`(全 workspace)、`cargo test -p dozer-app`、`cargo clippy --all-targets && cargo fmt`。
- 只有 code editor 和 pty 终端用 JetBrains Mono;本计划新增的 UI(只读 chip、加载更多横幅)一律用系统默认字体(`Font::default()`/`byteui::theme::font::*`),不上等宽代码字体。
- 中文/非 ASCII 文本渲染必须走 `Shaping::Advanced`,禁止 `Shaping::Basic`(已有约定,本计划不改变这一点,只影响 `set_text`/`insert_at` 的调用点)。

---

## Task 1: 根因修复 —— CodeView 统一走空 buffer + `Action::Edit(Paste)` 注入,绕开无界 shaping

**背景(已验证,非推测)**:读 `iced_graphics-0.14.0/src/text/editor.rs` 源码确认 `Editor::with_text` 用 `cosmic_text::Buffer::new_empty` 建空 buffer(无 `set_size`)后直接 `buffer.set_text(...)`;`cosmic-text-0.15.0/src/buffer.rs:441` 的 `shape_until_scroll` 里 `scroll_end = scroll_start + self.height_opt.unwrap_or(f32::INFINITY)`——`height_opt` 此刻是 `None`,窗口退化成 `[0, ∞)`,一次性 shape 全文档。反之,`iced_widget::text_editor::Action::Edit(Edit::Paste(Arc<String>))` 走 `cosmic_text::Editor::insert_string` → `insert_at`(`cosmic-text-0.15.0/src/edit/editor.rs:368`),这条路径只做 `BufferLine::new(..., Shaping::Advanced)` 切行(存 shaping *模式*,不做 shaping *计算*),完全不碰 `shape_until_scroll`。真正的 shaping 由 widget 每帧 `layout()` 触发的 `Editor::update()`(`iced_graphics-0.14.0/src/text/editor.rs:546`)负责,它总是先 `buffer.set_size(font_system, Some(w), Some(h))` 再 `shape_as_needed`——一旦 buffer 曾经历过一次 `with_text("")` → `update()`,后续任何 `insert_string` 都在"已知视口"下工作,shaping 天然有界。

结论:把 `code_editor/mod.rs` 里全部 4 处 `text_editor::Content::with_text(大文本)` 调用点(`CodeView::new`/`restore`/`replace_all`/`replace_nth`)统一改成"空 Content + `perform(Action::Edit(Edit::Paste(..)))`",无需任何"等下一帧"的延迟机制——`insert_at` 本身就不贵,贵的只是 shaping,而 shaping 已经交给后续天然有界的 `update()`。

**Files:**
- Modify: `crates/dozer-app/src/code_editor/mod.rs`(顶部 import、`CodeView::new`、`restore`、`replace_all`、`replace_nth`)
- Modify: `crates/dozer-app/src/code_editor/highlighter.rs:228-286`(删除硬编码本机路径的 `mod repro`)
- Test: `crates/dozer-app/src/code_editor/mod.rs` 内 `#[cfg(test)] mod tests`(新增)

**Interfaces:**
- Produces: `fn content_from_text(text: &str) -> iced_widget::text_editor::Content`(私有辅助,供本文件内 4 处调用点复用;后续任务不依赖它,但了解其存在有助于理解 `CodeView::new` 行为不变)。
- `CodeView::new`/`text()`/`line_count()` 等既有公开签名不变,行为不变(只是不再有无界 shaping 的隐患)。

- [ ] **Step 1: 写一个先失败的回归测试,证明当前实现在大文档上耗时会随行数近乎线性增长**

在 `crates/dozer-app/src/code_editor/mod.rs` 文件末尾已有的 `#[cfg(test)] mod tests { ... }` 块内追加(注意:此时 `CodeView::new` 尚未修复,这个测试预期会因为耗时过长而失败/超时——先确认失败,再进 Step 3 去修复):

```rust
    #[test]
    fn opening_large_document_does_not_scale_with_line_count() {
        // 20 万行、每行约 40 字节 ≈ 8MB——修复前(无界 shape,~13ms/KB)预期
        // 耗时数十秒到超百秒;修复后(shaping 交给有界的 update())应在
        // 数百毫秒内完成。用一个远低于"修复前实测值"、但远高于"修复后预期值"
        // 的绝对上限,不用比例基准——两者数量级差异(百毫秒 vs 数十秒)大到
        // 任何正常 CI 机器都不会因为慢而误判,比拿两个耗时做比值更不易抖动。
        let big_text: String = (0..200_000)
            .map(|i| format!("let line_{i:06} = {i}; // filler filler filler\n"))
            .collect();
        let start = std::time::Instant::now();
        let view = CodeView::new(&big_text, "rust", false);
        let elapsed = start.elapsed();
        assert_eq!(view.content.line_count(), 200_001, "200000 行 + 尾随空行");
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "构造 ~8MB/20万行文档耗时 {elapsed:?},应在数百毫秒量级(无界 shaping \
             会退化到数十秒);见根因修复设计"
        );
    }
```

- [ ] **Step 2: 跑测试确认当前实现确实慢(或直接跳过计时断言、只跑一次人工计时确认)**

Run: `cargo test -p dozer-app --lib code_editor::tests::opening_large_document_does_not_scale_with_line_count -- --nocapture`
Expected: 测试运行很久(可能超过默认 test harness 无超时限制但肉眼可见明显卡顿),或断言失败报出实际 `elapsed`(如"耗时 45.2s"),证明修复前的问题真实存在。若本地机器较快导致 8MB 仍在 5s 内通过,把行数改到 100 万行(≈40MB)重跑,确认能观察到二次增长后的明显延迟,再继续。

- [ ] **Step 3: 实现根因修复——4 处 `Content::with_text(大文本)` 改为空 Content + `Action::Edit(Paste)`**

在 `crates/dozer-app/src/code_editor/mod.rs` 顶部 import 区(现状第 39-42 行)追加 `Arc`:

```rust
use iced_widget::canvas;
use iced_widget::core::widget::Id as WidgetId;
use iced_widget::core::{Color, Element, Length, Pixels, Point, Rectangle, Size, mouse};
use iced_widget::text_editor::{self, Action, Edit};
use std::sync::Arc;
```

在 `Snapshot`/`EDIT_HISTORY_LIMIT` 定义之后、`CodeView` struct 定义之前新增私有辅助函数:

```rust
/// 建一个已装载 `text` 的 `Content`,但**不**直接 `Content::with_text(text)`——
/// 那条路径底层 `cosmic_text::Buffer::set_text` 在 buffer 尚无视口尺寸时会
/// 无界 `shape_until_scroll`(整份文档一次性做字体 shaping,大文件表现为
/// UI 线程卡死数秒到数十秒/分钟,已通过读 iced_graphics/cosmic-text 源码
/// 验证)。改用"空 Content 起手 + `Action::Edit(Edit::Paste(..))` 插入":
/// `insert_string`/`insert_at`(cosmic-text `edit/editor.rs`)只切行、把每行
/// 存成待 shape 的 `BufferLine`,不做 shaping 计算;真正的 shaping 交给 widget
/// 每帧 `layout()` 触发的 `Editor::update()`,它总是先 `buffer.set_size(..)`
/// 再 `shape_as_needed`——一旦视口尺寸已知,shaping 天然只处理可见窗口。
/// 空文本直接返回空 `Content`,不必走一次空字符串的 `Paste`。
fn content_from_text(text: &str) -> text_editor::Content {
    let mut content = text_editor::Content::with_text("");
    if !text.is_empty() {
        content.perform(Action::Edit(Edit::Paste(Arc::new(text.to_string()))));
    }
    content
}
```

把 `CodeView::new`(现状第 109-120 行)里的构造改成调用它:

```rust
    pub fn new(text: &str, token: impl Into<String>, read_only: bool) -> Self {
        Self {
            id: WidgetId::unique(),
            content: content_from_text(text),
            scroll_lines: 0.0,
            read_only,
            token: token.into(),
            undo: Vec::new(),
            redo: Vec::new(),
            typing_run: false,
        }
    }
```

`restore`(现状第 391-399 行)同样改用 `content_from_text`:

```rust
    fn restore(&mut self, snap: Snapshot) {
        let last_line = self.content.line_count();
        let line = snap.line.min(last_line.saturating_sub(1));
        self.content = content_from_text(&snap.text);
        self.scroll_lines = self
            .scroll_lines
            .clamp(0.0, self.content.line_count().saturating_sub(1) as f32);
        self.move_cursor_to((line, snap.column));
    }
```

`replace_all`(现状第 415-424 行):

```rust
    pub fn replace_all(&mut self, query: &str, case_sensitive: bool, replacement: &str) -> usize {
        let (new_text, count) =
            replace_pass_all(&self.content.text(), query, case_sensitive, replacement);
        if count == 0 {
            return 0;
        }
        self.content = content_from_text(&new_text);
        self.reset_edit_history();
        count
    }
```

`replace_nth`(现状第 431-446 行):

```rust
    pub fn replace_nth(
        &mut self,
        nth: usize,
        query: &str,
        case_sensitive: bool,
        replacement: &str,
    ) -> bool {
        let text = self.content.text();
        let Some(new_text) = replace_pass_nth(&text, nth, query, case_sensitive, replacement)
        else {
            return false;
        };
        self.content = content_from_text(&new_text);
        self.reset_edit_history();
        true
    }
```

- [ ] **Step 4: 跑回归测试确认通过**

Run: `cargo test -p dozer-app --lib code_editor::tests::opening_large_document_does_not_scale_with_line_count -- --nocapture`
Expected: PASS,`elapsed` 打印/断言均在数百毫秒量级。

- [ ] **Step 5: 跑现有 `code_editor` 全部测试确认未破坏既有行为(尤其 undo/redo/replace/find 相关用例)**

Run: `cargo test -p dozer-app --lib code_editor::`
Expected: 全部 PASS(既有测试覆盖 `read_only_filters_edit_but_allows_move_and_scroll`、undo/redo、find/replace 等,`content_from_text` 是纯内部实现替换,行为不变)。

- [ ] **Step 6: 删除 `highlighter.rs` 里硬编码本机路径的旧 repro 测试**

`crates/dozer-app/src/code_editor/highlighter.rs:228-286` 整个 `#[cfg(test)] mod repro { ... }` 块删除(3 个测试 `repro_large_json5`/`repro_content_with_text`/`repro_full_render` 全部依赖 `/Users/chrischiang/Projects/WorkProjects/...` 这个只在原作者机器上存在的路径,在任何其他环境/CI 上都会因 `unwrap()` 在缺失文件上 panic 而失败;本任务 Step 1-2 已经用运行时生成的可移植 fixture 取代了它们的验证目的)。

- [ ] **Step 7: 跑 `code_editor` 全量测试 + clippy + fmt 确认整体绿**

Run: `cargo test -p dozer-app --lib code_editor:: && cargo clippy -p dozer-app --all-targets && cargo fmt --check`
Expected: 全部 PASS/无警告(`cargo fmt --check` 若有差异用 `cargo fmt` 直接改)。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/code_editor/mod.rs crates/dozer-app/src/code_editor/highlighter.rs
git commit -m "fix(code_editor): 绕开 cosmic-text 无界 shaping,大文档打开耗时与行数解耦"
```

---

## Task 2: 三档阈值分类 + 只读路由 + 截断初读(整读上限内)+ 只读 UI 提示

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`(新增 `sysinfo` 依赖)
- Modify: `crates/dozer-app/src/preview/native_editor.rs`(阈值常量、分档、UTF-8 边界对齐、`read_and_build_native_editor` 改造)
- Modify: `crates/dozer-app/src/preview/state.rs`(`PreviewTab` 新增字段)
- Modify: `crates/dozer-app/src/preview/view.rs`(`push_tab` 接线新返回类型)
- Modify: `crates/dozer-app/src/code_editor/mod.rs`(新增 `is_read_only` 访问器)
- Modify: `crates/dozer-app/src/workspace/view.rs`(只读 chip UI,插入点 `preview_pane_for` 内 `container(editor.view()...)` 之前)
- Test: 上述 `native_editor.rs`/`state.rs` 内联单测

**Interfaces:**
- Consumes: Task 1 的 `CodeView::new(text, token, read_only)`(签名不变)。
- Produces:
  - `pub(crate) const EDIT_MODE_MAX_BYTES: u64`
  - `pub(crate) fn full_load_max_bytes_for(total_ram_bytes: u64) -> u64`(纯函数,供测试与 `full_load_max_bytes` 复用)
  - `pub(crate) fn full_load_max_bytes() -> u64`
  - `pub(crate) enum SizeTier { Edit, FullLoadReadOnly, ChunkedReadOnly }`
  - `pub(crate) fn classify_size(len: u64, full_load_max: u64) -> SizeTier`
  - `pub(crate) fn utf8_safe_prefix_len(bytes: &[u8], max_len: usize) -> usize`(供 Task 4 复用)
  - `pub(crate) struct NativeFileData { pub text: String, pub syntax_token: String, pub read_only: bool, pub loaded_bytes: u64, pub total_bytes: u64, pub truncated: bool }`(`#[derive(Debug, Clone)]`——纯数据,不含 `CodeView`,专给 Task 3 需要跨 `Message`/线程边界传递的场景用;`CodeView` 没有实现 `Clone`,而 `Message` enum 整体 `#[derive(Debug, Clone)]`,任何 `Message` 变体都不能直接携带 `CodeView`)
  - `pub(crate) fn read_native_file_data(path: &Path) -> std::io::Result<NativeFileData>`(真正的读盘 + 分档逻辑)
  - `pub(crate) struct NativeEditorLoad { pub view: crate::code_editor::CodeView, pub loaded_bytes: u64, pub total_bytes: u64, pub truncated: bool }`(薄封装,只给本任务内*同步*调用路径——如 `push_tab`——使用,不进 `Message`)
  - `pub(crate) fn read_and_build_native_editor(path: &Path) -> std::io::Result<NativeEditorLoad>`(基于 `read_native_file_data` 现场构造 `CodeView`,替换原返回裸 `CodeView` 的签名)
  - `CodeView::is_read_only(&self) -> bool`
  - `PreviewTab` 新字段:`pub loaded_bytes: u64`, `pub total_bytes: u64`, `pub truncated: bool`

- [ ] **Step 1: 加 `sysinfo` 依赖**

在 `crates/dozer-app/Cargo.toml` 依赖列表里追加(该 crate 已是 workspace 内某处的传递依赖,`Cargo.lock` 已锁定 `0.32.1`,这里改成直接依赖不会触发新的版本解析):

```toml
sysinfo = "0.32"
```

Run: `cargo build -p dozer-app 2>&1 | tail -20`
Expected: 编译通过(仅新增依赖声明,尚无代码使用)。

- [ ] **Step 2: 写分档函数的失败测试**

在 `crates/dozer-app/src/preview/native_editor.rs` 末尾新增 `#[cfg(test)] mod tests`(若文件已有测试模块则追加到其中,当前文件没有测试模块,新建):

```rust
#[cfg(test)]
mod size_tier_tests {
    use super::*;

    #[test]
    fn full_load_max_clamps_to_floor_on_small_machines() {
        // 4GB 机器:4×0.10/3 ≈ 137MB,应钳到 256MB 下限。
        let max = full_load_max_bytes_for(4 * 1024 * 1024 * 1024);
        assert_eq!(max, 256 * 1024 * 1024);
    }

    #[test]
    fn full_load_max_clamps_to_ceiling_on_huge_machines() {
        // 128GB 机器:128×0.10/3 ≈ 4.27GB,应钳到 4GB 上限。
        let max = full_load_max_bytes_for(128 * 1024 * 1024 * 1024);
        assert_eq!(max, 4 * 1024 * 1024 * 1024);
    }

    #[test]
    fn full_load_max_scales_between_clamps() {
        // 32GB 机器:32×0.10/3 ≈ 1.0667GB,应落在钳位区间内、非两端。
        let max = full_load_max_bytes_for(32 * 1024 * 1024 * 1024);
        assert!(max > 256 * 1024 * 1024 && max < 4 * 1024 * 1024 * 1024);
        // 32×1024×1024×1024×0.10/3 = 1_146_617_856(整数截断)。
        assert_eq!(max, 1_146_617_856);
    }

    #[test]
    fn classify_size_boundaries() {
        let full_max = 1_000_000_000u64; // 假设一个整读上限,便于断言边界
        assert!(matches!(classify_size(0, full_max), SizeTier::Edit));
        assert!(matches!(
            classify_size(EDIT_MODE_MAX_BYTES - 1, full_max),
            SizeTier::Edit
        ));
        assert!(matches!(
            classify_size(EDIT_MODE_MAX_BYTES, full_max),
            SizeTier::FullLoadReadOnly
        ));
        assert!(matches!(
            classify_size(full_max - 1, full_max),
            SizeTier::FullLoadReadOnly
        ));
        assert!(matches!(
            classify_size(full_max, full_max),
            SizeTier::ChunkedReadOnly
        ));
    }

    #[test]
    fn utf8_safe_prefix_len_trims_incomplete_multibyte_tail() {
        // "中" 是 3 字节 UTF-8(E4 B8 AD)。截在第 1、2 字节处都应回退到
        // 该字符起点之前;截在第 3 字节(字符完整)处应保留整个字符。
        let text = "ab中cd"; // a b [E4 B8 AD] c d
        let bytes = text.as_bytes();
        assert_eq!(utf8_safe_prefix_len(bytes, 2), 2); // "ab",未触及多字节字符
        assert_eq!(utf8_safe_prefix_len(bytes, 3), 2); // 截在"中"第1字节,回退
        assert_eq!(utf8_safe_prefix_len(bytes, 4), 2); // 截在"中"第2字节,回退
        assert_eq!(utf8_safe_prefix_len(bytes, 5), 5); // 截在"中"第3字节(完整),保留
        assert_eq!(utf8_safe_prefix_len(bytes, 100), bytes.len()); // 超过总长,钳到总长
    }
}
```

- [ ] **Step 3: 跑测试确认全部因符号不存在而编译失败**

Run: `cargo test -p dozer-app --lib preview::native_editor::size_tier_tests`
Expected: 编译错误(`full_load_max_bytes_for`/`classify_size`/`EDIT_MODE_MAX_BYTES`/`SizeTier`/`utf8_safe_prefix_len` 均未定义)。

- [ ] **Step 4: 实现分档常量、纯函数与 UTF-8 边界对齐辅助**

在 `crates/dozer-app/src/preview/native_editor.rs` 里,把现状第 4-18 行的 `MAX_NATIVE_EDITOR_BYTES`/`exceeds_native_editor_limit`(旧的单一 256KB 阈值,被本任务的三档取代)整段替换成:

```rust
/// 低于此值:全功能编辑(含 undo/保存)。固定值,不随机器内存缩放——这一档
/// 的瓶颈是 undo 栈本身的设计(`code_editor::EDIT_HISTORY_LIMIT` 份整文件
/// `String` 快照),不是单次读取的内存代价。Task 1 的根因修复让这个量级的
/// 打开耗时已经和行数解耦,不必再像旧阈值(256KB)那样保守。
pub(crate) const EDIT_MODE_MAX_BYTES: u64 = 20 * 1024 * 1024;

/// 按机器可用内存动态算"只读·整读"档上限(纯函数,供 [`full_load_max_bytes`]
/// 与单测复用)。公式见
/// `docs/superpowers/specs/2026-09-19-large-file-editor-performance-design.md`
/// "分档策略与阈值"一节:总内存 10% ÷ 3(读取+校验临时拷贝+常驻拷贝的峰值
/// 安全边际),钳到 [256MB, 4GB]。
pub(crate) fn full_load_max_bytes_for(total_ram_bytes: u64) -> u64 {
    ((total_ram_bytes as f64 * 0.10 / 3.0) as u64).clamp(256 * 1024 * 1024, 4 * 1024 * 1024 * 1024)
}

/// 查询系统总内存并套 [`full_load_max_bytes_for`]。查询失败(极端环境)时
/// 退化为 512MB 默认值,不 panic、不阻塞打开流程。
pub(crate) fn full_load_max_bytes() -> u64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let total = sys.total_memory();
    if total == 0 {
        return 512 * 1024 * 1024;
    }
    full_load_max_bytes_for(total)
}

/// 三档分类结果。
pub(crate) enum SizeTier {
    /// < `EDIT_MODE_MAX_BYTES`:全功能编辑。
    Edit,
    /// [`EDIT_MODE_MAX_BYTES`, full_load_max):只读,整文件读入内存。
    FullLoadReadOnly,
    /// >= full_load_max:只读,首屏只读入 full_load_max 字节,可"加载更多"。
    ChunkedReadOnly,
}

/// 按文件大小(`len`)与本次打开时算好的整读上限(`full_load_max`,调用方
/// 传入而非在这里现查,避免每次分类都重复查一次系统内存)分类。
pub(crate) fn classify_size(len: u64, full_load_max: u64) -> SizeTier {
    if len < EDIT_MODE_MAX_BYTES {
        SizeTier::Edit
    } else if len < full_load_max {
        SizeTier::FullLoadReadOnly
    } else {
        SizeTier::ChunkedReadOnly
    }
}

/// 在 `bytes` 里找不超过 `max_len` 的最大合法 UTF-8 前缀长度——分块读取截断
/// 点若落在多字节字符中间,回退到该字符起点之前,避免产生非法 UTF-8 或
/// 半个字符。`max_len` 超过 `bytes.len()` 时钳到 `bytes.len()`。
pub(crate) fn utf8_safe_prefix_len(bytes: &[u8], max_len: usize) -> usize {
    let max_len = max_len.min(bytes.len());
    match std::str::from_utf8(&bytes[..max_len]) {
        Ok(_) => max_len,
        Err(e) => e.valid_up_to(),
    }
}
```

- [ ] **Step 5: 跑测试确认 Step 2 的用例全部通过**

Run: `cargo test -p dozer-app --lib preview::native_editor::size_tier_tests`
Expected: PASS。

- [ ] **Step 6: 改造 `read_and_build_native_editor` 按分档路由,返回新的 `NativeEditorLoad`**

`native_editor.rs` 里原 `read_and_build_native_editor`(现状第 35-57 行,依赖刚删除的 `exceeds_native_editor_limit`)整段替换:

```rust
/// 读盘 + 三档分类的纯数据结果(不含 `CodeView`)。`#[derive(Debug, Clone)]`
/// ——专为跨 `Message`/线程边界传递设计(`CodeView` 没有实现 `Clone`,而
/// `Message` enum 整体 `#[derive(Debug, Clone)]`,见 Task 3):异步读盘任务
/// 只做 I/O 与分类,`CodeView::new`(涉及全局 `font_system` 锁、必须在
/// 「拥有窗口/事件循环」的上下文里做)留给收到结果的一端(`PreviewPane::
/// apply_native_load`,Task 3)现场构造。
#[derive(Debug, Clone)]
pub(crate) struct NativeFileData {
    pub text: String,
    pub syntax_token: String,
    pub read_only: bool,
    pub loaded_bytes: u64,
    pub total_bytes: u64,
    pub truncated: bool,
}

/// 读盘并按三档策略产出 [`NativeFileData`]:
/// - `SizeTier::Edit`(< 20MB):全量读入,可写(`read_only=false`),语义同
///   2026-09-06 起的"原生预览默认可编辑"。
/// - `SizeTier::FullLoadReadOnly`:全量读入,但 `read_only=true`。
/// - `SizeTier::ChunkedReadOnly`:只读入前 `full_load_max` 字节(按合法 UTF-8
///   边界截断),`read_only=true`,`truncated=true`。
///
/// 内容不是合法 UTF-8 时降级用 lossy 转换(不当错误);其余读取失败(不存在/
/// 权限不够等)原样透传 `std::io::Error`。
pub(crate) fn read_native_file_data(path: &std::path::Path) -> std::io::Result<NativeFileData> {
    let total_bytes = std::fs::metadata(path)?.len();
    let full_load_max = full_load_max_bytes();
    let tier = classify_size(total_bytes, full_load_max);

    let (raw, loaded_bytes, truncated) = match tier {
        SizeTier::Edit | SizeTier::FullLoadReadOnly => {
            let bytes = std::fs::read(path)?;
            let len = bytes.len() as u64;
            (bytes, len, false)
        }
        SizeTier::ChunkedReadOnly => {
            use std::io::Read;
            let mut file = std::fs::File::open(path)?;
            let cap = full_load_max as usize;
            let mut buf = vec![0u8; cap];
            let mut read_total = 0usize;
            while read_total < cap {
                let n = file.read(&mut buf[read_total..])?;
                if n == 0 {
                    break;
                }
                read_total += n;
            }
            buf.truncate(read_total);
            let safe_len = utf8_safe_prefix_len(&buf, buf.len());
            buf.truncate(safe_len);
            let len = buf.len() as u64;
            (buf, len, true)
        }
    };

    // `FromUtf8Error::as_bytes` 返回 `&[u8]`,`String::from_utf8_lossy` 接的
    // 正是这个签名,不需要额外 clone。
    let text = String::from_utf8(raw).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    let read_only = !matches!(tier, SizeTier::Edit);
    Ok(NativeFileData {
        text,
        syntax_token: extension_to_syntax(path),
        read_only,
        loaded_bytes,
        total_bytes,
        truncated,
    })
}

/// 单次打开(同步路径,`push_tab` 用)读到的载荷:在 [`read_native_file_data`]
/// 基础上现场构造好 `CodeView`。**不要**把这个类型放进任何 `Message` 变体——
/// `CodeView` 没有 `Clone`,`Message` 整体要求 `#[derive(Clone)]`(Task 3 的
/// 异步路径直接用 `NativeFileData`,不用这个类型跨边界)。
pub(crate) struct NativeEditorLoad {
    pub view: crate::code_editor::CodeView,
    pub loaded_bytes: u64,
    pub total_bytes: u64,
    pub truncated: bool,
}

pub(crate) fn read_and_build_native_editor(
    path: &std::path::Path,
) -> std::io::Result<NativeEditorLoad> {
    let data = read_native_file_data(path)?;
    let view = crate::code_editor::CodeView::new(&data.text, data.syntax_token, data.read_only);
    Ok(NativeEditorLoad {
        view,
        loaded_bytes: data.loaded_bytes,
        total_bytes: data.total_bytes,
        truncated: data.truncated,
    })
}
```

- [ ] **Step 7: 接线 `push_tab`,把返回类型从裸 `CodeView` 改成 `NativeEditorLoad` 并拆开填 `PreviewTab`**

`crates/dozer-app/src/preview/state.rs` 的 `PreviewTab` struct(现状第 8-30 行)新增三个字段,紧跟在 `dirty` 之后:

```rust
    pub dirty: bool,
    /// 只读大文件档(`FullLoadReadOnly`/`ChunkedReadOnly`)已载入的字节数;
    /// 编辑档/非文件 tab 恒为 0(无意义,不展示)。
    pub loaded_bytes: u64,
    /// 打开时 `fs::metadata` 测到的文件总字节数;语义同上,非只读大文件 tab
    /// 恒为 0。
    pub total_bytes: u64,
    /// 是否被截断(`ChunkedReadOnly` 档为真;其余恒假)。UI 据此渲染"仅加载
    /// 前 X MB"横幅(Task 4 补"加载更多"交互,本任务先只展示静态横幅)。
    pub truncated: bool,
```

`Debug` 手写实现(现状第 32-44 行)补上三个字段:

```rust
impl std::fmt::Debug for PreviewTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewTab")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("title", &self.title)
            .field("reload_nonce", &self.reload_nonce)
            .field("dirty", &self.dirty)
            .field("loaded_bytes", &self.loaded_bytes)
            .field("total_bytes", &self.total_bytes)
            .field("truncated", &self.truncated)
            .field("editor", &self.editor.is_some())
            .field("tabular", &self.tabular.is_some())
            .finish()
    }
}
```

`crates/dozer-app/src/preview/view.rs` 的 `push_tab`(现状第 172-212 行)与 `placeholder_tab`(未列出,占位 tab 构造函数,搜索 `fn placeholder_tab` 定位)都要补上新字段默认值。`push_tab` 里原来的:

```rust
            TabKind::File(path)
                if is_editable_extension(path) && !prefers_rendered_preview(path) =>
            {
                read_and_build_native_editor(path).ok()
            }
```

这一支现在返回 `Option<NativeEditorLoad>` 而不是 `Option<CodeView>`,需要拆开。把 `editor` 变量的构造改成:

```rust
        let native_load = match &kind {
            TabKind::File(path) if crate::tabular::is_tabular_extension(path) => None,
            TabKind::File(path)
                if is_editable_extension(path) && !prefers_rendered_preview(path) =>
            {
                read_and_build_native_editor(path).ok()
            }
            _ => None,
        };
        let (editor, loaded_bytes, total_bytes, truncated) = match native_load {
            Some(load) => (Some(load.view), load.loaded_bytes, load.total_bytes, load.truncated),
            None => (None, 0, 0, false),
        };
```

并删掉原来重复的 `let editor = match &kind { ... }` 块(两个匹配合并成上面这一个,避免打开文件两次)。`self.tabs.push(PreviewTab { ... })` 里补上新字段:

```rust
        self.tabs.push(PreviewTab {
            id,
            kind,
            title,
            reload_nonce: 0,
            editor,
            tabular,
            dirty: false,
            loaded_bytes,
            total_bytes,
            truncated,
        });
```

`placeholder_tab` 函数体内的 `PreviewTab { .. }` 构造同样补 `loaded_bytes: 0, total_bytes: 0, truncated: false`。

- [ ] **Step 8: 加 `CodeView::is_read_only` 访问器**

在 `crates/dozer-app/src/code_editor/mod.rs` 的 `impl CodeView` 块里,紧跟 `pub fn text(&self)`(现状第 122-125 行)之后新增:

```rust
    /// 只读态(见 `preview::native_editor::SizeTier`:整读/分块两档只读文件
    /// 走 `read_only=true` 构造)——UI 据此渲染"只读"提示 chip、隐藏保存
    /// 入口;`perform`/`record_before` 已经用同一个字段过滤编辑动作与跳过
    /// undo 记录(见模块文档"用途演进"),这里只是补一个公开只读访问器。
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }
```

- [ ] **Step 9: 跑 workspace 全量编译,修掉因签名变化产生的其它调用点错误**

Run: `cargo build -p dozer-app 2>&1 | grep -E "^error" | head -50`
Expected: 若有残留调用点仍假设 `read_and_build_native_editor` 返回裸 `CodeView`(例如 `bump_reload` 里"刷新原生编辑器"的路径,搜索 `read_and_build_native_editor` 的全部调用者:`grep -rn "read_and_build_native_editor" crates/dozer-app/src/`),按 Step 7 同样的拆解方式改掉,直到编译通过。

- [ ] **Step 10: 只读 UI chip——`workspace/view.rs` 里编辑器上方渲染一行提示**

`crates/dozer-app/src/workspace/view.rs` 里 `if let Some(editor) = &active_tab.editor { ... }` 块(现状第 821 行起),在 `content = content.push(container(editor.view()...))`(现状第 1185-1189 行)之前插入:

```rust
            if editor.is_read_only() {
                let colors = byteui::theme::color::current();
                let size_label = if active_tab.truncated {
                    format!(
                        "只读 · 文件过大 · 仅加载前 {:.1}MB,共 {:.1}MB",
                        active_tab.loaded_bytes as f64 / (1024.0 * 1024.0),
                        active_tab.total_bytes as f64 / (1024.0 * 1024.0)
                    )
                } else {
                    format!(
                        "只读 · 文件过大({:.1}MB)",
                        active_tab.total_bytes as f64 / (1024.0 * 1024.0)
                    )
                };
                content = content.push(
                    container(
                        text(size_label)
                            .size(byteui::theme::font::body())
                            .color(colors.dim),
                    )
                    .padding([4, 8]),
                );
            }
            content = content.push(
                container(editor.view().map(move |ev| editor_msg(tab_id, ev)))
                    .width(Length::Fill)
                    .height(Length::Fill),
            );
```

- [ ] **Step 11: 跑全量测试 + clippy + fmt**

Run: `cargo test -p dozer-app --lib preview:: code_editor:: workspace:: 2>&1 | tail -60 && cargo clippy -p dozer-app --all-targets 2>&1 | tail -40 && cargo fmt --check`
Expected: 全部 PASS/无警告。

- [ ] **Step 12: Commit**

```bash
git add crates/dozer-app/Cargo.toml crates/dozer-app/src/preview/native_editor.rs crates/dozer-app/src/preview/state.rs crates/dozer-app/src/preview/view.rs crates/dozer-app/src/code_editor/mod.rs crates/dozer-app/src/workspace/view.rs
git commit -m "feat(preview): 按内存动态三档分类打开路径,大文件只读+截断初读"
```

---

## Task 3: 打开流程异步化(Files + Project 双预览面板)

**Files:**
- Modify: `crates/dozer-app/src/app/message.rs`(新增 `PreviewFileLoaded`/`ProjectPreviewFileLoaded` 消息)
- Modify: `crates/dozer-app/src/app/update.rs`(`preview_open_path`/`project_preview_open_path` 改异步派发 + 新增两个结果处理函数)
- Modify: `crates/dozer-app/src/preview/view.rs`(`push_tab` 拆成"先插入占位 tab"+"异步结果回填"两步,新增 `insert_loading_tab`/`apply_native_load` 方法)
- Modify: `crates/dozer-app/src/preview/state.rs`(`PreviewTab.editor` 为 `None` 但仍是"原生候选"时需要一个 `loading: bool` 标记,区分"这个 tab 天生没有原生编辑器"与"原生编辑器正在异步加载中")
- Modify: `crates/dozer-app/src/workspace/view.rs`(loading 态渲染:tab 内容区显示"加载中…")
- Test: `preview/view.rs` 内联单测

**Interfaces:**
- Consumes: Task 2 的 `native_editor::read_native_file_data(path) -> io::Result<NativeFileData>`(异步边界专用,纯数据、`Clone`)、`PreviewTab` 的 `loaded_bytes`/`total_bytes`/`truncated` 字段。**不用** Task 2 的 `NativeEditorLoad`(内含不可 `Clone` 的 `CodeView`,不能进 `Message`)。
- Produces:
  - `PreviewTab.loading: bool` 新字段。
  - `PreviewPane::insert_loading_tab(&mut self, kind: TabKind, title: String) -> usize`(替代原 `push_tab` 对文件 tab 的同步构造部分,立即插入一个 `editor: None, loading: true` 的占位 tab,返回 `id`)。
  - `PreviewPane::apply_native_load(&mut self, tab_id: usize, result: Result<native_editor::NativeFileData, String>)`(异步结果回灌:成功时在这里现场 `CodeView::new(&data.text, data.syntax_token, data.read_only)`,tab 已被用户关闭时 no-op)。
  - `Message::PreviewFileLoaded(ProjectId, usize, Result<native_editor::NativeFileData, String>)` / `Message::ProjectPreviewFileLoaded(ProjectId, usize, Result<native_editor::NativeFileData, String>)`(`NativeFileData` 是纯数据、`#[derive(Debug, Clone)]`,满足 `Message` 整体的 `#[derive(Debug, Clone)]` 要求;Files/Project 两面板仍分开两条消息而不是共用 `PanelKind` 参数,对齐 `ProjectPreviewOpenPath`/`PreviewOpenPath` 本就是分开两条消息的既有先例)。

- [ ] **Step 1: `PreviewTab` 加 `loading` 字段**

`crates/dozer-app/src/preview/state.rs` 的 `PreviewTab` struct,紧跟 Task 2 加的 `truncated` 字段之后:

```rust
    pub truncated: bool,
    /// 原生编辑器正在异步读盘中(`PreviewPane::insert_loading_tab` 置真,
    /// `apply_native_load` 收到结果后置假)。为真时 `editor`/`tabular` 均
    /// `None`,但这个 tab **不**应该被当成"该文件没有原生编辑器"误判进
    /// webview 池——`desired_webviews()`/`active_webview_id()` 等判据要
    /// 额外排除 `loading` 为真的 tab(见本任务 Step 4)。
    pub loading: bool,
```

`Debug` 实现补一行 `.field("loading", &self.loading)`;`placeholder_tab` 补 `loading: false`。

- [ ] **Step 2: 写"loading tab 不进 webview 池"的失败测试**

在 `crates/dozer-app/src/preview/view.rs` 现有 `#[cfg(test)] mod tests`(搜索 `mod tests` 定位,该文件已有大量测试,如 `open_path_builds_native_editor_for_whitelisted_extension_only` 等)里新增:

```rust
    #[test]
    fn loading_tab_is_excluded_from_webview_pool() {
        let mut p = PreviewPane::default();
        let id = p.insert_loading_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/huge.log")),
            "huge.log".into(),
        );
        assert!(p.tabs().iter().any(|t| t.id == id && t.loading));
        assert!(
            p.desired_webviews().is_empty(),
            "loading 中的原生候选 tab 不该被当成 webview 文件插进期望清单"
        );
        assert_eq!(
            p.active_webview_id(),
            None,
            "loading 中的 tab 不是激活 webview"
        );
    }

    #[test]
    fn apply_native_load_fills_editor_and_clears_loading() {
        let mut p = PreviewPane::default();
        let id = p.insert_loading_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/small.rs")),
            "small.rs".into(),
        );
        let data = crate::preview::native_editor::NativeFileData {
            text: "fn main() {}".into(),
            syntax_token: "rust".into(),
            read_only: false,
            loaded_bytes: 12,
            total_bytes: 12,
            truncated: false,
        };
        p.apply_native_load(id, Ok(data));
        let tab = p.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(!tab.loading);
        assert!(tab.editor.is_some());
        assert_eq!(tab.total_bytes, 12);
    }

    #[test]
    fn apply_native_load_ignores_closed_tab() {
        // 结果回来前 tab 已被关掉:no-op,不 panic。
        let mut p = PreviewPane::default();
        let id = p.insert_loading_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/small.rs")),
            "small.rs".into(),
        );
        let idx = p.tabs().iter().position(|t| t.id == id).unwrap();
        p.close(idx);
        let data = crate::preview::native_editor::NativeFileData {
            text: "x".into(),
            syntax_token: "rust".into(),
            read_only: false,
            loaded_bytes: 1,
            total_bytes: 1,
            truncated: false,
        };
        p.apply_native_load(id, Ok(data)); // 不应 panic
    }
```

- [ ] **Step 3: 跑测试确认编译失败(`insert_loading_tab`/`apply_native_load` 未定义)**

Run: `cargo test -p dozer-app --lib preview::view::tests::loading_tab_is_excluded_from_webview_pool`
Expected: 编译错误。

- [ ] **Step 4: 实现 `insert_loading_tab`/`apply_native_load`,并把 webview 相关判据排除 `loading` tab**

在 `crates/dozer-app/src/preview/view.rs` 里,`push_tab`(Task 2 改造后的版本)旁边新增两个方法。先把 `push_tab` 拆开:非原生候选文件(webview/tabular)仍走同步构造(它们本来就不慢,不需要异步);只有"可能进原生编辑器"的文件走"先占位、后填充"两步。

```rust
    /// 追加一个"原生编辑器候选、但内容尚未读到"的占位 tab,立即返回 `id`——
    /// 供调用方(`App::preview_open_path`)紧接着 spawn 异步读盘任务,读完后
    /// 用 `apply_native_load` 回填。`title`/`pending_editor_focus` 等副作用
    /// 与 `push_tab` 对齐(新 tab 成为激活者、清 stale find)。
    pub fn insert_loading_tab(&mut self, kind: TabKind, title: String) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(PreviewTab {
            id,
            kind,
            title,
            reload_nonce: 0,
            editor: None,
            tabular: None,
            dirty: false,
            loaded_bytes: 0,
            total_bytes: 0,
            truncated: false,
            loading: true,
        });
        self.active = self.tabs.len() - 1;
        self.cull_stale_find();
        id
    }

    /// 异步读盘结果回灌:按 `tab_id` 定位(用户可能在结果回来前关掉/切走这个
    /// tab,找不到就静默丢弃,语义同 `App::with_project` 对"槽位不存在"的
    /// 处理)。成功则在这里现场构造 `CodeView`(`NativeFileData` 是跨线程/
    /// `Message` 传递的纯数据,`CodeView::new` 留到收结果这一端做,见其类型
    /// 文档)并填 `loaded_bytes`/`total_bytes`/`truncated`、置一次性聚焦位
    /// (同步路径 `push_tab` 原有行为);失败则保持 `editor: None`(该 tab
    /// 落回 webview/flyfish 兜底,同步路径的既有降级语义,由
    /// `desired_webviews()` 在下一帧自然把它纳入期望清单——此时 `loading`
    /// 已置假,不再被排除)。
    pub fn apply_native_load(
        &mut self,
        tab_id: usize,
        result: Result<native_editor::NativeFileData, String>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        tab.loading = false;
        if let Ok(data) = result {
            tab.editor = Some(crate::code_editor::CodeView::new(
                &data.text,
                data.syntax_token,
                data.read_only,
            ));
            tab.loaded_bytes = data.loaded_bytes;
            tab.total_bytes = data.total_bytes;
            tab.truncated = data.truncated;
            self.pending_editor_focus = true;
        }
    }
```

把 `desired_webviews()`(现状第 303-328 行)的过滤条件从 `tab.editor.is_none() && tab.tabular.is_none()` 改成额外排除 loading:

```rust
    pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab.editor.is_none() && tab.tabular.is_none() && !tab.loading)
            .filter_map(|(idx, tab)| {
```

`active_webview_id()`(现状第 218-225 行)同样加 `!t.loading`:

```rust
    pub fn active_webview_id(&self) -> Option<usize> {
        self.tabs
            .get(self.active)
            .filter(|t| {
                t.editor.is_none()
                    && t.tabular.is_none()
                    && !t.loading
                    && matches!(t.kind, TabKind::File(_))
            })
            .map(|t| t.id)
    }
```

`select()`(现状第 231-245 行)里 `is_webview_file` 判断同样排除 loading,避免切到一个还在加载中的 tab 时被误判成"wry 文件"触发 `bump_reload`:

```rust
        let is_webview_file = matches!(
            &self.tabs[idx].kind,
            TabKind::File(_)
                if self.tabs[idx].editor.is_none()
                    && self.tabs[idx].tabular.is_none()
                    && !self.tabs[idx].loading
        );
```

- [ ] **Step 5: `push_tab` 分流——原生候选走 `insert_loading_tab`,其余不变**

`push_tab` 本身(Task 2 改造后)保留给 tabular/webview 类型使用;新增一个专供 `App` 调用的判定函数,决定一个 `TabKind::File` 是否该走"原生候选 loading"路径,供 `app/update.rs` 在 Step 6 里调用:

```rust
    /// `path` 是否会被打开为"原生编辑器候选"(即 Task 2 的
    /// `is_editable_extension && !prefers_rendered_preview`)。`App` 用它决定
    /// 是走 `insert_loading_tab`(异步)还是原有 `push_tab`(表格/webview 类,
    /// 同步、本来就不慢)。
    pub fn is_native_editor_candidate(path: &std::path::Path) -> bool {
        is_editable_extension(path) && !prefers_rendered_preview(path)
    }
```

- [ ] **Step 6: `app/message.rs` 新增两条异步结果消息**

在 `crates/dozer-app/src/app/message.rs` 里 `PreviewOpenPath(PathBuf)`(现状第 275 行)之后新增:

```rust
    /// Files 面板原生编辑器异步读盘结果回灌。`ProjectId` 按打开时所属项目
    /// 路由(见 `App::with_project` 文档,不能假设用户没有切走项目),`usize`
    /// 是 `PreviewTab.id`。`NativeFileData` 是纯数据(`Debug, Clone`,不含
    /// `CodeView`)——`CodeView` 没有 `Clone`,不能放进要求整体 `Clone` 的
    /// `Message` enum,`CodeView::new` 留给 `PreviewPane::apply_native_load`
    /// 在收到结果时现场构造。
    PreviewFileLoaded(
        crate::app::layout::ProjectId,
        usize,
        Result<crate::preview::native_editor::NativeFileData, String>,
    ),
```

`ProjectPreviewOpenPath(PathBuf)`(现状第 348 行)之后新增:

```rust
    /// Project 面板右配对预览:语义同 `PreviewFileLoaded`,独立一条消息
    /// (两面板各自的 `PreviewPane` 是完全独立的状态,不共用一条消息)。
    ProjectPreviewFileLoaded(
        crate::app::layout::ProjectId,
        usize,
        Result<crate::preview::native_editor::NativeFileData, String>,
    ),
```

（`message.rs:21` 现状 `#[derive(Debug, Clone)] pub enum Message`——这正是为什么这两条消息携带的是 `NativeFileData` 而不是内含 `CodeView` 的 `NativeEditorLoad`:`NativeFileData` 是纯数据 struct,天然可以 `#[derive(Debug, Clone)]`,不需要任何手写实现或排除。）

- [ ] **Step 7: `app/update.rs` 改异步派发**

`preview_open_path`(现状第 3159-3192 行)改成:先同步做校验/loading tab 插入,再 spawn 异步读盘:

```rust
    pub(crate) fn preview_open_path(&mut self, path: PathBuf) {
        let avail_w = self.preview_tab_bar_avail_px(PanelKind::Files);
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let handle = self.handle.clone();
        self.with_focused_project(move |ws, io| {
            if !path.is_file() {
                ws.preview_error = Some(format!("文件不存在或不可读: {}", path.display()));
                return;
            }
            ws.preview_error = None;
            ws.files.set_tree_selected(path.clone());
            ws.allowed_files
                .lock()
                .expect("allowed_files 锁")
                .insert(path.clone());

            let tab_id = if crate::preview::view::is_native_editor_candidate(&path) {
                let title = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                let id = ws
                    .preview
                    .insert_loading_tab(preview::TabKind::File(path.clone()), title);
                let proxy = io.proxy.clone();
                let load_path = path.clone();
                handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        crate::preview::native_editor::read_native_file_data(&load_path)
                            .map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let _ = proxy.send_event(Message::PreviewFileLoaded(project_id, id, result));
                });
                id
            } else {
                ws.preview.open_path(path)
            };

            let active = ws.preview.active_idx();
            let widths: Vec<f32> = ws
                .preview
                .tabs()
                .iter()
                .map(|t| preview_tab_display_width(&t.title))
                .collect();
            ws.preview_tab_first =
                tab_widget::tab_window_reveal(&widths, 4.0, avail_w, ws.preview_tab_first, active);
            let _ = tab_id;
            ws.spawn_preview_state_save(io);
            ws.spawn_preview_context_push(io);
        });
    }
```

（`ws.preview.open_path(path)` 分支处理"同一文件已开则复用已有 tab"的既有逻辑——`insert_loading_tab` 分支目前每次都新开一个 tab,不做"已打开则复用"的判重;若要保留这个体验,在走 `insert_loading_tab` 之前先跑一次现有 `open_path` 内那段"已存在同路径 tab 就切过去返回"的查找逻辑,这里为保持 Step 7 聚焦异步化本身，把这一层判重逻辑抽成 `PreviewPane` 的新公开方法 `find_existing_file_tab(&self, path: &Path) -> Option<usize>`，`open_path` 与 `preview_open_path` 都调用它。）

补一个新增方法到 `preview/view.rs`(`open_path` 现状第 148-167 行,把判重逻辑抽出来复用):

```rust
    /// 同一文件已开的 tab 下标(供 `open_path` 与 `App::preview_open_path`
    /// 的异步路径共用同一份"已打开则复用"判重逻辑)。
    pub fn find_existing_file_tab(&self, path: &std::path::Path) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.kind == TabKind::File(path.to_path_buf()))
    }

    pub fn open_path(&mut self, path: PathBuf) -> usize {
        if let Some(idx) = self.find_existing_file_tab(&path) {
            let id = self.tabs[idx].id;
            self.active = idx;
            self.cull_stale_find();
            return id;
        }
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        self.push_tab(TabKind::File(path), title)
    }
```

`preview_open_path` 里走 `insert_loading_tab` 之前先插一句判重:

```rust
            let existing = ws.preview.find_existing_file_tab(&path);
            let tab_id = if let Some(idx) = existing {
                let id = ws.preview.tabs()[idx].id;
                ws.preview.select(idx);
                id
            } else if crate::preview::view::is_native_editor_candidate(&path) {
                // ...(同上 insert_loading_tab + spawn 分支)
```

新增消息处理(挂在 `app/update.rs` 的 `update()` 大 `match` 里,紧邻现有 `Message::PreviewOpenPath(path) => self.preview_open_path(path),`,现状第 799 行):

```rust
            Message::PreviewFileLoaded(project_id, tab_id, result) => {
                self.with_project(project_id, move |ws, _io| {
                    ws.preview.apply_native_load(tab_id, result);
                });
            }
```

`project_preview_open_path`(现状第 3233 行起)与 `Message::ProjectPreviewFileLoaded` 按完全对称的写法改造(`ws.project_preview` 替代 `ws.preview`)。

- [ ] **Step 8: loading 态渲染——tab 内容区显示"加载中"占位**

`crates/dozer-app/src/workspace/view.rs` 里 `preview_pane_for` 的内容渲染分支(`if let Some(editor) = &active_tab.editor { ... } else if let Some(tabular) = &active_tab.tabular { ... }`,现状第 821/1190 行起)补第三支,在 `else if let Some(tabular)` 之后、原有兜底(webview 情形,渲染空/由 wry 子视图接管)之前插入:

```rust
        } else if active_tab.loading {
            content = content.push(
                container(
                    text("加载中…")
                        .size(byteui::theme::font::body())
                        .color(byteui::theme::color::current().dim),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced_widget::core::alignment::Horizontal::Center)
                .align_y(iced_widget::core::alignment::Vertical::Center),
            );
        }
```

- [ ] **Step 9: 跑测试确认 Step 2 用例通过 + 全量回归**

Run: `cargo test -p dozer-app --lib preview:: workspace:: app:: 2>&1 | tail -80`
Expected: 全部 PASS。若有既有测试断言"打开文件后 tab 立即有 `editor.is_some()`"(同步语义),需要按新的异步语义改成"先 `insert_loading_tab` 再手动调用 `apply_native_load` 模拟异步完成"两步——逐个跑失败的测试、按此模式修。

- [ ] **Step 10: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | tail -40 && cargo fmt --check`
Expected: 无警告/无差异。

- [ ] **Step 11: Commit**

```bash
git add crates/dozer-app/src/app/message.rs crates/dozer-app/src/app/update.rs crates/dozer-app/src/preview/view.rs crates/dozer-app/src/preview/state.rs crates/dozer-app/src/workspace/view.rs
git commit -m "feat(preview): 原生编辑器打开改异步读盘,不阻塞 UI 线程"
```

---

## Task 4: 分块加载档"加载更多"交互

**Files:**
- Modify: `crates/dozer-app/src/preview/native_editor.rs`(新增续读函数)
- Modify: `crates/dozer-app/src/preview/view.rs`(`PreviewPane::apply_more_loaded` 追加内容到已有 `CodeView`)
- Modify: `crates/dozer-app/src/app/message.rs`(`PreviewLoadMore`/`PreviewMoreLoaded` 消息)
- Modify: `crates/dozer-app/src/app/update.rs`(派发续读)
- Modify: `crates/dozer-app/src/workspace/view.rs`(横幅加"加载更多"按钮)
- Test: `preview/native_editor.rs`/`preview/view.rs` 内联单测

**Interfaces:**
- Consumes: Task 2 的 `utf8_safe_prefix_len`、`PreviewTab.loaded_bytes/total_bytes/truncated`;Task 3 的异步派发模式(`handle.spawn` + `proxy.send_event`)、`CodeView::is_read_only`。
- Produces:
  - `pub(crate) fn read_more_bytes(path: &Path, start: u64, max_extra: u64) -> std::io::Result<(String, u64, bool)>`(返回续读到的文本、新的 `loaded_bytes`、`truncated` 是否仍为真)。
  - `CodeView::append_text(&mut self, more: &str)`(只读大文件档追加内容,不记 undo——只读态本来就不记,见 Task 1/既有 `record_before`)。
  - `Message::PreviewLoadMore(PanelKind, usize)` / `Message::PreviewMoreLoaded(ProjectId, PanelKind, usize, Result<(String, u64, bool), String>)`。

- [ ] **Step 1: `CodeView` 加 `append_text`(追加而非重建整个 buffer)**

在 `crates/dozer-app/src/code_editor/mod.rs` 的 `impl CodeView` 里,紧邻 `content_from_text` 辅助函数所在区域(`CodeView::new` 之后)新增:

```rust
    /// 只读大文件档"加载更多"专用:把 `more` 追加到 buffer 末尾,不经过
    /// `perform()`(不记 undo、不受 `read_only` 过滤——这不是用户编辑,是
    /// 继续把磁盘上的原有内容灌进来)。直接对 `self.content` 发
    /// `Action::Edit(Edit::Paste(..))`——同 Task 1 的 `content_from_text`,
    /// `insert_at` 只切行不 shaping,shaping 交给下一次有界的 `update()`。
    /// 插入点是当前 buffer 末尾(最后一行末尾),不影响用户已有的滚动位置/
    /// 光标(只读态下光标本来也不承载编辑语义)。
    pub fn append_text(&mut self, more: &str) {
        if more.is_empty() {
            return;
        }
        use text_editor::{Cursor, Position};
        let last_line = self.content.line_count().saturating_sub(1);
        let last_col = self
            .content
            .line(last_line)
            .map(|l| l.text.len())
            .unwrap_or(0);
        self.content.move_to(Cursor {
            position: Position {
                line: last_line,
                column: last_col,
            },
            selection: None,
        });
        self.content
            .perform(Action::Edit(Edit::Paste(Arc::new(more.to_string()))));
    }
```

- [ ] **Step 2: 写 `append_text` 的失败测试**

在 `crates/dozer-app/src/code_editor/mod.rs` 的 `#[cfg(test)] mod tests` 里新增:

```rust
    #[test]
    fn append_text_extends_buffer_without_touching_undo() {
        let mut view = CodeView::new("line1\nline2", "txt", true);
        assert_eq!(view.undo.len(), 0);
        view.append_text("\nline3\nline4");
        assert_eq!(view.text(), "line1\nline2\nline3\nline4");
        assert_eq!(view.undo.len(), 0, "追加加载不应产生 undo 记录");
    }
```

- [ ] **Step 3: 跑测试确认失败(方法未定义),实现后转 PASS**

Run(先失败): `cargo test -p dozer-app --lib code_editor::tests::append_text_extends_buffer_without_touching_undo`
（Step 1 已给出实现,这里按 TDD 顺序:先写测试、跑一次确认编译错误、加 Step 1 的实现、再跑一次转 PASS。）
Expected: 加完实现后 PASS。

- [ ] **Step 4: `native_editor.rs` 加续读函数**

紧邻 Task 2 的 `read_and_build_native_editor` 之后新增:

```rust
/// "加载更多"续读:从字节偏移 `start` 起读最多 `max_extra` 字节(按合法
/// UTF-8 边界截断),返回 `(续读到的文本, 新的已加载字节数, 是否仍被截断)`。
/// `truncated` 语义:`start + 实际读到的字节数 < 文件总大小` 则仍为真。
pub(crate) fn read_more_bytes(
    path: &std::path::Path,
    start: u64,
    max_extra: u64,
) -> std::io::Result<(String, u64, bool)> {
    use std::io::{Read, Seek, SeekFrom};
    let total_bytes = std::fs::metadata(path)?.len();
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let cap = max_extra as usize;
    let mut buf = vec![0u8; cap];
    let mut read_total = 0usize;
    while read_total < cap {
        let n = file.read(&mut buf[read_total..])?;
        if n == 0 {
            break;
        }
        read_total += n;
    }
    buf.truncate(read_total);
    let safe_len = utf8_safe_prefix_len(&buf, buf.len());
    buf.truncate(safe_len);
    let new_loaded = start + buf.len() as u64;
    let text = String::from_utf8(buf).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    let truncated = new_loaded < total_bytes;
    Ok((text, new_loaded, truncated))
}
```

- [ ] **Step 5: 写续读函数的单测(用 tempfile 生成含多字节字符的 fixture)**

在 `native_editor.rs` 的 `size_tier_tests` 模块(Task 2 建的)里新增:

```rust
    #[test]
    fn read_more_bytes_continues_from_offset_and_respects_utf8_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        // "中" 在字节 8..11(前 8 字节是 "aaaaaaaa"),故意把续读窗口卡在
        // 这个多字节字符中间,验证不产生半个字符。
        std::fs::write(&path, "aaaaaaaa中bbbbbbbb").unwrap();
        let (first, loaded1, truncated1) = read_more_bytes(&path, 0, 9).unwrap();
        // 请求 9 字节(正好切进"中"第1字节),应回退到 8 字节纯 ASCII。
        assert_eq!(first, "aaaaaaaa");
        assert_eq!(loaded1, 8);
        assert!(truncated1);

        let (second, loaded2, truncated2) = read_more_bytes(&path, loaded1, 100).unwrap();
        assert_eq!(second, "中bbbbbbbb");
        assert!(!truncated2);
        assert_eq!(loaded2, std::fs::metadata(&path).unwrap().len());
    }
```

- [ ] **Step 6: 跑测试确认 PASS**

Run: `cargo test -p dozer-app --lib preview::native_editor::size_tier_tests::read_more_bytes_continues_from_offset_and_respects_utf8_boundary`
Expected: PASS。

- [ ] **Step 7: `PreviewPane::apply_more_loaded`**

`crates/dozer-app/src/preview/view.rs` 里,紧邻 Task 3 的 `apply_native_load` 之后新增:

```rust
    /// "加载更多"异步续读结果回灌:tab 已不存在/没有 `editor` 则 no-op。
    pub fn apply_more_loaded(
        &mut self,
        tab_id: usize,
        result: Result<(String, u64, bool), String>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        let Ok((more_text, new_loaded, truncated)) = result else {
            return;
        };
        if let Some(editor) = &mut tab.editor {
            editor.append_text(&more_text);
        }
        tab.loaded_bytes = new_loaded;
        tab.truncated = truncated;
    }
```

- [ ] **Step 8: 消息 + 派发**

`app/message.rs`,紧邻 Task 3 加的 `PreviewFileLoaded` 之后:

```rust
    /// "加载更多"横幅点击:续读下一段。`PanelKind` 区分 Files/Project,
    /// `usize` 是 `PreviewTab.id`。
    PreviewLoadMore(PanelKind, usize),
    /// 续读结果回灌,语义同 `PreviewFileLoaded`。`PanelKind` 决定回填
    /// `ws.preview` 还是 `ws.project_preview`(与 `PreviewFileLoaded` 用两条
    /// 独立消息不同,这里两个面板共用一条消息——`(String, u64, bool)` 是
    /// `Clone`/`Debug` 都成立的普通值类型,不像 `NativeEditorLoad` 内含不可
    /// `Clone` 的 `CodeView`,没有理由拆两条)。
    PreviewMoreLoaded(
        crate::app::layout::ProjectId,
        PanelKind,
        usize,
        Result<(String, u64, bool), String>,
    ),
```

`app/update.rs`,紧邻 `Message::PreviewFileLoaded` 处理之后新增:

```rust
            Message::PreviewLoadMore(kind, tab_id) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let handle = self.handle.clone();
                self.with_focused_project(move |ws, io| {
                    let pane = match kind {
                        PanelKind::Project => &ws.project_preview,
                        _ => &ws.preview,
                    };
                    let Some(tab) = pane.tabs().iter().find(|t| t.id == tab_id) else {
                        return;
                    };
                    let preview::TabKind::File(path) = tab.kind.clone() else {
                        return;
                    };
                    let start = tab.loaded_bytes;
                    let proxy = io.proxy.clone();
                    handle.spawn(async move {
                        let max_extra = crate::preview::native_editor::full_load_max_bytes();
                        let result = tokio::task::spawn_blocking(move || {
                            crate::preview::native_editor::read_more_bytes(&path, start, max_extra)
                                .map_err(|e| e.to_string())
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                        let _ = proxy.send_event(Message::PreviewMoreLoaded(
                            project_id, kind, tab_id, result,
                        ));
                    });
                });
            }
            Message::PreviewMoreLoaded(project_id, kind, tab_id, result) => {
                self.with_project(project_id, move |ws, _io| {
                    let pane = match kind {
                        PanelKind::Project => &mut ws.project_preview,
                        _ => &mut ws.preview,
                    };
                    pane.apply_more_loaded(tab_id, result);
                });
            }
```

- [ ] **Step 9: UI——横幅加"加载更多"按钮**

`crates/dozer-app/src/workspace/view.rs` 里 Task 2 加的只读 chip 代码(`if editor.is_read_only() { ... }`)扩展:`truncated` 为真时横幅追加一个按钮:

```rust
            if editor.is_read_only() {
                let colors = byteui::theme::color::current();
                if active_tab.truncated {
                    let banner = row![
                        text(format!(
                            "只读 · 文件过大 · 仅加载前 {:.1}MB,共 {:.1}MB",
                            active_tab.loaded_bytes as f64 / (1024.0 * 1024.0),
                            active_tab.total_bytes as f64 / (1024.0 * 1024.0)
                        ))
                        .size(byteui::theme::font::body())
                        .color(colors.dim),
                        iced_widget::space::horizontal(),
                        button(text("加载更多").size(byteui::theme::font::body()).color(colors.gold))
                            .on_press(match find_panel() {
                                PanelKind::Project => Message::PreviewLoadMore(PanelKind::Project, tab_id),
                                _ => Message::PreviewLoadMore(PanelKind::Files, tab_id),
                            })
                            .padding([4, 10])
                            .style(crate::dialog::action_button_style(colors.gold)),
                    ]
                    .spacing(8)
                    .align_y(iced_widget::core::alignment::Alignment::Center);
                    content = content.push(container(banner).padding([4, 8]));
                } else {
                    content = content.push(
                        container(
                            text(format!(
                                "只读 · 文件过大({:.1}MB)",
                                active_tab.total_bytes as f64 / (1024.0 * 1024.0)
                            ))
                            .size(byteui::theme::font::body())
                            .color(colors.dim),
                        )
                        .padding([4, 8]),
                    );
                }
            }
```

- [ ] **Step 10: 跑全量测试 + clippy + fmt**

Run: `cargo test -p dozer-app --lib preview:: code_editor:: 2>&1 | tail -60 && cargo clippy -p dozer-app --all-targets 2>&1 | tail -40 && cargo fmt --check`
Expected: 全部 PASS/无警告。

- [ ] **Step 11: Commit**

```bash
git add crates/dozer-app/src/code_editor/mod.rs crates/dozer-app/src/preview/native_editor.rs crates/dozer-app/src/preview/view.rs crates/dozer-app/src/app/message.rs crates/dozer-app/src/app/update.rs crates/dozer-app/src/workspace/view.rs
git commit -m "feat(preview): 分块加载档支持异步'加载更多'续读"
```

---

## Task 5: 只读大文件档全文搜索(复用 grep-searcher 磁盘流式扫描)

**背景**:`crates/dozer-app/src/extensions/search.rs` 已经实现了单文件全文搜索——`search_scope(&Scope::File(path), query)` 返回 `Vec<(String, Vec<SearchHit>)>`(`SearchHit { path, line_no, line_text }`),底层 `grep_searcher::Searcher` 流式扫描,不整读文件进内存。只读大文件档(Task 2/3/4 的 `FullLoadReadOnly`/`ChunkedReadOnly`)不复用现有 ⌘F(`FindState`/`CodeView::find_matches_all`,内存线性扫描)——新增一个独立、轻量的搜索条,专给只读大文件用。

**Files:**
- Modify: `crates/dozer-app/src/preview/state.rs`(`PreviewTab` 新增 `large_file_search: Option<LargeFileSearch>` 会话态)
- Modify: `crates/dozer-app/src/preview/view.rs`(打开/关闭/结果回灌方法)
- Modify: `crates/dozer-app/src/app/message.rs`(打开/输入/提交/结果/跳转 消息)
- Modify: `crates/dozer-app/src/app/update.rs`(派发 + 跳转/超出加载范围判定)
- Modify: `crates/dozer-app/src/workspace/view.rs`(查询条 UI,只在 `editor.is_read_only()` 时出现,替代普通 Find 条)
- Test: `preview/view.rs` 内联单测

**Interfaces:**
- Consumes: `extensions::search::{search_scope, Scope, SearchHit}`(已存在,不改)、`CodeView::{is_read_only, move_cursor_to, content.line_count()}`。
- Produces:
  - `pub struct LargeFileSearch { pub query: String, pub hits: Vec<search::SearchHit>, pub current: usize, pub running: bool }`
  - `PreviewPane::{open_large_file_search, close_large_file_search, set_large_file_search_results}`
  - `Message::{PreviewLargeFileSearchOpen(PanelKind, usize), PreviewLargeFileSearchClose(PanelKind), PreviewLargeFileSearchSubmit(PanelKind, usize, String), PreviewLargeFileSearchResults(ProjectId, PanelKind, usize, Result<Vec<search::SearchHit>, String>), PreviewLargeFileSearchGo(PanelKind, bool)}`

- [ ] **Step 1: `PreviewTab` 加会话态**

`crates/dozer-app/src/preview/state.rs`,在 `FindState` struct 定义之后新增:

```rust
/// 只读大文件档的搜索会话——⌘F 在这类 tab 上不打开 `FindState`(内存线性
/// 扫描,大文件上代价不可接受),而是打开这个。`hits` 由
/// `extensions::search::search_scope` 异步扫描回填,`line_no` 是 1-based
/// (grep_searcher 惯例,见 `SearchHit` 文档)。
#[derive(Debug, Clone, Default)]
pub struct LargeFileSearch {
    pub tab_id: usize,
    pub query: String,
    pub hits: Vec<crate::extensions::search::SearchHit>,
    pub current: usize,
    pub running: bool,
}
```

`PreviewPane` struct(现状第 98-125 行)新增字段:

```rust
    /// 只读大文件档的搜索会话,`Some` 表示条已显示。与 `find`(小文件 ⌘F)
    /// 互斥:同一时刻一个 tab 只可能命中其中一种(`open_large_file_search`/
    /// `preview_find_open` 由调用方按 `editor.is_read_only()` 二选一触发)。
    pub(crate) large_file_search: Option<LargeFileSearch>,
```

`Default for PreviewPane` 补 `large_file_search: None`。

- [ ] **Step 2: 写打开/关闭/结果回灌 + 跳转-或-提示的失败测试**

`crates/dozer-app/src/preview/view.rs` 的 `#[cfg(test)] mod tests` 追加:

```rust
    #[test]
    fn large_file_search_open_close_roundtrip() {
        let mut p = PreviewPane::default();
        let id = p.push_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/x.log")),
            "x.log".into(),
        );
        p.open_large_file_search(id);
        assert!(p.large_file_search.as_ref().is_some_and(|s| s.tab_id == id));
        p.close_large_file_search();
        assert!(p.large_file_search.is_none());
    }

    #[test]
    fn large_file_search_results_fill_hits_and_reset_current() {
        let mut p = PreviewPane::default();
        let id = p.push_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/x.log")),
            "x.log".into(),
        );
        p.open_large_file_search(id);
        let hits = vec![crate::extensions::search::SearchHit {
            path: "/tmp/x.log".into(),
            line_no: 42,
            line_text: "needle here".into(),
        }];
        p.set_large_file_search_results(id, Ok(hits.clone()));
        let s = p.large_file_search.as_ref().unwrap();
        assert_eq!(s.hits, hits);
        assert_eq!(s.current, 0);
        assert!(!s.running);
    }

    #[test]
    fn large_file_search_results_ignore_stale_tab() {
        // 结果回来前用户已经关了搜索条/切走了文件:tab_id 不匹配,忽略。
        let mut p = PreviewPane::default();
        let id = p.push_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/x.log")),
            "x.log".into(),
        );
        p.open_large_file_search(id);
        p.close_large_file_search();
        p.set_large_file_search_results(id, Ok(vec![])); // 不应 panic,也不应重新打开条
        assert!(p.large_file_search.is_none());
    }
```

- [ ] **Step 3: 跑测试确认编译失败**

Run: `cargo test -p dozer-app --lib preview::view::tests::large_file_search_open_close_roundtrip`
Expected: 编译错误(方法未定义)。

- [ ] **Step 4: 实现三个方法**

`crates/dozer-app/src/preview/view.rs`,紧邻 `apply_more_loaded`(Task 4)之后:

```rust
    /// 打开只读大文件档搜索条,锁定 `tab_id`。已开着且锁的是同一个 tab 则
    /// no-op(保留已输入的 query,同 `FindState` 既有语义);换 tab 重开会
    /// 丢旧会话(`query`/`hits` 清空)。
    pub fn open_large_file_search(&mut self, tab_id: usize) {
        if self
            .large_file_search
            .as_ref()
            .is_some_and(|s| s.tab_id == tab_id)
        {
            return;
        }
        self.large_file_search = Some(LargeFileSearch {
            tab_id,
            ..Default::default()
        });
    }

    pub fn close_large_file_search(&mut self) {
        self.large_file_search = None;
    }

    /// 异步搜索结果回灌:会话已被关闭,或已换锁到别的 tab(用户在结果回来
    /// 前又做了别的操作)时静默丢弃。
    pub fn set_large_file_search_results(
        &mut self,
        tab_id: usize,
        result: Result<Vec<crate::extensions::search::SearchHit>, String>,
    ) {
        let Some(session) = self.large_file_search.as_mut() else {
            return;
        };
        if session.tab_id != tab_id {
            return;
        }
        session.running = false;
        if let Ok(hits) = result {
            session.hits = hits;
            session.current = 0;
        }
    }
```

- [ ] **Step 5: 跑测试确认 PASS**

Run: `cargo test -p dozer-app --lib preview::view::tests::large_file_search_`
Expected: 全部 PASS。

- [ ] **Step 6: 消息 + 派发(含"跳转 vs 超出已加载范围"判定)**

`app/message.rs`,紧邻 Task 4 的 `PreviewMoreLoaded` 之后:

```rust
    /// 只读大文件档 ⌘F:打开搜索条,锁定 `usize`(`PreviewTab.id`)。
    PreviewLargeFileSearchOpen(PanelKind, usize),
    PreviewLargeFileSearchClose(PanelKind),
    /// 查询框回车/点搜索:`String` 是本次提交的 query。
    PreviewLargeFileSearchSubmit(PanelKind, usize, String),
    /// 异步结果回灌,语义同 `PreviewMoreLoaded`。
    PreviewLargeFileSearchResults(
        crate::app::layout::ProjectId,
        PanelKind,
        usize,
        Result<Vec<crate::extensions::search::SearchHit>, String>,
    ),
    /// 上一条/下一条命中(`bool` = 是否前进)。命中若在已加载范围内
    /// (`line_no <= 当前 CodeView 行数`)直接跳转光标;否则置
    /// `preview_error`/`project_preview_error` 提示"超出已加载范围"
    /// (复用现有错误提示横幅,不新增一套错误 UI)。
    PreviewLargeFileSearchGo(PanelKind, bool),
```

`app/update.rs`,紧邻 Task 4 新增消息之后:

```rust
            Message::PreviewLargeFileSearchOpen(kind, tab_id) => {
                self.with_focused_project(move |ws, _io| match kind {
                    PanelKind::Project => ws.project_preview.open_large_file_search(tab_id),
                    _ => ws.preview.open_large_file_search(tab_id),
                });
            }
            Message::PreviewLargeFileSearchClose(kind) => {
                self.with_focused_project(move |ws, _io| match kind {
                    PanelKind::Project => ws.project_preview.close_large_file_search(),
                    _ => ws.preview.close_large_file_search(),
                });
            }
            Message::PreviewLargeFileSearchSubmit(kind, tab_id, query) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let handle = self.handle.clone();
                self.with_focused_project(move |ws, io| {
                    let pane = match kind {
                        PanelKind::Project => &mut ws.project_preview,
                        _ => &mut ws.preview,
                    };
                    let Some(tab) = pane.tabs().iter().find(|t| t.id == tab_id) else {
                        return;
                    };
                    let preview::TabKind::File(path) = tab.kind.clone() else {
                        return;
                    };
                    if let Some(session) = pane.large_file_search.as_mut() {
                        session.query = query.clone();
                        session.running = true;
                    }
                    let proxy = io.proxy.clone();
                    handle.spawn(async move {
                        let scope = crate::extensions::search::Scope::File(path);
                        let result = tokio::task::spawn_blocking(move || {
                            crate::extensions::search::search_scope(&scope, &query)
                                .map(|by_file| {
                                    by_file
                                        .into_iter()
                                        .flat_map(|(_, hits)| hits)
                                        .collect::<Vec<_>>()
                                })
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                        let _ = proxy.send_event(Message::PreviewLargeFileSearchResults(
                            project_id, kind, tab_id, result,
                        ));
                    });
                });
            }
            Message::PreviewLargeFileSearchResults(project_id, kind, tab_id, result) => {
                self.with_project(project_id, move |ws, _io| {
                    let pane = match kind {
                        PanelKind::Project => &mut ws.project_preview,
                        _ => &mut ws.preview,
                    };
                    pane.set_large_file_search_results(tab_id, result);
                });
            }
            Message::PreviewLargeFileSearchGo(kind, forward) => {
                self.with_focused_project(move |ws, _io| {
                    let pane = match kind {
                        PanelKind::Project => &mut ws.project_preview,
                        _ => &mut ws.preview,
                    };
                    let Some(session) = pane.large_file_search.clone() else {
                        return;
                    };
                    if session.hits.is_empty() {
                        return;
                    }
                    let next = if forward {
                        (session.current + 1) % session.hits.len()
                    } else {
                        (session.current + session.hits.len() - 1) % session.hits.len()
                    };
                    let hit = session.hits[next].clone();
                    let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == session.tab_id)
                    else {
                        return;
                    };
                    let Some(editor) = tab.editor.as_mut() else {
                        return;
                    };
                    let target_line = (hit.line_no.saturating_sub(1)) as usize;
                    let error_slot = match kind {
                        PanelKind::Project => &mut ws.project_preview_error,
                        _ => &mut ws.preview_error,
                    };
                    if target_line < editor.content_line_count() {
                        editor.move_cursor_to((target_line, 0));
                        *error_slot = None;
                        if let Some(s) = pane.large_file_search.as_mut() {
                            s.current = next;
                        }
                    } else {
                        *error_slot = Some(
                            "命中内容超出已加载范围,点击「加载更多」后再试".to_string(),
                        );
                    }
                });
            }
```

（`pane.tabs_mut()` 若当前不存在,需要在 `PreviewPane` 里新增一个 `pub fn tabs_mut(&mut self) -> &mut [PreviewTab]` 公开方法——检查 `preview/view.rs` 现有 `pub fn tabs(&self) -> &[PreviewTab]` 旁边是否已有可变版本,没有则照此签名新增。`editor.content_line_count()` 同理:`CodeView` 目前没有公开 `line_count`,新增 `pub fn content_line_count(&self) -> usize { self.content.line_count() }`。）

- [ ] **Step 7: 补 `PreviewPane::tabs_mut` 与 `CodeView::content_line_count`(若尚不存在)**

先检查:

Run: `grep -n "pub fn tabs_mut\|pub fn content_line_count" crates/dozer-app/src/preview/view.rs crates/dozer-app/src/code_editor/mod.rs`

若为空(两者都不存在),分别在 `preview/view.rs` 的 `pub fn tabs(&self) -> &[PreviewTab]` 旁边加:

```rust
    pub fn tabs_mut(&mut self) -> &mut [PreviewTab] {
        &mut self.tabs
    }
```

在 `code_editor/mod.rs` 的 `pub fn cursor_position` 旁边加:

```rust
    /// 当前 buffer 总行数——只读大文件档搜索用它判定一处命中是否落在
    /// "已加载"范围内(见 `App` 的 `PreviewLargeFileSearchGo` 处理)。
    pub fn content_line_count(&self) -> usize {
        self.content.line_count()
    }
```

- [ ] **Step 8: ⌘F 按只读态分流到大文件搜索**

`crates/dozer-app/src/app/update.rs:843-845` 现状:

```rust
            Message::PreviewFindOpen(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_open(kind));
            }
```

改成:激活 tab 若是只读(大文件档),打开 `large_file_search` 而不是普通 `FindState`;否则维持原行为。不改 `window_events.rs` 的 ⌘F 按键路由表(它已经统一发 `PreviewFindOpen(kind)`,分流放在消息处理这一层,改动面最小):

```rust
            Message::PreviewFindOpen(kind) => {
                self.with_focused_project(move |ws, _io| {
                    let pane = match kind {
                        PanelKind::Project => &mut ws.project_preview,
                        _ => &mut ws.preview,
                    };
                    let active_is_read_only = pane
                        .tabs()
                        .get(pane.active_idx())
                        .and_then(|t| t.editor.as_ref())
                        .is_some_and(|e| e.is_read_only());
                    if active_is_read_only {
                        if let Some(tab_id) = pane.tabs().get(pane.active_idx()).map(|t| t.id) {
                            pane.open_large_file_search(tab_id);
                        }
                    } else {
                        drop(pane);
                        ws.preview_find_open(kind);
                    }
                });
            }
```

（`drop(pane)` 是因为 `pane` 借用了 `ws.preview`/`ws.project_preview` 的可变引用,`ws.preview_find_open(kind)` 内部会再借一次同一个字段——必须先释放 `pane` 这个借用。若编译器仍报重复借用,改成把 `active_is_read_only` 判定挪到一个不持有 `pane` 借用的独立表达式里,再各自调用对应分支,两种写法任选其一,以实际编译结果为准。）

`PreviewFindOpenWithReplace(kind)`(现状第 846-848 行)同理——只读大文件档没有"替换"概念,分流到同一个 `open_large_file_search`(忽略"默认展开替换行"这个语义,大文件搜索条本来就没有替换行)。`PreviewFindClose`/`PreviewFindText`/`PreviewFindGo` 三条消息本任务不需要改:`PreviewLargeFileSearchClose`/`PreviewLargeFileSearchSubmit`/`PreviewLargeFileSearchGo` 是独立的新消息族,UI 层(Step 9)直接绑定新消息,不复用旧的 `PreviewFind*` 消息名。

Run: `cargo build -p dozer-app 2>&1 | tail -30`
Expected: 编译通过。

- [ ] **Step 9: UI——只读大文件档的搜索条(替代普通 Find 条)**

`crates/dozer-app/src/workspace/view.rs` 里 `if let Some(find) = preview.find_state() { ... }`(现状第 833 行起,普通 ⌘F 条)之前插入分支:只读大文件档不渲染普通 Find 条,改渲染大文件搜索条:

```rust
            if editor.is_read_only() {
                if let Some(session) = preview.large_file_search_state() {
                    let colors = byteui::theme::color::current();
                    let panel = find_panel();
                    let count_label = text(format!("{}/{}", 
                        if session.hits.is_empty() { 0 } else { session.current + 1 },
                        session.hits.len()
                    ))
                    .size(byteui::theme::font::body())
                    .color(colors.dim);
                    let input = byteui::form::input_text::view(
                        "搜索文件内容…",
                        &session.query,
                        false,
                        None,
                        true,
                        Some(Message::PreviewLargeFileSearchSubmit(panel, tab_id, session.query.clone())),
                        false,
                        move |s: String| Message::PreviewLargeFileSearchSubmit(panel, tab_id, s),
                    );
                    let row_el = row![
                        input,
                        count_label,
                        button(text("↑")).on_press(Message::PreviewLargeFileSearchGo(panel, false)),
                        button(text("↓")).on_press(Message::PreviewLargeFileSearchGo(panel, true)),
                        button(text("×")).on_press(Message::PreviewLargeFileSearchClose(panel)),
                    ]
                    .spacing(6)
                    .align_y(iced_widget::core::alignment::Alignment::Center);
                    content = content.push(container(row_el).padding(8));
                }
            } else if let Some(find) = preview.find_state() {
                // ...(既有 Find 条渲染逻辑不变)
```

（`preview.large_file_search_state()` 需要新增一个只读访问器:在 `PreviewPane` 里加 `pub fn large_file_search_state(&self) -> Option<&LargeFileSearch> { self.large_file_search.as_ref() }`,同 `find_state()` 的既有写法。⌘F 键盘快捷键的分流已在 Step 8 完成,这里只是渲染。）

- [ ] **Step 10: 跑全量测试 + clippy + fmt**

Run: `cargo test -p dozer-app --lib preview:: code_editor:: extensions::search:: 2>&1 | tail -80 && cargo clippy -p dozer-app --all-targets 2>&1 | tail -40 && cargo fmt --check`
Expected: 全部 PASS/无警告。

- [ ] **Step 11: Commit**

```bash
git add crates/dozer-app/src/preview/state.rs crates/dozer-app/src/preview/view.rs crates/dozer-app/src/app/message.rs crates/dozer-app/src/app/update.rs crates/dozer-app/src/workspace/view.rs crates/dozer-app/src/code_editor/mod.rs
git commit -m "feat(preview): 只读大文件档全文搜索复用 grep-searcher 磁盘流式扫描"
```

---

## 人工验收清单(自动化测试之外)

- [ ] 打开一个 ~1KB 小文本文件:秒开,可编辑,⌘S 保存正常,undo/redo 正常。
- [ ] 打开一个 ~15MB 代码/日志文件(编辑档边界内):秒开,可编辑。
- [ ] 打开一个 ~100MB 文本文件(只读整读档):数百毫秒内可交互,顶部出现"只读 · 文件过大"chip,无法编辑,⌘S 无效果,滚动/复制正常。
- [ ] 打开一个超过 `full_load_max_bytes()` 上限(视机器内存,通常 256MB~4GB)的文件(只读分块档):首屏在数百毫秒~数秒内可交互,横幅显示"仅加载前 XMB,共 YMB",点"加载更多"能继续追加、不卡顿。
- [ ] 打开一个真正 GB 级(1GB/3GB/6GB)文本/日志文件:全流程不卡死 UI(可以拖动窗口/切 tab),按上述分档规则正确落档。
- [ ] 只读大文件档 ⌘F(或点击搜索入口):输入查询词能搜到命中并跳转;搜索命中落在"加载更多"尚未拉到的范围时,提示"超出已加载范围"而不是静默失败或崩溃。
- [ ] 含中文/CJK 内容的大文件:分块边界不产生乱码/半个字符;语法高亮颜色正常(不因为本次改动影响 `Shaping::Advanced` 路径)。
- [ ] 非 UTF-8(如 GBK 编码)大文件:lossy 兜底仍生效,不 panic。
- [ ] Project 面板(右配对预览)同样验证以上要点(独立于 Files 面板的 `PreviewPane`)。
