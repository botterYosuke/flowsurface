#!/usr/bin/env bash
# s11_bar_step_discrete.sh — スイート S11: バーステップ離散化
#
# 検証シナリオ:
#   TC-S11-01: M1 10x 再生中 delta が 60000ms の倍数
#   TC-S11-02-1〜3: M1 StepForward × 3、各 delta = 60000ms
#   TC-S11-03: M5 StepForward delta = 300000ms
#   TC-S11-04: H1 StepForward delta = 3600000ms
#   TC-S11-05: M1+M5 混在 StepForward → min TF (M1=60000ms) が優先
#
# 仕様根拠:
#   docs/replay_header.md §6.2 — バーステップ離散化（step_size = min timeframe）
#
# フィクスチャ: BinanceLinear:BTCUSDT M1 / M5 / H1 / M1+ETHUSDT M5 混在（4パターン）
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S11: バーステップ離散化 ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── TC-S11-01: M1 10x 再生中 delta が 60000ms の倍数 ──────────────────────
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$(utc_offset -3)" "$(utc_offset -1)"
start_app
headless_play
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
headless_play
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
headless_play
if ! wait_playing 60; then
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
if is_headless; then
  # headless: pane/split + pane/set-timeframe で M1+M5 混在を再現してステップ幅を検証
  setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$(utc_offset -3)" "$(utc_offset -1)"
  start_app
  headless_play
  if ! wait_playing 30; then
    fail "TC-S11-05-pre" "Playing 到達せず"
  else
    curl -s -X POST "$API/replay/pause" > /dev/null
    wait_status Paused 10 || true

    PANES=$(curl -s "$API/pane/list")
    PANE0=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")

    # pane0 を分割して pane1 を作成し M5 に変更
    curl -s -X POST "$API/pane/split" \
      -H "Content-Type: application/json" \
      -d "{\"pane_id\":\"$PANE0\",\"axis\":\"Vertical\"}" > /dev/null
    sleep 0.3

    PANES=$(curl -s "$API/pane/list")
    PANE1=$(node -e "
      const ps=(JSON.parse(process.argv[1]).panes||[]);
      const p=ps.find(x=>x.id!=='$PANE0');
      console.log(p?p.id:'');
    " "$PANES")

    curl -s -X POST "$API/pane/set-timeframe" \
      -H "Content-Type: application/json" \
      -d "{\"pane_id\":\"$PANE1\",\"timeframe\":\"M5\"}" > /dev/null

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
  stop_app
else
  stop_app

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

  stop_app
fi

# ── TC-S11-06: M1 StepForward 10 連続 → 毎回 delta が厳密に 60000ms ─────────
# TC-S11-02 は倍数チェック（2 バー同時前進でも pass）。本 TC は厳密な exact match を検証する。
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$(utc_offset -3)" "$(utc_offset -1)"
start_app
headless_play
if ! wait_playing 30; then
  fail "TC-S11-06-pre" "Playing 到達せず"
else
  curl -s -X POST "$API/replay/pause" > /dev/null
  if ! wait_status Paused 10; then
    fail "TC-S11-06-pre" "Paused に遷移せず"
  else
    MONOTONE_OK=true
    for i in $(seq 1 10); do
      TB=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
      curl -s -X POST "$API/replay/step-forward" > /dev/null
      sleep 0.5
      wait_status Paused 10 || true
      TA=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
      DELTA=$(node -e "console.log(String(BigInt('$TA') - BigInt('$TB')))")
      if [ "$DELTA" = "60000" ]; then
        pass "TC-S11-06-$i: StepForward #$i delta=60000ms（exact）"
      else
        fail "TC-S11-06-$i" "delta=$DELTA (expected exactly 60000 — 複数バー同時前進の疑い)"
        MONOTONE_OK=false
      fi
    done
  fi
fi

print_summary
[ $FAIL -eq 0 ]
