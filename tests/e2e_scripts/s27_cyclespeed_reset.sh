#!/usr/bin/env bash
# s27_cyclespeed_reset.sh — S27: CycleSpeed 後に current_time が range.start にリセットされること
#
# 検証シナリオ（仕様 §6.6「ユーザー操作による初期状態リセット」CycleSpeed 部分）:
#
#   CycleSpeed / StartTimeChanged / EndTimeChanged は同一のリセットコードパスを共有する。
#   本テストは CycleSpeed でそのパスを検証し、共通フローを間接的にカバーする。
#
#   TC-A: Playing 中 (current_time が start_time より進んだ状態) に CycleSpeed
#         → status=Paused かつ current_time≈start_time かつ speed=2x
#   TC-B: Paused 状態 (TC-A 後) から Resume → status=Playing に戻れること
#   TC-C: Playing 中 (speed=2x) に再度 CycleSpeed (2x→5x)
#         → status=Paused かつ current_time≈start_time かつ speed=5x
#   TC-D: Paused 状態 (TC-C 後) から Resume → status=Playing に戻れること
#
# 既存 s9 との差分:
#   s9 は speed label のサイクルのみ検証。本テストは「リセット後の current_time≈start_time」を明示確認する。
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, UTC[-3h, -1h] (auto-play)
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S27: CycleSpeed 後に current_time が range.start にリセットされること ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── フィクスチャ ──────────────────────────────────────────────────────────────
START=$(utc_offset -3)
END=$(utc_offset -1)
START_MS=$(node -e "console.log(new Date('${START}:00Z').getTime())")

echo "  range: $START → $END (start_ms=$START_MS)"

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S27","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S27"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app

# Playing に到達するまで待機（最大 60 秒）
if ! wait_status "Playing" 60; then
  diagnose_playing_failure
  fail "precond" "auto-play で Playing に到達せず"
  print_summary
  exit 1
fi
echo "  Playing 到達"

# ヘルパー: current_time が start_time の ±1 バー (60s) 以内か確認
is_near_start() {
  local ct="$1"
  node -e "
    const ct  = BigInt('$ct');
    const st  = BigInt('$START_MS');
    const tol = BigInt('60000'); // 1 bar
    const diff = ct > st ? ct - st : st - ct;
    console.log(diff <= tol ? 'true' : 'false');
  "
}

# ─────────────────────────────────────────────────────────────────────────────
# 前準備: current_time が start_time より十分進んでいることを確認する
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── 前準備: current_time が start_time より前進するまで待機 (最大 15s)"
CT_ADVANCED="false"
for i in $(seq 1 15); do
  CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  if [ "$CT" != "null" ] && [ -n "$CT" ]; then
    ADVANCE=$(node -e "console.log(BigInt('$CT') > BigInt('$START_MS') + BigInt('60000'))")
    if [ "$ADVANCE" = "true" ]; then
      CT_ADVANCED="true"
      echo "  current_time 前進確認: $CT (start_ms=$START_MS)"
      break
    fi
  fi
  sleep 1
done

if [ "$CT_ADVANCED" != "true" ]; then
  echo "  WARN: current_time が 15s で十分に前進しなかった。リセット検証に影響する可能性あり"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-A: Playing 中に CycleSpeed → Paused + current_time≈start_time + speed=2x
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A: Playing 中に CycleSpeed → Paused + current_time≈start_time"

CT_BEFORE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
echo "  CycleSpeed 前 current_time=$CT_BEFORE"

RESP=$(curl -s -X POST "$API/replay/speed")
SPEED_AFTER=$(jqn "$RESP" "d.speed")
STATUS_AFTER=$(jqn "$RESP" "d.status")
CT_AFTER=$(jqn "$RESP" "d.current_time")
echo "  CycleSpeed 後: status=$STATUS_AFTER speed=$SPEED_AFTER current_time=$CT_AFTER"

# TC-A1: status=Paused
[ "$STATUS_AFTER" = "Paused" ] \
  && pass "TC-A1: CycleSpeed 後 status=Paused" \
  || fail "TC-A1" "status=$STATUS_AFTER (expected Paused)"

# TC-A2: speed=2x (1x→2x)
[ "$SPEED_AFTER" = "2x" ] \
  && pass "TC-A2: CycleSpeed 後 speed=2x" \
  || fail "TC-A2" "speed=$SPEED_AFTER (expected 2x)"

# TC-A3: current_time≈start_time (±1 bar)
if [ "$CT_AFTER" != "null" ] && [ -n "$CT_AFTER" ]; then
  IS_NEAR=$(is_near_start "$CT_AFTER")
  [ "$IS_NEAR" = "true" ] \
    && pass "TC-A3: CycleSpeed 後 current_time≈start_time ($CT_AFTER ≈ $START_MS)" \
    || fail "TC-A3" "current_time=$CT_AFTER は start_time=$START_MS から 1 bar 以上離れている（リセット未発生）"
else
  fail "TC-A3" "current_time が null (status=$STATUS_AFTER)"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-B: Resume → Playing
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-B: Paused → Resume → Playing"
curl -s -X POST "$API/replay/resume" > /dev/null
if wait_status "Playing" 30; then
  pass "TC-B: Resume 後 status=Playing"
else
  fail "TC-B" "status=$(jqn "$(curl -s "$API/replay/status")" "d.status") (expected Playing)"
fi

# Playing に戻った後、current_time が再び前進するのを待つ
sleep 3

# ─────────────────────────────────────────────────────────────────────────────
# TC-C: Playing 中 (speed=2x) に 2 回目の CycleSpeed (2x→5x)
#        → Paused + current_time≈start_time
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C: Playing 中 (speed=2x) に 2 回目の CycleSpeed → Paused + current_time≈start_time"

CT_BEFORE_C=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
echo "  2 回目 CycleSpeed 前 current_time=$CT_BEFORE_C"

RESP_C=$(curl -s -X POST "$API/replay/speed")
SPEED_C=$(jqn "$RESP_C" "d.speed")
STATUS_C=$(jqn "$RESP_C" "d.status")
CT_C=$(jqn "$RESP_C" "d.current_time")
echo "  2 回目 CycleSpeed 後: status=$STATUS_C speed=$SPEED_C current_time=$CT_C"

[ "$STATUS_C" = "Paused" ] \
  && pass "TC-C1: 2 回目 CycleSpeed 後 status=Paused" \
  || fail "TC-C1" "status=$STATUS_C (expected Paused)"

[ "$SPEED_C" = "5x" ] \
  && pass "TC-C2: 2 回目 CycleSpeed 後 speed=5x" \
  || fail "TC-C2" "speed=$SPEED_C (expected 5x)"

if [ "$CT_C" != "null" ] && [ -n "$CT_C" ]; then
  IS_NEAR_C=$(is_near_start "$CT_C")
  [ "$IS_NEAR_C" = "true" ] \
    && pass "TC-C3: 2 回目 CycleSpeed 後 current_time≈start_time ($CT_C ≈ $START_MS)" \
    || fail "TC-C3" "current_time=$CT_C は start_time=$START_MS から 1 bar 以上離れている"
else
  fail "TC-C3" "current_time が null"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-D: Resume → Playing
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-D: 2 回目 CycleSpeed 後 Resume → Playing"
curl -s -X POST "$API/replay/resume" > /dev/null
if wait_status "Playing" 30; then
  pass "TC-D: 2 回目 CycleSpeed 後 Resume → Playing 到達"
else
  fail "TC-D" "status=$(jqn "$(curl -s "$API/replay/status")" "d.status") (expected Playing)"
fi

stop_app
print_summary
