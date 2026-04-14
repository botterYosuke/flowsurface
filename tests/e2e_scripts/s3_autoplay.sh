#!/bin/bash
# s3_autoplay.sh — スイート S3: 起動時 Auto-play
source "$(dirname "$0")/common_helpers.sh"

echo "=== S3: Auto-play (Fixture 直接起動) ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)
START_MS=$(node -e "console.log(new Date('${START}:00Z').getTime())")
END_MS=$(node -e "console.log(new Date('${END}:00Z').getTime())")

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S3-AutoPlay","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S3-AutoPlay"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app

# --- TC-S3-01: 手動 toggle / play なしで Playing になる（最大 30s） ---
if wait_playing 30; then
  pass "TC-S3-01: auto-play → Playing（sleep 15 不要）"
else
  fail "TC-S3-01" "30s 以内に Playing にならなかった（streams 解決失敗？）"
fi

STATUS=$(curl -s "$API/replay/status")

# --- TC-S3-02: current_time が range 内 ---
CT=$(jqn "$STATUS" "d.current_time")
IN_RANGE=$(node -e "console.log(BigInt('$CT') >= BigInt('$START_MS') && BigInt('$CT') <= BigInt('$END_MS'))")
[ "$IN_RANGE" = "true" ] && pass "TC-S3-02: current_time in range" || \
  fail "TC-S3-02" "CT=$CT range=[$START_MS,$END_MS]"

# --- TC-S3-03: mode=Replay ---
MODE=$(jqn "$STATUS" "d.mode")
[ "$MODE" = "Replay" ] && pass "TC-S3-03: mode=Replay" || fail "TC-S3-03" "mode=$MODE"

# --- TC-S3-04: Pause → StepForward → diff=60000ms ---
curl -s -X POST "$API/replay/pause" > /dev/null
sleep 1
PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
POST_SF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF=$(bigt_sub "$POST_SF" "$PRE")
[ "$DIFF" = "60000" ] && pass "TC-S3-04: StepForward +60000ms" || \
  fail "TC-S3-04" "diff=$DIFF (expected 60000)"

# --- TC-S3-05: range_start が空文字のとき auto-play しない ---
stop_app
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S3-NoAutoPlay","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S3-NoAutoPlay"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"","range_end":""}
}
EOF

start_app
sleep 10

# --- TC-S3-05a: range 未設定 → auto-play しない & status=null ---
ST_CHECK=$(jqn "$(curl -s "$API/replay/status")" "d.status")
MODE_CHECK=$(jqn "$(curl -s "$API/replay/status")" "d.mode")
[ "$ST_CHECK" = "null" ] && pass "TC-S3-05a: range 未設定 → status=null" || \
  fail "TC-S3-05a" "status=$ST_CHECK (expected null)"
[ "$MODE_CHECK" = "Replay" ] && pass "TC-S3-05b: range 未設定でも mode は fixture 通り" || \
  fail "TC-S3-05b" "mode=$MODE_CHECK"

# --- TC-S3-05c: トーストに auto-play 起動エラーが無いこと ---
NOTIF=$(list_notifications)
ERR_COUNT=$(node -e "
  const d=JSON.parse(process.argv[1]);
  const e=(d.notifications||[]).filter(t=>t.level==='error'||t.level==='warning');
  console.log(e.length);
" "$NOTIF")
[ "$ERR_COUNT" = "0" ] && pass "TC-S3-05c: error/warning toast なし" || \
  fail "TC-S3-05c" "error/warning toast が $ERR_COUNT 件発火: $NOTIF"

restore_state
print_summary
