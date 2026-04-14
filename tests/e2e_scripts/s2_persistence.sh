#!/bin/bash
# s2_persistence.sh — スイート S2: 永続化往復テスト
source "$(dirname "$0")/common_helpers.sh"

echo "=== S2: 永続化往復テスト ==="
backup_state

START=$(utc_offset -4)
END=$(utc_offset -1)

# --- TC-S2-01: replay フィールドなしで起動（後方互換） ---
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S2","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S2"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app
STATUS=$(curl -s "$API/replay/status")
MODE=$(jqn "$STATUS" "d.mode")
RS=$(jqn "$STATUS" "d.range_start")
[ "$MODE" = "Live" ] && pass "TC-S2-01: replay なし → mode=Live" || fail "TC-S2-01" "mode=$MODE"
[ "$RS" = "" ] && pass "TC-S2-01b: range_start 空" || fail "TC-S2-01b" "range_start=$RS"
stop_app

# --- TC-S2-02: Replay モードで保存 → 再起動で復元 ---
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S2","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S2"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app
ST=$(curl -s "$API/replay/status")
for i in $(seq 1 30); do
  ST=$(curl -s "$API/replay/status")
  PSTATUS=$(jqn "$ST" "d.status")
  [[ "$PSTATUS" = "Playing" || "$PSTATUS" = "null" ]] && break
  sleep 1
done
MODE2=$(jqn "$ST" "d.mode")
RS2=$(jqn "$ST" "d.range_start")
RE2=$(jqn "$ST" "d.range_end")
ST_T=$(jqn "$ST" "d.start_time")
ET_T=$(jqn "$ST" "d.end_time")
[ "$MODE2" = "Replay" ] && pass "TC-S2-02: 再起動後 mode=Replay" || fail "TC-S2-02" "mode=$MODE2"
[ "$RS2" = "$START" ] && pass "TC-S2-02b: range_start 復元" || fail "TC-S2-02b" "got=$RS2 expected=$START"
[ "$RE2" = "$END" ] && pass "TC-S2-02c: range_end 復元" || fail "TC-S2-02c" "got=$RE2 expected=$END"

# --- TC-S2-02d: range_start (str) と start_time (ms) の整合 ---
EXPECT_ST=$(node -e "console.log(new Date('${START}:00Z').getTime())")
EXPECT_ET=$(node -e "console.log(new Date('${END}:00Z').getTime())")
if [ "$ST_T" = "null" ]; then
  pend "TC-S2-02d" "clock 未起動のため start_time=null（auto-play 前で計測不可）"
  pend "TC-S2-02e" "clock 未起動のため end_time=null"
else
  EQ_ST=$(bigt_eq "$ST_T" "$EXPECT_ST")
  EQ_ET=$(bigt_eq "$ET_T" "$EXPECT_ET")
  [ "$EQ_ST" = "true" ] && pass "TC-S2-02d: start_time ms 整合" || \
    fail "TC-S2-02d" "got=$ST_T expected=$EXPECT_ST"
  [ "$EQ_ET" = "true" ] && pass "TC-S2-02e: end_time ms 整合" || \
    fail "TC-S2-02e" "got=$ET_T expected=$EXPECT_ET"
fi
stop_app

# --- TC-S2-03: Play 実行後に保存 → 再起動で range_input 維持 ---
start_app
set +e; wait_playing 60; set -e
curl -s -X POST "$API/app/save" > /dev/null
stop_app

start_app
ST3=$(curl -s "$API/replay/status")
RS3=$(jqn "$ST3" "d.range_start")
RE3=$(jqn "$ST3" "d.range_end")
[ "$RS3" = "$START" ] && pass "TC-S2-03: 保存→復元で range_start 維持" || fail "TC-S2-03" "got=$RS3"
[ "$RE3" = "$END" ] && pass "TC-S2-03b: 保存→復元で range_end 維持" || fail "TC-S2-03b" "got=$RE3"
stop_app

# --- TC-S2-04: toggle → Live に戻してから保存 → 再起動で Live ---
start_app
curl -s -X POST "$API/replay/toggle" > /dev/null  # Replay → Live
sleep 1
curl -s -X POST "$API/app/save" > /dev/null
stop_app

start_app
ST4=$(curl -s "$API/replay/status")
MODE4=$(jqn "$ST4" "d.mode")
[ "$MODE4" = "Live" ] && pass "TC-S2-04: Live 保存→復元で mode=Live" || fail "TC-S2-04" "mode=$MODE4"

restore_state
print_summary
