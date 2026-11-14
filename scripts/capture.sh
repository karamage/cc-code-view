#!/bin/bash
# 全状態を撮影する。使い方: scripts/capture.sh <出力先> [リポジトリ] [状態...]
set -uo pipefail
OUT=$1; REPO=${2:-../cc-auto-review}; shift; shift || true
STATES=("$@"); [ ${#STATES[@]} -eq 0 ] && STATES=(home-empty home fresh analyzing done blueprint blueprint-focus blueprint-running blueprint-fresh tour code search deep error unavailable)
BIN=${BIN:-target/release/cc-code-view}; DATA=target/demo-data; EMPTY=$(mktemp -d)
[ -x target/winid ] || swiftc -O scripts/winid.swift -o target/winid
mkdir -p "$OUT"
for STATE in "${STATES[@]}"; do
  # 未解析の状態は空のデータで開く
  case $STATE in fresh|analyzing|error|unavailable|blueprint-fresh) DIR=$EMPTY ;; *) DIR=$DATA ;; esac
  # 設計図の空の状態は、未解析のデータで設計図のタブを開く
  DEMO=$STATE; [ "$STATE" = blueprint-fresh ] && DEMO=blueprint
  if [ "$STATE" = home-empty ]; then
    "$BIN" --data-dir "$EMPTY/none" --demo home 2>/dev/null & PID=$!
  else
    "$BIN" --data-dir "$DIR" --open "$REPO" --demo "$DEMO" 2>/dev/null & PID=$!
  fi
  sleep 3.5
  WID=$(target/winid cc-code-view)
  screencapture -x -o -l "$WID" "$OUT/$STATE.png"
  kill "$PID"; wait "$PID" 2>/dev/null
done
rm -rf "$EMPTY"
