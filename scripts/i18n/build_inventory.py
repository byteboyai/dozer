#!/usr/bin/env python3
"""从 `scan_sources.py` 的候选生成 `docs/dozer-v2/i18n/inventory.json`(Task 01)。

把"人工裁决"编码成两处声明:
  - `FILE_TASK`:每个源文件归到哪个实施 Task(§3 的任务号),这是任务分配的唯一真相。
  - `EXCLUSIONS`:已确认有理由排除的规则(reason),默认按 usage 归类,可逐条覆写。

生成器保证:候选总数 = 已迁移(状态 migrated,初始 0) + 有理由排除(excluded) + 未解决(unresolved)。
初始全部 UI 候选为 `unresolved`,excluded 依据 usage 归类。每次跑覆盖式重写 inventory,
让"未解决数可重跑核对"成立。运行后再由各迁移任务把 key/证据写回对应记录。
"""
from __future__ import annotations

import collections
import importlib.util
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
OUT = os.path.join(ROOT, "docs", "dozer-v2", "i18n", "inventory.json")

_spec = importlib.util.spec_from_file_location("scan_sources", os.path.join(HERE, "scan_sources.py"))
scan = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(scan)

# 文件 → 实施 Task。前缀匹配,更长前缀优先。未列出的 UI 文件保持 None(需再裁决)。
FILE_TASK = [
    # I1
    ("crates/dozer-app/src/product.rs", "05"),
    ("crates/dozer-app/src/panel_registry.rs", "05"),
    ("crates/dozer-app/src/extensions/settings.rs", "05"),
    ("crates/dozer-app/src/platform/settings_overlay.rs", "05"),
    ("crates/dozer-app/src/chrome/rail.rs", "05"),
    # I2 host chrome / menus / overlays
    ("crates/dozer-app/src/chrome/", "06"),
    ("crates/dozer-app/src/app/view.rs", "06"),
    ("crates/dozer-app/src/app/app.rs", "06"),
    ("crates/dozer-app/src/workspace/view.rs", "06"),
    ("crates/dozer-app/src/menu_spec.rs", "06"),
    ("crates/dozer-app/src/platform/", "06"),
    ("crates/dozer-app/src/extensions/footbar.rs", "06"),
    ("crates/dozer-app/src/extensions/file_history.rs", "06"),
    ("crates/dozer-app/src/extensions/edit_history.rs", "06"),
    ("crates/dozer-app/src/extensions/search.rs", "06"),
    ("crates/dozer-app/src/extensions/project_create.rs", "06"),
    ("crates/dozer-app/src/extensions/settings_apps_view.rs", "06"),
    # Toast & errors
    ("crates/dozer-app/src/extensions/toast.rs", "07"),
    ("crates/dozer-app/src/app/update.rs", "07"),
    ("crates/dozer-app/src/app/state.rs", "07"),
    ("crates/dozer-app/src/workspace/state.rs", "07"),
    ("crates/dozer-app/src/workspace/hook.rs", "07"),
    ("crates/dozer-app/src/git_accounts.rs", "07"),
    ("crates/dozer-app/src/secrets.rs", "07"),
    ("crates/dozer-app/src/external_apps.rs", "07"),
    ("crates/dozer-app/src/runtime.rs", "07"),
    ("crates/dozer-app/src/project.rs", "07"),
    ("crates/dozer-app/src/term/terminal.rs", "07"),
    # WebView locale passing
    ("crates/dozer-app/src/preview/webview_protocol.rs", "08"),
    # First panels
    ("crates/dozer-app/src/extensions/todo/", "09"),
    ("crates/dozer-app/web/todo-content/", "09"),
    ("crates/dozer-panel-usage/", "09"),
    ("crates/dozer-app/src/extensions/codehealth/", "09"),
    ("crates/dozer-app/web/codehealth-content/", "09"),
    # Remaining official panels + aux
    ("crates/dozer-app/src/extensions/files/", "10"),
    ("crates/dozer-app/src/extensions/project.rs", "10"),
    ("crates/dozer-app/src/extensions/project/", "10"),
    ("crates/dozer-app/src/extensions/database/", "10"),
    ("crates/dozer-app/src/extensions/ssh/", "10"),
    ("crates/dozer-app/src/extensions/ssh.rs", "10"),
    ("crates/dozer-app/src/extensions/browser.rs", "10"),
    ("crates/dozer-app/src/extensions/conversations.rs", "10"),
    ("crates/dozer-app/src/extensions/agent_context.rs", "10"),
    ("crates/dozer-app/src/extensions/group_chat/", "10"),
    ("crates/dozer-app/web/group-chat-content/", "10"),
    ("crates/dozer-app/src/extensions/git_log.rs", "10"),
    ("crates/dozer-app/src/extensions/app_logs.rs", "10"),
    ("crates/dozer-app/src/extensions/diff_content.rs", "10"),
    ("crates/dozer-app/web/review-trace/", "10"),
    # Preview web + html templates
    ("crates/dozer-app/src/preview/", "11"),
    ("crates/dozer-app/src/html_host.html", "11"),
    ("crates/dozer-app/src/image_annotate_host.html", "11"),
    ("crates/dozer-app/web/editor/", "11"),
    ("crates/dozer-app/web/json-editor/", "11"),
    ("crates/dozer-app/web/tabular-host/", "11"),
    ("crates/dozer-app/web/flyfish/", "11"),
    ("crates/dozer-app/web/plantuml-viewer/", "11"),
    ("crates/dozer-app/src/tabular/mod.rs", "11"),
]

# 明确不是面向用户的源码:测试辅助/黄金值工具/诊断探针/纯内部常量,排除。
EXCLUDED_FILES = [
    "crates/dozer-app/src/workspace/tests.rs",
    "crates/dozer-app/src/app/layout.rs",
    "crates/dozer-app/src/usage_probe/scenarios.rs",
    "crates/dozer-app/src/usage_probe/host.rs",
    "crates/dozer-app/src/service_fakes.rs",
    "crates/dozer-app/src/term/term_model.rs",
    "crates/dozer-core/src/log.rs",
]
EXCLUDED_FILE_REASON = "测试/黄金值工具/诊断探针/内部常量;非面向用户文案"

# 个别 UI 文件里"不是 UI 文案"的具体字符串:面板清单构造器的自检不变量错误——
# 只在产品组合根 `.expect(...)` 时以 panic 形式暴露,是开发者错误,不给最终用户看。
NON_UI_STRINGS = {
    ("crates/dozer-app/src/panel_registry.rs", "面板清单为空"): "面板清单构造器不变量错误;仅 `.expect` 时 panic,不面向用户",
    ("crates/dozer-app/src/panel_registry.rs", "面板描述符里有重复的面板"): "面板清单构造器不变量错误;仅 `.expect` 时 panic,不面向用户",
    ("crates/dozer-app/src/panel_registry.rs", "默认布局不合法:两栏都要非空,且恰好覆盖全部已注册面板、不重复、不含未注册面板"): "面板清单构造器不变量错误;仅 `.expect` 时 panic,不面向用户",
    ("crates/dozer-app/src/panel_registry.rs", "面板清单已经注册过"): "面板清单构造器不变量错误;仅 `.expect` 时 panic,不面向用户",
}

# 非 app 的后端 crate:字符串不是 UI,按 §10 归为"错误/诊断适配",由 Task 07 与
# 相关面板任务处理;这里统一标为排除理由,"UI 文案"仍由面板/宿主边界决定。
BACKEND_REASON = "后端 crate 非 UI 字符串(错误/协议/日志);如经服务边界展示给用户,由 Task 07 适配稳定 code + 本地化摘要"


def task_of(path: str) -> str | None:
    best = None
    for prefix, task in FILE_TASK:
        if path.startswith(prefix) and (best is None or len(prefix) > best[1]):
            best = (task, len(prefix))
    return best[0] if best else None


def owner_for(path: str) -> str:
    return scan.owner_of(path)


def classify_status(rec) -> tuple[str, str | None]:
    """返回 (status, reason)。"""
    usage = rec["usage"]
    path = rec["path"]
    if usage == "test":
        return "excluded", "测试/fixture 内文案;协议相关者随协议迁移(Task 08/09/11)"
    if usage == "log":
        return "excluded", "日志宏参数;机器日志不翻译(可另行保留原文)"
    if usage == "diag":
        return "excluded", "开发期诊断宏(assert/expect/panic);不面向用户"
    if any(path == f for f in EXCLUDED_FILES):
        return "excluded", EXCLUDED_FILE_REASON
    if (path, rec["text"]) in NON_UI_STRINGS:
        return "excluded", NON_UI_STRINGS[(path, rec["text"])]
    # usage == ui
    if path.startswith(("crates/dozer-client/", "crates/dozerd/", "crates/dozer-mcp/", "crates/dozer-hook/")):
        return "excluded", BACKEND_REASON
    return "unresolved", None


def build():
    records = scan.scan()
    out = []
    for r in records:
        status, reason = classify_status(r)
        out.append({
            "id": r["id"],
            "owner": r["owner"],
            "path": r["path"],
            "symbol_hint": f"{r['kind']}@offset {r['offset']}",
            "kind": r["kind"],
            "usage": r["usage"],
            "text": r["text"],
            "task": task_of(r["path"]) if status == "unresolved" else None,
            "key": None,
            "status": status,
            "reason": reason,
            "evidence": None,
        })
    out.sort(key=lambda x: (x["status"], x["path"], x["symbol_hint"]))
    return out


def main():
    recs = build()
    counts = collections.Counter(r["status"] for r in recs)
    tasks = collections.Counter(r["task"] for r in recs if r["status"] == "unresolved")
    unassigned = [r for r in recs if r["status"] == "unresolved" and not r["task"]]

    doc = {
        "_comment": "由 scripts/i18n/build_inventory.py 生成;每次覆盖重写,勿手改结构。",
        "schema": 1,
        "total": len(recs),
        "counts": {
            "unresolved": counts.get("unresolved", 0),
            "migrated": counts.get("migrated", 0),
            "excluded": counts.get("excluded", 0),
        },
        "unresolved_by_task": {str(k): v for k, v in sorted(tasks.items(), key=lambda kv: (kv[0] is None, kv[0] or ""))},
        "unassigned_unresolved": len(unassigned),
        "records": recs,
    }
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(doc, f, ensure_ascii=False, indent=1, sort_keys=True)
        f.write("\n")

    print(f"wrote {len(recs)} records -> {os.path.relpath(OUT, ROOT)}")
    print("counts:", json.dumps(doc["counts"], ensure_ascii=False, sort_keys=True))
    print("unresolved by task:", json.dumps(doc["unresolved_by_task"], ensure_ascii=False, sort_keys=True))
    if unassigned:
        print(f"WARNING: {len(unassigned)} unresolved records have no task:")
        for r in unassigned[:20]:
            print(f"   {r['path']}  {r['text']!r}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
