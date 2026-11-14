#!/bin/bash
# 設計図の図を実際の入力で触る。使い方: scripts/drive-blueprint.sh <出力先> <リポジトリ> [x y]
# 実際のマウスとキーの入力を送るので、実行中は Mac に触らない。窓が最前面でなければ何も送らずに止まる
# 既定の座標は、1280×800 の窓でデータフローの図を拡大したときの要素の位置
set -uo pipefail
O=$1; REPO=$2; X=${3:-553}; Y=${4:-300}
rm -rf "$O"; mkdir -p "$O"
BIN=${BIN:-target/release/cc-code-view}
DATA=$(mktemp -d); cp -R target/demo-data/. "$DATA"/
CCV_FOCUS=${CCV_FOCUS:-2,0} "$BIN" --data-dir "$DATA" --open "$REPO" --demo blueprint-focus 2>/dev/null & PID=$!
sleep 4
W=$(target/winid cc-code-view)
shot() { screencapture -x -o -l "$W" "$O/$1.png"; }
run() { target/poke cc-code-view "$@" || { echo "ABORT $*"; kill $PID; rm -rf "$DATA"; exit 2; }; }
run click 330 640; sleep .5                         # 図の空いた所を押して窓を入力の対象にする
shot a-before
run move "$X" "$Y"; sleep .9; shot b-hover          # 触れた要素とつながる線が灯り、説明が出る
run click "$X" "$Y"; sleep 1.4; shot c-opened       # 押すとそのファイルを開く
run key 19 cmd; sleep 1; shot d-back                # ⌘2 で設計図へ戻る
run key 53; sleep .6; shot e-closed                 # esc で拡大を閉じる
kill $PID; wait $PID 2>/dev/null; rm -rf "$DATA"
echo done
