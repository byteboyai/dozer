#!/usr/bin/env bash
# Builds crates/dozer-app and wraps it into a Dozer.app bundle with the icon
# from crates/dozer-app/packaging/macos/{Info.plist,AppIcon.icns}.
#
# Usage: scripts/build-macos-app.sh [debug|release]   (default: release)
set -euo pipefail

PROFILE="${1:-release}"
case "$PROFILE" in
  debug) CARGO_PROFILE_FLAG=() ;;
  release) CARGO_PROFILE_FLAG=(--release) ;;
  *) echo "usage: $0 [debug|release]" >&2; exit 1 ;;
esac

# 发布打包前自动把版本号 patch +1(debug 构建不自动 bump,避免无意义自增)。
# 想验证而不改文件时,先手动跑 scripts/bump-version.sh --dry-run。
if [[ "$PROFILE" == "release" ]]; then
  "$(dirname "${BASH_SOURCE[0]}")/bump-version.sh"
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGING_DIR="$ROOT_DIR/crates/dozer-app/packaging/macos"
APP_NAME="Dozer AI Coder"
BIN_NAME="dozer"
DAEMON_BIN_NAME="dozerd"
HOOK_BIN_NAME="dozer-hook"
MCP_BIN_NAME="dozer-mcp"

# Ask cargo for the built executables' actual paths via JSON output, rather
# than guessing target/<profile> vs target/<triple>/<profile> (this machine's
# ~/.cargo/config.toml pins a default --target, which changes the layout).
# dozerd must ship alongside dozer: dozer's daemon auto-spawn looks for it
# next to its own executable (crates/dozer-app/src/main.rs spawn_dozerd()).
# dozer-hook must ship alongside dozer too: agent hook auto-registration
# (crates/dozer-app/src/workspace.rs ensure_hook_installed()) writes hook
# commands that point at a "dozer-hook" binary sibling to dozer's own
# executable — if it's missing from the bundle, every registered hook fails
# with "no such file or directory" (2026-08 incident). dozer-mcp must ship
# alongside dozer for the same reason: ensure_mcp_installed() (spec
# 2026-08-27) registers agent MCP configs pointing at a "dozer-mcp" sibling
# binary, and v8agent-cli auto-mounts "dozer-mcp serve" by bare command name
# off PATH/its working dir assumptions — omitting it here reproduces the
# exact class of bug the dozer-hook incident already taught us to guard
# against.
BUILD_JSON=$(
  cargo build "${CARGO_PROFILE_FLAG[@]}" -p dozer-app -p dozerd -p dozer-hook -p dozer-mcp --message-format=json
)

BIN_PATH=$(
  echo "$BUILD_JSON" | jq -r --arg bin "$BIN_NAME" \
    'select(.reason=="compiler-artifact" and .target.name==$bin and .executable != null) | .executable' \
    | tail -n 1
)
DAEMON_BIN_PATH=$(
  echo "$BUILD_JSON" | jq -r --arg bin "$DAEMON_BIN_NAME" \
    'select(.reason=="compiler-artifact" and .target.name==$bin and .executable != null) | .executable' \
    | tail -n 1
)
HOOK_BIN_PATH=$(
  echo "$BUILD_JSON" | jq -r --arg bin "$HOOK_BIN_NAME" \
    'select(.reason=="compiler-artifact" and .target.name==$bin and .executable != null) | .executable' \
    | tail -n 1
)
MCP_BIN_PATH=$(
  echo "$BUILD_JSON" | jq -r --arg bin "$MCP_BIN_NAME" \
    'select(.reason=="compiler-artifact" and .target.name==$bin and .executable != null) | .executable' \
    | tail -n 1
)

if [ -z "$BIN_PATH" ] || [ ! -x "$BIN_PATH" ]; then
  echo "error: could not locate built '$BIN_NAME' executable" >&2
  exit 1
fi
if [ -z "$DAEMON_BIN_PATH" ] || [ ! -x "$DAEMON_BIN_PATH" ]; then
  echo "error: could not locate built '$DAEMON_BIN_NAME' executable" >&2
  exit 1
fi
if [ -z "$HOOK_BIN_PATH" ] || [ ! -x "$HOOK_BIN_PATH" ]; then
  echo "error: could not locate built '$HOOK_BIN_NAME' executable" >&2
  exit 1
fi
if [ -z "$MCP_BIN_PATH" ] || [ ! -x "$MCP_BIN_PATH" ]; then
  echo "error: could not locate built '$MCP_BIN_NAME' executable" >&2
  exit 1
fi

OUT_DIR="$(dirname "$(dirname "$BIN_PATH")")/bundle/macos"
APP_DIR="$OUT_DIR/$APP_NAME.app"

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

cp "$BIN_PATH" "$APP_DIR/Contents/MacOS/$BIN_NAME"
cp "$DAEMON_BIN_PATH" "$APP_DIR/Contents/MacOS/$DAEMON_BIN_NAME"
cp "$HOOK_BIN_PATH" "$APP_DIR/Contents/MacOS/$HOOK_BIN_NAME"
cp "$MCP_BIN_PATH" "$APP_DIR/Contents/MacOS/$MCP_BIN_NAME"
cp "$PACKAGING_DIR/Info.plist" "$APP_DIR/Contents/Info.plist"
cp "$PACKAGING_DIR/AppIcon.icns" "$APP_DIR/Contents/Resources/AppIcon.icns"
# flyfish 文件预览静态资源(text/image/pdf 渲染管线 + vendor/pdf)。
# dozer 的 `assets::assets_root()` 在打包态从 `Contents/Resources/flyfish`
# 读(dev 态才回退源码树),漏拷会导致分发后的文件预览 404。
cp -R "$ROOT_DIR/crates/dozer-app/assets/flyfish" "$APP_DIR/Contents/Resources/flyfish"
# CodeMirror editor host 静态资源(Phase B)。`assets::handle_protocol` 的
# `dozer://editor/` 命名空间从 flyfish 根的**兄弟目录** `Contents/Resources/
# editor` 读(dev 态同构:assets/editor),必须一并打包,否则 editor 预览 404。
cp -R "$ROOT_DIR/crates/dozer-app/assets/editor" "$APP_DIR/Contents/Resources/editor"

# cargo 链接期只对裸二进制做了 ad-hoc 签名(`codesign -dv` 显示
# `Info.plist=not bound`),装进 bundle 后这个签名并不覆盖 Info.plist/资源,
# 而且每次 rebuild 内容一变,ad-hoc identifier 也跟着变。macOS 对同一路径
# 突然"换了身份"的 app 会重新走一次首次运行校验,表现为图标第一次点击弹出
# 访达式的包内容预览而不是直接启动,得点第二次才真的打开(2026-09-21 用户
# 报告)。装配完整个 bundle 之后对它整体重签一次 ad-hoc 签名,让签名覆盖
# Info.plist,给这次构建一个自洽的身份。
codesign --force --deep --sign - "$APP_DIR"

# Nudge Finder/Dock to drop any cached icon for a previous build at this path.
touch "$APP_DIR"

# 光靠 touch 只能让 Finder 丢弃图标缓存,Spotlight/Quick Look 认不认这个路径
# 是"应用程序"归 Launch Services 的索引管,touch 碰不到。同一路径重复
# rm -rf 重建之后,LS 的索引可能还留着旧那份、或者干脆还没跟上,不强制刷新
# 就是前面那条 codesign 要解决的"第一次点击不对、第二次才行"的另一半成因。
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister"
if [ -x "$LSREGISTER" ]; then
  "$LSREGISTER" -f "$APP_DIR"
fi

echo "Bundled: $APP_DIR"
