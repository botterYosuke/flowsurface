#!/usr/bin/env bash
# s44_order_list.sh — 注文一覧取得 E2E テスト（シナリオ 2-1）
#
# 検証シナリオ:
#   1: Live モードで起動・デモ環境ガード確認
#   2: GET /api/tachibana/orders が HTTP 200 を返す
#   3: レスポンスが {"orders":[...]} 形式の JSON であることを確認
#   4: GET /api/tachibana/orders?eig_day=今日 でも動作確認
#
# 前提: DEV_IS_DEMO=true / DEV_USER_ID / DEV_PASSWORD が設定済みであること
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

# デモ環境ガード（本番誤発注防止）
[ "${DEV_IS_DEMO:-}" = "true" ] \
  || { echo "ERROR: DEV_IS_DEMO=true を設定してください（本番誤発注防止）"; exit 1; }
[ -n "${DEV_USER_ID:-}" ] \
  || { echo "ERROR: DEV_USER_ID が未設定です"; exit 1; }
[ -n "${DEV_PASSWORD:-}" ] \
  || { echo "ERROR: DEV_PASSWORD が未設定です"; exit 1; }

echo "=== 注文一覧取得 E2E テスト ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"Toyota-Live","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"TachibanaSpot:7203","timeframe":"D1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"D1"}},
      "indicators":[],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"Toyota-Live"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app

# ── Step 1: Live モード・セッション確認 ──────────────────────────────────────
echo ""
echo "── Step 1: Live モード確認"
STATUS=$(curl -s "$API/replay/status")
MODE=$(node -e "try{console.log(JSON.parse(process.argv[1]).mode||'null');}catch(e){console.log('null');}" "$STATUS")
[ "$MODE" = "Live" ] \
  && pass "Step 1: Live モード確認 (mode=$MODE)" \
  || fail "Step 1" "mode=$MODE (expected Live)"

echo ""
echo "── Step 1b: Tachibana デモセッション待機"
wait_tachibana_session 120 \
  && pass "Step 1b: デモセッション確立" \
  || fail "Step 1b" "セッション未確立"

# ── Step 2: 注文一覧取得（全件） ──────────────────────────────────────────────
echo ""
echo "── Step 2: GET /api/tachibana/orders (全件)"

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$API/tachibana/orders")
[ "$HTTP_CODE" = "200" ] \
  && pass "Step 2: GET /api/tachibana/orders → HTTP 200" \
  || fail "Step 2" "HTTP=$HTTP_CODE (expected 200)"

# ── Step 3: レスポンス形式確認 ────────────────────────────────────────────────
echo ""
echo "── Step 3: レスポンス JSON 形式確認"

RESP=$(curl -s "$API/tachibana/orders")
echo "  response: $RESP"

HAS_ORDERS=$(node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    console.log(Array.isArray(d.orders) ? 'true' : 'false');
  } catch(e) { console.log('false'); }
" "$RESP")
[ "$HAS_ORDERS" = "true" ] \
  && pass "Step 3: orders フィールドが配列であることを確認" \
  || fail "Step 3" "orders フィールドが配列でない: $RESP"

ORDER_COUNT=$(node -e "
  try { console.log(JSON.parse(process.argv[1]).orders.length); }
  catch(e) { console.log('0'); }
" "$RESP")
echo "  注文件数: $ORDER_COUNT"
pass "Step 3b: 注文件数確認 ($ORDER_COUNT 件)"

# ── Step 4: eig_day クエリパラメータ付き ─────────────────────────────────────
echo ""
echo "── Step 4: GET /api/tachibana/orders?eig_day=今日"

TODAY=$(date +%Y%m%d)
HTTP_CODE2=$(curl -s -o /dev/null -w "%{http_code}" "$API/tachibana/orders?eig_day=$TODAY")
[ "$HTTP_CODE2" = "200" ] \
  && pass "Step 4: GET /api/tachibana/orders?eig_day=$TODAY → HTTP 200" \
  || fail "Step 4" "HTTP=$HTTP_CODE2 (expected 200)"

# ── Step 5: エラーケース — 存在しない注文IDの明細取得 ─────────────────────────
echo ""
echo "── Step 5: GET /api/tachibana/order/00000000 (存在しない注文番号)"

DETAIL_RESP=$(curl -s "$API/tachibana/order/00000000")
echo "  response: $DETAIL_RESP"
HTTP_DETAIL=$(curl -s -o /dev/null -w "%{http_code}" "$API/tachibana/order/00000000")
# API は 200 を返し、body 内に error または空の executions が来る
[ "$HTTP_DETAIL" = "200" ] \
  && pass "Step 5: GET /api/tachibana/order/00000000 → HTTP $HTTP_DETAIL（正常応答）" \
  || fail "Step 5" "HTTP=$HTTP_DETAIL (expected 200)"

stop_app
print_summary
[ $FAIL -eq 0 ]
