#!/bin/bash
# s8_error_boundary.sh — スイート S8: エラー・境界値ケース
#
# 検証シナリオ:
#   TC-S8-01: 存在しないパス → HTTP 404
#   TC-S8-02: 不正 JSON → HTTP 400
#   TC-S8-03: 必須フィールド (end) 欠損 → HTTP 400
#   TC-S8-04: GET on POST エンドポイント → HTTP 404
#   TC-S8-05a〜c: start > end → HTTP 200・Playing 遷移なし・エラートースト発火
#   TC-S8-06a〜c: 未来日時 → HTTP 200・Playing/Paused 到達（Loading ハングなし）
#   TC-S8-07: 不正フォーマット（複数パターン）→ HTTP 400
#   TC-S8-08: pane/split に不正 UUID → HTTP 400
#   TC-S8-09: pane/split に不正 axis → HTTP 400
#
# 仕様根拠:
#   docs/replay_header.md §10 — エラーハンドリング・入力バリデーション
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, Live モード起動（HTTP API エラー系テスト）
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

# --- TC-S8-02: 不正 JSON → 400 + error フィールド ---
RESP=$(curl -s -w "\n%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" -d 'not json')
CODE=$(echo "$RESP" | tail -1)
BODY=$(echo "$RESP" | head -1)
HAS_ERR=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.error?'true':'false');}catch(e){console.log('false');}" "$BODY")
[ "$CODE" = "400" ] && pass "TC-S8-02a: 不正 JSON → 400" || fail "TC-S8-02a" "code=$CODE"
[ "$HAS_ERR" = "true" ] && pass "TC-S8-02b: 不正 JSON → error フィールドあり" || fail "TC-S8-02b" "body=$BODY"

# --- TC-S8-03: 必須フィールド欠損 → 400 + error フィールド ---
RESP=$(curl -s -w "\n%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" -d '{"start":"2026-04-10 09:00"}')
CODE=$(echo "$RESP" | tail -1)
BODY=$(echo "$RESP" | head -1)
HAS_ERR=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.error?'true':'false');}catch(e){console.log('false');}" "$BODY")
[ "$CODE" = "400" ] && pass "TC-S8-03a: end 欠損 → 400" || fail "TC-S8-03a" "code=$CODE"
[ "$HAS_ERR" = "true" ] && pass "TC-S8-03b: end 欠損 → error フィールドあり" || fail "TC-S8-03b" "body=$BODY"

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

# --- TC-S8-06: 未来日時 → 受理 → R4-6 以降: 空 klines でも Playing 開始 → クラッシュしない ---
# R4-6 でも「空 klines = stream 登録済み」とみなすよう変更したため、
# 未来日時でもプリフェッチ完了後に Playing に遷移する。これは意図した動作変更。
FUTURE_START="2030-01-01 00:00"
FUTURE_END="2030-01-01 06:00"
FUTURE_START_MS=$(node -e "console.log(new Date('${FUTURE_START}:00Z').getTime())")
FUTURE_END_MS=$(node -e "console.log(new Date('${FUTURE_END}:00Z').getTime())")
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$FUTURE_START\",\"end\":\"$FUTURE_END\"}")
[ "$CODE" = "200" ] && pass "TC-S8-06a: 未来日時 → HTTP 200" || fail "TC-S8-06a" "code=$CODE"
sleep 30
ST6=$(jqn "$(curl -s "$API/replay/status")" "d.status")
# R4-6 以降: 空 klines でも stream 登録 → Playing に遷移 (Loading ハングは発生しない)
[[ "$ST6" = "Playing" || "$ST6" = "Paused" ]] && \
  pass "TC-S8-06b: 未来日時でも Playing/Paused に遷移 (Loading ハングなし, status=$ST6)" || \
  fail "TC-S8-06b" "status=$ST6 (expected Playing or Paused — Loading ハングの疑い)"
CT6=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
if [ "$CT6" != "null" ] && [ -n "$CT6" ]; then
  IN_RANGE=$(node -e "console.log(BigInt('$CT6') >= BigInt('$FUTURE_START_MS') && BigInt('$CT6') <= BigInt('$FUTURE_END_MS'))")
  [ "$IN_RANGE" = "true" ] \
    && pass "TC-S8-06c: current_time=$CT6 は future range 内（clock 正常起動）" \
    || fail "TC-S8-06c" "current_time=$CT6 は range [$FUTURE_START_MS, $FUTURE_END_MS] 外"
else
  fail "TC-S8-06c" "current_time が null（clock 未起動）"
fi

# --- TC-S8-07: 不正なフォーマット → 400 + error フィールド ---
for bad_date in "2026/04/10 09:00" "2026-04-10" "not-a-date" ""; do
  RESP=$(curl -s -w "\n%{http_code}" -X POST "$API/replay/play" \
    -H "Content-Type: application/json" \
    -d "{\"start\":\"$bad_date\",\"end\":\"2026-04-10 15:00\"}")
  CODE=$(echo "$RESP" | tail -1)
  BODY=$(echo "$RESP" | head -1)
  HAS_ERR=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.error?'true':'false');}catch(e){console.log('false');}" "$BODY")
  [ "$CODE" = "400" ] && pass "TC-S8-07a: 不正フォーマット '$bad_date' → 400" || \
    fail "TC-S8-07a" "'$bad_date' → $CODE (expected 400)"
  [ "$HAS_ERR" = "true" ] && pass "TC-S8-07b: '$bad_date' → error フィールドあり" || \
    fail "TC-S8-07b" "'$bad_date' body=$BODY"
done

# --- TC-S8-07c: datetime 境界値（不正日付 → 400 / 有効うるう年日付 → 200）---
for inv_date in "2026-02-30 10:00" "2026-04-10 25:00" "2026-13-01 09:00"; do
  RESP=$(curl -s -w "\n%{http_code}" -X POST "$API/replay/play" \
    -H "Content-Type: application/json" \
    -d "{\"start\":\"$inv_date\",\"end\":\"2026-04-10 15:00\"}")
  CODE=$(echo "$RESP" | tail -1)
  BODY=$(echo "$RESP" | head -1)
  HAS_ERR=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.error?'true':'false');}catch(e){console.log('false');}" "$BODY")
  [ "$CODE" = "400" ] && pass "TC-S8-07c: 不正日付 '$inv_date' → 400" || \
    fail "TC-S8-07c" "'$inv_date' → $CODE (expected 400)"
  [ "$HAS_ERR" = "true" ] && pass "TC-S8-07d: '$inv_date' → error フィールドあり" || \
    fail "TC-S8-07d" "'$inv_date' body=$BODY"
done
# うるう年 2/29 は有効 → 200
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d '{"start":"2024-02-29 10:00","end":"2024-02-29 12:00"}')
[ "$CODE" = "200" ] && pass "TC-S8-07e: うるう年 2024-02-29 → 200（有効日付）" || \
  fail "TC-S8-07e" "code=$CODE (expected 200)"

# --- TC-S8-08: pane/split に不正 UUID → 400 + error フィールド ---
RESP=$(curl -s -w "\n%{http_code}" -X POST "$API/pane/split" \
  -H "Content-Type: application/json" \
  -d '{"pane_id":"not-a-uuid","axis":"Vertical"}')
CODE=$(echo "$RESP" | tail -1)
BODY=$(echo "$RESP" | head -1)
HAS_ERR=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.error?'true':'false');}catch(e){console.log('false');}" "$BODY")
[ "$CODE" = "400" ] && pass "TC-S8-08a: 不正 UUID → 400" || fail "TC-S8-08a" "code=$CODE"
[ "$HAS_ERR" = "true" ] && pass "TC-S8-08b: 不正 UUID → error フィールドあり" || fail "TC-S8-08b" "body=$BODY"

# --- TC-S8-09: pane/split に不正 axis → 400 + error フィールド ---
PANE_LIST=$(curl -s "$API/pane/list")
PANE_ID=$(node -e "
  const d = JSON.parse(process.argv[1]);
  const panes = d.panes || d;
  const arr = Array.isArray(panes) ? panes : Object.values(panes);
  console.log((arr[0]||{}).id || (arr[0]||{}).pane_id || '');
" "$PANE_LIST" 2>/dev/null || echo "")
if [ -n "$PANE_ID" ]; then
  RESP=$(curl -s -w "\n%{http_code}" -X POST "$API/pane/split" \
    -H "Content-Type: application/json" \
    -d "{\"pane_id\":\"$PANE_ID\",\"axis\":\"Diagonal\"}")
  CODE=$(echo "$RESP" | tail -1)
  BODY=$(echo "$RESP" | head -1)
  HAS_ERR=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.error?'true':'false');}catch(e){console.log('false');}" "$BODY")
  [ "$CODE" = "400" ] && pass "TC-S8-09a: 不正 axis → 400" || fail "TC-S8-09a" "code=$CODE"
  [ "$HAS_ERR" = "true" ] && pass "TC-S8-09b: 不正 axis → error フィールドあり" || fail "TC-S8-09b" "body=$BODY"
fi

# --- TC-S8-10: pane_id edge case（POST /api/pane/set-ticker）---
RESP=$(curl -s -w "\n%{http_code}" -X POST "$API/pane/set-ticker" \
  -H "Content-Type: application/json" \
  -d '{"pane_id":"","ticker":"BinanceLinear:BTCUSDT"}')
CODE=$(echo "$RESP" | tail -1)
[ "$CODE" = "400" ] && pass "TC-S8-10a: pane_id 空文字 → 400" || fail "TC-S8-10a" "code=$CODE"

RESP=$(curl -s -w "\n%{http_code}" -X POST "$API/pane/set-ticker" \
  -H "Content-Type: application/json" \
  -d '{"pane_id":"not-a-uuid","ticker":"BinanceLinear:BTCUSDT"}')
CODE=$(echo "$RESP" | tail -1)
[ "$CODE" = "404" ] || [ "$CODE" = "400" ] && pass "TC-S8-10b: 不正 UUID → 404 or 400" || \
  fail "TC-S8-10b" "code=$CODE (expected 404 or 400)"

# --- TC-S8-11: set-timeframe validation ---
if [ -n "$PANE_ID" ]; then
  for bad_tf in "M999" ""; do
    RESP=$(curl -s -w "\n%{http_code}" -X POST "$API/pane/set-timeframe" \
      -H "Content-Type: application/json" \
      -d "{\"pane_id\":\"$PANE_ID\",\"timeframe\":\"$bad_tf\"}")
    CODE=$(echo "$RESP" | tail -1)
    BODY=$(echo "$RESP" | head -1)
    HAS_ERR=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.error?'true':'false');}catch(e){console.log('false');}" "$BODY")
    # set-timeframe の不正値は app 層でエラーとなり HTTP 200 + error フィールドで返る（route 層では 400 にならない）
    [ "$CODE" = "200" ] && [ "$HAS_ERR" = "true" ] \
      && pass "TC-S8-11a: timeframe='$bad_tf' → HTTP 200 + error フィールド（app 層エラー）" \
      || fail "TC-S8-11a" "timeframe='$bad_tf' code=$CODE has_err=$HAS_ERR (expected 200+error)"
    [ "$HAS_ERR" = "true" ] && pass "TC-S8-11b: timeframe='$bad_tf' → error フィールドあり" || \
      fail "TC-S8-11b" "timeframe='$bad_tf' body=$BODY"
  done
fi

restore_state
print_summary
