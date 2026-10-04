#!/bin/sh
# 一次性验证矩阵:每种 origin 方案跑 write → read(同存储)→ read(另一个存储标识),结果写到 results.jsonl。
# 用法: sh run_matrix.sh   (会短暂弹出几个小窗口,每个 <10 秒)
set -e
cd "$(dirname "$0")"
B=./target/aarch64-apple-darwin/debug/origin-gateway-spike
OUT=results.jsonl
: > "$OUT"
for mode in custom localhost loopback; do
  M="MARK-$mode-$(date +%s)"
  $B --mode $mode --phase write --marker "$M" --store 11 | sed -n 's/^RESULT //p' | sed "s/^{/{\"run\":\"write\",\"expect\":\"$M\",/" >> "$OUT"
  $B --mode $mode --phase read  --store 11 | sed -n 's/^RESULT //p' | sed "s/^{/{\"run\":\"read-same-store\",\"expect\":\"$M\",/" >> "$OUT"
  $B --mode $mode --phase read  --store 12 | sed -n 's/^RESULT //p' | sed "s/^{/{\"run\":\"read-other-store\",\"expect\":\"$M\",/" >> "$OUT"
done
echo "done: $OUT"
