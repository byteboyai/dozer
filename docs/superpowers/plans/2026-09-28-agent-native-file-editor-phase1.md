# Agent-native 文件编辑器 Phase 1(精确修改 + 历史)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 Dozer 加两个新 MCP 工具(`locate_in_file`、`apply_precise_edit`),让能接
`dozer-mcp` 的 agent(Claude/CodeBuddy/Codex/OpenCode/v8agent)可以对项目内的文本
文件做"坐标定位 + 精确范围替换",改动自动触发预览刷新+定位+高亮,并写入一张
新的 `file_edit_history` 历史表。

**Architecture:** 新增 `dozerd::file_mutation`(纯逻辑:路径解析、UTF-8/边界校验、
Conflict Detection、写盘、坐标重算)+ `dozerd::file_edit_history`(SQLite 历史表,
照抄 `MemoryStore`/`TodoStore` 的既有模式)。`dozer-core::protocol` 新增
`Request::LocateInFile`/`ApplyPreciseEdit`、对应 `Reply` 变体、以及
`PreviewCommandAction::Highlight`(纯视觉高亮,和 `Select` 分开,不占用真实选区)。
`dozer-mcp` 新增两个 `#[tool]` 方法,复用既有 `resolve_project_and_agent()`。写盘
成功后通过既有的 `git_watch.rs` 文件监听 + `reload_webviews_for`(已存在,不用改)
触发干净 tab 的 `ReloadDocument`,`dozerd` 侧另外延迟一小段时间后把
`Select`+`Highlight` 两条 `PreviewCommand` 挂进既有的 `PreviewCommandBus`。

**Tech Stack:** Rust(dozer-core/dozerd/dozer-client/dozer-mcp/dozer-app,workspace
既有 crate)、TypeScript + CodeMirror 6(`crates/dozer-app/web/editor`)、SQLite
(`rusqlite`)。

**Spec:** `docs/superpowers/specs/2026-09-28-agent-native-file-editor-design.md`
(Phase 1 部分;该 spec 本身是
`docs/dozer-v2/Dozer-V2_Agent-native_Editor_Intentional_Requirements_v0.1.md` 的
Phase 1 落地,术语对齐 Anchor/Mutation Engine/Conflict Detection/Change Feedback/
Provenance)。

## Global Constraints

- `apply_precise_edit` 只接受严格合法 UTF-8 的文本文件;二进制/非 UTF-8 一律拒绝
  (Phase 1 不复用 GUI 那套 lossy 编码检测,daemon 侧独立、更严格地做
  `String::from_utf8` 校验——比 GUI 宽松版本更保守,不会把 lossy 文件误判成可写)。
- 路径必须解析到项目根目录子树内(`canonicalize` 后 `starts_with` 校验),越界一律
  拒绝,不允许 `..`/符号链接逃逸。
- 不引入 `expected_revision`;并发保护完全靠 `expected_text` 逐字节比对
  (Conflict Detection)。
- `insert`/`delete`/`rewrite` 都是 `apply_precise_edit`(即 `replace`)的特例,
  **不**新增额外工具。
- `file_edit_history` 是纯追加事件日志,不做 upsert,`task_ref`/`source_ref` 两列
  Phase 1 恒 `NULL`(见 spec 的 Provenance 前瞻性占位)。
- `Highlight` 是独立的 `PreviewCommandAction`,和 `Select` 分开派发,不得复用同一个
  命令(高亮不能动到用户实际的文本选区状态)。
- Goose/Aider 的 headless v8agent 兜底路由**不在本计划范围内**——本阶段没有可以
  触发它的信号来源(见 spec 的"依赖说明"),`actor`/`session_id` 只是原样透传的字符
  串参数,不需要为兜底路径写任何特殊分支。
- 坐标的"列(column)"按 Rust `chars().count()`(Unicode 标量值)计数,不是 CodeMirror
  内部的 UTF-16 code unit 计数——两者只在出现 astral plane 字符(如某些 emoji)时
  才会分叉,Phase 1 接受这个已知限制,不做额外换算。

## Review Focus

- 路径穿越:`path` 含 `../` 段或指向项目外的绝对路径/符号链接,必须被拒绝,不能
  意外写到项目目录之外——`file_mutation` 的边界测试要显式覆盖这个,不能只测"正常
  相对路径能过"。
- 坐标越界或颠倒:`start_line/col` 超出文件实际行列范围,或 `start` 位置排在
  `end` 之后,必须返回清楚的错误,不能 panic 或 index-out-of-bounds。
- 目标路径根本不存在(不是二进制/非 UTF-8,是压根没有这个文件):`locate_in_file`
  和 `apply_precise_edit` 都要给出明确"文件不存在"的结果,不是底层 IO 错误直接
  往上抛。
- `locate_in_file` 的空 `query`:空字符串在任意文本里都能"匹配"到无数个零宽位置,
  必须显式拒绝空 query,不能真的枚举返回一堆无意义的候选。
- 替换前后行数变化的坐标重算:用一行替换十行、或用十行替换一行,`apply_precise_
  edit` 算出来的"新坐标区间"必须跟着新增/减少的行数走,不能假设行数不变。

---

### Task 1: `dozer-core::protocol` 新增类型

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`(在现有 `PreviewCommandAction` 枚举、
  `Request`/`Reply` 枚举里追加,具体插入点见下方)

**Interfaces:**
- Consumes:无(纯新增类型,不依赖其它任务)
- Produces:`PreviewCommandAction::Highlight`(供 Task 2/3 消费)、
  `Request::LocateInFile`/`Request::ApplyPreciseEdit`、`Reply::LocateMatches`/
  `Reply::MutationResult`、`LocateMatch`、`MutationOutcome`(供 Task 7/8/9/10 消费)

- [ ] **Step 1: 写失败测试(序列化/反序列化往返)**

在 `crates/dozer-core/src/protocol.rs` 的测试模块(`mod tests`,文件末尾已有)追加:

```rust
#[test]
fn highlight_action_round_trips() {
    let action = PreviewCommandAction::Highlight {
        start_line: 3,
        start_column: 1,
        end_line: 5,
        end_column: 10,
        duration_ms: 2000,
    };
    let json = serde_json::to_string(&action).unwrap();
    let back: PreviewCommandAction = serde_json::from_str(&json).unwrap();
    assert_eq!(action, back);
}

#[test]
fn locate_in_file_request_round_trips() {
    let req = Request::LocateInFile {
        project_id: 1,
        path: "src/lib.rs".into(),
        query: "fn main".into(),
    };
    let json = serde_json::to_string(&req).unwrap();
    let back: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, back);
}

#[test]
fn apply_precise_edit_request_round_trips() {
    let req = Request::ApplyPreciseEdit {
        project_id: 1,
        path: "src/lib.rs".into(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 10,
        expected_text: "old".into(),
        new_text: "new".into(),
        summary: "修正拼写".into(),
        actor: "claude".into(),
        session_id: "sess-1".into(),
    };
    let json = serde_json::to_string(&req).unwrap();
    let back: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, back);
}

#[test]
fn mutation_outcome_variants_round_trip() {
    let variants = [
        MutationOutcome::Applied {
            new_start_line: 3,
            new_start_col: 1,
            new_end_line: 3,
            new_end_col: 12,
            history_id: 42,
        },
        MutationOutcome::Conflict {
            actual_text: "实际内容".into(),
        },
        MutationOutcome::NotFound,
        MutationOutcome::PathOutOfBounds,
        MutationOutcome::Unwritable {
            reason: "非 UTF-8".into(),
        },
    ];
    for v in variants {
        let json = serde_json::to_string(&v).unwrap();
        let back: MutationOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }
}
```

`Request`/`Reply` 的 `#[derive(...)]` 里已经有 `PartialEq`(比对 `todo!` 相关变体时
用过),这四个测试此时因为类型不存在会直接编译失败。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-core protocol:: 2>&1 | tail -40`
Expected: 编译错误,提示 `PreviewCommandAction::Highlight`/`Request::LocateInFile`
等不存在。

- [ ] **Step 3: 实现:`PreviewCommandAction::Highlight`**

在 `crates/dozer-core/src/protocol.rs` 的 `PreviewCommandAction` 枚举(`Replace`
变体之后)追加:

```rust
    /// 短暂高亮某段范围(纯视觉装饰,定时自动消退;不改变实际的文本选区/光标
    /// 状态——故意跟 `Select` 分开,`Select` 会占用 human 自己后续操作会用到
    /// 的"当前选中范围",高亮不应该有这个副作用)。
    Highlight {
        start_line: u32,
        start_column: u32,
        end_line: u32,
        end_column: u32,
        duration_ms: u32,
    },
```

- [ ] **Step 4: 实现:`LocateMatch`/`MutationOutcome` 类型**

在 `PreviewCommandOutcome` 枚举定义之后追加(同一文件):

```rust
/// `LocateInFile` 的一处匹配:坐标(1-based)+ 命中处附近的上下文,供 agent
/// 判断是不是自己想要的位置、或缩小 `query` 范围。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocateMatch {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub context: String,
}

/// `ApplyPreciseEdit` 的结果。`Applied` 里的 `new_*` 坐标是替换后
/// `new_text` 对应的新区间(供自动定位/高亮使用),`history_id` 是这次修改
/// 写入 `file_edit_history` 表的行 id。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MutationOutcome {
    Applied {
        new_start_line: u32,
        new_start_col: u32,
        new_end_line: u32,
        new_end_col: u32,
        history_id: i64,
    },
    /// Conflict Detection:磁盘当前内容跟调用方传的 `expected_text` 不一致。
    Conflict {
        actual_text: String,
    },
    NotFound,
    PathOutOfBounds,
    /// 二进制/非 UTF-8 等不可写的文件。
    Unwritable {
        reason: String,
    },
}
```

- [ ] **Step 5: 实现:`Request`/`Reply` 新增变体**

在 `Request` 枚举(`GetMemory` 变体之后合适位置)追加:

```rust
    /// 在项目内某文本文件里搜索 `query`,返回全部匹配的坐标。只读。
    LocateInFile {
        project_id: i64,
        path: String,
        query: String,
    },
    /// 精确替换项目内某文本文件的 `[start_line,start_col]`~`[end_line,end_col]`
    /// 区间(1-based,含端点)。`expected_text` 是调用方认为该区间当前的原样
    /// 内容,用于 Conflict Detection。`actor`/`session_id` 供历史记录署名。
    ApplyPreciseEdit {
        project_id: i64,
        path: String,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
        expected_text: String,
        new_text: String,
        summary: String,
        actor: String,
        session_id: String,
    },
```

在 `Reply` 枚举(`MemoryDetail` 变体之后)追加:

```rust
    /// `LocateInFile` 应答。
    LocateMatches {
        matches: Vec<LocateMatch>,
    },
    /// `ApplyPreciseEdit` 应答。
    MutationResult {
        outcome: MutationOutcome,
    },
```

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test -p dozer-core protocol:: 2>&1 | tail -40`
Expected: 4 个新测试全部 `ok`,其余既有测试不受影响。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): add LocateInFile/ApplyPreciseEdit request/reply and PreviewCommandAction::Highlight"
```

---

### Task 2: `EditorCommand::HighlightRange`(Rust 侧协议)

**Files:**
- Modify: `crates/dozer-app/src/preview/webview_protocol.rs`

**Interfaces:**
- Consumes:无
- Produces:`EditorCommand::HighlightRange { start: TextPosition, end: TextPosition,
  duration_ms: u32 }`,供 Task 3(Rust 下发)和 Task 4(TS 侧解码)消费

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/preview/webview_protocol.rs` 的测试模块追加:

```rust
#[test]
fn highlight_range_command_round_trips() {
    let cmd = EditorCommand::HighlightRange {
        start: TextPosition { line: 3, column: 1 },
        end: TextPosition { line: 5, column: 10 },
        duration_ms: 2000,
    };
    let json = serde_json::to_string(&cmd).unwrap();
    let back: EditorCommand = serde_json::from_str(&json).unwrap();
    assert_eq!(cmd, back);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app --lib webview_protocol:: 2>&1 | tail -30`
Expected: 编译错误,`EditorCommand::HighlightRange` 不存在。

- [ ] **Step 3: 实现**

在 `EditorCommand` 枚举(`ReplaceRange` 变体之后)追加:

```rust
    /// 短暂高亮某段范围(纯视觉装饰,`duration_ms` 后 host 自己清除;不影响
    /// 实际选区/光标)。
    HighlightRange {
        start: TextPosition,
        end: TextPosition,
        duration_ms: u32,
    },
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app --lib webview_protocol:: 2>&1 | tail -30`
Expected: 新测试 `ok`。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/preview/webview_protocol.rs
git commit -m "feat(preview): add EditorCommand::HighlightRange"
```

---

### Task 3: `apply_preview_command` 处理 `Highlight`

**Files:**
- Modify: `crates/dozer-app/src/preview/view.rs`(`apply_preview_command` 方法,
  紧跟在 `A::Select { .. }` 分支之后追加 `A::Highlight { .. }` 分支)

**Interfaces:**
- Consumes:Task 1 的 `PreviewCommandAction::Highlight`、Task 2 的
  `EditorCommand::HighlightRange`
- Produces:`apply_preview_command` 对 `Highlight` 动作的处理结果(`Accepted`/
  `UnsupportedBackend`),供 Task 7(dozerd 侧自动派发)间接依赖

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/preview/view.rs` 的测试模块里,参照已有的
`reveal_on_code_tab_queues_position` 之类测试的写法(用 `mk(action, expected)`
构造 `PreviewCommand`,调 `pane.apply_preview_command`),追加:

```rust
#[test]
fn highlight_on_code_tab_queues_highlight_command() {
    let mut pane = PreviewPane::default();
    let rs_id = pane.open_path(PathBuf::from("/tmp/highlight_test.rs"));
    let cmd = mk(
        PreviewCommandAction::Highlight {
            start_line: 2,
            start_column: 1,
            end_line: 2,
            end_column: 5,
            duration_ms: 1500,
        },
        None,
    );
    let out = pane.apply_preview_command(rs_id, &cmd);
    assert!(matches!(out, PreviewCommandOutcome::Accepted { .. }));
    assert!(pane.take_pending_editor_commands().iter().any(|(id, c)| {
        *id == rs_id
            && matches!(
                c,
                EditorCommand::HighlightRange { duration_ms: 1500, .. }
            )
    }));
}

#[test]
fn highlight_on_non_editor_tab_is_unsupported() {
    let mut pane = PreviewPane::default();
    let csv_id = pane.open_path(PathBuf::from("/tmp/highlight_test.csv"));
    let cmd = mk(
        PreviewCommandAction::Highlight {
            start_line: 1,
            start_column: 1,
            end_line: 1,
            end_column: 1,
            duration_ms: 1500,
        },
        None,
    );
    let out = pane.apply_preview_command(csv_id, &cmd);
    assert!(matches!(out, PreviewCommandOutcome::UnsupportedBackend { .. }));
}
```

(若该测试文件里已有的 `mk`/`open_path` 辅助函数签名跟这里假设的不完全一致,
以文件里实际的既有辅助函数为准调整调用方式,行为断言不变。)

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app --lib preview::view::tests::highlight 2>&1 | tail -40`
Expected: 编译错误(`PreviewCommandAction::Highlight` 分支未处理导致 `match` 非
穷尽,或直接因新测试引用的行为不存在而失败)。

- [ ] **Step 3: 实现**

在 `apply_preview_command` 的 `match &cmd.action { ... }` 里,`A::Select { .. } =>
{ ... }` 分支之后追加:

```rust
            A::Highlight {
                start_line,
                start_column,
                end_line,
                end_column,
                duration_ms,
            } => {
                if !is_editor {
                    return O::UnsupportedBackend {
                        request_id: rid,
                        detail: "该 tab 无文本编辑器".into(),
                    };
                }
                self.queue_editor_command(
                    tab_id,
                    EditorCommand::HighlightRange {
                        start: TextPosition {
                            line: *start_line,
                            column: *start_column,
                        },
                        end: TextPosition {
                            line: *end_line,
                            column: *end_column,
                        },
                        duration_ms: *duration_ms,
                    },
                );
                O::Accepted { request_id: rid }
            }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app --lib preview::view::tests::highlight 2>&1 | tail -40`
Expected: 两个新测试 `ok`。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/preview/view.rs
git commit -m "feat(preview): apply_preview_command handles Highlight action"
```

---

### Task 4: CodeMirror host 高亮装饰(TypeScript)

**Files:**
- Modify: `crates/dozer-app/web/editor/src/protocol.ts`
- Modify: `crates/dozer-app/web/editor/src/main.ts`
- Modify: `crates/dozer-app/assets/editor/editor.css`(源码在 `web/editor`,构建产物
  才落进 `assets/editor`——具体 CSS 源文件路径以 `web/editor` 目录下实际的 CSS
  源文件为准,若该目录没有独立 CSS 源文件而是内联在 `main.ts` 里插入
  `<style>`,则改成在 `main.ts` 里追加样式字符串,做法二选一,选择更贴近该文件
  现有写法的方式)

**Interfaces:**
- Consumes:Task 2 的 `EditorCommand::HighlightRange`(JSON 形状:
  `{kind:'highlight_range', start:{line,column}, end:{line,column},
  duration_ms:number}`)
- Produces:`main.ts` 里一个可正常工作的高亮装饰,`decodeCommand` 能正确解码
  `highlight_range`

- [ ] **Step 1: 写失败测试(`protocol.ts` 解码)**

在 `crates/dozer-app/web/editor/src/protocol.test.ts` 追加:

```ts
test('decodeCommand accepts highlight_range and rejects malformed', () => {
  const good = decodeCommand({
    kind: 'highlight_range',
    start: { line: 2, column: 1 },
    end: { line: 2, column: 5 },
    duration_ms: 1500,
  });
  assert.ok(good);
  assert.equal(good?.kind, 'highlight_range');

  const missingEnd = decodeCommand({
    kind: 'highlight_range',
    start: { line: 2, column: 1 },
    duration_ms: 1500,
  });
  assert.equal(missingEnd, null);
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd crates/dozer-app/web/editor && npm test 2>&1 | tail -40`
Expected: 类型/运行时错误,`highlight_range` 尚未被 `EditorCommand` 联合类型和
`decodeCommand` 认识(`good` 会是 `null`,断言失败)。

- [ ] **Step 3: 实现——`protocol.ts`**

在 `EditorCommand` 联合类型里(`replace_range` 分支之后)追加:

```ts
  | {
      kind: 'highlight_range';
      start: Position;
      end: Position;
      duration_ms: number;
    }
```

在 `decodeCommand` 的 `switch (c.kind)` 里(`case 'replace_range':` 分支之后)
追加:

```ts
    case 'highlight_range':
      return isRange(c) && typeof c.duration_ms === 'number'
        ? (c as unknown as EditorCommand)
        : null;
```

- [ ] **Step 4: 运行测试确认通过(protocol 部分)**

Run: `cd crates/dozer-app/web/editor && npm test 2>&1 | tail -40`
Expected: Step 1 加的测试 `ok`。

- [ ] **Step 5: 实现——`main.ts` 高亮装饰**

在文件顶部 import 区(`import { EditorState, Compartment, type Extension, type
StateEffect } from '@codemirror/state';` 这一行)改为同时引入 `StateField`
(值导入,不再只是 type):

```ts
import { EditorState, Compartment, StateEffect, StateField, type Extension } from '@codemirror/state';
```

在 `@codemirror/view` 的 import 列表里追加 `Decoration`、`type DecorationSet`:

```ts
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  drawSelection,
  dropCursor,
  rectangularSelection,
  crosshairCursor,
  highlightSpecialChars,
  Decoration,
  type DecorationSet,
} from '@codemirror/view';
```

在 `buildExtensions` 函数定义之前(靠近文件顶部的模块级作用域)追加高亮装饰的
`StateEffect`/`StateField` 定义,以及一个防止"新高亮被旧定时器提前清掉"的世代
计数器:

```ts
/** 设置(非 `null`)或清除(`null`)当前的 agent 编辑高亮范围。 */
const setHighlight = StateEffect.define<{ from: number; to: number } | null>();

/** 高亮装饰的持有字段;纯视觉,不影响实际选区/光标。 */
const highlightField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    deco = deco.map(tr.changes);
    for (const effect of tr.effects) {
      if (effect.is(setHighlight)) {
        deco = effect.value
          ? Decoration.set([
              Decoration.mark({ class: 'cm-agent-highlight' }).range(
                effect.value.from,
                effect.value.to,
              ),
            ])
          : Decoration.none;
      }
    }
    return deco;
  },
  provide: (field) => EditorView.decorations.from(field),
});

/** 每次收到 `highlight_range` 递增;定时器触发时只清除仍是"当前这一代"的高亮,
 * 避免新高亮被一个更早、更长 `duration_ms` 的旧定时器提前清掉。 */
let highlightGeneration = 0;
```

在 `buildExtensions()` 的返回数组里(`highlightActiveLine(),` 这一行之后)追加
`highlightField,`。

在处理下行命令的 `switch (cmd.kind)`(`case 'replace_range': { ... }` 分支之后)
追加:

```ts
    case 'highlight_range': {
      if (!isRange(cmd)) break;
      const from = positionToOffset(cmd.start);
      const to = positionToOffset(cmd.end);
      view.dispatch({ effects: setHighlight.of({ from, to }) });
      const gen = ++highlightGeneration;
      setTimeout(() => {
        if (gen === highlightGeneration) {
          view.dispatch({ effects: setHighlight.of(null) });
        }
      }, cmd.duration_ms);
      break;
    }
```

追加高亮的 CSS(选青色 `#47DEF0` 低透明度,不用金色——金色在 ByteBoy2077 主题里
专属"甲方动作",这次高亮的是 agent 的编辑结果,不是人的动作)。若
`web/editor/src` 目录下已有一个被 `main.ts` `import` 的 CSS 文件,就在那个文件
里追加;若没有独立 CSS 文件,在 `main.ts` 里紧跟在最后一个 import 之后插入一次
性的 `<style>` 注入(以该文件现有的样式组织方式为准,选择两者中更贴近既有代码
风格的一种):

```css
.cm-agent-highlight {
  background-color: rgba(71, 222, 240, 0.35);
  transition: background-color 1s ease-out;
}
```

- [ ] **Step 6: typecheck**

Run: `cd crates/dozer-app/web/editor && npm run typecheck 2>&1 | tail -40`
Expected: 无类型错误。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/web/editor/src/protocol.ts crates/dozer-app/web/editor/src/main.ts
git commit -m "feat(editor-host): add highlight_range decoration"
```

> 这一步的实际视觉效果(高亮是否正确显示、`duration_ms` 后是否正确消退)需要在
> 真实 GUI 里手动验证——`main.ts` 目前没有针对 CodeMirror 渲染行为的自动化测试
> 基础设施,`protocol.ts` 层的解码测试是这个任务里能自动化验证的部分。

---

### Task 5: `dozerd::file_edit_history` 历史存储

**Files:**
- Create: `crates/dozerd/src/file_edit_history.rs`
- Modify: `crates/dozerd/src/lib.rs`(追加 `pub mod file_edit_history;`)

**Interfaces:**
- Consumes:`dozer_core::protocol`(无需新类型,`FileEditHistoryEntry` 是本任务
  自己定义的新类型)
- Produces:`FileEditHistoryStore::new(path: &Path) -> Result<Self>`、
  `record(&self, entry: NewFileEditHistoryEntry) -> Result<i64>`(返回新行 id)、
  `list_for_path(&self, project_id: i64, path: &str) -> Result<Vec<FileEditHistoryEntry>>`
  ——供 Task 7 消费

- [ ] **Step 1: 写失败测试**

创建 `crates/dozerd/src/file_edit_history.rs`,先写测试模块:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> std::path::PathBuf {
        std::path::PathBuf::from(format!(
            "/tmp/dz-file-edit-history-test-{}.db",
            uuid::Uuid::new_v4()
        ))
    }

    fn sample_entry(project_id: i64, path: &str) -> NewFileEditHistoryEntry {
        NewFileEditHistoryEntry {
            project_id,
            target_path: path.to_string(),
            actor: "claude".to_string(),
            session_id: "sess-1".to_string(),
            start_line: 3,
            start_col: 1,
            end_line: 3,
            end_col: 10,
            old_text: "old".to_string(),
            new_text: "new".to_string(),
            summary: "修正拼写".to_string(),
        }
    }

    #[test]
    fn record_then_list_for_path_returns_in_order() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        let id1 = store.record(sample_entry(1, "src/lib.rs")).unwrap();
        let id2 = store.record(sample_entry(1, "src/lib.rs")).unwrap();
        assert_ne!(id1, id2);

        let rows = store.list_for_path(1, "src/lib.rs").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, id1);
        assert_eq!(rows[1].id, id2);
        assert_eq!(rows[0].actor, "claude");
        assert!(rows[0].task_ref.is_none());
        assert!(rows[0].source_ref.is_none());

        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn list_for_path_scopes_by_project_and_path() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        store.record(sample_entry(1, "src/lib.rs")).unwrap();
        store.record(sample_entry(1, "src/other.rs")).unwrap();
        store.record(sample_entry(2, "src/lib.rs")).unwrap();

        let rows = store.list_for_path(1, "src/lib.rs").unwrap();
        assert_eq!(rows.len(), 1);

        std::fs::remove_file(&db).ok();
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd --lib file_edit_history:: 2>&1 | tail -40`
Expected: 编译错误,`FileEditHistoryStore` 等类型不存在。

- [ ] **Step 3: 实现**

在 `crates/dozerd/src/file_edit_history.rs`(测试模块之前)写:

```rust
//! Agent 精确修改的历史记录:project 级 SQLite,纯追加事件日志(不像
//! `MemoryStore` 那样按标题 upsert——一次修改一行,同一路径的多次修改按时间
//! 顺序排列)。`task_ref`/`source_ref` 对应 v0.1 意向文档 Provenance 模型里的
//! `Task`/`Source`,Phase 1 恒为 `NULL`,先建列占位,详见
//! `docs/superpowers/specs/2026-09-28-agent-native-file-editor-design.md`。

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Mutex;

pub struct FileEditHistoryStore {
    conn: Mutex<Connection>,
}

/// 写入一条历史记录所需的字段;不含 `id`/`created_ms`(由存储层生成)。
#[derive(Debug, Clone)]
pub struct NewFileEditHistoryEntry {
    pub project_id: i64,
    pub target_path: String,
    pub actor: String,
    pub session_id: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub old_text: String,
    pub new_text: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileEditHistoryEntry {
    pub id: i64,
    pub project_id: i64,
    pub target_path: String,
    pub actor: String,
    pub session_id: String,
    pub task_ref: Option<String>,
    pub source_ref: Option<String>,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub old_text: String,
    pub new_text: String,
    pub summary: String,
    pub created_ms: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl FileEditHistoryStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS file_edit_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL,
                target_path TEXT NOT NULL,
                actor TEXT NOT NULL,
                session_id TEXT NOT NULL,
                task_ref TEXT,
                source_ref TEXT,
                start_line INTEGER NOT NULL,
                start_col INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                end_col INTEGER NOT NULL,
                old_text TEXT NOT NULL,
                new_text TEXT NOT NULL,
                summary TEXT NOT NULL,
                created_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS file_edit_history_project_path
                ON file_edit_history(project_id, target_path, created_ms);",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 写一条历史记录,返回新行 `id`(供 `MutationOutcome::Applied::history_id`
    /// 使用)。
    pub fn record(&self, entry: NewFileEditHistoryEntry) -> Result<i64> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "INSERT INTO file_edit_history
                (project_id, target_path, actor, session_id, task_ref, source_ref,
                 start_line, start_col, end_line, end_col, old_text, new_text,
                 summary, created_ms)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                entry.project_id,
                entry.target_path,
                entry.actor,
                entry.session_id,
                entry.start_line,
                entry.start_col,
                entry.end_line,
                entry.end_col,
                entry.old_text,
                entry.new_text,
                entry.summary,
                now_ms() as i64,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// 列出某项目某文件的历史,按时间正序(最早的在前)。
    pub fn list_for_path(
        &self,
        project_id: i64,
        target_path: &str,
    ) -> Result<Vec<FileEditHistoryEntry>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, project_id, target_path, actor, session_id, task_ref,
                    source_ref, start_line, start_col, end_line, end_col,
                    old_text, new_text, summary, created_ms
             FROM file_edit_history
             WHERE project_id = ?1 AND target_path = ?2
             ORDER BY created_ms ASC",
        )?;
        let rows = stmt.query_map(params![project_id, target_path], |r| {
            Ok(FileEditHistoryEntry {
                id: r.get(0)?,
                project_id: r.get(1)?,
                target_path: r.get(2)?,
                actor: r.get(3)?,
                session_id: r.get(4)?,
                task_ref: r.get(5)?,
                source_ref: r.get(6)?,
                start_line: r.get(7)?,
                start_col: r.get(8)?,
                end_line: r.get(9)?,
                end_col: r.get(10)?,
                old_text: r.get(11)?,
                new_text: r.get(12)?,
                summary: r.get(13)?,
                created_ms: r.get::<_, i64>(14)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
```

在 `crates/dozerd/src/lib.rs` 里(`pub mod default_agent_config;` 之后,保持字母
序)追加:

```rust
pub mod file_edit_history;
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd --lib file_edit_history:: 2>&1 | tail -40`
Expected: 两个新测试 `ok`。

- [ ] **Step 5: 提交**

```bash
git add crates/dozerd/src/file_edit_history.rs crates/dozerd/src/lib.rs
git commit -m "feat(dozerd): add FileEditHistoryStore"
```

---

### Task 6: `dozerd::file_mutation` —— `locate_in_file` 核心逻辑

**Files:**
- Create: `crates/dozerd/src/file_mutation.rs`
- Modify: `crates/dozerd/src/lib.rs`(追加 `pub mod file_mutation;`)

**Interfaces:**
- Consumes:`dozer_core::protocol::LocateMatch`(Task 1)
- Produces:`pub fn resolve_project_path(project_root: &Path, rel_path: &str) ->
  Result<PathBuf, LocateError>`、`pub fn locate_in_file(project_root: &Path,
  rel_path: &str, query: &str) -> Result<Vec<LocateMatch>, LocateError>`、
  `pub enum LocateError { OutOfBounds, NotFound, Unwritable(String), EmptyQuery }`
  ——供 Task 7(server.rs 接线)和 Task 7 之后的 `apply_precise_edit`(Task 7 同一
  文件)共用 `resolve_project_path`

- [ ] **Step 1: 写失败测试**

创建 `crates/dozerd/src/file_mutation.rs`,先写:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn locate_unique_match_returns_correct_coordinates() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "line one\nline two\nline three\n").unwrap();
        let matches = locate_in_file(dir.path(), "a.txt", "line two").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].start_line, 2);
        assert_eq!(matches[0].start_col, 1);
        assert_eq!(matches[0].end_line, 2);
        assert_eq!(matches[0].end_col, 9);
    }

    #[test]
    fn locate_multiple_matches_returns_all_candidates() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "foo\nfoo\nbar\n").unwrap();
        let matches = locate_in_file(dir.path(), "a.txt", "foo").unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].start_line, 1);
        assert_eq!(matches[1].start_line, 2);
    }

    #[test]
    fn locate_rejects_empty_query() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "content\n").unwrap();
        let err = locate_in_file(dir.path(), "a.txt", "").unwrap_err();
        assert!(matches!(err, LocateError::EmptyQuery));
    }

    #[test]
    fn locate_rejects_path_traversal() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "content\n").unwrap();
        let err = locate_in_file(dir.path(), "../a.txt", "content").unwrap_err();
        assert!(matches!(err, LocateError::OutOfBounds));

        let err2 = locate_in_file(dir.path(), "/etc/passwd", "root").unwrap_err();
        assert!(matches!(err2, LocateError::OutOfBounds));
    }

    #[test]
    fn locate_missing_file_returns_not_found() {
        let dir = project();
        let err = locate_in_file(dir.path(), "does-not-exist.txt", "x").unwrap_err();
        assert!(matches!(err, LocateError::NotFound));
    }

    #[test]
    fn locate_rejects_non_utf8_file() {
        let dir = project();
        fs::write(dir.path().join("bin.dat"), [0xFF, 0xFE, 0x00, 0x01]).unwrap();
        let err = locate_in_file(dir.path(), "bin.dat", "x").unwrap_err();
        assert!(matches!(err, LocateError::Unwritable(_)));
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd --lib file_mutation:: 2>&1 | tail -40`
Expected: 编译错误,`locate_in_file`/`LocateError` 不存在。先在 `Cargo.toml` 的
`[dev-dependencies]` 里确认/追加 `tempfile`(workspace 里其它 crate 已经在用,
版本对齐既有依赖)。

- [ ] **Step 3: 实现**

在测试模块之前写:

```rust
//! Agent 精确修改的核心逻辑(v0.1 意向文档里的 Mutation Engine,TextAdapter
//! 一种实现):路径解析/边界校验、UTF-8 校验、`locate_in_file` 搜索、
//! `apply_precise_edit` 的 Conflict Detection + 写盘 + 坐标重算。纯逻辑,不
//! 依赖任何 MCP/wire 类型,方便直接用 tempdir 测试。

use dozer_core::protocol::LocateMatch;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub enum LocateError {
    /// 路径落在项目根目录子树之外。
    OutOfBounds,
    /// 目标文件不存在。
    NotFound,
    /// 二进制/非 UTF-8,内容原因见携带的字符串。
    Unwritable(String),
    /// `query` 是空字符串。
    EmptyQuery,
}

/// 把 `rel_path`(agent 传入的项目内相对路径)解析成绝对路径,并校验落在
/// `project_root` 子树内。**边界校验基于父目录的 `canonicalize` 结果,不是
/// 目标文件本身**——如果对整条拼好的路径直接 `canonicalize`,一个越界但目标
/// 文件恰好不存在的路径(比如 `../nonexistent.txt`)会因为 `canonicalize`
/// 对不存在路径报错而被误判成 `NotFound`,而不是这次真正该报的
/// `OutOfBounds`,让越界检查形同虚设。父目录(`../` 之类的 `..` 段都在这一步
/// 被解析掉)通常总是存在的,`starts_with` 校验只针对父目录做,和目标文件
/// 存不存在无关;目标文件是否存在放在这之后单独判断,统一映射成
/// `NotFound`。符号链接逃逸同样被父目录的 `canonicalize` 挡住。
pub fn resolve_project_path(project_root: &Path, rel_path: &str) -> Result<PathBuf, LocateError> {
    let root = project_root
        .canonicalize()
        .map_err(|_| LocateError::OutOfBounds)?;
    let joined = project_root.join(rel_path);
    let parent = joined.parent().ok_or(LocateError::OutOfBounds)?;
    let parent_real = parent.canonicalize().map_err(|_| LocateError::OutOfBounds)?;
    if !parent_real.starts_with(&root) {
        return Err(LocateError::OutOfBounds);
    }
    let file_name = joined.file_name().ok_or(LocateError::OutOfBounds)?;
    let resolved = parent_real.join(file_name);
    if !resolved.is_file() {
        return Err(LocateError::NotFound);
    }
    Ok(resolved)
}

/// 读文件并校验是合法 UTF-8;二进制/非 UTF-8 一律拒绝(比 GUI 那套 lossy 编码
/// 检测更严格——Phase 1 daemon 侧不复用 GUI 的编码探测栈,宁可对某些 GUI 能
/// lossy 打开的文件也拒绝写,不做更复杂的探测)。
fn read_utf8(path: &Path) -> Result<String, LocateError> {
    let bytes = std::fs::read(path).map_err(|_| LocateError::NotFound)?;
    String::from_utf8(bytes).map_err(|_| LocateError::Unwritable("非 UTF-8 或二进制文件".into()))
}

/// 把 0-based 字节偏移转成 1-based (line, column);`column` 按 Unicode
/// 标量值(`chars().count()`)计数,不是 UTF-16 code unit——已知限制见计划的
/// Global Constraints。
fn offset_to_line_col(text: &str, byte_offset: usize) -> (u32, u32) {
    let mut line = 1u32;
    let mut last_newline_byte = 0usize;
    for (i, b) in text.as_bytes()[..byte_offset].iter().enumerate() {
        if *b == b'\n' {
            line += 1;
            last_newline_byte = i + 1;
        }
    }
    let col = text[last_newline_byte..byte_offset].chars().count() as u32 + 1;
    (line, col)
}

pub fn locate_in_file(
    project_root: &Path,
    rel_path: &str,
    query: &str,
) -> Result<Vec<LocateMatch>, LocateError> {
    if query.is_empty() {
        return Err(LocateError::EmptyQuery);
    }
    let path = resolve_project_path(project_root, rel_path)?;
    let text = read_utf8(&path)?;
    let mut matches = Vec::new();
    for (byte_start, _) in text.match_indices(query) {
        let byte_end = byte_start + query.len();
        let (start_line, start_col) = offset_to_line_col(&text, byte_start);
        let (end_line, end_col) = offset_to_line_col(&text, byte_end);
        let context_start = text[..byte_start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let context_end = text[byte_end..]
            .find('\n')
            .map(|i| byte_end + i)
            .unwrap_or(text.len());
        matches.push(LocateMatch {
            start_line,
            start_col,
            end_line,
            end_col,
            context: text[context_start..context_end].to_string(),
        });
    }
    Ok(matches)
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd --lib file_mutation:: 2>&1 | tail -40`
Expected: 6 个新测试全部 `ok`。

- [ ] **Step 5: 提交**

```bash
git add crates/dozerd/src/file_mutation.rs crates/dozerd/src/lib.rs crates/dozerd/Cargo.toml
git commit -m "feat(dozerd): add file_mutation::locate_in_file core logic"
```

---

### Task 7: `dozerd::file_mutation` —— `apply_precise_edit` 核心逻辑

**Files:**
- Modify: `crates/dozerd/src/file_mutation.rs`(在 Task 6 的基础上追加)

**Interfaces:**
- Consumes:Task 6 的 `resolve_project_path`/`read_utf8`/`offset_to_line_col`;
  `dozer_core::protocol::MutationOutcome`(Task 1)
- Produces:`pub struct ApplyEditInput { start_line, start_col, end_line, end_col,
  expected_text, new_text }`、`pub fn apply_precise_edit(project_root: &Path,
  rel_path: &str, input: ApplyEditInput) -> Result<AppliedEdit, LocateError>`,
  其中 `AppliedEdit { new_start_line, new_start_col, new_end_line, new_end_col,
  old_text }`(`old_text` 是校验通过时实际读到的原文,供 Task 8 写历史记录用)——
  供 Task 8(server.rs 接线)消费。冲突(`expected_text` 不匹配)通过返回值里的
  `AppliedEdit` 与否区分:本函数对"合法但冲突"的情况返回
  `Ok(Err(actual_text))`(见下方签名),不是 `LocateError`——冲突不是"这次调用
  有问题",是"这次修改不能应用",调用方(server.rs)据此映射成
  `MutationOutcome::Conflict`,其余 `LocateError` 变体映射成
  `NotFound`/`PathOutOfBounds`/`Unwritable`。

- [ ] **Step 1: 写失败测试**

在 `file_mutation.rs` 的测试模块里追加:

```rust
    #[test]
    fn apply_edit_replaces_range_and_computes_new_coordinates() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "line one\nline two\nline three\n").unwrap();
        let result = apply_precise_edit(
            dir.path(),
            "a.txt",
            ApplyEditInput {
                start_line: 2,
                start_col: 1,
                end_line: 2,
                end_col: 9,
                expected_text: "line two".into(),
                new_text: "replaced".into(),
            },
        )
        .unwrap();
        let applied = result.expect("不应冲突");
        assert_eq!(applied.new_start_line, 2);
        assert_eq!(applied.new_start_col, 1);
        assert_eq!(applied.new_end_line, 2);
        assert_eq!(applied.new_end_col, 9); // "replaced" 长度同为 8
        assert_eq!(applied.old_text, "line two");

        let on_disk = fs::read_to_string(dir.path().join("a.txt")).unwrap();
        assert_eq!(on_disk, "line one\nreplaced\nline three\n");
    }

    #[test]
    fn apply_edit_handles_line_count_change_in_new_coordinates() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "keep\nreplace me\nkeep too\n").unwrap();
        let result = apply_precise_edit(
            dir.path(),
            "a.txt",
            ApplyEditInput {
                start_line: 2,
                start_col: 1,
                end_line: 2,
                end_col: 11,
                expected_text: "replace me".into(),
                new_text: "one\ntwo\nthree".into(),
            },
        )
        .unwrap()
        .expect("不应冲突");
        assert_eq!(result.new_start_line, 2);
        assert_eq!(result.new_start_col, 1);
        assert_eq!(result.new_end_line, 4);
        assert_eq!(result.new_end_col, 6); // "three" 长度 5

        let on_disk = fs::read_to_string(dir.path().join("a.txt")).unwrap();
        assert_eq!(on_disk, "keep\none\ntwo\nthree\nkeep too\n");
    }

    #[test]
    fn apply_edit_conflict_when_expected_text_mismatches() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "actual content\n").unwrap();
        let result = apply_precise_edit(
            dir.path(),
            "a.txt",
            ApplyEditInput {
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 14,
                expected_text: "stale content".into(),
                new_text: "new".into(),
            },
        )
        .unwrap();
        let conflict = result.expect_err("应产生冲突");
        assert_eq!(conflict, "actual content");
        // 冲突时不应写盘。
        let on_disk = fs::read_to_string(dir.path().join("a.txt")).unwrap();
        assert_eq!(on_disk, "actual content\n");
    }

    #[test]
    fn apply_edit_rejects_reversed_coordinates() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "line one\nline two\n").unwrap();
        let err = apply_precise_edit(
            dir.path(),
            "a.txt",
            ApplyEditInput {
                start_line: 2,
                start_col: 1,
                end_line: 1,
                end_col: 1,
                expected_text: String::new(),
                new_text: "x".into(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, LocateError::Unwritable(_)));
    }

    #[test]
    fn apply_edit_rejects_out_of_range_coordinates() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "only one line\n").unwrap();
        let err = apply_precise_edit(
            dir.path(),
            "a.txt",
            ApplyEditInput {
                start_line: 99,
                start_col: 1,
                end_line: 99,
                end_col: 5,
                expected_text: String::new(),
                new_text: "x".into(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, LocateError::Unwritable(_)));
    }

    #[test]
    fn apply_edit_rejects_path_traversal() {
        let dir = project();
        fs::write(dir.path().join("a.txt"), "content\n").unwrap();
        let err = apply_precise_edit(
            dir.path(),
            "../a.txt",
            ApplyEditInput {
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 1,
                expected_text: String::new(),
                new_text: "x".into(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, LocateError::OutOfBounds));
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd --lib file_mutation:: 2>&1 | tail -60`
Expected: 编译错误,`apply_precise_edit`/`ApplyEditInput` 不存在。

- [ ] **Step 3: 实现**

在 `file_mutation.rs`(测试模块之前,`locate_in_file` 函数之后)追加:

```rust
pub struct ApplyEditInput {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub expected_text: String,
    pub new_text: String,
}

pub struct AppliedEdit {
    pub new_start_line: u32,
    pub new_start_col: u32,
    pub new_end_line: u32,
    pub new_end_col: u32,
    pub old_text: String,
}

/// 把 1-based (line, column) 转成字节偏移;越界/坐标非法时返回 `None`,调用方
/// 统一映射成 `LocateError::Unwritable`(和"文件类型不可写"共用同一个变体——
/// 从调用方视角都是"这次编辑请求本身有问题,不是环境/权限问题")。
fn line_col_to_offset(text: &str, line: u32, col: u32) -> Option<usize> {
    if line == 0 || col == 0 {
        return None;
    }
    let mut current_line = 1u32;
    let mut line_start = 0usize;
    if line > 1 {
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                current_line += 1;
                if current_line == line {
                    line_start = i + 1;
                    break;
                }
            }
        }
        if current_line != line {
            return None;
        }
    }
    let line_end = text[line_start..]
        .find('\n')
        .map(|i| line_start + i)
        .unwrap_or(text.len());
    let line_text = &text[line_start..line_end];
    let mut offset = line_start;
    for (count, (byte_i, ch)) in line_text.char_indices().enumerate() {
        if count as u32 + 1 == col {
            offset = line_start + byte_i;
            return Some(offset);
        }
        offset = line_start + byte_i + ch.len_utf8();
    }
    if line_text.chars().count() as u32 + 1 == col {
        Some(line_start + line_text.len())
    } else {
        None
    }
}

/// 精确替换 `[start_line,start_col]`~`[end_line,end_col]` 区间。返回值的外层
/// `Result` 是"这次调用本身合不合法"(路径/坐标/编码),内层 `Result` 是
/// Conflict Detection 的结果:`Ok(AppliedEdit)` 表示已成功写盘,`Err(String)`
/// 表示 `expected_text` 跟磁盘实际内容不一致(附带磁盘实际内容),这种情况下
/// **不写盘**。
pub fn apply_precise_edit(
    project_root: &Path,
    rel_path: &str,
    input: ApplyEditInput,
) -> Result<Result<AppliedEdit, String>, LocateError> {
    let path = resolve_project_path(project_root, rel_path)?;
    let text = read_utf8(&path)?;

    let start = line_col_to_offset(&text, input.start_line, input.start_col)
        .ok_or_else(|| LocateError::Unwritable("坐标超出文件范围".into()))?;
    let end = line_col_to_offset(&text, input.end_line, input.end_col)
        .ok_or_else(|| LocateError::Unwritable("坐标超出文件范围".into()))?;
    if start > end {
        return Err(LocateError::Unwritable(
            "start 坐标必须不晚于 end 坐标".into(),
        ));
    }

    let actual = &text[start..end];
    if actual != input.expected_text {
        return Ok(Err(actual.to_string()));
    }

    let mut new_content = String::with_capacity(text.len() - (end - start) + input.new_text.len());
    new_content.push_str(&text[..start]);
    new_content.push_str(&input.new_text);
    new_content.push_str(&text[end..]);
    std::fs::write(&path, &new_content)
        .map_err(|e| LocateError::Unwritable(format!("写盘失败: {e}")))?;

    let new_end_byte = start + input.new_text.len();
    let (new_start_line, new_start_col) = offset_to_line_col(&new_content, start);
    let (new_end_line, new_end_col) = offset_to_line_col(&new_content, new_end_byte);

    Ok(Ok(AppliedEdit {
        new_start_line,
        new_start_col,
        new_end_line,
        new_end_col,
        old_text: actual.to_string(),
    }))
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd --lib file_mutation:: 2>&1 | tail -60`
Expected:(Task 6 的 6 个 + 本任务的 6 个)共 12 个测试全部 `ok`。

- [ ] **Step 5: 提交**

```bash
git add crates/dozerd/src/file_mutation.rs
git commit -m "feat(dozerd): add file_mutation::apply_precise_edit core logic"
```

---

### Task 8: `dozerd::server` 接线 —— `Stores` + `Request` 处理 + 自动定位/高亮派发

**Files:**
- Modify: `crates/dozerd/src/server.rs`
- Modify: `crates/dozerd/src/main.rs`

**Interfaces:**
- Consumes:Task 5 的 `FileEditHistoryStore`(+ `NewFileEditHistoryEntry`)、
  Task 6/7 的 `file_mutation::{resolve_project_path, locate_in_file,
  apply_precise_edit, ApplyEditInput, LocateError}`、Task 1 的
  `Request::LocateInFile`/`ApplyPreciseEdit`、`Reply::LocateMatches`/
  `MutationResult`、`MutationOutcome`
- Produces:`dozerd::server::Stores` 新增 `file_edit_history` 字段(供 Task 9 的
  测试构造 `Stores` 时需要跟着补上这个字段);dozerd 对
  `Request::LocateInFile`/`ApplyPreciseEdit` 的完整处理

- [ ] **Step 1: 写失败测试**

这一步的测试是集成级的(起真实 `dozerd::server::serve` + `Client`),放在
`crates/dozerd/tests/file_mutation_requests.rs`(新文件):

```rust
use dozer_client::Client;
use dozer_core::protocol::MutationOutcome;
use std::sync::Arc;

fn temp_sock() -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "/tmp/dz-file-mutation-{}.sock",
        uuid::Uuid::new_v4()
    ))
}

struct CleanupGuard(std::path::PathBuf);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn start_daemon() -> (
    std::path::PathBuf,
    CleanupGuard,
    std::sync::Arc<dozerd::projects::ProjectStore>,
) {
    let sock = temp_sock();
    let db = std::path::PathBuf::from(format!("/tmp/dz-file-mutation-{}.db", uuid::Uuid::new_v4()));
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let stores = dozerd::server::Stores {
        registry: Arc::new(dozerd::registry::SessionRegistry::new()),
        projects: projects.clone(),
        bookmarks: Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap()),
        code_health: Arc::new(dozerd::code_health::CodeHealthStore::new(&db).unwrap()),
        transcripts: Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap()),
        session_summaries: Arc::new(
            dozerd::session_summary::SessionSummaryStore::open(&db).unwrap(),
        ),
        summary_jobs: Arc::new(dozerd::summary_jobs::SummaryJobStore::open(&db).unwrap()),
        backfill_registry: Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new()),
        todos: Arc::new(dozerd::todo::TodoStore::new(&db).unwrap()),
        categories: Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap()),
        memories: Arc::new(dozerd::memory::MemoryStore::new(&db).unwrap()),
        file_edit_history: Arc::new(dozerd::file_edit_history::FileEditHistoryStore::new(&db).unwrap()),
    };
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    let s = sock.clone();
    tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            stores,
            dozerd::task_poller::new_in_flight(),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (sock.clone(), CleanupGuard(sock), projects)
}

#[tokio::test]
async fn locate_then_apply_then_conflict_on_reapply() {
    let (sock, _guard, projects) = start_daemon().await;
    let project_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        project_dir.path().join("a.txt"),
        "line one\nline two\nline three\n",
    )
    .unwrap();
    let project = projects
        .open(project_dir.path().to_str().unwrap())
        .unwrap();

    let client = Client::new(sock);
    let matches = client
        .locate_in_file(project.id, "a.txt", "line two")
        .await
        .unwrap();
    assert_eq!(matches.len(), 1);
    let m = &matches[0];

    let outcome = client
        .apply_precise_edit(
            project.id,
            "a.txt",
            m.start_line,
            m.start_col,
            m.end_line,
            m.end_col,
            "line two",
            "replaced line",
            "把第二行替换掉",
            "claude",
            "sess-1",
        )
        .await
        .unwrap();
    assert!(matches!(outcome, MutationOutcome::Applied { .. }));

    // 用刚才(已经过期的)坐标+旧内容再打一次,应该产生 Conflict,并且不会
    // 真的再改一次盘。
    let stale_outcome = client
        .apply_precise_edit(
            project.id,
            "a.txt",
            m.start_line,
            m.start_col,
            m.end_line,
            m.end_col,
            "line two",
            "should not apply",
            "重复调用",
            "claude",
            "sess-1",
        )
        .await
        .unwrap();
    assert!(matches!(stale_outcome, MutationOutcome::Conflict { .. }));

    let on_disk = std::fs::read_to_string(project_dir.path().join("a.txt")).unwrap();
    assert_eq!(on_disk, "line one\nreplaced line\nline three\n");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd --test file_mutation_requests 2>&1 | tail -60`
Expected: 编译错误——`Stores` 缺 `file_edit_history` 字段、`Client` 没有
`locate_in_file`/`apply_precise_edit` 方法、`Request`/`Reply` 未处理这两个变体。
本任务先把 `dozerd` 侧编译通过(`Client` 方法留给 Task 9 实现;如果本任务单独
跑测试因为 `Client` 方法缺失而无法编译,属于预期中的失败,继续下一步)。

- [ ] **Step 3: 实现——`Stores` 结构体 + `main.rs` 构造**

在 `crates/dozerd/src/server.rs` 的 `Stores` 结构体(`pub memories:
std::sync::Arc<crate::memory::MemoryStore>,` 之后)追加:

```rust
    pub file_edit_history: std::sync::Arc<crate::file_edit_history::FileEditHistoryStore>,
```

在 `crates/dozerd/src/main.rs`,`let memories = Arc::new(...)` 语句之后追加:

```rust
    let file_edit_history = Arc::new(dozerd::file_edit_history::FileEditHistoryStore::new(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
```

在 `Stores { ... }` 字面量构造(`memories,` 这一行)之后追加 `file_edit_history,`。

- [ ] **Step 4: 实现——`handle_conn` 解构 + `Request` 处理**

在 `crates/dozerd/src/server.rs` 的 `handle_conn` 函数里,`let Stores { ... memories
} = stores;` 的解构列表(`memories` 之后)追加 `file_edit_history,`。

在 `Request::GetMemory { .. } => { .. }` 分支之后追加:

```rust
                        Request::LocateInFile { project_id, path, query } => {
                            let root = projects
                                .list()
                                .ok()
                                .and_then(|ps| ps.into_iter().find(|p| p.id == project_id))
                                .map(|p| std::path::PathBuf::from(p.path));
                            match root {
                                None => Reply::Error {
                                    message: "项目不存在".into(),
                                },
                                Some(root) => {
                                    match crate::file_mutation::locate_in_file(&root, &path, &query) {
                                        Ok(matches) => Reply::LocateMatches { matches },
                                        Err(e) => Reply::LocateMatches {
                                            matches: locate_error_to_empty_with_log(e, &path),
                                        },
                                    }
                                }
                            }
                        }
                        Request::ApplyPreciseEdit {
                            project_id,
                            path,
                            start_line,
                            start_col,
                            end_line,
                            end_col,
                            expected_text,
                            new_text,
                            summary,
                            actor,
                            session_id,
                        } => {
                            let root = projects
                                .list()
                                .ok()
                                .and_then(|ps| ps.into_iter().find(|p| p.id == project_id))
                                .map(|p| std::path::PathBuf::from(p.path));
                            match root {
                                None => Reply::Error {
                                    message: "项目不存在".into(),
                                },
                                Some(root) => {
                                    let input = crate::file_mutation::ApplyEditInput {
                                        start_line,
                                        start_col,
                                        end_line,
                                        end_col,
                                        expected_text,
                                        new_text: new_text.clone(),
                                    };
                                    let outcome = match crate::file_mutation::apply_precise_edit(
                                        &root, &path, input,
                                    ) {
                                        Ok(Ok(applied)) => {
                                            let history_id = file_edit_history
                                                .record(crate::file_edit_history::NewFileEditHistoryEntry {
                                                    project_id,
                                                    target_path: path.clone(),
                                                    actor,
                                                    session_id,
                                                    start_line,
                                                    start_col,
                                                    end_line,
                                                    end_col,
                                                    old_text: applied.old_text,
                                                    new_text,
                                                    summary,
                                                })
                                                .unwrap_or(-1);
                                            let outcome = dozer_core::protocol::MutationOutcome::Applied {
                                                new_start_line: applied.new_start_line,
                                                new_start_col: applied.new_start_col,
                                                new_end_line: applied.new_end_line,
                                                new_end_col: applied.new_end_col,
                                                history_id,
                                            };
                                            schedule_post_edit_reveal(
                                                preview_commands.clone(),
                                                project_id,
                                                path.clone(),
                                                applied.new_start_line,
                                                applied.new_start_col,
                                                applied.new_end_line,
                                                applied.new_end_col,
                                            );
                                            outcome
                                        }
                                        Ok(Err(actual_text)) => {
                                            dozer_core::protocol::MutationOutcome::Conflict { actual_text }
                                        }
                                        Err(crate::file_mutation::LocateError::OutOfBounds) => {
                                            dozer_core::protocol::MutationOutcome::PathOutOfBounds
                                        }
                                        Err(crate::file_mutation::LocateError::NotFound) => {
                                            dozer_core::protocol::MutationOutcome::NotFound
                                        }
                                        Err(crate::file_mutation::LocateError::Unwritable(reason)) => {
                                            dozer_core::protocol::MutationOutcome::Unwritable { reason }
                                        }
                                        Err(crate::file_mutation::LocateError::EmptyQuery) => {
                                            unreachable!("apply_precise_edit 不会产生 EmptyQuery")
                                        }
                                    };
                                    Reply::MutationResult { outcome }
                                }
                            }
                        }
```

在 `handle_conn` 函数所在的 impl 块附近(文件里任意合适的私有辅助函数区域)
追加两个辅助函数:

```rust
/// `LocateInFile` 遇到路径/编码问题时的兜底:不把 daemon 内部错误细节透传成
/// agent 能直接摸到的报错通道,统一表现成"空匹配列表"——调用方(`dozer-mcp`
/// 的 `locate_in_file` 工具)据此提示"没搜到,检查路径和 query"就够了,不需要
/// 额外区分"路径越界"和"文件不存在",这两者对 agent 来说都是同一句"重新确认
/// 一下路径"。
fn locate_error_to_empty_with_log(
    err: crate::file_mutation::LocateError,
    path: &str,
) -> Vec<dozer_core::protocol::LocateMatch> {
    tracing::debug!(?err, path, "locate_in_file 失败");
    Vec::new()
}

/// `ApplyPreciseEdit` 成功后,延迟一小段时间再把「定位到新范围 + 短暂高亮」
/// 两条命令挂进 `PreviewCommandBus`——延迟是为了大概率排在
/// `git_watch.rs`(通用文件监听)触发的 `ReloadDocument` 之后,避免定位先于
/// 重载发生、又被重载盖掉视图状态(具体排序不做强保证,是啓发式,见 spec
/// "Change Feedback"一节)。不等待这两条命令的应答——纯 best-effort,目标
/// tab 未必存在(文件没被打开过),`PreviewCommandBus` 对找不到 tab 的情况本来
/// 就有 `NotFound` 语义,这里不关心结果。
fn schedule_post_edit_reveal(
    bus: std::sync::Arc<crate::preview_commands::PreviewCommandBus>,
    project_id: i64,
    path: String,
    start_line: u32,
    start_col: u32,
    end_line: u32,
    end_col: u32,
) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let target = dozer_core::protocol::PreviewCommandTarget::Path { path: path.clone() };
        let select_id = format!("agent-edit-select-{}", uuid::Uuid::new_v4());
        let _ = bus.enqueue(dozer_core::protocol::PreviewCommand {
            request_id: select_id,
            project_id,
            target: target.clone(),
            action: dozer_core::protocol::PreviewCommandAction::Select {
                start_line,
                start_column: start_col,
                end_line,
                end_column: end_col,
            },
            expected_revision: None,
        });
        let highlight_id = format!("agent-edit-highlight-{}", uuid::Uuid::new_v4());
        let _ = bus.enqueue(dozer_core::protocol::PreviewCommand {
            request_id: highlight_id,
            project_id,
            target,
            action: dozer_core::protocol::PreviewCommandAction::Highlight {
                start_line,
                start_column: start_col,
                end_line,
                end_column: end_col,
                duration_ms: 2000,
            },
            expected_revision: None,
        });
    });
}
```

(`enqueue` 的返回值是一个 `oneshot::Receiver`,这里刻意用 `let _ =` 丢弃、不
`.await` 它——不阻塞、也不关心这次 best-effort 定位有没有真的被应用。)

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test -p dozerd --test file_mutation_requests 2>&1 | tail -60`
Expected:仍会因为 `Client` 缺方法而编译失败——这是预期的,Task 9 补上后本测试
才能真正跑通。本步骤改成先确认 `cargo check -p dozerd 2>&1 | tail -60` 干净
(dozerd 自身能编译),把这个测试文件的编译验证挪到 Task 9 的 Step 里一并完成。

- [ ] **Step 6: 提交**

```bash
git add crates/dozerd/src/server.rs crates/dozerd/src/main.rs crates/dozerd/tests/file_mutation_requests.rs
git commit -m "feat(dozerd): wire LocateInFile/ApplyPreciseEdit requests and post-edit reveal+highlight"
```

---

### Task 9: `dozer-client` 客户端方法

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`

**Interfaces:**
- Consumes:Task 1 的 `Request::LocateInFile`/`ApplyPreciseEdit`、
  `Reply::LocateMatches`/`MutationResult`
- Produces:`Client::locate_in_file(&self, project_id: i64, path: &str, query:
  &str) -> Result<Vec<LocateMatch>>`、`Client::apply_precise_edit(&self,
  project_id: i64, path: &str, start_line: u32, start_col: u32, end_line: u32,
  end_col: u32, expected_text: &str, new_text: &str, summary: &str, actor: &str,
  session_id: &str) -> Result<MutationOutcome>` —— 供 Task 8 的集成测试(补齐后
  真正跑通)和 Task 10 的 `dozer-mcp` 工具消费

- [ ] **Step 1: 确认 Task 8 的集成测试当前失败原因**

Run: `cargo test -p dozerd --test file_mutation_requests 2>&1 | tail -60`
Expected: 编译错误,提示 `Client` 没有 `locate_in_file`/`apply_precise_edit`
方法。

- [ ] **Step 2: 实现**

在 `crates/dozer-client/src/lib.rs`,紧跟在 `write_memory`/`get_memory` 方法
(前面 grep 到的位置)附近追加:

```rust
    pub async fn locate_in_file(
        &self,
        project_id: i64,
        path: &str,
        query: &str,
    ) -> Result<Vec<dozer_core::protocol::LocateMatch>> {
        match self
            .roundtrip(&Request::LocateInFile {
                project_id,
                path: path.into(),
                query: query.into(),
            })
            .await?
        {
            Reply::LocateMatches { matches } => Ok(matches),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn apply_precise_edit(
        &self,
        project_id: i64,
        path: &str,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
        expected_text: &str,
        new_text: &str,
        summary: &str,
        actor: &str,
        session_id: &str,
    ) -> Result<dozer_core::protocol::MutationOutcome> {
        match self
            .roundtrip(&Request::ApplyPreciseEdit {
                project_id,
                path: path.into(),
                start_line,
                start_col,
                end_line,
                end_col,
                expected_text: expected_text.into(),
                new_text: new_text.into(),
                summary: summary.into(),
                actor: actor.into(),
                session_id: session_id.into(),
            })
            .await?
        {
            Reply::MutationResult { outcome } => Ok(outcome),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

`apply_precise_edit` 的 11 个参数里有连续 4 个同类型 `u32`(`start_line,
start_col, end_line, end_col`)——按 `CLAUDE.md` 的参数裁决,这类"四个方向坐标"
天然同质、顺序传错的风险和"四个方向 padding"是同一类,不强制拆成具名结构体;
但如果 review 阶段觉得这个签名读起来不够清楚,拆成 `EditRange{start_line,
start_col,end_line,end_col}` 结构体也可以,不是本步骤的硬性要求。

- [ ] **Step 3: 运行测试确认通过**

Run: `cargo test -p dozerd --test file_mutation_requests 2>&1 | tail -60`
Expected: `locate_then_apply_then_conflict_on_reapply` 通过。

- [ ] **Step 4: 提交**

```bash
git add crates/dozer-client/src/lib.rs
git commit -m "feat(dozer-client): add locate_in_file/apply_precise_edit methods"
```

---

### Task 10: `dozer-mcp` 工具 —— `locate_in_file`/`apply_precise_edit`

**Files:**
- Modify: `crates/dozer-mcp/src/server.rs`
- Create: `crates/dozer-mcp/tests/file_mutation_tools.rs`

**Interfaces:**
- Consumes:Task 9 的 `Client::locate_in_file`/`Client::apply_precise_edit`、
  既有的 `DozerMcpServer::resolve_project_and_agent()`
- Produces:两个新的 `#[tool]` 方法,MCP 协议层面可被外部 agent 调用

- [ ] **Step 1: 写失败测试**

创建 `crates/dozer-mcp/tests/file_mutation_tools.rs`,参照
`crates/dozer-mcp/tests/memory_tools.rs` 的 `start_daemon`/`session_with_project`
写法(注意 `Stores` 字面量要带上 Task 8 新增的 `file_edit_history` 字段;因为这
个 feature 需要真实项目根目录,`session_with_project` 之外还要额外用
`ProjectStore::open` 建一个指向 tempdir 的真实项目,`session_id` 关联的
`project_id` 用这个真实项目的 id):

```rust
use dozer_client::Client;
use dozer_mcp::server::{
    ApplyPreciseEditParams, DozerMcpServer, LocateInFileParams,
};
use rmcp::handler::server::wrapper::Parameters;
use std::sync::Arc;
use std::time::Duration;

fn temp_sock() -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "/tmp/dz-mcp-fm-{}.sock",
        &uuid::Uuid::new_v4().to_string()[..8]
    ))
}

struct CleanupGuard(std::path::PathBuf);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn start_daemon() -> (std::path::PathBuf, CleanupGuard, Arc<dozerd::projects::ProjectStore>) {
    let sock = temp_sock();
    let db = std::path::PathBuf::from(format!("/tmp/dz-mcp-fm-{}.db", uuid::Uuid::new_v4()));
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let stores = dozerd::server::Stores {
        registry: Arc::new(dozerd::registry::SessionRegistry::new()),
        projects: projects.clone(),
        bookmarks: Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap()),
        code_health: Arc::new(dozerd::code_health::CodeHealthStore::new(&db).unwrap()),
        transcripts: Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap()),
        session_summaries: Arc::new(
            dozerd::session_summary::SessionSummaryStore::open(&db).unwrap(),
        ),
        summary_jobs: Arc::new(dozerd::summary_jobs::SummaryJobStore::open(&db).unwrap()),
        backfill_registry: Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new()),
        todos: Arc::new(dozerd::todo::TodoStore::new(&db).unwrap()),
        categories: Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap()),
        memories: Arc::new(dozerd::memory::MemoryStore::new(&db).unwrap()),
        file_edit_history: Arc::new(
            dozerd::file_edit_history::FileEditHistoryStore::new(&db).unwrap(),
        ),
    };
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    let s = sock.clone();
    tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            stores,
            dozerd::task_poller::new_in_flight(),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (sock.clone(), CleanupGuard(sock), projects)
}

async fn session_for_project(sock: &std::path::Path, project_id: i64) -> String {
    let client = Client::new(sock.to_path_buf());
    let session = client
        .create(
            "fm-test",
            "/bin/sh",
            &["-c".into(), "cat".into()],
            "/tmp",
            80,
            24,
            project_id,
        )
        .await
        .expect("建会话");
    session.id
}

#[tokio::test]
async fn locate_then_apply_roundtrip_via_mcp_tools() {
    let (sock, _guard, projects) = start_daemon().await;
    let project_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        project_dir.path().join("a.txt"),
        "line one\nline two\nline three\n",
    )
    .unwrap();
    let project = projects
        .open(project_dir.path().to_str().unwrap())
        .unwrap();
    let session_id = session_for_project(&sock, project.id).await;
    let server = DozerMcpServer::new(Client::new(sock), session_id);

    let locate_result = server
        .locate_in_file(Parameters(LocateInFileParams {
            path: "a.txt".into(),
            query: "line two".into(),
        }))
        .await
        .expect("locate_in_file 应该成功");
    let value = locate_result
        .structured_content
        .expect("locate_in_file 应返回结构化内容");
    let matches = value["matches"].as_array().expect("matches 应是数组");
    assert_eq!(matches.len(), 1);
    let start_line = matches[0]["start_line"].as_u64().unwrap() as u32;
    let start_col = matches[0]["start_col"].as_u64().unwrap() as u32;
    let end_line = matches[0]["end_line"].as_u64().unwrap() as u32;
    let end_col = matches[0]["end_col"].as_u64().unwrap() as u32;

    let apply_result = server
        .apply_precise_edit(Parameters(ApplyPreciseEditParams {
            path: "a.txt".into(),
            start_line,
            start_col,
            end_line,
            end_col,
            expected_text: "line two".into(),
            new_text: "replaced".into(),
            summary: "测试替换".into(),
        }))
        .await
        .expect("apply_precise_edit 应该成功");
    let outcome = apply_result
        .structured_content
        .expect("apply_precise_edit 应返回结构化内容");
    assert_eq!(outcome["kind"], "applied");

    let on_disk = std::fs::read_to_string(project_dir.path().join("a.txt")).unwrap();
    assert_eq!(on_disk, "line one\nreplaced\nline three\n");
}

#[tokio::test]
async fn apply_precise_edit_reports_conflict_with_actual_text() {
    let (sock, _guard, projects) = start_daemon().await;
    let project_dir = tempfile::tempdir().unwrap();
    std::fs::write(project_dir.path().join("a.txt"), "actual\n").unwrap();
    let project = projects
        .open(project_dir.path().to_str().unwrap())
        .unwrap();
    let session_id = session_for_project(&sock, project.id).await;
    let server = DozerMcpServer::new(Client::new(sock), session_id);

    let apply_result = server
        .apply_precise_edit(Parameters(ApplyPreciseEditParams {
            path: "a.txt".into(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 7,
            expected_text: "stale!".into(),
            new_text: "x".into(),
            summary: "应该冲突".into(),
        }))
        .await
        .expect("调用本身应该成功(冲突是结果,不是调用错误)");
    let outcome = apply_result.structured_content.expect("应有结构化内容");
    assert_eq!(outcome["kind"], "conflict");
    assert_eq!(outcome["actual_text"], "actual");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-mcp --test file_mutation_tools 2>&1 | tail -60`
Expected: 编译错误,`DozerMcpServer` 没有 `locate_in_file`/`apply_precise_edit`
方法,`LocateInFileParams`/`ApplyPreciseEditParams` 不存在。

- [ ] **Step 3: 实现**

在 `crates/dozer-mcp/src/server.rs`,在既有的参数结构体(`GetMemoryParams` 之后)
追加:

```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LocateInFileParams {
    pub path: String,
    pub query: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ApplyPreciseEditParams {
    pub path: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub expected_text: String,
    pub new_text: String,
    pub summary: String,
}
```

在 `#[tool_router(server_handler)] impl DozerMcpServer` 块里(`get_memory` 方法
之后)追加两个工具方法:

```rust
    #[tool(
        description = "在项目内某个文本文件里搜索一段文字,返回精确坐标(1-based 行列)。唯一匹配才算定位成功;多处匹配会把候选全部列出,重新传更长/更具体的 query 缩小范围。调用 apply_precise_edit 前应该先用这个工具拿到准确坐标,不要自己数行号。"
    )]
    pub async fn locate_in_file(
        &self,
        Parameters(LocateInFileParams { path, query }): Parameters<LocateInFileParams>,
    ) -> Result<CallToolResult, McpError> {
        let (project_id, _agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let matches = self
            .client
            .locate_in_file(project_id, &path, &query)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let value = json!(
            matches
                .into_iter()
                .map(|m| json!({
                    "start_line": m.start_line,
                    "start_col": m.start_col,
                    "end_line": m.end_line,
                    "end_col": m.end_col,
                    "context": m.context,
                }))
                .collect::<Vec<_>>()
        );
        Ok(CallToolResult::structured(json!({ "matches": value })))
    }

    #[tool(
        description = "精确替换项目内某文本文件 [start_line,start_col]~[end_line,end_col] 区间(1-based,含端点)的内容。expected_text 必须是这段区间当前的原样内容(用 locate_in_file 拿到坐标后紧跟着读到的那段文字),不一致会返回 conflict 并附带磁盘上的真实内容,不会写入;整篇重写就把区间设成整个文件。summary 必填,一句话说明这次改了什么。"
    )]
    pub async fn apply_precise_edit(
        &self,
        Parameters(ApplyPreciseEditParams {
            path,
            start_line,
            start_col,
            end_line,
            end_col,
            expected_text,
            new_text,
            summary,
        }): Parameters<ApplyPreciseEditParams>,
    ) -> Result<CallToolResult, McpError> {
        let (project_id, agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let outcome = self
            .client
            .apply_precise_edit(
                project_id,
                &path,
                start_line,
                start_col,
                end_line,
                end_col,
                &expected_text,
                &new_text,
                &summary,
                agent.label(),
                &self.session_id,
            )
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let value = match outcome {
            dozer_core::protocol::MutationOutcome::Applied {
                new_start_line,
                new_start_col,
                new_end_line,
                new_end_col,
                history_id,
            } => json!({
                "kind": "applied",
                "new_start_line": new_start_line,
                "new_start_col": new_start_col,
                "new_end_line": new_end_line,
                "new_end_col": new_end_col,
                "history_id": history_id,
            }),
            dozer_core::protocol::MutationOutcome::Conflict { actual_text } => json!({
                "kind": "conflict",
                "actual_text": actual_text,
            }),
            dozer_core::protocol::MutationOutcome::NotFound => json!({ "kind": "not_found" }),
            dozer_core::protocol::MutationOutcome::PathOutOfBounds => {
                json!({ "kind": "path_out_of_bounds" })
            }
            dozer_core::protocol::MutationOutcome::Unwritable { reason } => json!({
                "kind": "unwritable",
                "reason": reason,
            }),
        };
        Ok(CallToolResult::structured(value))
    }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-mcp --test file_mutation_tools 2>&1 | tail -60`
Expected: 两个新测试全部 `ok`。

- [ ] **Step 5: 跑一遍全 workspace 相关测试确认没有破坏既有行为**

Run: `cargo test -p dozer-core -p dozer-app -p dozerd -p dozer-client -p dozer-mcp 2>&1 | tail -100`
Expected: 全部通过(既有测试 + 本计划新增的全部测试)。

- [ ] **Step 6: `cargo clippy`/`cargo fmt` 检查**

Run: `cargo clippy -p dozer-core -p dozer-app -p dozerd -p dozer-client -p dozer-mcp --all-targets 2>&1 | tail -80 && cargo fmt --check 2>&1 | tail -80`
Expected: 无新增 warning(既有代码里的历史 warning 不在本计划修复范围内);
`fmt --check` 对本计划改过的文件无 diff(有 diff 就跑 `cargo fmt` 补一次格式化,
单独提交)。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-mcp/src/server.rs crates/dozer-mcp/tests/file_mutation_tools.rs
git commit -m "feat(dozer-mcp): add locate_in_file/apply_precise_edit MCP tools"
```

---

## 完成后的手动验证(不在自动化测试范围内)

- 真实起一个 Claude Code 会话,打开一个文本文件,让它调用 `locate_in_file` +
  `apply_precise_edit` 改一处内容,肉眼确认:磁盘内容确实改了;如果该文件当时
  开在某个 Preview tab 里,大约 300ms~2s 内应该看到内容刷新、视图滚动到改动
  处、并有一次短暂的青色高亮消退。
- 手动确认 `docs/user_guide/mcp.md` 里"没有对应 MCP 工具的面板"章节需要更新
  (Files 面板现在有工具了)——这是文档更新,不在本计划的代码任务里,但完成
  实现后不要忘记去改(spec 的"需要同步修订的既有结论"一节已经记了这一条)。
