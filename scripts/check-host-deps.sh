#!/usr/bin/env bash
# Check every workspace member except explicit host adapters, using one locked graph.
# Never rewrite Cargo.lock, suppress Cargo/JSON errors, or omit new business packages.
set -euo pipefail
cd "$(dirname "$0")/.."

# 自检使用隔离 workspace,不读取当前仓库可能带个人 patch 的锁文件。
if [ "${1:-}" = "--self-test" ]; then
  exec python3 scripts/tests/test_host_deps.py
fi

HOST_ADAPTER_CRATES='dozer-app dozer-client dozer-core dozerd dozer-mcp dozer-hook dozer-panel-agent dozer-panel-browser dozer-panel-conversations dozer-panel-files dozer-panel-group-chat dozer-panel-host dozer-panel-preview dozer-panel-project dozer-panel-term dozer-panel-todo dozer-panel-usage'
IDENTITY_CONTRACT=''
metadata=$(cargo metadata --locked --all-features --format-version 1) || {
  echo "无法读取锁定依赖图,依赖门禁失败" >&2
  exit 1
}
printf '%s' "$metadata" | python3 -c '
import json, re, sys
try:
    graph = json.load(sys.stdin)
    packages = {p["id"]: p for p in graph["packages"]}
    members = graph["workspace_members"]
    nodes = {n["id"]: n for n in graph["resolve"]["nodes"]}
    if not members:
        raise ValueError("workspace 成员为空")
    adapters = set(sys.argv[1].split())
    identity = sys.argv[2]
    banned = re.compile(r"^(bytehost-.*|iced|iced_.*|wry)$")
    checked = 0
    for member in members:
        name = packages[member]["name"]
        if name in adapters:
            continue
        checked += 1
        pending = [member]
        seen = set()
        while pending:
            package = pending.pop()
            if package in seen:
                continue
            seen.add(package)
            dep_name = packages[package]["name"]
            # ProductId/ProductDirs use the pure bytehost-apps wire contract.
            # Continue traversing it: GUI/other host dependencies remain forbidden.
            contract = name == identity and dep_name == "bytehost-apps"
            if banned.fullmatch(dep_name) and not contract:
                raise ValueError(f"业务 crate {name} 依赖了宿主层 / 界面框架 / webview: {dep_name}")
            for dep in nodes[package]["deps"]:
                if any(k["kind"] in (None, "build") for k in dep["dep_kinds"]):
                    pending.append(dep["pkg"])
    print(f"dozer host-deps check: ok(审视了 {checked} 个业务 crate)")
except (KeyError, TypeError, ValueError) as error:
    print(f"依赖门禁失败: {error}", file=sys.stderr)
    sys.exit(1)
' "$HOST_ADAPTER_CRATES" "$IDENTITY_CONTRACT"
