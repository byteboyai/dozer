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
