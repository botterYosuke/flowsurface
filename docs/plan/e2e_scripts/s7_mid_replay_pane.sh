#!/usr/bin/env bash
# s7_mid_replay_pane.sh — スイート S7: Mid-replay ペイン CRUD
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S7: Mid-replay ペイン CRUD ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -3)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S7","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S7"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app
if ! wait_playing 30; then
  fail "TC-S7-precond" "Playing 到達せず"
  exit 1
fi

# 初期ペイン ID 取得
PANES=$(curl -s "$API/pane/list")
PANE0=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
if [ -z "$PANE0" ]; then
  fail "TC-S7-precond" "初期ペイン ID 取得失敗"
  exit 1
fi
echo "  PANE0=$PANE0"

# TC-S7-01: Playing 中に split（Vertical）→ ペイン数 2
curl -s -X POST "$API/pane/split" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$PANE0\",\"axis\":\"Vertical\"}" > /dev/null
if wait_for_pane_count 2 10; then
  pass "TC-S7-01: split 後ペイン数=2"
else
  fail "TC-S7-01" "10 秒以内にペイン数が 2 にならなかった"
fi

# TC-S7-02: 新ペインで set-ticker → streams_ready=true
PANES=$(curl -s "$API/pane/list")
NEW_PANE=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p = ps.find(x => x.id !== '$PANE0');
  console.log(p ? p.id : '');
" "$PANES")
echo "  NEW_PANE=$NEW_PANE"
if [ -z "$NEW_PANE" ]; then
  fail "TC-S7-02" "新ペイン ID 取得失敗"
else
  curl -s -X POST "$API/pane/set-ticker" \
    -H "Content-Type: application/json" \
    -d "{\"pane_id\":\"$NEW_PANE\",\"ticker\":\"BinanceLinear:ETHUSDT\"}" > /dev/null
  if wait_for_streams_ready "$NEW_PANE" 30; then
    pass "TC-S7-02: 新ペイン ETHUSDT streams_ready=true"
  else
    fail "TC-S7-02" "streams_ready タイムアウト（30s）"
  fi

  # TC-S7-02b: 元ペインの streams_ready も維持されているか
  PANES=$(curl -s "$API/pane/list")
  PANE0_READY=$(node -e "
    const ps = (JSON.parse(process.argv[1]).panes || []);
    const p = ps.find(x => x.id === '$PANE0');
    console.log(p && p.streams_ready ? 'true' : 'false');
  " "$PANES")
  [ "$PANE0_READY" = "true" ] \
    && pass "TC-S7-02b: split 後 PANE0 streams_ready 維持" \
    || fail "TC-S7-02b" "PANE0 streams_ready=$PANE0_READY"
fi

# TC-S7-03: Replay 継続確認
STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$STATUS" = "Playing" ] \
  && pass "TC-S7-03: split 後も Playing 継続" \
  || fail "TC-S7-03" "status=$STATUS"

# TC-S7-04: 新ペインで set-timeframe M5 → streams_ready=true
if [ -n "$NEW_PANE" ]; then
  curl -s -X POST "$API/pane/set-timeframe" \
    -H "Content-Type: application/json" \
    -d "{\"pane_id\":\"$NEW_PANE\",\"timeframe\":\"M5\"}" > /dev/null
  if wait_for_streams_ready "$NEW_PANE" 30; then
    pass "TC-S7-04: M5 set-timeframe → streams_ready=true"
  else
    fail "TC-S7-04" "streams_ready タイムアウト（30s）"
  fi
fi

# TC-S7-05: 新ペインを close → ペイン数 1
if [ -n "$NEW_PANE" ]; then
  curl -s -X POST "$API/pane/close" \
    -H "Content-Type: application/json" \
    -d "{\"pane_id\":\"$NEW_PANE\"}" > /dev/null
  if wait_for_pane_count 1 10; then
    pass "TC-S7-05: close 後ペイン数=1"
  else
    fail "TC-S7-05" "10 秒以内にペイン数が 1 にならなかった"
  fi
fi

# TC-S7-06: Replay 継続確認
STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$STATUS" = "Playing" ] \
  && pass "TC-S7-06: close 後も Playing 継続" \
  || fail "TC-S7-06" "status=$STATUS"

stop_app

# TC-S7-07: range end 到達後に split してもクラッシュしない
# 短い range（1 時間）+ 10x 速度 → 約 6 分で end に到達
START_SHORT=$(utc_offset -2)
END_SHORT=$(utc_offset -1)
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S7b","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S7b"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START_SHORT","range_end":"$END_SHORT"}
}
EOF
start_app
if ! wait_playing 30; then
  fail "TC-S7-07-pre" "Playing 到達せず（S7b）"
  exit 1
fi
echo "  10x 速度で range end を待機（最大 480 秒）..."
speed_to_10x
if ! wait_status Paused 480; then
  fail "TC-S7-07-pre" "range end 到達せず（480 秒タイムアウト）"
  exit 1
fi

# range end 到達後に split
PANES_BEFORE=$(curl -s "$API/pane/list")
LAST_PANE=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES_BEFORE")
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/pane/split" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$LAST_PANE\",\"axis\":\"Vertical\"}")

NOTIFS=$(curl -s "$API/notification/list")
HAS_ERR=$(node -e "
  const ns = (JSON.parse(process.argv[1]).notifications || []);
  console.log(ns.some(n => n.level === 'error') ? 'true' : 'false');
" "$NOTIFS")

[ "$HTTP_CODE" = "200" ] && [ "$HAS_ERR" = "false" ] \
  && pass "TC-S7-07: range end 後 split → crash なし (HTTP=$HTTP_CODE, error_toast=$HAS_ERR)" \
  || fail "TC-S7-07" "HTTP=$HTTP_CODE, error_toast=$HAS_ERR"

print_summary
[ $FAIL -eq 0 ]
