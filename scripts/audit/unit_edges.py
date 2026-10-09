#!/usr/bin/env python3
"""单元依赖边清单(O12 探针用):列出一个单元(如 ext:usage)对 crate 内其他模块与外部 crate 的依赖,
按 A/B/C/S/T/U 分类。有 `?`(未分类)时退出码 1,逼着新增依赖被显式归类。

用法: unit_edges.py ext:usage
"""
import collections, importlib.util, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("edges", os.path.join(HERE, "edges.py"))
edges = importlib.util.module_from_spec(_spec); _spec.loader.exec_module(edges)

# A 已有契约 / B 通用 UI 件(应进 SDK 或 byteui)/ C 领域类型(O7)/ S 数据服务(无接缝)/ T 仅测试 / U 框架库
INTERNAL = {
    ("panel_host", "PanelHost"): "A",
    ("panel_host", "ActivationCtx"): "A",
    ("panel_host", "PanelIo"): "A",
    ("panel_host", "testing"): "T",
    ("chrome", "homespace"): "B",
    ("app", "HoverId"): "B",
    ("app", "PanelKind"): "B",
    ("workspace", "agent_icon"): "B",
    ("workspace", "agent_dot_color"): "B",
    ("conversation", "ConversationMeta"): "C",
}
EXTERNAL = {"dozer_core": "C", "dozer_model": "C", "dozer_client": "S", "bytegit": "S", "byteui": "B"}
EXTERNAL_RE = re.compile(r"\b(dozer_core|dozer_model|dozer_client|bytegit|byteui)::([A-Za-z_][A-Za-z0-9_]*)")


def classify(mod, sym):
    if (mod, sym) in INTERNAL:
        return INTERNAL[(mod, sym)]
    if mod in EXTERNAL:
        return EXTERNAL[mod]
    return "?"


def scan(unit):
    """[(class, mod, sym, files, count)],按 class、mod、sym 排序。"""
    acc = collections.defaultdict(lambda: [0, set()])
    for dp, _, fns in sorted(os.walk(edges.SRC)):
        for fn in sorted(fns):
            if not fn.endswith(".rs"):
                continue
            path = os.path.join(dp, fn)
            rel = os.path.relpath(path, edges.SRC).replace(os.sep, "/")
            if edges.unit_of(rel) != unit:
                continue
            with open(path, encoding="utf-8") as f:
                text = f.read()
            for (mod, sym), n in edges.edges_of(text).items():
                acc[(mod, sym)][0] += n
                acc[(mod, sym)][1].add(os.path.basename(rel))
            for m in EXTERNAL_RE.finditer(edges.strip_comments(text)):
                key = (m.group(1), m.group(2))
                acc[key][0] += 1
                acc[key][1].add(os.path.basename(rel))
    rows = [(classify(m, s), m, s, ",".join(sorted(fs)), n) for (m, s), (n, fs) in acc.items()]
    return sorted(rows)


def main(argv):
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    rows = scan(argv[1])
    print("| 类 | 模块 | 符号 | 出现文件 | 次数 |")
    print("|---|---|---|---|---|")
    for cls, mod, sym, files, n in rows:
        print(f"| {cls} | `{mod}` | `{sym}` | {files} | {n} |")
    return 1 if any(r[0] == "?" for r in rows) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
