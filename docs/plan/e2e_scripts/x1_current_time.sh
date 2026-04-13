#!/bin/bash
# x1_current_time.sh — 横断スイート X1: current_time 表示の不変条件
source "$(dirname "$0")/common_helpers.sh"

echo "=== X1: current_time 表示の不変条件 ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"X1","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"X1"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app
if ! wait_playing 60; then
  fail "X1-precond" "Playing 到達せず"
  restore_state
  print_summary
  exit 1
fi

# --- TC-X1-01: バー境界スナップ不変条件（10 サンプル）---
ALL_ON_BAR="true"
for i in $(seq 1 10); do
  CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  ON=$(is_bar_boundary "$CT" "$STEP_M1")
  [ "$ON" = "true" ] || { ALL_ON_BAR="false"; echo "  off-bar at i=$i ct=$CT"; }
  sleep 0.5
done
[ "$ALL_ON_BAR" = "true" ] && pass "TC-X1-01: 10 サンプル全てバー境界" || \
  fail "TC-X1-01" "バー境界違反あり"

# --- TC-X1-02: current_time の単調非減少 ---
PREV="0"
MONO="true"
for i in $(seq 1 8); do
  CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  GE=$(bigt_ge "$CT" "$PREV")
  [ "$GE" = "true" ] || MONO="false"
  PREV="$CT"
  sleep 0.4
done
[ "$MONO" = "true" ] && pass "TC-X1-02: current_time 単調非減少" || \
  fail "TC-X1-02" "逆行あり"

# --- TC-X1-03: range 内不変条件（連続サンプル）---
ST_T=$(jqn "$(curl -s "$API/replay/status")" "d.start_time")
ET_T=$(jqn "$(curl -s "$API/replay/status")" "d.end_time")
ALL_IN="true"
for i in $(seq 1 6); do
  CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  IN=$(ct_in_range "$CT" "$ST_T" "$ET_T")
  [ "$IN" = "true" ] || ALL_IN="false"
  sleep 0.5
done
[ "$ALL_IN" = "true" ] && pass "TC-X1-03: range 内不変" || \
  fail "TC-X1-03" "range 外"

# --- TC-X1-04: [要 API 拡張] current_time_display と current_time の整合 ---
DISPLAY=$(status_display)
if [ "$DISPLAY" = "null" ]; then
  pend "TC-X1-04" "ReplayStatus.current_time_display 未実装"
else
  CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  EXPECT=$(node -e "
    const d=new Date(Number('$CT'));
    const pad=n=>String(n).padStart(2,'0');
    console.log(d.getUTCFullYear()+'-'+pad(d.getUTCMonth()+1)+'-'+pad(d.getUTCDate())+' '+pad(d.getUTCHours())+':'+pad(d.getUTCMinutes())+':'+pad(d.getUTCSeconds()));
  ")
  [ "$DISPLAY" = "$EXPECT" ] && pass "TC-X1-04: display=$DISPLAY と current_time 整合" || \
    fail "TC-X1-04" "display=$DISPLAY expected=$EXPECT"
fi

# --- TC-X1-05: [要 API 拡張] display も連続して進む ---
D1=$(status_display)
if [ "$D1" = "null" ]; then
  pend "TC-X1-05" "current_time_display 未実装"
else
  sleep 3
  D2=$(status_display)
  [ "$D1" != "$D2" ] && pass "TC-X1-05: display が前進 ($D1 → $D2)" || \
    fail "TC-X1-05" "display 固定 ($D1)"
fi

# --- TC-X1-06: Live モードで current_time / display が null ---
curl -s -X POST "$API/replay/toggle" > /dev/null  # → Live
sleep 1
ST=$(curl -s "$API/replay/status")
CT=$(jqn "$ST" "d.current_time")
SP=$(jqn "$ST" "d.speed")
[ "$CT" = "null" ] && pass "TC-X1-06a: Live current_time=null" || fail "TC-X1-06a" "ct=$CT"
[ "$SP" = "null" ] && pass "TC-X1-06b: Live speed=null" || fail "TC-X1-06b" "speed=$SP"

restore_state
print_summary
