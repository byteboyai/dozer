#!/usr/bin/env python3
"""S0 盘点:各文件对 dozer_client::Client 公开方法的使用(识别换行书写的 `client\n .method(`)。
用法: client_uses.py   → 输出 markdown 表(文件 | 方法)。"""
import importlib.util, os, re, sys
HERE = os.path.dirname(os.path.abspath(__file__))
_s = importlib.util.spec_from_file_location("edges", os.path.join(HERE, "edges.py"))
edges = importlib.util.module_from_spec(_s); _s.loader.exec_module(edges)
STRING_RE = re.compile(r'"(?:\\.|[^"\\])*"')
CLIENT_LIB = os.path.join(HERE, "..", "..", "crates", "dozer-client", "src", "lib.rs")

def client_methods():
    with open(CLIENT_LIB, encoding="utf-8") as f:
        return set(re.findall(r"pub (?:async )?fn ([a-z_0-9]+)", f.read()))

def methods_used(text, names):
    text = STRING_RE.sub('""', edges.strip_comments(text))
    return {m for m in re.findall(r"\.\s*([a-z_0-9]+)\s*\(", text) if m in names}

def main():
    names = client_methods()
    print("| 文件 | 方法 |\n|---|---|")
    for dp, _, fns in sorted(os.walk(edges.SRC)):
        for fn in sorted(fns):
            if not fn.endswith(".rs"):
                continue
            p = os.path.join(dp, fn)
            with open(p, encoding="utf-8") as f:
                used = methods_used(f.read(), names) - {"clone"}
            if used:
                print(f"| `{os.path.relpath(p, edges.SRC)}` | {', '.join(sorted(used))} |")

if __name__ == "__main__":
    main()
