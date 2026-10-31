#!/usr/bin/env bash
# 跨仓库一致性检查:byteui 的外壳区域样式 ≡ Dozer `workspace.json` 的同名区域
# (S1 期间两处并存,迁移后才合并)。原在 digger 仓库,Tier5 Task3 移到属主 dozer。
#
#   scripts/check-shell-regions.sh                 默认 sibling ../byteui
#   BYTEUI=/path/to/byteui scripts/check-shell-regions.sh
#
# 不一致时打印每一处差异并以非零退出;找不到 byteui(如单独 checkout)时跳过并说明。
set -euo pipefail
cd "$(dirname "$0")/.."

BYTEUI="${BYTEUI:-../byteui}"
JSON="crates/dozer-app/assets/theme/workspace.json"
[ -f "$JSON" ] || { echo "找不到 Dozer 的 workspace.json:$JSON" >&2; exit 2; }
[ -d "$BYTEUI" ] || {
  echo "note: 找不到 byteui($BYTEUI),跳过外壳区域一致性检查(设 BYTEUI=/path/to/byteui)" >&2
  exit 0
}

ours="$(mktemp)"
trap 'rm -f "$ours"' EXIT
(cd "$BYTEUI" && cargo run -q --example dump_shell_regions) > "$ours"

python3 -I - "$JSON" "$ours" <<'PY'
import json, sys

dozer = json.load(open(sys.argv[1]))
ours = json.load(open(sys.argv[2]))

def pad4(p):
    """Dozer 的 padding 有三种写法:单值 / [v,h] / [top,right,bottom,left];统一展开成四边。"""
    if p is None:
        return [0.0, 0.0, 0.0, 0.0]
    if isinstance(p, (int, float)):
        return [float(p)] * 4
    if len(p) == 2:
        return [float(p[0]), float(p[1]), float(p[0]), float(p[1])]
    return [float(x) for x in p]

def norm_region(r):
    border = r.get("border")
    return {
        "background": r.get("background"),
        "border": None if border is None else {"color": border["color"], "width": float(border["width"]), "radius": float(border["radius"])},
        "padding": pad4(r["padding"]),
        "gap": float(r["gap"]),
        "margin": pad4(r.get("margin")),
    }

bad = []
for name in ["top_bar", "left_icon_rail", "right_icon_rail", "status_bar", "left_zone", "right_zone", "context_menu"]:
    if name not in ours:
        bad.append(f"{name}: byteui 没有这个区域(byteui 版本太旧?)")
        continue
    a, b = norm_region(dozer["regions"][name]), norm_region(ours[name])
    for key in a:
        if a[key] != b[key]:
            bad.append(f"{name}.{key}: Dozer={a[key]!r} byteui={b[key]!r}")

d, o = dozer["regions"]["maximize_overlay"], ours["maximize_overlay"]
if d["scrim"]["background"] != o["scrim"]["background"] or float(d["scrim"]["padding"]) != float(o["scrim"]["padding"]):
    bad.append(f"maximize_overlay.scrim: Dozer={d['scrim']!r} byteui={o['scrim']!r}")
db, ob = d["border"], o["border"]
if (db["color"], float(db["width"]), float(db["radius"])) != (ob["color"], float(ob["width"]), float(ob["radius"])):
    bad.append(f"maximize_overlay.border: Dozer={db!r} byteui={ob!r}")
if dozer["background"] != ours["background"]:
    bad.append(f"background: Dozer={dozer['background']!r} byteui={ours['background']!r}")

if bad:
    print("byteui 外壳区域样式与 Dozer 的 workspace.json 不一致:", file=sys.stderr)
    for line in bad:
        print("  " + line, file=sys.stderr)
    sys.exit(1)
print("shell regions: ok(byteui 的 8 个外壳区域与 Dozer workspace.json 逐项一致)")
PY
