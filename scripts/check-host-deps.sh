#!/usr/bin/env bash
# 门禁:Dozer 的**领域 / 持久化 / 业务运行时**必须与 bytehost 宿主层、界面框架、webview 解耦
# (bytehost 优化 Tier5 Task2,移植自 digger scripts/check-host-deps.sh)。
#
# 依赖方向:
#
#     dozer-app / dozer-client / dozer-core / dozerd / dozer-mcp / dozer-hook   (组合根与宿主适配)
#                          |
#                          v
#     dozer-model / dozer-services / dozer-store-project / dozer-preview-core / dozer-i18n
#                          (纯业务:不得直连 bytehost-* / iced* / wry)
#
#   bytehost-* 允许被宿主适配层与面板库依赖,绝不被业务层依赖;业务层需要宿主能力时,
#   由适配层注入(与面板库的 PanelIo / 适配器注入同一模式)。
#
# 非宿主白名单的 workspace 成员自动纳入,新业务 crate 不依赖名称约定。
#
# 运行:scripts/check-host-deps.sh
# 自检:scripts/check-host-deps.sh --self-test(注入非法依赖必须失败)

set -euo pipefail
cd "$(dirname "$0")/.."

# 组合根与现有面板宿主适配层。新成员默认按业务层检查,例外必须显式评审。
HOST_ADAPTER_CRATES='dozer-app dozer-client dozer-core dozerd dozer-mcp dozer-hook dozer-panel-agent dozer-panel-browser dozer-panel-conversations dozer-panel-files dozer-panel-group-chat dozer-panel-host dozer-panel-preview dozer-panel-project dozer-panel-term dozer-panel-todo dozer-panel-usage'

# 业务层禁用:bytehost-*(宿主层)、iced*(界面框架)、wry(webview)。
bytehost_banned='^(bytehost-.*|iced|iced_.*|wry)$'

tree_of() {
  local pkg="$1"
  local out
  out=$(cargo tree -p "$pkg" --all-features -e normal,build --target all --prefix none 2>&1) || {
    echo "cargo tree 失败:" >&2
    echo "$out" >&2
    exit 1
  }
  echo "$out" | awk '{print $1}' | sort -u
}

# 枚举 workspace 里所有生产包。macOS 自带 bash 3.2 没有 mapfile,用 read 循环。
package_names=$(cargo metadata --no-deps --format-version 1 |
  python3 -c 'import json,sys
m=json.load(sys.stdin)
members=set(m["workspace_members"])
for p in sorted(m["packages"], key=lambda p: p["name"]):
    if p["id"] in members:
        print(p["name"])') || {
  echo "无法枚举 workspace 成员,依赖门禁失败" >&2
  exit 1
}
packages=()
while IFS= read -r line; do
  [ -n "$line" ] && packages+=("$line")
done <<< "$package_names"
[ "${#packages[@]}" -gt 0 ] || { echo "workspace 成员为空,依赖门禁失败" >&2; exit 1; }

checked=0
for pkg in "${packages[@]}"; do
  case " $HOST_ADAPTER_CRATES " in
    *" $pkg "*) continue ;;
  esac
    tree=$(tree_of "$pkg") || exit 1
    hits=$(echo "$tree" | grep -E "$bytehost_banned" || true)
    if [ -n "$hits" ]; then
      echo "业务 crate $pkg 依赖了宿主层 / 界面框架 / webview(bytehost-*、iced*、wry;应经适配层注入,不得直连):" >&2
      echo "$hits" >&2
      exit 1
    fi
    checked=$((checked + 1))
done

echo "dozer host-deps check: ok(审视了 $checked 个业务 crate)"

if [ "${1:-}" = "--self-test" ]; then
  # 隔离的 Cargo workspace 与故障注入,不改当前仓库或 Cargo.lock。
  exec python3 scripts/tests/test_host_deps.py
fi
