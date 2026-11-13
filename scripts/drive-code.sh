#!/bin/bash
# コード表示の操作を実際の入力でたどる。使い方: scripts/drive-code.sh <出力先> <リポジトリ>
set -uo pipefail
O=$1; REPO=$2; rm -rf "$O"; mkdir -p "$O"
DATA=$(mktemp -d); cp -R target/demo-data/. "$DATA"/
"${BIN:-target/release/cc-code-view}" --data-dir "$DATA" --open "$REPO" --demo code 2>/dev/null & PID=$!
sleep 3
W=$(target/winid cc-code-view)
shot() { screencapture -x -o -l "$W" "$O/$1.png"; }
run() { target/poke cc-code-view "$@" || { echo "ABORT $*"; kill $PID; rm -rf "$DATA"; exit 2; }; }
run drag 700 200 700 300; sleep .5; shot a-drag
run scroll -600 800 400; sleep .5; shot b-scrolled
run scroll 600 800 400; sleep .3
run cmdclick 360 116; sleep 1.2; shot c-cmdclick
# JIS 配列では [ のキーコードが 30 になる。US 配列では 33
run key "${BRACKET_KEY:-30}" cmd; sleep 1.2; shot d-back
run cmdclick 360 116; sleep 1.2
run click 260 85; sleep 1.2; shot d2-back-button  # パンくずの ‹ ボタン
run key 13 cmd; sleep 1.5; shot e-tab-closed   # ⌘W でタブを閉じる
kill $PID; wait $PID 2>/dev/null; rm -rf "$DATA"
echo done
