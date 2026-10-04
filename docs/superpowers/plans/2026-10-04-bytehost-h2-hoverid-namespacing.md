# bytehost H2:`HoverId` 命名空间化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让宿主的悬停键枚举 `HoverId` 不再点名任何面板的按钮:把其中 **51 个"某面板的某个按钮"变体**(`TodoListCollapse`、`FilesSearchSubmit`、`DatabaseTabItem`、`ProjectPreviewFindPrev`…)换成一个通用变体 `HoverId::Panel(PanelKind, HoverSlot)`,`HoverSlot` 是不含面板名的槽位词汇(`ListCollapse`/`SearchSubmit`/`More`/`TabItem(u64)`/`TabClose(u64)`/`TabOverflow`/`Row(u64)`/`Choice(u64)`/`Named(&'static str)`)。**不改任何用户可见行为**(悬停动画、tooltip、拖拽换位后的键重映射都不变),并顺带去掉 host 对 `git_log::FileFilter` 的类型依赖。这是 H0 汇总候选 4(`docs/dozer-v2/bytehost-H0/00-summary.md` §4、E3-013、E3-016)。

**Architecture:** 两个任务。Task 1 用三个一次性脚本做完整个改动(重写 126 处引用 → 改枚举定义并加 helper 构造函数 → 订正 13 处注释),测试先行(键不碰撞测试);编译器是主要裁判。Task 2 把门禁扩一条"枚举变体数只许减不许增"的规则(`HoverId`/`HoverSlot`),基线更新,回填 H0 文档。

**Tech Stack:** Rust(`dozer-app`,iced 0.14);Python 3 标准库(一次性重写脚本 + 审计门禁)。不碰 `Cargo.toml`/`Cargo.lock`。

**Spec:** `docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md`(E2:宿主公共类型里不出现业务类型);依据 `docs/dozer-v2/bytehost-H0/02-panel-to-host.md` §3.2(HoverId)、`E3-registry.tsv`(E3-013、E3-016)、H1 计划与成果(`PanelHost` 在 `crates/dozer-app/src/panel_host.rs`)。

## Global Constraints

- **行为保持:** 键的**相等性关系**必须与改前完全一致——旧的每个变体对应一个新键,不同旧变体对应不同新键(单射)。`Project` 与 `Files` 的两套预览按钮(旧 `PreviewX` / `ProjectPreviewX`)在新键里靠 `PanelKind` 区分,不能互相碰撞。
- **在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytehost-h2/...`),分支 `bytehost-h2`,从当前 `main` 开(H1 已在 `main` 上:`PanelHost`、`preview_pane`、门禁 `R-PANE-PICK` 都在)。主 checkout 常有并发会话与未提交改动(此刻 `Cargo.lock` 就是脏的),不得在其上改代码或 `git add -A`;只 `git add` 指定路径;提交前 `git status` 与 `git diff --cached --stat` 看全貌。
- **`Cargo.lock`:** 本地 `.cargo/config.toml` 的 `[patch]` 会让 `Cargo.lock` 里 `byteui`/`bytegit` 的 `source = …` 行消失。**每次提交前 `git checkout -- Cargo.lock`**;`.cargo/config.toml` 不提交;本计划不改依赖。
- **变异检验前先 `git add` 暂存当前版本**,变异后用 `git checkout -- <文件>` 还原到暂存版本(未暂存的已修改文件直接 `git checkout` 会把整个迁移改动一起冲掉——草稿里踩过一次)。
- **每次改 Rust 后跑 `cargo fmt -p dozer-app`,再用 `git status --short` 确认只有本任务触碰的文件变了。**
- **编译与测试一律带 `RUSTFLAGS="-D unconditional_recursion"`**(沿用 H1 的防线)。
- **clippy 诊断必须与 `main` 逐文件一致:** 草稿里重写脚本曾给整数字面量也加了 `as u64`,触发 `clippy::unnecessary_cast`(14 条);脚本现在对整数字面量与已是 `u64` 的实参不加 `as`。
- 提交信息用 `refactor:`/`test:`/`chore(audit):`/`docs:` 前缀;末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## 已知基线

- `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 稳定失败;`extensions::git_log::tests::build_marks_head_branch_and_labels`、`build_populates_time_and_is_merge` 在 linked worktree 里可能失败(对 dozer 自己的仓库跑 `gleisbau`);`assets::tests::serves_vendored_asset_with_mime` 偶发失败。
- 草稿基线(`main` = H1 之后,linked worktree):`1729 passed; 1 failed`。草稿里 Task 1 之后 `1732 passed; 1 failed`(+3 个新测试)。执行时以 Task 1 Step 2 实测为准。

## 映射表:旧变体 → 新键(51 个)

**留在 `HoverId` 里的 11 个宿主自己的变体(不动):** `Topbar`、`Rail`、`ProjectTabClose`、`ProjectTabItem`(项目页签,宿主顶栏)、`TermTabItem`、`TermTabClose`、`TermTabOverflow`(终端页签)、`HomeTab`、`HomeProjectSearchSubmit`、`HomeProjectMore`(首页)、`TabOverflowRow`(溢出下拉行,宿主的共享控件)。

**换成 `HoverId::Panel(PanelKind, HoverSlot)` 的 51 个**(括号里是构造函数写法;`Files`/`Project` 指 `PanelKind` 变体):

| 面板 | 旧变体 → 新键 |
|---|---|
| Files(15) | `PreviewTabItem(i)`→`tab_item(Files, i)`;`PreviewTabClose(i)`→`tab_close(Files, i)`;`PreviewTabOverflow`→`tab_overflow(Files)`;`PreviewRenderMode`→`named(Files,"render_mode")`;`PreviewTabularMode`→`"tabular_mode"`;`PreviewJsonMode`→`"json_mode"`;`PreviewFindPrev`→`"find_prev"`;`PreviewFindNext`→`"find_next"`;`PreviewFindReplaceToggle`→`"find_replace_toggle"`;`PreviewFindReplaceCurrentBtn`→`"find_replace_current"`;`PreviewFindReplaceAllBtn`→`"find_replace_all"`;`FilesSearchSubmit`→`search_submit(Files)`;`FilesDotfiles`→`named(Files,"dotfiles")`;`FilesBranchSwitch`→`named(Files,"branch_switch")`;`FileTreeCollapse`→`list_collapse(Files)` |
| Project(15) | `ProjectPreview*` 那 11 个→与上面 Files 的同名槽位,面板换成 `Project`;`ProjectDocsAdd`→`named(Project,"docs_add")`;`ProjectRemoteAdd`→`"remote_add"`;`ProjectMemoryAdd`→`"memory_add"`;`ProjectListCollapse`→`list_collapse(Project)` |
| Ssh(5) | `SshTabItem(k)`→`tab_item(Ssh, k)`;`SshTabClose(k)`→`tab_close(Ssh, k)`;`SshTabOverflow`→`tab_overflow(Ssh)`;`SshListCollapse`→`list_collapse(Ssh)`;`HostCard(k)`→`row(Ssh, k)` |
| Database(4) | `DatabaseListCollapse`→`list_collapse(Database)`;`DatabaseTabItem(i)`→`tab_item(Database, i)`;`DatabaseTabClose(i)`→`tab_close(Database, i)`;`DatabaseTabOverflow`→`tab_overflow(Database)` |
| Todo(2) | `TodoListCollapse`→`list_collapse(Todo)`;`TodoCategoryRow(id)`→`row(Todo, id as u64)` |
| Agent(2) | `AgentListCollapse`→`list_collapse(Agent)`;`AgentPickerToggle`→`named(Agent,"picker_toggle")` |
| Conversations(3) | `ConversationsListCollapse`→`list_collapse(Conversations)`;`ConversationListMore`→`more(Conversations)`;`ConversationSearchSubmit`→`search_submit(Conversations)` |
| Usage(1) | `UsageListCollapse`→`list_collapse(Usage)` |
| GitLog(4) | `CommitListMore`→`more(GitLog)`;`GitFile(i)`→`row(GitLog, i)`;`GitFileFilter(f)`→`choice(GitLog, f as u64)`;`GitLogSearchSubmit`→`search_submit(GitLog)` |

(`Project` 预览页签的旧 `ProjectPreviewTabItem` 映射成 `tab_item(Project, i)`——Project 面板只有一条页签栏,就是它的预览;所以这个槽位名不会与 Project 别的元素冲突。)

## Review Focus

1. **键碰撞(行为回归的唯一来源)。** 单射性由映射表保证,由 Task 1 的三个测试固定:同一槽位不同面板不同键、同一面板不同槽位/key 不同键、`FileFilter` 各值映射到不同 `Choice` 键。**风险点:** `TodoCategoryRow(i64)` 用 `as u64`(位转换,单射)、`BLANK_HOVER_KEY = usize::MAX` 转 `u64::MAX`(与真实下标不冲突,同改前)、`FileFilter as u64` 要求它是无载荷枚举(编译期保证)。
2. **函数指针参数。** `rekey_hover_range`、`dehover_after_tab_close`、`shift_index_keys_after_close` 吃 `fn(usize) -> HoverId`,旧代码直接传 `HoverId::PreviewTabItem`。脚本把这类裸变体名改写成不捕获环境的闭包 `|i| HoverId::tab_item(PanelKind::Files, i as u64)`(可以协变成 `fn` 指针)。H1 前就有的 3 个 `shift_index_keys_after_close_*` 测试和拖拽换位测试覆盖它们,Task 1 Step 8 必须看到它们通过。
3. **脚本没改的写法。** 注释里的 `HoverId::Old` 脚本不改(避免误伤);Task 1 Step 6 的 13 处注释订正与 Step 9 的残留 `grep` 保证文档不指向已删除的变体。`Message::CommitListMore` 这类**面板自己 `Message` 枚举**的同名变体不是 `HoverId` 的,不能被改——脚本只认 `HoverId::` 前缀。
4. **非 macOS 代码路径从未在本机编译过。** 草稿里 diff 命中的 `#[cfg(target_os = …)]` 只有 1 处且在 macOS 侧、与改动无关;Step 9 要求你重新跑一次这条检查,若命中了 `cfg(not(target_os = "macos"))` 块里的改写,逐行核对。
5. **`HoverSlot::Named(&'static str)` 的字符串键靠约定唯一。** 同一面板内重名会让两个按钮共享悬停动画——编译器抓不到。Task 1 的测试只抽样断言;Task 2 的门禁只数变体不数名字。**接受这个风险**,理由:名字都由本计划的映射表一次性给定、同面板内已人工核对唯一(表中无重名);以后新增按钮由 code review 把关。

---

## File Structure

| 路径 | 职责 | 任务 |
|---|---|---|
| `crates/dozer-app/src/panel_host.rs` | 新增 `HoverSlot` 枚举;新增 3 个测试 | 1 |
| `crates/dozer-app/src/app/state.rs` | `HoverId` 删 51 个变体、加 `Panel(PanelKind, HoverSlot)`、加 9 个 helper 构造函数;去掉 `use …git_log::FileFilter` | 1 |
| `crates/dozer-app/src/{app/app,app/update,app/view,workspace/view,term/terminal}.rs`、`extensions/{conversations,git_log,ssh,database/view,todo/view,usage/view}.rs` | 126 处引用改写 | 1 |
| `crates/dozer-app/src/{app/message,app/state,app/update,app/view,workspace/view,chrome/tab_widget}.rs`、`extensions/{database/update,database/state,git_log,files/state}.rs` | 13 处注释订正 | 1 |
| `scripts/audit/check_panel_boundary.py`、`test_check_panel_boundary.py`、`panel-boundary.baseline.json` | 新增枚举变体数规则,基线更新 | 2 |
| `docs/dozer-v2/bytehost-H0/{00-summary,02-panel-to-host}.md`、`E3-registry.tsv`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` | 回填 H2 结果 | 2 |

一次性脚本(**不提交**)放 `$SCRATCH`(仓库外)。

---

### Task 1: `HoverSlot`、`HoverId::Panel` 与 126 处引用改写

**Files:**
- Modify: `crates/dozer-app/src/panel_host.rs`(测试 + `HoverSlot`)、`crates/dozer-app/src/app/state.rs`(枚举 + helper)
- Modify(脚本): 见上表 11 个文件;注释订正 10 个文件

**Interfaces:**
- Produces(Task 2 与后续切片依赖):`crate::panel_host::HoverSlot`(9 个变体);`HoverId::Panel(PanelKind, HoverSlot)`;`HoverId::{list_collapse, search_submit, more, tab_overflow}(panel: PanelKind)`、`HoverId::{tab_item, tab_close, row, choice}(panel: PanelKind, key: u64)`、`HoverId::named(panel: PanelKind, name: &'static str)`(均 `pub(crate)`,返回 `HoverId`)。`HoverId` 之后共 12 个变体。

- [ ] **Step 1: 建 worktree、分支、一次性脚本目录**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytehost-h2 -b bytehost-h2 main
mkdir -p ../dozer-bytehost-h2/.cargo && cp .cargo/config.toml ../dozer-bytehost-h2/.cargo/config.toml
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2 && git log --oneline | head -1 && git status --short
export SCRATCH=~/Projects/CoralProjects/byteboy/dozer-bytehost-h2-scratch && mkdir -p $SCRATCH
```

Expected: 干净、分支 `bytehost-h2`。**此后所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytehost-h2/` 前缀,且每个新 shell 先 `export SCRATCH=…`。**

- [ ] **Step 2: 记录基线**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2 && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^test result|FAILED" | tee $SCRATCH/baseline.txt; git checkout -- Cargo.lock`
Expected: 失败只有"已知基线"里的;记下通过数 N(草稿:1729)。

再记 clippy 基线:`RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-before.txt; git checkout -- Cargo.lock`(草稿:11 个文件有既有诊断)。

- [ ] **Step 3: 写失败的测试**

在 `crates/dozer-app/src/panel_host.rs` 的 `mod tests { … }` 里、最后一个测试之后追加(`mod tests` 已有 `use super::*;`):

```rust
    // ---- H2:HoverId 的面板作用域键 ----

    use crate::extensions::git_log::FileFilter;

    #[test]
    fn same_slot_in_different_panels_is_a_different_key() {
        assert_ne!(
            HoverId::tab_item(PanelKind::Files, 1),
            HoverId::tab_item(PanelKind::Project, 1)
        );
        assert_ne!(
            HoverId::named(PanelKind::Files, "find_prev"),
            HoverId::named(PanelKind::Project, "find_prev")
        );
        assert_ne!(
            HoverId::list_collapse(PanelKind::Todo),
            HoverId::list_collapse(PanelKind::Usage)
        );
    }

    #[test]
    fn different_slots_and_keys_in_one_panel_are_different_keys() {
        let p = PanelKind::Database;
        let all = [
            HoverId::list_collapse(p),
            HoverId::search_submit(p),
            HoverId::more(p),
            HoverId::tab_overflow(p),
            HoverId::tab_item(p, 0),
            HoverId::tab_item(p, 1),
            HoverId::tab_close(p, 0),
            HoverId::row(p, 0),
            HoverId::choice(p, 0),
            HoverId::named(p, "a"),
            HoverId::named(p, "b"),
        ];
        let set: std::collections::HashSet<_> = all.iter().copied().collect();
        assert_eq!(set.len(), all.len());
    }

    #[test]
    fn git_file_filters_map_to_distinct_choice_keys() {
        let keys: std::collections::HashSet<_> = [
            FileFilter::All,
            FileFilter::Modified,
            FileFilter::Added,
            FileFilter::Deleted,
            FileFilter::Renamed,
        ]
        .into_iter()
        .map(|f| HoverId::choice(PanelKind::GitLog, f as u64))
        .collect();
        assert_eq!(keys.len(), 5);
    }
```

- [ ] **Step 4: 确认测试失败**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2 && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app panel_host 2>&1 | grep -E "^error" -A3 | head -10; git checkout -- Cargo.lock`
Expected: 编译失败,`no variant, associated function, or constant named \`tab_item\` found for enum \`HoverId\``(`named`、`list_collapse` 等同)。

- [ ] **Step 5: 写重写脚本并先 dry-run**

创建 `$SCRATCH/rewrite_hover_id.py`:

```python
#!/usr/bin/env python3
"""一次性重写脚本(H2):把 `HoverId` 里 51 个"点名面板"的变体改成 `HoverId::<helper>(PanelKind::X, …)`
构造,并把这些变体从枚举里删掉。**不提交、不进产品构建。**

用法: rewrite_hover_id.py <src 根目录> [--dry-run]
两步:1) 重写所有 `HoverId::Old(..)` / `HoverId::Old`(含 `crate::app::` 前缀、含当函数指针传的 `HoverId::PreviewTabItem,`);
      2) 在 app/state.rs 里从 `pub enum HoverId` 删掉这 51 个变体,换成宿主自己的变体 + 一个通用的 `Panel` 变体。
注释行里的 `HoverId::Old` 不改。
"""
import os, re, sys

# 旧变体 -> (面板, helper, 载荷类型(None=无载荷), 具名按钮名)
M = {}
def add(old, panel, helper, payload=None, name=None):
    M[old] = (panel, helper, payload, name)

for pre, panel in (("Preview", "Files"), ("ProjectPreview", "Project")):
    add(pre + "TabItem", panel, "tab_item", "usize")
    add(pre + "TabClose", panel, "tab_close", "usize")
    add(pre + "TabOverflow", panel, "tab_overflow")
    add(pre + "RenderMode", panel, "named", None, "render_mode")
    add(pre + "TabularMode", panel, "named", None, "tabular_mode")
    add(pre + "JsonMode", panel, "named", None, "json_mode")
    add(pre + "FindPrev", panel, "named", None, "find_prev")
    add(pre + "FindNext", panel, "named", None, "find_next")
    add(pre + "FindReplaceToggle", panel, "named", None, "find_replace_toggle")
    add(pre + "FindReplaceCurrentBtn", panel, "named", None, "find_replace_current")
    add(pre + "FindReplaceAllBtn", panel, "named", None, "find_replace_all")
add("FilesSearchSubmit", "Files", "search_submit")
add("FilesDotfiles", "Files", "named", None, "dotfiles")
add("FilesBranchSwitch", "Files", "named", None, "branch_switch")
add("FileTreeCollapse", "Files", "list_collapse")
add("ProjectDocsAdd", "Project", "named", None, "docs_add")
add("ProjectRemoteAdd", "Project", "named", None, "remote_add")
add("ProjectMemoryAdd", "Project", "named", None, "memory_add")
add("ProjectListCollapse", "Project", "list_collapse")
add("SshTabItem", "Ssh", "tab_item", "u64")
add("SshTabClose", "Ssh", "tab_close", "u64")
add("SshTabOverflow", "Ssh", "tab_overflow")
add("SshListCollapse", "Ssh", "list_collapse")
add("HostCard", "Ssh", "row", "u64")
add("DatabaseListCollapse", "Database", "list_collapse")
add("DatabaseTabItem", "Database", "tab_item", "usize")
add("DatabaseTabClose", "Database", "tab_close", "usize")
add("DatabaseTabOverflow", "Database", "tab_overflow")
add("TodoListCollapse", "Todo", "list_collapse")
add("TodoCategoryRow", "Todo", "row", "i64")
add("AgentListCollapse", "Agent", "list_collapse")
add("AgentPickerToggle", "Agent", "named", None, "picker_toggle")
add("ConversationsListCollapse", "Conversations", "list_collapse")
add("ConversationListMore", "Conversations", "more")
add("ConversationSearchSubmit", "Conversations", "search_submit")
add("UsageListCollapse", "Usage", "list_collapse")
add("CommitListMore", "GitLog", "more")
add("GitFile", "GitLog", "row", "usize")
add("GitFileFilter", "GitLog", "choice", "FileFilter")
add("GitLogSearchSubmit", "GitLog", "search_submit")
assert len(M) == 51, len(M)

REF = re.compile(r"(?P<pre>crate::app::)?HoverId::(?P<v>[A-Za-z]+)")


def balanced(text, i):
    """text[i] == '(' ;返回 (括号内文本, 右括号之后的位置)。"""
    depth, j = 0, i
    while j < len(text):
        c = text[j]
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                return text[i + 1:j], j + 1
        j += 1
    raise ValueError("unbalanced")


def in_comment(text, pos):
    ls = text.rfind("\n", 0, pos) + 1
    return text[ls:pos].lstrip().startswith("//")


def arg_expr(arg, payload):
    a = arg.strip()
    if payload == "u64" or re.fullmatch(r"\d+", a):  # 已是 u64,或整数字面量(自动推成 u64,加 `as` 会触发 unnecessary_cast)
        return a
    if payload == "FileFilter":
        return f"{a} as u64"
    return f"{a} as u64"  # usize / i64 -> u64


def rewrite(text):
    out, i, n = [], 0, 0
    while True:
        m = REF.search(text, i)
        if not m:
            out.append(text[i:])
            break
        v = m.group("v")
        if v not in M or in_comment(text, m.start()):
            out.append(text[i:m.end()])
            i = m.end()
            continue
        panel, helper, payload, name = M[v]
        pre = m.group("pre") or ""
        kind = f"{pre}PanelKind::{panel}"
        head = f"{pre}HoverId::{helper}"
        out.append(text[i:m.start()])
        j = m.end()
        if payload is not None and j < len(text) and text[j] == "(":
            arg, j = balanced(text, j)
            out.append(f"{head}({kind}, {arg_expr(arg, payload)})")
        elif payload is not None:  # 当函数指针传:`HoverId::PreviewTabItem,`
            out.append(f"|i| {head}({kind}, i as u64)")
        elif helper == "named":
            out.append(f'{head}({kind}, "{name}")')
        else:
            out.append(f"{head}({kind})")
        n += 1
        i = j
    return "".join(out), n


def has_panelkind_import(text):
    return bool(re.search(r"use crate::app::(\{[^}]*\bPanelKind\b[^}]*\}|PanelKind;)", text))


def fix_imports(text):
    """文件里出现了脚本生成的裸 PanelKind:: 却没 import 它时,把它并进 use crate::app::{…HoverId…}。"""
    if not re.search(r"(?<![:\w])PanelKind::", text) or has_panelkind_import(text):
        return text
    t2 = text.replace("use crate::app::HoverId;", "use crate::app::{HoverId, PanelKind};", 1)
    if t2 != text:
        return t2
    m = re.search(r"use crate::app::\{([^}]*)\}", text)
    if m and "HoverId" in m.group(1):
        return text.replace(m.group(0), "use crate::app::{" + m.group(1).rstrip().rstrip(",") + ", PanelKind}", 1)
    return text


def main():
    root, dry = sys.argv[1], "--dry-run" in sys.argv
    total = 0
    for dp, _, fns in sorted(os.walk(root)):
        for fn in sorted(fns):
            if not fn.endswith(".rs"):
                continue
            p = os.path.join(dp, fn)
            old = open(p, encoding="utf-8").read()
            new, n = rewrite(old)
            if not n:
                continue
            new = fix_imports(new)
            print(f"{n:3d}  {os.path.relpath(p, root)}")
            total += n
            if not dry:
                open(p, "w", encoding="utf-8").write(new)
    print(f"total rewrites: {total}")


if __name__ == "__main__":
    main()
```

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2 && python3 $SCRATCH/rewrite_hover_id.py crates/dozer-app/src --dry-run`
Expected: 逐文件命中数,合计 **`total rewrites: 126`**(草稿值:`workspace/view.rs` 30、`app/view.rs` 25、`app/app.rs` 24、`app/update.rs` 12、`extensions/git_log.rs` 10、`extensions/database/view.rs` 10、`extensions/conversations.rs` 4、`extensions/todo/view.rs` 4、`extensions/ssh.rs` 3、`extensions/usage/view.rs` 2、`term/terminal.rs` 2)。**不是 126 就先停下**:`main` 上这些文件有改动。对照 `grep -rnE "HoverId::[A-Z]" crates/dozer-app/src | grep -vE "HoverId::(Topbar|Rail|ProjectTab|TermTab|HomeTab|HomeProject|TabOverflowRow)"` 逐个看新增的点位是否在映射表里;不在表里的(新面板按钮)先按表的规则补一行映射再继续。

- [ ] **Step 6: 真跑三个脚本**

**顺序不能换:** 先重写引用(此时枚举还没改,代码里引用的新 helper 还不存在,这没关系),再改枚举,最后订正注释。

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2
python3 $SCRATCH/rewrite_hover_id.py crates/dozer-app/src | tail -1
```

创建 `$SCRATCH/edit_enum.py` 并运行(依赖同目录的 `rewrite_hover_id.py` 里的映射表;删 51 个变体、加 `Panel` 变体与 9 个 helper、新增 `HoverSlot`、去掉不再需要的 `FileFilter` import):

```python
#!/usr/bin/env python3
"""H2 步骤 2:改 `HoverId` 枚举本身(app/state.rs)、新增 `HoverSlot`(panel_host.rs)与 helper 构造函数。
必须在 rewrite_hover_id.py 之后运行(此时代码里已没有人再用被删的变体)。**不提交。**"""
import importlib.util, os, re, sys

here = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("rw", os.path.join(here, "rewrite_hover_id.py"))
rw = importlib.util.module_from_spec(spec); spec.loader.exec_module(rw)
REMOVE = set(rw.M)

SRC = "crates/dozer-app/src/"
p = SRC + "app/state.rs"
s = open(p).read()
start = s.index("pub enum HoverId {")
end = s.index("\n}\n", start)
body = s[start + len("pub enum HoverId {"):end]
chunks, cur = [], []
for line in body.split("\n")[1:]:  # 第一项是 `{` 之后的空串
    cur.append(line)
    if re.match(r"^    [A-Z]\w*(\(.*\))?,\s*$", line):
        chunks.append(cur); cur = []
assert not [l for l in cur if l.strip()], cur
kept = []
for ch in chunks:
    name = re.match(r"^    ([A-Z]\w*)", ch[-1]).group(1)
    if name not in REMOVE:
        kept.append("\n".join(ch))
new_body = "\n" + "\n".join(kept) + '''
    /// 某个面板里的一个可悬停元素,按"面板 + 槽位"作用域取键——**宿主枚举里不再点名任何面板的按钮**
    /// (槽位词汇见 [`HoverSlot`],通用、不含面板名)。用下面的 `HoverId::tab_item(..)` 等构造函数造。
    Panel(PanelKind, HoverSlot),'''
s = s[:start + len("pub enum HoverId {")] + new_body + s[end:]
helpers = '''
impl HoverId {
    /// 面板列表列的折叠/展开按钮。
    pub(crate) fn list_collapse(panel: PanelKind) -> Self {
        Self::Panel(panel, HoverSlot::ListCollapse)
    }
    /// 面板搜索框旁的提交按钮。
    pub(crate) fn search_submit(panel: PanelKind) -> Self {
        Self::Panel(panel, HoverSlot::SearchSubmit)
    }
    /// 列表末尾的"更多…"翻页按钮。
    pub(crate) fn more(panel: PanelKind) -> Self {
        Self::Panel(panel, HoverSlot::More)
    }
    /// 页签标题(`key` 是页签下标或稳定 id)。
    pub(crate) fn tab_item(panel: PanelKind, key: u64) -> Self {
        Self::Panel(panel, HoverSlot::TabItem(key))
    }
    /// 页签关闭按钮(`key` 同 `tab_item`)。
    pub(crate) fn tab_close(panel: PanelKind, key: u64) -> Self {
        Self::Panel(panel, HoverSlot::TabClose(key))
    }
    /// 页签溢出菜单按钮。
    pub(crate) fn tab_overflow(panel: PanelKind) -> Self {
        Self::Panel(panel, HoverSlot::TabOverflow)
    }
    /// 列表行/卡片(`key` 是行下标或稳定 id)。
    pub(crate) fn row(panel: PanelKind, key: u64) -> Self {
        Self::Panel(panel, HoverSlot::Row(key))
    }
    /// 一组互斥选项/筛选里的一项。
    pub(crate) fn choice(panel: PanelKind, key: u64) -> Self {
        Self::Panel(panel, HoverSlot::Choice(key))
    }
    /// 面板内一次性的具名按钮(`name` 在同一面板内唯一)。
    pub(crate) fn named(panel: PanelKind, name: &'static str) -> Self {
        Self::Panel(panel, HoverSlot::Named(name))
    }
}
'''
end2 = s.index("\n}\n", s.index("pub enum HoverId {")) + 3
s = s[:end2] + helpers + s[end2:]
s = s.replace("use crate::extensions::git_log::FileFilter;\n", "")
s = s.replace("use crate::chrome::rail;\n", "use crate::chrome::rail;\nuse crate::panel_host::HoverSlot;\n", 1)
open(p, "w").write(s)

p = SRC + "panel_host.rs"
s = open(p).read()
slot = '''
/// 面板内一个可悬停元素的**槽位**——词汇通用,不含任何面板名(规格 E2:宿主公开类型里不出现业务类型)。
/// 与面板(`PanelKind`)一起构成 `HoverId::Panel(panel, slot)`。新增槽位种类前先看能不能用 `Named`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HoverSlot {
    /// 列表列折叠按钮。
    ListCollapse,
    /// 搜索框提交按钮。
    SearchSubmit,
    /// "更多…"翻页按钮。
    More,
    /// 页签标题(下标或稳定 id)。
    TabItem(u64),
    /// 页签关闭按钮。
    TabClose(u64),
    /// 页签溢出菜单按钮。
    TabOverflow,
    /// 列表行/卡片。
    Row(u64),
    /// 一组互斥选项里的一项。
    Choice(u64),
    /// 面板内一次性的具名按钮(同一面板内唯一)。
    Named(&'static str),
}
'''
s = s.replace("\npub trait PanelHost {", slot + "\npub trait PanelHost {", 1)
open(p, "w").write(s)
```

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2 && python3 $SCRATCH/edit_enum.py`

创建 `$SCRATCH/comment_edits.py` 并运行(13 处注释订正;每处先 `assert` 原文存在,原文不在就报错,不会静默跳过):

```python
#!/usr/bin/env python3
"""H2 步骤 3:订正注释里对已删除 `HoverId` 变体的引用(脚本不改注释行)。**不提交。**"""
base = "crates/dozer-app/src/"
EDITS = [
 ("app/update.rs", "语义完全对齐文件预览的 `FileTreeCollapse` 按钮(见 `preview_pane_for`)", "语义完全对齐文件预览的 `HoverId::list_collapse(PanelKind::Files)` 按钮(见 `preview_pane_for`)"),
 ("app/state.rs", "处理方式同 `FileTreeCollapse`(见 `terminal::tab_bar`)", "处理方式同 `HoverId::list_collapse(PanelKind::Files)`(见 `terminal::tab_bar`)"),
 ("app/message.rs", "`PreviewFindReplaceToggle` 再手动翻转", "`HoverId::named(PanelKind::Files, \"find_replace_toggle\")` 对应的按钮再手动翻转"),
 ("app/view.rs", "`HoverId::SshTab{Item,Close}`", "`HoverId::tab_item`/`tab_close`(`PanelKind::Ssh`)"),
 ("workspace/view.rs", "由 `HoverId::AgentPickerToggle` +", "由 `HoverId::named(PanelKind::Agent, \"picker_toggle\")` +"),
 ("extensions/database/update.rs", "`HoverId::DatabaseTabItem/DatabaseTabClose`", "`HoverId::tab_item`/`tab_close`(`PanelKind::Database`)"),
 ("extensions/database/state.rs", "`HoverId::DatabaseTabItem/DatabaseTabClose`", "`HoverId::tab_item`/`tab_close`(`PanelKind::Database`)"),
 ("extensions/git_log.rs", "统一的 `HoverId::GitFileFilter`", "统一的 `HoverId::choice(PanelKind::GitLog, ..)`"),
 ("extensions/git_log.rs", "当 `HoverId::GitFile` 的 key", "当 `HoverId::row(PanelKind::GitLog, ..)` 的 key"),
 ("extensions/files/state.rs", "(`HoverId::FilesSearchSubmit` / `FilesDotfiles` / `FilesBranchSwitch`)", "(`HoverId::search_submit` / `named(.., \"dotfiles\")` / `named(.., \"branch_switch\")`,面板均为 `PanelKind::Files`)"),
 ("chrome/tab_widget.rs", "(如 `HoverId::PreviewRenderMode`)", "(如 `HoverId::named(PanelKind::Files, \"render_mode\")`)"),
 ("chrome/tab_widget.rs", "(见 `HoverId::PreviewTabularMode`)", "(见 `HoverId::named(PanelKind::Files, \"tabular_mode\")`)"),
 ("chrome/tab_widget.rs", "(见 `HoverId::PreviewJsonMode`)", "(见 `HoverId::named(PanelKind::Files, \"json_mode\")`)"),
]
for path, old, new in EDITS:
    p = base + path
    s = open(p).read()
    assert old in s, (path, old)
    open(p, "w").write(s.replace(old, new, 1))
print(f"{len(EDITS)} comment edits")
```

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2 && python3 $SCRATCH/comment_edits.py`
Expected: `13 comment edits`。

- [ ] **Step 7: 格式化、编译**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2
cargo fmt -p dozer-app && git status --short
RUSTFLAGS="-D unconditional_recursion" cargo build -p dozer-app 2>&1 | grep -E "^(error|warning: (unused|function cannot))" -A8 | head -40
git checkout -- Cargo.lock
```
Expected: `git status --short` 只有 `panel_host.rs`、`app/{state,app,update,view,message}.rs`、`workspace/view.rs`、`term/terminal.rs`、`chrome/tab_widget.rs`、`extensions/{conversations,git_log,ssh,database/view,database/update,database/state,todo/view,usage/view,files/state}.rs`(和 `Cargo.lock`,马上还原);build 无输出。**若报 `PanelKind` 找不到:** 脚本的 `fix_imports` 只处理 `use crate::app::HoverId;` 与 `use crate::app::{…HoverId…}` 两种写法,别的写法的文件手工加 `use crate::app::PanelKind;`(草稿里没有这种文件)。

- [ ] **Step 8: 全量测试 + clippy 对比**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^error|^test result|FAILED"
git checkout -- Cargo.lock
RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-after.txt; git checkout -- Cargo.lock
diff $SCRATCH/clippy-before.txt $SCRATCH/clippy-after.txt && echo "clippy: no new diagnostics"
```
Expected: 测试通过数 = N + 3,失败只有"已知基线"里的;`shift_index_keys_after_close_*`、`rekey`/`dehover` 相关的既有测试在通过列表里(`cargo test -p dozer-app shift_index_keys 2>&1 | grep "test result"` 应为 `3 passed`);`clippy: no new diagnostics`。

- [ ] **Step 9: 残留检查与变异检验**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2
# 1) HoverId 只剩 12 个变体,没有被删变体的残留引用
grep -rnE "HoverId::[A-Z]" crates/dozer-app/src | grep -vE "HoverId::(Topbar|Rail|ProjectTab|TermTab|HomeTab|HomeProject|TabOverflowRow|Panel)\b" | head
# 2) 非 macOS cfg 块里没有被改写的点位
git diff -U10 -- crates | grep -nE "cfg\((not\()?target_os"
```
Expected: 第 1 条**无输出**;第 2 条无输出(草稿:只有 1 个与改动无关的 `cfg(target_os = "macos")` 上下文行)。若命中 `cfg(not(target_os = "macos"))` 块,逐行核对被改的 hunk 与 macOS 侧同形。

变异检验(**先暂存**):
```bash
git add crates && git status --short | wc -l
python3 - <<'EOF'
p = "crates/dozer-app/src/app/state.rs"
s = open(p).read()
a = "    pub(crate) fn named(panel: PanelKind, name: &'static str) -> Self {\n        Self::Panel(panel, HoverSlot::Named(name))\n    }"
b = "    pub(crate) fn named(_panel: PanelKind, name: &'static str) -> Self {\n        Self::Panel(PanelKind::Files, HoverSlot::Named(name))\n    }"
assert a in s
open(p, "w").write(s.replace(a, b))
EOF
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app same_slot_in_different_panels 2>&1 | grep -E "^test |test result"
git checkout -- crates/dozer-app/src/app/state.rs Cargo.lock; git diff --stat | tail -1
```
Expected: 变异后 `same_slot_in_different_panels_is_a_different_key … FAILED`(`Files` 与 `Project` 的 `named(.., "find_prev")` 碰撞);还原后 `git diff --stat` 无输出。

- [ ] **Step 10: 提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2
git status --short
git add crates
git diff --cached --stat | tail -22
git commit -m "refactor(hover): namespace panel hover keys under HoverId::Panel(PanelKind, HoverSlot)

51 panel-named HoverId variants (TodoListCollapse, FilesSearchSubmit, DatabaseTabItem,
ProjectPreviewFindPrev, ...) become HoverId::Panel(panel, slot) with a panel-agnostic slot
vocabulary; the host enum keeps only its own 11 variants. 126 call sites rewritten; key
equality is unchanged (injective mapping). Also drops the host's dependency on
git_log::FileFilter.

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: `git diff --cached --stat` 里只有 `crates/`(没有 `Cargo.lock`、`scripts/`、`docs/`)。

---

### Task 2: 门禁(枚举变体数棘轮)与文档回填

**Files:**
- Modify: `scripts/audit/test_check_panel_boundary.py`、`scripts/audit/check_panel_boundary.py`、`scripts/audit/panel-boundary.baseline.json`
- Modify: `docs/dozer-v2/bytehost-H0/{00-summary,02-panel-to-host}.md`、`docs/dozer-v2/bytehost-H0/E3-registry.tsv`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`

**Interfaces:**
- Consumes: Task 1 的提交。
- Produces: 门禁规则 `R-HOVERID-VARIANTS`(`app/state.rs` 里 `HoverId` 的变体数,基线 12)、`R-HOVERSLOT-VARIANTS`(`panel_host.rs` 里 `HoverSlot` 的变体数,基线 9),均只许减不许增。

- [ ] **Step 1: 写失败的测试**

在 `scripts/audit/test_check_panel_boundary.py` 里 `class Compare(unittest.TestCase):` 之前加入:

```python
class EnumVariants(unittest.TestCase):
    """R-HOVERID-VARIANTS / R-HOVERSLOT-VARIANTS:宿主的 HoverId 与通用槽位词汇 HoverSlot 的变体数只许减不许增
    (H2:面板按钮不得再写回 host 枚举;要新增悬停元素先用 `HoverId::named(panel, ..)`)。"""
    ENUM = "pub enum HoverId {\n    /// doc\n    Topbar(TopbarButton),\n    Rail(R),\n    HomeTab,\n    // c\n    Panel(PanelKind, HoverSlot),\n}\n"
    def test_counts_variants_not_docs_or_comments(self):
        got = g.scan({"app/state.rs": self.ENUM}).get("app/state.rs", {})
        self.assertEqual(got, {"R-HOVERID-VARIANTS": 4})
    def test_slot_enum_counted_in_panel_host(self):
        text = "pub enum HoverSlot {\n    ListCollapse,\n    TabItem(u64),\n    Named(&'static str),\n}\n"
        got = g.scan({"panel_host.rs": text}).get("panel_host.rs", {})
        self.assertEqual(got, {"R-HOVERSLOT-VARIANTS": 3})
    def test_other_files_ignore_same_named_enums(self):
        self.assertEqual(g.scan({"extensions/x.rs": self.ENUM}).get("extensions/x.rs", {}), {})
```

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2 && PYTHONDONTWRITEBYTECODE=1 python3 scripts/audit/test_check_panel_boundary.py 2>&1 | grep -E "^(FAIL|ERROR)|^Ran|^FAILED|^OK"`
Expected: `FAIL: test_counts_variants_not_docs_or_comments`、`FAIL: test_slot_enum_counted_in_panel_host`(第三个本来就该过),`FAILED (failures=2)`。

- [ ] **Step 2: 实现规则**

用下面的文件**整体替换** `scripts/audit/check_panel_boundary.py`(在 H1 版本上加了 `ENUM_RULES`/`enum_variant_count` 与 `scan` 里的分支):

```python
#!/usr/bin/env python3
"""面板边界棘轮门禁(报告模式):面板代码(extensions/**)里 `App`/`Workspace` 的引用只许减少、不许增加。

规则 R-APP:  extensions 下文件对 `crate::app::App` 的**使用次数**(import 行 + 每个 `&App` 参数/限定路径)
规则 R-WS:   extensions 下文件对 `crate::workspace::Workspace` 的使用次数
规则 R-PANE-PICK: 全库手写"按 PanelKind::Project 选预览窗格"的写法条数(应走 `Workspace::preview_pane[_mut]`)
规则 R-HOVERID-VARIANTS / R-HOVERSLOT-VARIANTS: `HoverId`(app/state.rs)与 `HoverSlot`(panel_host.rs)的变体数

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


# 枚举变体数棘轮:(文件, 枚举名) -> 规则名。HoverId 是宿主的悬停键枚举,HoverSlot 是面板作用域下的通用槽位词汇;
# 两者的变体数只许减不许增,新增悬停元素先用 `HoverId::named(panel, "..")`。
ENUM_RULES = {
    ("app/state.rs", "HoverId"): "R-HOVERID-VARIANTS",
    ("panel_host.rs", "HoverSlot"): "R-HOVERSLOT-VARIANTS",
}


def enum_variant_count(text, name):
    """`pub enum NAME { .. }` 里的变体个数(忽略文档注释/注释/属性)。找不到枚举返回 0。"""
    m = re.search(r"\bpub enum " + re.escape(name) + r"\s*\{", text)
    if not m:
        return 0
    end = text.index("\n}\n", m.end())
    body = edges.strip_comments(text[m.end():end])
    return len(re.findall(r"^    [A-Z]\w*", body, flags=re.M))


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
        for (path, name), rule in ENUM_RULES.items():
            if rel == path:
                k = enum_variant_count(text, name)
                if k:
                    hit[rule] = k
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
Expected: `Ran 18 tests` … `OK`。

- [ ] **Step 3: 更新基线**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2 && export PYTHONDONTWRITEBYTECODE=1 && python3 scripts/audit/check_panel_boundary.py; python3 scripts/audit/check_panel_boundary.py --update && cat scripts/audit/panel-boundary.baseline.json | tr -d '\n ' ; echo; python3 scripts/audit/check_panel_boundary.py`
Expected: 第一次检查对着旧基线报 `app/state.rs: R-HOVERID-VARIANTS 0 -> 12` 与 `panel_host.rs: R-HOVERSLOT-VARIANTS 0 -> 9`(新规则的现存值,正是预期);`--update` 之后基线为
`{"app/layout.rs":{"R-PANE-PICK":1},"app/state.rs":{"R-HOVERID-VARIANTS":12},"app/update.rs":{"R-PANE-PICK":1},"panel_host.rs":{"R-HOVERSLOT-VARIANTS":9},"platform/window_events.rs":{"R-PANE-PICK":3},"workspace/state.rs":{"R-PANE-PICK":2}}`;最后一次检查 `ok`。

- [ ] **Step 4: 变异检验(先暂存)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2 && export PYTHONDONTWRITEBYTECODE=1
git add scripts/audit
python3 - <<'EOF'
p = "crates/dozer-app/src/app/state.rs"
s = open(p).read()
a = "    Panel(PanelKind, HoverSlot),\n}"
assert a in s
open(p, "w").write(s.replace(a, "    Panel(PanelKind, HoverSlot),\n    TodoBogusButton,\n}", 1))
EOF
python3 scripts/audit/check_panel_boundary.py; echo "exit=$?"
git checkout -- crates/dozer-app/src/app/state.rs; python3 scripts/audit/check_panel_boundary.py; echo "exit=$?"; git status --short crates | wc -l
```
Expected: 变异后 `app/state.rs: R-HOVERID-VARIANTS 12 -> 13` 且 `exit=1`;还原后 `ok`、`exit=0`、最后输出 `0`(`crates/` 已回到 Task 1 提交的版本)。

- [ ] **Step 5: 回填文档**

每条先 `grep -n` 找到原文再改,改完 `git diff` 看一遍:
1. `docs/dozer-v2/bytehost-H0/00-summary.md` §4 的候选 4 一行,在"涉及"列后追加"**已完成(H2,`bytehost-h2`):<Task 1 提交短 id>**";§5 门禁说明的基线含义里补一句"另有 `R-HOVERID-VARIANTS`(基线 12)与 `R-HOVERSLOT-VARIANTS`(基线 9):`HoverId`/`HoverSlot` 变体数只许减不许增"。
2. `docs/dozer-v2/bytehost-H0/02-panel-to-host.md` §3.2 末尾追加一段"**H2 结果:** `HoverId` 51 个点名面板的变体已收敛为 `HoverId::Panel(PanelKind, HoverSlot)`,宿主枚举只剩 11 个自己的变体 + `Panel`;`HoverSlot` 是通用槽位词汇(9 个变体)。`PanelKind` 作为键的一部分仍是宿主类型,随注册制(`01-panelkind.md` B1)一起换成注册 id。"
3. `docs/dozer-v2/bytehost-H0/E3-registry.tsv`:删除 `E3-013`(`app/state.rs:11、:261` 的 `HoverId::GitFileFilter(FileFilter)`——枚举已不引用 `git_log::FileFilter`)与 `E3-016`(`HoverId` 点名面板按钮)两行。改完运行 `awk -F'\t' 'NR>1 && (NF!=6 || $6=="")' docs/dozer-v2/bytehost-H0/E3-registry.tsv | wc -l`,Expected: `0`;`grep -c "^E3-" docs/dozer-v2/bytehost-H0/E3-registry.tsv`,Expected: 比改前少 2(草稿:H1 之后 37 → 35)。
4. `docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` §7:在 H1 那条之后追加一条"**bytehost H2 已完成(2026-10-04):** 宿主 `HoverId` 不再点名面板按钮(51 个变体 → `HoverId::Panel(PanelKind, HoverSlot)`),门禁新增变体数棘轮;`PanelKind` 注册制、`window_events.rs` 面板事件声明化留给后续切片。"

- [ ] **Step 6: 最终校验并提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h2 && export PYTHONDONTWRITEBYTECODE=1
python3 scripts/audit/test_edges.py 2>&1 | tail -1; python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -1; python3 scripts/audit/check_panel_boundary.py
git status --short
git add scripts/audit/check_panel_boundary.py scripts/audit/test_check_panel_boundary.py scripts/audit/panel-boundary.baseline.json docs/dozer-v2
git diff --cached --stat | tail -10
git commit -m "chore(audit): ratchet HoverId/HoverSlot variant counts; backfill H0 docs for H2

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: 两组测试 `OK`、门禁 `ok`;`git diff --cached --stat` 里只有 `scripts/audit/` 与 `docs/`。

---

## Self-Review

**1. 覆盖:** H0 汇总候选 4(`HoverId` 命名空间化)→ Task 1;E3-013、E3-016 的消除与门禁防回退 → Task 2。候选 3(`window_events.rs` 面板事件声明化)与候选 5(假宿主完整形态)不在本计划:前者需要先有"面板事件钩子/Effect"的最小形态(要求文档 Q9),后者依赖本计划与 H1 的接口。

**2. 占位符扫描:** 全部脚本与 Rust 代码为草稿里跑通的版本;映射表给出 51 条的完整对应;文档回填给出要写入的文字。

**3. 一致性:** helper 名(`list_collapse`/`search_submit`/`more`/`tab_overflow`/`tab_item`/`tab_close`/`row`/`choice`/`named`)在映射表、重写脚本、`edit_enum.py`、测试里一致;`HoverSlot` 9 个变体与门禁基线 9 一致;`HoverId` 12 个变体 = 11 宿主 + `Panel`。

**4. Review Focus:** 5 条各有归属(1→Task 1 Step 3/9、2→Step 8、3→Step 6/9、4→Step 9、5→接受并说明)。
