#!/usr/bin/env python3
"""i18n 迁移硬编码门禁(Task 12,`scripts/i18n/check_migration.py`)。

在 `check_resources.py`(资源完整性)之外,做三件"存量迁移是否真的收口"的机械核对:

  1) 清单闭合:重跑 `build_inventory.build()`,要求 `unresolved == 0` 且
     `unassigned_unresolved == 0`。任何**未分类**的中文 UI 候选即失败——
     生产源里新写一条中文界面字面量会立刻在这里被拦下。

  2) key 有资源:从生产源里抽出处处 keyed 调用(`t("k"...)`、
     `panel_text(OWNER, "k"...)`、`host_text("k"...)`、`keyed*("k"...)`、
     以及前端 `t('k'...)`),把 `(owner, key)` 逐一对照 `locales/**` 的语言包:
     key 必须存在(否则是"缺译 key"/拼错),且 en / zh-CN 都要有。

  3) fallback 是英文:keyed 调用的 fallback 实参**不得含中日韩字**——中文是
     第二份词表的红线(spec §1.1:正常中文只从中文包解析,英文 fallback 是安全文案)。

外加一条防回归:构建产物 `assets/**` 不得在运行期读取仓库 `locales/`(静态打包
允许把 JSON 内联为字面量,但不允许出现指向 `locales/` 的路径访问)。

退出码非 0 表示失败。豁免只允许"具体文件/符号/文本/理由"(见
`build_inventory.py` 的 `NON_UI_STRINGS`/`EXCLUDED_FILES`,逐条带理由),
不允许整面板/整错误模块豁免。

用法:
  python3 scripts/i18n/check_migration.py            # 全量检查
  python3 scripts/i18n/check_migration.py --assets   # 额外查构建产物运行期读盘
"""
from __future__ import annotations

import importlib.util
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))


def _load(name: str):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, f"{name}.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


scan = _load("scan_sources")
build_inventory = _load("build_inventory")

CJK_RE = scan.CJK_RE


# --------------------------------------------------------------------------
# 语言包装载:owner → {locale: {key: value}}
# --------------------------------------------------------------------------
def load_bundles() -> dict:
    """扫描 `crates/**/locales*/**/{en,zh-CN}.json`,按**目录**聚合成一个 owner 包。

    owner id 不写死在 JSON 里(资源是纯键值表),这里用目录名/路径推断 owner,
    与 `i18n.rs` 的 `PANEL_OWNERS`/`dozer-panel-usage` 约定一致:
      - `locales/host/` → `dozer.host`
      - `locales/panels/<id>/` → `dozer.<id>`(codehealth 例外映射到 code_health)
      - 独立面板 `crates/dozer-panel-usage/locales/` → `dozer.usage`、`crates/dozer-panel-todo/locales/` → `dozer.todo`
    """
    dir_owner = {}
    for base, dirs, files in os.walk(os.path.join(ROOT, "crates")):
        dirs[:] = [d for d in dirs if d != "node_modules"]
        if "en.json" not in files and "zh-CN.json" not in files:
            continue
        rel = os.path.relpath(base, ROOT).replace(os.sep, "/")
        owner = _owner_from_dir(rel)
        dir_owner[base] = owner

    bundles: dict = {}
    for base, owner in dir_owner.items():
        for loc in ("en", "zh-CN"):
            path = os.path.join(base, f"{loc}.json")
            if not os.path.exists(path):
                continue
            with open(path, encoding="utf-8") as f:
                data = json.load(f)
            bundles.setdefault(owner, {})[loc] = data
    return bundles


def is_ts_test(rel: str) -> bool:
    base = os.path.basename(rel)
    return (
        ".test." in base
        or base in {"fixtures.ts", "fixtures.tsx", "render-smoke.tsx", "render-smoke.ts"}
    )


def _owner_from_dir(rel: str) -> str:
    if rel.endswith("locales/host"):
        return "dozer.host"
    m = re.search(r"locales/panels/([A-Za-z_]+)$", rel)
    if m:
        pid = m.group(1)
        # 目录名去下划线的历史命名:codehealth 资源 owner 是 dozer.code_health。
        return {"codehealth": "dozer.code_health"}.get(pid, f"dozer.{pid}")
    if rel.endswith("/locales"):
        # 独立面板:locales/ 直接挂在 crate 根(如 dozer-panel-usage)。
        crate = rel.rsplit("/locales", 1)[0].rsplit("/", 1)[-1]
        return f"dozer.{crate.replace('dozer-panel-', '').replace('-', '_')}"
    return "dozer.host"


# --------------------------------------------------------------------------
# keyed 调用抽取
# --------------------------------------------------------------------------
# `<FILE>_OWNER` / `HOST_OWNER` 常量 → owner 字面量(全仓可跨文件引用)。
OWNER_CONST_RE = re.compile(
    r"\b(?:pub(?:\(crate\))?\s+)?const\s+([A-Z][A-Z0-9_]*_OWNER|[A-Z][A-Z0-9_]*_OWNER)\s*:\s*&str\s*=\s*\"([^\"]+)\""
)
# `const OWNER: &str = super::PREVIEW_OWNER;`(别名到另一常量)
OWNER_ALIAS_RE = re.compile(
    r"\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=\s*([A-Z][A-Z0-9_]*)\s*;"
)


def _default_srcroots():
    return [
        os.path.join(ROOT, "crates", "dozer-app", "src"),
        os.path.join(ROOT, "crates", "dozer-panel-usage", "src"),
        os.path.join(ROOT, "crates", "dozer-panel-todo", "src"),
    ]


def collect_owner_consts(srcroots=None) -> dict:
    """收集 `NAME_OWNER -> "dozer.x"` 映射,跨全部生产源(含 crate 前缀)。"""
    out = {}
    srcroots = srcroots if srcroots is not None else _default_srcroots()
    files = []
    for r in srcroots:
        for dp, dns, fns in os.walk(r):
            dns[:] = [d for d in dns if d not in {"target", "node_modules"}]
            for fn in fns:
                if fn.endswith(".rs"):
                    files.append(os.path.join(dp, fn))
    alias = {}
    for fp in files:
        text = open(fp, encoding="utf-8", errors="replace").read()
        for name, val in OWNER_CONST_RE.findall(text):
            out[name] = val
        for name, target in OWNER_ALIAS_RE.findall(text):
            alias.setdefault(name, target)
    # 解析别名到字面量(如 OWNER = super::PREVIEW_OWNER)。
    for name, target in alias.items():
        if target in out:
            out.setdefault(name, out[target])
    return out


# keyed 调用的形态:函数名 + 第一个字符串实参(key)。owner 由文件内 `t`/`t_args`
# 的绑定或显式 owner 实参决定;这里对若干确定形态逐一匹配。
def extract_key_refs(owner_consts: dict, srcroots=None):
    """返回 {(owner, key): [位置描述]}。仅抽取能确定 owner 的调用。"""
    refs: dict = {}
    srcroots = srcroots if srcroots is not None else _default_srcroots()
    rs_files = []
    for r in srcroots:
        for dp, dns, fns in os.walk(r):
            dns[:] = [d for d in dns if d not in {"target", "node_modules"}]
            for fn in fns:
                if fn.endswith(".rs"):
                    rs_files.append(os.path.join(dp, fn))

    def add(owner, key, where):
        refs.setdefault((owner, key), []).append(where)

    # 显式 owner 实参:`panel_text(OWNER_CONST, "key"...` / `panel_keyed(...)`。
    explicit = re.compile(
        r"\bpanel_(?:text|text_args|keyed|keyed_args)\(\s*([A-Z][A-Z0-9_]*)\s*,\s*\"([^\"]+)\""
    )
    # host 便利:`host_text("key"...` / `host_text_args("key"...` / `host_keyed(...)`。
    host_call = re.compile(r"\bhost_(?:text|text_args|keyed|keyed_args)\(\s*\"([^\"]+)\"")
    # 文件内本地 `t("key"...` / `t_args("key"...` / `keyed("key"...` / `keyed_args(`。
    local_call = re.compile(r"\b(?:t|t_args|keyed|keyed_args)\(\s*\"([^\"]+)\"\s*,")
    local_t_def = re.compile(
        r"\bfn\s+(?:t|t_args|keyed|keyed_args)\s*\(.*?\{(.{0,600}?)\n\}"
    )

    for fp in rs_files:
        rel = os.path.relpath(fp, ROOT).replace(os.sep, "/")
        text = open(fp, encoding="utf-8", errors="replace").read()
        # 1) 显式 owner(跳过测试上下文:测试会用一次性 key 探针)
        for name, key in explicit.findall(text):
            owner = owner_consts.get(name)
            if owner and not _in_test(text, rel, key):
                add(owner, key, f"{rel}: panel_text({name})")
        # 2) host 便利
        for key in host_call.findall(text):
            if not _in_test(text, rel, key):
                add("dozer.host", key, f"{rel}: host_text")
        # 3) 本地 t():owner 取该文件 `t` 定义体里引用的 owner 常量。
        body_owner = None
        for body in local_t_def.findall(text):
            for name in re.findall(r"[A-Z][A-Z0-9_]*_OWNER", body):
                if name in owner_consts:
                    body_owner = owner_consts[name]
                    break
            if body_owner:
                break
        # 有的 `t` 直接内联字面量 owner:fn t(...) { panel_text("dozer.x", ...) }
        if body_owner is None:
            m = re.search(r"fn\s+(?:t|t_args)\s*\(.*?\b(?:panel_text|panel_keyed)[^(]*\(\s*\"([^\"]+)\"", text, re.S)
            if m:
                body_owner = m.group(1)
        if body_owner:
            for key in local_call.findall(text):
                if not _in_test(text, rel, key):
                    add(body_owner, key, f"{rel}: t()")
    return refs


def _in_test(text: str, rel: str, key: str) -> bool:
    """该 key 字面量是否只出现在测试上下文里(`#[cfg(test)]` / `mod tests`)。"""
    if re.search(r"_test\.(?:rs)$", rel) or ".test." in rel:
        return True
    for m in re.finditer(re.escape(f'"{key}"'), text):
        if not scan.is_test_context(rel, text, m.start()):
            return False
    return True


# --------------------------------------------------------------------------
# 前端 keyed 调用抽取:静态打包的 TS 宿主用 t('k', fallback, args)。
# --------------------------------------------------------------------------
def extract_ts_key_refs(owner_consts: dict):
    """前端各 web 包 `src/i18n.ts` 静态绑定一个 owner;抽 `t('key'...)` 并核对。

    owner 从 `src/i18n.ts` 里的 owner 字面量(如 `const OWNER = "dozer.preview"`)
    推断;同一包目录下的 `t(...)` 都归它。
    """
    refs: dict = {}
    webroot = os.path.join(ROOT, "crates")
    # 包目录 → owner
    pkg_owner = {}
    for dp, dns, fns in os.walk(webroot):
        dns[:] = [d for d in dns if d not in {"node_modules", "dist", "target"}]
        if os.path.basename(dp) == "i18n.ts" or "i18n.ts" in fns:
            i18n_path = os.path.join(dp, "i18n.ts")
            if not os.path.exists(i18n_path):
                continue
            text = open(i18n_path, encoding="utf-8", errors="replace").read()
            m = re.search(r"(?:OWNER|owner)\s*[:=]\s*[\"']([^\"']+)[\"']", text)
            if m:
                # 包目录 = i18n.ts 的 src/ 的上一级(src/i18n.ts)
                pkg = os.path.dirname(dp.rstrip("/"))
                pkg_owner[pkg] = m.group(1)
    t_call = re.compile(r"\bt\(\s*[\"']([^\"']+)[\"']")
    for pkg, owner in pkg_owner.items():
        for dp, dns, fns in os.walk(pkg):
            dns[:] = [d for d in dns if d not in {"node_modules", "dist", "target"}]
            for fn in fns:
                if not fn.endswith((".ts", ".tsx")):
                    continue
                fp = os.path.join(dp, fn)
                rel = os.path.relpath(fp, ROOT).replace(os.sep, "/")
                if is_ts_test(rel):
                    continue
                text = open(fp, encoding="utf-8", errors="replace").read()
                for key in t_call.findall(text):
                    refs.setdefault((owner, key), []).append(f"{rel}: t()")
    return refs


# --------------------------------------------------------------------------
# fallback 必须不含中文
# --------------------------------------------------------------------------
FALLBACK_CALL_RE = re.compile(
    r"\b(?:panel_text|panel_text_args|host_text|host_text_args|host_keyed|host_keyed_args|"
    r"panel_keyed|panel_keyed_args|t|t_args|keyed|keyed_args)\(\s*\"[^\"]*\"\s*,\s*\"([^\"]*)\"",
    re.S,
)


def check_english_fallbacks(srcroots=None):
    """keyed 调用的第二实参(fallback)不得含中日韩字。"""
    problems = []
    for r in srcroots if srcroots is not None else _default_srcroots():
        for dp, dns, fns in os.walk(r):
            dns[:] = [d for d in dns if d not in {"target", "node_modules"}]
            for fn in fns:
                if not fn.endswith(".rs"):
                    continue
                fp = os.path.join(dp, fn)
                rel = os.path.relpath(fp, ROOT).replace(os.sep, "/")
                text = open(fp, encoding="utf-8", errors="replace").read()
                if re.search(r"_test\.(?:rs)$", rel):
                    continue
                code = scan.strip_rust_comments(text)
                for m in FALLBACK_CALL_RE.finditer(code):
                    fb = m.group(1)
                    if CJK_RE.search(fb) and not scan.is_test_context(rel, code, m.start()):
                        problems.append(f"{rel}: 中文 fallback: {fb!r}")
    return problems


# --------------------------------------------------------------------------
# key 引用 ↔ 资源 比对(纯净函数,便于单测注入)
# --------------------------------------------------------------------------
def find_missing_keys(refs: dict, bundles: dict, locales=("en", "zh-CN")):
    """返回问题列表:`refs` 里每个 (owner, key) 必须在 `bundles` 的每个 locale 里。"""
    problems = []
    for (owner, key), where in sorted(refs.items()):
        langs = bundles.get(owner)
        w = where[0] if where else "?"
        if not langs:
            problems.append(f"key 引用了未知 owner `{owner}`: {key} ({w})")
            continue
        for loc in locales:
            if key not in langs.get(loc, {}):
                problems.append(f"缺译 key: owner={owner} locale={loc} key={key} ({w})")
    return problems


def find_unclassified(recs):
    """返回仍未分类(unresolved)的候选问题描述。"""
    return [
        f"未分类 UI 候选: {r['path']}  {r['text']!r}"
        for r in recs
        if r["status"] == "unresolved"
    ]


# --------------------------------------------------------------------------
# 构建产物运行期读盘
# --------------------------------------------------------------------------
RUNTIME_READ_RE = re.compile(r"""(?:readFile|readFileSync|fetch\s*\(\s*["'`]|__dirname)[^;\n]{0,80}locales/""")


def check_assets_no_runtime_locale_read(asset_dirs):
    problems = []
    for d in asset_dirs:
        for dp, dns, fns in os.walk(d):
            for fn in fns:
                if not fn.endswith((".js", ".mjs", ".html")):
                    continue
                fp = os.path.join(dp, fn)
                rel = os.path.relpath(fp, ROOT).replace(os.sep, "/")
                text = open(fp, encoding="utf-8", errors="replace").read()
                for m in RUNTIME_READ_RE.finditer(text):
                    problems.append(f"{rel}: 运行期读 locales: {m.group(0)[:80]!r}")
    return problems


# --------------------------------------------------------------------------
def main(argv):
    with_assets = "--assets" in argv

    problems = []

    # 1) 清单闭合。
    recs = build_inventory.build()
    unresolved = [r for r in recs if r["status"] == "unresolved"]
    print(f"inventory: total={len(recs)} unresolved={len(unresolved)}")
    problems.extend(find_unclassified(recs)[:50])

    # 2) key 有资源。
    owner_consts = collect_owner_consts()
    bundles = load_bundles()
    refs = extract_key_refs(owner_consts)
    refs.update(extract_ts_key_refs(owner_consts))
    n_keys = len(refs)
    problems.extend(find_missing_keys(refs, bundles))
    print(f"key refs checked: {n_keys} (owners: {len(bundles)})")

    # 3) fallback 是英文。
    for p in check_english_fallbacks():
        problems.append(p)

    # 4) 构建产物运行期读盘。
    if with_assets:
        asset_dirs = [
            os.path.join(ROOT, "crates", "dozer-app", "assets"),
            os.path.join(ROOT, "crates", "dozer-panel-usage", "assets"),
            os.path.join(ROOT, "crates", "dozer-panel-todo", "assets"),
        ]
        for p in check_assets_no_runtime_locale_read(asset_dirs):
            problems.append(p)

    if problems:
        print(f"check_migration: FAIL ({len(problems)} 个问题)")
        for p in problems:
            print("  -", p)
        return 1
    print("check_migration: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
