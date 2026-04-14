#!/usr/bin/env bash
# s11_bar_step_discrete.sh — スイート S11: バーステップ離散化
# current_time の変化量が timeframe の倍数になることを確認する
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S11: バーステップ離散化 ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── TC-S11-01: M1 10x 再生中 delta が 60000ms の倍数 ──────────────────────
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$(utc_offset -3)" "$(utc_offset -1)"
start_app
if ! wait_playing 30; then
  fail "TC-S11-01-pre" "Playing 到達せず"
  exit 1
fi

speed_to_10x
T1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
if T2=$(wait_for_time_advance "$T1" 15); then
  DELTA=$(node -e "console.log(String(BigInt('$T2') - BigInt('$T1')))")
  MOD=$(node -e "console.log(String(BigInt('$DELTA') % BigInt('60000')))")
  [ "$MOD" = "0" ] \
    && pass "TC-S11-01: M1 10x delta=$DELTA ms（60000ms の倍数）" \
    || fail "TC-S11-01" "delta=$DELTA, mod=$MOD (60000ms の倍数でない)"
else
  fail "TC-S11-01" "15 秒待機しても current_time が変化しなかった"
fi

# ── TC-S11-02: M1 Pause → StepForward × 3、各 delta = 60000ms ────────────
curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 10; then
  fail "TC-S11-02-pre" "Paused に遷移せず"
else
  for i in 1 2 3; do
    TB=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
    curl -s -X POST "$API/replay/step-forward" > /dev/null
    sleep 1
    wait_status Paused 10 || true
    TA=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
    DELTA=$(node -e "console.log(String(BigInt('$TA') - BigInt('$TB')))")
    [ "$DELTA" = "60000" ] \
      && pass "TC-S11-02-$i: StepForward #$i delta=60000ms" \
      || fail "TC-S11-02-$i" "delta=$DELTA (expected 60000)"
  done
fi

stop_app

# ── TC-S11-03: M5 ペイン StepForward delta = 300000ms ─────────────────────
setup_single_pane "BinanceLinear:BTCUSDT" "M5" "$(utc_offset -6)" "$(utc_offset -1)"
start_app
if ! wait_playing 30; then
  fail "TC-S11-03-pre" "Playing 到達せず"
else
  curl -s -X POST "$API/replay/pause" > /dev/null
  wait_status Paused 10 || true
  TB=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 1
  wait_status Paused 10 || true
  TA=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  DELTA=$(node -e "console.log(String(BigInt('$TA') - BigInt('$TB')))")
  [ "$DELTA" = "300000" ] \
    && pass "TC-S11-03: M5 StepForward delta=300000ms" \
    || fail "TC-S11-03" "delta=$DELTA (expected 300000)"
fi

stop_app

# ── TC-S11-04: H1 ペイン StepForward delta = 3600000ms ────────────────────
setup_single_pane "BinanceLinear:BTCUSDT" "H1" "$(utc_offset -24)" "$(utc_offset -1)"
start_app
if ! wait_playing 30; then
  fail "TC-S11-04-pre" "Playing 到達せず"
else
  curl -s -X POST "$API/replay/pause" > /dev/null
  wait_status Paused 10 || true
  TB=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 1
  wait_status Paused 10 || true
  TA=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  DELTA=$(node -e "console.log(String(BigInt('$TA') - BigInt('$TB')))")
  [ "$DELTA" = "3600000" ] \
    && pass "TC-S11-04: H1 StepForward delta=3600000ms" \
    || fail "TC-S11-04" "delta=$DELTA (expected 3600000)"
fi

stop_app

# ── TC-S11-05: M1+M5 混在 → 最小 TF (M1=60000ms) が優先 ─────────────────
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S11-mix","dashboard":{"pane":{
    "Split":{"axis":"Vertical","ratio":0.5,
      "a":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":[],"link_group":"A"
      }},
      "b":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:ETHUSDT","timeframe":"M5"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M5"}},
        "indicators":[],"link_group":"A"
      }}
    }
  },"popout":[]}}],"active_layout":"S11-mix"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$(utc_offset -3)","range_end":"$(utc_offset -1)"}
}
EOF
start_app
if ! wait_playing 30; then
  fail "TC-S11-05-pre" "Playing 到達せず"
else
  curl -s -X POST "$API/replay/pause" > /dev/null
  wait_status Paused 10 || true
  TB=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 1
  wait_status Paused 10 || true
  TA=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  DELTA=$(node -e "console.log(String(BigInt('$TA') - BigInt('$TB')))")
  [ "$DELTA" = "60000" ] \
    && pass "TC-S11-05: M1+M5 混在 StepForward delta=60000ms（M1 優先）" \
    || fail "TC-S11-05" "delta=$DELTA (expected 60000, M1 優先のはず)"
fi

print_summary
[ $FAIL -eq 0 ]
