#!/bin/bash
# s9_speed_step.sh — スイート S9: 再生速度・Step 精度
source "$(dirname "$0")/common_helpers.sh"

echo "=== S9: 再生速度・Step 精度 ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S9","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S9"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app
if ! wait_playing 30; then
  fail "TC-S9-precond" "auto-play で Playing に到達せず"
  restore_state
  print_summary
  exit 1
fi

# --- TC-S9-01: Speed サイクルの順序 (1x→2x→5x→10x→1x) ---
INIT_SPEED=$(jqn "$(curl -s "$API/replay/status")" "d.speed")
[ "$INIT_SPEED" = "1x" ] && pass "TC-S9-01a: 初期 speed=1x" || fail "TC-S9-01a" "speed=$INIT_SPEED"

for expected in "2x" "5x" "10x" "1x"; do
  SPEED=$(jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed")
  [ "$SPEED" = "$expected" ] && pass "TC-S9-01b: speed cycle → $SPEED" || \
    fail "TC-S9-01b" "expected=$expected got=$SPEED"
done

# --- TC-S9-02: 5x 速度で wall delay が概ね 200ms/bar ---
curl -s -X POST "$API/replay/pause" > /dev/null
# 1x → 2x → 5x
jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed" > /dev/null  # 2x
jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed" > /dev/null  # 5x
SP=$(jqn "$(curl -s "$API/replay/status")" "d.speed")
[ "$SP" = "5x" ] || fail "TC-S9-02-precond" "speed=$SP (expected 5x)"
CT_INIT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/resume" > /dev/null
sleep 5
curl -s -X POST "$API/replay/pause" > /dev/null
CT_END=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DELTA=$(bigt_sub "$CT_END" "$CT_INIT")
BARS=$(node -e "console.log(String(BigInt('$DELTA') / BigInt('$STEP_M1')))")
[[ $BARS -ge 15 && $BARS -le 35 ]] && pass "TC-S9-02: 5x で 5 秒に ${BARS} bar 前進" || \
  fail "TC-S9-02" "${BARS} bar (expected 15-35, delta=$DELTA)"

# --- TC-S9-03: Playing 中の StepForward の挙動を確定 ---
curl -s -X POST "$API/replay/resume" > /dev/null
PRE_PLAYING=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
POST_PLAYING=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DELTA_P=$(bigt_sub "$POST_PLAYING" "$PRE_PLAYING")
set +e
node -e "process.exit(BigInt('$DELTA_P') > BigInt('$STEP_M1') ? 1 : 0)"
RC=$?
set -e
[ $RC -eq 0 ] && pass "TC-S9-03: Playing 中 StepForward は no-op (delta=$DELTA_P)" || \
  fail "TC-S9-03" "Playing 中 Step が ${DELTA_P}ms 進めた（仕様違反）"

# --- TC-S9-04: StepBackward を連続 5 回 → 単調減少 ---
curl -s -X POST "$API/replay/pause" > /dev/null
for i in $(seq 1 5); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 0.3
done
TIMES=()
for i in $(seq 1 5); do
  T=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  TIMES+=("$T")
  curl -s -X POST "$API/replay/step-backward" > /dev/null
  sleep 0.3
done
MONOTONE="true"
for i in $(seq 1 4); do
  A="${TIMES[$i]}"
  B="${TIMES[$((i-1))]}"
  [ -n "$A" ] && [ -n "$B" ] || continue
  GT=$(bigt_gt "$B" "$A")
  [ "$GT" = "true" ] || MONOTONE="false"
done
[ "$MONOTONE" = "true" ] && pass "TC-S9-04: StepBackward 連続 5 回 単調減少" || \
  fail "TC-S9-04" "単調減少でない times=${TIMES[*]}"

restore_state
print_summary
