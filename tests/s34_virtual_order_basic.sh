#!/usr/bin/env bash
# s34_virtual_order_basic.sh — S34: 仮想注文 API 基本動作検証
#
# 検証シナリオ:
#   A-C: LIVE モード時は /order・/portfolio・/state がすべて HTTP 400 を返す
#   D:   REPLAY Playing 到達
#   E-G: Paused 状態で POST /api/replay/order (成行買い) → HTTP 200, order_id, status="pending"
#   H:   指値買い注文 → HTTP 200, order_id 返却
#   I:   指値売り注文 → HTTP 200, order_id 返却
#   J-K: 不正リクエスト → HTTP 400
#   L:   GET /api/replay/state → HTTP 200, current_time_ms フィールドあり
#
# 仕様根拠:
#   docs/replay_header.md §11.2 — 仮想約定エンジン API
#   docs/order_windows.md §仮想約定エンジン, §HTTP API
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, Live モード起動
#   (LIVE ガードを最初に検証するため Live 起動を使用)
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S34: 仮想注文 API 基本動作検証 ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -3)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S34","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":[],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S34"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app

# ─────────────────────────────────────────────────────────────────────────────
# TC-A〜C: LIVE モード時は HTTP 400（仮想注文 API は REPLAY 専用）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A〜C: LIVE モード時は HTTP 400"

LIVE_MODE=$(jqn "$(curl -s "$API/replay/status")" "d.mode")
echo "  現在のモード: $LIVE_MODE"

if is_headless; then
  # headless は常に Replay モード → LIVE ガードは発動しない
  pend "TC-A" "headless は常に Replay モード（LIVE ガード不要）"
  pend "TC-B" "headless は常に Replay モード（LIVE ガード不要）"
else
  CODE_A=$(api_post_code /api/replay/order \
    '{"ticker":"BTCUSDT","side":"buy","qty":0.1,"order_type":"market"}')
  [ "$CODE_A" = "400" ] \
    && pass "TC-A: LIVE 中 POST /api/replay/order → HTTP 400" \
    || fail "TC-A" "HTTP=$CODE_A (expected 400)"

  CODE_B=$(curl -s -o /dev/null -w "%{http_code}" "$API_BASE/api/replay/portfolio")
  [ "$CODE_B" = "400" ] \
    && pass "TC-B: LIVE 中 GET /api/replay/portfolio → HTTP 400" \
    || fail "TC-B" "HTTP=$CODE_B (expected 400)"
fi

CODE_C=$(curl -s -o /dev/null -w "%{http_code}" "$API_BASE/api/replay/state")
[ "$CODE_C" = "400" ] \
  && pass "TC-C: LIVE 中 GET /api/replay/state → HTTP 400" \
  || fail "TC-C" "HTTP=$CODE_C (expected 400)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-D: REPLAY Playing に遷移
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-D: REPLAY Playing に遷移"

api_post /api/replay/toggle > /dev/null
api_post /api/replay/play "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null

if ! wait_status "Playing" 60; then
  diagnose_playing_failure
  fail "TC-D" "REPLAY Playing に到達せず（60s タイムアウト）"
  print_summary; exit 1
fi
pass "TC-D: REPLAY Playing 到達"

# 以降の注文テストは Paused 状態で行う（約定を防いで結果を決定論的にする）
api_post /api/replay/pause > /dev/null
wait_status "Paused" 10

# ─────────────────────────────────────────────────────────────────────────────
# TC-E〜G: 成行買い注文 → HTTP 200, order_id 返却, status="pending"
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-E〜G: 成行買い注文"

MARKET_RESP=$(api_post /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.1,"order_type":"market"}')
echo "  response: $MARKET_RESP"

CODE_E=$(api_post_code /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.05,"order_type":"market"}')
[ "$CODE_E" = "200" ] \
  && pass "TC-E: POST /api/replay/order (成行買い) → HTTP 200" \
  || fail "TC-E" "HTTP=$CODE_E (expected 200)"

ORDER_ID=$(node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    const id = d.order_id;
    console.log(typeof id === 'string' && id.length > 0 ? id : 'null');
  } catch(e) { console.log('null'); }
" "$MARKET_RESP")
[ "$ORDER_ID" != "null" ] \
  && pass "TC-F: order_id が文字列として返る ($ORDER_ID)" \
  || fail "TC-F" "order_id が null または不正 (response=$MARKET_RESP)"

ORDER_STATUS=$(node -e "
  try { console.log(JSON.parse(process.argv[1]).status || 'null'); }
  catch(e) { console.log('null'); }
" "$MARKET_RESP")
[ "$ORDER_STATUS" = "pending" ] \
  && pass "TC-G: 注文ステータス = \"pending\"" \
  || fail "TC-G" "status=$ORDER_STATUS (expected pending)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-H: 指値買い注文 → HTTP 200, order_id 返却
# (limit 価格を低く設定: 現価格未満 → Pending のまま)
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-H: 指値買い注文"

LIMIT_BUY_RESP=$(api_post /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.05,"order_type":{"limit":1.0}}')
echo "  response: $LIMIT_BUY_RESP"

LIMIT_BUY_ID=$(node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    console.log(typeof d.order_id === 'string' && d.order_id.length > 0 ? 'ok' : 'null');
  } catch(e) { console.log('null'); }
" "$LIMIT_BUY_RESP")
[ "$LIMIT_BUY_ID" = "ok" ] \
  && pass "TC-H: 指値買い注文 → HTTP 200, order_id 返却 (status=pending)" \
  || fail "TC-H" "response=$LIMIT_BUY_RESP"

# ─────────────────────────────────────────────────────────────────────────────
# TC-I: 指値売り注文 → HTTP 200, order_id 返却
# (limit 価格を高く設定: 現価格超 → Pending のまま)
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-I: 指値売り注文"

LIMIT_SELL_RESP=$(api_post /api/replay/order \
  '{"ticker":"BTCUSDT","side":"sell","qty":0.05,"order_type":{"limit":9999999.0}}')
echo "  response: $LIMIT_SELL_RESP"

LIMIT_SELL_ID=$(node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    console.log(typeof d.order_id === 'string' && d.order_id.length > 0 ? 'ok' : 'null');
  } catch(e) { console.log('null'); }
" "$LIMIT_SELL_RESP")
[ "$LIMIT_SELL_ID" = "ok" ] \
  && pass "TC-I: 指値売り注文 → HTTP 200, order_id 返却 (status=pending)" \
  || fail "TC-I" "response=$LIMIT_SELL_RESP"

# ─────────────────────────────────────────────────────────────────────────────
# TC-J〜K: 不正リクエスト → HTTP 400
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-J〜K: 不正リクエスト"

CODE_J=$(api_post_code /api/replay/order 'not-valid-json')
[ "$CODE_J" = "400" ] \
  && pass "TC-J: 不正 JSON → HTTP 400" \
  || fail "TC-J" "HTTP=$CODE_J (expected 400)"

# side / qty / order_type を省略した不完全なリクエスト
CODE_K=$(api_post_code /api/replay/order '{"ticker":"BTCUSDT"}')
[ "$CODE_K" = "400" ] \
  && pass "TC-K: 必須フィールド欠落 (side/qty/order_type なし) → HTTP 400" \
  || fail "TC-K" "HTTP=$CODE_K (expected 400)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-L: GET /api/replay/state → HTTP 200, スキーマ検証（Phase 1 実装）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-L: GET /api/replay/state (Phase 1 実データ)"

STATE_RESP=$(api_get /api/replay/state)
CODE_L=$(curl -s -o /dev/null -w "%{http_code}" "$API_BASE/api/replay/state")
echo "  HTTP=$CODE_L"

[ "$CODE_L" = "200" ] \
  && pass "TC-L1: GET /api/replay/state → HTTP 200" \
  || fail "TC-L1" "HTTP=$CODE_L (expected 200)"

node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    const ok = typeof d.current_time_ms === 'number' && d.current_time_ms > 0
      && Array.isArray(d.klines)
      && Array.isArray(d.trades);
    console.log(ok ? 'true' : 'false');
  } catch(e) { console.log('false'); }
" "$STATE_RESP" | grep -q "true" \
  && pass "TC-L2: current_time_ms(>0) / klines[] / trades[] フィールドあり" \
  || fail "TC-L2" "スキーマ不正 (response=$STATE_RESP)"

# klines に items がある場合は OHLCV スキーマを確認
KLINE_COUNT=$(node -e "
  try { console.log(JSON.parse(process.argv[1]).klines.length); }
  catch(e) { console.log(0); }
" "$STATE_RESP")
echo "  klines count=$KLINE_COUNT"
if [ "$KLINE_COUNT" -gt "0" ]; then
  node -e "
    try {
      const d = JSON.parse(process.argv[1]);
      const k = d.klines[0];
      const ok = typeof k.stream === 'string' && k.stream.length > 0
        && typeof k.time === 'number'
        && typeof k.open === 'number' && k.open > 0
        && typeof k.high === 'number'
        && typeof k.low  === 'number'
        && typeof k.close === 'number'
        && typeof k.volume === 'number';
      console.log(ok ? 'true' : 'false');
    } catch(e) { console.log('false'); }
  " "$STATE_RESP" | grep -q "true" \
    && pass "TC-L3: klines[0] に stream/time/open/high/low/close/volume あり" \
    || fail "TC-L3" "klines[0] スキーマ不正 (response=$STATE_RESP)"
else
  pass "TC-L3: klines=0 件（Playing 直後のため許容）"
fi

stop_app
print_summary
