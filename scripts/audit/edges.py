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
