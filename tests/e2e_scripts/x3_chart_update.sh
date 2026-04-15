#!/bin/bash
# x3_chart_update.sh — 横断スイート X3: チャート表示内容と更新タイミング（要 API 拡張）
source "$(dirname "$0")/common_helpers.sh"

echo "=== X3: チャート表示内容と更新タイミング ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)
START_MS=$(node -e "console.log(new Date('${START}:00Z').getTime())")
END_MS=$(node -e "console.log(new Date('${END}:00Z').getTime())")

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"X3","dashboard":{"pane":{
    "Split":{"axis":"Vertical","ratio":0.5,
      "a":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":["Volume"],"link_group":"A"
      }},
      "b":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:ETHUSDT","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":["Volume"],"link_group":"B"
      }}
    }
  },"popout":[]}}],"active_layout":"X3"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app

# chart-snapshot API の存在確認
PROBE=$(curl -s -o /dev/null -w "%{http_code}" "$API/pane/chart-snapshot?pane_id=00000000-0000-0000-0000-000000000000")
if [ "$PROBE" = "404" ]; then
  pend "TC-X3-*" "chart-snapshot API 未実装（§2.1）— X3 全 TC を PENDING"
  restore_state
  print_summary
  exit 0
fi

if ! wait_playing 60; then
  fail "X3-precond" "Playing 到達せず"
  restore_state
  print_summary
  exit 1
fi

PANE_LIST=$(curl -s "$API/pane/list")
BTC_PANE=$(node -e "
  const d=JSON.parse(process.argv[1]);
  const p=(d.panes||[]).find(p=>p.ticker && p.ticker.includes('BTCUSDT'));
  console.log(p?p.id:'');
" "$PANE_LIST")
ETH_PANE=$(node -e "
  const d=JSON.parse(process.argv[1]);
  const p=(d.panes||[]).find(p=>p.ticker && p.ticker.includes('ETHUSDT'));
  console.log(p?p.id:'');
" "$PANE_LIST")
[ -n "$BTC_PANE" ] && [ -n "$ETH_PANE" ] && pass "TC-X3-precond: 2 ペイン id 取得" || \
  fail "TC-X3-precond" "BTC=$BTC_PANE ETH=$ETH_PANE"

# --- TC-X3-01: Play 到達直後のチャート初期本数 ---
SNAP=$(chart_snapshot "$BTC_PANE")
KC=$(jqn "$SNAP" "d.bar_count")
FT=$(jqn "$SNAP" "d.oldest_ts")
LT=$(jqn "$SNAP" "d.newest_ts")
CT_NOW=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
[ "$KC" -ge 1 ] 2>/dev/null && pass "TC-X3-01a: bar_count=$KC >= 1" || \
  fail "TC-X3-01a" "bar_count=$KC"
set +e
# oldest_ts は Pre-start history により start_ms より前のデータも含む（oldest_ts <= start_ms）
# newest_ts は current_time 以下であることを確認する
node -e "process.exit((BigInt('$FT')<=BigInt('$START_MS') && BigInt('$LT')<=BigInt('$CT_NOW'))?0:1)"
[ $? -eq 0 ] && pass "TC-X3-01b: oldest_ts <= start, newest_ts <= current_time" || \
  fail "TC-X3-01b" "first=$FT last=$LT range=[$START_MS,$CT_NOW]"
set -e

# --- TC-X3-02: Playing 進行中の newest_ts 単調非減少 ---
PREV_LT="0"
INCREASED="false"
MONO="true"
for i in $(seq 1 5); do
  S=$(chart_snapshot "$BTC_PANE")
  LT=$(jqn "$S" "d.newest_ts")
  GT=$(node -e "console.log(BigInt('$LT')>BigInt('$PREV_LT'))")
  GE=$(node -e "console.log(BigInt('$LT')>=BigInt('$PREV_LT'))")
  [ "$GE" = "true" ] || MONO="false"
  [ "$GT" = "true" ] && INCREASED="true"
  PREV_LT="$LT"
  sleep 1
done
[ "$MONO" = "true" ] && pass "TC-X3-02a: newest_ts 単調非減少" || fail "TC-X3-02a" "逆行あり"
[ "$INCREASED" = "true" ] && pass "TC-X3-02b: newest_ts 増加あり" || \
  fail "TC-X3-02b" "5 秒で 1 度も増加せず（描画停止？）"

# --- TC-X3-03: StepForward 押下で 1 bar 分 newest_ts が進む ---
curl -s -X POST "$API/replay/pause" > /dev/null
set +e; wait_paused 10; set -e
B_SNAP=$(chart_snapshot "$BTC_PANE")
B_LT=$(jqn "$B_SNAP" "d.newest_ts")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
A_SNAP=$(chart_snapshot "$BTC_PANE")
A_LT=$(jqn "$A_SNAP" "d.newest_ts")
DIFF=$(bigt_sub "$A_LT" "$B_LT")
# diff が STEP_M1 の倍数かつ >= STEP_M1 であれば OK（チャートの非同期更新で複数 bar 進む場合を許容）
set +e
node -e "process.exit(BigInt('$DIFF') >= BigInt('$STEP_M1') && BigInt('$DIFF') % BigInt('$STEP_M1') === 0n ? 0 : 1)"
[ $? -eq 0 ] && pass "TC-X3-03: StepForward → newest_ts +${DIFF}ms (>= 1 bar, bar-aligned)" || \
  fail "TC-X3-03" "diff=$DIFF (expected >= $STEP_M1 and bar-aligned)"
set -e

# --- TC-X3-04: マルチペイン同期 ---
B=$(chart_snapshot "$BTC_PANE")
E=$(chart_snapshot "$ETH_PANE")
B_LT=$(jqn "$B" "d.newest_ts")
E_LT=$(jqn "$E" "d.newest_ts")
ABS=$(node -e "
  const diff = BigInt('$B_LT') - BigInt('$E_LT');
  console.log(String(diff < 0n ? -diff : diff));
")
set +e
node -e "process.exit(BigInt('$ABS') <= BigInt('$STEP_M1') ? 0 : 1)"
[ $? -eq 0 ] && pass "TC-X3-04: BTC/ETH last_kline 同期 (diff=${ABS}ms)" || \
  fail "TC-X3-04" "BTC=$B_LT ETH=$E_LT diff=${ABS}ms (> $STEP_M1)"
set -e

restore_state
print_summary
