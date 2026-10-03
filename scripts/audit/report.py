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
