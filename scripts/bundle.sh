#!/bin/bash
# .app を作ってアドホック署名する。公証はしない。
set -euo pipefail
cargo build --release -p cc-code-view
APP="target/CC Code View.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/cc-code-view "$APP/Contents/MacOS/"
cp crates/app/assets/Info.plist "$APP/Contents/Info.plist"
cp crates/app/assets/AppIcon.icns "$APP/Contents/Resources/"
codesign --force --deep --sign - "$APP" 2>/dev/null
du -sh "$APP"
