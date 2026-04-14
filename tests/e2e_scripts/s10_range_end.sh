#!/bin/bash
# s10_range_end.sh — スイート S10: 範囲端・終端到達
source "$(dirname "$0")/common_helpers.sh"

echo "=== S10: 範囲端・終端到達 ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)
END_MS=$(node -e "console.log(new Date('${END}:00Z').getTime())")

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S10","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S10"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app
if ! wait_playing 30; then
  fail "TC-S10-precond" "auto-play で Playing に到達せず"
  restore_state
  print_summary
  exit 1
fi

# --- TC-S10-01: 速度を 10x にして終端まで再生 ---
for s in "2x" "5x" "10x"; do
  jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed" > /dev/null
done
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
# 10x 速度のまま。終端から 15 バー後退して余裕を持たせてから Resume
# (10x=100ms/bar → 15 bars=1.5s、0.5s 後チェック時は Playing 継続のはず)
for _ in $(seq 1 14); do curl -s -X POST "$API/replay/step-backward" > /dev/null; done
sleep 0.2
curl -s -X POST "$API/replay/resume" > /dev/null
sleep 0.4
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

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S10-Tiny","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S10-Tiny"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$TINY_START","range_end":"$TINY_END"}
}
EOF

start_app
if wait_playing 30; then
  pass "TC-S10-05: 2 分 range でも Playing 開始"
  if wait_paused 60; then
    pass "TC-S10-05b: 小 range で終端到達 → Paused"
  else
    fail "TC-S10-05b" "終端到達しなかった"
  fi
else
  fail "TC-S10-05" "2 分 range で Playing にならなかった"
fi

restore_state
print_summary
