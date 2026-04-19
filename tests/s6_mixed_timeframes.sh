#!/bin/bash
# s6_mixed_timeframes.sh — スイート S6: 異なる時間軸混在
#
# 検証シナリオ:
#   TC-S6-01: BTCUSDT M1+M5+H1 混在（3ペイン）で 60s 以内 Playing
#   TC-S6-02: step_size = min tf = M1 = 60000ms
#   TC-S6-03: M5/H1 疎 step でもクラッシュなし・status=Paused
#   TC-S6-04: M5 単独構成 → step_size = M5 = 300000ms
#
# 仕様根拠:
#   docs/replay_header.md §7.3 — min_step_size = min(timeframes)
#
# フィクスチャ: BinanceLinear:BTCUSDT M1+M5+H1（3ペイン）+ M5 単独の 2パターン
#   Live モード起動 → 手動 toggle/play
source "$(dirname "$0")/common_helpers.sh"

# CI US IP から Binance が到達不能の場合は PEND 扱いでスキップ（HTTP 451/403）
BINANCE_PROBE=$(curl -s -o /dev/null -w "%{http_code}" --max-time 10 "https://fapi.binance.com/fapi/v1/ping" 2>/dev/null || echo "000")
if [ "$BINANCE_PROBE" != "200" ]; then
  echo "  PEND: Binance unreachable (HTTP $BINANCE_PROBE) — S6 は Binance IP ブロック環境では実行不可"
  exit 0
fi

echo "=== S6: 異なる時間軸混在 ==="
backup_state

START=$(utc_offset -6)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S6-MixedTF","dashboard":{"pane":{
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
          "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M5"}}],
          "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M5"}},
          "indicators":["Volume"],"link_group":"A"
        }},
        "b":{"KlineChart":{
          "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
          "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"H1"}}],
          "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"H1"}},
          "indicators":["Volume"],"link_group":"A"
        }}
      }}
    }
  },"popout":[]}}],"active_layout":"S6-MixedTF"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app
for i in $(seq 1 30); do
  PLIST=$(curl -s "$API/pane/list" 2>/dev/null || echo '{}')
  READY=$(node -e "try{const d=JSON.parse(process.argv[1]);const ps=d.panes||[];const allReady=ps.length>0&&ps.every(p=>p.streams_ready===true);process.stdout.write(allReady?'true':'false');}catch(e){process.stdout.write('false');}" "$PLIST")
  [ "$READY" = "true" ] && echo "  all streams ready (${i}s)" && break
  sleep 1
done
curl -s -X POST "$API/replay/toggle" > /dev/null

# --- TC-S6-01: Play → Playing ---
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null

if wait_playing 60; then
  pass "TC-S6-01: M1+M5+H1 混在 → Playing"
else
  fail "TC-S6-01" "60s 以内に Playing にならなかった"
fi

# --- TC-S6-02: step_size は min_tf = M1 = 60000ms ---
curl -s -X POST "$API/replay/pause" > /dev/null
PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
POST_SF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF=$(bigt_sub "$POST_SF" "$PRE")
[ "$DIFF" = "60000" ] && pass "TC-S6-02: step_size=60000ms (M1 が最小 tf)" || \
  fail "TC-S6-02" "diff=$DIFF (expected 60000)"

# --- TC-S6-03: M5 と H1 は kline が疎になる（1 step で kline なしも正常） ---
for i in $(seq 1 5); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 0.5
done
ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ST" = "Paused" ] && pass "TC-S6-03: M5/H1 疎 step でもクラッシュなし" || \
  fail "TC-S6-03" "status=$ST (expected Paused)"

# --- TC-S6-04: M5 ペインのみ構成（step_size が M5 = 300000ms）---
stop_app
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S6-M5Only","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M5"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M5"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S6-M5Only"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app
for i in $(seq 1 30); do
  PLIST=$(curl -s "$API/pane/list" 2>/dev/null || echo '{}')
  READY=$(node -e "try{const d=JSON.parse(process.argv[1]);const ps=d.panes||[];const allReady=ps.length>0&&ps.every(p=>p.streams_ready===true);process.stdout.write(allReady?'true':'false');}catch(e){process.stdout.write('false');}" "$PLIST")
  [ "$READY" = "true" ] && echo "  all streams ready (${i}s)" && break
  sleep 1
done
curl -s -X POST "$API/replay/toggle" > /dev/null
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null
if ! wait_playing 60; then
  fail "TC-S6-04-precond" "M5 単独構成で Playing 到達せず"
  restore_state
  print_summary
  exit 1
fi
curl -s -X POST "$API/replay/pause" > /dev/null
PRE2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
POST2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF2=$(bigt_sub "$POST2" "$PRE2")
[ "$DIFF2" = "300000" ] && pass "TC-S6-04: M5 単独 → step=300000ms" || \
  fail "TC-S6-04" "diff=$DIFF2 (expected 300000)"

restore_state
print_summary
