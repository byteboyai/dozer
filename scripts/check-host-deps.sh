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
# 新业务 crate 只要命名落在 BUSINESS_PREFIX 内就**自动纳入**,不需要改本脚本。
#
# 运行:scripts/check-host-deps.sh
# 自检:scripts/check-host-deps.sh --self-test(注入非法依赖必须失败)

set -euo pipefail
cd "$(dirname "$0")/.."

# 业务层 crate 前缀:出现即不得依赖任何 bytehost-* / iced* / wry。
BUSINESS_PREFIX='^dozer-(model|services|store-project|preview-core|i18n)(-.*)?$'

# 业务层禁用:bytehost-*(宿主层)、iced*(界面框架)、wry(webview)。
bytehost_banned='^(bytehost-.*|iced|iced_.*|wry)$'

tree_of() {
  local pkg="$1"
  local out
  out=$(cargo tree -p "$pkg" -e normal,build --target all --prefix none 2>&1) || {
    echo "cargo tree 失败:" >&2
    echo "$out" >&2
    exit 1
  }
  echo "$out" | awk '{print $1}' | sort -u
}

# 枚举 workspace 里所有生产包。macOS 自带 bash 3.2 没有 mapfile,用 read 循环。
packages=()
while IFS= read -r line; do
  [ -n "$line" ] && packages+=("$line")
done < <(
  cargo metadata --no-deps --format-version 1 |
    python3 -c 'import json,sys
m=json.load(sys.stdin)
for p in sorted(m["packages"], key=lambda p: p["name"]):
    print(p["name"])'
)

checked=0
for pkg in "${packages[@]}"; do
  if [[ "$pkg" =~ $BUSINESS_PREFIX ]]; then
    hits=$(tree_of "$pkg" | grep -E "$bytehost_banned" || true)
    if [ -n "$hits" ]; then
      echo "业务 crate $pkg 依赖了宿主层 / 界面框架 / webview(bytehost-*、iced*、wry;应经适配层注入,不得直连):" >&2
      echo "$hits" >&2
      exit 1
    fi
    checked=$((checked + 1))
  fi
done

echo "dozer host-deps check: ok(审视了 $checked 个业务 crate)"

if [ "${1:-}" = "--self-test" ]; then
  echo "运行门禁变异测试…" >&2
  probe="crates/_host_deps_probe"
  lock_bak="$(mktemp)"
  fake_root="$(mktemp -d)"
  cleanup() {
    cp "$lock_bak" Cargo.lock 2>/dev/null || true
    rm -rf "$probe" "$fake_root" "$lock_bak"
  }
  trap cleanup EXIT
  cp Cargo.lock "$lock_bak"

  # 三个「被禁名字」的假 crate:用包名命中三条规则,避免自检依赖联网拉真实 iced/wry。
  for fake in bytehost-mutation-probe iced wry; do
    mkdir -p "$fake_root/$fake/src"
    echo "" > "$fake_root/$fake/src/lib.rs"
    cat > "$fake_root/$fake/Cargo.toml" <<TOMLEOF
[package]
name = "$fake"
version = "0.0.0"
edition = "2021"
publish = false
[workspace]
TOMLEOF
  done

  mkdir -p "$probe/src"
  echo "" > "$probe/src/lib.rs"

  # 每种禁用依赖各注入一次:必须被拦下;恢复后必须通过。
  inject() {
    local label="$1" dep="$2"
    cat > "$probe/Cargo.toml" <<TOMLEOF
[package]
name = "dozer-model-mutation-probe"
version = "0.0.0"
edition = "2024"
publish = false
[dependencies]
$dep
TOMLEOF
    # 门禁按 `crates/*` 通配即可发现探针,无需改根 members。
    if bash "$0" >/dev/null 2>&1; then
      echo "变异测试失败:业务包依赖 $label 后门禁仍然通过" >&2
      exit 1
    fi
    echo "  变异通过:业务包→$label 被拦下" >&2
  }
  inject "bytehost-*" "bytehost-mutation-probe = { path = \"$fake_root/bytehost-mutation-probe\" }"
  inject "iced" "iced = { path = \"$fake_root/iced\" }"
  inject "wry" "wry = { path = \"$fake_root/wry\" }"
  rm -rf "$probe"
  cp "$lock_bak" Cargo.lock

  if ! bash "$0" >/dev/null 2>&1; then
    echo "变异测试失败:恢复后门禁未通过" >&2
    exit 1
  fi
  echo "门禁变异测试:ok(注入即失败、恢复即通过)" >&2
fi
