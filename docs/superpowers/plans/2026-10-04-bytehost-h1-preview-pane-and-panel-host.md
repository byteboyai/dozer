# bytehost H1:预览窗格收口 + 面板视图契约(`PanelHost`)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地 H0 汇总里最小风险的两个切片(`docs/dozer-v2/bytehost-H0/00-summary.md` §4 的候选 1 与 2),**不改任何用户可见行为**:(1) 把全库约 82 处手写的"按 `PanelKind::Project` 选预览窗格"收口成 `Workspace::preview_pane[_mut](kind)`;(2) 让 `extensions/**` 里的面板代码不再 import `App`/`Workspace`——面板对宿主的只读需求集中成一个 `PanelHost` trait,其余数据改由调用方传参;面板边界门禁基线由 22 降到 **0**。

**Architecture:** 三个任务,每个都能单独编译、通过测试、合并。Task 1 用一个跑通的一次性重写脚本做机械替换(75 处)+ 4 个访问器 + 4 处手工编辑,测试先行(访问器的指针相等测试 + 自递归防线)。Task 2 先写"假宿主"测试(面板 view 能脱离 `App` 构造),再加 `PanelHost` trait 与 `impl PanelHost for App`,然后改 6 个面板文件的签名并处理 6 处"面板不该碰宿主"的点位。Task 3 把门禁扩一条全库规则(`R-PANE-PICK`)防回退,基线更新,回填 H0 文档。

**Tech Stack:** Rust(iced 0.14,`dozer-app`);Python 3 标准库(一次性重写脚本 + 审计门禁)。不碰 `Cargo.toml`/`Cargo.lock`(除非本地 `[patch]` 造成的 lock 抖动,见 Global Constraints)。

**Spec:** `docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md`(§2 E1/E2、§6 验收第 4、5 条);依据 `docs/dozer-v2/bytehost-H0/`(`01-panelkind.md` B0、`02-panel-to-host.md` §3.1、`03-host-to-panel.md`、`E3-registry.tsv`、`00-summary.md` §4)。

## Global Constraints

- **行为保持:** 不改任何用户可见行为。`preview_pane(kind)` 的语义逐字等于旧写法:`Project` → `project_preview`,**其余任何 kind(含 `Files`)→ `preview`**。
- **在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytehost-h1/...`),分支 `bytehost-h1`,从当前 `main` 开。主 checkout 常有并发会话与未提交改动,不得在其上改代码或 `git add -A`;只 `git add` 指定路径;提交前 `git status` 与 `git diff --cached --stat` 看全貌。
- **`Cargo.lock`:** 本地 `.cargo/config.toml` 的 `[patch]`(byteui/bytegit 指向 `../`)会让 `Cargo.lock` 里 `byteui`/`bytegit` 的 `source = …` 行消失。**每次提交前 `git checkout -- Cargo.lock`**(本计划不改任何依赖,锁文件不应出现在提交里);`.cargo/config.toml` 不提交。
- **每次改 Rust 后跑 `cargo fmt -p dozer-app`,再用 `git status --short` 确认只有本任务触碰的文件变了**;fmt 动了别的文件就 `git checkout` 回去。
- **编译与测试一律带 `RUSTFLAGS="-D unconditional_recursion"`:** 草稿里重写脚本曾把访问器自己的函数体也改成了"调用自己"(编译只给 warning,测试恰好没走到),这个 lint 把它变成硬错误。
- **新增/改造的函数参数 ≥7 个且有同类型相邻参数时用参数结构体**(项目 CLAUDE.md);`PanelHost::list_collapse_button` 与 `todo::view` 是**搬家**不是新增,沿用 `#[allow(clippy::too_many_arguments)]`,不在本计划里重构。
- **不新增 `Box<dyn Fn>`/不为未来第二个宿主实现预先加 trait 方法**(规格 §1 非目标):`PanelHost` 只含面板现在真的在问的 6 个方法。
- 提交信息用 `refactor:`/`test:`/`chore(audit):`/`docs:` 前缀;末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## 已知基线(在 `main` 上、本计划开始前就存在)

- `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 稳定失败。
- `extensions::git_log::tests::build_marks_head_branch_and_labels`、`build_populates_time_and_is_merge` 在**linked worktree** 里失败(对 dozer 自己的仓库跑 `gleisbau`;P1 起记录)。**在专用 worktree 里执行本计划时它们会失败,不是本计划造成的**——Task 3 末尾回到主 checkout 重跑确认。
- `assets::tests::serves_vendored_asset_with_mime` 偶发失败(单独重跑通过)。
- 草稿基线(`main`、linked worktree):`1725 passed; 1 failed`(只算 `delete_confirm`,git_log 两个当时碰巧通过)。执行时以 Task 1 Step 2 实测为准;**每个任务结束的通过数 = 基线通过数 + 本任务新增测试数,失败只有上面列的已知项**。

## Review Focus

计划里的测试没有逐条覆盖、但真实改动者会踩的情形;每条由指明的任务里的步骤固定:

1. **访问器借走整个 `Workspace`,与按字段的并存借用冲突。** 草稿里有 3 处重写后编译不过(`ws.project`、`ws.*_preview_error` 与 `ws.preview_pane_mut(..)` 并存)。这类冲突是**编译期硬错误**,不会静默变行为;Task 1 Step 6 给出 2 处的手工处理,其中 1 处刻意保留按字段写法(带注释)。
2. **重写脚本改了不该改的地方。** 最危险的一次:访问器自己的函数体被改成自递归。Task 1 的顺序(先跑脚本、后插访问器)+ `-D unconditional_recursion` + 3 个指针相等测试 + Step 8 的变异检验共同防住。
3. **`_ =>` 回落语义。** 旧写法里任何非 `Project` 的 kind 都落到 `preview`(包括 `Agent`、`Todo`)。Task 1 的测试对 5 个非 Project kind 逐个断言。
4. **非 macOS 代码路径从未在本机编译过**(`#[cfg(not(target_os = "macos"))]` 块;项目记忆里已记录)。脚本是文本级替换,会碰到这些块里的同形写法。Task 1 Step 9、Task 2 Step 9 要求逐个读这些 cfg 块里被改动的 hunk(命令已给),确认是同形替换。
5. **`PanelHost` 带 `impl Fn` 参数的泛型方法,不是对象安全的**——面板只能写 `&impl PanelHost`(静态分派),不能 `&dyn PanelHost`。这是有意的(零成本、不需要装箱),Task 2 的假宿主测试证明静态分派足够用;若以后要 `dyn`,再单独设计。

---

## File Structure

| 路径 | 职责 | 任务 |
|---|---|---|
| `crates/dozer-app/src/workspace/state.rs` | 新增 4 个访问器 `preview_pane`/`preview_pane_mut`/`preview_error_mut`/`preview_tab_first_mut`;被重写的 34 处调用点 | 1 |
| `crates/dozer-app/src/workspace/tests.rs` | 3 个访问器测试 | 1 |
| `crates/dozer-app/src/app/{app,update}.rs`、`platform/window_events.rs` | 被重写的窗格选择点(共 38 处)+ 3 处手工编辑 | 1 |
| `crates/dozer-app/src/panel_host.rs`(新) | `PanelHost` trait、`impl PanelHost for App`、假宿主测试 | 2 |
| `crates/dozer-app/src/main.rs` | `mod panel_host;` | 2 |
| `crates/dozer-app/src/extensions/{conversations,git_log,ssh,database/view,usage/view,todo/view,todo/protocol,files/tree,ssh/sftp}.rs` | 去掉对 `App`/`Workspace` 的引用 | 2 |
| `crates/dozer-app/src/app/{view,app,update}.rs`、`platform/todo_detail_overlay.rs` | 调用方改传参 | 2 |
| `scripts/audit/check_panel_boundary.py`、`test_check_panel_boundary.py`、`panel-boundary.baseline.json` | 门禁新增 `R-PANE-PICK` 规则,基线更新 | 3 |
| `docs/dozer-v2/bytehost-H0/{00-summary,01-panelkind,02-panel-to-host}.md`、`E3-registry.tsv`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` | 回填 H1 结果 | 3 |

一次性脚本(**不提交**)放在 `$SCRATCH`(见 Task 1 Step 1),不进仓库。

---

### Task 1: `Workspace::preview_pane[_mut]` 与预览窗格收口

**Files:**
- Modify: `crates/dozer-app/src/workspace/tests.rs`(追加 3 个测试)
- Modify(重写脚本 + 手工): `crates/dozer-app/src/workspace/state.rs`、`crates/dozer-app/src/app/app.rs`、`crates/dozer-app/src/app/update.rs`、`crates/dozer-app/src/platform/window_events.rs`

**Interfaces:**
- Produces(Task 3 与后续切片依赖):`Workspace::preview_pane(&self, PanelKind) -> &PreviewPane`、`preview_pane_mut(&mut self, PanelKind) -> &mut PreviewPane`、`preview_error_mut(&mut self, PanelKind) -> &mut Option<String>`、`preview_tab_first_mut(&mut self, PanelKind) -> &mut usize`(均 `pub(crate)`)。

- [ ] **Step 1: 建 worktree、分支、一次性脚本目录**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytehost-h1 -b bytehost-h1 main
mkdir -p ../dozer-bytehost-h1/.cargo && cp .cargo/config.toml ../dozer-bytehost-h1/.cargo/config.toml
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && git log --oneline | head -1 && git status --short
export SCRATCH=~/Projects/CoralProjects/byteboy/dozer-bytehost-h1-scratch && mkdir -p $SCRATCH
```

Expected: 干净、分支 `bytehost-h1`。`$SCRATCH` 在仓库外,放一次性脚本。**此后所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytehost-h1/` 前缀,且每个新 shell 先 `export SCRATCH=…`。**

- [ ] **Step 2: 记录基线**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^test result|FAILED" | tee $SCRATCH/baseline.txt; git checkout -- Cargo.lock`
Expected: 失败只有"已知基线"里列的(`delete_confirm…`,在 linked worktree 里还会有 2 个 `git_log::tests::build_…`);记下通过数 N(草稿:1725)。**这是后面每步比较的基线。**

- [ ] **Step 3: 写失败的测试**

在 `crates/dozer-app/src/workspace/tests.rs` 末尾追加:

```rust
// ---- bytehost H1:预览窗格访问器 ----

#[test]
fn preview_pane_is_project_preview_only_for_the_project_panel() {
    let ws = Workspace::empty_for_project_placeholder();
    assert!(std::ptr::eq(
        ws.preview_pane(PanelKind::Project),
        &ws.project_preview
    ));
    for kind in [
        PanelKind::Files,
        PanelKind::GitLog,
        PanelKind::Todo,
        PanelKind::Web,
        PanelKind::Agent,
    ] {
        assert!(
            std::ptr::eq(ws.preview_pane(kind), &ws.preview),
            "{kind:?} 应落到 Files 的预览窗格"
        );
    }
}

#[test]
fn preview_pane_mut_hits_the_same_pane_as_preview_pane() {
    let mut ws = Workspace::empty_for_project_placeholder();
    let project_ptr = ws.preview_pane(PanelKind::Project) as *const _;
    let files_ptr = ws.preview_pane(PanelKind::Files) as *const _;
    assert_eq!(
        ws.preview_pane_mut(PanelKind::Project) as *const _,
        project_ptr
    );
    assert_eq!(ws.preview_pane_mut(PanelKind::Files) as *const _, files_ptr);
    assert_ne!(project_ptr, files_ptr);
}

#[test]
fn per_pane_companion_slots_follow_the_same_selection() {
    let mut ws = Workspace::empty_for_project_placeholder();
    *ws.preview_tab_first_mut(PanelKind::Project) = 3;
    *ws.preview_tab_first_mut(PanelKind::Files) = 5;
    assert_eq!((ws.project_preview_tab_first, ws.preview_tab_first), (3, 5));
    *ws.preview_error_mut(PanelKind::Project) = Some("p".into());
    *ws.preview_error_mut(PanelKind::Files) = Some("f".into());
    assert_eq!(ws.project_preview_error.as_deref(), Some("p"));
    assert_eq!(ws.preview_error.as_deref(), Some("f"));
}
```

- [ ] **Step 4: 确认测试失败**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app workspace::tests::preview_pane 2>&1 | grep -E "^error" -A4 | head -12`
Expected: 编译失败,`no method named \`preview_pane\` found for struct \`Workspace\``(访问器还不存在)。

- [ ] **Step 5: 写重写脚本并先 dry-run**

创建 `$SCRATCH/rewrite_preview_pane.py`:

```python
#!/usr/bin/env python3
"""一次性重写脚本(H1 切片 1):把 `Project`/`Files` 预览窗格二选一的写法收口成
`Workspace::preview_pane(kind)` / `preview_pane_mut(kind)`。**不提交、不进产品构建**。

只改"选窗格、逻辑完全相同"的三种机械形态,其余留给人工:
  F1  let p = if K == PanelKind::Project { &[mut ]R.project_preview } else { &[mut ]R.preview };
  F2  match K { PanelKind::Project => R.project_preview<tail>, _ => R.preview<tail>, }
  F4  match K { [PanelKind::Files => &R.preview,] PanelKind::Project => &[mut ]R.project_preview, _ => &[mut ]R.preview, }
  F5  let project = kind == PanelKind::Project; ... let p = if project { &[mut ]R.project_preview } else { &[mut ]R.preview };
  F3  if K == PanelKind::Project { R.project_preview<tail>[;] } else { R.preview<tail>[;] }
可变性:F1 看原文 `&mut`/`&`;F2/F3 看最近的外层 `fn` 签名(`&mut self` → mut;`&self` → ref;
闭包/自由函数里默认 mut,编译不过再人工改成 ref)。用法: rewrite_preview_pane.py <src 根目录> [--dry-run]
"""
import os, re, sys

FN_RE = re.compile(r"\bfn\s+\w+[^{;]*?\(\s*(&mut self|&self|self|[^)]*)", re.S)

F1 = re.compile(
    r"let (?P<v>\w+) = if (?P<k>\*?[\w\.]+) == (?:crate::app::)?PanelKind::Project \{\s*"
    r"&(?P<m1>mut )?(?P<r>[\w\.]+)\.project_preview\s*\} else \{\s*"
    r"&(?P<m2>mut )?(?P=r)\.preview\s*\};")
F2 = re.compile(
    r"match (?P<k>[\w\.]+) \{\s*(?:crate::app::)?PanelKind::Project => "
    r"(?P<r>[\w\.]+)\.project_preview(?P<tail>[^,{}]*?),\s*_ => (?P=r)\.preview(?P=tail),?\s*\}")
F3 = re.compile(
    r"if (?P<k>[\w\.]+) == (?:crate::app::)?PanelKind::Project \{\s*"
    r"(?P<r>[\w\.]+)\.project_preview(?P<tail>\.[^;{}]+?)(?P<semi>;?)\s*\} else \{\s*"
    r"(?P=r)\.preview(?P=tail)(?P=semi)\s*\}")


F4 = re.compile(
    r"match (?P<k>[\w\.]+) \{\s*(?:PanelKind::Files => &(?:mut )?[\w\.]+\.preview,\s*)?"
    r"(?:crate::app::)?PanelKind::Project => &(?P<m1>mut )?(?P<r>[\w\.]+)\.project_preview,\s*"
    r"_ => &(?P<m2>mut )?(?P=r)\.preview,?\s*\}")


F5 = re.compile(
    r"let (?P<v>\w+) = if project \{\s*&(?P<m1>mut )?(?P<r>[\w\.]+)\.project_preview\s*\} else \{\s*"
    r"&(?P<m2>mut )?(?P=r)\.preview\s*\};")
PROJECT_FLAG = re.compile(r"[ \t]*let project = kind == PanelKind::Project;\n")


def enclosing_is_ref_self(text, pos):
    """pos 之前最近一个 `fn` 的签名是 `&self` 吗(是 → 只读访问器)。"""
    best = None
    for m in re.finditer(r"\bfn\s+\w+", text[:pos]):
        best = m
    if best is None:
        return False
    sig = text[best.start(): best.start() + 400]
    sig = sig[: sig.find("{")] if "{" in sig else sig
    return "&self" in sig and "&mut self" not in sig


def rewrite(text):
    n = 0

    def f1(m):
        nonlocal n
        n += 1
        acc = "preview_pane_mut" if m.group("m1") else "preview_pane"
        return f"let {m.group('v')} = {m.group('r')}.{acc}({m.group('k')});"

    text = F1.sub(f1, text)

    def mk(m_mut):
        def inner(m):
            nonlocal n
            n += 1
            acc = "preview_pane" if enclosing_is_ref_self(text_holder[0], m.start()) else "preview_pane_mut"
            tail = m.group("tail")
            semi = m.groupdict().get("semi", "")
            return f"{m.group('r')}.{acc}({m.group('k')}){tail}{semi}"
        return inner

    def f4(m):
        nonlocal n
        n += 1
        acc = "preview_pane_mut" if m.group("m1") else "preview_pane"
        return f"{m.group('r')}.{acc}({m.group('k')})"

    text = F4.sub(f4, text)

    def f5(m):
        nonlocal n
        n += 1
        acc = "preview_pane_mut" if m.group("m1") else "preview_pane"
        return f"let {m.group('v')} = {m.group('r')}.{acc}(kind);"

    if F5.search(text):  # F5 假定同一函数里有 `kind` 变量、且 `project` 布尔只用来选窗格
        text = F5.sub(f5, text)
        text = PROJECT_FLAG.sub("", text)
    text_holder = [text]
    text = F2.sub(mk(None), text)
    text_holder[0] = text
    text = F3.sub(mk(None), text)
    return text, n


def main():
    root = sys.argv[1]
    dry = "--dry-run" in sys.argv
    total = 0
    for dp, _, fns in sorted(os.walk(root)):
        for fn in sorted(fns):
            if not fn.endswith(".rs"):
                continue
            p = os.path.join(dp, fn)
            with open(p, encoding="utf-8") as f:
                old = f.read()
            new, n = rewrite(old)
            if n:
                print(f"{n:3d}  {os.path.relpath(p, root)}")
                total += n
                if not dry:
                    with open(p, "w", encoding="utf-8") as f:
                        f.write(new)
    print(f"total rewrites: {total}")


if __name__ == "__main__":
    main()
```

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && python3 $SCRATCH/rewrite_preview_pane.py crates/dozer-app/src --dry-run`
Expected: 逐文件打印命中数,合计 **`total rewrites: 75`**(草稿值:`app/app.rs` 8、`app/update.rs` 28、`platform/window_events.rs` 2、`workspace/state.rs` 37)。**不是 75 就先停下**:`main` 上这几个文件有改动,把新增/减少的点位对照 `grep -rnE "(==|!=) (crate::app::)?PanelKind::Project|PanelKind::Project\s*=>\s*&(mut )?[a-z_.]+\.project_preview" crates/dozer-app/src` 逐个看是否形态相同,再继续。

- [ ] **Step 6: 真跑脚本,插入访问器,做 4 处手工编辑**

**顺序不能换:先跑脚本、后插访问器**(否则脚本会把访问器自己的 `match` 改成自递归)。

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1
python3 $SCRATCH/rewrite_preview_pane.py crates/dozer-app/src | tail -1
```

创建 `$SCRATCH/accessors.py` 并运行(在 `workspace/state.rs` 里 `active_preview_tab_has_native_editor` 文档注释之前插入 4 个访问器):

```python
p="crates/dozer-app/src/workspace/state.rs"
s=open(p).read()
anchor="    /// `kind` 是当前 `FocusIntent::Preview` 携带的面板(`Files` 或\n"
acc='''    /// `kind` 对应的预览窗格:`Project` 是 Project 面板自己的预览列,其余(含 `Files`)都是 Files
    /// 面板的预览列。**所有"按面板选预览窗格"的地方都走这两个访问器**,不要再手写
    /// `if kind == PanelKind::Project { &self.project_preview } else { &self.preview }`。
    ///
    /// 注意:访问器借走整个 `Workspace`;同一作用域里还要并存地借用 `ws` 的其它字段(如
    /// `ws.project`、`ws.*_preview_error`)时,保持按字段的写法,别硬套访问器。
    pub(crate) fn preview_pane(&self, kind: PanelKind) -> &PreviewPane {
        match kind {
            PanelKind::Project => &self.project_preview,
            _ => &self.preview,
        }
    }

    pub(crate) fn preview_pane_mut(&mut self, kind: PanelKind) -> &mut PreviewPane {
        match kind {
            PanelKind::Project => &mut self.project_preview,
            _ => &mut self.preview,
        }
    }

    /// `kind` 对应面板的预览错误槽(`preview_error` / `project_preview_error`)。
    pub(crate) fn preview_error_mut(&mut self, kind: PanelKind) -> &mut Option<String> {
        match kind {
            PanelKind::Project => &mut self.project_preview_error,
            _ => &mut self.preview_error,
        }
    }

    /// `kind` 对应预览 tab 条的翻页窗口起点(`preview_tab_first` / `project_preview_tab_first`)。
    pub(crate) fn preview_tab_first_mut(&mut self, kind: PanelKind) -> &mut usize {
        match kind {
            PanelKind::Project => &mut self.project_preview_tab_first,
            _ => &mut self.preview_tab_first,
        }
    }

'''
assert anchor in s; s=s.replace(anchor,acc+anchor,1); open(p,"w").write(s)
```

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && python3 $SCRATCH/accessors.py`

创建 `$SCRATCH/manual_edits.py` 并运行(3 处脚本处理不了的点位 + 1 处把 `project` 布尔改成访问器):

```python
p="crates/dozer-app/src/app/update.rs"
s=open(p).read()
for a,b in [
('''                    let pane = ws.preview_pane_mut(kind);
                    pane.blank_info_in_flight = false;
                    let current_root = ws.project.as_ref().map(|p| p.path.as_str());
                    if !current_root.is_some_and(|r| r == info.path.to_string_lossy().as_ref()) {
                        return;
                    }''','''                    // 先取项目根(`preview_pane_mut` 会借走整个 `ws`,不能再和它并存)。
                    let current_root = ws.project.as_ref().map(|p| p.path.clone());
                    let pane = ws.preview_pane_mut(kind);
                    pane.blank_info_in_flight = false;
                    if !current_root
                        .as_deref()
                        .is_some_and(|r| r == info.path.to_string_lossy().as_ref())
                    {
                        return;
                    }'''),
('''                    let pane = ws.preview_pane_mut(kind);
                    let Some(session) = pane.large_file_search.clone() else {''','''                    // 这里要同时可变借用 `pane` 与 `ws.*_preview_error` 两个不相交字段,
                    // 访问器会借走整个 `ws`,所以保留按字段的写法。
                    let pane = match kind {
                        PanelKind::Project => &mut ws.project_preview,
                        _ => &mut ws.preview,
                    };
                    let Some(session) = pane.large_file_search.clone() else {'''),
('''                        if binding.panel == PanelKind::Project {
                            ws.project_preview_tab_first = 0;
                        } else {
                            ws.preview_tab_first = 0;
                        }''','''                        *ws.preview_tab_first_mut(binding.panel) = 0;'''),
]:
    assert a in s,a[:60]; s=s.replace(a,b)
open(p,"w").write(s)
p="crates/dozer-app/src/workspace/state.rs"
s=open(p).read()
a='''                if project {
                    self.project_preview_error = err;
                } else {
                    self.preview_error = err;
                }
                return None;'''
b='''                *self.preview_error_mut(kind) = err;
                return None;'''
assert a in s; s=s.replace(a,b); open(p,"w").write(s)
```

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && python3 $SCRATCH/manual_edits.py`

这 4 处各自的原因(都在草稿里编译器逐个报出来的):
- `PreviewBlankInfoLoaded`:`pane` 可变借用与随后读 `ws.project` 冲突 → 先取项目根再借窗格;
- `PreviewLargeFileSearchGo`:要同时可变借用 `pane` 与 `ws.*_preview_error` 两个不相交字段 → **保留按字段写法**(加了注释),这是 `main` 上唯一保留的 `PanelKind::Project => &mut ws.project_preview` 臂;
- `close_after_save` 分支:`if binding.panel == Project { ws.project_preview_tab_first = 0 } else { … }` → `*ws.preview_tab_first_mut(binding.panel) = 0;`;
- `preview_pane_toggle_render_mode` 的 Err 分支:`if project { … } else { … }` → `*self.preview_error_mut(kind) = err;`。

- [ ] **Step 7: 格式化、编译、测试**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1
cargo fmt -p dozer-app && git status --short
RUSTFLAGS="-D unconditional_recursion" cargo build -p dozer-app 2>&1 | grep -E "^(error|warning: (unused|function cannot))" -A10 | head -30
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^error|^test result|FAILED"
git checkout -- Cargo.lock
```
Expected: `git status --short` 只有 `workspace/state.rs`、`app/app.rs`、`app/update.rs`、`platform/window_events.rs`、`workspace/tests.rs`(和 `Cargo.lock`,马上还原);`cargo build` 无输出(无 error、无 unused、无 `function cannot return without recursing`);测试通过数 = 基线 N + 3,失败只有已知项。**若 build 报 `E0502`/`E0499`(借用冲突):** 说明 `main` 上有本计划没见过的并存借用点——读报错点,按 Step 6 第 1、2 条的做法处理(先取其它字段 / 保留按字段写法并加注释),不要硬套访问器。

- [ ] **Step 8: 变异检验:访问器自递归必须被拦住**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1
git add crates && git status --short
python3 - <<'EOF'
p = "crates/dozer-app/src/workspace/state.rs"
s = open(p).read()
a = "    pub(crate) fn preview_pane(&self, kind: PanelKind) -> &PreviewPane {\n        match kind {\n            PanelKind::Project => &self.project_preview,\n            _ => &self.preview,\n        }\n    }"
b = "    pub(crate) fn preview_pane(&self, kind: PanelKind) -> &PreviewPane {\n        self.preview_pane(kind)\n    }"
assert a in s
open(p, "w").write(s.replace(a, b))
EOF
RUSTFLAGS="-D unconditional_recursion" cargo build -p dozer-app 2>&1 | grep -E "unconditional_recursion|cannot return without recursing" | head -3
git checkout -- crates/dozer-app/src/workspace/state.rs && git diff --stat | tail -1; git checkout -- Cargo.lock
```
Expected: 变异后 build 失败并点名 `unconditional_recursion`;还原后 `git diff --stat` 无输出(暂存的版本被恢复)。

- [ ] **Step 9: 复核非 macOS 的 cfg 块里被改动的 hunk**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && git diff --cached -U10 -- crates | grep -nE "cfg\(not\(target_os|cfg\(target_os" `
Expected: 对每个命中的 cfg 块,读其上下文 hunk,确认被改的只是"窗格选择"的同形替换。若命中了 `#[cfg(not(target_os = "macos"))]` 块里的改写,逐行核对它与对应的 macOS 侧同形(这些块在本机不参与编译,编译器帮不上忙)。没有命中则本步通过。

- [ ] **Step 10: 门禁与提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1
RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-after.txt; git stash -q; RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-before.txt; git stash pop -q; git checkout -- Cargo.lock; diff $SCRATCH/clippy-before.txt $SCRATCH/clippy-after.txt && echo "clippy: no new diagnostics"
git status --short
git add crates/dozer-app/src/workspace crates/dozer-app/src/app crates/dozer-app/src/platform/window_events.rs
git diff --cached --stat | tail -8
git commit -m "refactor(workspace): route all preview-pane selection through Workspace::preview_pane[_mut]

75 hand-written 'if kind == PanelKind::Project { &project_preview } else { &preview }'
sites collapse into four accessors; behavior unchanged (non-Project kinds fall back to
the Files pane exactly as before). One site keeps per-field borrows on purpose.

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: `clippy: no new diagnostics`(诊断与基线逐文件一致);`git diff --cached --stat` 里只有上述 5 个文件(没有 `Cargo.lock`、没有 `scripts/`)。

---

### Task 2: `PanelHost` 面板视图契约,面板代码去掉 `App`/`Workspace`

**Files:**
- Create: `crates/dozer-app/src/panel_host.rs`
- Modify: `crates/dozer-app/src/main.rs`(`mod panel_host;`)
- Modify(脚本): `extensions/{conversations,git_log,ssh,usage/view,todo/view,database/view}.rs`
- Modify(手工): `extensions/{todo/view,todo/protocol,files/tree,ssh/sftp}.rs`、`app/{view,app,update}.rs`、`platform/todo_detail_overlay.rs`

**Interfaces:**
- Consumes: Task 1 提交(`bytehost-h1` 分支上)。
- Produces(后续切片依赖):`crate::panel_host::PanelHost`,6 个方法——`hover_progress(&self, HoverId) -> f32`、`hover_tooltip_ready(&self, HoverId) -> bool`、`list_collapsed(&self, PanelKind) -> bool`、`list_collapse_button<'a, M: Clone + 'a>(&self, PanelKind, bool, HoverId, &'a str, &'a str, M, impl Fn(bool) -> M + 'a) -> Element<'a, M, Theme, Renderer>`、`window_size(&self) -> (f32, f32)`、`last_cursor(&self) -> (f32, f32)`;`impl PanelHost for App`。改后的面板签名:`todo::view(app: &impl PanelHost, ws_state, webview_failed: Option<&str>, …)`、`files::tree_drag_ghost(host: &impl PanelHost, source: Option<(&Path, bool)>)`、`ssh::sftp::route(sftp_tabs: &mut HashMap<String, SftpTabState>, io, msg)`、`todo::protocol::{derive_states, current_view_payload}(todo: &WorkspaceState, session_alive: &dyn Fn(&str) -> bool, …)`、`todo::todo_detail_card(todo: &WorkspaceState)`。

- [ ] **Step 1: 写失败的测试(只有测试模块,trait 还不存在)**

创建 `crates/dozer-app/src/panel_host.rs`,内容**只有**:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use iced_widget::core::{Border, Length};
    use std::cell::Cell;

    /// 不依赖 `App` 的假宿主:证明"一个面板 + 宿主契约"就能构造出 view(规格 §6 第 5 条的第一个探针)。
    #[derive(Default)]
    struct FakeHost {
        collapse_buttons_asked: Cell<u32>,
    }

    impl PanelHost for FakeHost {
        fn hover_progress(&self, _id: HoverId) -> f32 {
            0.0
        }
        fn hover_tooltip_ready(&self, _id: HoverId) -> bool {
            false
        }
        fn list_collapsed(&self, _kind: PanelKind) -> bool {
            false
        }
        fn list_collapse_button<'a, M: Clone + 'a>(
            &self,
            _kind: PanelKind,
            _collapsed: bool,
            _hover_id: HoverId,
            _tooltip_collapse: &'a str,
            _tooltip_expand: &'a str,
            _on_select: M,
            _on_hover: impl Fn(bool) -> M + 'a,
        ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
            self.collapse_buttons_asked
                .set(self.collapse_buttons_asked.get() + 1);
            iced_widget::Space::new().into()
        }
        fn window_size(&self) -> (f32, f32) {
            (800.0, 600.0)
        }
        fn last_cursor(&self) -> (f32, f32) {
            (0.0, 0.0)
        }
    }

    #[test]
    fn usage_content_pane_builds_against_a_fake_host_and_asks_it_for_the_collapse_button() {
        let host = FakeHost::default();
        let ws_state = crate::extensions::usage::WorkspaceState::default();
        let _view = crate::extensions::usage::content_pane(
            &host,
            &ws_state,
            Length::Fill,
            Border::default(),
        );
        assert_eq!(host.collapse_buttons_asked.get(), 1);
    }
}
```

在 `crates/dozer-app/src/main.rs` 的 `mod osc;` 之后加一行 `mod panel_host;`。

- [ ] **Step 2: 确认测试失败**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app panel_host 2>&1 | grep -E "^error" -A3 | head -8; git checkout -- Cargo.lock`
Expected: 编译失败,`cannot find trait \`PanelHost\` in this scope`、`cannot find type \`HoverId\` in this scope`。

- [ ] **Step 3: 写 trait 与 `impl PanelHost for App`(放在测试模块之前)**

把 `crates/dozer-app/src/panel_host.rs` 改写成"下面这段 + 空行 + Step 1 的测试模块":

```rust
//! 面板 view 对宿主的**只读视图契约**:面板需要从宿主问的东西(悬停动画进度、列表列折叠态、
//! 窗口/光标位置、折叠按钮)集中在这一个 trait 里,面板代码写 `app: &impl PanelHost`,
//! 不再 import 宿主的 `App`。
//!
//! 这是 bytehost 面板边界的第一块契约(见 `docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md`
//! §2 的 E1/E2):trait 里不得出现任何面板的业务类型;`HoverId`/`PanelKind` 暂时仍是宿主类型,
//! 它们的命名空间化/注册制是后续切片(`docs/dozer-v2/bytehost-H0/00-summary.md` §4)。
//! 目前只有 `App` 一个实现;不为未来的第二个实现预先抽象更多方法——面板需要什么才加什么。

use crate::app::{App, HoverId, PanelKind};
use iced_widget::core::Element;

pub trait PanelHost {
    /// 某个按钮/页签的悬停动画进度 0.0..=1.0。
    fn hover_progress(&self, id: HoverId) -> f32;
    /// 悬停是否已持续满 tooltip 延迟(该弹标题全称 tooltip 了)。
    fn hover_tooltip_ready(&self, id: HoverId) -> bool;
    /// 两栏面板的列表列当前是否收起。
    fn list_collapsed(&self, kind: PanelKind) -> bool;
    /// 列表列的折叠/展开按钮(图标随面板所在栏位镜像)。
    #[allow(clippy::too_many_arguments)]
    fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        kind: PanelKind,
        collapsed: bool,
        hover_id: HoverId,
        tooltip_collapse: &'a str,
        tooltip_expand: &'a str,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 窗口逻辑尺寸 `(宽, 高)`。
    fn window_size(&self) -> (f32, f32);
    /// 最近一次光标位置 `(x, y)`(窗口坐标)。
    fn last_cursor(&self) -> (f32, f32);
}

impl PanelHost for App {
    fn hover_progress(&self, id: HoverId) -> f32 {
        App::hover_progress(self, id)
    }

    fn hover_tooltip_ready(&self, id: HoverId) -> bool {
        App::hover_tooltip_ready(self, id)
    }

    fn list_collapsed(&self, kind: PanelKind) -> bool {
        App::list_collapsed(self, kind)
    }

    fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        kind: PanelKind,
        collapsed: bool,
        hover_id: HoverId,
        tooltip_collapse: &'a str,
        tooltip_expand: &'a str,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
        App::list_collapse_button(
            self,
            kind,
            collapsed,
            hover_id,
            tooltip_collapse,
            tooltip_expand,
            on_select,
            on_hover,
        )
    }

    fn window_size(&self) -> (f32, f32) {
        self.window_size
    }

    fn last_cursor(&self) -> (f32, f32) {
        self.last_cursor
    }
}
```

(即文件 = trait 部分 + 原来的 `#[cfg(test)] mod tests { … }`。)

- [ ] **Step 4: 面板文件的签名与 import 脚本替换**

创建 `$SCRATCH/s2_panels.py` 并运行(6 个文件:把 `&App`/`&'a App`/`&crate::app::App` 换成 `&impl PanelHost`,改 `use`,`app.window_size`/`app.last_cursor` 字段改成契约方法;**正则带词边界,不会误伤 `AppState`**——草稿里朴素的字符串替换把 `&'a AppState` 改坏过一次):

```python
import re
files = ["extensions/conversations.rs", "extensions/database/view.rs", "extensions/git_log.rs",
         "extensions/ssh.rs", "extensions/usage/view.rs", "extensions/todo/view.rs"]
base = "crates/dozer-app/src/"
for f in files:
    p = base + f
    s = open(p).read()
    # 类型:&App / &'a App / &crate::app::App(带词边界,不误伤 AppState)
    s = re.sub(r"&('a )?(crate::app::)?App\b", lambda m: "&" + (m.group(1) or "") + "impl PanelHost", s)
    # import
    s = s.replace("use crate::app::App;\n", "use crate::panel_host::PanelHost;\n")
    s = s.replace("use crate::app::{App, HoverId};", "use crate::app::HoverId;\nuse crate::panel_host::PanelHost;")
    s = s.replace("use crate::app::{App, HoverId, ssh_tab_hover_key};",
                  "use crate::app::{HoverId, ssh_tab_hover_key};\nuse crate::panel_host::PanelHost;")
    # 字段 -> 契约方法
    s = s.replace("app.window_size,", "app.window_size(),").replace("= app.last_cursor;", "= app.last_cursor();")
    if "impl PanelHost" in s and "use crate::panel_host::PanelHost;" not in s:
        if "\nuse super::*;\n" in s:
            s = s.replace("\nuse super::*;\n", "\nuse super::*;\nuse crate::panel_host::PanelHost;\n", 1)
        else:
            s = s.replace("\nuse crate::", "\nuse crate::panel_host::PanelHost;\nuse crate::", 1)
    open(p, "w").write(s)
```

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && python3 $SCRATCH/s2_panels.py`

- [ ] **Step 5: 手工编辑 6 处"面板不该碰宿主"的点位**

创建 `$SCRATCH/s2_manual.py` 并运行:

```python
base = "crates/dozer-app/src/"
def edit(path, pairs):
    p = base + path
    s = open(p).read()
    for a, b in pairs:
        assert a in s, (path, a[:60])
        s = s.replace(a, b)
    open(p, "w").write(s)

# 1. todo::view 多一个 webview_failed 参数(原来读 app.todo_webview)
edit("extensions/todo/view.rs", [
("    ws_state: &'a WorkspaceState,\n    sidebar_width: Length,",
 "    ws_state: &'a WorkspaceState,\n    // 内容区 webview 宿主的失败原因(`App.todo_webview.failed()`,由调用方取了传进来:\n    // 面板 view 不再读宿主的 `App` 字段)。\n    webview_failed: Option<&'a str>,\n    sidebar_width: Length,"),
("todo_content_slot(app.todo_webview.failed())", "todo_content_slot(webview_failed)"),
("pub fn todo_detail_card(\n    ws: &Workspace,\n)", "pub fn todo_detail_card(\n    todo: &WorkspaceState,\n)"),
("use crate::workspace::Workspace;\n", ""),
])
s = open(base + "extensions/todo/view.rs").read()
i = s.index("pub fn todo_detail_card(")
open(base + "extensions/todo/view.rs", "w").write(s[:i] + s[i:].replace("ws.todo.", "todo."))
edit("app/view.rs", [
("""                return todo::view(
                    app,
                    &ws.todo,
                    Length::Fixed(0.0),""", """                return todo::view(
                    app,
                    &ws.todo,
                    app.todo_webview.failed(),
                    Length::Fixed(0.0),"""),
("""            let (sidebar_pane, content_pane) = todo::view(
                app,
                &ws.todo,
                Length::FillPortion(list_portion),""", """            let (sidebar_pane, content_pane) = todo::view(
                app,
                &ws.todo,
                app.todo_webview.failed(),
                Length::FillPortion(list_portion),"""),
("stack![with_maximize, files::tree_drag_ghost(self)]",
 "stack![\n                with_maximize,\n                files::tree_drag_ghost(\n                    self,\n                    self.active_workspace()\n                        .and_then(|ws| ws.files.tree_drag_ghost_source()),\n                )\n            ]"),
])
edit("platform/todo_detail_overlay.rs", [("todo::todo_detail_card(ws)", "todo::todo_detail_card(&ws.todo)")])

# 2. files::tree_drag_ghost:拖拽源由调用方传入
edit("extensions/files/tree.rs", [
("""pub(crate) fn tree_drag_ghost(
    app: &crate::app::App,
) -> Element<'_, crate::app::Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(ws) = app.active_workspace() else {
        return column![].into();
    };
    let Some((source, is_dir)) = ws.files.tree_drag_ghost_source() else {
        return column![].into();
    };""", """pub(crate) fn tree_drag_ghost<'a>(
    host: &impl crate::panel_host::PanelHost,
    // `WorkspaceState::tree_drag_ghost_source()` 的结果,由调用方取了传进来(面板 view 不再
    // 自己去宿主里翻"当前活动工作区")。
    source: Option<(&std::path::Path, bool)>,
) -> Element<'a, crate::app::Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some((source, is_dir)) = source else {
        return column![].into();
    };"""),
("let (cx, cy) = app.last_cursor;\n    let (window_w, window_h) = app.window_size;",
 "let (cx, cy) = host.last_cursor();\n    let (window_w, window_h) = host.window_size();"),
("不是 `files::view()` 的一部分,故吃 `&App` 不是 `&WorkspaceState`。",
 "不是 `files::view()` 的一部分,故吃宿主视图契约 `PanelHost` + 调用方取好的拖拽源,不是 `&WorkspaceState`。"),
])

# 3. ssh::sftp::route 只需要 sftp_tabs
edit("extensions/ssh/sftp.rs", [
("    ws: &mut crate::workspace::Workspace,\n    _io: &crate::workspace::ShellIo,",
 "    sftp_tabs: &mut HashMap<String, SftpTabState>,\n    _io: &crate::workspace::ShellIo,"),
])
s = open(base + "extensions/ssh/sftp.rs").read()
open(base + "extensions/ssh/sftp.rs", "w").write(s.replace("ws.sftp_tabs", "sftp_tabs"))
s = open(base + "app/update.rs").read()
s = s.replace("ssh::sftp::route(\n                            ws,", "ssh::sftp::route(\n                            &mut ws.sftp_tabs,")
s = s.replace("ssh::sftp::route(ws, io, msg);", "ssh::sftp::route(&mut ws.sftp_tabs, io, msg);")
open(base + "app/update.rs", "w").write(s)

# 4. todo::protocol:session 存活用一个查询闭包,不再吃 Workspace
s = open(base + "extensions/todo/protocol.rs").read()
a = s[s.index("/// 每张任务卡片的派生状态"):s.index("/// 纯函数,便于不构造 `Workspace` 地测试。")]
b = '''/// 每张任务卡片的派生状态(需要看派发的目标 session 是否存活,所以除了 `WorkspaceState` 还要
/// 一个"session 是否存活"的查询,由调用方按宿主的会话表给出——本模块不认识宿主的 `Workspace`)。
/// 顺序与 `todo.items` 一一对应。
pub(crate) fn derive_states(
    todo: &WorkspaceState,
    session_alive: &dyn Fn(&str) -> bool,
) -> Vec<TodoState> {
    todo.items()
        .iter()
        .map(|item| {
            let target_alive = item
                .dispatch_session_id
                .as_deref()
                .map(session_alive)
                .unwrap_or(false);
            todo_display_state(item, target_alive)
        })
        .collect()
}

pub(crate) fn current_view_payload(
    todo: &WorkspaceState,
    session_alive: &dyn Fn(&str) -> bool,
    project_id: i64,
    today: (i32, u32, u32),
) -> TodoViewPayload {
    build_payload(
        todo,
        &derive_states(todo, session_alive),
        project_id,
        today,
        agent_vms(),
    )
}

'''
s = s.replace(a, b).replace("use crate::workspace::{Workspace, agent_icon};", "use crate::workspace::agent_icon;")
open(base + "extensions/todo/protocol.rs", "w").write(s)
edit("app/app.rs", [("""        let desired = crate::extensions::todo::current_view_payload(
            ws,
            project_id,""", """        let desired = crate::extensions::todo::current_view_payload(
            &ws.todo,
            &|sid: &str| {
                ws.tabs
                    .iter()
                    .chain(ws.ssh_tabs.iter())
                    .any(|t| t.alive && t.info.id == sid)
            },
            project_id,""")])
```

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && python3 $SCRATCH/s2_manual.py`

各点位的改法与原因:
- `todo::view` 多一个 `webview_failed: Option<&str>` 参数(原来读 `app.todo_webview.failed()`,即面板 view 读宿主 `App` 的字段),`app/view.rs` 两处调用方传入;
- `files::tree_drag_ghost`:原来自己 `app.active_workspace()` 去宿主里翻拖拽源,改为调用方传 `Option<(&Path, bool)>`;
- `ssh::sftp::route`:只用到 `ws.sftp_tabs`,改吃 `&mut HashMap<String, SftpTabState>`,`app/update.rs` 里 3 个调用点传 `&mut ws.sftp_tabs`(其中一个在 `#[cfg(not(target_os = "macos"))]` 块里);
- `todo::protocol::{derive_states, current_view_payload}`:原来吃 `&Workspace` 只为问"派发目标 session 是否存活",改吃 `session_alive: &dyn Fn(&str) -> bool`,`app/app.rs` 里用 `ws.tabs`/`ws.ssh_tabs` 造闭包;
- `todo::todo_detail_card`:原来吃 `&Workspace` 但只用 `ws.todo`,改吃 `&WorkspaceState`。

- [ ] **Step 6: 格式化、编译、全量测试**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1
cargo fmt -p dozer-app && git status --short
RUSTFLAGS="-D unconditional_recursion" cargo build -p dozer-app 2>&1 | grep -E "^(error|warning: (unused|function cannot))" -A10 | head -40
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^error|^test result|FAILED"
git checkout -- Cargo.lock
```
Expected: `git status --short` 里是 `panel_host.rs`(新)、`main.rs` 和上面列的面板/调用方文件,**没有别的**;build 无输出;通过数 = Task 1 之后的数 + 1(`usage_content_pane_builds_against_a_fake_host…`),失败只有已知项。若 build 报别的 `&App` 用法(`main` 上有新增面板代码):按同样思路——读它到底问宿主什么,属于 6 个方法之一就套 `PanelHost`,否则按 Step 5 的做法改成调用方传参;**不要往 trait 里加方法,除非面板确实在问且没有别的办法**。

- [ ] **Step 7: 面板边界门禁应降到 0**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && PYTHONDONTWRITEBYTECODE=1 python3 scripts/audit/check_panel_boundary.py; python3 - <<'EOF'
import sys, importlib.util
sys.dont_write_bytecode = True
s = importlib.util.spec_from_file_location("g", "scripts/audit/check_panel_boundary.py")
g = importlib.util.module_from_spec(s); s.loader.exec_module(g)
print(g.scan(g.load_sources()))
EOF`
Expected: 第一行 `panel boundary check: ok (0 refs in 0 files, baseline ratchet)`,第二行 `{}`(旧基线 22 → 0;Task 3 才更新基线文件,所以此刻门禁对着旧基线仍是 ok)。

- [ ] **Step 8: 变异检验:加回一个 `&App` 必须被门禁抓到**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1
git add crates && git status --short | wc -l
printf '\nuse crate::app::App;\nfn __probe(a: &App) {}\n' >> crates/dozer-app/src/extensions/usage/view.rs
PYTHONDONTWRITEBYTECODE=1 python3 - <<'EOF'
import sys, importlib.util
sys.dont_write_bytecode = True
s = importlib.util.spec_from_file_location("g", "scripts/audit/check_panel_boundary.py")
g = importlib.util.module_from_spec(s); s.loader.exec_module(g)
print(g.scan(g.load_sources()))
EOF
git checkout -- crates/dozer-app/src/extensions/usage/view.rs && git diff --stat | tail -1
```
Expected: 变异后打印 `{'extensions/usage/view.rs': {'R-APP': 2}}`;还原后 `git diff --stat` 无输出。

- [ ] **Step 9: 复核非 macOS 的 cfg 块**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && git diff --cached -U12 -- crates/dozer-app/src/app/update.rs | grep -nE "cfg\(not\(target_os" `
Expected: 命中 `ssh::sftp::route` 的非 macOS 分支;读该 hunk,确认只是 `ws,` → `&mut ws.sftp_tabs,`,与 macOS 分支同形。

- [ ] **Step 10: clippy 对比与提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1
RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-after2.txt; git checkout -- Cargo.lock
diff $SCRATCH/clippy-after.txt $SCRATCH/clippy-after2.txt && echo "clippy: no new diagnostics"
git status --short
git add crates/dozer-app/src/panel_host.rs crates/dozer-app/src/main.rs crates/dozer-app/src/extensions crates/dozer-app/src/app crates/dozer-app/src/platform/todo_detail_overlay.rs
git diff --cached --stat | tail -16
git commit -m "refactor(panels): introduce PanelHost and drop App/Workspace from panel code

Panels now take '&impl PanelHost' (hover progress, list collapse, window/cursor) instead of
the host's App; four call sites that only needed a slice of the host state take that slice
as a parameter (todo webview failure, tree drag source, sftp tabs, session-alive query,
todo detail state). A fake-host test builds a panel view without App. Panel-boundary
baseline drops from 22 to 0.

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: `clippy: no new diagnostics`;`git diff --cached --stat` 里没有 `Cargo.lock`、没有 `scripts/`。

---

### Task 3: 门禁新增 `R-PANE-PICK`、基线更新、回填 H0 文档

**Files:**
- Modify: `scripts/audit/test_check_panel_boundary.py`、`scripts/audit/check_panel_boundary.py`、`scripts/audit/panel-boundary.baseline.json`
- Modify: `docs/dozer-v2/bytehost-H0/{00-summary,01-panelkind,02-panel-to-host}.md`、`docs/dozer-v2/bytehost-H0/E3-registry.tsv`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`

**Interfaces:**
- Consumes: Task 1、2 的提交。
- Produces: 门禁新规则 `R-PANE-PICK`(全库;命中 `== / != PanelKind::Project` 与"取 `project_preview` 的 match 臂";棘轮,只许减不许增)。基线:`R-APP`/`R-WS` 清零,`R-PANE-PICK` 剩 7 处 / 4 个文件。

- [ ] **Step 1: 写失败的测试**

在 `scripts/audit/test_check_panel_boundary.py` 里 `class Compare(unittest.TestCase):` 之前加入:

```python
class PanePick(unittest.TestCase):
    """R-PANE-PICK:全库里手写"按 PanelKind::Project 选预览窗格"的写法只许减不许增(H1 切片 1 的回退防线)。"""
    def scan1(self, rel, text):
        return g.scan({rel: text}).get(rel, {})
    def test_if_eq_project_counts_anywhere(self):
        self.assertEqual(self.scan1("workspace/state.rs", "if kind == PanelKind::Project { a } else { b }"), {"R-PANE-PICK": 1})
    def test_match_arm_selecting_project_preview_counts(self):
        text = "match kind { PanelKind::Project => &mut ws.project_preview, _ => &mut ws.preview }"
        self.assertEqual(self.scan1("app/update.rs", text), {"R-PANE-PICK": 1})
    def test_other_project_arms_do_not_count(self):
        self.assertEqual(self.scan1("chrome/rail.rs", 'PanelKind::Project => (icon, "项目"),'), {})
    def test_comments_do_not_count(self):
        self.assertEqual(self.scan1("a.rs", "// if kind == PanelKind::Project {"), {})
```

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && PYTHONDONTWRITEBYTECODE=1 python3 scripts/audit/test_check_panel_boundary.py 2>&1 | grep -E "^(FAIL|ERROR)|^Ran|^FAILED|^OK"`
Expected: `FAIL: test_if_eq_project_counts_anywhere`、`FAIL: test_match_arm_selecting_project_preview_counts`(另两个本来就该过),`FAILED (failures=2)`。

- [ ] **Step 2: 实现规则**

用下面的文件**整体替换** `scripts/audit/check_panel_boundary.py`(在 H0 版本上加了 `PANE_PICK_RE` 与 `scan` 里的全库分支,注释与措辞同步更新):

```python
#!/usr/bin/env python3
"""面板边界棘轮门禁(报告模式):面板代码(extensions/**)里 `App`/`Workspace` 的引用只许减少、不许增加。

规则 R-APP:  extensions 下文件对 `crate::app::App` 的**使用次数**(import 行 + 每个 `&App` 参数/限定路径)
规则 R-WS:   extensions 下文件对 `crate::workspace::Workspace` 的使用次数
规则 R-PANE-PICK: 全库手写"按 PanelKind::Project 选预览窗格"的写法条数(应走 `Workspace::preview_pane[_mut]`)

用法:
  check_panel_boundary.py            对照基线检查,有文件的引用数上升(或新文件出现违规)则退出 1
  check_panel_boundary.py --update   用当前扫描结果重写基线(只在引用数下降或经评审的迁移后使用)
基线:scripts/audit/panel-boundary.baseline.json,格式 {"<文件>": {"R-APP": n, "R-WS": n, "R-PANE-PICK": n}}。
测试代码里的引用同样计入:H0 阶段不区分生产与测试,迁移完成后再评估是否放宽测试。
"""
import collections, importlib.util, json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("edges", os.path.join(HERE, "edges.py"))
edges = importlib.util.module_from_spec(_spec); _spec.loader.exec_module(edges)

BASELINE = os.path.join(HERE, "panel-boundary.baseline.json")
RULES = {"R-APP": ("app", "App"), "R-WS": ("workspace", "Workspace")}


STRING_RE = re.compile(r'"(?:\\.|[^"\\])*"')
SUPER_RE = re.compile(r"\buse\s+super::(?:super::)+([a-z_]+)::([A-Za-z_]+)")


def count_uses(text, mod, name):
    """文件里对 `crate::<mod>::<name>` 的**使用次数**(含 import 那一行)。

    - 按名字 import(`use crate::app::App`、`use crate::app::{App, ..}`、`use super::super::app::App`)
      后,数该名字(及 `as` 别名)的全部词出现;
    - 只 import 模块(`use crate::app;`、`use crate::app as host;`)或不 import 时,数 `app::App` /
      `host::App` 这种限定路径的出现。
    注释与字符串字面量不计。import 路径数(旧口径)会漏掉同一文件里的多个 `&App` 参数。
    """
    text = STRING_RE.sub('""', edges.strip_comments(text))
    found = collections.Counter()
    aliases = {name}
    mod_aliases = {mod}
    for m in edges.USE_RE.finditer(text):
        body = m.group(1)
        for path in edges.flatten(body):
            seg = path.split("::")
            if seg[0] == mod and len(seg) > 1 and seg[1] == name:
                found["named"] += 1
        for a in re.finditer(r"\b" + name + r"\s+as\s+([A-Za-z_][A-Za-z0-9_]*)", body):
            aliases.add(a.group(1))
        for a in re.finditer(r"^" + mod + r"\s+as\s+([A-Za-z_][A-Za-z0-9_]*)", body.strip()):
            mod_aliases.add(a.group(1))
    for m in SUPER_RE.finditer(text):
        if m.group(1) == mod and m.group(2) == name:
            found["named"] += 1
    n = 0
    if found["named"]:
        outside_use = edges.USE_RE.sub("", text)
        for a in aliases:
            hay = text if a == name else outside_use  # 别名在 import 行里不重复计
            n += len(re.findall(r"\b" + re.escape(a) + r"\b", hay))
    else:
        for ma in mod_aliases:
            n += len(re.findall(r"\b" + re.escape(ma) + r"::" + re.escape(name) + r"\b", text))
    return n


# 全库规则:手写"按 PanelKind::Project 选预览窗格"的写法(应走 `Workspace::preview_pane[_mut]`)。
# 命中 `== / != PanelKind::Project` 与"取 project_preview 的 match 臂";窗口层对 `right_view`
# 的比较等合理写法也会命中,所以用基线棘轮(只许减不许增),不是零容忍。
PANE_PICK_RE = re.compile(
    r"PanelKind::Project\s*=>\s*&(?:mut\s+)?[\w\.]+\.project_preview\b"
    r"|(?:==|!=)\s*(?:crate::app::)?PanelKind::Project\b"
)


def scan(files):
    """files: {相对路径: 源码文本} -> {相对路径: {规则: 次数}},只含 extensions/ 下且有违规的文件。"""
    out = {}
    for rel, text in files.items():
        hit = {}
        if rel.startswith("extensions/"):
            for rule, (mod, name) in RULES.items():
                n = count_uses(text, mod, name)
                if n:
                    hit[rule] = n
        n = len(PANE_PICK_RE.findall(edges.strip_comments(text)))
        if n:
            hit["R-PANE-PICK"] = n
        if hit:
            out[rel] = hit
    return out


def compare(baseline, current):
    """返回问题列表:引用数高于基线(或基线中没有)的 (文件, 规则, 基线, 当前)。"""
    problems = []
    for rel, hit in sorted(current.items()):
        for rule, n in sorted(hit.items()):
            old = baseline.get(rel, {}).get(rule, 0)
            if n > old:
                problems.append((rel, rule, old, n))
    return problems


def load_sources():
    files = {}
    for dp, _, fns in os.walk(edges.SRC):
        for fn in fns:
            if fn.endswith(".rs"):
                p = os.path.join(dp, fn)
                with open(p, encoding="utf-8") as f:
                    files[os.path.relpath(p, edges.SRC).replace(os.sep, "/")] = f.read()
    return files


def main(argv):
    current = scan(load_sources())
    if "--update" in argv:
        with open(BASELINE, "w", encoding="utf-8") as f:
            json.dump(current, f, ensure_ascii=False, indent=1, sort_keys=True)
            f.write("\n")
        print(f"baseline updated: {sum(sum(h.values()) for h in current.values())} refs in {len(current)} files")
        return 0
    with open(BASELINE, encoding="utf-8") as f:
        baseline = json.load(f)
    problems = compare(baseline, current)
    if problems:
        print("面板边界回退(引用/写法条数高于基线):", file=sys.stderr)
        for rel, rule, old, n in problems:
            print(f"  {rel}: {rule} {old} -> {n}", file=sys.stderr)
        return 1
    total = sum(sum(h.values()) for h in current.values())
    print(f"panel boundary check: ok ({total} refs in {len(current)} files, baseline ratchet)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
```

Run: `PYTHONDONTWRITEBYTECODE=1 python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -3`
Expected: `Ran 15 tests` … `OK`。

- [ ] **Step 3: 更新基线并核对**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && export PYTHONDONTWRITEBYTECODE=1 && python3 scripts/audit/check_panel_boundary.py; python3 scripts/audit/check_panel_boundary.py --update && cat scripts/audit/panel-boundary.baseline.json && python3 scripts/audit/check_panel_boundary.py`
Expected: 第一次检查对着旧基线报 `app/layout.rs: R-PANE-PICK 0 -> 1`、`app/update.rs … 0 -> 1`、`platform/window_events.rs … 0 -> 3`、`workspace/state.rs … 0 -> 2`(新规则的现存命中,正是预期的剩余点位);`--update` 输出 `baseline updated: 7 refs in 4 files`;基线 JSON 里**不再有 `R-APP`/`R-WS`**,只有这 4 个文件的 `R-PANE-PICK`;最后一次检查 `ok`。

这 7 处剩余各是什么(都是合理的,不应被"收口"):`app/layout.rs:715` 的 `side != PanelKind::Project.default_side()`(布局镜像判断)、`app/update.rs` 里 `PreviewLargeFileSearchGo` 保留的按字段 match、`platform/window_events.rs` 3 处 `app.right_view == PanelKind::Project`(窗口层命中判断)、`workspace/state.rs` 里两个访问器自己的 match 臂。

- [ ] **Step 4: 变异检验**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && export PYTHONDONTWRITEBYTECODE=1
git add scripts/audit
printf '\nfn __probe(kind: PanelKind) -> bool { kind == PanelKind::Project }\n' >> crates/dozer-app/src/extensions/todo/mod.rs
python3 scripts/audit/check_panel_boundary.py; echo "exit=$?"
git checkout -- crates/dozer-app/src/extensions/todo/mod.rs; python3 scripts/audit/check_panel_boundary.py; echo "exit=$?"; git status --short crates | wc -l
```
Expected: 变异后 `extensions/todo/mod.rs: R-PANE-PICK 0 -> 1` 且 `exit=1`;还原后 `ok`、`exit=0`、最后一条输出 `0`。

- [ ] **Step 5: 回填文档**

对 `docs/dozer-v2/bytehost-H0/` 与要求文档做下列**精确**修改(每条都先 `grep -n` 找到原文再改,改完 `git diff` 看一遍):
1. `00-summary.md` §4 的候选 1、2 两行,在"涉及"列后追加"**已完成(H1,`bytehost-h1`):<提交短 id>**";§5 门禁说明里"当前 22 处 / 9 个文件"改为"`R-APP`/`R-WS` 已清零;新增全库规则 `R-PANE-PICK`(手写按 `PanelKind::Project` 选预览窗格),基线 7 处 / 4 个文件",并把"识别的写法"一句里补上 `R-PANE-PICK` 的正则含义。
2. `01-panelkind.md` §6 的 B0 行末追加"**已完成(H1):75 处机械重写 + 4 个访问器 + 4 处手工编辑;剩余 7 处是合理写法(见基线)**"。
3. `02-panel-to-host.md` §3.1 末尾追加一段"**H1 结果:**`PanelHost`(`crates/dozer-app/src/panel_host.rs`,6 个方法)已落地,面板代码对 `App`/`Workspace` 的使用降为 0;`HoverId`/`PanelKind`/`TextInputTarget` 仍是宿主类型,留给后续切片。"
4. `E3-registry.tsv`:删除 `E3-017`(`extensions/files/tree.rs:145` 的 `app.active_workspace()`,已消除);`E3-004` 的"对象/原因"列改为"约 82 处中 75 处已收口为 `preview_pane[_mut]`;剩余按字段保留 1 处(借用冲突)与 Preview 内部的 webview id/URL 编码(见 E3-005)"。改完运行 `awk -F'\t' 'NR>1 && (NF!=6 || $6=="")' docs/dozer-v2/bytehost-H0/E3-registry.tsv | wc -l`,Expected: `0`。
5. `docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` §7:在 H0 那条之后追加一条"**bytehost H1 已完成(2026-10-04):** `Workspace::preview_pane[_mut]` 收口 75 处预览窗格选择;`PanelHost` 契约落地,面板代码不再引用 `App`/`Workspace`(门禁基线 22 → 0);`HoverId` 命名空间化、`window_events.rs` 面板事件声明化留给后续切片。"

- [ ] **Step 6: 最终校验并提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h1 && export PYTHONDONTWRITEBYTECODE=1
python3 scripts/audit/test_edges.py 2>&1 | tail -1; python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -1; python3 scripts/audit/check_panel_boundary.py
git status --short
git add scripts/audit/check_panel_boundary.py scripts/audit/test_check_panel_boundary.py scripts/audit/panel-boundary.baseline.json docs/dozer-v2
git diff --cached --stat | tail -10
git commit -m "chore(audit): ratchet preview-pane selection (R-PANE-PICK); panel App/Workspace baseline to 0

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: 两组测试 `OK`、门禁 `ok`;`git diff --cached --stat` 里只有 `scripts/audit/` 与 `docs/`(没有 `crates/`、`Cargo.lock`)。

- [ ] **Step 7: 回到主 checkout 验证已知基线失败不是本计划造成的**

在**合并之后**、`main` 主 checkout 里(不是 linked worktree)跑一次:`cd ~/Projects/CoralProjects/byteboy/dozer && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^test result|FAILED"`
Expected: 失败只剩 `delete_confirm_spec_reflects_pending_target`(以及偶发的 `serves_vendored_asset_with_mime`);`git_log::tests::build_…` 两个在主 checkout 通过。**若它们在主 checkout 也失败,先对比 `git stash` 掉合并前后的结果再下结论。**

---

## Self-Review

**1. 覆盖(H1 候选 1、2 → 任务):** 候选 1(`preview_pane(kind)` 收口)→ Task 1;候选 2(面板引用 `App`/`Workspace` 降到 0)→ Task 2;门禁与文档回填 → Task 3。H0 汇总 §4 里 3、4、5 号候选(`window_events.rs` 声明化、`HoverId` 命名空间化、"一个面板 + host 可运行"夹具的完整形态)**不在本计划**——但 Task 2 的假宿主测试是 5 号候选(O12)的第一个探针(证明一个面板 view 能脱离 `App` 构造),已在回填文案里点明。

**2. 占位符扫描:** 所有脚本与 Rust 代码为草稿里跑通的完整版本;Task 3 Step 5 的文档修改给的是"找到原文再改"的精确指令和要写入的文字,不是"以后再补"。

**3. 类型一致性:** `preview_pane`/`preview_pane_mut`/`preview_error_mut`/`preview_tab_first_mut` 在 Task 1 定义,Task 1 的手工编辑与测试、Task 2 无依赖;`PanelHost` 的 6 个方法签名在 Task 2 的 trait、`impl PanelHost for App`、假宿主测试里一致;`todo::view` 新增的 `webview_failed` 参数位置在 Task 2 Step 5 的脚本与 `app/view.rs` 两处调用里一致。

**4. Review Focus:** 5 条各有归属(1→Task 1 Step 6/7、2→Task 1 Step 5/8、3→Task 1 Step 3、4→Task 1 Step 9 与 Task 2 Step 9、5→Task 2 Step 1)。
