#!/usr/bin/env python3
"""i18n 候选扫描工具(Task 01):从自有生产源里抽出**用户可见的中文候选**,产出可重跑的清单。

它只做"候选抽取 + 粗分类",不做 owner/key 最终裁决——那一步靠人工沿
view/presenter/payload/错误/Toast 调用链核对(见 docs/dozer-v2/i18n/inventory.json)。
本工具的职责是"不漏、可复跑、结论稳定":同样的输入永远产出同样的记录 ID,
让盘点总账(候选总数 = 已迁移 + 有理由排除 + 未解决)可机械核对。

覆盖:
  - Rust:普通字符串、raw string(`r".."`/`r#".."#`)、`format!` 模板。
  - TS/TSX/JS:单双引号字符串、反引号模板串(含 `${}` 保留原样)、JSX 文本节点、
    JSX 字符串属性(title/aria-label/placeholder/alt)。
  - HTML:文本节点、title/aria-label/placeholder/alt 属性。

分类(usage)是粗判,不代表最终裁决:
  - ui        :默认,疑似界面文案(交给人工核销)。
  - comment   :注释里的中文——**默认排除**,不计入候选。
  - log       :`tracing`/`log_*`/`plain_*!`/`eprintln!` 等里的中文——默认排除。
  - diag      :`assert!`/`expect`/`panic!`/`unreachable!`/`debug_assert!` 等开发期诊断——默认排除。
  - test      :`#[cfg(test)]`/`mod tests` 内、或 `*_test.rs`/`*.test.ts` 里的中文——默认排除。
  - protocol  :形如协议稳定值(`"zh-CN"` 这类语言标签除外由术语表管)——默认排除。

排除项仍写入 inventory(状态 `excluded` + reason),但默认不计入"未解决",供总账核对。

用法:
  scan_sources.py                 打印候选统计到 stdout
  scan_sources.py --json OUT.json 写出候选记录(稳定排序;供 inventory 生成/核对)
  scan_sources.py --list          逐条打印(owner 粗判 + 路径 + 文字)

设计:规则尽量"宁可多报"。CJK 只判断是否含 `\\u4e00-\\u9fff` + 常用扩展,不试图区分
日韩(本项目术语集中在简体中文,日韩字符会出现则人工归类)。英文 UI 硬编码不由本工具
发现(它扫不了"是否是 UI"),留给 Task 12 的入口规则 + 人工清单。
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))

# CJK 统一表意文字(基本区 + 扩展 A)+ 常用全角标点。只用于"是否中文候选"的粗判。
CJK_RE = re.compile(r"[\u4e00-\u9fff\u3400-\u4dbf]")

# ---- 扫描根(相对仓库根)。assets/ 是构建产物(见 spec §1.2),不列;由 build 从 web/ 派生。----
SRC_ROOTS = [
    "crates/dozer-app/src",
    "crates/dozer-app/web",               # TS/TSX/JS 源 + html 模板
    "crates/dozer-app/src/html_host.html",
    "crates/dozer-app/src/image_annotate_host.html",
    "crates/dozer-panel-usage/src",
    "crates/dozer-panel-usage/web",
    "crates/dozerd/src",
    "crates/dozer-core/src",
    "crates/dozer-model/src",
    "crates/dozer-services/src",
    "crates/dozer-client/src",
    "crates/dozer-hook/src",
    "crates/dozer-mcp/src",
]

EXCLUDE_DIR_MARKERS = ["/node_modules/", "/target/", "/dist/", "/.git/", "/__pycache__/"]
EXCLUDE_FILE_SUFFIXES = [
    "package-lock.json", "tsconfig.json", ".d.ts", ".svg", ".png", ".ico",
]
SOURCE_SUFFIXES = [".rs", ".ts", ".tsx", ".js", ".mjs", ".jsx", ".html"]

# ---- owner 粗判(路径 → owner_id)。最终 owner 由注册来源绑定,这里只给初值。----
OWNER_RULES = [
    ("crates/dozer-panel-usage/", "dozer.usage"),
    ("crates/dozer-app/src/extensions/todo/", "dozer.todo"),
    ("crates/dozer-app/src/extensions/codehealth/", "dozer.codehealth"),
    ("crates/dozer-app/src/extensions/files/", "dozer.files"),
    ("crates/dozer-app/src/extensions/project", "dozer.project"),
    ("crates/dozer-app/src/extensions/git_log.rs", "dozer.git_log"),
    ("crates/dozer-app/src/extensions/database/", "dozer.database"),
    ("crates/dozer-app/src/extensions/ssh", "dozer.ssh"),
    ("crates/dozer-app/src/extensions/browser.rs", "dozer.web"),
    ("crates/dozer-app/src/extensions/group_chat/", "dozer.group_chat"),
    ("crates/dozer-app/src/extensions/conversations.rs", "dozer.conversations"),
    ("crates/dozer-app/src/extensions/agent_context.rs", "dozer.agent"),
    ("crates/dozer-app/src/extensions/settings", "dozer.host"),
    ("crates/dozer-app/src/extensions/toast.rs", "dozer.host"),
    ("crates/dozer-app/src/extensions/footbar.rs", "dozer.host"),
    ("crates/dozer-app/web/editor/", "dozer.preview"),
    ("crates/dozer-app/web/json-editor/", "dozer.preview"),
    ("crates/dozer-app/web/tabular-host/", "dozer.preview"),
    ("crates/dozer-app/web/flyfish/", "dozer.preview"),
    ("crates/dozer-app/web/plantuml-viewer/", "dozer.preview"),
    ("crates/dozer-app/web/todo-content/", "dozer.todo"),
    ("crates/dozer-app/web/codehealth-content/", "dozer.codehealth"),
    ("crates/dozer-app/web/group-chat-content/", "dozer.group_chat"),
    ("crates/dozer-app/web/review-trace/", "dozer.agent"),
    ("crates/dozer-app/src/preview/", "dozer.preview"),
    ("crates/dozerd/", "dozer.host"),
    ("crates/dozer-app/", "dozer.host"),
]

# ---- 分类启发式 ----
LOG_MACRO_RE = re.compile(
    r"\b(?:tracing::)?(?:log_(?:trace|debug|info|warn|error)|plain_(?:trace|debug|info|warn|error)"
    r"|eprintln|println|dbg|error|warn|info|debug)\s*!\s*\("
)
DIAG_MACRO_RE = re.compile(r"\b(?:assert|assert_eq|assert_ne|debug_assert|debug_assert_eq|panic|unreachable|todo|unimplemented|expect)\s*!?\s*\(")


def relpath(path: str) -> str:
    return os.path.relpath(path, ROOT).replace(os.sep, "/")


def is_source(path: str) -> bool:
    rel = "/" + relpath(path) + "/"
    if any(m in rel for m in EXCLUDE_DIR_MARKERS):
        return False
    base = os.path.basename(path)
    if any(base.endswith(s) for s in EXCLUDE_FILE_SUFFIXES):
        return False
    return any(base.endswith(s) for s in SOURCE_SUFFIXES)


def owner_of(rel: str) -> str:
    for prefix, owner in OWNER_RULES:
        if rel.startswith(prefix):
            return owner
    return "dozer.host"


def strip_rust_comments(text: str) -> str:
    """把 Rust 注释替换为等长空白(保留行号/列偏移)。

    字符串字面量内的 `//`/`/*` 不是注释,必须先跳过字符串再判注释,否则含
    `dozer://x` 的字符串会被误当成行注释截断,导致后续引号配对错位、
    抽出跨行的伪字符串候选。raw string 亦跳过。
    """
    out = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        # raw string: r"..." / r#"..."#
        if c == "r" and i + 1 < n and text[i + 1] in '#"':
            j = i + 1
            hashes = 0
            while j < n and text[j] == "#":
                hashes += 1
                j += 1
            if j < n and text[j] == '"':
                close = '"' + "#" * hashes
                end = text.find(close, j + 1)
                end = n if end < 0 else end + len(close)
                out.append(text[i:end])
                i = end
                continue
        if c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                if text[j] == "\\":
                    j += 2
                else:
                    j += 1
            j = min(j + 1, n)
            out.append(text[i:j])
            i = j
            continue
        if c == "'":
            # Rust 字符字面量 'x' / '\n' / '\'' / '"'——必须整段跳过,否则其中的
            # `"`(如 trim_matches('"'))会被当成字符串开头,吞掉其后一大段代码,
            # 令该区间内的真实 UI 文案全部落入"被吞的字符串体"而漏扫、注释检测错位。
            if i + 1 < n and text[i + 1] == "\\":
                j = i + 2
                if j < n and text[j] == "u" and j + 1 < n and text[j + 1] == "{":
                    k = text.find("}", j)
                    j = n if k < 0 else k + 1
                else:
                    j += 1
            else:
                j = i + 2
            if j < n and text[j] == "'":
                out.append(text[i:j + 1])
                i = j + 1
                continue
            # 不是合法字符字面量(可能是生命周期 'a),按普通字符处理。
            out.append(c)
            i += 1
            continue
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif c == "/" and i + 1 < n and text[i + 1] == "*":
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            seg = text[i:j]
            out.append("".join("\n" if ch == "\n" else " " for ch in seg))
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)


def c_comment_ranges(text: str):
    """返回 ((start,end), kind) 列表,kind ∈ {line, block}。JS/TS/HTML 用。"""
    ranges = []
    i, n = 0, len(text)
    in_str = None
    while i < n:
        c = text[i]
        if in_str:
            if c == "\\":
                i += 2
                continue
            if c == in_str:
                in_str = None
            i += 1
            continue
        if c in "\"'`":
            in_str = c
            i += 1
            continue
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            j = text.find("\n", i)
            j = n if j < 0 else j
            ranges.append(((i, j), "line"))
            i = j
            continue
        if c == "/" and i + 1 < n and text[i + 1] == "*":
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            ranges.append(((i, j), "block"))
            i = j
            continue
        i += 1
    return ranges


def in_ranges(pos: int, ranges) -> bool:
    return any(s <= pos < e for (s, e), _ in ranges)


def is_test_context(rel: str, text: str, pos: int) -> bool:
    if re.search(r"_test\.(?:rs)$", rel) or ".test." in rel:
        return True
    # `#[cfg(test)]` 之后视为测试区(粗判:取最后一个 cfg(test) 位置)。
    m = None
    for m2 in re.finditer(r"#\[cfg\(test\)\]", text):
        m = m2
    return m is not None and pos > m.start()


# ---- 字符串抽取 ----

RUST_STRING_RE = re.compile(
    r'(?P<raw>r#*")|(?P<str>"(?:[^"\\]|\\.)*")'
)


def extract_rust(text: str, rel: str, records: list):
    # 先按字符串感知地blank注释;之后在 code 上找字符串字面量,不会撞到注释里的引号。
    code = strip_rust_comments(text)
    i, n = 0, len(code)
    while i < n:
        c = code[i]
        # raw string r"..." / r#"..."#
        if c == "r" and i + 1 < n and code[i + 1] in '#"':
            j = i + 1
            hashes = 0
            while j < n and code[j] == "#":
                hashes += 1
                j += 1
            if j < n and code[j] == '"':
                close = '"' + "#" * hashes
                end = code.find(close, j + 1)
                if end < 0:
                    i = j + 1
                    continue
                content = code[j + 1:end]
                _maybe_record(content, j + 1, rel, text, records, "raw")
                i = end + len(close)
                continue
        if c == "'":
            # Rust 字符字面量 'x' / '\n' / '\'' / '"'——跳过,否则其中的 " 会被
            # 当成字符串开头,导致后续引号全部错位(ssh.rs 的 trim_matches('"') 实证)。
            if i + 1 < n and code[i + 1] == "\\":
                j = i + 2
                if j < n and code[j] == "u" and j + 1 < n and code[j + 1] == "{":
                    j = code.find("}", j)
                    j = n if j < 0 else j + 1
                else:
                    j += 1
            else:
                j = i + 2
            if j < n and code[j] == "'":
                i = j + 1
                continue
        if c == '"':
            j = i + 1
            while j < n and code[j] != '"':
                if code[j] == "\\":
                    j += 2
                else:
                    j += 1
            raw_content = code[i + 1:j]
            content = raw_content.encode().decode("unicode_escape", "ignore") if "\\" in raw_content else raw_content
            _maybe_record(content, i + 1, rel, text, records, "string")
            i = j + 1
            continue
        i += 1


def extract_js(text: str, rel: str, records: list):
    ranges = c_comment_ranges(text)
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c in "\"'`":
            quote = c
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == quote:
                    break
                j += 1
            content = text[i + 1:j]
            if not in_ranges(i, ranges):
                _maybe_record(content, i + 1, rel, text, records, "template" if quote == "`" else "string")
            i = j + 1
            continue
        i += 1
    # JSX 文本节点:>  中文  <
    for m in re.finditer(r">([^<>{}]*[\u4e00-\u9fff][^<>{}]*)<", text):
        if not in_ranges(m.start(1), ranges):
            _maybe_record(m.group(1).strip(), m.start(1), rel, text, records, "jsx_text")


HTML_ATTR_RE = re.compile(r'\b(?:title|aria-label|placeholder|alt|label)\s*=\s*"([^"]*)"')


def extract_html(text: str, rel: str, records: list):
    for m in HTML_ATTR_RE.finditer(text):
        _maybe_record(m.group(1), m.start(1), rel, text, records, "html_attr")
    # 去掉 <script>/<style>/标签后剩的文本节点
    stripped = re.sub(r"<(script|style)\b.*?</\1>", " ", text, flags=re.S | re.I)
    stripped = re.sub(r"<[^>]*>", "\n", stripped)
    base = text.find(stripped[:0])  # 不精确定位,取整文偏移 0 作为近似
    for m in re.finditer(r"[^\s<>][^<>\n]*[\u4e00-\u9fff][^<>\n]*", stripped):
        _maybe_record(m.group(0).strip(), 0, rel, text, records, "html_text")


def _statement_start(text: str, pos: int, back: int = 4000) -> int:
    """返回 pos 之前最近的语句边界(`;`/`{`/`}`)之后的偏移。

    宏调用(尤其 `dozer_core::log_warn!` 这类多行字段列表)里,字符串可能是最后一个
    实参,离宏名远超固定窗口;用语句边界兜住整条调用,避免把日志/诊断误判为 UI。
    """
    start = max(0, pos - back)
    seg = text[start:pos]
    cut = max(seg.rfind(";"), seg.rfind("{"), seg.rfind("}"))
    return start + cut + 1


def classify(rel: str, text: str, pos: int, kind: str) -> tuple[str, str | None]:
    """返回 (usage, note)。pos 为字符串在文件中的近似偏移(用于日志/诊断/测试判定)。"""
    window = text[max(0, pos - 120):pos]
    # 日志/诊断宏的实参可能跨多行(字段列表在前),用整条语句兜底。
    stmt = text[_statement_start(text, pos):pos]
    if is_test_context(rel, text, pos):
        return "test", "测试/fixture 内(默认排除)"
    if LOG_MACRO_RE.search(window) or LOG_MACRO_RE.search(stmt):
        return "log", "日志宏参数(默认排除)"
    if DIAG_MACRO_RE.search(window) or DIAG_MACRO_RE.search(stmt):
        return "diag", "开发期诊断宏(默认排除)"
    return "ui", None


def _maybe_record(content: str, off: int, rel: str, text: str, records: list, kind: str):
    if not content or not CJK_RE.search(content):
        return
    # 注释泄漏防护:字符串扫描在极少数注释含引号时可能跨进注释区,抽出伪候选。
    # 只要候选的起始偏移落在注释区间内,就按注释丢弃(注释里的中文本就不翻译)。
    if kind != "comment" and _inside_rust_comment(text, off):
        return
    # 兜底:漏网的注释/代码泄漏候选。真实 UI 文案是单行,若抽到的字符串跨行,
    # 或含 Rust 语法标记,几乎必是引号错位产生的伪候选。
    if kind != "comment" and ("\n" in content or "//" in content or "#[" in content):
        return
    usage, note = classify(rel, text, off, kind)
    records.append({
        "owner": owner_of(rel),
        "path": rel,
        "offset": off,
        "kind": kind,
        "usage": usage,
        "note": note,
        "text": content,
    })


def _inside_rust_comment(text: str, off: int) -> bool:
    """off 是否落在 Rust `//` 或 `/* */` 注释里(字符串感知,避免误判)。"""
    i, n = 0, len(text)
    while i < n and i <= off:
        c = text[i]
        if c == "r" and i + 1 < n and text[i + 1] in '#"':
            j = i + 1
            hashes = 0
            while j < n and text[j] == "#":
                hashes += 1
                j += 1
            if j < n and text[j] == '"':
                close = '"' + "#" * hashes
                end = text.find(close, j + 1)
                end = n if end < 0 else end + len(close)
                if off < end:
                    return False
                i = end
                continue
        if c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                if text[j] == "\\":
                    j += 2
                else:
                    j += 1
            if off < j:
                return False
            i = j + 1
            continue
        if c == "'" and i + 1 < n:
            if text[i + 1] == "\\":
                j = i + 3
            else:
                j = i + 2
            if j < n and text[j] == "'":
                if off < j:
                    return False
                i = j + 1
                continue
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            j = text.find("\n", i)
            j = n if j < 0 else j
            return i <= off < j
        if c == "/" and i + 1 < n and text[i + 1] == "*":
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            if i <= off < j:
                return True
            i = j
            continue
        i += 1
    return False


def iter_files():
    for root in SRC_ROOTS:
        p = os.path.join(ROOT, root)
        if os.path.isfile(p):
            if is_source(p):
                yield p
            continue
        for dp, dns, fns in os.walk(p):
            dns[:] = [d for d in dns if d not in {"node_modules", "target", "dist", ".git", "__pycache__"}]
            for fn in fns:
                fp = os.path.join(dp, fn)
                if is_source(fp):
                    yield fp


def scan() -> list:
    records = []
    for fp in iter_files():
        rel = relpath(fp)
        with open(fp, encoding="utf-8", errors="replace") as f:
            text = f.read()
        if rel.endswith(".rs"):
            extract_rust(text, rel, records)
        elif rel.endswith((".ts", ".tsx", ".js", ".mjs", ".jsx")):
            extract_js(text, rel, records)
        elif rel.endswith(".html"):
            extract_html(text, rel, records)
    # 稳定 ID:owner + 规范化路径 + 文字(不用 offset,避免无关编辑改变身份)。
    seen = {}
    for r in records:
        key = (r["owner"], r["path"], r["text"])
        r["id"] = "cand-" + hashlib.sha1("|".join(key).encode()).hexdigest()[:12]
    records.sort(key=lambda r: (r["owner"], r["path"], r["offset"], r["text"]))
    return records


def main(argv):
    ap = argparse.ArgumentParser(description="i18n 候选扫描(Task 01)")
    ap.add_argument("--json", metavar="OUT", help="写出候选记录 JSON")
    ap.add_argument("--list", action="store_true", help="逐条打印")
    args = ap.parse_args(argv)

    records = scan()
    by_usage = {}
    by_owner = {}
    for r in records:
        by_usage[r["usage"]] = by_usage.get(r["usage"], 0) + 1
        by_owner[r["owner"]] = by_owner.get(r["owner"], 0) + 1

    if args.list:
        for r in records:
            print(f"[{r['usage']:<8}] {r['owner']:<18} {r['path']}:{r['offset']}  {r['text']!r}")
    if args.json:
        with open(args.json, "w", encoding="utf-8") as f:
            json.dump(records, f, ensure_ascii=False, indent=1, sort_keys=True)
            f.write("\n")
        print(f"wrote {len(records)} candidates -> {args.json}")

    print(f"total candidates: {len(records)}")
    print("by usage:", json.dumps(by_usage, ensure_ascii=False, sort_keys=True))
    ui = by_usage.get("ui", 0)
    excluded = len(records) - ui
    print(f"UI (needs triage): {ui}  excluded(comment/log/diag/test): {excluded}")
    print("by owner:", json.dumps(by_owner, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
