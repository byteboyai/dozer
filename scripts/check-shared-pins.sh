#!/usr/bin/env bash
# 门禁:bytehost / byteui / bytegit 在整个 workspace 里只许出现一个 tag。
# 原因:同一 git 源的两个 tag 在无本地 [patch] 的消费方(digger)里会解析成两份 crate,
# `PanelId` 之类类型互不兼容,byteui 的全局主题状态也会分裂。
# 用法: scripts/check-shared-pins.sh [--self-test]
set -euo pipefail
ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$ROOT"

collect() {
  # 输出 "<仓库名> <tag>" 去重列表
  grep -rhoE 'github.com/byteboyai/(bytehost|byteui|bytegit)", *tag *= *"[^"]+"' crates/*/Cargo.toml Cargo.toml 2>/dev/null \
    | sed -E 's#github.com/byteboyai/([a-z]+)", *tag *= *"([^"]+)"#\1 \2#' | sort -u
}

check() {
  local bad=0
  for repo in bytehost byteui bytegit; do
    local n; n=$(collect | awk -v r="$repo" '$1==r' | wc -l | tr -d ' ')
    if [ "$n" -gt 1 ]; then
      echo "CONFLICT: $repo 有 $n 个不同 tag:" >&2
      collect | awk -v r="$repo" '$1==r {print "  " $2}' >&2
      bad=1
    fi
  done
  return $bad
}

if [ "${1:-}" = "--self-test" ]; then
  tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/crates/a" "$tmp/crates/b"
  echo 'x = { git = "https://github.com/byteboyai/byteui", tag = "v1" }' > "$tmp/crates/a/Cargo.toml"
  echo 'x = { git = "https://github.com/byteboyai/byteui", tag = "v2" }' > "$tmp/crates/b/Cargo.toml"
  if ROOT="$tmp" bash "$0" >/dev/null 2>&1; then
    echo "self-test FAILED: 冲突没有被抓到" >&2; exit 1
  fi
  echo "self-test ok"; exit 0
fi

if check; then echo "shared pins: ok"; else exit 1; fi
