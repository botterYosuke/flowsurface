#!/bin/bash
# s10_range_end.sh — スイート S10: 範囲端・終端到達
#
# 検証シナリオ:
#   TC-S10-01: 10x 速度で終端到達 → 自動 Paused（最大 300s 待機）
#   TC-S10-02: 終端到達後 StepForward は no-op
#   TC-S10-03: 終端から StepBackward で戻れる
#   TC-S10-04: 終端付近から Resume → Playing
#   TC-S10-05: 2 分幅の最小 range で Playing/終端 Paused 到達
#
# 仕様根拠:
#   docs/replay_header.md §6.4 — range end 到達時の自動 Pause・終端クランプ
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play (UTC[-3h, -1h]) + 2 分 range パターン
source "$(dirname "$0")/common_helpers.sh"

echo "=== S10: 範囲端・終端到達 ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)
END_MS=$(node -e "console.log(new Date('${END}:00Z').getTime())")

setup_single_pane "$E2E_TICKER" "M1" "$START" "$END"

start_app
headless_play
if ! wait_playing 30; then
  fail "TC-S10-precond" "auto-play で Playing に到達せず"
  restore_state
  print_summary
  exit 1
fi

# --- TC-S10-01: 速度を 10x にして終端まで再生 ---
# 新仕様: CycleSpeed は pause + seek(range.start) を伴う。速度変更後に Resume が必要。
for s in "2x" "5x" "10x"; do
  jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed" > /dev/null
done
curl -s -X POST "$API/replay/resume" > /dev/null
wait_status Playing 10 || true
echo "  10x 速度で終端まで待機（最大 300s）..."

REACHED_END="false"
for i in $(seq 1 300); do
  STATUS=$(curl -s "$API/replay/status")
  CT=$(jqn "$STATUS" "d.current_time")
  ST=$(jqn "$STATUS" "d.status")
  if [ "$ST" = "Paused" ]; then
    NEAR_END=$(node -e "console.log(BigInt('$CT') >= BigInt('$END_MS') - BigInt('120000'))")
    [ "$NEAR_END" = "true" ] && REACHED_END="true"
    break
  fi
  sleep 1
done
[ "$REACHED_END" = "true" ] && pass "TC-S10-01: 終端到達で自動 Paused" || \
  fail "TC-S10-01" "終端到達しなかった or Paused にならなかった"

# --- TC-S10-02: 終端到達後 StepForward は完全 no-op ---
CT_AT_END=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
CT_AFTER_SF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
EQ=$(bigt_eq "$CT_AT_END" "$CT_AFTER_SF")
[ "$EQ" = "true" ] && pass "TC-S10-02: 終端後 StepForward は no-op" || \
  fail "TC-S10-02" "終端後 StepForward が前進 (before=$CT_AT_END after=$CT_AFTER_SF)"

# --- TC-S10-03: 終端から StepBackward で戻れる ---
curl -s -X POST "$API/replay/step-backward" > /dev/null
sleep 1
CT_BACK=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
IS_BACK=$(bigt_gt "$CT_AT_END" "$CT_BACK")
[ "$IS_BACK" = "true" ] && pass "TC-S10-03: 終端から StepBackward 可能" || \
  fail "TC-S10-03" "後退しない (end=$CT_AT_END back=$CT_BACK)"

# --- TC-S10-04: Resume で再び Playing になる ---
# BASE_STEP_DELAY_MS=100ms / 10x = 10ms/bar。
# 60 バー後退 (600ms) して Resume し、即座に status を確認。
for _ in $(seq 1 59); do curl -s -X POST "$API/replay/step-backward" > /dev/null; done
curl -s -X POST "$API/replay/resume" > /dev/null
# 10ms/bar × 60 bars = 600ms の余裕。100ms 以内にチェック
ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ST" = "Playing" ] && pass "TC-S10-04: StepBackward 後に Resume → Playing" || \
  fail "TC-S10-04" "status=$ST"

# --- TC-S10-05: 2 分幅のレンジ（最小動作確認） ---
stop_app
TINY_START=$(utc_offset -2)
TINY_END=$(node -e "
  const d = new Date('${TINY_START}:00Z');
  d.setMinutes(d.getMinutes() + 2);
  const pad = n => String(n).padStart(2,'0');
  console.log(d.getUTCFullYear()+'-'+pad(d.getUTCMonth()+1)+'-'+pad(d.getUTCDate())+' '+pad(d.getUTCHours())+':'+pad(d.getUTCMinutes()));
")

setup_single_pane "$E2E_TICKER" "M1" "$TINY_START" "$TINY_END"

start_app
headless_play
# 2 分 range (2 bars) は BASE_STEP_DELAY_MS=100ms/1x では 200ms で完走する。
# wait_playing (1s ポーリング) では捕捉できないため、Paused 終端も合格条件とする。
TINY_END_MS=$(node -e "console.log(new Date('${TINY_END}:00Z').getTime())")
TC05_OK=false
for i in $(seq 1 30); do
  RESP=$(curl -s "$API/replay/status")
  ST05=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.status||'null');}catch(e){console.log('null');}" "$RESP")
  CT05=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.current_time||'0');}catch(e){console.log('0');}" "$RESP")
  if [ "$ST05" = "Playing" ]; then TC05_OK=true; break; fi
  # Paused かつ終端近く（高速完走）も合格
  NEAR=$(node -e "console.log(BigInt('$CT05') >= BigInt('$TINY_END_MS') - BigInt('120000'))" 2>/dev/null || echo "false")
  if [ "$ST05" = "Paused" ] && [ "$NEAR" = "true" ]; then TC05_OK=true; break; fi
  sleep 1
done
if $TC05_OK; then
  pass "TC-S10-05: 2 分 range で Playing/終端 Paused 到達 (status=$ST05)"
  # Playing だった場合は Paused 終端も確認
  if [ "$ST05" = "Playing" ]; then
    if wait_paused 60; then
      pass "TC-S10-05b: 小 range で終端到達 → Paused"
    else
      fail "TC-S10-05b" "終端到達しなかった"
    fi
  fi
else
  fail "TC-S10-05" "2 分 range で Playing/終端 Paused にならなかった"
fi

restore_state
print_summary
