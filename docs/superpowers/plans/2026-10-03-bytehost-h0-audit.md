# bytehost H0:只读审计与迁移清单 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 bytehost 的边界规格拿到**真实的依赖数据**:面板对 host 的依赖、host 对面板的硬编码、共享领域模块的画像、`platform/` 下 overlay 的归属、`PanelKind` 引用分类,以及六个未决项的证据;并落一个只许回退不许增长的面板边界门禁。H0 **不改任何产品代码**(`crates/` 下零改动),产出是脚本、数据、审计文档和对规格的数据回填。

**Architecture:** 一个小型静态分析工具集(`scripts/audit/*.py`,标准库 Python,一次性开发工具,不进产品构建)从 `dozer-app` 源码抽出 `crate::` 依赖边,再由报表脚本生成 Markdown 表;每个审计文档 = 机械报表 + 人工裁决列。门禁用"棘轮基线"(只许减不许增)而不是"一步到零",因为迁移是分切片进行的。

**Tech Stack:** Python 3 标准库(`unittest`)、bash/grep;产品代码不动。

**Spec:** `docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md`(§2 三条硬边界、§4 现状差距、§6 验收标准、§8 未决项 O1–O12、§10 下一步)。前序依据:`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`(Q7/Q8/Q10/Q13/Q15)。

## Global Constraints

- **H0 只读:** 不得修改 `crates/` 下任何文件、不得改 `Cargo.toml`/`Cargo.lock`。允许新增/修改的路径只有 `scripts/audit/`、`docs/dozer-v2/bytehost-H0/`、`docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`。每个任务提交前 `git diff --cached --stat` 确认没有 `crates/`。
- **在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytehost-h0/...`),分支 `bytehost-h0`;主 checkout 常有并发会话与未提交改动(此刻就有未跟踪的 `docs/group_chat/`),不得在其上改文件或 `git add -A`。提交只 `git add` 指定路径。
- **每份数据与审计文档的文件头必须记录:** 生成时的 `git rev-parse --short HEAD`(基线提交)、生成命令。`main` 在动,数字会过期,没有基线提交的数字不得被后续文档引用。
- **报表是机械判断,不是裁决。** 报表里的"归属建议"列是启发式输出;审计文档里必须有一列"人工裁决"由执行者读代码后填写,并写明依据(文件:行)。不得把启发式输出原样当结论。
- **每个审计文档的结论都要能被复现:** 引用的数字都能由文档里列出的命令重新得到。
- 产品原有关键裁决照常适用(见项目 `CLAUDE.md`):新浮层默认走独立原生子窗口;瞬时消息走 Toast;日志走 `dozer_core::log`;字体、主题约束不变。H0 只是审计,但裁决建议不得与之冲突。
- 提交信息用 `docs:`/`chore(audit):`/`test:` 前缀;末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## 已知事实(2026-10-03 起草时在 `main` 上核到的,执行时用 Task 2 的报表重新核实)

起草计划时在草稿里跑通了下面所有脚本(本计划里的代码就是跑过的版本),并拿到这些**与 v2 要求文档 §4 不一致**的数字:

- `PanelKind` 现在有 **12 个变体**(多了 `GroupChat`),不是文档里的 11;`PanelKind` 一词在 `crates/dozer-app/src` 里出现 **973 次、分布在 31 个文件**(文档写的是约 887 处、约 25 个文件)。
- 面板代码(`extensions/**`)里 `crate::app::App` 只有 **8 处引用、分布在 7 个面板**,`crate::workspace::Workspace` 只有 3 处;`App`/`Workspace` 的合计 11 处引用只在 **9 个文件**里。比文档 §4.2 暗示的要少得多——面板边界门禁的第一个基线会很小。
- `app::HoverId`(23 次)、`app::TextInputTarget`(19 次)、`chrome::native_menu`(34 次)、`chrome::homespace`(11 次,10 个面板用)才是面板对 host 的大头依赖,不是 `App` 本身。
- host 对面板的直接引用里,`app/` 对 `group_chat`(22)、`agent_context`(16)、`todo`(11)最重;`platform/window_events.rs` 单文件对 `files` 12 次。
- `platform/` 下约 20 个 `*_overlay.rs`:机械判断里 `file_drag`、`overlay_focus`、`overlay_gpu`、`overlay_window`、`picker` 等无 extension 引用(通用候选),`confirm_overlay`、`file_history_overlay` 引用多个面板(需人工裁决)。
- 根下共享模块里,`conversation`(4 个扇入)、`project`(5)、`capabilities`(4)、`project_meta`(4)、`transcript`(2)、`secrets`(2)、`external_apps`(2)、`git_accounts`(2) 都没有 iced 依赖;`runtime`(772 行)和 `menu_spec`、`event`、`frosted` 依赖 iced。

这些数字是 H0 要验证的**假设**:Task 2 的报表若与它们不符,以报表为准,并在汇总文档里记下差异。

## Review Focus

规格对下列情形没有逐条说明,但真实使用这些审计结论的人会遇到;每条都由指明的任务里的步骤固定下来:

1. **静态 grep/解析会漏依赖。** `pub use` 重导出、`use super::super::`、`#[path]`、宏生成的路径、`crate::` 之外的绝对路径都不会被边提取器抓到;重导出会让"面板 A 引用 host 符号"被记在 `mod.rs` 名下。Task 1 的测试固定提取器对嵌套花括号、多行 `use`、`as` 别名、注释的处理;Task 3 与 Task 4 要求对每张表的前 10 行做**人工抽查**(读源码确认),抽查结果写进文档。
2. **测试代码被计入。** 边提取器不区分生产代码与 `#[cfg(test)]`、`tests.rs`:`workspace/tests.rs` 对 `Workspace` 的引用会让"面板对 host 的依赖"被高估。Task 3 要求对表中每个高频符号检查其引用是否只出现在测试里,并在文档中标注;门禁(Task 8)的基线明确"测试引用同样计入"。
3. **`PanelKind` 的"点名变体"不等于"必须特判"。** 报表按行把引用分成"点名变体"与"仅类型",并找出"遍历候选簇"(15 行内 ≥3 个不同变体);但一个 `match` 穷举全部变体既可能是遍历、也可能是真特判。Task 3 要求对前 6 个文件逐个读代码给出人工分类,并抽样 30 行校验启发式的准确率,准确率低于 80% 就在文档里声明报表只作线索。
4. **范围只覆盖 `dozer-app`。** `dozerd`、`dozer-core`、`dozer-client`、`dozer-mcp` 里也有"host 机制"(日志 scope、UDS 协议、会话/hook 服务),单看 `dozer-app` 会误以为 host 只在 GUI 进程里。Task 7 要求对 Agent 相关代码做跨 crate 的分布统计,Task 5 对 `dozer-core` 的公开模块列清单并标注是否产品无关。
5. **数字会过期。** `main` 上有并发提交(如 group_chat 刚新增一个 `PanelKind` 变体就让文档数字过期)。每份文档带基线提交;汇总任务(Task 9)在写回规格之前**重新跑一遍全部报表**,与各文档里的数字比对,不一致的以最新为准并标注。

---

## File Structure

| 路径 | 职责 | 任务 |
|---|---|---|
| `scripts/audit/edges.py` | 从 `crates/dozer-app/src` 提取 `crate::` 依赖边(展开花括号 `use`、行内路径),定义"单元"(`ext:`/`layer:`/`root:`) | 1 |
| `scripts/audit/test_edges.py` | 提取器的单元测试 | 1 |
| `scripts/audit/report.py` | 五种 Markdown 报表:`symbols`、`host-to-ext`、`modules`、`platform`、`panelkind` | 2 |
| `scripts/audit/check_panel_boundary.py` | 面板边界棘轮门禁(`extensions/**` 里 `App`/`Workspace` 引用只减不增) | 8 |
| `scripts/audit/test_check_panel_boundary.py` | 门禁的单元测试 | 8 |
| `scripts/audit/panel-boundary.baseline.json` | 门禁基线 | 8 |
| `docs/dozer-v2/bytehost-H0/data/*.md` | 机械报表快照(带基线提交) | 2 |
| `docs/dozer-v2/bytehost-H0/01-panelkind.md` | Q8:`PanelKind` 引用分类与注册制分批建议 | 3 |
| `docs/dozer-v2/bytehost-H0/02-panel-to-host.md` | Q10:面板对 host 的符号依赖表与去向建议 | 4 |
| `docs/dozer-v2/bytehost-H0/03-host-to-panel.md` | Q13 + E3 登记清单初版 | 5 |
| `docs/dozer-v2/bytehost-H0/E3-registry.tsv` | E3 登记清单(特判/寄存条目) | 5 |
| `docs/dozer-v2/bytehost-H0/04-shared-modules.md` | Q7:共享领域模块画像与归属建议 | 6 |
| `docs/dozer-v2/bytehost-H0/05-platform-overlays.md` | `platform/` overlay 迁移清单 | 6 |
| `docs/dozer-v2/bytehost-H0/06-open-items-evidence.md` | O1/O2/O3/O4/O6/O8 的证据 | 7 |
| `docs/dozer-v2/bytehost-H0/00-summary.md` | H0 汇总:结论、规格需要的修订、H1 的候选范围(不是 H1 计划) | 9 |

---

### Task 1: 审计工具 — 依赖边提取器

**Files:**
- Create: `scripts/audit/test_edges.py`
- Create: `scripts/audit/edges.py`

**Interfaces:**
- Produces(后续任务依赖):
  - `edges.SRC: str`——`crates/dozer-app/src` 的绝对路径
  - `edges.unit_of(rel: str) -> str`——相对 `SRC` 的路径 → 单元名:`extensions/todo/view.rs` → `ext:todo`,`extensions/git_log.rs` → `ext:git_log`,`app/update.rs` → `layer:app`,`secrets.rs` → `root:secrets`
  - `edges.flatten(tree: str) -> list[str]`——`a::{B, c::{D, E as F}}` → `["a::B","a::c::D","a::c::E"]`
  - `edges.edges_of(text: str) -> collections.Counter[(module, symbol)]`——一个文件里所有 `crate::<module>[::<symbol>]` 引用(含 `use crate::…;` 展开与行内路径,忽略注释与 `super::`/`self::`)
  - CLI:`python3 scripts/audit/edges.py` 输出 TSV(`file from_unit to_module to_symbol count`)

- [ ] **Step 1: 建专用 worktree 与分支**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytehost-h0 -b bytehost-h0 main
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0
git log --oneline | head -1 && git status --short
ls docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md
```

Expected: 干净、分支 `bytehost-h0`。**最后一条 `ls` 若报文件不存在**,说明规格还没提交到 `main`(它此刻是主 checkout 里的未跟踪文件):把规格从主 checkout 复制进 worktree(`cp ../dozer/docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md docs/superpowers/specs/`),并在 Task 9 提交规格改动时一并提交它。此后**所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytehost-h0/` 前缀**。

- [ ] **Step 2: 写失败的测试**

创建 `scripts/audit/test_edges.py`:

```python
import importlib.util, os, unittest
spec = importlib.util.spec_from_file_location("edges", os.path.join(os.path.dirname(__file__), "edges.py"))
edges = importlib.util.module_from_spec(spec); spec.loader.exec_module(edges)

class Flatten(unittest.TestCase):
    def test_nested_and_alias(self):
        self.assertEqual(edges.flatten("a::{B, c::{D, E as F}, self}"), ["a::B", "a::c::D", "a::c::E", "a::self"])
class Edges(unittest.TestCase):
    def test_multiline_use_and_inline(self):
        got = dict(edges.edges_of("use crate::app::{\n  App, HoverId,\n};\nlet x = crate::theme::color();"))
        self.assertEqual(got, {("app", "App"): 1, ("app", "HoverId"): 1, ("theme", "color"): 1})
    def test_comments_ignored(self):
        self.assertEqual(dict(edges.edges_of("// use crate::app::App;\n/* crate::theme::x */")), {})
    def test_super_not_counted(self):
        self.assertEqual(dict(edges.edges_of("use super::*; use self::a::B;")), {})
    def test_unit_of(self):
        u = edges.unit_of
        self.assertEqual(u("extensions/todo/view.rs"), "ext:todo")
        self.assertEqual(u("extensions/git_log.rs"), "ext:git_log")
        self.assertEqual(u("app/update.rs"), "layer:app")
        self.assertEqual(u("secrets.rs"), "root:secrets")
if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 3: 确认测试失败**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0 && python3 scripts/audit/test_edges.py 2>&1 | tail -5`
Expected: FAIL(`FileNotFoundError`/`No such file`:`edges.py` 还不存在)。

- [ ] **Step 4: 写提取器**

创建 `scripts/audit/edges.py`:

```python
#!/usr/bin/env python3
"""dozer-app 内部依赖边提取(bytehost H0 审计用,一次性工具,不进产品构建)。

输出 TSV,表头:file  from_unit  to_module  to_symbol  count
- `use crate::a::{B, c::D};` 会被展开成 (a, B) 与 (a, c::D) 两条边的符号部分取到第一段;
- 行内 `crate::a::B::f()` 也算,符号取 `crate::a::` 之后的第一段;
- `use super::` / `use self::` 不算(它们是单元内部引用)。
- 单元(from_unit):extensions/<名>[/…] -> ext:<名>;app|workspace|chrome|platform|preview|theme|term|
  tabular|assets 等目录 -> layer:<目录>;根下文件 -> root:<文件名>。
"""
import os, re, sys, collections

ROOT = os.path.normpath(os.path.join(os.path.dirname(__file__), "..", ".."))
SRC = os.path.join(ROOT, "crates", "dozer-app", "src")


def unit_of(rel):
    parts = rel.split("/")
    if parts[0] == "extensions":
        if len(parts) == 2:
            return "ext:" + parts[1][:-3]
        return "ext:" + parts[1]
    if len(parts) > 1:
        return "layer:" + parts[0]
    return "root:" + parts[0][:-3]


def strip_comments(text):
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def flatten(tree):
    """`a::{B, c::{D, E}}` -> ['a::B', 'a::c::D', 'a::c::E'];`a::*` 保留为 'a::*'。"""
    tree = re.sub(r"\s+as\s+[A-Za-z_][A-Za-z0-9_]*", "", tree)  # 去掉 `as X`
    tree = re.sub(r"\s+", "", tree)
    def parse(s, i, prefix):
        out, cur = [], ""
        while i < len(s):
            c = s[i]
            if c == "{":
                sub, i = parse(s, i + 1, prefix + cur)
                out += sub
                cur = ""
                continue
            if c == "}":
                if cur:
                    out.append(prefix + cur)
                return out, i + 1
            if c == ",":
                if cur:
                    out.append(prefix + cur)
                cur = ""
            else:
                cur += c
            i += 1
        if cur:
            out.append(prefix + cur)
        return out, i
    res, _ = parse(tree, 0, "")
    return res


USE_RE = re.compile(r"\buse\s+crate::([^;]*);")
INLINE_RE = re.compile(r"\bcrate::([a-z_][a-z0-9_]*)(?:::([A-Za-z_][A-Za-z0-9_]*))?")


def edges_of(text):
    text = strip_comments(text)
    found = collections.Counter()
    for m in USE_RE.finditer(text):
        body = m.group(1)
        for path in flatten(body):
            seg = path.split("::")
            mod = seg[0]
            sym = seg[1] if len(seg) > 1 else ""
            found[(mod, sym)] += 1
    stripped = USE_RE.sub("", text)
    for m in INLINE_RE.finditer(stripped):
        found[(m.group(1), m.group(2) or "")] += 1
    return found


def main():
    print("file\tfrom_unit\tto_module\tto_symbol\tcount")
    for dp, _, fns in sorted(os.walk(SRC)):
        for fn in sorted(fns):
            if not fn.endswith(".rs"):
                continue
            path = os.path.join(dp, fn)
            rel = os.path.relpath(path, SRC).replace(os.sep, "/")
            with open(path, encoding="utf-8") as f:
                text = f.read()
            unit = unit_of(rel)
            for (mod, sym), n in sorted(edges_of(text).items()):
                print(f"{rel}\t{unit}\t{mod}\t{sym}\t{n}")


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 5: 测试通过,并在真实源码上跑一遍**

Run: `python3 scripts/audit/test_edges.py 2>&1 | tail -3`
Expected: `Ran 5 tests` … `OK`。

Run: `python3 scripts/audit/edges.py | head -3 && python3 scripts/audit/edges.py | wc -l`
Expected: 第一行是表头 `file	from_unit	to_module	to_symbol	count`;总行数在 800–1100 之间(草稿里是 916;大幅偏离说明 `main` 上有大改动或提取器坏了,先排查再继续)。

Run: `python3 scripts/audit/edges.py | awk -F'\t' '$3=="app"&&$4=="App"&&$2 ~ /^ext:/{n+=$5}END{print n}'`
Expected: `8`(面板代码引用 `app::App` 的次数,草稿值;不同请记入 Task 9 的差异表)。

- [ ] **Step 6: Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0
git status --short
git add scripts/audit/edges.py scripts/audit/test_edges.py
git diff --cached --stat
git commit -m "chore(audit): add crate:: dependency edge extractor for the bytehost H0 audit

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: `git diff --cached --stat` 只有这两个文件。

---

### Task 2: 审计工具 — 五种报表与数据快照

**Files:**
- Create: `scripts/audit/report.py`
- Create: `docs/dozer-v2/bytehost-H0/data/{symbols,host-to-ext,modules,platform,panelkind}.md`

**Interfaces:**
- Consumes: Task 1 的 `edges.{SRC, unit_of, edges_of}`
- Produces(Task 3–7 的输入):`python3 scripts/audit/report.py <cmd>` 向 stdout 输出 Markdown 表;`<cmd>` 取 `symbols`(面板→host 符号)、`host-to-ext`(host 层→面板)、`modules`(根下共享模块画像)、`platform`(`platform/*.rs` 逐文件)、`panelkind`(`PanelKind` 按文件分类)。`data/*.md` 是它们在某个基线提交上的快照。

- [ ] **Step 1: 写报表脚本**

创建 `scripts/audit/report.py`:

```python
#!/usr/bin/env python3
"""bytehost H0 审计报表:基于 edges.py 的依赖边,输出 Markdown 表格到 stdout。

用法: report.py {symbols|host-to-ext|modules|platform|panelkind}
"""
import collections, importlib.util, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("edges", os.path.join(HERE, "edges.py"))
edges = importlib.util.module_from_spec(_spec); _spec.loader.exec_module(edges)

HOST_LAYERS = {"layer:app", "layer:workspace", "layer:chrome", "layer:platform", "layer:preview"}
SRC = edges.SRC


def all_files():
    for dp, _, fns in sorted(os.walk(SRC)):
        for fn in sorted(fns):
            if fn.endswith(".rs"):
                p = os.path.join(dp, fn)
                yield os.path.relpath(p, SRC).replace(os.sep, "/"), p


def read(p):
    with open(p, encoding="utf-8") as f:
        return f.read()


def all_edges():
    """[(file, unit, module, symbol, count)]"""
    out = []
    for rel, p in all_files():
        unit = edges.unit_of(rel)
        for (mod, sym), n in sorted(edges.edges_of(read(p)).items()):
            out.append((rel, unit, mod, sym, n))
    return out


def table(header, rows):
    print("| " + " | ".join(header) + " |")
    print("|" + "|".join("---" for _ in header) + "|")
    for r in rows:
        print("| " + " | ".join(str(c) for c in r) + " |")


def cmd_symbols():
    """面板(ext:*)依赖的 host/根模块符号,按 (模块,符号) 聚合——Q10 的原始表。"""
    agg = collections.defaultdict(lambda: [0, set()])
    for _, unit, mod, sym, n in all_edges():
        if unit.startswith("ext:") and mod != "extensions":
            agg[(mod, sym)][0] += n
            agg[(mod, sym)][1].add(unit[4:])
    rows = [(f"`{m}::{s}`" if s else f"`{m}`", c, len(u), ", ".join(sorted(u)))
            for (m, s), (c, u) in agg.items()]
    rows.sort(key=lambda r: (-r[2], -r[1], r[0]))
    table(["符号", "引用次数", "使用面板数", "使用面板"], rows)


def cmd_host_to_ext():
    """host 层(app/workspace/chrome/platform/preview)对 extensions 的直接引用:按文件×被引用扩展聚合。"""
    agg = collections.Counter()
    for rel, unit, mod, sym, n in all_edges():
        if unit in HOST_LAYERS and mod == "extensions" and sym:
            agg[(rel, sym)] += n
    rows = [(f"`{f}`", e, n) for (f, e), n in sorted(agg.items(), key=lambda kv: (-kv[1], kv[0]))]
    table(["host 文件", "被引用的 extension", "引用次数"], rows)


ROOT_MODULES = None


def cmd_modules():
    """根下共享模块的画像:扇入(谁用它)、扇出(它用谁)、是否依赖 iced、公开项数、行数。"""
    es = all_edges()
    roots = sorted({rel[:-3] for rel, _ in all_files() if "/" not in rel} - {"main", "extensions"})
    rows = []
    for r in roots:
        fan_in = sorted({u for _, u, m, _, _ in es if m == r and u != f"root:{r}"})
        fan_out = sorted({m for _, u, m, _, _ in es if u == f"root:{r}" and m != r})
        text = read(os.path.join(SRC, r + ".rs"))
        iced = bool(re.search(r"^\s*(pub\s+)?use\s+(iced|byteui)", text, re.M))
        pubs = len(re.findall(r"^\s*pub(\([a-z]+\))?\s+(fn|struct|enum|trait|type|const)\b", text, re.M))
        rows.append((f"`{r}`", len(fan_in), ", ".join(x.replace("root:", "").replace("layer:", "") for x in fan_in),
                     ", ".join(fan_out), "是" if iced else "否", pubs, text.count("\n") + 1))
    rows.sort(key=lambda r: (-r[1], r[0]))
    table(["模块", "扇入单元数", "扇入", "扇出(模块)", "依赖 iced/byteui", "公开项", "行数"], rows)


def cmd_platform():
    """platform/*.rs 逐文件:引用了哪些 extension/根模块,据此给出归属建议。"""
    per = collections.defaultdict(lambda: {"ext": collections.Counter(), "root": collections.Counter()})
    for rel, unit, mod, sym, n in all_edges():
        if not rel.startswith("platform/") or rel == "platform/mod.rs":
            continue
        if mod == "extensions" and sym:
            per[rel]["ext"][sym] += n
        elif mod not in ("platform", "extensions"):
            per[rel]["root"][mod] += n
    rows = []
    for rel, _ in all_files():
        if not rel.startswith("platform/") or rel == "platform/mod.rs":
            continue
        e, r = per[rel]["ext"], per[rel]["root"]
        if len(e) == 0:
            kind = "通用(候选 host)"
        elif len(e) == 1:
            kind = f"面板专属:{next(iter(e))}"
        else:
            kind = "混合,需人工裁决"
        rows.append((f"`{rel}`", ", ".join(f"{k}×{v}" for k, v in e.most_common()),
                     ", ".join(f"{k}×{v}" for k, v in r.most_common(5)), kind))
    table(["文件", "引用的 extension", "引用的根/层模块(前5)", "归属建议(机械判断)"], rows)


VARIANT_RE = re.compile(r"PanelKind::([A-Z][A-Za-z]+)")


def cmd_panelkind():
    """PanelKind 引用分类(Q8):按行分"点名变体"与"仅类型";连续 15 行内出现 ≥3 个不同变体记为"遍历候选簇"。"""
    rows, total_named, total_type = [], 0, 0
    for rel, p in all_files():
        lines = read(p).split("\n")
        named, typ, clusters = 0, 0, 0
        last_cluster_end = -1
        for i, ln in enumerate(lines):
            if "PanelKind" not in ln:
                continue
            if VARIANT_RE.search(ln):
                named += 1
            else:
                typ += 1
        for i in range(len(lines)):
            window = "\n".join(lines[i:i + 15])
            if len(set(VARIANT_RE.findall(window))) >= 3 and i > last_cluster_end:
                clusters += 1
                last_cluster_end = i + 15
        if named or typ:
            rows.append((f"`{rel}`", named, typ, clusters))
            total_named += named
            total_type += typ
    rows.sort(key=lambda r: (-(r[1] + r[2]), r[0]))
    rows.append(("**合计**", total_named, total_type, sum(r[3] for r in rows)))
    table(["文件", "点名变体的行", "仅类型的行", "遍历候选簇"], rows)


CMDS = {"symbols": cmd_symbols, "host-to-ext": cmd_host_to_ext, "modules": cmd_modules,
        "platform": cmd_platform, "panelkind": cmd_panelkind}

if __name__ == "__main__":
    if len(sys.argv) != 2 or sys.argv[1] not in CMDS:
        sys.exit(__doc__)
    CMDS[sys.argv[1]]()
```

- [ ] **Step 2: 逐个跑五个报表,核对输出形状**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0 && for c in symbols host-to-ext modules platform panelkind; do echo "== $c"; python3 scripts/audit/report.py $c | head -4; done`
Expected: 每个都以一行表头、一行 `|---|…` 分隔线开头,后面是数据行,无 Traceback。

Run: `python3 scripts/audit/report.py symbols | grep -E 'app::App`|app::HoverId`|chrome::native_menu`'`
Expected: 三行都在;`app::App` 的"引用次数"为 8、"使用面板数"为 7(草稿值)。

Run: `python3 scripts/audit/report.py platform | grep -E 'overlay_window|overlay_gpu|picker'`
Expected: 三个文件的"归属建议"都是 `通用(候选 host)`。

Run: `python3 scripts/audit/report.py panelkind | tail -1`
Expected: 合计行,点名变体的行与仅类型的行之和接近 `grep -rn PanelKind crates/dozer-app/src | wc -l`(草稿里该 grep 得 973;允许差几行——一行里既点名又出现多次时 grep 与报表按行计数一致,差异应为 0)。如差异超过 5,先查原因。

- [ ] **Step 3: 生成快照,文件头记录基线提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0
mkdir -p docs/dozer-v2/bytehost-H0/data
BASE=$(git rev-parse --short HEAD)
for c in symbols host-to-ext modules platform panelkind; do
  { echo "<!-- bytehost H0 机械报表快照;基线提交 $BASE;生成命令: python3 scripts/audit/report.py $c -->"
    echo
    python3 scripts/audit/report.py $c; } > docs/dozer-v2/bytehost-H0/data/$c.md
done
head -3 docs/dozer-v2/bytehost-H0/data/symbols.md; wc -l docs/dozer-v2/bytehost-H0/data/*.md
```

Expected: 五个文件,首行是带基线提交的注释。

- [ ] **Step 4: Commit**

```bash
git status --short
git add scripts/audit/report.py docs/dozer-v2/bytehost-H0/data
git diff --cached --stat
git commit -m "chore(audit): add H0 report generators and baseline snapshots

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: `git diff --cached --stat` 里没有 `crates/`。

---

### Task 3: Q8 — `PanelKind` 引用分类与注册制分批

**Files:**
- Create: `docs/dozer-v2/bytehost-H0/01-panelkind.md`

**Interfaces:**
- Consumes: `data/panelkind.md`、`scripts/audit/report.py panelkind`
- Produces(Task 5、Task 9 依赖):文档里的"逐文件人工分类表"与"注册制迁移批次建议"。每个文件被标成下面四类之一:**遍历**(host 对所有面板一视同仁地循环/穷举,迁移后改成遍历 registry)、**特判**(只对某几个面板有特殊行为,迁移后必须变成面板自带的钩子或登记进 E3)、**类型传递**(只是把 `PanelKind` 当作 id 传来传去,迁移后改成注册制 id,机械替换)、**元数据**(默认栏位、标题、图标之类,迁移后进注册信息)。

- [ ] **Step 1: 取机械数据并核实总数**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0 && python3 scripts/audit/report.py panelkind | head -14 && grep -rn "PanelKind" crates/dozer-app/src | wc -l && grep -rln "PanelKind" crates/dozer-app/src | wc -l && grep -c '^    [A-Z]' <(sed -n '/pub enum PanelKind/,/^}/p' crates/dozer-app/src/app/state.rs)`
Expected: 前 12 个文件按引用量降序(草稿里依次是 `app/app.rs`、`webview_geometry.rs`、`chrome/rail.rs`、`app/update.rs`、`workspace/state.rs`、`app/layout.rs`、`platform/window_events.rs`、`workspace/view.rs`、`app/message.rs`、`preview/view.rs`、`app/view.rs`、`app/state.rs`);总行数 973、文件数 31、变体数 12(草稿值)。

- [ ] **Step 2: 对前 6 个文件逐个读代码,人工分类**

对 `app/app.rs`、`webview_geometry.rs`、`chrome/rail.rs`、`app/update.rs`、`workspace/state.rs`、`app/layout.rs` 逐个:

Run: `grep -n "PanelKind" crates/dozer-app/src/<文件> | head -60`(再用 Read 看每个"遍历候选簇"附近 15 行)
Expected: 能为每个文件回答:(a) 它的 `PanelKind` 引用主要是哪一类;(b) 如果有"特判",列出**被特判的变体与行为**(例:"`webview_geometry.rs` 对 `Web`/`Ssh` 特判预览矩形下推");(c) 迁移成注册制需要面板提供什么钩子。

- [ ] **Step 3: 抽样校验启发式**

随机取 30 行含 `PanelKind` 的行(`grep -rn "PanelKind" crates/dozer-app/src | shuf -n 30`),为每行判断"点名变体"/"仅类型"是否与报表规则一致(规则:行内有 `PanelKind::<大写开头>` 即点名)。

Expected: 一致率 ≥ 80%。低于 80% 时,文档里写明"启发式只作线索,以人工分类为准",并把不一致的典型写法记下来(例:`use PanelKind::*` 后直接写变体名)。

- [ ] **Step 4: 写文档**

创建 `docs/dozer-v2/bytehost-H0/01-panelkind.md`,章节固定为:

1. **头部**:基线提交、生成命令(Step 1 的命令)、本文回答的问题(要求文档 Q8、规格 O5)。
2. **总数**:`PanelKind` 引用行数、文件数、变体数(Step 1 的实测值)。
3. **逐文件分类表**:列 = 文件 / 点名变体的行 / 仅类型的行 / 遍历候选簇 / **人工分类**(遍历、特判、类型传递、元数据,可多选) / **依据**(文件:行)。前 6 个文件必须填人工分类;其余文件至少填"类型传递"或"特判"并写一句话依据。
4. **特判清单**:每条特判一行——文件:行、被特判的变体、行为描述、迁移后落点(面板钩子 / 登记 E3 / 产品组合根)。这张表是 Task 5 的 E3 登记清单的主要来源之一。
5. **默认栏位(O5)**:`PanelKind::default_side()`(`app/state.rs`)与 `panel_layouts.rs`/`app/layout.rs` 里默认布局的用法清单;给出倾向——默认栏位进注册信息还是产品组合根——并写依据。**这是倾向不是裁决**,规格 O5 仍标未决。
6. **注册制分批建议**:按"类型传递(机械替换)→ 元数据 → 遍历 → 特判"的顺序,估算每批涉及的文件数与行数(用上面的表汇总),并写明建议的第一批(最小、最安全)。
7. **抽样校验结果**:Step 3 的一致率与不一致写法。

- [ ] **Step 5: Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0
git status --short
git add docs/dozer-v2/bytehost-H0/01-panelkind.md
git diff --cached --stat
git commit -m "docs(bytehost): H0 audit of PanelKind references (Q8, O5)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Q10 — 面板对 host 的符号依赖与去向

**Files:**
- Create: `docs/dozer-v2/bytehost-H0/02-panel-to-host.md`

**Interfaces:**
- Consumes: `data/symbols.md`(`python3 scripts/audit/report.py symbols`)
- Produces(Task 8、Task 9 依赖):一张"符号 → 去向"表,每个符号被判为四种去向之一——**byteui**(通用 UI 组件,应进 byteui 仓库)、**host SDK**(宿主契约,面板依赖的稳定接口)、**留产品**(Dozer 专属,Digger 不需要)、**重构消除**(面板不该依赖它,靠重构去掉,如 `App`/`Workspace`)。

- [ ] **Step 1: 取机械数据**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0 && python3 scripts/audit/report.py symbols | head -40`
Expected: 按"使用面板数"降序;前几行是 `chrome::homespace`、`app::TextInputTarget`、`app::App`、`app::HoverId`、`chrome::menu`、`theme`、`theme::region`、`chrome::native_menu`、`app::PanelKind`、`chrome::tab_widget`……(草稿值)。

- [ ] **Step 2: 检查每个高频符号的引用是否只在测试里**

对表中"使用面板数 ≥ 2"的每个符号:

Run: `grep -rn "<符号的最后一段>" crates/dozer-app/src/extensions | grep -v "^.*://" | head -20`,再判断每处是不是在 `#[cfg(test)]`/`mod tests` 里(Read 看上下文)。
Expected: 对每个符号能写出"生产引用 N 处 / 仅测试引用 M 处"。仅测试引用的符号标记"测试专用,不计入依赖"。

- [ ] **Step 3: 对每个符号读定义,判去向**

对每个"使用面板数 ≥ 2"的符号,读它的定义(`grep -rn "pub .* <符号>" crates/dozer-app/src`)和 2–3 个使用点,回答:它是通用 UI 能力吗(→ byteui)?是宿主要提供给所有面板的契约吗(→ host SDK)?是否带 Dozer 专属语义(→ 留产品)?是否本不该被面板依赖(→ 重构消除,并写"面板需要它是为了什么、怎样不需要它")?

Expected: 每个符号一行结论 + 依据。`App`、`Workspace`、`PanelKind` 预计落"重构消除/元数据",`HoverId`、`TextInputTarget`、`theme::region`、`chrome::native_menu`/`menu`/`tab_widget` 预计需要逐个看——这些是 Q10 的核心。

- [ ] **Step 4: 抽查机械表**

取表的前 10 行,每行随机选一个使用面板,打开它的源码确认确实有这个引用(`grep -n`)。
Expected: 10/10 确认;有不符的写进文档的"抽查结果"并说明原因(多半是 `pub use` 重导出或宏)。

- [ ] **Step 5: 写文档**

创建 `docs/dozer-v2/bytehost-H0/02-panel-to-host.md`,章节固定为:头部(基线提交 + 命令 + 回答 Q10);**符号去向表**(列:符号 / 引用次数 / 使用面板 / 生产引用 / 仅测试引用 / **去向** / 依据);**被判为"重构消除"的符号的消除思路**(每个一两句,不是设计);**抽查结果**;**对 byteui 的影响**(被判进 byteui 的符号列表,供另开 byteui 仓库的计划使用,**本任务不改 byteui**)。

- [ ] **Step 6: Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0
git status --short
git add docs/dozer-v2/bytehost-H0/02-panel-to-host.md
git diff --cached --stat
git commit -m "docs(bytehost): H0 audit of panel-to-host symbol dependencies (Q10)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Q13 与 E3 — host 对面板的硬编码与登记清单初版

**Files:**
- Create: `docs/dozer-v2/bytehost-H0/03-host-to-panel.md`
- Create: `docs/dozer-v2/bytehost-H0/E3-registry.tsv`

**Interfaces:**
- Consumes: `data/host-to-ext.md`、`01-panelkind.md` 的特判清单
- Produces(Task 8、Task 9 依赖):`E3-registry.tsv`,列固定为(制表符分隔,首行为表头):`id	位置	类别	对象	原因	移除条件`。`类别` 取 `panel-special-case`(host 内点名某面板的特判)、`parked-service`(暂存在 host 的公共服务,见规格 §3.5)、`overlay`(面板专属 overlay 仍在 host 的 `platform/`)三者之一。`id` 形如 `E3-001`,顺序编号。

- [ ] **Step 1: 取机械数据**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0 && python3 scripts/audit/report.py host-to-ext | head -40`
Expected: 按引用次数降序,首批是 `platform/window_events.rs`×files(12)、`app/update.rs`×group_chat(11)、`app/app.rs`×agent_context(9)、`workspace/state.rs`×toast(8)……(草稿值)。**`toast` 是 host 基础设施,不算面板耦合**(规格 §3.1),审计时单独标注,不进 E3。

- [ ] **Step 2: 对每个 host 文件×面板对读代码分类**

对表中引用次数 ≥ 3 的每一行(以及 Task 3 特判清单里的条目):读该 host 文件中引用该面板的位置,分三类:
- **A 编排**:host 持有该面板的 state/message 并转发(`Message::Todo(...)` 包装、`App` 里有 `todo: todo::State` 字段)——是"面板接入 host"的正常形态,迁移后由 registry 的通用接口承担。
- **B 特判**:host 对这个面板有**别的面板没有的**特殊逻辑(例:窗口事件里对 `files` 的拖拽、`app/update.rs` 里分支选择器直接调 `git_log::update`)。
- **C 越界**:host 直接读写面板内部字段或调用面板内部函数(应改为面板暴露的接口)。

Expected: 每行一个分类 + 依据(文件:行);B、C 类每条一行写进 `E3-registry.tsv`。

- [ ] **Step 3: 专查 Q13**

Run: `grep -n "git_log::\|FileFilter" crates/dozer-app/src/app/update.rs crates/dozer-app/src/app/state.rs`
Expected: 找到 `app/update.rs` 里分支选择器对 `git_log::update`/`git_log::load_branch_picker_data` 的直接调用与 `app/state.rs` 对 `FileFilter` 的引用;对每处写明它是 A/B/C 哪类,以及"经 registry"需要什么新接口。

- [ ] **Step 4: 暂存服务入 E3**

规格 §3.5 列出的暂存条目——Project context、`secrets`、`external_apps`、`capabilities`、`git_accounts`、`conversation`、`transcript`——各在 `E3-registry.tsv` 里登记一行,类别 `parked-service`,"移除条件"写"候选最终去向 + 触发条件"(去向在 Task 6 的 `04-shared-modules.md` 里裁决,这里先写"待 Task 6 裁决"之外的**具体触发条件**,例如"Digger 接入 Todo 面板时需要它")。**若 Task 6 还没做,本步先登记行,"原因"列写客观事实(扇入单元),"移除条件"在 Task 6 完成后回填。**

- [ ] **Step 5: 写文档与登记清单**

创建 `03-host-to-panel.md`:头部(基线提交 + 命令 + 回答 Q13);**host×面板引用分类表**(列:host 文件 / 面板 / 引用次数 / **A/B/C** / 依据);**Q13 专节**;**对 `Message` 包装与 `App` 字段的观察**(`app/app.rs`、`app/message.rs` 里 host 持有面板 state 的数量,给出数字);**E3 登记清单的口径**(什么算特判、什么不算,及 `toast` 的例外)。

创建 `E3-registry.tsv`(首行表头,后续每条一行,制表符分隔)。

- [ ] **Step 6: 校验登记清单格式**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0 && awk -F'\t' 'NR==1{print NF; next} NF!=6{bad++} END{print (bad+0) " malformed rows"}' docs/dozer-v2/bytehost-H0/E3-registry.tsv && cut -f3 docs/dozer-v2/bytehost-H0/E3-registry.tsv | tail -n +2 | sort | uniq -c`
Expected: 第一行 `6`;`0 malformed rows`;类别只有 `panel-special-case`、`parked-service`、`overlay` 三种。

- [ ] **Step 7: Commit**

```bash
git status --short
git add docs/dozer-v2/bytehost-H0/03-host-to-panel.md docs/dozer-v2/bytehost-H0/E3-registry.tsv
git diff --cached --stat
git commit -m "docs(bytehost): H0 audit of host-to-panel coupling and E3 registry v0 (Q13)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Q7 与 overlay — 共享领域模块画像与 `platform/` 迁移清单

**Files:**
- Create: `docs/dozer-v2/bytehost-H0/04-shared-modules.md`
- Create: `docs/dozer-v2/bytehost-H0/05-platform-overlays.md`
- Modify: `docs/dozer-v2/bytehost-H0/E3-registry.tsv`(回填 Task 5 的 `parked-service` 与新增 `overlay` 行)

**Interfaces:**
- Consumes: `data/modules.md`、`data/platform.md`
- Produces: 每个根下模块的去向建议——**host**(产品无关机制)、**面板**(只被一个面板用,应随该面板走)、**领域库**(多面板共享的纯数据/领域模型,应成独立库或下沉 `dozerd`)、**产品层**(Dozer 专属);每个 `platform/*.rs` 的归属(通用 → host;专属 → 随面板走;混合 → 拆)。

- [ ] **Step 1: 取机械数据**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0 && python3 scripts/audit/report.py modules && python3 scripts/audit/report.py platform`
Expected: `modules` 里能看到 `theme`、`assets`、`project`、`capabilities`、`conversation`、`project_meta`、`runtime`、`menu_spec`、`external_apps`、`git_accounts`、`layout`、`secrets`、`transcript`、`webview_geometry`、`event`、`frosted`、`keymap`、`open_projects`、`osc`、`panel_layouts`、`preview_state`(草稿值,共 21 行);`platform` 约 21 行。

- [ ] **Step 2: 对规格 §3.5 点名的模块逐个读,判去向**

对 `conversation`、`transcript`、`project`、`project_meta`、`open_projects`、`secrets`、`external_apps`、`capabilities`、`git_accounts`:读模块的公开项(`grep -n "^pub" crates/dozer-app/src/<m>.rs`)与 2–3 个扇入点,回答:(a) 是纯数据/领域逻辑,还是带 UI(报表"依赖 iced"列只是线索);(b) 是否带 Dozer 专属语义(治理、交付、验收);(c) 扇入里有几个独立面板——**Digger 复用的五个面板(Todo/Conversation/Agent/Project/Files)里有几个用它**;(d) 去向建议与依据。

Expected: 每个模块一行结论;`conversation` 与 `transcript` 要特别检查它们是否应下沉 `dozerd`(`crates/dozerd/src/transcripts` 已存在,对比两边是否重复)。

- [ ] **Step 3: 对其余根模块与 `dozer-core` 公开模块做归类**

对 `runtime`、`menu_spec`、`event`、`frosted`、`keymap`、`osc`、`panel_layouts`、`layout`、`preview_state`、`webview_geometry`、`assets`、`theme`:各给一行去向建议(host / 面板 / 领域库 / 产品层)与一句依据。

Run: `grep -n "^pub mod\|^pub use" crates/dozer-core/src/lib.rs`
Expected: 列出 `dozer-core` 的公开模块;对每个标注"产品无关 / Dozer 专属"(例:`log` 的 scope 约定产品无关;UDS 协议专属于 dozerd 通信)。**规格 §3.1 的"日志机制归属"在这一步裁决**。

- [ ] **Step 4: 对每个 `platform/*.rs` 读代码,核对机械归属建议**

对 `data/platform.md` 里每一行:读文件头注释与主要类型,判断机械建议是否正确;"混合"的逐个读(预计 `confirm_overlay.rs`、`file_history_overlay.rs`)。例外检查:`overlay_window.rs`/`overlay_gpu.rs`/`overlay_focus.rs`/`window.rs`/`window_events.rs`/`picker.rs`/`file_drag.rs`——预计是 host 机制,但 `window_events.rs` 对 `files`×12、`browser`×6、`project`×6 的引用说明它混了面板特判(Task 5 已登记,这里引用其 E3 id)。

Expected: 每个文件三选一:**host**、**随面板走(写明哪个面板)**、**拆分(写明拆成什么)**。

- [ ] **Step 5: 写文档、回填 E3**

创建 `04-shared-modules.md`:头部;**根模块去向表**(列:模块 / 扇入面板 / 是否 Digger 复用面板使用 / 是否带 iced / **去向** / 依据);**§3.5 点名模块的专节**;**`dozer-core` 公开模块清单**;**对 O4、O7、O8 的证据摘要**(各三五行,不下裁决)。

创建 `05-platform-overlays.md`:头部;**逐文件表**(列:文件 / 机械建议 / **人工裁决** / 依据 / 对应 E3 id);**迁移顺序建议**(哪些随面板走时可以先动、哪些依赖 host 的 Surface 机制稳定)。

回填 `E3-registry.tsv`:Task 5 的 `parked-service` 行补全"移除条件";为每个"随面板走"的 overlay 新增 `overlay` 类行(移除条件 = "对应面板迁出 host 时随之迁走")。

- [ ] **Step 6: 复核登记清单格式**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0 && awk -F'\t' 'NR>1 && (NF!=6 || $6=="")' docs/dozer-v2/bytehost-H0/E3-registry.tsv | wc -l`
Expected: `0`(没有列数不对或"移除条件"为空的行)。

- [ ] **Step 7: Commit**

```bash
git status --short
git add docs/dozer-v2/bytehost-H0/04-shared-modules.md docs/dozer-v2/bytehost-H0/05-platform-overlays.md docs/dozer-v2/bytehost-H0/E3-registry.tsv
git diff --cached --stat
git commit -m "docs(bytehost): H0 audit of shared modules (Q7) and platform overlays

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 7: 未决项证据 — Agent 分布、Files→Preview、Terminal、Project context

**Files:**
- Create: `docs/dozer-v2/bytehost-H0/06-open-items-evidence.md`

**Interfaces:**
- Consumes: `data/*.md`、`E3-registry.tsv`
- Produces: 对规格 O1、O2、O3、O6 各一节证据(只摆事实与倾向,**不下裁决**——规格要求这些保持未决),供用户评审时拍板。

- [ ] **Step 1: O1 / Q15 — Agent 代码分布**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0
for d in crates/dozerd/src crates/dozer-app/src crates/dozer-core/src crates/dozer-client/src crates/dozer-hook/src crates/dozer-mcp/src; do
  printf '%s\t' "$d"; grep -rlE "AgentKind|agent_launch|launch_agent|spawn_agent|headless_agent|agent_context|hook" "$d" 2>/dev/null | wc -l
done
grep -rn "enum AgentKind" crates/*/src | head
ls crates/dozerd/src | grep -E "agent|session|registry|headless|hook"
ls crates/dozer-app/src/extensions | grep -E "agent"
```
Expected: 每个 crate 一行"含 agent 相关符号的文件数";能找到 `AgentKind` 的定义 crate、`dozerd` 里的 `session.rs`/`headless_agent.rs`/`registry.rs`/`agent_context.rs`、`dozer-app` 里的 `extensions/agent_context.rs`。

再读 `extensions/agent_context.rs`、`app/` 中 agent 会话相关代码(`grep -rn "agent" crates/dozer-app/src/app/update.rs | head -40`),回答:**哪些 agent 能力在 `dozerd`(会话存活、启动、hook)、哪些在 `dozer-app`(卡片展示、picker、icon)**;Agent 面板主体("展示")与会话运行服务("机制")的天然切口在哪里。

- [ ] **Step 2: O3 — Files 对 Preview 的现有引用**

Run: `python3 scripts/audit/edges.py | awk -F'\t' '$2=="ext:files" && ($3=="preview"||($3=="extensions"&&$4!="files")||$3=="preview_state"){print $1"\t"$3"\t"$4"\t"$5}'`
Expected: 列出 Files 面板对 `preview`、`preview_state` 及其他 extension 的引用;再读这些引用点,回答:**Files 触发"打开"时现在走哪条消息/函数、传的什么**(路径?`PreviewTarget`?),以及一个"通用打开命令"最小需要携带什么(路径、是否新 tab、行号定位?)。给出 2–3 种协议形态的利弊,**不选定**。

- [ ] **Step 3: O6 — Terminal 的现状**

Run: `ls crates/dozer-app/src/term && grep -rn "PanelKind" crates/dozer-app/src/term/terminal.rs | head -5 && python3 scripts/audit/edges.py | awk -F'\t' '$2=="layer:term"{c[$3]+=$5}END{for(k in c)print c[k],k}' | sort -rn`
Expected: `term/` 的文件构成与它依赖的模块;回答:Terminal 与 `PanelKind` 的关系(`term/terminal.rs` 为何引用它)、它是否像面板一样有自己的 state/message、依赖 host 的哪些部分。**不判断 Digger 是否需要**(规格 O6:需用户确认产品需求),只回答"如果共享,切口在哪"。

- [ ] **Step 4: O2 — Project context 的最小范围**

Run: `python3 scripts/audit/edges.py | awk -F'\t' '$3=="project"||$3=="open_projects"||$3=="project_meta"{print $2"\t"$3"\t"$4"\t"$5}' | sort | uniq -c | sort -rn | head -30` 以及 `grep -n "ProjectId" crates/dozer-app/src/app/state.rs | head`。
Expected: 回答:面板实际需要的"当前项目"信息是什么(`ProjectId`、项目路径、项目名?),哪些是面板私有的 per-project 状态(各面板在 `Workspace` 里按项目持有的部分);给出"host 只提供 id + 路径"的最小形态是否够用的判断依据。**不选定**。

- [ ] **Step 5: 写文档**

创建 `06-open-items-evidence.md`,按 O1、O2、O3、O6 各一节;每节固定三段:**事实**(命令 + 输出摘要 + 文件:行)、**切口分析**、**倾向与理由(明确标"倾向,非裁决")**。末尾加一节"仍需用户产品确认的问题"(至少含 O6 的 Digger 需求与 O11 的 Todo 语义)。

- [ ] **Step 6: Commit**

```bash
git status --short
git add docs/dozer-v2/bytehost-H0/06-open-items-evidence.md
git diff --cached --stat
git commit -m "docs(bytehost): H0 evidence for open items O1/O2/O3/O6

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 8: 面板边界棘轮门禁(报告模式)

**Files:**
- Create: `scripts/audit/test_check_panel_boundary.py`
- Create: `scripts/audit/check_panel_boundary.py`
- Create: `scripts/audit/panel-boundary.baseline.json`

**Interfaces:**
- Consumes: Task 1 的 `edges.edges_of`、`edges.SRC`
- Produces: `python3 scripts/audit/check_panel_boundary.py`(检查,有回退退出 1)与 `--update`(重写基线);导出 `scan(files) -> dict`、`compare(baseline, current) -> list`。规则:`R-APP`(`extensions/**` 引用 `crate::app::App`)、`R-WS`(`extensions/**` 引用 `crate::workspace::Workspace`)。这是规格 §6 第 4 条的**第一版**:棘轮,不是零容忍。

- [ ] **Step 1: 写失败的测试**

创建 `scripts/audit/test_check_panel_boundary.py`:

```python
import importlib.util, os, unittest
spec = importlib.util.spec_from_file_location("g", os.path.join(os.path.dirname(__file__), "check_panel_boundary.py"))
g = importlib.util.module_from_spec(spec); spec.loader.exec_module(g)

class Scan(unittest.TestCase):
    def test_only_extensions_files_count(self):
        files = {"extensions/todo/view.rs": "use crate::app::{App, HoverId};\nuse crate::workspace::Workspace;",
                 "app/update.rs": "use crate::app::App;",
                 "extensions/ssh.rs": "use crate::theme;"}
        self.assertEqual(g.scan(files), {"extensions/todo/view.rs": {"R-APP": 1, "R-WS": 1}})
class Compare(unittest.TestCase):
    def test_decrease_and_equal_pass(self):
        base = {"extensions/a.rs": {"R-APP": 3}}
        self.assertEqual(g.compare(base, {"extensions/a.rs": {"R-APP": 3}}), [])
        self.assertEqual(g.compare(base, {"extensions/a.rs": {"R-APP": 1}}), [])
        self.assertEqual(g.compare(base, {}), [])
    def test_increase_and_new_file_fail(self):
        base = {"extensions/a.rs": {"R-APP": 1}}
        cur = {"extensions/a.rs": {"R-APP": 2}, "extensions/b.rs": {"R-WS": 1}}
        self.assertEqual(g.compare(base, cur), [("extensions/a.rs", "R-APP", 1, 2), ("extensions/b.rs", "R-WS", 0, 1)])
if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: 确认测试失败**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0 && python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -4`
Expected: FAIL(`check_panel_boundary.py` 不存在)。

- [ ] **Step 3: 写门禁**

创建 `scripts/audit/check_panel_boundary.py`:

```python
#!/usr/bin/env python3
"""面板边界棘轮门禁(报告模式):面板代码(extensions/**)里 `App`/`Workspace` 的引用只许减少、不许增加。

规则 R-APP:  extensions 下文件引用 `crate::app::App`
规则 R-WS:   extensions 下文件引用 `crate::workspace::Workspace`

用法:
  check_panel_boundary.py            对照基线检查,有文件的引用数上升(或新文件出现违规)则退出 1
  check_panel_boundary.py --update   用当前扫描结果重写基线(只在引用数下降或经评审的迁移后使用)
基线:scripts/audit/panel-boundary.baseline.json,格式 {"<文件>": {"R-APP": n, "R-WS": n}}。
测试代码里的引用同样计入:H0 阶段不区分生产与测试,迁移完成后再评估是否放宽测试。
"""
import importlib.util, json, os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("edges", os.path.join(HERE, "edges.py"))
edges = importlib.util.module_from_spec(_spec); _spec.loader.exec_module(edges)

BASELINE = os.path.join(HERE, "panel-boundary.baseline.json")
RULES = {"R-APP": ("app", "App"), "R-WS": ("workspace", "Workspace")}


def scan(files):
    """files: {相对路径: 源码文本} -> {相对路径: {规则: 次数}},只含 extensions/ 下且有违规的文件。"""
    out = {}
    for rel, text in files.items():
        if not rel.startswith("extensions/"):
            continue
        found = edges.edges_of(text)
        hit = {rule: found[key] for rule, key in RULES.items() if found.get(key)}
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
        print("面板边界回退(面板代码里 App/Workspace 引用增加):", file=sys.stderr)
        for rel, rule, old, n in problems:
            print(f"  {rel}: {rule} {old} -> {n}", file=sys.stderr)
        return 1
    total = sum(sum(h.values()) for h in current.values())
    print(f"panel boundary check: ok ({total} refs in {len(current)} files, baseline ratchet)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
```

- [ ] **Step 4: 测试通过**

Run: `python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -3`
Expected: `Ran 3 tests` … `OK`。

- [ ] **Step 5: 生成基线并验证门禁行为**

Run: `python3 scripts/audit/check_panel_boundary.py --update && python3 scripts/audit/check_panel_boundary.py`
Expected: 先输出 `baseline updated: 11 refs in 9 files`(草稿值;与 Task 4 的 `app::App`=8 + `workspace::Workspace`=3 对得上,若不是 11/9,先确认差异来源再继续),再输出 `panel boundary check: ok (11 refs in 9 files, baseline ratchet)`。

**变异检验(证明门禁真的会拦):**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0
git add scripts/audit
sed -i '' '1i\
use crate::app::App;
' crates/dozer-app/src/extensions/toast.rs
python3 scripts/audit/check_panel_boundary.py; echo "exit=$?"
git checkout -- crates/dozer-app/src/extensions/toast.rs
python3 scripts/audit/check_panel_boundary.py; echo "exit=$?"
git status --short crates
```

Expected: 第一次输出 `面板边界回退…  extensions/toast.rs: R-APP 0 -> 1` 且 `exit=1`;还原后 `ok` 且 `exit=0`;最后一条 `git status --short crates` **无输出**(产品代码已还原,H0 不留任何 `crates/` 改动)。

- [ ] **Step 6: Commit**

```bash
git status --short
git add scripts/audit/test_check_panel_boundary.py scripts/audit/check_panel_boundary.py scripts/audit/panel-boundary.baseline.json
git diff --cached --stat
git commit -m "chore(audit): add ratcheting panel-boundary gate (App/Workspace refs in extensions)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: `git diff --cached --stat` 里没有 `crates/`。

---

### Task 9: 汇总、回填规格与要求文档

**Files:**
- Create: `docs/dozer-v2/bytehost-H0/00-summary.md`
- Modify: `docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md`(§4 的数字、§8 的证据指针、§10 的下一步)
- Modify: `docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`(§4.3 数字订正、§6 的 Q7/Q8/Q13 状态、§7 追加 H0 完成记录)

**Interfaces:**
- Consumes: Task 3–8 的全部产出
- Produces: H1 的**候选范围**(不是 H1 计划);规格与要求文档里与实测不符的数字被订正。

- [ ] **Step 1: 重跑全部报表,与各文档里的数字比对**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0
git fetch -q 2>/dev/null; git log --oneline main -1; git log --oneline -1
for c in symbols host-to-ext modules platform panelkind; do python3 scripts/audit/report.py $c > /tmp/h0-$c.new; diff <(tail -n +3 docs/dozer-v2/bytehost-H0/data/$c.md) /tmp/h0-$c.new > /dev/null && echo "$c: unchanged" || echo "$c: CHANGED"; done
python3 scripts/audit/check_panel_boundary.py
```
Expected: 若本分支基线与此刻 `main` 之间 `crates/dozer-app/src` 有改动,会出现 `CHANGED`——**在 worktree 里 `git merge main`**(不要 rebase 已提交的审计提交),重新生成受影响的 `data/*.md` 与相关审计文档里的数字,提交为 `docs: refresh H0 numbers`。全部 `unchanged` 则无需处理。

- [ ] **Step 2: 写 `00-summary.md`**

章节固定为:
1. **头部**:基线提交、范围声明(只覆盖 `dozer-app`,`dozer-core`/`dozerd` 只做了 Task 6/7 里的局部统计)。
2. **数字订正表**:列 = 项 / 要求文档与规格里的旧值 / H0 实测值 / 来源文档。至少含:`PanelKind` 变体数与引用数与文件数、面板→`App`/`Workspace` 引用、host→面板引用最重的前三、`platform/` overlay 数、根下共享模块数。
3. **对规格三条硬边界的影响**:E1/E2/E3 在实测数据下是否可行;E3 登记清单初版有多少条(按类别)。
4. **未决项进展**:O1–O12 逐项一行:H0 是否提供了证据、证据在哪份文档、是否因此可以收窄(**只写"可以收窄成什么问题",不替用户裁决**)。
5. **H1 候选范围(不是计划)**:按"风险最低、收益最高"排序列 3–5 个候选切片(例:"类型传递的 `PanelKind` 机械替换为注册 id"、"把 `App`/`Workspace` 的 11 处面板引用消除并把基线降到 0"、"把 `platform/` 里被判为通用的 overlay 机制归拢"),每个写:涉及文件数(来自 H0 数据)、前置未决项、风险。**明确写"H1 plan 需用户在评审本文后决定做哪个再写"。**
6. **门禁说明**:怎么跑、基线含义、何时可 `--update`、尚未接入 CI(仓库当前没有 CI 配置则写明"手动运行"),以及 E3 门禁(清单外无新增特判)留待 H1 设计的原因。

- [ ] **Step 3: 回填规格**

修改 `docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md`:
- §4 表里的"约 887 处/11 变体"等数字改为 H0 实测值,并加脚注指向 `bytehost-H0/00-summary.md`;
- §8 的每个未决项"现状/倾向"列追加"证据:`bytehost-H0/<文档>`"指针;**不改未决状态**;
- §10 把"H0 plan"标为已完成并指向本计划与 `00-summary.md`,下一步改为"评审 H0 汇总 → 选定 H1 切片 → 写 H1 plan"。

- [ ] **Step 4: 回填要求文档**

修改 `docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`:
- §4.3 的 `PanelKind` 数字订正(带"2026-10-03 H0 实测"标注,保留旧值作对照);
- §6 的 Q7、Q8、Q13:状态从"待审计"改为"H0 已审计,结论见 `bytehost-H0/…`",**仅当对应审计文档确实给出了可执行结论才改**,否则保持"待审计"并写明缺什么;
- §7 追加一条"H0 已完成(日期):产出 `scripts/audit/`、`docs/dozer-v2/bytehost-H0/`;面板边界棘轮门禁已建立,基线 11 处引用(以 Task 8 实测为准)"。

- [ ] **Step 5: 最终校验:H0 没碰产品代码**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h0
git diff --stat $(git merge-base main HEAD) HEAD -- crates Cargo.toml Cargo.lock | tail -1
git diff --stat $(git merge-base main HEAD) HEAD | tail -1
python3 scripts/audit/test_edges.py 2>&1 | tail -1 && python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -1 && python3 scripts/audit/check_panel_boundary.py
```
Expected: 第一条**无输出**(`crates/`、`Cargo.*` 零改动);第二条列出只含 `scripts/audit/`、`docs/` 的改动;两组测试 `OK`,门禁 `ok`。

- [ ] **Step 6: Commit**

```bash
git status --short
git add docs/dozer-v2/bytehost-H0/00-summary.md docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md
git diff --cached --stat
git commit -m "docs(bytehost): H0 summary; backfill the boundary spec and the v2 requirements with measured data

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

## Self-Review

**1. 规格覆盖(规格 §10 的 H0 范围 → 任务):**
- 各候选模块对 `App`/`Workspace`/其他面板的依赖统计 → Task 1–2(工具与报表)、Task 4(面板→host)、Task 5(host→面板)。
- 顶层共享模型与 host 反向调用审计,覆盖 Q7/Q8/Q13 → Task 6(Q7)、Task 3(Q8)、Task 5(Q13)。
- 面板边界门禁设计(§6 第 4 条) → Task 8(第一版棘轮,E3 门禁留 H1 并在 Task 9 说明)。
- `platform/` overlay 迁移清单 → Task 6。
- E3 登记清单初版 → Task 5、Task 6。
- 未决项的证据(O1/O2/O3/O6,及 O4/O7/O8 在 Task 6) → Task 7、Task 6。
- 规格 §4 数字订正 → Task 9。
- 缺口:规格 §6 第 5 条"一个面板 + host 可运行"的验收夹具(O12)——H0 只需"定义形式",计划里**没有单独任务**。归入 Task 9 的"H1 候选范围"一节:要求为最小 host + 一个面板 + fake 服务写一句形式定义。

**2. 占位符扫描:** 代码块全部为跑过的完整脚本;审计文档任务给的是**固定章节与必答问题**,不是"以后再补"——文档内容由读代码得出,无法在计划里预先写出。

**3. 类型/命名一致性:** `edges.edges_of`、`edges.unit_of`、`edges.flatten`、`edges.SRC` 在 Task 1 定义,Task 2、Task 8 的脚本按同名使用;`R-APP`/`R-WS` 规则名在 Task 8 的测试与实现里一致;`E3-registry.tsv` 的六列在 Task 5 定义,Task 6 回填与校验沿用。

**4. Review Focus:** 五条各有归属任务(1→Task 1/4、2→Task 4/8、3→Task 3、4→Task 5/6/7、5→Task 9)。
