#!/usr/bin/env bash
# s27_cyclespeed_reset.sh — S27: CycleSpeed は速度のみ変更する（停止・シーク副作用なし）
#
# 検証シナリオ（仕様 R4-3-2「CycleSpeed 副作用除去」）:
#   TC-A: Playing 中に CycleSpeed (1x→2x) → status=Playing のまま・speed=2x・current_time 前進維持
#   TC-B: Playing 中に CycleSpeed (2x→5x) → status=Playing のまま・speed=5x
#   TC-C: Playing 中に CycleSpeed (5x→10x) → status=Playing のまま・speed=10x
#   TC-D: Playing 中に CycleSpeed (10x→1x ラップ) → status=Playing のまま・speed=1x
#   TC-E: Pause 後に CycleSpeed (1x→2x) → status=Paused のまま・speed=2x
#   TC-F: Paused 状態から Resume → Playing 到達
#
# 仕様根拠:
#   docs/replay_header.md §8.1 — R4-3-2「CycleSpeed 副作用除去」
#   旧仕様: CycleSpeed は pause + seek(range.start) を伴っていた
#   新仕様: CycleSpeed は speed ラベルのサイクルのみ。status・current_time に影響しない
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play (UTC[-3h, -1h])
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S27: CycleSpeed は速度のみ変更する（停止・シーク副作用なし） ==="
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

# 前準備: current_time が start_time より十分進んでいることを確認する
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
  echo "  WARN: current_time が 15s で十分に前進しなかった"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-A: Playing 中に CycleSpeed (1x→2x) → Playing のまま、speed=2x
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A: Playing 中に CycleSpeed (1x→2x) → Playing のまま"

CT_BEFORE_A=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
echo "  CycleSpeed 前 current_time=$CT_BEFORE_A status=Playing"

RESP_A=$(curl -s -X POST "$API/replay/speed")
SPEED_A=$(jqn "$RESP_A" "d.speed")
STATUS_A=$(jqn "$RESP_A" "d.status")
CT_A=$(jqn "$RESP_A" "d.current_time")
echo "  CycleSpeed 後: status=$STATUS_A speed=$SPEED_A current_time=$CT_A"

# TC-A1: status=Playing のまま（停止しない）
[ "$STATUS_A" = "Playing" ] \
  && pass "TC-A1: CycleSpeed 後 status=Playing（停止なし）" \
  || fail "TC-A1" "status=$STATUS_A (expected Playing — CycleSpeed が意図せず停止)"

# TC-A2: speed=2x
[ "$SPEED_A" = "2x" ] \
  && pass "TC-A2: CycleSpeed 後 speed=2x" \
  || fail "TC-A2" "speed=$SPEED_A (expected 2x)"

# TC-A3: current_time が start_time より前進したまま（range.start にリセットされない）
if [ "$CT_A" != "null" ] && [ -n "$CT_A" ]; then
  NOT_RESET=$(node -e "console.log(BigInt('$CT_A') > BigInt('$START_MS') + BigInt('60000'))")
  [ "$NOT_RESET" = "true" ] \
    && pass "TC-A3: current_time=$CT_A は start_time にリセットされていない" \
    || fail "TC-A3" "current_time=$CT_A は start_time=$START_MS から 1 bar 以内（不正リセット）"
else
  fail "TC-A3" "current_time が null"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-B: Playing 中に CycleSpeed (2x→5x) → Playing のまま
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-B: CycleSpeed (2x→5x) → Playing のまま"

RESP_B=$(curl -s -X POST "$API/replay/speed")
SPEED_B=$(jqn "$RESP_B" "d.speed")
STATUS_B=$(jqn "$RESP_B" "d.status")
echo "  CycleSpeed 後: status=$STATUS_B speed=$SPEED_B"

[ "$STATUS_B" = "Playing" ] \
  && pass "TC-B1: CycleSpeed (2x→5x) 後 status=Playing" \
  || fail "TC-B1" "status=$STATUS_B (expected Playing)"

[ "$SPEED_B" = "5x" ] \
  && pass "TC-B2: speed=5x" \
  || fail "TC-B2" "speed=$SPEED_B (expected 5x)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-C: Playing 中に CycleSpeed (5x→10x) → Playing のまま
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C: CycleSpeed (5x→10x) → Playing のまま"

RESP_C=$(curl -s -X POST "$API/replay/speed")
SPEED_C=$(jqn "$RESP_C" "d.speed")
STATUS_C=$(jqn "$RESP_C" "d.status")
echo "  CycleSpeed 後: status=$STATUS_C speed=$SPEED_C"

[ "$STATUS_C" = "Playing" ] \
  && pass "TC-C1: CycleSpeed (5x→10x) 後 status=Playing" \
  || fail "TC-C1" "status=$STATUS_C (expected Playing)"

[ "$SPEED_C" = "10x" ] \
  && pass "TC-C2: speed=10x" \
  || fail "TC-C2" "speed=$SPEED_C (expected 10x)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-D: Playing 中に CycleSpeed (10x→1x ラップ) → Playing のまま
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-D: CycleSpeed (10x→1x ラップ) → Playing のまま"

RESP_D=$(curl -s -X POST "$API/replay/speed")
SPEED_D=$(jqn "$RESP_D" "d.speed")
STATUS_D=$(jqn "$RESP_D" "d.status")
echo "  CycleSpeed 後: status=$STATUS_D speed=$SPEED_D"

[ "$STATUS_D" = "Playing" ] \
  && pass "TC-D1: CycleSpeed (10x→1x) 後 status=Playing" \
  || fail "TC-D1" "status=$STATUS_D (expected Playing)"

[ "$SPEED_D" = "1x" ] \
  && pass "TC-D2: speed=1x（ラップ確認）" \
  || fail "TC-D2" "speed=$SPEED_D (expected 1x)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-E: Paused 中に CycleSpeed → Paused のまま（再生開始しない）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-E: Paused 中に CycleSpeed → Paused のまま"

curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status "Paused" 10; then
  fail "TC-E-precond" "Paused に遷移せず"
else
  RESP_E=$(curl -s -X POST "$API/replay/speed")
  SPEED_E=$(jqn "$RESP_E" "d.speed")
  STATUS_E=$(jqn "$RESP_E" "d.status")
  echo "  CycleSpeed 後: status=$STATUS_E speed=$SPEED_E"

  [ "$STATUS_E" = "Paused" ] \
    && pass "TC-E1: Paused 中 CycleSpeed 後 status=Paused のまま" \
    || fail "TC-E1" "status=$STATUS_E (expected Paused)"

  [ "$SPEED_E" = "2x" ] \
    && pass "TC-E2: Paused 中 CycleSpeed 後 speed=2x" \
    || fail "TC-E2" "speed=$SPEED_E (expected 2x)"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-F: Paused → Resume → Playing
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-F: Paused → Resume → Playing"
curl -s -X POST "$API/replay/resume" > /dev/null
if wait_status "Playing" 30; then
  pass "TC-F: Resume 後 status=Playing"
else
  fail "TC-F" "status=$(jqn "$(curl -s "$API/replay/status")" "d.status") (expected Playing)"
fi

stop_app
print_summary
