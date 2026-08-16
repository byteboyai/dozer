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

# Ask cargo for the built executables' actual paths via JSON output, rather
# than guessing target/<profile> vs target/<triple>/<profile> (this machine's
# ~/.cargo/config.toml pins a default --target, which changes the layout).
# dozerd must ship alongside dozer: dozer's daemon auto-spawn looks for it
# next to its own executable (crates/dozer-app/src/main.rs spawn_dozerd()).
# dozer-hook must ship alongside dozer too: agent hook auto-registration
# (crates/dozer-app/src/workspace.rs ensure_hook_installed()) writes hook
# commands that point at a "dozer-hook" binary sibling to dozer's own
# executable — if it's missing from the bundle, every registered hook fails
# with "no such file or directory" (2026-08 incident).
BUILD_JSON=$(
  cargo build "${CARGO_PROFILE_FLAG[@]}" -p dozer-app -p dozerd -p dozer-hook --message-format=json
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

OUT_DIR="$(dirname "$(dirname "$BIN_PATH")")/bundle/macos"
APP_DIR="$OUT_DIR/$APP_NAME.app"

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

cp "$BIN_PATH" "$APP_DIR/Contents/MacOS/$BIN_NAME"
cp "$DAEMON_BIN_PATH" "$APP_DIR/Contents/MacOS/$DAEMON_BIN_NAME"
cp "$HOOK_BIN_PATH" "$APP_DIR/Contents/MacOS/$HOOK_BIN_NAME"
cp "$PACKAGING_DIR/Info.plist" "$APP_DIR/Contents/Info.plist"
cp "$PACKAGING_DIR/AppIcon.icns" "$APP_DIR/Contents/Resources/AppIcon.icns"

# Nudge Finder/Dock to drop any cached icon for a previous build at this path.
touch "$APP_DIR"

echo "Bundled: $APP_DIR"
