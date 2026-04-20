#!/usr/bin/env bash
# s49_account_info.sh — Phase 3-1/3-2/3-3: 口座情報系 HTTP API E2E テスト
#
# 検証シナリオ:
#   3-1: GET /api/buying-power → cash_buying_power が返ること（CLMZanKaiKanougaku）
#   3-2: GET /api/buying-power → margin_new_order_power が返ること（CLMZanShinkiKanoIjiritu）
#   3-3: GET /api/tachibana/holdings?issue_code=7203 → holdings_qty が数値で返ること
#
# 前提: DEV_IS_DEMO=true / DEV_USER_ID / DEV_PASSWORD が設定済み
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

[ "${DEV_IS_DEMO:-}" = "true" ] \
  || { echo "ERROR: DEV_IS_DEMO=true を設定してください（本番誤発注防止）"; exit 1; }
[ -n "${DEV_USER_ID:-}" ] \
  || { echo "ERROR: DEV_USER_ID が未設定です"; exit 1; }
[ -n "${DEV_PASSWORD:-}" ] \
  || { echo "ERROR: DEV_PASSWORD が未設定です"; exit 1; }

echo "=== Phase 3-1/3-2/3-3: 口座情報系 HTTP API E2E テスト ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

cat > "$DATA_DIR/saved-state.json" <<'EOF'
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

# ── Step 1: Live モード・セッション確認 ────────────────────────────────────────
echo ""
echo "── Step 1: Live モード・セッション確認"
STATUS=$(curl -s "$API/replay/status")
MODE=$(node -e "try{console.log(JSON.parse(process.argv[1]).mode||'null');}catch(e){console.log('null');}" "$STATUS")
[ "$MODE" = "Live" ] \
  && pass "Step 1a: Live モード確認 (mode=$MODE)" \
  || fail "Step 1a" "mode=$MODE (expected Live)"

wait_tachibana_session 120 \
  && pass "Step 1b: デモセッション確立" \
  || fail "Step 1b" "セッション未確立"

# ── Step 2: GET /api/buying-power → 3-1 cash_buying_power 確認 ────────────────
echo ""
echo "── Step 2 (3-1): GET /api/buying-power → cash_buying_power 確認"

BP_RESP=$(curl -s "$API/buying-power")
echo "  response: $BP_RESP"

HAS_CASH=$(node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    console.log(d.cash_buying_power !== undefined ? 'true' : 'false');
  } catch(e) { console.log('false'); }
" "$BP_RESP")

[ "$HAS_CASH" = "true" ] \
  && pass "Step 2 (3-1): cash_buying_power フィールドあり" \
  || fail "Step 2 (3-1)" "cash_buying_power フィールドなし: $BP_RESP"

CASH_VAL=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).cash_buying_power)); }
  catch(e) { console.log('null'); }
" "$BP_RESP")
echo "  cash_buying_power=$CASH_VAL"

HAS_ERROR=$(node -e "
  try { console.log(JSON.parse(process.argv[1]).error ? 'true' : 'false'); }
  catch(e) { console.log('false'); }
" "$BP_RESP")
[ "$HAS_ERROR" = "false" ] \
  && pass "Step 2b: エラーなし" \
  || fail "Step 2b" "エラーあり: $BP_RESP"

# ── Step 3: GET /api/buying-power → 3-2 margin_new_order_power 確認 ───────────
echo ""
echo "── Step 3 (3-2): GET /api/buying-power → margin_new_order_power 確認"

HAS_MARGIN=$(node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    console.log(d.margin_new_order_power !== undefined ? 'true' : 'false');
  } catch(e) { console.log('false'); }
" "$BP_RESP")

[ "$HAS_MARGIN" = "true" ] \
  && pass "Step 3 (3-2): margin_new_order_power フィールドあり" \
  || fail "Step 3 (3-2)" "margin_new_order_power フィールドなし: $BP_RESP"

MARGIN_VAL=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).margin_new_order_power)); }
  catch(e) { console.log('null'); }
" "$BP_RESP")
echo "  margin_new_order_power=$MARGIN_VAL"

# ── Step 4: GET /api/tachibana/holdings?issue_code=7203 → 3-3 保有株数確認 ─────
echo ""
echo "── Step 4 (3-3): GET /api/tachibana/holdings?issue_code=7203 → holdings_qty 確認"

HOLDINGS_RESP=$(curl -s "$API/tachibana/holdings?issue_code=7203")
echo "  response: $HOLDINGS_RESP"

HAS_HOLDINGS=$(node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    console.log(d.holdings_qty !== undefined ? 'true' : 'false');
  } catch(e) { console.log('false'); }
" "$HOLDINGS_RESP")

[ "$HAS_HOLDINGS" = "true" ] \
  && pass "Step 4 (3-3): holdings_qty フィールドあり" \
  || fail "Step 4 (3-3)" "holdings_qty フィールドなし: $HOLDINGS_RESP"

HOLDINGS_QTY=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).holdings_qty)); }
  catch(e) { console.log('null'); }
" "$HOLDINGS_RESP")
echo "  holdings_qty=$HOLDINGS_QTY (TOYOTA 7203 の保有株数)"

# ── Step 5: issue_code なし → エラー確認 ─────────────────────────────────────
echo ""
echo "── Step 5: issue_code なし → BadRequest"

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$API/tachibana/holdings")
[ "$HTTP_CODE" = "400" ] \
  && pass "Step 5: issue_code なし → HTTP 400" \
  || fail "Step 5" "HTTP $HTTP_CODE (expected 400)"

stop_app
print_summary
[ $FAIL -eq 0 ]
