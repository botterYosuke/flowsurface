#!/bin/bash
# x2_buttons.sh — 横断スイート X2: ボタンの厳密挙動
source "$(dirname "$0")/common_helpers.sh"

echo "=== X2: ボタンの厳密挙動 ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"X2","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"X2"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app
if ! wait_playing 60; then
  fail "X2-precond" "Playing 到達せず"
  restore_state
  print_summary
  exit 1
fi
curl -s -X POST "$API/replay/pause" > /dev/null
set +e; wait_paused 5; set -e

# --- TC-X2-01: StepForward x5 = +300000ms ---
PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
for i in $(seq 1 5); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 0.2
done
POST=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF=$(bigt_sub "$POST" "$PRE")
[ "$DIFF" = "300000" ] && pass "TC-X2-01: StepForward x5 = +300000ms" || \
  fail "TC-X2-01" "diff=$DIFF (expected 300000)"

# --- TC-X2-02: StepBackward x5 で完全可逆 ---
for i in $(seq 1 5); do
  curl -s -X POST "$API/replay/step-backward" > /dev/null
  sleep 0.2
done
BACK=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
[ "$BACK" = "$PRE" ] && pass "TC-X2-02: 可逆 (back=$BACK)" || \
  fail "TC-X2-02" "back=$BACK pre=$PRE"

# --- TC-X2-03: start 端での StepBackward は no-op ---
ST_T=$(jqn "$(curl -s "$API/replay/status")" "d.start_time")
for i in $(seq 1 200); do
  CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  EQ=$(bigt_eq "$CT" "$ST_T")
  [ "$EQ" = "true" ] && break
  curl -s -X POST "$API/replay/step-backward" > /dev/null
  sleep 0.05
done
AT_START=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-backward" > /dev/null
sleep 0.5
BEYOND=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
EQ2=$(bigt_eq "$AT_START" "$BEYOND")
[ "$EQ2" = "true" ] && pass "TC-X2-03: start 端 StepBackward は no-op" || \
  fail "TC-X2-03" "AT_START=$AT_START BEYOND=$BEYOND"

# --- TC-X2-04: Pause 冪等性 ---
curl -s -X POST "$API/replay/pause" > /dev/null
ST1=$(jqn "$(curl -s "$API/replay/status")" "d.status")
CT1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/pause" > /dev/null
ST2=$(jqn "$(curl -s "$API/replay/status")" "d.status")
CT2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
CT_EQ=$(bigt_eq "$CT1" "$CT2")
[ "$ST1" = "$ST2" ] && [ "$CT_EQ" = "true" ] && pass "TC-X2-04: Pause 冪等" || \
  fail "TC-X2-04" "ST=$ST1→$ST2 CT=$CT1→$CT2"

# --- TC-X2-05: Resume → Pause → Resume の往復で current_time の継続性 ---
curl -s -X POST "$API/replay/resume" > /dev/null
sleep 1
PRE_R=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/pause" > /dev/null
sleep 1
PAUSED_AT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
GE1=$(bigt_ge "$PAUSED_AT" "$PRE_R")
[ "$GE1" = "true" ] && pass "TC-X2-05a: Pause 後の時刻 >= Pause 前" || \
  fail "TC-X2-05a" "PAUSED_AT=$PAUSED_AT PRE_R=$PRE_R"
curl -s -X POST "$API/replay/resume" > /dev/null
sleep 1
RESUMED=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
GE2=$(bigt_ge "$RESUMED" "$PAUSED_AT")
[ "$GE2" = "true" ] && pass "TC-X2-05b: Resume 後 >= Pause 時刻" || \
  fail "TC-X2-05b" "RESUMED=$RESUMED PAUSED_AT=$PAUSED_AT"

# --- TC-X2-06: Speed サイクル一周 + speed 値の永続 ---
curl -s -X POST "$API/replay/pause" > /dev/null
for i in $(seq 1 5); do
  SP=$(jqn "$(curl -s "$API/replay/status")" "d.speed")
  [ "$SP" = "1x" ] && break
  curl -s -X POST "$API/replay/speed" > /dev/null
done
EXPECTED=("2x" "5x" "10x" "1x")
ALL_OK="true"
for e in "${EXPECTED[@]}"; do
  GOT=$(jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed")
  [ "$GOT" = "$e" ] || { ALL_OK="false"; echo "  cycle break: expected=$e got=$GOT"; }
done
[ "$ALL_OK" = "true" ] && pass "TC-X2-06: Speed cycle 1→2→5→10→1" || fail "TC-X2-06" "cycle 異常"

# --- TC-X2-07: Speed 変更で current_time が range.start にリセットされる ---
# 新仕様: CycleSpeed は pause + seek(range.start) を伴う。
# TC-X2-06 後は range.start にいるため、まず StepForward で前進させてから確認する。
set +e; wait_paused 5; set -e
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 0.3
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 0.3
PRE_SP=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
START_T=$(jqn "$(curl -s "$API/replay/status")" "d.start_time")
IS_AHEAD=$(bigt_gt "$PRE_SP" "$START_T")
[ "$IS_AHEAD" = "true" ] || fail "TC-X2-07-pre" "pre-condition: not ahead of start (pre=$PRE_SP start=$START_T)"
curl -s -X POST "$API/replay/speed" > /dev/null
POST_SP=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
EQ_SP=$(bigt_eq "$POST_SP" "$START_T")
[ "$EQ_SP" = "true" ] \
  && pass "TC-X2-07: Speed 切替で current_time が range.start にリセット (pre=$PRE_SP → post=$POST_SP)" \
  || fail "TC-X2-07" "post=$POST_SP (expected start=$START_T)"

# --- TC-X2-08: Live 中はボタンが意味を持たない ---
curl -s -X POST "$API/replay/toggle" > /dev/null  # → Live
LIVE_BEFORE=$(curl -s "$API/replay/status")
curl -s -X POST "$API/replay/step-forward" > /dev/null
curl -s -X POST "$API/replay/pause" > /dev/null
curl -s -X POST "$API/replay/resume" > /dev/null
LIVE_AFTER=$(curl -s "$API/replay/status")
B_MODE=$(jqn "$LIVE_BEFORE" "d.mode")
A_MODE=$(jqn "$LIVE_AFTER" "d.mode")
B_CT=$(jqn "$LIVE_BEFORE" "d.current_time")
A_CT=$(jqn "$LIVE_AFTER" "d.current_time")
[ "$A_MODE" = "Live" ] && [ "$B_MODE" = "Live" ] && [ "$B_CT" = "null" ] && [ "$A_CT" = "null" ] && \
  pass "TC-X2-08: Live 中ボタン操作は no-op" || \
  fail "TC-X2-08" "mode=$B_MODE→$A_MODE ct=$B_CT→$A_CT"

restore_state
print_summary
