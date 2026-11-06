#!/bin/bash
# 性能目標を実測する。使い方: scripts/measure.sh [大きなリポジトリ]
set -uo pipefail
BIN=target/release/cc-code-view
REPO=${1:-../cc-auto-review}
DATA=target/demo-data
[ -x target/winid ] || swiftc -O scripts/winid.swift -o target/winid

echo "== コールドスタート (ms, 5 回)"
for i in 1 2 3 4 5; do
  LOG=$(mktemp)
  "$BIN" --data-dir "$DATA" --demo home 2>"$LOG" & PID=$!
  for _ in $(seq 1 50); do grep -q CCV_READY "$LOG" && break; sleep 0.1; done
  grep CCV_READY "$LOG" | awk '{print $2}'
  kill "$PID"; wait "$PID" 2>/dev/null; rm -f "$LOG"
done | sort -n | tr '\n' ' '; echo

echo "== .app サイズ"
du -sh "target/CC Code View.app" 2>/dev/null || echo "scripts/bundle.sh を先に実行してください"

echo "== メモリ (done 状態、10 秒後)"
"$BIN" --data-dir "$DATA" --open "$REPO" --demo done 2>/dev/null & PID=$!
sleep 10; ps -o rss= -p "$PID" | awk '{printf "%.0f MB\n", $1/1024}'
echo "== 待機中の CPU (%, 5 秒間の累積 CPU 時間から)"
cpu() { ps -o time= -p "$PID" | awk -F'[:.]' '{print $1*60 + $2 + $3/100}'; }
A=$(cpu); sleep 5; B=$(cpu); awk -v a="$A" -v b="$B" 'BEGIN{printf "%.1f%%\n", (b-a)/5*100}'
kill "$PID"; wait "$PID" 2>/dev/null

echo "== フレーム時間 (連続ズームとパン、ms)"
LOG=$(mktemp)
CCV_FRAME_LOG=1 CCV_FRAME_ORBIT=1 "$BIN" --data-dir "$DATA" --open "$REPO" --demo fresh 2>"$LOG" & PID=$!
sleep 8; kill "$PID"; wait "$PID" 2>/dev/null
grep CCV_FRAME "$LOG" | awk 'NR>30{print $2}' | sort -n | awk '{a[NR]=$1} END{if(NR){printf "frames=%d p50=%.1f p95=%.1f p99=%.1f\n",NR,a[int(NR*.5)],a[int(NR*.95)],a[int(NR*.99)]}}'
rm -f "$LOG"
