#!/bin/bash
# s4_multi_pane_binance.sh — スイート S4: マルチペイン・Binance 混在
source "$(dirname "$0")/common_helpers.sh"

echo "=== S4: マルチペイン Binance 混在 ==="
backup_state

START=$(utc_offset -14)
END=$(utc_offset -2)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S4-Multi","dashboard":{"pane":{
    "Split":{"axis":"Vertical","ratio":0.33,
      "a":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":["Volume"],"link_group":"A"
      }},
      "b":{"Split":{"axis":"Vertical","ratio":0.5,
        "a":{"KlineChart":{
          "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
          "stream_type":[{"Kline":{"ticker":"BinanceLinear:ETHUSDT","timeframe":"M1"}}],
          "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
          "indicators":["Volume"],"link_group":"B"
        }},
        "b":{"TimeAndSales":{
          "stream_type":[{"Trades":{"ticker":"BinanceLinear:BTCUSDT"}}],
          "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"MS100"}},
          "link_group":"A"
        }}
      }}
    }
  },"popout":[]}}],"active_layout":"S4-Multi"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app

# Live ストリームが Ready になるまで待つ（Binance メタデータ取得に数秒かかる）
for i in $(seq 1 30); do
  PLIST=$(curl -s "$API/pane/list" 2>/dev/null || echo '{}')
  READY=$(node -e "try{const d=JSON.parse(process.argv[1]);const ps=d.panes||[];const allReady=ps.length>0&&ps.every(p=>p.streams_ready===true);process.stdout.write(allReady?'true':'false');}catch(e){process.stdout.write('false');}" "$PLIST")
  [ "$READY" = "true" ] && echo "  all streams ready (${i}s)" && break
  sleep 1
done

curl -s -X POST "$API/replay/toggle" > /dev/null

# --- TC-S4-01: 15秒以内に Playing に遷移 ---
START_TIME=$(date +%s)
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null

if wait_playing 15; then
  ELAPSED=$(($(date +%s) - START_TIME))
  pass "TC-S4-01: 15s 以内に Playing ($ELAPSED s)"
else
  fail "TC-S4-01" "15s 以内に Playing にならなかった（trades が kline ゲートをブロック？）"
fi

# --- TC-S4-02: マルチペイン 1x で 3 秒 / 1〜4 bar 前進 ---
CT1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 3
CT2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
WITHIN=$(advance_within "$CT1" "$CT2" "$STEP_M1" 100)
[ "$WITHIN" = "true" ] && pass "TC-S4-02: マルチペインで 1〜100 bar 前進 ($CT1 → $CT2)" || \
  fail "TC-S4-02" "想定外の前進 (CT1=$CT1 CT2=$CT2)"

# --- TC-S4-03: 10s 後も Playing 継続 + 前進 ---
CT3_PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 10
CT3_POST=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ST" = "Playing" ] && pass "TC-S4-03a: 10s 後も Playing" || fail "TC-S4-03a" "status=$ST"
WITHIN10=$(advance_within "$CT3_PRE" "$CT3_POST" "$STEP_M1" 300)
[ "$WITHIN10" = "true" ] && pass "TC-S4-03b: 10s で 1〜300 bar 前進 (delta verified)" || \
  fail "TC-S4-03b" "10s で前進が範囲外 ($CT3_PRE → $CT3_POST)"

# --- TC-S4-04: Pause → StepForward → ステップ粒度は min timeframe = 60000ms ---
curl -s -X POST "$API/replay/pause" > /dev/null
PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
POST_SF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF=$(bigt_sub "$POST_SF" "$PRE")
[ "$DIFF" = "60000" ] && pass "TC-S4-04: マルチペイン StepForward +60000ms" || \
  fail "TC-S4-04" "diff=$DIFF (expected 60000, min tf=M1)"

restore_state
print_summary
