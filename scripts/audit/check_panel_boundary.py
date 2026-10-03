#!/usr/bin/env python3
"""面板边界棘轮门禁(报告模式):面板代码(extensions/**)里 `App`/`Workspace` 的引用只许减少、不许增加。

规则 R-APP:  extensions 下文件对 `crate::app::App` 的**使用次数**(import 行 + 每个 `&App` 参数/限定路径)
规则 R-WS:   extensions 下文件对 `crate::workspace::Workspace` 的使用次数

用法:
  check_panel_boundary.py            对照基线检查,有文件的引用数上升(或新文件出现违规)则退出 1
  check_panel_boundary.py --update   用当前扫描结果重写基线(只在引用数下降或经评审的迁移后使用)
基线:scripts/audit/panel-boundary.baseline.json,格式 {"<文件>": {"R-APP": n, "R-WS": n}}。
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


def scan(files):
    """files: {相对路径: 源码文本} -> {相对路径: {规则: 次数}},只含 extensions/ 下且有违规的文件。"""
    out = {}
    for rel, text in files.items():
        if not rel.startswith("extensions/"):
            continue
        hit = {}
        for rule, (mod, name) in RULES.items():
            n = count_uses(text, mod, name)
            if n:
                hit[rule] = n
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
