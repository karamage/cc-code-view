#!/bin/bash
# 実際の入力で主な操作をたどり、途中を撮影する。使い方: scripts/drive.sh <出力先> <リポジトリ>
set -uo pipefail
OUT=$1; REPO=$2; mkdir -p "$OUT"
DATA=$(mktemp -d); cp -R target/demo-data/. "$DATA"/
target/release/cc-code-view --data-dir "$DATA" --open "$REPO" 2>"$OUT/stderr.log" & PID=$!
sleep 2.5
P="target/poke cc-code-view"
shot() { screencapture -x -o -l "$(target/winid cc-code-view)" "$OUT/$1.png"; }
send() { $P "$@" || { echo "ABORT: $*"; kill $PID; exit 2; }; }
send click 640 44; sleep .2
send key 35 cmd; sleep .4                 # ⌘P
keys() { for k in "$@"; do send key "$k"; done; }
keys 46 0 34 45; sleep .4; shot a-files  # m a i n
send key 36; sleep 1.5; shot b-opened    # Enter
send scroll -400 900 400; sleep .4; shot c-scrolled
send key 33 cmd; sleep 1; shot d-back
send key 53; sleep 1.3; shot e-closed      # esc
send key 3 cmd shift; sleep .4; keys 4 31 45 31; sleep 1; shot f-search  # h o n o
send key 53; sleep .3
send key 17; sleep 1.3; shot g-tour; send key 124; sleep 1.3; shot h-tour-next; send key 123; sleep 1.2
send key 53; sleep .2; send key 53; sleep 1.2; shot i-tour-exit
send scroll -300 700 400; sleep .3; shot j-panned
ps -o rss= -p $PID | awk '{printf "RSS %.0f MB\n", $1/1024}'
kill $PID; wait $PID 2>/dev/null; rm -rf "$DATA"
