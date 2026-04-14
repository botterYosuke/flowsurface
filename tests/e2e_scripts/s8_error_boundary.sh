#!/bin/bash
# s8_error_boundary.sh — スイート S8: エラー・境界値ケース
source "$(dirname "$0")/common_helpers.sh"

echo "=== S8: エラー・境界値ケース ==="
backup_state

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S8","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S8"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app

# Live ストリームが Ready になるまで待つ（Binance メタデータ取得に数秒かかる）
for i in $(seq 1 30); do
  PLIST=$(curl -s "$API/pane/list" 2>/dev/null || echo '{}')
  READY=$(node -e "try{const d=JSON.parse(process.argv[1]);const ps=d.panes||[];const allReady=ps.length>0&&ps.every(p=>p.streams_ready===true);process.stdout.write(allReady?'true':'false');}catch(e){process.stdout.write('false');}" "$PLIST")
  [ "$READY" = "true" ] && echo "  streams ready (${i}s)" && break
  sleep 1
done

# --- TC-S8-01: 存在しないパス → 404 ---
CODE=$(curl -s -o /dev/null -w "%{http_code}" "$API/nonexistent")
[ "$CODE" = "404" ] && pass "TC-S8-01: 存在しないパス → 404" || fail "TC-S8-01" "code=$CODE"

# --- TC-S8-02: 不正 JSON → 400 ---
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" -d 'not json')
[ "$CODE" = "400" ] && pass "TC-S8-02: 不正 JSON → 400" || fail "TC-S8-02" "code=$CODE"

# --- TC-S8-03: 必須フィールド欠損 → 400 ---
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" -d '{"start":"2026-04-10 09:00"}')
[ "$CODE" = "400" ] && pass "TC-S8-03: end 欠損 → 400" || fail "TC-S8-03" "code=$CODE"

# --- TC-S8-04: GET on POST endpoint → 404 ---
CODE=$(curl -s -o /dev/null -w "%{http_code}" "$API/replay/toggle")
[ "$CODE" = "404" ] && pass "TC-S8-04: GET on POST endpoint → 404" || fail "TC-S8-04" "code=$CODE"

curl -s -X POST "$API/replay/toggle" > /dev/null  # Replay モードへ

# --- TC-S8-05: start > end → 200 で受理されるが Toast 通知 + Playing にならない ---
TMPBODY=$(mktemp /tmp/e2e_s8.XXXXXX)
CODE=$(curl -s -o "$TMPBODY" -w "%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d '{"start":"2026-04-13 10:00","end":"2026-04-13 09:00"}')
rm -f "$TMPBODY"
[ "$CODE" = "200" ] && pass "TC-S8-05a: start>end → HTTP 200" || fail "TC-S8-05a" "code=$CODE"
ST_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[[ "$ST_AFTER" = "null" || "$ST_AFTER" = "Paused" ]] && pass "TC-S8-05b: Playing に遷移しない" || \
  fail "TC-S8-05b" "status=$ST_AFTER"
HAS_ERR=$(has_notification "Start time")
[ "$HAS_ERR" = "true" ] && pass "TC-S8-05c: エラートーストが発火" || \
  fail "TC-S8-05c" "start>end の toast が発火していない"

# --- TC-S8-06: 未来日時 → 受理 → プリフェッチ完了するが EventStore 空 → Step 無効 ---
FUTURE_START="2030-01-01 00:00"
FUTURE_END="2030-01-01 06:00"
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$FUTURE_START\",\"end\":\"$FUTURE_END\"}")
[ "$CODE" = "200" ] && pass "TC-S8-06a: 未来日時 → HTTP 200" || fail "TC-S8-06a" "code=$CODE"
sleep 30
ST6=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ST6" != "Playing" ] && pass "TC-S8-06b: 未来日時 → Playing にならない (status=$ST6)" || \
  fail "TC-S8-06b" "想定外に Playing 到達"
PRE6=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
POST6=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
if [ "$PRE6" = "null" ] && [ "$POST6" = "null" ]; then
  pass "TC-S8-06c: clock 未起動のまま Step 無効"
else
  EQ6=$(bigt_eq "$PRE6" "$POST6")
  [ "$EQ6" = "true" ] && pass "TC-S8-06c: 空 EventStore → Step 無効" || \
    fail "TC-S8-06c" "想定外の前進 pre=$PRE6 post=$POST6"
fi

# --- TC-S8-07: 不正なフォーマット → 400 ---
for bad_date in "2026/04/10 09:00" "2026-04-10" "not-a-date" ""; do
  CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
    -H "Content-Type: application/json" \
    -d "{\"start\":\"$bad_date\",\"end\":\"2026-04-10 15:00\"}")
  [ "$CODE" = "400" ] && pass "TC-S8-07: 不正フォーマット '$bad_date' → 400" || \
    fail "TC-S8-07" "'$bad_date' → $CODE (expected 400)"
done

# --- TC-S8-08: pane/split に不正 UUID → 400 ---
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/pane/split" \
  -H "Content-Type: application/json" \
  -d '{"pane_id":"not-a-uuid","axis":"Vertical"}')
[ "$CODE" = "400" ] && pass "TC-S8-08: 不正 UUID → 400" || fail "TC-S8-08" "code=$CODE"

# --- TC-S8-09: pane/split に不正 axis → 400 ---
PANE_LIST=$(curl -s "$API/pane/list")
PANE_ID=$(node -e "
  const d = JSON.parse(process.argv[1]);
  const panes = d.panes || d;
  const arr = Array.isArray(panes) ? panes : Object.values(panes);
  console.log((arr[0]||{}).id || (arr[0]||{}).pane_id || '');
" "$PANE_LIST" 2>/dev/null || echo "")
if [ -n "$PANE_ID" ]; then
  CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/pane/split" \
    -H "Content-Type: application/json" \
    -d "{\"pane_id\":\"$PANE_ID\",\"axis\":\"Diagonal\"}")
  [ "$CODE" = "400" ] && pass "TC-S8-09: 不正 axis → 400" || fail "TC-S8-09" "code=$CODE"
fi

restore_state
print_summary
