#!/bin/bash
# ビューアーとしての基本操作を実際の入力でたどる。使い方: scripts/drive-viewer.sh <出力先> <リポジトリ>
# 文字はキーコードで送るので、日本語入力をオフ（英数）にしてから実行する
set -uo pipefail
O=$1; REPO=$2; rm -rf "$O"; mkdir -p "$O"
BIN=${BIN:-target/release/cc-code-view}
DATA=$(mktemp -d); cp -R target/demo-data/. "$DATA"/
"$BIN" --data-dir "$DATA" --open "$REPO" --demo done 2>/dev/null & PID=$!
sleep 3.5
W=$(target/winid cc-code-view)
shot() { screencapture -x -o -l "$W" "$O/$1.png"; }
run() { target/poke cc-code-view "$@" || { echo "ABORT $*"; kill $PID; rm -rf "$DATA"; exit 2; }; }
# 左のツリーの行は 24pt 間隔で、先頭の行は y=81 付近
run click 40 172; sleep .6; shot a-folder-open        # src を開く
run click 60 196; sleep .6; shot b-subfolder           # 最初の子フォルダー
run click 80 220; sleep 1.2; shot c-file-open          # その中の最初のファイル
run key 125; sleep .8; run key 125; sleep 1.2; shot d-arrow-preview   # ↓ で隣のファイルを下見のタブで開く
run click 560 400; sleep .3                            # コードを触ってキー操作を画面へ戻す
run key 3 cmd; sleep .4; run key 17; sleep .6; shot e-find-t   # 検索欄で t を押してもツアーが始まらない
run keys "const"; sleep 1; shot f-find
run key 36; sleep .6; shot g-find-next
run key 53; sleep .4
run key 37 cmd; sleep .4; run keys "40"; sleep .6; shot h-goto
run key 36; sleep .8; shot i-goto-done
run drag 256 400 340 400; sleep .6; shot j-resized      # ツリーの幅を広げる
run click 290 54; sleep 1; shot k-first-tab            # 先頭のファイルのタブへ戻る
run key 13 cmd; sleep 1; shot l-closed                 # ⌘W
run key 11 cmd; sleep .8; shot m-no-sidebar            # ⌘B
run key 11 cmd; sleep .5
run key 18 cmd; sleep 1.4; shot n-map
kill $PID; wait $PID 2>/dev/null; rm -rf "$DATA"
echo done
