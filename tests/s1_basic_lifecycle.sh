#!/bin/bash
# s1_basic_lifecycle.sh — スイート S1: 基本ライフサイクル
#
# 検証シナリオ:
#   TC-S1-01: 起動時 mode=Live
#   TC-S1-02: toggle → mode=Replay
#   TC-S1-03〜04: play → Loading/Playing 到達（最大 120s）
#   TC-S1-05〜05c: 1x 速度で current_time 前進・バー境界スナップ・range 内
#   TC-S1-06〜07: Pause 中 current_time 固定・status=Paused
#   TC-S1-08: Resume 後 current_time 前進
#   TC-S1-09〜12: Speed サイクル（1x→2x→5x→10x→1x）
#   TC-S1-13〜13b: StepForward +60000ms・バー境界維持
#   TC-S1-14: StepBackward -60000ms
#   TC-S1-15a〜f: Live 復帰で状態リセット・range 値は保持
#
# 仕様根拠:
#   docs/replay_header.md §4〜§8 — Replay ライフサイクル・clock 状態機械
#
# フィクスチャ: E2E_TICKER(デフォルト BinanceLinear:BTCUSDT) M1, Live モード起動 → 手動 toggle/play
# E2E_TICKER 環境変数でティッカーを切り替え可能（例: HyperliquidLinear:BTC）
source "$(dirname "$0")/common_helpers.sh"

TICKER="${E2E_TICKER:-BinanceLinear:BTCUSDT}"
echo "=== S1: 基本ライフサイクル (ticker=$TICKER) ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager": {
    "layouts": [{"name":"S1-Basic","dashboard":{"pane":{
      "KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"$TICKER","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":["Volume"],"link_group":"A"
      }
    },"popout":[]}}],
    "active_layout":"S1-Basic"
  },
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app

# Live ストリームが Ready になるまで待つ（メタデータ取得に数秒かかる）
# pane/list の streams_ready が true になるまでポーリング（最大 30s）
for i in $(seq 1 30); do
  PLIST=$(curl -s "$API/pane/list" 2>/dev/null || echo '{}')
  READY=$(node -e "try{const d=JSON.parse(process.argv[1]);const p=(d.panes||[])[0];process.stdout.write(p&&p.streams_ready===true?'true':'false');}catch(e){process.stdout.write('false');}" "$PLIST")
  if [ "$READY" = "true" ]; then
    TICKER=$(node -e "try{const d=JSON.parse(process.argv[1]);const p=(d.panes||[])[0];process.stdout.write(p&&p.ticker?p.ticker:'');}catch(e){}" "$PLIST")
    echo "  streams ready (${i}s, ticker=$TICKER)"
    break
  fi
  sleep 1
done

# --- TC-S1-01: Live モードで起動 ---
STATUS=$(curl -s "$API/replay/status")
MODE=$(jqn "$STATUS" "d.mode")
[ "$MODE" = "Live" ] && pass "TC-S1-01: 起動時 mode=Live" || fail "TC-S1-01" "mode=$MODE"

# --- TC-S1-02: Replay に切替 ---
TOGGLE=$(curl -s -X POST "$API/replay/toggle")
MODE2=$(jqn "$TOGGLE" "d.mode")
[ "$MODE2" = "Replay" ] && pass "TC-S1-02: toggle → mode=Replay" || fail "TC-S1-02" "mode=$MODE2"

# --- TC-S1-03: Play 開始 ---
PLAY_RES=$(curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}")
PLAY_ST=$(jqn "$PLAY_RES" "d.status")
[[ "$PLAY_ST" = "Loading" || "$PLAY_ST" = "Playing" ]] && \
  pass "TC-S1-03: play → Loading or Playing" || fail "TC-S1-03" "status=$PLAY_ST"

# --- TC-S1-04: Playing 到達（最大 120s） ---
if wait_playing 120; then
  pass "TC-S1-04: Playing に到達"
else
  fail "TC-S1-04" "120秒以内に Playing にならなかった"
fi

# --- TC-S1-05: current_time が前進する ---
CT1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
if CT2=$(wait_for_time_advance "$CT1" 30); then
  WITHIN=$(advance_within "$CT1" "$CT2" "$STEP_M1" 100)
  [ "$WITHIN" = "true" ] && pass "TC-S1-05: 1x で current_time 前進 ($CT1 → $CT2)" || \
    fail "TC-S1-05" "想定外の前進 (CT1=$CT1 CT2=$CT2 step=$STEP_M1)"
else
  fail "TC-S1-05" "30 秒待機しても current_time が前進しなかった (CT1=$CT1)"
  CT2="$CT1"
fi

# --- TC-S1-05b: current_time はバー境界値 ---
ON_BAR=$(is_bar_boundary "$CT2" "$STEP_M1")
[ "$ON_BAR" = "true" ] && pass "TC-S1-05b: current_time バー境界スナップ" || \
  fail "TC-S1-05b" "CT2=$CT2 はバー境界ではない"

# --- TC-S1-05c: current_time ∈ [start_time, end_time] ---
ST_NOW=$(curl -s "$API/replay/status")
START_T=$(jqn "$ST_NOW" "d.start_time")
END_T=$(jqn "$ST_NOW" "d.end_time")
IN=$(ct_in_range "$CT2" "$START_T" "$END_T")
[ "$IN" = "true" ] && pass "TC-S1-05c: current_time ∈ [start,end]" || \
  fail "TC-S1-05c" "CT2=$CT2 range=[$START_T,$END_T]"

# --- TC-S1-06: Pause で固定 ---
curl -s -X POST "$API/replay/pause" > /dev/null
sleep 1
P1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 3
P2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
EQ=$(bigt_eq "$P1" "$P2")
[ "$EQ" = "true" ] && pass "TC-S1-06: Pause 中は current_time 固定" || \
  fail "TC-S1-06" "Pause 中に時刻が変化 ($P1 → $P2)"

# --- TC-S1-07: status=Paused ---
ST_PAUSED=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ST_PAUSED" = "Paused" ] && pass "TC-S1-07: status=Paused" || fail "TC-S1-07" "status=$ST_PAUSED"

# --- TC-S1-08: Resume で再開 ---
curl -s -X POST "$API/replay/resume" > /dev/null
sleep 3
R1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
ADV2=$(bigt_gt "$R1" "$P2")
[ "$ADV2" = "true" ] && pass "TC-S1-08: Resume 後に current_time 前進" || \
  fail "TC-S1-08" "Resume 後に前進しない ($P2 → $R1)"

# --- TC-S1-09〜12: Speed サイクル ---
curl -s -X POST "$API/replay/pause" > /dev/null
for expected in "2x" "5x" "10x" "1x"; do
  SPEED=$(jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed")
  [ "$SPEED" = "$expected" ] && pass "TC-S1-speed: speed=$SPEED" || \
    fail "TC-S1-speed" "expected=$expected got=$SPEED"
done

# --- TC-S1-13: StepForward は 1 バーきっかり進む ---
PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
POST_SF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF=$(bigt_sub "$POST_SF" "$PRE")
[ "$DIFF" = "60000" ] && pass "TC-S1-13: StepForward +60000ms" || \
  fail "TC-S1-13" "diff=$DIFF (expected 60000)"
ON_BAR=$(is_bar_boundary "$POST_SF" "$STEP_M1")
[ "$ON_BAR" = "true" ] && pass "TC-S1-13b: StepForward 後もバー境界" || \
  fail "TC-S1-13b" "POST_SF=$POST_SF"

# --- TC-S1-14: StepBackward は 1 バーきっかり後退 ---
BEF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-backward" > /dev/null
sleep 1
AFT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF_B=$(bigt_sub "$BEF" "$AFT")
[ "$DIFF_B" = "60000" ] && pass "TC-S1-14: StepBackward -60000ms" || \
  fail "TC-S1-14" "diff=$DIFF_B (expected 60000, before=$BEF after=$AFT)"

# --- TC-S1-15: Live 復帰時の状態完全リセット ---
LIVE_TOGGLE=$(curl -s -X POST "$API/replay/toggle")
LIVE_MODE=$(jqn "$LIVE_TOGGLE" "d.mode")
LIVE_ST=$(jqn "$LIVE_TOGGLE" "d.status")
LIVE_CT=$(jqn "$LIVE_TOGGLE" "d.current_time")
LIVE_SP=$(jqn "$LIVE_TOGGLE" "d.speed")
LIVE_RS=$(jqn "$LIVE_TOGGLE" "d.range_start")
LIVE_RE=$(jqn "$LIVE_TOGGLE" "d.range_end")
[ "$LIVE_MODE" = "Live" ] && pass "TC-S1-15a: mode=Live" || fail "TC-S1-15a" "mode=$LIVE_MODE"
[ "$LIVE_ST" = "null" ] && pass "TC-S1-15b: status=null" || fail "TC-S1-15b" "status=$LIVE_ST"
[ "$LIVE_CT" = "null" ] && pass "TC-S1-15c: current_time=null" || fail "TC-S1-15c" "ct=$LIVE_CT"
[ "$LIVE_SP" = "null" ] && pass "TC-S1-15d: speed=null" || fail "TC-S1-15d" "speed=$LIVE_SP"
[ -n "$LIVE_RS" ] && pass "TC-S1-15e: range_start は最後の Replay 値を保持 ($LIVE_RS)" || fail "TC-S1-15e" "rs が空 (保持されていない)"
[ -n "$LIVE_RE" ] && pass "TC-S1-15f: range_end は最後の Replay 値を保持 ($LIVE_RE)" || fail "TC-S1-15f" "re が空 (保持されていない)"

restore_state
print_summary
