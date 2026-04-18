#!/bin/bash
# s9_speed_step.sh — スイート S9: 再生速度・Step 精度
#
# 検証シナリオ:
#   TC-S9-01a〜b: Speed サイクル順序（1x→2x→5x→10x→1x）
#   TC-S9-02: 5x 速度で 5 秒に 1〜500 bar 前進
#   TC-S9-03a〜b: Playing 中 StepForward → Paused・End 近傍到達
#   TC-S9-04: StepBackward 連続 5 回 → 単調減少
#
# 仕様根拠:
#   docs/replay_header.md §8 — 速度制御・CycleSpeed, §6 — StepForward/StepBackward
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play (UTC[-3h, -1h])
source "$(dirname "$0")/common_helpers.sh"

echo "=== S9: 再生速度・Step 精度 ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)

setup_single_pane "$E2E_TICKER" "M1" "$START" "$END"

start_app
headless_play
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
if CT_TICK=$(wait_for_time_advance "$CT_INIT" 30); then
  curl -s -X POST "$API/replay/pause" > /dev/null
  wait_status Paused 10 || true
  CT_END=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  DELTA=$(bigt_sub "$CT_END" "$CT_INIT")
  BARS=$(node -e "console.log(String(BigInt('$DELTA') / BigInt('$STEP_M1')))")
  [[ $BARS -ge 1 && $BARS -le 500 ]] && pass "TC-S9-02: 5x で ${BARS} bar 前進" || \
    fail "TC-S9-02" "${BARS} bar (expected 1-500, delta=$DELTA)"
else
  curl -s -X POST "$API/replay/pause" > /dev/null
  fail "TC-S9-02" "30 秒待機しても current_time が前進しなかった (CT_INIT=$CT_INIT)"
fi

# --- TC-S9-03: Playing 中の StepForward は End まで一気に進んで Paused になる ---
curl -s -X POST "$API/replay/resume" > /dev/null
sleep 0.3
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 0.3
STATUS_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.status")
CT_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
END_TIME_MS=$(node -e "console.log(new Date('${END}:00Z').getTime())")
[ "$STATUS_AFTER" = "Paused" ] \
  && pass "TC-S9-03a: Playing 中 StepForward → Paused" \
  || fail "TC-S9-03a" "status=$STATUS_AFTER (expected Paused)"
IS_AT_END=$(node -e "console.log(BigInt('$CT_AFTER') >= BigInt('$END_TIME_MS') - BigInt('120000'))")
[ "$IS_AT_END" = "true" ] \
  && pass "TC-S9-03b: Playing 中 StepForward → End 近傍到達 (ct=$CT_AFTER)" \
  || fail "TC-S9-03b" "ct=$CT_AFTER not near end=$END_TIME_MS"

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
