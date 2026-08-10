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

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGING_DIR="$ROOT_DIR/crates/dozer-app/packaging/macos"
APP_NAME="Dozer AI Coder"
BIN_NAME="dozer"

# Ask cargo for the built executable's actual path via JSON output, rather
# than guessing target/<profile> vs target/<triple>/<profile> (this machine's
# ~/.cargo/config.toml pins a default --target, which changes the layout).
BIN_PATH=$(
  cargo build "${CARGO_PROFILE_FLAG[@]}" -p dozer-app --message-format=json \
    | jq -r --arg bin "$BIN_NAME" \
        'select(.reason=="compiler-artifact" and .target.name==$bin and .executable != null) | .executable' \
    | tail -n 1
)

if [ -z "$BIN_PATH" ] || [ ! -x "$BIN_PATH" ]; then
  echo "error: could not locate built '$BIN_NAME' executable" >&2
  exit 1
fi

OUT_DIR="$(dirname "$(dirname "$BIN_PATH")")/bundle/macos"
APP_DIR="$OUT_DIR/$APP_NAME.app"

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

cp "$BIN_PATH" "$APP_DIR/Contents/MacOS/$BIN_NAME"
cp "$PACKAGING_DIR/Info.plist" "$APP_DIR/Contents/Info.plist"
cp "$PACKAGING_DIR/AppIcon.icns" "$APP_DIR/Contents/Resources/AppIcon.icns"

# Nudge Finder/Dock to drop any cached icon for a previous build at this path.
touch "$APP_DIR"

echo "Bundled: $APP_DIR"
