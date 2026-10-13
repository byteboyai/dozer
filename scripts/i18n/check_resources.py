#!/usr/bin/env python3
"""资源完整性检查(Task 02,`scripts/i18n/check_resources.py`)。

校验 `crates/**/locales/**`(以及后续独立面板的 locales)下同一 owner 的
`en.json` / `zh-CN.json`:
  - 顶层是字符串键值对象;值均为非空字符串;
  - 无重复 key(用 `object_pairs_hook` 在覆盖前发现);
  - 英中 key 集合一致;
  - 占位符集合一致,且模板括号配对、占位符名合法(spec §6.2)。

退出码非 0 表示失败。规则须与 `dozer-i18n`(Rust)与共享 TS 解析器保持一致;
这里独立实现一份"检查器",三方以同一份 `vectors/interpolation.json` +
本规则为准。

用法:
  python3 scripts/i18n/check_resources.py            # 扫描仓库默认 locale 目录
  python3 scripts/i18n/check_resources.py PATH...    # 只查指定 owner 目录
"""
from __future__ import annotations

import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))

# 首批支持语言;资源目录里出现的、属于这套的视为需对齐的语言包。
SUPPORTED = ["en", "zh-CN"]
NAME_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


class DuplicateKey(Exception):
    pass


def _pairs_hook(pairs):
    seen = {}
    for k, v in pairs:
        if k in seen:
            raise DuplicateKey(k)
        seen[k] = v
    return seen


def load_bundle(path: str) -> dict:
    with open(path, "r", encoding="utf-8") as f:
        text = f.read()
    try:
        data = json.loads(text, object_pairs_hook=_pairs_hook)
    except DuplicateKey as e:
        raise ValueError(f"重复 key: {e}") from None
    except json.JSONDecodeError as e:
        raise ValueError(f"JSON 语法错误: {e}") from None
    if not isinstance(data, dict):
        raise ValueError("顶层必须是字符串键值对象")
    for k, v in data.items():
        if not isinstance(v, str):
            raise ValueError(f"值不是字符串: {k}")
        if v == "":
            raise ValueError(f"空翻译: {k}")
        validate_template(k, v)
    return data


def validate_template(key: str, template: str) -> None:
    i, n = 0, len(template)
    while i < n:
        c = template[i]
        if c == "{" and i + 1 < n and template[i + 1] == "{":
            i += 2
            continue
        if c == "}" and i + 1 < n and template[i + 1] == "}":
            i += 2
            continue
        if c == "{":
            end = template.find("}", i + 1)
            if end < 0:
                raise ValueError(f"模板非法({key}): 未闭合的 `{{`")
            name = template[i + 1:end]
            if not NAME_RE.match(name):
                raise ValueError(f"模板非法({key}): 非法占位符名 `{{{name}}}`")
            i = end + 1
            continue
        if c == "}":
            raise ValueError(f"模板非法({key}): 未配对的 `}}`")
        i += 1


def placeholders(template: str) -> set:
    out = set()
    i, n = 0, len(template)
    while i < n:
        c = template[i]
        if c == "{" and i + 1 < n and template[i + 1] == "{":
            i += 2
            continue
        if c == "{":
            end = template.find("}", i + 1)
            if end > 0:
                out.add(template[i + 1:end])
                i = end + 1
                continue
        i += 1
    return out


def owner_dirs() -> list:
    """返回直接含 `en.json` 或 `zh-CN.json` 的目录(即一个 owner 的语言包目录)。

    `locales/` 本身可能只是分组目录(如 `locales/host/`),不当作 owner;
    只有直接持有语言包的目录才算。
    """
    dirs = []
    for base, dirs_, files in os.walk(os.path.join(ROOT, "crates")):
        # 第三方正文(node_modules)不机械搬入资源,也不纳入检查(spec §1.2)。
        dirs_[:] = [d for d in dirs_ if d != "node_modules"]
        if any(f"{loc}.json" in files for loc in SUPPORTED):
            dirs.append(base)
    return sorted(dirs)


def check_owner(locale_dir: str) -> list:
    errors = []
    bundles = {}
    for loc in SUPPORTED:
        path = os.path.join(locale_dir, f"{loc}.json")
        if not os.path.exists(path):
            errors.append(f"缺少语言包: {os.path.relpath(path, ROOT)}")
            continue
        try:
            bundles[loc] = load_bundle(path)
        except ValueError as e:
            errors.append(f"{os.path.relpath(path, ROOT)}: {e}")

    if len(bundles) == len(SUPPORTED):
        ref = "en"
        ref_keys = set(bundles[ref].keys())
        for loc in SUPPORTED:
            if loc == ref:
                continue
            cand_keys = set(bundles[loc].keys())
            for k in sorted(ref_keys - cand_keys):
                errors.append(f"{loc}: 缺少 key `{k}`")
            for k in sorted(cand_keys - ref_keys):
                errors.append(f"{loc}: 多余 key `{k}`")
            for k in sorted(ref_keys & cand_keys):
                pa = placeholders(bundles[ref][k])
                pb = placeholders(bundles[loc][k])
                if pa != pb:
                    errors.append(
                        f"{loc}: key `{k}` 占位符不一致 {sorted(pa)} vs {sorted(pb)}"
                    )
    return errors


def main(argv: list) -> int:
    if len(argv) > 1:
        targets = [os.path.abspath(p) for p in argv[1:]]
    else:
        targets = owner_dirs()

    if not targets:
        print("check_resources: 未找到 locales 目录")
        return 1

    all_errors = []
    checked = 0
    for d in targets:
        rel = os.path.relpath(d, ROOT)
        errs = check_owner(d)
        checked += 1
        for e in errs:
            all_errors.append(f"{rel}: {e}")

    if all_errors:
        print(f"check_resources: FAIL ({len(all_errors)} 个问题)")
        for e in all_errors:
            print("  -", e)
        return 1
    print(f"check_resources: ok ({checked} 个 owner 目录)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
