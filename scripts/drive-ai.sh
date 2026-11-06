#!/bin/bash
# 実際の claude を使って、解析から深掘りまでを画面からたどる。利用枠を消費する。
set -uo pipefail
OUT=$1; REPO=$2; mkdir -p "$OUT"
DATA=$(mktemp -d)
printf '{"repos":[],"last_model":"Haiku"}' > "$DATA/settings.json"
target/release/cc-code-view --data-dir "$DATA" --open "$REPO" 2>"$OUT/stderr.log" & PID=$!
sleep 3
P="target/poke cc-code-view"
shot() { screencapture -x -o -l "$(target/winid cc-code-view)" "$OUT/$1.png"; }
send() { $P "$@" || { echo "ABORT: $*"; kill $PID; exit 2; }; }
send click 640 22; sleep .3; shot 1-fresh
send key 36; sleep 20; shot 2-running   # Enter
for i in $(seq 1 36); do
  ls "$DATA"/repos/*/analysis.json >/dev/null 2>&1 && break
  sleep 5
done
ls "$DATA"/repos/*/analysis.json >/dev/null 2>&1 || { echo "解析が完了しませんでした"; shot 9-failed; pgrep -fl "claude -p" | cut -c1-80; cat "$OUT/stderr.log"; ls -R "$DATA" | head; kill $PID; rm -rf "$DATA"; exit 1; }
sleep .6; shot 3-morphing; sleep 2.5; shot 4-done
# 星座を押して深掘りを実行する。座標は星図の中央付近
send click 640 300; sleep 1.6; shot 5-panel
send click 170 ${DEEP_BUTTON_Y:-200}; sleep 25; shot 6-deep-running
for i in $(seq 1 24); do
  find "$DATA/repos" -path '*deep*' -name '*.json' | grep -q . && break
  sleep 5
done
sleep 1; shot 7-deep-done
find "$DATA/repos" -path '*deep*' -name '*.json' | wc -l | xargs echo deep_files=
python3 -c "
import json,glob
a=json.load(open(glob.glob('$DATA/repos/*/analysis.json')[0]))
print('model',a['model'],'modules',len(a['modules']),'tour',len(a['tour']))
print(a['overview'][:120])"
kill $PID; wait $PID 2>/dev/null
rm -rf "$DATA"
